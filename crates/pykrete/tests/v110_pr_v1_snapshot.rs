//! v1.10 PR-V1 — `pykrete check --deprecation-report --snapshot=<path>`
//! file-write.
//!
//! `--snapshot=<path>` writes the same v2 envelope that v1.9 PR-V1 emits
//! on stdout to a file instead, via a sibling tempfile + `fs::rename`
//! for atomicity. Compatible with `--ack`; mutually exclusive with
//! `--report-aliases`; requires `--deprecation-report`.
//!
//! CRITICAL CARVE-OUT (spec §5.5): NO `--compare-to` flag in this PR.
//! Consumer state model is product fork (deferred to v1.11). PR-V1
//! just writes the file; consumers diff externally. Negative-space
//! test guards the schema invariants.
//!
//! Negative-space coverage per v14-rule 4:
//! - `--snapshot` without `--deprecation-report` → exit 2
//! - `--snapshot` + `--report-aliases` → exit 2 (mutex)
//! - parent dir does not exist → exit 2, clean error
//! - permission denied on parent → exit 2, clean error
//! - empty path value → exit 2
//! - stdout stays empty on snapshot success
//! - existing file is overwritten silently
//! - atomic write: no `.tmp` sibling lingers after success
//! - envelope schema unchanged (`deprecationReportVersion` `"2"`)
//! - envelope has no `--compare-to`-implying fields

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn tmpdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("pykrete-v110-pr-v1")
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

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("run pykrete");
    let exit = out.status.code().unwrap_or(-1);
    (
        exit,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const ONE_SITE: &str = "def f(df: DataFrame[Sale]) -> int:\n    return 0\n";

const TWO_MIXED: &str = "\
# pykrete: ack-deprecation
def ack(df: DataFrame[Sale]) -> int:
    return 0


def pend(df: DataFrame[Sale]) -> int:
    return 0
";

// -----------------------------------------------------------------
// Positive — snapshot writes the same envelope as stdout
// -----------------------------------------------------------------

#[test]
fn snapshot_writes_valid_json_matching_stdout() {
    let dir = tmpdir("matches-stdout");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("out.json");

    let (exit, stdout, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");
    assert_eq!(
        stdout, "",
        "stdout must be empty when --snapshot redirects the envelope"
    );
    assert!(snap.exists(), "snapshot file must exist");

    let on_disk = fs::read_to_string(&snap).expect("read snapshot");
    let _: serde_json::Value =
        serde_json::from_str(&on_disk).expect("snapshot contents are valid JSON");

    // Byte-for-byte equal to the stdout form (with a trailing newline
    // from println!). Per spec §5.2 the envelope is unchanged; only
    // the sink differs.
    let (exit_stdout, stdout_form, _) =
        run(&["check", "--deprecation-report", p.to_str().unwrap()]);
    assert_eq!(exit_stdout, 0);
    assert_eq!(
        on_disk,
        stdout_form.trim_end_matches('\n'),
        "snapshot file contents equal stdout form (sans trailing newline)"
    );
}

#[test]
fn snapshot_envelope_version_unchanged() {
    let dir = tmpdir("v2-unchanged");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&snap).unwrap()).unwrap();
    assert_eq!(
        v["deprecationReportVersion"], "2",
        "snapshot does not bump the envelope version"
    );
}

#[test]
fn snapshot_envelope_has_no_compare_to_fields() {
    // Per spec §5.5 carve-out: NO --compare-to in v1.10. The envelope
    // MUST NOT pre-allocate fields a future consumer-state-model
    // surface would need (`previous_snapshot_diff`, `resolvedSites`,
    // etc.). v1.11 spec-time owns the consumer surface.
    //
    // Exact-keys allowlist (round-2 reviewer): a denylist only catches
    // the names we anticipate. Asserting the full top-level key set
    // catches ANY future addition regardless of name — defense in depth.
    let dir = tmpdir("no-compare");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("out.json");

    run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&snap).unwrap()).unwrap();
    let obj = v.as_object().expect("envelope is an object");
    let keys: BTreeSet<&str> = obj.keys().map(|s| s.as_str()).collect();
    // v1.14 PR-D3 added `pykreteSourceCommit` + `generatedAt` (provenance
    // round-trip into `--compare-to`). The v1.10 §5.5 carve-out forbade
    // CONSUMER-STATE pre-allocation (`previous_snapshot_diff`,
    // `resolvedSites`); provenance keys are source-of-truth metadata and
    // explicitly allowed.
    let expected: BTreeSet<&str> = [
        "deprecationReportVersion",
        "pykreteSourceCommit",
        "generatedAt",
        "sites",
        "summary",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        keys, expected,
        "envelope keys must match v1.14 schema (v1.9 base + v1.14 provenance pair); got {obj:?}"
    );
}

#[test]
fn snapshot_with_ack_pending_filters() {
    let dir = tmpdir("ack-pending");
    let p = write_pyk(&dir, "mix.pyk", TWO_MIXED);
    let snap = dir.join("out.json");

    let (exit, stdout, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--ack=pending",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");
    assert_eq!(stdout, "", "stdout empty on snapshot success");

    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&snap).unwrap()).unwrap();
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1, "only the pending site survives the filter");
    assert_eq!(sites[0]["migrationStatus"], "pending");
    assert_eq!(v["summary"]["totalSites"], 1);
}

#[test]
fn snapshot_to_nested_existing_dir() {
    let dir = tmpdir("nested");
    let nested = dir.join("sub");
    fs::create_dir_all(&nested).expect("mkdir sub");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = nested.join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");
    assert!(snap.exists());
}

#[test]
fn snapshot_overwrites_existing_file() {
    let dir = tmpdir("overwrite");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("out.json");
    fs::write(&snap, "GARBAGE-NOT-JSON").expect("seed snapshot file");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");

    let on_disk = fs::read_to_string(&snap).expect("read snapshot");
    assert!(
        !on_disk.contains("GARBAGE"),
        "pre-existing garbage must be replaced; got {on_disk:?}"
    );
    let v: serde_json::Value = serde_json::from_str(&on_disk).expect("snapshot is valid JSON");
    assert_eq!(v["deprecationReportVersion"], "2");
}

#[test]
fn snapshot_leaves_no_tmp_sibling_after_success() {
    // Atomic-write verification: after success, the sibling
    // `.<name>.pykrete-snapshot.<pid>.tmp` file is gone (renamed
    // over the target).
    let dir = tmpdir("no-tmp-lingers");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");

    for entry in fs::read_dir(&dir).expect("read dir") {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.contains("pykrete-snapshot"),
            "no tempfile should remain; found {name}"
        );
    }
}

#[test]
fn snapshot_with_space_separated_value() {
    // `--snapshot <path>` (separate arg) parses the same as
    // `--snapshot=<path>`. Mirrors v1.9's `--ack` shape.
    let dir = tmpdir("space-arg");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--snapshot",
        snap.to_str().unwrap(),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");
    assert!(snap.exists());
}

// -----------------------------------------------------------------
// Negative — usage errors exit 2 with clear stderr
// -----------------------------------------------------------------

#[test]
fn snapshot_without_deprecation_report_errors() {
    let dir = tmpdir("snap-alone");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2, "--snapshot alone exits 2");
    assert!(
        stderr.contains("--snapshot requires --deprecation-report"),
        "stderr should explain the requirement: {stderr:?}"
    );
    assert!(!snap.exists(), "no file written on usage error");
}

#[test]
fn snapshot_with_report_aliases_is_mutex() {
    let dir = tmpdir("mutex");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--report-aliases",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2, "--snapshot + --report-aliases exits 2");
    assert!(
        stderr.contains("--snapshot is for the --deprecation-report envelope"),
        "stderr should explain mutex: {stderr:?}"
    );
    assert!(!snap.exists(), "no file written on usage error");
}

#[test]
fn snapshot_to_nonexistent_parent_dir_errors() {
    let dir = tmpdir("missing-parent");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let snap = dir.join("does-not-exist").join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2, "missing parent dir exits 2");
    assert!(
        stderr.contains("--snapshot: cannot write to"),
        "stderr should name the path: {stderr:?}"
    );
    assert!(
        stderr.contains("directory does not exist"),
        "stderr should explain the cause: {stderr:?}"
    );
    assert!(!snap.exists(), "no file written");
}

#[cfg(unix)]
#[test]
fn snapshot_to_readonly_parent_errors() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tmpdir("readonly");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);
    let ro = dir.join("ro");
    fs::create_dir_all(&ro).expect("mkdir ro");
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).expect("chmod ro");
    let snap = ro.join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        p.to_str().unwrap(),
    ]);

    // Restore writable perms so cleanup works regardless of outcome.
    let _ = fs::set_permissions(&ro, fs::Permissions::from_mode(0o755));

    assert_eq!(exit, 2, "permission denied exits 2");
    assert!(
        stderr.contains("--snapshot: cannot write to"),
        "stderr should name the path: {stderr:?}"
    );
    assert!(!snap.exists(), "no file written");
}

#[test]
fn snapshot_empty_path_value_errors() {
    let dir = tmpdir("empty-path");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--snapshot=",
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("--snapshot requires a path"),
        "stderr should reject empty value: {stderr:?}"
    );
}

#[test]
fn snapshot_missing_value_after_space_errors() {
    // `--snapshot` with no successor at all: the args parser must
    // surface "--snapshot requires a path" before falling through to
    // the "no input file" error. Without an input file the path-list
    // check also fires, but the args parse happens first so the
    // --snapshot message wins.
    let (exit, _, stderr) = run(&["check", "--deprecation-report", "--snapshot"]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("--snapshot requires a path"),
        "stderr should reject missing successor: {stderr:?}"
    );
}

// -----------------------------------------------------------------
// Round-2 — `--snapshot <flag>` space form must NOT steal the flag
// -----------------------------------------------------------------
//
// Without this guard, `pykrete check --deprecation-report --snapshot
// --ack=pending x.pyk` silently consumed `--ack=pending` as the path,
// dropping the ack filter and writing the report to a file literally
// named `--ack=pending`. Reject `--`-prefixed values up front.

#[test]
fn snapshot_space_form_rejects_long_flag_successor() {
    let dir = tmpdir("flag-successor-ack");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--snapshot",
        "--ack=pending",
        p.to_str().unwrap(),
    ]);
    assert_eq!(
        exit, 2,
        "flag-shaped successor must error; stderr: {stderr}"
    );
    assert!(
        stderr.contains("--snapshot: value '--ack=pending' looks like a flag"),
        "stderr should name the offending value: {stderr:?}"
    );
    // The fake "path" must NOT have been created.
    assert!(
        !PathBuf::from("--ack=pending").exists(),
        "no file should be written when the flag is misparsed as a path"
    );
}

#[test]
fn snapshot_space_form_rejects_report_aliases_successor() {
    let dir = tmpdir("flag-successor-aliases");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--snapshot",
        "--report-aliases",
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("looks like a flag"),
        "stderr should explain the rejection: {stderr:?}"
    );
}

#[test]
fn snapshot_space_form_rejects_snapshot_successor() {
    // The pathological `--snapshot --snapshot=foo path.pyk`: the first
    // `--snapshot` must NOT swallow the second as its value.
    let dir = tmpdir("flag-successor-snapshot");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--snapshot",
        "--snapshot=foo",
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("looks like a flag"),
        "stderr should explain the rejection: {stderr:?}"
    );
}

#[test]
fn snapshot_space_form_rejects_dash_sentinel() {
    // `-` alone is reserved as a stdin-style sentinel; pykrete doesn't
    // support stdout-snapshot via `-`, so we reject it explicitly
    // rather than silently writing to a file literally named `-`.
    let dir = tmpdir("dash-sentinel");
    let p = write_pyk(&dir, "x.pyk", ONE_SITE);

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--snapshot",
        "-",
        p.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("'-' is not a valid path"),
        "stderr should reject single-dash: {stderr:?}"
    );
}
