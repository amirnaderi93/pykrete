//! Explicit `name: SparkFrame[Schema] = …` annotations on local variable
//! assignments — the bridge to external dataframe sources.
//!
//! The motivating use case: real codebases load DataFrames from external
//! sources (`dal.read(...)`, `spark.read.csv(...)`, …) that pykrete can't
//! infer a schema for. Annotating the receiving local variable lets the
//! user re-enter the typed world, and downstream checks just work.
//!
//! Exercises:
//! - `D0020` — annotated local references an unknown schema.
//! - `D0021` — annotated local's `SparkFrame[…]` inner is not a bare name.
//! - The annotation is **authoritative**: the RHS is analyzed for its own
//!   diagnostics, but the local's bound schema comes from the annotation.

#![allow(non_snake_case)] // DataFrame, Schema appear in test names.

mod common;
use common::*;

// ===========================================================================
// Happy path — annotation bridges from an untyped expression
// ===========================================================================

#[test]
fn annotated_local_binds_to_the_declared_schema_even_when_RHS_is_opaque() {
    // `external_source(...)` is just a function call pykrete can't track.
    // Without the annotation, `raw` would be unbound and the .select
    // below would silently not check. With the annotation, .select runs
    // against Orders' field set and catches the typo.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f() -> SparkFrame[Orders]:
    raw: SparkFrame[Orders] = external_source()
    return raw.select(col("nope"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn annotated_local_propagates_to_downstream_chained_operations() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f() -> SparkFrame[Orders]:
    raw: SparkFrame[Orders] = some_loader()
    return raw.filter(col("price") > 0).select(col("place_code"), col("price"))
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn annotated_local_without_an_RHS_still_binds_the_schema() {
    // `raw: SparkFrame[Orders]` alone (no `= ...`) is uncommon in Python
    // but legal syntax. Treat the annotation as authoritative.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f() -> SparkFrame:
    raw: SparkFrame[Orders]
    return raw.select(col("place_code"))
"#,
    );
    assert_no_diagnostics(&result);
}

// ===========================================================================
// Diagnostics on the annotation itself
// ===========================================================================

#[test]
fn d0020_fires_when_annotated_local_references_an_unknown_schema() {
    let result = check(
        r#"
def f() -> SparkFrame:
    raw: SparkFrame[NoSuchSchema] = external_source()
    return raw
"#,
    );
    assert_has_code(&result, "D0020");
    assert_message_contains(&result, "D0020", "NoSuchSchema");
}

#[test]
fn d0021_fires_when_annotated_local_uses_a_subscripted_inner() {
    let result = check(
        r#"
def f() -> SparkFrame:
    raw: SparkFrame[list[str]] = external_source()
    return raw
"#,
    );
    assert_has_code(&result, "D0021");
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn annotation_does_not_double_check_when_RHS_is_also_a_typed_expression() {
    // The RHS is itself a typed expression we can analyze. We trust the
    // annotation; the RHS analysis runs its own diagnostics independently.
    // In this case, RHS produces Derived [place_code] but annotation says
    // Orders. There's no D0070-style "annotation-doesnt-match-RHS" check
    // in v0.1 — we just trust the annotation.
    //
    // The downstream `.select(col("price"))` should succeed because the
    // annotation says raw is Orders, which DOES have `price`.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(other: SparkFrame[Orders]) -> SparkFrame:
    raw: SparkFrame[Orders] = other.select(col("place_code"))
    return raw.select(col("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn annotation_with_a_non_DataFrame_type_is_silently_ignored() {
    // Type-annotating a regular Python variable with int / str / etc.
    // is fine — pykrete doesn't care about those.
    let result = check(
        r#"
class Orders(Schema):
    x: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    threshold: int = 42
    return raw.filter(col("x") > threshold)
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn body_diagnostics_on_RHS_still_fire_even_when_annotation_is_present() {
    // The RHS does its own check; col("ghost") doesn't exist on Orders.
    // The annotation on the LHS is processed independently.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(other: SparkFrame[Orders]) -> SparkFrame:
    raw: SparkFrame[Orders] = other.select(col("ghost"))
    return raw
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "ghost");
}
