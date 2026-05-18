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

pub mod completion;
pub mod dataframe;
pub mod diagnostics;
pub mod hover;
pub mod imports;
pub mod operations;
pub mod registry;
pub mod schema;
pub mod sql;
pub mod symbols;
pub mod transpiler;
pub mod types;
pub mod walk;

pub use completion::{CompletionItem, CompletionItemKind, completions};
pub use hover::{HoverInfo, hover};
pub use symbols::{
    DocumentSymbol, Span, SymbolKind, definition, document_symbols, prepare_rename, references,
    rename,
};
pub use transpiler::transpile;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use ruff_python_ast::{ModModule, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot, typed_slots};
pub use crate::diagnostics::CheckMode;
use crate::diagnostics::{Diagnostic, Severity};
use crate::imports::{ModulePath, find_pyproject_root, longest_common_ancestor, parse_imports};
use crate::operations::{
    BodyContext, CallResultTrace, ColumnRefTrace, LocalBindingTrace, check_function_body,
};
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

/// Run the full checker on a single source file at the default
/// [`CheckMode::Standard`].
///
/// Equivalent to calling [`check_project`] with a one-element list. Kept as
/// a convenience for callers that only have one file (and for backward
/// compatibility with the existing test suite).
pub fn check(path: &str, source: &str) -> CheckResult {
    check_with_mode(path, source, CheckMode::Standard)
}

/// Run the full checker on a single source file at `mode`.
pub fn check_with_mode(path: &str, source: &str, mode: CheckMode) -> CheckResult {
    let project = check_project_with_mode(&[(path.to_string(), source.to_string())], mode);
    project.files.into_iter().next().unwrap().result
}

/// Run the full checker over multiple source files as one project.
///
/// All declared Schemas, classes, and top-level annotated constants across
/// every supplied file are pooled into a single resolution scope. Each
/// file is then analyzed against that pooled scope, and per-file
/// diagnostics are returned in input order.
///
/// Scoping in v0.1:
/// - Schemas / typed constants are visible inside a file only if (a) the
///   file declared them itself, or (b) the file pulled them in with a
///   `from X import Y` statement. Bare `import X` is parsed but doesn't
///   make `X.Y` references resolve (qualified access is deferred).
/// - The project root is the deepest `pyproject.toml`-bearing directory
///   above the first input file; if none exists we fall back to the
///   longest common ancestor of every input file. Absolute imports
///   (`from pkg.X import Y`) are anchored at this root.
/// - Directory walking happens at the CLI layer; this entry point still
///   takes `(path, source)` pairs.
/// - Duplicate top-level names across files: not currently diagnosed.
///
/// Runs at the default [`CheckMode::Standard`]; use
/// [`check_project_with_mode`] to pick a strictness level.
pub fn check_project(files: &[(String, String)]) -> ProjectCheckResult {
    check_project_with_mode(files, CheckMode::Standard)
}

/// Run the project checker at the given [`CheckMode`]. Diagnostics
/// whose [`Diagnostic::min_mode`] is stricter than `mode` are filtered
/// out — and `mode` of [`CheckMode::Off`] drops them all.
pub fn check_project_with_mode(files: &[(String, String)], mode: CheckMode) -> ProjectCheckResult {
    let mut project = check_project_unfiltered(files);
    for file in &mut project.files {
        file.result.diagnostics.retain(|d| mode.shows(d.min_mode));
    }
    project
}

/// The checker proper — every diagnostic, unfiltered by mode.
fn check_project_unfiltered(files: &[(String, String)]) -> ProjectCheckResult {
    let ctx = ProjectContext::build(files);
    let bundles = ctx.build_bundles();
    let mut file_results = Vec::with_capacity(files.len());
    for (i, (path, source)) in files.iter().enumerate() {
        let line_index = LineIndex::from_source_text(source);
        let result = match &ctx.parsed[i] {
            Ok(p) => {
                let module = p.syntax();
                let scope = ctx.build_file_scope(i, source, &line_index, &bundles);
                let mut analysis = analyze_module(
                    module,
                    source,
                    &line_index,
                    &scope.visible_schemas,
                    &scope.combined_registry,
                );
                analysis.diagnostics.splice(0..0, scope.import_diagnostics);
                analysis
            }
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

/// Per-file local declarations. Held by-value at the call site that
/// builds them so the borrows into the AST stay tied to a stack frame
/// the caller owns — avoids the self-referential storage that would
/// come from caching bundles inside [`ProjectContext`].
pub(crate) struct FileBundle<'a> {
    pub(crate) local_classes: Vec<crate::walk::DiscoveredClass<'a>>,
    pub(crate) local_registry: Registry<'a>,
}

impl<'a> FileBundle<'a> {
    pub(crate) fn build(module: &'a ModModule) -> Self {
        Self {
            local_classes: discover_top_level_classes(module),
            local_registry: Registry::build(module),
        }
    }
}

/// Resolved per-file scope: every schema the file can see (its own
/// declarations plus everything its `from … import …` clauses pull in),
/// the combined class / constant registry, and any import-resolution
/// diagnostics produced along the way.
pub(crate) struct FileScope<'a> {
    pub(crate) visible_schemas: Vec<Schema<'a>>,
    pub(crate) combined_registry: Registry<'a>,
    pub(crate) import_diagnostics: Vec<Diagnostic>,
}

/// Project-wide context shared by [`check_project`] and the
/// project-aware LSP entry points. Parses every file up front; `build`
/// is shared so the LSP can run the same scope-resolution logic the
/// CLI does for cross-file hover / completion / definition.
///
/// Bundles are built on demand to dodge a self-referential struct
/// (parsed modules and the AST views into them would have to live in
/// the same value). For typical project sizes (a few dozen files)
/// re-walking a small handful of modules per request is cheap enough.
pub(crate) struct ProjectContext<'a> {
    pub(crate) files: &'a [(String, String)],
    pub(crate) parsed:
        Vec<Result<ruff_python_parser::Parsed<ModModule>, ruff_python_parser::ParseError>>,
    pub(crate) project_root: PathBuf,
    pub(crate) path_to_index: HashMap<PathBuf, usize>,
}

impl<'a> ProjectContext<'a> {
    pub(crate) fn build(files: &'a [(String, String)]) -> Self {
        let parsed: Vec<_> = files
            .iter()
            .map(|(_, src)| ruff_python_parser::parse_module(src))
            .collect();
        let project_root = resolve_project_root(files);
        let path_to_index: HashMap<PathBuf, usize> = files
            .iter()
            .enumerate()
            .map(|(i, (p, _))| (PathBuf::from(p), i))
            .collect();
        Self {
            files,
            parsed,
            project_root,
            path_to_index,
        }
    }

    pub(crate) fn focus_idx(&self, path: &str) -> Option<usize> {
        self.files.iter().position(|(p, _)| p == path)
    }

    pub(crate) fn focus_module(&self, idx: usize) -> Option<&ModModule> {
        self.parsed[idx].as_ref().ok().map(|p| p.syntax())
    }

    /// For the focus file, every `from .X import …` whose module path
    /// resolves to a project `.dpy` file — the source range of the
    /// module-name token paired with that file's index. Powers
    /// go-to-definition on the module name of a relative import.
    pub(crate) fn dpy_import_module_targets(&self, focus_idx: usize) -> Vec<(TextRange, usize)> {
        let Some(focus_module) = self.focus_module(focus_idx) else {
            return Vec::new();
        };
        let importing_path = PathBuf::from(&self.files[focus_idx].0);
        let mut out = Vec::new();
        for stmt in &focus_module.body {
            let Stmt::ImportFrom(import) = stmt else {
                continue;
            };
            let Some(module_ident) = import.module.as_ref() else {
                continue;
            };
            let module_path = ModulePath {
                level: import.level,
                segments: module_ident
                    .id
                    .as_str()
                    .split('.')
                    .map(String::from)
                    .collect(),
            };
            if let Some(target_path) = module_path.resolve(&importing_path, &self.project_root)
                && let Some(&target_idx) = self.path_to_index.get(&target_path)
            {
                out.push((module_ident.range, target_idx));
            }
        }
        out
    }

    /// Resolve the focus file's scope: own schemas + imports + combined
    /// registry + any import-resolution diagnostics. The caller owns
    /// the bundles list so its lifetime ties to the right stack frame.
    pub(crate) fn build_file_scope<'scope>(
        &'scope self,
        focus_idx: usize,
        source: &str,
        line_index: &LineIndex,
        bundles: &'scope [Option<FileBundle<'scope>>],
    ) -> FileScope<'scope> {
        let mut visible_schemas: Vec<Schema<'scope>> = bundles[focus_idx]
            .as_ref()
            .map(|b| discover_schemas(&b.local_classes))
            .unwrap_or_default();
        // Schemas declared in the focus file itself — tag them so
        // go-to-definition reports the right file.
        for schema in &mut visible_schemas {
            schema.file_index = focus_idx;
        }
        let mut combined_registry: Registry<'scope> = bundles[focus_idx]
            .as_ref()
            .map(|b| b.local_registry.clone())
            .unwrap_or_else(|| Registry {
                classes: HashMap::new(),
                constants: HashMap::new(),
                class_constants: HashMap::new(),
                functions: HashMap::new(),
                udfs: HashMap::new(),
            });
        let mut import_diagnostics: Vec<Diagnostic> = Vec::new();

        let Some(focus_module) = self.focus_module(focus_idx) else {
            return FileScope {
                visible_schemas,
                combined_registry,
                import_diagnostics,
            };
        };

        let importing_path = PathBuf::from(&self.files[focus_idx].0);
        for imp in parse_imports(focus_module) {
            let Some(target_path) = imp.module.resolve(&importing_path, &self.project_root) else {
                import_diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0070",
                    format!(
                        "Cannot resolve module path '{}' — too many leading dots.",
                        format_module_path(&imp.module),
                    ),
                    imp.range,
                    source,
                    line_index,
                ));
                continue;
            };
            let Some(&target_idx) = self.path_to_index.get(&target_path) else {
                // Module path doesn't match any `.dpy` file in the
                // project — treat it as an external Python import
                // (`from pyspark.sql.functions import col`, etc.) and
                // skip silently. The imported names become opaque to
                // dathon; downstream uses fall through to whatever
                // built-in handling we have (e.g. `col(...)` is still
                // recognized by `col_reference` regardless of where
                // it came from). The companion Python LSP handles
                // external-import validation, not dathon.
                continue;
            };
            let Some(target_bundle) = bundles[target_idx].as_ref() else {
                continue;
            };
            let target_schemas = discover_schemas(&target_bundle.local_classes);
            let found_schema = target_schemas
                .iter()
                .find(|s| s.declared_name() == imp.source_name)
                .map(|s| {
                    let alias = if imp.local_name == imp.source_name {
                        None
                    } else {
                        Some(imp.local_name)
                    };
                    Schema {
                        class: s.class,
                        alias,
                        // The schema lives in the imported module's file;
                        // go-to-definition must point there, not here.
                        file_index: target_idx,
                    }
                });
            let found_class = target_bundle.local_registry.classes.get(imp.source_name);
            let found_constant = target_bundle.local_registry.constants.get(imp.source_name);
            let mut imported_anything = false;
            if let Some(schema) = found_schema {
                visible_schemas.push(schema);
                imported_anything = true;
            }
            if let Some(class) = found_class {
                combined_registry
                    .classes
                    .insert(imp.local_name, class.clone());
                for ((cls, cname), info) in &target_bundle.local_registry.class_constants {
                    if *cls == imp.source_name {
                        combined_registry
                            .class_constants
                            .insert((imp.local_name, cname), info.clone());
                    }
                }
                imported_anything = true;
            }
            if let Some(constant) = found_constant {
                combined_registry
                    .constants
                    .insert(imp.local_name, constant.clone());
                imported_anything = true;
            }
            if !imported_anything {
                import_diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0071",
                    format!(
                        "Name '{}' is not exported by module '{}'.",
                        imp.source_name,
                        format_module_path(&imp.module),
                    ),
                    imp.range,
                    source,
                    line_index,
                ));
            }
        }

        FileScope {
            visible_schemas,
            combined_registry,
            import_diagnostics,
        }
    }

    /// Build a `FileBundle` for every parsed file. The returned slice
    /// is what the caller hands to [`build_file_scope`].
    pub(crate) fn build_bundles(&'a self) -> Vec<Option<FileBundle<'a>>> {
        self.parsed
            .iter()
            .map(|pr| pr.as_ref().ok().map(|p| FileBundle::build(p.syntax())))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Project-aware LSP entry points
// ---------------------------------------------------------------------------

/// Hover at `(line, column)` on `focus_path`, treating `files` as the
/// whole project. The focus file's imports are resolved across all
/// other files in the list, so `DataFrame[X]` and other Schema
/// references show information about `X` even when `X` lives in a
/// sibling file. Returns `None` if the focus file isn't in `files`,
/// fails to parse, or the cursor isn't on a recognized symbol.
pub fn hover_in_project(
    files: &[(String, String)],
    focus_path: &str,
    line: usize,
    column: usize,
) -> Option<HoverInfo> {
    let ctx = ProjectContext::build(files);
    let bundles = ctx.build_bundles();
    let focus_idx = ctx.focus_idx(focus_path)?;
    let focus_module = ctx.focus_module(focus_idx)?;
    let focus_source = &files[focus_idx].1;
    let line_index = LineIndex::from_source_text(focus_source);
    let scope = ctx.build_file_scope(focus_idx, focus_source, &line_index, &bundles);
    crate::hover::hover_with_scope(
        focus_module,
        focus_source,
        &line_index,
        line,
        column,
        &scope.visible_schemas,
        &scope.combined_registry,
    )
}

/// Completion at `(line, column)` on `focus_path`, project-aware
/// (same shape as [`hover_in_project`]). Schema-name suggestions
/// inside `DataFrame[…]` include every imported schema; column
/// completions resolve through cross-file constants.
pub fn completions_in_project(
    files: &[(String, String)],
    focus_path: &str,
    line: usize,
    column: usize,
) -> Vec<CompletionItem> {
    let ctx = ProjectContext::build(files);
    let bundles = ctx.build_bundles();
    let Some(focus_idx) = ctx.focus_idx(focus_path) else {
        return Vec::new();
    };
    let Some(focus_module) = ctx.focus_module(focus_idx) else {
        return Vec::new();
    };
    let focus_source = &files[focus_idx].1;
    let line_index = LineIndex::from_source_text(focus_source);
    let scope = ctx.build_file_scope(focus_idx, focus_source, &line_index, &bundles);
    crate::completion::completions_with_scope(
        focus_module,
        focus_source,
        &line_index,
        line,
        column,
        &scope.visible_schemas,
        &scope.combined_registry,
    )
}

/// Go-to-definition at `(line, column)` on `focus_path`,
/// project-aware. `DataFrame[X]` and other Schema references now
/// resolve to the class declaration even when it lives in a sibling
/// file — the returned `Span` still anchors against `focus_path`
/// today, since the LSP layer can match the span to the correct URI.
/// Cross-file location reporting is a follow-up.
/// Project-aware go-to-definition. Returns `(path, span)` — the file
/// the definition lives in and its position there. The target file may
/// differ from `focus_path` when the cursor points at something
/// declared in an imported module (a `col("…")` reference, a
/// `DataFrame[X]` schema, …).
pub fn definition_in_project(
    files: &[(String, String)],
    focus_path: &str,
    line: usize,
    column: usize,
) -> Option<(String, Span)> {
    let ctx = ProjectContext::build(files);
    let bundles = ctx.build_bundles();
    let focus_idx = ctx.focus_idx(focus_path)?;
    let focus_module = ctx.focus_module(focus_idx)?;
    let focus_source = &files[focus_idx].1;
    let line_index = LineIndex::from_source_text(focus_source);
    let scope = ctx.build_file_scope(focus_idx, focus_source, &line_index, &bundles);
    let import_modules = ctx.dpy_import_module_targets(focus_idx);
    let (target_idx, range) = crate::symbols::definition_with_scope(
        focus_module,
        focus_source,
        &line_index,
        line,
        column,
        focus_idx,
        &import_modules,
        &scope.visible_schemas,
        &scope.combined_registry,
    )?;
    // The range is a byte range in `files[target_idx]` — convert it to
    // line/column against *that* file's text, not the focus file's.
    let (target_path, target_source) = &files[target_idx];
    let target_line_index = LineIndex::from_source_text(target_source);
    let span = crate::symbols::span_from_range(range, target_source, &target_line_index);
    Some((target_path.clone(), span))
}

/// Pick the project root: deepest `pyproject.toml`-bearing dir above the
/// first input file, falling back to the longest common ancestor of the
/// inputs and then to the current dir.
fn resolve_project_root(files: &[(String, String)]) -> PathBuf {
    if let Some((first, _)) = files.first() {
        if let Some(root) = find_pyproject_root(&PathBuf::from(first)) {
            return root;
        }
    }
    longest_common_ancestor(files.iter().map(|(p, _)| p)).unwrap_or_else(|| PathBuf::from("."))
}

fn format_module_path(module: &crate::imports::ModulePath) -> String {
    let dots = ".".repeat(module.level as usize);
    if module.segments.is_empty() {
        dots
    } else {
        format!("{dots}{}", module.segments.join("."))
    }
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

/// Run body analysis on every typed function and return the captured
/// column-reference traces. Diagnostics are discarded — this entry
/// point exists for the LSP layer's hover / go-to-definition on
/// `col("foo")` references.
///
/// `functions` must be the result of [`discover_top_level_functions`]
/// on `module` — we take it as a parameter so the caller owns the Vec
/// and the borrow-checker can see the traces' `&'a str` names borrow
/// from the same scope as the function discovery, not from a Vec local
/// to this helper.
pub(crate) fn collect_module_column_refs<'a>(
    functions: &'a [DiscoveredFunction<'a>],
    source: &'a str,
    line_index: &LineIndex,
    schemas: &'a [Schema<'a>],
    registry: &'a Registry<'a>,
) -> Vec<ColumnRefTrace<'a>> {
    collect_module_traces(functions, source, line_index, schemas, registry).column_refs
}

/// Internal: run body analysis once and return both trace flavors. The
/// LSP-facing helpers above wrap this so callers can pick the slice they
/// need without re-running analysis twice. Crate-visible so the hover
/// entry point can grab both lists in one pass.
pub(crate) fn collect_module_traces<'a>(
    functions: &'a [DiscoveredFunction<'a>],
    source: &'a str,
    line_index: &LineIndex,
    schemas: &'a [Schema<'a>],
    registry: &'a Registry<'a>,
) -> ModuleTraces<'a> {
    let mut traces = ModuleTraces::default();
    for func in functions {
        let slots = typed_slots(func);
        if slots.is_empty() {
            continue;
        }
        let mut ctx = BodyContext::from_function(func, &slots, schemas, registry);
        let declared_return = declared_return_schema(&slots, schemas);
        let mut throwaway: Vec<Diagnostic> = Vec::new();
        check_function_body(
            func,
            declared_return,
            &mut ctx,
            source,
            line_index,
            &mut throwaway,
        );
        traces.column_refs.extend(ctx.take_column_refs());
        traces.local_bindings.extend(ctx.take_local_bindings());
        traces.call_results.extend(ctx.take_call_results());
    }
    traces
}

/// The three flavors of trace body analysis collects for the LSP layer:
/// `col(...)`-style column references, local `x = …` bindings, and
/// method-call result schemas.
#[derive(Default)]
pub(crate) struct ModuleTraces<'a> {
    pub column_refs: Vec<ColumnRefTrace<'a>>,
    pub local_bindings: Vec<LocalBindingTrace<'a>>,
    pub call_results: Vec<CallResultTrace<'a>>,
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
