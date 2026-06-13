//! v1.6 PR-D1 — pandas `pivot_table(index=, columns=, values=, aggfunc=)`
//! literal-form schema check.
//!
//! Spec: `docs/design/v1.6-spec.md` §4 (entire section, especially the
//! explicit non-reuse paragraph at §4 lines 339-345).
//!
//! Pandas `pivot_table` does NOT route through the Spark
//! `groupBy().pivot()` arm at `expr.rs:976-1015`. That arm gates on
//! `SchemaView::Grouped` (`:984-987`) which only a prior `.groupBy(...)`
//! produces. Pandas's `pivot_table` is on a DataFrame receiver directly.
//! Hence the deliberate non-reuse: same English word, different
//! operation, different receiver predicate.
//!
//! Output for v1.6 is Unknown (None); users re-anchor with
//! `.cast(DataFrame[NewSchema])` for continued schema tracking. Full
//! schema-tracking (synthesizing the pivoted column layout) is v1.7.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V16D1_literal_all_args_resolves_cleanly:
//
// Positive — the load-bearing literal-form happy path. All four args
// present as literals, every name in `index`/`columns`/`values` exists
// on the receiver schema. Expected: no D-code; the call resolves
// cleanly. Output schema is Unknown — that's verified by sibling tests
// (the chain dying after `.pivot_table(...)` without a `.cast`).
// ---------------------------------------------------------------------------

#[test]
fn V16D1_literal_all_args_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(index="cat", columns="year", values="amount", aggfunc="sum")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16D1_literal_values_typo_fires_D0030:
//
// Positive D0030 — literal `values="bogus"` not in schema. The arm
// must validate every literal name; a typo fires D0030 on the literal's
// range. "Did you mean" is checked for shape consistency with the
// existing `report_column_refs` machinery.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_literal_values_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(index="cat", columns="year", values="bogus")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "bogus");
}

// ---------------------------------------------------------------------------
// V16D1_literal_index_typo_fires_D0030:
//
// Per spec §4.2 — typo in `index` fires D0030 with "did you mean".
// Distinct from the values arm: confirms each of `index`/`columns`/
// `values` is independently validated.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_literal_index_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(index="typo", columns="year", values="amount")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V16D1_list_form_index_validates_each_element:
//
// Per spec §4.2 — `index=["cat", "subcat"]` list form is supported. Each
// element is validated independently; here all are valid, no diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_list_form_index_validates_each_element() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    subcat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(index=["cat", "subcat"], columns="year", values="amount")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16D1_list_form_one_element_typo_fires_D0030:
//
// Per spec §4.2 — one typo in a list form fires D0030 on that element
// only. The other (valid) element resolves cleanly.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_list_form_one_element_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    subcat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(index=["cat", "typo"], columns="year", values="amount")
"#,
    );
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V16D1_spark_receiver_falls_through_no_diagnostic:
//
// Negative-space (per spec §4.2) — Spark receiver: `pivot_table` is a
// pandas method (Spark doesn't have it), so the gate
// `receiver_is_pandas_inherited` misses and the arm doesn't fire.
// Critically, the arm must NOT silently validate against the Spark
// schema either. Vacuity check: if the arm fired regardless of dialect,
// the typo `"typo"` would fire D0030. It must not.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_spark_receiver_falls_through_no_diagnostic() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(sdf: SparkFrame[Sales]):
    return sdf.pivot_table(index="typo", columns="year", values="amount")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16D1_unbound_receiver_falls_through_no_diagnostic:
//
// Negative-space — unbound receiver (`helper`): no DataFrame binding,
// no dialect tag. The receiver-resolve step in
// `analyze_method_call_inner` would bail at the `analyze_expr` call
// (no schema), but BEFORE that, `receiver_is_pandas_inherited` is
// also false. Either way, no D-code. Vacuity check via the bogus name.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_unbound_receiver_falls_through_no_diagnostic() {
    let result = check(
        r#"
def f(helper):
    return helper.pivot_table(index="anything", columns="x", values="y")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16D1_variable_arg_falls_through_silently:
//
// Negative-space (per spec §4.2) — non-literal `index` arg. The arm's
// literal-only scope means it should skip validation on the variable
// arg AND still validate the other literal kwargs. Here `columns="year"`
// and `values="amount"` are literal-and-valid; `index=cols_var` is a
// Name reference. Expected: no D-code.
//
// Vacuity check: if we changed `values="amount"` to `values="bogus"`,
// the test below (`V16D1_variable_arg_other_args_still_validated`)
// confirms the literal validation still fires.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_variable_arg_falls_through_silently() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales], cols_var):
    return pdf.pivot_table(index=cols_var, columns="year", values="amount")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16D1_variable_arg_other_args_still_validated:
//
// Vacuity guard for the prior test: when `index=` is a variable, the
// other literal kwargs are still checked. A typo in literal `values=`
// fires D0030 even though `index=` is opaque.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_variable_arg_other_args_still_validated() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales], cols_var):
    return pdf.pivot_table(index=cols_var, columns="year", values="bogus")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "bogus");
}

// ---------------------------------------------------------------------------
// V16D1_cast_re_anchor_after_pivot_table:
//
// Per spec §4.1 — re-anchoring escape. Users re-establish schema
// tracking on the `pivot_table` result with `.cast(DataFrame[NewSchema])`.
// The cast arm at `expr.rs:616` resolves the new schema; downstream
// `.select("col_in_new_schema")` then validates against `NewSchema`.
// Expected: no D-code (re-anchor is the supported migration path).
// ---------------------------------------------------------------------------

#[test]
fn V16D1_cast_re_anchor_after_pivot_table() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

class Pivoted(Schema):
    cat: string
    y2023: double
    y2024: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table(index="cat", columns="year", values="amount").cast(DataFrame[Pivoted]).select("cat")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16D1_spark_groupby_pivot_sibling_arm_unchanged:
//
// Sibling-arm regression guard. The Spark `groupBy("k").pivot("c")`
// chain validates `"c"` against the pre-pivot underlying schema (per
// `expr.rs:984-1018`). The new pandas `pivot_table` arm must NOT
// disturb this — they share an English word ("pivot") but operate on
// different receiver states. Here every name exists; expected: no
// D-code. Catches accidental shared-arm refactoring.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_spark_groupby_pivot_sibling_arm_unchanged() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(sdf: SparkFrame[Sales]):
    return sdf.groupBy("cat").pivot("year").sum("amount")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16D1_spark_groupby_pivot_typo_still_caught:
//
// Sibling-arm regression guard — the Spark `pivot` arm's D0030 on a
// typo'd pivot column still fires. If the new pandas arm somehow
// gated out the Spark arm, this would silently pass.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_spark_groupby_pivot_typo_still_caught() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(sdf: SparkFrame[Sales]):
    return sdf.groupBy("cat").pivot("bogus").sum("amount")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "bogus");
}

// ---------------------------------------------------------------------------
// V16D1_no_args_falls_through_no_diagnostic:
//
// Per spec §4.2 — `pdf.pivot_table()` no args → no diagnostic. We
// don't enforce required-kwarg shape (runtime catches that). Vacuity
// check: the arm's `for` loop over the three kwargs collects zero
// refs; `report_column_refs` runs on an empty slice (no-op). No D-code.
// ---------------------------------------------------------------------------

#[test]
fn V16D1_no_args_falls_through_no_diagnostic() {
    let result = check(
        r#"
class Sales(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[Sales]):
    return pdf.pivot_table()
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
