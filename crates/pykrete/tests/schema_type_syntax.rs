//! Schema field types are written as ordinary Python type annotations:
//!
//!     class Forecast(Schema):
//!         predicted_value: double
//!         forecast_date: date
//!         lead_time: int
//!         tags: Array[string]
//!         by_city: Map[string, int]
//!
//! Atomic types are bare names; collections use `Array[…]` / `Map[…]`
//! subscripts; a nested struct is a bare reference to a declared
//! `Schema` class.

#![allow(non_snake_case)]

mod common;

use common::check;

#[test]
fn bare_atomic_types_resolve() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: double
    log_date: timestamp
    checkin: date
    is_active: bool
    label: string
    cnt: long
"#;
    let result = check(src);
    assert!(
        result.diagnostics.is_empty(),
        "expected a clean run; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
    assert_eq!(result.schema_count, 1);
}

#[test]
fn collection_subscript_types_resolve() {
    let src = r#"
class Orders(Schema):
    tags: Array[string]
    grid: Array[Array[int]]
    by_city: Map[string, int]
"#;
    assert!(check(src).diagnostics.is_empty());
}

#[test]
fn unknown_type_name_fires_D0010() {
    let src = r#"
class Orders(Schema):
    weird: weirdtype
"#;
    assert!(check(src).has_code("D0010"));
}

#[test]
fn nested_struct_uses_a_bare_schema_reference() {
    let src = r#"
class Address(Schema):
    street: string
    city: string

class User(Schema):
    name: string
    address: Address
"#;
    let result = check(src);
    assert!(
        result.diagnostics.is_empty(),
        "expected clean nested-struct resolution; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn column_ref_typo_fires_against_a_bare_name_schema() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: double

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("priec"))
"#;
    assert!(check(src).has_code("D0030"));
}
