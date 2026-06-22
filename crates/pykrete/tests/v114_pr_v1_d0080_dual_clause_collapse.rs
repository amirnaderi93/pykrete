//! v1.14 PR-V1 — D0080 dual-clause collapse on combined dialect+type
//! mismatch.
//!
//! Pre-PR-V1, a function annotated `-> SparkFrame[X]` whose body
//! returned a chain that BOTH flipped to Pandas AND had a column-type
//! mismatch on a shared column emitted TWO D0080 diagnostics at the
//! same return range: one with the dialect clause (`"declared as
//! SparkFrame ... but the body produces PandasFrame ..."`) and one
//! with the legacy column-type clause (`"column 'foo' is declared
//! int ... but the body produces str"`). Both are independent facts,
//! but two diagnostics at the same range is noise.
//!
//! PR-V1 collects every D0080 clause `check_return_type` produces on a
//! single return into a `Vec<String>` and emits ONE diagnostic. When
//! only one clause is present, the message is the pre-PR-V1 shape
//! verbatim (regression-safe). When two or more fire, they are joined
//! with `"; additionally, "` into a single sentence.
//!
//! Out of scope: D0083 (nullability) and D0050 (column-set) keep their
//! own codes and ranges; PR-V1 only collapses D0080-coded clauses.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Positive — combined dialect + type mismatch collapses into ONE D0080
// ---------------------------------------------------------------------------

#[test]
fn V114V1_dialect_flip_plus_column_type_mismatch_collapses_to_one_d0080() {
    // Body flips Spark→Pandas via `.toPandas()` AND the body's column
    // type doesn't match the declared schema's type on the shared
    // column `amount`.
    let result = check(
        r#"
class In(Schema):
    city: string
    amount: int

class Out(Schema):
    city: string
    amount: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.toPandas()
"#,
    );
    assert_count(&result, "D0080", 1);
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
    assert_message_contains(&result, "D0080", "column 'amount'");
    assert_message_contains(&result, "D0080", "additionally");
}

#[test]
fn V114V1_dialect_flip_plus_two_column_type_mismatches_collapses_to_one_d0080() {
    // Three clauses: dialect + two distinct column-type mismatches.
    let result = check(
        r#"
class In(Schema):
    city: string
    amount: int
    qty: int

class Out(Schema):
    city: string
    amount: string
    qty: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.toPandas()
"#,
    );
    assert_count(&result, "D0080", 1);
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
    assert_message_contains(&result, "D0080", "column 'amount'");
    assert_message_contains(&result, "D0080", "column 'qty'");
}

#[test]
fn V114V1_pandas_decl_spark_body_plus_column_type_mismatch_collapses() {
    // Symmetric direction: Pandas-declared, Spark body via `.select(...)`,
    // shared column type mismatch.
    let result = check(
        r#"
class In(Schema):
    city: string
    amount: int

class Out(Schema):
    city: string
    amount: string

def f(x: SparkFrame[In]) -> PandasFrame[Out]:
    return x.select("city", "amount")
"#,
    );
    assert_count(&result, "D0080", 1);
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
    assert_message_contains(&result, "D0080", "body produces SparkFrame");
    assert_message_contains(&result, "D0080", "column 'amount'");
    assert_message_contains(&result, "D0080", "additionally");
}

// ---------------------------------------------------------------------------
// Negative — only one clause fires; pre-PR-V1 single-clause message shape
// ---------------------------------------------------------------------------

#[test]
fn V114V1_dialect_only_no_additionally_phrase() {
    // Dialect flip but body schema matches declared schema (same columns,
    // same types). Only the dialect clause fires; the message MUST NOT
    // contain "additionally".
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(x: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return x.toPandas()
"#,
    );
    assert_count(&result, "D0080", 1);
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
    let has_additionally = result
        .diagnostics
        .iter()
        .any(|d| d.code == "D0080" && d.message.contains("additionally"));
    assert!(
        !has_additionally,
        "single-clause D0080 must not contain the 'additionally' connector"
    );
}

#[test]
fn V114V1_type_only_same_dialect_no_additionally_phrase() {
    // Same dialect on both sides — dialect clause stays silent. Only the
    // column-type clause fires; message MUST NOT contain "additionally".
    let result = check(
        r#"
class In(Schema):
    city: string
    amount: int

class Out(Schema):
    city: string
    amount: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x
"#,
    );
    assert_count(&result, "D0080", 1);
    assert_message_contains(&result, "D0080", "column 'amount'");
    let has_additionally = result
        .diagnostics
        .iter()
        .any(|d| d.code == "D0080" && d.message.contains("additionally"));
    assert!(
        !has_additionally,
        "single-clause D0080 must not contain the 'additionally' connector"
    );
}

#[test]
fn V114V1_dialect_flip_matching_schemas_single_d0080() {
    // toPandas() flip with EXACT schema match — only the dialect clause
    // is a fact. Pre-PR-V1 emitted a single D0080 here too; this is the
    // regression guard for that path.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return sdf.toPandas()
"#,
    );
    assert_count(&result, "D0080", 1);
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
}

// ---------------------------------------------------------------------------
// Regression — silent paths stay silent
// ---------------------------------------------------------------------------

#[test]
fn V114V1_same_dialect_matching_schemas_silent() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return sdf
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

#[test]
fn V114V1_opaque_receiver_silent_no_phantom_clauses() {
    // Honest silence per v1.13 spec §4.1.3 still holds — unknown body
    // dialect means no clauses collected, no diagnostic emitted.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f() -> SparkFrame[Orders]:
    return some_opaque_helper()
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

// ---------------------------------------------------------------------------
// Cross-code independence — D0050 / D0083 stay separate
// ---------------------------------------------------------------------------

#[test]
fn V114V1_column_set_mismatch_keeps_d0050_separate_from_d0080() {
    // Body flips dialect AND has a missing column. D0080 fires for the
    // dialect clause; D0050 fires for the column-set mismatch — they
    // carry different codes, so they MUST remain separate diagnostics.
    let result = check(
        r#"
class In(Schema):
    city: string

class Out(Schema):
    city: string
    amount: int

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.toPandas()
"#,
    );
    assert_has_code(&result, "D0080");
    assert_has_code(&result, "D0050");
    // The D0080 message should be the dialect clause (no type clause —
    // `amount` isn't shared between the schemas, so no type comparison).
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
}

#[test]
fn V114V1_combined_clauses_share_single_diagnostic_range() {
    // Both clauses MUST be carried on the same Diagnostic, which means
    // they share line + column. Asserting count==1 already implies this,
    // but pinning the message content here guards against a future
    // refactor that resurrects the double-fire by emitting two D0080s
    // at the same range (which would still satisfy "both clauses
    // appear somewhere in the result" but would not be a single
    // diagnostic).
    let result = check(
        r#"
class In(Schema):
    city: string
    amount: int

class Out(Schema):
    city: string
    amount: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.toPandas()
"#,
    );
    let d0080s: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.code == "D0080")
        .collect();
    assert_eq!(d0080s.len(), 1, "expected exactly one D0080");
    let msg = &d0080s[0].message;
    assert!(
        msg.contains("declared as SparkFrame") && msg.contains("column 'amount'"),
        "single D0080 must carry both clauses in one message; got: {msg}"
    );
}
