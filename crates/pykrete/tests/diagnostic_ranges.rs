//! Integration tests verifying that diagnostics carry an end position
//! covering the offending token, not just a start point.
//!
//! Editors render the squiggle from the diagnostic's start to its end.
//! Before iteration 26, end equalled start, so column-name errors on
//! `col("BadName")` only underlined the opening quote. These tests pin
//! the behaviour: the range must span the entire string literal so the
//! whole `"BadName"` is highlighted.

mod common;

use common::check;

#[test]
fn column_name_error_range_covers_the_full_string_literal() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("BadName"))
"#;
    let result = check(src);
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0030")
        .expect("expected a D0030");
    // The literal `"BadName"` is 9 characters wide (including both quotes).
    // start.column points at the opening `"`; end.column should sit one
    // past the closing `"` — i.e. start.column + 9.
    assert_eq!(d.line, d.end_line);
    assert_eq!(
        d.end_column - d.column,
        "\"BadName\"".len(),
        "diagnostic range should span the whole \"BadName\" literal \
         (start {}, end {})",
        d.column,
        d.end_column,
    );
}

#[test]
fn unknown_schema_error_range_is_a_non_trivial_span() {
    // D0020 anchors on the whole `SparkFrame[X]` annotation expression —
    // we don't bother narrowing to just the bad schema name. That's fine
    // for the LSP: any non-zero range produces a real squiggle.
    let src = r#"
def f(raw: SparkFrame[Missing]) -> SparkFrame[Missing]:
    return raw
"#;
    let result = check(src);
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0020")
        .expect("expected a D0020");
    assert_eq!(d.line, d.end_line);
    assert!(
        d.end_column > d.column,
        "expected a non-zero-width range, got start={}, end={}",
        d.column,
        d.end_column,
    );
}

#[test]
fn join_key_error_range_covers_the_full_string_literal() {
    let src = r#"
class A(Schema):
    a: int

class B(Schema):
    b: int

def f(left: SparkFrame[A], right: SparkFrame[B]) -> SparkFrame[A]:
    return left.join(right, on="missing")
"#;
    let result = check(src);
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0060")
        .expect("expected a D0060");
    assert_eq!(d.line, d.end_line);
    assert_eq!(
        d.end_column - d.column,
        "\"missing\"".len(),
        "join-key diagnostic range should span the whole \"missing\" \
         literal (start {}, end {})",
        d.column,
        d.end_column,
    );
}
