//! PySpark DataFrame operations — checking calls inside function bodies.
//!
//! Today, only `.select(col("X"), ...)` calls are recognized, and only when
//! the receiver is a function parameter typed `DataFrame[S]`. The check:
//! every literal `col("X")` argument must name a field of S.
//!
//! Scope cuts in v0.1:
//! - Only direct calls on parameters. No tracking of local variable bindings.
//! - Only top-level statements in the body (no nested blocks, ifs, loops).
//! - Only `col("X")` literal column refs. No `df.X` attribute access, no
//!   expression `col("X") + col("Y")`, no keyword arguments.
//! - Only one operation: `select`. More land iteration by iteration.

use std::collections::HashMap;

use ruff_python_ast::{Expr, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::Ranged;

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot};
use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::Schema;
use crate::walk::DiscoveredFunction;

/// Maps a function-parameter name to the Schema it's typed as.
pub struct ParamScope<'a> {
    bindings: HashMap<&'a str, &'a Schema<'a>>,
}

impl<'a> ParamScope<'a> {
    pub fn build(slots: &[TypedSlot<'a>], schemas: &'a [Schema<'a>]) -> Self {
        let mut bindings = HashMap::new();
        for slot in slots {
            let SlotLabel::Param(name) = slot.label else {
                continue;
            };
            let DataFrameAnnotation::Typed(schema_name) = slot.kind else {
                continue;
            };
            if let Some(schema) = schemas.iter().find(|s| s.name() == schema_name) {
                bindings.insert(name, schema);
            }
        }
        Self { bindings }
    }

    pub fn lookup(&self, name: &str) -> Option<&'a Schema<'a>> {
        self.bindings.get(name).copied()
    }
}

/// Walk a function body, find direct `<param>.select(...)` calls, and check
/// each `col("X")` argument against the parameter's schema.
pub fn check_function_body(
    func: &DiscoveredFunction<'_>,
    scope: &ParamScope<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in &func.def.body {
        if let Some(expr) = stmt_top_expr(stmt) {
            check_select_call(expr, scope, source, line_index, diagnostics);
        }
    }
}

/// The single top-level expression of a statement, if any. Statements with no
/// directly-attached expression (e.g. `if`, `for`, `pass`) return `None` —
/// we'll handle nested blocks in a later iteration.
fn stmt_top_expr(stmt: &Stmt) -> Option<&Expr> {
    match stmt {
        Stmt::Return(r) => r.value.as_deref(),
        Stmt::Assign(a) => Some(&a.value),
        Stmt::Expr(e) => Some(&e.value),
        _ => None,
    }
}

fn check_select_call(
    expr: &Expr,
    scope: &ParamScope<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(call) = expr.as_call_expr() else {
        return;
    };
    let Some(attr) = call.func.as_attribute_expr() else {
        return;
    };
    if attr.attr.id.as_str() != "select" {
        return;
    }
    let Some(receiver) = attr.value.as_name_expr() else {
        return;
    };
    let Some(schema) = scope.lookup(receiver.id.as_str()) else {
        return;
    };

    for arg in &call.arguments.args {
        let Some((col_name, col_range)) = col_reference(arg) else {
            continue;
        };
        if !schema.has_field(col_name) {
            diagnostics.push(Diagnostic::at(
                Severity::Error,
                "D0030",
                format!(
                    "Column '{col_name}' does not exist on schema '{}'.",
                    schema.name(),
                ),
                col_range.start(),
                source,
                line_index,
            ));
        }
    }
}

/// Match `col("X")` and return ("X", range-of-the-string-literal).
fn col_reference(expr: &Expr) -> Option<(&str, ruff_text_size::TextRange)> {
    let call = expr.as_call_expr()?;
    let func = call.func.as_name_expr()?;
    if func.id.as_str() != "col" {
        return None;
    }
    let arg = call.arguments.args.first()?;
    let lit = arg.as_string_literal_expr()?;
    Some((lit.value.to_str(), lit.range()))
}
