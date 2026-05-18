//! Integration tests for `references` — find every use of a column.

use dathon::references;

fn cursor_at(source: &str, needle: &str) -> (usize, usize) {
    let idx = source.find(needle).expect("needle not found in source");
    let prefix = &source[..idx];
    let line = prefix.matches('\n').count() + 1;
    let line_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
    (line, idx - line_start + 1)
}

const SRC: &str = "\
class Orders(Schema):
    amount: \"int\"
    city: \"string\"

def f(raw: DataFrame[Orders]) -> DataFrame:
    a = raw.select(col(\"amount\"), \"city\")
    return a.filter(col(\"amount\") > 0)
";

#[test]
fn references_from_a_col_literal_finds_all_uses_and_the_declaration() {
    // Cursor inside the first `col("amount")` — `amount` is declared
    // once and referenced by two `col("amount")` calls.
    let (line, col) = cursor_at(SRC, "\"amount\"");
    let refs = references(SRC, line, col + 1);
    assert_eq!(refs.len(), 3, "expected decl + 2 uses, got {refs:?}");
}

#[test]
fn references_from_the_field_declaration_finds_the_uses() {
    // Cursor on the `amount` token of the `amount: "int"` declaration.
    let (line, col) = cursor_at(SRC, "amount: \"int\"");
    let refs = references(SRC, line, col + 1);
    assert_eq!(refs.len(), 3);
}

#[test]
fn references_from_a_bare_string_column_argument() {
    // `"city"` — declared once, used once as a bare-string `select` arg.
    let src = "\
class Orders(Schema):
    amount: \"int\"
    city: \"string\"

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.select(\"city\")
";
    let (line, col) = cursor_at(src, "\"city\")");
    let refs = references(src, line, col + 1);
    assert_eq!(refs.len(), 2);
}

#[test]
fn references_off_a_column_returns_empty() {
    // Cursor in the `def` keyword — not on any column.
    let (line, col) = cursor_at(SRC, "def f");
    assert!(references(SRC, line, col).is_empty());
}

#[test]
fn references_on_unparseable_source_returns_empty() {
    assert!(references("def broken(:\n", 1, 1).is_empty());
}
