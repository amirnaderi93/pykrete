//! v1.4 PR-B — three checker bug closures (spec §4).
//!
//! All three are PRE-EXISTING on `main` at v1.3.0 tag (surfaced by v1.3
//! audits, deferred from v1.3 scope). Each bug ships with positive
//! (fires after fix) + negative-guard (no false positive) + vacuity-
//! falsifying tests.
//!
//! - **Bug 1**: `util(df["typo"])` where `util` is registry-tracked but
//!   has NO `DataFrame[X]`-typed param — `df["typo"]` silently passed
//!   because the §10 widening was gated off by `is_registry_function_call`
//!   AND the typed-slot walk in `check_call_argument_schemas` skipped
//!   non-typed args. Fix: walk every arg unconditionally inside
//!   `check_call_argument_schemas`, re-using the captured schema in
//!   `check_one_call_arg` to keep the walk idempotent (no double D0030).
//!
//! - **Bug 2**: `(pdf := build()).rename(columns={...})` lost the pandas
//!   dialect because `inherited_dialect` only walked `Expr::Name`,
//!   `Expr::Call`, `Expr::Attribute` — not `Expr::Named` (walrus). Fix:
//!   add `Expr::Named` arm that recurses into `named.value`. Subscript
//!   receivers (container indexing) stay deferred to v1.5+ per spec.
//!
//! - **Bug 3**: `pdf.transform(helper)` dropped the pandas dialect
//!   because `infer_transform_output` called `bind_df(... None)`. The
//!   helper's body inference then saw `Dialect::Spark` default, pandas
//!   dispatch (`.assign`, etc.) silently skipped, inferred return came
//!   back `None`, downstream col-refs went silent. Fix: thread the
//!   receiver's `inherited_dialect` into the bind.
//!
//! Spec: `docs/design/v1.4-spec.md` §4.

#![allow(non_snake_case)]

mod common;
use common::*;

// ===========================================================================
// Bug 1 — Registry-call §10 widening edge case
// ===========================================================================

#[test]
fn V14B_bug1_util_call_with_typo_fires_d0030() {
    // `util(x: int)` has no `DataFrame[X]`-typed slot, so the v1.3
    // typed-slot walk inside `check_call_argument_schemas` skipped this
    // arg entirely. The §10 fallback at the Expr::Call site was then
    // gated off by `is_registry_function_call`. Result on `main`:
    // silent pass. After PR-B, every arg is walked unconditionally.
    let result = check(
        r#"
class Sales(Schema):
    amount: int

def util(x: int):
    pass

def f(df: SparkFrame[Sales]):
    util(df["typo"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

#[test]
fn V14B_bug1_util_call_with_real_col_quiet() {
    // Same util but with a real column — no D0030. Regression-guard
    // that the unconditional walk doesn't fire on legit code.
    let result = check(
        r#"
class Sales(Schema):
    amount: int

def util(x: int):
    pass

def f(df: SparkFrame[Sales]):
    util(df["amount"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V14B_bug1_dataframe_typed_param_still_quiet() {
    // `util(d: DataFrame[Sales])` accepted a frame of the right schema —
    // the pre-existing D0051 path stays clean. Without this guard the
    // refactor in `check_one_call_arg` (which now consults pre-walked
    // schemas) could over-fire.
    let result = check(
        r#"
class Sales(Schema):
    amount: int

def util(d: DataFrame[Sales]):
    pass

def f(df: SparkFrame[Sales]):
    util(df)
"#,
    );
    assert_does_not_have_code(&result, "D0051");
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V14B_bug1_no_double_fire_on_unrecognized_call() {
    // `print(...)` is neither a method dispatch nor a registry function
    // — only the §10 fallback walks it. Confirms the walk fires
    // exactly once per Subscript (idempotency: not 2× via overlap with
    // any other path).
    let result = check(
        r#"
class Sales(Schema):
    amount: int

def f(df: SparkFrame[Sales]):
    print(df["typo"])
"#,
    );
    assert_count(&result, "D0030", 1);
}

#[test]
fn V14B_bug1_no_double_fire_on_registry_call() {
    // The vacuity-paired idempotency check: after the fix, registry
    // calls walk args inside `check_call_argument_schemas`. The §10
    // fallback is still gated by `is_registry_function_call`, so no
    // second walk occurs. D0030 must fire exactly once.
    let result = check(
        r#"
class Sales(Schema):
    amount: int

def util(x: int):
    pass

def f(df: SparkFrame[Sales]):
    util(df["typo"])
"#,
    );
    assert_count(&result, "D0030", 1);
}

#[test]
fn V14B_bug1_kwarg_typo_fires_d0030() {
    // Coverage parity: the keyword-arg path now also walks unconditionally
    // (kwarg target = `x` is `int`-typed; without Bug-1 fix this skipped).
    let result = check(
        r#"
class Sales(Schema):
    amount: int

def util(x: int):
    pass

def f(df: SparkFrame[Sales]):
    util(x=df["typo"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ===========================================================================
// Bug 2 — `inherited_dialect` walks Expr::Named (walrus) receivers
// ===========================================================================

#[test]
fn V14B_bug2_walrus_pandas_rename_dispatches() {
    // `(pdf2 := pdf).rename(columns={"status": "state"})` — walrus on
    // a pandas-tagged receiver. After Bug-2 fix, `inherited_dialect`
    // recurses through `Expr::Named`, the rename dispatches under
    // Pandas dialect, and the post-rename schema is [state, amount].
    // Reading the old `status` column then fires D0030.
    let result = check(
        r#"
class Stat(Schema):
    status: string
    amount: int

def f(pdf: PandasFrame[Stat]):
    out = (pdf2 := pdf).rename(columns={"status": "state"})
    _ = out["status"]
    _ = out["state"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "status");
}

#[test]
fn V14B_bug2_walrus_spark_rename_does_not_mutate() {
    // The same walrus on a Spark receiver: pandas-only `.rename(columns=...)`
    // does not dispatch on Spark frames (the dispatch is dialect-gated).
    // Confirms walrus correctly inherits Spark dialect — gate skips,
    // no schema mutation, no false D0030.
    let result = check(
        r#"
class Stat(Schema):
    status: string
    amount: int

def f(sdf: SparkFrame[Stat]):
    out = (sdf2 := sdf).rename(columns={"status": "state"})
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V14B_bug2_walrus_no_dialect_chain_quiet() {
    // Walrus on an opaque RHS — `inherited_dialect` recurses through
    // `Expr::Named`, lands on the unknown Call, returns None. No false
    // dialect, no false D0030.
    let result = check(
        r#"
class Stat(Schema):
    status: string

def f():
    out = (x := some_unknown_func()).rename(columns={"status": "state"})
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Bug 3 — `df.transform(helper)` dialect preservation
// ===========================================================================

#[test]
fn V14B_bug3_transform_preserves_pandas_dialect() {
    // `pdf.transform(helper)` with an annotated-param / unannotated-return
    // helper: the helper's body inference must run under Pandas dialect
    // so `.assign(_probe=...)` dispatches and the inferred return schema
    // extends with `_probe`. Without the fix, `bind_df(... None)` gave
    // helper's `p` no dialect, `.assign` silently skipped, inferred
    // return came back `None`, and `out["xxx"]` was silently quiet.
    // After fix: `out` has `[status, amount, _probe]`; `out["xxx"]`
    // fires D0030.
    let result = check(
        r#"
class Stat(Schema):
    status: string
    amount: int

def helper(p: PandasFrame[Stat]):
    return p.assign(_probe=p["status"])

def f(pdf: PandasFrame[Stat]):
    out = pdf.transform(helper)
    _ = out["xxx"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "xxx");
}

#[test]
fn V14B_bug3_transform_preserves_spark_dialect() {
    // The Spark mirror: sdf.transform(helper) where helper applies a
    // Spark op via select(col(...)). Spark dispatch ungates via
    // `Dialect::Spark`. Confirms the dialect-thread also covers Spark
    // (no regression to the existing Spark path).
    let result = check(
        r#"
from pyspark.sql.functions import col

class Stat(Schema):
    status: string
    amount: int

def helper(p: SparkFrame[Stat]):
    return p.select(col("status"), col("amount"))

def f(sdf: SparkFrame[Stat]):
    out = sdf.transform(helper)
    _ = out["xxx"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "xxx");
}

#[test]
fn V14B_bug3_transform_unknown_receiver_quiet() {
    // Opaque receiver — `inherited_dialect` returns None, helper's
    // body inference runs without a dialect tag. No false tagging, no
    // false dispatch. Confirms the fix only THREADS the receiver's
    // dialect, doesn't INVENT one.
    let result = check(
        r#"
class Stat(Schema):
    status: string

def helper(p):
    return p

def f():
    out = some_unknown_func().transform(helper)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
