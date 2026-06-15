//! Shared vocabulary for dialect-aware analysis.
//!
//! v1.7 PR-A1 extract: prior to this module the checker carried THREE
//! separate sources of truth for "what is a pandas-side method":
//!
//! 1. `PANDAS_DISCRIMINATORS` in `alias_adjudicate.rs` — names that
//!    classify a `DataFrame[X]` binding as Pandas when the call-graph
//!    adjudicator sees them in the binding's downstream usage.
//! 2. `SPARK_DISCRIMINATORS` in `alias_adjudicate.rs` — the Spark
//!    counterpart, same role.
//! 3. Inline `matches!(method, "head" | "tail" | "first" | "take")`
//!    arms in `operations/expr.rs`, guarded by
//!    `receiver_is_pandas_inherited`, that dispatch shared-with-Spark
//!    methods through the pandas pass-through arm so a chain like
//!    `pdf.head().assign(...)` keeps its schema.
//!
//! The v1.6 architecture audit flagged this as a drift hazard
//! (Important #3): adding a new pandas-dispatched arm in `expr.rs` did
//! not force a corresponding update to the discriminator lists, and
//! vice versa. This module collects all three into one place and the
//! companion CI-guard test (`tests/v17_pr_a1_dialect_signals_guard.rs`)
//! enforces cross-consistency.
//!
//! # The semantic split is load-bearing
//!
//! `PANDAS_ONLY_SIGNALS` and `PANDAS_INHERITED_ARMS` are NOT the same
//! list. They serve different concerns and MUST stay disjoint:
//!
//! - **`PANDAS_ONLY_SIGNALS`** are pandas-only names — methods that
//!   exist on pandas but not Spark (or whose Spark counterpart is
//!   spelled camelCase: pandas's `rename` vs Spark's
//!   `withColumnRenamed`). Seeing one in a binding's downstream usage
//!   is sufficient to classify the binding as Pandas. If
//!   shared-with-Spark names like `head`/`drop` leaked in here, every
//!   Spark codebase that calls `df.head()` would suddenly classify as
//!   Pandas — a false positive on the D0090 adjudicator.
//!
//! - **`PANDAS_INHERITED_ARMS`** are shared-with-Spark names that have
//!   a pandas-specific dispatch arm in
//!   `operations::expr::analyze_method_call_inner`. The arm is gated
//!   on `receiver_is_pandas_inherited` (so a Spark receiver still
//!   routes to Spark semantics) and exists because pandas semantics
//!   differ: `pdf.head(5)` returns a row-sliced DataFrame, while
//!   Spark's `df.head(5)` is terminal (returns `list[Row]`). These
//!   names CANNOT classify a binding — they're ambiguous on their
//!   own.
//!
//! `SPARK_DISCRIMINATORS` is the Spark mirror of `PANDAS_ONLY_SIGNALS`
//! — Spark-only names. Lives here for symmetry; consumed only by
//! `alias_adjudicate.rs` today.

/// Spark-discriminating method/attribute names. A
/// `binding.METHOD(...)` call or `binding.ATTR` access where the
/// symbol is one of these tags the binding as Spark in the call-graph
/// adjudicator.
///
/// All names here MUST be Spark-only — pandas DataFrame must NOT
/// expose a same-spelled method/attribute. Notable collisions that are
/// explicitly EXCLUDED:
///
/// - `corr`, `cov`: pandas exposes both as DataFrame methods (return a
///   correlation/covariance matrix DataFrame). Spark's `df.corr(col1,
///   col2)` returns a scalar float; same spelling, different shape.
///   Including either would mis-classify a pandas binding as Spark.
/// - `crosstab`: pandas has `pd.crosstab(...)` as a top-level function
///   only, not a DataFrame method, so `df.crosstab(...)` IS Spark-only.
/// - `unpivot`: pandas uses `df.melt(...)`; `df.unpivot()` does not
///   exist on pandas DataFrame.
/// - `summary`: pandas uses `df.describe()`; `df.summary()` does not
///   exist on pandas DataFrame.
pub const SPARK_DISCRIMINATORS: &[&str] = &[
    "withColumn",
    "withColumns",
    "withColumnsRenamed",
    "withColumnRenamed",
    "createOrReplaceTempView",
    "createOrReplaceGlobalTempView",
    "createTempView",
    "createGlobalTempView",
    "repartition",
    "coalesce",
    "persist",
    "unpersist",
    "cache",
    "checkpoint",
    "printSchema",
    "toPandas",
    "show",
    "collect",
    "crossJoin",
    "unionByName",
    "subtract",
    "exceptAll",
    "intersectAll",
    "toDF",
    "sampleBy",
    "foreachPartition",
    // v1.7 PR-A2 (spark-D1 closure): Spark-only DataFrame surface that
    // pandas has no same-spelled method/attribute for. Each is
    // cross-checked against the pandas DataFrame API in the doc above.
    "selectExpr",
    "freqItems",
    "approxQuantile",
    "crosstab",
    "colRegex",
    "summary",
    "mapInPandas",
    "mapInArrow",
    "writeTo",
    "writeStream",
    "unpivot",
    "rdd",
    "isStreaming",
    "sparkSession",
];

/// Pandas-discriminating method and attribute names. A
/// `binding.METHOD(...)` or `binding.ATTR` access where the symbol is
/// one of these tags the binding as Pandas in the call-graph
/// adjudicator.
///
/// Names here MUST be pandas-only. Shared-with-Spark names like
/// `head`/`tail`/`first`/`take`/`drop` are deliberately excluded —
/// they live in [`PANDAS_INHERITED_ARMS`] instead.
///
/// Case sensitivity is load-bearing: Spark's analogues are camelCase
/// (`groupBy`, `withColumnRenamed`) and must not collapse into the
/// lowercase pandas names (`groupby`, `rename`).
pub const PANDAS_ONLY_SIGNALS: &[&str] = &[
    "assign",
    "pivot_table",
    "pivot",
    "melt",
    "merge",
    "applymap",
    "to_dict",
    "idxmax",
    "idxmin",
    "loc",
    "iloc",
    "iat",
    "at",
    "groupby",
    "rename",
    "query",
    "eval",
    "astype",
    "set_index",
    "reset_index",
    "value_counts",
    "nlargest",
    "nsmallest",
    "copy",
];

/// Shared-with-Spark method names that have a pandas-specific
/// dispatch arm in `operations::expr::analyze_method_call_inner`,
/// gated on `receiver_is_pandas_inherited`. The arm preserves the
/// schema through methods that are terminal on Spark but
/// schema-preserving on pandas (`head`/`tail`/`first`/`take` return a
/// row-sliced DataFrame on pandas; `drop` here is the pandas-spelling
/// fall-through that prevents Spark's column-drop semantics from
/// silently erasing a column on a pandas receiver).
///
/// Names here MUST overlap with Spark by definition — that's why the
/// dispatch is dialect-gated. Pandas-only names go in
/// [`PANDAS_ONLY_SIGNALS`].
pub const PANDAS_INHERITED_ARMS: &[&str] = &["head", "tail", "first", "take", "drop"];
