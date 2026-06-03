//! Method-shape tables — argument-role classifiers for column methods
//! (`select`, `withColumn`, …), two-DataFrame methods (`union`, `join`,
//! …), terminal recognizers, and the `spark.read.*` opaque-source
//! recognizer.

use ruff_python_ast::{Expr, ExprCall};

#[derive(Debug, Clone, Copy)]
pub(super) enum ArgRole {
    ColumnName,
    Expression,
    NewName,
}

pub(super) enum ColumnMethodShape {
    AllColumnName,
    AllExpression,
    Positional(&'static [ArgRole]),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TwoDfMethod {
    Union,
    UnionByName,
    /// `intersect`, `intersectAll`, `subtract`, `exceptAll` — set
    /// operations that all return a DataFrame with the same schema as
    /// the receiver and require the other side's schema to match.
    /// (Spark's docs also reference `except`, but Python's `except`
    /// keyword makes that name unavailable as an attribute access —
    /// real PySpark code uses `exceptAll`/`subtract`.)
    /// We don't distinguish which one here because the check is the
    /// same shape (set equality on the column-name set) and the
    /// diagnostic message reads the same.
    SetOp,
    Join,
    CrossJoin,
}

pub(super) fn column_method_shape(method: &str) -> Option<ColumnMethodShape> {
    match method {
        "select" | "drop" | "dropDuplicates" | "drop_duplicates" | "groupBy" | "groupby"
        | "cube" | "rollup" => Some(ColumnMethodShape::AllColumnName),
        "filter" | "where" | "dropna" => Some(ColumnMethodShape::AllExpression),
        "withColumn" => Some(ColumnMethodShape::Positional(&[
            ArgRole::NewName,
            ArgRole::Expression,
        ])),
        "withColumnRenamed" => Some(ColumnMethodShape::Positional(&[
            ArgRole::ColumnName,
            ArgRole::NewName,
        ])),
        _ => None,
    }
}

pub(super) fn two_df_method(method: &str) -> Option<TwoDfMethod> {
    match method {
        // `unionAll` is a Spark 1.x deprecated alias for `union` — same
        // shape, same schema-mismatch check; treat it identically. The
        // D0040 message uses the alias name the user actually wrote.
        "union" | "unionAll" => Some(TwoDfMethod::Union),
        "unionByName" => Some(TwoDfMethod::UnionByName),
        "intersect" | "intersectAll" | "subtract" | "exceptAll" => Some(TwoDfMethod::SetOp),
        // v1.3 pandas spec §5: `.merge` is pandas' join shape. Same
        // dispatch — `on=` / `how=` mirror Spark `.join` argument
        // names. The on-key col-ref check uses the existing Join path.
        "join" | "merge" => Some(TwoDfMethod::Join),
        "crossJoin" => Some(TwoDfMethod::CrossJoin),
        _ => None,
    }
}

/// Methods that are typically the last step in a chain — they return
/// something other than a DataFrame (a row, a list of rows, a scalar,
/// None). Recognizing them centrally rather than letting them fall
/// through serves two purposes:
///
/// 1. The intent ("this is a terminal") is visible in the code.
/// 2. It's the natural seam to flag a chained call after a terminal
///    (almost always a bug) once that diagnostic lands — v0.1.16's
///    polish-pass work.
///
/// Today the behavior is the same as falling through: return `None`
/// so the chain dies cleanly. No new diagnostic.
pub(super) fn is_terminal_method(method: &str) -> bool {
    matches!(
        method,
        // → long
        "count"
        // → list of Row
        | "collect"
        | "take"
        | "tail"
        // → None
        | "show"
        | "printSchema"
        | "explain"
        // → Row
        | "first"
        | "head"
    )
}

/// Whether `method` is a `DataFrameReader` terminal that lands a
/// DataFrame (the `<format>` call at the end of a `spark.read.<format>(...)`
/// chain, or `.load(...)` from the builder form). The result schema is
/// genuinely runtime data — pykrete returns Unknown and expects the user
/// to re-anchor via `.cast(DataFrame[X])` or a typed variable annotation.
fn is_dataframe_reader_format(method: &str) -> bool {
    matches!(
        method,
        "parquet" | "csv" | "json" | "orc" | "text" | "table" | "load" | "jdbc" | "xml"
    )
}

/// Whether `expr` is a `DataFrameReader` — i.e. `<X>.read` or a chain of
/// recognized builder methods (`format`, `option`, `options`, `schema`) on
/// top of one. Match `<X>.read` (the base reader attribute) OR a chain of
/// recognized builder methods on top of one. We deliberately don't
/// restrict `<X>` to the literal `spark` name — codebases name their
/// session variable many ways (`spark`, `ss`, `sess`). As a side effect,
/// an unrelated `myloader.read.parquet(...)` API would also be
/// intercepted, but that just yields Unknown — same as falling through —
/// so no incorrect behavior surfaces.
fn is_dataframe_reader_expr(expr: &Expr) -> bool {
    match expr {
        // `<X>.read` — the base reader attribute.
        Expr::Attribute(a) => a.attr.id.as_str() == "read",
        // Builder methods chain reader → reader.
        Expr::Call(call) => {
            let Some(attr) = call.func.as_attribute_expr() else {
                return false;
            };
            matches!(
                attr.attr.id.as_str(),
                "format" | "option" | "options" | "schema"
            ) && is_dataframe_reader_expr(&attr.value)
        }
        _ => false,
    }
}

/// Whether this call is an opaque Spark IO source — a `DataFrameReader`
/// terminal (`spark.read.parquet(...)`, `spark.read.format(...).load(...)`,
/// …) or a bare `spark.table(...)` / `spark.read.table(...)`. These all
/// return a DataFrame whose schema can't be inferred statically; the user
/// re-anchors with `.cast(DataFrame[X])` or `name: DataFrame[X] = …`.
///
/// Recognized centrally rather than left to fall through the
/// DataFrame-receiver path as Unknown — keeps the intent visible, and is
/// the natural seam to emit a "re-anchor your chain" hint at when we
/// have an informational-severity track. Today we just return None; a
/// `TODO(spark-read-rehint)` placeholder is left for that polish pass.
pub(super) fn is_spark_opaque_source_call(call: &ExprCall) -> bool {
    let Some(attr) = call.func.as_attribute_expr() else {
        return false;
    };
    let method = attr.attr.id.as_str();
    // `spark.read.<format>(...)` / `spark.read.format(...).load(...)` /
    // `spark.read.schema(...).<format>(...)` — receiver is a reader.
    if is_dataframe_reader_format(method) && is_dataframe_reader_expr(&attr.value) {
        return true;
    }
    // `spark.table("db.x")` — SparkSession method that bypasses the reader.
    // Match `<X>.table(<args>)`. The check is structural — we don't verify
    // `<X>` is a SparkSession by lookup; in practice it's the session
    // variable, but a user calling `df.table(...)` (not a real DataFrame
    // method) would also match. Either way we return Unknown, so no
    // incorrect behavior surfaces.
    if method == "table" && attr.value.is_name_expr() {
        return true;
    }
    false
}

pub(super) fn role_at(shape: &ColumnMethodShape, index: usize) -> ArgRole {
    match shape {
        ColumnMethodShape::AllColumnName => ArgRole::ColumnName,
        ColumnMethodShape::AllExpression => ArgRole::Expression,
        ColumnMethodShape::Positional(roles) => roles
            .get(index)
            .copied()
            .or_else(|| roles.last().copied())
            .unwrap_or(ArgRole::Expression),
    }
}
