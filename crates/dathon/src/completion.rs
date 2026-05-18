//! Position-aware completion suggestions for the LSP layer.
//!
//! Given a cursor position in a source file, returns the list of
//! completion items the editor should offer. Three surfaces are covered:
//!
//! 1. **`DataFrame[...]`** — cursor inside the subscript slice → suggest
//!    every declared `Schema` class.
//! 2. **`col("...")`** — cursor inside a string literal argument of a
//!    `col(...)` call → suggest every column on the surrounding schema.
//!    The schema is determined the same way as iteration 28's hover for
//!    column references (immediate receiver of the surrounding method
//!    call).
//! 3. **`df.attr`** — cursor inside an attribute reference whose value
//!    is a function parameter / annotated local bound to a known
//!    `DataFrame[X]` → suggest every column on `X`.
//!
//! Out of scope for v0.1: completion on intermediate chain results
//! (`raw.select(...).<here>`), completion on local-variable bindings
//! produced by `x = raw.select(...)`, completion across files.

use std::collections::HashSet;

use ruff_python_ast::Expr;
use ruff_source_file::{LineIndex, OneIndexed};
use ruff_text_size::{Ranged, TextSize};

use crate::dataframe::{DataFrameAnnotation, SlotLabel, typed_slots};
use crate::registry::Registry;
use crate::schema::{FieldResolution, Schema, SchemaView, discover_schemas};
use crate::walk::{DiscoveredFunction, discover_top_level_classes, discover_top_level_functions};

/// One completion suggestion. `detail` is a short label shown next to the
/// name in VS Code's completion popup (column type, schema marker, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    pub kind: CompletionItemKind,
}

/// Kind of a completion item — narrower than LSP's enum so dathon
/// doesn't leak `lsp_types` into its public surface. The LSP layer
/// maps these to LSP kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionItemKind {
    /// A Schema class (`class X(Schema):`) — suggested inside `DataFrame[…]`.
    Class,
    /// A column / field on a schema — suggested inside `col("…")` and
    /// after `df.`.
    Field,
}

/// Resolve completion suggestions at `(line, column)` (both 1-indexed).
///
/// Returns an empty vector when the cursor isn't on a recognized
/// completion surface or when the source fails to parse. The empty-on-
/// parse-error behaviour matches `document_symbols`: a transient syntax
/// error in the middle of an edit shouldn't drop the suggestion list to
/// "error" — just to "nothing to suggest right now".
pub fn completions(source: &str, line: usize, column: usize) -> Vec<CompletionItem> {
    let Ok(parsed) = ruff_python_parser::parse_module(source) else {
        return Vec::new();
    };
    let module = parsed.syntax();
    let line_index = LineIndex::from_source_text(source);
    let classes = discover_top_level_classes(module);
    let schemas = discover_schemas(&classes);
    let registry = Registry::build(module);
    completions_with_scope(
        module,
        source,
        &line_index,
        line,
        column,
        &schemas,
        &registry,
    )
}

/// Project-aware completion: same shape as `hover_with_scope`. The
/// caller passes the focus file's parsed module plus pre-resolved
/// visible schemas + registry so cross-file imports show up in
/// `DataFrame[…]` and column-name suggestions.
pub(crate) fn completions_with_scope<'a>(
    module: &'a ruff_python_ast::ModModule,
    source: &str,
    line_index: &LineIndex,
    line: usize,
    column: usize,
    schemas: &[Schema<'a>],
    registry: &Registry<'a>,
) -> Vec<CompletionItem> {
    let Some(offset) = offset_from_line_column(line_index, source, line, column) else {
        return Vec::new();
    };
    let functions = discover_top_level_functions(module);

    if completes_dataframe_schema_slot(offset, &functions) {
        return schema_completions(schemas);
    }

    // Inside a `name: <cursor>` type annotation whose enclosing class
    // extends `Schema`: suggest the column-type vocabulary (the atomic
    // types, `Array` / `Map`, and the declared schema classes).
    if completes_schema_field_type(offset, module, schemas) {
        return column_type_completions(schemas);
    }

    let traces = crate::collect_module_traces(&functions, source, line_index, schemas, registry);

    if let Some(items) = column_completions_in_col_literal(offset, &traces.column_refs, schemas) {
        return items;
    }

    if let Some(items) =
        column_completions_after_df_dot(offset, &functions, schemas, &traces.local_bindings)
    {
        return items;
    }

    // `raw.select(...).<cursor>` — the receiver of the `.attr` is a call;
    // offer the chain result's columns.
    if let Some(items) =
        column_completions_after_chain_dot(offset, &functions, &traces.call_results, schemas)
    {
        return items;
    }

    Vec::new()
}

// ---------------------------------------------------------------------------
// Surface 0: cursor inside a field type annotation within a Schema
// class — suggest the column-type vocabulary.
// ---------------------------------------------------------------------------

fn completes_schema_field_type(
    offset: TextSize,
    module: &ruff_python_ast::ModModule,
    schemas: &[Schema<'_>],
) -> bool {
    // True when the cursor sits inside the type annotation of a
    // `name: <type>` field assignment in a Schema-class body.
    let body_has_annotation_at = |body: &[ruff_python_ast::Stmt]| {
        body.iter().any(|stmt| {
            matches!(stmt, ruff_python_ast::Stmt::AnnAssign(ann)
                if ann.annotation.range().contains_inclusive(offset))
        })
    };
    for schema in schemas {
        if body_has_annotation_at(&schema.class.def.body) {
            return true;
        }
    }
    // Schema classes can also be undeclared (the cursor is in a class
    // with syntax errors above it, so no Schema view was built). Fall
    // back to walking every top-level class with a `Schema` base.
    for stmt in &module.body {
        let ruff_python_ast::Stmt::ClassDef(class) = stmt else {
            continue;
        };
        let is_schema = class
            .bases()
            .iter()
            .any(|b| b.as_name_expr().is_some_and(|n| n.id.as_str() == "Schema"));
        if is_schema && body_has_annotation_at(&class.body) {
            return true;
        }
    }
    false
}

fn column_type_completions(schemas: &[Schema<'_>]) -> Vec<CompletionItem> {
    // The atomic types and `Array` / `Map`…
    let mut items: Vec<CompletionItem> = crate::types::COLUMN_TYPE_NAMES_LIST
        .iter()
        .map(|&name| CompletionItem {
            label: name.to_string(),
            detail: Some("dathon column type".to_string()),
            kind: CompletionItemKind::Field,
        })
        .collect();
    // …plus every declared Schema class (a field can be a nested struct).
    for schema in schemas {
        items.push(CompletionItem {
            label: schema.name().to_string(),
            detail: Some("Schema".to_string()),
            kind: CompletionItemKind::Class,
        });
    }
    items
}

// ---------------------------------------------------------------------------
// Surface 1: DataFrame[ <cursor> ] — schema names
// ---------------------------------------------------------------------------

fn completes_dataframe_schema_slot(offset: TextSize, functions: &[DiscoveredFunction<'_>]) -> bool {
    // A typed slot's annotation is a `Subscript(Name("DataFrame"), <slice>)`.
    // If the cursor is inside the slice, we're completing a schema name.
    for func in functions {
        for slot in typed_slots(func) {
            let Some(sub) = slot.annotation.as_subscript_expr() else {
                continue;
            };
            if sub.slice.range().contains_inclusive(offset) {
                return true;
            }
        }
    }
    false
}

fn schema_completions(schemas: &[Schema<'_>]) -> Vec<CompletionItem> {
    schemas
        .iter()
        .map(|s| CompletionItem {
            label: s.name().to_string(),
            detail: Some("Schema".to_string()),
            kind: CompletionItemKind::Class,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Surface 2: col("<cursor>") — column names from the surrounding schema
// ---------------------------------------------------------------------------

fn column_completions_in_col_literal(
    offset: TextSize,
    traces: &[crate::operations::ColumnRefTrace<'_>],
    schemas: &[Schema<'_>],
) -> Option<Vec<CompletionItem>> {
    let trace = traces.iter().find(|t| t.range.contains_inclusive(offset))?;
    Some(fields_of(&trace.schema, schemas))
}

// ---------------------------------------------------------------------------
// Surface 3: df.<cursor> — column names from df's schema
// ---------------------------------------------------------------------------

fn column_completions_after_df_dot(
    offset: TextSize,
    functions: &[DiscoveredFunction<'_>],
    schemas: &[Schema<'_>],
    local_bindings: &[crate::operations::LocalBindingTrace<'_>],
) -> Option<Vec<CompletionItem>> {
    // Walk every function looking for an Attribute(value=Name(name), attr)
    // where the cursor sits on `attr` (or in the gap between '.' and the
    // identifier). The receiver name resolves through, in order:
    //
    // 1. Typed function parameters (`raw: DataFrame[X]`).
    // 2. Local bindings produced by `x = raw.select(...)` and friends —
    //    captured by body analysis. Picks the LATEST trace for that
    //    name in the function, which is the right answer for the
    //    common single-assignment case.
    for func in functions {
        let Some((receiver, _attr_range)) = find_attribute_access_at(offset, &func.def.body)
        else {
            continue;
        };
        // Only a bare-name receiver is a parameter / local binding —
        // a call receiver is the chain-result surface's job.
        let Some(df_name) = receiver.as_name_expr().map(|n| n.id.as_str()) else {
            continue;
        };
        if let Some(schema) = df_param_schema(func, df_name, schemas) {
            return Some(fields_of(&SchemaView::Declared(schema), schemas));
        }
        if let Some(view) = latest_binding_for(df_name, local_bindings) {
            return Some(fields_of(view, schemas));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Surface 4: <call>.<cursor> — column names from the chain result schema
// ---------------------------------------------------------------------------

fn column_completions_after_chain_dot(
    offset: TextSize,
    functions: &[DiscoveredFunction<'_>],
    call_results: &[crate::operations::CallResultTrace<'_>],
    schemas: &[Schema<'_>],
) -> Option<Vec<CompletionItem>> {
    for func in functions {
        let Some((receiver, _attr_range)) = find_attribute_access_at(offset, &func.def.body)
        else {
            continue;
        };
        // Match the receiver expression against a recorded call result —
        // `raw.select(...)` in `raw.select(...).<cursor>`.
        if let Some(trace) = call_results.iter().find(|t| t.range == receiver.range()) {
            return Some(fields_of(&trace.schema, schemas));
        }
    }
    None
}

fn latest_binding_for<'a>(
    name: &str,
    bindings: &'a [crate::operations::LocalBindingTrace<'a>],
) -> Option<&'a SchemaView<'a>> {
    bindings
        .iter()
        .rev()
        .find(|b| b.name == name)
        .map(|b| &b.schema)
}

/// Walk a list of statements looking for an `Attribute(receiver, attr)`
/// expression where `offset` sits on the `attr` token. Returns the
/// receiver expression and the attr range.
fn find_attribute_access_at<'a>(
    offset: TextSize,
    body: &'a [ruff_python_ast::Stmt],
) -> Option<(&'a Expr, ruff_text_size::TextRange)> {
    for stmt in body {
        if let Some(hit) = stmt_attribute_at(offset, stmt) {
            return Some(hit);
        }
    }
    None
}

fn stmt_attribute_at<'a>(
    offset: TextSize,
    stmt: &'a ruff_python_ast::Stmt,
) -> Option<(&'a Expr, ruff_text_size::TextRange)> {
    use ruff_python_ast::Stmt;
    match stmt {
        Stmt::Expr(e) => expr_attribute_at(offset, &e.value),
        Stmt::Return(r) => r
            .value
            .as_deref()
            .and_then(|e| expr_attribute_at(offset, e)),
        Stmt::Assign(a) => expr_attribute_at(offset, &a.value),
        Stmt::AnnAssign(a) => a
            .value
            .as_deref()
            .and_then(|e| expr_attribute_at(offset, e)),
        _ => None,
    }
}

fn expr_attribute_at<'a>(
    offset: TextSize,
    expr: &'a Expr,
) -> Option<(&'a Expr, ruff_text_size::TextRange)> {
    if !expr.range().contains_inclusive(offset) {
        return None;
    }
    match expr {
        Expr::Attribute(a) => {
            // Cursor on the attr identifier itself? Return the receiver.
            if a.attr.range.contains_inclusive(offset) {
                return Some((a.value.as_ref(), a.attr.range));
            }
            // Otherwise descend.
            expr_attribute_at(offset, &a.value)
        }
        Expr::Call(c) => {
            if let Some(hit) = expr_attribute_at(offset, &c.func) {
                return Some(hit);
            }
            for arg in &c.arguments.args {
                if let Some(hit) = expr_attribute_at(offset, arg) {
                    return Some(hit);
                }
            }
            for kw in &c.arguments.keywords {
                if let Some(hit) = expr_attribute_at(offset, &kw.value) {
                    return Some(hit);
                }
            }
            None
        }
        Expr::Subscript(s) => {
            expr_attribute_at(offset, &s.value).or_else(|| expr_attribute_at(offset, &s.slice))
        }
        Expr::BinOp(b) => {
            expr_attribute_at(offset, &b.left).or_else(|| expr_attribute_at(offset, &b.right))
        }
        Expr::Compare(c) => {
            if let Some(hit) = expr_attribute_at(offset, &c.left) {
                return Some(hit);
            }
            for comp in &c.comparators {
                if let Some(hit) = expr_attribute_at(offset, comp) {
                    return Some(hit);
                }
            }
            None
        }
        _ => None,
    }
}

/// Resolve `df_name` to a Schema iff it's a function parameter annotated
/// `DataFrame[X]` and `X` is a declared Schema.
fn df_param_schema<'a>(
    func: &DiscoveredFunction<'a>,
    df_name: &str,
    schemas: &'a [Schema<'a>],
) -> Option<&'a Schema<'a>> {
    for slot in typed_slots(func) {
        let SlotLabel::Param(name) = slot.label else {
            continue;
        };
        if name != df_name {
            continue;
        }
        if let DataFrameAnnotation::Typed(schema_name) = slot.kind {
            return schemas.iter().find(|s| s.name() == schema_name);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Render field-name completions for a schema view. Declared views give
/// us types; derived views just give us names.
fn fields_of(view: &SchemaView<'_>, schemas: &[Schema<'_>]) -> Vec<CompletionItem> {
    let names: Vec<&str> = match view {
        SchemaView::Declared(s) => {
            return s
                .fields()
                .into_iter()
                .map(|f| CompletionItem {
                    label: f.name.to_string(),
                    detail: Some(field_detail(&f, schemas)),
                    kind: CompletionItemKind::Field,
                })
                .collect();
        }
        SchemaView::Derived(fields) => fields.iter().map(|f| f.name).collect(),
        SchemaView::Grouped { keys, .. } => keys.clone(),
    };
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::with_capacity(names.len());
    for n in names {
        if seen.insert(n) {
            out.push(CompletionItem {
                label: n.to_string(),
                detail: None,
                kind: CompletionItemKind::Field,
            });
        }
    }
    out
}

fn field_detail(f: &crate::schema::SchemaField<'_>, schemas: &[Schema<'_>]) -> String {
    match f.resolve(schemas) {
        FieldResolution::Resolved(ct) => ct.as_str().to_string(),
        FieldResolution::ResolvedNested(nested) => format!("{} (nested)", nested.name()),
        FieldResolution::UnknownType { name } => format!("{name} (unresolved)"),
        FieldResolution::NotABareName => "(unresolved)".to_string(),
    }
}

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
