//! v1.4 PR-A (spec §3c, fork (a)) — `Expr::Subscript`-on-Name arm in
//! `infer_expr_type`.
//!
//! Before v1.4, `infer_expr_type` had no Subscript arm: `df["x"] + 1`
//! evaluated to `None` on both operands, so the D0081 dispatch in
//! `report_expr_type_errors` could not see the textual-vs-numeric
//! conflict — every pandas TYPE-IS probe came back inconclusive
//! against `main` (vacuity trap).
//!
//! The new arm handles `Expr::Subscript` whose receiver is `Expr::Name`
//! and whose slice is a single string literal — the pandas scalar
//! col-ref idiom `df["x"]`. It looks the name up via
//! `schema.field_type(...)`, mirroring how `col("x")` is resolved.
//! Other receiver shapes (Attribute, Call, nested Subscript) and
//! non-literal slices fall through silently per spec §3c.
//!
//! Spec: `docs/design/v1.4-spec.md` §3c.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Positive: arm fires AND returns a numeric type → D0081 stays silent.
// ---------------------------------------------------------------------------

#[test]
fn V14A_pandas_subscript_arm_returns_column_type() {
    // `df["amount"] + 1` where amount is `int` — arithmetic on a numeric
    // column is valid; D0081 must NOT fire. Confirms the new arm
    // recognizes the receiver and returns a non-textual type.
    let result = check_strict(
        r#"
class Sales(Schema):
    amount: int

def f(df: PandasFrame[Sales]) -> PandasFrame[Sales]:
    return df.assign(_probe=df["amount"] + 1)
"#,
    );
    assert_does_not_have_code(&result, "D0081");
}

// ---------------------------------------------------------------------------
// Negative (vacuity-falsifying): arm fires AND returns a textual type
// → D0081 fires. This is THE test that flips from PASS-by-vacuity on
// `main` (arm missing → infer returns None → D0081 silent) to
// FAIL-then-fire after the arm lands.
// ---------------------------------------------------------------------------

#[test]
fn V14A_pandas_subscript_arm_fires_d0081_on_string_col() {
    let result = check_strict(
        r#"
class Customer(Schema):
    name: string

def f(df: PandasFrame[Customer]) -> PandasFrame[Customer]:
    return df.assign(_probe=df["name"] + 1)
"#,
    );
    assert_has_code(&result, "D0081");
    assert_message_contains(&result, "D0081", "non-numeric");
}

// ---------------------------------------------------------------------------
// Bound: unknown column → arm returns None → no panic, no false D0081.
// (Pre-existing pandas-piece-(b) col-ref check still fires D0030 on
// the kwarg-value walk, but D0081 specifically must stay silent.)
// ---------------------------------------------------------------------------

#[test]
fn V14A_pandas_subscript_arm_returns_none_on_unknown_col() {
    let result = check_strict(
        r#"
class Sales(Schema):
    amount: int

def f(df: PandasFrame[Sales]) -> PandasFrame[Sales]:
    return df.assign(_probe=df["missing"] + 1)
"#,
    );
    // The arm returns None because "missing" isn't in Sales. D0081
    // looks at the operand families; with both sides None it stays
    // silent (no false positive).
    assert_does_not_have_code(&result, "D0081");
}

// ---------------------------------------------------------------------------
// Spark-path regression: the new arm is dialect-agnostic, but Spark's
// canonical column-ref is `col("x")` rather than `df["x"]`. Confirm
// that adding the new arm doesn't break the Spark D0081 path that
// already worked via `col_reference`.
// ---------------------------------------------------------------------------

#[test]
fn V14A_pandas_subscript_arm_does_not_fire_on_spark_dialect() {
    // Spark `df.select(col("name") + 1)` — D0081 should fire because
    // name is string. The Subscript arm doesn't apply here (no
    // subscript expression), so this verifies the existing `col`
    // dispatch is untouched by the new arm.
    let result = check_strict(
        r#"
from pyspark.sql.functions import col

class Customer(Schema):
    name: string

def f(df: SparkFrame[Customer]) -> SparkFrame[Customer]:
    return df.select(col("name") + 1)
"#,
    );
    assert_has_code(&result, "D0081");
}
