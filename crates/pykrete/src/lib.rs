//! pykrete — a static schema checker for PySpark code.
//!
//! This is the library crate. The CLI shell at `src/main.rs` is a thin
//! wrapper around [`check`].
//!
//! The high-level entry point is [`check`], which runs the whole analysis
//! pipeline on one source file's text and returns a [`CheckResult`].
//! Integration tests under `crates/pykrete/tests/` call [`check`] directly
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

pub mod alias_adjudicate;
pub mod alias_report;
#[doc(hidden)]
pub mod build_helpers;
pub mod compare_to;
pub mod completion;
pub mod config;
pub mod dataframe;
pub mod diagnostics;
pub mod dialect_signals;
pub mod hover;
pub mod imports;
pub mod operations;
pub mod registry;
pub mod schema;
pub mod sql;
pub mod symbols;
pub mod temp_views;
pub mod transpiler;
pub mod types;
pub mod walk;

pub use alias_adjudicate::{
    adjudicate as adjudicate_alias_sites, ambiguous_site_offsets, has_ambiguous_in_file,
};
pub use alias_report::{
    AckFilter, AdjudicatedDialect, AliasSite, MigrationStatus, collect_alias_sites,
    render_alias_report_json, render_deprecation_report_json,
};
pub use compare_to::{
    Snapshot, SnapshotProvenance, SnapshotSite, added_count, parse_snapshot, render_diff,
    snapshot_from_current_envelope,
};
pub use completion::{CompletionItem, CompletionItemKind, completions};
pub use config::Config;
pub use dialect_signals::{
    PANDAS_INHERITED_ARM_METHODS_GENERATED, PANDAS_INHERITED_ARMS, PANDAS_INHERITED_PROPERTIES,
    PANDAS_ONLY_SIGNALS, SPARK_DISCRIMINATOR_PROPERTIES, SPARK_DISCRIMINATORS,
};
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

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot, render_annotation, typed_slots};
pub use crate::diagnostics::CheckMode;
use crate::diagnostics::{Diagnostic, Severity};
use crate::imports::{ModulePath, find_pyproject_root, longest_common_ancestor, parse_imports};
use crate::operations::{
    BodyContext, CallResultTrace, ColumnRefTrace, DeclaredReturn, LocalBindingTrace,
    check_function_body,
};
use crate::registry::Registry;
use crate::schema::{
    FieldResolution, Schema, SchemaView, derived_arg_names, derived_schema_errors,
    discover_schemas, resolve_derived_schema, schema_base_chain,
};
use crate::temp_views::TempViewRegistry;
use crate::types::COLUMN_TYPE_NAMES;
use crate::walk::{
    DiscoveredClass, DiscoveredFunction, discover_top_level_classes, discover_top_level_functions,
};

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
/// - Duplicate schema names across files surface as `D0072
///   duplicateSchemaName` warnings on every duplicate past the first
///   (sorted alphabetically by file path). Other top-level name
///   duplicates (functions, constants) are not yet diagnosed.
///
/// Runs at the default [`CheckMode::Standard`]; use
/// [`check_project_with_mode`] to pick a strictness level.
pub fn check_project(files: &[(String, String)]) -> ProjectCheckResult {
    check_project_with_mode(files, CheckMode::Standard)
}

/// Run the project checker at the given [`CheckMode`]. Diagnostics
/// whose [`Diagnostic::min_mode`] is stricter than `mode` are filtered
/// out — and `mode` of [`CheckMode::Off`] drops them all.
///
/// v1.6 PR-M3: D0090 `deprecatedDataFrameAlias` is emitted as a warning
/// by the renderer and ann-assign driver. Under `CheckMode::Strict`,
/// escalate it to an error here — the migration story for v2.0 is that
/// strict-mode projects fail the build until `pykrete migrate` is run.
/// Non-strict modes keep the warning unchanged, preserving the v1.5
/// behavior for users who haven't opted into strict.
///
/// v1.9 PR-D1: same treatment for `D0091 crossDialectMethodMismatch`.
/// v1.8 shipped D0091 as warning-only; v1.9 escalates under strict (spec
/// §3.1.1, mirroring the v1.6 PR-M3 D0090 precedent). The cascade hits
/// any `probes_negative/` fixture exercising cross-dialect method
/// patterns — flagged for the v1.9.0 catalog pin bump.
pub fn check_project_with_mode(files: &[(String, String)], mode: CheckMode) -> ProjectCheckResult {
    let mut project = check_project_unfiltered(files);
    for file in &mut project.files {
        file.result.diagnostics.retain(|d| mode.shows(d.min_mode));
        if mode == CheckMode::Strict {
            for d in &mut file.result.diagnostics {
                if d.code == "D0090" || d.code == "D0091" {
                    d.severity = Severity::Error;
                }
            }
        }
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

    // Project-wide D0072: warn when the same schema name is declared in
    // more than one file. Fires once per duplicate, attached to the
    // alphabetically-later file so the user sees the warning in a
    // deterministic spot.
    emit_duplicate_schema_diagnostics(files, &bundles, &mut file_results);

    ProjectCheckResult {
        files: file_results,
    }
}

/// Post-process per-file results to add D0072 `duplicateSchemaName`
/// warnings. Iterates every (schema_name, [(file_idx, range)]) group; for
/// each group with more than one declaration, picks the alphabetically-
/// later file path as the canonical "duplicate" site and emits a warning
/// at the class declaration range there, naming every file that declared
/// the same schema for context.
fn emit_duplicate_schema_diagnostics(
    files: &[(String, String)],
    bundles: &[Option<FileBundle<'_>>],
    file_results: &mut [ProjectFileResult],
) {
    use crate::diagnostics::CheckMode;
    use std::collections::BTreeMap;

    // schema name -> sorted list of (file_path, file_idx, decl_range).
    // BTreeMap to get deterministic output order regardless of HashMap
    // iteration order; the inner Vec is sorted by file path below.
    let mut sites: BTreeMap<&str, Vec<(&str, usize, TextRange)>> = BTreeMap::new();
    for (i, bundle) in bundles.iter().enumerate() {
        let Some(bundle) = bundle else { continue };
        // Only Schema-derived classes count — non-Schema classes can be
        // duplicated freely (a `class Helper` in two files is fine; only
        // the user's data-model schemas are project-wide singletons).
        for schema in discover_schemas(&bundle.local_classes) {
            sites.entry(schema.name()).or_default().push((
                files[i].0.as_str(),
                i,
                schema.class.def.range,
            ));
        }
    }
    for (name, decls) in sites {
        // Collapse to one entry per file — a class redeclared inside a
        // single file is a separate concern (and would be reported by a
        // future intra-file diagnostic), not D0072. Picking the first
        // declaration's range per file keeps the warning on the topmost
        // site in that file's source.
        let mut by_file: BTreeMap<&str, (usize, TextRange)> = BTreeMap::new();
        for (path, idx, range) in decls {
            by_file.entry(path).or_insert((idx, range));
        }
        if by_file.len() < 2 {
            continue;
        }
        // BTreeMap already iterates in sorted-path order, so the first
        // entry is alphabetically earliest and acts as the "original";
        // every later file gets a D0072 warning. With three or more
        // declarations, each file past the first gets its own diagnostic.
        let mut iter = by_file.into_iter();
        let (first_path, _) = iter.next().expect("at least 2 entries");
        for (dup_path, (dup_idx, dup_range)) in iter {
            let dup_source = &files[dup_idx].1;
            let dup_line_index = LineIndex::from_source_text(dup_source);
            let message = format!(
                "Schema '{name}' is declared in multiple files: {first_path}, {dup_path}.",
            );
            let diag = Diagnostic::at_range(
                Severity::Warning,
                "D0072",
                message,
                dup_range,
                dup_source,
                &dup_line_index,
            )
            .with_min_mode(CheckMode::Basic);
            // The result Vec is sized to `files.len()` in input order, so
            // indexing by the file's input index lands the warning in the
            // right per-file bucket.
            if let Some(target) = file_results.get_mut(dup_idx) {
                target.result.diagnostics.push(diag);
            }
        }
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
    /// resolves to a project `.pyk` file — the source range of the
    /// module-name token paired with that file's index. Powers
    /// go-to-definition on the module name of a relative import.
    pub(crate) fn pyk_import_module_targets(&self, focus_idx: usize) -> Vec<(TextRange, usize)> {
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
                // Module path doesn't match any `.pyk` file in the
                // project — treat it as an external Python import
                // (`from pyspark.sql.functions import col`, etc.) and
                // skip silently. The imported names become opaque to
                // pykrete; downstream uses fall through to whatever
                // built-in handling we have (e.g. `col(...)` is still
                // recognized by `col_reference` regardless of where
                // it came from). The companion Python LSP handles
                // external-import validation, not pykrete.
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
                    // Carry the resolved ancestor chain so an imported
                    // schema's inherited columns still resolve. The
                    // bases were resolved against the target file's
                    // classes, where this schema and its bases live.
                    // The schema lives in the imported module's file;
                    // go-to-definition must point there, not here.
                    Schema::new(s.class, s.bases.clone(), alias, target_idx)
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

        // Cross-file schema inheritance. The local-only `discover_schemas`
        // above ran before imports were resolved, so a local
        // `class Premium(Orders)` extending a schema *imported* into this
        // file was not recognized. Now that the imported schemas are in
        // `visible_schemas`, promote any local class that extends one —
        // to any depth (a promoted schema can itself be a base).
        if let Some(bundle) = bundles[focus_idx].as_ref() {
            loop {
                let promote: Vec<&DiscoveredClass> = bundle
                    .local_classes
                    .iter()
                    .filter(|class| {
                        !visible_schemas
                            .iter()
                            .any(|s| std::ptr::eq(s.class, *class))
                            && class
                                .base_names()
                                .iter()
                                .any(|base| visible_schemas.iter().any(|s| s.name() == *base))
                    })
                    .collect();
                if promote.is_empty() {
                    break;
                }
                for class in promote {
                    let bases = schema_base_chain(class, &bundle.local_classes, &visible_schemas);
                    visible_schemas.push(Schema::new(class, bases, None, focus_idx));
                }
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
    let import_modules = ctx.pyk_import_module_targets(focus_idx);
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
    if let Some((first, _)) = files.first()
        && let Some(root) = find_pyproject_root(&PathBuf::from(first))
    {
        return root;
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

    // One tempView registry per file, shared across every typed
    // function in the file — so a view created in one function is
    // visible to `spark.sql(...)` in another. Cross-file resolution is
    // deliberately not supported: each file gets a fresh registry.
    let temp_views = TempViewRegistry::new();

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
        let mut ctx = BodyContext::from_function(func, slots, all_schemas, registry)
            .with_temp_views(&temp_views);
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

/// Resolve a function's `-> DataFrame[…]` return slot to a schema view:
/// a bare name to the declared schema, a `Pick`/`Omit` expression to its
/// derived view. `None` when there's no return slot, or it doesn't
/// resolve (the unresolved case is reported by the signature renderer).
fn declared_return_schema<'a>(
    slots: &[TypedSlot<'a>],
    schemas: &'a [Schema<'a>],
) -> Option<DeclaredReturn<'a>> {
    for slot in slots {
        if matches!(slot.label, SlotLabel::Return) {
            let view = match slot.kind {
                DataFrameAnnotation::Typed(name) => schemas
                    .iter()
                    .find(|s| s.name() == name)
                    .map(SchemaView::Declared),
                DataFrameAnnotation::Derived(expr) => resolve_derived_schema(expr, schemas),
                _ => None,
            };
            return view.map(|view| DeclaredReturn {
                view,
                dialect: slot.dialect,
            });
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
    // The LSP-facing trace collector doesn't need a tempView registry —
    // hover / go-to-definition fire on individual column refs, and the
    // result-of-`spark.sql` schema attribution lands in `call_results`
    // regardless of view registration. Skipping the registry here also
    // dodges a lifetime tangle: `ModuleTraces<'a>` borrows `schemas`,
    // and threading a stack-local `TempViewRegistry` through `ctx`
    // would over-constrain its borrow.
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
                        "Column type '{raw_text}' is not a recognized pykrete type. \
                         Use a bare atomic ({COLUMN_TYPE_NAMES}), a declared Schema, \
                         or an `Array[…]` / `Map[…, …]` collection.",
                    ),
                    ann_range,
                    source,
                    line_index,
                ));
            }
            FieldResolution::InvalidEnum(err) => {
                writeln!(out, "          {}: {}  (unresolved)", field.name, raw_text).unwrap();
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0011",
                    crate::schema::enum_parse_error_message(&err),
                    ann_range,
                    source,
                    line_index,
                ));
            }
        }
    }
}

// `spark_frame_rewrite` + `format_d0090_message` now live in
// `dataframe.rs` next to the recognition logic — round-2 M2.
pub(crate) use crate::dataframe::format_d0090_message;

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

    // PEP-695 type parameters declared by this function — `def f[T, U]`.
    // A `DataFrame[T]` annotation whose inner name is one of these is
    // generic, not a schema reference, so the D0020 "unknown schema"
    // check must skip it. The generic-inference path in operations.rs
    // substitutes T against the call-site argument.
    let func_type_params: Vec<&str> = func
        .def
        .type_params
        .as_deref()
        .map(|tp| {
            tp.type_params
                .iter()
                .filter_map(|p| match p {
                    ruff_python_ast::TypeParam::TypeVar(tv) => Some(tv.name.id.as_str()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    for slot in slots {
        let ann_range = slot.annotation.range();
        let raw_text = &source[ann_range];
        let prefix = match slot.label {
            SlotLabel::Param(name) => format!("          {name}: "),
            SlotLabel::Return => "          -> ".to_string(),
        };

        // v1.3 pandas spec §6: `DataFrame[X]` is a deprecated alias for
        // `SparkFrame[X]`. Fire D0090 once per slot; the message echoes
        // the source text so users see what they wrote (Q7-resolved).
        // Suggestion fills the SparkFrame[X] canonical form for quick-fix.
        // Wording is owned by `format_d0090_message` so both this site
        // and the ann-assign emitter in `operations::driver` stay locked.
        if slot.is_deprecated_alias {
            let (message, suggestion) = format_d0090_message(raw_text);
            diagnostics.push(
                Diagnostic::at_range(
                    Severity::Warning,
                    "D0090",
                    message,
                    ann_range,
                    source,
                    line_index,
                )
                .with_suggestion(Some(suggestion)),
            );
        }

        // Render the slot's frame surface through `render_annotation` so
        // every rendering site agrees on the dialect prefix —
        // `SparkFrame` / `PandasFrame` / the deprecated `DataFrame`
        // alias — and the CLI text output matches hover/symbols.
        let rendered = render_annotation(&slot.kind, slot.dialect, slot.is_deprecated_alias);
        match slot.kind {
            DataFrameAnnotation::Typed(name) => {
                if schemas.iter().any(|s| s.name() == name) {
                    writeln!(out, "{prefix}{rendered}").unwrap();
                } else if func_type_params.contains(&name) {
                    // `<Frame>[T]` where T is a TypeVar on this
                    // function — generic, resolved at call sites.
                    writeln!(out, "{prefix}{rendered}  (generic)").unwrap();
                } else {
                    writeln!(out, "{prefix}{rendered}  (unresolved)").unwrap();
                    diagnostics.push(Diagnostic::at_range(
                        Severity::Error,
                        "D0020",
                        format!(
                            "Unknown schema '{name}' referenced in {rendered}. \
                             Declare it as a class extending Schema.",
                        ),
                        ann_range,
                        source,
                        line_index,
                    ));
                }
            }
            DataFrameAnnotation::Derived(expr) => {
                // `Merge[A, B]` / `Pick[T, …]` with TypeVar args — generic,
                // resolved at call sites. Skip both the resolution attempt
                // and the error reporter (which would otherwise flag the
                // TypeVar names as "unknown schema"). The check below
                // matches when every bare-name arg is either a known
                // TypeVar or a known schema AND at least one is a TypeVar
                // (a pure all-schemas form like `Merge[Orders, Refunds]`
                // resolves now, not at call sites).
                let arg_names = derived_arg_names(expr);
                let is_generic_derived =
                    arg_names.iter().all(|n| {
                        func_type_params.contains(n) || schemas.iter().any(|s| s.name() == *n)
                    }) && arg_names.iter().any(|n| func_type_params.contains(n));
                if is_generic_derived {
                    writeln!(out, "{prefix}{raw_text}  (generic)").unwrap();
                    continue;
                }
                if resolve_derived_schema(expr, schemas).is_some() {
                    writeln!(out, "{prefix}{raw_text}").unwrap();
                } else {
                    writeln!(out, "{prefix}{raw_text}  (unresolved)").unwrap();
                }
                for (code, message, range) in derived_schema_errors(expr, schemas, source) {
                    diagnostics.push(Diagnostic::at_range(
                        Severity::Error,
                        code,
                        message,
                        range,
                        source,
                        line_index,
                    ));
                }
            }
            DataFrameAnnotation::Untyped => {
                // Architecture-audit nit #11: untyped path must carry
                // the slot's actual dialect — bare `PandasFrame` reads
                // as "PandasFrame (untyped)", not the prior hardcoded
                // "DataFrame (untyped)".
                writeln!(out, "{prefix}{rendered}  (untyped)").unwrap();
            }
            DataFrameAnnotation::NonBareName => {
                // `raw_text` already carries the user-typed dialect
                // prefix verbatim — preserve it so the user sees what
                // they wrote (e.g. `DataFrame[list[str]]`, not the
                // lossy `DataFrame[?]` from `render_annotation`).
                writeln!(out, "{prefix}{raw_text}  (unresolved)").unwrap();
                let frame_name = render_annotation(
                    &DataFrameAnnotation::Untyped,
                    slot.dialect,
                    slot.is_deprecated_alias,
                );
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0021",
                    format!(
                        "{frame_name} schema must be a bare name; got '{raw_text}'. \
                         Subscripted/complex schema expressions are not supported.",
                    ),
                    ann_range,
                    source,
                    line_index,
                ));
            }
        }
    }
}
