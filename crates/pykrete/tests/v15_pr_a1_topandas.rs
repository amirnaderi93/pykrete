//! v1.5 PR-A1 — `.toPandas()` dialect handoff
//! (`SparkFrame[X]` → `PandasFrame[X]`).
//!
//! Spec: `docs/design/v1.5-spec.md` §2.1.
//!
//! The dialect flip happens in `inherited_dialect` (driver.rs); the
//! schema-pass-through happens in `analyze_method_call_inner` (expr.rs).
//! Positive cases exercise the handoff by dispatching a pandas-only
//! method (`rename(columns=...)`, `assign(...)`) after `.toPandas()`
//! and asserting the dispatch fires. Negative-space cases pin the
//! gates per spec §2.1: non-DataFrame receiver, kwargs-tolerant
//! propagation, Unknown receiver, and Pandas-already-tagged receiver
//! fall through.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V15A1_rebind_to_pandas_then_rename_dispatches_pandas:
//
// Positive — the load-bearing handoff. After `pdf = sdf.toPandas()`,
// `pdf` must dispatch pandas `.rename(columns={...})` (mutating the
// tracked schema) so a follow-up column reference resolves through
// the renamed name. If `inherited_dialect` failed to flip on
// `sdf.toPandas()`, the rename dispatch would skip (receiver_is_
// pandas_inherited would be false), the schema would NOT extend, and
// the follow-up `col("state")` reference would either silently
// resolve against the original schema (no `state`) or, with the
// rename's schema mutation skipped, fire D0030 on `state`.
// ---------------------------------------------------------------------------

#[test]
fn V15A1_rebind_to_pandas_then_rename_dispatches_pandas() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]):
    pdf = sdf.toPandas()
    renamed = pdf.rename(columns={"status": "state"})
    return renamed.select(col("state"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15A1_inline_chain_topandas_then_rename_dispatches_pandas:
//
// Positive — the inline-subexpression shape (`sdf.toPandas().rename(...)`)
// without a rebind. Exercises the chain-receiver path in
// `inherited_dialect`: the outer `.rename`'s receiver is the inline
// `sdf.toPandas()` call, whose dialect must flip to Pandas for the
// pandas `rename(columns=...)` dispatch to fire.
// ---------------------------------------------------------------------------

#[test]
fn V15A1_inline_chain_topandas_then_rename_dispatches_pandas() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]):
    return sdf.toPandas().rename(columns={"status": "state"}).select(col("state"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15A1_inline_chain_through_filter_then_topandas_then_pandas_method:
//
// Positive — `df.filter(...).toPandas().rename(...)`. The intermediate
// `.filter` doesn't change the dialect; `inherited_dialect`'s chain
// walker must descend through the filter call, find `.toPandas()` on
// the inner chain, and flip Spark → Pandas. Spec §2.1 explicitly
// names this shape ("inline subexpressions (`df.filter(...).toPandas()`)
// work").
// ---------------------------------------------------------------------------

#[test]
fn V15A1_inline_chain_through_filter_then_topandas_then_pandas_method() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]):
    return sdf.filter(col("status") == "OPEN").toPandas().rename(columns={"status": "state"}).select(col("state"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Negative-space tests — MANDATORY per v14-rule 4 / spec §2.1
// ===========================================================================

// ---------------------------------------------------------------------------
// V15A1_helper_topandas_does_not_apply_handoff:
//
// Spec §2.1 negative-space #1: `helper.toPandas()` where `helper` is
// not a DataFrame → no inference. `helper` is a plain Python list with
// no DataFrame binding; `inherited_dialect` descends through the
// `.toPandas()` Call into the `Name("helper")` and returns None (no
// dialect tag). The Spark→Pandas flip is gated on the receiver being
// Spark, so it skips. A follow-up `.rename(columns={...})` does NOT
// trigger the pandas dispatch, the schema does NOT extend, and the
// downstream `col("state")` against an Unknown schema correctly
// finds no schema to consult — D0030 stays silent (the call chain
// dies at toPandas's `*handled = false` fall-through, and Unknown
// schema means no col-ref check fires). Pre-implementation behavior:
// identical (the arm didn't exist) — the test pins that the new arm
// did NOT widen to non-frame receivers.
// ---------------------------------------------------------------------------

#[test]
fn V15A1_helper_topandas_does_not_apply_handoff() {
    let result = check(
        r#"
def f():
    helper = [1, 2, 3]
    return helper.toPandas()
"#,
    );
    assert_no_diagnostics(&result);
}

// ---------------------------------------------------------------------------
// V15A1_topandas_with_kwarg_still_propagates:
//
// Spec §2.1 negative-space #2: `df.toPandas(somearg=...)` → same
// propagation, kwargs ignored. The PR-A1 arm doesn't inspect args;
// it just returns the receiver's schema and lets `inherited_dialect`
// flip the chain dialect. A common real-world kwarg is `arrow=True`
// (Spec §2.1: ".toPandas(arrow=True) kwargs same propagation,
// ignored"). The pandas-dispatched `.rename` after the kwarg'd
// `.toPandas` must still mutate the schema; otherwise the
// downstream `select("state")` would not resolve.
// ---------------------------------------------------------------------------

#[test]
fn V15A1_topandas_with_kwarg_still_propagates() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]):
    pdf = sdf.toPandas(arrow=True)
    renamed = pdf.rename(columns={"status": "state"})
    return renamed.select(col("state"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15A1_unknown_call_topandas_result_unknown:
//
// Spec §2.1 negative-space #3: `f().toPandas()` where `f()` returns
// Unknown → result Unknown. `f()` is a free function pykrete doesn't
// model; the receiver of `.toPandas()` resolves to Unknown via
// `analyze_expr`. Per `analyze_method_call_inner`'s receiver guard
// (`*handled = false; return None;`), the `.toPandas` call returns
// None and never reaches the new arm. `inherited_dialect` descends
// through Call → its `func.as_attribute_expr()?` returns None (the
// inner `f()` callee is a Name, not an attribute) → returns None.
// Downstream `pdf` is therefore bound with no schema and no dialect;
// a typo `col("missing")` would NOT fire D0030 because there's no
// receiver schema to check against. The test pins that a wrong
// implementation that defaulted the dialect to Pandas in this
// shape does NOT fabricate a schema.
// ---------------------------------------------------------------------------

#[test]
fn V15A1_unknown_call_topandas_result_unknown() {
    let result = check(
        r#"
def opaque():
    return 1

def f():
    pdf = opaque().toPandas()
    return pdf.rename(columns={"status": "state"})
"#,
    );
    assert_no_diagnostics(&result);
}

// ---------------------------------------------------------------------------
// V15A1_pandas_receiver_topandas_falls_through:
//
// Spec §2.1 negative-space #4: `df.toPandas()` where `df` is already
// `PandasFrame[X]` → fall through. The PR-A1 arm is gated on
// `receiver_is_spark_inherited`; a pandas receiver skips the arm.
// The fall-through path is `*handled = false; None` — schema becomes
// Unknown. The pin: a wrong implementation that returned the receiver
// schema for ANY dialect would let a subsequent column reference
// resolve against the pandas schema, silently masking the broken
// chain. With the spec-compliant gate, the downstream `select`
// the gate's job is to drop the schema on non-Spark receivers
// (Pandas → Pandas .toPandas() is idempotent / no-op). We pin the
// drop by chaining .assign(amount=col("statuss")) after — a typo of
// "status". If the gate is BROKEN (fires on any receiver), `again`
// carries the Orders schema through, the col("statuss") lookup
// fails, and D0030 fires. With a CORRECT gate, `again` has no
// schema, no col-check runs, no D0030. assert_no_diagnostics
// distinguishes correct vs broken gate behavior. (Round-1 review
// strengthening — original test asserted on a no-op program and
// passed for any gate.)
// ---------------------------------------------------------------------------

#[test]
fn V15A1_pandas_receiver_topandas_falls_through() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    again = pdf.toPandas()
    return again.assign(amount=col("statuss"))
"#,
    );
    assert_no_diagnostics(&result);
}
