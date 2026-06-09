//! PySpark DataFrame operations — checking calls inside function bodies, with
//! result-schema inference.
//!
//! ## Recursive expression analysis
//!
//! `analyze_expr` (in [`expr`]) walks an expression that *might* evaluate
//! to a DataFrame. It returns a [`SchemaView`](crate::schema::SchemaView)
//! when it can determine the value's schema, and `None` otherwise. Along
//! the way, every recognized method call has its arguments checked against
//! the receiver's schema and emits diagnostics when something is wrong.
//!
//! The recursion is what enables chained-call support — for
//! `raw.filter(col("a") > 0).select("madeup")`, `analyze_expr` on the outer
//! `select` first analyzes its receiver (the `filter` call), which in turn
//! analyzes *its* receiver (`raw`). Each level reports diagnostics and
//! returns its result schema.
//!
//! ## Bindings
//!
//! `BodyContext` (in [`context`]) is a name → schema map. It starts populated
//! from typed function parameters, and grows as assignments produce new
//! schemas. This turns `x = raw.select("a"); x.select("b")` into a checkable
//! chain.
//!
//! ## Return-type validation
//!
//! For each `return <value>` statement, the value's inferred schema is
//! compared to the function's declared return schema (`-> DataFrame[X]`).
//! Mismatches emit `D0050`.
//!
//! ## Module layout
//!
//! The implementation is split into siblings along the historical section
//! banners. Each sibling owns a self-contained slice; cross-section
//! helpers are reached via `super::other_module::Item`.
//!
//! - [`shapes`] — method-shape tables (`column_method_shape`, `two_df_method`,
//!   terminal recognizers).
//! - [`context`] — `BodyContext` plus the synthetic-name intern pool.
//! - [`driver`] — `check_function_body` + statement walker + return-type check.
//! - [`expr`] — `analyze_expr` and method-call dispatch.
//! - [`column_methods`] — column-method argument checking + reshape application.
//! - [`column_exprs`] — column-expression type inference (`infer_expr_type`).
//! - [`strict_operators`] — strict-mode operator checks (D0081 / D0082).
//! - [`two_df`] — `union`, `unionByName`, `join`, `crossJoin`.
//! - [`col_refs`] — `col(...)` / `df.X` / `F.sum("x")` reference discovery.

pub mod col_refs;
pub mod column_exprs;
pub mod column_methods;
pub mod context;
pub mod driver;
mod enum_checks;
pub mod expr;
pub mod shapes;
pub mod strict_operators;
pub mod two_df;

// Internal types — kept crate-visible for cross-module use by `completion.rs`,
// `hover.rs`, `symbols.rs`, and the `lib.rs` driver. Not part of the public API.
pub(crate) use context::{BodyContext, CallResultTrace, ColumnRefTrace, LocalBindingTrace};
pub(crate) use driver::check_function_body;

// Exposed to the integration-test crate that pins the synthetic-name pool's
// growth invariant — see `tests/groupby_aggregates.rs` and
// `tests/v15_pr_e_pool_softcap.rs`.
pub use context::{
    intern_synthetic_for_test, pool_full_warned_for_test, reset_synthetic_pool_for_test,
    synthetic_pool_len, synthetic_pool_sentinel,
};

#[cfg(test)]
mod tests {
    use crate::types::ColumnType;

    use super::column_exprs::function_result_type;
    use super::expr::aggregate_output_type;

    /// `aggregate_output_type` (groupBy.<method>(col)) and
    /// `function_result_type` (F.<name>(col) inside agg) must agree on
    /// numeric/decimal aggregates — otherwise the same expression
    /// produces different types depending on which surface the user
    /// happens to reach for.
    fn assert_agg_paths_agree(method: &str, input: ColumnType) {
        let via_shortcut = aggregate_output_type(method, Some(&input));
        let via_function = function_result_type(method, Some(input.clone()));
        assert_eq!(
            via_shortcut, via_function,
            "{method}({input}) — groupBy shortcut returned {via_shortcut:?}, F.{method} returned {via_function:?}",
        );
    }

    #[test]
    fn sum_decimal_agrees_between_groupby_shortcut_and_agg_function() {
        let input = ColumnType::Decimal {
            precision: 18,
            scale: 2,
        };
        assert_agg_paths_agree("sum", input.clone());
        assert_eq!(
            aggregate_output_type("sum", Some(&input)),
            Some(ColumnType::DEFAULT_DECIMAL),
        );
    }

    #[test]
    fn mean_decimal_agrees_between_groupby_shortcut_and_agg_function() {
        let input = ColumnType::Decimal {
            precision: 18,
            scale: 2,
        };
        assert_agg_paths_agree("mean", input.clone());
        assert_agg_paths_agree("avg", input.clone());
        // The unified rule: mean(decimal) stays decimal, not double.
        assert_eq!(
            aggregate_output_type("mean", Some(&input)),
            Some(ColumnType::DEFAULT_DECIMAL),
        );
    }

    #[test]
    fn min_max_decimal_agree_and_preserve_precision_scale() {
        let input = ColumnType::Decimal {
            precision: 18,
            scale: 2,
        };
        assert_agg_paths_agree("min", input.clone());
        assert_agg_paths_agree("max", input.clone());
        assert_eq!(
            aggregate_output_type("min", Some(&input)),
            Some(input.clone()),
        );
    }

    #[test]
    fn mean_non_decimal_numerics_still_promote_to_double_on_both_paths() {
        for input in [
            ColumnType::Int,
            ColumnType::Long,
            ColumnType::Byte,
            ColumnType::Short,
            ColumnType::Float,
            ColumnType::Double,
        ] {
            assert_agg_paths_agree("mean", input.clone());
            assert_eq!(
                aggregate_output_type("mean", Some(&input)),
                Some(ColumnType::Double),
                "mean({input}) should promote to Double",
            );
        }
    }

    /// Multi-lens-review finding: previously the `F.mean(...)` /
    /// `F.avg(...)` path returned `Some(Double)` for non-numeric input
    /// (string, bool, date, binary) via a permissive `Some(_)` arm,
    /// while the `groupBy.mean(col)` shortcut returned `None`. Both
    /// surfaces must now return `None` — Spark rejects the aggregate
    /// at runtime, so pinning a (wrong) Double from the function form
    /// was an actively misleading signal. `sum`/`min`/`max` already
    /// agreed on `None`; this assertion locks all five against future
    /// drift.
    #[test]
    fn aggregates_on_non_numeric_input_agree_between_paths() {
        for method in ["sum", "mean", "avg", "min", "max"] {
            for input in [
                ColumnType::String,
                ColumnType::Bool,
                ColumnType::Date,
                ColumnType::Binary,
            ] {
                assert_agg_paths_agree(method, input.clone());
            }
        }
        for input in [
            ColumnType::String,
            ColumnType::Bool,
            ColumnType::Date,
            ColumnType::Binary,
        ] {
            assert_eq!(
                function_result_type("mean", Some(input.clone())),
                None,
                "F.mean({input}) must return None — Spark rejects this aggregate",
            );
            assert_eq!(
                function_result_type("avg", Some(input.clone())),
                None,
                "F.avg({input}) must return None — Spark rejects this aggregate",
            );
            assert_eq!(
                function_result_type("sum", Some(input.clone())),
                None,
                "F.sum({input}) must return None — Spark rejects this aggregate",
            );
        }
    }
}
