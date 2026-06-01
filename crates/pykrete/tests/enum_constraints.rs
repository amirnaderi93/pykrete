//! v1.1 enum constraints — parser, ColumnType representation, schema
//! operators, and D0084 reservation (PR-A scope only).
//!
//! Check-site emission of D0084 — `==`, `.isin`, `.fillna`, `lit`,
//! `F.expr`, and branch-form expressions — lands in PR-B. These tests
//! pin down what PR-A delivers: the type-system foundation that PR-B
//! plugs into.

mod common;

use common::{assert_has_code, assert_message_contains, assert_no_diagnostics, check};

use pykrete::diagnostics::DIAGNOSTIC_CATALOG;
use pykrete::types::{ColumnType, EnumParseError, render_enum_vocab};

// ---------------------------------------------------------------------------
// Schema parser — `enum["a", "b", ...]` as a type annotation
// ---------------------------------------------------------------------------

#[test]
fn schema_declares_enum_with_two_values() {
    let src = "\
class Order(Schema):
    id: long
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn schema_declares_enum_with_a_single_value() {
    let src = "\
class Order(Schema):
    status: enum[\"only\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn empty_enum_vocabulary_is_rejected_via_the_enum_from_values_api() {
    // `enum[]` is a Python syntax error (the subscript can't be empty),
    // so D0001 fires before pykrete sees a subscript shape. The
    // `EnumParseError::Empty` guard exists for the type-level API where
    // the values vector can be empty in principle; it is the spec-named
    // safety net per the "rejected at parse time" guarantee.
    assert_eq!(
        ColumnType::enum_from_values(Vec::new()),
        Err(EnumParseError::Empty),
    );
}

#[test]
fn duplicate_enum_entry_fires_d0011_and_names_the_duplicate() {
    let src = "\
class Order(Schema):
    status: enum[\"pending\", \"pending\"]
";
    let result = check(src);
    assert_has_code(&result, "D0011");
    assert_message_contains(&result, "D0011", "pending");
    assert_message_contains(&result, "D0011", "unique");
}

#[test]
fn enum_values_are_case_sensitive() {
    // `enum["Pending"]` and `enum["pending"]` are distinct vocabularies —
    // no normalization happens at parse time per the spec.
    let values_a = vec!["Pending".to_string()];
    let values_b = vec!["pending".to_string()];
    let a = ColumnType::enum_from_values(values_a).unwrap();
    let b = ColumnType::enum_from_values(values_b).unwrap();
    assert_ne!(a, b, "enum vocabularies must be byte-exact");
    assert!(!ColumnType::enum_vocab_eq(&a, &b));
}

#[test]
fn enum_values_preserve_unicode_and_whitespace_exactly() {
    let values = vec![
        "café".to_string(),
        "us-east".to_string(),
        "pending ".to_string(),
    ];
    let ct = ColumnType::enum_from_values(values.clone()).unwrap();
    let ColumnType::Enum(actual) = ct else {
        panic!("expected Enum variant");
    };
    assert_eq!(actual, values, "raw byte-for-byte preservation expected");
}

#[test]
fn enum_from_values_rejects_empty() {
    assert_eq!(
        ColumnType::enum_from_values(Vec::new()),
        Err(EnumParseError::Empty),
    );
}

#[test]
fn enum_from_values_rejects_duplicate_and_names_first_repeat() {
    assert_eq!(
        ColumnType::enum_from_values(vec!["a".into(), "b".into(), "a".into()]),
        Err(EnumParseError::Duplicate("a".into())),
    );
}

// ---------------------------------------------------------------------------
// Set-equality on enum vocabularies — Q4 / Q9 semantics
// ---------------------------------------------------------------------------

#[test]
fn enum_vocab_eq_ignores_declaration_order() {
    let a = ColumnType::enum_from_values(vec!["a".into(), "b".into(), "c".into()]).unwrap();
    let b = ColumnType::enum_from_values(vec!["c".into(), "a".into(), "b".into()]).unwrap();
    assert!(ColumnType::enum_vocab_eq(&a, &b));
}

#[test]
fn enum_vocab_eq_is_false_when_one_side_has_an_extra_value() {
    let a = ColumnType::enum_from_values(vec!["a".into(), "b".into()]).unwrap();
    let b = ColumnType::enum_from_values(vec!["a".into(), "b".into(), "c".into()]).unwrap();
    assert!(!ColumnType::enum_vocab_eq(&a, &b));
}

#[test]
fn enum_vocab_eq_peels_nullable_wrappers() {
    let inner_a = ColumnType::enum_from_values(vec!["a".into(), "b".into()]).unwrap();
    let inner_b = ColumnType::enum_from_values(vec!["b".into(), "a".into()]).unwrap();
    let a = ColumnType::Nullable(Box::new(inner_a));
    let b = ColumnType::Nullable(Box::new(inner_b));
    assert!(ColumnType::enum_vocab_eq(&a, &b));
}

#[test]
fn enum_vocab_eq_is_false_against_a_non_enum_type() {
    let enum_ty = ColumnType::enum_from_values(vec!["a".into()]).unwrap();
    assert!(!ColumnType::enum_vocab_eq(&enum_ty, &ColumnType::String));
}

// ---------------------------------------------------------------------------
// Hover / completion rendering — truncate-to-5
// ---------------------------------------------------------------------------

#[test]
fn render_enum_vocab_inlines_all_values_up_to_five() {
    let values: Vec<String> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        render_enum_vocab(&values),
        "\"a\", \"b\", \"c\", \"d\", \"e\""
    );
}

#[test]
fn render_enum_vocab_truncates_past_five_values_with_more_suffix() {
    let values: Vec<String> = ["a", "b", "c", "d", "e", "f", "g", "h"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        render_enum_vocab(&values),
        "\"a\", \"b\", \"c\", \"d\", \"e\", ... (3 more)",
    );
}

#[test]
fn enum_display_format_uses_the_shared_render_helper() {
    let ct = ColumnType::enum_from_values(vec!["a".into(), "b".into()]).unwrap();
    assert_eq!(format!("{ct}"), "enum[\"a\", \"b\"]");
}

#[test]
fn enum_as_str_is_the_bare_kind_name() {
    // Used by symbol/outline renders that don't want the inline
    // vocabulary; hover and completion go through the full render
    // helper instead.
    let ct = ColumnType::enum_from_values(vec!["a".into()]).unwrap();
    assert_eq!(ct.as_str(), "Enum");
}

// ---------------------------------------------------------------------------
// Schema operators — Pick / Omit / Merge carry-through
// ---------------------------------------------------------------------------

#[test]
fn pick_preserves_enum_constraint_on_kept_column() {
    let src = "\
class Order(Schema):
    id: long
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Pick[Order, \"status\"]]) -> DataFrame:
    return orders.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn omit_preserves_enum_constraint_on_surviving_column() {
    let src = "\
class Order(Schema):
    id: long
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Omit[Order, \"id\"]]) -> DataFrame:
    return orders.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn merge_with_set_equal_enum_vocabularies_is_silent() {
    // Declaration order differs; the vocabulary set is the same. Per Q4,
    // no D0040 fires.
    let src = "\
class A(Schema):
    status: enum[\"pending\", \"shipped\"]

class B(Schema):
    status: enum[\"shipped\", \"pending\"]

def f(d: DataFrame[Merge[A, B]]) -> DataFrame:
    return d.select(col(\"status\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn merge_with_non_set_equal_enum_vocabularies_fires_d0040() {
    let src = "\
class A(Schema):
    status: enum[\"pending\", \"shipped\"]

class B(Schema):
    status: enum[\"pending\", \"delivered\"]

def f(d: DataFrame[Merge[A, B]]) -> DataFrame:
    return d
";
    let result = check(src);
    assert_has_code(&result, "D0040");
    assert_message_contains(&result, "D0040", "status");
}

#[test]
fn merge_with_enum_vs_plain_string_fires_d0040() {
    let src = "\
class A(Schema):
    status: enum[\"pending\", \"shipped\"]

class B(Schema):
    status: string

def f(d: DataFrame[Merge[A, B]]) -> DataFrame:
    return d
";
    let result = check(src);
    assert_has_code(&result, "D0040");
    assert_message_contains(&result, "D0040", "status");
}

#[test]
fn merge_with_only_one_side_declaring_status_is_silent() {
    let src = "\
class A(Schema):
    status: enum[\"pending\", \"shipped\"]
    amount: int

class B(Schema):
    rating: int

def f(d: DataFrame[Merge[A, B]]) -> DataFrame:
    return d.select(col(\"status\"), col(\"amount\"), col(\"rating\"))
";
    assert_no_diagnostics(&check(src));
}

// ---------------------------------------------------------------------------
// D0084 reservation in the diagnostic catalog
// ---------------------------------------------------------------------------

#[test]
fn d0084_is_reserved_in_the_diagnostic_catalog() {
    let entry = DIAGNOSTIC_CATALOG
        .iter()
        .find(|(code, _)| *code == "D0084")
        .expect("DIAGNOSTIC_CATALOG should reserve D0084 in PR-A");
    assert_eq!(entry.1, "enumValueMismatch");
}

#[test]
fn d0084_immediately_follows_d0083_in_the_catalog() {
    // The spec is explicit: D0084 is appended at the end of
    // DIAGNOSTIC_CATALOG, immediately after the existing D0083 entry.
    let d83_idx = DIAGNOSTIC_CATALOG
        .iter()
        .position(|(c, _)| *c == "D0083")
        .expect("D0083 must be in the catalog");
    let d84_idx = DIAGNOSTIC_CATALOG
        .iter()
        .position(|(c, _)| *c == "D0084")
        .expect("D0084 must be in the catalog");
    assert_eq!(d84_idx, d83_idx + 1);
}

// ---------------------------------------------------------------------------
// PR-A non-scope guard — D0084 should NOT yet fire from any check site.
// This test must FAIL the moment PR-B wires up the first check site
// (which is the desired signal: PR-B updates this guard to assert
// emission, no longer absence).
// ---------------------------------------------------------------------------

#[test]
fn d0084_does_not_yet_fire_on_off_enum_literal_comparison() {
    // The headline PR-B scenario: `col("status") == "pendig"` against an
    // enum-typed column. In PR-A, no check site emits D0084 yet, so this
    // source produces no D0084 diagnostic.
    let src = "\
class Order(Schema):
    id: long
    status: enum[\"pending\", \"shipped\"]

def f(orders: DataFrame[Order]) -> DataFrame:
    return orders.filter(col(\"status\") == \"pendig\")
";
    let result = check(src);
    assert!(
        !result.has_code("D0084"),
        "PR-A reserves D0084 only; PR-B wires up the emission. \
         If this test now fails, update it to the PR-B emission shape."
    );
}
