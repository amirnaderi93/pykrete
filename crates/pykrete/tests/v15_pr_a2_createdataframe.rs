//! v1.5 PR-A2 — `<sparksession>.createDataFrame(...)` dialect-handoff
//! inference arm in `analyze_method_call_inner`. Spec §2.2.
//!
//! The arm returns `SparkFrame[X]` only when one of two schema-source
//! gates fires: a `schema=` kwarg whose value resolves through binding
//! lookup to a Spark-tagged frame, or a positional first arg that
//! resolves to a Pandas-tagged frame. Receiver shape (`<X>` being any
//! `Name`) is deliberately NOT a gate — a structural match would
//! mis-tag `not_spark.createDataFrame(pdf)` as Spark when neither
//! gate is present (spec §2.2 "Why not mirror `spark.read.<format>`").
//!
//! Each test below pins one cell of the gate matrix:
//! - V15A2_gate_b_pandas_arg_re_tags_as_spark (positive, gate b).
//! - V15A2_gate_a_schema_kwarg_re_tags_as_spark (positive, gate a).
//! - V15A2_lhs_dialect_flips_to_spark_via_pr_a3_terminal (positive,
//!   pins the LHS dialect-tag flip independently of the schema view —
//!   uses PR-A3's `.head()` Spark-terminal gate as a discriminator).
//! - V15A2_neither_gate_falls_through_to_unknown (regression-guard —
//!   passes via dispatcher fall-through pre-PR-A2; locks the negative
//!   behavior in for any future structural-match arm).
//! - V15A2_not_spark_receiver_no_gate_falls_through (regression-guard
//!   for the same — passes pre-PR-A2 via fall-through; pins the no-
//!   structural-match constraint against future drift).
//! - V15A2_schema_kwarg_unresolvable_falls_back_to_arg (negative on
//!   gate a, positive on gate b — gate ordering / fall-back).

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V15A2_gate_b_pandas_arg_re_tags_as_spark:
//
// Gate (b): positional first arg has a `PandasFrame[Y]` annotation. The
// result must be `SparkFrame[Y]` — same schema, dialect flipped to Spark.
// Verified by chaining `.select("nonexistent")` on the result: D0030 must
// fire (i.e. the schema view was carried through, not dropped to Unknown).
// ---------------------------------------------------------------------------

#[test]
fn V15A2_gate_b_pandas_arg_re_tags_as_spark() {
    let result = check_strict(
        r#"
class Sales(Schema):
    amount: int

def f(spark: SparkSession, pdf: PandasFrame[Sales]):
    sdf = spark.createDataFrame(pdf)
    return sdf.select("nonexistent")
"#,
    );
    assert_has_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15A2_gate_a_schema_kwarg_re_tags_as_spark:
//
// Gate (a): `schema=` kwarg is a Name with a Spark-tagged annotation.
// `schema=` at runtime takes a `StructType` or list, but the *binding*
// for `schema_var` is `SparkFrame[Sales]`, so pykrete reads the schema
// through that binding. Result is `SparkFrame[Sales]`; D0030 fires on
// the chained `.select("nonexistent")`.
// ---------------------------------------------------------------------------

#[test]
fn V15A2_gate_a_schema_kwarg_re_tags_as_spark() {
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
// V15A2_lhs_dialect_flips_to_spark_via_pr_a3_terminal:
//
// Pins the LHS dialect-tag flip INDEPENDENTLY of the schema view. The
// gate-a/b tests above only exercise the schema; they pass whether the
// LHS is tagged Spark or stays tagged Pandas (since col-existence checks
// fire on either dialect with Sales schema). This test uses PR-A3's
// `.head()` Spark-terminal gate as a discriminator:
//
// - Correct flip (sdf = SparkFrame[Sales]): .head() is a Spark terminal,
//   chain dies, schema dropped; .assign(amount=col("nonexistent")) has
//   no schema to check → no D0030.
// - Broken flip (sdf has dialect=None, schema=Sales): if the
//   inherited_dialect Call-arm override at driver.rs:139 were reverted,
//   the cursor would walk past the Call into `spark` (a Name with no
//   dialect), returning None — NOT Pandas. .head() with dialect=None
//   is not Spark-terminal (gate requires Spark dialect explicitly) and
//   falls through to the schema-preserving pass-through; .assign with
//   col("nonexistent") then checks against Sales schema → D0030 fires.
//
// assert_does_not_have_code distinguishes correct flip vs stayed-Pandas.
// ---------------------------------------------------------------------------

#[test]
fn V15A2_lhs_dialect_flips_to_spark_via_pr_a3_terminal() {
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

// ---------------------------------------------------------------------------
// V15A2_neither_gate_falls_through_to_unknown:
//
// `pdf` has no `PandasFrame[...]` annotation and there is no `schema=`
// kwarg. Per spec §2.2 step 3, auto-inference without either source is
// explicitly OUT for v1.5 — the call result is Unknown. A chained
// `.select("nonexistent")` against an Unknown receiver does NOT fire
// D0030 because there is no schema to compare against.
//
// REGRESSION GUARD (not load-bearing today): this test passes both
// WITH and WITHOUT PR-A2 because pre-PR-A2 createDataFrame falls
// through every dispatcher arm to Unknown anyway. We keep the test as
// a future-drift guard — if anyone later adds a structural-match arm
// to PR-A2's site, this test will fail. The load-bearing positive
// coverage is in V15A2_gate_a/b/lhs_dialect_flips.
// ---------------------------------------------------------------------------

#[test]
fn V15A2_neither_gate_falls_through_to_unknown() {
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
// V15A2_not_spark_receiver_no_gate_falls_through:
//
// The receiver `not_spark` is a bound Name with no SparkSession
// annotation; the call has no `schema=` kwarg and `pdf` is un-annotated.
// Both gates fail → fall through to Unknown. This pins that the PR
// does NOT use structural matching on the receiver (which would have
// mis-tagged this call as Spark, mirroring the `is_spark_opaque_source_call`
// pattern that the spec explicitly rejected for PR-A2).
//
// REGRESSION GUARD (not load-bearing today): same shape as
// V15A2_neither_gate — pre-PR-A2 fall-through gives the same outcome.
// Kept to pin the no-structural-match constraint against future drift.
// ---------------------------------------------------------------------------

#[test]
fn V15A2_not_spark_receiver_no_gate_falls_through() {
    let result = check_strict(
        r#"
def f(not_spark, pdf):
    sdf = not_spark.createDataFrame(pdf)
    return sdf.select("nonexistent")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15A2_schema_kwarg_unresolvable_falls_back_to_arg:
//
// `schema=` is a Name with no resolvable annotation (just a plain
// parameter, no SparkFrame/DataFrame tag), so gate (a) fails. The
// positional first arg `pdf: PandasFrame[Sales]` satisfies gate (b),
// so the result is `SparkFrame[Sales]` and D0030 fires on the chained
// `.select("nonexistent")`. This pins fall-back semantics: gate (a)
// failing does not poison gate (b).
// ---------------------------------------------------------------------------

#[test]
fn V15A2_schema_kwarg_unresolvable_falls_back_to_arg() {
    let result = check_strict(
        r#"
class Sales(Schema):
    amount: int

def f(spark: SparkSession, pdf: PandasFrame[Sales], schema_var):
    sdf = spark.createDataFrame(pdf, schema=schema_var)
    return sdf.select("nonexistent")
"#,
    );
    assert_has_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15A2_round_trip_topandas_then_createdataframe_re_tags_as_spark:
//
// Spec §2.2 line 133-134 mandates this regression guard:
// `spark.createDataFrame(df.toPandas())` must re-tag PR-A1's inferred
// `PandasFrame[Orders]` back to `SparkFrame[Orders]` end-to-end. The
// inner `df.toPandas()` resolves through PR-A1's recursive
// `infer_expr_type` walk to `PandasFrame[Orders]`; PR-A2's gate (b)
// then re-tags the call result to `SparkFrame[Orders]`. A chained
// `.select("nonexistent")` fires D0030 — proves the schema view rode
// the full round-trip.
// ---------------------------------------------------------------------------

#[test]
fn V15A2_round_trip_topandas_then_createdataframe_re_tags_as_spark() {
    let result = check_strict(
        r#"
class Orders(Schema):
    id: int

def f(sdf: SparkFrame[Orders], spark: SparkSession):
    sdf2 = spark.createDataFrame(sdf.toPandas())
    return sdf2.select("nonexistent")
"#,
    );
    assert_has_code(&result, "D0030");
}
