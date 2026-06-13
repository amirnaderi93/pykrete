//! v1.6 PR-M3 — call-graph adjudication + D0090 strict-mode escalation.
//!
//! Three surfaces, one PR per v1.5 retro rule 6:
//! 1. Adjudication: `pykrete migrate` walks each binding's downstream
//!    usage and tags `spark` / `pandas` / `ambiguous`.
//! 2. D0090 escalation: warning under default mode, error under strict.
//! 3. (Docs ship alongside in the same PR; not tested here — they're
//!    prose.)
//!
//! Negative-space coverage per v14-rule 4: every pure case, every mixed
//! case, every fallback. Default-mode D0090 stays a warning as a
//! regression guard against the escalation accidentally firing globally.

use std::fs;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, source).expect("write fixture");
    path
}

fn tmpdir(label: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("pykrete-v16-m3-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create tmpdir");
    base
}

// ---------------------------------------------------------------
// Adjudication — Spark verdict
// ---------------------------------------------------------------

#[test]
fn pure_spark_usage_rewrites_to_sparkframe() {
    let dir = tmpdir("spark-pure");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.createOrReplaceTempView('t')\n    return df.withColumn('c', 1)\n",
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
        "spark verdict must rewrite to SparkFrame[Sale]: {after}"
    );
    assert!(
        !after.contains("PandasFrame["),
        "spark verdict must not emit PandasFrame: {after}"
    );
}

// ---------------------------------------------------------------
// Adjudication — Pandas verdict
// ---------------------------------------------------------------

#[test]
fn pure_pandas_usage_rewrites_to_pandasframe() {
    let dir = tmpdir("pandas-pure");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.assign(x=1)\n    return df.pivot_table(values='v', index='i')\n",
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
        "pandas verdict must rewrite to PandasFrame[Sale]: {after}"
    );
    assert!(
        !after.contains("SparkFrame[Sale]"),
        "pandas verdict must not keep SparkFrame: {after}"
    );
}

#[test]
fn loc_subscript_alone_signals_pandas() {
    let dir = tmpdir("loc");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return df.loc[0, 'region']\n",
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
        "df.loc[...] must signal pandas: {after}"
    );
}

#[test]
fn iloc_subscript_alone_signals_pandas() {
    let dir = tmpdir("iloc");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return df.iloc[0]\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(after.contains("PandasFrame[Sale]"), "{after}");
}

// ---------------------------------------------------------------
// Adjudication — Ambiguous verdict
// ---------------------------------------------------------------

#[test]
fn mixed_usage_is_ambiguous_and_keeps_dataframe_with_marker() {
    let dir = tmpdir("ambiguous");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.withColumn('a', 1)\n    return df.assign(b=2)\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    // Ambiguous: the rewriter is a no-op. The annotation stays
    // `DataFrame[Sale]` and the migrator prepends a marker so the user
    // sees what needs human review.
    assert!(
        after.contains("DataFrame[Sale]"),
        "ambiguous verdict must keep DataFrame[Sale]: {after}"
    );
    assert!(
        !after.contains("SparkFrame["),
        "ambiguous verdict must not rewrite to SparkFrame: {after}"
    );
    assert!(
        !after.contains("PandasFrame["),
        "ambiguous verdict must not rewrite to PandasFrame: {after}"
    );
    assert!(
        after.contains("# pykrete: ambiguous"),
        "migrator must emit `# pykrete: ambiguous` marker for ambiguous sites: {after}"
    );
}

#[test]
fn ambiguous_marker_lands_on_the_line_above_the_site() {
    let dir = tmpdir("marker-placement");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    df.withColumn('a', 1)\n    return df.assign(b=2)\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    let marker_line_idx = after
        .lines()
        .position(|l| l.trim() == "# pykrete: ambiguous")
        .expect("marker present");
    let site_line_idx = after
        .lines()
        .position(|l| l.contains("DataFrame[Sale]"))
        .expect("site present");
    assert_eq!(
        marker_line_idx + 1,
        site_line_idx,
        "marker must sit on the line directly above the site:\n{after}"
    );
}

// ---------------------------------------------------------------
// Adjudication — no-usage fallback (defaults to Spark per spec)
// ---------------------------------------------------------------

#[test]
fn unused_binding_defaults_to_spark() {
    let dir = tmpdir("unused");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 42\n",
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
        "no-usage fallback must default to SparkFrame: {after}"
    );
}

#[test]
fn return_only_annotation_defaults_to_spark() {
    let dir = tmpdir("return-only");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(x: int) -> DataFrame[Sale]:\n    return spark.read.parquet('x')\n",
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
        "return slot with no body binding must default to SparkFrame: {after}"
    );
}

// ---------------------------------------------------------------
// Adjudication — local ann-assign
// ---------------------------------------------------------------

#[test]
fn local_annassign_is_adjudicated_against_its_body_usage() {
    let dir = tmpdir("local-pandas");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f():\n    df: DataFrame[Sale] = something()\n    return df.assign(x=1)\n",
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
        "local ann-assign must inherit its usage's verdict: {after}"
    );
}

// ---------------------------------------------------------------
// D0090 escalation — strict mode
// ---------------------------------------------------------------

#[test]
fn d0090_is_error_under_strict_mode() {
    let dir = tmpdir("strict");
    write_fixture(&dir, "pykrete.json", r#"{"typeCheckingMode": "strict"}"#);
    write_fixture(
        &dir,
        "x.pyk",
        "class Sale(Schema):\n    region: string\n\ndef f(df: DataFrame[Sale]) -> int:\n    return 1\n",
    );

    let out = Command::new(bin())
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run pykrete check");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("D0090") || combined.contains("deprecatedDataFrameAlias"),
        "D0090 must fire under strict mode: stdout={stdout} stderr={stderr}"
    );
    assert!(
        combined.contains("error deprecatedDataFrameAlias"),
        "D0090 must be escalated to error under strict mode: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !out.status.success(),
        "strict mode + D0090 must exit non-zero: stdout={stdout} stderr={stderr}"
    );
}

// ---------------------------------------------------------------
// D0090 escalation — default mode regression guard
// ---------------------------------------------------------------

#[test]
fn d0090_is_warning_under_default_mode() {
    let dir = tmpdir("default-mode");
    // No pykrete.json — default `standard` mode.
    write_fixture(
        &dir,
        "x.pyk",
        "class Sale(Schema):\n    region: string\n\ndef f(df: DataFrame[Sale]) -> int:\n    return 1\n",
    );

    let out = Command::new(bin())
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run pykrete check");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("warning deprecatedDataFrameAlias"),
        "D0090 must remain warning under default mode: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !combined.contains("error deprecatedDataFrameAlias"),
        "D0090 must NOT escalate under default mode: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn d0090_is_warning_under_basic_and_standard_modes_too() {
    for mode in ["basic", "standard"] {
        let dir = tmpdir(&format!("mode-{mode}"));
        let config = format!("{{\"typeCheckingMode\": \"{mode}\"}}");
        write_fixture(&dir, "pykrete.json", &config);
        write_fixture(
            &dir,
            "x.pyk",
            "class Sale(Schema):\n    region: string\n\ndef f(df: DataFrame[Sale]) -> int:\n    return 1\n",
        );

        let out = Command::new(bin())
            .arg("check")
            .arg(&dir)
            .output()
            .expect("run pykrete check");

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let combined = format!("{stdout}{stderr}");
        assert!(
            !combined.contains("error deprecatedDataFrameAlias"),
            "D0090 must not escalate under {mode}: stdout={stdout} stderr={stderr}"
        );
    }
}

// ---------------------------------------------------------------
// Adjudication + migrator: full end-to-end
// ---------------------------------------------------------------

#[test]
fn migrate_then_check_strict_passes() {
    let dir = tmpdir("e2e");
    write_fixture(&dir, "pykrete.json", r#"{"typeCheckingMode": "strict"}"#);
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "class Sale(Schema):\n    region: string\n\ndef f(df: DataFrame[Sale]) -> int:\n    return df.withColumn('x', 1)\n",
    );

    let migrate_out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(migrate_out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(after.contains("SparkFrame[Sale]"), "{after}");

    let check_out = Command::new(bin())
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run pykrete check");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&check_out.stdout),
        String::from_utf8_lossy(&check_out.stderr)
    );
    assert!(
        !combined.contains("D0090"),
        "D0090 must be gone after migrate: {combined}"
    );
}
