//! Dotted column access through nested struct schemas.
//!
//! `col("address.street")` walks the path through nested `Schema` classes:
//! non-final segments must resolve to a nested-schema field; the final
//! segment is checked with the usual `has_field` against whichever schema
//! the walk lands on.
//!
//! Diagnostic refinement: when a path fails mid-way, the `D0030` message
//! points at the **failed segment** and the **schema we were searching at
//! that point**, not the whole dotted string against the outer schema.
//! For `col("address.missing")` against a `User` schema where `address`
//! is a nested `Address`, the diagnostic says `Column 'missing' does not
//! exist on schema 'Address'` — the user can see exactly where to look.

#![allow(non_snake_case)]

mod common;
use common::*;

const NESTED_SCHEMAS: &str = r#"
class Address(Schema):
    street: "string"
    city: "string"

class User(Schema):
    name: "string"
    address: Address
"#;

// ===========================================================================
// Happy path
// ===========================================================================

#[test]
fn dotted_path_resolves_through_a_single_level_of_nesting() {
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: DataFrame[User]) -> DataFrame[User]:
    return u.select(col("address.street"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn dotted_path_resolves_through_multiple_levels() {
    let result = check(
        r#"
class Inner(Schema):
    leaf: "int"

class Middle(Schema):
    inner: Inner

class Outer(Schema):
    middle: Middle

def f(o: DataFrame[Outer]) -> DataFrame:
    return o.select(col("middle.inner.leaf"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn top_level_field_access_still_works_when_field_is_a_nested_struct() {
    // No dot in the path; just the top-level `address` (which is itself
    // a nested struct field). Should resolve as a normal flat lookup.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("address"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn dotted_path_works_inside_filter_and_withColumn_too() {
    // Same resolution rule applies in any context where col() refs are
    // checked. Just verify the wiring.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: DataFrame[User]) -> DataFrame[User]:
    return u.filter(col("address.city") == "Tehran").withColumn("c", col("address.street"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Failure: diagnostic pinpoints the failed segment and the nested schema
// ===========================================================================

#[test]
fn d0030_on_failed_inner_segment_names_the_nested_schema() {
    // `address` exists as a nested Address, but `street` is mis-typed
    // as `streetz`. The diagnostic should reference Address, not User —
    // that's where the missing field actually is.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("address.streetz"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "streetz");
    assert_message_contains(&result, "D0030", "Address");
}

#[test]
fn d0030_on_failed_outer_segment_names_the_outer_schema() {
    // `nope` doesn't exist on User at all. Diagnostic reports against User.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("nope.street"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
    assert_message_contains(&result, "D0030", "User");
}

#[test]
fn d0030_when_intermediate_segment_is_not_a_nested_schema() {
    // `name` exists as a top-level field but it's `string`, not a nested
    // schema. Trying to descend into it should fail.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("name.street"))
"#
    ));
    assert_has_code(&result, "D0030");
    // The failure is on `name` — we couldn't descend into a non-nested field.
    assert_message_contains(&result, "D0030", "name");
}

#[test]
fn deep_dotted_path_fails_at_the_first_missing_segment() {
    // Deep nesting; we mistype the deepest leaf. The diagnostic should
    // point at the innermost schema where the failure happened.
    let result = check(
        r#"
class Inner(Schema):
    leaf: "int"

class Middle(Schema):
    inner: Inner

class Outer(Schema):
    middle: Middle

def f(o: DataFrame[Outer]) -> DataFrame:
    return o.select(col("middle.inner.wrong"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "wrong");
    assert_message_contains(&result, "D0030", "Inner");
}

// ===========================================================================
// Unit-test the `resolve_path` helper directly
// ===========================================================================

mod resolve_path_unit {
    use dathon::schema::{FieldPathResult, SchemaView, resolve_path};

    #[test]
    fn derived_schema_with_dotted_path_always_fails() {
        // Derived schemas (results of select / agg / etc.) don't carry
        // nested-struct info — their fields are just names. Any dotted
        // path on a Derived view fails at the first segment.
        let view = SchemaView::derived_untyped(vec!["a", "b"]);
        let result = resolve_path(&view, "a.b", &[]);
        assert!(matches!(result, FieldPathResult::Missing { .. }));
    }

    #[test]
    fn derived_schema_with_flat_path_resolves_normally() {
        let view = SchemaView::derived_untyped(vec!["a", "b"]);
        let result = resolve_path(&view, "a", &[]);
        assert!(matches!(result, FieldPathResult::Resolved));
    }
}
