//! Position-aware hover info for the LSP layer.
//!
//! Given a `(line, column)` cursor position in a source file, return a
//! markdown blob describing the symbol at that point. The LSP server
//! wraps the markdown in a `textDocument/hover` response.
//!
//! v0.1 supports four positions:
//!
//! 1. Cursor on a Schema class **declaration** (`class Orders(Schema):`)
//!    — return the schema's fields with their `ColumnType`s.
//! 2. Cursor on a typed function **declaration** (`def f(...) -> ...:`)
//!    — return the function's typed signature.
//! 3. Cursor on a Schema **reference** by name in an annotation we
//!    recognize (the `X` inside `DataFrame[X]` on a function signature,
//!    or the bare-name annotation of a Schema field) — return that
//!    schema's info.
//! 4. Cursor on a `col("foo")` **string literal** in a function body
//!    — return the column's resolved type on the surrounding schema.
//!    Requires running body analysis to know which schema each `col(…)`
//!    refers to (handled lazily, only when cases 1–3 don't match).
//!
//! Everything else returns `None` (the LSP server then sends no hover).
//! Hover for local-variable bindings (`x = raw.select(...)`) and cross-file
//! references is intentionally deferred to follow-up iterations.

use std::fmt::Write as _;

use ruff_python_ast::Expr;
use ruff_source_file::{LineIndex, OneIndexed};
use ruff_text_size::TextSize;

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot, typed_slots};
use crate::operations::ColumnRefTrace;
use crate::registry::Registry;
use crate::schema::{
    FieldPathResult, FieldResolution, Schema, SchemaView, discover_schemas, resolve_path,
};
use crate::walk::{DiscoveredFunction, discover_top_level_classes, discover_top_level_functions};

/// The hover payload returned by [`hover`]. `markdown` is the rendered
/// content for the editor's hover popup; clients typically render it as
/// CommonMark with embedded code blocks.
#[derive(Debug)]
pub struct HoverInfo {
    pub markdown: String,
}

/// Resolve the symbol at `(line, column)` (both 1-indexed, matching the
/// convention used elsewhere in dathon) and return a hover payload.
///
/// Returns `None` if no hoverable symbol is at that position, or if the
/// source file fails to parse.
pub fn hover(source: &str, line: usize, column: usize) -> Option<HoverInfo> {
    let parsed = ruff_python_parser::parse_module(source).ok()?;
    let module = parsed.syntax();
    let line_index = LineIndex::from_source_text(source);
    let offset = offset_from_line_column(&line_index, source, line, column)?;

    let classes = discover_top_level_classes(module);
    let schemas = discover_schemas(&classes);
    let functions = discover_top_level_functions(module);

    if let Some(info) = hover_on_schema_declaration(offset, &schemas) {
        return Some(info);
    }
    if let Some(info) = hover_on_typed_function_declaration(offset, &functions, &schemas) {
        return Some(info);
    }
    if let Some(info) =
        hover_on_schema_reference_in_function_signature(offset, &functions, &schemas)
    {
        return Some(info);
    }
    if let Some(info) = hover_on_schema_reference_in_schema_field(offset, &schemas) {
        return Some(info);
    }
    // Body-context-aware case: cursor on a `col("foo")` string literal.
    // Building the registry + running body analysis is the expensive
    // path, so we do it last after the cheap AST-lookup cases above
    // have all failed.
    let registry = Registry::build(module);
    let traces =
        crate::collect_module_column_refs(&functions, source, &line_index, &schemas, &registry);
    if let Some(info) = hover_on_column_ref(offset, &traces, &schemas) {
        return Some(info);
    }
    None
}

// ---------------------------------------------------------------------------
// Case 1: cursor on a Schema class declaration name.
// ---------------------------------------------------------------------------

fn hover_on_schema_declaration(offset: TextSize, schemas: &[Schema<'_>]) -> Option<HoverInfo> {
    for schema in schemas {
        if schema.class.def.name.range.contains_inclusive(offset) {
            return Some(render_schema_hover(schema, schemas));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Case 2: cursor on a typed function declaration name.
// ---------------------------------------------------------------------------

fn hover_on_typed_function_declaration(
    offset: TextSize,
    functions: &[DiscoveredFunction<'_>],
    schemas: &[Schema<'_>],
) -> Option<HoverInfo> {
    for func in functions {
        let slots = typed_slots(func);
        if slots.is_empty() {
            continue;
        }
        if func.def.name.range.contains_inclusive(offset) {
            return Some(render_function_hover(func, &slots, schemas));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Case 3a: cursor on a Schema name reference in a `DataFrame[X]` slot on
// a function signature.
// ---------------------------------------------------------------------------

fn hover_on_schema_reference_in_function_signature(
    offset: TextSize,
    functions: &[DiscoveredFunction<'_>],
    schemas: &[Schema<'_>],
) -> Option<HoverInfo> {
    for func in functions {
        for slot in typed_slots(func) {
            if let Some(name) = name_inside_subscript_slice(slot.annotation, offset) {
                if let Some(schema) = schemas.iter().find(|s| s.name() == name) {
                    return Some(render_schema_hover(schema, schemas));
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Case 3b: cursor on a Schema name used as a field type in another schema
// (nested-struct field).
// ---------------------------------------------------------------------------

fn hover_on_schema_reference_in_schema_field(
    offset: TextSize,
    schemas: &[Schema<'_>],
) -> Option<HoverInfo> {
    for schema in schemas {
        for field in schema.fields() {
            // Only bare-name annotations like `address: Address` qualify.
            if let Expr::Name(name) = field.annotation {
                if name.range.contains_inclusive(offset) {
                    if let Some(target) = schemas.iter().find(|s| s.name() == name.id.as_str()) {
                        return Some(render_schema_hover(target, schemas));
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Case 4: cursor inside a `col("foo")` string literal — show the column's
// resolved type on the surrounding schema. Requires running body analysis
// to know which schema each col() refers to.
// ---------------------------------------------------------------------------

fn hover_on_column_ref(
    offset: TextSize,
    traces: &[ColumnRefTrace<'_>],
    schemas: &[Schema<'_>],
) -> Option<HoverInfo> {
    let trace = traces.iter().find(|t| t.range.contains_inclusive(offset))?;
    Some(render_column_ref_hover(trace, schemas))
}

fn render_column_ref_hover(trace: &ColumnRefTrace<'_>, schemas: &[Schema<'_>]) -> HoverInfo {
    let mut md = String::new();
    writeln!(md, "**column `{}`**", trace.name).unwrap();
    writeln!(md).unwrap();
    writeln!(md, "on {}", trace.schema.display_name()).unwrap();
    writeln!(md).unwrap();

    match resolve_path(&trace.schema, trace.name, schemas) {
        FieldPathResult::Missing { field, on } => {
            writeln!(
                md,
                "_Column `{field}` does not exist on {} — see D0030._",
                on.display_name(),
            )
            .unwrap();
        }
        FieldPathResult::Resolved => {
            // resolve_path only confirms existence. For type info we have
            // to look the field up on the underlying Schema (if the trace
            // carries a Declared one) — Derived/Grouped views drop types,
            // so we just show the field name.
            if let Some(label) = column_type_label(&trace.schema, trace.name, schemas) {
                writeln!(md, "Type: {label}").unwrap();
            }
        }
    }
    HoverInfo { markdown: md }
}

/// Look up `name` on `view` (a `Declared` view) and return a markdown
/// type label. For `Derived` and `Grouped` views we don't carry the
/// per-field annotation, so this returns `None`.
fn column_type_label(view: &SchemaView<'_>, name: &str, schemas: &[Schema<'_>]) -> Option<String> {
    let schema = match view {
        SchemaView::Declared(s) => *s,
        _ => return None,
    };
    let field = schema.fields().into_iter().find(|f| f.name == name)?;
    Some(match field.resolve(schemas) {
        FieldResolution::Resolved(ct) => format!("`{}`", ct.as_str()),
        FieldResolution::ResolvedNested(nested) => format!("`{}` (nested)", nested.name()),
        FieldResolution::UnknownType { name } => format!("`{name}` (unresolved)"),
        FieldResolution::NotABareName => "_unresolved_".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn render_schema_hover(schema: &Schema<'_>, all_schemas: &[Schema<'_>]) -> HoverInfo {
    let mut md = String::new();
    writeln!(md, "**schema `{}`**", schema.name()).unwrap();
    let fields = schema.fields();
    if fields.is_empty() {
        writeln!(md).unwrap();
        writeln!(md, "_no fields_").unwrap();
    } else {
        writeln!(md).unwrap();
        writeln!(md, "Fields:").unwrap();
        writeln!(md).unwrap();
        for field in fields {
            let type_label = match field.resolve(all_schemas) {
                FieldResolution::Resolved(ct) => format!("`{}`", ct.as_str()),
                FieldResolution::ResolvedNested(nested) => {
                    format!("`{}` (nested)", nested.name())
                }
                FieldResolution::UnknownType { name } => format!("`{name}` (unresolved)"),
                FieldResolution::NotABareName => "_unresolved_".to_string(),
            };
            writeln!(md, "- `{}`: {}", field.name, type_label).unwrap();
        }
    }
    HoverInfo { markdown: md }
}

fn render_function_hover(
    func: &DiscoveredFunction<'_>,
    slots: &[TypedSlot<'_>],
    _schemas: &[Schema<'_>],
) -> HoverInfo {
    let mut md = String::new();
    writeln!(md, "**fn `{}`**", func.name()).unwrap();
    writeln!(md).unwrap();
    writeln!(md, "Typed signature:").unwrap();
    writeln!(md).unwrap();
    writeln!(md, "```").unwrap();
    write!(md, "{}(", func.name()).unwrap();
    let params: Vec<String> = slots
        .iter()
        .filter_map(|slot| match slot.label {
            SlotLabel::Param(name) => Some(format!("{name}: {}", render_annotation(&slot.kind))),
            SlotLabel::Return => None,
        })
        .collect();
    write!(md, "{}", params.join(", ")).unwrap();
    write!(md, ")").unwrap();
    if let Some(ret) = slots.iter().find(|s| matches!(s.label, SlotLabel::Return)) {
        write!(md, " -> {}", render_annotation(&ret.kind)).unwrap();
    }
    writeln!(md).unwrap();
    writeln!(md, "```").unwrap();
    HoverInfo { markdown: md }
}

fn render_annotation(kind: &DataFrameAnnotation<'_>) -> String {
    match kind {
        DataFrameAnnotation::Typed(name) => format!("DataFrame[{name}]"),
        DataFrameAnnotation::Untyped => "DataFrame".to_string(),
        DataFrameAnnotation::NonBareName => "DataFrame[?]".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Position / AST helpers
// ---------------------------------------------------------------------------

/// Convert 1-indexed `(line, column)` to a `TextSize` byte offset.
///
/// Returns `None` if `line` is 0 (1-indexed line numbers start at 1) or if
/// `column` is 0. Columns past end-of-line are clamped to end-of-line; the
/// caller treats that as "no symbol here" and gets `None` from `hover`.
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

/// If `expr` is a `Subscript(Name(_), Name(X))` and `offset` falls within
/// the inner `Name(X)`'s range, return `X` — the schema name being
/// referenced. This is the `DataFrame[X]` case where we want to hover on
/// `X`.
fn name_inside_subscript_slice<'a>(expr: &'a Expr, offset: TextSize) -> Option<&'a str> {
    let subscript = expr.as_subscript_expr()?;
    let inner = subscript.slice.as_name_expr()?;
    if inner.range.contains_inclusive(offset) {
        Some(inner.id.as_str())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the position helpers (cases involving real AST are tested
// as integration tests under tests/hover.rs).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_for_first_position_in_file_is_zero() {
        let source = "abc\ndef\n";
        let li = LineIndex::from_source_text(source);
        let offset = offset_from_line_column(&li, source, 1, 1).unwrap();
        assert_eq!(offset, TextSize::from(0));
    }

    #[test]
    fn offset_advances_within_a_line_and_across_lines() {
        let source = "abc\ndef\n";
        let li = LineIndex::from_source_text(source);
        // Line 1, column 3 → byte offset 2 (the 'c').
        assert_eq!(
            offset_from_line_column(&li, source, 1, 3).unwrap(),
            TextSize::from(2)
        );
        // Line 2, column 1 → byte offset 4 (the 'd' after the '\n').
        assert_eq!(
            offset_from_line_column(&li, source, 2, 1).unwrap(),
            TextSize::from(4)
        );
    }

    #[test]
    fn zero_line_or_zero_column_returns_none() {
        let source = "abc\n";
        let li = LineIndex::from_source_text(source);
        assert!(offset_from_line_column(&li, source, 0, 1).is_none());
        assert!(offset_from_line_column(&li, source, 1, 0).is_none());
    }
}
