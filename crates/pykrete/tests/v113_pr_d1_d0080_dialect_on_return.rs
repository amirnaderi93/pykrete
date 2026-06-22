//! v1.13 PR-D1 — D0080 dialect-on-return checker arm.
//!
//! Spec: `docs/design/v1.13-spec.md` §4.
//!
//! Pre-PR-D1, `check_return_type` (driver.rs) compared declared vs
//! actual column SETS and TYPES only — a function annotated
//! `-> SparkFrame[X]` whose body produced a `PandasFrame[X]` value
//! (e.g., via `.toPandas()`) passed silently because the column set
//! and types matched. PR-D1 plumbs the declared and actual dialect
//! into the same site and fires D0080 with a dedicated dialect-
//! mismatch clause when both sides resolve to concrete dialects and
//! the dialects differ.
//!
//! Per spec §4.1.3, the clause stays silent if either side's dialect
//! is unknown (opaque receiver, untracked constructor) — honest
//! silence per the TypeScript-style "narrow when you can, widen when
//! you can't" convention.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Spark-decl / pandas-body — D0080 fires with dialect-mismatch clause
// ---------------------------------------------------------------------------

#[test]
fn V113D1_spark_decl_returns_pandas_param_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]) -> SparkFrame[Orders]:
    return pdf
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
}

#[test]
fn V113D1_spark_decl_returns_topandas_chain_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return sdf.toPandas()
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
}

#[test]
fn V113D1_spark_decl_returns_pandas_chain_method_fires_d0080() {
    // pandas param flowing through a pandas-style method call still
    // carries the Pandas dialect tag through the chain walker.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]) -> SparkFrame[Orders]:
    return pdf.rename(columns={"status": "status"})
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
}

#[test]
fn V113D1_spark_decl_returns_topandas_through_filter_fires_d0080() {
    // `sdf.filter(...).toPandas()` — the chain walker descends past
    // the intermediate `.filter` to find the `.toPandas` flip.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return sdf.filter(col("status") == "OPEN").toPandas()
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
}

// ---------------------------------------------------------------------------
// Pandas-decl / spark-body — D0080 fires with dialect-mismatch clause
// ---------------------------------------------------------------------------

#[test]
fn V113D1_pandas_decl_returns_spark_param_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]) -> PandasFrame[Orders]:
    return sdf
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
    assert_message_contains(&result, "D0080", "body produces SparkFrame");
}

#[test]
fn V113D1_pandas_decl_returns_create_dataframe_kwarg_flip_fires_d0080() {
    // `spark.createDataFrame(rows, schema=schema_var)` with a Spark-
    // tagged `schema_var` flips the inherited dialect to Spark via
    // gate (a) of `cross_dialect_handoff_gate`.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(spark: SparkSession, rows: list, schema_var: SparkFrame[Orders]) -> PandasFrame[Orders]:
    return spark.createDataFrame(rows, schema=schema_var)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
    assert_message_contains(&result, "D0080", "body produces SparkFrame");
}

#[test]
fn V113D1_pandas_decl_returns_create_dataframe_pandas_positional_fires_d0080() {
    // `spark.createDataFrame(pdf)` with a Pandas-tagged `pdf` flips
    // to Spark via gate (b).
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(spark: SparkSession, pdf: PandasFrame[Orders]) -> PandasFrame[Orders]:
    return spark.createDataFrame(pdf)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
    assert_message_contains(&result, "D0080", "body produces SparkFrame");
}

#[test]
fn V113D1_pandas_decl_returns_spark_select_chain_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]) -> PandasFrame[Orders]:
    return sdf.select("id", "status")
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
    assert_message_contains(&result, "D0080", "body produces SparkFrame");
}

// ---------------------------------------------------------------------------
// Negative-space — D0080 dialect clause does NOT fire
// ---------------------------------------------------------------------------

#[test]
fn V113D1_spark_decl_returns_spark_body_silent() {
    // Same dialect on both sides, schemas match — no diagnostic.
    // Regression guard for the pre-existing arm.
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
fn V113D1_pandas_decl_returns_pandas_body_silent() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]) -> PandasFrame[Orders]:
    return pdf
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

#[test]
fn V113D1_opaque_receiver_dialect_unknown_no_dialect_clause() {
    // Spec §4.1.3 — when the actual dialect can't be resolved (opaque
    // function call), the dialect clause stays silent. The base
    // schema-set check still runs and stays silent here because the
    // call returns an Unknown schema (no fields to compare).
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

#[test]
fn V113D1_same_dialect_wrong_schema_type_still_fires_legacy_d0080() {
    // Regression guard for the pre-PR-D1 D0080 arm: same dialect on
    // both sides, but a column type mismatch on a shared column. The
    // legacy column-type clause still fires (no double-fire from the
    // dialect arm, since dialects match).
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
    assert_has_code(&result, "D0080");
    // Pre-existing message shape on the type-mismatch path mentions
    // the column name and the declared/actual types.
    assert_message_contains(&result, "D0080", "column 'amount'");
    // And NOT the dialect-mismatch phrasing — both sides are Spark.
    let dialect_message_present = result
        .diagnostics
        .iter()
        .any(|d| d.code == "D0080" && d.message.contains("body produces PandasFrame"));
    assert!(
        !dialect_message_present,
        "same-dialect mismatch must not emit the dialect-mismatch clause"
    );
}

#[test]
fn V113D1_same_dialect_wrong_columns_still_fires_legacy_d0050() {
    // Regression guard for the pre-PR-D1 column-set arm (D0050). Same
    // dialect, different column sets — D0050 fires; D0080 dialect
    // clause does NOT.
    let result = check(
        r#"
class In(Schema):
    city: string

class Out(Schema):
    name: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x
"#,
    );
    assert_has_code(&result, "D0050");
    let dialect_message_present = result
        .diagnostics
        .iter()
        .any(|d| d.code == "D0080" && d.message.contains("body produces"));
    assert!(
        !dialect_message_present,
        "same-dialect column-set mismatch must not emit a dialect-mismatch D0080"
    );
}

#[test]
fn V113D1_no_return_annotation_no_dialect_check() {
    // Without a declared return slot, there's nothing to check
    // against — the arm stays silent regardless of body dialect.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]):
    return sdf.toPandas()
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}
