//! v1.7 PR-M1 — `pykrete migrate` UX overhaul.
//!
//! Three atomic deliverables:
//! 1. `--check` becomes the default mode (TS-style `--noEmit`-by-default
//!    safety). `--apply` is the new opt-in for rewriting on disk.
//! 2. Parse-error surface: per-skipped-file `skipped (parse error): ...`
//!    line on stderr. Closes v1.6 architecture-audit Important 7.
//! 3. CRLF marker normalization: the `# pykrete: ambiguous` marker
//!    matches the source's predominant line ending. Closes v1.6 PR-M3
//!    round-3 deferral.
//!
//! v1.6 rewriter-correctness tests in `v16_pr_m2_rewriter_core.rs` and
//! `v16_pr_m3_adjudication_d0090.rs` were updated to pass `--apply`
//! explicitly; this file covers the default-mode flip and the two
//! companion changes per v1.7 spec §3.

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
    let base = std::env::temp_dir().join(format!("pykrete-v17-m1-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("create tmpdir");
    base
}

// ---------------------------------------------------------------
// Part 1 — `--check` default flip
// ---------------------------------------------------------------

#[test]
fn migrate_no_flag_runs_check_mode_does_not_write() {
    let dir = tmpdir("default-aliased");
    let original = "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n";
    let pyk = write_fixture(&dir, "aliased.pyk", original);

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");

    // Default == check; aliased file → exit 1, like `--check`.
    assert_eq!(
        out.status.code(),
        Some(1),
        "default mode on aliased file should exit 1 (check semantics), got {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    // File untouched.
    let after = fs::read_to_string(&pyk).expect("read back");
    assert_eq!(after, original, "default mode must NOT rewrite the file");

    // Per-site lines go to stdout (v1.6 PR-M3 round-2 reviewer B2:
    // matches --check's stdout-of-data convention).
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("would rewrite to SparkFrame[Sale]"),
        "stdout missing 'would rewrite to': {stdout}"
    );

    // Stderr warning fires when the user did not pass an explicit mode.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("v1.7+: `pykrete migrate` is dry-run by default"),
        "stderr missing v1.7 dry-run warning: {stderr}"
    );
    assert!(
        stderr.contains("--apply"),
        "stderr warning must mention --apply remediation: {stderr}"
    );
}

#[test]
fn migrate_no_flag_on_clean_tree_exits_zero_and_warns() {
    let dir = tmpdir("default-clean");
    let pyk = write_fixture(
        &dir,
        "clean.pyk",
        "def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:\n    return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");

    assert!(
        out.status.success(),
        "default mode on clean tree should exit 0, got {:?}",
        out.status
    );
    // Warning still fires (the user passed no mode flag — they need to
    // know rewrite-on-default was retired). Spec §3.1: warn once when
    // no `--check` / `--apply` / `--diff` was passed.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("v1.7+: `pykrete migrate` is dry-run by default"),
        "stderr missing v1.7 dry-run warning on clean tree: {stderr}"
    );
}

#[test]
fn migrate_explicit_check_flag_does_not_warn() {
    // The warning's purpose is to teach the v1.6 user that default
    // semantics flipped. A user who already typed `--check` knows.
    let dir = tmpdir("explicit-check");
    let pyk = write_fixture(
        &dir,
        "clean.pyk",
        "def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:\n    return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--check")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --check");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("v1.7+: `pykrete migrate` is dry-run by default"),
        "explicit --check should NOT emit the migration warning: {stderr}"
    );
}

#[test]
fn migrate_apply_writes_to_disk_no_warning() {
    let dir = tmpdir("apply");
    let original = "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n";
    let pyk = write_fixture(&dir, "aliased.pyk", original);

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --apply");

    assert!(
        out.status.success(),
        "--apply on aliased file should exit 0, got {:?}",
        out.status
    );
    let after = fs::read_to_string(&pyk).expect("read back");
    assert_eq!(
        after, "def f(s: SparkFrame[Sale]) -> SparkFrame[Sale]:\n    return s\n",
        "--apply must rewrite",
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("rewrote:") && stdout.contains("aliased.pyk"),
        "--apply stdout missing rewrote line: {stdout}"
    );
    // Explicit mode → no warning.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("v1.7+: `pykrete migrate` is dry-run by default"),
        "explicit --apply should NOT emit the migration warning: {stderr}"
    );
}

#[test]
fn migrate_diff_unchanged_from_v16() {
    let dir = tmpdir("diff");
    let original = "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n";
    let pyk = write_fixture(&dir, "aliased.pyk", original);

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--diff")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --diff");

    assert!(
        out.status.success(),
        "--diff should exit 0: {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--- a/"), "diff missing header: {stdout}");
    assert!(stdout.contains("+++ b/"), "diff missing header: {stdout}");

    // File on disk unchanged.
    let after = fs::read_to_string(&pyk).expect("read back");
    assert_eq!(after, original, "--diff must not write");

    // Explicit mode → no warning.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("v1.7+: `pykrete migrate` is dry-run by default"),
        "explicit --diff should NOT emit the migration warning: {stderr}"
    );
}

#[test]
fn migrate_apply_and_check_mutually_exclusive() {
    let dir = tmpdir("mutex-apply-check");
    let pyk = write_fixture(&dir, "x.pyk", "def f() -> int: return 0\n");

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg("--check")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --apply --check");

    assert_eq!(
        out.status.code(),
        Some(2),
        "exit 2 expected on mutex mode combo, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "stderr missing 'mutually exclusive': {stderr}"
    );
}

#[test]
fn migrate_apply_and_diff_mutually_exclusive() {
    let dir = tmpdir("mutex-apply-diff");
    let pyk = write_fixture(&dir, "x.pyk", "def f() -> int: return 0\n");

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg("--diff")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --apply --diff");

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("mutually exclusive"));
}

// ---------------------------------------------------------------
// Part 2 — Parse-error surface (closes v1.6 arch-audit Important 7)
// ---------------------------------------------------------------

#[test]
fn migrate_emits_stderr_per_parse_error_file() {
    let dir = tmpdir("parse-error");
    let bad = write_fixture(
        &dir,
        "bad.pyk",
        // Unbalanced paren — guaranteed parse error.
        "def f(:\n    return\n",
    );
    let clean = write_fixture(
        &dir,
        "clean.pyk",
        "def g(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--check")
        .arg(&bad)
        .arg(&clean)
        .output()
        .expect("run pykrete migrate");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skipped (parse error):"),
        "stderr missing 'skipped (parse error):' line: {stderr}"
    );
    // Must mention the bad file by path.
    assert!(
        stderr.contains(&*bad.to_string_lossy()),
        "stderr 'skipped' line missing bad-file path: {stderr}"
    );
    // The clean file is still processed → exit 1 (alias sites found).
    assert_eq!(
        out.status.code(),
        Some(1),
        "clean.pyk should still surface its aliases in check mode: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("clean.pyk") && stdout.contains("would rewrite"),
        "clean.pyk's sites must still surface: {stdout}"
    );
}

#[test]
fn migrate_apply_skips_parse_error_files_without_writing() {
    let dir = tmpdir("apply-parse-error");
    let bad_source = "def f(:\n    return\n";
    let bad = write_fixture(&dir, "bad.pyk", bad_source);
    let clean = write_fixture(
        &dir,
        "clean.pyk",
        "def g(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&bad)
        .arg(&clean)
        .output()
        .expect("run pykrete migrate --apply");

    assert!(
        out.status.success(),
        "--apply with one parse-error file should still succeed for the clean files: {:?}; stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    // Bad file untouched.
    let bad_after = fs::read_to_string(&bad).expect("read bad");
    assert_eq!(bad_after, bad_source, "bad file must NOT be rewritten");
    // Clean file rewritten.
    let clean_after = fs::read_to_string(&clean).expect("read clean");
    assert!(
        clean_after.contains("SparkFrame[Sale]"),
        "clean file should be rewritten despite bad sibling: {clean_after}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skipped (parse error):"),
        "stderr missing skipped line: {stderr}"
    );
}

// ---------------------------------------------------------------
// Part 3 — CRLF marker normalization (closes v1.6 PR-M3 round-3
// deferral). Unix-only via #[cfg(unix)] since CI is Linux.
// ---------------------------------------------------------------

#[test]
#[cfg(unix)]
fn migrate_apply_on_crlf_source_emits_crlf_marker() {
    // Source has Windows-style CRLF line endings. The ambiguous-marker
    // injection must match — no mixed-EOL output.
    let dir = tmpdir("crlf-marker");
    let crlf_source = concat!(
        "class Sale(Schema):\r\n",
        "    region: string\r\n",
        "\r\n",
        "\r\n",
        "def ambiguous(df: DataFrame[Sale]) -> int:\r\n",
        "    a = df.withColumn('x', 1)\r\n",
        "    b = df.assign(x=1)\r\n",
        "    return 0\r\n",
    );
    let pyk = write_fixture(&dir, "x.pyk", crlf_source);

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --apply");
    assert!(
        out.status.success(),
        "--apply failed: {:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let after = fs::read_to_string(&pyk).expect("read back");
    // Marker present.
    assert!(
        after.contains("# pykrete: ambiguous"),
        "marker missing: {after:?}"
    );
    // No raw LF-only `# pykrete: ambiguous\n` line — the marker line
    // must end with \r\n. Scan for the marker byte sequence.
    let bytes = fs::read(&pyk).expect("raw read");
    // Find `# pykrete: ambiguous` and verify the next 2 bytes are \r\n.
    let needle = b"# pykrete: ambiguous";
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("marker not found in raw bytes");
    let tail = &bytes[pos + needle.len()..];
    assert!(
        tail.starts_with(b"\r\n"),
        "marker line must end with CRLF on CRLF source; found tail={:?}",
        &tail[..tail.len().min(8)]
    );

    // No mixed EOL: every \n in the output must be preceded by \r.
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            assert!(
                i > 0 && bytes[i - 1] == b'\r',
                "mixed-EOL byte at index {i}: bare \\n found in CRLF source output"
            );
        }
    }
}

#[test]
#[cfg(unix)]
fn migrate_apply_on_crlf_source_is_idempotent() {
    // Re-running --apply on an unresolved CRLF-ambiguous file must NOT
    // double-stack the marker (the v1.6 PR-M3 round-2 idempotency
    // contract must continue to hold on CRLF input).
    let dir = tmpdir("crlf-idempotent");
    let crlf_source = concat!(
        "class Sale(Schema):\r\n",
        "    region: string\r\n",
        "\r\n",
        "\r\n",
        "def ambiguous(df: DataFrame[Sale]) -> int:\r\n",
        "    a = df.withColumn('x', 1)\r\n",
        "    b = df.assign(x=1)\r\n",
        "    return 0\r\n",
    );
    let pyk = write_fixture(&dir, "x.pyk", crlf_source);

    let out1 = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&pyk)
        .output()
        .expect("first run");
    assert!(out1.status.success(), "first run failed: {out1:?}");
    let after1 = fs::read_to_string(&pyk).expect("read 1");
    assert_eq!(
        after1.matches("# pykrete: ambiguous").count(),
        1,
        "first run should produce exactly one marker: {after1:?}"
    );

    let out2 = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&pyk)
        .output()
        .expect("second run");
    assert!(out2.status.success(), "second run failed: {out2:?}");
    let after2 = fs::read_to_string(&pyk).expect("read 2");
    assert_eq!(
        after2.matches("# pykrete: ambiguous").count(),
        1,
        "re-run on CRLF source must not double-stack marker: {after2:?}"
    );
    assert_eq!(
        after1, after2,
        "second --apply on a clean CRLF tree must be a no-op"
    );
}

#[test]
#[cfg(unix)]
fn migrate_apply_on_lf_source_still_emits_lf_marker() {
    // Regression guard: the LF case must still produce LF markers (per
    // spec §3.5 — "marker injected with `\n` EOL"). Without this guard
    // a future CRLF detection bug could silently convert LF sources to
    // CRLF on the marker line.
    let dir = tmpdir("lf-marker");
    let lf_source = "\
class Sale(Schema):
    region: string


def ambiguous(df: DataFrame[Sale]) -> int:
    a = df.withColumn('x', 1)
    b = df.assign(x=1)
    return 0
";
    let pyk = write_fixture(&dir, "x.pyk", lf_source);

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --apply");
    assert!(out.status.success(), "{out:?}");

    let bytes = fs::read(&pyk).expect("raw read");
    let needle = b"# pykrete: ambiguous";
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("marker not found");
    let tail = &bytes[pos + needle.len()..];
    assert!(
        tail.starts_with(b"\n") && !tail.starts_with(b"\r\n"),
        "marker on LF source must end with bare \\n; tail={:?}",
        &tail[..tail.len().min(8)]
    );
    // No \r anywhere — pure LF source must remain pure LF.
    assert!(
        !bytes.contains(&b'\r'),
        "LF source must NOT acquire any \\r bytes after rewrite"
    );
}
