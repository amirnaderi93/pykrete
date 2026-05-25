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
