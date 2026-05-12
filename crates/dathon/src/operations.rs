//! PySpark DataFrame operations — checking calls inside function bodies.
//!
//! Two flavors of recognized method:
//!
//! - **Column-name methods**: `select`, `drop`, `dropDuplicates`, `groupBy`.
//!   Top-level string-literal arguments are treated as column names (and a
//!   `dropDuplicates(["a", "b"])`-style list arg is unpacked). `col("X")`
//!   references anywhere inside an arg are also collected.
//! - **Expression methods**: `filter`, `where`. String literals are treated
//!   as values; only `col("X")` references count as column refs.
//!
//! Every collected column name is checked against the receiver's schema.
//! Mismatches emit `D0030`.
//!
//! Scope cuts in v0.1:
//! - Only direct calls on parameters typed `DataFrame[S]`. No tracking of
//!   local variable bindings.
//! - Only top-level statements in the body. Nested blocks ignored.
//! - `withColumn` / `withColumnRenamed` not handled yet (different arg shape
//!   from the two flavors above).
//! - `df.X` attribute access for columns not handled yet.
//! - Calls on chained results (`df.filter(...).select(...)`) only check the
//!   outermost direct-on-param call.

use std::collections::HashMap;

use ruff_python_ast::{Expr, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot};
use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::Schema;
use crate::walk::DiscoveredFunction;

const COLUMN_NAME_METHODS: &[&str] = &["select", "drop", "dropDuplicates", "groupBy"];
const EXPRESSION_METHODS: &[&str] = &["filter", "where"];

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

pub fn check_function_body(
    func: &DiscoveredFunction<'_>,
    scope: &ParamScope<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in &func.def.body {
        let Some(expr) = stmt_top_expr(stmt) else {
            continue;
        };
        check_column_method_call(expr, scope, source, line_index, diagnostics);
    }
}

fn stmt_top_expr(stmt: &Stmt) -> Option<&Expr> {
    match stmt {
        Stmt::Return(r) => r.value.as_deref(),
        Stmt::Assign(a) => Some(&a.value),
        Stmt::Expr(e) => Some(&e.value),
        _ => None,
    }
}

fn check_column_method_call(
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
    let method = attr.attr.id.as_str();
    let strings_are_columns = if COLUMN_NAME_METHODS.contains(&method) {
        true
    } else if EXPRESSION_METHODS.contains(&method) {
        false
    } else {
        return;
    };
    let Some(receiver) = attr.value.as_name_expr() else {
        return;
    };
    let Some(schema) = scope.lookup(receiver.id.as_str()) else {
        return;
    };

    let mut refs: Vec<(&str, TextRange)> = Vec::new();
    for arg in &call.arguments.args {
        collect_arg_column_refs(arg, strings_are_columns, &mut refs);
    }
    for (col_name, col_range) in refs {
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

/// Collect column references from a single argument expression.
///
/// `strings_are_columns` controls whether top-level string literals (and
/// string literals inside a top-level list literal) are interpreted as
/// column names. Either way, `col("X")` refs anywhere inside the expression
/// are collected too.
fn collect_arg_column_refs<'a>(
    arg: &'a Expr,
    strings_are_columns: bool,
    out: &mut Vec<(&'a str, TextRange)>,
) {
    if strings_are_columns {
        if let Some(s) = arg.as_string_literal_expr() {
            out.push((s.value.to_str(), s.range()));
            return;
        }
        if let Some(list) = arg.as_list_expr() {
            for elt in &list.elts {
                if let Some(s) = elt.as_string_literal_expr() {
                    out.push((s.value.to_str(), s.range()));
                } else {
                    collect_col_refs(elt, out);
                }
            }
            return;
        }
    }
    collect_col_refs(arg, out);
}

/// Match `col("X")` and return ("X", range-of-the-string-literal).
fn col_reference(expr: &Expr) -> Option<(&str, TextRange)> {
    let call = expr.as_call_expr()?;
    let func = call.func.as_name_expr()?;
    if func.id.as_str() != "col" {
        return None;
    }
    let arg = call.arguments.args.first()?;
    let lit = arg.as_string_literal_expr()?;
    Some((lit.value.to_str(), lit.range()))
}

/// Recursively collect every `col("X")` reference inside `expr`.
///
/// When we find a `col("X")` we record it and stop recursing into that
/// subexpression. Scopes that bind new names (lambdas, comprehensions) are
/// not entered.
fn collect_col_refs<'a>(expr: &'a Expr, out: &mut Vec<(&'a str, TextRange)>) {
    if let Some(found) = col_reference(expr) {
        out.push(found);
        return;
    }
    match expr {
        Expr::Call(c) => {
            collect_col_refs(&c.func, out);
            for arg in &c.arguments.args {
                collect_col_refs(arg, out);
            }
            for kw in &c.arguments.keywords {
                collect_col_refs(&kw.value, out);
            }
        }
        Expr::Attribute(a) => collect_col_refs(&a.value, out),
        Expr::Subscript(s) => {
            collect_col_refs(&s.value, out);
            collect_col_refs(&s.slice, out);
        }
        Expr::BinOp(b) => {
            collect_col_refs(&b.left, out);
            collect_col_refs(&b.right, out);
        }
        Expr::UnaryOp(u) => collect_col_refs(&u.operand, out),
        Expr::Compare(c) => {
            collect_col_refs(&c.left, out);
            for cmp in &c.comparators {
                collect_col_refs(cmp, out);
            }
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                collect_col_refs(v, out);
            }
        }
        Expr::If(if_exp) => {
            collect_col_refs(&if_exp.test, out);
            collect_col_refs(&if_exp.body, out);
            collect_col_refs(&if_exp.orelse, out);
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                collect_col_refs(e, out);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                collect_col_refs(e, out);
            }
        }
        Expr::Starred(s) => collect_col_refs(&s.value, out),
        _ => {}
    }
}
