//! `BodyContext` — the per-function-body bindings + LSP trace state.
//! Folds in the process-wide synthetic-name intern pool used to materialize
//! `sum(amount)`-style fields produced by `groupBy.<agg>` shortcuts.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use ruff_python_ast::{Expr, StmtFunctionDef};
use ruff_text_size::TextRange;

use crate::dataframe::{DataFrameAnnotation, SlotLabel, TypedSlot};
use crate::registry::Registry;
use crate::schema::{Schema, SchemaView};
use crate::temp_views::TempViewRegistry;
use crate::walk::DiscoveredFunction;

/// The project-wide context the type-inference engine needs: every
/// declared schema (for nested-struct resolution) and the registry (for
/// UDF return types). Bundled into one `Copy` value so the recursive
/// inference functions take a single argument instead of threading two.
#[derive(Clone, Copy)]
pub(super) struct TypeCtx<'a> {
    pub(super) schemas: &'a [Schema<'a>],
    pub(super) registry: &'a Registry<'a>,
}

/// Process-wide dedup pool for synthetic column names (`sum(amount)` &
/// friends produced by the `groupBy.<agg>` shortcut). See
/// [`BodyContext::intern_synthetic`] for the lifetime / leak story.
fn synthetic_name_pool() -> &'static Mutex<HashSet<&'static str>> {
    static POOL: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashSet::new()))
}

fn intern_synthetic_global(name: String) -> &'static str {
    let mut pool = synthetic_name_pool().lock().expect("pool mutex poisoned");
    if let Some(existing) = pool.get(name.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.into_boxed_str());
    pool.insert(leaked);
    leaked
}

/// Test-only view onto the synthetic-name pool's size. Used by the
/// stress test that pins the leak's growth to "one entry per distinct
/// synthetic name" regardless of how many analysis passes ran.
#[doc(hidden)]
pub fn synthetic_pool_len() -> usize {
    synthetic_name_pool()
        .lock()
        .expect("pool mutex poisoned")
        .len()
}

// ---------------------------------------------------------------------------
// BodyContext — parameter and local-variable bindings
// ---------------------------------------------------------------------------

pub(crate) struct BodyContext<'a> {
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
    pub(super) infer_depth: u32,
    /// File-scoped `createOrReplaceTempView` registrations, shared by
    /// reference across every function-body context in the same file.
    /// `None` means "no registry was wired in" — used by call sites that
    /// build a throwaway context (the inference recursion in
    /// `infer_transform_output`) or by future internal entry points
    /// that don't care about view tracking; the tempView paths degrade
    /// to no-ops in that case.
    ///
    /// The borrow's lifetime is allowed to be shorter than `'a` —
    /// `'static` would otherwise pin the registry to live as long as
    /// the source, which the per-file driver doesn't want. The
    /// projection-style `*const _` is enough: the registry is only
    /// touched while `analyze_expr` is running, which is shorter than
    /// the call site that owns the registry. Modeled as `Option<&...>`
    /// rather than `*const` so safe-Rust semantics survive.
    temp_views: Option<&'a TempViewRegistry<'a>>,
}

/// Recursion ceiling for `transform`-body inference — deep enough for any
/// realistic pipeline-step nesting, shallow enough to stop a cycle.
pub(super) const MAX_INFER_DEPTH: u32 = 8;

/// One `col("name")` (or string-arg) site captured during body analysis,
/// with the schema that was active at the time the column was resolved.
///
/// The schema is what the user is *thinking against* at that site — i.e.
/// the immediate receiver of the surrounding method call. For
/// `raw.filter(col("a") > 0).select(col("b"))`, both `col("a")` and
/// `col("b")` carry `raw`'s schema (filter preserves shape).
#[derive(Debug, Clone)]
pub(crate) struct ColumnRefTrace<'a> {
    pub range: TextRange,
    pub name: &'a str,
    pub schema: SchemaView<'a>,
}

/// One method-call site and the schema it evaluates to. `range` is the
/// whole call expression's range — the receiver of a following `.<attr>`
/// access, which the completion layer matches against to offer the
/// chain result's columns.
#[derive(Debug, Clone)]
pub(crate) struct CallResultTrace<'a> {
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
pub(crate) struct LocalBindingTrace<'a> {
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
            temp_views: None,
        }
    }

    /// Wire `temp_views` as this context's tempView registry — the
    /// shared per-file map that `createOrReplaceTempView` writes into
    /// and `spark.sql("… FROM view")` reads from. Returns `self` for
    /// fluent construction in [`Self::from_function`].
    pub fn with_temp_views(mut self, temp_views: &'a TempViewRegistry<'a>) -> Self {
        self.temp_views = Some(temp_views);
        self
    }

    /// The file-scoped tempView registry, if one is wired in.
    pub fn temp_views(&self) -> Option<&'a TempViewRegistry<'a>> {
        self.temp_views
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
        func: &DiscoveredFunction<'a>,
        slots: &[TypedSlot<'a>],
        schemas: &'a [Schema<'a>],
        registry: &'a Registry<'a>,
    ) -> Self {
        Self::from_function_def(func.def, slots, schemas, registry)
    }

    /// Same as [`Self::from_function`] but takes the underlying
    /// `StmtFunctionDef` directly. Used by the nested-funcdef walker,
    /// where wrapping the AST node in a stack-local `DiscoveredFunction`
    /// would over-constrain the borrow lifetime.
    pub fn from_function_def(
        func_def: &'a StmtFunctionDef,
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
        for pwd in func_def
            .parameters
            .posonlyargs
            .iter()
            .chain(&func_def.parameters.args)
            .chain(&func_def.parameters.kwonlyargs)
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
        if let Some(vararg) = func_def.parameters.vararg.as_deref() {
            ctx.mark_local(vararg.name.id.as_str());
        }
        if let Some(kwarg) = func_def.parameters.kwarg.as_deref() {
            ctx.mark_local(kwarg.name.id.as_str());
        }

        ctx
    }

    pub fn bind_df(&mut self, name: &'a str, view: SchemaView<'a>) {
        self.df_bindings.insert(name, view);
        self.mark_local(name);
    }

    /// Intern a synthetic column name (e.g. `sum(amount)` produced by
    /// `groupBy.sum("amount")`) so it can be carried in a
    /// [`DerivedField`] that needs a `&'a str`.
    ///
    /// The pool is process-wide and de-duped: the same `(method, column)`
    /// pair from any analysis call returns the same `&'static str`
    /// without re-leaking. This caps the leak set at "every distinct
    /// synthetic name the workspace has ever produced" — a small
    /// finite vocabulary (`sum(col)`, `max(col)`, …) bounded by
    /// `aggregate_method × unique_column_name`. Per-context interning
    /// would leak a fresh copy on every LSP keystroke, which is what
    /// this dedup is here to prevent.
    ///
    /// Returns `&'a str` because that's what the trait bound at the
    /// `DerivedField` callsite asks for; `&'static: 'a` so the
    /// conversion is sound. The synthesizer here is the only kind of
    /// name that doesn't originate in the user's source; all other
    /// field names borrow directly from the source.
    pub fn intern_synthetic(&self, name: String) -> &'a str {
        intern_synthetic_global(name)
    }

    /// Mark `name` as locally bound — even when the RHS schema is unknown.
    /// Used by D0051 to spot a local rebind that shadows a top-level
    /// function: the call resolves to the local at runtime, so the
    /// top-level signature shouldn't be consulted.
    ///
    /// `&self` rather than `&mut self` so the analysis pass can mark a
    /// walrus-bound name through an otherwise-immutable context — the
    /// underlying set lives in a `RefCell`.
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
    pub(super) fn type_ctx(&self) -> TypeCtx<'a> {
        TypeCtx {
            schemas: self.schemas,
            registry: self.registry,
        }
    }
}
