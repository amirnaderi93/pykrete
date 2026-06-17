//! v1.12 PR-D1 — pandas `pivot_table(aggfunc=)` literal-form
//! recognition. Extension on the v1.6 PR-D1 `pivot_table` arm
//! (`crates/pykrete/src/operations/expr.rs` — `receiver_is_pandas_inherited &&
//! method == "pivot_table"`).
//!
//! Spec: `docs/design/v1.12-spec.md` §4.
//!
//! Behavior contract: the v1.6 PR-D1 arm validates each literal
//! `index`/`columns`/`values` name against the receiver schema and
//! returns Unknown for the result schema. v1.12 PR-D1 closes the
//! "aggfunc= is opaque" carve-out by adding a recognition pass for
//! the `aggfunc=` kwarg: literal allowlist strings + literal lists
//! of allowlist strings are recognized; callable / dict / dynamic /
//! out-of-allowlist forms fall through silently.
//!
//! v1.x observable behavior is UNCHANGED — recognition is
//! informational per spec §4.1.2 ("aggfunc choice doesn't influence
//! which columns survive given the v1.4 pandas index-modeling
//! carve-out"). These tests pin the arm against future regressions:
//! - the recognition pass MUST NOT introduce spurious D0030s on any
//!   aggfunc shape;
//! - the existing v1.6 PR-D1 D0030 validation on `values`/`index`/
//!   `columns` MUST continue to fire independently of `aggfunc`;
//! - the Spark / unbound-receiver / variable-arg fall-through paths
//!   from v1.6 stay intact.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V112D1_aggfunc_allowlist_string_no_diagnostic:
//
// Positive — literal allowlist string `aggfunc="sum"`. The recognition
// path is exercised; the arm returns Unknown as before; no D0030.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_allowlist_string_no_diagnostic() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", columns="year", aggfunc="sum")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_allowlist_string_mean_no_diagnostic:
//
// Positive — a different allowlist entry (`"mean"`). Confirms the
// allowlist isn't pinned to a single string.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_allowlist_string_mean_no_diagnostic() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", aggfunc="mean")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_allowlist_list_no_diagnostic:
//
// Positive — literal list of allowlist strings (`["sum", "mean"]`).
// Both members in-list; recognition succeeds; arm returns Unknown;
// no D0030.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_allowlist_list_no_diagnostic() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", aggfunc=["sum", "mean"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_exotic_string_falls_through_silently:
//
// Negative-space (spec §4.1) — `aggfunc="exotic_thing"` is a literal
// string OUTSIDE the allowlist. Falls through silently (no D-code,
// arm returns Unknown). We don't model pandas's agg registry; an
// exotic string is either a user-registered aggregator (legitimate)
// or a typo (pandas raises KeyError at runtime). Silence is honest.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_exotic_string_falls_through_silently() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", aggfunc="exotic_thing")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_callable_falls_through_silently:
//
// Negative-space (spec §4.1) — `aggfunc=np.sum` is a Name reference,
// not a string literal. Falls through silently. Recognition's
// fall-through arm covers any non-literal expression.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_callable_falls_through_silently() {
    let result = check(
        r#"
import numpy as np

class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", aggfunc=np.sum)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_lambda_falls_through_silently:
//
// Negative-space (spec §4.1) — `aggfunc=lambda x: x.sum()` is a
// Lambda expression, not a literal. Falls through silently.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_lambda_falls_through_silently() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", aggfunc=lambda x: x.sum())
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_dict_falls_through_silently:
//
// Negative-space (spec §4.1) — `aggfunc={"amount": "sum"}` is a dict
// literal. Recognition's fall-through arm covers it; no D-code.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_dict_falls_through_silently() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", aggfunc={"amount": "sum"})
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_dynamic_name_falls_through_silently:
//
// Negative-space (spec §4.1) — `aggfunc=some_var` is a dynamic Name.
// Falls through silently.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_dynamic_name_falls_through_silently() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales], some_var):
    return pdf.pivot_table(values="amount", index="cat", aggfunc=some_var)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_absent_default_mean_no_diagnostic:
//
// Regression guard (spec §4.1 / pre-extension behavior) — no
// `aggfunc=` kwarg; pandas defaults to `"mean"`. The arm must not
// fire and must not error from the recognition path's `Absent`
// branch.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_absent_default_mean_no_diagnostic() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_mixed_list_falls_through_silently:
//
// Negative-space — `aggfunc=["sum", lambda x: x]` is a list with a
// non-literal member. `parse_string_list` returns `None`;
// recognition falls through. No D-code.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_mixed_list_falls_through_silently() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", aggfunc=["sum", lambda x: x])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_aggfunc_list_with_out_of_allowlist_falls_through_silently:
//
// Negative-space — `aggfunc=["sum", "exotic"]`. Per spec §4.1
// "literal list of strings from the allowlist": "first-unknown
// short-circuits". One out-of-allowlist member → recognition falls
// through silently; no D-code.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_aggfunc_list_with_out_of_allowlist_falls_through_silently() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="cat", aggfunc=["sum", "exotic"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V112D1_values_typo_with_allowlist_aggfunc_still_fires_D0030:
//
// Composition guard — `aggfunc="sum"` (recognized) does NOT swallow
// the v1.6 PR-D1 D0030 on a typo'd `values=`. The two recognition
// passes are independent; the column-ref validation runs before
// `classify_pivot_table_aggfunc` and reports unchanged.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_values_typo_with_allowlist_aggfunc_still_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="bogus", index="cat", aggfunc="sum")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "bogus");
}

// ---------------------------------------------------------------------------
// V112D1_index_typo_with_callable_aggfunc_still_fires_D0030:
//
// Composition guard — `aggfunc=np.sum` (fell through) does NOT
// swallow the v1.6 PR-D1 D0030 on a typo'd `index=`. The arm's
// column-ref pass is independent of the aggfunc form.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_index_typo_with_callable_aggfunc_still_fires_D0030() {
    let result = check(
        r#"
import numpy as np

class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(values="amount", index="typo", aggfunc=np.sum)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V112D1_spark_receiver_with_aggfunc_falls_through_no_diagnostic:
//
// Sibling-arm regression guard (spec §4.1 negative case 9) —
// `sdf.pivot_table(..., aggfunc='sum')` on a Spark receiver doesn't
// hit the pandas arm at all (Spark doesn't have `pivot_table` as
// a DataFrame method; the gate is `receiver_is_pandas_inherited`).
// Critically, the v1.12 recognition extension MUST NOT widen the
// gate. Vacuity check: `index="typo"` would fire D0030 if the arm
// somehow ran against the Spark receiver.
// ---------------------------------------------------------------------------

#[test]
fn V112D1_spark_receiver_with_aggfunc_falls_through_no_diagnostic() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(sdf: SparkFrame[Sales]):
    return sdf.pivot_table(values="amount", index="typo", aggfunc="sum")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
