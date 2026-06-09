//! v1.5 PR-D — `pykrete check --report-aliases` end-to-end tests.
//!
//! Drives the built `pykrete` binary the same way `cli.rs` does (via
//! `CARGO_BIN_EXE_pykrete`), so these run against the exact build under
//! test. The library-level walker and serializer have their own unit
//! tests in `crates/pykrete/src/alias_report.rs`; this file covers the
//! CLI wiring: flag parsing, suppression of normal diagnostic output,
//! exit-code-0 guarantee, JSON envelope on stdout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn tmpdir(test_name: &str) -> PathBuf {
    // Per-test directory under the OS temp root. Avoids collisions when
    // tests run concurrently and gives a stable cleanup target.
    let dir = std::env::temp_dir()
        .join("pykrete-v15-pr-d")
        .join(test_name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create tmp dir");
    dir
}

fn write_pyk(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, body).expect("write pyk");
    p
}

fn run_report(file_or_dir: &Path) -> (i32, serde_json::Value) {
    let out = Command::new(bin())
        .args(["check", "--report-aliases"])
        .arg(file_or_dir)
        .output()
        .expect("run pykrete check --report-aliases");
    let exit = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout:\n{stdout}"));
    (exit, v)
}

#[test]
fn report_aliases_positive_single_site_records_path_line_dialect() {
    let dir = tmpdir("positive_single_site");
    let p = write_pyk(
        &dir,
        "one.pyk",
        "\
class Sales(Schema):
    region: string


def f(df: DataFrame[Sales]) -> SparkFrame[Sales]:
    return df
",
    );
    let (exit, v) = run_report(&p);
    assert_eq!(exit, 0, "report-mode must exit 0 even when records emitted");
    assert_eq!(v["aliasReportVersion"], "1");
    let aliases = v["aliases"].as_array().expect("aliases array");
    assert_eq!(
        aliases.len(),
        1,
        "exactly one DataFrame[X] alias site: {aliases:?}"
    );
    let a = &aliases[0];
    assert_eq!(a["resolvedDialect"], "spark");
    assert_eq!(a["wouldBeReplacement"], "SparkFrame[Sales]");
    assert!(
        a["file"].as_str().unwrap().ends_with("one.pyk"),
        "file: {a:?}"
    );
    // The annotation sits on line 5 (the def line, 1-indexed).
    assert_eq!(a["line"], 5);
    assert!(a["column"].as_u64().unwrap() >= 1);
}

#[test]
fn report_aliases_positive_multi_site_records_every_alias() {
    let dir = tmpdir("positive_multi_site");
    let p = write_pyk(
        &dir,
        "multi.pyk",
        "\
class Sales(Schema):
    region: string


def f(a: DataFrame[Sales], b: DataFrame[Sales]) -> DataFrame[Sales]:
    return a


def g(df: DataFrame[Sales]) -> int:
    return 0
",
    );
    let (exit, v) = run_report(&p);
    assert_eq!(exit, 0);
    let aliases = v["aliases"].as_array().expect("array");
    // 3 in `f` (two params + return) + 1 in `g` (param) = 4.
    assert_eq!(aliases.len(), 4, "want 4 sites: {aliases:?}");
    assert!(
        aliases
            .iter()
            .all(|a| a["wouldBeReplacement"] == "SparkFrame[Sales]")
    );
    assert!(aliases.iter().all(|a| a["resolvedDialect"] == "spark"));
}

#[test]
fn report_aliases_negative_sparkframe_is_not_reported() {
    let dir = tmpdir("negative_sparkframe");
    let p = write_pyk(
        &dir,
        "spark_only.pyk",
        "\
class Sales(Schema):
    region: string


def f(df: SparkFrame[Sales]) -> SparkFrame[Sales]:
    return df
",
    );
    let (exit, v) = run_report(&p);
    assert_eq!(exit, 0);
    let aliases = v["aliases"].as_array().expect("array");
    assert!(aliases.is_empty(), "no alias: {aliases:?}");
}

#[test]
fn report_aliases_negative_pandasframe_is_not_reported() {
    let dir = tmpdir("negative_pandasframe");
    let p = write_pyk(
        &dir,
        "pandas_only.pyk",
        "\
class Sales(Schema):
    region: string


def f(df: PandasFrame[Sales]) -> PandasFrame[Sales]:
    return df
",
    );
    let (exit, v) = run_report(&p);
    assert_eq!(exit, 0);
    let aliases = v["aliases"].as_array().expect("array");
    assert!(aliases.is_empty(), "no alias: {aliases:?}");
}

#[test]
fn report_aliases_negative_empty_report_when_no_dataframe_annotations() {
    let dir = tmpdir("negative_empty");
    let p = write_pyk(
        &dir,
        "no_frames.pyk",
        "\
def f(x: int) -> int:
    return x
",
    );
    let (exit, v) = run_report(&p);
    assert_eq!(exit, 0, "exit code 0 with empty report");
    assert_eq!(v["aliasReportVersion"], "1");
    let aliases = v["aliases"].as_array().expect("array");
    assert!(aliases.is_empty(), "zero records: {aliases:?}");
}

#[test]
fn report_aliases_edge_nested_type_expression_is_still_reported() {
    let dir = tmpdir("edge_nested");
    let p = write_pyk(
        &dir,
        "nested.pyk",
        "\
class Sales(Schema):
    region: string


def f() -> Dict[str, DataFrame[Sales]]:
    return {}
",
    );
    let (exit, v) = run_report(&p);
    assert_eq!(exit, 0);
    let aliases = v["aliases"].as_array().expect("array");
    assert_eq!(aliases.len(), 1, "nested-alias site reported: {aliases:?}");
    assert_eq!(aliases[0]["wouldBeReplacement"], "SparkFrame[Sales]");
    assert_eq!(aliases[0]["resolvedDialect"], "spark");
}

#[test]
fn report_aliases_suppresses_normal_diagnostic_output_on_stderr() {
    // A file with a `DataFrame[X]` annotation normally fires D0090 on
    // stderr (warning); report-mode must not emit it. We also verify
    // stdout carries the JSON report instead of the usual text/JSON
    // diagnostic output.
    let dir = tmpdir("suppresses_diagnostics");
    let p = write_pyk(
        &dir,
        "warn.pyk",
        "\
class Sales(Schema):
    region: string


def f(df: DataFrame[Sales]) -> DataFrame[Sales]:
    return df
",
    );
    let out = Command::new(bin())
        .args(["check", "--report-aliases"])
        .arg(&p)
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("D0090"),
        "stderr leaked D0090 in report mode: {stderr:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["aliasReportVersion"], "1");
    assert_eq!(v["aliases"].as_array().expect("array").len(), 2);
}

#[test]
fn report_aliases_help_lists_the_flag() {
    let out = Command::new(bin())
        .args(["check", "--help"])
        .output()
        .expect("run check --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--report-aliases"),
        "check --help missing --report-aliases: {stdout}"
    );
}
