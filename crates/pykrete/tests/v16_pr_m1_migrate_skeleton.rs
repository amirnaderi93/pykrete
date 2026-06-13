//! v1.6 PR-M1 — `pykrete migrate` subcommand skeleton.
//!
//! Covers the CLI dispatch + dry-run modes (`--check` / `--diff`).
//! Default mode is the in-place rewriter, exercised by PR-M2; this
//! file keeps a smoke test there (`migrate_default_mode_rewrites_in_place`)
//! and defers exhaustive rewriter-correctness tests to
//! `v16_pr_m2_rewriter_core.rs`.
//!
//! Negative-space tests per v14-rule 4: no-args usage, mutually
//! exclusive flags, nonexistent file, unknown option.

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
    let base = std::env::temp_dir().join(format!("pykrete-v16-m1-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create tmpdir");
    base
}

// ---------------------------------------------------------------
// Happy-path: --check on clean tree vs aliased tree
// ---------------------------------------------------------------

#[test]
fn migrate_check_on_clean_file_exits_zero() {
    let dir = tmpdir("check-clean");
    let pyk = write_fixture(
        &dir,
        "clean.pyk",
        "\
class Sale(Schema):
    region: string

def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:
    return s
",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--check")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --check");

    assert!(
        out.status.success(),
        "expected exit 0 on clean tree, got {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn migrate_check_on_aliased_file_exits_one_and_lists_sites() {
    let dir = tmpdir("check-aliased");
    let pyk = write_fixture(
        &dir,
        "aliased.pyk",
        "\
class Sale(Schema):
    region: string

def f(s: DataFrame[Sale]) -> DataFrame[Sale]:
    return s
",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--check")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --check");

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 on aliased tree, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("would rewrite to SparkFrame[Sale]"),
        "stderr missing 'would rewrite to SparkFrame[Sale]': {stderr}"
    );
    // Both the param and the return annotation → at least two site lines.
    let site_lines: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("would rewrite to"))
        .collect();
    assert!(
        site_lines.len() >= 2,
        "expected >=2 site lines, got {}: {stderr}",
        site_lines.len()
    );
}

// ---------------------------------------------------------------
// --diff: produces a unified diff, makes no on-disk changes
// ---------------------------------------------------------------

#[test]
fn migrate_diff_on_aliased_file_prints_diff_and_does_not_write() {
    let dir = tmpdir("diff");
    let original = "\
class Sale(Schema):
    region: string

def f(s: DataFrame[Sale]) -> DataFrame[Sale]:
    return s
";
    let pyk = write_fixture(&dir, "aliased.pyk", original);
    let mtime_before = fs::metadata(&pyk).expect("stat").modified().expect("mtime");

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--diff")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --diff");

    assert!(
        out.status.success(),
        "diff should exit 0 even when changes are pending, got {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--- a/"),
        "diff missing '--- a/' header: {stdout}"
    );
    assert!(
        stdout.contains("+++ b/"),
        "diff missing '+++ b/' header: {stdout}"
    );
    // PR-M2: per-edit hunks. The def line is `4` in `original` (1: class,
    // 2: indent, 3: blank, 4: def). Both aliases on one line collapse
    // into a single `@@ -4,1 +4,1 @@` hunk.
    assert!(
        stdout.contains("@@ -4,1 +4,1 @@"),
        "diff missing per-edit hunk header for line 4: {stdout}"
    );
    assert!(
        stdout.contains("-def f(s: DataFrame[Sale]) -> DataFrame[Sale]:"),
        "diff missing minus-line for original signature: {stdout}"
    );
    assert!(
        stdout.contains("+def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:"),
        "diff missing plus-line for rewritten signature: {stdout}"
    );

    // File on disk must be unchanged.
    let after = fs::read_to_string(&pyk).expect("read back");
    assert_eq!(after, original, "--diff must not write to disk");
    let mtime_after = fs::metadata(&pyk).expect("stat").modified().expect("mtime");
    assert_eq!(mtime_before, mtime_after, "--diff must not touch mtime");
}

#[test]
fn migrate_diff_on_clean_file_produces_no_diff() {
    let dir = tmpdir("diff-clean");
    let pyk = write_fixture(
        &dir,
        "clean.pyk",
        "\
def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:
    return s
",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--diff")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --diff");

    assert!(out.status.success(), "exit non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.is_empty(),
        "expected empty diff on clean file, got: {stdout}"
    );
}

// ---------------------------------------------------------------
// Default mode: in-place rewrite (PR-M2; smoke test here, full
// coverage in v16_pr_m2_rewriter_core.rs)
// ---------------------------------------------------------------

#[test]
fn migrate_default_mode_rewrites_in_place() {
    let dir = tmpdir("default");
    let pyk = write_fixture(
        &dir,
        "aliased.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");

    assert!(
        out.status.success(),
        "default mode should exit 0 once the rewriter ships, got {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("rewrote:"),
        "stdout should report 'rewrote: <path>': {stdout}"
    );
    let after = fs::read_to_string(&pyk).expect("read back");
    assert_eq!(
        after, "def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:\n    return s\n",
        "file contents must be rewritten",
    );
}

// ---------------------------------------------------------------
// Help: --help and -h short-circuit
// ---------------------------------------------------------------

#[test]
fn migrate_help_prints_usage() {
    for flag in ["--help", "-h"] {
        let out = Command::new(bin())
            .arg("migrate")
            .arg(flag)
            .output()
            .expect("run pykrete migrate --help");
        assert!(
            out.status.success(),
            "{flag} exit non-zero: {:?}",
            out.status
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("pykrete migrate"),
            "{flag} help missing 'pykrete migrate': {stdout}"
        );
        assert!(
            stdout.contains("--check"),
            "{flag} help missing '--check': {stdout}"
        );
        assert!(
            stdout.contains("--diff"),
            "{flag} help missing '--diff': {stdout}"
        );
    }
}

#[test]
fn top_level_help_lists_migrate() {
    let out = Command::new(bin())
        .arg("--help")
        .output()
        .expect("run pykrete --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("migrate"),
        "top-level help missing 'migrate': {stdout}"
    );
}

// ---------------------------------------------------------------
// Negative-space tests (v14-rule 4)
// ---------------------------------------------------------------

#[test]
fn migrate_no_args_errors_with_usage_pointer() {
    let out = Command::new(bin())
        .arg("migrate")
        .output()
        .expect("run pykrete migrate");
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 on no-args, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("specify a file") || stderr.contains("pykrete migrate --help"),
        "stderr missing usage pointer: {stderr}"
    );
}

#[test]
fn migrate_check_and_diff_are_mutually_exclusive() {
    let dir = tmpdir("mutex");
    let pyk = write_fixture(&dir, "x.pyk", "def f() -> int: return 0\n");

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--check")
        .arg("--diff")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --check --diff");

    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 on mutex flag combo, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "stderr missing 'mutually exclusive': {stderr}"
    );
}

#[test]
fn migrate_diff_and_check_in_reverse_order_also_rejected() {
    let dir = tmpdir("mutex-rev");
    let pyk = write_fixture(&dir, "x.pyk", "def f() -> int: return 0\n");

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--diff")
        .arg("--check")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --diff --check");

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("mutually exclusive"));
}

#[test]
fn migrate_nonexistent_path_errors() {
    let out = Command::new(bin())
        .arg("migrate")
        .arg("--check")
        .arg("/this/path/should/not/exist/pykrete-test.pyk")
        .output()
        .expect("run pykrete migrate --check <nope>");

    assert_ne!(
        out.status.code(),
        Some(0),
        "expected non-zero on missing file"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not a file or directory") || stderr.contains("error"),
        "stderr missing not-found message: {stderr}"
    );
}

#[test]
fn migrate_unknown_option_errors() {
    let out = Command::new(bin())
        .arg("migrate")
        .arg("--bogus")
        .output()
        .expect("run pykrete migrate --bogus");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown option"),
        "stderr missing 'unknown option': {stderr}"
    );
}

// ---------------------------------------------------------------
// Directory walking reuses the check-side expander
// ---------------------------------------------------------------

#[test]
fn migrate_check_walks_directory_for_pyk_files() {
    let dir = tmpdir("dir-walk");
    let sub = dir.join("nested");
    fs::create_dir_all(&sub).expect("mkdir nested");
    write_fixture(
        &sub,
        "alias.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );
    write_fixture(
        &dir,
        "clean.pyk",
        "def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:\n    return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--check")
        .arg(&dir)
        .output()
        .expect("run pykrete migrate --check <dir>");

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 because nested alias.pyk has DataFrame[Sale], got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("alias.pyk"),
        "stderr should name the nested file: {stderr}"
    );
}
