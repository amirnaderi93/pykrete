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
