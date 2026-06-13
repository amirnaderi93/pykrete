//! v1.6 PR-A1 — `cross_dialect_handoff_gate` recognizer extract.
//!
//! Spec §3.1. The dialect-side path (`inherited_dialect` in
//! `driver.rs`) and the view-side path (`createdataframe_handoff_view`
//! in `expr.rs`) used to carry duplicated gate logic plus a
//! "Keep in sync" comment on each. PR-A1 extracts the gate match into
//! a single `cross_dialect_handoff_gate(call, ctx) -> Option<HandoffGate>`
//! recognizer that both consumers call. The recognizer does not invoke
//! `analyze_expr`; the view-side consumer descends into the matched
//! expression to resolve the schema view, and the dialect-side consumer
//! short-circuits to `Dialect::Spark` without descent.
//!
//! These tests pin observable behavior across both consumers
//! simultaneously, so a future drift between them (the failure mode the
//! deleted "Keep in sync" comments warned about) regresses these tests:
//!
//! - Positive gate (a) — `schema=` Spark-tagged kwarg routes to
//!   `SparkFrame[X]` and a typo on the chained `.select` fires D0030.
//! - Positive gate (b) — pandas positional arg routes to
//!   `SparkFrame[Y]` and a typo on the chained `.select` fires D0030.
//! - Negative — neither gate fires; the call result is Unknown and a
//!   chained `.select("nonexistent")` does not fire D0030.
//!
//! Regression-guard coverage is also pinned by the existing v15 PR-A2
//! suite (`v15_pr_a2_createdataframe.rs`), which exercises the same end-
//! to-end shapes pre- and post-extract — those tests continue passing
//! is the regression signal that the refactor preserved semantics.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V16A1_gate_a_schema_kwarg_resolves_to_sparkframe:
//
// Gate (a) — `schema=` kwarg is a Spark-tagged binding. After extract,
// the recognizer must return `HandoffGate::SchemaKwarg(&kw.value)` and
// the view-side consumer must descend into that expression via
// `analyze_expr`. D0030 on the chained `.select("nonexistent")`
// witnesses both the schema view (carried through gate (a)) and the
// dialect-tag flip (carried through `inherited_dialect`).
// ---------------------------------------------------------------------------

#[test]
fn V16A1_gate_a_schema_kwarg_resolves_to_sparkframe() {
    let result = check_strict(
        r#"
class Sales(Schema):
    amount: int

def f(spark: SparkSession, rows: list, schema_var: SparkFrame[Sales]):
    sdf = spark.createDataFrame(rows, schema=schema_var)
    return sdf.select("nonexistent")
"#,
    );
    assert_has_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16A1_gate_b_pandas_positional_resolves_to_sparkframe:
//
// Gate (b) — first positional arg is a Pandas-tagged binding. After
// extract, the recognizer must return `HandoffGate::PandasPositional(arg)`
// and the view-side consumer descends via `analyze_expr` to resolve the
// schema; the dialect-side consumer short-circuits to
// `Dialect::Spark`. D0030 on the chained `.select("nonexistent")` is the
// dual witness.
// ---------------------------------------------------------------------------

#[test]
fn V16A1_gate_b_pandas_positional_resolves_to_sparkframe() {
    let result = check_strict(
        r#"
class Orders(Schema):
    id: int

def f(spark: SparkSession, pdf: PandasFrame[Orders]):
    sdf = spark.createDataFrame(pdf)
    return sdf.select("nonexistent")
"#,
    );
    assert_has_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16A1_neither_gate_falls_through_to_unknown:
//
// Neither gate fires — un-annotated positional arg, no `schema=` kwarg.
// The recognizer returns `None`; both consumers fall through. The call
// result is Unknown so a chained `.select("nonexistent")` does NOT fire
// D0030 (no schema to compare against). Negative-space pin per
// v14-rule 4: locks "neither gate" against future drift in either arm.
// ---------------------------------------------------------------------------

#[test]
fn V16A1_neither_gate_falls_through_to_unknown() {
    let result = check_strict(
        r#"
def f(spark: SparkSession, pdf):
    sdf = spark.createDataFrame(pdf)
    return sdf.select("nonexistent")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16A1_dialect_side_short_circuits_without_view:
//
// Pins the dialect-side consumer specifically — `inherited_dialect` must
// return `Dialect::Spark` on a recognizer match WITHOUT descending into
// the matched arg. Uses PR-A3's `.head()` Spark-terminal gate as a
// discriminator (sibling to v15's lhs_dialect_flips test): a correct
// Spark flip drops the chain at `.head()` so the col("nonexistent") on
// the next `.assign` has no schema to check (no D0030). A broken
// dialect-side gate would leave the binding Pandas, `.head()` would not
// terminate, and D0030 would fire.
// ---------------------------------------------------------------------------

#[test]
fn V16A1_dialect_side_short_circuits_without_view() {
    let result = check_strict(
        r#"
class Sales(Schema):
    amount: int

def f(spark: SparkSession, pdf: PandasFrame[Sales]):
    sdf = spark.createDataFrame(pdf)
    return sdf.head().assign(amount=col("nonexistent"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
