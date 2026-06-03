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
def f(raw: SparkFrame[Sale], debug: bool) -> SparkFrame[Sale]:
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
def f(raw: SparkFrame[Sale], mode: int) -> SparkFrame[Sale]:
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
def f(raw: SparkFrame[Sale], debug: bool) -> SparkFrame[Sale]:
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
def f(raw: SparkFrame[Sale]) -> None:
    for i in range(3):
        raw.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn for_loop_target_marks_callee_local_for_d0051() {
    // Top-level `helper` takes a `SparkFrame[Sale]`. Inside `f`, the for
    // loop binds `helper` as its iteration target (`for helper in [...]`),
    // and then the body calls `helper(other)` — at runtime that's the
    // loop variable, not the top-level function. D0051 should NOT fire:
    // the for-target's name shadowed the function in the local scope.
    let src = format!(
        "{SCHEMA}
class Other(Schema):
    code: int

def helper(df: SparkFrame[Sale]) -> None:
    pass

def f(other: SparkFrame[Other]) -> None:
    for helper in [1, 2, 3]:
        helper(other)
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0051");
}

#[test]
fn typo_inside_while_body_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale], n: int) -> None:
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
def f(raw: SparkFrame[Sale]) -> None:
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
def f(raw: SparkFrame[Sale]) -> None:
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
def f(raw: SparkFrame[Sale]) -> None:
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
def f(raw: SparkFrame[Sale]) -> None:
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
def f(raw: SparkFrame[Sale], debug: bool) -> None:
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
def f(raw: SparkFrame[Sale], debug: bool) -> None:
    with open(\"x\") as fh:
        if debug:
            for i in range(3):
                raw.select(\"region\").show()
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// assert / raise — expression children get walked
//
// The assert/raise expression IS the top of the analyzed expression
// (analyze_expr handles the outermost call chain, not arbitrarily nested
// sub-expressions inside builtin calls), so the column ref needs to be
// the top of the test/msg/exc/cause expression, not buried inside a
// `print(...)` or builtin wrapper. The walker change here is: an
// assert / raise statement now reaches its expression children at all,
// where before they were dropped on the floor by the `_ => {}` arm.
// ---------------------------------------------------------------------------

#[test]
fn typo_at_top_of_assert_test_is_caught() {
    // `assert <call>` — the test expression is the call chain itself.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> None:
    assert raw.select(\"regoin\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_at_top_of_assert_message_is_caught() {
    // `assert <test>, <msg>` — the message slot also gets walked.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> None:
    assert raw is not None, raw.select(\"regoin\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn clean_chain_in_assert_does_not_fire() {
    // Positive case: same shape, valid column name → no D0030.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> None:
    assert raw.select(\"region\")
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn typo_at_top_of_raise_exc_is_caught() {
    // `raise <call>` — bare-call form (raises whatever the expression
    // evaluates to). The exc slot's expression IS the call chain.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> None:
    raise raw.select(\"regoin\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_at_top_of_raise_cause_is_caught() {
    // `raise X from <call>` — the `from` slot goes through `cause`.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> None:
    raise Exception(\"x\") from raw.select(\"regoin\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// AugAssign (`x += …`) — expression children get walked
// ---------------------------------------------------------------------------

#[test]
fn typo_inside_aug_assign_rhs_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> None:
    total = 0
    total += raw.select(\"regoin\").count()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// `with df.cache() as df_cached: …` — bound name carries the schema
// ---------------------------------------------------------------------------

#[test]
fn with_cache_alias_bind_carries_schema_into_body() {
    // `raw.cache()` is pass-through (same schema). Binding it to
    // `df_cached` should make `df_cached.select("regoin")` fire D0030
    // against `Sale` — i.e. the `with` walker's bind-DataFrame branch
    // hooks the alias up to the receiver schema.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> None:
    with raw.cache() as df_cached:
        df_cached.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// D0051 — nested-block shadowing of a top-level function. Pre-v0.1.26 the
// walker only saw top-level statements, so a `helper = ...` rebind inside
// an `if` block didn't register, and a call to `helper(...)` in the same
// branch would still get checked against the top-level signature.
// ---------------------------------------------------------------------------

#[test]
fn d0051_does_not_fire_when_callee_is_shadowed_inside_nested_block() {
    let src = format!(
        "{SCHEMA}
class Other(Schema):
    code: int

def helper(df: SparkFrame[Sale]) -> None:
    pass

def f(raw: SparkFrame[Sale], other: SparkFrame[Other], debug: bool) -> None:
    if debug:
        helper = lambda x: None
        helper(other)
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0051");
}

#[test]
fn d0051_fires_against_top_level_callee_without_shadow() {
    // Sanity: same shape, drop the shadow — D0051 should fire because
    // `helper` resolves to the top-level signature and `other` is the
    // wrong schema.
    let src = format!(
        "{SCHEMA}
class Other(Schema):
    code: int

def helper(df: SparkFrame[Sale]) -> None:
    pass

def f(other: SparkFrame[Other]) -> None:
    helper(other)
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0051");
}

#[test]
fn assignment_inside_if_branch_binds_local_schema_for_following_chain() {
    // Inside `if debug:`, we rebind `out = raw.select(col("region"))`.
    // A follow-up `out.select("regoin")` should fire D0030 against the
    // inferred schema — the body walker now records the local binding
    // even inside the conditional.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale], debug: bool) -> None:
    if debug:
        out = raw.select(col(\"region\"))
        out.select(\"regoin\").show()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// Nested function / class defs inside a body
// ---------------------------------------------------------------------------

#[test]
fn typo_inside_a_nested_funcdef_body_is_caught() {
    // Nested helper has its own typed param `raw: SparkFrame[Sale]`. A
    // typo in the inner body must reach the col-ref checker.
    let src = format!(
        "{SCHEMA}
def outer(raw: SparkFrame[Sale]) -> SparkFrame[Sale]:
    def inner(d: SparkFrame[Sale]) -> SparkFrame[Sale]:
        return d.select(col(\"regoin\"))
    return inner(raw)
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn nested_funcdef_locals_do_not_leak_to_outer_scope() {
    // `inner` binds `out` locally to a different schema. After `inner`,
    // the outer `out` should still resolve to its outer binding
    // (the receiver schema), so `out.select("region")` is fine.
    let src = format!(
        "{SCHEMA}
class Other(Schema):
    other_field: int

def outer(raw: SparkFrame[Sale]) -> SparkFrame[Sale]:
    out = raw
    def inner(d: SparkFrame[Other]) -> SparkFrame[Other]:
        out = d
        return out.select(col(\"other_field\"))
    return out.select(col(\"region\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn nested_funcdef_can_be_called_after_definition_without_d0051() {
    // The nested name `helper` is marked local in the outer scope after
    // the def, so a same-named top-level function (if any) doesn't take
    // precedence — and calling `helper` doesn't fire D0051 spuriously.
    let src = format!(
        "{SCHEMA}
def outer(raw: SparkFrame[Sale]) -> SparkFrame[Sale]:
    def helper(d: SparkFrame[Sale]) -> SparkFrame[Sale]:
        return d.select(col(\"region\"))
    return helper(raw)
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn typo_inside_a_nested_classdef_method_body_is_caught() {
    // Nested classdef bodies are walked (best-effort) so col refs
    // inside method bodies still reach the checker. The outer needs a
    // typed signature so the outer function is itself analyzed; the
    // inner method then needs typed params so col refs inside resolve.
    let src = format!(
        "{SCHEMA}
def outer(raw: SparkFrame[Sale]) -> SparkFrame[Sale]:
    class Inner:
        def m(self, d: SparkFrame[Sale]) -> None:
            d.select(col(\"regoin\")).show()
    return raw
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// Expression-level descent — typos inside compound expressions
//
// v0.1.26 fixed statement-level descent. v0.1.37 fixes the
// expression-level analog: a method call embedded inside a Compare,
// BoolOp, UnaryOp, BinOp, Tuple, List, Set, Dict, or IfExp still gets
// its column refs checked. Before this, `if df.select("typo").count()
// > 0:` was a silent miss because the test expression was a Compare
// and analyze_expr returned None without descending.
// ---------------------------------------------------------------------------

#[test]
fn typo_inside_compare_left_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> bool:
    if df.select(\"regoin\").count() > 0:
        return True
    return False
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_is_not_none_compare_is_caught() {
    let src = format!(
        "{SCHEMA}
def g(df: SparkFrame[Sale]) -> bool:
    return df.select(\"regoin\") is not None
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_boolop_value_is_caught() {
    // `or` short-circuit — embedded df.select call must be analyzed.
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> int:
    return df.select(\"regoin\").count() or df.select(\"region\").count()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_unary_not_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> bool:
    return not df.select(\"regoin\").isEmpty()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_binop_arm_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> int:
    return df.select(\"regoin\").count() + df.select(\"region\").count()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_tuple_element_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> tuple:
    return (df.select(\"regoin\"), df.select(\"region\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_list_element_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> list:
    return [df.select(\"regoin\"), df.select(\"region\")]
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_dict_value_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> dict:
    return {{\"k\": df.select(\"regoin\")}}
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_ifexp_branch_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale], debug: bool) -> int:
    return df.select(\"regoin\").count() if debug else df.select(\"region\").count()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_nested_compound_expression_is_caught() {
    // A Compare wrapping a BoolOp wrapping a UnaryOp wrapping a Call —
    // descent must reach the innermost method call.
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> bool:
    return (not df.select(\"regoin\").isEmpty()) and (df.count() > 0)
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// Round-2 walker extensions — Subscript / FString / comprehensions.
// Round-1 covered Compare/BoolOp/UnaryOp/BinOp/Tuple/List/Set/Dict/IfExp;
// these add the rest of the compound forms a real expression can sit
// inside. Lambda stays deferred (its body needs a new parameter scope
// we don't track yet) — tracked as a v1.0.1 follow-up.

#[test]
fn typo_inside_subscript_value_is_caught() {
    // `(df.select("typo"), df2)[0]` — the call is inside the value
    // half of a Subscript.
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> int:
    return (df.select(\"regoin\"), df)[0].count()
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_f_string_interpolation_is_caught() {
    // `f"count={df.select('typo').count()}"` — the call sits inside
    // an interpolation, not a top-level expression.
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> str:
    return f\"count={{df.select('regoin').count()}}\"
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_list_comprehension_iter_is_caught() {
    // `[x for x in df.select("typo").collect()]` — the typo's call is
    // the iteration source. The bound name `x` lives in a new scope
    // (so the comprehension's `elt` / `ifs` are not walked), but the
    // `iter` half evaluates in the outer scope.
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]) -> list:
    return [x for x in df.select(\"regoin\").collect()]
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn typo_inside_generator_expression_iter_is_caught() {
    // `(x for x in df.select("typo").collect())` returned directly —
    // the parenthesized generator expression's `iter` half is the
    // call we want walked.
    let src = format!(
        "{SCHEMA}
def f(df: SparkFrame[Sale]):
    return (x for x in df.select(\"regoin\").collect())
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}
