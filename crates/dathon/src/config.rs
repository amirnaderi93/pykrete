//! `dathon.json` — project configuration.
//!
//! A `dathon.json` at (or above) the directory `dathon check` runs in
//! tunes the run: the analysis strictness (`typeCheckingMode`), files to
//! skip (`exclude`), and per-diagnostic-code overrides (`rules`). Every
//! field is optional — no file at all means every default.
//!
//! ```json
//! {
//!   "typeCheckingMode": "strict",
//!   "exclude": ["legacy/", "generated/"],
//!   "rules": { "D0080": "off", "D0030": "warning" }
//! }
//! ```
//!
//! Finding and reading the file is the CLI's job; this module owns the
//! shape, the parse, and the lookups so they stay unit-testable.

use std::collections::HashMap;

use serde::Deserialize;

use crate::diagnostics::{CheckMode, Diagnostic, Severity};

/// Parsed `dathon.json`. Unknown keys are ignored, so a config written
/// for a newer dathon still loads.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// `"off"` / `"basic"` / `"standard"` / `"strict"`. Absent or
    /// unrecognized → `standard`.
    type_checking_mode: Option<String>,
    /// Path substrings; a file whose path contains any of them is not
    /// checked — `"tests/"` skips everything under a `tests` directory.
    exclude: Vec<String>,
    /// Per-diagnostic-code overrides, keyed by code (`"D0080"`).
    rules: HashMap<String, RuleSetting>,
}

/// What a `rules` entry does to its diagnostic code.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RuleSetting {
    /// Suppress every diagnostic with this code.
    Off,
    /// Force the diagnostic to a warning.
    Warning,
    /// Force the diagnostic to an error.
    Error,
}

impl Config {
    /// Parse `dathon.json` content. A syntactically invalid file is an
    /// `Err` carrying the parse message; the caller decides whether to
    /// warn-and-default or abort.
    pub fn parse(json: &str) -> Result<Config, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// The configured analysis strictness — `Standard` if unset.
    pub fn check_mode(&self) -> CheckMode {
        self.type_checking_mode
            .as_deref()
            .map_or(CheckMode::Standard, CheckMode::parse)
    }

    /// Whether `path` should be skipped — it contains a configured
    /// `exclude` substring.
    pub fn is_excluded(&self, path: &str) -> bool {
        self.exclude
            .iter()
            .any(|pat| !pat.is_empty() && path.contains(pat.as_str()))
    }

    /// Apply the `rules` overrides to one file's diagnostics in place:
    /// drop every code set to `off`, re-level the rest.
    pub fn apply_rules(&self, diagnostics: &mut Vec<Diagnostic>) {
        if self.rules.is_empty() {
            return;
        }
        diagnostics.retain_mut(|d| match self.rules.get(d.code) {
            Some(RuleSetting::Off) => false,
            Some(RuleSetting::Warning) => {
                d.severity = Severity::Warning;
                true
            }
            Some(RuleSetting::Error) => {
                d.severity = Severity::Error;
                true
            }
            None => true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(code: &'static str, severity: Severity) -> Diagnostic {
        Diagnostic {
            severity,
            code,
            message: String::new(),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
            suggestion: None,
            min_mode: CheckMode::Basic,
        }
    }

    #[test]
    fn an_empty_object_is_all_defaults() {
        let config = Config::parse("{}").expect("empty object parses");
        assert_eq!(config.check_mode(), CheckMode::Standard);
        assert!(!config.is_excluded("anything.dpy"));
        let mut diags = vec![diag("D0030", Severity::Error)];
        config.apply_rules(&mut diags);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn type_checking_mode_is_parsed() {
        let config = Config::parse(r#"{"typeCheckingMode": "strict"}"#).unwrap();
        assert_eq!(config.check_mode(), CheckMode::Strict);
        // An unrecognized value falls back to standard.
        let config = Config::parse(r#"{"typeCheckingMode": "ultra"}"#).unwrap();
        assert_eq!(config.check_mode(), CheckMode::Standard);
    }

    #[test]
    fn exclude_matches_a_path_substring() {
        let config = Config::parse(r#"{"exclude": ["legacy/", "generated/"]}"#).unwrap();
        assert!(config.is_excluded("src/legacy/orders.dpy"));
        assert!(config.is_excluded("generated/schema.dpy"));
        assert!(!config.is_excluded("src/orders.dpy"));
    }

    #[test]
    fn rules_off_drops_the_code() {
        let config = Config::parse(r#"{"rules": {"D0080": "off"}}"#).unwrap();
        let mut diags = vec![
            diag("D0080", Severity::Error),
            diag("D0030", Severity::Error),
        ];
        config.apply_rules(&mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "D0030");
    }

    #[test]
    fn rules_can_re_level_a_code() {
        let config = Config::parse(r#"{"rules": {"D0080": "warning"}}"#).unwrap();
        let mut diags = vec![diag("D0080", Severity::Error)];
        config.apply_rules(&mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        // A config key dathon doesn't know must not fail the parse —
        // forward compatibility.
        let config = Config::parse(r#"{"futureOption": true, "typeCheckingMode": "off"}"#).unwrap();
        assert_eq!(config.check_mode(), CheckMode::Off);
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(Config::parse("{not json").is_err());
    }
}
