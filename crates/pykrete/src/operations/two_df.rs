use std::collections::HashSet;

use super::col_refs::collect_col_refs;
use super::context::BodyContext;
use super::expr::analyze_expr;
use super::shapes::TwoDfMethod;

use ruff_python_ast::{Expr, ExprCall};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::{DerivedField, Schema, SchemaView, suggest_field_name};
use crate::types::ColumnType;

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
    /// A Column-expression on-clause — typically
    /// `col("a") == col("b")`, `df1.a == df2.b`, or boolean conjunctions
    /// of these. The collected column references can't be assigned to a
    /// specific side without name-equality reasoning, so each is checked
    /// against the union (must exist in at least ONE side). The result
    /// schema is the concatenation of both sides with NO dedup — Spark
    /// keeps both join-key columns when the on-clause is a Column
    /// expression rather than a string.
    Expression(Vec<(&'a str, TextRange)>),
}

#[allow(clippy::too_many_arguments)] // mostly source/line_index/diagnostics plumbing
pub(super) fn handle_two_df_method<'a>(
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
        TwoDfMethod::UnionByName if allow_missing_columns(call) => {
            // `unionByName(other, allowMissingColumns=True)` — schemas
            // are allowed to differ; missing fields on either side
            // become nullable in the output. Suppress D0040 and return
            // the union of both schemas.
            Some(apply_union_by_name_with_missing(
                left,
                &right,
                ctx.schemas(),
            ))
        }
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
            let on = parse_on_arg(extract_on_arg(call), ctx);
            check_join_keys(left, &right, &on, ctx, source, line_index, diagnostics);
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

/// Whether the call carries an `allowMissingColumns` keyword that
/// suppresses D0040.
///
/// Three cases:
/// - `True` literal → suppress.
/// - `False` literal or kwarg absent → keep the strict-match default.
/// - Non-literal (a variable, an expression) → suppress, on the
///   conservative-against-false-positives principle: the user has
///   asked for `unionByName` with a flag we can't statically resolve,
///   and emitting D0040 against a possibly-True flag is worse than
///   missing a possibly-False one.
fn allow_missing_columns(call: &ExprCall) -> bool {
    for kw in &call.arguments.keywords {
        if let Some(name) = kw.arg.as_ref()
            && name.id.as_str() == "allowMissingColumns"
        {
            return match kw.value.as_boolean_literal_expr() {
                Some(b) => b.value,
                None => true,
            };
        }
    }
    false
}

/// Output schema of `df.unionByName(other, allowMissingColumns=True)` —
/// the union of both sides' columns. Left columns appear first in
/// their original order; right-only columns are appended in the order
/// they appear on the right. Every field is made nullable (a row from
/// either side may not have populated the other side's columns).
fn apply_union_by_name_with_missing<'a>(
    left: &SchemaView<'a>,
    right: &SchemaView<'a>,
    schemas: &'a [Schema<'a>],
) -> SchemaView<'a> {
    let left_fields = left.typed_fields(schemas);
    let right_fields = right.typed_fields(schemas);
    let left_names: HashSet<&str> = left_fields.iter().map(|f| f.name).collect();
    let mut out: Vec<DerivedField<'a>> = left_fields
        .into_iter()
        .map(|f| with_nullability(f, true))
        .collect();
    for f in right_fields {
        if left_names.contains(f.name) {
            continue;
        }
        out.push(with_nullability(f, true));
    }
    SchemaView::Derived(out)
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

fn parse_on_arg<'a>(expr: Option<&'a Expr>, ctx: &BodyContext<'a>) -> JoinOn<'a> {
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
                // Mixed list (some strings, some Columns): bail out to
                // "complex expression" rather than half-checking.
                let mut refs = Vec::new();
                collect_col_refs(expr, ctx, &mut refs);
                return JoinOn::Expression(refs);
            };
            keys.push((s.value.to_str(), s.range()));
        }
        return JoinOn::Keys(keys);
    }
    // Column-expression on-clause — `col("a") == col("b")`, an AND of
    // them, etc. Walk the expression for every column reference so
    // `check_join_keys` can validate each against the union of both
    // sides.
    let mut refs = Vec::new();
    collect_col_refs(expr, ctx, &mut refs);
    JoinOn::Expression(refs)
}

fn check_join_keys<'a>(
    left: &SchemaView<'a>,
    right: &SchemaView<'a>,
    on: &JoinOn<'_>,
    ctx: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match on {
        JoinOn::None => {}
        JoinOn::Keys(keys) => {
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
        JoinOn::Expression(refs) => {
            // Column-expression on-clause: each ref must live on at
            // least one side (we can't know WHICH side a bare `col("X")`
            // belongs to). Names missing from both → D0030.
            //
            // Alias-qualified refs (`col("L.region")`) split on the
            // first `.` and resolve the prefix against any
            // `df.alias("L")` registered in scope. With the prefix
            // recognized, the suffix is checked against the aliased
            // schema specifically — the canonical join-disambiguation
            // pattern in real PySpark code. An unrecognized prefix
            // still fires D0030, but on the alias rather than the
            // whole name, with the in-scope aliases listed so the typo
            // is obvious.
            for (name, range) in refs {
                if let Some((prefix, suffix)) = split_qualified(name) {
                    if let Some(aliased) = ctx.lookup_alias(prefix) {
                        if !aliased.has_field(suffix) {
                            let suggestion = suggest_field_name(suffix, &aliased);
                            let mut message = format!(
                                "Column '{suffix}' does not exist on alias '{prefix}' ({}).",
                                aliased.display_name(),
                            );
                            if let Some(s) = &suggestion {
                                message.push_str(&format!(" Did you mean '{s}'?"));
                            }
                            diagnostics.push(
                                Diagnostic::at_range(
                                    Severity::Error,
                                    "D0030",
                                    message,
                                    *range,
                                    source,
                                    line_index,
                                )
                                .with_suggestion(suggestion),
                            );
                        }
                        continue;
                    }
                    // Prefix didn't match a registered alias. The
                    // diagnostic targets the prefix (the actual typo
                    // site) and lists what aliases ARE in scope so the
                    // fix is obvious.
                    let known = ctx.known_aliases();
                    let mut message = format!("Alias '{prefix}' is not in scope on this join.");
                    if known.is_empty() {
                        message.push_str(" No `.alias(...)` bindings have been registered.");
                    } else {
                        message.push_str(&format!(" Known aliases: {}.", known.join(", ")));
                    }
                    diagnostics.push(Diagnostic::at_range(
                        Severity::Error,
                        "D0030",
                        message,
                        *range,
                        source,
                        line_index,
                    ));
                    continue;
                }
                if left.has_field(name) || right.has_field(name) {
                    continue;
                }
                let suggestion =
                    suggest_field_name(name, left).or_else(|| suggest_field_name(name, right));
                let mut message = format!(
                    "Column '{name}' does not exist on {} or {}.",
                    left.display_name(),
                    right.display_name(),
                );
                if let Some(s) = &suggestion {
                    message.push_str(&format!(" Did you mean '{s}'?"));
                }
                diagnostics.push(
                    Diagnostic::at_range(
                        Severity::Error,
                        "D0030",
                        message,
                        *range,
                        source,
                        line_index,
                    )
                    .with_suggestion(suggestion),
                );
            }
        }
    }
}

/// Split a column reference of the form `"alias.col"` into its alias
/// and column halves. Returns `None` for unqualified names (no `.`),
/// for empty halves on either side, and for names with more than one
/// `.` (nested-struct accesses like `addr.city` aren't aliases and
/// must keep their existing field-path semantics — those are handled
/// elsewhere, never reaching this resolver). The narrow allowance
/// captures the canonical `df.alias("L"); col("L.region")` pattern
/// without rerouting real dotted paths.
fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let (prefix, suffix) = name.split_once('.')?;
    if prefix.is_empty() || suffix.is_empty() || suffix.contains('.') {
        return None;
    }
    Some((prefix, suffix))
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
/// - For `on=Expression`: Spark KEEPS both columns when the on-clause is a
///   Column expression (only the string/list form coalesces the keys).
///   Concatenate without dedup so subsequent `select` / `withColumn` calls
///   see the full schema of both sides.
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
    // For an expression-form on-clause, keep both sides' columns
    // verbatim (Spark's behavior); for the string/list and absent
    // forms, dedup shared non-key names with left winning.
    let dedup_shared = !matches!(on, JoinOn::Expression(_));
    for f in right.typed_fields(schemas) {
        // The named join key(s) are already in result from the left side.
        if dedup_set.contains(f.name) {
            continue;
        }
        if dedup_shared && result.iter().any(|r| r.name == f.name) {
            continue;
        }
        result.push(with_nullability(f, right_nullable));
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
pub(super) fn strip_nullability<'a>(
    view: &SchemaView<'a>,
    schemas: &'a [Schema<'a>],
) -> SchemaView<'a> {
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
