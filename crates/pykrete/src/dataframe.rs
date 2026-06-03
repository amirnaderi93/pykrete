//! DataFrame-typed annotations on function signatures.
//!
//! Today this module only recognizes the *shape* of a `DataFrame[X]` or bare
//! `DataFrame` annotation. Resolving the inner schema name against the
//! discovered schemas, and producing diagnostics for unknowns, happens in
//! the driver layer.
//!
//! `DataFrame` is matched by literal name only — if a user writes
//! `from pyspark.sql import DataFrame as DF`, pykrete won't recognize `DF[...]`
//! until import resolution lands.
//!
//! v1.3 widens the recognized name set to `{SparkFrame, PandasFrame,
//! DataFrame}`. `SparkFrame` is the canonical Spark form; `PandasFrame`
//! is the pandas form; `DataFrame` is a deprecated alias for `SparkFrame`
//! (D0090) removed in v2.0. See `docs/design/pandas-support.md`.

use ruff_python_ast::{Expr, StmtFunctionDef};

use crate::walk::DiscoveredFunction;

/// Which DataFrame dialect a typed slot binds — drives the call-site
/// dispatch table in v1.3. Spark-style operations (`df.select`,
/// `df.withColumn`, …) are checked against `Dialect::Spark` slots;
/// pandas-style operations (`df[["a", "b"]]`, `df["new"] = expr`, …)
/// are checked against `Dialect::Pandas` slots. The PR-A landing only
/// stamps the tag; PR-B wires the per-dialect dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Spark,
    Pandas,
}

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

/// A recognized frame annotation plus the parser-level metadata: dialect
/// tag (Spark vs Pandas) and a flag marking the deprecated `DataFrame`
/// alias use (drives D0090 emission).
#[derive(Debug, Clone, Copy)]
pub struct RecognizedFrame<'ast> {
    pub kind: DataFrameAnnotation<'ast>,
    pub dialect: Dialect,
    /// True iff the annotation's base name was the literal `DataFrame`
    /// (the deprecated alias for `SparkFrame`). Used at slot-emission
    /// sites to fire D0090.
    pub is_deprecated_alias: bool,
}

/// Recognize a DataFrame-shaped annotation. Returns `None` if `expr`
/// isn't a frame annotation. Backwards-compatible shim returning just
/// the inner [`DataFrameAnnotation`]; callers that need the dialect
/// tag or the deprecated-alias flag use [`recognize_with_dialect`].
pub fn recognize<'ast>(expr: &'ast Expr) -> Option<DataFrameAnnotation<'ast>> {
    recognize_with_dialect(expr).map(|r| r.kind)
}

/// Recognize a DataFrame-shaped annotation along with its dialect tag.
/// `SparkFrame[X]` → `Dialect::Spark`; `PandasFrame[X]` → `Dialect::Pandas`;
/// `DataFrame[X]` → `Dialect::Spark` with `is_deprecated_alias = true`.
pub fn recognize_with_dialect<'ast>(expr: &'ast Expr) -> Option<RecognizedFrame<'ast>> {
    match expr {
        Expr::Name(name) => {
            let (dialect, alias) = base_name_to_dialect(name.id.as_str())?;
            Some(RecognizedFrame {
                kind: DataFrameAnnotation::Untyped,
                dialect,
                is_deprecated_alias: alias,
            })
        }
        Expr::Subscript(sub) => {
            let base = sub.value.as_name_expr()?;
            let (dialect, alias) = base_name_to_dialect(base.id.as_str())?;
            let kind = match sub.slice.as_ref() {
                Expr::Name(inner) => DataFrameAnnotation::Typed(inner.id.as_str()),
                // `DataFrame[Pick[…]]` / `Omit[…]` / `Merge[…]` — a
                // derived-schema operator. Recognized loosely here (the
                // base names a known operator); the inner shape is
                // validated when resolved against schemas. The dialect
                // is carried by the outer frame wrapping — `Pick`/`Omit`/
                // `Merge` themselves are dialect-agnostic, so the
                // resolved view inherits whatever dialect the frame
                // declared.
                inner @ Expr::Subscript(s)
                    if s.value
                        .as_name_expr()
                        .is_some_and(|n| matches!(n.id.as_str(), "Pick" | "Omit" | "Merge")) =>
                {
                    DataFrameAnnotation::Derived(inner)
                }
                // `DataFrame[{col: type, …}]` — an inline structural
                // schema (a dict literal).
                inner @ Expr::Dict(_) => DataFrameAnnotation::Derived(inner),
                _ => DataFrameAnnotation::NonBareName,
            };
            Some(RecognizedFrame {
                kind,
                dialect,
                is_deprecated_alias: alias,
            })
        }
        _ => None,
    }
}

/// Map a recognized base-name identifier to its dialect tag plus a
/// deprecated-alias flag. Centralized so the parser and any future
/// tooling agree on the canonical set.
///
/// Per spec §3 union annotations (`SparkFrame[X] | PandasFrame[X]`) are
/// out of scope for v1.3 — they never reach this function because the
/// outer `Expr::BinOp` is filtered out by [`recognize_with_dialect`]'s
/// shape match (only `Expr::Name` / `Expr::Subscript` enter here). The
/// union case "quiet-ignores" — no dialect committed, no slot bound,
/// no piece-(b) check ever fires.
fn base_name_to_dialect(name: &str) -> Option<(Dialect, bool)> {
    match name {
        "SparkFrame" => Some((Dialect::Spark, false)),
        "PandasFrame" => Some((Dialect::Pandas, false)),
        "DataFrame" => Some((Dialect::Spark, true)),
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
    /// Which dialect this slot binds — set by the parser from the base
    /// frame name. Drives piece (b)'s per-dialect dispatch (PR-B).
    pub dialect: Dialect,
    /// True iff the annotation used the deprecated `DataFrame[X]` alias.
    /// The signature renderer fires D0090 once per slot when this is set.
    pub is_deprecated_alias: bool,
}

/// Return every parameter/return slot of `func` whose annotation is DataFrame-shaped.
/// Slots whose annotations are anything else (or absent) are silently skipped.
///
/// Positional-only and keyword-only parameters are included; `*args` / `**kwargs`
/// are not.
pub fn typed_slots<'ast>(func: &'ast DiscoveredFunction<'ast>) -> Vec<TypedSlot<'ast>> {
    typed_slots_for_def(func.def)
}

/// Same as [`typed_slots`] but takes the underlying `StmtFunctionDef`
/// directly — used by the nested-funcdef walker, where wrapping the
/// AST node in a stack-local `DiscoveredFunction` would over-constrain
/// the returned slots' lifetime.
pub fn typed_slots_for_def<'ast>(func_def: &'ast StmtFunctionDef) -> Vec<TypedSlot<'ast>> {
    let mut slots = Vec::new();

    let params = &*func_def.parameters;
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
        if let Some(rec) = recognize_with_dialect(ann) {
            slots.push(TypedSlot {
                label: SlotLabel::Param(param.name.id.as_str()),
                annotation: ann,
                kind: rec.kind,
                dialect: rec.dialect,
                is_deprecated_alias: rec.is_deprecated_alias,
            });
        }
    }

    if let Some(ret) = func_def.returns.as_deref()
        && let Some(rec) = recognize_with_dialect(ret)
    {
        slots.push(TypedSlot {
            label: SlotLabel::Return,
            annotation: ret,
            kind: rec.kind,
            dialect: rec.dialect,
            is_deprecated_alias: rec.is_deprecated_alias,
        });
    }

    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruff_python_parser::parse_expression;

    fn parse(src: &str) -> ruff_python_ast::ModExpression {
        parse_expression(src).expect("parse").into_syntax()
    }

    fn rec(src: &str) -> RecognizedFrame<'_> {
        let owned = Box::leak(Box::new(parse(src)));
        recognize_with_dialect(&owned.body).expect("recognize")
    }

    #[test]
    fn spark_frame_subscript_parses_with_spark_dialect_no_alias() {
        let r = rec("SparkFrame[Order]");
        assert_eq!(r.dialect, Dialect::Spark);
        assert!(!r.is_deprecated_alias);
        assert!(matches!(r.kind, DataFrameAnnotation::Typed("Order")));
    }

    #[test]
    fn pandas_frame_subscript_parses_with_pandas_dialect_no_alias() {
        let r = rec("PandasFrame[Order]");
        assert_eq!(r.dialect, Dialect::Pandas);
        assert!(!r.is_deprecated_alias);
        assert!(matches!(r.kind, DataFrameAnnotation::Typed("Order")));
    }

    #[test]
    fn dataframe_alias_subscript_resolves_to_spark_with_deprecation_flag() {
        let r = rec("DataFrame[Order]");
        assert_eq!(r.dialect, Dialect::Spark);
        assert!(r.is_deprecated_alias);
        assert!(matches!(r.kind, DataFrameAnnotation::Typed("Order")));
    }

    #[test]
    fn bare_pandas_frame_parses_as_untyped_pandas() {
        let r = rec("PandasFrame");
        assert_eq!(r.dialect, Dialect::Pandas);
        assert!(!r.is_deprecated_alias);
        assert!(matches!(r.kind, DataFrameAnnotation::Untyped));
    }

    #[test]
    fn bare_dataframe_carries_deprecation_flag() {
        let r = rec("DataFrame");
        assert_eq!(r.dialect, Dialect::Spark);
        assert!(r.is_deprecated_alias);
    }

    #[test]
    fn unrelated_name_is_not_recognized() {
        let owned = Box::leak(Box::new(parse("MyDataFrame[Order]")));
        assert!(recognize_with_dialect(&owned.body).is_none());
    }

    #[test]
    fn union_annotation_is_quiet_ignored_for_v1_3() {
        // SparkFrame[X] | PandasFrame[X] — the outer Expr is a BinOp,
        // not a Name or Subscript, so no dialect commits. Piece (b)
        // downstream will see no frame-typed slot bound, per spec §3.
        let owned = Box::leak(Box::new(parse("SparkFrame[Order] | PandasFrame[Order]")));
        assert!(recognize_with_dialect(&owned.body).is_none());
    }

    #[test]
    fn pandas_frame_with_derived_pick_preserves_pandas_dialect() {
        // §3: Pick is dialect-agnostic; the wrapping frame carries the
        // dialect. A PandasFrame[Pick[Order, "id"]] stays Pandas.
        let r = rec("PandasFrame[Pick[Order, \"id\"]]");
        assert_eq!(r.dialect, Dialect::Pandas);
        assert!(!r.is_deprecated_alias);
        assert!(matches!(r.kind, DataFrameAnnotation::Derived(_)));
    }

    #[test]
    fn spark_frame_with_derived_merge_stays_spark() {
        let r = rec("SparkFrame[Merge[A, B]]");
        assert_eq!(r.dialect, Dialect::Spark);
        assert!(matches!(r.kind, DataFrameAnnotation::Derived(_)));
    }

    #[test]
    fn dataframe_alias_with_derived_pick_flags_deprecation_and_keeps_spark() {
        let r = rec("DataFrame[Omit[Order, \"id\"]]");
        assert_eq!(r.dialect, Dialect::Spark);
        assert!(r.is_deprecated_alias);
        assert!(matches!(r.kind, DataFrameAnnotation::Derived(_)));
    }
}
