use std::collections::{HashMap, HashSet};

use super::col_refs::collect_col_refs;
use super::column_exprs::select_arg_type;
use super::column_methods::{
    apply_melt, apply_pandas_assign, apply_pandas_drop_columns, apply_rename_dict,
    apply_with_columns, apply_with_columns_renamed, check_column_method_args,
    check_fillna_dict_keys, check_pandas_assign_enum_sinks, check_subset_kwarg,
    check_with_column_enum_sink, check_with_columns_enum_sinks, melt_value_column_type,
    parse_string_list, report_column_refs,
};
use super::context::{BodyContext, MAX_INFER_DEPTH};
use super::driver::{check_function_body, inherited_dialect};
use super::shapes::{
    column_method_shape, is_spark_opaque_source_call, is_terminal_method, two_df_method,
};
use super::strict_operators::{
    apply_column_method, apply_select_expr, explode_map_aliased_fields, posexplode_fields,
    report_expr_sql_refs, report_expr_type_errors, report_sql_column_refs, resolve_spark_sql_call,
    select_output_name,
};
use super::two_df::{handle_two_df_method, strip_nullability};

use ruff_python_ast::{Expr, ExprAttribute, ExprCall, StmtFunctionDef};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{self, DataFrameAnnotation, Dialect};
use crate::diagnostics::{Diagnostic, Severity};
use crate::dialect_signals::PANDAS_INHERITED_ARMS;
use crate::registry::{MethodParam, ParamKind};
use crate::schema::{
    DerivedField, FieldPathResult, Schema, SchemaView, resolve_path, suggest_field_name,
};
use crate::types::ColumnType;
use crate::walk::DiscoveredFunction;

// ---------------------------------------------------------------------------
// analyze_expr — the recursive heart
// ---------------------------------------------------------------------------

/// `check_call_argument_schemas` already walked this call's args via
/// `analyze_expr` (one walk per arg, regardless of slot annotation —
/// see v1.4 PR-B Bug 1) and re-walking via the §10 fallback would
/// duplicate every nested D0030/D0051. True when the callee is a
/// bare-Name reference to a non-shadowed registry function — matches
/// the early-return guards inside `check_call_argument_schemas`.
fn is_registry_function_call(call: &ExprCall, ctx: &BodyContext<'_>) -> bool {
    let Some(name_expr) = call.func.as_name_expr() else {
        return false;
    };
    if ctx.is_locally_bound(name_expr.id.as_str()) {
        return false;
    }
    ctx.registry()
        .find_function(name_expr.id.as_str())
        .is_some()
}

/// Outcome of dispatching one of pykrete's call-shape recognizers.
/// `Handled` means the dispatcher matched the call's shape (method name,
/// free-function generic, …) and walked its own args; the caller must
/// NOT re-walk the arg list. `Unhandled` means no recognizer matched —
/// the caller is responsible for walking args so embedded `df["col"]`
/// references still surface D0030. See the v1.3 §10 widening callout
/// in `Expr::Call` for the rationale.
pub(super) enum CallOutcome<'a> {
    Handled(Option<SchemaView<'a>>),
    Unhandled,
}

pub(super) fn analyze_expr<'a>(
    expr: &'a Expr,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    match expr {
        Expr::Name(n) => ctx.lookup(n.id.as_str()),
        Expr::Named(named) => {
            // Walrus (`(target := value)`) — record the target as a local
            // binding before descending, so a subsequent call on the same
            // name within this body doesn't fall through to the
            // top-level-function signature check. Python's grammar
            // restricts the LHS to a single `Name`; anything else is
            // a syntax error.
            if let Some(target) = named.target.as_name_expr() {
                ctx.mark_local(target.id.as_str());
            }
            analyze_expr(&named.value, ctx, source, line_index, diagnostics)
        }
        Expr::Call(call) => {
            // Check arguments at the call site against the callee's
            // declared `DataFrame[Schema]` parameters. Side-effect only —
            // doesn't change what schema this call evaluates to.
            check_call_argument_schemas(call, ctx, source, line_index, diagnostics);

            let method_outcome = analyze_method_call(call, ctx, source, line_index, diagnostics);
            let (result, handled) = match method_outcome {
                CallOutcome::Handled(view) => (view, true),
                CallOutcome::Unhandled => {
                    // Free-function fallback — `my_join(orders, refunds)`
                    // on a generic `def my_join[A, B](...)`. Tried only
                    // when the attribute-method path didn't recognize
                    // the call.
                    match handle_free_function_call(call, ctx, source, line_index, diagnostics) {
                        CallOutcome::Handled(view) => (view, true),
                        CallOutcome::Unhandled => (None, false),
                    }
                }
            };
            // v1.3 §10 widening — for a call neither path dispatched
            // (`print(df["typo"])`, `logger.info(msg=df["typo"])`), walk
            // every positional arg and kwarg value so an embedded
            // `df["col"]` still surfaces D0030. Handled calls already
            // walked their own args through the column-method machinery;
            // re-walking here would double-fire (e.g. D0051 for nested
            // `f(f(b))` would emit twice).
            //
            // **v1.4 PR-B Bug 1**: the bare-Name registry-call path now
            // walks every arg (typed or not) inside
            // `check_call_argument_schemas`, so the gate stays — but the
            // walk is unconditional rather than DataFrame-typed-slot
            // only. Without that, `util(df["typo"])` where `util(x: int)`
            // had no DataFrame slot went silent (the §10 fallback was
            // gated off, and the typed-slot walk skipped non-typed args).
            if !handled && !is_registry_function_call(call, ctx) {
                for arg in &call.arguments.args {
                    let _ = analyze_expr(arg, ctx, source, line_index, diagnostics);
                }
                for kw in &call.arguments.keywords {
                    let _ = analyze_expr(&kw.value, ctx, source, line_index, diagnostics);
                }
            }
            // Record the call's result schema so completion can offer
            // the chain's columns at `<call>.<cursor>`.
            if let Some(schema) = &result {
                ctx.record_call_result(call.range(), schema.clone());
            }
            result
        }
        Expr::Attribute(a) => {
            // `DataSources.RAW_ORDERS` — class-qualified annotated
            // constant. Real codebases declare every data source inside a
            // `@dataclass(frozen=True) class DataSources:` body, so this
            // shape resolves to the same `SchemaView::Declared` a
            // module-level `RAW_ORDERS: DataSource[X] = ...` would.
            if let Some(class) = a.value.as_name_expr()
                && let Some(constant) = ctx
                    .registry()
                    .find_class_constant(class.id.as_str(), a.attr.id.as_str())
                && let Some(schema) = ctx.find_schema(constant.schema_name)
            {
                return Some(SchemaView::Declared(schema));
            }
            // Otherwise this is a column access (`<chain>.colname`) — not
            // a DataFrame itself, but analyze the receiver so a call in
            // it still records its result trace for completion.
            let _ = analyze_expr(&a.value, ctx, source, line_index, diagnostics);
            None
        }
        // Compound expressions — descend into each child so any embedded
        // method call (`df.select("typo").count() > 0`) is still
        // analyzed. The result of these forms isn't itself a DataFrame,
        // so this returns None unconditionally; descent is purely a
        // side-effect to drive diagnostic collection on the children.
        Expr::Compare(c) => {
            let _ = analyze_expr(&c.left, ctx, source, line_index, diagnostics);
            for cmp in &c.comparators {
                let _ = analyze_expr(cmp, ctx, source, line_index, diagnostics);
            }
            None
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                let _ = analyze_expr(v, ctx, source, line_index, diagnostics);
            }
            None
        }
        Expr::UnaryOp(u) => {
            let _ = analyze_expr(&u.operand, ctx, source, line_index, diagnostics);
            None
        }
        Expr::BinOp(b) => {
            let _ = analyze_expr(&b.left, ctx, source, line_index, diagnostics);
            let _ = analyze_expr(&b.right, ctx, source, line_index, diagnostics);
            None
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                let _ = analyze_expr(e, ctx, source, line_index, diagnostics);
            }
            None
        }
        Expr::List(l) => {
            for e in &l.elts {
                let _ = analyze_expr(e, ctx, source, line_index, diagnostics);
            }
            None
        }
        Expr::Set(s) => {
            for e in &s.elts {
                let _ = analyze_expr(e, ctx, source, line_index, diagnostics);
            }
            None
        }
        Expr::Dict(d) => {
            for item in &d.items {
                if let Some(k) = item.key.as_ref() {
                    let _ = analyze_expr(k, ctx, source, line_index, diagnostics);
                }
                let _ = analyze_expr(&item.value, ctx, source, line_index, diagnostics);
            }
            None
        }
        Expr::If(if_exp) => {
            let _ = analyze_expr(&if_exp.test, ctx, source, line_index, diagnostics);
            let _ = analyze_expr(&if_exp.body, ctx, source, line_index, diagnostics);
            let _ = analyze_expr(&if_exp.orelse, ctx, source, line_index, diagnostics);
            None
        }
        Expr::Starred(s) => {
            let _ = analyze_expr(&s.value, ctx, source, line_index, diagnostics);
            None
        }
        Expr::Subscript(s) => {
            // v1.3 piece (b) — bare `df["col"]` / `df[["a", "b"]]` col-ref
            // check (pandas-support.md §9). Only fires when the receiver
            // is an `Expr::Name` directly bound to a frame slot; other
            // receiver shapes (Attribute, Call, IfExp) are quiet-ignored
            // per spec §9 receiver-shape bound. Spark + Pandas share the
            // same D0030 path — see spec §10 for the bonus Spark
            // coverage widening this lands.
            //
            // For all-string-literal List slices (`df[["a", "b"]]`),
            // returns a `SchemaView::Derived` of the projected columns
            // so chained analysis (`df[["a"]].select(col("a"))`)
            // preserves schema fidelity. Other slice shapes (string
            // literal scalar, variable, integer, slice, boolean mask)
            // return None — the Subscript value isn't itself a frame.
            let projection = if let Some(name) = s.value.as_name_expr()
                && let Some(view) = ctx.lookup(name.id.as_str())
            {
                report_subscript_col_refs(&s.slice, &view, ctx, source, line_index, diagnostics);
                subscript_list_projection(&s.slice, &view, ctx)
            } else if let Some((name, col_lit)) = loc_literal_subscript(s)
                && ctx.lookup_dialect(name.id.as_str()) == Some(Dialect::Pandas)
                && let Some(view) = ctx.lookup(name.id.as_str())
            {
                // v1.5 PR-C — `pdf.loc[:, "col"]` literal column projection.
                // Spec §4 promises D0030 on a typo in the column literal
                // against the pandas-tagged receiver's schema. Sibling to
                // piece (b) above: same D0030 path, narrower receiver shape
                // (`<Name>.loc` with a `(:, "literal")` tuple slice). Gated
                // to `Dialect::Pandas` because `.loc` doesn't exist on
                // Spark — a SparkFrame receiver falls through unchanged.
                // Non-literal column indexers (list-of-strings, slice,
                // callable, boolean-mask rows) are out of scope for v1.5
                // and deliberately fall through here.
                let refs = vec![(None, col_lit.value.to_str(), col_lit.range())];
                report_column_refs(&refs, &view, ctx, source, line_index, diagnostics);
                None
            } else {
                None
            };
            // `(df, df2)[1]` — neither half is a DataFrame on its own,
            // but a method call buried in either side still needs its
            // diagnostics collected. Descent here is for nested-call
            // collection only — the col-ref report above already
            // covered the literal-name shapes piece (b) owns.
            let _ = analyze_expr(&s.value, ctx, source, line_index, diagnostics);
            let _ = analyze_expr(&s.slice, ctx, source, line_index, diagnostics);
            projection
        }
        Expr::FString(f) => {
            // `f"{df.select('typo').count()}"` — descend into every
            // formatted interpolation so the embedded calls are still
            // checked. The literal parts are pure text and carry
            // nothing to analyze.
            for element in f.value.elements() {
                if let ruff_python_ast::InterpolatedStringElement::Interpolation(interp) = element {
                    let _ = analyze_expr(&interp.expression, ctx, source, line_index, diagnostics);
                }
            }
            None
        }
        Expr::ListComp(c) => {
            descend_comprehension(
                &c.generators,
                &[c.elt.as_ref()],
                ctx,
                source,
                line_index,
                diagnostics,
            );
            None
        }
        Expr::SetComp(c) => {
            descend_comprehension(
                &c.generators,
                &[c.elt.as_ref()],
                ctx,
                source,
                line_index,
                diagnostics,
            );
            None
        }
        Expr::DictComp(c) => {
            descend_comprehension(
                &c.generators,
                &[c.key.as_ref(), c.value.as_ref()],
                ctx,
                source,
                line_index,
                diagnostics,
            );
            None
        }
        Expr::Generator(g) => {
            descend_comprehension(
                &g.generators,
                &[g.elt.as_ref()],
                ctx,
                source,
                line_index,
                diagnostics,
            );
            None
        }
        // `Lambda` is deliberately NOT descended: its body sits inside a
        // new parameter scope (`lambda x: x.select("y")` binds `x` to
        // each row, not to a DataFrame in the outer ctx), and analyzing
        // the body without that scope could fire spurious D0030 on the
        // lambda parameter name. Tracked as a v1.0.1 follow-up.
        _ => None,
    }
}

/// v1.3 piece (b) col-ref entry point — given a frame-typed receiver
/// view and the *slice* of a bare `Expr::Subscript`, fire `D0030` for
/// any string-literal column name that doesn't resolve. Handles the
/// two literal shapes the spec calls out in §5 / §9:
///
/// - scalar string literal: `df["status"]`
/// - list of string literals: `df[["a", "b"]]`
///
/// Every other slice shape (boolean-mask `df[mask]`, `df[var]`, `df[0]`,
/// `df[:5]`, `df["a" + "b"]`, chained `df["a"]["b"]`) is quiet-ignored
/// per the §5 Subscript-slice taxonomy — the surrounding analyzer
/// already descends into the slice expression separately, so any
/// embedded col-ref still surfaces through the normal pipeline.
pub(super) fn report_subscript_col_refs<'a>(
    slice: &'a Expr,
    view: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    match slice {
        Expr::StringLiteral(lit) => {
            refs.push((None, lit.value.to_str(), lit.range()));
        }
        Expr::List(list) => {
            for elt in &list.elts {
                if let Expr::StringLiteral(lit) = elt {
                    refs.push((None, lit.value.to_str(), lit.range()));
                } else {
                    // Mixed shapes (`df[["a", some_var]]`) — drop the
                    // entire list. v1.3 only checks all-literal lists;
                    // partial firing would be confusing.
                    return;
                }
            }
        }
        _ => return,
    }
    if refs.is_empty() {
        return;
    }
    report_column_refs(&refs, view, ctx, source, line_index, diagnostics);
}

/// `df[["a", "b"]]` — the column-projection arm of piece (b). When the
/// slice is an all-string-literal List, returns a `SchemaView::Derived`
/// containing just those columns (with types pulled from the receiver
/// when present). Any other slice shape — scalar string, variable, slice,
/// integer, boolean mask, mixed list — returns `None` so the Subscript
/// value isn't itself treated as a frame downstream.
fn subscript_list_projection<'a>(
    slice: &'a Expr,
    view: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
) -> Option<SchemaView<'a>> {
    let list = slice.as_list_expr()?;
    let mut names: Vec<&'a str> = Vec::with_capacity(list.elts.len());
    for elt in &list.elts {
        let lit = elt.as_string_literal_expr()?;
        names.push(lit.value.to_str());
    }
    if names.is_empty() {
        return None;
    }
    let recv_fields = view.typed_fields(ctx.schemas());
    let fields: Vec<DerivedField<'a>> = names
        .into_iter()
        .map(|name| match recv_fields.iter().find(|f| f.name == name) {
            Some(f) => f.clone(),
            None => DerivedField { name, ty: None },
        })
        .collect();
    Some(SchemaView::Derived(fields))
}

/// v1.5 PR-C — `<Name>.loc[:, "col"]` literal-form match. Returns the
/// receiver `Name` and the column-name string literal when the subscript
/// has exactly this shape: receiver is `<Name>.loc` (Attribute on Name)
/// and slice is a 2-tuple of a bare `:` slice and a string literal. Every
/// other shape (boolean-mask rows, integer/label rows, list-of-strings
/// columns, slice columns, callable, variable) is v1.6+ scope and
/// returns `None` here.
pub(super) fn loc_literal_subscript(
    sub: &ruff_python_ast::ExprSubscript,
) -> Option<(
    &ruff_python_ast::ExprName,
    &ruff_python_ast::ExprStringLiteral,
)> {
    let attr = sub.value.as_attribute_expr()?;
    if attr.attr.id.as_str() != "loc" {
        return None;
    }
    let name = attr.value.as_name_expr()?;
    let tup = sub.slice.as_tuple_expr()?;
    if tup.elts.len() != 2 {
        return None;
    }
    let slice = tup.elts[0].as_slice_expr()?;
    if slice.lower.is_some() || slice.upper.is_some() || slice.step.is_some() {
        return None;
    }
    let lit = tup.elts[1].as_string_literal_expr()?;
    Some((name, lit))
}

/// Walk a comprehension expression — generator iters, ifs, and the
/// outer body parts (`elt` for list/set/generator, `key`+`value` for
/// dict). Each generator's bound names (the `target` LHS of `for X in
/// Y`) are pushed onto the ctx's comp-bound shadow set before the body
/// is walked, so a Subscript receiver matching a bound name is treated
/// as a fresh loop variable rather than checked against an outer-
/// scope frame of the same name. Bound names are popped on exit in
/// reverse order to keep nested comprehensions correct.
fn descend_comprehension<'a>(
    generators: &'a [ruff_python_ast::Comprehension],
    body_parts: &[&'a Expr],
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut pushed: Vec<&'a str> = Vec::new();
    for comp in generators {
        let _ = analyze_expr(&comp.iter, ctx, source, line_index, diagnostics);
        let mut bound_here: Vec<&'a str> = Vec::new();
        collect_comp_target_names(&comp.target, &mut bound_here);
        for name in &bound_here {
            if ctx.push_comp_bound(name) {
                pushed.push(name);
            }
        }
        for if_expr in &comp.ifs {
            let _ = analyze_expr(if_expr, ctx, source, line_index, diagnostics);
        }
    }
    for body in body_parts {
        let _ = analyze_expr(body, ctx, source, line_index, diagnostics);
    }
    for name in pushed.into_iter().rev() {
        ctx.pop_comp_bound(name);
    }
}

/// Collect every name bound by a comprehension generator's `target`
/// expression — a `Name`, a `Tuple`/`List` unpack, or a `Starred`
/// wrapper. Mirrors `BodyContext::mark_local_target` but writes into a
/// caller-owned `Vec` instead of the local-names set.
fn collect_comp_target_names<'a>(target: &'a Expr, out: &mut Vec<&'a str>) {
    match target {
        Expr::Name(n) => out.push(n.id.as_str()),
        Expr::Tuple(t) => {
            for elt in &t.elts {
                collect_comp_target_names(elt, out);
            }
        }
        Expr::List(l) => {
            for elt in &l.elts {
                collect_comp_target_names(elt, out);
            }
        }
        Expr::Starred(s) => collect_comp_target_names(&s.value, out),
        _ => {}
    }
}

/// Lookup a `name=` kwarg on `call` and require its value be a dict
/// literal. Used by the v1.3 pandas dispatch (`df.rename(columns=...)`).
fn pandas_dict_kwarg<'a>(call: &'a ExprCall, name: &str) -> Option<&'a ruff_python_ast::ExprDict> {
    let kw = call
        .arguments
        .keywords
        .iter()
        .find(|k| k.arg.as_ref().is_some_and(|n| n.id.as_str() == name))?;
    kw.value.as_dict_expr()
}

/// Lookup a `name=` kwarg on `call` and require its value be a list
/// literal. Used by the v1.3 pandas dispatch (`df.drop(columns=[...])`).
fn pandas_list_kwarg<'a>(call: &'a ExprCall, name: &str) -> Option<&'a ruff_python_ast::ExprList> {
    let kw = call
        .arguments
        .keywords
        .iter()
        .find(|k| k.arg.as_ref().is_some_and(|n| n.id.as_str() == name))?;
    kw.value.as_list_expr()
}

/// Lookup a `name=` kwarg on `call` and return its value expression
/// without constraining shape. Used by v1.6 PR-D1 `pivot_table` where
/// each of `index=`/`columns=`/`values=` accepts either a string literal
/// or a list/tuple of string literals.
fn pandas_kwarg_value<'a>(call: &'a ExprCall, name: &str) -> Option<&'a Expr> {
    call.arguments
        .keywords
        .iter()
        .find(|k| k.arg.as_ref().is_some_and(|n| n.id.as_str() == name))
        .map(|k| &k.value)
}

/// v1.5 PR-A2 — resolve a `<X>.createDataFrame(...)` call's schema-source
/// per spec §2.2. Returns `Some(view)` only when one of two gates fires
/// (the call's result is `SparkFrame[X]`); returns `None` when neither
/// gate is present (fall through to Unknown). Receiver shape is NOT a
/// gate — see the "Why not mirror `spark.read.<format>`" paragraph in
/// the spec for why structural matching on `<X>` is unsafe here.
///
/// Gate (a) — `schema=` kwarg resolves to a Spark-tagged frame binding.
/// Gate (b) — first positional arg resolves to a Pandas-tagged frame.
///
/// v1.6 PR-A1 — gate recognition is shared with the dialect-side path
/// (`inherited_dialect` in `driver.rs`) via [`cross_dialect_handoff_gate`].
/// This helper is the view-side consumer: it descends into the matched
/// arg via [`analyze_expr`] to resolve the schema view; the dialect-side
/// consumer short-circuits to `Dialect::Spark` without descent.
fn createdataframe_handoff_view<'a>(
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let matched = match super::driver::cross_dialect_handoff_gate(call, ctx)? {
        super::driver::HandoffGate::SchemaKwarg(e)
        | super::driver::HandoffGate::PandasPositional(e) => e,
    };
    analyze_expr(matched, ctx, source, line_index, diagnostics)
}

pub(super) fn analyze_method_call<'a>(
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> CallOutcome<'a> {
    let Some(attr) = call.func.as_attribute_expr() else {
        return CallOutcome::Unhandled;
    };
    let mut handled = true;
    let view = analyze_method_call_inner(
        call,
        attr,
        ctx,
        source,
        line_index,
        diagnostics,
        &mut handled,
    );
    if handled {
        CallOutcome::Handled(view)
    } else {
        CallOutcome::Unhandled
    }
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
fn analyze_method_call_inner<'a>(
    call: &'a ExprCall,
    attr: &'a ExprAttribute,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
    handled: &mut bool,
) -> Option<SchemaView<'a>> {
    let method = attr.attr.id.as_str();

    // pykrete schema-cast — `<chain>.cast(DataFrame[Schema])` re-anchors a
    // chain whose schema pykrete has lost (after a pivot, an un-modeled
    // op, …) to an explicit `Schema`. Handled before the receiver is
    // resolved on purpose: the receiver is *expected* to be unknown —
    // that's the whole reason the cast is there. The `DataFrame[…]`
    // argument shape is what distinguishes this from `Column.cast("int")`
    // (whose argument is a type string, and whose receiver is a `Column`).
    if method == "cast"
        && let Some(arg) = call.arguments.args.first()
        && let Some(rec) = dataframe::recognize_with_dialect(arg)
        && let DataFrameAnnotation::Typed(name) = rec.kind
    {
        // Analyze the receiver for its own diagnostics; its schema
        // is discarded — the cast overrides whatever it was.
        let _ = analyze_expr(&attr.value, ctx, source, line_index, diagnostics);
        return match ctx.find_schema(name) {
            Some(schema) => Some(SchemaView::Declared(schema)),
            None => {
                let rendered = dataframe::render_annotation(
                    &DataFrameAnnotation::Untyped,
                    rec.dialect,
                    rec.is_deprecated_alias,
                );
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0020",
                    format!(
                        "Unknown schema '{name}' in .cast({rendered}[…]). \
                                 Declare it as a class extending Schema.",
                    ),
                    arg.range(),
                    source,
                    line_index,
                ));
                None
            }
        };
    }
    // Not a `DataFrame[Schema]` argument — ordinary `Column.cast`, or
    // a form pykrete doesn't model. Fall through to the default path.
    // `df.transform(fn)` — Spark's chaining sugar; equivalent to `fn(df)`.
    // The result schema is `fn`'s declared return; the receiver is checked
    // against `fn`'s parameter. Reachable on an unknown receiver too (the
    // function's annotation re-types the chain regardless).
    if method == "transform" {
        return handle_transform(call, attr, ctx, source, line_index, diagnostics);
    }
    // `df.na.fill/drop/replace(...)` — the DataFrameNaFunctions methods.
    // All three reshape rows only, never columns, so the result is the
    // schema of `df` unchanged. Intercepted here, against the `.na`
    // receiver shape, so `df.na.drop("all")` isn't mistaken for
    // `df.drop("col")` (the `"all"` would be a bogus column reference).
    if matches!(method, "fill" | "drop" | "replace")
        && let Some(inner) = attr.value.as_attribute_expr()
        && inner.attr.id.as_str() == "na"
    {
        let recv = analyze_expr(&inner.value, ctx, source, line_index, diagnostics)?;
        check_subset_kwarg(call, &recv, ctx, source, line_index, diagnostics);
        // `na.fill({"col": value, ...})` — same dict-key form as
        // `df.fillna(...)`; check each literal key against the receiver.
        if method == "fill" {
            check_fillna_dict_keys(call, &recv, ctx, source, line_index, diagnostics);
        }
        // `na.fill` / `na.drop` clear nulls from the affected
        // columns; `na.replace` doesn't.
        return Some(if matches!(method, "fill" | "drop") {
            strip_nullability(&recv, ctx.schemas())
        } else {
            recv
        });
    }

    // `spark.sql("SELECT … FROM …")` — infer the result schema from the
    // query's projection columns. A query pykrete can't read cleanly (a
    // `WITH` clause, a `*` wildcard, an unaliased computed column)
    // yields no schema; the user annotates the result in that case.
    //
    // If the query's single FROM table names a registered tempView
    // (`df.createOrReplaceTempView("v")` earlier in the file), the
    // query's column references — projection, WHERE, GROUP BY, ORDER
    // BY, HAVING — are all checked against the view's schema. A
    // `SELECT *` against a known view also returns the view's full
    // schema rather than degrading.
    //
    // `.sql(...)` is a SparkSession method, so the receiver isn't a
    // DataFrame — this is handled before the DataFrame-receiver path.
    if method == "sql" {
        if let Some(lit) = call
            .arguments
            .args
            .first()
            .and_then(|a| a.as_string_literal_expr())
        {
            let query = lit.value.to_str();
            return resolve_spark_sql_call(
                query,
                lit.range(),
                ctx,
                source,
                line_index,
                diagnostics,
            );
        }
        return None;
    }

    // `spark.read.<format>(path)` / `spark.read.format(...).load(...)` /
    // `spark.table(name)` — opaque IO sources. The schema is genuinely
    // runtime data; we return Unknown and rely on `.cast(DataFrame[X])`
    // or a typed variable annotation to re-anchor the chain.
    //
    // Recognized explicitly (rather than left to fall through the
    // DataFrame-receiver path) so the intent is visible in the code and
    // so this site is the natural place to emit a re-anchor hint once
    // we have an informational-severity track.
    // TODO(spark-read-rehint): when an informational/hint diagnostic
    // channel exists, emit a hint here when this call's result isn't
    // re-anchored downstream by `.cast(DataFrame[X])` or a typed
    // variable annotation.
    if is_spark_opaque_source_call(call) {
        return None;
    }

    // v1.5 PR-A2 — `<sess>.createDataFrame(pdf_or_rows, schema=...)` dialect
    // handoff. The result is `SparkFrame[X]` only when one of two
    // schema-sources is present (spec §2.2): a `schema=` kwarg whose value
    // resolves through binding lookup to a Spark-tagged frame, or a
    // positional first arg that resolves to a Pandas-tagged frame. Receiver
    // shape is NOT a gate — structural matching like `is_spark_opaque_source_call`
    // does is unsafe here because PR-A2 returns a TAG (not Unknown), so a
    // `not_spark.createDataFrame(pdf)` with no schema source would be
    // mis-tagged as Spark. See spec §2.2 "Why not mirror `spark.read.<format>`".
    if method == "createDataFrame"
        && let Some(view) = createdataframe_handoff_view(call, ctx, source, line_index, diagnostics)
    {
        return Some(view);
    }

    // `F.broadcast(df)` (or any `<X>.broadcast(df)`) — a join hint that
    // tells Spark to broadcast the dataframe; the schema is the
    // argument's, unchanged. Handled before the receiver-must-be-a-
    // DataFrame guard because `F` isn't a DataFrame; the schema lives
    // on the argument. Used as the start of a chain
    // (`F.broadcast(df).select(col("x"))`) or as a join arg
    // (`df1.join(F.broadcast(df2), "key")`) — both reach this path
    // through `analyze_expr` on the call.
    if method == "broadcast"
        && let Some(arg) = call.arguments.args.first()
    {
        return analyze_expr(arg, ctx, source, line_index, diagnostics);
    }

    // Class-instance receiver: `dal.read(...)` where `dal` is bound as an
    // instance of a known class. Look the method up on the class and do
    // generic substitution. We try this BEFORE the DataFrame-receiver
    // path because the same name can't be both — instance bindings and
    // DataFrame bindings live in disjoint maps.
    //
    // [`lookup_receiver_class`] also walks a self-returning method chain
    // — `dal.with_path("/x").read(SOURCE)` resolves through `with_path`'s
    // `-> "DataAccessLayer"` return so the outer `.read` still sees the
    // class.
    if let Some(class_name) = lookup_receiver_class(&attr.value, ctx) {
        return handle_class_method_call(
            class_name,
            method,
            call,
            ctx,
            source,
            line_index,
            diagnostics,
        );
    }

    let Some(receiver) = analyze_expr(&attr.value, ctx, source, line_index, diagnostics) else {
        // Receiver isn't a frame pykrete can dispatch against — the
        // per-method handlers below can't run, so this call's args
        // haven't been checked through the column-method machinery.
        // Mark unhandled so the caller walks them via `analyze_expr`
        // for embedded `df["col"]` references.
        *handled = false;
        return None;
    };

    // v1.3 — surface the dialect inherited by the receiver chain. For
    // a Name receiver this is the Name's own tag; for a chain receiver
    // (`pdf.merge(other).rename(...)`) this walks down to the deepest
    // Name and reports ITS dialect. Used by the pandas-only method
    // dispatches (`merge`, `assign`, `rename`, `drop(columns=)`) so:
    //  - Spark Name AND Spark chains (`sdf.cache().rename(...)`) skip
    //    the pandas dispatch — preventing silent schema mutation.
    //  - Pandas Name AND pandas chains (`pdf.merge(x).rename(...)`)
    //    correctly dispatch — preserving the refactor pattern of
    //    chaining pandas methods.
    //  - Truly opaque receivers (no Name at the bottom of the chain)
    //    return None and fall through.
    let receiver_dialect = inherited_dialect(&attr.value, ctx);
    let receiver_is_spark_inherited = receiver_dialect == Some(Dialect::Spark);
    let receiver_is_pandas_inherited = receiver_dialect == Some(Dialect::Pandas);

    // `df.createOrReplaceTempView("name")` — register `df`'s schema
    // against `name` in the file's tempView registry, so a later
    // `spark.sql("SELECT … FROM name")` in the same file can resolve
    // its column references. Registration is silent — Spark's view
    // call returns None, and so does pykrete's chain (`None` below);
    // no result schema to track. If the receiver is Unknown (a chain
    // off `spark.read.parquet(...)` that wasn't re-anchored, say),
    // analysis already returned earlier via the `?` above, so we
    // never reach here — Unknown receivers don't register, matching
    // the brief.
    if method == "createOrReplaceTempView"
        && let Some(tv) = ctx.temp_views()
        && let Some(lit) = call
            .arguments
            .args
            .first()
            .and_then(|a| a.as_string_literal_expr())
    {
        tv.register(lit.value.to_str(), receiver.clone());
        return None;
    }

    // `groupBy(keys).count()` is NOT terminal — it returns a DataFrame
    // shaped `{keys..., count: long}`, and a follow-up `.filter(col("count") > 10)`
    // is the routine pattern. Handle it before the general terminal
    // recognizer (which fires for `df.count()` → long, the genuinely
    // terminal case).
    if method == "count"
        && let SchemaView::Grouped {
            keys,
            underlying,
            after_pivot,
        } = &receiver
    {
        // After `.pivot(...)` the output columns are runtime data (a
        // column per distinct pivot value), so the `{keys..., count}`
        // shape no longer applies — bail with Unknown to let the chain
        // die cleanly. Re-anchor with `.cast(DataFrame[X])`.
        if *after_pivot {
            return None;
        }
        return Some(grouped_count_schema(keys, underlying, ctx.schemas()));
    }

    // Terminal recognizers — `count`, `collect`, `show`, …. Spark
    // semantics: each returns something other than a DataFrame (a
    // scalar, a row, a list of rows, None), so the chain dies cleanly
    // here. Recognized centrally (rather than allowed to fall through)
    // both for self-documentation and as the seam for a future
    // "chain-after-terminal" diagnostic.
    // TODO(chain-after-terminal): once an informational/hint channel
    // exists, flag a method call chained after one of these — almost
    // always a bug.
    if is_terminal_method(method, receiver_dialect) {
        return None;
    }
    // v1.5 PR-A3 / v1.6 PR-A2: `head`/`tail`/`first`/`take` on a pandas
    // receiver are NOT terminals — they return a row-sliced DataFrame.
    // The terminal recognizer above already dialect-gates and returns
    // false here; route the chain to the pass-through arm so a follow-up
    // `pdf.head().assign(...)` / `pdf.take([0, 2]).assign(...)` still
    // sees the schema. Spec §2.3 (v1.5 PR-A3) / §3.2 (v1.6 PR-A2 adds
    // `take`). v1.7 PR-A1: the four method names plus `drop` are
    // tracked in `PANDAS_INHERITED_ARMS`; the membership check ties
    // this arm to the shared list (`drop` has its own dispatch arms
    // below, so it stays out of this specific pass-through subset).
    //
    // The `matches!` is the load-bearing dispatch (compile-time exhaustive
    // over the literal subset this arm handles). The trailing
    // `PANDAS_INHERITED_ARMS.contains` is a TRIPWIRE — if a contributor
    // removes one of the four names from the shared list, this arm
    // silently stops firing for that method and the v17_pr_a1 guard test
    // surfaces the drift. Do NOT "simplify" by dropping either side; the
    // pairing is intentional.
    if receiver_is_pandas_inherited
        && matches!(method, "head" | "tail" | "first" | "take")
        && PANDAS_INHERITED_ARMS.contains(&method)
    {
        return Some(receiver);
    }

    // v1.5 PR-A1 — `.toPandas()` dialect handoff. `SparkFrame[X]` →
    // `PandasFrame[X]`: schema preserved, dialect flipped. The dialect
    // flip itself happens in `inherited_dialect` (driver.rs) so chain
    // receivers (`df.toPandas().rename(...)`) and rebind sites
    // (`pdf = df.toPandas()`) both see Pandas downstream; this arm
    // preserves the schema by returning the receiver unchanged.
    // Kwargs (`df.toPandas(arrow=True)`) are ignored — propagation is
    // the same shape. Gated on `receiver_is_spark_inherited`: a
    // pandas-tagged or opaque receiver falls through to the existing
    // `*handled = false; None` path.
    if method == "toPandas" && receiver_is_spark_inherited {
        return Some(receiver);
    }

    // Several methods take a `subset=` of column names — check it
    // uniformly, before the per-method dispatch.
    if matches!(
        method,
        "fillna" | "dropna" | "dropDuplicates" | "drop_duplicates" | "replace"
    ) {
        check_subset_kwarg(call, &receiver, ctx, source, line_index, diagnostics);
    }

    // `fillna({"col": value, ...})` — the first positional arg can be a
    // dict whose keys are column names. Check each literal key against
    // the receiver. Non-dict-literal first args (a scalar, a variable
    // holding a dict) fall through silently — we can only check the
    // syntactically-visible form.
    if method == "fillna" {
        check_fillna_dict_keys(call, &receiver, ctx, source, line_index, diagnostics);
    }

    if method == "agg" {
        return handle_agg(call, &receiver, ctx, source, line_index, diagnostics);
    }
    if method == "selectExpr" {
        // Args are SQL expression strings, not column names. Check the
        // column references *inside* each fragment against the receiver,
        // then model the output schema so the chain keeps its shape.
        for arg in &call.arguments.args {
            if let Some(lit) = arg.as_string_literal_expr() {
                report_sql_column_refs(
                    lit.value.to_str(),
                    lit.range(),
                    &receiver,
                    source,
                    line_index,
                    diagnostics,
                );
            }
        }
        return apply_select_expr(call, &receiver, ctx.schemas());
    }
    if method == "toDF" {
        // `df.toDF("a", "b", …)` renames every column to the given names;
        // `df.toDF()` keeps the receiver's columns. With a splatted
        // `*cols` argument the names aren't statically known — fall back
        // to the receiver so the chain at least stays alive.
        let names: Vec<&'a str> = call
            .arguments
            .args
            .iter()
            .filter_map(|a| a.as_string_literal_expr().map(|s| s.value.to_str()))
            .collect();
        if names.is_empty() {
            return Some(receiver);
        }
        // `toDF` renames positionally — the i-th new name takes the
        // i-th receiver column's type.
        let recv_fields = receiver.typed_fields(ctx.schemas());
        let fields: Vec<DerivedField<'a>> = names
            .into_iter()
            .enumerate()
            .map(|(i, name)| DerivedField {
                name,
                ty: recv_fields.get(i).and_then(|f| f.ty.clone()),
            })
            .collect();
        return Some(SchemaView::Derived(fields));
    }
    // `groupBy(keys).max("col")` / `.min("col")` / `.sum("col")` /
    // `.mean("col")` / `.avg("col")` — Spark's GroupedData aggregate
    // shortcuts, equivalent to `groupBy(keys).agg(F.<method>(col))`.
    // Each string-literal arg is a column name on the underlying schema;
    // dotted paths into nested structs (`"b.c"`) are walked through
    // `resolve_path`, same as `col("b.c")`. The output schema is the
    // grouping keys plus one synthetic field per input string, named
    // following Spark's convention (`sum(col)`, `max(col)`, …); a
    // follow-up `.filter(col("sum(amount)") > 100)` checks against it.
    if matches!(method, "max" | "min" | "sum" | "mean" | "avg")
        && matches!(receiver, SchemaView::Grouped { .. })
    {
        return grouped_aggregate_schema(
            method,
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        );
    }
    if method == "pivot" {
        // `groupBy(keys).pivot("col")` — verify the pivot column exists
        // on the grouped input, then carry the Grouped state forward
        // with `after_pivot: true`. A follow-up `.agg(F.sum("amount"))`
        // still checks `amount` against the pre-pivot underlying
        // schema; the agg's result schema is then Unknown (pivot values
        // are runtime data). The user re-anchors the chain with
        // `.cast(DataFrame[…])` when ready.
        if let SchemaView::Grouped {
            keys, underlying, ..
        } = &receiver
        {
            if let Some(lit) = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
            {
                let name = lit.value.to_str();
                if let FieldPathResult::Missing { field, on } =
                    resolve_path(underlying.as_ref(), name, ctx.schemas())
                {
                    let suggestion = on.as_ref().and_then(|v| suggest_field_name(field, v));
                    let on_phrase = on
                        .as_ref()
                        .map_or_else(|| "the nested struct".to_string(), SchemaView::display_name);
                    let mut message = format!("Column '{field}' does not exist on {on_phrase}.");
                    if let Some(s) = &suggestion {
                        message.push_str(&format!(" Did you mean '{s}'?"));
                    }
                    diagnostics.push(
                        Diagnostic::at_range(
                            Severity::Error,
                            "D0030",
                            message,
                            lit.range(),
                            source,
                            line_index,
                        )
                        .with_suggestion(suggestion),
                    );
                }
            }
            return Some(SchemaView::Grouped {
                keys: keys.clone(),
                underlying: underlying.clone(),
                after_pivot: true,
            });
        }
        return None;
    }
    if method == "withColumns" {
        check_with_columns_enum_sinks(call, &receiver, ctx, source, line_index, diagnostics);
        return Some(apply_with_columns(
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        ));
    }
    if method == "withColumnsRenamed" {
        return Some(apply_with_columns_renamed(
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        ));
    }
    // v1.3 pandas dispatch (spec §5). Internal branching — same
    // analyzer module, no sibling-module split (spec §3 Q9). The
    // pandas-side method names route through the same Spark check-
    // site impls because the col-ref check + result-schema
    // computation are dialect-agnostic; only the surface method
    // name + argument shape differ. Gated on `receiver_is_pandas_
    // inherited` — `inherited_dialect` walks chain receivers down to
    // the deepest Name and uses ITS dialect. So `pdf.merge(x).rename(
    // ...)` correctly dispatches (pdf is pandas-tagged), AND
    // `sdf.cache().rename(...)` correctly skips (sdf is spark-tagged),
    // preventing silent schema mutation on a Spark chain.
    //
    // `df.rename(columns={"old": "new"})` → mirror of
    // `withColumnsRenamed`. The `columns=` kwarg carries the rename
    // dict; legacy positional `mapper=` + `axis=1` forms are not
    // statically inspected (deferred to v1.4).
    if receiver_is_pandas_inherited
        && method == "rename"
        && let Some(dict) = pandas_dict_kwarg(call, "columns")
    {
        return Some(apply_rename_dict(dict, &receiver, ctx));
    }
    // `df.assign(new=expr, …)` → mirror of `withColumns`. Pandas
    // passes the new columns as kwargs (not a dict positional);
    // route them through the shared adder. Enum-sink checks (D0084)
    // fire on string-literal kwarg values written into an enum-typed
    // sink column.
    if receiver_is_pandas_inherited && method == "assign" {
        check_pandas_assign_enum_sinks(call, &receiver, ctx, source, line_index, diagnostics);
        return Some(apply_pandas_assign(
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        ));
    }
    // `df.drop(columns=[...])` — pandas drop's column-only shape.
    // (Spark `.drop(...)` takes positional column names; pandas's
    // unkwarg'd `.drop("x")` is *row-by-label*, not column drop, so
    // we deliberately ONLY recognize the explicit `columns=` form
    // here.) The check reuses Spark's `.drop` logic — col-refs +
    // schema minus the dropped names.
    if receiver_is_pandas_inherited
        && method == "drop"
        && let Some(list) = pandas_list_kwarg(call, "columns")
    {
        return Some(apply_pandas_drop_columns(
            list,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        ));
    }
    // Positional `pdf.drop("x")` on a pandas-named receiver is
    // row-by-label, NOT column drop (v1.3 spec §5). Block the Spark
    // column-method dispatch below so it doesn't fire a wrong D0030
    // AND silently erase the column from the tracked schema.
    // Out-of-scope for v1.3 — quiet ignore, returning the receiver
    // unchanged so chained analysis stays coherent.
    if receiver_is_pandas_inherited
        && method == "drop"
        && pandas_list_kwarg(call, "columns").is_none()
    {
        return Some(receiver);
    }
    // v1.6 PR-D1 — `pdf.pivot_table(index=, columns=, values=, aggfunc=)`
    // literal-form schema check (spec §4). Validates each literal
    // `index`/`columns`/`values` name against the receiver schema
    // (D0030). Non-literal arg shapes (variable, computed) fall through
    // on that arg only; `aggfunc=` is opaque. Spark's `groupBy().pivot()`
    // arm at `:976-1015` is NOT reused — its predicate is
    // `SchemaView::Grouped` (`:984-987`), incompatible with pandas's
    // DataFrame receiver. Output is Unknown (None) for v1.6; users
    // re-anchor with `.cast(DataFrame[NewSchema])` for continued tracking
    // (v1.7 will extend to synthesized output schema).
    if receiver_is_pandas_inherited && method == "pivot_table" {
        let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
        for kw_name in ["index", "columns", "values"] {
            let Some(value) = pandas_kwarg_value(call, kw_name) else {
                continue;
            };
            if let Some(lit) = value.as_string_literal_expr() {
                refs.push((None, lit.value.to_str(), lit.range()));
            } else if let Some(list) = parse_string_list(value) {
                refs.extend(list.into_iter().map(|(n, r)| (None, n, r)));
            }
        }
        report_column_refs(&refs, &receiver, ctx, source, line_index, diagnostics);
        return None;
    }
    // v1.7 PR-D1 — `pdf.melt(id_vars=, value_vars=, var_name=, value_name=)`
    // literal-form schema synthesis (spec §4). Pandas's `melt` uses a
    // different argument shape from Spark's (which routes to the
    // `apply_melt` arm below): kwargs `id_vars=` / `value_vars=` /
    // `var_name=` / `value_name=` instead of positional column lists +
    // `variableColumnName=` / `valueColumnName=`. Both kwargs required
    // as literals for the arm to fire; missing either → fall through.
    // Output schema = `id_vars + [var_name, value_name]`; id_vars retain
    // their declared types from the receiver, `var_name` is string, and
    // `value_name` is the common type of the `value_vars` columns
    // (Unknown if they disagree). Non-literal arg shapes (variable,
    // computed) fall through, no diagnostic.
    if receiver_is_pandas_inherited && method == "melt" {
        let id_vars_expr = pandas_kwarg_value(call, "id_vars");
        let value_vars_expr = pandas_kwarg_value(call, "value_vars");
        let (Some(id_vars_expr), Some(value_vars_expr)) = (id_vars_expr, value_vars_expr) else {
            return None;
        };
        let id_vars: Vec<(&'a str, TextRange)> =
            if let Some(lit) = id_vars_expr.as_string_literal_expr() {
                vec![(lit.value.to_str(), lit.range())]
            } else {
                parse_string_list(id_vars_expr)?
            };
        let value_vars: Vec<(&'a str, TextRange)> =
            if let Some(lit) = value_vars_expr.as_string_literal_expr() {
                vec![(lit.value.to_str(), lit.range())]
            } else {
                parse_string_list(value_vars_expr)?
            };
        let var_name: &'a str = match pandas_kwarg_value(call, "var_name") {
            Some(e) => match e.as_string_literal_expr() {
                Some(s) => s.value.to_str(),
                None => return None,
            },
            None => "variable",
        };
        let value_name: &'a str = match pandas_kwarg_value(call, "value_name") {
            Some(e) => match e.as_string_literal_expr() {
                Some(s) => s.value.to_str(),
                None => return None,
            },
            None => "value",
        };
        let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
        refs.extend(id_vars.iter().map(|&(n, r)| (None, n, r)));
        refs.extend(value_vars.iter().map(|&(n, r)| (None, n, r)));
        report_column_refs(&refs, &receiver, ctx, source, line_index, diagnostics);
        let recv_fields = receiver.typed_fields(ctx.schemas());
        let mut out_fields: Vec<DerivedField<'a>> = Vec::new();
        for (name, _) in &id_vars {
            if let Some(f) = recv_fields.iter().find(|f| f.name == *name) {
                out_fields.push(f.clone());
            }
        }
        let value_field_types: Vec<Option<ColumnType>> = value_vars
            .iter()
            .map(|(n, _)| {
                recv_fields
                    .iter()
                    .find(|f| f.name == *n)
                    .and_then(|f| f.ty.clone())
            })
            .collect();
        let value_ty = melt_value_column_type(&value_field_types);
        out_fields.push(DerivedField {
            name: var_name,
            ty: Some(ColumnType::String),
        });
        out_fields.push(DerivedField {
            name: value_name,
            ty: value_ty,
        });
        return Some(SchemaView::Derived(out_fields));
    }
    if matches!(method, "melt" | "unpivot") {
        return Some(apply_melt(
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        ));
    }
    if let Some(shape) = column_method_shape(method) {
        check_column_method_args(
            call,
            &receiver,
            &shape,
            method,
            ctx,
            source,
            line_index,
            diagnostics,
        );
        // `filter("age > 21")` / `where("city = 'x'")` accept a SQL
        // predicate string. A string-literal arg isn't a column name —
        // it's a SQL fragment whose identifiers are checked here.
        // (`collect_col_refs` skips bare string literals, so without this
        // those references would go unchecked.)
        if matches!(method, "filter" | "where") {
            for arg in &call.arguments.args {
                if let Some(lit) = arg.as_string_literal_expr() {
                    report_sql_column_refs(
                        lit.value.to_str(),
                        lit.range(),
                        &receiver,
                        source,
                        line_index,
                        diagnostics,
                    );
                }
            }
        }
        // `df.withColumn("status", lit("X"))` / `df.withColumn("status",
        // F.coalesce(col("status"), lit("X")))` — when the named sink
        // column is enum-typed on the receiver, check each literal
        // value (peeling branch-form expressions) against the
        // vocabulary. Q1 / Q6 / Q9.
        if method == "withColumn" {
            check_with_column_enum_sink(call, &receiver, ctx, source, line_index, diagnostics);
        }
        return apply_column_method(method, &receiver, call, ctx, ctx.type_ctx());
    }
    if let Some(kind) = two_df_method(method) {
        // `merge` is the pandas spelling of join (spec §5). Skip the
        // Join dispatch when the receiver inherits the Spark dialect —
        // `inherited_dialect` walks chain receivers down to the deepest
        // Name, so `sdf.cache().merge(other, ...)` is gated just like
        // `sdf.merge(other, ...)`. A misspelled `.merge` on a SparkFrame
        // (or a Spark chain) is wrong code, not a join.
        if method == "merge" && receiver_is_spark_inherited {
            return None;
        }
        return handle_two_df_method(
            kind,
            method,
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        );
    }
    // PySpark has a handful of methods that don't alter the receiver's
    // schema — caching hints, partitioning hints, materialization
    // points. Treat them as pass-throughs so a chain like
    // `raw.persist().filter(col("x"))` keeps tracking the schema
    // instead of dying at `.persist()`.
    // `fillna` substitutes a value for nulls — the filled columns are no
    // longer nullable. (`dropna` is a column method, handled there.)
    if method == "fillna" {
        return Some(strip_nullability(&receiver, ctx.schemas()));
    }
    // `sampleBy(col, fractions, seed=None)` — stratified sampling. The
    // first positional arg is a column name (or `col(...)` expression);
    // the schema is unchanged (rows change, columns don't). Sits between
    // pass-through (for the result schema) and column-method (for the
    // first arg).
    if method == "sampleBy" {
        check_sample_by_args(call, &receiver, ctx, source, line_index, diagnostics);
        return Some(receiver);
    }
    // `describe(*cols)` — when called with explicit column names, each
    // string arg is a column ref on the receiver. The output schema is
    // a statistics table whose columns depend on runtime data, so we
    // still return Unknown — but the user's typos in the arg list are
    // caught here.
    if method == "describe" {
        check_describe_args(call, &receiver, ctx, source, line_index, diagnostics);
        return None;
    }
    // `observe(name, *exprs)` — the first positional is a metric name
    // (a string literal, NOT a column ref). Subsequent args are
    // expressions whose embedded column refs need checking. The
    // receiver's schema is returned unchanged.
    if method == "observe" {
        check_observe_args(call, &receiver, ctx, source, line_index, diagnostics);
        return Some(receiver);
    }
    if is_pass_through_method(method) {
        // `df.alias("L")` registers `L` as a SQL-style alias for the
        // receiver's schema. Consulted by the join expression-form
        // resolver so `col("L.region")` resolves through to the
        // underlying schema (the canonical join-disambiguation pattern).
        // The alias name is the first string-literal positional arg;
        // anything else (a non-literal, missing arg) is left to runtime.
        if method == "alias"
            && let Some(lit) = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
        {
            ctx.record_alias(lit.value.to_str(), receiver.clone());
        }
        return Some(receiver);
    }
    if is_opaque_reshape_method(method) {
        return None;
    }
    *handled = false;
    None
}

/// `sampleBy(col, fractions, seed=None)` — check the first positional
/// arg as a column reference on the receiver. The arg may be a bare
/// string literal (`"region"`) or a `col(...)` expression; both forms
/// resolve through the normal column-ref collector. The dict/fractions
/// arg and seed are values, not column refs, and are left alone.
fn check_sample_by_args<'a>(
    call: &'a ExprCall,
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(first) = call.arguments.args.first() else {
        return;
    };
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    if let Some(lit) = first.as_string_literal_expr() {
        refs.push((None, lit.value.to_str(), lit.range()));
    } else {
        collect_col_refs(first, ctx, &mut refs);
    }
    report_column_refs(&refs, receiver, ctx, source, line_index, diagnostics);
}

/// `describe(*cols)` — every positional string-literal arg is a column
/// name on the receiver. Non-string args (a splatted list, a `col(...)`
/// expression) are walked via the normal collector. The result schema
/// is left Unknown by the caller — only the input column refs are
/// validated here.
fn check_describe_args<'a>(
    call: &'a ExprCall,
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    for arg in &call.arguments.args {
        if let Some(lit) = arg.as_string_literal_expr() {
            refs.push((None, lit.value.to_str(), lit.range()));
        } else {
            collect_col_refs(arg, ctx, &mut refs);
        }
    }
    report_column_refs(&refs, receiver, ctx, source, line_index, diagnostics);
}

/// `observe(name, *exprs)` — the first arg is a metric name (skip),
/// remaining args are expressions whose embedded column refs need
/// checking against the receiver's schema. Mirrors the per-arg pattern
/// `handle_agg` uses for its expression args.
fn check_observe_args<'a>(
    call: &'a ExprCall,
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    for arg in call.arguments.args.iter().skip(1) {
        collect_col_refs(arg, ctx, &mut refs);
        report_expr_sql_refs(arg, receiver, source, line_index, diagnostics);
        report_expr_type_errors(
            arg,
            receiver,
            ctx.type_ctx(),
            ctx,
            source,
            line_index,
            diagnostics,
        );
    }
    report_column_refs(&refs, receiver, ctx, source, line_index, diagnostics);
}

/// Methods that return a DataFrame with the same schema as the
/// receiver. Real PySpark code uses these all the time for caching,
/// partitioning, and materialization hints; before this iteration
/// each one quietly broke schema tracking the moment it appeared in a
/// chain.
///
/// `sampleBy` and `observe` are *also* schema-preserving, but they
/// carry column-ref arguments that need checking — they are intercepted
/// earlier in `apply_method` and not listed here.
fn is_pass_through_method(method: &str) -> bool {
    matches!(
        method,
        "persist"
            | "cache"
            | "unpersist"
            | "checkpoint"
            | "localCheckpoint"
            | "coalesce"
            | "repartition"
            | "repartitionByRange"
            | "hint"
            | "sortWithinPartitions"
            | "orderBy"
            | "sort"
            | "limit"
            | "offset"
            | "distinct"
            | "sample"
            | "alias"
            // `replace` swaps values but doesn't specifically clear
            // nulls, so the schema — nullability included — is the
            // receiver's. (`fillna` / `dropna` *do* clear nulls; they
            // are handled explicitly, not as pass-throughs.)
            | "replace"
    )
}

/// Methods that return a DataFrame whose schema is **not** the
/// receiver's — `summary()` produces a statistics table whose columns
/// are `summary` + the numeric subset of the receiver's columns
/// (data-dependent). Pykrete leaves the result Unknown and lets the
/// chain die; the user re-anchors with `.cast(DataFrame[X])` if they
/// want further checking.
///
/// `describe(*cols)` is similar but its positional args ARE column refs
/// that need checking — it's intercepted earlier in `apply_method`.
fn is_opaque_reshape_method(method: &str) -> bool {
    matches!(method, "summary")
}

/// Walk a receiver expression and report the class name its result
/// carries, if any. The base case is a `Name` bound as a class instance
/// in this body. The recursive case is a method call whose method
/// returns the same class — `dal.with_path("/x")` returns
/// `DataAccessLayer`, so the chain stays anchored to the class and the
/// next `.read(SOURCE)` can dispatch through generic inference. The
/// chain breaks at any boundary where the return type isn't the same
/// class as the receiver (a different class, a generic, an Unknown).
fn lookup_receiver_class<'a>(expr: &'a Expr, ctx: &BodyContext<'a>) -> Option<&'a str> {
    if let Some(name) = expr.as_name_expr() {
        return ctx.lookup_instance(name.id.as_str());
    }
    let call = expr.as_call_expr()?;
    let attr = call.func.as_attribute_expr()?;
    let inner_class = lookup_receiver_class(&attr.value, ctx)?;
    let class_info = ctx.registry().find_class(inner_class)?;
    let method = class_info.methods.get(attr.attr.id.as_str())?;
    if method_return_targets_class(method.return_annotation?, inner_class) {
        Some(inner_class)
    } else {
        None
    }
}

/// Does this return annotation name the given class itself? Matches
/// `-> ClassName` (a bare Name), `-> "ClassName"` (the PEP 484 forward
/// reference string), and `-> Self`. Other shapes — a subscript, a
/// different name, an Optional/Union wrapper — return false; pykrete
/// degrades to Unknown rather than guess.
fn method_return_targets_class(ann: &Expr, class_name: &str) -> bool {
    if let Some(name) = ann.as_name_expr() {
        return name.id.as_str() == class_name || name.id.as_str() == "Self";
    }
    if let Some(lit) = ann.as_string_literal_expr() {
        return lit.value.to_str() == class_name;
    }
    false
}

/// Resolve a method call on a class instance — `dal.read(...)`.
///
/// Looks up the method on the receiver's class, and if the method is
/// generic, binds each type variable independently from the matching
/// argument's schema, then substitutes through the return annotation.
///
/// Scope:
/// - Multiple type variables — `def m[A, B](x: G[A], y: G[B]) -> G[A]` —
///   each parameter slot binds its own TypeVar; the return is substituted
///   per-TypeVar.
/// - Nested generic parameter shapes — `List[G[T]]`, `Dict[str, G[T]]`,
///   `Optional[G[T]]`, `List[List[G[T]]]` — the matcher recurses through
///   subscript layers and into the matching argument-literal elements.
/// - Return annotations of shape `G[T]` resolve to `T`'s bound schema; an
///   N-arg subscript like `G[Joined[A, B]]` is treated like `Merge[A, B]`
///   under the substituted names (fields concatenated, first occurrence
///   wins), with the display name carrying the substituted form.
/// - Non-generic methods, or shapes that fail to bind, return `None` —
///   degrade quietly rather than emit a false-positive diagnostic.
fn handle_class_method_call<'a>(
    class_name: &'a str,
    method_name: &str,
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let class_info = ctx.registry().find_class(class_name)?;
    let method = class_info.methods.get(method_name)?;
    let subst = bind_type_vars(
        &method.params,
        method.type_params.as_slice(),
        &call.arguments.args,
        true,
        ctx,
        source,
        line_index,
        diagnostics,
    );
    resolve_return_type(method.return_annotation?, &method.type_params, &subst, ctx)
}

/// Resolve a top-level free-function call — `my_join(orders, refunds)`.
///
/// Same machinery as [`handle_class_method_call`], but with no `self`
/// parameter to skip and a Name-keyed registry lookup. Used to type the
/// result of a generic `def my_join[A, B](left: DataFrame[A], right:
/// DataFrame[B]) -> DataFrame[Joined[A, B]]` so the downstream chain
/// sees `Joined[Orders, Refunds]` columns rather than going Unknown.
fn handle_free_function_call<'a>(
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> CallOutcome<'a> {
    let Some(name_expr) = call.func.as_name_expr() else {
        return CallOutcome::Unhandled;
    };
    // A local binding (an assignment, a walrus) shadows the top-level
    // function; resolving against the registry would then be a false
    // positive.
    if ctx.is_locally_bound(name_expr.id.as_str()) {
        return CallOutcome::Unhandled;
    }
    let Some(sig) = ctx.registry().find_function(name_expr.id.as_str()) else {
        return CallOutcome::Unhandled;
    };
    if sig.type_params.is_empty() {
        return CallOutcome::Unhandled;
    }
    let subst = bind_type_vars(
        &sig.params,
        sig.type_params.as_slice(),
        &call.arguments.args,
        false,
        ctx,
        source,
        line_index,
        diagnostics,
    );
    let view = sig
        .return_annotation
        .and_then(|ann| resolve_return_type(ann, &sig.type_params, &subst, ctx));
    CallOutcome::Handled(view)
}

/// Walk each parameter / argument pair to bind every TypeVar that
/// appears in the parameter annotations. The mapping is independent per
/// TypeVar — `def f[A, B](x: G[A], y: G[B])` collects `A` from `x`'s
/// argument and `B` from `y`'s, each unaffected by the other.
///
/// `skip_self` drops the first parameter (for class-method calls). The
/// per-param walker [`bind_type_vars_from_annotation`] handles the
/// nested-generic / multi-subscript shapes.
#[allow(clippy::too_many_arguments)]
fn bind_type_vars<'a>(
    params: &[MethodParam<'a>],
    type_params: &[&'a str],
    args: &'a [Expr],
    skip_self: bool,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> HashMap<&'a str, &'a Schema<'a>> {
    let mut subst: HashMap<&'a str, &'a Schema<'a>> = HashMap::new();
    // Names of TypeVars that have already seen a conflicting binding —
    // poisoning is sticky. A later element/slot that would otherwise
    // re-bind the same name must instead leave it unbound, so the
    // return type degrades to Unknown rather than silently reviving an
    // earlier (now-rejected) schema. See `bind_type_vars_from_annotation`.
    let mut poisoned: HashSet<&'a str> = HashSet::new();
    if type_params.is_empty() {
        return subst;
    }
    let skip = usize::from(skip_self);
    // TODO(d0051-class-method-vararg): positional pairing here walks
    // every parameter in declaration order, so a `*args` / kw-only /
    // `**kwargs` slot interleaved before a generic-bearing param will
    // mis-align the arg-to-param pairing. No test exercises this yet;
    // revisit when a real generic shape needs the segment-aware matcher.
    for (i, mp) in params.iter().skip(skip).enumerate() {
        let Some(arg) = args.get(i) else { continue };
        let Some(pann) = mp.annotation else { continue };
        bind_type_vars_from_annotation(
            pann,
            arg,
            type_params,
            &mut subst,
            &mut poisoned,
            ctx,
            source,
            line_index,
            diagnostics,
        );
    }
    subst
}

/// Match one parameter annotation against one argument expression, and
/// record any TypeVar bindings the pairing implies.
///
/// Base case: the annotation is `G[T]` for some declared TypeVar `T` —
/// the argument's schema is bound to `T`. Recursive cases:
///
/// - `List[X]` / `Tuple[X, …]` / `Sequence[X]` / `Set[X]` — recurse `X`
///   against each list/tuple/set literal element.
/// - `Dict[K, V]` / `Mapping[K, V]` — recurse `V` against each dict
///   literal value (the key slot is not modeled).
/// - `Optional[X]` — recurse `X` against the same argument (the `None`
///   case is degenerate and binds nothing).
///
/// An element whose schema doesn't agree with the existing binding for
/// the same TypeVar poisons the binding — degrade to Unknown rather
/// than fabricate a join. Poisoning is sticky: once a TypeVar conflicts
/// at any element/slot, later elements/slots cannot revive it (their
/// would-be binding is ignored). Without stickiness, three elements
/// `[Orders, Refunds, Orders]` would leave T = Orders after the third
/// element silently re-binds the conflict-cleared key.
#[allow(clippy::too_many_arguments)]
fn bind_type_vars_from_annotation<'a>(
    ann: &'a Expr,
    arg: &'a Expr,
    type_params: &[&'a str],
    subst: &mut HashMap<&'a str, &'a Schema<'a>>,
    poisoned: &mut HashSet<&'a str>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // `type[T]` / `Type[T]` — the arg is a class object (its static type
    // is `type[Schema]`), not a value of the schema's type. Match the
    // shape before the generic base case so the schema is read from the
    // arg's identifier (`OrdersSchema`) rather than from its runtime
    // value (which would be Unknown — a class isn't a DataFrame).
    if let Some(tv) = extract_type_var_from_type_subscript(ann, type_params)
        && let Some(schema) = type_arg_schema(arg, ctx)
    {
        record_binding(tv, schema, subst, poisoned);
        return;
    }

    // Base case — `G[T]`. The arg's schema is T's binding (if compatible
    // with any earlier binding of the same T).
    if let Some(tv) = extract_type_var_from_subscript(ann, type_params)
        && let Some(schema) = arg_schema(arg, ctx, source, line_index, diagnostics)
    {
        record_binding(tv, schema, subst, poisoned);
        return;
    }

    // Recursive case — strip a collection wrapper and recurse into the
    // matching argument literal.
    let Some(sub) = ann.as_subscript_expr() else {
        return;
    };
    let Some(base) = sub.value.as_name_expr() else {
        return;
    };
    match base.id.as_str() {
        // Single-element collection — `List[X]`, `Sequence[X]`, `Set[X]`.
        // The slice is the element annotation; the arg is a list/tuple
        // literal whose elements we recurse against.
        "List" | "list" | "Sequence" | "Iterable" | "Set" | "set" | "FrozenSet" | "frozenset" => {
            let inner = sub.slice.as_ref();
            for elem in arg_iter_elements(arg) {
                bind_type_vars_from_annotation(
                    inner,
                    elem,
                    type_params,
                    subst,
                    poisoned,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                );
            }
        }
        // Tuple — `Tuple[X, ...]` (variadic) or `Tuple[X, Y, Z]` (fixed).
        // The variadic form treats every element as `X`; the fixed form
        // pairs each annotation slot with the corresponding element.
        "Tuple" | "tuple" => {
            let slots: Vec<&Expr> = match sub.slice.as_ref() {
                Expr::Tuple(t) => t.elts.iter().collect(),
                single => vec![single],
            };
            if let [inner, Expr::EllipsisLiteral(_)] = slots.as_slice() {
                for elem in arg_iter_elements(arg) {
                    bind_type_vars_from_annotation(
                        inner,
                        elem,
                        type_params,
                        subst,
                        poisoned,
                        ctx,
                        source,
                        line_index,
                        diagnostics,
                    );
                }
            } else {
                for (slot, elem) in slots.iter().zip(arg_iter_elements(arg).iter()) {
                    bind_type_vars_from_annotation(
                        slot,
                        elem,
                        type_params,
                        subst,
                        poisoned,
                        ctx,
                        source,
                        line_index,
                        diagnostics,
                    );
                }
            }
        }
        // Mapping — `Dict[K, V]` / `Mapping[K, V]`. Key types aren't
        // carriers of schemas in this PR — recurse into value only. A
        // TypeVar that only appears as a Dict key annotation stays
        // unbound (silent degrade to Unknown — never a false positive).
        "Dict" | "dict" | "Mapping" => {
            let value_ann = match sub.slice.as_ref() {
                Expr::Tuple(t) if t.elts.len() == 2 => &t.elts[1],
                _ => return,
            };
            for value_expr in arg_dict_values(arg) {
                bind_type_vars_from_annotation(
                    value_ann,
                    value_expr,
                    type_params,
                    subst,
                    poisoned,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                );
            }
        }
        // Optional[X] — recurse `X` against the same arg. `None` is
        // accepted at runtime but binds nothing.
        "Optional" => {
            if arg.is_none_literal_expr() {
                return;
            }
            bind_type_vars_from_annotation(
                sub.slice.as_ref(),
                arg,
                type_params,
                subst,
                poisoned,
                ctx,
                source,
                line_index,
                diagnostics,
            );
        }
        _ => {
            // Unknown wrapper — don't recurse blindly. The base case
            // already covered the `G[T]` direct shape above.
        }
    }
}

/// Substitute the bound TypeVars through the return annotation and
/// resolve it to a [`SchemaView`].
///
/// Shapes handled:
/// - `G[T]` where `T` was bound — returns `T`'s schema as
///   `SchemaView::Declared`.
/// - `G[Merge[A, B, …]]` where each inner slot is a TypeVar (any subset
///   of which may be bound) — returns the concatenated columns of every
///   bound schema as `SchemaView::Derived` (first occurrence of each
///   column name wins, preserving listed order). Unbound slots are
///   silently dropped; the view carries no display name (Derived has no
///   name field).
fn resolve_return_type<'a>(
    return_ann: &'a Expr,
    type_params: &[&'a str],
    subst: &HashMap<&'a str, &'a Schema<'a>>,
    ctx: &BodyContext<'a>,
) -> Option<SchemaView<'a>> {
    // `G[T]` — direct TypeVar in the outer subscript.
    if let Some(tv) = extract_type_var_from_subscript(return_ann, type_params) {
        return subst.get(tv).copied().map(SchemaView::Declared);
    }
    // `G[Joined[A, B, …]]` — a parameterized inner shape. Outer subscript's
    // slice is itself a subscript whose own slice lists TypeVars.
    let outer = return_ann.as_subscript_expr()?;
    let inner = outer.slice.as_subscript_expr()?;
    let inner_args: Vec<&Expr> = match inner.slice.as_ref() {
        Expr::Tuple(t) => t.elts.iter().collect(),
        single => vec![single],
    };
    // Pull the schema bound to each inner TypeVar slot. An unbound slot
    // (its arg-site value was Unknown) is silently dropped — the
    // remaining bound schemas still contribute their fields, so a
    // call where only one of two arguments resolved still produces a
    // usable downstream view.
    let mut bound_schemas: Vec<&'a Schema<'a>> = Vec::new();
    let mut saw_typevar = false;
    for arg in &inner_args {
        let Some(name) = arg.as_name_expr() else {
            // A non-TypeVar slot in the inner subscript (e.g. a bare
            // schema name) isn't part of the substitution model
            // pykrete handles; degrade.
            return None;
        };
        if !type_params.contains(&name.id.as_str()) {
            return None;
        }
        saw_typevar = true;
        if let Some(s) = subst.get(name.id.as_str()) {
            bound_schemas.push(*s);
        }
    }
    if !saw_typevar || bound_schemas.is_empty() {
        return None;
    }
    // Concatenate fields Merge-style — first occurrence of each column
    // name wins, preserving the order the schemas were listed.
    let mut fields: Vec<DerivedField<'a>> = Vec::new();
    for s in &bound_schemas {
        for field in SchemaView::Declared(s).typed_fields(ctx.schemas()) {
            if !fields.iter().any(|f| f.name == field.name) {
                fields.push(field);
            }
        }
    }
    Some(SchemaView::Derived(fields))
}

/// Iterate over the elements of a list/tuple-literal argument. Returns
/// an empty slice for any other shape — `arg_schema` will silently fail
/// downstream and the binding will degrade.
fn arg_iter_elements(arg: &Expr) -> &[Expr] {
    match arg {
        Expr::List(l) => &l.elts,
        Expr::Tuple(t) => &t.elts,
        Expr::Set(s) => &s.elts,
        _ => &[],
    }
}

/// Iterate over the value expressions of a dict-literal argument.
fn arg_dict_values(arg: &Expr) -> Vec<&Expr> {
    match arg {
        Expr::Dict(d) => d.items.iter().map(|item| &item.value).collect(),
        _ => Vec::new(),
    }
}

/// Match `GenericClass[T]` where `T` is one of the supplied type variable
/// names. Returns `T` if matched.
fn extract_type_var_from_subscript<'a>(expr: &'a Expr, type_params: &[&'a str]) -> Option<&'a str> {
    let sub = expr.as_subscript_expr()?;
    let inner = sub.slice.as_name_expr()?.id.as_str();
    if type_params.contains(&inner) {
        Some(inner)
    } else {
        None
    }
}

/// Match `type[T]` / `Type[T]` where `T` is one of the supplied type
/// variable names. Returns `T` if matched. Used for `def m[T](_:
/// type[T]) -> DataFrame[T]` — the arg is a class object whose static
/// type is `type[T]`, so T binds from the arg's identifier name.
fn extract_type_var_from_type_subscript<'a>(
    expr: &'a Expr,
    type_params: &[&'a str],
) -> Option<&'a str> {
    let sub = expr.as_subscript_expr()?;
    let base = sub.value.as_name_expr()?.id.as_str();
    if base != "type" && base != "Type" {
        return None;
    }
    let inner = sub.slice.as_name_expr()?.id.as_str();
    if type_params.contains(&inner) {
        Some(inner)
    } else {
        None
    }
}

/// Resolve a `type[T]`-positioned argument to the schema it names.
///
/// The arg must be a bare `Name` whose identifier matches a known
/// schema. Anything else — a string literal, a subscript, an
/// arbitrary expression — degrades to `None` (the binding is skipped
/// and T silently goes Unknown, never a false positive).
fn type_arg_schema<'a>(arg: &'a Expr, ctx: &BodyContext<'a>) -> Option<&'a Schema<'a>> {
    let name = arg.as_name_expr()?;
    ctx.find_schema(name.id.as_str())
}

/// Apply a `T → schema` binding with the project's sticky-poisoning
/// semantics. A conflict between the new schema and an existing
/// binding of the same T poisons T: the prior binding is dropped and
/// no future call to this helper can revive T from a later arg/slot,
/// so the return type degrades to Unknown rather than silently
/// fabricate a join.
fn record_binding<'a>(
    tv: &'a str,
    schema: &'a Schema<'a>,
    subst: &mut HashMap<&'a str, &'a Schema<'a>>,
    poisoned: &mut HashSet<&'a str>,
) {
    if poisoned.contains(tv) {
        return;
    }
    match subst.get(tv) {
        None => {
            subst.insert(tv, schema);
        }
        Some(existing) if (*existing).name() == schema.name() => {
            // Consistent re-binding — keep it.
        }
        Some(_) => {
            subst.remove(tv);
            poisoned.insert(tv);
        }
    }
}

/// What schema does a given call-argument carry?
///
/// For Name expressions, consult the body context (which already falls back
/// through top-level constants). For more complex expressions, recursively
/// analyze them as expressions and pull the schema out of a Declared view.
fn arg_schema<'a>(
    arg: &'a Expr,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<&'a Schema<'a>> {
    match analyze_expr(arg, ctx, source, line_index, diagnostics) {
        Some(SchemaView::Declared(s)) => Some(s),
        _ => None,
    }
}

/// Model `df.transform(fn)` — Spark's chaining sugar, equivalent to
/// `fn(df)`. `fn` is an ordinary `DataFrame -> DataFrame` function (not a
/// UDF). The result schema is `fn`'s declared `-> DataFrame[Schema]`
/// return; inferring it from `fn`'s body when the return is undeclared
/// is a follow-up.
///
/// As a bonus, the receiver's schema is checked against `fn`'s first
/// parameter — feeding the wrong DataFrame into a named pipeline step is
/// exactly the mistake `transform`-as-a-named-step is meant to catch.
fn handle_transform<'a>(
    call: &'a ExprCall,
    attr: &'a ExprAttribute,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    // Analyze the receiver for its own diagnostics and the input check.
    // It may be unknown (`None`) — transform re-types the chain anyway.
    let receiver = analyze_expr(&attr.value, ctx, source, line_index, diagnostics);

    // The transform function — `df.transform(add_features)`. Only a bare
    // name referring to a top-level `def` is resolved; lambdas and
    // imported callables are left unmodeled.
    let func_name = call.arguments.args.first()?.as_name_expr()?.id.as_str();
    let sig = ctx.registry().find_function(func_name)?;

    // Input-compatibility check: receiver schema vs. fn's first parameter.
    if let (Some(recv), Some(first_param)) = (&receiver, sig.params.first())
        && let Some(DataFrameAnnotation::Typed(pname)) =
            first_param.annotation.and_then(dataframe::recognize)
        && let Some(param_schema) = ctx.find_schema(pname)
    {
        check_transform_input(
            recv,
            param_schema,
            func_name,
            attr.value.range(),
            source,
            line_index,
            diagnostics,
        );
    }

    // Result schema — fn's declared `-> DataFrame[Schema]` if it has one…
    if let Some(DataFrameAnnotation::Typed(rname)) =
        sig.return_annotation.and_then(dataframe::recognize)
        && let Some(schema) = ctx.find_schema(rname)
    {
        return Some(SchemaView::Declared(schema));
    }
    // …otherwise infer it by analyzing fn's body with the receiver bound
    // to fn's parameter. Needs a known receiver to feed in.
    //
    // **v1.4 PR-B Bug 3**: thread the receiver's dialect (pandas /
    // Spark) into the bind so pandas-only dispatch (`.assign`,
    // `.rename(columns={...})`, etc.) inside the helper body fires
    // under the correct dialect. Without this, `pdf.transform(helper)`
    // bound `helper`'s param with `None` → pandas dispatch silently
    // skipped → inferred return schema came back `None` and downstream
    // col-refs went silent.
    let receiver_dialect = crate::operations::driver::inherited_dialect(&attr.value, ctx);
    infer_transform_output(
        sig.def,
        receiver?,
        receiver_dialect,
        ctx,
        source,
        line_index,
    )
}

/// Infer the output schema of a `transform` function whose return type
/// isn't declared, by walking its body with the parameter bound to the
/// actual input schema — the TypeScript-style "infer the return" path.
///
/// Recursion through nested `transform` calls is bounded by
/// [`MAX_INFER_DEPTH`]. Diagnostics raised while walking the body are
/// discarded: an annotated function gets its real diagnostics from the
/// standalone analysis pass; a fully un-annotated one reached only here
/// goes unchecked, an accepted v1 gap.
pub(super) fn infer_transform_output<'a>(
    func_def: &'a StmtFunctionDef,
    input: SchemaView<'a>,
    input_dialect: Option<crate::dataframe::Dialect>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
) -> Option<SchemaView<'a>> {
    if ctx.infer_depth >= MAX_INFER_DEPTH {
        return None;
    }
    // fn's first positional parameter — the DataFrame it transforms.
    let params = &func_def.parameters;
    let first = params.posonlyargs.iter().chain(&params.args).next()?;
    let param_name = first.parameter.name.id.as_str();

    let mut child = BodyContext::new(ctx.schemas(), ctx.registry());
    if let Some(tv) = ctx.temp_views() {
        child = child.with_temp_views(tv);
    }
    child.infer_depth = ctx.infer_depth + 1;
    child.bind_df(param_name, input, input_dialect);

    let discovered = DiscoveredFunction { def: func_def };
    let mut sink: Vec<Diagnostic> = Vec::new();
    check_function_body(&discovered, None, &mut child, source, line_index, &mut sink)
}

/// Check that the DataFrame fed into `df.transform(fn)` matches `fn`'s
/// declared parameter schema. Compares column-name sets; a mismatch
/// emits `D0073`.
fn check_transform_input(
    receiver: &SchemaView<'_>,
    param: &Schema<'_>,
    func_name: &str,
    range: TextRange,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let recv_names: HashSet<&str> = receiver.field_names().into_iter().collect();
    let param_names: HashSet<&str> = param.fields().iter().map(|f| f.name).collect();
    if recv_names == param_names {
        return;
    }
    let mut missing: Vec<&str> = param_names.difference(&recv_names).copied().collect();
    let mut extra: Vec<&str> = recv_names.difference(&param_names).copied().collect();
    missing.sort();
    extra.sort();
    let message = format!(
        "transform('{func_name}') expects a DataFrame matching schema '{}', \
         but the receiver ({}) does not. Missing: [{}]; extra: [{}].",
        param.name(),
        receiver.display_name(),
        missing.join(", "),
        extra.join(", "),
    );
    diagnostics.push(Diagnostic::at_range(
        Severity::Error,
        "D0073",
        message,
        range,
        source,
        line_index,
    ));
}

/// Check the arguments of a free-function call against the callee's
/// declared `DataFrame[Schema]` parameters. The mirror of return-type
/// checking (`D0050`), one frame earlier: it catches `f(refunds)` when
/// `f` was declared `def f(sales: DataFrame[Sale])` and `refunds`
/// resolves to a different schema. Emits `D0051`
/// (`argumentColumnsMismatch`).
///
/// Method calls (`df.method(...)`) go through `analyze_method_call`;
/// `df.transform(fn)` has its own input check in `handle_transform`.
/// This function only fires when `call.func` is a bare `Name` and that
/// name resolves to a user-defined top-level function in the registry.
///
/// An argument whose schema can't be inferred (untyped local, an
/// opaque `spark.read.json(...)` chain) is silently skipped — the same
/// degrade-rather-than-false-flag stance the rest of the checker takes.
///
/// **v1.4 PR-B Bug 1**: every positional arg and kwarg value is
/// `analyze_expr`-walked unconditionally, regardless of whether the
/// matching param is `DataFrame[X]`-typed. The walk surfaces embedded
/// `df["typo"]` D0030s on registry calls like `util(df["typo"])` where
/// `util(x: int)` has no DataFrame-typed slot — without this, the §10
/// widening at the Expr::Call site is gated off
/// (`is_registry_function_call` short-circuit) and the typo went silent.
/// The captured schema is re-used by `check_one_call_arg` for the D0051
/// mismatch check, so no arg is `analyze_expr`-walked twice
/// (idempotency requirement).
fn check_call_argument_schemas<'a>(
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(name_expr) = call.func.as_name_expr() else {
        return;
    };
    // A local binding with the same name shadows the top-level function:
    // the call resolves to the local at runtime, so checking the top-level
    // signature would be a false positive. We check the broader
    // `is_locally_bound` set rather than `lookup` because the shadowing
    // assignment's RHS may have an un-inferred schema, yet still binds
    // the name.
    if ctx.is_locally_bound(name_expr.id.as_str()) {
        return;
    }
    let Some(sig) = ctx.registry().find_function(name_expr.id.as_str()) else {
        return;
    };

    // Walk every positional arg and kwarg value once; record the
    // resulting schema by arg index so `check_one_call_arg` doesn't
    // re-walk. See the §10-widening / Bug-1 note above.
    let pos_schemas: Vec<Option<SchemaView<'a>>> = call
        .arguments
        .args
        .iter()
        .map(|arg| analyze_expr(arg, ctx, source, line_index, diagnostics))
        .collect();
    let kw_schemas: Vec<Option<SchemaView<'a>>> = call
        .arguments
        .keywords
        .iter()
        .map(|kw| analyze_expr(&kw.value, ctx, source, line_index, diagnostics))
        .collect();

    // Walk positional args in lockstep with positional-or-regular params;
    // overflow goes to `*args` if the function declared one. A positional
    // arg that lands past `*` with no vararg slot is a Python TypeError —
    // skip it rather than emit a misleading schema diagnostic.
    //
    // `consumed_positional` records which slots a positional arg filled
    // (by parameter name). The kwarg loop consults it to skip any kwarg
    // whose name targets an already-filled slot: that's Python's
    // `TypeError: got multiple values for argument`, and firing D0051 a
    // second time on the same slot is double-diagnosis, not double bugs.
    let mut pos_idx = 0;
    let mut consumed_positional: HashSet<&str> = HashSet::new();
    for (arg, arg_schema) in call.arguments.args.iter().zip(pos_schemas.iter()) {
        let matched = match_positional(&sig.params, &mut pos_idx);
        if let Some(param) = matched {
            // VarPositional is sticky — every remaining positional arg
            // lands in it. Don't record it as "consumed" for the kwarg
            // dedupe, since `*args` and `**kwargs` are independent slots.
            if !matches!(param.kind, ParamKind::VarPositional) {
                consumed_positional.insert(param.name);
            }
            check_one_call_arg(
                arg,
                arg_schema.as_ref(),
                param,
                ctx,
                source,
                line_index,
                diagnostics,
            );
        }
    }

    // Keyword arguments — match by name against regular / kw-only params;
    // unrecognized names fall through to `**kwargs` if present. A keyword
    // arg whose name targets a positional-only param is a Python TypeError —
    // skip it. Likewise, skip any kwarg whose name targets a slot already
    // filled positionally (Python rejects this as a TypeError, and we'd
    // otherwise re-diagnose the same parameter).
    for (kw, arg_schema) in call.arguments.keywords.iter().zip(kw_schemas.iter()) {
        let Some(name) = kw.arg.as_ref().map(|n| n.id.as_str()) else {
            continue;
        };
        if consumed_positional.contains(name) {
            continue;
        }
        let named = sig.params.iter().find(|p| {
            p.name == name && matches!(p.kind, ParamKind::Regular | ParamKind::KeywordOnly)
        });
        let param = named.or_else(|| sig.params.iter().find(|p| p.kind == ParamKind::VarKeyword));
        if let Some(param) = param {
            check_one_call_arg(
                &kw.value,
                arg_schema.as_ref(),
                param,
                ctx,
                source,
                line_index,
                diagnostics,
            );
        }
    }
}

/// Pick the parameter slot a positional argument should bind to,
/// advancing `cursor` past consumed positional-or-regular slots. A
/// `*args` slot is sticky — every remaining positional arg lands in it.
/// Returns `None` when the call has overflowed past `*` into kw-only
/// territory: Python itself would TypeError, so a schema diagnostic
/// would be the wrong cause to blame.
fn match_positional<'a, 'p>(
    params: &'p [MethodParam<'a>],
    cursor: &mut usize,
) -> Option<&'p MethodParam<'a>> {
    let p = params.get(*cursor)?;
    match p.kind {
        ParamKind::PositionalOnly | ParamKind::Regular => {
            *cursor += 1;
            Some(p)
        }
        ParamKind::VarPositional => Some(p),
        ParamKind::KeywordOnly | ParamKind::VarKeyword => {
            *cursor = params.len();
            None
        }
    }
}

fn check_one_call_arg<'a>(
    arg: &'a Expr,
    arg_schema: Option<&SchemaView<'a>>,
    param: &MethodParam<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Only checks `<Frame>[Schema]` parameters — other annotation
    // shapes (an unannotated param, a non-frame type, bare `<Frame>`
    // without a schema, a derived expression like `<Frame>[Pick[...]]`)
    // fall through. Recognize with dialect so the D0051 message echoes
    // the actual frame surface the parameter declared.
    let Some(rec) = param.annotation.and_then(dataframe::recognize_with_dialect) else {
        return;
    };
    let DataFrameAnnotation::Typed(pname) = rec.kind else {
        return;
    };
    let Some(param_schema) = ctx.find_schema(pname) else {
        return;
    };

    // The arg's schema was already resolved by
    // `check_call_argument_schemas` (one walk per arg per call, by index).
    // If the walk produced None (untyped local, opaque chain) the D0051
    // mismatch can't be checked — silently skip in keeping with the
    // rest of the checker's degrade-rather-than-false-flag stance.
    let Some(arg_schema) = arg_schema else {
        return;
    };

    let arg_names: HashSet<&str> = arg_schema.field_names().into_iter().collect();
    let param_names: HashSet<&str> = param_schema.fields().iter().map(|f| f.name).collect();
    if arg_names == param_names {
        return;
    }
    let mut missing: Vec<&str> = param_names.difference(&arg_names).copied().collect();
    let mut extra: Vec<&str> = arg_names.difference(&param_names).copied().collect();
    missing.sort();
    extra.sort();
    // v1.3 PR-E2: render the parameter's actual frame surface
    // (`SparkFrame[X]` / `PandasFrame[X]` / `DataFrame[X]` alias)
    // through `dataframe::render_annotation`, not a hardcoded
    // "DataFrame[X]".
    let expected = dataframe::render_annotation(&rec.kind, rec.dialect, rec.is_deprecated_alias);
    let message = format!(
        "Argument schema mismatch for parameter '{}': expected {}, \
         got {}. Missing: [{}]; extra: [{}].",
        param.name,
        expected,
        arg_schema.display_name(),
        missing.join(", "),
        extra.join(", "),
    );
    diagnostics.push(Diagnostic::at_range(
        Severity::Error,
        "D0051",
        message,
        arg.range(),
        source,
        line_index,
    ));
}

/// Handle `.agg(...)` on either a `Grouped` or a regular DataFrame receiver.
///
/// Each argument expression's column references (both `col("x")` and the
/// string-arg form `F.sum("x")` for known aggregate function names) are
/// checked against the underlying schema. The output schema is the group
/// keys (if any) plus each argument's output name (from `.alias("name")`
/// or a bare column ref).
fn handle_agg<'a>(
    call: &'a ExprCall,
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let (keys, underlying, after_pivot): (Vec<&'a str>, SchemaView<'a>, bool) = match receiver {
        SchemaView::Grouped {
            keys,
            underlying,
            after_pivot,
        } => (keys.clone(), (**underlying).clone(), *after_pivot),
        other => (Vec::new(), other.clone(), false),
    };

    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    // Group keys keep their type from the underlying schema; aggregate
    // outputs are typed via `select_arg_type` (a plain column ref is
    // typed, an aggregate function result is unknown for now).
    let mut fields: Vec<DerivedField<'a>> = keys
        .iter()
        .map(|&name| DerivedField {
            name,
            ty: underlying.field_type(name, ctx.schemas()),
        })
        .collect();
    for arg in &call.arguments.args {
        collect_col_refs(arg, ctx, &mut refs);
        report_expr_sql_refs(arg, &underlying, source, line_index, diagnostics);
        report_expr_type_errors(
            arg,
            &underlying,
            ctx.type_ctx(),
            ctx,
            source,
            line_index,
            diagnostics,
        );
        if let Some(pair) = posexplode_fields(arg, &underlying, ctx.type_ctx(), ctx) {
            for f in pair {
                if !fields.iter().any(|existing| existing.name == f.name) {
                    fields.push(f);
                }
            }
            continue;
        }
        if let Some(pair) = explode_map_aliased_fields(arg, &underlying, ctx.type_ctx(), ctx) {
            for f in pair {
                if !fields.iter().any(|existing| existing.name == f.name) {
                    fields.push(f);
                }
            }
            continue;
        }
        if let Some(name) = select_output_name(arg, Some(ctx))
            && !fields.iter().any(|f| f.name == name)
        {
            fields.push(DerivedField {
                name,
                ty: select_arg_type(arg, &underlying, ctx.type_ctx(), ctx),
            });
        }
    }
    report_column_refs(&refs, &underlying, ctx, source, line_index, diagnostics);
    if after_pivot {
        // `groupBy(...).pivot(...).agg(...)` — the agg input was checked
        // against the pre-pivot underlying schema (above), but the OUTPUT
        // columns depend on the runtime pivot values pykrete can't see.
        // Bail with Unknown so the chain dies cleanly here; the user
        // re-anchors with `.cast(DataFrame[X])` for further checking.
        return None;
    }
    Some(SchemaView::Derived(fields))
}

/// Result schema of `groupBy(keys).count()` — the grouping keys followed
/// by a `count: long` column. Spark always names this column `count`
/// regardless of how many keys there are.
fn grouped_count_schema<'a>(
    keys: &[&'a str],
    underlying: &SchemaView<'a>,
    schemas: &'a [Schema<'a>],
) -> SchemaView<'a> {
    let mut fields: Vec<DerivedField<'a>> = keys
        .iter()
        .map(|&name| DerivedField {
            name,
            ty: underlying.field_type(name, schemas),
        })
        .collect();
    // If a key was already named `count` we'd double-emit; Spark's
    // behavior is that the synthetic shadows the original, so drop the
    // earlier field before appending the new one.
    fields.retain(|f| f.name != "count");
    fields.push(DerivedField {
        name: "count",
        ty: Some(ColumnType::Long),
    });
    SchemaView::Derived(fields)
}

/// Result schema of `groupBy(keys).sum("c1", "c2") / .max(...) / .min(...) /
/// .mean(...) / .avg(...)`. Each string-literal argument is verified to
/// exist on the underlying schema (D0030 fires otherwise, matching the
/// pre-v0.1.26 behavior), then a synthetic column named after Spark's
/// convention — `sum(c1)`, `max(c1)`, … — is added with a type derived
/// from the input column:
///
/// - `sum`: int → long, long → long, double → double; non-numeric → None.
/// - `mean`/`avg`: any numeric → double; otherwise None.
/// - `max`/`min`: same type as input.
fn grouped_aggregate_schema<'a>(
    method: &str,
    call: &'a ExprCall,
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let SchemaView::Grouped {
        keys,
        underlying,
        after_pivot,
    } = receiver
    else {
        return Some(receiver.clone());
    };
    let underlying = underlying.as_ref();
    let mut fields: Vec<DerivedField<'a>> = keys
        .iter()
        .map(|&name| DerivedField {
            name,
            ty: underlying.field_type(name, ctx.schemas()),
        })
        .collect();
    for arg in &call.arguments.args {
        let Some(lit) = arg.as_string_literal_expr() else {
            continue;
        };
        let name = lit.value.to_str();
        match resolve_path(underlying, name, ctx.schemas()) {
            FieldPathResult::Missing { field, on } => {
                let suggestion = on.as_ref().and_then(|v| suggest_field_name(field, v));
                let on_phrase = on
                    .as_ref()
                    .map_or_else(|| "the nested struct".to_string(), SchemaView::display_name);
                let mut message = format!("Column '{field}' does not exist on {on_phrase}.");
                if let Some(s) = &suggestion {
                    message.push_str(&format!(" Did you mean '{s}'?"));
                }
                diagnostics.push(
                    Diagnostic::at_range(
                        Severity::Error,
                        "D0030",
                        message,
                        lit.range(),
                        source,
                        line_index,
                    )
                    .with_suggestion(suggestion),
                );
            }
            FieldPathResult::Resolved => {
                let synthetic = ctx.intern_synthetic(format!("{method}({name})"));
                let input_ty = underlying.field_type(name, ctx.schemas());
                let ty = aggregate_output_type(method, input_ty.as_ref());
                if !fields.iter().any(|f| f.name == synthetic) {
                    fields.push(DerivedField {
                        name: synthetic,
                        ty,
                    });
                }
            }
        }
    }
    // `groupBy(...).pivot(...).max("amount")` and the other shorthand
    // aggregates: column args were checked against the pre-pivot
    // underlying schema (above), but the OUTPUT columns are a pivot
    // value per column — runtime data, not statically knowable. Mirror
    // `handle_agg` and bail with Unknown so the chain dies cleanly.
    if *after_pivot {
        return None;
    }
    Some(SchemaView::Derived(fields))
}

/// Spark's type rule for the `groupBy.<method>(col)` shortcuts.
///
/// `sum` widens `int` to `long` (matching `pyspark.sql.functions.sum`),
/// preserves `long` and `double`, and is `None` for any other input
/// (the user shouldn't be summing strings anyway — the existing type-
/// checker covers that on `.agg(F.sum(col("x")))`).
///
/// `mean` / `avg` always return `double`; pyspark casts integer inputs
/// up. `max` / `min` return the input's type unchanged. A `Nullable`
/// wrapper is stripped at the boundary — grouped aggregates over a
/// nullable column don't preserve nullability in Spark's output schema.
pub(super) fn aggregate_output_type(
    method: &str,
    input: Option<&ColumnType>,
) -> Option<ColumnType> {
    let unwrapped = input.map(|t| match t {
        ColumnType::Nullable(inner) => inner.as_ref(),
        other => other,
    });
    // Exhaustive per-variant arms (no `_`) so a future `ColumnType`
    // variant addition is a compile error here, mirroring the §9 gate
    // discipline. The cross-path agreement test in `mod.rs` locks this
    // function to `column_exprs::aggregate_call_type` — both surfaces
    // must update together.
    match method {
        "sum" => match unwrapped? {
            ColumnType::Byte | ColumnType::Short | ColumnType::Int | ColumnType::Long => {
                Some(ColumnType::Long)
            }
            ColumnType::Double => Some(ColumnType::Double),
            // Float widens to Double — Spark's float-collapse convention
            // (see `from_type_constructor`); matches `column_exprs::sum`.
            ColumnType::Float => Some(ColumnType::Double),
            // Spark widens to `decimal(p + 10, s)` (capped at 38); pykrete
            // collapses that to Spark's default `decimal(10, 0)` for v1.0
            // — the audit flagged precision-growth modeling as v1.1 polish.
            ColumnType::Decimal { .. } => Some(ColumnType::DEFAULT_DECIMAL),
            ColumnType::String
            | ColumnType::Enum(_)
            | ColumnType::Binary
            | ColumnType::Bool
            | ColumnType::Date
            | ColumnType::Timestamp
            | ColumnType::Array(_)
            | ColumnType::Map(..)
            | ColumnType::Struct(_) => None,
            // `Nullable` was already stripped by `unwrapped`; this arm
            // is unreachable in practice — listed for exhaustiveness.
            ColumnType::Nullable(_) => None,
        },
        "mean" | "avg" => match unwrapped? {
            ColumnType::Byte
            | ColumnType::Short
            | ColumnType::Int
            | ColumnType::Long
            | ColumnType::Double => Some(ColumnType::Double),
            // Float promotes to Double — matches the `sum` discipline
            // above and `column_exprs::avg`.
            ColumnType::Float => Some(ColumnType::Double),
            // Same simplification as `sum`: Spark widens to
            // `decimal(p + 4, s + 4)` (cap 38); pykrete collapses.
            ColumnType::Decimal { .. } => Some(ColumnType::DEFAULT_DECIMAL),
            ColumnType::String
            | ColumnType::Enum(_)
            | ColumnType::Binary
            | ColumnType::Bool
            | ColumnType::Date
            | ColumnType::Timestamp
            | ColumnType::Array(_)
            | ColumnType::Map(..)
            | ColumnType::Struct(_) => None,
            ColumnType::Nullable(_) => None,
        },
        "max" | "min" => unwrapped.cloned(),
        _ => None,
    }
}
