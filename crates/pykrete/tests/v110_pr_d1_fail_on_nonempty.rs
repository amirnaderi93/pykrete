//! v1.10 PR-D1 — `pykrete check --deprecation-report --fail-on-nonempty`.
//!
//! Closes v1.9 architecture finding arch-I4. v1.9 PR-V1 shipped the
//! `--deprecation-report` v2 envelope with `--ack` filtering but
//! always exited 0. CI gating consumers had to `jq '.summary.totalSites'
//! | xargs test 0 -eq` to convert the envelope into a pass/fail signal.
//! `--fail-on-nonempty` collapses that boilerplate into a single flag.
//!
//! Semantics:
//! - Default (flag absent): exit 0 regardless of site count
//!   (v1.9 behavior preserved).
//! - Flag set + post-`--ack` filtered site count > 0: exit 1.
//! - Flag set + post-`--ack` filtered site count == 0: exit 0.
//! - Flag set without `--deprecation-report`: exit 2 with stderr error
//!   (mirrors the v1.9 `--ack` requires `--deprecation-report` guard).
//! - Stdout JSON envelope ALWAYS emits, regardless of exit code, so CI
//!   pipelines can pipe report + gate in one invocation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn tmpdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("pykrete-v110-pr-d1")
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

fn run(args: &[&str], target: &Path) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .arg(target)
        .output()
        .expect("run pykrete");
    let exit = out.status.code().unwrap_or(-1);
    (
        exit,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// One pending site in source. With `--fail-on-nonempty` → exit 1.
#[test]
fn pending_site_with_fail_on_nonempty_exits_1() {
    let dir = tmpdir("pending-fail");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, stdout, _) = run(&["check", "--deprecation-report", "--fail-on-nonempty"], &p);
    assert_eq!(exit, 1, "non-empty filtered envelope MUST exit 1");
    // Stdout envelope still emits — CI can pipe report + gate in one pass.
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("envelope still emits on exit 1");
    assert_eq!(v["summary"]["totalSites"], 1);
}

// One pending site, `--ack=acknowledged` filters it out → exit 0.
// The filter takes precedence; nonzero unfiltered site count does NOT
// drive the gate when the filter narrows to zero.
#[test]
fn pending_site_filtered_to_acknowledged_exits_0() {
    let dir = tmpdir("ack-filter");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, stdout, _) = run(
        &[
            "check",
            "--deprecation-report",
            "--ack=acknowledged",
            "--fail-on-nonempty",
        ],
        &p,
    );
    assert_eq!(exit, 0, "filter to acknowledged drops the pending site");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["summary"]["totalSites"], 0);
}

// Zero sites in source. With `--fail-on-nonempty` → exit 0.
#[test]
fn zero_sites_with_fail_on_nonempty_exits_0() {
    let dir = tmpdir("zero-sites");
    let p = write_pyk(
        &dir,
        "x.pyk",
        // No DataFrame[X] annotation = no D0090 site.
        "def f(x: int) -> int:\n    return x\n",
    );
    let (exit, stdout, _) = run(&["check", "--deprecation-report", "--fail-on-nonempty"], &p);
    assert_eq!(exit, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["summary"]["totalSites"], 0);
}

// One pending site, NO `--fail-on-nonempty` → exit 0 (v1.9 default behavior
// preserved). Regression guard against accidentally gating the default
// path.
#[test]
fn pending_site_without_fail_on_nonempty_exits_0() {
    let dir = tmpdir("no-gate");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, stdout, _) = run(&["check", "--deprecation-report"], &p);
    assert_eq!(exit, 0, "v1.9 default behavior MUST be preserved");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["summary"]["totalSites"], 1);
}

// One acknowledged site, `--ack=pending` filters to zero → exit 0.
// Same shape as `pending_site_filtered_to_acknowledged_exits_0` from the
// opposite side: the line-above ack marker is present, default
// `--ack=pending` filter rejects it.
#[test]
fn acknowledged_site_filtered_to_pending_exits_0() {
    let dir = tmpdir("ack-marker-pending");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
# pykrete: ack-deprecation
def f(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (exit, stdout, _) = run(
        &[
            "check",
            "--deprecation-report",
            "--ack=pending",
            "--fail-on-nonempty",
        ],
        &p,
    );
    assert_eq!(exit, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["summary"]["totalSites"], 0);
}

// `--fail-on-nonempty` without `--deprecation-report` → exit 2 with
// clear stderr error. Mirrors the v1.9 `--ack` requires
// `--deprecation-report` enforcement.
#[test]
fn fail_on_nonempty_without_deprecation_report_exits_2() {
    let dir = tmpdir("missing-report");
    let p = write_pyk(&dir, "x.pyk", "def f(x: int) -> int:\n    return x\n");
    let (exit, _, stderr) = run(&["check", "--fail-on-nonempty"], &p);
    assert_eq!(
        exit, 2,
        "--fail-on-nonempty without --deprecation-report MUST exit 2"
    );
    assert!(
        stderr.contains("--fail-on-nonempty") && stderr.contains("--deprecation-report"),
        "stderr MUST name both flags; got: {stderr}"
    );
}

// `--fail-on-nonempty` combined with `--report-aliases` → exit 2 because
// `--report-aliases` is mutex with `--deprecation-report`, so the
// `--fail-on-nonempty` requires-`--deprecation-report` guard fires.
// Regression guard: D1's flag doesn't accidentally leak gating into the
// `--report-aliases` path.
#[test]
fn fail_on_nonempty_with_report_aliases_exits_2() {
    let dir = tmpdir("mutex-report-aliases");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, _, _) = run(&["check", "--report-aliases", "--fail-on-nonempty"], &p);
    assert_eq!(exit, 2);
}

// Two pending sites + no `--ack` filter + `--fail-on-nonempty` → exit 1.
// Covers the "no filter, sites exist" branch (spec §3.1.5 case 5).
#[test]
fn multiple_pending_sites_unfiltered_fails() {
    let dir = tmpdir("two-sites");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
def a(df: DataFrame[Sale]) -> int:
    return 0

def b(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (exit, stdout, _) = run(&["check", "--deprecation-report", "--fail-on-nonempty"], &p);
    assert_eq!(exit, 1);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["summary"]["totalSites"], 2);
}

// Stdout envelope on exit 1 is byte-for-byte identical to the same
// invocation without `--fail-on-nonempty`. The flag is exit-code-only;
// it does NOT mutate the envelope shape.
#[test]
fn envelope_unchanged_by_fail_on_nonempty() {
    let dir = tmpdir("envelope-stable");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (_, with_flag, _) = run(&["check", "--deprecation-report", "--fail-on-nonempty"], &p);
    let (_, without_flag, _) = run(&["check", "--deprecation-report"], &p);
    assert_eq!(
        with_flag, without_flag,
        "--fail-on-nonempty MUST be exit-code-only; envelope bytes must match"
    );
}

// ---------------------------------------------------------------------
// Cross-flag composition: --snapshot (PR-V1) × --fail-on-nonempty (PR-D1).
// The two flags are orthogonal: --snapshot picks the envelope destination
// (file vs stdout), --fail-on-nonempty shapes the exit code. Both must
// be able to fire on the same invocation.
// ---------------------------------------------------------------------

// `--snapshot` + `--fail-on-nonempty` + 1 pending site:
// → envelope written to file AND exit 1.
#[test]
fn snapshot_with_fail_on_nonempty_writes_file_and_exits_1() {
    let dir = tmpdir("snap-fail-one");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let snap = dir.join("out.json");

    let out = Command::new(bin())
        .args([
            "check",
            "--deprecation-report",
            &format!("--snapshot={}", snap.display()),
            "--fail-on-nonempty",
        ])
        .arg(&p)
        .output()
        .expect("run pykrete");
    let exit = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    assert_eq!(
        exit, 1,
        "non-empty filtered envelope + --fail-on-nonempty MUST exit 1, \
         regardless of --snapshot redirect"
    );
    assert_eq!(
        stdout, "",
        "stdout MUST be empty when --snapshot redirects the envelope, \
         even on exit 1"
    );
    assert!(
        snap.exists(),
        "snapshot file MUST be written before the gate fires"
    );

    let on_disk = fs::read_to_string(&snap).expect("read snapshot");
    let v: serde_json::Value =
        serde_json::from_str(&on_disk).expect("snapshot is valid JSON on exit 1");
    assert_eq!(v["summary"]["totalSites"], 1);

    let _ = fs::remove_dir_all(&dir);
}

// `--snapshot` + `--fail-on-nonempty` + `--ack=acknowledged` filtering
// the one pending site to zero:
// → envelope written to file AND exit 0. Filter takes precedence over
// raw site count, identical to the stdout path's behavior.
#[test]
fn snapshot_with_fail_on_nonempty_zero_sites_exits_0() {
    let dir = tmpdir("snap-fail-zero");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let snap = dir.join("out.json");

    let out = Command::new(bin())
        .args([
            "check",
            "--deprecation-report",
            "--ack=acknowledged",
            &format!("--snapshot={}", snap.display()),
            "--fail-on-nonempty",
        ])
        .arg(&p)
        .output()
        .expect("run pykrete");
    let exit = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    assert_eq!(
        exit, 0,
        "--ack=acknowledged filters the pending site to zero; gate MUST NOT fire"
    );
    assert_eq!(stdout, "");
    assert!(
        snap.exists(),
        "snapshot file MUST still be written on exit 0"
    );

    let on_disk = fs::read_to_string(&snap).expect("read snapshot");
    let v: serde_json::Value = serde_json::from_str(&on_disk).expect("snapshot is valid JSON");
    assert_eq!(v["summary"]["totalSites"], 0);

    let _ = fs::remove_dir_all(&dir);
}
