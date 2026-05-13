//! Parsing, schema discovery, and field-type resolution.

#![allow(non_snake_case)] // Type names (Schema, DataFrame) appear in test names.

//!
//! Exercises diagnostics:
//! - `D0001` — Python parse error.
//! - `D0010` — unknown column type name in a schema field.
//! - `D0011` — column type expression isn't a bare name
//!             (subscripts, attribute access, etc.).
//!
//! Each test embeds a small `.dpy` source as a raw string literal and asserts
//! on the diagnostics produced by `dathon::check`.

mod common;
use common::*;

// ===========================================================================
// D0001 — parse errors
// ===========================================================================

#[test]
fn d0001_fires_on_invalid_python_syntax() {
    // Unbalanced parenthesis after `def foo(` is not valid Python.
    let result = check("def foo(:\n    pass\n");
    assert_has_code(&result, "D0001");
    assert!(result.parse_error);
}

#[test]
fn valid_python_with_no_dathon_constructs_produces_no_diagnostics() {
    // Plain Python with no schemas, typed DataFrames, or bodies to check.
    // The parser is happy and nothing else fires.
    let result = check("x = 1\ny = 2\n");
    assert_no_diagnostics(&result);
    assert!(!result.parse_error);
    assert_eq!(result.schema_count, 0);
    assert_eq!(result.typed_function_count, 0);
}

// ===========================================================================
// Schema discovery — which classes are recognized as Schemas
// ===========================================================================

#[test]
fn class_extending_Schema_is_recognized_as_a_schema() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: "int"
"#,
    );
    assert_eq!(result.schema_count, 1);
}

#[test]
fn class_without_Schema_base_is_not_recognized_even_with_annotations() {
    // A plain class with field annotations is NOT a dathon schema.
    // It must explicitly inherit from Schema.
    let result = check(
        r#"
class Orders:
    place_code: "int"
"#,
    );
    assert_eq!(result.schema_count, 0);
}

#[test]
fn class_with_Schema_and_additional_mixin_is_still_a_schema() {
    // Multiple inheritance is fine as long as Schema is in the bases.
    let result = check(
        r#"
class Orders(Schema, Mixin):
    x: "int"
"#,
    );
    assert_eq!(result.schema_count, 1);
}

#[test]
fn class_with_subscripted_base_is_not_treated_as_a_schema() {
    // `Generic[T]` is a Subscript expression, not the bare name `Schema`,
    // so the class is not recognized as a Schema.
    let result = check(
        r#"
class Orders(Generic[T]):
    x: "int"
"#,
    );
    assert_eq!(result.schema_count, 0);
}

#[test]
fn multiple_schemas_in_one_file_are_all_recognized() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: "int"

class RawOrders(Schema):
    ProductCode: "int"

class NotMe:
    x: "int"
"#,
    );
    assert_eq!(result.schema_count, 2);
}

// ===========================================================================
// D0010 — unknown column types
// ===========================================================================

#[test]
fn all_v0_1_atomic_types_resolve_without_diagnostics() {
    // The full vocabulary, one of each.
    let result = check(
        r#"
class All(Schema):
    a: "int"
    b: "long"
    c: "double"
    d: "string"
    e: "bool"
    f: "date"
    g: "timestamp"
"#,
    );
    assert_count(&result, "D0010", 0);
    assert_count(&result, "D0011", 0);
}

#[test]
fn d0010_fires_when_field_type_is_unknown_name() {
    let result = check(
        r#"
class Orders(Schema):
    x: WeirdType
"#,
    );
    assert_has_code(&result, "D0010");
    assert_message_contains(&result, "D0010", "WeirdType");
}

#[test]
fn d0010_fires_for_python_float_which_we_do_not_accept() {
    // PySpark distinguishes FloatType (32-bit) from DoubleType (64-bit).
    // dathon v0.1 only accepts `double`; `float` is rejected to keep
    // the runtime mapping unambiguous.
    let result = check(
        r#"
class Orders(Schema):
    x: float
"#,
    );
    assert_has_code(&result, "D0010");
    assert_message_contains(&result, "D0010", "float");
}

#[test]
fn d0010_fires_once_per_offending_field() {
    let result = check(
        r#"
class Orders(Schema):
    a: "int"
    b: WeirdType
    c: AlsoWeird
    d: "long"
"#,
    );
    // Two bad fields, two D0010s. The good ones (a, d) are silent.
    assert_count(&result, "D0010", 2);
}

// ===========================================================================
// D0011 — non-bare-name types (subscripts, etc.)
// ===========================================================================

#[test]
fn d0011_fires_when_field_type_is_subscripted() {
    // `list[str]` is a Subscript expression — not a bare name we can
    // resolve against the v0.1 atomic-type vocabulary.
    let result = check(
        r#"
class Orders(Schema):
    tags: list[str]
"#,
    );
    assert_has_code(&result, "D0011");
    assert_message_contains(&result, "D0011", "list[str]");
}

#[test]
fn d0010_and_d0011_can_coexist_on_different_fields() {
    let result = check(
        r#"
class Orders(Schema):
    a: WeirdType
    b: list[str]
"#,
    );
    assert_count(&result, "D0010", 1);
    assert_count(&result, "D0011", 1);
}

// ===========================================================================
// Nested struct fields — `name: AnotherSchemaClass`
// ===========================================================================

#[test]
fn class_field_typed_with_another_Schema_class_is_resolved_as_nested() {
    // `address: Address` — Address is another Schema declared in the same
    // file. The field resolves to a nested struct; no diagnostic.
    let result = check(
        r#"
class Address(Schema):
    street: "string"
    city: "string"

class User(Schema):
    name: "string"
    address: Address
"#,
    );
    assert_no_diagnostics(&result);
    assert_eq!(result.schema_count, 2);
}

#[test]
fn nested_schema_can_appear_before_or_after_its_referencer() {
    // Forward reference works too — schemas are resolved as a set, not
    // in declaration order.
    let result = check(
        r#"
class User(Schema):
    name: "string"
    address: Address

class Address(Schema):
    street: "string"
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn d0010_fires_when_nested_class_does_not_extend_Schema() {
    // `Foo` is a plain class with field annotations; without `Schema` as a
    // base it's not discovered as a dathon schema, so the `nested: Foo`
    // reference falls through to D0010 just like any unknown type name.
    let result = check(
        r#"
class Foo:
    x: "int"

class Bar(Schema):
    nested: Foo
"#,
    );
    assert_has_code(&result, "D0010");
    assert_message_contains(&result, "D0010", "Foo");
}

#[test]
fn deeply_nested_schemas_resolve_independently() {
    // Three layers of nesting — each level's resolution is independent;
    // dathon doesn't need to walk through the chain.
    let result = check(
        r#"
class Inner(Schema):
    leaf: "int"

class Middle(Schema):
    inner: Inner

class Outer(Schema):
    middle: Middle
    label: "string"
"#,
    );
    assert_no_diagnostics(&result);
    assert_eq!(result.schema_count, 3);
}

#[test]
fn nested_schema_field_in_a_function_typed_with_outer_schema_is_recognized() {
    // The outer schema's `address` field (typed as nested Address) exists
    // as a top-level field by name. A select on it doesn't fire D0030.
    let result = check(
        r#"
class Address(Schema):
    street: "string"

class User(Schema):
    name: "string"
    address: Address

def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("name"), col("address"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
