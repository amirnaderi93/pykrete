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
    // v1.10 PR-D1: Spark-only DataFrame attribute surface added to the
    // property table; mirrored here so the v1.10 PR-A1 tripwire
    // (SPARK_DISCRIMINATOR_PROPERTIES ⊂ SPARK_DISCRIMINATORS) holds.
    // Each carries an entry in NO_SUGGESTION_ALLOWLIST_SPARK; see
    // `operations/expr.rs` for rationale.
    "na",
    "write",
    "storageLevel",
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
    // v1.10 PR-D2 — pandas `stack` is a DataFrame method (wide-to-long
    // reshape). Spark's `stack` is `pyspark.sql.functions.stack`, a
    // column-level free function — NOT a DataFrame method — so the name
    // is pandas-discriminating on DataFrame receivers.
    "stack",
    // v1.11 PR-D1 — pandas `unstack` is the inverse of `stack` (long-to-
    // wide reshape, pivots an index level into the columns). Spark has
    // no DataFrame `unstack` method, so the name is pandas-
    // discriminating on DataFrame receivers.
    "unstack",
    // v1.10 PR-D1: pandas-only DataFrame attribute surface added to the
    // property table; mirrored here so the v1.10 PR-A1 tripwire
    // (PANDAS_INHERITED_PROPERTIES ⊂ PANDAS_ONLY_SIGNALS) holds.
    // Each carries an entry in NO_SUGGESTION_ALLOWLIST_PANDAS; see
    // `operations/expr.rs` for rationale.
    "index",
    "values",
    "shape",
    "T",
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

/// v1.9 PR-D2 — Spark-only DataFrame **property** names (not methods).
/// Used by the `Expr::Attribute` arm in
/// `operations::expr::analyze_expr` to fire D0091 when a Pandas-tagged
/// receiver accesses a Spark-only property (`pdf.rdd`,
/// `pdf.isStreaming`). The `Expr::Call` arm uses [`SPARK_DISCRIMINATORS`]
/// for the method case; this list is the property mirror.
///
/// These names also appear in [`SPARK_DISCRIMINATORS`] (the adjudicator
/// treats `rdd` / `isStreaming` / `sparkSession` as Spark-side signals
/// regardless of access shape). The duplication is deliberate: the
/// adjudicator scans every binding-downstream access; the D0091 arms
/// dispatch on AST shape. Bare-attribute access needs its own list so
/// adding a Spark-only property to the adjudicator doesn't silently fail
/// to fire D0091.
pub const SPARK_DISCRIMINATOR_PROPERTIES: &[&str] = &[
    "rdd",
    "isStreaming",
    "sparkSession",
    // v1.10 PR-D1 (closes v1.9 spark-I1): Spark-only DataFrame property
    // surface. Each is verified against the pandas DataFrame API as
    // having no same-spelled attribute (collision check in PR body).
    //   - `na`            DataFrameNaFunctions accessor (`sdf.na.fill(0)`).
    //                     Pandas uses `df.fillna(0)` directly; no `.na`
    //                     namespace.
    //   - `write`         DataFrameWriter accessor (`sdf.write.parquet(...)`).
    //                     Pandas uses top-level `df.to_parquet(...)`; no
    //                     `.write` namespace.
    //   - `writeStream`   Spark Structured Streaming writer. No pandas
    //                     equivalent (pandas has no streaming surface).
    //                     Already in `SPARK_DISCRIMINATORS` as a method-
    //                     style discriminator; bare-attribute path needed
    //                     its own entry.
    //   - `storageLevel`  Spark RDD persistence info (`sdf.storageLevel`).
    //                     No pandas analog (pandas is eager; no caching
    //                     surface).
    "na",
    "write",
    "writeStream",
    "storageLevel",
];

/// v1.9 PR-D2 — Pandas-only DataFrame **property/indexer** names. Used
/// by the `Expr::Attribute` arm in `operations::expr::analyze_expr` to
/// fire D0091 when a Spark-tagged receiver accesses a pandas-only
/// property (`sdf.loc`, `sdf.iloc`, `sdf.iat`, `sdf.at`). The
/// `Expr::Call` arm uses [`PANDAS_ONLY_SIGNALS`] for the method case;
/// this list is the property mirror.
///
/// These names also appear in [`PANDAS_ONLY_SIGNALS`]. Same rationale as
/// [`SPARK_DISCRIMINATOR_PROPERTIES`]: the adjudicator and the D0091
/// arms have separate dispatch shapes, so each maintains its own
/// inventory.
pub const PANDAS_INHERITED_PROPERTIES: &[&str] = &[
    "loc", "iloc", "iat", "at",
    // v1.10 PR-D1 (closes v1.9 spark-I2): pandas-only DataFrame property
    // surface. Each is verified against the Spark DataFrame API as
    // having no same-spelled attribute (collision check in PR body).
    //   - `index`   pandas Index object. Spark DataFrames have no row-
    //               index concept.
    //   - `values`  numpy-backed array of values. Spark uses `.collect()`
    //               which returns `list[Row]`.
    //   - `shape`   `(rows, cols)` tuple. Spark uses `.count()` +
    //               `len(df.columns)`.
    //   - `T`       transpose alias. Spark has no DataFrame transpose.
    "index", "values", "shape", "T",
];

// v1.8 PR-A1 — `PANDAS_INHERITED_ARM_METHODS_GENERATED` is produced by
// `crates/pykrete/build.rs` at compile time. The constant lives in
// `$OUT_DIR/pandas_inherited_arm_methods.rs` and lists every method
// name appearing in a `receiver_is_pandas_inherited &&` arm in
// `operations/expr.rs`. The hand-maintained inventory in
// `tests/v17_pr_a1_dialect_signals_guard.rs` checks against it for
// drift (closes v1.7 retro rule 4).
include!(concat!(env!("OUT_DIR"), "/pandas_inherited_arm_methods.rs"));
