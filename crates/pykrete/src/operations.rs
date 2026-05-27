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

use ruff_python_ast::{
    CmpOp, Expr, ExprAttribute, ExprCall, Number, Operator, Stmt, StmtFunctionDef,
};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{self, DataFrameAnnotation, SlotLabel, TypedSlot};
use crate::diagnostics::{CheckMode, Diagnostic, Severity};
use crate::registry::{MethodParam, ParamKind, Registry};
use crate::schema::{
    DerivedField, FieldPathResult, FieldResolution, Schema, SchemaView, resolve_path,
    suggest_field_name,
};
use crate::types::ColumnType;
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
    /// `intersect`, `intersectAll`, `subtract`, `exceptAll` — set
    /// operations that all return a DataFrame with the same schema as
    /// the receiver and require the other side's schema to match.
    /// (Spark's docs also reference `except`, but Python's `except`
    /// keyword makes that name unavailable as an attribute access —
    /// real PySpark code uses `exceptAll`/`subtract`.)
    /// We don't distinguish which one here because the check is the
    /// same shape (set equality on the column-name set) and the
    /// diagnostic message reads the same.
    SetOp,
    Join,
    CrossJoin,
}

fn column_method_shape(method: &str) -> Option<ColumnMethodShape> {
    match method {
        "select" | "drop" | "dropDuplicates" | "groupBy" | "groupby" | "cube" | "rollup" => {
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
        "intersect" | "intersectAll" | "subtract" | "exceptAll" => Some(TwoDfMethod::SetOp),
        "join" => Some(TwoDfMethod::Join),
        "crossJoin" => Some(TwoDfMethod::CrossJoin),
        _ => None,
    }
}

/// Whether `method` is a `DataFrameReader` terminal that lands a
/// DataFrame (the `<format>` call at the end of a `spark.read.<format>(...)`
/// chain, or `.load(...)` from the builder form). The result schema is
/// genuinely runtime data — pykrete returns Unknown and expects the user
/// to re-anchor via `.cast(DataFrame[X])` or a typed variable annotation.
fn is_dataframe_reader_format(method: &str) -> bool {
    matches!(
        method,
        "parquet" | "csv" | "json" | "orc" | "text" | "table" | "load" | "jdbc" | "xml"
    )
}

/// Whether `expr` is a `DataFrameReader` — the `spark.read` attribute, or
/// a builder-chain extension of it (`spark.read.format(...)`,
/// `spark.read.option(...)`, `spark.read.options(...)`,
/// `spark.read.schema(...)`). Anchored on the `.read` attribute and not on
/// any specific session-variable name — real codebases use `spark`, `ss`,
/// `session`, … and the shape is the same either way.
fn is_dataframe_reader_expr(expr: &Expr) -> bool {
    match expr {
        // `<anything>.read` — the base reader.
        Expr::Attribute(a) => a.attr.id.as_str() == "read",
        // Builder methods chain reader → reader.
        Expr::Call(call) => {
            let Some(attr) = call.func.as_attribute_expr() else {
                return false;
            };
            matches!(
                attr.attr.id.as_str(),
                "format" | "option" | "options" | "schema"
            ) && is_dataframe_reader_expr(&attr.value)
        }
        _ => false,
    }
}

/// Whether this call is an opaque Spark IO source — a `DataFrameReader`
/// terminal (`spark.read.parquet(...)`, `spark.read.format(...).load(...)`,
/// …) or a bare `spark.table(...)` / `spark.read.table(...)`. These all
/// return a DataFrame whose schema can't be inferred statically; the user
/// re-anchors with `.cast(DataFrame[X])` or `name: DataFrame[X] = …`.
///
/// Recognized centrally rather than left to fall through the
/// DataFrame-receiver path as Unknown — keeps the intent visible, and is
/// the natural seam to emit a "re-anchor your chain" hint at when we
/// have an informational-severity track. Today we just return None; a
/// `TODO(spark-read-rehint)` placeholder is left for that polish pass.
fn is_spark_opaque_source_call(call: &ExprCall) -> bool {
    let Some(attr) = call.func.as_attribute_expr() else {
        return false;
    };
    let method = attr.attr.id.as_str();
    // `spark.read.<format>(...)` / `spark.read.format(...).load(...)` /
    // `spark.read.schema(...).<format>(...)` — receiver is a reader.
    if is_dataframe_reader_format(method) && is_dataframe_reader_expr(&attr.value) {
        return true;
    }
    // `spark.table("db.x")` — SparkSession method that bypasses the reader.
    // Distinguished from `<df>.<table>` by the receiver being a Name (the
    // session variable), not an expression that could be a DataFrame.
    if method == "table" && attr.value.is_name_expr() {
        return true;
    }
    false
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
    /// Every name that has been bound locally in this body — including
    /// assignments whose RHS schema pykrete couldn't infer. D0051 consults
    /// this to skip the top-level-function check when the callee name has
    /// been shadowed locally, even if we can't say what schema it now
    /// refers to.
    ///
    /// Held in a `RefCell` so the analysis pass can record a walrus-bound
    /// name (`(x := expr)`) through an otherwise-immutable `&BodyContext`.
    local_names: RefCell<HashSet<&'a str>>,
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
    /// Method-call sites and the schema each one evaluates to. Powers
    /// completion on a chain result — `raw.select(...).<cursor>` — where
    /// the receiver is a call rather than a bound name.
    call_results: RefCell<Vec<CallResultTrace<'a>>>,
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

/// One method-call site and the schema it evaluates to. `range` is the
/// whole call expression's range — the receiver of a following `.<attr>`
/// access, which the completion layer matches against to offer the
/// chain result's columns.
#[derive(Debug, Clone)]
pub struct CallResultTrace<'a> {
    pub range: TextRange,
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
            local_names: RefCell::new(HashSet::new()),
            schemas,
            registry,
            column_refs: RefCell::new(Vec::new()),
            local_bindings: RefCell::new(Vec::new()),
            call_results: RefCell::new(Vec::new()),
            infer_depth: 0,
        }
    }

    /// Record a method-call site and the schema it produced. Drained by
    /// the LSP layer to power chain-result completion.
    pub fn record_call_result(&self, range: TextRange, schema: SchemaView<'a>) {
        self.call_results
            .borrow_mut()
            .push(CallResultTrace { range, schema });
    }

    /// Drain all call-result traces captured during analysis.
    pub fn take_call_results(&self) -> Vec<CallResultTrace<'a>> {
        std::mem::take(&mut self.call_results.borrow_mut())
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
            let view = match slot.kind {
                DataFrameAnnotation::Typed(schema_name) => {
                    ctx.find_schema(schema_name).map(SchemaView::Declared)
                }
                // `DataFrame[Pick[…]]` / `Omit[…]` / `Merge[…]` —
                // resolve the derived-schema expression to a view.
                DataFrameAnnotation::Derived(expr) => {
                    crate::schema::resolve_derived_schema(expr, ctx.schemas())
                }
                DataFrameAnnotation::Untyped | DataFrameAnnotation::NonBareName => None,
            };
            if let Some(view) = view {
                ctx.bind_df(name, view);
            }
        }

        // Non-DataFrame typed params — `dal: DataAccessLayer` etc. Look at
        // every positional parameter; if its annotation is a bare name and
        // that name is a known class in the registry, bind the parameter
        // name as an instance of that class. Every parameter (typed or
        // not) is also tracked as a local name so a param-shadowed
        // top-level function doesn't get D0051-checked.
        for pwd in func
            .def
            .parameters
            .posonlyargs
            .iter()
            .chain(&func.def.parameters.args)
            .chain(&func.def.parameters.kwonlyargs)
        {
            let p = &pwd.parameter;
            ctx.mark_local(p.name.id.as_str());
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
        if let Some(vararg) = func.def.parameters.vararg.as_deref() {
            ctx.mark_local(vararg.name.id.as_str());
        }
        if let Some(kwarg) = func.def.parameters.kwarg.as_deref() {
            ctx.mark_local(kwarg.name.id.as_str());
        }

        ctx
    }

    pub fn bind_df(&mut self, name: &'a str, view: SchemaView<'a>) {
        self.df_bindings.insert(name, view);
        self.mark_local(name);
    }

    /// Mark `name` as locally bound — even when the RHS schema is unknown.
    /// Used by D0051 to spot a local rebind that shadows a top-level
    /// function: the call resolves to the local at runtime, so the
    /// top-level signature shouldn't be consulted.
    ///
    /// `&self` rather than `&mut self` so the analysis pass can mark a
    /// walrus-bound name through an otherwise-immutable context — the
    /// underlying set lives in a `RefCell`.
    // TODO(d0051-nested-block-shadowing): the driver only walks top-level
    // statements in the function body, so an assignment inside an `if` /
    // `for` / `with` / `try` block doesn't mark its names as local. A
    // shadowing assignment in a nested block followed by a call in the
    // same (or deeper) block will still fall through to the top-level
    // function signature.
    pub fn mark_local(&self, name: &'a str) {
        self.local_names.borrow_mut().insert(name);
    }

    /// Walk an assignment-target expression and mark every name it binds
    /// as locally shadowed. Handles plain names, tuple/list unpack,
    /// starred targets, and parenthesized targets; ignores subscript and
    /// attribute targets (those mutate an existing object — they don't
    /// introduce a new local name).
    pub fn mark_local_target(&self, expr: &'a Expr) {
        match expr {
            Expr::Name(n) => self.mark_local(n.id.as_str()),
            Expr::Tuple(t) => {
                for elt in &t.elts {
                    self.mark_local_target(elt);
                }
            }
            Expr::List(l) => {
                for elt in &l.elts {
                    self.mark_local_target(elt);
                }
            }
            Expr::Starred(s) => self.mark_local_target(&s.value),
            _ => {}
        }
    }

    /// Whether `name` has been bound locally in this body.
    pub fn is_locally_bound(&self, name: &str) -> bool {
        self.local_names.borrow().contains(name)
    }

    /// Bind a local name as an instance of `class_name`. Used to thread
    /// class-method calls (`data_access.read(...)`) through the
    /// generic-inference path when the receiver was assigned locally
    /// rather than passed in as a typed parameter.
    pub fn bind_instance(&mut self, name: &'a str, class_name: &'a str) {
        self.instance_bindings.insert(name, class_name);
        self.mark_local(name);
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
        if let Some(constant) = self.registry.find_constant(name)
            && let Some(schema) = self.find_schema(constant.schema_name)
        {
            return Some(SchemaView::Declared(schema));
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

    /// The bundle the type-inference engine needs — declared schemas and
    /// the UDF/function registry.
    fn type_ctx(&self) -> TypeCtx<'a> {
        TypeCtx {
            schemas: self.schemas,
            registry: self.registry,
        }
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
    declared_return: Option<SchemaView<'a>>,
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
                // Always walk every target to mark its names as locally
                // bound — covers plain names, tuple/list unpack, and
                // starred targets. Schema/instance binding below is the
                // plain-name single-target case; tuple unpack falls
                // through to mark-local-only.
                for target in &a.targets {
                    ctx.mark_local_target(target);
                }
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
                if let (Some(declared), Some(actual)) = (declared_return.as_ref(), actual.as_ref())
                {
                    check_return_type(
                        declared,
                        actual,
                        ctx.schemas(),
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
/// similar calls return something pykrete can't track, but with the
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
    ctx.mark_local(target_name);

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
        Some(DataFrameAnnotation::Derived(expr)) => {
            // `x: DataFrame[Pick[…]] = …` — a local derived-schema
            // re-annotation. Surface its validation errors, then bind
            // the resolved view.
            for (code, message, range) in
                crate::schema::derived_schema_errors(expr, ctx.schemas(), source)
            {
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    code,
                    message,
                    range,
                    source,
                    line_index,
                ));
            }
            if let Some(view) = crate::schema::resolve_derived_schema(expr, ctx.schemas()) {
                ctx.bind_df(target_name, view.clone());
                ctx.record_local_binding(target_name, target_range, view);
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

/// Conservative type compatibility for the return-type contract check.
/// Two known types count as compatible when equal, or when both are
/// numeric — `int`/`long`/`double` widening is something Spark does
/// freely and pykrete infers imprecisely (`lit(5)` could be either int
/// or long), so a numeric-to-numeric difference is not flagged.
fn types_compatible(a: &ColumnType, b: &ColumnType) -> bool {
    fn is_numeric(t: &ColumnType) -> bool {
        matches!(t, ColumnType::Int | ColumnType::Long | ColumnType::Double)
    }
    // An unknown element/key/value type is permissive — like an unknown
    // column type, it is never itself a mismatch.
    fn element_ok(a: &Option<Box<ColumnType>>, b: &Option<Box<ColumnType>>) -> bool {
        match (a, b) {
            (Some(x), Some(y)) => types_compatible(x, y),
            _ => true,
        }
    }
    // A struct field whose type is unknown is permissive, as elsewhere.
    fn field_ok(a: &Option<ColumnType>, b: &Option<ColumnType>) -> bool {
        match (a, b) {
            (Some(x), Some(y)) => types_compatible(x, y),
            _ => true,
        }
    }
    match (a, b) {
        // Nullability is transparent to the conservative check —
        // `Optional[T]` behaves as `T`. The strict mode flags a nullable
        // value declared non-null separately (`D0083`).
        (ColumnType::Nullable(x), _) => types_compatible(x, b),
        (_, ColumnType::Nullable(y)) => types_compatible(a, y),
        (ColumnType::Array(x), ColumnType::Array(y)) => element_ok(x, y),
        (ColumnType::Map(k1, v1), ColumnType::Map(k2, v2)) => {
            element_ok(k1, k2) && element_ok(v1, v2)
        }
        // Structs compare structurally — same field names in the same
        // order, each field type compatible.
        (ColumnType::Struct(xs), ColumnType::Struct(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys)
                    .all(|(x, y)| x.name == y.name && field_ok(&x.ty, &y.ty))
        }
        _ => a == b || (is_numeric(a) && is_numeric(b)),
    }
}

fn check_return_type<'a>(
    declared: &SchemaView<'a>,
    actual: &SchemaView<'a>,
    schemas: &'a [Schema<'a>],
    range: TextRange,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // How to name the declared return type in messages: a named schema
    // by name, a `Pick`/`Omit`-derived one by its column list.
    let declared_label = match declared {
        SchemaView::Declared(s) => format!("schema '{}'", s.name()),
        _ => format!("[{}]", declared.field_names().join(", ")),
    };
    let declared_names: HashSet<&str> = declared.field_names().into_iter().collect();
    let actual_names: HashSet<&str> = actual.field_names().into_iter().collect();

    // Type check — a column present in both, with confidently-known but
    // incompatible types, is a real schema-contract mismatch regardless
    // of any missing/extra columns. Both types must be known: an unknown
    // (`None`) type is permissive and never flagged.
    let mut shared: Vec<&str> = declared_names
        .intersection(&actual_names)
        .copied()
        .collect();
    shared.sort();
    for name in shared {
        if let (Some(declared_ty), Some(actual_ty)) = (
            declared.field_type(name, schemas),
            actual.field_type(name, schemas),
        ) {
            if !types_compatible(&declared_ty, &actual_ty) {
                diagnostics.push(Diagnostic::at_range(
                    Severity::Error,
                    "D0080",
                    format!(
                        "Return type mismatch: column '{name}' is declared {declared_ty} \
                         in {declared_label}, but the body produces {actual_ty}.",
                    ),
                    range,
                    source,
                    line_index,
                ));
            }
            // Strict: a nullable value flowing into a column the return
            // type declares non-nullable. Conservative mode stays quiet
            // — Spark's nullable flag is loose — so this is `min_mode:
            // Strict`, like the other strict type checks.
            if actual_ty.is_nullable() && !declared_ty.is_nullable() {
                diagnostics.push(
                    Diagnostic::at_range(
                        Severity::Warning,
                        "D0083",
                        format!(
                            "Column '{name}' is nullable in the body, but {declared_label} \
                             declares it non-nullable."
                        ),
                        range,
                        source,
                        line_index,
                    )
                    .with_min_mode(CheckMode::Strict),
                );
            }
        }
    }

    if declared_names == actual_names {
        return;
    }
    let mut only_declared: Vec<&str> = declared_names.difference(&actual_names).copied().collect();
    let mut only_actual: Vec<&str> = actual_names.difference(&declared_names).copied().collect();
    only_declared.sort();
    only_actual.sort();

    let message = format!(
        "Return type mismatch with declared {declared_label}. \
         Missing in body: [{}]; extra in body: [{}].",
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
        Expr::Named(named) => {
            // Walrus (`(target := value)`) — record the target as a local
            // binding before descending, so a subsequent call on the same
            // name within this body doesn't fall through to the
            // top-level-function signature check. Python's grammar
            // restricts the LHS to a single `Name`; anything else is
            // a syntax error.
            if let Some(target) = named.target.as_name_expr() {
                ctx.mark_local(target.id.as_str());
            }
            analyze_expr(&named.value, ctx, source, line_index, diagnostics)
        }
        Expr::Call(call) => {
            // Check arguments at the call site against the callee's
            // declared `DataFrame[Schema]` parameters. Side-effect only —
            // doesn't change what schema this call evaluates to.
            check_call_argument_schemas(call, ctx, source, line_index, diagnostics);

            let result = analyze_method_call(call, ctx, source, line_index, diagnostics);
            // Record the call's result schema so completion can offer
            // the chain's columns at `<call>.<cursor>`.
            if let Some(schema) = &result {
                ctx.record_call_result(call.range(), schema.clone());
            }
            result
        }
        Expr::Attribute(a) => {
            // `DataSources.RAW_ORDERS` — class-qualified annotated
            // constant. Real codebases declare every data source inside a
            // `@dataclass(frozen=True) class DataSources:` body, so this
            // shape resolves to the same `SchemaView::Declared` a
            // module-level `RAW_ORDERS: DataSource[X] = ...` would.
            if let Some(class) = a.value.as_name_expr()
                && let Some(constant) = ctx
                    .registry()
                    .find_class_constant(class.id.as_str(), a.attr.id.as_str())
                && let Some(schema) = ctx.find_schema(constant.schema_name)
            {
                return Some(SchemaView::Declared(schema));
            }
            // Otherwise this is a column access (`<chain>.colname`) — not
            // a DataFrame itself, but analyze the receiver so a call in
            // it still records its result trace for completion.
            let _ = analyze_expr(&a.value, ctx, source, line_index, diagnostics);
            None
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

    // pykrete schema-cast — `<chain>.cast(DataFrame[Schema])` re-anchors a
    // chain whose schema pykrete has lost (after a pivot, an un-modeled
    // op, …) to an explicit `Schema`. Handled before the receiver is
    // resolved on purpose: the receiver is *expected* to be unknown —
    // that's the whole reason the cast is there. The `DataFrame[…]`
    // argument shape is what distinguishes this from `Column.cast("int")`
    // (whose argument is a type string, and whose receiver is a `Column`).
    if method == "cast"
        && let Some(arg) = call.arguments.args.first()
        && let Some(DataFrameAnnotation::Typed(name)) = dataframe::recognize(arg)
    {
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
    // Not a `DataFrame[Schema]` argument — ordinary `Column.cast`, or
    // a form pykrete doesn't model. Fall through to the default path.
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
    if matches!(method, "fill" | "drop" | "replace")
        && let Some(inner) = attr.value.as_attribute_expr()
        && inner.attr.id.as_str() == "na"
    {
        let recv = analyze_expr(&inner.value, ctx, source, line_index, diagnostics)?;
        check_subset_kwarg(call, &recv, ctx, source, line_index, diagnostics);
        // `na.fill` / `na.drop` clear nulls from the affected
        // columns; `na.replace` doesn't.
        return Some(if matches!(method, "fill" | "drop") {
            strip_nullability(&recv, ctx.schemas())
        } else {
            recv
        });
    }

    // `spark.sql("SELECT … FROM …")` — infer the result schema from the
    // query's projection columns. A query pykrete can't read cleanly (a
    // `WITH` clause, a `*` wildcard, an unaliased computed column)
    // yields no schema; the user annotates the result in that case.
    // `.sql(...)` is a SparkSession method, so the receiver isn't a
    // DataFrame — this is handled before the DataFrame-receiver path.
    if method == "sql" {
        if let Some(lit) = call
            .arguments
            .args
            .first()
            .and_then(|a| a.as_string_literal_expr())
            && let Some(cols) = crate::sql::select_projection_columns(lit.value.to_str())
        {
            return Some(SchemaView::derived_untyped(cols));
        }
        return None;
    }

    // `spark.read.<format>(path)` / `spark.read.format(...).load(...)` /
    // `spark.table(name)` — opaque IO sources. The schema is genuinely
    // runtime data; we return Unknown and rely on `.cast(DataFrame[X])`
    // or a typed variable annotation to re-anchor the chain.
    //
    // Recognized explicitly (rather than left to fall through the
    // DataFrame-receiver path) so the intent is visible in the code and
    // so this site is the natural place to emit a re-anchor hint once
    // we have an informational-severity track.
    // TODO(spark-read-rehint): when an informational/hint diagnostic
    // channel exists, emit a hint here when this call's result isn't
    // re-anchored downstream by `.cast(DataFrame[X])` or a typed
    // variable annotation.
    if is_spark_opaque_source_call(call) {
        return None;
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

    // Several methods take a `subset=` of column names — check it
    // uniformly, before the per-method dispatch.
    if matches!(
        method,
        "fillna" | "dropna" | "dropDuplicates" | "drop_duplicates" | "replace"
    ) {
        check_subset_kwarg(call, &receiver, ctx, source, line_index, diagnostics);
    }

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
        return apply_select_expr(call, &receiver, ctx.schemas());
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
        // `toDF` renames positionally — the i-th new name takes the
        // i-th receiver column's type.
        let recv_fields = receiver.typed_fields(ctx.schemas());
        let fields: Vec<DerivedField<'a>> = names
            .into_iter()
            .enumerate()
            .map(|(i, name)| DerivedField {
                name,
                ty: recv_fields.get(i).and_then(|f| f.ty.clone()),
            })
            .collect();
        return Some(SchemaView::Derived(fields));
    }
    // `groupBy(keys).max("col")` / `.min("col")` / `.sum("col")` /
    // `.mean("col")` / `.avg("col")` — Spark's GroupedData aggregate
    // shortcuts, equivalent to `groupBy(keys).agg(F.<method>(col))`.
    // Each string-literal arg is a column name on the underlying schema;
    // dotted paths into nested structs (`"b.c"`) are walked through
    // `resolve_path`, same as `col("b.c")`. The chain bails after — the
    // output column name is auto-generated (`max(col)`, ...) and not
    // statically modeled here; users re-anchor with `.cast(...)` if they
    // need to continue.
    if matches!(method, "max" | "min" | "sum" | "mean" | "avg")
        && let SchemaView::Grouped { underlying, .. } = &receiver
    {
        for arg in &call.arguments.args {
            if let Some(lit) = arg.as_string_literal_expr() {
                let name = lit.value.to_str();
                if let FieldPathResult::Missing { field, on } =
                    resolve_path(underlying.as_ref(), name, ctx.schemas())
                {
                    let suggestion = on.as_ref().and_then(|v| suggest_field_name(field, v));
                    let on_phrase = on
                        .as_ref()
                        .map_or_else(|| "the nested struct".to_string(), SchemaView::display_name);
                    let mut message = format!("Column '{field}' does not exist on {on_phrase}.");
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
    if method == "pivot" {
        // `groupBy(keys).pivot("col")` — verify the pivot column exists
        // on the grouped input, then bail: the pivoted output has one
        // column per distinct value of `col`, so the result schema is
        // genuinely data-dependent. The user re-anchors the chain with
        // `.cast(DataFrame[…])`.
        if let SchemaView::Grouped { underlying, .. } = &receiver
            && let Some(lit) = call
                .arguments
                .args
                .first()
                .and_then(|a| a.as_string_literal_expr())
        {
            let name = lit.value.to_str();
            if let FieldPathResult::Missing { field, on } =
                resolve_path(underlying.as_ref(), name, ctx.schemas())
            {
                let suggestion = on.as_ref().and_then(|v| suggest_field_name(field, v));
                let on_phrase = on
                    .as_ref()
                    .map_or_else(|| "the nested struct".to_string(), SchemaView::display_name);
                let mut message = format!("Column '{field}' does not exist on {on_phrase}.");
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
        return None;
    }
    if method == "withColumns" {
        return Some(apply_with_columns(
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        ));
    }
    if method == "withColumnsRenamed" {
        return Some(apply_with_columns_renamed(
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
        return apply_column_method(method, &receiver, call, ctx.type_ctx());
    }
    if let Some(kind) = two_df_method(method) {
        return handle_two_df_method(
            kind,
            method,
            call,
            &receiver,
            ctx,
            source,
            line_index,
            diagnostics,
        );
    }
    // PySpark has a handful of methods that don't alter the receiver's
    // schema — caching hints, partitioning hints, materialization
    // points. Treat them as pass-throughs so a chain like
    // `raw.persist().filter(col("x"))` keeps tracking the schema
    // instead of dying at `.persist()`.
    // `fillna` substitutes a value for nulls — the filled columns are no
    // longer nullable. (`dropna` is a column method, handled there.)
    if method == "fillna" {
        return Some(strip_nullability(&receiver, ctx.schemas()));
    }
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
            // `replace` swaps values but doesn't specifically clear
            // nulls, so the schema — nullability included — is the
            // receiver's. (`fillna` / `dropna` *do* clear nulls; they
            // are handled explicitly, not as pass-throughs.)
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
    // TODO(d0051-class-method-vararg): positional indexing here walks
    // every parameter in declaration order, so a `*args` / kw-only /
    // `**kwargs` slot interleaved before a generic-bearing param will
    // mis-align the arg-to-param pairing. No test exercises this yet
    // (real generic class methods are `def m[T](self, x: G[T]) -> G[T]`-
    // shaped); revisit when a multi-param generic method needs the
    // segment-aware matcher.
    for (i, mp) in method.params.iter().skip(1).enumerate() {
        let Some(arg) = call.arguments.args.get(i) else {
            continue;
        };
        let Some(pann) = mp.annotation else {
            continue;
        };
        if let Some(tv) = extract_type_var_from_subscript(pann, &method.type_params)
            && let Some(schema) = arg_schema(arg, ctx, source, line_index, diagnostics)
        {
            subst.insert(tv, schema);
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
    if let (Some(recv), Some(first_param)) = (&receiver, sig.params.first())
        && let Some(DataFrameAnnotation::Typed(pname)) =
            first_param.annotation.and_then(dataframe::recognize)
        && let Some(param_schema) = ctx.find_schema(pname)
    {
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

    // Result schema — fn's declared `-> DataFrame[Schema]` if it has one…
    if let Some(DataFrameAnnotation::Typed(rname)) =
        sig.return_annotation.and_then(dataframe::recognize)
        && let Some(schema) = ctx.find_schema(rname)
    {
        return Some(SchemaView::Declared(schema));
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

/// Check the arguments of a free-function call against the callee's
/// declared `DataFrame[Schema]` parameters. The mirror of return-type
/// checking (`D0050`), one frame earlier: it catches `f(refunds)` when
/// `f` was declared `def f(sales: DataFrame[Sale])` and `refunds`
/// resolves to a different schema. Emits `D0051`
/// (`argumentColumnsMismatch`).
///
/// Method calls (`df.method(...)`) go through `analyze_method_call`;
/// `df.transform(fn)` has its own input check in `handle_transform`.
/// This function only fires when `call.func` is a bare `Name` and that
/// name resolves to a user-defined top-level function in the registry.
///
/// An argument whose schema can't be inferred (untyped local, an
/// opaque `spark.read.json(...)` chain) is silently skipped — the same
/// degrade-rather-than-false-flag stance the rest of the checker takes.
fn check_call_argument_schemas<'a>(
    call: &'a ExprCall,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(name_expr) = call.func.as_name_expr() else {
        return;
    };
    // A local binding with the same name shadows the top-level function:
    // the call resolves to the local at runtime, so checking the top-level
    // signature would be a false positive. We check the broader
    // `is_locally_bound` set rather than `lookup` because the shadowing
    // assignment's RHS may have an un-inferred schema, yet still binds
    // the name.
    if ctx.is_locally_bound(name_expr.id.as_str()) {
        return;
    }
    let Some(sig) = ctx.registry().find_function(name_expr.id.as_str()) else {
        return;
    };

    // Walk positional args in lockstep with positional-or-regular params;
    // overflow goes to `*args` if the function declared one. A positional
    // arg that lands past `*` with no vararg slot is a Python TypeError —
    // skip it rather than emit a misleading schema diagnostic.
    //
    // `consumed_positional` records which slots a positional arg filled
    // (by parameter name). The kwarg loop consults it to skip any kwarg
    // whose name targets an already-filled slot: that's Python's
    // `TypeError: got multiple values for argument`, and firing D0051 a
    // second time on the same slot is double-diagnosis, not double bugs.
    let mut pos_idx = 0;
    let mut consumed_positional: HashSet<&str> = HashSet::new();
    for arg in &call.arguments.args {
        let matched = match_positional(&sig.params, &mut pos_idx);
        if let Some(param) = matched {
            // VarPositional is sticky — every remaining positional arg
            // lands in it. Don't record it as "consumed" for the kwarg
            // dedupe, since `*args` and `**kwargs` are independent slots.
            if !matches!(param.kind, ParamKind::VarPositional) {
                consumed_positional.insert(param.name);
            }
            check_one_call_arg(arg, param, ctx, source, line_index, diagnostics);
        }
    }

    // Keyword arguments — match by name against regular / kw-only params;
    // unrecognized names fall through to `**kwargs` if present. A keyword
    // arg whose name targets a positional-only param is a Python TypeError —
    // skip it. Likewise, skip any kwarg whose name targets a slot already
    // filled positionally (Python rejects this as a TypeError, and we'd
    // otherwise re-diagnose the same parameter).
    for kw in &call.arguments.keywords {
        let Some(name) = kw.arg.as_ref().map(|n| n.id.as_str()) else {
            continue;
        };
        if consumed_positional.contains(name) {
            continue;
        }
        let named = sig.params.iter().find(|p| {
            p.name == name && matches!(p.kind, ParamKind::Regular | ParamKind::KeywordOnly)
        });
        let param = named.or_else(|| sig.params.iter().find(|p| p.kind == ParamKind::VarKeyword));
        if let Some(param) = param {
            check_one_call_arg(&kw.value, param, ctx, source, line_index, diagnostics);
        }
    }
}

/// Pick the parameter slot a positional argument should bind to,
/// advancing `cursor` past consumed positional-or-regular slots. A
/// `*args` slot is sticky — every remaining positional arg lands in it.
/// Returns `None` when the call has overflowed past `*` into kw-only
/// territory: Python itself would TypeError, so a schema diagnostic
/// would be the wrong cause to blame.
fn match_positional<'a, 'p>(
    params: &'p [MethodParam<'a>],
    cursor: &mut usize,
) -> Option<&'p MethodParam<'a>> {
    let p = params.get(*cursor)?;
    match p.kind {
        ParamKind::PositionalOnly | ParamKind::Regular => {
            *cursor += 1;
            Some(p)
        }
        ParamKind::VarPositional => Some(p),
        ParamKind::KeywordOnly | ParamKind::VarKeyword => {
            *cursor = params.len();
            None
        }
    }
}

fn check_one_call_arg<'a>(
    arg: &'a Expr,
    param: &MethodParam<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Only checks `DataFrame[Schema]` parameters — other annotation
    // shapes (an unannotated param, a non-DataFrame type, `DataFrame`
    // without a schema, a derived expression like `DataFrame[Pick[...]]`)
    // fall through.
    let Some(DataFrameAnnotation::Typed(pname)) = param.annotation.and_then(dataframe::recognize)
    else {
        return;
    };
    let Some(param_schema) = ctx.find_schema(pname) else {
        return;
    };

    // Resolve the argument's schema. Pass the real diagnostics sink so
    // that nested calls inside this argument (e.g. `f(f(b))`) get their
    // own D0051s reported — the normal walker doesn't recurse into call
    // arguments, so this is the only path that visits them.
    let Some(arg_schema) = analyze_expr(arg, ctx, source, line_index, diagnostics) else {
        return;
    };

    let arg_names: HashSet<&str> = arg_schema.field_names().into_iter().collect();
    let param_names: HashSet<&str> = param_schema.fields().iter().map(|f| f.name).collect();
    if arg_names == param_names {
        return;
    }
    let mut missing: Vec<&str> = param_names.difference(&arg_names).copied().collect();
    let mut extra: Vec<&str> = arg_names.difference(&param_names).copied().collect();
    missing.sort();
    extra.sort();
    let message = format!(
        "Argument schema mismatch for parameter '{}': expected DataFrame[{}], \
         got {}. Missing: [{}]; extra: [{}].",
        param.name,
        param_schema.name(),
        arg_schema.display_name(),
        missing.join(", "),
        extra.join(", "),
    );
    diagnostics.push(Diagnostic::at_range(
        Severity::Error,
        "D0051",
        message,
        arg.range(),
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
    // Group keys keep their type from the underlying schema; aggregate
    // outputs are typed via `select_arg_type` (a plain column ref is
    // typed, an aggregate function result is unknown for now).
    let mut fields: Vec<DerivedField<'a>> = keys
        .iter()
        .map(|&name| DerivedField {
            name,
            ty: underlying.field_type(name, ctx.schemas()),
        })
        .collect();
    for arg in &call.arguments.args {
        collect_col_refs(arg, ctx, &mut refs);
        report_expr_sql_refs(arg, &underlying, source, line_index, diagnostics);
        report_expr_type_errors(
            arg,
            &underlying,
            ctx.type_ctx(),
            source,
            line_index,
            diagnostics,
        );
        if let Some(name) = select_output_name(arg)
            && !fields.iter().any(|f| f.name == name)
        {
            fields.push(DerivedField {
                name,
                ty: select_arg_type(arg, &underlying, ctx.type_ctx()),
            });
        }
    }
    report_column_refs(&refs, &underlying, ctx, source, line_index, diagnostics);
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
        report_expr_type_errors(arg, schema, ctx.type_ctx(), source, line_index, diagnostics);
        // Chained Column-on-Column accesses (`df.r.X`, `df["r"].X`,
        // `df.r["X"]`, `df["r"]["X"]`) that drill into a nested struct
        // column — checked separately because they don't reduce to a
        // single name in the source. Single-step refs (`df.X`, `df["X"]`)
        // are still handled by collect_arg_column_refs above.
        check_chained_field_access(arg, schema, ctx, source, line_index, diagnostics);
    }
    report_column_refs(&refs, schema, ctx, source, line_index, diagnostics);
}

/// Walks `expr` for chained DataFrame-column accesses of the form
/// `df.<f1>.<f2>...` / `df["<f1>"]["<f2>"]...` / mixed, and verifies
/// each step against the receiver schema (descending into nested
/// structs as it goes). Single-step accesses are left for the
/// existing `collect_col_refs` arm to handle.
///
/// Emits D0030 with the failing segment's range and the schema we
/// failed on (so the diagnostic for `df.r.typo` reads "Column 'typo'
/// does not exist on schema 'R'", not "...on schema 'LR'").
fn check_chained_field_access<'a>(
    expr: &'a Expr,
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Walk into every sub-expression so we catch chained accesses inside
    // bigger trees (e.g. `df.select(df.r.typo, ...)`).
    match expr {
        Expr::Attribute(_) | Expr::Subscript(_) => {
            if let Some(chain) = extract_chained_access(expr, ctx)
                && chain.len() >= 2
            {
                report_chain_against_schema(&chain, receiver, ctx, source, line_index, diagnostics);
                return;
            }
            // Single-step or a chain we couldn't parse — descend into
            // the inner value to keep searching.
            match expr {
                Expr::Attribute(a) => check_chained_field_access(
                    &a.value,
                    receiver,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                ),
                Expr::Subscript(s) => {
                    check_chained_field_access(
                        &s.value,
                        receiver,
                        ctx,
                        source,
                        line_index,
                        diagnostics,
                    );
                    check_chained_field_access(
                        &s.slice,
                        receiver,
                        ctx,
                        source,
                        line_index,
                        diagnostics,
                    );
                }
                _ => unreachable!(),
            }
        }
        Expr::Call(c) => {
            // For a method call `<recv>.<method>(...)`, c.func is
            // Attribute(<recv>, <method>). The METHOD name isn't a
            // field on `<recv>`'s schema — it's a method on the Column
            // object. Only walk INTO the receiver; don't let
            // extract_chained_access pick up the method as a final
            // step. (Without this, `df["r"].withField(...)` would
            // flag 'withField' as a missing field on schema R.)
            match c.func.as_ref() {
                Expr::Attribute(a) => check_chained_field_access(
                    &a.value,
                    receiver,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                ),
                other => check_chained_field_access(
                    other,
                    receiver,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                ),
            }
            for arg in &c.arguments.args {
                check_chained_field_access(arg, receiver, ctx, source, line_index, diagnostics);
            }
            for kw in &c.arguments.keywords {
                check_chained_field_access(
                    &kw.value,
                    receiver,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                );
            }
        }
        Expr::BinOp(b) => {
            check_chained_field_access(&b.left, receiver, ctx, source, line_index, diagnostics);
            check_chained_field_access(&b.right, receiver, ctx, source, line_index, diagnostics);
        }
        Expr::UnaryOp(u) => {
            check_chained_field_access(&u.operand, receiver, ctx, source, line_index, diagnostics)
        }
        Expr::Compare(c) => {
            check_chained_field_access(&c.left, receiver, ctx, source, line_index, diagnostics);
            for cmp in &c.comparators {
                check_chained_field_access(cmp, receiver, ctx, source, line_index, diagnostics);
            }
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                check_chained_field_access(v, receiver, ctx, source, line_index, diagnostics);
            }
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                check_chained_field_access(e, receiver, ctx, source, line_index, diagnostics);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                check_chained_field_access(e, receiver, ctx, source, line_index, diagnostics);
            }
        }
        _ => {}
    }
}

/// Extract a chain of (name, range) steps from a possibly-chained
/// attribute / subscript expression bottoming out at a DataFrame-bound
/// Name. Returns `None` if the bottom isn't a known DataFrame, or any
/// step is a non-string-literal subscript (computed). The chain is in
/// access order — outermost step last.
fn extract_chained_access<'a>(
    expr: &'a Expr,
    ctx: &BodyContext<'a>,
) -> Option<Vec<(&'a str, TextRange)>> {
    let mut parts: Vec<(&'a str, TextRange)> = Vec::new();
    let mut cursor = expr;
    loop {
        match cursor {
            Expr::Attribute(a) => {
                parts.push((a.attr.id.as_str(), a.attr.range));
                cursor = &a.value;
            }
            Expr::Subscript(s) => {
                let lit = s.slice.as_string_literal_expr()?;
                parts.push((lit.value.to_str(), lit.range()));
                cursor = &s.value;
            }
            Expr::Name(n) if ctx.lookup(n.id.as_str()).is_some() => {
                parts.reverse();
                return Some(parts);
            }
            _ => return None,
        }
    }
}

/// Walk a pre-extracted chain of segments against `receiver`, emitting
/// D0030 at the first failing step. Mirrors `resolve_path`'s descent
/// logic but consumes segments segment-by-segment (no path String
/// allocation needed).
fn report_chain_against_schema<'a>(
    chain: &[(&'a str, TextRange)],
    receiver: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut current = receiver.clone();
    for (i, &(segment, segment_range)) in chain.iter().enumerate() {
        let is_last = i + 1 == chain.len();
        if !current.has_field(segment) {
            let suggestion = suggest_field_name(segment, &current);
            let on_phrase = current.display_name();
            let mut message = format!("Column '{segment}' does not exist on {on_phrase}.");
            if let Some(s) = &suggestion {
                message.push_str(&format!(" Did you mean '{s}'?"));
            }
            diagnostics.push(
                Diagnostic::at_range(
                    Severity::Error,
                    "D0030",
                    message,
                    segment_range,
                    source,
                    line_index,
                )
                .with_suggestion(suggestion),
            );
            return;
        }
        if is_last {
            return;
        }
        // Descend into a nested Declared schema if possible. For other
        // composite field types (array, map, opaque struct) we stop
        // walking rather than risk a false positive — the single-step
        // arm already verified the field exists, so the diagnostic
        // budget for this access is spent.
        let next = match &current {
            SchemaView::Declared(s) => {
                s.fields().iter().find(|f| f.name == segment).and_then(|f| {
                    match f.resolve(ctx.schemas()) {
                        FieldResolution::ResolvedNested(nested) => Some(nested),
                        _ => None,
                    }
                })
            }
            _ => None,
        };
        match next {
            Some(nested) => current = SchemaView::Declared(nested),
            None => return,
        }
    }
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
    let mut fields: Vec<DerivedField<'a>> = recv.typed_fields(ctx.schemas());
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    for item in &dict.items {
        if let Some(key) = item.key.as_ref().and_then(|k| k.as_string_literal_expr()) {
            let name = key.value.to_str();
            let ty = infer_expr_type(&item.value, recv, ctx.type_ctx());
            // `withColumns` replaces an existing column or adds a new one.
            if let Some(existing) = fields.iter_mut().find(|f| f.name == name) {
                existing.ty = ty;
            } else {
                fields.push(DerivedField { name, ty });
            }
        }
        collect_col_refs(&item.value, ctx, &mut refs);
        report_expr_sql_refs(&item.value, recv, source, line_index, diagnostics);
        report_expr_type_errors(
            &item.value,
            recv,
            ctx.type_ctx(),
            source,
            line_index,
            diagnostics,
        );
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
    let fields: Vec<DerivedField<'a>> = recv
        .typed_fields(ctx.schemas())
        .into_iter()
        .map(|mut f| {
            if let Some((_, new)) = renames.iter().find(|(old, _)| *old == f.name) {
                f.name = new;
            }
            f
        })
        .collect();
    SchemaView::Derived(fields)
}

/// Check the `subset=` keyword argument against the receiver schema.
/// `subset` — present on `fillna`, `dropna`, `dropDuplicates`,
/// `replace`, and the `df.na.*` methods — names the columns the
/// operation applies to, as a single string or a list/tuple of them.
fn check_subset_kwarg<'a>(
    call: &'a ExprCall,
    schema: &SchemaView<'a>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(kw) = call
        .arguments
        .keywords
        .iter()
        .find(|k| k.arg.as_ref().is_some_and(|n| n.id.as_str() == "subset"))
    else {
        return;
    };
    let mut refs: Vec<(&'a str, TextRange)> = Vec::new();
    match &kw.value {
        Expr::StringLiteral(s) => refs.push((s.value.to_str(), s.range())),
        Expr::List(l) => {
            for elt in &l.elts {
                if let Some(s) = elt.as_string_literal_expr() {
                    refs.push((s.value.to_str(), s.range()));
                }
            }
        }
        Expr::Tuple(t) => {
            for elt in &t.elts {
                if let Some(s) = elt.as_string_literal_expr() {
                    refs.push((s.value.to_str(), s.range()));
                }
            }
        }
        _ => {}
    }
    report_column_refs(&refs, schema, ctx, source, line_index, diagnostics);
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
            let suggestion = on.as_ref().and_then(|v| suggest_field_name(field, v));
            let on_phrase = on
                .as_ref()
                .map_or_else(|| "the nested struct".to_string(), SchemaView::display_name);
            let mut message = format!("Column '{field}' does not exist on {on_phrase}.");
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

// ---------------------------------------------------------------------------
// Column-expression type inference
// ---------------------------------------------------------------------------

/// The project-wide context the type-inference engine needs: every
/// declared schema (for nested-struct resolution) and the registry (for
/// UDF return types). Bundled into one `Copy` value so the recursive
/// inference functions take a single argument instead of threading two.
#[derive(Clone, Copy)]
struct TypeCtx<'a> {
    schemas: &'a [Schema<'a>],
    registry: &'a Registry<'a>,
}

/// Infer the atomic type of a column expression evaluated against
/// `schema`. `None` means "couldn't determine" — a function result
/// pykrete doesn't model, a column off an un-inferred schema, an
/// unmodeled literal. `None` is permissive: it is never itself a type
/// error, only the absence of information.
/// True if `expr` is `lit(None)` / `F.lit(None)` — an explicit null
/// literal. An untyped null is rarely useful on its own; it is usually
/// `.cast(...)` to a concrete type, and the cast carries the type while
/// this carries the nullability.
fn expr_is_null_literal(expr: &Expr) -> bool {
    let Some(call) = expr.as_call_expr() else {
        return false;
    };
    let fname = match call.func.as_ref() {
        Expr::Name(n) => n.id.as_str(),
        Expr::Attribute(a) => a.attr.id.as_str(),
        _ => return false,
    };
    fname == "lit"
        && call
            .arguments
            .args
            .first()
            .is_some_and(|a| a.is_none_literal_expr())
}

fn infer_expr_type<'a>(
    expr: &Expr,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
) -> Option<ColumnType> {
    // `col("x")` / `column("x")` — the column's declared/inferred type.
    if let Some((name, _)) = col_reference(expr) {
        return schema.field_type(name, tcx.schemas);
    }
    match expr {
        Expr::Call(call) => {
            if let Some(attr) = call.func.as_attribute_expr() {
                match attr.attr.id.as_str() {
                    // `<expr>.alias("y")` / `.name("y")` — type unchanged.
                    // `<windowed>.over(w)` likewise carries the windowed
                    // expression's type through.
                    "alias" | "name" | "over" => {
                        return infer_expr_type(&attr.value, schema, tcx);
                    }
                    // `<expr>.cast("int")` / `.cast(IntegerType())`.
                    // A cast carries nullability through: a nullable
                    // input — or a `lit(None)` — casts to a nullable
                    // column of the target type.
                    "cast" => {
                        let target = call
                            .arguments
                            .args
                            .first()
                            .and_then(crate::registry::spark_type_from_expr)?;
                        let nullable = expr_is_null_literal(&attr.value)
                            || infer_expr_type(&attr.value, schema, tcx)
                                .is_some_and(|t| t.is_nullable());
                        return Some(if nullable {
                            ColumnType::Nullable(Box::new(target))
                        } else {
                            target
                        });
                    }
                    _ => {}
                }
            }
            let fname = match call.func.as_ref() {
                Expr::Name(n) => n.id.as_str(),
                Expr::Attribute(a) => a.attr.id.as_str(),
                _ => return None,
            };
            // `F.lit(value)` — the literal's own type.
            if fname == "lit" {
                return call.arguments.args.first().and_then(python_literal_type);
            }
            // A call to a user-defined UDF — its declared return type.
            if let Some(udf_ty) = tcx.registry.find_udf(fname) {
                return Some(udf_ty);
            }
            // Any other recognized `pyspark.sql.functions` call — look its
            // result type up in the catalog, resolving the first argument
            // (a column name or expression) for the input-dependent ones.
            let first_arg = call
                .arguments
                .args
                .first()
                .and_then(|a| select_arg_type(a, schema, tcx));
            function_result_type(fname, first_arg)
        }
        // A bare Python literal in column position acts as `lit(...)`.
        _ => python_literal_type(expr),
    }
}

/// The type of a `select` output column, given its argument. A bare
/// string literal is a column *name* there (not a string value), so it
/// is resolved against the receiver before falling back to
/// [`infer_expr_type`].
fn select_arg_type<'a>(arg: &Expr, recv: &SchemaView<'a>, tcx: TypeCtx<'a>) -> Option<ColumnType> {
    if let Some(s) = arg.as_string_literal_expr() {
        return recv.field_type(s.value.to_str(), tcx.schemas);
    }
    infer_expr_type(arg, recv, tcx)
}

/// The result [`ColumnType`] of a `pyspark.sql.functions` call, given
/// the function name and (for input-dependent functions) the type of
/// its first argument. `None` for functions pykrete doesn't model or
/// whose result isn't an atomic type (`collect_list` → array, …) —
/// permissive, never a type error.
fn function_result_type(name: &str, first_arg: Option<ColumnType>) -> Option<ColumnType> {
    use ColumnType::{Array, Bool, Date, Double, Int, Long, Map, String, Timestamp};
    // Functions with a fixed result type, regardless of input.
    match name {
        "count"
        | "countDistinct"
        | "count_distinct"
        | "approx_count_distinct"
        | "unix_timestamp"
        | "monotonically_increasing_id"
        | "factorial" => return Some(Long),
        "avg" | "mean" | "stddev" | "stddev_pop" | "stddev_samp" | "variance" | "var_pop"
        | "var_samp" | "skewness" | "kurtosis" | "corr" | "covar_pop" | "covar_samp"
        | "percent_rank" | "cume_dist" | "rand" | "randn" | "sqrt" | "exp" | "expm1" | "ln"
        | "log" | "log2" | "log10" | "log1p" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan"
        | "atan2" | "sinh" | "cosh" | "tanh" | "degrees" | "radians" | "cbrt" | "pow" | "power"
        | "hypot" | "signum" | "months_between" => return Some(Double),
        "length" | "char_length" | "character_length" | "ascii" | "instr" | "locate"
        | "levenshtein" | "year" | "month" | "dayofmonth" | "day" | "dayofweek" | "dayofyear"
        | "hour" | "minute" | "second" | "weekofyear" | "quarter" | "datediff" | "row_number"
        | "rank" | "dense_rank" | "ntile" | "spark_partition_id" | "size" => return Some(Int),
        "lower" | "upper" | "initcap" | "trim" | "ltrim" | "rtrim" | "reverse" | "concat_ws"
        | "substring" | "substring_index" | "regexp_replace" | "regexp_extract" | "lpad"
        | "rpad" | "translate" | "repeat" | "soundex" | "base64" | "format_string"
        | "format_number" | "hex" | "sha1" | "sha2" | "md5" => return Some(String),
        "to_date" | "current_date" | "last_day" | "next_day" | "date_add" | "date_sub"
        | "add_months" | "trunc" => return Some(Date),
        "to_timestamp" | "current_timestamp" | "date_trunc" | "from_utc_timestamp"
        | "to_utc_timestamp" => return Some(Timestamp),
        "isnull" | "isnan" => return Some(Bool),
        "split" => return Some(Array(Some(Box::new(String)))),
        "create_map" | "map_from_arrays" | "map_from_entries" | "map_concat" | "str_to_map"
        | "transform_keys" | "transform_values" | "map_filter" => {
            return Some(Map(None, None));
        }
        // Result not an atomic type pykrete models (an array of structs).
        "arrays_zip" | "map_entries" => return Some(Array(None)),
        _ => {}
    }
    // Functions whose result type depends on the first argument.
    match name {
        // Null-coalescing — the result is non-null when *any* argument
        // is. Conservatively drop nullability (this only under-reports).
        "coalesce" | "nvl" | "ifnull" => first_arg.map(|t| t.base().clone()),
        "min" | "max" | "first" | "last" | "first_value" | "last_value" | "greatest" | "least"
        | "nanvl" | "abs" | "round" | "bround" | "negative" | "positive" => first_arg,
        "ceil" | "ceiling" | "floor" => Some(Long),
        // `sum` widens an integral input to long; a double stays double.
        "sum" | "sumDistinct" | "sum_distinct" => match first_arg {
            Some(Int | Long) => Some(Long),
            Some(Double) => Some(Double),
            _ => None,
        },
        // Collection constructors — wrap the input as the element type.
        "collect_list" | "collect_set" | "array" | "array_repeat" | "sequence" => {
            Some(Array(first_arg.map(Box::new)))
        }
        // Array → array of the same element type.
        "array_distinct" | "array_sort" | "sort_array" | "array_union" | "array_except"
        | "array_intersect" | "array_remove" | "array_compact" | "shuffle" | "slice" => {
            match first_arg {
                Some(array @ Array(_)) => Some(array),
                _ => Some(Array(None)),
            }
        }
        // `flatten` peels one array layer: `array<array<T>>` → `array<T>`.
        "flatten" => match first_arg {
            Some(Array(Some(inner))) if inner.is_composite() => Some(*inner),
            _ => Some(Array(None)),
        },
        // `explode` unwraps an array to its element type. (On a map it
        // yields two columns — not a single type — so it's left `None`.)
        "explode" | "explode_outer" => match first_arg {
            Some(Array(elem)) => elem.map(|b| *b),
            _ => None,
        },
        // `element_at` indexes into a collection — array element or map
        // value type.
        "element_at" => match first_arg {
            Some(Array(elem)) => elem.map(|b| *b),
            Some(Map(_, value)) => value.map(|b| *b),
            _ => None,
        },
        // `map_keys` / `map_values` → an array of the key / value type.
        "map_keys" => match first_arg {
            Some(Map(key, _)) => Some(Array(key)),
            _ => Some(Array(None)),
        },
        "map_values" => match first_arg {
            Some(Map(_, value)) => Some(Array(value)),
            _ => Some(Array(None)),
        },
        _ => None,
    }
}

/// The pykrete type of a Python literal used as a column value.
fn python_literal_type(expr: &Expr) -> Option<ColumnType> {
    match expr {
        Expr::StringLiteral(_) => Some(ColumnType::String),
        Expr::BooleanLiteral(_) => Some(ColumnType::Bool),
        Expr::NumberLiteral(n) => match n.value {
            Number::Int(_) => Some(ColumnType::Int),
            Number::Float(_) => Some(ColumnType::Double),
            Number::Complex { .. } => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Strict-mode operator type checks (D0081 / D0082)
//
// These flag type combinations that Spark *coerces* rather than rejects —
// legal, but usually a mistake. Because the coercion is legal, they would
// be too noisy on by default; they are tagged `min_mode: Strict` so the
// driver only surfaces them under `typeCheckingMode: "strict"`.
// ---------------------------------------------------------------------------

/// Family of types that behave alike under operators.
#[derive(PartialEq, Clone, Copy)]
enum TypeFamily {
    Numeric,
    Textual,
    Boolean,
    Temporal,
    /// `array` / `map` — collections; they don't combine with atomics.
    Collection,
}

fn type_family(t: &ColumnType) -> TypeFamily {
    match t {
        ColumnType::Int | ColumnType::Long | ColumnType::Double => TypeFamily::Numeric,
        ColumnType::String => TypeFamily::Textual,
        ColumnType::Bool => TypeFamily::Boolean,
        ColumnType::Date | ColumnType::Timestamp => TypeFamily::Temporal,
        ColumnType::Array(_) | ColumnType::Map(..) | ColumnType::Struct(_) => {
            TypeFamily::Collection
        }
        // Nullability doesn't change the family — `Optional[int]` is
        // still numeric.
        ColumnType::Nullable(inner) => type_family(inner),
    }
}

/// Whether two atomic types may sensibly be compared. Same family is
/// always fine; a string-vs-temporal comparison is allowed because Spark
/// idiomatically casts the string (`col("date") > "2024-01-01"`).
fn comparable(a: &ColumnType, b: &ColumnType) -> bool {
    let (fa, fb) = (type_family(a), type_family(b));
    fa == fb
        || matches!(
            (fa, fb),
            (TypeFamily::Textual, TypeFamily::Temporal)
                | (TypeFamily::Temporal, TypeFamily::Textual)
        )
}

fn is_arithmetic_op(op: Operator) -> bool {
    matches!(
        op,
        Operator::Add
            | Operator::Sub
            | Operator::Mult
            | Operator::Div
            | Operator::FloorDiv
            | Operator::Mod
            | Operator::Pow
    )
}

fn is_value_comparison(op: CmpOp) -> bool {
    matches!(
        op,
        CmpOp::Eq | CmpOp::NotEq | CmpOp::Lt | CmpOp::LtE | CmpOp::Gt | CmpOp::GtE
    )
}

/// Walk a column expression for strict-mode operator type errors:
/// arithmetic on a string column (`D0081`) and comparisons between
/// unrelated atomic types (`D0082`). Both are emitted at
/// [`CheckMode::Strict`] — only surfaced under `typeCheckingMode: strict`.
fn report_expr_type_errors<'a>(
    expr: &Expr,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::BinOp(b) => {
            if is_arithmetic_op(b.op) {
                // A string / array / map operand can't take part in
                // arithmetic — Spark coerces a string (often to null) and
                // errors on a collection.
                let bad = [
                    infer_expr_type(&b.left, schema, tcx),
                    infer_expr_type(&b.right, schema, tcx),
                ]
                .into_iter()
                .flatten()
                .find(|t| {
                    matches!(
                        t,
                        ColumnType::String
                            | ColumnType::Array(_)
                            | ColumnType::Map(..)
                            | ColumnType::Struct(_)
                    )
                });
                if let Some(bad) = bad {
                    diagnostics.push(
                        Diagnostic::at_range(
                            Severity::Warning,
                            "D0081",
                            format!(
                                "Arithmetic operator applied to a non-numeric ({bad}) column. \
                                 Spark coerces or errors here; cast it explicitly if intended.",
                            ),
                            b.range(),
                            source,
                            line_index,
                        )
                        .with_min_mode(CheckMode::Strict),
                    );
                }
            }
            report_expr_type_errors(&b.left, schema, tcx, source, line_index, diagnostics);
            report_expr_type_errors(&b.right, schema, tcx, source, line_index, diagnostics);
        }
        Expr::Compare(c) => {
            let mut left = c.left.as_ref();
            for (op, right) in c.ops.iter().zip(&c.comparators) {
                if is_value_comparison(*op)
                    && let (Some(lt), Some(rt)) = (
                        infer_expr_type(left, schema, tcx),
                        infer_expr_type(right, schema, tcx),
                    )
                    && !comparable(&lt, &rt)
                {
                    diagnostics.push(
                        Diagnostic::at_range(
                            Severity::Warning,
                            "D0082",
                            format!(
                                "Comparison between unrelated types {lt} and {rt}. \
                                         Spark coerces them; cast explicitly if intended.",
                            ),
                            c.range(),
                            source,
                            line_index,
                        )
                        .with_min_mode(CheckMode::Strict),
                    );
                }
                left = right;
            }
            report_expr_type_errors(&c.left, schema, tcx, source, line_index, diagnostics);
            for cmp in &c.comparators {
                report_expr_type_errors(cmp, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::Call(call) => {
            report_expr_type_errors(&call.func, schema, tcx, source, line_index, diagnostics);
            for arg in &call.arguments.args {
                report_expr_type_errors(arg, schema, tcx, source, line_index, diagnostics);
            }
            for kw in &call.arguments.keywords {
                report_expr_type_errors(&kw.value, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::Attribute(a) => {
            report_expr_type_errors(&a.value, schema, tcx, source, line_index, diagnostics);
        }
        Expr::UnaryOp(u) => {
            report_expr_type_errors(&u.operand, schema, tcx, source, line_index, diagnostics);
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                report_expr_type_errors(v, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::If(if_exp) => {
            report_expr_type_errors(&if_exp.test, schema, tcx, source, line_index, diagnostics);
            report_expr_type_errors(&if_exp.body, schema, tcx, source, line_index, diagnostics);
            report_expr_type_errors(&if_exp.orelse, schema, tcx, source, line_index, diagnostics);
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                report_expr_type_errors(e, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                report_expr_type_errors(e, schema, tcx, source, line_index, diagnostics);
            }
        }
        Expr::Subscript(s) => {
            report_expr_type_errors(&s.value, schema, tcx, source, line_index, diagnostics);
        }
        Expr::Starred(s) => {
            report_expr_type_errors(&s.value, schema, tcx, source, line_index, diagnostics);
        }
        _ => {}
    }
}

fn apply_column_method<'a>(
    method: &str,
    recv: &SchemaView<'a>,
    call: &'a ExprCall,
    tcx: TypeCtx<'a>,
) -> Option<SchemaView<'a>> {
    match method {
        "select" => {
            let mut fields: Vec<DerivedField<'a>> = Vec::new();
            for arg in &call.arguments.args {
                // `select("*")` — the star expands to every column of the
                // receiver, rather than naming a literal column `*`.
                if arg
                    .as_string_literal_expr()
                    .is_some_and(|s| s.value.to_str() == "*")
                {
                    fields.extend(recv.typed_fields(tcx.schemas));
                    continue;
                }
                if let Some(name) = select_output_name(arg) {
                    fields.push(DerivedField {
                        name,
                        ty: select_arg_type(arg, recv, tcx),
                    });
                }
            }
            Some(SchemaView::Derived(fields))
        }
        "filter" | "where" | "dropDuplicates" => Some(recv.clone()),
        // `dropna` drops rows containing nulls — the surviving rows have
        // none, so the columns are no longer nullable.
        "dropna" => Some(strip_nullability(recv, tcx.schemas)),
        "drop" => {
            let drop_set: HashSet<&str> = call
                .arguments
                .args
                .iter()
                .filter_map(column_name_arg)
                .collect();
            let remaining: Vec<DerivedField<'a>> = recv
                .typed_fields(tcx.schemas)
                .into_iter()
                .filter(|f| !drop_set.contains(f.name))
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
            // The new column's type is inferred from the value
            // expression; if `new_name` already exists, `withColumn`
            // *replaces* it, so its type is updated rather than kept.
            let ty = call
                .arguments
                .args
                .get(1)
                .and_then(|v| infer_expr_type(v, recv, tcx));
            let mut fields: Vec<DerivedField<'a>> = recv.typed_fields(tcx.schemas);
            if let Some(existing) = fields.iter_mut().find(|f| f.name == new_name) {
                existing.ty = ty;
            } else {
                fields.push(DerivedField { name: new_name, ty });
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
            let fields: Vec<DerivedField<'a>> = recv
                .typed_fields(tcx.schemas)
                .into_iter()
                .map(|mut f| {
                    if f.name == old {
                        f.name = new;
                    }
                    f
                })
                .collect();
            Some(SchemaView::Derived(fields))
        }
        "groupBy" | "groupby" | "cube" | "rollup" => {
            // `groupby` is the lowercase Spark alias of `groupBy` —
            // identical semantics; PySpark accepts both and a lot of
            // real-world code (e.g. examples/src/main/python/sql/
            // arrow.py) uses the lowercase form. None of these return
            // a DataFrame; they return a GroupedData that captures
            // the group keys and remembers the input schema. The
            // follow-up .agg(...) call uses that to check its column
            // references and produce the final DataFrame schema.
            // `cube` and `rollup` differ from `groupBy` only in which
            // subtotal rows they emit — irrelevant to the column schema.
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
fn apply_select_expr<'a>(
    call: &'a ExprCall,
    recv: &SchemaView<'a>,
    schemas: &'a [Schema<'a>],
) -> Option<SchemaView<'a>> {
    let mut fields: Vec<DerivedField<'a>> = Vec::new();
    for arg in &call.arguments.args {
        let item = arg.as_string_literal_expr()?.value.to_str().trim();
        if item == "*" {
            fields.extend(recv.typed_fields(schemas));
        } else {
            let name = select_expr_output_name(item);
            // A bare identifier item (no `AS`, no operators) is a plain
            // column reference — typed from the receiver. Anything else
            // is a computed SQL expression whose type pykrete leaves open.
            let ty = if split_sql_alias(item).is_none()
                && !item.is_empty()
                && item.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                recv.field_type(name, schemas)
            } else {
                None
            };
            fields.push(DerivedField { name, ty });
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
        if func_name == Some("expr")
            && let Some(lit) = call
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

fn select_output_name(arg: &Expr) -> Option<&str> {
    if let Some(call) = arg.as_call_expr()
        && let Some(attr) = call.func.as_attribute_expr()
        && attr.attr.id.as_str() == "alias"
        && let Some(lit) = call
            .arguments
            .args
            .first()
            .and_then(|a| a.as_string_literal_expr())
    {
        return Some(lit.value.to_str());
    }
    if let Some(s) = arg.as_string_literal_expr() {
        return Some(s.value.to_str());
    }
    if let Some((name, _)) = col_reference(arg) {
        return Some(name);
    }
    if let Some(call) = arg.as_call_expr() {
        if let Some(attr) = call.func.as_attribute_expr()
            && attr.attr.id.as_str() == "cast"
        {
            return select_output_name(&attr.value);
        }
        // `F.explode("arr")` / `explode_outer(...)` — Spark names the
        // unnested column `col` when no `.alias(...)` is given.
        let fname = match call.func.as_ref() {
            Expr::Name(n) => Some(n.id.as_str()),
            Expr::Attribute(a) => Some(a.attr.id.as_str()),
            _ => None,
        };
        if matches!(fname, Some("explode" | "explode_outer")) {
            return Some("col");
        }
    }
    None
}

fn column_name_arg(arg: &Expr) -> Option<&str> {
    if let Some(s) = arg.as_string_literal_expr() {
        return Some(s.value.to_str());
    }
    if let Some((name, _)) = col_reference(arg) {
        return Some(name);
    }
    // `df.colname` attribute access — `groupBy(df.key, ...)` and
    // `drop(df.col)` both accept Column objects, not just name strings,
    // and grouping by a column carried in from a joined DataFrame is
    // idiomatic. The key is the attribute name; which DataFrame it came
    // from is irrelevant to the resulting column. Restricted to a bare
    // `Name` base so a called `F.func(...)` or a chained `a.b.c` can't be
    // mistaken for a column reference.
    if let Some(attr) = arg.as_attribute_expr()
        && attr.value.is_name_expr()
    {
        return Some(attr.attr.id.as_str());
    }
    // `df["colname"]` subscript — sibling of the attribute form above.
    // Same rationale: `drop(df["col"])`, `groupBy(df["key"], ...)` accept
    // Column objects. Restricted to `df["literal"]` (bare-name receiver,
    // string-literal slice).
    if let Some(sub) = arg.as_subscript_expr()
        && sub.value.is_name_expr()
        && let Some(s) = sub.slice.as_string_literal_expr()
    {
        return Some(s.value.to_str());
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
    /// pykrete doesn't analyze the on-clause; the result schema is the
    /// concatenation of both sides.
    Expression,
}

#[allow(clippy::too_many_arguments)] // mostly source/line_index/diagnostics plumbing
fn handle_two_df_method<'a>(
    kind: TwoDfMethod,
    method: &str,
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
        TwoDfMethod::Union | TwoDfMethod::UnionByName | TwoDfMethod::SetOp => {
            check_union_schemas(
                method,
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
            // Record each `on=` key as a column reference so the LSP
            // layer offers column completion inside `join(on="…")`.
            if let JoinOn::Keys(keys) = &on {
                for &(name, range) in keys {
                    ctx.record_column_ref(range, name, left.clone());
                }
            }
            Some(apply_join(
                left,
                &right,
                &on,
                extract_how_arg(call),
                ctx.schemas(),
            ))
        }
        TwoDfMethod::CrossJoin => Some(apply_concat(left, &right, ctx.schemas())),
    }
}

fn check_union_schemas(
    method: &str,
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
        "{method} between {} and {}: schemas differ. \
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
fn extract_on_arg(call: &ExprCall) -> Option<&Expr> {
    for kw in &call.arguments.keywords {
        if let Some(name) = kw.arg.as_ref()
            && name.id.as_str() == "on"
        {
            return Some(&kw.value);
        }
    }
    call.arguments.args.get(1)
}

/// The join strategy — which side, if any, an unmatched row leaves null.
#[derive(Clone, Copy)]
enum JoinHow {
    /// `inner` (the default), `cross`, `semi`, `anti` — no new nulls.
    Inner,
    /// `left` / `left_outer` — unmatched left rows null the right side.
    Left,
    /// `right` / `right_outer` — unmatched right rows null the left side.
    Right,
    /// `outer` / `full` / `full_outer` — either side can be null.
    Outer,
}

/// Map a Spark `how=` string to a [`JoinHow`]. Unknown / inner-like
/// strategies (`inner`, `cross`, `semi`, `anti`) all map to `Inner` —
/// none introduce nulls.
fn join_how(s: &str) -> JoinHow {
    match s.to_ascii_lowercase().replace('_', "").as_str() {
        "left" | "leftouter" => JoinHow::Left,
        "right" | "rightouter" => JoinHow::Right,
        "outer" | "full" | "fullouter" => JoinHow::Outer,
        _ => JoinHow::Inner,
    }
}

/// The join strategy of a `.join(...)` call — the `how=` keyword, or the
/// third positional argument. Absent / non-string → `Inner`.
fn extract_how_arg(call: &ExprCall) -> JoinHow {
    let how_expr = call
        .arguments
        .keywords
        .iter()
        .find(|kw| kw.arg.as_ref().is_some_and(|n| n.id.as_str() == "how"))
        .map(|kw| &kw.value)
        .or_else(|| call.arguments.args.get(2));
    match how_expr.and_then(|e| e.as_string_literal_expr()) {
        Some(s) => join_how(s.value.to_str()),
        None => JoinHow::Inner,
    }
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
    how: JoinHow,
    schemas: &'a [Schema<'a>],
) -> SchemaView<'a> {
    let dedup_set: HashSet<&str> = match on {
        JoinOn::Keys(keys) => keys.iter().map(|(n, _)| *n).collect(),
        _ => HashSet::new(),
    };
    // An outer join leaves the other side's columns null on an unmatched
    // row: a `left` join makes the right side nullable, `right` the left,
    // `outer` both. The join keys are coalesced and stay non-null.
    let (left_nullable, right_nullable) = match how {
        JoinHow::Inner => (false, false),
        JoinHow::Left => (false, true),
        JoinHow::Right => (true, false),
        JoinHow::Outer => (true, true),
    };
    let mut result: Vec<DerivedField<'a>> = left
        .typed_fields(schemas)
        .into_iter()
        .map(|f| {
            let nullable = left_nullable && !dedup_set.contains(f.name);
            with_nullability(f, nullable)
        })
        .collect();
    for f in right.typed_fields(schemas) {
        // The join key(s) are already in result from the left side.
        if dedup_set.contains(f.name) {
            continue;
        }
        // Non-key shared names: left wins.
        if !result.iter().any(|r| r.name == f.name) {
            result.push(with_nullability(f, right_nullable));
        }
    }
    SchemaView::Derived(result)
}

/// Wrap a field's type in `Nullable` when `nullable` and a type is
/// known. A no-op when `nullable` is false or the type is unknown, and
/// idempotent — an already-nullable type isn't double-wrapped.
fn with_nullability(mut f: DerivedField<'_>, nullable: bool) -> DerivedField<'_> {
    if nullable && let Some(ty) = f.ty {
        f.ty = Some(match ty {
            ColumnType::Nullable(_) => ty,
            other => ColumnType::Nullable(Box::new(other)),
        });
    }
    f
}

/// Every column of `view` with any `Nullable` wrapper peeled off — the
/// effect of a null-clearing operation (`fillna`, `dropna`, `na.fill`,
/// `na.drop`). Conservative: it clears nullability from the whole
/// schema, which can only under-report (never false-flag).
fn strip_nullability<'a>(view: &SchemaView<'a>, schemas: &'a [Schema<'a>]) -> SchemaView<'a> {
    let fields = view
        .typed_fields(schemas)
        .into_iter()
        .map(|mut f| {
            if let Some(ty) = &f.ty {
                f.ty = Some(ty.base().clone());
            }
            f
        })
        .collect();
    SchemaView::Derived(fields)
}

/// Schema concatenation for crossJoin: every field from both sides; shared
/// names are kept once (left wins).
fn apply_concat<'a>(
    left: &SchemaView<'a>,
    right: &SchemaView<'a>,
    schemas: &'a [Schema<'a>],
) -> SchemaView<'a> {
    let mut result: Vec<DerivedField<'a>> = left.typed_fields(schemas);
    for f in right.typed_fields(schemas) {
        if !result.iter().any(|r| r.name == f.name) {
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
    if let Some(attr) = expr.as_attribute_expr()
        && let Some(name) = attr.value.as_name_expr()
        && ctx.lookup(name.id.as_str()).is_some()
    {
        out.push((attr.attr.id.as_str(), attr.attr.range));
        return;
    }
    // `df["X"]` subscript access — the sibling of `df.X`. Real PySpark code
    // uses this ubiquitously (`df["age"]`, `df["name"]`), and a typo in the
    // string slot should be a D0030 just like a typo on `df.X` or
    // `col("X")`. The receiver name must be bound in the current scope
    // (same ctx discriminator as the attribute arm) and the slice must be
    // a string literal — computed subscripts fall through to the default
    // walker.
    if let Some(sub) = expr.as_subscript_expr()
        && let Some(name) = sub.value.as_name_expr()
        && ctx.lookup(name.id.as_str()).is_some()
        && let Some(s) = sub.slice.as_string_literal_expr()
    {
        out.push((s.value.to_str(), s.range()));
        return;
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
        if let Some(name) = func_name
            && COLUMN_REF_FUNCTIONS.contains(&name)
        {
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
