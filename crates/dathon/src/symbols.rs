//! Document-symbol outline + position-aware definition lookup.
//!
//! Two LSP-facing entry points share this module because they walk the
//! same top-level declarations:
//!
//! - [`document_symbols`] returns the outline of every top-level class
//!   and function in the source, with schema fields nested under their
//!   Schema class. Used by `textDocument/documentSymbol` (the outline
//!   view in VS Code's editor breadcrumb / file explorer panel).
//! - [`definition`] resolves the symbol at a `(line, column)` cursor to
//!   the source range of its declaration. Used by
//!   `textDocument/definition`. The project-aware path
//!   ([`crate::definition_in_project`]) resolves cross-file: a column
//!   or schema reference jumps to the imported module that declares it.
//!
//! Span coordinates are 1-indexed `(line, column)` pairs to match the
//! convention used by [`crate::hover`] and [`crate::diagnostics`]. The
//! LSP layer converts these to LSP's 0-indexed positions.

use ruff_python_ast::Expr;
use ruff_source_file::{LineIndex, OneIndexed};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot, typed_slots};
use crate::operations::ColumnRefTrace;
use crate::registry::Registry;
use crate::schema::{Schema, SchemaView, discover_schemas};
use crate::walk::{
    DiscoveredClass, DiscoveredFunction, discover_top_level_classes, discover_top_level_functions,
};

/// A 1-indexed source span. `start_line == end_line` for single-line
/// spans, which is the common case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

/// Kind of a symbol returned in the outline. We deliberately use a
/// small dathon-local enum rather than re-exporting `lsp_types`'s much
/// larger one — the LSP layer maps these to LSP kinds at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// Top-level `class` definition.
    Class,
    /// Schema field — an annotated assignment inside a Schema class
    /// body. Only emitted as a child of a `Class` symbol when the class
    /// is a Schema.
    Field,
    /// Top-level `def` definition.
    Function,
}

#[derive(Debug)]
pub struct DocumentSymbol {
    pub kind: SymbolKind,
    pub name: String,
    /// One-line summary shown next to the name in the outline. For
    /// schema fields this is the column type / nested-schema name; for
    /// typed functions this is the signature.
    pub detail: Option<String>,
    /// Full extent of the symbol (e.g. the entire `class Foo: ...` block).
    pub range: Span,
    /// Just the name token's extent — what the editor highlights when
    /// the symbol is focused in the outline.
    pub selection_range: Span,
    pub children: Vec<DocumentSymbol>,
}

/// Walk the top-level classes and functions of `source` and return an
/// outline structure for an LSP `textDocument/documentSymbol` response.
///
/// Returns an empty vector if the source fails to parse — the LSP layer
/// is expected to call this often, and a transient syntax error in the
/// middle of an edit shouldn't strip every existing outline entry.
pub fn document_symbols(source: &str) -> Vec<DocumentSymbol> {
    let Ok(parsed) = ruff_python_parser::parse_module(source) else {
        return Vec::new();
    };
    let module = parsed.syntax();
    let line_index = LineIndex::from_source_text(source);

    let classes = discover_top_level_classes(module);
    let schemas = discover_schemas(&classes);
    let functions = discover_top_level_functions(module);

    let mut out = Vec::with_capacity(classes.len() + functions.len());
    for class in &classes {
        out.push(class_symbol(class, &schemas, source, &line_index));
    }
    for func in &functions {
        out.push(function_symbol(func, source, &line_index));
    }
    out
}

fn class_symbol(
    class: &DiscoveredClass<'_>,
    schemas: &[Schema<'_>],
    source: &str,
    line_index: &LineIndex,
) -> DocumentSymbol {
    let is_schema = schemas.iter().any(|s| s.name() == class.name());
    let children = if is_schema {
        let schema = schemas
            .iter()
            .find(|s| s.name() == class.name())
            .expect("just checked");
        schema
            .fields()
            .iter()
            .map(|f| {
                let ann_range = f.annotation.range();
                let detail = render_field_detail(f, schemas);
                let name_range = field_name_range(class, f.name).unwrap_or(ann_range);
                DocumentSymbol {
                    kind: SymbolKind::Field,
                    name: f.name.to_string(),
                    detail: Some(detail),
                    range: span_from_range(
                        TextRange::new(name_range.start(), ann_range.end()),
                        source,
                        line_index,
                    ),
                    selection_range: span_from_range(name_range, source, line_index),
                    children: Vec::new(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    DocumentSymbol {
        kind: SymbolKind::Class,
        name: class.name().to_string(),
        detail: if is_schema {
            Some("Schema".to_string())
        } else {
            None
        },
        range: span_from_range(class.def.range, source, line_index),
        selection_range: span_from_range(class.def.name.range, source, line_index),
        children,
    }
}

/// Source range of an annotated-assignment target name inside a class
/// body. None if the field's `target` isn't a simple name.
fn field_name_range(class: &DiscoveredClass<'_>, name: &str) -> Option<TextRange> {
    for stmt in &class.def.body {
        let ann = stmt.as_ann_assign_stmt()?;
        let target = ann.target.as_name_expr()?;
        if target.id.as_str() == name {
            return Some(target.range);
        }
    }
    None
}

fn function_symbol(
    func: &DiscoveredFunction<'_>,
    source: &str,
    line_index: &LineIndex,
) -> DocumentSymbol {
    let slots = typed_slots(func);
    let detail = if slots.is_empty() {
        None
    } else {
        Some(render_function_signature(func.name(), &slots))
    };
    DocumentSymbol {
        kind: SymbolKind::Function,
        name: func.name().to_string(),
        detail,
        range: span_from_range(func.def.range, source, line_index),
        selection_range: span_from_range(func.def.name.range, source, line_index),
        children: Vec::new(),
    }
}

fn render_field_detail(f: &crate::schema::SchemaField<'_>, schemas: &[Schema<'_>]) -> String {
    use crate::schema::FieldResolution;
    match f.resolve(schemas) {
        FieldResolution::Resolved(ct) => ct.as_str().to_string(),
        FieldResolution::ResolvedNested(nested) => format!("{} (nested)", nested.name()),
        FieldResolution::UnknownType { name } => format!("{name} (unresolved)"),
        FieldResolution::NotABareName => "(unresolved)".to_string(),
    }
}

fn render_function_signature(name: &str, slots: &[TypedSlot<'_>]) -> String {
    let params: Vec<String> = slots
        .iter()
        .filter_map(|s| match s.label {
            SlotLabel::Param(p) => Some(format!("{p}: {}", render_annotation(&s.kind))),
            SlotLabel::Return => None,
        })
        .collect();
    let ret = slots
        .iter()
        .find(|s| matches!(s.label, SlotLabel::Return))
        .map(|s| format!(" -> {}", render_annotation(&s.kind)))
        .unwrap_or_default();
    format!("{name}({}){ret}", params.join(", "))
}

fn render_annotation(kind: &DataFrameAnnotation<'_>) -> String {
    match kind {
        DataFrameAnnotation::Typed(name) => format!("DataFrame[{name}]"),
        DataFrameAnnotation::Untyped => "DataFrame".to_string(),
        DataFrameAnnotation::NonBareName => "DataFrame[?]".to_string(),
    }
}

// ---------------------------------------------------------------------------
// definition
// ---------------------------------------------------------------------------

/// Resolve the symbol at `(line, column)` to the source range of its
/// declaration. Both line and column are 1-indexed.
///
/// Returns `None` if the cursor isn't on a recognized reference (or if
/// the source fails to parse). v0.1 covers four cases:
///
/// 1. Cursor inside a `DataFrame[X]` subscript's `X` → jump to `class X`.
/// 2. Cursor on a Schema field's bare-name annotation (nested struct,
///    e.g. the `Address` in `address: Address`) → jump to `class Address`.
/// 3. Cursor on a Schema class declaration name → jump to itself (LSP
///    convention: declarations are their own definitions).
/// 4. Cursor on a `col("foo")` string literal where `foo` exists on the
///    surrounding schema → jump to the field's annotation in the Schema
///    class. Requires running body analysis to know which schema each
///    `col(...)` refers to.
///
/// Function-call sites and `df.foo` attribute access are deferred.
pub fn definition(source: &str, line: usize, column: usize) -> Option<Span> {
    let parsed = ruff_python_parser::parse_module(source).ok()?;
    let module = parsed.syntax();
    let line_index = LineIndex::from_source_text(source);
    let classes = discover_top_level_classes(module);
    let schemas = discover_schemas(&classes);
    let registry = Registry::build(module);
    // Single file → the only file is index 0, no cross-file imports.
    let (_file, range) = definition_with_scope(
        module,
        source,
        &line_index,
        line,
        column,
        0,
        &[],
        &schemas,
        &registry,
    )?;
    Some(span_from_range(range, source, &line_index))
}

/// Every reference to the column under the cursor — its declaration in
/// the Schema class body plus every `col("…")`, bare-string-argument,
/// and `df.X` use of it. Empty when the cursor isn't on a column.
///
/// References are matched by column *name* across the file (the v1
/// scope) — schema-precise scoping is a follow-up.
pub fn references(source: &str, line: usize, column: usize) -> Vec<Span> {
    let Ok(parsed) = ruff_python_parser::parse_module(source) else {
        return Vec::new();
    };
    let module = parsed.syntax();
    let line_index = LineIndex::from_source_text(source);
    let classes = discover_top_level_classes(module);
    let schemas = discover_schemas(&classes);
    let registry = Registry::build(module);
    let functions = discover_top_level_functions(module);
    let Some(offset) = offset_from_line_column(&line_index, source, line, column) else {
        return Vec::new();
    };
    let traces =
        crate::collect_module_column_refs(&functions, source, &line_index, &schemas, &registry);

    let Some(target) = column_name_at(offset, &traces, &schemas) else {
        return Vec::new();
    };

    let mut ranges: Vec<TextRange> = Vec::new();
    for schema in &schemas {
        for stmt in &schema.class.def.body {
            if let Some(ann) = stmt.as_ann_assign_stmt() {
                if let Some(t) = ann.target.as_name_expr() {
                    if t.id.as_str() == target {
                        ranges.push(t.range);
                    }
                }
            }
        }
    }
    for trace in &traces {
        if trace.name == target {
            ranges.push(trace.range);
        }
    }
    ranges.sort_by_key(|r| r.start());
    ranges.dedup();
    ranges
        .into_iter()
        .map(|r| span_from_range(r, source, &line_index))
        .collect()
}

/// The column name the cursor sits on — from a column-reference trace,
/// or a Schema field declaration's target name.
fn column_name_at<'a>(
    offset: TextSize,
    traces: &[ColumnRefTrace<'a>],
    schemas: &[Schema<'a>],
) -> Option<&'a str> {
    if let Some(trace) = traces.iter().find(|t| t.range.contains_inclusive(offset)) {
        return Some(trace.name);
    }
    for schema in schemas {
        for stmt in &schema.class.def.body {
            if let Some(ann) = stmt.as_ann_assign_stmt() {
                if let Some(t) = ann.target.as_name_expr() {
                    if t.range.contains_inclusive(offset) {
                        return Some(t.id.as_str());
                    }
                }
            }
        }
    }
    None
}

/// Project-aware go-to-definition: takes the focus file's parsed
/// module plus pre-resolved visible schemas + registry. Used by the
/// LSP layer so cross-file `DataFrame[X]` / column references jump to
/// the right file even when the schema lives in a sibling module.
///
/// Returns `(file_index, range)` — the range is a byte range in the
/// file at `file_index`, which the caller converts to a [`Span`]
/// against that file's text. `focus_idx` is the file the cursor is in,
/// used to keep declaration/field lookups from matching a same-named
/// schema imported from elsewhere.
#[allow(clippy::too_many_arguments)] // focus file context + cursor + scope
pub(crate) fn definition_with_scope<'a>(
    module: &'a ruff_python_ast::ModModule,
    source: &str,
    line_index: &LineIndex,
    line: usize,
    column: usize,
    focus_idx: usize,
    dpy_import_modules: &[(TextRange, usize)],
    schemas: &[Schema<'a>],
    registry: &Registry<'a>,
) -> Option<(usize, TextRange)> {
    let offset = offset_from_line_column(line_index, source, line, column)?;

    // Cursor on the module name of a `from .X import …` whose module is
    // a project `.dpy` file → jump to the top of that file.
    for (range, target_idx) in dpy_import_modules {
        if range.contains_inclusive(offset) {
            return Some((*target_idx, TextRange::default()));
        }
    }

    let functions = discover_top_level_functions(module);

    if let Some(target) = definition_on_schema_declaration(offset, schemas, focus_idx) {
        return Some(target);
    }
    if let Some(target) =
        definition_on_schema_reference_in_function_signature(offset, &functions, schemas)
    {
        return Some(target);
    }
    if let Some(target) = definition_on_schema_reference_in_schema_field(offset, schemas, focus_idx)
    {
        return Some(target);
    }
    let traces =
        crate::collect_module_column_refs(&functions, source, line_index, schemas, registry);
    if let Some(target) = definition_on_column_ref(offset, &traces) {
        return Some(target);
    }
    None
}

/// Cursor on a `class X(Schema)` declaration → its name token. Only
/// declarations in the focus file can be under the cursor.
fn definition_on_schema_declaration(
    offset: TextSize,
    schemas: &[Schema<'_>],
    focus_idx: usize,
) -> Option<(usize, TextRange)> {
    for schema in schemas {
        if schema.file_index == focus_idx && schema.class.def.name.range.contains_inclusive(offset)
        {
            return Some((schema.file_index, schema.class.def.name.range));
        }
    }
    None
}

/// Cursor on the `X` inside a `DataFrame[X]` slot of a typed function
/// signature → the range of `class X`'s name token, in whatever file
/// `X` is declared.
fn definition_on_schema_reference_in_function_signature(
    offset: TextSize,
    functions: &[DiscoveredFunction<'_>],
    schemas: &[Schema<'_>],
) -> Option<(usize, TextRange)> {
    for func in functions {
        for slot in typed_slots(func) {
            let Some(sub) = slot.annotation.as_subscript_expr() else {
                continue;
            };
            let Some(inner) = sub.slice.as_name_expr() else {
                continue;
            };
            if inner.range.contains_inclusive(offset) {
                if let Some(target) = schemas.iter().find(|s| s.name() == inner.id.as_str()) {
                    return Some((target.file_index, target.class.def.name.range));
                }
            }
        }
    }
    None
}

/// Cursor on the bare-name annotation of a nested-struct Schema field
/// (`address: Address`) → the range of `class Address`'s name. The
/// field being pointed at is in the focus file; the target class may
/// be imported.
fn definition_on_schema_reference_in_schema_field(
    offset: TextSize,
    schemas: &[Schema<'_>],
    focus_idx: usize,
) -> Option<(usize, TextRange)> {
    for schema in schemas {
        if schema.file_index != focus_idx {
            continue;
        }
        for field in schema.fields() {
            if let Expr::Name(name) = field.annotation {
                if name.range.contains_inclusive(offset) {
                    if let Some(target) = schemas.iter().find(|s| s.name() == name.id.as_str()) {
                        return Some((target.file_index, target.class.def.name.range));
                    }
                }
            }
        }
    }
    None
}

/// Cursor on a `col("foo")` string literal whose schema and field both
/// resolve → the range of the field's `name: type` annotation in the
/// Schema class body, in the file that schema is declared in. Only
/// `Declared` views point at a real source location; derived/grouped
/// views drop AST provenance, so no jump.
fn definition_on_column_ref(
    offset: TextSize,
    traces: &[ColumnRefTrace<'_>],
) -> Option<(usize, TextRange)> {
    let trace = traces.iter().find(|t| t.range.contains_inclusive(offset))?;
    let schema = match &trace.schema {
        SchemaView::Declared(s) => *s,
        _ => return None,
    };
    let field = schema.fields().into_iter().find(|f| f.name == trace.name)?;
    // Return the range of the field's target name (`foo` in `foo: int`)
    // by walking the class body for the matching AnnAssign. Falls back
    // to the annotation range if anything looks off.
    for stmt in &schema.class.def.body {
        let Some(ann) = stmt.as_ann_assign_stmt() else {
            continue;
        };
        let Some(target) = ann.target.as_name_expr() else {
            continue;
        };
        if target.id.as_str() == field.name {
            return Some((schema.file_index, target.range));
        }
    }
    Some((schema.file_index, field.annotation.range()))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn offset_from_line_column(
    line_index: &LineIndex,
    source: &str,
    line: usize,
    column: usize,
) -> Option<TextSize> {
    if column == 0 {
        return None;
    }
    let one_indexed = OneIndexed::new(line)?;
    let line_start = line_index.line_start(one_indexed, source);
    let column_offset = TextSize::from((column - 1) as u32);
    Some(line_start + column_offset)
}

pub(crate) fn span_from_range(range: TextRange, source: &str, line_index: &LineIndex) -> Span {
    let start = line_index.line_column(range.start(), source);
    let end = line_index.line_column(range.end(), source);
    Span {
        start_line: start.line.get(),
        start_column: start.column.get(),
        end_line: end.line.get(),
        end_column: end.column.get(),
    }
}
