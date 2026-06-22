//! v1.12 PR-G — direct unit tests for `classify_pivot_table_aggfunc`.
//!
//! The v1.12 PR-D1 integration tests pin observable behavior (no
//! diagnostic fires, existing D0030 paths intact) but don't pin the
//! classifier output directly — a refactor could silently revert the
//! classifier to no-op without flipping any integration probe. These
//! tests pin each variant of `PivotTableAggfuncForm` against the
//! classifier output, plus a tripwire on the allowlist count so the
//! CHANGELOG-claimed 11-string set can't drift unnoticed.

#![allow(non_snake_case)]

use pykrete::operations::expr::{
    PIVOT_TABLE_AGGFUNC_ALLOWLIST, PivotTableAggfuncForm, classify_pivot_table_aggfunc,
};
use ruff_python_ast::{Expr, ExprCall};
use ruff_python_parser::parse_expression;

fn parse_call(src: &str) -> ExprCall {
    let parsed = parse_expression(src).expect("expression should parse");
    match *parsed.into_syntax().body {
        Expr::Call(c) => c,
        other => panic!("expected ExprCall, got {other:?}"),
    }
}

#[test]
fn V112G_classifier_absent_when_aggfunc_kwarg_missing() {
    let call = parse_call(r#"pdf.pivot_table(values="amount", index="cat", columns="year")"#);
    assert_eq!(
        classify_pivot_table_aggfunc(&call),
        PivotTableAggfuncForm::Absent
    );
}

#[test]
fn V112G_classifier_allowlisted_string_for_each_documented_aggfunc() {
    for name in PIVOT_TABLE_AGGFUNC_ALLOWLIST {
        let src = format!(r#"pdf.pivot_table(values="v", aggfunc="{name}")"#);
        let call = parse_call(&src);
        assert_eq!(
            classify_pivot_table_aggfunc(&call),
            PivotTableAggfuncForm::AllowlistedString(name),
            "expected AllowlistedString({name:?}) for aggfunc={name:?}",
        );
    }
}

#[test]
fn V112G_classifier_allowlisted_string_list_for_homogeneous_literal_list() {
    let call = parse_call(r#"pdf.pivot_table(values="v", aggfunc=["sum", "mean", "count"])"#);
    assert_eq!(
        classify_pivot_table_aggfunc(&call),
        PivotTableAggfuncForm::AllowlistedStringList,
    );
}

#[test]
fn V112G_classifier_fell_through_for_out_of_allowlist_string() {
    let call = parse_call(r#"pdf.pivot_table(values="v", aggfunc="prod")"#);
    assert_eq!(
        classify_pivot_table_aggfunc(&call),
        PivotTableAggfuncForm::FellThrough,
    );
}

#[test]
fn V112G_classifier_fell_through_for_mixed_literal_list() {
    let call = parse_call(r#"pdf.pivot_table(values="v", aggfunc=["sum", "prod"])"#);
    assert_eq!(
        classify_pivot_table_aggfunc(&call),
        PivotTableAggfuncForm::FellThrough,
    );
}

#[test]
fn V112G_classifier_fell_through_for_callable() {
    let call = parse_call(r#"pdf.pivot_table(values="v", aggfunc=np.sum)"#);
    assert_eq!(
        classify_pivot_table_aggfunc(&call),
        PivotTableAggfuncForm::FellThrough,
    );
}

// Tripwire — pins the CHANGELOG-claimed allowlist count. v1.12 PR-D1
// shipped exactly 11 documented canonical aggfunc strings (per spec
// §4.1.1). Adding to the allowlist requires updating both the
// CHANGELOG entry and this assertion in the same change.
#[test]
fn V112G_allowlist_count_matches_changelog_pin_of_11() {
    assert_eq!(PIVOT_TABLE_AGGFUNC_ALLOWLIST.len(), 11);
}
