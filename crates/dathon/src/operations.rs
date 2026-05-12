//! PySpark DataFrame operations — checking calls inside function bodies.
//!
//! Two families of method-call checks today:
//!
//! - **Column-method calls** — methods that take column references or
//!   expressions: `select`, `filter`, `where`, `drop`, `dropDuplicates`,
//!   `groupBy`, `withColumn`, `withColumnRenamed`. Each has a *shape*
//!   (`AllColumnName`, `AllExpression`, `Positional([ArgRole])`) that
//!   determines how each argument position is interpreted.
//!   Mismatches against the receiver's schema emit `D0030`.
//!
//! - **Two-DataFrame calls** — methods whose first argument is another
//!   `DataFrame`: `unionByName`, `union`. The other DataFrame must also be a
//!   typed parameter; the two schemas must have the same set of field
//!   names. Mismatches emit `D0040`.
//!
//! Scope cuts in v0.1:
//! - Only direct calls on parameters typed `DataFrame[S]`. No tracking of
//!   local variable bindings, and no support for chained call receivers.
//! - Only top-level statements in the body. Nested blocks ignored.
//! - `df.X` attribute access for columns not handled yet.
//! - `join`, `crossJoin` not handled yet (rich on-key surface — separate
//!   iteration).
//! - `union`/`unionByName` currently check the *name set* only. Column-order
//!   strictness for plain `union` will land separately.

use std::collections::{HashMap, HashSet};

use ruff_python_ast::{Expr, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot};
use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::Schema;
use crate::walk::DiscoveredFunction;

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum ArgRole {
    /// Top-level string lit (or list of string lits) is a column name to
    /// check; col() refs collected recursively.
    ColumnName,
    /// String literals are values; only col() refs count as column refs.
    Expression,
    /// New column name being introduced — don't check anything.
    NewName,
}

enum ColumnMethodShape {
    AllColumnName,
    AllExpression,
    /// Each position has its own role. Extra args reuse the last role.
    Positional(&'static [ArgRole]),
}

#[derive(Debug, Clone, Copy)]
enum TwoDfMethod {
    /// Name-set match required. `union` will become stricter (order) later.
    Union,
    UnionByName,
}

fn column_method_shape(method: &str) -> Option<ColumnMethodShape> {
    match method {
        "select" | "drop" | "dropDuplicates" | "groupBy" => {
            Some(ColumnMethodShape::AllColumnName)
        }
        "filter" | "where" => Some(ColumnMethodShape::AllExpression),
        "withColumn" => Some(ColumnMethodShape::Positional(&[
            ArgRole::NewName,
            ArgRole::Expression,
        ])),
        "withColumnRenamed" => Some(ColumnMethodShape::Positional(&[
            ArgRole::ColumnName,
            ArgRole::NewName,
        ])),
        _ => None,
    }
}

fn two_df_method(method: &str) -> Option<TwoDfMethod> {
    match method {
        "union" => Some(TwoDfMethod::Union),
        "unionByName" => Some(TwoDfMethod::UnionByName),
        _ => None,
    }
}

fn role_at(shape: &ColumnMethodShape, index: usize) -> ArgRole {
    match shape {
        ColumnMethodShape::AllColumnName => ArgRole::ColumnName,
        ColumnMethodShape::AllExpression => ArgRole::Expression,
        ColumnMethodShape::Positional(roles) => roles
            .get(index)
            .copied()
            .or_else(|| roles.last().copied())
            .unwrap_or(ArgRole::Expression),
    }
}

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

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

    fn lookup_param_expr(&self, expr: &Expr) -> Option<&'a Schema<'a>> {
        let name = expr.as_name_expr()?.id.as_str();
        self.lookup(name)
    }
}

// ---------------------------------------------------------------------------
// Body driver
// ---------------------------------------------------------------------------

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
        dispatch_call(expr, scope, source, line_index, diagnostics);
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

fn dispatch_call(
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
    let Some(receiver) = attr.value.as_name_expr() else {
        return;
    };
    let Some(receiver_schema) = scope.lookup(receiver.id.as_str()) else {
        return;
    };

    if let Some(shape) = column_method_shape(method) {
        check_column_method(
            call,
            receiver_schema,
            &shape,
            source,
            line_index,
            diagnostics,
        );
        return;
    }
    if let Some(kind) = two_df_method(method) {
        check_two_df_method(call, receiver_schema, kind, scope, source, line_index, diagnostics);
    }
}

// ---------------------------------------------------------------------------
// Column-method checking
// ---------------------------------------------------------------------------

fn check_column_method(
    call: &ruff_python_ast::ExprCall,
    schema: &Schema<'_>,
    shape: &ColumnMethodShape,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut refs: Vec<(&str, TextRange)> = Vec::new();
    for (i, arg) in call.arguments.args.iter().enumerate() {
        let role = role_at(shape, i);
        collect_arg_column_refs(arg, role, &mut refs);
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

fn collect_arg_column_refs<'a>(
    arg: &'a Expr,
    role: ArgRole,
    out: &mut Vec<(&'a str, TextRange)>,
) {
    match role {
        ArgRole::NewName => {}
        ArgRole::ColumnName => {
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
            collect_col_refs(arg, out);
        }
        ArgRole::Expression => collect_col_refs(arg, out),
    }
}

// ---------------------------------------------------------------------------
// Two-DataFrame checking
// ---------------------------------------------------------------------------

fn check_two_df_method(
    call: &ruff_python_ast::ExprCall,
    left: &Schema<'_>,
    _kind: TwoDfMethod,
    scope: &ParamScope<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(arg) = call.arguments.args.first() else {
        return;
    };
    let Some(right) = scope.lookup_param_expr(arg) else {
        // For now, only direct param-to-param. Local-variable / chained
        // receivers will be handled when binding propagation lands.
        return;
    };

    let left_names: HashSet<&str> = left.fields().iter().map(|f| f.name).collect();
    let right_names: HashSet<&str> = right.fields().iter().map(|f| f.name).collect();
    if left_names == right_names {
        return;
    }

    let mut only_left: Vec<&str> = left_names.difference(&right_names).copied().collect();
    let mut only_right: Vec<&str> = right_names.difference(&left_names).copied().collect();
    only_left.sort();
    only_right.sort();

    let message = format!(
        "unionByName between '{}' and '{}': schemas differ. \
         Missing in '{}': [{}]; missing in '{}': [{}].",
        left.name(),
        right.name(),
        right.name(),
        only_left.join(", "),
        left.name(),
        only_right.join(", "),
    );

    diagnostics.push(Diagnostic::at(
        Severity::Error,
        "D0040",
        message,
        arg.range().start(),
        source,
        line_index,
    ));
}

// ---------------------------------------------------------------------------
// col() reference discovery
// ---------------------------------------------------------------------------

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
