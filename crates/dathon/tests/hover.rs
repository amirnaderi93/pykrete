//! Integration tests for the position-aware hover entry point.
//!
//! v0.1 covers three positions:
//! 1. Cursor on a Schema class declaration name.
//! 2. Cursor on a typed function declaration name.
//! 3. Cursor on a Schema reference (the `X` in `DataFrame[X]` or a
//!    nested-struct field's annotation).
//!
//! Each integration test embeds a small `.dpy` source as a raw string,
//! locates a known `(line, column)` cursor position by reading the
//! source, and asserts about the markdown returned by `dathon::hover`.

#![allow(non_snake_case)]

use dathon::hover;

/// Locate `needle` in `source` and return its (line, column) 1-indexed,
/// pointing at the start of the needle. Panics if the needle isn't found
/// — tests that depend on a particular cursor position should fail loudly
/// when the source diverges from the test's expectations.
fn cursor_at(source: &str, needle: &str) -> (usize, usize) {
    let idx = source.find(needle).expect("needle not found in source");
    let prefix = &source[..idx];
    let line = prefix.matches('\n').count() + 1;
    let line_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let column = idx - line_start + 1;
    (line, column)
}

// ===========================================================================
// Case 1 — cursor on a Schema class declaration name
// ===========================================================================

#[test]
fn hover_on_schema_class_name_returns_fields() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int
"#;
    let (line, col) = cursor_at(src, "Orders");
    let info = hover(src, line, col).expect("expected hover info");
    assert!(info.markdown.contains("schema"));
    assert!(info.markdown.contains("Orders"));
    assert!(info.markdown.contains("place_code"));
    assert!(info.markdown.contains("price"));
    assert!(info.markdown.contains("Int"));
}

#[test]
fn hover_on_schema_with_nested_field_marks_the_field_as_nested() {
    let src = r#"
class Address(Schema):
    street: string

class User(Schema):
    name: string
    address: Address
"#;
    let (line, col) = cursor_at(src, "class User");
    // Cursor on "User" (skip past "class ")
    let info = hover(src, line, col + "class ".len()).expect("expected hover info");
    assert!(info.markdown.contains("address"));
    assert!(info.markdown.contains("nested"));
}

// ===========================================================================
// Case 2 — cursor on a typed function declaration name
// ===========================================================================

#[test]
fn hover_on_typed_function_name_returns_signature() {
    let src = r#"
class Orders(Schema):
    place_code: int

def prepare_orders(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw
"#;
    let (line, col) = cursor_at(src, "prepare_orders(");
    let info = hover(src, line, col).expect("expected hover info");
    assert!(info.markdown.contains("fn"));
    assert!(info.markdown.contains("prepare_orders"));
    assert!(info.markdown.contains("DataFrame[Orders]"));
    assert!(info.markdown.contains("->"));
}

#[test]
fn hover_on_untyped_function_name_returns_nothing() {
    // No DataFrame annotations → not a "typed" function in dathon's
    // sense; we don't generate hover info for it.
    let src = r#"
def regular_helper(x: int) -> int:
    return x
"#;
    let (line, col) = cursor_at(src, "regular_helper(");
    assert!(hover(src, line, col).is_none());
}

// ===========================================================================
// Case 3 — cursor on a Schema reference
// ===========================================================================

#[test]
fn hover_on_DataFrame_inner_schema_returns_that_schemas_info() {
    // The `Orders` inside `DataFrame[Orders]` is a Schema reference.
    // Hovering on it should show Orders' field list.
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw
"#;
    // The first "Orders" in the file is the class definition; the second
    // is the param annotation. Aim cursor at the second.
    let first_idx = src.find("Orders").unwrap();
    let second_idx = src[first_idx + 1..].find("Orders").unwrap() + first_idx + 1;
    let prefix = &src[..second_idx];
    let line = prefix.matches('\n').count() + 1;
    let col = second_idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;

    let info = hover(src, line, col).expect("expected hover info");
    assert!(info.markdown.contains("Orders"));
    assert!(info.markdown.contains("place_code"));
    assert!(info.markdown.contains("price"));
}

#[test]
fn hover_on_nested_schema_field_type_returns_the_nested_schemas_info() {
    let src = r#"
class Address(Schema):
    street: string
    city: string

class User(Schema):
    name: string
    address: Address
"#;
    // The `Address` after "address: " is the bare-name annotation of
    // the nested-struct field. Hover should describe Address.
    let idx = src.find("address: Address").unwrap();
    let address_idx = idx + "address: ".len();
    let prefix = &src[..address_idx];
    let line = prefix.matches('\n').count() + 1;
    let col = address_idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;

    let info = hover(src, line, col).expect("expected hover info");
    assert!(info.markdown.contains("Address"));
    assert!(info.markdown.contains("street"));
    assert!(info.markdown.contains("city"));
}

// ===========================================================================
// Negative space
// ===========================================================================

#[test]
fn hover_on_whitespace_returns_nothing() {
    let src = r#"
class Orders(Schema):
    x: int
"#;
    // Position at column 1 on line 1 (the blank line at the start).
    assert!(hover(src, 1, 1).is_none());
}

#[test]
fn hover_on_unparseable_source_returns_nothing() {
    // Parse errors should not panic — just return None.
    let src = "def broken(:\n";
    assert!(hover(src, 1, 1).is_none());
}

#[test]
fn hover_outside_of_a_recognized_position_returns_nothing() {
    // Cursor on a column annotation (`int`) — not yet a hover target.
    let src = r#"
class Orders(Schema):
    place_code: int
"#;
    let (line, col) = cursor_at(src, "int");
    assert!(hover(src, line, col).is_none());
}
