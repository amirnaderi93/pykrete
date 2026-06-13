//! v1.6 PR-A2 — two small audit-traceable fixes bundled:
//!
//! 1. `.take()` dialect-gate (sibling to v1.5 PR-A3 `.head/.tail/.first`).
//!    Pandas `pdf.take([i, j, k])` returns a row-sliced DataFrame; on a
//!    PandasFrame[X] receiver the chain stays alive and the schema is
//!    preserved. Spark `df.take(n)` returns `list[Row]` — terminal,
//!    unchanged.
//!
//! 2. `pdf.loc` D0030 false-positive in nested method-arg position. The
//!    pre-fix walker treated `pdf.loc` (Attribute on a DF-bound Name) as
//!    a column reference to `loc`, falsely firing D0030 on
//!    `pdf.assign(bag=pdf.loc[:, "x"])`. The fix gates the attribute arm
//!    in `col_refs::collect_col_refs` so `loc`/`iloc` (pandas indexer
//!    accessors) are NOT treated as column names.
//!
//! Spec: `docs/design/v1.6-spec.md` §3.2.

#![allow(non_snake_case)]

mod common;
use common::*;

// ===========================================================================
// Fix 1 — `.take()` dialect-gate
// ===========================================================================

// ---------------------------------------------------------------------------
// V16A2_spark_take_terminates_chain:
//
// Positive Spark-side regression guard. On a `SparkFrame[X]` receiver,
// `.take(n)` returns `list[Row]` — terminal. The chain dies, so a
// follow-up `.select(col("statuss"))` does NOT fire D0030 even though
// `"statuss"` would be a typo. Terminal-recognizer behavior on Spark is
// unchanged by PR-A2; this test pins that.
// ---------------------------------------------------------------------------

#[test]
fn V16A2_spark_take_terminates_chain() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.take(5).select(col("statuss"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16A2_pandas_take_preserves_chain_fires_d0030_on_downstream_typo:
//
// THE PR-A2 Fix 1. On a `PandasFrame[X]` receiver, `.take([0, 2])`
// returns a row-sliced DataFrame; the chain stays alive and the
// PandasFrame dialect tag survives. A follow-up
// `.assign(amount=col("statuss"))` fires D0030 on the typo because the
// schema is still tracked. Before PR-A2, `.take` was a blanket terminal,
// the chain died, and D0030 did NOT fire — silent miss. Sibling to v1.5
// PR-A3 `.head`/`.tail`/`.first` coverage.
// ---------------------------------------------------------------------------

#[test]
fn V16A2_pandas_take_preserves_chain_fires_d0030_on_downstream_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.take([0, 2]).assign(amount=col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ---------------------------------------------------------------------------
// V16A2_unbound_receiver_take_no_inference:
//
// `helper.take(...)` — `helper` has no DataFrame binding. The receiver
// resolution at the top of method-call dispatch returns None and the
// chain bails before reaching the terminal recognizer. No diagnostic
// should fire, including no D0030 from a misread of the surrounding
// `df`'s schema. Sibling-arm fall-through guard.
// ---------------------------------------------------------------------------

#[test]
fn V16A2_unbound_receiver_take_no_inference() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    helper = 1
    helper.take(2)
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16A2_pr_a3_sibling_head_still_preserves_chain:
//
// Regression guard for the v1.5 PR-A3 siblings — adding `take` to the
// pandas pass-through arm must not regress `head`/`tail`/`first`. If a
// later refactor breaks the matches!() shape, this fires first.
// ---------------------------------------------------------------------------

#[test]
fn V16A2_pr_a3_sibling_head_still_preserves_chain() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.head().assign(amount=col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ===========================================================================
// Fix 2 — `pdf.loc` D0030 false-positive in nested method-arg position
// ===========================================================================

// ---------------------------------------------------------------------------
// V16A2_loc_in_assign_arg_no_d0030_on_loc:
//
// THE PR-A2 Fix 2 reproducer. Pre-fix, the col-ref walker descended
// `pdf.assign(bag=pdf.loc[:, "status"])` → subscript-fall-through →
// `pdf.loc` (Attribute) → attribute arm fires → false D0030 on `loc`.
// Post-fix the attribute arm skips `loc`/`iloc` (pandas indexer
// accessors, never column names). "status" IS a real field on `Orders`,
// so no D0030 should fire anywhere.
// ---------------------------------------------------------------------------

#[test]
fn V16A2_loc_in_assign_arg_no_d0030_on_loc() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    return pdf.assign(bag=pdf.loc[:, "status"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16A2_iloc_in_assign_arg_no_d0030_on_iloc:
//
// Sibling-arm coverage for `iloc`. Same gate: `pdf.iloc` is the
// integer-indexer accessor, NOT a column reference to `iloc`. The
// gate matches `loc | iloc`, so this lights up the second slot of the
// matches!().
// ---------------------------------------------------------------------------

#[test]
fn V16A2_iloc_in_assign_arg_no_d0030_on_iloc() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    return pdf.assign(bag=pdf.iloc[0])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16A2_loc_literal_form_typo_still_fires_d0030:
//
// Spec §3.2 regression-guard: the gate must NOT silence true-positives.
// The literal-form `.loc[:, "typo"]` D0030 path lives in
// `expr.rs:246-262` (via `analyze_expr` → `report_column_refs`); it's
// independent of the col-ref walker arm we gated. Asserts that path
// still fires when `pdf.loc[:, "typo"]` is the standalone return value
// and "typo" is NOT in the schema.
//
// NOTE: the spec also listed `pdf.assign(bad=pdf.loc[:, "typo"] + 1)`
// as a regression-guard expecting D0030 on "typo". Pre-fix, that form
// only fired D0030 on `loc` (the FP we removed), NEVER on `typo` — the
// nested method-arg path goes through `apply_add_columns_iter` →
// `collect_col_refs`, which does NOT recognize `.loc` literal-form col
// refs (the literal-form `analyze_expr` arm is not reached for nested
// args). Wiring nested-arg `.loc[:, "typo"]` detection is broader
// `.loc`/`.iloc` scope deferred to v1.7 per spec §9.1. The standalone
// form below is the v1.5 PR-C true-positive surface that PR-A2 must
// not regress.
// ---------------------------------------------------------------------------

#[test]
fn V16A2_loc_literal_form_typo_still_fires_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    return pdf.loc[:, "typo"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V16A2_unbound_receiver_loc_no_d0030:
//
// `helper.loc[:, "x"]` — `helper` has no DataFrame binding. The gate
// only fires when the receiver name lookup succeeds (preserves the
// general filter-out-non-DF-receivers shape). With no binding, neither
// the attribute arm nor the literal-form arm engages, so no
// diagnostic. Sibling-arm fall-through guard (PR-F1 shape).
// ---------------------------------------------------------------------------

#[test]
fn V16A2_unbound_receiver_loc_no_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    helper = 1
    return pdf.assign(bag=helper.loc[0])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V16A2_loc_arg_real_column_still_resolves:
//
// Positive guard: `pdf.assign(bag=pdf.loc[:, "id"])` where "id" IS in
// schema. Asserts no D0030 fires (the inference arm in
// `column_exprs.rs:82-88` resolves the type cleanly), and that the
// col-ref walker's gate doesn't accidentally silence a real downstream
// typo elsewhere in the assign — the second kwarg references
// `col("statuss")` which IS a typo and MUST still fire D0030. Pins
// that the gate is narrowly scoped to the `loc`/`iloc` attribute
// names and doesn't leak.
// ---------------------------------------------------------------------------

#[test]
fn V16A2_loc_arg_real_column_still_resolves() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    return pdf.assign(bag=pdf.loc[:, "id"], more=col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ---------------------------------------------------------------------------
// V16A2_non_loc_attribute_still_fires_d0030:
//
// Sibling-arm regression guard for the gate. The attribute arm in
// `col_refs.rs` is load-bearing for the v1.5 I1 cross-DataFrame leak
// fix — `df.select(df_other.x)` checks `x` on `df_other`. The gate
// only excludes `loc`/`iloc`; other attribute names must still flow
// through. Asserts `df_other.statuss` (typo on a real DF-bound name)
// still fires D0030 on `statuss`.
// ---------------------------------------------------------------------------

#[test]
fn V16A2_non_loc_attribute_still_fires_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders], df_other: SparkFrame[Orders]):
    return df.select(df_other.statuss)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}
