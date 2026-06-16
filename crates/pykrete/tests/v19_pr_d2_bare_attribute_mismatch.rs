//! v1.9 PR-D2 — D0091 bare-attribute cross-dialect mismatch warning.
//!
//! Spec: `docs/design/v1.9-spec.md` §3.2. v1.8 PR-D1 shipped D0091 against
//! the `Expr::Call` arm only — method-call mismatches like
//! `pdf.withColumn(...)` fire, but bare-attribute mismatches
//! (`pdf.rdd`, `sdf.loc[0]`) silently typecheck. PR-D2 closes v1.8 spark-
//! audit finding spark-I1 by adding a parallel firing site in the
//! `Expr::Attribute` arm at `crates/pykrete/src/operations/expr.rs:146`.
//!
//! Two new property tables in `dialect_signals.rs` drive the dispatch:
//!
//! - `SPARK_DISCRIMINATOR_PROPERTIES` — `rdd`, `isStreaming`,
//!   `sparkSession`. Pandas receiver accessing one of these fires
//!   D0091.
//! - `PANDAS_INHERITED_PROPERTIES` — `loc`, `iloc`, `iat`, `at`. Spark
//!   receiver accessing one of these fires D0091.
//!
//! Shared-method names in `PANDAS_INHERITED_ARMS` (`head`/`tail`/…)
//! continue to NEVER fire — a bare `df.head` reference is legitimately
//! callable on either dialect. Deprecated-alias receivers
//! (`df: DataFrame[X]`) skip the gate per spec §3.5 (the v2.0 migration
//! narrative is "adjudicate, then enforce"). Unannotated bindings
//! skip via the `Some(dialect)` gate.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Positive — Pandas receiver, Spark-only property
// (`SPARK_DISCRIMINATOR_PROPERTIES` entries).
// ---------------------------------------------------------------------------

#[test]
fn V19D2_pandas_rdd_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.rdd
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "rdd");
    assert_message_contains(&result, "D0091", "PandasFrame");
}

#[test]
fn V19D2_pandas_isstreaming_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.isStreaming
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "isStreaming");
}

#[test]
fn V19D2_pandas_sparksession_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.sparkSession
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "sparkSession");
}

// ---------------------------------------------------------------------------
// Positive — Spark receiver, pandas-only indexer
// (`PANDAS_INHERITED_PROPERTIES` entries).
// ---------------------------------------------------------------------------

#[test]
fn V19D2_spark_loc_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.loc
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "loc");
    assert_message_contains(&result, "D0091", "SparkFrame");
}

#[test]
fn V19D2_spark_iloc_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.iloc
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "iloc");
}

#[test]
fn V19D2_spark_iat_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.iat
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "iat");
}

#[test]
fn V19D2_spark_at_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.at
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "at");
}

// ---------------------------------------------------------------------------
// Negative — correct-dialect property accesses do not fire.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_spark_rdd_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.rdd
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V19D2_spark_isstreaming_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.isStreaming
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V19D2_pandas_loc_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.loc
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V19D2_pandas_iloc_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.iloc
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Negative — `PANDAS_INHERITED_ARMS` (shared method names) NEVER fire
// D0091 on bare attribute access. Regression guard against
// `head`/`tail`/`first`/`take`/`drop` leaking into the property
// dispatch.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_pandas_head_bare_attr_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.head
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V19D2_spark_head_bare_attr_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.head
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V19D2_pandas_drop_bare_attr_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.drop
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Negative — unannotated bindings skip the gate.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_unannotated_rdd_does_not_fire() {
    let result = check(
        r#"
def f(bare_df):
    return bare_df.rdd
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V19D2_unannotated_loc_does_not_fire() {
    let result = check(
        r#"
def f(bare_df):
    return bare_df.loc
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Negative — deprecated `DataFrame[X]` alias receiver skips the gate
// per spec §3.5. The v2.0 migration narrative is "adjudicate, then
// enforce" — flagging mismatch on a binding that hasn't yet picked its
// dialect is noise. Mirrors PR-D1's carve-out for the method case.
// D0090 still fires for the alias itself.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_deprecated_alias_rdd_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(df: DataFrame[Sale]):
    return df.rdd
"#,
    );
    assert_does_not_have_code(&result, "D0091");
    // D0090 still fires for the deprecated alias annotation itself.
    assert_has_code(&result, "D0090");
}

#[test]
fn V19D2_deprecated_alias_loc_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(df: DataFrame[Sale]):
    return df.loc
"#,
    );
    assert_does_not_have_code(&result, "D0091");
    assert_has_code(&result, "D0090");
}

// ---------------------------------------------------------------------------
// Negative — a bare attribute access NOT in either property table does
// NOT fire. Regression guard against over-firing.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_pandas_arbitrary_attr_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.columns
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V19D2_spark_arbitrary_attr_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.columns
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Subscript precedence — `pdf.loc[:, "x"]` on a Pandas receiver runs
// through the `Expr::Subscript` projection AND the recursive
// `analyze_expr(&s.value)` walk into the new D0091 arm. Pandas
// receiver + pandas `loc` property = no D0091 (correct dialect).
// Regression guard that the new arm doesn't double-fire on the
// established `loc_literal_subscript` recognizer's path.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_pandas_loc_literal_subscript_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.loc[:, "region"]
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// `sdf.loc[0]` — Spark receiver indexing through a pandas-only
// indexer. The subscript itself isn't directly diagnosed (the
// `loc_literal_subscript` recognizer is Pandas-gated and falls
// through), but the bare `sdf.loc` Attribute walked by
// `analyze_expr(&s.value)` IS in the new D0091 arm and fires.
#[test]
fn V19D2_spark_loc_subscript_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.loc[0]
"#,
    );
    assert_has_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Chain propagation — `inherited_dialect` walks through chain
// receivers so `pdf.head(5).rdd` reports Pandas at the chain bottom
// and the bare `.rdd` access fires D0091. Mirrors the
// `V18D1_chained_withcolumn_after_head` test from PR-D1.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_chained_rdd_after_head_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.head(5).rdd
"#,
    );
    assert_has_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Severity assertion — D0091 is `warning` always. The strict-mode arm
// for `Expr::Attribute` lands when PR-D1's strict-mode mechanism is
// merged and rebased into this arm; this test pins the default-mode
// severity so the rebase-time flip is explicit.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_severity_is_warning() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.rdd
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
// Local annotated-assign — same propagation behavior as PR-D1's
// `V18D1_local_annassign_*` tests. A `df: PandasFrame[X] = sdf.toPandas()`
// rebind carries the PandasFrame tag through; the bare `df.rdd`
// access fires D0091.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_local_annassign_pandas_rdd_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    pdf: PandasFrame[Sale] = sdf.toPandas()
    return pdf.rdd
"#,
    );
    assert_has_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Disjointness pin — the two new property tables share NO entries with
// `PANDAS_INHERITED_ARMS` (shared-with-Spark method names like
// `head`/`tail`). A leak would either over-fire D0091 (a `head` bare
// reference shouldn't be flagged) or under-fire (a `loc` access on a
// Spark frame must always fire). Regression guard for future
// property-list growth.
// ---------------------------------------------------------------------------

#[test]
fn V19D2_property_tables_are_disjoint_from_inherited_arms() {
    use pykrete::{
        PANDAS_INHERITED_ARMS, PANDAS_INHERITED_PROPERTIES, SPARK_DISCRIMINATOR_PROPERTIES,
    };
    for prop in SPARK_DISCRIMINATOR_PROPERTIES {
        assert!(
            !PANDAS_INHERITED_ARMS.contains(prop),
            "'{prop}' appears in both SPARK_DISCRIMINATOR_PROPERTIES and \
             PANDAS_INHERITED_ARMS. The lists must be disjoint — a \
             shared-with-Spark method name in the Spark-property table \
             would over-fire D0091 on `df.<name>` bare references."
        );
    }
    for prop in PANDAS_INHERITED_PROPERTIES {
        assert!(
            !PANDAS_INHERITED_ARMS.contains(prop),
            "'{prop}' appears in both PANDAS_INHERITED_PROPERTIES and \
             PANDAS_INHERITED_ARMS. Same disjointness rationale."
        );
    }
}
