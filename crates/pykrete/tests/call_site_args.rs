//! Call-site argument checking — `D0051 argumentColumnsMismatch`.
//!
//! Closes the function boundary on the input side. The return is already
//! checked by `D0050 returnColumnsMismatch`; this one fires when a
//! caller passes a `DataFrame[Wrong]` where the function declared
//! `DataFrame[Right]`. An argument whose schema can't be inferred
//! (an opaque chain, an untyped local) is silently skipped — the same
//! degrade-rather-than-false-flag stance the rest of the checker uses.

#![allow(non_snake_case)] // Type names (DataFrame) appear in test names.

mod common;
use common::*;

// ===========================================================================
// Happy path — no diagnostic when the arg matches
// ===========================================================================

#[test]
fn matching_argument_schema_is_clean() {
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

def revenue(sales: DataFrame[Sale]) -> DataFrame:
    return sales

def caller(sales: DataFrame[Sale]) -> None:
    revenue(sales)
"#,
    );
    assert_does_not_have_code(&result, "D0051");
}

// ===========================================================================
// The headline case — wrong DataFrame schema passed
// ===========================================================================

#[test]
fn d0051_fires_when_argument_schema_differs_from_parameter() {
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

class Refund(Schema):
    region: string
    refund: int

def revenue(sales: DataFrame[Sale]) -> DataFrame:
    return sales

def caller(refunds: DataFrame[Refund]) -> None:
    revenue(refunds)
"#,
    );
    assert_has_code(&result, "D0051");
    // The message names the parameter, the expected schema, the actual
    // schema, and the missing/extra columns.
    assert_message_contains(&result, "D0051", "sales");
    assert_message_contains(&result, "D0051", "Sale");
    assert_message_contains(&result, "D0051", "Refund");
    assert_message_contains(&result, "D0051", "amount"); // missing
    assert_message_contains(&result, "D0051", "refund"); // extra
}

// ===========================================================================
// Keyword arguments — matched by name
// ===========================================================================

#[test]
fn d0051_fires_on_keyword_argument_mismatch() {
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

class Refund(Schema):
    region: string
    refund: int

def revenue(sales: DataFrame[Sale]) -> DataFrame:
    return sales

def caller(refunds: DataFrame[Refund]) -> None:
    revenue(sales=refunds)
"#,
    );
    assert_has_code(&result, "D0051");
    assert_message_contains(&result, "D0051", "Sale");
    assert_message_contains(&result, "D0051", "Refund");
}

// ===========================================================================
// Multiple typed parameters — each independently checked
// ===========================================================================

#[test]
fn d0051_fires_for_each_mismatched_parameter_independently() {
    let result = check(
        r#"
class Left(Schema):
    id: int

class Right(Schema):
    id: int
    name: string

class A(Schema):
    id: int

def combine(l: DataFrame[Left], r: DataFrame[Right]) -> DataFrame:
    return l

def caller(a: DataFrame[A], b: DataFrame[A]) -> None:
    combine(a, b)
"#,
    );
    // Argument `a` matches Left (both have just `id`) — no diag for it.
    // Argument `b` is DataFrame[A] (just id) but param is DataFrame[Right]
    // (id, name) — diag.
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        1,
        "expected one D0051 for the `b` arg only, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert_message_contains(&result, "D0051", "name"); // missing on `b`
}

// ===========================================================================
// Unknown / un-inferrable argument — silent (degrade, not false-flag)
// ===========================================================================

#[test]
fn opaque_argument_schema_is_silently_skipped() {
    // `mystery_df` has no annotation; pykrete can't infer its schema.
    // The function expects DataFrame[Sale]. The check must NOT fire —
    // we don't know enough to claim there's a mismatch.
    let result = check(
        r#"
class Sale(Schema):
    region: string

def revenue(sales: DataFrame[Sale]) -> DataFrame:
    return sales

def caller(mystery_df) -> None:
    revenue(mystery_df)
"#,
    );
    assert_does_not_have_code(&result, "D0051");
}

// ===========================================================================
// Non-DataFrame parameter — not our jurisdiction
// ===========================================================================

#[test]
fn parameter_without_dataframe_annotation_is_skipped() {
    // The second parameter is just `int` — D0051 only checks
    // `DataFrame[Schema]` parameters. Mismatched non-DataFrame args are
    // someone else's problem (the embedded Python engine).
    let result = check(
        r#"
class Sale(Schema):
    region: string

def revenue(sales: DataFrame[Sale], limit: int) -> DataFrame:
    return sales

def caller(sales: DataFrame[Sale]) -> None:
    revenue(sales, "not an int")
"#,
    );
    assert_does_not_have_code(&result, "D0051");
}

// ===========================================================================
// Argument is a chain result with a derived schema
// ===========================================================================

#[test]
fn d0051_fires_when_chained_argument_resolves_to_wrong_schema() {
    // The argument is `sales.select("region")` — a derived schema with
    // just one column. The function expects DataFrame[Sale] (two
    // columns). The chain result is inferred; the check should fire.
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

def revenue(sales: DataFrame[Sale]) -> DataFrame:
    return sales

def caller(sales: DataFrame[Sale]) -> None:
    revenue(sales.select("region"))
"#,
    );
    assert_has_code(&result, "D0051");
    assert_message_contains(&result, "D0051", "amount"); // missing in the derived arg
}

// ===========================================================================
// Method calls — `df.method(...)` are NOT in scope (analyze_method_call
// owns those; transform has its own input check). Sanity-check that we
// don't accidentally fire on a method call to a function-named method.
// ===========================================================================

// ===========================================================================
// Nested calls — the inner call's argument is checked too
// ===========================================================================

#[test]
fn d0051_fires_for_nested_call_argument_mismatch() {
    // `f(f(b))` — the outer call passes `f(b)`, whose return type is
    // DataFrame[A], to a parameter declared DataFrame[A]: clean. But the
    // inner `f(b)` passes DataFrame[B] to a parameter declared DataFrame[A]:
    // mismatch. Exactly one D0051 should fire, on the inner call.
    let result = check(
        r#"
class A(Schema):
    x: int

class B(Schema):
    y: int

def f(a: DataFrame[A]) -> DataFrame[A]:
    return a

def caller(b: DataFrame[B]) -> None:
    f(f(b))
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        1,
        "expected exactly one D0051 for the inner `f(b)` call, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert_message_contains(&result, "D0051", "A");
    assert_message_contains(&result, "D0051", "B");
    assert_message_contains(&result, "D0051", "x"); // missing
    assert_message_contains(&result, "D0051", "y"); // extra
}

#[test]
fn method_calls_are_not_checked_as_free_function_calls() {
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

def revenue(sales: DataFrame[Sale]) -> DataFrame:
    return sales

def caller(sales: DataFrame[Sale]) -> None:
    # `sales.revenue()` is a method call on `sales`, not a free call to
    # the `revenue` function. The check must not fire — even though the
    # name collides with our typed function.
    sales.revenue()
"#,
    );
    assert_does_not_have_code(&result, "D0051");
}

// ===========================================================================
// Local-name shadowing — a local rebind hides the top-level function
// ===========================================================================

#[test]
fn d0051_skipped_when_callee_is_locally_shadowed() {
    // `revenue` is a top-level function expecting DataFrame[Sale]; the
    // caller rebinds the name locally. The subsequent call resolves to
    // the local at runtime, not the top-level function — pykrete must
    // not fire D0051 against the top-level signature.
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

class Order(Schema):
    qty: int

def revenue(sales: DataFrame[Sale]) -> DataFrame[Sale]:
    return sales

def some_helper(o: DataFrame[Order]) -> DataFrame[Order]:
    return o

def caller(orders: DataFrame[Order]) -> None:
    revenue = some_helper(orders)
    revenue(orders)
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        0,
        "expected no D0051 when the callee name is locally shadowed, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ===========================================================================
// Positional-only / keyword-only param markers — `/` and `*`
// ===========================================================================

#[test]
fn d0051_does_not_match_keyword_arg_to_positional_only_param() {
    // `a` is positional-only (trailing `/`). Calling `f(a=b)` is a Python
    // TypeError. pykrete must not fire D0051 on this — the root cause
    // isn't a schema mismatch, it's a calling-convention error.
    let result = check(
        r#"
class A(Schema):
    x: int

class B(Schema):
    y: int

def f(a: DataFrame[A], /) -> None:
    return

def caller(b: DataFrame[B]) -> None:
    f(a=b)
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        0,
        "expected no D0051 when a keyword arg targets a positional-only param, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn d0051_does_not_match_positional_arg_to_kwonly_param() {
    // `b` is keyword-only (after `*`). Calling `g(1, wrong)` positionally
    // would TypeError in Python. pykrete must not fire D0051 — the second
    // positional has no matching positional-or-regular slot.
    let result = check(
        r#"
class A(Schema):
    x: int

class B(Schema):
    y: int

def g(x: int, *, b: DataFrame[A]) -> None:
    return

def caller(wrong: DataFrame[B]) -> None:
    g(1, wrong)
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        0,
        "expected no D0051 when a positional arg slides past `*` into a kw-only param, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ===========================================================================
// Variadics — `*args` / `**kwargs` typed as DataFrame[Schema]
// ===========================================================================

#[test]
fn d0051_fires_for_each_vararg_with_wrong_schema() {
    // `union_all(*items: DataFrame[Sale])`. The caller passes one matching
    // arg (Sale) and one mismatching arg (Refund). pykrete must fire
    // exactly one D0051 — for the Refund argument.
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

class Refund(Schema):
    region: string
    refund: int

def union_all(*items: DataFrame[Sale]) -> DataFrame[Sale]:
    return items[0]

def caller(sale: DataFrame[Sale], refund: DataFrame[Refund]) -> None:
    union_all(sale, refund)
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        1,
        "expected exactly one D0051 for the Refund vararg, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert_message_contains(&result, "D0051", "Sale");
    assert_message_contains(&result, "D0051", "Refund");
    assert_message_contains(&result, "D0051", "amount"); // missing
    assert_message_contains(&result, "D0051", "refund"); // extra
}

#[test]
fn d0051_fires_for_kwarg_with_wrong_schema() {
    // `pile(**parts: DataFrame[Sale])`. The caller passes one matching
    // kwarg (Sale) and one mismatching kwarg (Refund). Exactly one
    // D0051 — for the Refund keyword argument.
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

class Refund(Schema):
    region: string
    refund: int

def pile(**parts: DataFrame[Sale]) -> DataFrame[Sale]:
    return parts["a"]

def caller(sale: DataFrame[Sale], refund: DataFrame[Refund]) -> None:
    pile(a=sale, b=refund)
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        1,
        "expected exactly one D0051 for the Refund kwarg, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert_message_contains(&result, "D0051", "Sale");
    assert_message_contains(&result, "D0051", "Refund");
}

// ===========================================================================
// Tuple-unpack shadowing — LHS is `(name, junk) = ...`
// ===========================================================================

#[test]
fn d0051_skipped_when_callee_shadowed_via_tuple_unpack() {
    // `revenue` is a top-level function expecting DataFrame[Sale]; the
    // caller rebinds the name locally via a tuple-unpack LHS. The
    // subsequent call resolves to the local at runtime, not the top-level
    // function — pykrete must not fire D0051 against the top-level signature.
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

class Order(Schema):
    qty: int

def revenue(sales: DataFrame[Sale]) -> DataFrame[Sale]:
    return sales

def some_helper(o: DataFrame[Order]) -> DataFrame[Order]:
    return o

def caller(orders: DataFrame[Order]) -> None:
    revenue, junk = some_helper(orders), 1
    revenue(orders)
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        0,
        "expected no D0051 when the callee name is shadowed via tuple-unpack, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ===========================================================================
// Walrus (:=) shadowing — name bound from inside an expression
// ===========================================================================

#[test]
fn d0051_skipped_when_callee_shadowed_via_walrus() {
    // The walrus operator (`(revenue := some_helper(orders))`) introduces
    // a local binding from inside an expression. The subsequent call
    // resolves to the local at runtime — pykrete must not fire D0051
    // against the top-level signature.
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int

class Order(Schema):
    qty: int

def revenue(sales: DataFrame[Sale]) -> DataFrame[Sale]:
    return sales

def some_helper(o: DataFrame[Order]) -> DataFrame[Order]:
    return o

def caller(orders: DataFrame[Order]) -> None:
    x = (revenue := some_helper(orders))
    revenue(orders)
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        0,
        "expected no D0051 when the callee name is shadowed via walrus, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ===========================================================================
// Duplicate-name kwarg — positional + same-name keyword on one slot
// ===========================================================================

#[test]
fn d0051_does_not_double_fire_when_kwarg_duplicates_positional() {
    // `f(wrong, a=wrong)` — Python rejects this as `TypeError: got
    // multiple values for argument 'a'`. The schema mismatch is real,
    // but it should fire exactly once (via the positional pass); the
    // keyword arg targeting an already-consumed slot must be skipped.
    let result = check(
        r#"
class A(Schema):
    x: int

class B(Schema):
    y: int

class Wrong(Schema):
    z: int

def f(a: DataFrame[A], b: DataFrame[B]) -> None:
    return

def caller(wrong: DataFrame[Wrong]) -> None:
    f(wrong, a=wrong)
"#,
    );
    let d0051s = result.diagnostics_with_code("D0051");
    assert_eq!(
        d0051s.len(),
        1,
        "expected exactly one D0051 on the positional `wrong` arg, got: {:?}",
        d0051s.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
