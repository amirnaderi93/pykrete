use std::collections::HashSet;

use super::col_refs::collect_col_refs;
use super::column_exprs::{common_branch_type, infer_expr_type};
use super::context::BodyContext;
use super::shapes::{ColumnMethodShape, role_at};
use super::strict_operators::{
    collect_arg_column_refs, report_expr_sql_refs, report_expr_type_errors,
};

use ruff_python_ast::{Expr, ExprCall};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::{
    DerivedField, FieldPathResult, FieldResolution, SchemaView, resolve_path, suggest_field_name,
};
use crate::types::ColumnType;

// ---------------------------------------------------------------------------
// Column-method checking + result inference
// ---------------------------------------------------------------------------

pub(super) fn check_column_method_args<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'a>,
    shape: &ColumnMethodShape,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for (i, arg) in call.arguments.args.iter().enumerate() {
        let role = role_at(shape, i);
        collect_arg_column_refs(arg, role, ctx, &mut refs);
        // `F.expr("…")` anywhere in the argument carries a SQL fragment;
        // its identifiers are checked against the same schema.
        report_expr_sql_refs(arg, schema, source, line_index, diagnostics);
        report_expr_type_errors(arg, schema, ctx.type_ctx(), source, line_index, diagnostics);
        // Chained Column-on-Column accesses (`df.r.X`, `df["r"].X`,
        // `df.r["X"]`, `df["r"]["X"]`) that drill into a nested struct
        // column — checked separately because they don't reduce to a
        // single name in the source. Single-step refs (`df.X`, `df["X"]`)
        // are still handled by collect_arg_column_refs above.
        check_chained_field_access(arg, schema, ctx, source, line_index, diagnostics);
    }
    report_column_refs(&refs, schema, ctx, source, line_index, diagnostics);
}

/// Walks `expr` for chained DataFrame-column accesses of the form
/// `df.<f1>.<f2>...` / `df["<f1>"]["<f2>"]...` / mixed, and verifies
/// each step against the receiver schema (descending into nested
/// structs as it goes). Single-step accesses are left for the
/// existing `collect_col_refs` arm to handle.
///
/// Emits D0030 with the failing segment's range and the schema we
/// failed on (so the diagnostic for `df.r.typo` reads "Column 'typo'
/// does not exist on schema 'R'", not "...on schema 'LR'").
fn check_chained_field_access<'a>(
    expr: &'a Expr,
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Walk into every sub-expression so we catch chained accesses inside
    // bigger trees (e.g. `df.select(df.r.typo, ...)`).
    match expr {
        Expr::Attribute(_) | Expr::Subscript(_) => {
            if let Some(chain) = extract_chained_access(expr, ctx)
                && chain.len() >= 2
            {
                report_chain_against_schema(&chain, receiver, ctx, source, line_index, diagnostics);
                return;
            }
            // Single-step or a chain we couldn't parse — descend into
            // the inner value to keep searching.
            match expr {
                Expr::Attribute(a) => check_chained_field_access(
                    &a.value,
                    receiver,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                ),
                Expr::Subscript(s) => {
                    check_chained_field_access(
                        &s.value,
                        receiver,
                        ctx,
                        source,
                        line_index,
                        diagnostics,
                    );
                    check_chained_field_access(
                        &s.slice,
                        receiver,
                        ctx,
                        source,
                        line_index,
                        diagnostics,
                    );
                }
                // The outer match arm already pinned `expr` to
                // `Attribute | Subscript` — the inner re-match cannot see
                // any other variant. A new `Expr::Foo` would have to add
                // itself to BOTH matches before reaching here.
                _ => unreachable!(),
            }
        }
        Expr::Call(c) => {
            // For a method call `<recv>.<method>(...)`, c.func is
            // Attribute(<recv>, <method>). The METHOD name isn't a
            // field on `<recv>`'s schema — it's a method on the Column
            // object. Only walk INTO the receiver; don't let
            // extract_chained_access pick up the method as a final
            // step. (Without this, `df["r"].withField(...)` would
            // flag 'withField' as a missing field on schema R.)
            match c.func.as_ref() {
                Expr::Attribute(a) => check_chained_field_access(
                    &a.value,
                    receiver,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                ),
                other => check_chained_field_access(
                    other,
                    receiver,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                ),
            }
            for arg in &c.arguments.args {
                check_chained_field_access(arg, receiver, ctx, source, line_index, diagnostics);
            }
            for kw in &c.arguments.keywords {
                check_chained_field_access(
                    &kw.value,
                    receiver,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                );
            }
        }
        Expr::BinOp(b) => {
            check_chained_field_access(&b.left, receiver, ctx, source, line_index, diagnostics);
            check_chained_field_access(&b.right, receiver, ctx, source, line_index, diagnostics);
        }
        Expr::UnaryOp(u) => {
            check_chained_field_access(&u.operand, receiver, ctx, source, line_index, diagnostics)
        }
        Expr::Compare(c) => {
            check_chained_field_access(&c.left, receiver, ctx, source, line_index, diagnostics);
            for cmp in &c.comparators {
                check_chained_field_access(cmp, receiver, ctx, source, line_index, diagnostics);
            }
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                check_chained_field_access(v, receiver, ctx, source, line_index, diagnostics);
            }
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                check_chained_field_access(e, receiver, ctx, source, line_index, diagnostics);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                check_chained_field_access(e, receiver, ctx, source, line_index, diagnostics);
            }
        }
        _ => {}
    }
}

/// Extract a chain of (name, range) steps from a possibly-chained
/// attribute / subscript expression bottoming out at a DataFrame-bound
/// Name. Returns `None` if the bottom isn't a known DataFrame, or any
/// step is a non-string-literal subscript (computed). The chain is in
/// access order — outermost step last.
fn extract_chained_access<'a>(
    expr: &'a Expr,
    ctx: &BodyContext<'a>,
) -> Option<Vec<(&'a str, TextRange)>> {
    let mut parts: Vec<(&'a str, TextRange)> = Vec::new();
    let mut cursor = expr;
    loop {
        match cursor {
            Expr::Attribute(a) => {
                parts.push((a.attr.id.as_str(), a.attr.range));
                cursor = &a.value;
            }
            Expr::Subscript(s) => {
                let lit = s.slice.as_string_literal_expr()?;
                parts.push((lit.value.to_str(), lit.range()));
                cursor = &s.value;
            }
            Expr::Name(n) if ctx.lookup(n.id.as_str()).is_some() => {
                parts.reverse();
                return Some(parts);
            }
            _ => return None,
        }
    }
}

/// Walk a pre-extracted chain of segments against `receiver`, emitting
/// D0030 at the first failing step. Mirrors `resolve_path`'s descent
/// logic but consumes segments segment-by-segment (no path String
/// allocation needed).
fn report_chain_against_schema<'a>(
    chain: &[(&'a str, TextRange)],
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut current = receiver.clone();
    for (i, &(segment, segment_range)) in chain.iter().enumerate() {
        let is_last = i + 1 == chain.len();
        if !current.has_field(segment) {
            let suggestion = suggest_field_name(segment, &current);
            let on_phrase = current.display_name();
            let mut message = format!("Column '{segment}' does not exist on {on_phrase}.");
            if let Some(s) = &suggestion {
                message.push_str(&format!(" Did you mean '{s}'?"));
            }
            diagnostics.push(
                Diagnostic::at_range(
                    Severity::Error,
                    "D0030",
                    message,
                    segment_range,
                    source,
                    line_index,
                )
                .with_suggestion(suggestion),
            );
            return;
        }
        if is_last {
            return;
        }
        // Descend into a nested Declared schema if possible. For other
        // composite field types (array, map, opaque struct) we stop
        // walking rather than risk a false positive — the single-step
        // arm already verified the field exists, so the diagnostic
        // budget for this access is spent.
        let next = match &current {
            SchemaView::Declared(s) => {
                s.fields().iter().find(|f| f.name == segment).and_then(|f| {
                    match f.resolve(ctx.schemas()) {
                        FieldResolution::ResolvedNested(nested) => Some(nested),
                        _ => None,
                    }
                })
            }
            _ => None,
        };
        match next {
            Some(nested) => current = SchemaView::Declared(nested),
            None => return,
        }
    }
}

/// Model `df.withColumns({"a": expr, "b": expr})` (Spark 3.3+) — adds
/// one column per dict entry. The keys are the new column names; the
/// values are column expressions whose own references are checked
/// against the receiver. Result schema = receiver columns + new keys.
///
/// If the argument isn't a dict literal the added names are unknown —
/// the receiver schema is returned so the chain at least stays alive.
pub(super) fn apply_with_columns<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let Some(dict) = call.arguments.args.first().and_then(Expr::as_dict_expr) else {
        return recv.clone();
    };
    let mut fields: Vec<DerivedField<'a>> = recv.typed_fields(ctx.schemas());
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for item in &dict.items {
        if let Some(key) = item.key.as_ref().and_then(|k| k.as_string_literal_expr()) {
            let name = key.value.to_str();
            let ty = infer_expr_type(&item.value, recv, ctx.type_ctx());
            // `withColumns` replaces an existing column or adds a new one.
            if let Some(existing) = fields.iter_mut().find(|f| f.name == name) {
                existing.ty = ty;
            } else {
                fields.push(DerivedField { name, ty });
            }
        }
        collect_col_refs(&item.value, ctx, &mut refs);
        report_expr_sql_refs(&item.value, recv, source, line_index, diagnostics);
        report_expr_type_errors(
            &item.value,
            recv,
            ctx.type_ctx(),
            source,
            line_index,
            diagnostics,
        );
    }
    report_column_refs(&refs, recv, ctx, source, line_index, diagnostics);
    SchemaView::Derived(fields)
}

/// Model `df.withColumnsRenamed({"old": "new", …})` (Spark 3.4+). Each
/// key is an existing column (checked against the receiver) renamed to
/// its value. Result schema = receiver columns with the renames applied.
pub(super) fn apply_with_columns_renamed<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let Some(dict) = call.arguments.args.first().and_then(Expr::as_dict_expr) else {
        return recv.clone();
    };
    let mut renames: Vec<(&'a str, &'a str)> = Vec::new();
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for item in &dict.items {
        let (Some(key), Some(val)) = (
            item.key.as_ref().and_then(|k| k.as_string_literal_expr()),
            item.value.as_string_literal_expr(),
        ) else {
            continue;
        };
        // The old name must exist — check it like any column reference.
        refs.push((key.value.to_str(), key.range()));
        renames.push((key.value.to_str(), val.value.to_str()));
    }
    report_column_refs(&refs, recv, ctx, source, line_index, diagnostics);
    let fields: Vec<DerivedField<'a>> = recv
        .typed_fields(ctx.schemas())
        .into_iter()
        .map(|mut f| {
            if let Some((_, new)) = renames.iter().find(|(old, _)| *old == f.name) {
                f.name = new;
            }
            f
        })
        .collect();
    SchemaView::Derived(fields)
}

/// Model `df.melt(ids, values, variableColumnName, valueColumnName)` and
/// its alias `df.unpivot(...)` (Spark 3.4+). Reshapes a wide table into a
/// long one: the `ids` columns are preserved, each `values` column is
/// emitted as a row whose key (= the original column name) goes into the
/// `variable` column and whose payload goes into the `value` column.
///
/// Result schema: the `ids` columns (preserved with their declared types
/// and nullability), the variable column (`string`, non-nullable), and
/// the value column (common type across all `values` columns;
/// `Nullable(T)` if any of them is nullable).
///
/// `values=None` (or omitted) means "unpivot every non-`ids` column" — the
/// value column's common type is computed across those.
///
/// `variableColumnName` / `valueColumnName` default to Spark's `"variable"`
/// / `"value"` when not given. If `ids` or `values` aren't a static list
/// of string literals, or the variable/value names aren't strings, the
/// call falls back to the receiver schema rather than fabricate one.
pub(super) fn apply_melt<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let ids_arg = melt_arg(call, "ids", 0);
    let values_arg = melt_arg(call, "values", 1);
    let var_name_arg = melt_arg(call, "variableColumnName", 2);
    let val_name_arg = melt_arg(call, "valueColumnName", 3);

    // `ids` is required and must be a list of string literals.
    let Some(ids) = ids_arg.and_then(parse_string_list) else {
        return recv.clone();
    };
    // `values` may be omitted or `None` → unpivot all non-`ids` columns.
    let values: Option<Vec<(&'a str, TextRange)>> =
        match values_arg.map(|e| (e, parse_string_list(e), expr_is_none_literal(e))) {
            None => None,
            Some((_, _, true)) => None,
            Some((_, Some(list), _)) => Some(list),
            // Non-literal `values` expr — bail out to receiver.
            Some((_, None, _)) => return recv.clone(),
        };

    // Validate ids + (if present) values column refs against the receiver.
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    refs.extend(ids.iter().copied());
    if let Some(ref vs) = values {
        refs.extend(vs.iter().copied());
    }
    report_column_refs(&refs, recv, ctx, source, line_index, diagnostics);

    // Variable / value column names — string literals only, or Spark defaults.
    let var_name: &'a str = match var_name_arg {
        Some(e) => match e.as_string_literal_expr() {
            Some(s) => s.value.to_str(),
            None => return recv.clone(),
        },
        None => "variable",
    };
    let val_name: &'a str = match val_name_arg {
        Some(e) => match e.as_string_literal_expr() {
            Some(s) => s.value.to_str(),
            None => return recv.clone(),
        },
        None => "value",
    };

    let recv_fields = recv.typed_fields(ctx.schemas());
    let id_set: HashSet<&str> = ids.iter().map(|(n, _)| *n).collect();

    // Carry forward the ids' types and nullability from the receiver.
    let mut out_fields: Vec<DerivedField<'a>> = Vec::new();
    for (name, _) in &ids {
        // Use the receiver field if we have it (preserves the borrowed
        // schema-source &'a str). Skip silently if the name didn't
        // resolve — `report_column_refs` already emitted D0030.
        if let Some(f) = recv_fields.iter().find(|f| f.name == *name) {
            out_fields.push(f.clone());
        }
    }

    // The set of value columns whose types feed the common-type computation.
    let value_field_types: Vec<Option<ColumnType>> = match values {
        Some(vs) => vs
            .iter()
            .map(|(n, _)| {
                recv_fields
                    .iter()
                    .find(|f| f.name == *n)
                    .and_then(|f| f.ty.clone())
            })
            .collect(),
        None => recv_fields
            .iter()
            .filter(|f| !id_set.contains(f.name))
            .map(|f| f.ty.clone())
            .collect(),
    };

    let value_ty = melt_value_column_type(&value_field_types);

    // The variable column is always `string`, non-nullable.
    out_fields.push(DerivedField {
        name: var_name,
        ty: Some(ColumnType::String),
    });
    out_fields.push(DerivedField {
        name: val_name,
        ty: value_ty,
    });
    SchemaView::Derived(out_fields)
}

/// Resolve `melt`'s named/positional arg at one shot — `melt` takes
/// `ids, values, variableColumnName, valueColumnName` in that order, and
/// every slot may also be passed by keyword.
fn melt_arg<'a>(call: &'a ExprCall, kw: &str, pos: usize) -> Option<&'a Expr> {
    call.arguments
        .keywords
        .iter()
        .find(|k| k.arg.as_ref().is_some_and(|n| n.id.as_str() == kw))
        .map(|k| &k.value)
        .or_else(|| call.arguments.args.get(pos))
}

/// Parse `[<lit>, <lit>, ...]` (list or tuple) into a vec of
/// `(name, range)` pairs. Returns `None` if the expression isn't a
/// homogeneous list/tuple of string literals.
fn parse_string_list(expr: &Expr) -> Option<Vec<(&str, TextRange)>> {
    let elts: &[Expr] = match expr {
        Expr::List(l) => &l.elts,
        Expr::Tuple(t) => &t.elts,
        _ => return None,
    };
    let mut out = Vec::with_capacity(elts.len());
    for elt in elts {
        let s = elt.as_string_literal_expr()?;
        out.push((s.value.to_str(), s.range()));
    }
    Some(out)
}

/// `True` if `expr` is the Python literal `None`. `melt(values=None, ...)`
/// is equivalent to omitting `values`.
fn expr_is_none_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::NoneLiteral(_))
}

/// The common type of `melt`'s `values` columns. Atomic equality with
/// numeric widening (`int` < `long` < `double`); any branch nullable →
/// `Nullable(T)`. Returns `None` (Unknown) when no two branches share a
/// reconcilable type, so downstream checks stay permissive.
fn melt_value_column_type(branch_types: &[Option<ColumnType>]) -> Option<ColumnType> {
    let common = common_branch_type(branch_types)?;
    let any_nullable = branch_types
        .iter()
        .any(|t| t.as_ref().is_some_and(ColumnType::is_nullable));
    Some(if any_nullable {
        ColumnType::Nullable(Box::new(common.base().clone()))
    } else {
        common
    })
}

/// Check the dict-literal first positional arg of `fillna` / `na.fill`,
/// whose keys are column names. Non-dict-literal first args (a bare
/// value, a variable) fall through silently — only the syntactically
/// visible dict can be checked here.
pub(super) fn check_fillna_dict_keys<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(arg) = call.arguments.args.first() else {
        return;
    };
    let Some(dict) = arg.as_dict_expr() else {
        return;
    };
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for item in &dict.items {
        let Some(key) = item.key.as_ref() else {
            continue;
        };
        let Some(s) = key.as_string_literal_expr() else {
            continue;
        };
        refs.push((s.value.to_str(), s.range()));
    }
    report_column_refs(&refs, schema, ctx, source, line_index, diagnostics);
}

/// Check the `subset=` keyword argument against the receiver schema.
/// `subset` — present on `fillna`, `dropna`, `dropDuplicates`,
/// `replace`, and the `df.na.*` methods — names the columns the
/// operation applies to, as a single string or a list/tuple of them.
pub(super) fn check_subset_kwarg<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(kw) = call
        .arguments
        .keywords
        .iter()
        .find(|k| k.arg.as_ref().is_some_and(|n| n.id.as_str() == "subset"))
    else {
        return;
    };
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    match &kw.value {
        Expr::StringLiteral(s) => refs.push((s.value.to_str(), s.range())),
        Expr::List(l) => {
            for elt in &l.elts {
                if let Some(s) = elt.as_string_literal_expr() {
                    refs.push((s.value.to_str(), s.range()));
                }
            }
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                if let Some(s) = elt.as_string_literal_expr() {
                    refs.push((s.value.to_str(), s.range()));
                }
            }
        }
        _ => {}
    }
    report_column_refs(&refs, schema, ctx, source, line_index, diagnostics);
}

/// Resolve each collected `(name, range)` column reference against
/// `schema`: record it for the LSP layer, and emit a `D0030` (with a
/// "did you mean" suggestion) for any that doesn't resolve. The shared
/// tail of every `col(...)`-style column-existence check.
pub(super) fn report_column_refs<'a>(
    refs: &[(&'a str, TextRange)],
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for &(col_name, col_range) in refs {
        if try_resolve_alias_ref(col_name, col_range, ctx, source, line_index, diagnostics) {
            continue;
        }
        ctx.record_column_ref(col_range, col_name, schema.clone());
        if let FieldPathResult::Missing { field, on } =
            resolve_path(schema, col_name, ctx.schemas())
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
                    col_range,
                    source,
                    line_index,
                )
                .with_suggestion(suggestion),
            );
        }
    }
}

/// Split a column reference of the form `"alias.col"` into its alias
/// and column halves. Returns `None` for unqualified names (no `.`),
/// for empty halves on either side, and for names with more than one
/// `.` (nested-struct accesses like `addr.city` aren't aliases and
/// must keep their existing field-path semantics — those are handled
/// by the caller's normal resolver). The narrow allowance captures the
/// canonical `df.alias("L"); col("L.region")` pattern without
/// rerouting real dotted paths.
pub(super) fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let (prefix, suffix) = name.split_once('.')?;
    if prefix.is_empty() || suffix.is_empty() || suffix.contains('.') {
        return None;
    }
    Some((prefix, suffix))
}

/// Apply `df.alias("L")`-style resolution to a single column ref. If
/// `col_name` is `"alias.suffix"` AND any aliases are registered in
/// scope, route the lookup through the aliased schema (firing D0030 on
/// the suffix typo, or on the prefix when the alias isn't in scope).
/// Returns `true` when the helper resolved or diagnosed the ref —
/// callers should then skip their normal resolver. Returns `false`
/// when the ref isn't alias-shaped (or no aliases are in scope), in
/// which case the caller's nested-struct / single-name resolver
/// continues to apply.
///
/// Shared between `report_column_refs` (the central col-ref check
/// used by select/filter/withColumn/groupBy/…) and `check_join_keys`
/// (the join expression-form on-clause) so every column-checking site
/// honors the alias pattern uniformly. Without this lift, sites other
/// than the join-on path would false-fire D0030 on the
/// `L = raw.alias("L"); L.select(col("L.region"))` shape — the most
/// common form of the alias pattern in production PySpark.
pub(super) fn try_resolve_alias_ref<'a>(
    col_name: &'a str,
    col_range: TextRange,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let Some((prefix, suffix)) = split_qualified(col_name) else {
        return false;
    };
    if ctx.known_aliases().is_empty() {
        return false;
    }
    if let Some(aliased) = ctx.lookup_alias(prefix) {
        ctx.record_column_ref(col_range, col_name, aliased.clone());
        if !aliased.has_field(suffix) {
            let suggestion = suggest_field_name(suffix, &aliased);
            let mut message = format!(
                "Column '{suffix}' does not exist on alias '{prefix}' ({}).",
                aliased.display_name(),
            );
            if let Some(s) = &suggestion {
                message.push_str(&format!(" Did you mean '{s}'?"));
            }
            diagnostics.push(
                Diagnostic::at_range(
                    Severity::Error,
                    "D0030",
                    message,
                    col_range,
                    source,
                    line_index,
                )
                .with_suggestion(suggestion),
            );
        }
        return true;
    }
    let known = ctx.known_aliases();
    let mut message = format!("Alias '{prefix}' is not in scope.");
    if !known.is_empty() {
        message.push_str(&format!(" Known aliases: {}.", known.join(", ")));
    }
    diagnostics.push(Diagnostic::at_range(
        Severity::Error,
        "D0030",
        message,
        col_range,
        source,
        line_index,
    ));
    true
}
