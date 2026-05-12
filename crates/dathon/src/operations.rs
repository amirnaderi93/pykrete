//! PySpark DataFrame operations — checking calls inside function bodies, with
//! result-schema inference.
//!
//! ## Recursive expression analysis
//!
//! [`analyze_expr`] walks an expression that *might* evaluate to a DataFrame.
//! It returns a [`SchemaView`] when it can determine the value's schema, and
//! `None` otherwise. Along the way, every recognized method call has its
//! arguments checked against the receiver's schema and emits diagnostics
//! when something is wrong.
//!
//! The recursion is what enables chained-call support — for
//! `raw.filter(col("a") > 0).select("madeup")`, `analyze_expr` on the outer
//! `select` first analyzes its receiver (the `filter` call), which in turn
//! analyzes *its* receiver (`raw`). Each level reports diagnostics and
//! returns its result schema.
//!
//! ## Bindings
//!
//! [`BodyContext`] is a name → schema map. It starts populated from typed
//! function parameters, and grows as assignments produce new schemas. This
//! turns `x = raw.select("a"); x.select("b")` into a checkable chain.
//!
//! ## Return-type validation
//!
//! For each `return <value>` statement, the value's inferred schema is
//! compared to the function's declared return schema (`-> DataFrame[X]`).
//! Mismatches emit `D0050`.

use std::collections::{HashMap, HashSet};

use ruff_python_ast::{Expr, ExprCall, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot};
use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::{Schema, SchemaView};
use crate::walk::DiscoveredFunction;

// ---------------------------------------------------------------------------
// Method shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum ArgRole {
    ColumnName,
    Expression,
    NewName,
}

enum ColumnMethodShape {
    AllColumnName,
    AllExpression,
    Positional(&'static [ArgRole]),
}

#[derive(Debug, Clone, Copy)]
enum TwoDfMethod {
    Union,
    UnionByName,
    Join,
    CrossJoin,
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
        "join" => Some(TwoDfMethod::Join),
        "crossJoin" => Some(TwoDfMethod::CrossJoin),
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
// BodyContext — parameter and local-variable bindings
// ---------------------------------------------------------------------------

pub struct BodyContext<'a> {
    bindings: HashMap<&'a str, SchemaView<'a>>,
}

impl<'a> BodyContext<'a> {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    pub fn from_slots(slots: &[TypedSlot<'a>], schemas: &'a [Schema<'a>]) -> Self {
        let mut ctx = Self::new();
        for slot in slots {
            let SlotLabel::Param(name) = slot.label else {
                continue;
            };
            let DataFrameAnnotation::Typed(schema_name) = slot.kind else {
                continue;
            };
            if let Some(schema) = schemas.iter().find(|s| s.name() == schema_name) {
                ctx.bind(name, SchemaView::Declared(schema));
            }
        }
        ctx
    }

    pub fn bind(&mut self, name: &'a str, view: SchemaView<'a>) {
        self.bindings.insert(name, view);
    }

    pub fn lookup(&self, name: &str) -> Option<SchemaView<'a>> {
        self.bindings.get(name).cloned()
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub fn check_function_body<'a>(
    func: &'a DiscoveredFunction<'a>,
    declared_return: Option<&'a Schema<'a>>,
    ctx: &mut BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in &func.def.body {
        match stmt {
            Stmt::Assign(a) => {
                let schema = analyze_expr(&a.value, ctx, source, line_index, diagnostics);
                if let Some(schema) = schema {
                    for target in &a.targets {
                        if let Some(name) = target.as_name_expr() {
                            ctx.bind(name.id.as_str(), schema.clone());
                        }
                    }
                }
            }
            Stmt::Return(r) => {
                let Some(value) = r.value.as_deref() else {
                    continue;
                };
                let actual = analyze_expr(value, ctx, source, line_index, diagnostics);
                if let (Some(declared), Some(actual)) = (declared_return, actual.as_ref()) {
                    check_return_type(
                        declared,
                        actual,
                        value.range(),
                        source,
                        line_index,
                        diagnostics,
                    );
                }
            }
            Stmt::Expr(e) => {
                analyze_expr(&e.value, ctx, source, line_index, diagnostics);
            }
            _ => {}
        }
    }
}

fn check_return_type(
    declared: &Schema<'_>,
    actual: &SchemaView<'_>,
    range: TextRange,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let declared_names: HashSet<&str> = declared.fields().iter().map(|f| f.name).collect();
    let actual_names: HashSet<&str> = actual.field_names().into_iter().collect();
    if declared_names == actual_names {
        return;
    }
    let mut only_declared: Vec<&str> = declared_names
        .difference(&actual_names)
        .copied()
        .collect();
    let mut only_actual: Vec<&str> = actual_names
        .difference(&declared_names)
        .copied()
        .collect();
    only_declared.sort();
    only_actual.sort();

    let message = format!(
        "Return type mismatch with declared schema '{}'. \
         Missing in body: [{}]; extra in body: [{}].",
        declared.name(),
        only_declared.join(", "),
        only_actual.join(", "),
    );
    diagnostics.push(Diagnostic::at(
        Severity::Error,
        "D0050",
        message,
        range.start(),
        source,
        line_index,
    ));
}

// ---------------------------------------------------------------------------
// analyze_expr — the recursive heart
// ---------------------------------------------------------------------------

fn analyze_expr<'a>(
    expr: &'a Expr,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    match expr {
        Expr::Name(n) => ctx.lookup(n.id.as_str()),
        Expr::Call(call) => analyze_method_call(call, ctx, source, line_index, diagnostics),
        _ => None,
    }
}

fn analyze_method_call<'a>(
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let attr = call.func.as_attribute_expr()?;
    let method = attr.attr.id.as_str();
    let receiver = analyze_expr(&attr.value, ctx, source, line_index, diagnostics)?;

    if let Some(shape) = column_method_shape(method) {
        check_column_method_args(call, &receiver, &shape, source, line_index, diagnostics);
        return apply_column_method(method, &receiver, call);
    }
    if let Some(kind) = two_df_method(method) {
        return handle_two_df_method(kind, call, &receiver, ctx, source, line_index, diagnostics);
    }
    None
}

// ---------------------------------------------------------------------------
// Column-method checking + result inference
// ---------------------------------------------------------------------------

fn check_column_method_args(
    call: &ExprCall,
    schema: &SchemaView<'_>,
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
                    "Column '{col_name}' does not exist on {}.",
                    schema.display_name(),
                ),
                col_range.start(),
                source,
                line_index,
            ));
        }
    }
}

fn apply_column_method<'a>(
    method: &str,
    recv: &SchemaView<'a>,
    call: &'a ExprCall,
) -> Option<SchemaView<'a>> {
    match method {
        "select" => {
            let mut fields: Vec<&'a str> = Vec::new();
            for arg in &call.arguments.args {
                if let Some(name) = select_output_name(arg) {
                    fields.push(name);
                }
            }
            Some(SchemaView::Derived(fields))
        }
        "filter" | "where" | "dropDuplicates" => Some(recv.clone()),
        "drop" => {
            let drop_set: HashSet<&str> = call
                .arguments
                .args
                .iter()
                .filter_map(column_name_arg)
                .collect();
            let remaining: Vec<&'a str> = recv
                .field_names()
                .into_iter()
                .filter(|n| !drop_set.contains(n))
                .collect();
            Some(SchemaView::Derived(remaining))
        }
        "withColumn" => {
            let new_name = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
                .map(|s| s.value.to_str())?;
            let mut fields: Vec<&'a str> = recv.field_names();
            if !fields.contains(&new_name) {
                fields.push(new_name);
            }
            Some(SchemaView::Derived(fields))
        }
        "withColumnRenamed" => {
            let old = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
                .map(|s| s.value.to_str())?;
            let new = call
                .arguments
                .args
                .get(1)
                .and_then(|a| a.as_string_literal_expr())
                .map(|s| s.value.to_str())?;
            let fields: Vec<&'a str> = recv
                .field_names()
                .into_iter()
                .map(|n| if n == old { new } else { n })
                .collect();
            Some(SchemaView::Derived(fields))
        }
        // groupBy returns a GroupedData, not a DataFrame.
        "groupBy" => None,
        _ => None,
    }
}

fn select_output_name<'a>(arg: &'a Expr) -> Option<&'a str> {
    if let Some(call) = arg.as_call_expr() {
        if let Some(attr) = call.func.as_attribute_expr() {
            if attr.attr.id.as_str() == "alias" {
                if let Some(lit) = call
                    .arguments
                    .args
                    .first()
                    .and_then(|a| a.as_string_literal_expr())
                {
                    return Some(lit.value.to_str());
                }
            }
        }
    }
    if let Some(s) = arg.as_string_literal_expr() {
        return Some(s.value.to_str());
    }
    if let Some((name, _)) = col_reference(arg) {
        return Some(name);
    }
    if let Some(call) = arg.as_call_expr() {
        if let Some(attr) = call.func.as_attribute_expr() {
            if attr.attr.id.as_str() == "cast" {
                return select_output_name(&attr.value);
            }
        }
    }
    None
}

fn column_name_arg<'a>(arg: &'a Expr) -> Option<&'a str> {
    if let Some(s) = arg.as_string_literal_expr() {
        return Some(s.value.to_str());
    }
    if let Some((name, _)) = col_reference(arg) {
        return Some(name);
    }
    None
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
// Two-DataFrame methods (union, unionByName, join, crossJoin)
// ---------------------------------------------------------------------------

/// How a `.join(other, on=…)` call's `on=` argument is interpreted.
#[derive(Debug)]
enum JoinOn<'a> {
    /// No on-clause was given (`df.join(other)`). Equivalent to a cross
    /// product. No keys to check.
    None,
    /// One or more named keys, either `on="k"` or `on=["k1", "k2"]`. Each
    /// key must exist in BOTH schemas; in the result they appear once.
    Keys(Vec<(&'a str, TextRange)>),
    /// A complex on-expression (Column expression, mixed list, etc.).
    /// dathon doesn't analyze the on-clause; the result schema is the
    /// concatenation of both sides.
    Expression,
}

fn handle_two_df_method<'a>(
    kind: TwoDfMethod,
    call: &'a ExprCall,
    left: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let arg = call.arguments.args.first()?;
    let right = analyze_expr(arg, ctx, source, line_index, diagnostics)?;

    match kind {
        TwoDfMethod::Union | TwoDfMethod::UnionByName => {
            check_union_schemas(
                left,
                &right,
                arg.range(),
                source,
                line_index,
                diagnostics,
            );
            Some(left.clone())
        }
        TwoDfMethod::Join => {
            let on = parse_on_arg(extract_on_arg(call));
            check_join_keys(left, &right, &on, source, line_index, diagnostics);
            Some(apply_join(left, &right, &on))
        }
        TwoDfMethod::CrossJoin => Some(apply_concat(left, &right)),
    }
}

fn check_union_schemas(
    left: &SchemaView<'_>,
    right: &SchemaView<'_>,
    range: TextRange,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let left_names: HashSet<&str> = left.field_names().into_iter().collect();
    let right_names: HashSet<&str> = right.field_names().into_iter().collect();
    if left_names == right_names {
        return;
    }
    let mut only_left: Vec<&str> = left_names.difference(&right_names).copied().collect();
    let mut only_right: Vec<&str> = right_names.difference(&left_names).copied().collect();
    only_left.sort();
    only_right.sort();
    let message = format!(
        "unionByName between {} and {}: schemas differ. \
         Missing in {}: [{}]; missing in {}: [{}].",
        left.display_name(),
        right.display_name(),
        right.display_name(),
        only_left.join(", "),
        left.display_name(),
        only_right.join(", "),
    );
    diagnostics.push(Diagnostic::at(
        Severity::Error,
        "D0040",
        message,
        range.start(),
        source,
        line_index,
    ));
}

/// The on= argument of a join call, looked up either as `on=…` keyword or as
/// the second positional argument.
fn extract_on_arg<'a>(call: &'a ExprCall) -> Option<&'a Expr> {
    for kw in &call.arguments.keywords {
        if let Some(name) = kw.arg.as_ref() {
            if name.id.as_str() == "on" {
                return Some(&kw.value);
            }
        }
    }
    call.arguments.args.get(1)
}

fn parse_on_arg<'a>(expr: Option<&'a Expr>) -> JoinOn<'a> {
    let Some(expr) = expr else {
        return JoinOn::None;
    };
    if let Some(s) = expr.as_string_literal_expr() {
        return JoinOn::Keys(vec![(s.value.to_str(), s.range())]);
    }
    if let Some(list) = expr.as_list_expr() {
        let mut keys = Vec::new();
        for elt in &list.elts {
            let Some(s) = elt.as_string_literal_expr() else {
                // Mixed: bail out to "complex expression" rather than
                // half-checking.
                return JoinOn::Expression;
            };
            keys.push((s.value.to_str(), s.range()));
        }
        return JoinOn::Keys(keys);
    }
    JoinOn::Expression
}

fn check_join_keys(
    left: &SchemaView<'_>,
    right: &SchemaView<'_>,
    on: &JoinOn<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let JoinOn::Keys(keys) = on else { return };
    for (key, range) in keys {
        if !left.has_field(key) {
            diagnostics.push(Diagnostic::at(
                Severity::Error,
                "D0060",
                format!(
                    "Join key '{key}' does not exist on the left side ({}).",
                    left.display_name(),
                ),
                range.start(),
                source,
                line_index,
            ));
        }
        if !right.has_field(key) {
            diagnostics.push(Diagnostic::at(
                Severity::Error,
                "D0060",
                format!(
                    "Join key '{key}' does not exist on the right side ({}).",
                    right.display_name(),
                ),
                range.start(),
                source,
                line_index,
            ));
        }
    }
}

/// Result schema of a join, given the on= clause.
///
/// - For `on=[key, …]`: keys appear once (left's value); other columns are
///   concatenated, left first, right second, with shared non-key column names
///   silently kept once (left wins). This isn't fully faithful to Spark
///   (Spark would actually keep both with `df.col`-style disambiguation), but
///   it matches what most pipelines do in practice.
/// - For `on=None` (no on-clause): same as crossJoin — concatenate with
///   left-wins dedup.
/// - For `on=Expression`: same as crossJoin — concatenate with left-wins
///   dedup (we couldn't determine which keys to dedup, so we keep everything).
fn apply_join<'a>(
    left: &SchemaView<'a>,
    right: &SchemaView<'a>,
    on: &JoinOn<'_>,
) -> SchemaView<'a> {
    let dedup_set: HashSet<&str> = match on {
        JoinOn::Keys(keys) => keys.iter().map(|(n, _)| *n).collect(),
        _ => HashSet::new(),
    };
    let mut result: Vec<&'a str> = left.field_names();
    for f in right.field_names() {
        // The join key(s) are already in result from the left side.
        if dedup_set.contains(f) {
            continue;
        }
        // Non-key shared names: left wins.
        if !result.contains(&f) {
            result.push(f);
        }
    }
    SchemaView::Derived(result)
}

/// Schema concatenation for crossJoin: every field from both sides; shared
/// names are kept once (left wins).
fn apply_concat<'a>(left: &SchemaView<'a>, right: &SchemaView<'a>) -> SchemaView<'a> {
    let mut result: Vec<&'a str> = left.field_names();
    for f in right.field_names() {
        if !result.contains(&f) {
            result.push(f);
        }
    }
    SchemaView::Derived(result)
}

// ---------------------------------------------------------------------------
// col() reference discovery
// ---------------------------------------------------------------------------

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
