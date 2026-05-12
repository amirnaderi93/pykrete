//! Shared helpers for the integration tests under `tests/`.
//!
//! Each test file does `mod common;` to pull these in. The helpers all
//! center around running `dathon::check` against a small inline source
//! string and then asserting things about the resulting diagnostics.

#![allow(dead_code)]

use dathon::CheckResult;

/// Run dathon's analysis on a `.dpy` source string.
///
/// Tests write a small example as a raw string literal and pass it here.
/// The path embedded in diagnostics is `<test>.dpy` — purely cosmetic for
/// the tests, since we assert on codes and message contents, not on paths.
pub fn check(source: &str) -> CheckResult {
    dathon::check("<test>.dpy", source)
}

/// Assert that the analysis produced **no** diagnostics. On failure the
/// panic message lists every diagnostic that *was* produced.
pub fn assert_no_diagnostics(result: &CheckResult) {
    if !result.diagnostics.is_empty() {
        let messages: Vec<String> = result
            .diagnostics
            .iter()
            .map(|d| format!("{} {}: {}", d.code, d.severity_label(), d.message))
            .collect();
        panic!(
            "expected no diagnostics, but got {}:\n  {}",
            result.diagnostics.len(),
            messages.join("\n  ")
        );
    }
}

/// Assert that at least one diagnostic with the given code was produced.
pub fn assert_has_code(result: &CheckResult, code: &str) {
    if !result.has_code(code) {
        let actual_codes: Vec<&str> = result.diagnostics.iter().map(|d| d.code).collect();
        panic!(
            "expected at least one {} diagnostic; got codes: {:?}",
            code, actual_codes,
        );
    }
}

/// Assert that **no** diagnostic with the given code was produced.
pub fn assert_does_not_have_code(result: &CheckResult, code: &str) {
    if result.has_code(code) {
        let messages: Vec<String> = result
            .diagnostics_with_code(code)
            .iter()
            .map(|d| d.message.clone())
            .collect();
        panic!(
            "expected no {} diagnostic, but got {}:\n  {}",
            code,
            messages.len(),
            messages.join("\n  ")
        );
    }
}

/// Assert that exactly `expected` diagnostics with this code were produced.
pub fn assert_count(result: &CheckResult, code: &str, expected: usize) {
    let actual = result.count_code(code);
    if actual != expected {
        let messages: Vec<String> = result
            .diagnostics_with_code(code)
            .iter()
            .map(|d| format!("{}:{} {}", d.line, d.column, d.message))
            .collect();
        panic!(
            "expected {} {} diagnostic(s), got {}:\n  {}",
            expected,
            code,
            actual,
            messages.join("\n  ")
        );
    }
}

/// Assert that at least one diagnostic with the given code has a message
/// containing the given substring.
pub fn assert_message_contains(result: &CheckResult, code: &str, needle: &str) {
    let found = result
        .diagnostics
        .iter()
        .any(|d| d.code == code && d.message.contains(needle));
    if !found {
        let messages: Vec<String> = result
            .diagnostics_with_code(code)
            .iter()
            .map(|d| d.message.clone())
            .collect();
        panic!(
            "expected a {} diagnostic containing {:?}, got:\n  {}",
            code,
            needle,
            messages.join("\n  ")
        );
    }
}
