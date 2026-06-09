//! v1.5 PR-A3 — dialect-gate `.head` / `.tail` / `.first` so they only
//! count as terminals on a Spark receiver. On pandas they return a
//! row-sliced DataFrame; the chain stays alive and the schema is
//! preserved.
//!
//! Spec: `docs/design/v1.5-spec.md` §2.3.
//!
//! Each test below is a negative-space regression-guard or a positive
//! shape-preservation guard. The observable signal is downstream
//! schema-checking: `.assign(amount=col("statuss"))` on a tracked
//! `PandasFrame[X]` fires D0030 on the missing `"statuss"`; on a dead
//! chain (terminal hit, receiver discarded) no D0030 fires. The
//! Spark-side counterpart uses `.select(col("statuss"))` — same logic,
//! opposite expectation.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V15A3_spark_head_terminates_chain:
//
// Positive Spark-side regression guard. On a `SparkFrame[X]` receiver,
// `.head()` is a terminal — it returns a `Row`. The chain dies, so a
// follow-up `.select(col("statuss"))` does NOT fire D0030 even though
// `"statuss"` would be a typo on `Orders`. The terminal-recognizer
// behavior on Spark is unchanged by PR-A3; this test pins that.
// ---------------------------------------------------------------------------

#[test]
fn V15A3_spark_head_terminates_chain() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.head().select(col("statuss"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15A3_pandas_head_preserves_chain_fires_d0030_on_downstream_typo:
//
// THE PR-A3 fix. On a `PandasFrame[X]` receiver, `.head()` returns a
// row-sliced DataFrame; the chain stays alive and the schema is
// preserved. A follow-up `.assign(amount=col("statuss"))` fires D0030
// on the typo because the schema is still tracked. Before PR-A3,
// `.head()` was a blanket terminal, the chain died, and D0030 did NOT
// fire — silent miss. Vacuity-verified by reverting the gate locally
// and confirming this test fails (documented in the PR brief).
// ---------------------------------------------------------------------------

#[test]
fn V15A3_pandas_head_preserves_chain_fires_d0030_on_downstream_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.head().assign(amount=col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ---------------------------------------------------------------------------
// V15A3_pandas_head_with_positional_arg_preserves_chain:
//
// `pdf.head(5)` — same behavior as `pdf.head()`. Positional arg is the
// row count; pykrete doesn't inspect it. The dialect-gate fires on the
// method name, so the chain stays alive regardless of args.
// ---------------------------------------------------------------------------

#[test]
fn V15A3_pandas_head_with_positional_arg_preserves_chain() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.head(5).assign(amount=col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ---------------------------------------------------------------------------
// V15A3_pandas_tail_preserves_chain:
//
// `pdf.tail()` — symmetric with `.head()`. Pandas semantics: last-N
// rows, returns a DataFrame. Spec §2.3 enumerates `tail` alongside
// `head`; this is the sibling-arm coverage.
// ---------------------------------------------------------------------------

#[test]
fn V15A3_pandas_tail_preserves_chain() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.tail().assign(amount=col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ---------------------------------------------------------------------------
// V15A3_pandas_first_preserves_chain:
//
// `pdf.first()` — sibling-arm coverage. Pandas `.first(offset)` returns
// rows within an initial DateOffset window; still a DataFrame. Treated
// the same as `.head`/`.tail` for schema-tracking purposes.
// ---------------------------------------------------------------------------

#[test]
fn V15A3_pandas_first_preserves_chain() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.first().assign(amount=col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ---------------------------------------------------------------------------
// V15A3_unbound_receiver_head_no_inference:
//
// `helper.head()` — `helper` has no DataFrame binding. The receiver
// resolution at the top of the method-call dispatch returns None and
// the chain bails before reaching the terminal recognizer. No
// diagnostic should fire, including no D0030 from a misread of the
// surrounding `df`'s schema.
// ---------------------------------------------------------------------------

#[test]
fn V15A3_unbound_receiver_head_no_inference() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    helper = 1
    helper.head()
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15A3_pandas_count_stays_terminal:
//
// `pdf.count()` returns a Series of per-column counts in pandas, NOT a
// DataFrame. Series tracking is out of v1.5 scope (spec §2.3); the
// terminal recognizer treats `.count` as terminal on both dialects.
// This test pins that a future PR doesn't silently route `.count`
// through the pandas pass-through arm. A follow-up `.assign(...)` on
// the chain MUST NOT trip the schema-check, because the chain is dead.
// ---------------------------------------------------------------------------

#[test]
fn V15A3_pandas_count_stays_terminal() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.count().assign(amount=col("statuss"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
