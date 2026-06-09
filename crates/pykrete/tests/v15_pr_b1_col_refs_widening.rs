//! v1.5 PR-B1 — `collect_col_refs` widened output threads the
//! receiver-Name from `<recv>.X` / `<recv>["X"]` leaf arms through to
//! `report_column_refs`, which then routes the schema lookup to that
//! receiver's own schema rather than the surrounding method-chain's.
//!
//! Spec §3.1 enumerates four negative-space cases:
//! - `df.select(df_other["col"])` both bound → checks against
//!   `df_other`'s schema (positive I1 fix).
//! - `df.select(df["col"])` → checks against `df`'s schema
//!   (same-receiver regression guard).
//! - `df.select(df_other.x)` attribute form → `df_other` (sibling arm).
//! - `df.select(helper["col"])` unbound `helper` → fall through
//!   (existing-gate regression guard from v1.4 PR-F1).

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V15B1_subscript_cross_dataframe_routes_to_other_schema:
//
// Positive I1 fix. `Orders` has `name`; `Customers` does NOT.
// `orders.select(customers["nope"])` must fire D0030 against Customers
// (the receiver of the subscript), NOT against Orders (the chain head).
// Before the widening, `nope` was checked against `orders`, missed
// `nope` on Orders just as it would on Customers, but with the WRONG
// `on` schema in the diagnostic message — and worse, `orders.select(
// customers["name"])` would silently PASS because `name` exists on
// Orders even though the subscript is on Customers. This test pins
// that the routing now follows the receiver Name.
// ---------------------------------------------------------------------------

#[test]
fn V15B1_subscript_cross_dataframe_routes_to_other_schema() {
    let result = check(
        r#"
class Orders(Schema):
    order_id: int
    name: string

class Customers(Schema):
    customer_id: int

def f(orders: SparkFrame[Orders], customers: SparkFrame[Customers]) -> SparkFrame[Orders]:
    return orders.select(customers["name"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "Customers");
}

// ---------------------------------------------------------------------------
// V15B1_subscript_same_dataframe_routes_to_self:
//
// Same-receiver regression guard. `df.select(df["col"])` is the
// idiomatic form; it MUST keep working. `name` exists on Orders, so
// `orders.select(orders["name"])` must NOT fire D0030.
// ---------------------------------------------------------------------------

#[test]
fn V15B1_subscript_same_dataframe_routes_to_self() {
    let result = check(
        r#"
class Orders(Schema):
    order_id: int
    name: string

def f(orders: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return orders.select(orders["name"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15B1_attribute_cross_dataframe_routes_to_other_schema:
//
// Sibling arm — attribute form `df_other.x`. Same routing logic as the
// subscript arm; both leaf arms in `collect_col_refs` populate the
// receiver-Name slot. `orders.select(customers.nope)` must fire D0030
// against Customers, not Orders.
// ---------------------------------------------------------------------------

#[test]
fn V15B1_attribute_cross_dataframe_routes_to_other_schema() {
    let result = check(
        r#"
class Orders(Schema):
    order_id: int
    name: string

class Customers(Schema):
    customer_id: int

def f(orders: SparkFrame[Orders], customers: SparkFrame[Customers]) -> SparkFrame[Orders]:
    return orders.select(customers.nope)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "Customers");
}

// ---------------------------------------------------------------------------
// V15B1_unbound_receiver_subscript_falls_through:
//
// Existing-gate regression guard (v1.4 PR-F1). `helper` is unbound —
// not a DataFrame param, not a constant. The `ctx.lookup(name).is_some()`
// gate on the subscript arm prevents `helper["col"]` from being
// collected as a column ref at all. No D0030 should fire on this shape
// even though `col` doesn't exist on Orders.
// ---------------------------------------------------------------------------

#[test]
fn V15B1_unbound_receiver_subscript_falls_through() {
    let result = check(
        r#"
class Orders(Schema):
    order_id: int
    name: string

def f(orders: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return orders.select(helper["nope"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
