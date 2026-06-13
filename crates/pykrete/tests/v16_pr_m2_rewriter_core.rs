//! v1.6 PR-M2 — `pykrete migrate` in-place rewriter core.
//!
//! Covers token-preserving `DataFrame[X]` → `SparkFrame[X]` substitution,
//! atomic write semantics, non-ASCII preservation, and multi-file/clean
//! mtime guarantees. Negative-space tests per v14-rule 4.
//!
//! Adjudication is single-discriminator (`spark`) — call-graph-aware
//! adjudication (`pandas` / `ambiguous`) lands in PR-M3.

use std::fs;
use std::process::Command;
use std::time::Duration;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, source).expect("write fixture");
    path
}

fn tmpdir(label: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("pykrete-v16-m2-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create tmpdir");
    base
}

// ---------------------------------------------------------------
// Positive: token-preserving rewrite of every alias in a file
// ---------------------------------------------------------------

#[test]
fn rewrites_every_alias_in_a_single_file() {
    let dir = tmpdir("triple");
    let original = "\
class Sale(Schema):
    region: string

def f(s: DataFrame[Sale]) -> DataFrame[Sale]:
    return s

def g(t: DataFrame[Sale]) -> int:
    return 0
";
    let pyk = write_fixture(&dir, "triple.pyk", original);

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");

    assert!(
        out.status.success(),
        "exit non-zero: {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read_to_string(&pyk).expect("read back");
    let expected = "\
class Sale(Schema):
    region: string

def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:
    return s

def g(t: SparkFrame[Sale]) -> int:
    return 0
";
    assert_eq!(after, expected, "rewrite must be byte-exact");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("rewrote:") && stdout.contains("triple.pyk"),
        "stdout missing rewrote line: {stdout}"
    );
}

// ---------------------------------------------------------------
// Negative-space: clean file is not written (mtime unchanged)
// ---------------------------------------------------------------

#[test]
fn clean_file_is_not_rewritten_and_mtime_unchanged() {
    let dir = tmpdir("clean");
    let pyk = write_fixture(
        &dir,
        "clean.pyk",
        "def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:\n    return s\n",
    );
    let mtime_before = fs::metadata(&pyk).expect("stat").modified().expect("mtime");
    // Filesystem mtime resolution is ~1s on some platforms; give the
    // mtime check a real chance to fail by sleeping past the boundary.
    std::thread::sleep(Duration::from_millis(1100));

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");

    assert!(
        out.status.success(),
        "exit non-zero on clean tree: {:?}",
        out.status
    );
    let mtime_after = fs::metadata(&pyk).expect("stat").modified().expect("mtime");
    assert_eq!(
        mtime_before, mtime_after,
        "clean file must not be re-written (mtime drift)"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("rewrote:"),
        "stdout must not announce a rewrite for a clean file: {stdout}"
    );
}

// ---------------------------------------------------------------
// Non-ASCII: bytes outside the alias range preserved exactly
// ---------------------------------------------------------------

#[test]
fn non_ascii_bytes_are_preserved() {
    let dir = tmpdir("nonascii");
    // Japanese "column" + a few accented letters in the body — every
    // byte outside `DataFrame[Sale]` must survive the rewrite.
    let original = "\
# カラム名: region (résumé)
def f(s: DataFrame[Sale]) -> DataFrame[Sale]:
    # 漢字 → préservé
    return s
";
    let pyk = write_fixture(&dir, "nonascii.pyk", original);

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success(), "exit non-zero: {:?}", out.status);

    let after = fs::read_to_string(&pyk).expect("read back");
    let expected = "\
# カラム名: region (résumé)
def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:
    # 漢字 → préservé
    return s
";
    assert_eq!(after, expected, "non-ASCII bytes must round-trip");
    // Sanity: the file must still contain the multi-byte sequences as-is.
    let bytes = fs::read(&pyk).expect("raw read");
    assert!(
        bytes.windows(3).any(|w| w == [0xE6, 0xBC, 0xA2]), // 漢
        "multi-byte sequence for 漢 missing from output"
    );
}

// ---------------------------------------------------------------
// Two aliases on the same line both rewrite
// ---------------------------------------------------------------

#[test]
fn two_aliases_on_one_line_both_rewritten() {
    let dir = tmpdir("oneline");
    let pyk = write_fixture(
        &dir,
        "oneline.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]: return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success(), "exit non-zero: {:?}", out.status);

    let after = fs::read_to_string(&pyk).expect("read back");
    assert_eq!(
        after, "def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]: return s\n",
        "both same-line aliases must be rewritten"
    );
}

// ---------------------------------------------------------------
// Bare DataFrame (no subscript) rewrites to bare SparkFrame
// ---------------------------------------------------------------

#[test]
fn bare_dataframe_rewrites_to_bare_sparkframe() {
    let dir = tmpdir("bare");
    let pyk = write_fixture(
        &dir,
        "bare.pyk",
        "def f(s: DataFrame) -> DataFrame:\n    return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success(), "exit non-zero: {:?}", out.status);

    let after = fs::read_to_string(&pyk).expect("read back");
    assert_eq!(
        after, "def f(s: SparkFrame) -> SparkFrame:\n    return s\n",
        "bare DataFrame must rewrite to bare SparkFrame"
    );
}

// ---------------------------------------------------------------
// Idempotence: running migrate twice yields the same file
// ---------------------------------------------------------------

#[test]
fn idempotent_second_run_is_a_no_op() {
    let dir = tmpdir("idemp");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );

    let out1 = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("first run");
    assert!(out1.status.success());
    let after1 = fs::read_to_string(&pyk).expect("read back 1");
    let mtime1 = fs::metadata(&pyk).expect("stat").modified().expect("mtime");
    std::thread::sleep(Duration::from_millis(1100));

    let out2 = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("second run");
    assert!(out2.status.success());
    let after2 = fs::read_to_string(&pyk).expect("read back 2");
    let mtime2 = fs::metadata(&pyk).expect("stat").modified().expect("mtime");

    assert_eq!(after1, after2, "second migrate must not change the file");
    assert_eq!(
        mtime1, mtime2,
        "second migrate must skip the write (file was clean)"
    );
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    assert!(
        !stdout2.contains("rewrote:"),
        "second run must not announce a rewrite: {stdout2}"
    );
}

// ---------------------------------------------------------------
// Multi-file: modified files get rewritten, clean files untouched
// ---------------------------------------------------------------

#[test]
fn multi_file_modified_written_clean_untouched() {
    let dir = tmpdir("multi");
    let aliased1 = write_fixture(
        &dir,
        "alpha.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );
    let aliased2 = write_fixture(
        &dir,
        "beta.pyk",
        "def g(s: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let clean = write_fixture(
        &dir,
        "clean.pyk",
        "def h(s: SparkFrame[Sale]) -> SparkFrame[Sale]:\n    return s\n",
    );
    let clean_mtime_before = fs::metadata(&clean)
        .expect("stat")
        .modified()
        .expect("mtime");
    std::thread::sleep(Duration::from_millis(1100));

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&dir)
        .output()
        .expect("run pykrete migrate <dir>");
    assert!(
        out.status.success(),
        "exit non-zero: {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let after_a = fs::read_to_string(&aliased1).expect("read alpha");
    assert!(
        after_a.contains("SparkFrame[Sale]") && !after_a.contains("DataFrame[Sale]"),
        "alpha not rewritten: {after_a}"
    );
    let after_b = fs::read_to_string(&aliased2).expect("read beta");
    assert!(
        after_b.contains("SparkFrame[Sale]") && !after_b.contains("DataFrame[Sale]"),
        "beta not rewritten: {after_b}"
    );

    let clean_mtime_after = fs::metadata(&clean)
        .expect("stat")
        .modified()
        .expect("mtime");
    assert_eq!(
        clean_mtime_before, clean_mtime_after,
        "clean file must not be re-written"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let rewrote_lines: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with("rewrote:"))
        .collect();
    assert_eq!(
        rewrote_lines.len(),
        2,
        "expected exactly 2 'rewrote:' lines, got: {stdout}"
    );
}

// ---------------------------------------------------------------
// Atomicity: a pre-existing tempfile with the migrator's naming
// convention is harmless (overwritten on next run) — and the
// rewriter never leaves a tempfile behind on success.
// ---------------------------------------------------------------

#[test]
fn tempfile_is_cleaned_up_after_successful_write() {
    let dir = tmpdir("atomic");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    // No `.<name>.pykrete-migrate.tmp` left dangling.
    let dangling: Vec<_> = fs::read_dir(&dir)
        .expect("read tmpdir")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains(".pykrete-migrate.tmp")
        })
        .collect();
    assert!(dangling.is_empty(), "tempfile not cleaned up: {dangling:?}");
}

// ---------------------------------------------------------------
// Sites adjudicated as Spark today (call-graph adjudication is PR-M3)
// ---------------------------------------------------------------

#[test]
fn every_site_resolves_to_sparkframe_in_pr_m2() {
    let dir = tmpdir("dialect");
    // Even when the binding is later used like a pandas DataFrame
    // (.assign(...)), v1.6 PR-M2 still emits `SparkFrame[X]` —
    // ambiguous-aware adjudication is PR-M3's scope.
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(pdf: DataFrame[Sale]) -> int:\n    return pdf.assign(x=1)\n",
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
        "PR-M2 must emit SparkFrame[X] for every alias (PR-M3 adds adjudication): {after}"
    );
    assert!(
        !after.contains("PandasFrame["),
        "PR-M2 must NOT emit PandasFrame[X] (that's PR-M3): {after}"
    );
}
