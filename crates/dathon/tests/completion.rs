//! Integration tests for `completion`.
//!
//! Each test picks a known cursor position inside a small `.dpy` source
//! (using the same `cursor_at` helper as the hover/symbols tests) and
//! asserts on the labels of the returned completion items.

#![allow(non_snake_case)]

use dathon::{CompletionItemKind, completions};

fn cursor_at(source: &str, needle: &str) -> (usize, usize) {
    let idx = source.find(needle).expect("needle not found in source");
    let prefix = &source[..idx];
    let line = prefix.matches('\n').count() + 1;
    let line_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let column = idx - line_start + 1;
    (line, column)
}

fn labels(items: &[dathon::CompletionItem]) -> Vec<&str> {
    items.iter().map(|i| i.label.as_str()).collect()
}

// ===========================================================================
// Surface 1: DataFrame[…] → schema names
// ===========================================================================

#[test]
fn completions_inside_dataframe_subscript_list_schema_classes() {
    let src = r#"
class Orders(Schema):
    x: "int"

class Returns(Schema):
    y: "int"

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw
"#;
    // Cursor on the `Orders` inside `DataFrame[Orders]` on the def line —
    // we expect completions for the schema slot, which is every declared
    // Schema in this module.
    let first = src.find("Orders").unwrap();
    let second = src[first + 1..].find("Orders").unwrap() + first + 1;
    let prefix = &src[..second];
    let line = prefix.matches('\n').count() + 1;
    let col = second - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;

    let items = completions(src, line, col);
    let mut names = labels(&items);
    names.sort();
    assert_eq!(names, vec!["Orders", "Returns"]);
    for item in &items {
        assert_eq!(item.kind, CompletionItemKind::Class);
        assert_eq!(item.detail.as_deref(), Some("Schema"));
    }
}

// ===========================================================================
// Surface 2: col("…") → column names from the surrounding schema
// ===========================================================================

#[test]
fn completions_inside_col_literal_list_columns_of_the_active_schema() {
    let src = r#"
class Orders(Schema):
    place_code: "int"
    price: "int"

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code"))
"#;
    let (line, col) = cursor_at(src, "\"place_code\"");
    // +1 puts the cursor INSIDE the string literal, not on the opening
    // quote — that's where typing actually happens.
    let items = completions(src, line, col + 1);
    let mut names = labels(&items);
    names.sort();
    assert_eq!(names, vec!["place_code", "price"]);
    for item in &items {
        assert_eq!(item.kind, CompletionItemKind::Field);
    }
}

#[test]
fn completions_for_col_field_carry_the_column_type_in_detail() {
    let src = r#"
class Orders(Schema):
    place_code: "int"
    price: "int"

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code"))
"#;
    let (line, col) = cursor_at(src, "\"place_code\"");
    let items = completions(src, line, col + 1);
    let price = items
        .iter()
        .find(|i| i.label == "price")
        .expect("expected `price` in completions");
    assert!(
        price
            .detail
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains("int"),
        "expected detail to carry the column type, got {:?}",
        price.detail,
    );
}

// ===========================================================================
// Surface 3: df.<cursor> → column names from df's schema
// ===========================================================================

#[test]
fn completions_after_df_dot_list_columns_of_the_param_schema() {
    let src = r#"
class Orders(Schema):
    place_code: "int"
    price: "int"

def f(raw: DataFrame[Orders]) -> int:
    return raw.place_code
"#;
    // Cursor on `place_code` after `raw.` — the attr identifier.
    let idx = src.find("raw.place_code").unwrap() + "raw.".len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() + 1;
    let col = idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;
    let items = completions(src, line, col);
    let mut names = labels(&items);
    names.sort();
    assert_eq!(names, vec!["place_code", "price"]);
}

#[test]
fn completions_after_local_binding_dot_resolves_through_the_assignment() {
    // `x = raw.filter(...)` — typing `x.<cursor>` should suggest the
    // Orders fields because filter preserves the receiver's schema.
    let src = r#"
class Orders(Schema):
    place_code: "int"
    price: "int"

def f(raw: DataFrame[Orders]) -> int:
    x = raw.filter(col("price") > 0)
    return x.place_code
"#;
    let idx = src.find("x.place_code").unwrap() + "x.".len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() + 1;
    let col = idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;
    let items = completions(src, line, col);
    let mut names = labels(&items);
    names.sort();
    assert_eq!(names, vec!["place_code", "price"]);
}

#[test]
fn completions_after_annotated_local_binding_use_the_annotation_schema() {
    // `x: DataFrame[Orders] = ...` — the annotation is authoritative,
    // so completion on `x.` lists Orders' fields with column types.
    let src = r#"
class Orders(Schema):
    place_code: "int"
    price: "int"

def f(raw: DataFrame[Orders]) -> int:
    x: DataFrame[Orders] = raw.select(col("price"))
    return x.place_code
"#;
    let idx = src.find("x.place_code").unwrap() + "x.".len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() + 1;
    let col = idx - prefix.rfind('\n').map(|p| p + 1).unwrap_or(0) + 1;
    let items = completions(src, line, col);
    let mut names = labels(&items);
    names.sort();
    assert_eq!(names, vec!["place_code", "price"]);
}

// ===========================================================================
// Negative space
// ===========================================================================

#[test]
fn completions_outside_recognized_surface_returns_empty() {
    let src = r#"
class Orders(Schema):
    x: "int"
"#;
    // Cursor in whitespace before any declaration.
    assert!(completions(src, 1, 1).is_empty());
}

#[test]
fn completions_on_unparseable_source_returns_empty() {
    let src = "def broken(:\n";
    assert!(completions(src, 1, 1).is_empty());
}
