//! v1.7 PR-A1 — paired positive + negative regression tests for the
//! `PANDAS_ONLY_SIGNALS` + `PANDAS_INHERITED_ARMS` extract.
//!
//! The extract is a pure refactor: the lists move from
//! `alias_adjudicate.rs` (and the inline `expr.rs` match arms) into the
//! shared `dialect_signals` module. These tests pin the load-bearing
//! consumer behaviors so a future drift between either consumer and
//! the shared lists regresses here.
//!
//! Paired structure per v1.6 retro rule 8 / v1.4 rule 4:
//!
//! - **Adjudicator (consumer of `PANDAS_ONLY_SIGNALS`)** — positive:
//!   a `.assign(...)` on a `DataFrame[X]` binding adjudicates as
//!   Pandas (rewriter emits `PandasFrame[X]`). Negative: a binding
//!   that uses no pandas-only signal (only `withColumn`) stays Spark.
//! - **Inherited-arm dispatch (consumer of `PANDAS_INHERITED_ARMS`)**
//!   — positive: `pdf.head(5).assign(x=1)` continues the chain on a
//!   pandas receiver (no col-ref D0030 against the synthesized
//!   `assign` schema). Negative: `sdf.head(5)` on a Spark receiver
//!   short-circuits at the terminal recognizer (the existing Spark
//!   semantics), so a chained `.select(...)` after `head` reports no
//!   schema-tracked diagnostic from this arm — proving the dispatch
//!   stays dialect-gated.

use std::fs;
use std::process::Command;

mod common;
use common::*;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn tmpdir(label: &str) -> std::path::PathBuf {
    let base =
        std::env::temp_dir().join(format!("pykrete-v17-pr-a1-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create tmpdir");
    base
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, source).expect("write fixture");
    path
}

#[test]
fn V17PRA1_assign_signals_pandas_through_adjudicator() {
    let dir = tmpdir("assign-signals-pandas");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.assign(x=1)\n    return 0\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success(), "migrate exit failed: {out:?}");

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(
        after.contains("PandasFrame[Sale]"),
        "assign-only binding must adjudicate as Pandas; got: {after}"
    );
    assert!(
        !after.contains("SparkFrame[Sale]"),
        "assign-only binding must not stay Spark; got: {after}"
    );
}

#[test]
fn V17PRA1_with_column_only_binding_stays_spark() {
    let dir = tmpdir("withcolumn-only");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.withColumn('x', 1)\n    return 0\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(
        after.contains("SparkFrame[Sale]"),
        "withColumn-only binding must adjudicate as Spark; got: {after}"
    );
    assert!(
        !after.contains("PandasFrame[Sale]"),
        "withColumn-only binding must not flip to Pandas; got: {after}"
    );
}

#[test]
fn V17PRA1_pivot_table_signals_pandas_through_adjudicator() {
    let dir = tmpdir("pivot-table-signals");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.pivot_table(values='v', index='i')\n    return 0\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(
        after.contains("PandasFrame[Sale]"),
        "pivot_table-only binding must adjudicate as Pandas; got: {after}"
    );
}

#[test]
fn V17PRA1_pdf_head_then_assign_chain_preserves_schema() {
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int


def f(pdf: PandasFrame[Sale]) -> int:
    pdf.head(5).assign(x=1)
    return 0
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn V17PRA1_sdf_head_terminates_chain_on_spark() {
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int


def f(sdf: SparkFrame[Sale]) -> int:
    sdf.head(5).assign(x=col("region"))
    return 0
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V17PRA1_pdf_take_then_assign_chain_preserves_schema() {
    let result = check(
        r#"
class Sale(Schema):
    region: string
    amount: int


def f(pdf: PandasFrame[Sale]) -> int:
    pdf.take([0, 2]).assign(x=1)
    return 0
"#,
    );
    assert_no_diagnostics(&result);
}

#[test]
fn V17PRA1_head_alone_does_not_classify_binding_as_pandas() {
    let dir = tmpdir("head-alone");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.head(5)\n    return 0\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(
        after.contains("SparkFrame[Sale]"),
        "head-only binding must stay Spark (default), not be classified Pandas: {after}"
    );
    assert!(
        !after.contains("PandasFrame[Sale]"),
        "head-only binding must NOT classify Pandas: {after}"
    );
}

#[test]
fn V17PRA1_drop_alone_does_not_classify_binding_as_pandas() {
    let dir = tmpdir("drop-alone");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.drop('x')\n    return 0\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(
        after.contains("SparkFrame[Sale]"),
        "drop-only binding must stay Spark; got: {after}"
    );
}
