//! v1.15 PR-D1 — direct unit tests for `resolve_override_ty`.
//!
//! Extracted as a shared primitive between
//! `synthesize_pivot_result_from_aggfunc` (v1.13 PR-D2) and
//! `synthesize_groupby_agg_from_aggfunc` (v1.14 PR-D2). Both
//! synthesizers consult the same spec §5.1.1 aggregate-to-dtype
//! table; this file pins that table at the primitive directly so a
//! refactor can't silently drift one synthesizer's behavior away
//! from the other's. Observable behavior of each synthesizer is
//! already covered by the v1.13 PR-D2 and v1.14 PR-D2 suites —
//! this PR is a pure refactor with no behavior change.
//!
//! Per v1.14 retro rule 8 (synthesis-arm positive coverage) the
//! D0080-fire positive coverage lives in the integration suites;
//! the unit tests here pin the primitive's contract only.

#![allow(non_snake_case)]

use pykrete::operations::expr::resolve_override_ty;
use pykrete::types::ColumnType;

#[test]
fn V115D1_count_overrides_to_long() {
    assert_eq!(resolve_override_ty("count"), Some(Some(ColumnType::Long)));
}

#[test]
fn V115D1_nunique_overrides_to_long() {
    assert_eq!(resolve_override_ty("nunique"), Some(Some(ColumnType::Long)));
}

#[test]
fn V115D1_mean_std_var_median_override_to_double() {
    for name in ["mean", "std", "var", "median"] {
        assert_eq!(
            resolve_override_ty(name),
            Some(Some(ColumnType::Double)),
            "{name} should override to Double",
        );
    }
}

#[test]
fn V115D1_sum_min_max_first_last_preserve_receiver_type() {
    for name in ["sum", "min", "max", "first", "last"] {
        assert_eq!(
            resolve_override_ty(name),
            Some(None),
            "{name} should preserve receiver column type (Some(None))",
        );
    }
}

#[test]
fn V115D1_unrecognised_aggfunc_returns_none() {
    assert_eq!(resolve_override_ty("prod"), None);
    assert_eq!(resolve_override_ty("sem"), None);
    assert_eq!(resolve_override_ty("size"), None);
}

#[test]
fn V115D1_empty_string_returns_none() {
    assert_eq!(resolve_override_ty(""), None);
}

#[test]
fn V115D1_case_sensitive_uppercase_returns_none() {
    assert_eq!(resolve_override_ty("COUNT"), None);
    assert_eq!(resolve_override_ty("Mean"), None);
}
