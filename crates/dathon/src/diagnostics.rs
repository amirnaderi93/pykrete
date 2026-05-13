//! Diagnostic types and formatting.
//!
//! Diagnostics are how the checker reports issues to the user.
//! Format mirrors TypeScript: `path:line:col - {severity} {code}: {message}`.

use ruff_source_file::LineIndex;
use ruff_text_size::{TextRange, TextSize};

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
        }
    }

    pub fn format(&self, path: &str) -> String {
        format!(
            "{}:{}:{} - {} {}: {}",
            path,
            self.line,
            self.column,
            self.severity.label(),
            self.code,
            self.message,
        )
    }

    /// Convenience accessor used by test helpers.
    pub fn severity_label(&self) -> &'static str {
        self.severity.label()
    }
}
