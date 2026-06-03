//! Integration tests for the v1.3 pandas-spec parser landing.
//!
//! Covers piece (a) only — the annotation-recognition extension and
//! D0090 deprecation emission. Piece (b)'s check-site dispatch lands in
//! PR-B; this file does not exercise it.
//!
//! Spec: `docs/design/pandas-support.md` §3 (syntax), §6 (deprecation).

#![allow(non_snake_case)]

mod common;
use common::*;

// ===========================================================================
// SparkFrame[X] — canonical Spark form, no D0090
// ===========================================================================

#[test]
fn spark_frame_annotation_parses_and_checks_columns() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return df.select(col("id"), col("status"))
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn spark_frame_annotation_does_not_emit_d0090() {
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0090");
}

#[test]
fn spark_frame_catches_unknown_column_just_like_dataframe_used_to() {
    // The classic D0030 emission still fires under SparkFrame[X] —
    // proves the dialect-tag plumbing didn't sever the col-ref pipeline.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return df.select(col("typo"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ===========================================================================
// PandasFrame[X] — new canonical pandas form
// ===========================================================================

#[test]
fn pandas_frame_annotation_parses_and_binds_the_schema() {
    // PR-A only stamps the dialect tag; piece (b) check sites land in
    // PR-B. Verifying the slot binds (no D0020 on the schema name) is
    // the load-bearing check here — it proves `recognize_with_dialect`
    // accepted PandasFrame and resolved the inner schema.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]) -> PandasFrame[Orders]:
    return df
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn pandas_frame_d0020_fires_on_unknown_schema() {
    // Falsifiable: if PandasFrame[X] silently fell through (no
    // recognition), no D0020 would fire on the unknown schema name.
    let result = check(
        r#"
def f(df: PandasFrame[NoSuchSchema]) -> PandasFrame[NoSuchSchema]:
    return df
"#,
    );
    assert_has_code(&result, "D0020");
    assert_message_contains(&result, "D0020", "NoSuchSchema");
}

#[test]
fn pandas_frame_does_not_emit_d0090() {
    // PandasFrame is canonical, not an alias — D0090 must not fire.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: PandasFrame[Orders]) -> PandasFrame[Orders]:
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0090");
}

// ===========================================================================
// DataFrame[X] — deprecated alias, fires D0090
// ===========================================================================

#[test]
fn deprecated_dataframe_alias_emits_d0090_once_per_slot() {
    // Two slots (param + return), so two D0090 emissions. Falsifiable:
    // a per-file emission would yield 1; a missing emission would yield 0.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: DataFrame[Orders]) -> DataFrame[Orders]:
    return df
"#,
    );
    assert_count(&result, "D0090", 2);
}

#[test]
fn d0090_message_echoes_source_text() {
    // Spec §6 / Q7: the message uses what the user wrote
    // (`DataFrame[Orders]`), not the canonical `SparkFrame[Orders]`.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: DataFrame[Orders]) -> DataFrame[Orders]:
    return df
"#,
    );
    assert_message_contains(&result, "D0090", "'DataFrame[Orders]'");
    // The suggested rewrite is the canonical form.
    assert_message_contains(&result, "D0090", "SparkFrame[Orders]");
}

#[test]
fn dataframe_alias_still_resolves_schema_normally() {
    // D0090 is a warning — the alias still parses and binds. The body
    // check fires D0030 on the bad column, proving the slot was bound
    // (otherwise the col-ref check would silently skip).
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: DataFrame[Orders]) -> DataFrame[Orders]:
    return df.select(col("ghost"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "ghost");
}

#[test]
fn bare_dataframe_alias_fires_d0090() {
    // Bare `DataFrame` (no subscript) is still the deprecated alias.
    let result = check(
        r#"
def f(df: DataFrame) -> DataFrame:
    return df
"#,
    );
    assert_has_code(&result, "D0090");
}

// ===========================================================================
// Dialect preservation through Pick / Omit / Merge
// ===========================================================================
//
// Per spec §3: Pick/Omit/Merge are dialect-agnostic; the wrapping FRAME
// carries the dialect. The schema-derivation pipeline resolves the
// derived view the same way regardless of dialect — these tests verify
// no false-D0020 fires across the three operators on each dialect, and
// that the deprecated-alias variant still flags D0090 once.

#[test]
fn pandas_frame_with_pick_resolves_derived_schema_and_stays_clean() {
    let result = check(
        r#"
class Order(Schema):
    id: int
    status: string

def f(df: PandasFrame[Pick[Order, "id"]]) -> PandasFrame[Pick[Order, "id"]]:
    return df
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn pandas_frame_with_omit_resolves_derived_schema_and_stays_clean() {
    let result = check(
        r#"
class Order(Schema):
    id: int
    status: string

def f(df: PandasFrame[Omit[Order, "status"]]) -> PandasFrame[Omit[Order, "status"]]:
    return df
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn pandas_frame_with_merge_resolves_derived_schema_and_stays_clean() {
    let result = check(
        r#"
class A(Schema):
    a: int

class B(Schema):
    b: string

def f(df: PandasFrame[Merge[A, B]]) -> PandasFrame[Merge[A, B]]:
    return df
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn dataframe_alias_with_pick_still_fires_d0090() {
    // The frame is the deprecated alias; the inner Pick is dialect-
    // agnostic. D0090 fires once per slot (2 here) on the FRAME, not
    // on the Pick. The message echoes the full source text (Q7) and
    // the SparkFrame rewrite preserves the nested-bracket Pick payload
    // byte-for-byte — assert both literal forms appear in the message.
    let result = check(
        r#"
class Order(Schema):
    id: int
    status: string

def f(df: DataFrame[Pick[Order, "id"]]) -> DataFrame[Pick[Order, "id"]]:
    return df
"#,
    );
    assert_count(&result, "D0090", 2);
    assert_message_contains(&result, "D0090", "'DataFrame[Pick[Order, \"id\"]]'");
    assert_message_contains(&result, "D0090", "'SparkFrame[Pick[Order, \"id\"]]'");
}

#[test]
fn spark_frame_with_merge_does_not_emit_d0090() {
    let result = check(
        r#"
class A(Schema):
    a: int

class B(Schema):
    b: string

def f(df: SparkFrame[Merge[A, B]]) -> SparkFrame[Merge[A, B]]:
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0090");
}

// ===========================================================================
// Union annotations — spec §3 quiet ignore
// ===========================================================================

#[test]
fn cross_dialect_union_annotation_does_not_commit_a_dialect_or_fire_d0090() {
    // Per spec §3: SparkFrame[X] | PandasFrame[X] is deferred to v1.4.
    // The annotation does not commit a dialect tag, so no slot is
    // bound, no piece-(b) check fires, and no D0090 (neither name is
    // the deprecated alias). What we assert: no D0090.
    //
    // Falsifiable: if we accidentally drilled into the union and saw
    // `DataFrame` somewhere we'd fire D0090. We don't.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: SparkFrame[Orders] | PandasFrame[Orders]) -> SparkFrame[Orders] | PandasFrame[Orders]:
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0090");
}

// ===========================================================================
// I2 (round 2): Optional unwrap + Union skip pinning
// ===========================================================================

#[test]
fn optional_dataframe_alias_fires_d0090() {
    // `Optional[DataFrame[X]]` — the Optional wrapper does not suppress
    // the alias warning. The parser peels one Optional level and
    // re-recognizes; the alias flag survives. Two slots (param + return)
    // → two D0090 emissions.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: Optional[DataFrame[Orders]]) -> Optional[DataFrame[Orders]]:
    return df
"#,
    );
    assert_count(&result, "D0090", 2);
}

#[test]
fn union_dataframe_alias_does_not_fire_d0090() {
    // Union annotations are spec §3 quiet-ignored — even one arm being
    // the deprecated `DataFrame` alias does not fire D0090. Pinning
    // test: the alias check does not drill into BinOp arms.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: DataFrame[Orders] | PandasFrame[Orders]) -> DataFrame[Orders] | PandasFrame[Orders]:
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0090");
}

#[test]
fn optional_spark_frame_does_not_fire_d0090() {
    // Falsifiable companion: `Optional[SparkFrame[X]]` peels the same
    // way but the inner name is canonical — no D0090.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: Optional[SparkFrame[Orders]]) -> Optional[SparkFrame[Orders]]:
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0090");
}

// ===========================================================================
// M7 (round 2): PR-A / PR-B contract — dialect carried but not enforced
// ===========================================================================

#[test]
fn pandas_frame_slot_still_fires_d0030_via_spark_dispatch_today() {
    // M7 pinning (round 2): PR-A only stamps the dialect tag; the
    // check-site dispatch is still uniformly Spark-style. A PandasFrame
    // slot therefore still surfaces D0030 on an unknown column under
    // today's `.select(col(...))`-form, because the body walker doesn't
    // gate by dialect yet. Locks the PR-A-vs-PR-B contract: the dialect
    // is *carried* but not *enforced*. PR-B will replace this expectation
    // with a pandas-specific check.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: PandasFrame[Orders]) -> PandasFrame[Orders]:
    return df.select(col("ghost"))
"#,
    );
    assert_has_code(&result, "D0030");
}
