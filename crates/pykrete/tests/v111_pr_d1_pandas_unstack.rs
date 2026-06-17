//! v1.11 PR-D1 — pandas `unstack(level=, fill_value=)` literal-form arm.
//!
//! Spec: `docs/design/v1.11-spec.md` §4.1. Pandas's `unstack` is the
//! inverse of `stack` (long-to-wide reshape: pivots an index level into
//! the columns). Spark has no DataFrame `unstack` method — the arm's
//! receiver-dialect gate (`receiver_is_pandas_inherited`) keeps Spark
//! receivers out so the dispatch never misroutes. Mirrors v1.10 PR-D2
//! `stack` (Unknown result) directly.
//!
//! Result-schema scope (v1.11 minimum viable): the arm returns Unknown
//! (None). Full MultiIndex schema synthesis is deferred to v1.13+. The
//! chain dies gracefully on the next method call rather than mis-
//! tracking schema.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V111D1_literal_int_level_resolves_cleanly:
//
// Positive — `pdf.unstack(level=0)` is the load-bearing happy path
// (single-int level selector). No column-name validation (int levels
// can't typo against column names); arm fires, returns Unknown, no
// diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_literal_int_level_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack(level=0)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_literal_string_level_resolves_cleanly:
//
// Positive — `pdf.unstack(level='outer')` where `'outer'` is a column in
// the receiver. The arm validates the literal string against the
// receiver schema (D0030 path) and finds it; no diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_literal_string_level_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack(level="outer")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_literal_string_list_level_resolves_cleanly:
//
// Positive — `pdf.unstack(level=['outer', 'inner'])`, each entry
// validated independently. Both names resolve; no diagnostic. Vacuity
// guard for the typo-in-list case below.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_literal_string_list_level_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack(level=["outer", "inner"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_literal_string_typo_fires_D0030:
//
// Negative — typo'd `level='outerr'` fires D0030 with "did you mean".
// Receiver schema has `id`/`outer`/`inner`; `outerr` matches none.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_literal_string_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack(level="outerr")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "outer");
}

// ---------------------------------------------------------------------------
// V111D1_literal_string_list_typo_fires_D0030:
//
// Negative — typo'd entry in `level=['outerr', 'inner']` fires D0030 on
// the bad entry only. Vacuity guard for the all-valid list case.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_literal_string_list_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack(level=["outerr", "inner"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "outer");
}

// ---------------------------------------------------------------------------
// V111D1_no_kwargs_resolves_cleanly:
//
// Edge case — `pdf.unstack()` with no args uses pandas defaults
// (`level=-1, fill_value=None`). Arm fires (the gate is method-name only,
// not kwarg-presence), no `level=` to validate, returns Unknown. No
// diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_no_kwargs_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack()
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_fill_value_non_literal_does_not_crash:
//
// Edge case — `fill_value=some_var` (a Name). The arm validates
// `level=0` (literal int — no col refs) and ignores the opaque
// fill_value. No diagnostic and no panic on the unrecognized shape.
// `fill_value` accepts anything; no validation.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_fill_value_non_literal_does_not_crash() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales], fv):
    return pdf.unstack(level=0, fill_value=fv)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_non_literal_level_falls_through:
//
// Negative-space — `level=some_var` (a Name reference). Arm recognizes
// the method but the level shape is opaque; no col-refs collected, no
// diagnostic. The dynamic level is honest user code; the arm trusts it.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_non_literal_level_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales], lvl):
    return pdf.unstack(level=lvl)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_spark_receiver_does_not_fire_D0030:
//
// Sibling-arm regression guard (per spec §4.1 receiver-dialect gate).
// Spark has no DataFrame `unstack` method. A `SparkFrame[X]` receiver
// invoking `.unstack(...)` is wrong code, but the pandas arm MUST NOT
// fire D0030 against the receiver's columns — that would be a misroute.
// The chain dies silently; D0091 is the right vocabulary for the
// cross-dialect mistake (and it's NOT misrouted from the unstack arm
// either, since the arm's gate doesn't fire).
// ---------------------------------------------------------------------------

#[test]
fn V111D1_spark_receiver_does_not_fire_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(sdf: SparkFrame[Sales]):
    return sdf.unstack(level=0)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_spark_receiver_typo_string_level_does_not_fire_D0030:
//
// Sibling-arm regression — stronger form. A Spark receiver passing a
// string `level='outerr'` would, if the pandas arm misrouted, fire
// D0030 (the receiver schema has `outer` but not `outerr`). The
// receiver-dialect gate prevents that; D0030 must NOT fire.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_spark_receiver_typo_string_level_does_not_fire_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(sdf: SparkFrame[Sales]):
    return sdf.unstack(level="outerr")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_string_tuple_level_resolves_cleanly:
//
// Positive — `pdf.unstack(level=("outer",))` (tuple-of-strings form).
// The arm's string-list parser treats tuples the same as lists;
// `outer` resolves cleanly. Closes the literal-shape matrix on the
// container axis.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_string_tuple_level_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack(level=("outer",))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_none_level_falls_through:
//
// Negative-space — `pdf.unstack(level=None)` (pandas accepts None as
// "unstack all levels"). Arm matches the method but None has no
// column-name shape; refs collection is empty, no diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_none_level_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack(level=None)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V111D1_int_list_level_falls_through:
//
// Negative-space — `pdf.unstack(level=[0, 1])` (int-list level
// selector, the MultiIndex shape). Arm matches the method but int
// entries have no column-name shape to validate; refs collection is
// empty, no diagnostic. Closes the literal-shape matrix.
// ---------------------------------------------------------------------------

#[test]
fn V111D1_int_list_level_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    outer: int
    inner: int

def f(pdf: PandasFrame[Sales]):
    return pdf.unstack(level=[0, 1])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
