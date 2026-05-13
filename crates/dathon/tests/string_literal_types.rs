//! Iteration 41: column types are written as **string literals**.
//!
//!     class RawEvents(Schema):
//!         EventDate: "timestamp"
//!         City: "string"
//!         RecordDate: "date"
//!         EventCount: "int"
//!
//! The new shape pairs cleanly with Pylance / basedpyright: string
//! annotations are forward references, and with iteration 42's bundled
//! typeshed (or until then with the `dathon.py` stub on path) those
//! forward references resolve to real Python type aliases. dathon
//! itself recognizes the string contents as column-type names from
//! its small vocabulary.
//!
//! Bare-name annotations are still used for **nested-struct fields**
//! (`address: Address`) — those need to resolve to a declared Schema
//! class, not to a column-type name.

mod common;

use common::check;

#[test]
fn string_literal_column_types_resolve_against_the_atomic_vocabulary() {
    let src = r#"
class Orders(Schema):
    place_code: "int"
    price: "double"
    log_date: "timestamp"
    checkin: "date"
    is_active: "bool"
    label: "string"
    cnt: "long"
"#;
    let result = check(src);
    assert!(
        result.diagnostics.is_empty(),
        "expected clean run; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
    assert_eq!(result.schema_count, 1);
}

#[test]
fn unknown_string_literal_type_fires_D0010() {
    let src = r#"
class Orders(Schema):
    weird: "weirdtype"
"#;
    let result = check(src);
    assert!(result.has_code("D0010"));
}

#[test]
fn nested_struct_still_uses_bare_name_annotation() {
    let src = r#"
class Address(Schema):
    street: "string"
    city: "string"

class User(Schema):
    name: "string"
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
fn bare_name_atomic_type_no_longer_resolves() {
    // After iteration 41, atomic column types MUST be string literals.
    // The old bare-name form should fail.
    let src = r#"
class Orders(Schema):
    place_code: int
"#;
    let result = check(src);
    assert!(
        result.has_code("D0010"),
        "expected D0010 for bare-name int; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn column_ref_typo_fires_against_string_literal_schema() {
    // The downstream `col("priec")` check should still work — the
    // schema was built from string-literal annotations but the
    // resolved field vocabulary is the same.
    let src = r#"
class Orders(Schema):
    place_code: "int"
    price: "double"

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("priec"))
"#;
    let result = check(src);
    assert!(result.has_code("D0030"));
}
