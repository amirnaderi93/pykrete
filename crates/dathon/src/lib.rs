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
pub mod hover;
pub mod operations;
pub mod registry;
pub mod schema;
pub mod symbols;
pub mod transpiler;
pub mod types;
pub mod walk;

pub use hover::{HoverInfo, hover};
pub use symbols::{DocumentSymbol, Span, SymbolKind, definition, document_symbols};
pub use transpiler::transpile;

use std::collections::HashMap;
use std::fmt::Write as _;

use ruff_python_ast::ModModule;
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
///
/// In multi-file mode, `schema_count` and `typed_function_count` are the
/// LOCAL counts for this file — schemas declared in OTHER files of the same
/// project are still visible to this file for resolution, but they don't
/// add to its declared-here counts.
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

/// One file's analysis result inside a multi-file project.
pub struct ProjectFileResult {
    pub path: String,
    pub result: CheckResult,
}

/// Outcome of running the checker on a multi-file project. Each file's
/// analysis is reported separately, but all files share a combined view of
/// declared Schemas, classes, and typed constants — so cross-file
/// references resolve.
pub struct ProjectCheckResult {
    pub files: Vec<ProjectFileResult>,
}

impl ProjectCheckResult {
    pub fn total_diagnostics(&self) -> usize {
        self.files.iter().map(|f| f.result.diagnostics.len()).sum()
    }

    pub fn has_any_diagnostic(&self) -> bool {
        self.files.iter().any(|f| !f.result.diagnostics.is_empty())
    }
}

/// Run the full checker on a single source file.
///
/// Equivalent to calling [`check_project`] with a one-element list. Kept as
/// a convenience for callers that only have one file (and for backward
/// compatibility with the existing test suite).
pub fn check(path: &str, source: &str) -> CheckResult {
    let project = check_project(&[(path.to_string(), source.to_string())]);
    project.files.into_iter().next().unwrap().result
}

/// Run the full checker over multiple source files as one project.
///
/// All declared Schemas, classes, and top-level annotated constants across
/// every supplied file are pooled into a single resolution scope. Each
/// file is then analyzed against that pooled scope, and per-file
/// diagnostics are returned in input order.
///
/// Scope cuts in v0.1:
/// - No `import` statement parsing — every declaration is visible
///   everywhere, regardless of which file declared it.
/// - No directory walking; the CLI shell does that itself if needed.
/// - Duplicate top-level names across files: last one wins in the
///   combined registry (last-declared overrides). We don't currently warn
///   about duplicates.
pub fn check_project(files: &[(String, String)]) -> ProjectCheckResult {
    // Phase 1: parse every file. Successful parses produce a module; parse
    // failures produce a single D0001 for that file.
    let parsed: Vec<_> = files
        .iter()
        .map(|(_, src)| ruff_python_parser::parse_module(src))
        .collect();

    // Phase 2: collect every module reference for the successfully-parsed
    // files. These are kept alive by `parsed` for the rest of the function.
    let modules: Vec<&ModModule> = parsed
        .iter()
        .filter_map(|r| r.as_ref().ok().map(|p| p.syntax()))
        .collect();

    // Phase 3: build the combined view across all parsed modules.
    // - all_classes: every top-level class def from every file.
    // - all_schemas: those of all_classes whose bases include `Schema`.
    // - combined_registry: classes + constants from every file (last-wins
    //   on name collisions).
    let mut all_classes = Vec::new();
    for module in &modules {
        all_classes.extend(discover_top_level_classes(module));
    }
    let all_schemas = discover_schemas(&all_classes);
    let combined_registry = build_combined_registry(&modules);

    // Phase 4: analyze each file against the combined view.
    let mut file_results = Vec::with_capacity(files.len());
    for (i, (path, source)) in files.iter().enumerate() {
        let line_index = LineIndex::from_source_text(source);
        let result = match &parsed[i] {
            Ok(p) => analyze_module(
                p.syntax(),
                source,
                &line_index,
                &all_schemas,
                &combined_registry,
            ),
            Err(err) => {
                let d = Diagnostic::at_range(
                    Severity::Error,
                    "D0001",
                    err.error.to_string(),
                    err.location,
                    source,
                    &line_index,
                );
                CheckResult {
                    diagnostics: vec![d],
                    body: String::new(),
                    schema_count: 0,
                    typed_function_count: 0,
                    parse_error: true,
                }
            }
        };
        file_results.push(ProjectFileResult {
            path: path.clone(),
            result,
        });
    }

    ProjectCheckResult {
        files: file_results,
    }
}

/// Build a combined `Registry` covering every supplied module. On
/// duplicate names (same class or constant declared in two files),
/// last-write wins.
fn build_combined_registry<'a>(modules: &[&'a ModModule]) -> Registry<'a> {
    let mut combined = Registry {
        classes: HashMap::new(),
        constants: HashMap::new(),
    };
    for module in modules {
        let local = Registry::build(module);
        combined.classes.extend(local.classes);
        combined.constants.extend(local.constants);
    }
    combined
}

/// Analyze one parsed module given the project-wide schema list and
/// registry. Used by both `check` (single file) and `check_project`
/// (per-file step). All cross-file resolution happens via the
/// `all_schemas` / `registry` arguments, which span the whole project.
fn analyze_module<'a>(
    module: &'a ModModule,
    source: &'a str,
    line_index: &LineIndex,
    all_schemas: &'a [Schema<'a>],
    registry: &'a Registry<'a>,
) -> CheckResult {
    let local_classes = discover_top_level_classes(module);
    let local_schemas = discover_schemas(&local_classes);
    let functions = discover_top_level_functions(module);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut body = String::new();

    // Schemas DECLARED in this file are rendered; their field annotations
    // are resolved against the project-wide schema list so nested-struct
    // references can span files.
    for schema in &local_schemas {
        render_schema(
            schema,
            all_schemas,
            source,
            line_index,
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
            all_schemas,
            source,
            line_index,
            &mut body,
            &mut diagnostics,
        );
        let declared_return = declared_return_schema(slots, all_schemas);
        let mut ctx = BodyContext::from_function(func, slots, all_schemas, registry);
        check_function_body(
            func,
            declared_return,
            &mut ctx,
            source,
            line_index,
            &mut diagnostics,
        );
    }

    CheckResult {
        diagnostics,
        body,
        schema_count: local_schemas.len(),
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
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0010",
                    format!("Unknown column type '{name}'. Expected one of: {COLUMN_TYPE_NAMES}.",),
                    ann_range,
                    source,
                    line_index,
                ));
            }
            FieldResolution::NotABareName => {
                writeln!(out, "          {}: {}  (unresolved)", field.name, raw_text).unwrap();
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0011",
                    format!(
                        "Column type '{raw_text}' is not a bare name. \
                         Subscripted/complex column types are not yet \
                         supported in v0.1. Use one of: {COLUMN_TYPE_NAMES}.",
                    ),
                    ann_range,
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
                    diagnostics.push(Diagnostic::at_range(
                        Severity::Error,
                        "D0020",
                        format!(
                            "Unknown schema '{name}' referenced in DataFrame[…]. \
                             Declare it as a class extending Schema.",
                        ),
                        ann_range,
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
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0021",
                    format!(
                        "DataFrame schema must be a bare name; got '{raw_text}'. \
                         Subscripted/complex schema expressions are not supported in v0.1.",
                    ),
                    ann_range,
                    source,
                    line_index,
                ));
            }
        }
    }
}
