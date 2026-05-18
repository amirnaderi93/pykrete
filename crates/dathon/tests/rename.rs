//! Integration tests for `rename` / `prepare_rename` — renaming a
//! column across its declaration and every reference.

use dathon::{Span, prepare_rename, rename};

fn cursor_at(source: &str, needle: &str) -> (usize, usize) {
    let idx = source.find(needle).expect("needle not found in source");
    let prefix = &source[..idx];
    let line = prefix.matches('\n').count() + 1;
    let line_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
    (line, idx - line_start + 1)
}

/// The source text a (single-line) span covers.
fn span_text(source: &str, span: Span) -> String {
    let line = source.split('\n').nth(span.start_line - 1).unwrap();
    line[span.start_column - 1..span.end_column - 1].to_string()
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
fn rename_finds_the_declaration_and_every_use() {
    let (line, col) = cursor_at(SRC, "col(\"amount\")");
    let spans = rename(SRC, line, col + 5);
    assert_eq!(spans.len(), 3, "expected decl + 2 uses, got {spans:?}");
}

#[test]
fn rename_spans_exclude_the_string_literal_quotes() {
    // Every editable span must cover only `amount`, not `"amount"` —
    // a rename replaces the identifier, leaving the quotes.
    let (line, col) = cursor_at(SRC, "col(\"amount\")");
    for span in rename(SRC, line, col + 5) {
        assert_eq!(span_text(SRC, span), "amount");
    }
}

#[test]
fn rename_covers_a_bare_string_column_argument() {
    for span in rename(SRC, cursor_at(SRC, "\"city\")").0, cursor_at(SRC, "\"city\")").1 + 1) {
        assert_eq!(span_text(SRC, span), "city");
    }
}

#[test]
fn rename_off_a_column_returns_empty() {
    let (line, col) = cursor_at(SRC, "def f");
    assert!(rename(SRC, line, col).is_empty());
}

#[test]
fn prepare_rename_accepts_a_column_and_rejects_other_positions() {
    let (line, col) = cursor_at(SRC, "col(\"amount\")");
    let prepared = prepare_rename(SRC, line, col + 5).expect("expected a rename span");
    assert_eq!(span_text(SRC, prepared), "amount");

    let (dline, dcol) = cursor_at(SRC, "def f");
    assert!(prepare_rename(SRC, dline, dcol).is_none());
}
