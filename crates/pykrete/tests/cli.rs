//! End-to-end CLI tests. Invokes the built `pykrete` binary via
//! `std::process::Command` against fixture `.pyk` files. Cargo sets
//! `CARGO_BIN_EXE_pykrete` to the path of the test binary, so these
//! tests run against the exact build under test.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pykrete")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn version_flag_prints_workspace_version() {
    let out = Command::new(bin())
        .arg("--version")
        .output()
        .expect("run pykrete --version");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("pykrete "),
        "stdout did not start with 'pykrete ': {stdout:?}"
    );
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "stdout did not contain workspace version {}: {stdout:?}",
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn version_short_flag_also_works() {
    let out = Command::new(bin())
        .arg("-V")
        .output()
        .expect("run pykrete -V");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("pykrete "));
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_flag_prints_usage() {
    let out = Command::new(bin())
        .arg("--help")
        .output()
        .expect("run pykrete --help");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("check"), "help missing 'check': {stdout}");
    assert!(
        stdout.contains("transpile"),
        "help missing 'transpile': {stdout}"
    );
    assert!(
        stdout.contains("--version"),
        "help missing '--version': {stdout}"
    );
}

#[test]
fn no_args_prints_help_and_exits_zero() {
    let out = Command::new(bin()).output().expect("run pykrete");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Usage:"), "no-arg output missing 'Usage:'");
    assert!(stdout.contains("check"));
}

#[test]
fn unknown_command_errors() {
    let out = Command::new(bin())
        .arg("bogus")
        .output()
        .expect("run pykrete bogus");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown command"));
}

#[test]
fn check_with_no_file_errors() {
    let out = Command::new(bin())
        .arg("check")
        .output()
        .expect("run pykrete check");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("specify a file"));
}

#[test]
fn check_default_output_is_quiet_on_success() {
    let out = Command::new(bin())
        .arg("check")
        .arg(fixture("clean.pyk"))
        .output()
        .expect("run pykrete check clean.pyk");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "default output should be a single summary line, got:\n{stdout}"
    );
    assert!(
        lines[0].contains("parsed OK"),
        "summary missing 'parsed OK': {stdout}"
    );
    assert!(
        lines[0].contains("schema(s)") && lines[0].contains("typed function(s)"),
        "summary missing schema/function counts: {stdout}"
    );
    assert!(
        !stdout.contains("schema Sale"),
        "default output leaked schema dump: {stdout}"
    );
}

#[test]
fn check_verbose_flag_restores_full_dump() {
    let out = Command::new(bin())
        .arg("check")
        .arg("--verbose")
        .arg(fixture("clean.pyk"))
        .output()
        .expect("run pykrete check --verbose clean.pyk");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("parsed OK"),
        "verbose missing summary: {stdout}"
    );
    assert!(
        stdout.contains("schema Sale"),
        "verbose missing schema dump: {stdout}"
    );
    assert!(
        stdout.contains("fn revenue"),
        "verbose missing function signature: {stdout}"
    );
}

#[test]
fn check_verbose_short_flag_also_works() {
    let out = Command::new(bin())
        .arg("check")
        .arg("-v")
        .arg(fixture("clean.pyk"))
        .output()
        .expect("run pykrete check -v clean.pyk");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("schema Sale"));
}

#[test]
fn check_with_diagnostics_prints_them() {
    let out = Command::new(bin())
        .arg("check")
        .arg(fixture("typo.pyk"))
        .output()
        .expect("run pykrete check typo.pyk");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknownColumn"),
        "stderr missing unknownColumn diagnostic: {stderr}"
    );
    assert!(
        stderr.contains("regoin"),
        "stderr missing the typo'd column name: {stderr}"
    );
}

#[test]
fn check_help_flag_prints_check_usage() {
    let out = Command::new(bin())
        .arg("check")
        .arg("--help")
        .output()
        .expect("run pykrete check --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--verbose"));
    assert!(stdout.contains("pykrete check"));
}

#[test]
fn check_format_json_clean_file_emits_valid_json() {
    let out = Command::new(bin())
        .args(["check", "--format", "json"])
        .arg(fixture("clean.pyk"))
        .output()
        .expect("run pykrete check --format json clean.pyk");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    // schemaVersion is the wire-format version; pinned to "1" until a
    // SemVer-major breaking change forces a bump.
    assert_eq!(v["schemaVersion"], "1");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    // Empty array, NOT null — important for downstream tools.
    let diags = v["diagnostics"]
        .as_array()
        .expect("diagnostics is an array");
    assert!(diags.is_empty(), "expected zero diagnostics: {stdout}");
    assert_eq!(v["summary"]["filesChecked"], 1);
    assert_eq!(v["summary"]["errorCount"], 0);
    assert_eq!(v["summary"]["warningCount"], 0);
}

#[test]
fn check_format_json_dirty_file_carries_full_diagnostic_shape() {
    let out = Command::new(bin())
        .args(["check", "--format", "json"])
        .arg(fixture("typo.pyk"))
        .output()
        .expect("run pykrete check --format json typo.pyk");
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    let diags = v["diagnostics"]
        .as_array()
        .expect("diagnostics is an array");
    // The `regoin` typo triggers both D0030 (unknown column) and D0050
    // (return columns mismatch). The shape assertion targets the D0030
    // entry.
    let d = diags
        .iter()
        .find(|d| d["code"] == "D0030")
        .unwrap_or_else(|| panic!("D0030 missing in JSON: {stdout}"));
    assert_eq!(d["ruleName"], "unknownColumn");
    assert_eq!(d["severity"], "error");
    // Source pins to the literal string "pykrete" so CI aggregators
    // (reviewdog etc.) can disambiguate from other linters in the same
    // workflow.
    assert_eq!(d["source"], "pykrete");
    // 1-indexed positions; the typo is in `col("regoin")` on line 7.
    assert_eq!(d["line"], 7);
    assert!(d["column"].as_u64().unwrap() >= 1);
    assert_eq!(d["endLine"], 7);
    assert!(d["message"].as_str().unwrap().contains("regoin"));
    assert!(d["file"].as_str().unwrap().ends_with("typo.pyk"));
    assert!(d["relatedInformation"].is_array());
    // D0030 carries the "did you mean" suggestion as its own structured
    // field so quick-fix tooling doesn't have to grep the message.
    assert_eq!(d["suggestion"], "region");
    assert_eq!(v["summary"]["errorCount"], diags.len());
    assert_eq!(v["summary"]["warningCount"], 0);
}

#[test]
fn check_format_json_emits_null_suggestion_when_absent() {
    // The companion D0050 diagnostic on the same fixture doesn't carry
    // a suggestion — it should serialize as JSON null, never omitted.
    let out = Command::new(bin())
        .args(["check", "--format", "json"])
        .arg(fixture("typo.pyk"))
        .output()
        .expect("run pykrete check --format json typo.pyk");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    let diags = v["diagnostics"]
        .as_array()
        .expect("diagnostics is an array");
    let d = diags
        .iter()
        .find(|d| d["code"] == "D0050")
        .unwrap_or_else(|| panic!("D0050 missing in JSON: {stdout}"));
    assert!(
        d.get("suggestion").is_some(),
        "suggestion field should always be present: {d}"
    );
    assert!(
        d["suggestion"].is_null(),
        "expected null suggestion on D0050, got {}: {stdout}",
        d["suggestion"]
    );
}

#[test]
fn check_format_json_source_always_present() {
    // Even with zero diagnostics the empty diagnostics array is still
    // structured correctly — this test pins source for non-empty cases
    // and schemaVersion across the board.
    let out = Command::new(bin())
        .args(["check", "--format", "json"])
        .arg(fixture("typo.pyk"))
        .output()
        .expect("run pykrete check --format json typo.pyk");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON: {e}\n{stdout}"));
    assert_eq!(v["schemaVersion"], "1");
    let diags = v["diagnostics"]
        .as_array()
        .expect("diagnostics is an array");
    assert!(!diags.is_empty(), "expected diagnostics: {stdout}");
    for d in diags {
        assert_eq!(d["source"], "pykrete", "source missing on: {d}");
    }
}

#[test]
fn check_format_text_matches_default() {
    // `--format text` must be byte-identical to the bare default form,
    // both on stdout and stderr.
    let default = Command::new(bin())
        .arg("check")
        .arg(fixture("typo.pyk"))
        .output()
        .expect("run pykrete check typo.pyk");
    let explicit_text = Command::new(bin())
        .args(["check", "--format", "text"])
        .arg(fixture("typo.pyk"))
        .output()
        .expect("run pykrete check --format text typo.pyk");
    assert_eq!(default.status, explicit_text.status);
    assert_eq!(default.stdout, explicit_text.stdout);
    assert_eq!(default.stderr, explicit_text.stderr);
}

#[test]
fn check_format_json_eq_syntax_works() {
    let out = Command::new(bin())
        .args(["check", "--format=json"])
        .arg(fixture("clean.pyk"))
        .output()
        .expect("run pykrete check --format=json clean.pyk");
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let _: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid JSON for --format=json: {e}\n{stdout}"));
}

#[test]
fn check_format_unknown_value_errors() {
    let out = Command::new(bin())
        .args(["check", "--format", "yaml"])
        .arg(fixture("clean.pyk"))
        .output()
        .expect("run pykrete check --format yaml");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown --format"),
        "stderr missing 'unknown --format': {stderr}"
    );
}

#[test]
fn check_format_missing_value_errors() {
    let out = Command::new(bin())
        .args(["check", "--format"])
        .output()
        .expect("run pykrete check --format (no value)");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("requires a value"),
        "stderr missing 'requires a value': {stderr}"
    );
}

#[test]
fn check_help_lists_format_flag() {
    let out = Command::new(bin())
        .args(["check", "--help"])
        .output()
        .expect("run pykrete check --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--format"),
        "check --help missing '--format': {stdout}"
    );
}

#[test]
fn transpile_help_flag_prints_transpile_usage() {
    let out = Command::new(bin())
        .arg("transpile")
        .arg("--help")
        .output()
        .expect("run pykrete transpile --help");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("pykrete transpile"));
}
