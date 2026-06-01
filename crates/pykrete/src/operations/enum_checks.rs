//! v1.1 enum constraint check sites — emitting `D0084 enumValueMismatch`.
//!
//! Each call site that compares or assigns a literal against an
//! enum-typed sink resolves to one of the helpers here. The precedence
//! chain `D0030 > D0082 > D0084` is enforced at the call site: callers
//! invoke an enum check only after the column has resolved and (when
//! applicable) the cross-type check has passed.
//!
//! Suggestion search uses the same `max(1, target.len() / 3)`
//! Levenshtein threshold as `D0030` and breaks ties by Unicode
//! code-point order (`str::cmp`), per the spec's locked tiebreaker.

use ruff_python_ast::Expr;
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::levenshtein;
use crate::types::ColumnType;

/// The enum vocabulary, if `ty` is an enum (peeling any `Nullable`
/// wrapper).
pub(super) fn enum_vocab(ty: &ColumnType) -> Option<&[String]> {
    match ty.base() {
        ColumnType::Enum(values) => Some(values),
        _ => None,
    }
}

/// Closest vocabulary entry to `target` within the shared
/// `max(1, target.len() / 3)` Levenshtein threshold. Ties resolved by
/// Unicode code-point order (Rust `str::cmp`).
pub(super) fn closest_enum_value(target: &str, vocab: &[String]) -> Option<String> {
    let threshold = std::cmp::max(1, target.len() / 3);
    let mut best: Option<(&str, usize)> = None;
    for candidate in vocab {
        let d = levenshtein(target, candidate);
        if d > threshold {
            continue;
        }
        best = Some(match best {
            None => (candidate.as_str(), d),
            Some((_, prev_d)) if d < prev_d => (candidate.as_str(), d),
            Some((prev, prev_d)) if d == prev_d && candidate.as_str() < prev => {
                (candidate.as_str(), d)
            }
            Some(prev) => prev,
        });
    }
    best.map(|(name, _)| name.to_string())
}

/// Emit a `D0084` for `value` not in the enum vocabulary of `column`'s
/// declared type. `range` is the source span of the offending literal.
/// A close-match suggestion is attached and surfaced in the message
/// when one exists within the shared Levenshtein threshold.
pub(super) fn emit_d0084(
    column: &str,
    value: &str,
    vocab: &[String],
    range: TextRange,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let suggestion = closest_enum_value(value, vocab);
    let mut message = format!("'{value}' is not in the enum vocabulary for '{column}'.");
    if let Some(s) = &suggestion {
        message.push_str(&format!(" Did you mean '{s}'?"));
    }
    diagnostics.push(
        Diagnostic::at_range(Severity::Error, "D0084", message, range, source, line_index)
            .with_suggestion(suggestion),
    );
}

/// True if `value` is in `vocab` (byte-exact, Q1a semantics).
pub(super) fn vocab_contains(vocab: &[String], value: &str) -> bool {
    vocab.iter().any(|v| v == value)
}

/// If `expr` is `lit("...")` / `F.lit("...")` with a string-literal
/// argument, the (literal text, source range) pair. Used by the
/// branch-form helpers to peel a `lit(...)` wrapper before checking.
/// A bare string literal also qualifies — `coalesce(col("x"), "a")`
/// is the same shape as `coalesce(col("x"), F.lit("a"))` in PySpark.
pub(super) fn peel_string_literal(expr: &Expr) -> Option<(&str, TextRange)> {
    if let Some(s) = expr.as_string_literal_expr() {
        return Some((s.value.to_str(), s.range()));
    }
    let call = expr.as_call_expr()?;
    let fname = match call.func.as_ref() {
        Expr::Name(n) => n.id.as_str(),
        Expr::Attribute(a) => a.attr.id.as_str(),
        _ => return None,
    };
    if fname != "lit" {
        return None;
    }
    let arg = call.arguments.args.first()?;
    let s = arg.as_string_literal_expr()?;
    Some((s.value.to_str(), s.range()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_enum_value_picks_within_threshold() {
        let vocab = vec!["pending".to_string(), "shipped".to_string()];
        assert_eq!(
            closest_enum_value("pendig", &vocab),
            Some("pending".to_string()),
        );
    }

    #[test]
    fn closest_enum_value_returns_none_when_nothing_close() {
        let vocab = vec!["pending".to_string()];
        // 'totally_unrelated' / 'pending' edit distance way beyond
        // max(1, 17/3) = 5.
        assert_eq!(closest_enum_value("totally_unrelated", &vocab), None);
    }

    #[test]
    fn closest_enum_value_breaks_ties_by_unicode_code_point_order() {
        // Both "pendinx" and "pendiny" are edit distance 1 from
        // "pendinz" (target len 7 → threshold max(1, 7/3) = 2). Tie
        // broken by str::cmp — "pendinx" sorts first. Falsifies a
        // declaration-order tiebreaker.
        let vocab = vec!["pendiny".to_string(), "pendinx".to_string()];
        assert_eq!(
            closest_enum_value("pendinz", &vocab),
            Some("pendinx".to_string()),
        );
    }

    #[test]
    fn enum_vocab_peels_nullable_wrapper() {
        let inner = ColumnType::Enum(vec!["a".into()]);
        let ty = ColumnType::Nullable(Box::new(inner));
        assert_eq!(enum_vocab(&ty), Some(&["a".to_string()][..]));
    }
}
