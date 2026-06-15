//! v1.7 PR-A2 — Spark-D1 discriminator closure: paired positive +
//! negative tests for the 14 Spark-only methods/attributes added to
//! `SPARK_DISCRIMINATORS` (`dialect_signals.rs`).
//!
//! Per v1.6 retro rule 8: every new discriminator gets a paired test
//! pair — positive (the signal classifies the binding as Spark) and
//! negative-defense (the signal does NOT classify the binding as
//! Pandas, i.e. it isn't silently in `PANDAS_ONLY_SIGNALS` too).
//!
//! Each test asserts both halves in one go (the v1.7 PR-A1 behavior
//! file uses the same one-test-two-asserts shape, mirrored here):
//!
//! - positive: rewritten file contains `SparkFrame[Sale]`,
//! - negative: rewritten file does NOT contain `PandasFrame[Sale]`.
//!
//! v1.6 spark-coverage audit D1 catalogued these as Spark-only surface
//! that was missing from the adjudicator's vocabulary. `corr` and
//! `cov` were in the original D1 candidate list but DROPPED here:
//! pandas exposes both as DataFrame methods with different return
//! shapes, so including either would mis-classify pandas bindings.

use std::fs;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn tmpdir(label: &str) -> std::path::PathBuf {
    let base =
        std::env::temp_dir().join(format!("pykrete-v17-pr-a2-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create tmpdir");
    base
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, source).expect("write fixture");
    path
}

/// Run `pykrete migrate --apply` on a single-file fixture with the
/// given `body` inside `def f(df: DataFrame[Sale]) -> int:`. Asserts
/// both halves of the discriminator: the binding becomes
/// `SparkFrame[Sale]` AND does not become `PandasFrame[Sale]`.
fn assert_spark_discriminator(label: &str, body: &str) {
    let dir = tmpdir(label);
    let source = format!("def f(df: DataFrame[Sale]) -> int:\n{body}    return 0\n");
    let pyk = write_fixture(&dir, "x.pyk", &source);

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success(), "migrate exit failed: {out:?}");

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(
        after.contains("SparkFrame[Sale]"),
        "[{label}] binding must adjudicate as Spark; got: {after}"
    );
    assert!(
        !after.contains("PandasFrame[Sale]"),
        "[{label}] binding must NOT classify as Pandas (negative defense); got: {after}"
    );
}

#[test]
fn v17_pr_a2_select_expr_classifies_spark() {
    assert_spark_discriminator("selectExpr", "    df.selectExpr('a + 1 as b')\n");
}

#[test]
fn v17_pr_a2_freq_items_classifies_spark() {
    assert_spark_discriminator("freqItems", "    df.freqItems(['a'])\n");
}

#[test]
fn v17_pr_a2_approx_quantile_classifies_spark() {
    assert_spark_discriminator(
        "approxQuantile",
        "    df.approxQuantile('a', [0.5], 0.01)\n",
    );
}

#[test]
fn v17_pr_a2_crosstab_classifies_spark() {
    assert_spark_discriminator("crosstab", "    df.crosstab('a', 'b')\n");
}

#[test]
fn v17_pr_a2_col_regex_classifies_spark() {
    assert_spark_discriminator("colRegex", "    df.colRegex('`a.*`')\n");
}

#[test]
fn v17_pr_a2_summary_classifies_spark() {
    assert_spark_discriminator("summary", "    df.summary()\n");
}

#[test]
fn v17_pr_a2_map_in_pandas_classifies_spark() {
    assert_spark_discriminator("mapInPandas", "    df.mapInPandas(fn, 'a int')\n");
}

#[test]
fn v17_pr_a2_map_in_arrow_classifies_spark() {
    assert_spark_discriminator("mapInArrow", "    df.mapInArrow(fn, 'a int')\n");
}

#[test]
fn v17_pr_a2_write_to_classifies_spark() {
    assert_spark_discriminator("writeTo", "    df.writeTo('cat.tbl').append()\n");
}

#[test]
fn v17_pr_a2_write_stream_classifies_spark() {
    assert_spark_discriminator("writeStream", "    df.writeStream.start()\n");
}

#[test]
fn v17_pr_a2_unpivot_classifies_spark() {
    assert_spark_discriminator("unpivot", "    df.unpivot(['a'], ['b', 'c'], 'k', 'v')\n");
}

#[test]
fn v17_pr_a2_rdd_attribute_classifies_spark() {
    assert_spark_discriminator("rdd", "    _ = df.rdd\n");
}

#[test]
fn v17_pr_a2_is_streaming_attribute_classifies_spark() {
    assert_spark_discriminator("isStreaming", "    _ = df.isStreaming\n");
}

#[test]
fn v17_pr_a2_spark_session_attribute_classifies_spark() {
    assert_spark_discriminator("sparkSession", "    _ = df.sparkSession\n");
}

/// Explicit guard that `corr` and `cov` are NOT in
/// `SPARK_DISCRIMINATORS` — pandas exposes both as DataFrame methods,
/// so promoting either to a Spark signal would mis-classify pandas
/// bindings. The D1 audit listed them as candidates; this test pins
/// the deliberate exclusion. If a future PR adds either name to
/// `SPARK_DISCRIMINATORS` without removing the corresponding pandas
/// surface, this test fails and the author has to re-justify.
#[test]
fn v17_pr_a2_corr_and_cov_excluded_from_spark_discriminators() {
    use pykrete::dialect_signals::SPARK_DISCRIMINATORS;
    assert!(
        !SPARK_DISCRIMINATORS.contains(&"corr"),
        "`corr` is shared with pandas; must not be a Spark discriminator"
    );
    assert!(
        !SPARK_DISCRIMINATORS.contains(&"cov"),
        "`cov` is shared with pandas; must not be a Spark discriminator"
    );
}
