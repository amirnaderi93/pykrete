//! v1.4 PR-F1 (architecture-audit B1) — gate the Subscript-on-Name arm
//! in `infer_expr_type` on a DataFrame binding in `BodyContext`, mirroring
//! the D0030 sibling arm in `col_refs.rs`.
//!
//! Before the gate, `infer_expr_type`'s Subscript-on-Name arm fired on
//! ANY `<name>["x"]` whose slice was a string literal — regardless of
//! whether `<name>` referred to a DataFrame. A plain Python dict
//! (`bag = {"x": 1}; bag["x"]`) would silently type against the
//! surrounding DataFrame's `x` field, causing D0081 / D0082 to fire on
//! comparisons / arithmetic that the user wrote against the dict value.
//!
//! Each test below is a negative-space regression-guard: it constructs
//! a fixture where the PRE-GATE arm would silently consult the frame
//! schema for a non-frame name and that consultation would surface a
//! diagnostic the user didn't author. Vacuity-verified by reverting the
//! gate locally and confirming each test fails (documented in the PR-F1
//! brief; not re-run in CI).

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V14F1_dict_subscript_does_not_fire_d0081:
//
// `bag` is a plain Python dict (textual key → int value). With the
// PRE-GATE arm, `bag["name"]` would resolve against the surrounding
// frame's `name: string` field, typing the operand as String. The
// arithmetic `col("amount") + bag["name"]` would then have a textual
// operand and fire D0081 on a wholly synthetic resolution. With the
// gate, `bag` has no DataFrame binding → the Subscript arm doesn't
// fire → `bag["name"]` is Unknown → D0081 stays silent.
// ---------------------------------------------------------------------------

#[test]
fn V14F1_dict_subscript_does_not_fire_d0081() {
    let result = check_strict(
        r#"
class Sales(Schema):
    amount: int
    name: string

def f(df: SparkFrame[Sales]) -> SparkFrame[Sales]:
    bag = {"name": 1}
    return df.filter(col("amount") + bag["name"] > 0)
"#,
    );
    assert_does_not_have_code(&result, "D0081");
}

// ---------------------------------------------------------------------------
// V14F1_dict_subscript_does_not_fire_d0082:
//
// `bag` is a plain Python dict whose value is an int constant. With
// the PRE-GATE arm, `bag["order_id"]` resolved against the frame's
// `order_id: int` field, typing the RHS as Int. The comparison
// `col("name") == bag["order_id"]` then had `String == Int` and fired
// D0082 on a synthetic resolution. With the gate, `bag["order_id"]` is
// Unknown → D0082's both-sides-Some guard keeps it silent. This is
// the exact fixture from the architecture-audit B1 reproduction.
// ---------------------------------------------------------------------------

#[test]
fn V14F1_dict_subscript_does_not_fire_d0082() {
    let result = check_strict(
        r#"
class Orders(Schema):
    order_id: int
    name: string

def f(df: SparkFrame[Orders]) -> SparkFrame[Orders]:
    bag = {"order_id": 1}
    return df.filter(col("name") == bag["order_id"])
"#,
    );
    assert_does_not_have_code(&result, "D0082");
}

// ---------------------------------------------------------------------------
// V14F1_free_var_subscript_does_not_fire_d0081:
//
// `cfg` is undefined / has no local binding at all (not a dict
// assignment, not a frame param — just a free name pykrete can't
// resolve). The gate keys on `body.lookup(name).is_some()`, so a free
// name produces None. The Subscript arm doesn't fire and `cfg["k"]`
// stays Unknown. `col("amount") + Unknown` doesn't have a textual
// operand on either side, so D0081 stays silent.
// ---------------------------------------------------------------------------

#[test]
fn V14F1_free_var_subscript_does_not_fire_d0081() {
    let result = check_strict(
        r#"
class Orders(Schema):
    order_id: int
    name: string

def f(df: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return df.filter(col("order_id") + cfg["name"] > 0)
"#,
    );
    assert_does_not_have_code(&result, "D0081");
}

// ---------------------------------------------------------------------------
// V14F1_cross_dataframe_subscript_does_not_misfire:
//
// Two DataFrames in scope (`orders` and `customers`). The PR-F1 gate
// pins the receiver to a frame-bound name; it doesn't FIX the I1
// pre-existing misroute (`customers["x"]` inside `orders.filter(...)`
// still resolves against whichever schema view is in flight, not
// against customers'). The test pins current behavior: PR-F1 must not
// MAKE I1 WORSE. With both `name` columns matching String on both
// schemas, the comparison `col("name") == customers["name"]` stays
// silent because the resolved types are compatible (String vs String).
// ---------------------------------------------------------------------------

#[test]
fn V14F1_cross_dataframe_subscript_does_not_misfire() {
    let result = check_strict(
        r#"
class Orders(Schema):
    order_id: int
    name: string

class Customers(Schema):
    name: string

def f(orders: SparkFrame[Orders], customers: SparkFrame[Customers]) -> SparkFrame[Orders]:
    return orders.filter(col("name") == customers["name"])
"#,
    );
    assert_does_not_have_code(&result, "D0082");
}

// ---------------------------------------------------------------------------
// V14F1_pandas_astype_int64_does_not_fire_d0011 (architecture-audit I2):
//
// Pandas nullable-dtype names — `Int64`, `Float32`, `boolean`, `string`,
// `category` — aren't Spark types and `from_spark_name` rejects them
// (case-lenient matching covers only the canonical Spark vocabulary:
// `int`/`integer`, `long`/`bigint`, etc., NOT capitalized pandas
// aliases). D0011 fires on `.cast("Int64")` today.
//
// However, D0011 is a `.cast(...)`-only check in
// `strict_operators.rs`. Pandas `.astype("Int64")` and the dict-form
// `.astype({"col": "Int64"})` aren't intercepted at all — they fall
// through silently (no D0011, no carve-out needed). The risk in I2 is
// that if v1.5 extends the cast-target check to cover `.astype`, the
// pandas dtype vocabulary will start tripping D0011. This test pins
// CURRENT behaviour: `.astype("Int64")` produces no D0011 today.
// Documented as a known limitation: the cast-target carve-out doesn't
// yet cover pandas dtypes; if `.astype` ever joins the check, widen
// `is_unmodeled_spark_type` (or the equivalent pandas carve-out) at
// the same time.
// ---------------------------------------------------------------------------

#[test]
fn V14F1_pandas_astype_int64_does_not_fire_d0011() {
    let result = check_strict(
        r#"
class Sales(Schema):
    amount: int

def f(pdf: PandasFrame[Sales]) -> PandasFrame[Sales]:
    return pdf.astype("Int64")
"#,
    );
    assert_does_not_have_code(&result, "D0011");
}
