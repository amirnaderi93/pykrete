use std::collections::HashSet;

use super::col_refs::{col_reference, collect_col_refs};
use super::column_exprs::{infer_expr_type, report_get_field_typo, select_arg_type};
use super::context::{BodyContext, TypeCtx};
use super::shapes::ArgRole;
use super::two_df::strip_nullability;

use ruff_python_ast::{CmpOp, Expr, ExprCall, Operator};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::diagnostics::{CheckMode, Diagnostic, Severity};
use crate::schema::{DerivedField, Schema, SchemaView, suggest_field_name};
use crate::types::{COLUMN_TYPE_NAMES, ColumnType};

// ---------------------------------------------------------------------------
// Strict-mode operator type checks (D0081 / D0082)
//
// These flag type combinations that Spark *coerces* rather than rejects —
// legal, but usually a mistake. Because the coercion is legal, they would
// be too noisy on by default; they are tagged `min_mode: Strict` so the
// driver only surfaces them under `typeCheckingMode: "strict"`.
// ---------------------------------------------------------------------------

/// Family of types that behave alike under operators.
#[derive(PartialEq, Clone, Copy)]
enum TypeFamily {
    Numeric,
    Textual,
    Boolean,
    Temporal,
    /// `array` / `map` — collections; they don't combine with atomics.
    Collection,
}

fn type_family(t: &ColumnType) -> TypeFamily {
    match t {
        ColumnType::Int
        | ColumnType::Long
        | ColumnType::Double
        | ColumnType::Byte
        | ColumnType::Short
        | ColumnType::Decimal { .. } => TypeFamily::Numeric,
        // `binary` doesn't fit cleanly into any family — Spark won't
        // arithmetic on it, compare it with strings, etc. Group it with
        // collections (the catch-all for non-combining atomics) so the
        // strict checks treat it as opaque.
        ColumnType::String => TypeFamily::Textual,
        ColumnType::Binary => TypeFamily::Collection,
        ColumnType::Bool => TypeFamily::Boolean,
        ColumnType::Date | ColumnType::Timestamp => TypeFamily::Temporal,
        ColumnType::Array(_) | ColumnType::Map(..) | ColumnType::Struct(_) => {
            TypeFamily::Collection
        }
        // Nullability doesn't change the family — `Optional[int]` is
        // still numeric.
        ColumnType::Nullable(inner) => type_family(inner),
    }
}

/// Whether two atomic types may sensibly be compared. Same family is
/// always fine; a string-vs-temporal comparison is allowed because Spark
/// idiomatically casts the string (`col("date") > "2024-01-01"`).
fn comparable(a: &ColumnType, b: &ColumnType) -> bool {
    let (fa, fb) = (type_family(a), type_family(b));
    fa == fb
        || matches!(
            (fa, fb),
            (TypeFamily::Textual, TypeFamily::Temporal)
                | (TypeFamily::Temporal, TypeFamily::Textual)
        )
}

fn is_arithmetic_op(op: Operator) -> bool {
    matches!(
        op,
        Operator::Add
            | Operator::Sub
            | Operator::Mult
            | Operator::Div
            | Operator::FloorDiv
            | Operator::Mod
            | Operator::Pow
    )
}

fn is_value_comparison(op: CmpOp) -> bool {
    matches!(
        op,
        CmpOp::Eq | CmpOp::NotEq | CmpOp::Lt | CmpOp::LtE | CmpOp::Gt | CmpOp::GtE
    )
}

/// Walk a column expression for strict-mode operator type errors:
/// arithmetic on a string column (`D0081`) and comparisons between
/// unrelated atomic types (`D0082`). Both are emitted at
/// [`CheckMode::Strict`] — only surfaced under `typeCheckingMode: strict`.
pub(super) fn report_expr_type_errors<'a>(
    expr: &Expr,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::BinOp(b) => {
            if is_arithmetic_op(b.op) {
                // A string / array / map operand can't take part in
                // arithmetic — Spark coerces a string (often to null) and
                // errors on a collection.
                let bad = [
                    infer_expr_type(&b.left, schema, tcx),
                    infer_expr_type(&b.right, schema, tcx),
                ]
                .into_iter()
                .flatten()
                .find(|t| {
                    matches!(
                        t,
                        ColumnType::String
                            | ColumnType::Array(_)
                            | ColumnType::Map(..)
                            | ColumnType::Struct(_)
                    )
                });
                if let Some(bad) = bad {
                    diagnostics.push(
                        Diagnostic::at_range(
                            Severity::Warning,
                            "D0081",
                            format!(
                                "Arithmetic operator applied to a non-numeric ({bad}) column. \
                                 Spark coerces or errors here; cast it explicitly if intended.",
                            ),
                            b.range(),
                            source,
                            line_index,
                        )
                        .with_min_mode(CheckMode::Strict),
                    );
                }
            }
            report_expr_type_errors(&b.left, schema, tcx, source, line_index, diagnostics);
            report_expr_type_errors(&b.right, schema, tcx, source, line_index, diagnostics);
        }
        Expr::Compare(c) => {
            let mut left = c.left.as_ref();
            for (op, right) in c.ops.iter().zip(&c.comparators) {
                if is_value_comparison(*op)
                    && let (Some(lt), Some(rt)) = (
                        infer_expr_type(left, schema, tcx),
                        infer_expr_type(right, schema, tcx),
                    )
                    && !comparable(&lt, &rt)
                {
                    diagnostics.push(
                        Diagnostic::at_range(
                            Severity::Warning,
                            "D0082",
                            format!(
                                "Comparison between unrelated types {lt} and {rt}. \
                                         Spark coerces them; cast explicitly if intended.",
                            ),
                            c.range(),
                            source,
                            line_index,
                        )
                        .with_min_mode(CheckMode::Strict),
                    );
                }
                left = right;
            }
            report_expr_type_errors(&c.left, schema, tcx, source, line_index, diagnostics);
            for cmp in &c.comparators {
                report_expr_type_errors(cmp, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::Call(call) => {
            // `<col>.getField("name")` — flag a typo on the field name
            // against the receiver's struct type. Stays quiet on a
            // non-struct receiver or a non-literal name (computed access
            // pykrete can't verify).
            if let Some(attr) = call.func.as_attribute_expr()
                && attr.attr.id.as_str() == "getField"
                && let Some(lit) = call
                    .arguments
                    .args
                    .first()
                    .and_then(Expr::as_string_literal_expr)
                && let Some(recv_ty) = infer_expr_type(&attr.value, schema, tcx)
            {
                report_get_field_typo(&recv_ty, lit, source, line_index, diagnostics);
            }
            // `<struct-col>.dropFields("a", "b", …)` — flag each literal
            // string arg that isn't a field of the receiver struct.
            // Symmetric to the `.getField` typo handling above; a silent
            // miss here means the user thinks they removed a field and
            // didn't (Spark 3.4+ matches that behavior at runtime, so it's
            // a genuine footgun).
            if let Some(attr) = call.func.as_attribute_expr()
                && attr.attr.id.as_str() == "dropFields"
                && let Some(recv_ty) = infer_expr_type(&attr.value, schema, tcx)
            {
                for arg in &call.arguments.args {
                    if let Some(lit) = arg.as_string_literal_expr() {
                        report_get_field_typo(&recv_ty, lit, source, line_index, diagnostics);
                    }
                }
            }
            // `<col>.cast("string-literal")` — flag a target that doesn't
            // parse as a Spark type. Without this, a typo like
            // `.cast("decimial(18,2)")` is silently accepted (pykrete
            // can't pin the result, but never warns either). Restricted
            // to string literals so the type-constructor form
            // (`.cast(IntegerType())`) and any computed expression stay
            // permissive. Targets that are real Spark types pykrete
            // hasn't modeled yet (`varchar(n)`, `timestamp_ntz`, …)
            // fall through silently via `is_unmodeled_spark_type` — a
            // false D0011 on legitimate code would be a v0.1.27 regression.
            if let Some(attr) = call.func.as_attribute_expr()
                && attr.attr.id.as_str() == "cast"
                && let Some(lit) = call
                    .arguments
                    .args
                    .first()
                    .and_then(Expr::as_string_literal_expr)
            {
                let target = lit.value.to_str();
                if ColumnType::from_spark_name(target).is_none()
                    && !ColumnType::is_unmodeled_spark_type(target)
                {
                    diagnostics.push(Diagnostic::at_range(
                        Severity::Error,
                        "D0011",
                        format!(
                            "Cast target '{target}' is not a recognized Spark type. \
                             Expected one of: {COLUMN_TYPE_NAMES}.",
                        ),
                        lit.range(),
                        source,
                        line_index,
                    ));
                }
            }
            report_expr_type_errors(&call.func, schema, tcx, source, line_index, diagnostics);
            for arg in &call.arguments.args {
                report_expr_type_errors(arg, schema, tcx, source, line_index, diagnostics);
            }
            for kw in &call.arguments.keywords {
                report_expr_type_errors(&kw.value, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::Attribute(a) => {
            report_expr_type_errors(&a.value, schema, tcx, source, line_index, diagnostics);
        }
        Expr::UnaryOp(u) => {
            report_expr_type_errors(&u.operand, schema, tcx, source, line_index, diagnostics);
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                report_expr_type_errors(v, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::If(if_exp) => {
            report_expr_type_errors(&if_exp.test, schema, tcx, source, line_index, diagnostics);
            report_expr_type_errors(&if_exp.body, schema, tcx, source, line_index, diagnostics);
            report_expr_type_errors(&if_exp.orelse, schema, tcx, source, line_index, diagnostics);
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                report_expr_type_errors(e, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                report_expr_type_errors(e, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::Subscript(s) => {
            report_expr_type_errors(&s.value, schema, tcx, source, line_index, diagnostics);
        }
        Expr::Starred(s) => {
            report_expr_type_errors(&s.value, schema, tcx, source, line_index, diagnostics);
        }
        _ => {}
    }
}

pub(super) fn apply_column_method<'a>(
    method: &str,
    recv: &SchemaView<'a>,
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    tcx: TypeCtx<'a>,
) -> Option<SchemaView<'a>> {
    match method {
        "select" => {
            let mut fields: Vec<DerivedField<'a>> = Vec::new();
            for arg in &call.arguments.args {
                // `select("*")` — the star expands to every column of the
                // receiver, rather than naming a literal column `*`.
                if arg
                    .as_string_literal_expr()
                    .is_some_and(|s| s.value.to_str() == "*")
                {
                    fields.extend(recv.typed_fields(tcx.schemas));
                    continue;
                }
                if let Some(pair) = posexplode_fields(arg, recv, tcx) {
                    fields.extend(pair);
                    continue;
                }
                if let Some(pair) = explode_map_aliased_fields(arg, recv, tcx) {
                    fields.extend(pair);
                    continue;
                }
                if let Some(name) = select_output_name(arg, Some(ctx)) {
                    fields.push(DerivedField {
                        name,
                        ty: select_arg_type(arg, recv, tcx),
                    });
                }
            }
            Some(SchemaView::Derived(fields))
        }
        "filter" | "where" | "dropDuplicates" | "drop_duplicates" => Some(recv.clone()),
        // `dropna` drops rows containing nulls — the surviving rows have
        // none, so the columns are no longer nullable.
        "dropna" => Some(strip_nullability(recv, tcx.schemas)),
        "drop" => {
            let drop_set: HashSet<&str> = call
                .arguments
                .args
                .iter()
                .filter_map(column_name_arg)
                .collect();
            let remaining: Vec<DerivedField<'a>> = recv
                .typed_fields(tcx.schemas)
                .into_iter()
                .filter(|f| !drop_set.contains(f.name))
                .collect();
            Some(SchemaView::Derived(remaining))
        }
        "withColumn" => {
            let new_name = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
                .map(|s| s.value.to_str())?;
            // The new column's type is inferred from the value
            // expression; if `new_name` already exists, `withColumn`
            // *replaces* it, so its type is updated rather than kept.
            let ty = call
                .arguments
                .args
                .get(1)
                .and_then(|v| infer_expr_type(v, recv, tcx));
            let mut fields: Vec<DerivedField<'a>> = recv.typed_fields(tcx.schemas);
            if let Some(existing) = fields.iter_mut().find(|f| f.name == new_name) {
                existing.ty = ty;
            } else {
                fields.push(DerivedField { name: new_name, ty });
            }
            Some(SchemaView::Derived(fields))
        }
        "withColumnRenamed" => {
            let old = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
                .map(|s| s.value.to_str())?;
            let new = call
                .arguments
                .args
                .get(1)
                .and_then(|a| a.as_string_literal_expr())
                .map(|s| s.value.to_str())?;
            let fields: Vec<DerivedField<'a>> = recv
                .typed_fields(tcx.schemas)
                .into_iter()
                .map(|mut f| {
                    if f.name == old {
                        f.name = new;
                    }
                    f
                })
                .collect();
            Some(SchemaView::Derived(fields))
        }
        "groupBy" | "groupby" | "cube" | "rollup" => {
            // `groupby` is the lowercase Spark alias of `groupBy` —
            // identical semantics; PySpark accepts both and a lot of
            // real-world code (e.g. examples/src/main/python/sql/
            // arrow.py) uses the lowercase form. None of these return
            // a DataFrame; they return a GroupedData that captures
            // the group keys and remembers the input schema. The
            // follow-up .agg(...) call uses that to check its column
            // references and produce the final DataFrame schema.
            // `cube` and `rollup` differ from `groupBy` only in which
            // subtotal rows they emit — irrelevant to the column schema.
            let mut keys: Vec<&'a str> = Vec::new();
            for arg in &call.arguments.args {
                if let Some(name) = column_name_arg(arg) {
                    keys.push(name);
                }
            }
            Some(SchemaView::Grouped {
                keys,
                underlying: Box::new(recv.clone()),
                after_pivot: false,
            })
        }
        _ => None,
    }
}

/// Model the result schema of a `df.selectExpr("…", "…")` call. Each
/// argument is a SQL expression string; the output schema is the list
/// of their result column names. `*` expands to the receiver's columns.
///
/// Returns `None` only when an argument isn't a string literal (a
/// computed expression list) — there the result schema is genuinely
/// unknowable. Checking the column references *inside* the SQL is a
/// follow-up (needs a SQL parse); this only recovers the output shape
/// so a chain doesn't die at `.selectExpr(...)`.
pub(super) fn apply_select_expr<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    schemas: &'a [Schema<'a>],
) -> Option<SchemaView<'a>> {
    let mut fields: Vec<DerivedField<'a>> = Vec::new();
    for arg in &call.arguments.args {
        let item = arg.as_string_literal_expr()?.value.to_str().trim();
        if item == "*" {
            fields.extend(recv.typed_fields(schemas));
        } else {
            let name = select_expr_output_name(item);
            // A bare identifier item (no `AS`, no operators) is a plain
            // column reference — typed from the receiver. Anything else
            // is a computed SQL expression whose type pykrete leaves open.
            let ty = if split_sql_alias(item).is_none()
                && !item.is_empty()
                && item.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                recv.field_type(name, schemas)
            } else {
                None
            };
            fields.push(DerivedField { name, ty });
        }
    }
    Some(SchemaView::Derived(fields))
}

/// The result column name of one `selectExpr` item: the `AS` alias if
/// present, else the last segment of a bare (dotted) identifier, else
/// the expression text verbatim (Spark auto-names it after the expr).
fn select_expr_output_name(item: &str) -> &str {
    if let Some(alias) = split_sql_alias(item) {
        return alias;
    }
    if !item.is_empty()
        && item
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return item.rsplit('.').next().unwrap_or(item);
    }
    item
}

/// The alias of a SQL `expr AS name` item, if it has one. Matches the
/// last ` as ` case-insensitively and strips quoting around the name.
fn split_sql_alias(item: &str) -> Option<&str> {
    let idx = item.to_ascii_lowercase().rfind(" as ")?;
    let alias = item[idx + 4..].trim().trim_matches(['`', '"', '\'']);
    (!alias.is_empty()).then_some(alias)
}

/// Resolve `spark.sql(query)` against the file's tempView registry.
///
/// Two paths:
///
/// 1. **View hit.** The query's single FROM table is a registered
///    tempView. Every column identifier in the query (projection,
///    WHERE, GROUP BY, ORDER BY, HAVING) is checked against the view's
///    schema — typos fire `D0030`. The result schema is the projection
///    (`SELECT a, b`) when pykrete can read it, or the whole view
///    (`SELECT *`) otherwise. If the projection has an unaliased
///    computed column the result degrades to `None`, same as the
///    no-view fallback.
///
/// 2. **No view (or unknown view).** Best-effort fallback: infer the
///    result schema from the projection columns alone — what pykrete
///    did before tempView resolution existed. No column-existence
///    checks happen, because there's no schema to check against.
///
/// `range` is the source range of the SQL string literal; D0030
/// diagnostics anchor there.
pub(super) fn resolve_spark_sql_call<'a>(
    query: &'a str,
    range: TextRange,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let view_schema = ctx
        .temp_views()
        .and_then(|tv| crate::sql::single_from_table(query).and_then(|name| tv.lookup(&name)));

    if let Some(view) = view_schema {
        // Every identifier in the query is checked against the view —
        // `query_column_refs` walks the whole AST, so WHERE / GROUP BY /
        // ORDER BY / HAVING references come along for the ride.
        for name in crate::sql::query_column_refs(query) {
            if view.has_field(&name) {
                continue;
            }
            let suggestion = suggest_field_name(&name, &view);
            let mut message =
                format!("Column '{name}' does not exist on {}.", view.display_name(),);
            if let Some(s) = &suggestion {
                message.push_str(&format!(" Did you mean '{s}'?"));
            }
            diagnostics.push(
                Diagnostic::at_range(Severity::Error, "D0030", message, range, source, line_index)
                    .with_suggestion(suggestion),
            );
        }
        // `SELECT *` against a known view returns the view's columns
        // directly. Otherwise read the projection.
        return Some(match crate::sql::select_projection_columns(query) {
            Some(cols) => SchemaView::derived_untyped(cols),
            None => view,
        });
    }

    // No registered view — fall back to the pre-tempView behavior:
    // infer the result schema from the projection columns, with no
    // column-existence checks (there's no schema to check against).
    crate::sql::select_projection_columns(query).map(SchemaView::derived_untyped)
}

/// Check the column identifiers inside a SQL expression string against
/// `schema`, emitting a `D0030` for every name that doesn't resolve.
///
/// Used for `selectExpr("…")` items and string-form `filter("…")` /
/// `where("…")` predicates — the places where Spark accepts a SQL
/// fragment in lieu of a `Column` expression. The fragment is parsed
/// best-effort by [`crate::sql::column_refs`]; an unparseable one yields
/// no references rather than a spurious error.
///
/// Diagnostics anchor on `range` — the whole string literal — rather
/// than the offset of the offending identifier: the parsed names are
/// owned `String`s with no span back into the original source.
pub(super) fn report_sql_column_refs(
    sql: &str,
    range: TextRange,
    schema: &SchemaView<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for name in crate::sql::column_refs(sql) {
        if schema.has_field(&name) {
            continue;
        }
        let suggestion = suggest_field_name(&name, schema);
        let mut message = format!(
            "Column '{name}' does not exist on {}.",
            schema.display_name(),
        );
        if let Some(s) = &suggestion {
            message.push_str(&format!(" Did you mean '{s}'?"));
        }
        diagnostics.push(
            Diagnostic::at_range(Severity::Error, "D0030", message, range, source, line_index)
                .with_suggestion(suggestion),
        );
    }
}

/// Walk a column expression for `F.expr("…")` / `expr("…")` calls and
/// SQL-check the fragment inside each against `schema`.
///
/// `F.expr(...)` wraps a SQL string as a `Column`, so it can appear
/// anywhere a column expression is expected — `select`, `filter`,
/// `withColumn`, `agg`, and nested inside `F.when(...)`. The walk mirrors
/// [`collect_col_refs`] so a deeply nested `expr(...)` is still reached.
///
/// This is the sibling of [`collect_col_refs`] for the SQL-string family:
/// the latter collects borrowed `col("…")` names, but `expr(...)` names
/// come from a SQL parse as owned `String`s and so are checked (and
/// reported) on the spot rather than collected.
pub(super) fn report_expr_sql_refs(
    expr: &Expr,
    schema: &SchemaView<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(call) = expr.as_call_expr() {
        let func_name = match call.func.as_ref() {
            Expr::Name(n) => Some(n.id.as_str()),
            Expr::Attribute(a) => Some(a.attr.id.as_str()),
            _ => None,
        };
        if func_name == Some("expr")
            && let Some(lit) = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
        {
            report_sql_column_refs(
                lit.value.to_str(),
                lit.range(),
                schema,
                source,
                line_index,
                diagnostics,
            );
        }
    }
    match expr {
        Expr::Call(c) => {
            report_expr_sql_refs(&c.func, schema, source, line_index, diagnostics);
            for arg in &c.arguments.args {
                report_expr_sql_refs(arg, schema, source, line_index, diagnostics);
            }
            for kw in &c.arguments.keywords {
                report_expr_sql_refs(&kw.value, schema, source, line_index, diagnostics);
            }
        }
        Expr::Attribute(a) => {
            report_expr_sql_refs(&a.value, schema, source, line_index, diagnostics);
        }
        Expr::Subscript(s) => {
            report_expr_sql_refs(&s.value, schema, source, line_index, diagnostics);
            report_expr_sql_refs(&s.slice, schema, source, line_index, diagnostics);
        }
        Expr::BinOp(b) => {
            report_expr_sql_refs(&b.left, schema, source, line_index, diagnostics);
            report_expr_sql_refs(&b.right, schema, source, line_index, diagnostics);
        }
        Expr::UnaryOp(u) => {
            report_expr_sql_refs(&u.operand, schema, source, line_index, diagnostics);
        }
        Expr::Compare(c) => {
            report_expr_sql_refs(&c.left, schema, source, line_index, diagnostics);
            for cmp in &c.comparators {
                report_expr_sql_refs(cmp, schema, source, line_index, diagnostics);
            }
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                report_expr_sql_refs(v, schema, source, line_index, diagnostics);
            }
        }
        Expr::If(if_exp) => {
            report_expr_sql_refs(&if_exp.test, schema, source, line_index, diagnostics);
            report_expr_sql_refs(&if_exp.body, schema, source, line_index, diagnostics);
            report_expr_sql_refs(&if_exp.orelse, schema, source, line_index, diagnostics);
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                report_expr_sql_refs(e, schema, source, line_index, diagnostics);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                report_expr_sql_refs(e, schema, source, line_index, diagnostics);
            }
        }
        Expr::Starred(s) => {
            report_expr_sql_refs(&s.value, schema, source, line_index, diagnostics);
        }
        _ => {}
    }
}

pub(super) fn select_output_name<'a>(
    arg: &'a Expr,
    ctx: Option<&BodyContext<'a>>,
) -> Option<&'a str> {
    if let Some(call) = arg.as_call_expr()
        && let Some(attr) = call.func.as_attribute_expr()
        && attr.attr.id.as_str() == "alias"
        && let Some(lit) = call
            .arguments
            .args
            .first()
            .and_then(|a| a.as_string_literal_expr())
    {
        return Some(lit.value.to_str());
    }
    if let Some(s) = arg.as_string_literal_expr() {
        return Some(s.value.to_str());
    }
    if let Some((name, _)) = col_reference(arg) {
        return Some(name);
    }
    // `df.X` — attribute-access column ref. Spark projects this as a
    // column named `X` in the output schema, so a select arg like
    // `lines.timestamp` carries `timestamp` through. Gate on the
    // receiver being a DataFrame in scope when ctx is available —
    // otherwise `helper.foo` (non-DF) would be misread as a column ref
    // and silently leak into the inferred output schema.
    if let Some(attr) = arg.as_attribute_expr()
        && let Some(name) = attr.value.as_name_expr()
        && ctx.is_none_or(|c| c.lookup(name.id.as_str()).is_some())
    {
        return Some(attr.attr.id.as_str());
    }
    // `df["X"]` — subscript sibling of the attribute form above. Same
    // ctx-gating so `helper["foo"]` doesn't sneak through.
    if let Some(sub) = arg.as_subscript_expr()
        && let Some(name) = sub.value.as_name_expr()
        && let Some(s) = sub.slice.as_string_literal_expr()
        && ctx.is_none_or(|c| c.lookup(name.id.as_str()).is_some())
    {
        return Some(s.value.to_str());
    }
    if let Some(call) = arg.as_call_expr() {
        if let Some(attr) = call.func.as_attribute_expr()
            && attr.attr.id.as_str() == "cast"
        {
            return select_output_name(&attr.value, ctx);
        }
        // `F.explode("arr")` / `explode_outer(...)` — Spark names the
        // unnested column `col` when no `.alias(...)` is given.
        let fname = match call.func.as_ref() {
            Expr::Name(n) => Some(n.id.as_str()),
            Expr::Attribute(a) => Some(a.attr.id.as_str()),
            _ => None,
        };
        if matches!(fname, Some("explode" | "explode_outer")) {
            return Some("col");
        }
    }
    None
}

/// Resolve the column type of an `F.explode(<arg>)` payload — `<arg>`
/// may be a string literal (`"map_col"`), a `col("map_col")` /
/// `F.col(...)` ref, OR an attribute/subscript-style ref
/// (`data.map_col`, `data["map_col"]`). The last two shapes are common
/// in real PySpark code but `infer_expr_type` doesn't bottom them out
/// to the receiver field, so this helper steps in.
fn explode_map_arg_type<'a>(
    arg: &Expr,
    recv: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
) -> Option<ColumnType> {
    if let Some(ty) = select_arg_type(arg, recv, tcx) {
        return Some(ty);
    }
    if let Some(attr) = arg.as_attribute_expr()
        && attr.value.is_name_expr()
    {
        return recv.field_type(attr.attr.id.as_str(), tcx.schemas);
    }
    if let Some(sub) = arg.as_subscript_expr()
        && sub.value.is_name_expr()
        && let Some(s) = sub.slice.as_string_literal_expr()
    {
        return recv.field_type(s.value.to_str(), tcx.schemas);
    }
    None
}

/// Returns `Some` if `arg` is `F.explode(map_col).alias(k_name, v_name)`
/// — a two-arg alias on an explode of a Map column. Spark expands this
/// to TWO columns named per the alias args, typed as the map's key and
/// value. A single-arg alias on a map explode is a runtime error in
/// Spark; this helper bails on it and lets `select_output_name` produce
/// the one-name interpretation.
pub(super) fn explode_map_aliased_fields<'a>(
    arg: &'a Expr,
    recv: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
) -> Option<Vec<DerivedField<'a>>> {
    let outer = arg.as_call_expr()?;
    let outer_attr = outer.func.as_attribute_expr()?;
    if outer_attr.attr.id.as_str() != "alias" {
        return None;
    }
    // Two string-literal positional args to .alias(...) — Spark's
    // signature for naming a (key, value) pair.
    if outer.arguments.args.len() != 2 {
        return None;
    }
    let key_name = outer.arguments.args[0]
        .as_string_literal_expr()?
        .value
        .to_str();
    let val_name = outer.arguments.args[1]
        .as_string_literal_expr()?
        .value
        .to_str();
    let inner = outer_attr.value.as_call_expr()?;
    let inner_fname = match inner.func.as_ref() {
        Expr::Name(n) => n.id.as_str(),
        Expr::Attribute(a) => a.attr.id.as_str(),
        _ => return None,
    };
    if !matches!(inner_fname, "explode" | "explode_outer") {
        return None;
    }
    let map_arg = inner.arguments.args.first()?;
    let map_ty = explode_map_arg_type(map_arg, recv, tcx)?;
    // Peel `Optional[Map<...>]` so the outer-explode nullability
    // wrapper doesn't hide the underlying map.
    let base = match &map_ty {
        ColumnType::Nullable(inner) => inner.as_ref(),
        other => other,
    };
    let ColumnType::Map(key_ty, val_ty) = base else {
        return None;
    };
    let key_ty = key_ty.as_deref().cloned();
    let val_ty = val_ty.as_deref().cloned();
    Some(vec![
        DerivedField {
            name: key_name,
            ty: key_ty,
        },
        DerivedField {
            name: val_name,
            ty: val_ty,
        },
    ])
}

/// Returns `Some` if `arg` is a bare `F.posexplode(arr)` /
/// `posexplode_outer(arr)` call (not wrapped in `.alias(...)`), in which
/// case the call expands to TWO columns: `pos: int` and `col: <element>`.
///
/// An explicit `.alias(...)` on the call falls through here — Spark
/// interprets `posexplode(arr).alias("p", "c")` as renaming the pair
/// (multi-name alias), but the single-name `.alias("x")` form is a
/// runtime error. Pykrete chooses to leave any aliased posexplode to
/// `select_output_name` (which returns one name) rather than guess.
pub(super) fn posexplode_fields<'a>(
    arg: &Expr,
    recv: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
) -> Option<Vec<DerivedField<'a>>> {
    let call = arg.as_call_expr()?;
    let fname = match call.func.as_ref() {
        Expr::Name(n) => n.id.as_str(),
        Expr::Attribute(a) => a.attr.id.as_str(),
        _ => return None,
    };
    if !matches!(fname, "posexplode" | "posexplode_outer") {
        return None;
    }
    let first_arg = call
        .arguments
        .args
        .first()
        .and_then(|a| select_arg_type(a, recv, tcx));
    let elem_ty = match first_arg {
        Some(ColumnType::Array(elem)) => elem.map(|b| *b),
        _ => None,
    };
    Some(vec![
        DerivedField {
            name: "pos",
            ty: Some(ColumnType::Int),
        },
        DerivedField {
            name: "col",
            ty: elem_ty,
        },
    ])
}

fn column_name_arg(arg: &Expr) -> Option<&str> {
    if let Some(s) = arg.as_string_literal_expr() {
        return Some(s.value.to_str());
    }
    if let Some((name, _)) = col_reference(arg) {
        return Some(name);
    }
    // `df.colname` attribute access — `groupBy(df.key, ...)` and
    // `drop(df.col)` both accept Column objects, not just name strings,
    // and grouping by a column carried in from a joined DataFrame is
    // idiomatic. The key is the attribute name; which DataFrame it came
    // from is irrelevant to the resulting column. Restricted to a bare
    // `Name` base so a called `F.func(...)` or a chained `a.b.c` can't be
    // mistaken for a column reference.
    if let Some(attr) = arg.as_attribute_expr()
        && attr.value.is_name_expr()
    {
        return Some(attr.attr.id.as_str());
    }
    // `df["colname"]` subscript — sibling of the attribute form above.
    // Same rationale: `drop(df["col"])`, `groupBy(df["key"], ...)` accept
    // Column objects. Restricted to `df["literal"]` (bare-name receiver,
    // string-literal slice).
    if let Some(sub) = arg.as_subscript_expr()
        && sub.value.is_name_expr()
        && let Some(s) = sub.slice.as_string_literal_expr()
    {
        return Some(s.value.to_str());
    }
    None
}

/// Collect refs from a method-call arg, partitioned into two buckets:
/// `string_out` for string-literal column names (`df.drop("x")`,
/// `df.drop(["x", "y"])`) and `column_out` for everything else
/// (`F.col("x")`, `df.x`, `df["x"]`, `F.sum("x")`, …). Callers that
/// need to apply different policies to string-form vs Column-form refs
/// use the partitioning — `df.drop` and `df.withColumnsRenamed`
/// tolerate missing names only in the string form.
pub(super) fn collect_arg_column_refs_split<'a>(
    arg: &'a Expr,
    role: ArgRole,
    ctx: &BodyContext<'a>,
    string_out: &mut Vec<(&'a str, TextRange)>,
    column_out: &mut Vec<(&'a str, TextRange)>,
) {
    match role {
        ArgRole::NewName => {}
        ArgRole::ColumnName => {
            // `"*"` is the all-columns wildcard, not a literal column
            // name — never check or collect it as a reference.
            if let Some(s) = arg.as_string_literal_expr() {
                if s.value.to_str() != "*" {
                    string_out.push((s.value.to_str(), s.range()));
                }
                return;
            }
            if let Some(list) = arg.as_list_expr() {
                for elt in &list.elts {
                    if let Some(s) = elt.as_string_literal_expr() {
                        if s.value.to_str() != "*" {
                            string_out.push((s.value.to_str(), s.range()));
                        }
                    } else {
                        collect_col_refs(elt, ctx, column_out);
                    }
                }
                return;
            }
            collect_col_refs(arg, ctx, column_out);
        }
        ArgRole::Expression => collect_col_refs(arg, ctx, column_out),
    }
}
