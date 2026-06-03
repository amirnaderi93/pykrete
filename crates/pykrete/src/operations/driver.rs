use std::collections::HashSet;

use super::context::BodyContext;
use super::expr::analyze_expr;

use ruff_python_ast::{Expr, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{self, DataFrameAnnotation, SlotLabel, TypedSlot, typed_slots_for_def};
use crate::diagnostics::{CheckMode, Diagnostic, Severity};
use crate::schema::{Schema, SchemaView};
use crate::types::ColumnType;
use crate::walk::DiscoveredFunction;

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Walk a function body, checking calls and validating the return type.
///
/// Returns the inferred schema of the function's *first* `return`
/// statement, if one could be determined — used by [`infer_transform_output`]
/// to type a `transform` function whose return isn't declared. Callers
/// that only want the diagnostics can ignore the result.
pub(crate) fn check_function_body<'a>(
    func: &DiscoveredFunction<'a>,
    declared_return: Option<SchemaView<'a>>,
    ctx: &mut BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SchemaView<'a>> {
    let mut inferred_return: Option<SchemaView<'a>> = None;
    walk_body(
        &func.def.body,
        declared_return.as_ref(),
        ctx,
        source,
        line_index,
        diagnostics,
        &mut inferred_return,
    );
    inferred_return
}

/// Walk a sequence of statements, dispatching per-stmt handling and
/// recursing into the bodies of control-flow statements (`if`/`for`/
/// `while`/`with`/`try`/`match`). Every column reference inside a
/// conditional branch or loop body needs to reach the checker, so the
/// walker descends unconditionally — pykrete doesn't reason about
/// reachability (return-after-return, etc.). Loop and `with` targets
/// are marked as local names; exception bindings (`except E as e`)
/// likewise. The body checker is the only seam that pushes
/// diagnostics for column references, so missing a branch was a
/// silent blind spot — see v0.1.26.
fn walk_body<'a>(
    body: &'a [Stmt],
    declared_return: Option<&SchemaView<'a>>,
    ctx: &mut BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
    inferred_return: &mut Option<SchemaView<'a>>,
) {
    for stmt in body {
        walk_stmt(
            stmt,
            declared_return,
            ctx,
            source,
            line_index,
            diagnostics,
            inferred_return,
        );
    }
}

/// Resolve `slots`' return slot to a `SchemaView`, mirroring
/// `lib::declared_return_schema` for the nested-funcdef path. Used so a
/// `return raw.select(...)` inside a nested helper can be type-checked
/// against the helper's own annotated return.
fn declared_return_view<'a>(
    slots: &[TypedSlot<'a>],
    schemas: &'a [Schema<'a>],
) -> Option<SchemaView<'a>> {
    for slot in slots {
        if matches!(slot.label, SlotLabel::Return) {
            return match slot.kind {
                DataFrameAnnotation::Typed(name) => schemas
                    .iter()
                    .find(|s| s.name() == name)
                    .map(SchemaView::Declared),
                DataFrameAnnotation::Derived(expr) => {
                    crate::schema::resolve_derived_schema(expr, schemas)
                }
                _ => None,
            };
        }
    }
    None
}

fn walk_stmt<'a>(
    stmt: &'a Stmt,
    declared_return: Option<&SchemaView<'a>>,
    ctx: &mut BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
    inferred_return: &mut Option<SchemaView<'a>>,
) {
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
        Stmt::AugAssign(a) => {
            // `x += expr` — both sides are normal expressions; analyze
            // them so column refs inside surface, and mark the target
            // local (it could shadow a top-level name).
            analyze_expr(&a.value, ctx, source, line_index, diagnostics);
            analyze_expr(&a.target, ctx, source, line_index, diagnostics);
            ctx.mark_local_target(&a.target);
        }
        Stmt::Return(r) => {
            let Some(value) = r.value.as_deref() else {
                return;
            };
            let actual = analyze_expr(value, ctx, source, line_index, diagnostics);
            if inferred_return.is_none() {
                *inferred_return = actual.clone();
            }
            if let (Some(declared), Some(actual)) = (declared_return, actual.as_ref()) {
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
        Stmt::If(if_stmt) => {
            analyze_expr(&if_stmt.test, ctx, source, line_index, diagnostics);
            walk_body(
                &if_stmt.body,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
            for clause in &if_stmt.elif_else_clauses {
                if let Some(test) = clause.test.as_ref() {
                    analyze_expr(test, ctx, source, line_index, diagnostics);
                }
                walk_body(
                    &clause.body,
                    declared_return,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                    inferred_return,
                );
            }
        }
        Stmt::For(for_stmt) => {
            analyze_expr(&for_stmt.iter, ctx, source, line_index, diagnostics);
            ctx.mark_local_target(&for_stmt.target);
            walk_body(
                &for_stmt.body,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
            walk_body(
                &for_stmt.orelse,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
        }
        Stmt::While(while_stmt) => {
            analyze_expr(&while_stmt.test, ctx, source, line_index, diagnostics);
            walk_body(
                &while_stmt.body,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
            walk_body(
                &while_stmt.orelse,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
        }
        Stmt::With(with_stmt) => {
            for item in &with_stmt.items {
                let schema = analyze_expr(&item.context_expr, ctx, source, line_index, diagnostics);
                if let Some(vars) = item.optional_vars.as_deref() {
                    ctx.mark_local_target(vars);
                    if let (Some(name), Some(schema)) = (vars.as_name_expr(), schema) {
                        ctx.bind_df(name.id.as_str(), schema.clone());
                        ctx.record_local_binding(name.id.as_str(), name.range, schema);
                    }
                }
            }
            walk_body(
                &with_stmt.body,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
        }
        Stmt::Try(try_stmt) => {
            walk_body(
                &try_stmt.body,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
            for handler in &try_stmt.handlers {
                let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                if let Some(name) = h.name.as_ref() {
                    ctx.mark_local(name.id.as_str());
                }
                walk_body(
                    &h.body,
                    declared_return,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                    inferred_return,
                );
            }
            walk_body(
                &try_stmt.orelse,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
            walk_body(
                &try_stmt.finalbody,
                declared_return,
                ctx,
                source,
                line_index,
                diagnostics,
                inferred_return,
            );
        }
        Stmt::Match(_) => {
            // Match bodies are NOT walked: `case` patterns can introduce
            // new local names (`case MyClass(field=x):` binds `x`) that
            // would otherwise look like undefined symbols to the D0051
            // / column-ref machinery, since the walker has no pattern-
            // binding extractor. Once pattern binding is modeled we can
            // walk subject / guards / bodies through here. Tracked as a
            // v1.1 follow-up — see `docs/design/spark-coverage.md`.
        }
        Stmt::FunctionDef(func_def) => {
            // Nested function — `def helper(...): ...` inside the body
            // of an outer function. Walk the inner body in a CHILD
            // context so the inner fn's params and locals don't leak
            // into the parent. The child inherits the parent's schemas,
            // registry, and tempView registry. Typed `DataFrame[X]` /
            // class-instance params are re-bound from the inner sig so
            // column refs inside the helper resolve correctly.
            let inner_slots = typed_slots_for_def(func_def);
            let mut child = BodyContext::from_function_def(
                func_def,
                &inner_slots,
                ctx.schemas(),
                ctx.registry(),
            );
            if let Some(tv) = ctx.temp_views() {
                child = child.with_temp_views(tv);
            }
            child.infer_depth = ctx.infer_depth;
            let inner_declared_return = declared_return_view(&inner_slots, ctx.schemas());
            let mut inner_inferred: Option<SchemaView<'a>> = None;
            walk_body(
                &func_def.body,
                inner_declared_return.as_ref(),
                &mut child,
                source,
                line_index,
                diagnostics,
                &mut inner_inferred,
            );
            // Mark the function's NAME as local in the OUTER scope so a
            // subsequent reference resolves to the nested helper, not
            // (e.g.) a top-level same-named function.
            ctx.mark_local(func_def.name.id.as_str());
        }
        Stmt::ClassDef(class_def) => {
            // Nested class — `class C: ...` inside a function body. We
            // don't model nested classes as Schemas (Schema discovery
            // is module-level), but we walk the body so column refs
            // inside any method bodies still reach the checker.
            for inner_stmt in &class_def.body {
                walk_stmt(
                    inner_stmt,
                    declared_return,
                    ctx,
                    source,
                    line_index,
                    diagnostics,
                    inferred_return,
                );
            }
            ctx.mark_local(class_def.name.id.as_str());
        }
        Stmt::Assert(a) => {
            analyze_expr(&a.test, ctx, source, line_index, diagnostics);
            if let Some(msg) = a.msg.as_deref() {
                analyze_expr(msg, ctx, source, line_index, diagnostics);
            }
        }
        Stmt::Raise(r) => {
            if let Some(exc) = r.exc.as_deref() {
                analyze_expr(exc, ctx, source, line_index, diagnostics);
            }
            if let Some(cause) = r.cause.as_deref() {
                analyze_expr(cause, ctx, source, line_index, diagnostics);
            }
        }
        _ => {}
    }
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

    // v1.3 pandas spec §6: fire D0090 once for the deprecated
    // `DataFrame[X]` alias on local annotated assignments, mirroring
    // the signature-renderer's emission. Echo source text per Q7.
    // `recognize_with_dialect` is called once and reused for the schema
    // match below; wording is owned by `format_d0090_message`.
    let recognized = dataframe::recognize_with_dialect(&ann.annotation);
    if let Some(rec) = recognized
        && rec.is_deprecated_alias
    {
        let raw_text = &source[ann.annotation.range()];
        let (message, suggestion) = crate::dataframe::format_d0090_message(raw_text);
        diagnostics.push(
            Diagnostic::at_range(
                Severity::Warning,
                "D0090",
                message,
                ann.annotation.range(),
                source,
                line_index,
            )
            .with_suggestion(Some(suggestion)),
        );
    }

    match recognized.map(|r| r.kind) {
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
    // Exhaustive (no `_` arm) so a future `ColumnType` variant is a
    // compile error here — mirrors `strict_operators::type_family`.
    fn is_numeric(t: &ColumnType) -> bool {
        match t {
            ColumnType::Int
            | ColumnType::Long
            | ColumnType::Double
            | ColumnType::Float
            | ColumnType::Byte
            | ColumnType::Short
            | ColumnType::Decimal { .. } => true,
            ColumnType::String
            | ColumnType::Enum(_)
            | ColumnType::Binary
            | ColumnType::Bool
            | ColumnType::Date
            | ColumnType::Timestamp
            | ColumnType::Array(_)
            | ColumnType::Map(..)
            | ColumnType::Struct(_) => false,
            ColumnType::Nullable(inner) => is_numeric(inner),
        }
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
        // order, each field type compatible. The empty-fields shape is
        // F4's opaque-Struct sentinel ("I'm a struct, shape unknown"),
        // and pykrete declines to guess: an opaque struct flowing into a
        // typed one (typical pass-through-with-`cast` refinement) is
        // permissive, not a D0080.
        (ColumnType::Struct(xs), ColumnType::Struct(ys)) => {
            xs.is_empty()
                || ys.is_empty()
                || (xs.len() == ys.len()
                    && xs
                        .iter()
                        .zip(ys)
                        .all(|(x, y)| x.name == y.name && field_ok(&x.ty, &y.ty)))
        }
        // Two enum vocabularies are compatible when set-equal — the spec
        // contract is order-independent. Derived `PartialEq` on
        // `ColumnType::Enum(Vec<String>)` is order-sensitive, which would
        // false-flag `enum["a","b"]` vs `enum["b","a"]` for the same
        // column. Use the explicit set-equality helper at every
        // comparison site.
        (ColumnType::Enum(_), ColumnType::Enum(_)) => ColumnType::enum_vocab_eq(a, b),
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
