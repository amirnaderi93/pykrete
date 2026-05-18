//! DataFrame-typed annotations on function signatures.
//!
//! Today this module only recognizes the *shape* of a `DataFrame[X]` or bare
//! `DataFrame` annotation. Resolving the inner schema name against the
//! discovered schemas, and producing diagnostics for unknowns, happens in
//! the driver layer.
//!
//! `DataFrame` is matched by literal name only — if a user writes
//! `from pyspark.sql import DataFrame as DF`, dathon won't recognize `DF[...]`
//! until import resolution lands.

use ruff_python_ast::Expr;

use crate::walk::DiscoveredFunction;

/// Shape of a DataFrame-typed annotation, before resolution against schemas.
#[derive(Debug, Clone, Copy)]
pub enum DataFrameAnnotation<'ast> {
    /// Bare `DataFrame` with no schema parameter.
    Untyped,
    /// `DataFrame[X]` where X is a bare name; X has not yet been resolved.
    Typed(&'ast str),
    /// `DataFrame[Pick[…]]` / `DataFrame[Omit[…]]` — a derived-schema
    /// expression. The inner `Pick`/`Omit` subscript is carried for
    /// resolution against the discovered schemas (see
    /// [`crate::schema::resolve_pick_omit`]).
    Derived(&'ast Expr),
    /// `DataFrame[<complex>]` — e.g. `DataFrame[list[str]]`, `DataFrame[A | B]`.
    NonBareName,
}

/// Recognize a DataFrame-shaped annotation. Returns `None` if `expr` isn't
/// a `DataFrame` or `DataFrame[…]` annotation at all.
pub fn recognize<'ast>(expr: &'ast Expr) -> Option<DataFrameAnnotation<'ast>> {
    match expr {
        Expr::Name(name) if name.id.as_str() == "DataFrame" => Some(DataFrameAnnotation::Untyped),
        Expr::Subscript(sub) => {
            let base = sub.value.as_name_expr()?;
            if base.id.as_str() != "DataFrame" {
                return None;
            }
            match sub.slice.as_ref() {
                Expr::Name(inner) => Some(DataFrameAnnotation::Typed(inner.id.as_str())),
                // `DataFrame[Pick[…]]` / `DataFrame[Omit[…]]` — a
                // derived-schema operator. Recognized loosely here (the
                // base is `Pick`/`Omit`); the inner shape is validated
                // when the expression is resolved against schemas.
                inner @ Expr::Subscript(s)
                    if s.value
                        .as_name_expr()
                        .is_some_and(|n| matches!(n.id.as_str(), "Pick" | "Omit")) =>
                {
                    Some(DataFrameAnnotation::Derived(inner))
                }
                _ => Some(DataFrameAnnotation::NonBareName),
            }
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SlotLabel<'ast> {
    Param(&'ast str),
    Return,
}

/// One DataFrame-touching slot from a function signature: a parameter or the
/// return type. The annotation expression is carried so the caller can
/// produce diagnostics at the right source location.
#[derive(Debug)]
pub struct TypedSlot<'ast> {
    pub label: SlotLabel<'ast>,
    pub annotation: &'ast Expr,
    pub kind: DataFrameAnnotation<'ast>,
}

/// Return every parameter/return slot of `func` whose annotation is DataFrame-shaped.
/// Slots whose annotations are anything else (or absent) are silently skipped.
///
/// Positional-only and keyword-only parameters are included; `*args` / `**kwargs`
/// are not.
pub fn typed_slots<'ast>(func: &'ast DiscoveredFunction<'ast>) -> Vec<TypedSlot<'ast>> {
    let mut slots = Vec::new();

    let params = &*func.def.parameters;
    let positional = params
        .posonlyargs
        .iter()
        .chain(&params.args)
        .chain(&params.kwonlyargs);
    for pwd in positional {
        let param = &pwd.parameter;
        let Some(ann) = param.annotation.as_deref() else {
            continue;
        };
        if let Some(kind) = recognize(ann) {
            slots.push(TypedSlot {
                label: SlotLabel::Param(param.name.id.as_str()),
                annotation: ann,
                kind,
            });
        }
    }

    if let Some(ret) = func.def.returns.as_deref() {
        if let Some(kind) = recognize(ret) {
            slots.push(TypedSlot {
                label: SlotLabel::Return,
                annotation: ret,
                kind,
            });
        }
    }

    slots
}
