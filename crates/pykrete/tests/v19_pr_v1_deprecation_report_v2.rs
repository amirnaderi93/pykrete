//! v1.9 PR-V1 — `pykrete check --deprecation-report` v2 envelope.
//!
//! Three surfaces, one PR:
//! 1. Envelope bump: `deprecationReportVersion` `"1"` → `"2"`.
//! 2. New per-site `migrationStatus` field (`"pending"` /
//!    `"acknowledged"`). The pykrete binary never emits `"resolved"`;
//!    that's a stateful-delta sentinel for external consumers.
//! 3. New `--ack=<value>` filter flag (`pending` / `acknowledged`).
//!    `resolved` and unknown values exit 2.
//!
//! Per spec §4.5: NO `target_version` field, NO calendar quarter,
//! NO date marker. Negative-space test §4.7 guards that explicitly.
//!
//! Negative-space coverage per v14-rule 4:
//! - marker present → acknowledged
//! - marker absent → pending
//! - marker on wrong line (not immediately above) → still pending
//! - marker mis-typed → still pending
//! - filter rejects `resolved`, garbage values
//! - filter without `--deprecation-report` errors
//! - marker is envelope-only, not a D0090 suppression
//! - schema lacks `target_version` field

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn tmpdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("pykrete-v19-pr-v1")
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

fn run_report_json(args: &[&str], target: &Path) -> (i32, serde_json::Value, String) {
    let (exit, stdout, stderr) = run(args, target);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\nstdout:\n{stdout}\nstderr:\n{stderr}"));
    (exit, v, stderr)
}

// ---------------------------------------------------------------
// Envelope schema bump v1 → v2
// ---------------------------------------------------------------

#[test]
fn envelope_version_is_v2() {
    let dir = tmpdir("v2-bump");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    assert_eq!(exit, 0);
    assert_eq!(
        v["deprecationReportVersion"], "2",
        "v1.9 bump 1 → 2; v1 consumers must fail-fast"
    );
}

#[test]
fn envelope_has_no_target_version_field() {
    // Per spec §4.5 carve-out: no ship-date commitment in the envelope.
    // Guard against a future contributor adding `target_version`,
    // `targetVersion`, or any calendar marker.
    let dir = tmpdir("no-date");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    let obj = v.as_object().expect("envelope is an object");
    for forbidden in [
        "target_version",
        "targetVersion",
        "removalVersion",
        "removal_version",
        "shipDate",
        "ship_date",
    ] {
        assert!(
            !obj.contains_key(forbidden),
            "envelope MUST NOT contain `{forbidden}` (spec §4.5 carve-out); got {obj:?}"
        );
    }
    let site = &v["sites"][0];
    let site_obj = site.as_object().expect("site is an object");
    for forbidden in ["target_version", "targetVersion", "shipDate"] {
        assert!(
            !site_obj.contains_key(forbidden),
            "per-site MUST NOT contain `{forbidden}`; got {site_obj:?}"
        );
    }
}

// ---------------------------------------------------------------
// migrationStatus — pending / acknowledged
// ---------------------------------------------------------------

#[test]
fn no_marker_means_pending() {
    let dir = tmpdir("pending");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0]["migrationStatus"], "pending");
}

#[test]
fn marker_on_line_immediately_above_means_acknowledged() {
    let dir = tmpdir("acknowledged");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
# pykrete: ack-deprecation
def f(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0]["migrationStatus"], "acknowledged");
}

#[test]
fn marker_with_leading_indent_is_recognized() {
    // The marker is line-trimmed, so an indented call-site marker still
    // matches. Mirrors v1.6 `# pykrete: ambiguous` mechanic.
    let dir = tmpdir("indented");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
def outer():
    # pykrete: ack-deprecation
    def f(df: DataFrame[Sale]) -> int:
        return 0
",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0]["migrationStatus"], "acknowledged");
}

#[test]
fn marker_with_blank_line_between_is_pending() {
    // Marker must be on the line IMMEDIATELY above. A blank line in
    // between breaks the association.
    let dir = tmpdir("blank-gap");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
# pykrete: ack-deprecation

def f(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1);
    assert_eq!(
        sites[0]["migrationStatus"], "pending",
        "blank line between marker and annotation breaks the association"
    );
}

#[test]
fn marker_with_extra_whitespace_inside_does_not_match() {
    // Comment marker must match exactly after trim. Extra internal
    // whitespace (`# pykrete:  ack-deprecation`) does NOT match.
    let dir = tmpdir("extra-ws");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
# pykrete:  ack-deprecation
def f(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    assert_eq!(v["sites"][0]["migrationStatus"], "pending");
}

#[test]
fn marker_case_mismatch_does_not_match() {
    // Case-sensitive per spec §4.4.
    let dir = tmpdir("case");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
# pykrete: ACK-DEPRECATION
def f(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    assert_eq!(v["sites"][0]["migrationStatus"], "pending");
}

#[test]
fn unrelated_comment_above_does_not_acknowledge() {
    let dir = tmpdir("unrelated");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
# TODO: migrate this
def f(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    assert_eq!(v["sites"][0]["migrationStatus"], "pending");
}

#[test]
fn marker_first_line_of_file_acknowledges_module_annotation() {
    // Edge: marker on line 1, annotation on line 2.
    let dir = tmpdir("first-line");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "# pykrete: ack-deprecation\ndef f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (_, v, _) = run_report_json(&["check", "--deprecation-report"], &p);
    assert_eq!(v["sites"][0]["migrationStatus"], "acknowledged");
}

// ---------------------------------------------------------------
// --ack filter
// ---------------------------------------------------------------

#[test]
fn ack_pending_filters_out_acknowledged_sites() {
    let dir = tmpdir("ack-pending");
    let p = write_pyk(
        &dir,
        "mix.pyk",
        "\
# pykrete: ack-deprecation
def ack(df: DataFrame[Sale]) -> int:
    return 0


def pend(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (exit, v, _) = run_report_json(&["check", "--deprecation-report", "--ack=pending"], &p);
    assert_eq!(exit, 0);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1, "only the un-acknowledged site survives");
    assert_eq!(sites[0]["migrationStatus"], "pending");
    assert_eq!(sites[0]["bindingName"], "df");
    assert_eq!(
        v["summary"]["totalSites"], 1,
        "summary reflects filtered count, not raw"
    );
}

#[test]
fn ack_acknowledged_filters_out_pending_sites() {
    let dir = tmpdir("ack-ack");
    let p = write_pyk(
        &dir,
        "mix.pyk",
        "\
# pykrete: ack-deprecation
def ack(df: DataFrame[Sale]) -> int:
    return 0


def pend(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (exit, v, _) =
        run_report_json(&["check", "--deprecation-report", "--ack=acknowledged"], &p);
    assert_eq!(exit, 0);
    let sites = v["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0]["migrationStatus"], "acknowledged");
    assert_eq!(v["summary"]["totalSites"], 1);
}

#[test]
fn ack_with_space_separated_value() {
    // `--ack pending` (separate arg) parses the same as `--ack=pending`.
    let dir = tmpdir("ack-space");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, v, _) = run_report_json(&["check", "--deprecation-report", "--ack", "pending"], &p);
    assert_eq!(exit, 0);
    assert_eq!(v["sites"].as_array().unwrap().len(), 1);
}

#[test]
fn ack_without_deprecation_report_errors() {
    // The flag is `--deprecation-report`-scoped; standalone use is a
    // usage error.
    let dir = tmpdir("ack-alone");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, _, stderr) = run(&["check", "--ack=pending"], &p);
    assert_eq!(exit, 2, "--ack alone exits 2");
    assert!(
        stderr.contains("--ack requires --deprecation-report"),
        "stderr should explain: {stderr:?}"
    );
}

#[test]
fn ack_resolved_value_is_rejected() {
    // pykrete never emits `resolved` (it's a consumer-side stateful-
    // delta sentinel). Filtering on it would always return empty,
    // masking user confusion. Reject at parse time.
    let dir = tmpdir("ack-resolved");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, _, stderr) = run(&["check", "--deprecation-report", "--ack=resolved"], &p);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("unknown --ack value 'resolved'"),
        "stderr should reject resolved: {stderr:?}"
    );
}

#[test]
fn ack_garbage_value_is_rejected() {
    let dir = tmpdir("ack-garbage");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "def f(df: DataFrame[Sale]) -> int:\n    return 0\n",
    );
    let (exit, _, stderr) = run(&["check", "--deprecation-report", "--ack=foo"], &p);
    assert_eq!(exit, 2);
    assert!(
        stderr.contains("unknown --ack value 'foo'"),
        "stderr should reject garbage: {stderr:?}"
    );
}

#[test]
fn ack_pending_against_all_acknowledged_returns_empty_sites() {
    // The CI-gate use case: every site is acknowledged → pending
    // filter returns empty → CI passes.
    let dir = tmpdir("all-ack");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
# pykrete: ack-deprecation
def f(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let (exit, v, _) = run_report_json(&["check", "--deprecation-report", "--ack=pending"], &p);
    assert_eq!(exit, 0);
    assert_eq!(v["sites"].as_array().unwrap().len(), 0);
    assert_eq!(v["summary"]["totalSites"], 0);
    assert_eq!(v["deprecationReportVersion"], "2");
}

// ---------------------------------------------------------------
// Marker is envelope-only, not a D0090 suppression
// ---------------------------------------------------------------

#[test]
fn ack_marker_does_not_suppress_d0090_in_text_mode() {
    // Per spec §4.4: the marker is a planning instrument, not a
    // suppression mechanism. Plain `pykrete check` (no
    // `--deprecation-report`) STILL fires D0090.
    let dir = tmpdir("no-suppress");
    let p = write_pyk(
        &dir,
        "x.pyk",
        "\
# pykrete: ack-deprecation
def f(df: DataFrame[Sale]) -> int:
    return 0
",
    );
    let out = Command::new(bin())
        .args(["check"])
        .arg(&p)
        .output()
        .expect("run pykrete check");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("deprecatedDataFrameAlias"),
        "D0090 must still fire under plain check: stderr={stderr:?}"
    );
}
