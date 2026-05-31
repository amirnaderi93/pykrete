//! Diagnostic types and formatting.
//!
//! Diagnostics are how the checker reports issues to the user.
//! Format mirrors TypeScript: `path:line:col - {severity} {code}: {message}`.

use ruff_source_file::LineIndex;
use ruff_text_size::{TextRange, TextSize};

/// How strict the checker is. Mirrors the embedded Python engine's
/// `typeCheckingMode` so a single `pykrete.typeCheckingMode` setting
/// drives both engines. Ordered weakest → strongest: a diagnostic
/// surfaces when the active mode is at least its [`Diagnostic::min_mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum CheckMode {
    /// No checking — the checker stays silent.
    Off,
    /// Fundamental errors only.
    Basic,
    /// The default — every error pykrete reports today.
    #[default]
    Standard,
    /// Standard plus advisory checks (e.g. structural-compatibility
    /// warnings) that would be too noisy for everyday use.
    Strict,
}

impl CheckMode {
    /// Parse the `typeCheckingMode` setting string. Unknown or missing
    /// values fall back to [`CheckMode::Standard`].
    pub fn parse(value: &str) -> CheckMode {
        match value {
            "off" => CheckMode::Off,
            "basic" => CheckMode::Basic,
            "strict" => CheckMode::Strict,
            _ => CheckMode::Standard,
        }
    }

    /// The setting-string form — also exactly what the embedded Python
    /// engine's `typeCheckingMode` expects.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckMode::Off => "off",
            CheckMode::Basic => "basic",
            CheckMode::Standard => "standard",
            CheckMode::Strict => "strict",
        }
    }

    /// Whether a diagnostic whose minimum mode is `min` surfaces at this
    /// mode. `Off` shows nothing.
    pub fn shows(self, min: CheckMode) -> bool {
        self != CheckMode::Off && self >= min
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    #[allow(dead_code)] // used once non-strict mode lands
    Warning,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
    /// A suggested replacement for the offending token, when one is
    /// available (e.g. for `D0030` we run a Levenshtein search over the
    /// schema's field names and surface the closest match). The LSP
    /// layer round-trips this so `textDocument/codeAction` can offer a
    /// quick-fix that swaps the literal in place.
    pub suggestion: Option<String>,
    /// The weakest [`CheckMode`] at which this diagnostic surfaces.
    /// Defaults to [`CheckMode::Basic`] (shown unless checking is off);
    /// advisory checks set [`CheckMode::Strict`].
    pub min_mode: CheckMode,
}

impl Diagnostic {
    /// Build a diagnostic anchored at a single source offset. End position
    /// equals start — produces a zero-width range. Prefer `at_range` when
    /// the offending token's extent is known.
    pub fn at(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        offset: TextSize,
        source: &str,
        line_index: &LineIndex,
    ) -> Self {
        Self::at_range(
            severity,
            code,
            message,
            TextRange::empty(offset),
            source,
            line_index,
        )
    }

    /// Build a diagnostic that spans the given source range. The CLI still
    /// prints only the start position, but LSP clients use the end to
    /// underline the entire offending token.
    pub fn at_range(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        range: TextRange,
        source: &str,
        line_index: &LineIndex,
    ) -> Self {
        let start = line_index.line_column(range.start(), source);
        let end = line_index.line_column(range.end(), source);
        Self {
            severity,
            code,
            message: message.into(),
            line: start.line.get(),
            column: start.column.get(),
            end_line: end.line.get(),
            end_column: end.column.get(),
            suggestion: None,
            min_mode: CheckMode::Basic,
        }
    }

    /// Attach a suggested replacement for the offending token. Used by
    /// `D0030` to surface the closest matching column name as a quick-fix.
    pub fn with_suggestion(mut self, suggestion: Option<String>) -> Self {
        self.suggestion = suggestion;
        self
    }

    /// Set the weakest [`CheckMode`] at which this diagnostic surfaces.
    /// Defaults to [`CheckMode::Basic`]; advisory checks that should
    /// only fire under `strict` use [`CheckMode::Strict`].
    pub fn with_min_mode(mut self, min_mode: CheckMode) -> Self {
        self.min_mode = min_mode;
        self
    }

    pub fn format(&self, path: &str) -> String {
        format!(
            "{}:{}:{} - {} {}: {}",
            path,
            self.line,
            self.column,
            self.severity.label(),
            rule_name(self.code),
            self.message,
        )
    }

    /// Convenience accessor used by test helpers.
    pub fn severity_label(&self) -> &'static str {
        self.severity.label()
    }
}

/// The canonical list of `(code, rule_name)` pairs — the source of truth
/// for every diagnostic pykrete can emit. `rule_name` looks up against
/// this table; the diagnostic-catalog snapshot test iterates it to assert
/// that every code has a fixture and a snapshot. Adding a new code without
/// updating the catalog breaks the test, forcing the discipline.
pub const DIAGNOSTIC_CATALOG: &[(&str, &str)] = &[
    ("D0001", "parseError"),
    ("D0010", "unknownColumnType"),
    ("D0011", "invalidColumnType"),
    ("D0020", "unknownSchema"),
    ("D0021", "invalidSchemaExpression"),
    ("D0030", "unknownColumn"),
    ("D0040", "unionSchemaMismatch"),
    ("D0050", "returnColumnsMismatch"),
    ("D0051", "argumentColumnsMismatch"),
    ("D0060", "missingJoinKey"),
    ("D0070", "unresolvedImport"),
    ("D0071", "unexportedName"),
    ("D0072", "duplicateSchemaName"),
    ("D0073", "transformInputMismatch"),
    ("D0080", "returnTypeMismatch"),
    ("D0081", "nonNumericArithmetic"),
    ("D0082", "crossTypeComparison"),
    ("D0083", "nullabilityMismatch"),
];

/// The human-readable name for a diagnostic code — what the CLI and
/// editor display, and what `pykrete.json`'s `rules` block keys on. The
/// `D00xx` code stays the stable internal identifier (and the form the
/// docs index by); this is the friendly surface. An unrecognized code
/// (shouldn't happen) is returned unchanged.
pub fn rule_name(code: &str) -> &str {
    for (c, name) in DIAGNOSTIC_CATALOG {
        if *c == code {
            return name;
        }
    }
    code
}
