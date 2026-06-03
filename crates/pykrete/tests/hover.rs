//! Integration tests for the position-aware hover entry point.
//!
//! v0.1 covers three positions:
//! 1. Cursor on a Schema class declaration name.
//! 2. Cursor on a typed function declaration name.
//! 3. Cursor on a Schema reference (the `X` in `SparkFrame[X]` or a
//!    nested-struct field's annotation).
//!
//! Each integration test embeds a small `.pyk` source as a raw string,
//! locates a known `(line, column)` cursor position by reading the
//! source, and asserts about the markdown returned by `pykrete::hover`.

#![allow(non_snake_case)]

use pykrete::hover;

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
    // Schema with field names of varying length so the colon-alignment
    // padding is visible — a non-aligned implementation would put the
    // type names at different columns.
    let src = r#"
class RawOrders(Schema):
    order_id: int
    City: string
    amount: double
    quantity: int
    status: string
"#;
    let (line, col) = cursor_at(src, "RawOrders");
    let info = hover(src, line, col).expect("expected hover info");
    let md = &info.markdown;

    // Heading: bold "schema" followed by inline-code class name.
    assert!(
        md.contains("**schema** `RawOrders`"),
        "unexpected heading, got {md:?}",
    );

    // Body is a fenced python code block with a Schema class def.
    assert!(
        md.contains("```python"),
        "expected fenced python block, got {md:?}"
    );
    assert!(
        md.contains("class RawOrders(Schema):"),
        "missing class line, got {md:?}"
    );

    // Each field appears on its own indented line as `name: Type` with
    // the colon padded so the type column lines up. Longest field name
    // is `quantity` (8 chars), so shorter names get trailing spaces.
    assert!(
        md.contains("    order_id: Int"),
        "order_id row wrong, got {md:?}"
    );
    assert!(
        md.contains("    City:     String"),
        "City row wrong (alignment?), got {md:?}"
    );
    assert!(
        md.contains("    amount:   Double"),
        "amount row wrong (alignment?), got {md:?}"
    );
    assert!(
        md.contains("    quantity: Int"),
        "quantity row wrong, got {md:?}"
    );
    assert!(
        md.contains("    status:   String"),
        "status row wrong (alignment?), got {md:?}"
    );

    // Old bulleted format must be gone.
    assert!(
        !md.contains("Fields:"),
        "old 'Fields:' header still present: {md:?}"
    );
    assert!(!md.contains("- `"), "old bullet rows still present: {md:?}");
}

#[test]
fn hover_on_schema_with_nested_field_renders_the_nested_schema_name() {
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
    let md = &info.markdown;
    assert!(
        md.contains("class User(Schema):"),
        "missing class line, got {md:?}"
    );
    // Nested-struct field shows the nested schema's name as the type —
    // i.e. it reads as actual Python (`address: Address`), no extra tag.
    assert!(md.contains("address:"), "missing address field, got {md:?}");
    assert!(
        md.contains("Address"),
        "missing nested type name, got {md:?}"
    );
}

// ===========================================================================
// Case 2 — cursor on a typed function declaration name
// ===========================================================================

#[test]
fn hover_on_typed_function_name_returns_signature() {
    let src = r#"
class Orders(Schema):
    place_code: int

def prepare_orders(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw
"#;
    let (line, col) = cursor_at(src, "prepare_orders(");
    let info = hover(src, line, col).expect("expected hover info");
    assert!(info.markdown.contains("fn"));
    assert!(info.markdown.contains("prepare_orders"));
    assert!(info.markdown.contains("SparkFrame[Orders]"));
    assert!(info.markdown.contains("->"));
}

#[test]
fn hover_on_untyped_function_name_returns_nothing() {
    // No DataFrame annotations → not a "typed" function in pykrete's
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
    // The `Orders` inside `SparkFrame[Orders]` is a Schema reference.
    // Hovering on it should show Orders' field list.
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
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

// ===========================================================================
// Case 4 — cursor on a `col("foo")` string literal in a function body
// ===========================================================================

#[test]
fn hover_on_col_literal_returns_the_columns_type() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("price"))
"#;
    // Cursor inside the string literal `"price"`. cursor_at finds the
    // opening quote; +1 puts us safely inside the literal.
    let (line, col) = cursor_at(src, "\"price\"");
    let info = hover(src, line, col + 1).expect("expected hover info");
    assert!(info.markdown.contains("price"));
    assert!(info.markdown.contains("Orders"));
    assert!(
        info.markdown.to_lowercase().contains("int"),
        "expected the column's type in the hover, got {:?}",
        info.markdown,
    );
}

#[test]
fn hover_on_col_literal_for_a_missing_column_says_so() {
    let src = r#"
class Orders(Schema):
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("priec"))
"#;
    let (line, col) = cursor_at(src, "\"priec\"");
    let info = hover(src, line, col + 1).expect("expected hover info");
    assert!(info.markdown.contains("priec"));
    assert!(info.markdown.contains("does not exist"));
}

#[test]
fn hover_on_col_literal_after_filter_resolves_against_the_input_schema() {
    // .filter() preserves the schema, so col() args inside a chain of
    // .filter() still resolve against the input.
    let src = r#"
class Orders(Schema):
    price: int
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.filter(col("price") > 0).select(col("place_code"))
"#;
    let (line, col) = cursor_at(src, "\"place_code\"");
    let info = hover(src, line, col + 1).expect("expected hover info");
    assert!(info.markdown.contains("place_code"));
    assert!(info.markdown.contains("Orders"));
}

// ===========================================================================
// Case 5 — cursor on a local-variable DataFrame binding
// ===========================================================================

#[test]
fn hover_on_lhs_of_assignment_shows_the_inferred_schema() {
    // `x = raw.select(...)` — hovering on the `x` should show what the
    // value evaluates to (here, the inferred schema with fields `price`
    // and `place_code` because raw is Orders and we select both fields).
    let src = r#"
class Orders(Schema):
    price: int
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    x = raw.select(col("price"), col("place_code"))
    return x
"#;
    let (line, col) = cursor_at(src, "x = raw");
    let info = hover(src, line, col).expect("expected hover info");
    assert!(info.markdown.contains("local"));
    assert!(info.markdown.contains("x"));
    // For Declared receivers carried through filter/where, we'd get the
    // schema name. Here select produces a Derived view — the popup
    // mentions the inferred fields instead.
    assert!(
        info.markdown.contains("price") || info.markdown.contains("place_code"),
        "expected the popup to surface the schema's fields, got {:?}",
        info.markdown,
    );
}

#[test]
fn hover_on_use_of_local_binding_resolves_through_the_assignment() {
    // The cursor is on the `x` in `return x` — a use, not the
    // declaration. Hover should still resolve through to the binding.
    let src = r#"
class Orders(Schema):
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    x = raw.filter(col("price") > 0)
    return x
"#;
    let idx = src.rfind("return x").unwrap() + "return ".len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() + 1;
    let col = idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;
    let info = hover(src, line, col).expect("expected hover info");
    assert!(info.markdown.contains("local"));
    assert!(info.markdown.contains("x"));
    // `.filter()` preserves the Declared receiver, so the popup mentions
    // the Orders schema name.
    assert!(info.markdown.contains("Orders"));
}

#[test]
fn hover_on_local_binding_with_typed_annotation_shows_the_declared_schema() {
    // `x: SparkFrame[Orders] = ...` — the annotation is authoritative;
    // hover should describe Orders' fields regardless of what the RHS
    // evaluates to.
    let src = r#"
class Orders(Schema):
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    x: SparkFrame[Orders] = raw.select(col("price"))
    return x
"#;
    let (line, col) = cursor_at(src, "x: SparkFrame");
    let info = hover(src, line, col).expect("expected hover info");
    assert!(info.markdown.contains("Orders"));
    assert!(info.markdown.contains("price"));
}
