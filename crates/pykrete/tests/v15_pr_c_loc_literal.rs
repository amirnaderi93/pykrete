//! v1.5 PR-C — `.loc[:, "col"]` literal-form column inference.
//!
//! Spec §4 promises that `pdf.loc[:, "col"]` fires D0030 when the
//! literal column name doesn't resolve against the pandas-tagged
//! receiver's schema, and that every non-literal column-indexer shape
//! (list, slice, callable, boolean-mask row, non-Pandas receiver) falls
//! through silently — deferred to v1.6.
//!
//! Each test below is a negative-space regression-guard per v14-rule 4:
//! the positive arm pins D0030 fires on the typo, and the deferral arms
//! pin that the non-literal shapes don't false-fire today (a future PR
//! that widens the arm can flip them).

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V15C_loc_literal_known_column_no_diagnostic:
//
// Positive base case — `pdf.loc[:, "status"]` where `status` is in the
// schema. No D0030, no other diagnostic on the literal.
// ---------------------------------------------------------------------------

#[test]
fn V15C_loc_literal_known_column_no_diagnostic() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    return pdf.loc[:, "status"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15C_loc_literal_typo_fires_d0030:
//
// Positive D0030 — `pdf.loc[:, "statuss"]` against a schema with
// `status`. D0030 fires with the typo'd name in the message and the
// "Did you mean 'status'?" suggestion attached.
// ---------------------------------------------------------------------------

#[test]
fn V15C_loc_literal_typo_fires_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    return pdf.loc[:, "statuss"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
    assert_message_contains(&result, "D0030", "status");
}

// ---------------------------------------------------------------------------
// V15C_loc_list_of_strings_does_not_fire_d0030:
//
// Deferred to v1.6 — `pdf.loc[:, ["status", "amount"]]` is the list-of-
// strings column-indexer shape. v1.5 PR-C is literal-only. The list
// element with a typo (`amountz`) must NOT fire D0030 today; the test
// pins the deferral so a future widening PR is an explicit choice, not
// an accidental drop.
// ---------------------------------------------------------------------------

#[test]
fn V15C_loc_list_of_strings_does_not_fire_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string
    amount: int

def f(pdf: PandasFrame[Orders]):
    return pdf.loc[:, ["status", "amountz"]]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15C_loc_non_slice_row_indexer_does_not_fire_d0030:
//
// Deferred to v1.6 — `pdf.loc[idx, "status"]` is a label-indexed row.
// The column literal happens to be a typo (`statuss`), but the spec
// scopes PR-C to bare-`:` rows only. No D0030 today; a future row-
// indexer-aware arm flips this.
// ---------------------------------------------------------------------------

#[test]
fn V15C_loc_non_slice_row_indexer_does_not_fire_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders], idx: int):
    return pdf.loc[idx, "statuss"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15C_loc_slice_column_does_not_fire_d0030:
//
// Deferred to v1.6 — `pdf.loc[:, "a":"z"]` is the column-range slice
// shape. Pandas walks columns lexicographically between the bounds at
// runtime; static checking needs an ordered-columns model pykrete
// doesn't have today. No D0030 today.
// ---------------------------------------------------------------------------

#[test]
fn V15C_loc_slice_column_does_not_fire_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    return pdf.loc[:, "a":"z"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15C_loc_on_spark_frame_does_not_fire_d0030:
//
// Negative — `.loc` doesn't exist on Spark DataFrames. A SparkFrame[X]
// receiver with `.loc[:, "typo"]` must fall through cleanly: pykrete
// shouldn't pretend `.loc` works on Spark and silently consult the
// schema. The shape is most likely a user mistake (wrong dialect) but
// PR-C's job is the pandas idiom; flagging the Spark case is a separate
// design decision.
// ---------------------------------------------------------------------------

#[test]
fn V15C_loc_on_spark_frame_does_not_fire_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]):
    return sdf.loc[:, "statuss"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15C_loc_on_unbound_receiver_does_not_fire_d0030:
//
// Existing-gate regression guard per v14-rule 4 + v14 PR-F1 — an
// unbound `helper.loc[:, "col"]` (no DataFrame binding in the body)
// must fall through with no schema lookup. Mirrors the PR-F1 dict-
// subscript gate test for the new arm.
// ---------------------------------------------------------------------------

#[test]
fn V15C_loc_on_unbound_receiver_does_not_fire_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    return helper.loc[:, "statuss"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15C_loc_literal_variable_column_does_not_fire_d0030:
//
// Deferred — `pdf.loc[:, col_var]` where the column is a variable.
// v1.5 PR-C is literal-only. A variable column reference is opaque to
// the static check; the deferral keeps this silent today.
// ---------------------------------------------------------------------------

#[test]
fn V15C_loc_literal_variable_column_does_not_fire_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders], col_var: str):
    return pdf.loc[:, col_var]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
