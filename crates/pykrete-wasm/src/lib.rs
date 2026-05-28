//! WebAssembly wrapper around the pykrete analyzer.
//!
//! This crate is intentionally thin. It exposes a single
//! `check_source(source)` function across the wasm-bindgen boundary;
//! that function runs pykrete's standard single-file analyzer against
//! the supplied `.pyk` source and returns a list of diagnostics as
//! plain JS objects.
//!
//! The browser playground (Monaco editor + this wasm module, wired up
//! in a follow-up PR) is the consumer. No filesystem access, no
//! project-config loading, no cross-file resolution — those all need
//! a real file system and are out of scope for the playground.
//!
//! The analyzer call goes through the same public entry point the CLI
//! and LSP use (`pykrete::check`), so behavior in the playground
//! matches what users get locally.
//!
//! ## Panic safety
//!
//! A Rust panic that crosses the wasm-bindgen boundary aborts the
//! whole module and tears down the JS host's view of it — every
//! subsequent `check_source` call would fail with a poisoned-instance
//! error, and the playground would visibly break. The analyzer is
//! well-tested, but the playground feeds it arbitrary user input from
//! a public website, so a defensive `catch_unwind` around the
//! analyzer call is cheap insurance.
//!
//! The hook from `console_error_panic_hook` routes panic messages to
//! `console.error` in the browser. That doesn't *prevent* the panic
//! propagating — `catch_unwind` does — but it makes the panic
//! debuggable when someone files a bug report.

use serde::Serialize;
use std::panic;
use wasm_bindgen::prelude::*;

/// One diagnostic, in a shape friendly to a JS / TS host.
///
/// Mirrors `pykrete::diagnostics::Diagnostic` but trims the Rust-only
/// bits (the `min_mode` filter is already applied; the optional
/// `suggestion` is dropped for v1 — the playground doesn't render
/// quick-fixes yet). Line and column are 1-indexed, matching pykrete's
/// CLI output and what Monaco expects for marker positions.
#[derive(Serialize)]
struct DiagnosticOut {
    /// Stable diagnostic identifier, e.g. `"D0030"`.
    code: String,
    /// Friendly rule name, e.g. `"unknownColumn"`.
    rule_name: String,
    /// `"error"` or `"warning"`.
    severity: String,
    message: String,
    line: u32,
    column: u32,
    end_line: u32,
    end_column: u32,
}

/// Runs once when the wasm module is instantiated. Installs a panic
/// hook so panic messages show up in the browser's devtools console
/// — without this, panics print nothing useful (just a generic wasm
/// trap).
///
/// `#[wasm_bindgen(start)]` makes wasm-bindgen call this from the
/// auto-generated JS init function — the user doesn't have to do
/// anything extra.
#[wasm_bindgen(start)]
pub fn _start() {
    console_error_panic_hook::set_once();
}

/// Run pykrete on a single `.pyk` source string. Returns a JS array of
/// diagnostic objects (see [`DiagnosticOut`]).
///
/// The path passed to the analyzer is a synthetic `"<playground>.pyk"`
/// — the analyzer uses it for error formatting only, and the
/// single-file mode doesn't trigger any cross-file resolution that
/// would require a real path.
///
/// ## Failure modes
///
/// - If the analyzer panics, this function returns a single synthetic
///   `D9999` "internal error" diagnostic pointing the user at the
///   bug tracker. A visible failure is better than a silent one — a
///   user filing an issue with a reproducer is more useful than
///   "everything looked fine but my code was broken."
/// - If `serde_wasm_bindgen` fails to serialize the diagnostics list
///   (very unlikely — the shape is plain owned strings and numbers),
///   we return an empty array. Returning `JsValue::NULL` would crash
///   JS callers iterating `.length`; an empty array is safe.
#[wasm_bindgen]
pub fn check_source(source: &str) -> JsValue {
    // `catch_unwind` requires an `UnwindSafe` closure. `&str` is
    // unwind-safe, and `run_analyzer` returns an owned `Vec` — no
    // shared mutable state crosses the boundary, so this is fine.
    let result = panic::catch_unwind(|| run_analyzer(source));
    let diagnostics = result.unwrap_or_else(|_| vec![internal_error_diagnostic()]);
    serde_wasm_bindgen::to_value(&diagnostics).unwrap_or_else(|_| {
        // Fall back to an empty JS array, never null — JS code doing
        // `for (const d of result)` or `result.length` would crash on
        // null but handle `[]` cleanly.
        serde_wasm_bindgen::to_value(&Vec::<DiagnosticOut>::new()).expect("empty Vec serializes")
    })
}

/// Synthetic diagnostic shown when the analyzer panics. The position
/// is `(1, 1, 1, 1)` — there's no real source range to point at, and
/// putting it at the top of the file keeps it visible and out of the
/// way of whatever the user is typing.
fn internal_error_diagnostic() -> DiagnosticOut {
    DiagnosticOut {
        code: "D9999".to_string(),
        rule_name: "internalError".to_string(),
        severity: "error".to_string(),
        message: "pykrete panicked during analysis — please report this as a bug at \
             github.com/amirnaderi93/pykrete/issues"
            .to_string(),
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 1,
    }
}

/// Core analyzer call, factored out so the unit tests below can hit
/// it without paying the wasm-bindgen / `JsValue` round-trip.
fn run_analyzer(source: &str) -> Vec<DiagnosticOut> {
    let result = pykrete::check("<playground>.pyk", source);
    result
        .diagnostics
        .into_iter()
        .map(|d| DiagnosticOut {
            code: d.code.to_string(),
            rule_name: pykrete::diagnostics::rule_name(d.code).to_string(),
            severity: d.severity.label().to_string(),
            message: d.message,
            line: d.line as u32,
            column: d.column as u32,
            end_line: d.end_line as u32,
            end_column: d.end_column as u32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.pyk` source with no schema issues should produce no
    /// diagnostics. Keeps the wrapper honest — if pykrete starts
    /// flagging a benign file, the test catches it here too.
    #[test]
    fn check_source_with_clean_pyk_returns_no_diagnostics() {
        // `.pyk` syntax: column types are bare atomic names (no
        // quotes), Schema / DataFrame / col are pre-imported by the
        // transpiler — same shape as `examples/orders.pyk`.
        let source = r#"
class Users(Schema):
    id: int
    name: string

def keep(df: DataFrame[Users]) -> DataFrame[Users]:
    return df.select(col("id"), col("name"))
"#;
        let diags = run_analyzer(source);
        assert!(
            diags.is_empty(),
            "expected no diagnostics, got: {:?}",
            diags
                .iter()
                .map(|d| (&d.code, &d.message))
                .collect::<Vec<_>>()
        );
    }

    /// A `.pyk` source that selects a misspelled column should
    /// surface `D0030` (`unknownColumn`).
    #[test]
    fn check_source_with_typo_returns_d0030() {
        let source = r#"
class Users(Schema):
    id: int
    name: string

def keep(df: DataFrame[Users]) -> DataFrame[Users]:
    return df.select(col("id"), col("nme"))
"#;
        let diags = run_analyzer(source);
        assert!(
            diags.iter().any(|d| d.code == "D0030"),
            "expected a D0030 diagnostic, got: {:?}",
            diags
                .iter()
                .map(|d| (&d.code, &d.message))
                .collect::<Vec<_>>()
        );
        let d = diags.iter().find(|d| d.code == "D0030").unwrap();
        assert_eq!(d.rule_name, "unknownColumn");
        assert_eq!(d.severity, "error");
        assert!(d.line >= 1 && d.column >= 1);
    }

    /// A source with a hard Python parse error should surface
    /// `D0001` (`parseError`) and short-circuit further analysis.
    #[test]
    fn check_source_with_invalid_python_returns_d0001() {
        let source = "def broken(\n";
        let diags = run_analyzer(source);
        assert!(
            diags.iter().any(|d| d.code == "D0001"),
            "expected a D0001 diagnostic, got: {:?}",
            diags
                .iter()
                .map(|d| (&d.code, &d.message))
                .collect::<Vec<_>>()
        );
    }

    /// Smoke test for the panic-safety net.
    ///
    /// We can't easily construct an input that makes pykrete itself
    /// panic — the analyzer is well-tested and the wasm wrapper is
    /// thin. But we *can* verify the `catch_unwind` / fallback wiring
    /// works in isolation by panicking from a closure with the same
    /// shape `check_source` uses, then checking the fallback path
    /// produces the synthetic `D9999` diagnostic.
    ///
    /// If pykrete ever does panic on real input, this test confirms
    /// the safety net would catch it instead of tearing down the
    /// wasm host.
    #[test]
    fn catch_unwind_falls_back_to_internal_error_diagnostic() {
        let result: Result<Vec<DiagnosticOut>, _> =
            panic::catch_unwind(|| panic!("simulated analyzer panic"));
        let diagnostics = result.unwrap_or_else(|_| vec![internal_error_diagnostic()]);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "D9999");
        assert_eq!(diagnostics[0].rule_name, "internalError");
        assert_eq!(diagnostics[0].severity, "error");
        assert!(diagnostics[0].message.contains("pykrete panicked"));
        assert!(diagnostics[0].message.contains("github.com"));
    }
}
