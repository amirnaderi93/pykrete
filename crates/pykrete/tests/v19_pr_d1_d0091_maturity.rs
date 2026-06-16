//! v1.9 PR-D1 — D0091 maturity tests.
//!
//! Spec: `docs/design/v1.9-spec.md` §3.1. Three atomic closures on the
//! v1.8 D0091 `Expr::Call` arm:
//!
//! 1. Strict-mode escalation: D0091 → error under
//!    `typeCheckingMode: "strict"`. Mirrors v1.6 PR-M3 D0090 precedent.
//! 2. Suggestion-drift tripwire: every `SPARK_DISCRIMINATORS` /
//!    `PANDAS_ONLY_SIGNALS` entry must have a suggestion OR be
//!    allowlisted. Tripwire test lives as a `#[cfg(test)] mod tests`
//!    inside `operations/expr.rs` (has access to private items); the
//!    integration-test side here exercises the message-text shape.
//! 3. Suggestion struct expansion: `CrossDialectSuggestion` carries
//!    `shape_changes: bool`; when set, D0091 appends `— note arg shape
//!    differs`.
//!
//! Closes v1.8 §10.4 deferral + v1.8 architecture audit findings
//! arch-I2 / I3.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Strict-mode escalation — D0091 → error under `CheckMode::Strict`.
// Default mode keeps warning (v1.8 behavior). Mirrors v1.6 PR-M3 D0090.
// ---------------------------------------------------------------------------

#[test]
fn V19D1_default_mode_cross_dialect_fires_as_warning() {
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

#[test]
fn V19D1_strict_mode_cross_dialect_escalates_to_error() {
    let result = check_strict(
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
        .expect("D0091 should fire under strict");
    assert_eq!(d.severity_label(), "error");
}

#[test]
fn V19D1_strict_mode_matching_dialect_does_not_fire() {
    let result = check_strict(
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
fn V19D1_default_mode_matching_dialect_does_not_fire() {
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
fn V19D1_strict_mode_spark_receiver_pandas_method_escalates() {
    let result = check_strict(
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
        .expect("D0091 should fire under strict");
    assert_eq!(d.severity_label(), "error");
}

// ---------------------------------------------------------------------------
// Suggestion-struct expansion — `shape_changes: true` appends "— note
// arg shape differs" to the D0091 message text. Paired positive +
// negative per v1.6 retro rule 8 + v14-rule 4.
// ---------------------------------------------------------------------------

const SHAPE_DIFFERS_HINT: &str = "— note arg shape differs";

#[test]
fn V19D1_withcolumnrenamed_message_contains_shape_differs_hint() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumnRenamed("region", "area")
"#,
    );
    assert_message_contains(&result, "D0091", SHAPE_DIFFERS_HINT);
}

#[test]
fn V19D1_withcolumn_message_contains_shape_differs_hint() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumn("region", lit("X"))
"#,
    );
    assert_message_contains(&result, "D0091", SHAPE_DIFFERS_HINT);
}

#[test]
fn V19D1_selectexpr_message_contains_shape_differs_hint() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.selectExpr("region")
"#,
    );
    assert_message_contains(&result, "D0091", SHAPE_DIFFERS_HINT);
}

#[test]
fn V19D1_assign_message_contains_shape_differs_hint() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.assign(area="X")
"#,
    );
    assert_message_contains(&result, "D0091", SHAPE_DIFFERS_HINT);
}

#[test]
fn V19D1_rename_message_contains_shape_differs_hint() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.rename(columns={"region": "area"})
"#,
    );
    assert_message_contains(&result, "D0091", SHAPE_DIFFERS_HINT);
}

#[test]
fn V19D1_merge_message_contains_shape_differs_hint() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale], other: SparkFrame[Sale]):
    return sdf.merge(other, on="region")
"#,
    );
    assert_message_contains(&result, "D0091", SHAPE_DIFFERS_HINT);
}

// `groupby` is the case-only rename → `groupBy`. Same shape, no hint.
#[test]
fn V19D1_groupby_message_does_not_contain_shape_differs_hint() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.groupby("region")
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire");
    assert!(
        !d.message.contains(SHAPE_DIFFERS_HINT),
        "groupby → groupBy is case-only rename, no shape-change hint expected. \
         Message was: {}",
        d.message
    );
}

// `toPandas` is in `SPARK_DISCRIMINATORS` but suggestion is `copy`
// with `shape_changes: false` (both are no-arg). Tested via the
// adjudicator route: a pandas-tagged receiver calling `toPandas` fires
// D0091 without the shape hint.
#[test]
fn V19D1_topandas_on_pandas_receiver_does_not_contain_shape_differs_hint() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.toPandas()
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire (PandasFrame calling Spark-only toPandas)");
    assert!(
        !d.message.contains(SHAPE_DIFFERS_HINT),
        "toPandas → copy is no-arg → no-arg, no shape-change hint expected. \
         Message was: {}",
        d.message
    );
}

// ---------------------------------------------------------------------------
// Suggestion-name expected per the audit table. Each maps to the
// `cross_dialect_suggestion_*` table; the test reads the suggestion
// field of the diagnostic. Asymmetric defense (v1.6 retro rule 8) +
// paste-from-source positive table for the 5+4 existing mappings.
// ---------------------------------------------------------------------------

fn d0091_suggestion(result: &pykrete::CheckResult) -> Option<&str> {
    result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .and_then(|d| d.suggestion.as_deref())
}

#[test]
fn V19D1_pandas_target_withcolumn_suggests_assign() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumn("region", lit("X"))
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("assign"));
}

#[test]
fn V19D1_pandas_target_withcolumns_suggests_assign() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumns({"region": lit("X")})
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("assign"));
}

#[test]
fn V19D1_pandas_target_withcolumnrenamed_suggests_rename() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumnRenamed("region", "area")
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("rename"));
}

#[test]
fn V19D1_pandas_target_withcolumnsrenamed_suggests_rename() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.withColumnsRenamed({"region": "area"})
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("rename"));
}

#[test]
fn V19D1_pandas_target_selectexpr_suggests_eval() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.selectExpr("region")
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("eval"));
}

#[test]
fn V19D1_spark_target_assign_suggests_withcolumn() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.assign(area="X")
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("withColumn"));
}

#[test]
fn V19D1_spark_target_rename_suggests_withcolumnrenamed() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.rename(columns={"region": "area"})
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("withColumnRenamed"));
}

#[test]
fn V19D1_spark_target_groupby_suggests_groupBy() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.groupby("region")
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("groupBy"));
}

#[test]
fn V19D1_spark_target_merge_suggests_join() {
    let r = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale], other: SparkFrame[Sale]):
    return sdf.merge(other, on="region")
"#,
    );
    assert_eq!(d0091_suggestion(&r), Some("join"));
}

// ---------------------------------------------------------------------------
// Allowlist negatives — methods deliberately in `NO_SUGGESTION_ALLOWLIST_*`
// fire D0091 but with NO suggestion (the message has the bare scold;
// adding a misleading rewrite is worse than no rewrite per the
// allowlist rationale).
//
// `repartition` → `NO_SUGGESTION_ALLOWLIST_SPARK` (Spark execution-plan
// control with no pandas analog).
// `applymap` → `NO_SUGGESTION_ALLOWLIST_PANDAS` (no clean Spark analog).
// ---------------------------------------------------------------------------

#[test]
fn V19D1_allowlisted_repartition_fires_d0091_without_suggestion() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.repartition(4)
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire (PandasFrame calling Spark-only repartition)");
    assert!(
        d.suggestion.is_none(),
        "repartition is allowlisted — no suggestion expected. Got: {:?}",
        d.suggestion
    );
    assert!(
        !d.message.contains("Use '."),
        "Allowlisted method should not have a 'Use ...' clause in the message. \
         Message was: {}",
        d.message
    );
}

#[test]
fn V19D1_allowlisted_applymap_fires_d0091_without_suggestion() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.applymap(lambda x: x)
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire (SparkFrame calling pandas-only applymap)");
    assert!(
        d.suggestion.is_none(),
        "applymap is allowlisted — no suggestion expected. Got: {:?}",
        d.suggestion
    );
    assert!(
        !d.message.contains("Use '."),
        "Allowlisted method should not have a 'Use ...' clause in the message. \
         Message was: {}",
        d.message
    );
}

// ---------------------------------------------------------------------------
// Strict-mode + each mapping: severity is `error`. Sample two
// (withColumn, rename) — a smoke check that the strict flip applies
// uniformly across suggestion arms, not just the one tested above.
// ---------------------------------------------------------------------------

#[test]
fn V19D1_strict_mode_withcolumnrenamed_severity_is_error() {
    let result = check_strict(
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
        .expect("D0091 should fire under strict");
    assert_eq!(d.severity_label(), "error");
}

#[test]
fn V19D1_strict_mode_rename_severity_is_error() {
    let result = check_strict(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.rename(columns={"region": "area"})
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire under strict");
    assert_eq!(d.severity_label(), "error");
}

#[test]
fn V19D1_strict_mode_allowlisted_repartition_severity_is_error() {
    let result = check_strict(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.repartition(4)
"#,
    );
    let d = result
        .diagnostics
        .iter()
        .find(|d| d.code == "D0091")
        .expect("D0091 should fire under strict (allowlisted methods still fire)");
    assert_eq!(d.severity_label(), "error");
}
