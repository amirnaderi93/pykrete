//! v1.10 PR-D2 — pandas `stack(level=, dropna=)` literal-form arm.
//!
//! Spec: `docs/design/v1.10-spec.md` §4.1. Pandas's `stack` reshapes
//! wide-to-long by moving column levels into the row index. Spark's
//! `stack` is `pyspark.sql.functions.stack` (a column free function),
//! NOT a DataFrame method — the arm's receiver-dialect gate
//! (`receiver_is_pandas_inherited`) keeps Spark receivers out so the
//! dispatch never misroutes. Mirrors v1.6 PR-D1 `pivot_table` (Unknown
//! result) and v1.7 PR-D1 `melt` (kwarg shape) gating.
//!
//! Result-schema scope (v1.10 minimum viable): the arm returns Unknown
//! (None). Full MultiIndex schema synthesis is deferred to v1.11+. The
//! chain dies gracefully on the next method call rather than
//! mis-tracking schema.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V110D2_literal_int_level_resolves_cleanly:
//
// Positive — `pdf.stack(level=0)` is the load-bearing happy path
// (single-int level selector). No column-name validation (int levels
// can't typo against column names); arm fires, returns Unknown, no
// diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_literal_int_level_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level=0)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_literal_string_level_resolves_cleanly:
//
// Positive — `pdf.stack(level='a')` where `'a'` is a column in the
// receiver. The arm validates the literal string against the receiver
// schema (D0030 path) and finds it; no diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_literal_string_level_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level="a")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_literal_string_list_level_resolves_cleanly:
//
// Positive — `pdf.stack(level=['a', 'b'])`, each entry validated
// independently. Both names resolve; no diagnostic. Vacuity guard for
// the typo-in-list case below.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_literal_string_list_level_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level=["a", "b"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_literal_string_typo_fires_D0030:
//
// Negative — typo'd `level='typo'` fires D0030 with "did you mean".
// Receiver schema has `id`/`a`/`b`; `typo` matches none.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_literal_string_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level="typo")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V110D2_literal_string_list_typo_fires_D0030:
//
// Negative — typo'd entry in `level=['typo', 'a']` fires D0030 on the
// bad entry only. Vacuity guard for the all-valid list case.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_literal_string_list_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level=["typo", "a"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V110D2_no_kwargs_resolves_cleanly:
//
// Edge case — `pdf.stack()` with no args uses pandas defaults
// (`level=-1, dropna=True`). Arm fires (the gate is method-name only,
// not kwarg-presence), no `level=` to validate, returns Unknown. No
// diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_no_kwargs_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack()
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_dropna_kwarg_resolves_cleanly:
//
// Positive — `dropna=` is accepted but not validated (any bool literal,
// or even a non-literal, is honest). Pairs with `level=0` for the
// common shape.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_dropna_kwarg_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level=0, dropna=True)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_non_literal_level_falls_through:
//
// Negative-space — `level=some_var` (a Name reference). Arm recognizes
// the method but the level shape is opaque; no col-refs collected, no
// diagnostic. The dynamic level is honest user code; the arm trusts it.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_non_literal_level_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales], lvl):
    return pdf.stack(level=lvl)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_non_literal_dropna_does_not_crash:
//
// Negative-space — `dropna=some_var` (a Name). The arm validates
// `level=0` (literal int — no col refs) and ignores the opaque
// dropna. No diagnostic and no panic on the unrecognized shape.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_non_literal_dropna_does_not_crash() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales], dn):
    return pdf.stack(level=0, dropna=dn)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_chain_after_stack_dies_silently:
//
// Positive regression — the arm returns Unknown (Series isn't a
// PandasFrame schema in pykrete's model). A follow-up `.reset_index()`
// on the Unknown result should NOT spuriously fire D0030 — the chain
// dies silently. Spec §4.1: "v1.10 minimum viable, full MultiIndex
// tracking deferred to v1.11+".
// ---------------------------------------------------------------------------

#[test]
fn V110D2_chain_after_stack_dies_silently() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level=0).reset_index()
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_spark_receiver_does_not_fire_D0030:
//
// Sibling-arm regression guard (per spec §4.1 receiver-dialect gate).
// Spark's `stack` is `pyspark.sql.functions.stack`, a column free
// function — NOT a DataFrame method. A `SparkFrame[X]` receiver
// invoking `.stack(...)` is wrong code (the method doesn't exist on
// Spark DataFrame), but the pandas arm MUST NOT fire D0030 against the
// receiver's columns — that would be a misroute. The chain dies
// silently; D0091 (cross-dialect) is the right vocabulary, not D0030.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_spark_receiver_does_not_fire_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(sdf: SparkFrame[Sales]):
    return sdf.stack(level=0)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_spark_receiver_typo_string_level_does_not_fire_D0030:
//
// Sibling-arm regression — stronger form of the above. A Spark
// receiver passing a string `level='typo'` would, if the pandas arm
// misrouted, fire D0030 (the receiver schema has no `typo` column).
// The receiver-dialect gate prevents that; D0030 must NOT fire.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_spark_receiver_typo_string_level_does_not_fire_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(sdf: SparkFrame[Sales]):
    return sdf.stack(level="typo")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_int_list_level_falls_through:
//
// Negative-space — `pdf.stack(level=[0, 1])` (int-list level selector,
// the MultiIndex shape). Arm matches the method but int entries have no
// column-name shape to validate; refs collection is empty, no
// diagnostic. Closes the literal-shape matrix.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_int_list_level_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level=[0, 1])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_none_level_falls_through:
//
// Negative-space — `pdf.stack(level=None)` (pandas accepts None as
// "stack all levels"). Arm matches the method but None has no
// column-name shape; refs collection is empty, no diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_none_level_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level=None)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V110D2_string_tuple_level_typo_fires_D0030:
//
// Negative — `pdf.stack(level=("typo",))` (tuple-of-strings form). The
// arm's string-list parser treats tuples the same as lists; the typo
// fires D0030. Closes the literal-shape matrix on the container axis.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_string_tuple_level_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.stack(level=("typo",))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V110D2_chain_from_pandas_arm_after_stack:
//
// Positive regression — composing with existing pandas arms. A
// `pdf.assign(c=expr).stack(level=0)` chain exercises the pandas arm
// dispatch order: assign's schema flows into stack's receiver
// resolution. Stack returns Unknown; no D0030 on the stack call.
// ---------------------------------------------------------------------------

#[test]
fn V110D2_chain_from_pandas_arm_after_stack() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.assign(c=1).stack(level=0)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
