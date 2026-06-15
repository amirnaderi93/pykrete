//! v1.6 PR-M2 — `pykrete migrate` in-place rewriter core.
//!
//! Covers token-preserving `DataFrame[X]` → `SparkFrame[X]` substitution,
//! atomic write semantics, non-ASCII preservation, and multi-file/clean
//! mtime guarantees. Negative-space tests per v14-rule 4.
//!
//! Call-graph adjudication (`spark` / `pandas` / `ambiguous`) lands in
//! PR-M3 — see `v16_pr_m3_adjudication_d0090.rs` for that surface.
//!
//! v1.7 PR-M1 flipped `pykrete migrate <path>`'s default mode from
//! rewrite to check; tests below pass `--apply` explicitly to keep
//! exercising the rewriter path. The flipped-default semantics are
//! covered in `v17_pr_m1_check_default.rs`.

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
        .arg("--apply")
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
        .arg("--apply")
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
        .arg("--apply")
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
        .arg("--apply")
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
        .arg("--apply")
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
        .arg("--apply")
        .arg(&pyk)
        .output()
        .expect("first run");
    assert!(out1.status.success());
    let after1 = fs::read_to_string(&pyk).expect("read back 1");
    let mtime1 = fs::metadata(&pyk).expect("stat").modified().expect("mtime");
    std::thread::sleep(Duration::from_millis(1100));

    let out2 = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
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
        .arg("--apply")
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
        .arg("--apply")
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
// PR-M3 adjudication contract — pandas-shaped bindings emit PandasFrame[X]
// ---------------------------------------------------------------
//
// v1.6 PR-M3 shipped call-graph adjudication: a binding used only via
// pandas-only methods (here `.assign(...)`) re-tags from the parser-level
// Spark default to Pandas, and the rewriter emits `PandasFrame[X]`. This
// test was deliberately forward-incompatible in PR-M2 — it asserted the
// opposite so PR-M3's landing would force the flip. The inversion below
// is that flip.
#[test]
fn pandas_shaped_binding_resolves_to_pandasframe_post_m3() {
    let dir = tmpdir("dialect");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(pdf: DataFrame[Sale]) -> int:\n    return pdf.assign(x=1)\n",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success());

    let after = fs::read_to_string(&pyk).expect("read back");
    assert!(
        after.contains("PandasFrame[Sale]"),
        "PR-M3 adjudication: pandas-shaped binding must emit PandasFrame[X]: {after}"
    );
    assert!(
        !after.contains("SparkFrame[Sale]"),
        "PR-M3 adjudication: must NOT keep SparkFrame[X] when usage is pandas-only: {after}"
    );
}

// ---------------------------------------------------------------
// Round-2 reviewer findings — regression tests
// ---------------------------------------------------------------

// Symlink corruption: round-1 atomic_write replaced the symlink ENTRY
// with a regular file, silently de-linking. Round 2 canonicalizes
// through the symlink first, so the migration writes through to the
// real target and the symlink topology survives.
#[test]
#[cfg(unix)]
fn migrate_through_symlink_writes_target_not_link() {
    let dir = tmpdir("symlink");
    let target = write_fixture(
        &dir,
        "real.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );
    let link = dir.join("link.pyk");
    std::os::unix::fs::symlink(&target, &link).expect("create symlink");

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&link)
        .output()
        .expect("run pykrete migrate");
    assert!(out.status.success(), "exit was {:?}", out.status);

    // Real target was rewritten.
    let after = fs::read_to_string(&target).expect("read real");
    assert!(
        after.contains("SparkFrame[Sale]"),
        "target file not rewritten: {after}"
    );
    // Symlink entry survives as a symlink.
    let link_meta = fs::symlink_metadata(&link).expect("symlink_metadata");
    assert!(
        link_meta.file_type().is_symlink(),
        "link.pyk is no longer a symlink — atomic_write de-linked it"
    );
}

// No-trailing-newline files: --diff must emit the POSIX marker
// `\ No newline at end of file` so `patch -p1` accepts the hunk.
#[test]
fn diff_emits_no_newline_marker_when_source_lacks_trailing_newline() {
    let dir = tmpdir("no-nl");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        // intentional: no trailing newline
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]: return s",
    );

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--diff")
        .arg(&pyk)
        .output()
        .expect("run pykrete migrate --diff");
    assert!(out.status.success(), "exit was {:?}", out.status);
    let diff = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(
        diff.contains("\\ No newline at end of file"),
        "diff missing POSIX no-newline marker: {diff}"
    );
}

// Multi-file partial failure: when one file fails to write, the
// summary line tells the user what was/wasn't done and exit is 2.
#[test]
#[cfg(unix)]
fn multi_file_partial_failure_emits_summary_and_exit_2() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmpdir("partial");
    let writable_dir = dir.join("writable");
    fs::create_dir(&writable_dir).unwrap();
    let writable = write_fixture(
        &writable_dir,
        "a.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );
    // Make the second file's directory read-only so atomic_write fails.
    let readonly_dir = dir.join("readonly");
    fs::create_dir(&readonly_dir).unwrap();
    let readonly = write_fixture(
        &readonly_dir,
        "b.pyk",
        "def g(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );
    let mut perms = fs::metadata(&readonly_dir).unwrap().permissions();
    perms.set_mode(0o555);
    fs::set_permissions(&readonly_dir, perms).unwrap();

    let out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&writable)
        .arg(&readonly)
        .output()
        .expect("run pykrete migrate");

    // Restore perms so tempdir cleanup works.
    let mut restore = fs::metadata(&readonly_dir).unwrap().permissions();
    restore.set_mode(0o755);
    let _ = fs::set_permissions(&readonly_dir, restore);

    assert_eq!(out.status.code(), Some(2), "exit was {:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("migrate: rewrote") && stderr.contains("failed"),
        "stderr missing summary line: {stderr}"
    );
}

// Diff header path normalization: round-1 absolute paths produced
// `--- a//abs/path` (double slash). Round 2 strips a single leading
// `/` so the `a/path` convention reads cleanly for both relative
// and absolute inputs.
#[test]
fn diff_headers_strip_double_slash_on_absolute_paths() {
    let dir = tmpdir("abs-path");
    let pyk = write_fixture(
        &dir,
        "x.pyk",
        "def f(s: DataFrame[Sale]) -> DataFrame[Sale]:\n    return s\n",
    );
    // Force absolute path.
    let abs = pyk.canonicalize().expect("canonicalize");
    let out = Command::new(bin())
        .arg("migrate")
        .arg("--diff")
        .arg(&abs)
        .output()
        .expect("run pykrete migrate --diff");
    assert!(out.status.success(), "exit was {:?}", out.status);
    let diff = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(
        !diff.contains("a//"),
        "diff header has double slash from absolute path: {diff}"
    );
    assert!(
        !diff.contains("b//"),
        "diff header has double slash from absolute path: {diff}"
    );
}

// ---------------------------------------------------------------
// Round-2 PR-M3 reviewer (B1): --diff with ambiguous sites must
// produce a `patch -p1`-applicable diff whose result matches what
// --apply writes. Pre-fix, ambiguous sites produced a tautological
// `-line` / `+line` no-op hunk AND omitted the `# pykrete: ambiguous`
// marker, breaking round-trip equivalence with --apply.
// ---------------------------------------------------------------

#[test]
#[cfg(unix)]
fn diff_for_ambiguous_inserts_marker_and_round_trips_to_apply() {
    let dir = tmpdir("ambiguous_diff_a");
    let src = "\
class Sale(Schema):
    region: string


def f(df: DataFrame[Sale]) -> int:
    a = df.withColumn('x', 1)
    b = df.assign(x=1)
    return 0
";
    let original_pyk = write_fixture(&dir, "x.pyk", src);

    // 1. Capture --diff output.
    let diff_out = Command::new(bin())
        .arg("migrate")
        .arg("--diff")
        .arg(&original_pyk)
        .output()
        .expect("run pykrete migrate --diff");
    assert!(diff_out.status.success(), "exit was {:?}", diff_out.status);
    let diff = String::from_utf8(diff_out.stdout).expect("utf8 stdout");

    // The diff must contain the ambiguous-marker insertion.
    assert!(
        diff.contains("# pykrete: ambiguous"),
        "diff missing ambiguous marker insertion: {diff}"
    );
    // It must NOT contain a tautological `-DataFrame[Sale]` / `+DataFrame[Sale]` no-op.
    let lines: Vec<&str> = diff.lines().collect();
    let has_tautological = lines
        .windows(2)
        .any(|w| w[0].starts_with("-DataFrame[Sale]") && w[1].starts_with("+DataFrame[Sale]"));
    assert!(
        !has_tautological,
        "diff has tautological -DataFrame[Sale] / +DataFrame[Sale] no-op: {diff}"
    );

    // 2. Run --apply to capture what the final tree should look like.
    let apply_out = Command::new(bin())
        .arg("migrate")
        .arg("--apply")
        .arg(&original_pyk)
        .output()
        .expect("run pykrete migrate --apply");
    assert!(
        apply_out.status.success(),
        "exit was {:?}",
        apply_out.status
    );
    let applied = fs::read_to_string(&original_pyk).expect("read applied");

    // 3. Apply the same diff via `patch -p1` to a fresh copy of `src`
    //    and verify the patched content matches `applied`.
    let target_dir = tmpdir("ambiguous_diff_b");
    let target_pyk = target_dir.join("x.pyk");
    fs::write(&target_pyk, src).unwrap();

    // The diff was emitted with the original absolute path; rewrite to
    // a relative `a/x.pyk` so `patch -p1` resolves within target_dir.
    let abs_strip = original_pyk
        .strip_prefix("/")
        .unwrap_or(&original_pyk)
        .to_string_lossy()
        .to_string();
    let rel_diff = diff.replace(&abs_strip, "x.pyk");

    let mut child = std::process::Command::new("patch")
        .arg("-p1")
        .current_dir(&target_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn patch -p1");
    {
        use std::io::Write as _;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(rel_diff.as_bytes())
            .expect("write to patch stdin");
    }
    let patch_out = child.wait_with_output().expect("wait patch");
    assert!(
        patch_out.status.success(),
        "patch -p1 rejected diff: stderr={} stdout={}",
        String::from_utf8_lossy(&patch_out.stderr),
        String::from_utf8_lossy(&patch_out.stdout),
    );

    let patched = fs::read_to_string(&target_pyk).expect("read patched");
    assert_eq!(
        applied, patched,
        "diff did NOT round-trip through patch -p1 to match --apply result.\n=== applied ===\n{applied}\n=== patched ===\n{patched}"
    );
}
