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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use ruff_python_ast::{Expr, ExprAttribute, ExprCall, Stmt, StmtFunctionDef};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{self, DataFrameAnnotation, SlotLabel, TypedSlot};
use crate::diagnostics::{Diagnostic, Severity};
use crate::registry::Registry;
use crate::schema::{FieldPathResult, Schema, SchemaView, resolve_path, suggest_field_name};
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
        "select" | "drop" | "dropDuplicates" | "groupBy" | "cube" | "rollup" => {
            Some(ColumnMethodShape::AllColumnName)
        }
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
    /// Sites where `col("name")` (or the equivalent string-arg form) is
    /// resolved against a known schema. Always populated during analysis;
    /// the LSP layer drains this to power hover and go-to-definition for
    /// column references. The diagnostic path simply ignores it.
    ///
    /// Held in a `RefCell` so the analysis pass can push to it through an
    /// otherwise-immutable `&BodyContext` — keeps the inner functions
    /// from needing `&mut` signatures everywhere.
    column_refs: RefCell<Vec<ColumnRefTrace<'a>>>,
    /// Local-variable DataFrame bindings discovered during body analysis.
    /// Each entry records the assignment-target name range and the schema
    /// the name ended up bound to. The LSP layer uses this to power
    /// hover on the LHS of `x = raw.select(...)` and on uses of `x`
    /// elsewhere in the function body, plus completion on `x.<cursor>`.
    local_bindings: RefCell<Vec<LocalBindingTrace<'a>>>,
    /// How many nested `transform`-body inferences deep this context is.
    /// `0` for a context built to analyze a function directly; bumped by
    /// one each time [`infer_transform_output`] recurses into a transform
    /// function's body. Caps runaway recursion on a `transform` cycle.
    infer_depth: u32,
}

/// Recursion ceiling for `transform`-body inference — deep enough for any
/// realistic pipeline-step nesting, shallow enough to stop a cycle.
const MAX_INFER_DEPTH: u32 = 8;

/// One `col("name")` (or string-arg) site captured during body analysis,
/// with the schema that was active at the time the column was resolved.
///
/// The schema is what the user is *thinking against* at that site — i.e.
/// the immediate receiver of the surrounding method call. For
/// `raw.filter(col("a") > 0).select(col("b"))`, both `col("a")` and
/// `col("b")` carry `raw`'s schema (filter preserves shape).
#[derive(Debug, Clone)]
pub struct ColumnRefTrace<'a> {
    pub range: TextRange,
    pub name: &'a str,
    pub schema: SchemaView<'a>,
}

/// One local-variable DataFrame binding captured during body analysis.
///
/// `name_range` is the source range of the assignment target on the LHS
/// (`x` in `x = raw.select(...)`); LSP hover anchors on this so the user
/// gets a popup when their cursor is on the variable name. `schema` is
/// what the value evaluates to at the assignment site.
#[derive(Debug, Clone)]
pub struct LocalBindingTrace<'a> {
    pub name: &'a str,
    pub name_range: TextRange,
    pub schema: SchemaView<'a>,
}

impl<'a> BodyContext<'a> {
    pub fn new(schemas: &'a [Schema<'a>], registry: &'a Registry<'a>) -> Self {
        Self {
            df_bindings: HashMap::new(),
            instance_bindings: HashMap::new(),
            schemas,
            registry,
            column_refs: RefCell::new(Vec::new()),
            local_bindings: RefCell::new(Vec::new()),
            infer_depth: 0,
        }
    }

    /// Record a `col(...)`-style column reference and the schema it was
    /// resolved against. Called from inside the analysis pass; the LSP
    /// layer drains the collected refs with [`take_column_refs`].
    pub fn record_column_ref(&self, range: TextRange, name: &'a str, schema: SchemaView<'a>) {
        self.column_refs.borrow_mut().push(ColumnRefTrace {
            range,
            name,
            schema,
        });
    }

    /// Record a local DataFrame binding (`x = raw.select(...)`). Called
    /// from `check_function_body` when an assignment's RHS resolves to
    /// a known schema view.
    pub fn record_local_binding(
        &self,
        name: &'a str,
        name_range: TextRange,
        schema: SchemaView<'a>,
    ) {
        self.local_bindings.borrow_mut().push(LocalBindingTrace {
            name,
            name_range,
            schema,
        });
    }

    /// Drain all column references captured during analysis. Intended for
    /// the LSP entry points (hover, go-to-definition) that re-run analysis
    /// against a fresh context.
    pub fn take_column_refs(&self) -> Vec<ColumnRefTrace<'a>> {
        std::mem::take(&mut self.column_refs.borrow_mut())
    }

    /// Drain all local-binding traces captured during analysis.
    pub fn take_local_bindings(&self) -> Vec<LocalBindingTrace<'a>> {
        std::mem::take(&mut self.local_bindings.borrow_mut())
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

    /// Bind a local name as an instance of `class_name`. Used to thread
    /// class-method calls (`data_access.read(...)`) through the
    /// generic-inference path when the receiver was assigned locally
    /// rather than passed in as a typed parameter.
    pub fn bind_instance(&mut self, name: &'a str, class_name: &'a str) {
        self.instance_bindings.insert(name, class_name);
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

/// Walk a function body, checking calls and validating the return type.
///
/// Returns the inferred schema of the function's *first* `return`
/// statement, if one could be determined — used by [`infer_transform_output`]
/// to type a `transform` function whose return isn't declared. Callers
/// that only want the diagnostics can ignore the result.
pub fn check_function_body<'a>(
    func: &DiscoveredFunction<'a>,
    declared_return: Option<&'a Schema<'a>>,
    ctx: &mut BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let mut inferred_return: Option<SchemaView<'a>> = None;
    for stmt in &func.def.body {
        match stmt {
            Stmt::Assign(a) => {
                let schema = analyze_expr(&a.value, ctx, source, line_index, diagnostics);
                if let Some(schema) = schema {
                    for target in &a.targets {
                        if let Some(name) = target.as_name_expr() {
                            ctx.bind_df(name.id.as_str(), schema.clone());
                            ctx.record_local_binding(name.id.as_str(), name.range, schema.clone());
                        }
                    }
                } else if let Some(class_name) = class_instance_from_call(&a.value, ctx) {
                    // RHS didn't resolve to a DataFrame, but it's a
                    // `ClassName(...)` call whose target class lives in
                    // the project's registry — bind the LHS as an
                    // instance so downstream method calls route through
                    // the generic-inference path.
                    for target in &a.targets {
                        if let Some(name) = target.as_name_expr() {
                            ctx.bind_instance(name.id.as_str(), class_name);
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
                if inferred_return.is_none() {
                    inferred_return = actual.clone();
                }
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
    inferred_return
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

    let Some(target_expr) = ann.target.as_name_expr() else {
        return;
    };
    let target_name = target_expr.id.as_str();
    let target_range = target_expr.range;

    match dataframe::recognize(&ann.annotation) {
        Some(DataFrameAnnotation::Typed(schema_name)) => {
            if let Some(schema) = ctx.find_schema(schema_name) {
                let view = SchemaView::Declared(schema);
                ctx.bind_df(target_name, view.clone());
                ctx.record_local_binding(target_name, target_range, view);
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
            // Annotation isn't a DataFrame shape. If it's a bare class
            // name in the registry, bind the local as an instance of
            // that class — same as the un-annotated `x = ClassName(...)`
            // path but driven by the annotation rather than the RHS.
            if let Some(name_expr) = ann.annotation.as_name_expr() {
                let class_name = name_expr.id.as_str();
                if ctx.registry().find_class(class_name).is_some() {
                    ctx.bind_instance(target_name, class_name);
                }
            }
        }
    }
}

/// Inspect a value expression and decide whether it's a constructor
/// call for a class in the registry — `DataAccessLayer(spark)` style.
/// Used by the assignment-statement handler to bind local class
/// instances.
fn class_instance_from_call<'a>(expr: &'a Expr, ctx: &BodyContext<'a>) -> Option<&'a str> {
    let call = expr.as_call_expr()?;
    let func_name = call.func.as_name_expr()?;
    let class_name = func_name.id.as_str();
    if ctx.registry().find_class(class_name).is_some() {
        Some(class_name)
    } else {
        None
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
        // `DataSources.RAW_ORDERS` — class-qualified annotated
        // constant. Real codebases declare every data source inside a
        // `@dataclass(frozen=True) class DataSources:` body, so this
        // shape needs to resolve to the same `SchemaView::Declared`
        // that a module-level `RAW_ORDERS: DataSource[X] = ...` would.
        Expr::Attribute(a) => {
            let class_name = a.value.as_name_expr()?.id.as_str();
            let constant = ctx
                .registry()
                .find_class_constant(class_name, a.attr.id.as_str())?;
            let schema = ctx.find_schema(constant.schema_name)?;
            Some(SchemaView::Declared(schema))
        }
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

    // dathon schema-cast — `<chain>.cast(DataFrame[Schema])` re-anchors a
    // chain whose schema dathon has lost (after a pivot, an un-modeled
    // op, …) to an explicit `Schema`. Handled before the receiver is
    // resolved on purpose: the receiver is *expected* to be unknown —
    // that's the whole reason the cast is there. The `DataFrame[…]`
    // argument shape is what distinguishes this from `Column.cast("int")`
    // (whose argument is a type string, and whose receiver is a `Column`).
    if method == "cast" {
        if let Some(arg) = call.arguments.args.first() {
            if let Some(DataFrameAnnotation::Typed(name)) = dataframe::recognize(arg) {
                // Analyze the receiver for its own diagnostics; its schema
                // is discarded — the cast overrides whatever it was.
                let _ = analyze_expr(&attr.value, ctx, source, line_index, diagnostics);
                return match ctx.find_schema(name) {
                    Some(schema) => Some(SchemaView::Declared(schema)),
                    None => {
                        diagnostics.push(Diagnostic::at_range(
                            Severity::Error,
                            "D0020",
                            format!(
                                "Unknown schema '{name}' in .cast(DataFrame[…]). \
                                 Declare it as a class extending Schema.",
                            ),
                            arg.range(),
                            source,
                            line_index,
                        ));
                        None
                    }
                };
            }
        }
        // Not a `DataFrame[Schema]` argument — ordinary `Column.cast`, or
        // a form dathon doesn't model. Fall through to the default path.
    }
    // `df.transform(fn)` — Spark's chaining sugar; equivalent to `fn(df)`.
    // The result schema is `fn`'s declared return; the receiver is checked
    // against `fn`'s parameter. Reachable on an unknown receiver too (the
    // function's annotation re-types the chain regardless).
    if method == "transform" {
        return handle_transform(call, attr, ctx, source, line_index, diagnostics);
    }
    // `df.na.fill/drop/replace(...)` — the DataFrameNaFunctions methods.
    // All three reshape rows only, never columns, so the result is the
    // schema of `df` unchanged. Intercepted here, against the `.na`
    // receiver shape, so `df.na.drop("all")` isn't mistaken for
    // `df.drop("col")` (the `"all"` would be a bogus column reference).
    if matches!(method, "fill" | "drop" | "replace") {
        if let Some(inner) = attr.value.as_attribute_expr() {
            if inner.attr.id.as_str() == "na" {
                return analyze_expr(&inner.value, ctx, source, line_index, diagnostics);
            }
        }
    }

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
    if method == "selectExpr" {
        // Args are SQL expression strings, not column names. Check the
        // column references *inside* each fragment against the receiver,
        // then model the output schema so the chain keeps its shape.
        for arg in &call.arguments.args {
            if let Some(lit) = arg.as_string_literal_expr() {
                report_sql_column_refs(
                    lit.value.to_str(),
                    lit.range(),
                    &receiver,
                    source,
                    line_index,
                    diagnostics,
                );
            }
        }
        return apply_select_expr(call, &receiver);
    }
    if method == "toDF" {
        // `df.toDF("a", "b", …)` renames every column to the given names;
        // `df.toDF()` keeps the receiver's columns. With a splatted
        // `*cols` argument the names aren't statically known — fall back
        // to the receiver so the chain at least stays alive.
        let names: Vec<&'a str> = call
            .arguments
            .args
            .iter()
            .filter_map(|a| a.as_string_literal_expr().map(|s| s.value.to_str()))
            .collect();
        if names.is_empty() {
            return Some(receiver);
        }
        return Some(SchemaView::Derived(names));
    }
    if method == "pivot" {
        // `groupBy(keys).pivot("col")` — verify the pivot column exists
        // on the grouped input, then bail: the pivoted output has one
        // column per distinct value of `col`, so the result schema is
        // genuinely data-dependent. The user re-anchors the chain with
        // `.cast(DataFrame[…])`.
        if let SchemaView::Grouped { underlying, .. } = &receiver {
            if let Some(lit) = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
            {
                let name = lit.value.to_str();
                if let FieldPathResult::Missing { field, on } =
                    resolve_path(underlying.as_ref(), name, ctx.schemas())
                {
                    let suggestion = suggest_field_name(field, &on);
                    let mut message =
                        format!("Column '{field}' does not exist on {}.", on.display_name());
                    if let Some(s) = &suggestion {
                        message.push_str(&format!(" Did you mean '{s}'?"));
                    }
                    diagnostics.push(
                        Diagnostic::at_range(
                            Severity::Error,
                            "D0030",
                            message,
                            lit.range(),
                            source,
                            line_index,
                        )
                        .with_suggestion(suggestion),
                    );
                }
            }
        }
        return None;
    }
    if method == "withColumns" {
        return Some(apply_with_columns(
            call, &receiver, ctx, source, line_index, diagnostics,
        ));
    }
    if method == "withColumnsRenamed" {
        return Some(apply_with_columns_renamed(
            call, &receiver, ctx, source, line_index, diagnostics,
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
        // `filter("age > 21")` / `where("city = 'x'")` accept a SQL
        // predicate string. A string-literal arg isn't a column name —
        // it's a SQL fragment whose identifiers are checked here.
        // (`collect_col_refs` skips bare string literals, so without this
        // those references would go unchecked.)
        if matches!(method, "filter" | "where") {
            for arg in &call.arguments.args {
                if let Some(lit) = arg.as_string_literal_expr() {
                    report_sql_column_refs(
                        lit.value.to_str(),
                        lit.range(),
                        &receiver,
                        source,
                        line_index,
                        diagnostics,
                    );
                }
            }
        }
        return apply_column_method(method, &receiver, call);
    }
    if let Some(kind) = two_df_method(method) {
        return handle_two_df_method(kind, call, &receiver, ctx, source, line_index, diagnostics);
    }
    // PySpark has a handful of methods that don't alter the receiver's
    // schema — caching hints, partitioning hints, materialization
    // points. Treat them as pass-throughs so a chain like
    // `raw.persist().filter(col("x"))` keeps tracking the schema
    // instead of dying at `.persist()`.
    if is_pass_through_method(method) {
        return Some(receiver);
    }
    None
}

/// Methods that return a DataFrame with the same schema as the
/// receiver. Real PySpark code uses these all the time for caching,
/// partitioning, and materialization hints; before this iteration
/// each one quietly broke schema tracking the moment it appeared in a
/// chain.
fn is_pass_through_method(method: &str) -> bool {
    matches!(
        method,
        "persist"
            | "cache"
            | "unpersist"
            | "checkpoint"
            | "localCheckpoint"
            | "coalesce"
            | "repartition"
            | "repartitionByRange"
            | "hint"
            | "sortWithinPartitions"
            | "orderBy"
            | "sort"
            | "limit"
            | "offset"
            | "distinct"
            | "sample"
            | "alias"
            // Null-handling methods reshape rows, never columns — the
            // output schema is exactly the receiver's. Their arguments
            // are fill values / subsets, not column expressions, so
            // there's nothing to check (and treating them as
            // pass-throughs keeps the chain alive for what follows).
            | "fillna"
            | "replace"
    )
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

/// Model `df.transform(fn)` — Spark's chaining sugar, equivalent to
/// `fn(df)`. `fn` is an ordinary `DataFrame -> DataFrame` function (not a
/// UDF). The result schema is `fn`'s declared `-> DataFrame[Schema]`
/// return; inferring it from `fn`'s body when the return is undeclared
/// is a follow-up.
///
/// As a bonus, the receiver's schema is checked against `fn`'s first
/// parameter — feeding the wrong DataFrame into a named pipeline step is
/// exactly the mistake `transform`-as-a-named-step is meant to catch.
fn handle_transform<'a>(
    call: &'a ExprCall,
    attr: &'a ExprAttribute,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    // Analyze the receiver for its own diagnostics and the input check.
    // It may be unknown (`None`) — transform re-types the chain anyway.
    let receiver = analyze_expr(&attr.value, ctx, source, line_index, diagnostics);

    // The transform function — `df.transform(add_features)`. Only a bare
    // name referring to a top-level `def` is resolved; lambdas and
    // imported callables are left unmodeled.
    let func_name = call.arguments.args.first()?.as_name_expr()?.id.as_str();
    let sig = ctx.registry().find_function(func_name)?;

    // Input-compatibility check: receiver schema vs. fn's first parameter.
    if let (Some(recv), Some(first_param)) = (&receiver, sig.params.first()) {
        if let Some(DataFrameAnnotation::Typed(pname)) =
            first_param.annotation.and_then(dataframe::recognize)
        {
            if let Some(param_schema) = ctx.find_schema(pname) {
                check_transform_input(
                    recv,
                    param_schema,
                    func_name,
                    attr.value.range(),
                    source,
                    line_index,
                    diagnostics,
                );
            }
        }
    }

    // Result schema — fn's declared `-> DataFrame[Schema]` if it has one…
    if let Some(DataFrameAnnotation::Typed(rname)) =
        sig.return_annotation.and_then(dataframe::recognize)
    {
        if let Some(schema) = ctx.find_schema(rname) {
            return Some(SchemaView::Declared(schema));
        }
    }
    // …otherwise infer it by analyzing fn's body with the receiver bound
    // to fn's parameter. Needs a known receiver to feed in.
    infer_transform_output(sig.def, receiver?, ctx, source, line_index)
}

/// Infer the output schema of a `transform` function whose return type
/// isn't declared, by walking its body with the parameter bound to the
/// actual input schema — the TypeScript-style "infer the return" path.
///
/// Recursion through nested `transform` calls is bounded by
/// [`MAX_INFER_DEPTH`]. Diagnostics raised while walking the body are
/// discarded: an annotated function gets its real diagnostics from the
/// standalone analysis pass; a fully un-annotated one reached only here
/// goes unchecked, an accepted v1 gap.
fn infer_transform_output<'a>(
    func_def: &'a StmtFunctionDef,
    input: SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
) -> Option<SchemaView<'a>> {
    if ctx.infer_depth >= MAX_INFER_DEPTH {
        return None;
    }
    // fn's first positional parameter — the DataFrame it transforms.
    let params = &func_def.parameters;
    let first = params.posonlyargs.iter().chain(&params.args).next()?;
    let param_name = first.parameter.name.id.as_str();

    let mut child = BodyContext::new(ctx.schemas(), ctx.registry());
    child.infer_depth = ctx.infer_depth + 1;
    child.bind_df(param_name, input);

    let discovered = DiscoveredFunction { def: func_def };
    let mut sink: Vec<Diagnostic> = Vec::new();
    check_function_body(&discovered, None, &mut child, source, line_index, &mut sink)
}

/// Check that the DataFrame fed into `df.transform(fn)` matches `fn`'s
/// declared parameter schema. Compares column-name sets; a mismatch
/// emits `D0070`.
fn check_transform_input(
    receiver: &SchemaView<'_>,
    param: &Schema<'_>,
    func_name: &str,
    range: TextRange,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let recv_names: HashSet<&str> = receiver.field_names().into_iter().collect();
    let param_names: HashSet<&str> = param.fields().iter().map(|f| f.name).collect();
    if recv_names == param_names {
        return;
    }
    let mut missing: Vec<&str> = param_names.difference(&recv_names).copied().collect();
    let mut extra: Vec<&str> = recv_names.difference(&param_names).copied().collect();
    missing.sort();
    extra.sort();
    let message = format!(
        "transform('{func_name}') expects a DataFrame matching schema '{}', \
         but the receiver ({}) does not. Missing: [{}]; extra: [{}].",
        param.name(),
        receiver.display_name(),
        missing.join(", "),
        extra.join(", "),
    );
    diagnostics.push(Diagnostic::at_range(
        Severity::Error,
        "D0070",
        message,
        range,
        source,
        line_index,
    ));
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
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let (keys, underlying): (Vec<&'a str>, SchemaView<'a>) = match receiver {
        SchemaView::Grouped { keys, underlying } => (keys.clone(), (**underlying).clone()),
        other => (Vec::new(), other.clone()),
    };

    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    let mut outputs: Vec<&'a str> = Vec::new();
    for arg in &call.arguments.args {
        collect_col_refs(arg, ctx, &mut refs);
        report_expr_sql_refs(arg, &underlying, source, line_index, diagnostics);
        if let Some(name) = select_output_name(arg) {
            outputs.push(name);
        }
    }
    report_column_refs(&refs, &underlying, ctx, source, line_index, diagnostics);

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
    schema: &SchemaView<'a>,
    shape: &ColumnMethodShape,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for (i, arg) in call.arguments.args.iter().enumerate() {
        let role = role_at(shape, i);
        collect_arg_column_refs(arg, role, ctx, &mut refs);
        // `F.expr("…")` anywhere in the argument carries a SQL fragment;
        // its identifiers are checked against the same schema.
        report_expr_sql_refs(arg, schema, source, line_index, diagnostics);
    }
    report_column_refs(&refs, schema, ctx, source, line_index, diagnostics);
}

/// Model `df.withColumns({"a": expr, "b": expr})` (Spark 3.3+) — adds
/// one column per dict entry. The keys are the new column names; the
/// values are column expressions whose own references are checked
/// against the receiver. Result schema = receiver columns + new keys.
///
/// If the argument isn't a dict literal the added names are unknown —
/// the receiver schema is returned so the chain at least stays alive.
fn apply_with_columns<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let Some(dict) = call.arguments.args.first().and_then(Expr::as_dict_expr) else {
        return recv.clone();
    };
    let mut fields: Vec<&'a str> = recv.field_names();
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for item in &dict.items {
        if let Some(key) = item.key.as_ref().and_then(|k| k.as_string_literal_expr()) {
            let name = key.value.to_str();
            if !fields.contains(&name) {
                fields.push(name);
            }
        }
        collect_col_refs(&item.value, ctx, &mut refs);
        report_expr_sql_refs(&item.value, recv, source, line_index, diagnostics);
    }
    report_column_refs(&refs, recv, ctx, source, line_index, diagnostics);
    SchemaView::Derived(fields)
}

/// Model `df.withColumnsRenamed({"old": "new", …})` (Spark 3.4+). Each
/// key is an existing column (checked against the receiver) renamed to
/// its value. Result schema = receiver columns with the renames applied.
fn apply_with_columns_renamed<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> SchemaView<'a> {
    let Some(dict) = call.arguments.args.first().and_then(Expr::as_dict_expr) else {
        return recv.clone();
    };
    let mut renames: Vec<(&'a str, &'a str)> = Vec::new();
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for item in &dict.items {
        let (Some(key), Some(val)) = (
            item.key.as_ref().and_then(|k| k.as_string_literal_expr()),
            item.value.as_string_literal_expr(),
        ) else {
            continue;
        };
        // The old name must exist — check it like any column reference.
        refs.push((key.value.to_str(), key.range()));
        renames.push((key.value.to_str(), val.value.to_str()));
    }
    report_column_refs(&refs, recv, ctx, source, line_index, diagnostics);
    let fields: Vec<&'a str> = recv
        .field_names()
        .into_iter()
        .map(|n| {
            renames
                .iter()
                .find(|(old, _)| *old == n)
                .map_or(n, |(_, new)| *new)
        })
        .collect();
    SchemaView::Derived(fields)
}

/// Resolve each collected `(name, range)` column reference against
/// `schema`: record it for the LSP layer, and emit a `D0030` (with a
/// "did you mean" suggestion) for any that doesn't resolve. The shared
/// tail of every `col(...)`-style column-existence check.
fn report_column_refs<'a>(
    refs: &[(&'a str, TextRange)],
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for &(col_name, col_range) in refs {
        ctx.record_column_ref(col_range, col_name, schema.clone());
        if let FieldPathResult::Missing { field, on } =
            resolve_path(schema, col_name, ctx.schemas())
        {
            let suggestion = suggest_field_name(field, &on);
            let mut message = format!("Column '{field}' does not exist on {}.", on.display_name());
            if let Some(s) = &suggestion {
                message.push_str(&format!(" Did you mean '{s}'?"));
            }
            diagnostics.push(
                Diagnostic::at_range(
                    Severity::Error,
                    "D0030",
                    message,
                    col_range,
                    source,
                    line_index,
                )
                .with_suggestion(suggestion),
            );
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
                // `select("*")` — the star expands to every column of the
                // receiver, rather than naming a literal column `*`.
                if arg
                    .as_string_literal_expr()
                    .is_some_and(|s| s.value.to_str() == "*")
                {
                    fields.extend(recv.field_names());
                    continue;
                }
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
        "groupBy" | "cube" | "rollup" => {
            // None of these return a DataFrame; they return a GroupedData
            // that captures the group keys and remembers the input schema.
            // The follow-up .agg(...) call uses that to check its column
            // references and produce the final DataFrame schema. `cube`
            // and `rollup` differ from `groupBy` only in which subtotal
            // rows they emit — irrelevant to the column schema.
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

/// Model the result schema of a `df.selectExpr("…", "…")` call. Each
/// argument is a SQL expression string; the output schema is the list
/// of their result column names. `*` expands to the receiver's columns.
///
/// Returns `None` only when an argument isn't a string literal (a
/// computed expression list) — there the result schema is genuinely
/// unknowable. Checking the column references *inside* the SQL is a
/// follow-up (needs a SQL parse); this only recovers the output shape
/// so a chain doesn't die at `.selectExpr(...)`.
fn apply_select_expr<'a>(call: &'a ExprCall, recv: &SchemaView<'a>) -> Option<SchemaView<'a>> {
    let mut fields: Vec<&'a str> = Vec::new();
    for arg in &call.arguments.args {
        let item = arg.as_string_literal_expr()?.value.to_str().trim();
        if item == "*" {
            fields.extend(recv.field_names());
        } else {
            fields.push(select_expr_output_name(item));
        }
    }
    Some(SchemaView::Derived(fields))
}

/// The result column name of one `selectExpr` item: the `AS` alias if
/// present, else the last segment of a bare (dotted) identifier, else
/// the expression text verbatim (Spark auto-names it after the expr).
fn select_expr_output_name(item: &str) -> &str {
    if let Some(alias) = split_sql_alias(item) {
        return alias;
    }
    if !item.is_empty()
        && item
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return item.rsplit('.').next().unwrap_or(item);
    }
    item
}

/// The alias of a SQL `expr AS name` item, if it has one. Matches the
/// last ` as ` case-insensitively and strips quoting around the name.
fn split_sql_alias(item: &str) -> Option<&str> {
    let idx = item.to_ascii_lowercase().rfind(" as ")?;
    let alias = item[idx + 4..].trim().trim_matches(['`', '"', '\'']);
    (!alias.is_empty()).then_some(alias)
}

/// Check the column identifiers inside a SQL expression string against
/// `schema`, emitting a `D0030` for every name that doesn't resolve.
///
/// Used for `selectExpr("…")` items and string-form `filter("…")` /
/// `where("…")` predicates — the places where Spark accepts a SQL
/// fragment in lieu of a `Column` expression. The fragment is parsed
/// best-effort by [`crate::sql::column_refs`]; an unparseable one yields
/// no references rather than a spurious error.
///
/// Diagnostics anchor on `range` — the whole string literal — rather
/// than the offset of the offending identifier: the parsed names are
/// owned `String`s with no span back into the original source.
fn report_sql_column_refs(
    sql: &str,
    range: TextRange,
    schema: &SchemaView<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for name in crate::sql::column_refs(sql) {
        if schema.has_field(&name) {
            continue;
        }
        let suggestion = suggest_field_name(&name, schema);
        let mut message = format!(
            "Column '{name}' does not exist on {}.",
            schema.display_name(),
        );
        if let Some(s) = &suggestion {
            message.push_str(&format!(" Did you mean '{s}'?"));
        }
        diagnostics.push(
            Diagnostic::at_range(Severity::Error, "D0030", message, range, source, line_index)
                .with_suggestion(suggestion),
        );
    }
}

/// Walk a column expression for `F.expr("…")` / `expr("…")` calls and
/// SQL-check the fragment inside each against `schema`.
///
/// `F.expr(...)` wraps a SQL string as a `Column`, so it can appear
/// anywhere a column expression is expected — `select`, `filter`,
/// `withColumn`, `agg`, and nested inside `F.when(...)`. The walk mirrors
/// [`collect_col_refs`] so a deeply nested `expr(...)` is still reached.
///
/// This is the sibling of [`collect_col_refs`] for the SQL-string family:
/// the latter collects borrowed `col("…")` names, but `expr(...)` names
/// come from a SQL parse as owned `String`s and so are checked (and
/// reported) on the spot rather than collected.
fn report_expr_sql_refs(
    expr: &Expr,
    schema: &SchemaView<'_>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(call) = expr.as_call_expr() {
        let func_name = match call.func.as_ref() {
            Expr::Name(n) => Some(n.id.as_str()),
            Expr::Attribute(a) => Some(a.attr.id.as_str()),
            _ => None,
        };
        if func_name == Some("expr") {
            if let Some(lit) = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
            {
                report_sql_column_refs(
                    lit.value.to_str(),
                    lit.range(),
                    schema,
                    source,
                    line_index,
                    diagnostics,
                );
            }
        }
    }
    match expr {
        Expr::Call(c) => {
            report_expr_sql_refs(&c.func, schema, source, line_index, diagnostics);
            for arg in &c.arguments.args {
                report_expr_sql_refs(arg, schema, source, line_index, diagnostics);
            }
            for kw in &c.arguments.keywords {
                report_expr_sql_refs(&kw.value, schema, source, line_index, diagnostics);
            }
        }
        Expr::Attribute(a) => {
            report_expr_sql_refs(&a.value, schema, source, line_index, diagnostics);
        }
        Expr::Subscript(s) => {
            report_expr_sql_refs(&s.value, schema, source, line_index, diagnostics);
            report_expr_sql_refs(&s.slice, schema, source, line_index, diagnostics);
        }
        Expr::BinOp(b) => {
            report_expr_sql_refs(&b.left, schema, source, line_index, diagnostics);
            report_expr_sql_refs(&b.right, schema, source, line_index, diagnostics);
        }
        Expr::UnaryOp(u) => {
            report_expr_sql_refs(&u.operand, schema, source, line_index, diagnostics);
        }
        Expr::Compare(c) => {
            report_expr_sql_refs(&c.left, schema, source, line_index, diagnostics);
            for cmp in &c.comparators {
                report_expr_sql_refs(cmp, schema, source, line_index, diagnostics);
            }
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                report_expr_sql_refs(v, schema, source, line_index, diagnostics);
            }
        }
        Expr::If(if_exp) => {
            report_expr_sql_refs(&if_exp.test, schema, source, line_index, diagnostics);
            report_expr_sql_refs(&if_exp.body, schema, source, line_index, diagnostics);
            report_expr_sql_refs(&if_exp.orelse, schema, source, line_index, diagnostics);
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                report_expr_sql_refs(e, schema, source, line_index, diagnostics);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                report_expr_sql_refs(e, schema, source, line_index, diagnostics);
            }
        }
        Expr::Starred(s) => {
            report_expr_sql_refs(&s.value, schema, source, line_index, diagnostics);
        }
        _ => {}
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
            // `"*"` is the all-columns wildcard, not a literal column
            // name — never check or collect it as a reference.
            if let Some(s) = arg.as_string_literal_expr() {
                if s.value.to_str() != "*" {
                    out.push((s.value.to_str(), s.range()));
                }
                return;
            }
            if let Some(list) = arg.as_list_expr() {
                for elt in &list.elts {
                    if let Some(s) = elt.as_string_literal_expr() {
                        if s.value.to_str() != "*" {
                            out.push((s.value.to_str(), s.range()));
                        }
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

/// PySpark functions where every positional **string-literal** argument
/// is a column name (mixed args with int literals or column expressions
/// are fine — those don't match the string-literal arm). Used so that
/// `F.sum("price")`, `F.add_months("checkin", 1)`, `F.lower("city")`,
/// `F.coalesce("a", "b")`, etc. are recognized as column references and
/// checked against the surrounding schema.
///
/// Iteration 37 widened this from the aggregate-only list to cover the
/// rest of the column-y subset of `pyspark.sql.functions`. The rule
/// for adding a function: every position where a string literal is
/// LEGAL must mean "column name." Functions that take a value-shaped
/// string literal in any position are deliberately omitted to avoid
/// false positives:
///
/// - `lit("default")`, `expr("a > 1")` — string is a value / SQL.
/// - `date_format(col, "yyyy-MM-dd")` — second string is a format.
/// - `regexp_replace`, `regexp_extract`, `split`, `to_date`,
///   `to_timestamp`, `from_utc_timestamp`, `from_unixtime`,
///   `unix_timestamp`, `date_trunc`, `trunc`, `next_day`, `lpad`,
///   `rpad`, `translate`, `locate`, `instr`, `concat_ws`, `format_string`,
///   `substring_index`, `cast`, `when` — mixed.
const COLUMN_REF_FUNCTIONS: &[&str] = &[
    // Aggregation — single column or all-column args.
    "sum",
    "avg",
    "mean",
    "max",
    "min",
    "count",
    "countDistinct",
    "approx_count_distinct",
    "median",
    "percentile",
    "percentile_approx",
    "var_pop",
    "var_samp",
    "variance",
    "stddev",
    "stddev_pop",
    "stddev_samp",
    "first",
    "first_value",
    "last",
    "last_value",
    "max_by",
    "min_by",
    "collect_list",
    "collect_set",
    "skewness",
    "kurtosis",
    "corr",
    "covar_pop",
    "covar_samp",
    "grouping",
    // Window
    "row_number",
    "rank",
    "dense_rank",
    "percent_rank",
    "cume_dist",
    "ntile",
    "lag",
    "lead",
    "nth_value",
    // Window-spec builders — `Window.partitionBy("city").orderBy("amount")`.
    // Every string arg is a column name; the spec is checked against the
    // schema of the DataFrame the surrounding `.over(...)` is applied to.
    // (`orderBy` is also a DataFrame method, but that form is a method
    // call routed through `analyze_method_call`, never seen here.)
    "partitionBy",
    "orderBy",
    // Date / time — single-column extractors and arithmetic helpers
    // where any non-column arg is an int (not a string).
    "year",
    "month",
    "day",
    "dayofmonth",
    "dayofweek",
    "dayofyear",
    "hour",
    "minute",
    "second",
    "weekofyear",
    "quarter",
    "last_day",
    "date_add",
    "date_sub",
    "add_months",
    "months_between",
    "datediff",
    // String — single-column or all-column-arg helpers
    "length",
    "char_length",
    "character_length",
    "lower",
    "upper",
    "initcap",
    "trim",
    "ltrim",
    "rtrim",
    "reverse",
    "ascii",
    "soundex",
    "base64",
    "unbase64",
    "concat",
    // Math — single-column or column + int helpers
    "abs",
    "ceil",
    "ceiling",
    "floor",
    "round",
    "bround",
    "sqrt",
    "exp",
    "ln",
    "log",
    "log2",
    "log10",
    "log1p",
    "expm1",
    "pow",
    "power",
    "signum",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "degrees",
    "radians",
    "factorial",
    "hypot",
    "negative",
    "positive",
    // Null handling — every string arg is a column name.
    "isnan",
    "isnull",
    "coalesce",
    "nanvl",
    "nullif",
    "ifnull",
    "nvl",
    "nvl2",
    "least",
    "greatest",
    // Hash / misc column-y — every string is a column.
    "hash",
    "md5",
    "sha1",
    "sha2",
    "crc32",
    "monotonically_increasing_id",
    "spark_partition_id",
    "input_file_name",
    "asc",
    "asc_nulls_first",
    "asc_nulls_last",
    "desc",
    "desc_nulls_first",
    "desc_nulls_last",
    "col",
    "column",
    "size",
    "sort_array",
    "array",
    "array_distinct",
    "array_except",
    "array_intersect",
    "array_union",
    "array_max",
    "array_min",
    "array_sort",
    "explode",
    "explode_outer",
    "posexplode",
    "posexplode_outer",
    "map_keys",
    "map_values",
    "map_entries",
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
                // Descend into the callee too, so an *earlier* link in a
                // builder chain is still reached — e.g. the `partitionBy`
                // in `Window.partitionBy("city").orderBy("amount")`, which
                // lives in this call's `func`, not its arguments.
                collect_col_refs(&call.func, ctx, out);
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
