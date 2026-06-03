//! Integration tests for the "Did you mean 'X'?" suggestion that the
//! checker appends to `D0030` diagnostics for misspelled column names.
//!
//! The suggestion is also surfaced on the LSP side as a quick-fix code
//! action — those tests live in `crates/pykrete-lsp/src/lib.rs` under the
//! `handle_code_action` test block, since they exercise the LSP-layer
//! conversion.

mod common;

use common::check;

#[test]
fn close_typo_produces_did_you_mean_suggestion_and_carries_it_in_data() {
    let src = r#"
class Orders(Schema):
    price: int
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("prce"))
"#;
    let result = check(src);
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0030")
        .expect("expected a D0030");
    assert!(
        d.message.contains("Did you mean 'price'?"),
        "expected suggestion in message, got {:?}",
        d.message,
    );
    assert_eq!(d.suggestion.as_deref(), Some("price"));
}

#[test]
fn unrelated_name_does_not_produce_a_suggestion() {
    let src = r#"
class Orders(Schema):
    price: int
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("totally_unrelated_field_name"))
"#;
    let result = check(src);
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0030")
        .expect("expected a D0030");
    assert!(
        !d.message.contains("Did you mean"),
        "did not expect a suggestion for an unrelated name, got {:?}",
        d.message,
    );
    assert!(d.suggestion.is_none());
}

#[test]
fn suggestion_picks_the_closest_match_among_several_fields() {
    let src = r#"
class Orders(Schema):
    place_code: int
    plate_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("plac_code"))
"#;
    let result = check(src);
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0030")
        .expect("expected a D0030");
    // `plac_code` is distance 1 from `place_code` and distance 2 from
    // `plate_code` — Levenshtein picks the closer one.
    assert_eq!(d.suggestion.as_deref(), Some("place_code"));
}
