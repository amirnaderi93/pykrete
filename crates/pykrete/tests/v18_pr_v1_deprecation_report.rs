//! v1.8 PR-V1 — `pykrete check --deprecation-report` JSON envelope.
//!
//! Two surfaces, one PR:
//! 1. D0090 message amend: the diagnostic text gains "future pykrete
//!    v2.0" (less committal than the v1.6 "removed in pykrete v2.0")
//!    and surfaces the `--deprecation-report` flag inline.
//! 2. `--deprecation-report` envelope: every D0090-firing site gets a
//!    structured record with code, rule name, binding name, raw
//!    annotation, adjudicated dialect, and suggested rewrite. Summary
//!    counts pre-computed so CI gates / migration dashboards consume
//!    the report without re-aggregating.
//!
//! Negative-space coverage per v14-rule 4: empty fixture, canonical-
//! only fixture, every verdict (Spark / Pandas / Ambiguous), mutex
//! with `--report-aliases`, suppression of normal diagnostic output.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn tmpdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("pykrete-v18-pr-v1")
        .join(format!("{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create tmp dir");
    dir
}

fn write_pyk(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, body).expect("write pyk");
    p
}

fn run_deprecation_report(target: &Path) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(["check", "--deprecation-report"])
        .arg(target)
        .output()
        .expect("run pykrete check --deprecation-report");
    let exit = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (exit, stdout, stderr)
}

fn run_deprecation_report_json(target: &Path) -> (i32, serde_json::Value, String) {
    let (exit, stdout, stderr) = run_deprecation_report(target);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout:\n{stdout}"));
    (exit, v, stderr)
}

// ---------------------------------------------------------------
// Positive — one site per verdict
// ---------------------------------------------------------------

#[test]
fn single_spark_adjudicated_site_emits_one_record() {
    let dir = tmpdir("spark-one");
    let p = write_pyk(
        &dir,
        "one.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.createOrReplaceTempView('t')\n    return 0\n",
    );
    let (exit, v, stderr) = run_deprecation_report_json(&p);
    assert_eq!(exit, 0, "deprecation-report exits 0 (inventory, not gate)");
    assert!(
        !stderr.contains("D0090"),
        "stderr must not leak D0090 in report mode: {stderr:?}"
    );
    assert_eq!(v["deprecationReportVersion"], "1");
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1, "exactly one site: {sites:?}");
    let s = &sites[0];
    assert_eq!(s["code"], "D0090");
    assert_eq!(s["ruleName"], "deprecatedDataFrameAlias");
    assert_eq!(s["adjudicatedDialect"], "spark");
    assert_eq!(s["suggestedRewrite"], "SparkFrame[Sale]");
    assert_eq!(s["rawAnnotation"], "DataFrame[Sale]");
    assert_eq!(s["bindingName"], "df");
    assert!(s["file"].as_str().unwrap().ends_with("one.pyk"));
    assert_eq!(s["line"], 1);
    assert!(s["column"].as_u64().unwrap() >= 1);
    assert_eq!(v["summary"]["totalSites"], 1);
    assert_eq!(v["summary"]["byDialect"]["spark"], 1);
    assert_eq!(v["summary"]["byDialect"]["pandas"], 0);
    assert_eq!(v["summary"]["byDialect"]["ambiguous"], 0);
}

#[test]
fn single_pandas_adjudicated_site_suggests_pandasframe_rewrite() {
    let dir = tmpdir("pandas-one");
    let p = write_pyk(
        &dir,
        "one.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.assign(x=1)\n    return 0\n",
    );
    let (exit, v, _) = run_deprecation_report_json(&p);
    assert_eq!(exit, 0);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0]["adjudicatedDialect"], "pandas");
    assert_eq!(sites[0]["suggestedRewrite"], "PandasFrame[Sale]");
    assert_eq!(sites[0]["bindingName"], "df");
    assert_eq!(v["summary"]["byDialect"]["pandas"], 1);
    assert_eq!(v["summary"]["byDialect"]["spark"], 0);
}

#[test]
fn ambiguous_site_emits_null_suggested_rewrite() {
    let dir = tmpdir("ambiguous-one");
    // Mix Spark (createOrReplaceTempView) and pandas (.assign) signals on
    // the same binding → ambiguous verdict.
    let p = write_pyk(
        &dir,
        "one.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.assign(x=1)\n    df.createOrReplaceTempView('t')\n    return 0\n",
    );
    let (exit, v, _) = run_deprecation_report_json(&p);
    assert_eq!(exit, 0);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0]["adjudicatedDialect"], "ambiguous");
    assert!(
        sites[0]["suggestedRewrite"].is_null(),
        "ambiguous → null suggestedRewrite: {:?}",
        sites[0]
    );
    assert_eq!(v["summary"]["byDialect"]["ambiguous"], 1);
    assert_eq!(v["summary"]["byDialect"]["spark"], 0);
    assert_eq!(v["summary"]["byDialect"]["pandas"], 0);
}

#[test]
fn mixed_verdicts_summary_counts_match_per_verdict() {
    let dir = tmpdir("mixed");
    let p = write_pyk(
        &dir,
        "mixed.pyk",
        "\
def s(df: DataFrame[Sale]) -> int:
    df.createOrReplaceTempView('t')
    return 0


def p(df: DataFrame[Sale]) -> int:
    df.assign(x=1)
    return 0


def a(df: DataFrame[Sale]) -> int:
    df.assign(x=1)
    df.createOrReplaceTempView('t')
    return 0
",
    );
    let (exit, v, _) = run_deprecation_report_json(&p);
    assert_eq!(exit, 0);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 3, "three sites total: {sites:?}");
    assert_eq!(v["summary"]["totalSites"], 3);
    assert_eq!(v["summary"]["byDialect"]["spark"], 1);
    assert_eq!(v["summary"]["byDialect"]["pandas"], 1);
    assert_eq!(v["summary"]["byDialect"]["ambiguous"], 1);
}

// ---------------------------------------------------------------
// Negative — no D0090 sites
// ---------------------------------------------------------------

#[test]
fn no_dataframe_annotations_emits_empty_envelope() {
    let dir = tmpdir("empty");
    let p = write_pyk(&dir, "clean.pyk", "def f(x: int) -> int:\n    return x\n");
    let (exit, v, stderr) = run_deprecation_report_json(&p);
    assert_eq!(exit, 0);
    assert!(stderr.is_empty() || !stderr.contains("D0090"));
    assert_eq!(v["deprecationReportVersion"], "1");
    assert_eq!(v["sites"].as_array().unwrap().len(), 0);
    assert_eq!(v["summary"]["totalSites"], 0);
    assert_eq!(v["summary"]["byDialect"]["spark"], 0);
    assert_eq!(v["summary"]["byDialect"]["pandas"], 0);
    assert_eq!(v["summary"]["byDialect"]["ambiguous"], 0);
}

#[test]
fn canonical_frame_annotations_are_not_reported() {
    let dir = tmpdir("canonical");
    let p = write_pyk(
        &dir,
        "canonical.pyk",
        "\
def s(df: SparkFrame[Sale]) -> SparkFrame[Sale]:
    return df


def p(df: PandasFrame[Sale]) -> PandasFrame[Sale]:
    return df
",
    );
    let (exit, v, _) = run_deprecation_report_json(&p);
    assert_eq!(exit, 0);
    assert_eq!(
        v["sites"].as_array().unwrap().len(),
        0,
        "SparkFrame and PandasFrame are canonical, not aliases"
    );
    assert_eq!(v["summary"]["totalSites"], 0);
}

// ---------------------------------------------------------------
// Flag interactions
// ---------------------------------------------------------------

#[test]
fn deprecation_report_and_report_aliases_together_errors() {
    let dir = tmpdir("mutex");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let out = Command::new(bin())
        .args(["check", "--report-aliases", "--deprecation-report"])
        .arg(&p)
        .output()
        .expect("run pykrete check");
    let exit = out.status.code().unwrap_or(-1);
    assert_eq!(
        exit, 2,
        "mutex violation must exit 2 (usage error); got {exit}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "stderr should explain mutex: {stderr:?}"
    );
}

#[test]
fn deprecation_report_suppresses_normal_diagnostic_output() {
    let dir = tmpdir("suppress");
    // A file that would normally fire D0090 on every annotation slot —
    // verifies the report mode is invocation-only, like --report-aliases.
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> DataFrame[Sale]:\n    return df\n",
    );
    let (exit, stdout, stderr) = run_deprecation_report(&p);
    assert_eq!(exit, 0);
    assert!(
        !stderr.contains("warning D0090") && !stderr.contains("error D0090"),
        "no text diagnostics on stderr in report mode: {stderr:?}"
    );
    // The stdout should be a single JSON envelope, not the text
    // analyzer's per-file summary line.
    assert!(
        !stdout.contains("parsed OK"),
        "no text-mode summary line on stdout: {stdout:?}"
    );
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["deprecationReportVersion"], "1");
}

// ---------------------------------------------------------------
// D0090 message amend
// ---------------------------------------------------------------

#[test]
fn d0090_message_text_carries_new_v18_wording() {
    use pykrete::dataframe::format_d0090_message;
    let (message, suggestion) = format_d0090_message("DataFrame[Sale]");
    assert_eq!(suggestion, "SparkFrame[Sale]");
    assert!(
        message.contains("future pykrete v2.0"),
        "v1.8 wording softens 'removed in v2.0' to 'future pykrete v2.0': {message:?}"
    );
    assert!(
        message.contains("--deprecation-report"),
        "v1.8 message must surface the new flag: {message:?}"
    );
    assert!(
        message.contains("'DataFrame[Sale]'") && message.contains("'SparkFrame[Sale]'"),
        "both raw and rewrite must appear: {message:?}"
    );
}

#[test]
fn d0090_message_does_not_promise_removal_in_v2_0() {
    use pykrete::dataframe::format_d0090_message;
    let (message, _) = format_d0090_message("DataFrame[Sale]");
    // Guard against a regression to the v1.6/v1.7 wording. The bare
    // "will be removed in pykrete v2.0" was a committal claim; v1.8
    // shifts to "future pykrete v2.0" without a ship-date commitment.
    assert!(
        !message.contains("will be removed in pykrete v2.0"),
        "bare 'will be removed in pykrete v2.0' was v1.6/v1.7 wording: {message:?}"
    );
}
