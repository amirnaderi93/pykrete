//! Diagnostic types and formatting.
//!
//! Diagnostics are how the checker reports issues to the user.
//! Format mirrors TypeScript: `path:line:col - {severity} {code}: {message}`.

use ruff_source_file::LineIndex;
use ruff_text_size::TextSize;

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
}

impl Diagnostic {
    pub fn at(
        severity: Severity,
        code: &'static str,
        message: impl Into<String>,
        offset: TextSize,
        source: &str,
        line_index: &LineIndex,
    ) -> Self {
        let lc = line_index.line_column(offset, source);
        Self {
            severity,
            code,
            message: message.into(),
            line: lc.line.get(),
            column: lc.column.get(),
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
