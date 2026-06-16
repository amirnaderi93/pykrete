//! v1.8 PR-D1 — spark-D2 cross-dialect method-mismatch warning
//! (D0091).
//!
//! Spec: `docs/design/v1.8-spec.md` §3. D0091 fires as a warning when
//! the receiver's dialect tag and the called method's vocabulary
//! disagree:
//!
//! - Pandas receiver calling a `SPARK_DISCRIMINATORS` method
//!   (`pdf.withColumn(...)`).
//! - Spark receiver calling a `PANDAS_ONLY_SIGNALS` method
//!   (`sdf.assign(...)`).
//!
//! Shared-with-Spark methods in `PANDAS_INHERITED_ARMS`
//! (`head`/`tail`/`first`/`take`/`drop`) NEVER fire D0091. Deprecated-
//! alias receivers (`df: DataFrame[X]`) skip the gate per spec §3.5
//! ("adjudicate, then enforce"). Untagged bindings (no annotation)
//! skip via the `Some(dialect)` gate.
//!
//! Severity is fixed at `Warning` this cycle; strict-mode escalation
//! is deferred to v1.9 (closes v1.7 §10.3 deferral).
//!
//! Back-compat: D0091 fires ALONGSIDE the existing dispatch arms.
//! `pdf.withColumn(...)` still typechecks as Spark today via the
//! un-gated `column_method_shape` arm at `expr.rs:1245-1283`; the
//! schema flows through unchanged. The warning surfaces the
//! mismatch; the v1.9 strict-mode arm will escalate it.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Positive — Pandas receiver, Spark-only method (the spec §3.1 lead case).
// ---------------------------------------------------------------------------

#[test]
fn V18D1_pandas_withcolumn_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumn("region", lit("X"))
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "withColumn");
    assert_message_contains(&result, "D0091", "PandasFrame");
}

#[test]
fn V18D1_pandas_withcolumn_suggestion_is_assign() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumn("region", lit("X"))
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire");
    assert_eq!(d.suggestion.as_deref(), Some("assign"));
}

#[test]
fn V18D1_pandas_selectexpr_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.selectExpr("region")
"#,
    );
    assert_has_code(&result, "D0091");
}

#[test]
fn V18D1_pandas_withcolumnrenamed_suggestion_is_rename() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumnRenamed("region", "area")
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire");
    assert_eq!(d.suggestion.as_deref(), Some("rename"));
}

// ---------------------------------------------------------------------------
// Positive — Spark receiver, pandas-only method.
// ---------------------------------------------------------------------------

#[test]
fn V18D1_spark_assign_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.assign(area="X")
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "assign");
    assert_message_contains(&result, "D0091", "SparkFrame");
}

#[test]
fn V18D1_spark_assign_suggestion_is_withcolumn() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.assign(area="X")
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire");
    assert_eq!(d.suggestion.as_deref(), Some("withColumn"));
}

// `sdf.melt(...)` and `sdf.pivot(...)` are deliberately NOT flagged on
// Spark receivers: both names appear in `PANDAS_ONLY_SIGNALS` (the
// adjudicator uses them as discriminators) but Spark exposes
// same-spelled surfaces too — `df.melt(ids, values, ...)` (Spark
// 3.4+) and `groupBy(...).pivot(...)`. See
// `SHARED_NAMES_NOT_PANDAS_ONLY_ON_SPARK` in `operations/expr.rs`.
// Regression-guard: confirm we do NOT fire on Spark's chained
// `groupBy().pivot()` form.
#[test]
fn V18D1_spark_groupby_pivot_chain_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string
    month: string
    amount: int

def f(sdf: SparkFrame[Sale]):
    return sdf.groupBy("region").pivot("month")
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V18D1_spark_rename_lowercase_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.rename(columns={"region": "area"})
"#,
    );
    assert_has_code(&result, "D0091");
}

#[test]
fn V18D1_spark_groupby_lowercase_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.groupby("region")
"#,
    );
    assert_has_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Negative — correct-dialect calls do not fire.
// ---------------------------------------------------------------------------

#[test]
fn V18D1_spark_withcolumn_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.withColumn("region", lit("X"))
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V18D1_pandas_assign_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.assign(area="X")
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Negative — shared methods in PANDAS_INHERITED_ARMS never fire D0091.
// Regression guard against `head`/`tail`/`first`/`take`/`drop` leaking
// into the discriminator lists or the cross-dialect classifier.
// ---------------------------------------------------------------------------

#[test]
fn V18D1_spark_head_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.head(5)
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V18D1_pandas_head_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.head(5)
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V18D1_pandas_tail_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.tail(5)
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Negative — unadjudicated bindings skip D0091.
//   - Bare un-annotated parameter: no dialect tag, no firing.
//   - Deprecated `DataFrame[X]` alias: dialect tag is "Spark" for
//     D0090's sake but spec §3.5 treats it as unadjudicated for the
//     spark-D2 carve-out. The v2.0 migration narrative is
//     "adjudicate, then enforce".
// ---------------------------------------------------------------------------

#[test]
fn V18D1_unannotated_assign_does_not_fire() {
    let result = check(
        r#"
def f(bare_df):
    return bare_df.assign(area="X")
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V18D1_deprecated_alias_withcolumn_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(df: DataFrame[Sale]):
    return df.withColumn("region", lit("X"))
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V18D1_deprecated_alias_assign_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(df: DataFrame[Sale]):
    return df.assign(area="X")
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Back-compat preservation — D0091 fires ALONGSIDE the existing arm
// dispatch. `pdf.withColumn(...)` still typechecks as Spark (the
// un-gated `column_method_shape` arm at expr.rs:1245 preserves the
// receiver schema). A chained `.select("region")` after the withColumn
// resolves cleanly — no spurious D0030. This is the load-bearing
// v1.9-strict-mode-deferral fact: D0091 is informational, not
// blocking.
// ---------------------------------------------------------------------------

#[test]
fn V18D1_back_compat_pandas_withcolumn_chain_still_typechecks() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumn("region", lit("X")).select("region")
"#,
    );
    // D0091 fires (the warning surface)...
    assert_has_code(&result, "D0091");
    // ...but the chain's downstream `.select("region")` still resolves
    // against the receiver schema; no spurious D0030 on the valid
    // column name.
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// Severity assertion — D0091 is `warning` always. The v1.9 strict-mode
// arm hasn't landed; flipping the severity here would let the v1.9
// escalation regression-slip.
// ---------------------------------------------------------------------------

#[test]
fn V18D1_severity_is_warning() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumn("region", lit("X"))
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire");
    assert_eq!(d.severity_label(), "warning");
}

// ---------------------------------------------------------------------------
// Dual-arm — `inherited_dialect` propagates through chain receivers
// (v1.5 retro rule 2 / v16-rule 1). `pdf.head(5).withColumn(...)`
// fires D0091 because `head` doesn't flip the dialect; the chain's
// bottom Name carries the Pandas tag through.
// ---------------------------------------------------------------------------

#[test]
fn V18D1_chained_withcolumn_after_head_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.head(5).withColumn("region", lit("X"))
"#,
    );
    assert_has_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Local annotated-assign — `df: PandasFrame[X] = sdf.toPandas()` then
// `df.withColumn(...)` fires D0091. The local rebind carries the
// PandasFrame tag through `bind_df` + the spec §3.5 gate fires on the
// local Name.
// ---------------------------------------------------------------------------

#[test]
fn V18D1_local_annassign_pandas_withcolumn_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    pdf: PandasFrame[Sale] = sdf.toPandas()
    return pdf.withColumn("region", lit("X"))
"#,
    );
    assert_has_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Local annotated-assign — `df: DataFrame[X]` annotated re-binding
// inherits the deprecated-alias bit and skips D0091. Mirror of the
// param-form deprecated-alias test above.
// ---------------------------------------------------------------------------

#[test]
fn V18D1_local_annassign_deprecated_alias_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(src: SparkFrame[Sale]):
    df: DataFrame[Sale] = src
    return df.assign(area="X")
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}
