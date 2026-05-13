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

use crate::dataframe::{self, DataFrameAnnotation, SlotLabel, TypedSlot};
use crate::diagnostics::{Diagnostic, Severity};
use crate::registry::Registry;
use crate::schema::{FieldPathResult, Schema, SchemaView, resolve_path};
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
        "select" | "drop" | "dropDuplicates" | "groupBy" => Some(ColumnMethodShape::AllColumnName),
        "filter" | "where" | "dropna" => Some(ColumnMethodShape::AllExpression),
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
    df_bindings: HashMap<&'a str, SchemaView<'a>>,
    /// Function-parameter / local names that are class **instances** (not
    /// DataFrames). Maps `name` → class name (e.g. `"dal"` → `"DataAccessLayer"`).
    instance_bindings: HashMap<&'a str, &'a str>,
    schemas: &'a [Schema<'a>],
    registry: &'a Registry<'a>,
}

impl<'a> BodyContext<'a> {
    pub fn new(schemas: &'a [Schema<'a>], registry: &'a Registry<'a>) -> Self {
        Self {
            df_bindings: HashMap::new(),
            instance_bindings: HashMap::new(),
            schemas,
            registry,
        }
    }

    /// Build a body context for `func`, drawing on:
    /// - typed DataFrame slots (parameters annotated `DataFrame[X]`),
    /// - other typed parameters whose annotation is a known class name
    ///   (e.g. `dal: DataAccessLayer`).
    pub fn from_function(
        func: &'a DiscoveredFunction<'a>,
        slots: &[TypedSlot<'a>],
        schemas: &'a [Schema<'a>],
        registry: &'a Registry<'a>,
    ) -> Self {
        let mut ctx = Self::new(schemas, registry);

        for slot in slots {
            let SlotLabel::Param(name) = slot.label else {
                continue;
            };
            let DataFrameAnnotation::Typed(schema_name) = slot.kind else {
                continue;
            };
            if let Some(schema) = ctx.find_schema(schema_name) {
                ctx.bind_df(name, SchemaView::Declared(schema));
            }
        }

        // Non-DataFrame typed params — `dal: DataAccessLayer` etc. Look at
        // every positional parameter; if its annotation is a bare name and
        // that name is a known class in the registry, bind the parameter
        // name as an instance of that class.
        for pwd in func
            .def
            .parameters
            .posonlyargs
            .iter()
            .chain(&func.def.parameters.args)
            .chain(&func.def.parameters.kwonlyargs)
        {
            let p = &pwd.parameter;
            let Some(ann) = p.annotation.as_deref() else {
                continue;
            };
            let Some(name_expr) = ann.as_name_expr() else {
                continue;
            };
            let class_name = name_expr.id.as_str();
            if registry.find_class(class_name).is_some() {
                ctx.instance_bindings.insert(p.name.id.as_str(), class_name);
            }
        }

        ctx
    }

    pub fn bind_df(&mut self, name: &'a str, view: SchemaView<'a>) {
        self.df_bindings.insert(name, view);
    }

    /// Resolve a name in the body's scope as a DataFrame value, if possible.
    ///
    /// Three sources are consulted, in order:
    /// 1. local DataFrame bindings (function params + `x = …` assignments),
    /// 2. top-level annotated constants (`X: GenericClass[Schema]`), where
    ///    the constant carries the named schema regardless of the outer
    ///    generic class.
    pub fn lookup(&self, name: &str) -> Option<SchemaView<'a>> {
        if let Some(view) = self.df_bindings.get(name).cloned() {
            return Some(view);
        }
        if let Some(constant) = self.registry.find_constant(name) {
            if let Some(schema) = self.find_schema(constant.schema_name) {
                return Some(SchemaView::Declared(schema));
            }
        }
        None
    }

    /// Resolve a name as a *class instance* (not a DataFrame). Used to
    /// route method calls through the class registry.
    pub fn lookup_instance(&self, name: &str) -> Option<&'a str> {
        self.instance_bindings.get(name).copied()
    }

    pub fn find_schema(&self, name: &str) -> Option<&'a Schema<'a>> {
        self.schemas.iter().find(|s| s.name() == name)
    }

    pub fn schemas(&self) -> &'a [Schema<'a>] {
        self.schemas
    }

    pub fn registry(&self) -> &'a Registry<'a> {
        self.registry
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
                            ctx.bind_df(name.id.as_str(), schema.clone());
                        }
                    }
                }
            }
            Stmt::AnnAssign(ann) => {
                handle_ann_assign(ann, ctx, source, line_index, diagnostics);
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

/// Handle `name: DataFrame[Schema] = …` (and the no-value form).
///
/// The annotation is authoritative: if it names a known Schema, `name` is
/// bound to it in the body context, regardless of what (if anything) the
/// RHS does. This is the bridge to external sources — `dal.read(...)` and
/// similar calls return something dathon can't track, but with the
/// annotation in place the user re-enters the typed world.
///
/// The RHS, if present, is still analyzed for its own diagnostics.
fn handle_ann_assign<'a>(
    ann: &'a ruff_python_ast::StmtAnnAssign,
    ctx: &mut BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Analyze the RHS for its own diagnostics. Result is discarded — the
    // annotation wins.
    if let Some(value) = ann.value.as_deref() {
        analyze_expr(value, ctx, source, line_index, diagnostics);
    }

    let Some(target_name) = ann.target.as_name_expr().map(|n| n.id.as_str()) else {
        return;
    };

    match dataframe::recognize(&ann.annotation) {
        Some(DataFrameAnnotation::Typed(schema_name)) => {
            if let Some(schema) = ctx.find_schema(schema_name) {
                ctx.bind_df(target_name, SchemaView::Declared(schema));
            } else {
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0020",
                    format!(
                        "Unknown schema '{schema_name}' referenced in DataFrame[…]. \
                         Declare it as a class extending Schema.",
                    ),
                    ann.annotation.range(),
                    source,
                    line_index,
                ));
            }
        }
        Some(DataFrameAnnotation::NonBareName) => {
            let raw_text = &source[ann.annotation.range()];
            diagnostics.push(Diagnostic::at_range(
                Severity::Error,
                "D0021",
                format!(
                    "DataFrame schema must be a bare name; got '{raw_text}'. \
                     Subscripted/complex schema expressions are not supported in v0.1.",
                ),
                ann.annotation.range(),
                source,
                line_index,
            ));
        }
        Some(DataFrameAnnotation::Untyped) => {
            // Bare `DataFrame` — no schema to bind. Nothing to do.
        }
        None => {
            // Annotation is not DataFrame-shaped at all. Not our concern.
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
    let mut only_declared: Vec<&str> = declared_names.difference(&actual_names).copied().collect();
    let mut only_actual: Vec<&str> = actual_names.difference(&declared_names).copied().collect();
    only_declared.sort();
    only_actual.sort();

    let message = format!(
        "Return type mismatch with declared schema '{}'. \
         Missing in body: [{}]; extra in body: [{}].",
        declared.name(),
        only_declared.join(", "),
        only_actual.join(", "),
    );
    diagnostics.push(Diagnostic::at_range(
        Severity::Error,
        "D0050",
        message,
        range,
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

    // Class-instance receiver: `dal.read(...)` where `dal` is bound as an
    // instance of a known class. Look the method up on the class and do
    // generic substitution. We try this BEFORE the DataFrame-receiver
    // path because the same name can't be both — instance bindings and
    // DataFrame bindings live in disjoint maps.
    if let Some(class_name) = attr
        .value
        .as_name_expr()
        .and_then(|n| ctx.lookup_instance(n.id.as_str()))
    {
        return handle_class_method_call(
            class_name,
            method,
            call,
            ctx,
            source,
            line_index,
            diagnostics,
        );
    }

    let receiver = analyze_expr(&attr.value, ctx, source, line_index, diagnostics)?;

    if method == "agg" {
        return Some(handle_agg(
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        ));
    }
    if let Some(shape) = column_method_shape(method) {
        check_column_method_args(
            call,
            &receiver,
            &shape,
            ctx,
            source,
            line_index,
            diagnostics,
        );
        return apply_column_method(method, &receiver, call);
    }
    if let Some(kind) = two_df_method(method) {
        return handle_two_df_method(kind, call, &receiver, ctx, source, line_index, diagnostics);
    }
    None
}

/// Resolve a method call on a class instance — `dal.read(...)`.
///
/// Looks up the method on the receiver's class, and if the method is
/// generic, binds the type parameter from one of the arguments and
/// substitutes through the return annotation.
///
/// Scope cuts in v0.1:
/// - Only `def m[T](self, x: Generic[T]) -> Generic[T]`-shaped methods are
///   inferable. The return annotation must be `Generic[T]` where `T` is
///   one of the method's type parameters; the parameter annotation must
///   match the same shape against an argument whose schema we can resolve.
/// - Non-generic methods, or generic methods we can't pattern-match
///   against this shape, return `None`.
fn handle_class_method_call<'a>(
    class_name: &'a str,
    method_name: &str,
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let class_info = ctx.registry().find_class(class_name)?;
    let method = class_info.methods.get(method_name)?;

    // No type params → nothing to substitute. We don't infer non-generic
    // method results yet (would require fully resolving the static return
    // annotation, which v0.1 only does for DataFrame[Schema] forms — and
    // we already cover that path via DataFrame-receiver methods).
    if method.type_params.is_empty() {
        return None;
    }

    // Try each method parameter (skipping self) to bind one of the
    // method's type variables from the corresponding argument's schema.
    // Even a single binding is enough for the simple v0.1 shape.
    let mut subst: HashMap<&str, &Schema<'a>> = HashMap::new();
    for (i, mp) in method.params.iter().skip(1).enumerate() {
        let Some(arg) = call.arguments.args.get(i) else {
            continue;
        };
        let Some(pann) = mp.annotation else {
            continue;
        };
        if let Some(tv) = extract_type_var_from_subscript(pann, &method.type_params) {
            if let Some(schema) = arg_schema(arg, ctx, source, line_index, diagnostics) {
                subst.insert(tv, schema);
            }
        }
    }

    // Substitute through the return annotation. Expecting
    // `GenericClass[T]` where T was bound above.
    let return_ann = method.return_annotation?;
    let tv = extract_type_var_from_subscript(return_ann, &method.type_params)?;
    let schema = subst.get(tv)?;
    Some(SchemaView::Declared(schema))
}

/// Match `GenericClass[T]` where `T` is one of the supplied type variable
/// names. Returns `T` if matched.
fn extract_type_var_from_subscript<'a>(expr: &'a Expr, type_params: &[&'a str]) -> Option<&'a str> {
    let sub = expr.as_subscript_expr()?;
    let inner = sub.slice.as_name_expr()?.id.as_str();
    if type_params.contains(&inner) {
        Some(inner)
    } else {
        None
    }
}

/// What schema does a given call-argument carry?
///
/// For Name expressions, consult the body context (which already falls back
/// through top-level constants). For more complex expressions, recursively
/// analyze them as expressions and pull the schema out of a Declared view.
fn arg_schema<'a>(
    arg: &'a Expr,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<&'a Schema<'a>> {
    match analyze_expr(arg, ctx, source, line_index, diagnostics) {
        Some(SchemaView::Declared(s)) => Some(s),
        _ => None,
    }
}

/// Handle `.agg(...)` on either a `Grouped` or a regular DataFrame receiver.
///
/// Each argument expression's column references (both `col("x")` and the
/// string-arg form `F.sum("x")` for known aggregate function names) are
/// checked against the underlying schema. The output schema is the group
/// keys (if any) plus each argument's output name (from `.alias("name")`
/// or a bare column ref).
fn handle_agg<'a>(
    call: &'a ExprCall,
    receiver: &SchemaView<'a>,
    _ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let (keys, underlying): (Vec<&'a str>, &SchemaView<'a>) = match receiver {
        SchemaView::Grouped { keys, underlying } => (keys.clone(), underlying.as_ref()),
        other => (Vec::new(), other),
    };

    let mut refs: Vec<(&str, TextRange)> = Vec::new();
    let mut outputs: Vec<&'a str> = Vec::new();
    for arg in &call.arguments.args {
        collect_col_refs(arg, _ctx, &mut refs);
        if let Some(name) = select_output_name(arg) {
            outputs.push(name);
        }
    }
    for (col_name, col_range) in refs {
        if let FieldPathResult::Missing { field, on } =
            resolve_path(underlying, col_name, _ctx.schemas())
        {
            diagnostics.push(Diagnostic::at_range(
                Severity::Error,
                "D0030",
                format!("Column '{field}' does not exist on {}.", on.display_name()),
                col_range,
                source,
                line_index,
            ));
        }
    }

    let mut fields: Vec<&'a str> = keys;
    for name in outputs {
        if !fields.contains(&name) {
            fields.push(name);
        }
    }
    SchemaView::Derived(fields)
}

// ---------------------------------------------------------------------------
// Column-method checking + result inference
// ---------------------------------------------------------------------------

fn check_column_method_args<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'_>,
    shape: &ColumnMethodShape,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut refs: Vec<(&str, TextRange)> = Vec::new();
    for (i, arg) in call.arguments.args.iter().enumerate() {
        let role = role_at(shape, i);
        collect_arg_column_refs(arg, role, ctx, &mut refs);
    }
    for (col_name, col_range) in refs {
        if let FieldPathResult::Missing { field, on } =
            resolve_path(schema, col_name, ctx.schemas())
        {
            diagnostics.push(Diagnostic::at_range(
                Severity::Error,
                "D0030",
                format!("Column '{field}' does not exist on {}.", on.display_name()),
                col_range,
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
        "filter" | "where" | "dropDuplicates" | "dropna" => Some(recv.clone()),
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
        "groupBy" => {
            // groupBy doesn't return a DataFrame; it returns a GroupedData
            // that captures the group keys and remembers the input schema.
            // The follow-up .agg(...) call uses that to check its column
            // references and produce the final DataFrame schema.
            let mut keys: Vec<&'a str> = Vec::new();
            for arg in &call.arguments.args {
                if let Some(name) = column_name_arg(arg) {
                    keys.push(name);
                }
            }
            Some(SchemaView::Grouped {
                keys,
                underlying: Box::new(recv.clone()),
            })
        }
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
    ctx: &BodyContext<'a>,
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
                        collect_col_refs(elt, ctx, out);
                    }
                }
                return;
            }
            collect_col_refs(arg, ctx, out);
        }
        ArgRole::Expression => collect_col_refs(arg, ctx, out),
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
            check_union_schemas(left, &right, arg.range(), source, line_index, diagnostics);
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
    diagnostics.push(Diagnostic::at_range(
        Severity::Error,
        "D0040",
        message,
        range,
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
            diagnostics.push(Diagnostic::at_range(
                Severity::Error,
                "D0060",
                format!(
                    "Join key '{key}' does not exist on the left side ({}).",
                    left.display_name(),
                ),
                *range,
                source,
                line_index,
            ));
        }
        if !right.has_field(key) {
            diagnostics.push(Diagnostic::at_range(
                Severity::Error,
                "D0060",
                format!(
                    "Join key '{key}' does not exist on the right side ({}).",
                    right.display_name(),
                ),
                *range,
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

/// PySpark aggregate / column-y functions that take string-literal arguments
/// as column names (in addition to `col(...)` expressions). Used so that
/// `F.sum("price")` is recognized as a column reference to `"price"`.
///
/// Conservatively scoped to functions where ALL positional string-lit args
/// are column names. Functions like `F.lit("foo")` (value, not column) are
/// excluded.
const COLUMN_REF_FUNCTIONS: &[&str] = &[
    "sum",
    "avg",
    "mean",
    "max",
    "min",
    "count",
    "countDistinct",
    "median",
    "percentile_approx",
    "var_pop",
    "var_samp",
    "stddev",
    "first",
    "last",
    "max_by",
    "min_by",
    "collect_list",
    "collect_set",
];

fn collect_col_refs<'a>(
    expr: &'a Expr,
    ctx: &BodyContext<'a>,
    out: &mut Vec<(&'a str, TextRange)>,
) {
    if let Some(found) = col_reference(expr) {
        out.push(found);
        return;
    }
    // `df.X` attribute access — recognized as a column reference to `X`
    // when `df` is a Name bound to a DataFrame in the current scope. We
    // ignore which `df` is referenced (Spark would have ambiguity issues
    // for non-joined references; that's runtime's problem). The column
    // name `X` is checked against the receiver's schema.
    //
    // Importantly, this filters out things like `F.add_months(...)` —
    // `F` is not in `ctx`, so the attribute is left for the default walker
    // to descend into, and `add_months` is not collected.
    if let Some(attr) = expr.as_attribute_expr() {
        if let Some(name) = attr.value.as_name_expr() {
            if ctx.lookup(name.id.as_str()).is_some() {
                out.push((attr.attr.id.as_str(), attr.attr.range));
                return;
            }
        }
    }
    // Recognize `F.sum("x")` and similar — for the listed function names,
    // every string-literal positional arg is a column reference. Non-string
    // args are walked normally.
    if let Some(call) = expr.as_call_expr() {
        let func_name = match call.func.as_ref() {
            Expr::Name(n) => Some(n.id.as_str()),
            Expr::Attribute(a) => Some(a.attr.id.as_str()),
            _ => None,
        };
        if let Some(name) = func_name {
            if COLUMN_REF_FUNCTIONS.contains(&name) {
                for arg in &call.arguments.args {
                    if let Some(s) = arg.as_string_literal_expr() {
                        out.push((s.value.to_str(), s.range()));
                    } else {
                        collect_col_refs(arg, ctx, out);
                    }
                }
                for kw in &call.arguments.keywords {
                    collect_col_refs(&kw.value, ctx, out);
                }
                return;
            }
        }
    }
    match expr {
        Expr::Call(c) => {
            collect_col_refs(&c.func, ctx, out);
            for arg in &c.arguments.args {
                collect_col_refs(arg, ctx, out);
            }
            for kw in &c.arguments.keywords {
                collect_col_refs(&kw.value, ctx, out);
            }
        }
        Expr::Attribute(a) => collect_col_refs(&a.value, ctx, out),
        Expr::Subscript(s) => {
            collect_col_refs(&s.value, ctx, out);
            collect_col_refs(&s.slice, ctx, out);
        }
        Expr::BinOp(b) => {
            collect_col_refs(&b.left, ctx, out);
            collect_col_refs(&b.right, ctx, out);
        }
        Expr::UnaryOp(u) => collect_col_refs(&u.operand, ctx, out),
        Expr::Compare(c) => {
            collect_col_refs(&c.left, ctx, out);
            for cmp in &c.comparators {
                collect_col_refs(cmp, ctx, out);
            }
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                collect_col_refs(v, ctx, out);
            }
        }
        Expr::If(if_exp) => {
            collect_col_refs(&if_exp.test, ctx, out);
            collect_col_refs(&if_exp.body, ctx, out);
            collect_col_refs(&if_exp.orelse, ctx, out);
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                collect_col_refs(e, ctx, out);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                collect_col_refs(e, ctx, out);
            }
        }
        Expr::Starred(s) => collect_col_refs(&s.value, ctx, out),
        _ => {}
    }
}
