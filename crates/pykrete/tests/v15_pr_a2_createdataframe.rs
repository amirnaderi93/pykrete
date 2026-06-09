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
//! - V15A2_neither_gate_falls_through_to_unknown (negative-space).
//! - V15A2_not_spark_receiver_no_gate_falls_through (negative-space —
//!   the structural-match anti-pattern this PR explicitly avoids).
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
// V15A2_neither_gate_falls_through_to_unknown:
//
// `pdf` has no `PandasFrame[...]` annotation and there is no `schema=`
// kwarg. Per spec §2.2 step 3, auto-inference without either source is
// explicitly OUT for v1.5 — the call result is Unknown. A chained
// `.select("nonexistent")` against an Unknown receiver does NOT fire
// D0030 because there is no schema to compare against. This is the
// load-bearing negative-space test for the "no structural match"
// constraint.
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
