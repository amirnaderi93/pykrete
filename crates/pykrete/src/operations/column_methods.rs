use std::collections::HashSet;

use super::col_refs::collect_col_refs;
use super::column_exprs::{common_branch_type, infer_expr_type};
use super::context::BodyContext;
use super::enum_checks::{emit_d0084, enum_vocab, peel_string_literal, vocab_contains};
use super::shapes::{ColumnMethodShape, role_at};
use super::strict_operators::{
    collect_arg_column_refs_split, report_expr_sql_refs, report_expr_type_errors,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn check_column_method_args<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'a>,
    shape: &ColumnMethodShape,
    method: &str,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut string_refs: Vec<(&'a str, TextRange)> = Vec::new();
    let mut column_refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    for (i, arg) in call.arguments.args.iter().enumerate() {
        let role = role_at(shape, i);
        collect_arg_column_refs_split(arg, role, ctx, &mut string_refs, &mut column_refs);
        // `F.expr("…")` anywhere in the argument carries a SQL fragment;
        // its identifiers are checked against the same schema.
        report_expr_sql_refs(arg, schema, source, line_index, diagnostics);
        report_expr_type_errors(
            arg,
            schema,
            ctx.type_ctx(),
            ctx,
            source,
            line_index,
            diagnostics,
        );
        // Chained Column-on-Column accesses (`df.r.X`, `df["r"].X`,
        // `df.r["X"]`, `df["r"]["X"]`) that drill into a nested struct
        // column — checked separately because they don't reduce to a
        // single name in the source. Single-step refs (`df.X`, `df["X"]`)
        // are still handled by collect_arg_column_refs above.
        check_chained_field_access(arg, schema, ctx, source, line_index, diagnostics);
    }
    if tolerates_missing_column_names(method) {
        // Spark silently tolerates the string-literal form of missing
        // names; Column-form refs (e.g. `df.drop(col("typo"))`) are
        // explicit references and still get the D0030 check.
        record_column_refs_tolerating_missing(&string_refs, schema, ctx);
        report_column_refs(&column_refs, schema, ctx, source, line_index, diagnostics);
    } else {
        let mut all: Vec<(Option<&'a str>, &'a str, TextRange)> = string_refs
            .into_iter()
            .map(|(name, range)| (None, name, range))
            .collect();
        all.extend(column_refs);
        report_column_refs(&all, schema, ctx, source, line_index, diagnostics);
    }
}

// Spark's `df.drop(*cols)` silently ignores names that aren't in the
// schema (per its source / docs: "drop columns ... ignored if schema
// doesn't contain column name(s)"). Firing D0030 on a missing name
// here would flag working production code; match Spark's behavior
// instead. Scope: STRING-FORM ONLY — `df.drop(col("typo"))` is an
// explicit Column reference and still gets the typo check.
pub(super) fn tolerates_missing_column_names(method: &str) -> bool {
    matches!(method, "drop")
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
    let pairs = dict.items.iter().filter_map(|item| {
        let key = item.key.as_ref()?.as_string_literal_expr()?;
        Some((key.value.to_str(), &item.value))
    });
    apply_add_columns_iter(pairs, recv, ctx, source, line_index, diagnostics)
}

/// Shared body for `withColumns` (dict-keyed) and `assign` (kwarg-keyed):
/// for each `(name, value-expr)` pair, add-or-replace `name` on the
/// receiver schema with the value-expr's inferred type, and check the
/// value-expr's column refs / SQL fragments / type errors against the
/// receiver. Returns the resulting derived schema. Used by both Spark
/// `.withColumns` and pandas `.assign` so future enum-sink / col-ref /
/// type-checking improvements land in one place.
fn apply_add_columns_iter<'a, I>(
    pairs: I,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a>
where
    I: IntoIterator<Item = (&'a str, &'a Expr)>,
{
    let mut fields: Vec<DerivedField<'a>> = recv.typed_fields(ctx.schemas());
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    for (name, value) in pairs {
        let ty = infer_expr_type(value, recv, ctx.type_ctx(), ctx);
        add_or_replace_column(&mut fields, name, ty);
        collect_col_refs(value, ctx, &mut refs);
        report_expr_sql_refs(value, recv, source, line_index, diagnostics);
        report_expr_type_errors(
            value,
            recv,
            ctx.type_ctx(),
            ctx,
            source,
            line_index,
            diagnostics,
        );
    }
    report_column_refs(&refs, recv, ctx, source, line_index, diagnostics);
    SchemaView::Derived(fields)
}

/// Add a column named `name` with type `ty` to `fields`, replacing any
/// existing entry of the same name. Shared by every site that grows a
/// schema column-by-column (`withColumn`, `withColumns`, pandas `assign`,
/// pandas `df["x"] = expr`).
pub(super) fn add_or_replace_column<'a>(
    fields: &mut Vec<DerivedField<'a>>,
    name: &'a str,
    ty: Option<ColumnType>,
) {
    if let Some(existing) = fields.iter_mut().find(|f| f.name == name) {
        existing.ty = ty;
    } else {
        fields.push(DerivedField { name, ty });
    }
}

/// Model `df.withColumnsRenamed({"old": "new", …})` (Spark 3.4+). Each
/// key is an existing column (checked against the receiver) renamed to
/// its value. Result schema = receiver columns with the renames applied.
pub(super) fn apply_with_columns_renamed<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    _source: &str,
    _line_index: &LineIndex,
    _diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let Some(dict) = call.arguments.args.first().and_then(Expr::as_dict_expr) else {
        return recv.clone();
    };
    apply_rename_dict_inner(dict, recv, ctx)
}

/// v1.3 pandas dispatch (spec §5) — `df.rename(columns={...})`. Same
/// shape as `withColumnsRenamed`'s positional-dict form; routed here
/// so the kwarg surface doesn't have to be re-derived inside the
/// Spark helper.
pub(super) fn apply_rename_dict<'a>(
    dict: &'a ruff_python_ast::ExprDict,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
) -> SchemaView<'a> {
    apply_rename_dict_inner(dict, recv, ctx)
}

fn apply_rename_dict_inner<'a>(
    dict: &'a ruff_python_ast::ExprDict,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
) -> SchemaView<'a> {
    let mut renames: Vec<(&'a str, &'a str)> = Vec::new();
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for item in &dict.items {
        let (Some(key), Some(val)) = (
            item.key.as_ref().and_then(|k| k.as_string_literal_expr()),
            item.value.as_string_literal_expr(),
        ) else {
            continue;
        };
        // Record for LSP. PySpark's `withColumnsRenamed` silently
        // ignores keys that aren't in the schema (same design as
        // `df.drop`), so missing names must not fire D0030. Pandas
        // `.rename` shares the same tolerant behavior.
        refs.push((key.value.to_str(), key.range()));
        renames.push((key.value.to_str(), val.value.to_str()));
    }
    record_column_refs_tolerating_missing(&refs, recv, ctx);
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

/// v1.3 pandas dispatch (spec §5) — `df.assign(new=expr, …)`. Pandas
/// passes the new columns as keyword arguments; each kwarg's name is
/// a new (or replacement) column, and the value is an expression
/// whose embedded col-refs are checked against the receiver. Shares
/// the add-or-replace + col-ref / SQL / type-error machinery with
/// Spark `.withColumns` via [`apply_add_columns_iter`].
pub(super) fn apply_pandas_assign<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let pairs = call.arguments.keywords.iter().filter_map(|kw| {
        let name = kw.arg.as_ref()?.id.as_str();
        Some((name, &kw.value))
    });
    apply_add_columns_iter(pairs, recv, ctx, source, line_index, diagnostics)
}

/// v1.3 pandas dispatch (spec §5) — `df.drop(columns=[...])`. Each
/// list element is a column on the receiver; the result schema is
/// the receiver minus those names. Missing names fire D0030 (same
/// as Spark `.drop`).
pub(super) fn apply_pandas_drop_columns<'a>(
    list: &'a ruff_python_ast::ExprList,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    let mut drop_set: HashSet<&str> = HashSet::new();
    for elt in &list.elts {
        if let Some(lit) = elt.as_string_literal_expr() {
            refs.push((None, lit.value.to_str(), lit.range()));
            drop_set.insert(lit.value.to_str());
        }
    }
    report_column_refs(&refs, recv, ctx, source, line_index, diagnostics);
    let remaining: Vec<DerivedField<'a>> = recv
        .typed_fields(ctx.schemas())
        .into_iter()
        .filter(|f| !drop_set.contains(f.name))
        .collect();
    SchemaView::Derived(remaining)
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
    // ids/values are string-literal-form refs (no receiver-Name) — None.
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    refs.extend(ids.iter().map(|&(n, r)| (None, n, r)));
    if let Some(ref vs) = values {
        refs.extend(vs.iter().map(|&(n, r)| (None, n, r)));
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
pub(super) fn parse_string_list(expr: &Expr) -> Option<Vec<(&str, TextRange)>> {
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

/// `df.withColumn("status", value)` — when `status` is already declared
/// enum-typed on the receiver, every literal in `value` (including
/// branches inside `coalesce` / `when` / `nvl` / `nullif` / `ifnull`)
/// is checked against the enum vocabulary. Q1 / Q6 / Q9 sink site.
pub(super) fn check_with_column_enum_sink<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(name_arg) = call.arguments.args.first() else {
        return;
    };
    let Some(name_lit) = name_arg.as_string_literal_expr() else {
        return;
    };
    let column = name_lit.value.to_str();
    let Some(value_expr) = call.arguments.args.get(1) else {
        return;
    };
    let Some(ty) = schema.field_type(column, ctx.schemas()) else {
        return;
    };
    let Some(vocab) = enum_vocab(&ty) else {
        return;
    };
    let mut cx = EnumCheckCtx {
        source,
        line_index,
        diagnostics,
    };
    check_value_against_enum_vocab(column, vocab, value_expr, &mut cx);
}

/// v1.3 pandas dispatch (spec §5) — `df["status"] = value`. When
/// `status` is enum-typed on the receiver, route through the same
/// vocabulary check as `withColumn` / `withColumns`. Slice name is
/// always a literal here; the col-ref check on the slice has
/// already fired via piece (b)'s descent.
pub(super) fn check_pandas_subscript_assign_enum_sink<'a>(
    column: &str,
    value_expr: &'a Expr,
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(ty) = schema.field_type(column, ctx.schemas()) else {
        return;
    };
    let Some(vocab) = enum_vocab(&ty) else {
        return;
    };
    let mut cx = EnumCheckCtx {
        source,
        line_index,
        diagnostics,
    };
    check_value_against_enum_vocab(column, vocab, value_expr, &mut cx);
}

/// `df.withColumns({"status": value, ...})` — same Q1 / Q6 / Q9 sink
/// check as `withColumn`, but iterating every dict entry. Only the
/// keys that name existing enum-typed columns on the receiver
/// participate; brand-new columns introduced by this `withColumns`
/// call have no declared enum constraint to check against.
pub(super) fn check_with_columns_enum_sinks<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(dict) = call.arguments.args.first().and_then(Expr::as_dict_expr) else {
        return;
    };
    for item in &dict.items {
        let Some(key_lit) = item.key.as_ref().and_then(|k| k.as_string_literal_expr()) else {
            continue;
        };
        let column = key_lit.value.to_str();
        let Some(ty) = schema.field_type(column, ctx.schemas()) else {
            continue;
        };
        let Some(vocab) = enum_vocab(&ty) else {
            continue;
        };
        let mut cx = EnumCheckCtx {
            source,
            line_index,
            diagnostics,
        };
        check_value_against_enum_vocab(column, vocab, &item.value, &mut cx);
    }
}

/// v1.3 pandas dispatch (spec §5) — `pdf.assign(status="…", …)`. Pandas
/// analog of [`check_with_columns_enum_sinks`]: kwargs carry the new-
/// column writes, so iterate `call.arguments.keywords` instead of a
/// dict-literal positional arg. Only kwargs whose key names an existing
/// enum-typed column on the receiver participate; brand-new columns
/// have no declared constraint to check against.
pub(super) fn check_pandas_assign_enum_sinks<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for kw in &call.arguments.keywords {
        let Some(name) = kw.arg.as_ref() else {
            continue;
        };
        let column = name.id.as_str();
        let Some(ty) = schema.field_type(column, ctx.schemas()) else {
            continue;
        };
        let Some(vocab) = enum_vocab(&ty) else {
            continue;
        };
        let mut cx = EnumCheckCtx {
            source,
            line_index,
            diagnostics,
        };
        check_value_against_enum_vocab(column, vocab, &kw.value, &mut cx);
    }
}

/// Check the dict-literal first positional arg of `fillna` / `na.fill`,
/// whose keys are column names. Non-dict-literal first args (a bare
/// value, a variable) fall through silently — only the syntactically
/// visible dict can be checked here.
///
/// Per the v1.1 enum-constraints spec (Q1, Q6), each dict value that
/// is a string literal (`"X"`) or `lit("X")` is also checked against
/// the corresponding column's enum vocabulary. Off-vocabulary values
/// emit `D0084`; the column-existence `D0030` check on the key takes
/// precedence (Q1c).
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
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    for item in &dict.items {
        let Some(key) = item.key.as_ref() else {
            continue;
        };
        let Some(s) = key.as_string_literal_expr() else {
            continue;
        };
        refs.push((None, s.value.to_str(), s.range()));
        let column = s.value.to_str();
        if let Some(ty) = schema.field_type(column, ctx.schemas())
            && let Some(vocab) = enum_vocab(&ty)
        {
            let mut cx = EnumCheckCtx {
                source,
                line_index,
                diagnostics,
            };
            check_value_against_enum_vocab(column, vocab, &item.value, &mut cx);
        }
    }
    report_column_refs(&refs, schema, ctx, source, line_index, diagnostics);
}

/// Shared diagnostic-emission inputs used by every D0084 check site:
/// the source text, the line index, and the diagnostics accumulator.
/// Bundling these into a single struct keeps the per-arg helper count
/// at a non-clippy-warning level (`too_many_arguments` threshold of
/// 7).
pub(super) struct EnumCheckCtx<'b, 's> {
    pub source: &'s str,
    pub line_index: &'b LineIndex,
    pub diagnostics: &'b mut Vec<Diagnostic>,
}

/// Check `value_expr` against an enum vocabulary for `column`. A
/// string-literal or `lit("…")` value is checked directly. A
/// branch-form expression (`coalesce`, `when`/`otherwise`, `nvl`,
/// `ifnull`, `nullif`) is decomposed and each branch's literal value
/// is checked against the same vocabulary (Q9 part 1).
pub(super) fn check_value_against_enum_vocab(
    column: &str,
    vocab: &[String],
    value_expr: &Expr,
    cx: &mut EnumCheckCtx<'_, '_>,
) {
    if let Some((value, range)) = peel_string_literal(value_expr) {
        if !vocab_contains(vocab, value) {
            emit_d0084(
                column,
                value,
                vocab,
                range,
                cx.source,
                cx.line_index,
                cx.diagnostics,
            );
        }
        return;
    }
    let Some(call) = value_expr.as_call_expr() else {
        return;
    };
    let fname = match call.func.as_ref() {
        Expr::Name(n) => Some(n.id.as_str()),
        Expr::Attribute(a) => Some(a.attr.id.as_str()),
        _ => None,
    };
    match fname {
        // `coalesce(a, b, ...)` / `nvl(a, b)` / `ifnull(a, b)` /
        // `nullif(a, b)` — every arg is a branch.
        Some("coalesce" | "nvl" | "ifnull" | "nullif") => {
            for arg in &call.arguments.args {
                check_value_against_enum_vocab(column, vocab, arg, cx);
            }
        }
        // `F.when(p, v).when(p2, v2).otherwise(e)` — walk the chain,
        // checking every value-position branch.
        Some("when" | "otherwise") => {
            check_when_chain_against_vocab(column, vocab, value_expr, cx);
        }
        _ => {}
    }
}

/// Walk a `F.when(p, v).when(p2, v2)…[.otherwise(e)]` chain and check
/// each value-position branch's literal against the surrounding enum
/// vocabulary (Q9). Non-literal branches (a `col("…")` of unknown
/// type, a computed expression) fall through silently.
fn check_when_chain_against_vocab(
    column: &str,
    vocab: &[String],
    chain: &Expr,
    cx: &mut EnumCheckCtx<'_, '_>,
) {
    let mut cursor = chain;
    loop {
        let Some(call) = cursor.as_call_expr() else {
            return;
        };
        let fname = match call.func.as_ref() {
            Expr::Name(n) => n.id.as_str(),
            Expr::Attribute(a) => a.attr.id.as_str(),
            _ => return,
        };
        // `.otherwise(e)` — its single arg is the final branch; then
        // descend into the receiver chain.
        if fname == "otherwise" {
            if let Some(arg) = call.arguments.args.first() {
                check_value_against_enum_vocab(column, vocab, arg, cx);
            }
            let Some(attr) = call.func.as_attribute_expr() else {
                return;
            };
            cursor = &attr.value;
            continue;
        }
        if fname == "when" {
            if let Some(v) = call.arguments.args.get(1) {
                check_value_against_enum_vocab(column, vocab, v, cx);
            }
            // Chained `.when(...)` on an earlier `.when(...)` — peel one
            // layer. Bare `when(...)` is the root.
            if let Some(attr) = call.func.as_attribute_expr() {
                cursor = &attr.value;
                continue;
            }
            return;
        }
        return;
    }
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
    let mut refs: Vec<(Option<&'a str>, &'a str, TextRange)> = Vec::new();
    match &kw.value {
        Expr::StringLiteral(s) => refs.push((None, s.value.to_str(), s.range())),
        Expr::List(l) => {
            for elt in &l.elts {
                if let Some(s) = elt.as_string_literal_expr() {
                    refs.push((None, s.value.to_str(), s.range()));
                }
            }
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                if let Some(s) = elt.as_string_literal_expr() {
                    refs.push((None, s.value.to_str(), s.range()));
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
    refs: &[(Option<&'a str>, &'a str, TextRange)],
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for &(receiver_name, col_name, col_range) in refs {
        // I1 fix (v1.5 §3.1): when the col-ref came from `<recv>.X` or
        // `<recv>["X"]` (receiver-Name captured at the leaf), route the
        // schema lookup to `<recv>`'s own schema rather than the
        // surrounding chain's. `df.select(df_other["col"])` then checks
        // `col` on `df_other`, closing the cross-DataFrame leak.
        let receiver_schema = receiver_name.and_then(|n| ctx.lookup(n));
        let active = receiver_schema.as_ref().unwrap_or(schema);
        // Receiver-first disambiguation: if the receiver schema can
        // resolve this dotted path (e.g. `addr.city` on a `Customer`
        // with `addr: Addr`), trust the nested-struct interpretation
        // even when the prefix ALSO happens to match a registered
        // alias name. Otherwise an unrelated `X = orders.alias("X")`
        // in the same body would hijack `col("X.y")` on a `df` whose
        // schema has its own `X: struct<y: ...>`. Only when the
        // receiver can't resolve the path do we fall back to alias
        // routing (the canonical `df.alias("L"); col("L.region")` case
        // still works — `L` isn't a field of the aliased df itself).
        if split_qualified(col_name).is_some()
            && matches!(
                resolve_path(active, col_name, ctx.schemas()),
                FieldPathResult::Resolved
            )
        {
            ctx.record_column_ref(col_range, col_name, active.clone());
            continue;
        }
        if try_resolve_alias_ref(col_name, col_range, ctx, source, line_index, diagnostics) {
            continue;
        }
        ctx.record_column_ref(col_range, col_name, active.clone());
        if let FieldPathResult::Missing { field, on } =
            resolve_path(active, col_name, ctx.schemas())
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

/// LSP-only variant of `report_column_refs`: record each ref against
/// `schema` for hover/jump-to-definition, but DO NOT emit D0030 for
/// names that fail to resolve. Used by `df.drop` and
/// `df.withColumnsRenamed`, which Spark designs to silently ignore
/// missing names rather than error.
pub(super) fn record_column_refs_tolerating_missing<'a>(
    refs: &[(&'a str, TextRange)],
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
) {
    for &(col_name, col_range) in refs {
        ctx.record_column_ref(col_range, col_name, schema.clone());
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
    // Backtick-aware split so `` `a.b` `` (one literal name) doesn't
    // get torn apart into an alias prefix, and `` `info`.`name` ``
    // (struct access) splits at the unquoted dot.
    let segments = crate::schema::split_backtick_aware(name);
    if segments.len() != 2 {
        return None;
    }
    let prefix = segments[0];
    let suffix = segments[1];
    if prefix.is_empty() || suffix.is_empty() {
        return None;
    }
    Some((prefix, suffix))
}

/// Apply `df.alias("L")`-style resolution to a single column ref. If
/// `col_name` is `"alias.suffix"` AND the prefix matches a registered
/// alias, route the lookup through the aliased schema (firing D0030
/// on the suffix typo). Returns `true` ONLY when the alias path
/// unambiguously owned the ref — callers then skip their normal
/// resolver. Returns `false` otherwise (not alias-shaped, no aliases
/// in scope, OR alias-shaped but prefix isn't a registered alias) so
/// the caller's nested-struct / single-name resolver can take over.
///
/// The "prefix-doesn't-match-any-alias → defer" branch is load-bearing:
/// a body containing both `X = orders.alias("X")` and `col("addr.city")`
/// must NOT have the alias helper hijack `addr.city`. `addr` isn't a
/// known alias, so the helper defers and the nested-struct resolver
/// (which knows `addr: struct<city: string>`) handles it correctly.
///
/// Shared between `report_column_refs` (the central col-ref check used
/// by select/filter/withColumn/groupBy/…) and `check_join_keys` (the
/// join expression-form on-clause) so every column-checking site
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
    let Some(aliased) = ctx.lookup_alias(prefix) else {
        return false;
    };
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
    true
}
