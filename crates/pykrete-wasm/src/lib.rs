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

use serde::Serialize;
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

/// Run pykrete on a single `.pyk` source string. Returns a JS array of
/// diagnostic objects (see [`DiagnosticOut`]).
///
/// The path passed to the analyzer is a synthetic `"<playground>.pyk"`
/// — the analyzer uses it for error formatting only, and the
/// single-file mode doesn't trigger any cross-file resolution that
/// would require a real path.
///
/// `serde_wasm_bindgen` returning an `Err` would mean a programming
/// error in the serialization layer, not a user-facing analyzer
/// problem; on that unlikely path we fall back to an empty array
/// rather than panicking (which on wasm would unwind the JS host).
#[wasm_bindgen]
pub fn check_source(source: &str) -> JsValue {
    let diagnostics = run_analyzer(source);
    serde_wasm_bindgen::to_value(&diagnostics).unwrap_or(JsValue::NULL)
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
}
