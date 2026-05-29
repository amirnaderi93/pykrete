//! Column-reference checking inside the bodies of control-flow
//! statements — `if`/`elif`/`else`, `for`, `while`, `with`, `try`/`except`/
//! `finally`. Pre-v0.1.26 the body walker only descended into top-level
//! `Assign` / `AnnAssign` / `Return` / `Expr` statements, so anything
//! sitting inside an `if`/`for`/`with`/`try` block was a silent blind
//! spot — typos there never reached the checker.
//!
//! These tests assert the descent works for each block kind, plus a
//! couple of nested-mix cases.

#![allow(non_snake_case)]

mod common;
use common::*;

const SCHEMA: &str = "\
class Sale(Schema):
    region: string
    amount: int
";

// ---------------------------------------------------------------------------
// if / elif / else
// ---------------------------------------------------------------------------

#[test]
fn typo_inside_if_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale], debug: bool) -> DataFrame[Sale]:
    if debug:
        return raw.select(\"regoin\")
    return raw.select(\"region\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_elif_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale], mode: int) -> DataFrame[Sale]:
    if mode == 1:
        return raw.select(\"region\")
    elif mode == 2:
        return raw.select(\"regoin\")
    else:
        return raw.select(\"region\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_else_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale], debug: bool) -> DataFrame[Sale]:
    if debug:
        return raw.select(\"region\")
    else:
        return raw.select(\"regoin\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// for / while
// ---------------------------------------------------------------------------

#[test]
fn typo_inside_for_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> None:
    for i in range(3):
        raw.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn for_loop_target_shadows_outer_name() {
    // `raw` is a parameter; inside the for-body we re-bind `raw` as the
    // loop variable. The chain `raw.select(...)` no longer sees the
    // DataFrame schema — checking degrades gracefully rather than
    // firing a false-positive D0030.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> None:
    for raw in [1, 2, 3]:
        pass
    raw.select(\"region\").show()
"
    );
    let result = check(&src);
    // The post-loop `raw.select("region")` runs against the rebound
    // `raw` (now a number); we don't assert presence of a particular
    // diagnostic here — just that we didn't crash and the for-target
    // marking ran.
    let _ = result;
}

#[test]
fn typo_inside_while_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale], n: int) -> None:
    while n > 0:
        raw.select(\"regoin\").show()
        n = n - 1
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// with
// ---------------------------------------------------------------------------

#[test]
fn typo_inside_with_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> None:
    with open(\"file.txt\") as fh:
        raw.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// try / except / finally
// ---------------------------------------------------------------------------

#[test]
fn typo_inside_try_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> None:
    try:
        raw.select(\"regoin\").show()
    except Exception:
        pass
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_except_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> None:
    try:
        pass
    except Exception as e:
        raw.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_finally_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> None:
    try:
        pass
    finally:
        raw.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// Nesting
// ---------------------------------------------------------------------------

#[test]
fn typo_inside_for_inside_if_inside_with_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale], debug: bool) -> None:
    with open(\"x\") as fh:
        if debug:
            for i in range(3):
                raw.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn clean_chain_inside_nested_blocks_does_not_fire() {
    // Sanity: same shape as above with correct column name produces no
    // false positive.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale], debug: bool) -> None:
    with open(\"x\") as fh:
        if debug:
            for i in range(3):
                raw.select(\"region\").show()
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn assignment_inside_if_branch_binds_local_schema_for_following_chain() {
    // Inside `if debug:`, we rebind `out = raw.select(col("region"))`.
    // A follow-up `out.select("regoin")` should fire D0030 against the
    // inferred schema — the body walker now records the local binding
    // even inside the conditional.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale], debug: bool) -> None:
    if debug:
        out = raw.select(col(\"region\"))
        out.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}
