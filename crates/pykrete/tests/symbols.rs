//! Integration tests for `document_symbols` and `definition`.
//!
//! `document_symbols` is exercised by feeding a small `.pyk` source in and
//! asserting on the resulting outline shape (top-level entries, schema-field
//! children, typed-function signatures).
//!
//! `definition` is exercised by locating a known cursor position with the
//! `cursor_at` helper (copied from `hover.rs` — same convention) and
//! asserting the returned `Span` matches the declaration site.

#![allow(non_snake_case)]

use pykrete::{Span, SymbolKind, definition, document_symbols};

fn cursor_at(source: &str, needle: &str) -> (usize, usize) {
    let idx = source.find(needle).expect("needle not found in source");
    let prefix = &source[..idx];
    let line = prefix.matches('\n').count() + 1;
    let line_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let column = idx - line_start + 1;
    (line, column)
}

// ===========================================================================
// document_symbols
// ===========================================================================

#[test]
fn document_symbols_lists_top_level_schemas_and_functions() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def prepare_orders(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw
"#;
    let syms = document_symbols(src);
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Orders", "prepare_orders"]);
}

#[test]
fn document_symbols_schema_class_carries_fields_as_children() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int
"#;
    let syms = document_symbols(src);
    let orders = syms.first().expect("expected a symbol");
    assert_eq!(orders.kind, SymbolKind::Class);
    assert_eq!(orders.detail.as_deref(), Some("Schema"));
    let child_names: Vec<&str> = orders.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(child_names, vec!["place_code", "price"]);
    for child in &orders.children {
        assert_eq!(child.kind, SymbolKind::Field);
    }
}

#[test]
fn document_symbols_schema_field_detail_shows_column_type() {
    let src = r#"
class Orders(Schema):
    place_code: int
"#;
    let syms = document_symbols(src);
    let field = &syms[0].children[0];
    // ColumnType::as_str renders Int — we don't pin the exact casing
    // beyond it containing "Int" so renaming the formatter doesn't
    // require updating every test.
    assert!(
        field
            .detail
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains("int"),
        "expected the field detail to mention its column type, got {:?}",
        field.detail,
    );
}

#[test]
fn document_symbols_typed_function_detail_shows_signature() {
    let src = r#"
class Orders(Schema):
    x: int

def prepare_orders(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw
"#;
    let syms = document_symbols(src);
    let func = syms
        .iter()
        .find(|s| s.name == "prepare_orders")
        .expect("expected the function symbol");
    assert_eq!(func.kind, SymbolKind::Function);
    let detail = func.detail.as_deref().unwrap_or("");
    assert!(detail.contains("SparkFrame[Orders]"));
    assert!(detail.contains("->"));
}

#[test]
fn document_symbols_untyped_function_has_no_signature_detail() {
    let src = r#"
def helper(x):
    return x
"#;
    let syms = document_symbols(src);
    let func = &syms[0];
    assert_eq!(func.name, "helper");
    assert_eq!(func.kind, SymbolKind::Function);
    assert!(
        func.detail.is_none(),
        "expected no signature detail for an untyped function, got {:?}",
        func.detail,
    );
}

#[test]
fn document_symbols_non_schema_class_has_no_field_children() {
    // A bare `class Foo:` (no Schema base) shouldn't be treated as a
    // schema — outline shows it but doesn't expand fields.
    let src = r#"
class Helper:
    x: int
"#;
    let syms = document_symbols(src);
    let class = &syms[0];
    assert_eq!(class.name, "Helper");
    assert!(class.detail.is_none());
    assert!(class.children.is_empty());
}

#[test]
fn document_symbols_returns_empty_on_parse_error() {
    // Mid-edit syntax errors shouldn't strip the outline; we just return
    // an empty list rather than panicking.
    let src = "def broken(:\n";
    let syms = document_symbols(src);
    assert!(syms.is_empty());
}

#[test]
fn document_symbols_class_selection_range_covers_only_the_name() {
    let src = r#"
class Orders(Schema):
    x: int
"#;
    let syms = document_symbols(src);
    let cls = &syms[0];
    // The selection range is what the editor highlights — it must be
    // narrower than the full class body range.
    let sel_width = (cls.selection_range.end_column - cls.selection_range.start_column) as usize;
    assert_eq!(sel_width, "Orders".len());
    assert_eq!(cls.selection_range.start_line, cls.selection_range.end_line);
}

// ===========================================================================
// definition
// ===========================================================================

fn assert_span_points_to(span: Span, source: &str, needle: &str) {
    let (line, col) = cursor_at(source, needle);
    assert_eq!(
        (span.start_line, span.start_column),
        (line, col),
        "expected the definition span to start at {:?} but got {:?}",
        (line, col),
        (span.start_line, span.start_column),
    );
}

#[test]
fn definition_on_DataFrame_inner_schema_jumps_to_its_class() {
    let src = r#"
class Orders(Schema):
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw
"#;
    // Cursor on the second occurrence of "Orders" — the one in
    // `SparkFrame[Orders]`, not the class declaration.
    let first = src.find("Orders").unwrap();
    let second = src[first + 1..].find("Orders").unwrap() + first + 1;
    let prefix = &src[..second];
    let line = prefix.matches('\n').count() + 1;
    let col = second - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;

    let span = definition(src, line, col).expect("expected a definition");
    // The definition should point at the `Orders` token in `class Orders(Schema)`.
    assert_span_points_to(span, src, "Orders");
}

#[test]
fn definition_on_nested_schema_field_annotation_jumps_to_that_class() {
    let src = r#"
class Address(Schema):
    street: string

class User(Schema):
    name: string
    address: Address
"#;
    // Cursor on the `Address` after `address: ` (the field annotation,
    // not the class declaration).
    let idx = src.find("address: Address").unwrap() + "address: ".len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() + 1;
    let col = idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;

    let span = definition(src, line, col).expect("expected a definition");
    // Should land on `class Address(Schema):` — the first occurrence
    // of "Address" in the source.
    assert_span_points_to(span, src, "Address");
}

#[test]
fn definition_on_a_schema_nested_in_an_array_subscript_jumps_to_that_class() {
    let src = r#"
class Event(Schema):
    id: int

class Log(Schema):
    events: Array[Event]
"#;
    // Cursor on the `Event` inside `Array[Event]`.
    let idx = src.find("Array[Event]").unwrap() + "Array[".len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() + 1;
    let col = idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;

    let span = definition(src, line, col).expect("expected a definition");
    assert_span_points_to(span, src, "Event");
}

#[test]
fn definition_on_schema_class_declaration_name_returns_itself() {
    let src = r#"
class Orders(Schema):
    x: int
"#;
    let (line, col) = cursor_at(src, "Orders");
    let span = definition(src, line, col).expect("expected a definition");
    assert_span_points_to(span, src, "Orders");
}

#[test]
fn definition_on_whitespace_returns_none() {
    let src = r#"
class Orders(Schema):
    x: int
"#;
    assert!(definition(src, 1, 1).is_none());
}

#[test]
fn definition_on_unknown_schema_reference_returns_none() {
    // The schema name in SparkFrame[Missing] doesn't resolve — there's
    // no class to jump to.
    let src = r#"
def f(raw: SparkFrame[Missing]) -> SparkFrame[Missing]:
    return raw
"#;
    let (line, col) = cursor_at(src, "Missing");
    assert!(definition(src, line, col).is_none());
}

#[test]
fn definition_on_unparseable_source_returns_none() {
    let src = "def broken(:\n";
    assert!(definition(src, 1, 1).is_none());
}

// ===========================================================================
// definition — `col("foo")` references (body-context aware)
// ===========================================================================

#[test]
fn definition_on_col_literal_jumps_to_field_name_in_schema_class() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("price"))
"#;
    // Cursor inside the string literal `"price"` (one past the opening
    // quote). The definition should land on `price` in the class body.
    let idx = src.find("col(\"price\")").unwrap() + "col(\"".len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() + 1;
    let col = idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;

    let span = definition(src, line, col).expect("expected a definition");

    // The expected target: the `price` token in `price: int` inside the
    // class body — the *first* occurrence of "price" (the field's
    // target name).
    let target_idx = src.find("price: int").unwrap();
    let tprefix = &src[..target_idx];
    let expected_line = tprefix.matches('\n').count() + 1;
    let expected_col = target_idx - tprefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;
    assert_eq!(
        (span.start_line, span.start_column),
        (expected_line, expected_col),
        "definition should jump to the field's name token",
    );
}

#[test]
fn definition_on_col_literal_for_missing_column_returns_none() {
    let src = r#"
class Orders(Schema):
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("priec"))
"#;
    let idx = src.find("col(\"priec\")").unwrap() + "col(\"".len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() + 1;
    let col = idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;
    assert!(definition(src, line, col).is_none());
}
