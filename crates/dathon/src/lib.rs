//! dathon — a static schema checker for PySpark code.
//!
//! This is the library crate. The CLI shell at `src/main.rs` is a thin
//! wrapper around [`check`].
//!
//! The high-level entry point is [`check`], which runs the whole analysis
//! pipeline on one source file's text and returns a [`CheckResult`].
//! Integration tests under `crates/dathon/tests/` call [`check`] directly
//! rather than spawning the binary.
//!
//! See [`docs/design/architecture.md`](../../docs/design/architecture.md) for
//! the full picture; the short version of the pipeline is:
//!
//! 1. Parse with `ruff_python_parser` → Python AST.
//! 2. Walk the module for top-level class and function declarations.
//! 3. Recognize Schema classes; resolve their fields against the
//!    `ColumnType` vocabulary.
//! 4. Recognize `DataFrame[X]` annotations on function signatures.
//! 5. For each typed function, walk the body — checking column references
//!    on `select`/`filter`/etc., inferring result schemas through chains,
//!    propagating bindings on `x = …` assignments, and validating the
//!    `return` value against the declared return type.

pub mod dataframe;
pub mod diagnostics;
pub mod operations;
pub mod registry;
pub mod schema;
pub mod types;
pub mod walk;

use std::fmt::Write as _;

use ruff_source_file::LineIndex;
use ruff_text_size::Ranged;

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot, typed_slots};
use crate::diagnostics::{Diagnostic, Severity};
use crate::operations::{BodyContext, check_function_body};
use crate::registry::Registry;
use crate::schema::{FieldResolution, Schema, discover_schemas};
use crate::types::COLUMN_TYPE_NAMES;
use crate::walk::{DiscoveredFunction, discover_top_level_classes, discover_top_level_functions};

/// Outcome of running the checker on a single source file.
///
/// `diagnostics` is the user-facing list of errors/warnings. `body` is the
/// pretty-printed schema + function summary that the CLI prints to stdout.
/// `parse_error` is true when the file couldn't be parsed at all (the only
/// diagnostic is the parse error and the body is empty).
pub struct CheckResult {
    pub diagnostics: Vec<Diagnostic>,
    pub body: String,
    pub schema_count: usize,
    pub typed_function_count: usize,
    pub parse_error: bool,
}

impl CheckResult {
    /// Whether any diagnostic with the given code appears.
    pub fn has_code(&self, code: &str) -> bool {
        self.diagnostics.iter().any(|d| d.code == code)
    }

    /// How many diagnostics with the given code appear.
    pub fn count_code(&self, code: &str) -> usize {
        self.diagnostics.iter().filter(|d| d.code == code).count()
    }

    /// All diagnostics with the given code, in order.
    pub fn diagnostics_with_code(&self, code: &str) -> Vec<&Diagnostic> {
        self.diagnostics.iter().filter(|d| d.code == code).collect()
    }
}

/// Run the full checker pipeline against one source file's text.
///
/// `path` is used only for diagnostic-message formatting; the source is read
/// from `source`. Returns a [`CheckResult`].
pub fn check(_path: &str, source: &str) -> CheckResult {
    let line_index = LineIndex::from_source_text(source);

    let parsed = match ruff_python_parser::parse_module(source) {
        Ok(p) => p,
        Err(err) => {
            let d = Diagnostic::at(
                Severity::Error,
                "D0001",
                err.error.to_string(),
                err.location.start(),
                source,
                &line_index,
            );
            return CheckResult {
                diagnostics: vec![d],
                body: String::new(),
                schema_count: 0,
                typed_function_count: 0,
                parse_error: true,
            };
        }
    };

    let module = parsed.syntax();
    let classes = discover_top_level_classes(module);
    let schemas = discover_schemas(&classes);
    let functions = discover_top_level_functions(module);
    let registry = Registry::build(module);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut body = String::new();

    for schema in &schemas {
        render_schema(
            schema,
            &schemas,
            source,
            &line_index,
            &mut body,
            &mut diagnostics,
        );
    }

    let typed_functions: Vec<_> = functions
        .iter()
        .map(|f| (f, typed_slots(f)))
        .filter(|(_, slots)| !slots.is_empty())
        .collect();

    for (func, slots) in &typed_functions {
        render_function(
            func,
            slots,
            &schemas,
            source,
            &line_index,
            &mut body,
            &mut diagnostics,
        );
        let declared_return = declared_return_schema(slots, &schemas);
        let mut ctx = BodyContext::from_function(func, slots, &schemas, &registry);
        check_function_body(
            func,
            declared_return,
            &mut ctx,
            source,
            &line_index,
            &mut diagnostics,
        );
    }

    CheckResult {
        diagnostics,
        body,
        schema_count: schemas.len(),
        typed_function_count: typed_functions.len(),
        parse_error: false,
    }
}

fn declared_return_schema<'a>(
    slots: &[TypedSlot<'a>],
    schemas: &'a [Schema<'a>],
) -> Option<&'a Schema<'a>> {
    for slot in slots {
        if matches!(slot.label, SlotLabel::Return) {
            if let DataFrameAnnotation::Typed(name) = slot.kind {
                return schemas.iter().find(|s| s.name() == name);
            }
        }
    }
    None
}

fn render_schema<'a>(
    schema: &'a Schema<'a>,
    schemas: &'a [Schema<'a>],
    source: &str,
    line_index: &LineIndex,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let lc = line_index.line_column(schema.class.def.range.start(), source);
    writeln!(
        out,
        "  {}:{}  schema {}",
        lc.line.get(),
        lc.column.get(),
        schema.name(),
    )
    .unwrap();
    for field in schema.fields() {
        let ann_range = field.annotation.range();
        let raw_text = &source[ann_range];
        match field.resolve(schemas) {
            FieldResolution::Resolved(ct) => {
                writeln!(out, "          {}: {}", field.name, ct).unwrap();
            }
            FieldResolution::ResolvedNested(nested) => {
                writeln!(out, "          {}: {} (nested)", field.name, nested.name()).unwrap();
            }
            FieldResolution::UnknownType { name } => {
                writeln!(out, "          {}: {}  (unresolved)", field.name, raw_text).unwrap();
                diagnostics.push(Diagnostic::at(
                    Severity::Error,
                    "D0010",
                    format!("Unknown column type '{name}'. Expected one of: {COLUMN_TYPE_NAMES}.",),
                    ann_range.start(),
                    source,
                    line_index,
                ));
            }
            FieldResolution::NotABareName => {
                writeln!(out, "          {}: {}  (unresolved)", field.name, raw_text).unwrap();
                diagnostics.push(Diagnostic::at(
                    Severity::Error,
                    "D0011",
                    format!(
                        "Column type '{raw_text}' is not a bare name. \
                         Subscripted/complex column types are not yet \
                         supported in v0.1. Use one of: {COLUMN_TYPE_NAMES}.",
                    ),
                    ann_range.start(),
                    source,
                    line_index,
                ));
            }
        }
    }
}

fn render_function(
    func: &DiscoveredFunction<'_>,
    slots: &[TypedSlot<'_>],
    schemas: &[Schema<'_>],
    source: &str,
    line_index: &LineIndex,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let lc = line_index.line_column(func.def.range.start(), source);
    writeln!(
        out,
        "  {}:{}  fn {}",
        lc.line.get(),
        lc.column.get(),
        func.name()
    )
    .unwrap();

    for slot in slots {
        let ann_range = slot.annotation.range();
        let raw_text = &source[ann_range];
        let prefix = match slot.label {
            SlotLabel::Param(name) => format!("          {name}: "),
            SlotLabel::Return => "          -> ".to_string(),
        };

        match slot.kind {
            DataFrameAnnotation::Typed(name) => {
                if schemas.iter().any(|s| s.name() == name) {
                    writeln!(out, "{prefix}DataFrame[{name}]").unwrap();
                } else {
                    writeln!(out, "{prefix}{raw_text}  (unresolved)").unwrap();
                    diagnostics.push(Diagnostic::at(
                        Severity::Error,
                        "D0020",
                        format!(
                            "Unknown schema '{name}' referenced in DataFrame[…]. \
                             Declare it as a class extending Schema.",
                        ),
                        ann_range.start(),
                        source,
                        line_index,
                    ));
                }
            }
            DataFrameAnnotation::Untyped => {
                writeln!(out, "{prefix}DataFrame  (untyped)").unwrap();
            }
            DataFrameAnnotation::NonBareName => {
                writeln!(out, "{prefix}{raw_text}  (unresolved)").unwrap();
                diagnostics.push(Diagnostic::at(
                    Severity::Error,
                    "D0021",
                    format!(
                        "DataFrame schema must be a bare name; got '{raw_text}'. \
                         Subscripted/complex schema expressions are not supported in v0.1.",
                    ),
                    ann_range.start(),
                    source,
                    line_index,
                ));
            }
        }
    }
}
