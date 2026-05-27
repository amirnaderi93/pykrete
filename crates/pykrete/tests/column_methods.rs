//! `Column` method chains — `.isNull` / `.isNotNull` / `.isin` /
//! `.between` / `.like` / `.rlike` / `.ilike` / `.contains` /
//! `.startswith` / `.endswith` (the predicate family) and `.getField` /
//! `.getItem` / `.withField` / `.dropFields` (the struct / collection
//! navigation family).
//!
//! These are extremely common in real `filter` / `withColumn` code.
//! Before v0.1.13, pykrete only reached column refs *inside* these
//! method calls via the generic walker — the method itself was not
//! modeled, so a typo on the method name went unflagged AND the result
//! type was forgotten downstream. This file pins down both behaviors:
//! a clean chain stays clean, and a typo on a struct field referenced
//! through `.getField` fires D0030.

#![allow(non_snake_case)]

mod common;
use common::*;

const SIMPLE: &str = r#"
class In(Schema):
    x: int
    y: int
    z: int
    name: string
"#;

// ===========================================================================
// Predicate family — boolean-returning Column methods
// ===========================================================================

#[test]
fn is_null_returns_boolean_and_preserves_chain() {
    let result = check(&format!(
        r#"{SIMPLE}
def f(d: DataFrame[In]) -> DataFrame[In]:
    return d.filter(col("x").isNull())
"#
    ));
    assert_count(&result, "D0030", 0);
}

#[test]
fn is_not_null_returns_boolean_and_preserves_chain() {
    let result = check(&format!(
        r#"{SIMPLE}
def f(d: DataFrame[In]) -> DataFrame[In]:
    return d.filter(col("x").isNotNull())
"#
    ));
    assert_count(&result, "D0030", 0);
}

#[test]
fn is_in_validates_value_arguments() {
    // `y` and `z` exist — clean.
    let ok = check(&format!(
        r#"{SIMPLE}
def f(d: DataFrame[In]) -> DataFrame[In]:
    return d.filter(col("x").isin(col("y"), col("z")))
"#
    ));
    assert_count(&ok, "D0030", 0);

    // `nope` doesn't — D0030 fires on it via the generic-walker
    // descent into the call's args.
    let bad = check(&format!(
        r#"{SIMPLE}
def f(d: DataFrame[In]) -> DataFrame[In]:
    return d.filter(col("x").isin(col("y"), col("nope")))
"#
    ));
    assert_count(&bad, "D0030", 1);
    assert_message_contains(&bad, "D0030", "nope");
}

#[test]
fn between_recognized() {
    let result = check(&format!(
        r#"{SIMPLE}
def f(d: DataFrame[In]) -> DataFrame[In]:
    return d.filter(col("x").between(1, 10))
"#
    ));
    assert_count(&result, "D0030", 0);
}

#[test]
fn like_rlike_ilike_recognized() {
    let result = check(&format!(
        r#"{SIMPLE}
def f(d: DataFrame[In]) -> DataFrame[In]:
    return d.filter(col("name").like("A%")).filter(col("name").rlike("^A")).filter(col("name").ilike("a%"))
"#
    ));
    assert_count(&result, "D0030", 0);
}

#[test]
fn contains_startswith_endswith_recognized() {
    let result = check(&format!(
        r#"{SIMPLE}
def f(d: DataFrame[In]) -> DataFrame[In]:
    return d.filter(col("name").contains("foo")).filter(col("name").startswith("bar")).filter(col("name").endswith("baz"))
"#
    ));
    assert_count(&result, "D0030", 0);
}

#[test]
fn predicate_methods_inside_with_column_value_carry_bool_type() {
    // `withColumn` infers the new column's type from the value
    // expression; `.isNull()` should be inferred as Bool, which means
    // the return-type check against an Out schema declaring it as
    // `bool` passes strict mode without D0080.
    let src = format!(
        r#"{SIMPLE}
class Out(Schema):
    x: int
    y: int
    z: int
    name: string
    flag: bool

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("flag", col("x").isNull())
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

// ===========================================================================
// Struct / collection navigation family
// ===========================================================================

const NESTED: &str = r#"
class Address(Schema):
    city: string
    zipcode: string

class User(Schema):
    name: string
    address: Address
"#;

#[test]
fn getField_navigates_nested_struct() {
    let result = check(&format!(
        r#"{NESTED}
def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("address").getField("city"))
"#
    ));
    assert_count(&result, "D0030", 0);
}

#[test]
fn getField_on_typo_fires_diagnostic() {
    let result = check(&format!(
        r#"{NESTED}
def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("address").getField("citi"))
"#
    ));
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "citi");
}

#[test]
fn getField_on_non_literal_name_does_not_false_flag() {
    // A computed field name isn't statically resolvable. Pykrete must
    // not false-flag; the chain just runs unchecked through getField.
    let result = check(&format!(
        r#"{NESTED}
def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("address").getField(col("name")))
"#
    ));
    assert_count(&result, "D0030", 0);
}

const COLLECTIONS: &str = r#"
class Order(Schema):
    sku: string
    qty: int

class In(Schema):
    orders: Array[Order]
    by_id: Map[string, Order]
"#;

#[test]
fn getItem_array_returns_element_type() {
    // `col("orders").getItem(0)` yields an Order struct. Using it as
    // the value of `withColumn` and re-anchoring against a schema that
    // declares the new column as `Order` should produce no D0080.
    let src = format!(
        r#"{COLLECTIONS}
class Out(Schema):
    orders: Array[Order]
    by_id: Map[string, Order]
    first: Order

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("first", col("orders").getItem(0))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn getItem_map_returns_value_type() {
    let src = format!(
        r#"{COLLECTIONS}
class Out(Schema):
    orders: Array[Order]
    by_id: Map[string, Order]
    one: Order

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("one", col("by_id").getItem("123"))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

const NESTED_FOR_WITH_FIELD: &str = r#"
class Address(Schema):
    city: string

class AddressZ(Schema):
    city: string
    zipcode: string

class User(Schema):
    address: Address

class UserZ(Schema):
    address: AddressZ
"#;

#[test]
fn withField_adds_or_replaces_struct_field() {
    // `address.withField("zipcode", lit("00000"))` rebuilds the
    // address struct as `{city, zipcode}`. The output schema declares
    // that exact shape, so no D0080.
    let src = format!(
        r#"{NESTED_FOR_WITH_FIELD}
def f(d: DataFrame[User]) -> DataFrame[UserZ]:
    return d.withColumn("address", col("address").withField("zipcode", lit("00000")))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn dropFields_removes_struct_fields() {
    // The reverse: starting from AddressZ (city + zipcode) and dropping
    // zipcode, we get back to Address (just city).
    let src = format!(
        r#"{NESTED_FOR_WITH_FIELD}
def f(d: DataFrame[UserZ]) -> DataFrame[User]:
    return d.withColumn("address", col("address").dropFields("zipcode"))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}
