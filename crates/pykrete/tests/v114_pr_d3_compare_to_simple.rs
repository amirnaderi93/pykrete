//! v1.14 PR-D3 — `pykrete check --deprecation-report --compare-to <path>`
//! SIMPLE diff-only output (5-cycle calendared user-decision lands).
//!
//! Output shape pinned at v1.14-spec.md §1.i.1 (forward-binding
//! amendment, user signoff 2026-06-22 Option A):
//!
//! ```json
//! {
//!   "snapshot_a": {"sha": ..., "timestamp": ...},
//!   "snapshot_b": {"sha": ..., "timestamp": ...},
//!   "diff": {
//!     "added":     [<full site payload>],
//!     "removed":   [<full site payload>],
//!     "unchanged": [<full site payload>]
//!   }
//! }
//! ```
//!
//! Coverage:
//! - bucket population: identical / disjoint / mixed / status-flip
//! - exit code: nonzero iff `added.length > 0`
//! - mutex with --ack, --snapshot, --fail-on-nonempty
//! - requires --deprecation-report
//! - malformed snapshot: clear error + exit 2
//! - missing required fields: clear error + exit 2
//! - missing file: exit 2 with read error

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn tmpdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("pykrete-v114-pr-d3")
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

/// Run `pykrete check --deprecation-report --snapshot=<dest> <pyk>` and
/// return the dest path. Lets the diff tests reuse the canonical
/// envelope-write path instead of hand-rolling JSON.
fn snapshot(dir: &Path, dest_name: &str, pyk: &Path) -> PathBuf {
    let dest = dir.join(dest_name);
    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", dest.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "snapshot --snapshot exit nonzero: {stderr}");
    assert!(
        dest.exists(),
        "snapshot did not write to {}",
        dest.display()
    );
    dest
}

const ONE_SITE_PENDING: &str = "def f(df: DataFrame[Sale]) -> int:\n    return 0\n";

const TWO_SITES_PENDING: &str = "\
def f(df: DataFrame[Sale]) -> int:
    return 0


def g(df: DataFrame[Sale]) -> int:
    return 0
";

const ONE_SITE_ACK: &str = "\
# pykrete: ack-deprecation
def f(df: DataFrame[Sale]) -> int:
    return 0
";

// -------------------------------------------------------------------
// Positive — bucket population
// -------------------------------------------------------------------

#[test]
fn identical_snapshots_all_unchanged_exit_zero() {
    let dir = tmpdir("identical");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = snapshot(&dir, "prior.json", &pyk);

    let (exit, stdout, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 0);
    assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 0);
    assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 1);
}

#[test]
fn empty_prior_full_current_all_added_exit_one() {
    let dir = tmpdir("empty-prior");
    let clean = write_pyk(&dir, "clean.pyk", "def f() -> int: return 0\n");
    let snap = snapshot(&dir, "prior.json", &clean);

    // Replace the file with one that has a deprecated alias.
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let (exit, stdout, _) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 1, "added > 0 must trip exit 1");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 1);
    assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 0);
    assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 0);
}

#[test]
fn full_prior_empty_current_all_removed_exit_zero() {
    let dir = tmpdir("empty-current");
    let pyk = write_pyk(&dir, "x.pyk", TWO_SITES_PENDING);
    let snap = snapshot(&dir, "prior.json", &pyk);

    // Replace with a clean file → all prior sites are now "removed."
    let clean = write_pyk(&dir, "x.pyk", "def f() -> int: return 0\n");
    let (exit, stdout, _) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        clean.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "removed-only diff stays exit 0");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 0);
    assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 2);
    assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 0);
}

#[test]
fn ack_marker_insertion_surfaces_as_add_plus_remove_via_line_shift() {
    // Rename of round-2 `status_flip_pending_to_acknowledged_remove_plus_add`:
    // adding the ack marker shifts the def's line by 1, so the new site
    // has a DIFFERENT (file, line) key than the prior — the diff walker
    // hits the disjoint-keys path (line shift), not the same-key
    // payload-mismatch path. See `payload_drift_at_same_site_id_…` below
    // for the genuine same-key/different-payload pin.
    let dir = tmpdir("ack-marker-line-shift");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = snapshot(&dir, "prior.json", &pyk);

    let pyk_acked = write_pyk(&dir, "x.pyk", ONE_SITE_ACK);
    let (exit, stdout, _) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk_acked.to_str().unwrap(),
    ]);
    assert_eq!(exit, 1, "added > 0 must trip exit 1");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let added = v["diff"]["added"].as_array().unwrap();
    let removed = v["diff"]["removed"].as_array().unwrap();
    assert_eq!(added.len(), 1, "ack add: {added:?}");
    assert_eq!(added[0]["migrationStatus"], "acknowledged");
    assert_eq!(removed.len(), 1, "pre-ack remove: {removed:?}");
    assert_eq!(removed[0]["migrationStatus"], "pending");
}

#[test]
fn payload_drift_at_same_site_id_surfaces_as_remove_plus_add() {
    // Force a (file, line) collision: prior snapshot has a doctored
    // `migrationStatus` field for line 1; current envelope sees the
    // real value. This exercises the payload-mismatch branch of the
    // diff walker where SiteId matches but contents differ.
    let dir = tmpdir("payload-drift");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);

    // Capture the canonical envelope then mutate one field to force
    // a payload mismatch at the same (file, line) key.
    let canonical = dir.join("canonical.json");
    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", canonical.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");
    let mut envelope: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&canonical).unwrap()).unwrap();
    envelope["sites"][0]["migrationStatus"] = serde_json::Value::String("acknowledged".to_owned());
    let prior = dir.join("prior.json");
    fs::write(&prior, serde_json::to_string_pretty(&envelope).unwrap()).unwrap();

    // Current run vs. doctored prior: same (file, line), differing
    // `migrationStatus` → 1 added (current) + 1 removed (prior).
    let (exit, stdout, _) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", prior.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 1, "added > 0 must trip exit 1");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let added = v["diff"]["added"].as_array().unwrap();
    let removed = v["diff"]["removed"].as_array().unwrap();
    assert_eq!(added.len(), 1, "added: {added:?}");
    assert_eq!(added[0]["migrationStatus"], "pending");
    assert_eq!(removed.len(), 1, "removed: {removed:?}");
    assert_eq!(removed[0]["migrationStatus"], "acknowledged");
    assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 0);
}

#[test]
fn mixed_diff_one_added_one_removed_one_unchanged() {
    let dir = tmpdir("mixed");
    // Prior snapshot has two pyk files; "moved" by removing one and
    // adding a third.
    let pyk_a = write_pyk(&dir, "a.pyk", ONE_SITE_PENDING);
    let pyk_b = write_pyk(&dir, "b.pyk", ONE_SITE_PENDING);
    let snap = snapshot(&dir, "prior.json", &pyk_a);

    // Build a combined prior by running snapshot once on both files.
    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", snap.display()),
        pyk_a.to_str().unwrap(),
        pyk_b.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");

    // Current run: a.pyk (unchanged), b.pyk removed (no file), c.pyk added.
    let _ = fs::remove_file(&pyk_b);
    let pyk_c = write_pyk(&dir, "c.pyk", ONE_SITE_PENDING);
    let (exit, stdout, _) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk_a.to_str().unwrap(),
        pyk_c.to_str().unwrap(),
    ]);
    assert_eq!(exit, 1, "added > 0 must trip exit 1");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["diff"]["added"][0]["file"].as_str().unwrap(),
        pyk_c.to_string_lossy()
    );
    assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["diff"]["removed"][0]["file"].as_str().unwrap(),
        pyk_b.to_string_lossy()
    );
    assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 1);
    assert_eq!(
        v["diff"]["unchanged"][0]["file"].as_str().unwrap(),
        pyk_a.to_string_lossy()
    );
}

// -------------------------------------------------------------------
// Envelope shape — top-level provenance + diff buckets
// -------------------------------------------------------------------

#[test]
fn diff_envelope_top_level_keys_match_spec() {
    let dir = tmpdir("envelope-keys");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = snapshot(&dir, "prior.json", &pyk);

    let (exit, stdout, _) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(v.get("snapshot_a").is_some(), "missing snapshot_a");
    assert!(v.get("snapshot_b").is_some(), "missing snapshot_b");
    assert!(v.get("diff").is_some(), "missing diff");
    assert!(v["snapshot_a"].get("sha").is_some());
    assert!(v["snapshot_a"].get("timestamp").is_some());
    assert!(v["snapshot_b"].get("sha").is_some());
    assert!(v["snapshot_b"].get("timestamp").is_some());
    assert!(v["diff"].get("added").is_some());
    assert!(v["diff"].get("removed").is_some());
    assert!(v["diff"].get("unchanged").is_some());
}

#[test]
fn snapshot_b_timestamp_is_iso8601_z() {
    let dir = tmpdir("iso8601");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = snapshot(&dir, "prior.json", &pyk);

    let (_, stdout, _) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let ts = v["snapshot_b"]["timestamp"]
        .as_str()
        .expect("timestamp present");
    // YYYY-MM-DDTHH:MM:SSZ — 20 chars exact.
    assert_eq!(ts.len(), 20, "ts shape: {ts}");
    assert!(ts.ends_with('Z'), "ts should end in Z: {ts}");
    assert!(ts.chars().nth(4) == Some('-'));
    assert!(ts.chars().nth(7) == Some('-'));
    assert!(ts.chars().nth(10) == Some('T'));
}

// -------------------------------------------------------------------
// Provenance round-trip — v1.14 PR-D3 round-2 BLOCKER #1 fix
// -------------------------------------------------------------------

#[test]
fn snapshot_envelope_emits_provenance_top_level_keys() {
    // v1.14+ snapshots ALWAYS emit `pykreteSourceCommit` + `generatedAt`
    // at the top level. The git SHA may be null (snapshot taken outside
    // a git repo, etc.); the timestamp is best-effort but populated
    // under normal conditions.
    let dir = tmpdir("snapshot-provenance");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = snapshot(&dir, "prior.json", &pyk);
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snap).unwrap()).expect("valid JSON");
    let obj = v.as_object().expect("envelope is an object");
    assert!(
        obj.contains_key("pykreteSourceCommit"),
        "snapshot envelope MUST emit pykreteSourceCommit top-level key"
    );
    assert!(
        obj.contains_key("generatedAt"),
        "snapshot envelope MUST emit generatedAt top-level key"
    );
    // Timestamp is captured from system clock; should always populate
    // to ISO-8601 UTC second-precision when the system clock works.
    let ts = v["generatedAt"]
        .as_str()
        .expect("generatedAt should be ISO-8601 string");
    assert_eq!(ts.len(), 20, "ts shape: {ts}");
    assert!(ts.ends_with('Z'), "ts should end in Z: {ts}");
}

#[test]
fn compare_to_round_trips_snapshot_a_provenance_from_v114_snapshot() {
    // A snapshot taken by v1.14+ carries provenance; feeding it to
    // `--compare-to` MUST surface `snapshot_a.timestamp` (and
    // `snapshot_a.sha` if a git SHA was captured) — not null.
    let dir = tmpdir("provenance-roundtrip");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = snapshot(&dir, "prior.json", &pyk);

    // Sanity-check that the snapshot itself has the keys populated
    // (timestamp always; sha when run inside a git repo, which the
    // pykrete worktree always is during `cargo test`).
    let prior_v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&snap).unwrap()).unwrap();
    let prior_ts = prior_v["generatedAt"]
        .as_str()
        .expect("v1.14 snapshot generatedAt populated");

    let (exit, stdout, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 0, "stderr: {stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(
        v["snapshot_a"]["timestamp"].as_str(),
        Some(prior_ts),
        "snapshot_a.timestamp must round-trip from v1.14 snapshot's generatedAt"
    );
    // sha may be Some(real SHA) or None — what matters is the
    // round-trip: if the snapshot carried a sha, the diff doc carries
    // the same one verbatim.
    assert_eq!(
        v["snapshot_a"]["sha"], prior_v["pykreteSourceCommit"],
        "snapshot_a.sha must round-trip from v1.14 snapshot's pykreteSourceCommit"
    );
}

// -------------------------------------------------------------------
// Mutex validation
// -------------------------------------------------------------------

#[test]
fn compare_to_without_deprecation_report_errors() {
    let dir = tmpdir("mutex-noreport");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = dir.join("prior.json");
    fs::write(&snap, r#"{"deprecationReportVersion":"2","sites":[],"summary":{"totalSites":0,"byDialect":{"spark":0,"pandas":0,"ambiguous":0}}}"#).unwrap();

    let (exit, _, stderr) = run(&[
        "check",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("--compare-to requires --deprecation-report"),
        "stderr: {stderr}"
    );
}

#[test]
fn compare_to_with_ack_errors() {
    let dir = tmpdir("mutex-ack");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = dir.join("prior.json");
    fs::write(&snap, r#"{"deprecationReportVersion":"2","sites":[],"summary":{"totalSites":0,"byDialect":{"spark":0,"pandas":0,"ambiguous":0}}}"#).unwrap();

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--ack=pending",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("--compare-to and --ack are mutually exclusive"),
        "stderr: {stderr}"
    );
}

#[test]
fn compare_to_with_snapshot_errors() {
    let dir = tmpdir("mutex-snapshot");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = dir.join("prior.json");
    fs::write(&snap, r#"{"deprecationReportVersion":"2","sites":[],"summary":{"totalSites":0,"byDialect":{"spark":0,"pandas":0,"ambiguous":0}}}"#).unwrap();
    let out = dir.join("out.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--snapshot={}", out.display()),
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("--compare-to and --snapshot are mutually exclusive"),
        "stderr: {stderr}"
    );
}

#[test]
fn compare_to_with_fail_on_nonempty_errors() {
    let dir = tmpdir("mutex-fail");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = dir.join("prior.json");
    fs::write(&snap, r#"{"deprecationReportVersion":"2","sites":[],"summary":{"totalSites":0,"byDialect":{"spark":0,"pandas":0,"ambiguous":0}}}"#).unwrap();

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--fail-on-nonempty",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("--compare-to and --fail-on-nonempty are mutually exclusive"),
        "stderr: {stderr}"
    );
}

// -------------------------------------------------------------------
// Error handling — malformed snapshot, missing file, missing fields
// -------------------------------------------------------------------

#[test]
fn compare_to_missing_file_errors() {
    let dir = tmpdir("missing");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let missing = dir.join("does-not-exist.json");

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", missing.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("--compare-to: cannot read"),
        "stderr: {stderr}"
    );
}

#[test]
fn compare_to_malformed_json_errors() {
    let dir = tmpdir("malformed");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = dir.join("prior.json");
    fs::write(&snap, "{not valid json").unwrap();

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(stderr.contains("not valid JSON"), "stderr: {stderr}");
}

#[test]
fn compare_to_missing_sites_field_errors() {
    let dir = tmpdir("missing-sites");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = dir.join("prior.json");
    fs::write(&snap, r#"{"deprecationReportVersion":"2"}"#).unwrap();

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("missing required field 'sites'"),
        "stderr: {stderr}"
    );
}

#[test]
fn compare_to_site_missing_file_errors() {
    let dir = tmpdir("missing-file-field");
    let pyk = write_pyk(&dir, "x.pyk", ONE_SITE_PENDING);
    let snap = dir.join("prior.json");
    fs::write(
        &snap,
        r#"{"deprecationReportVersion":"2","sites":[{"line":1}],"summary":{"totalSites":0,"byDialect":{"spark":0,"pandas":0,"ambiguous":0}}}"#,
    )
    .unwrap();

    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        &format!("--compare-to={}", snap.display()),
        pyk.to_str().unwrap(),
    ]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("missing required string 'file'"),
        "stderr: {stderr}"
    );
}

#[test]
fn compare_to_flag_with_no_value_errors() {
    let (exit, _, stderr) = run(&["check", "--deprecation-report", "--compare-to"]);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("--compare-to requires a path"),
        "stderr: {stderr}"
    );
}

#[test]
fn compare_to_flag_with_flag_shaped_value_errors() {
    let (exit, _, stderr) = run(&[
        "check",
        "--deprecation-report",
        "--compare-to",
        "--ack=pending",
        "fake.pyk",
    ]);
    assert_eq!(exit, 2);
    assert!(stderr.contains("looks like a flag"), "stderr: {stderr}");
}

// -------------------------------------------------------------------
// Help text + flag visibility
// -------------------------------------------------------------------

#[test]
fn check_help_mentions_compare_to() {
    let (exit, stdout, _) = run(&["check", "--help"]);
    assert_eq!(exit, 0);
    assert!(
        stdout.contains("--compare-to"),
        "check --help missing --compare-to: {stdout}"
    );
}
