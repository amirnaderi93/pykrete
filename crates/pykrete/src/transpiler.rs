//! `.pyk` → `.py` transpiler.
//!
//! Most of a `.pyk` file is already valid Python — that's a load-bearing
//! design property (a "strict superset" — see the v0.1 spec). The
//! transpiler bridges the two places `.pyk` source diverges from runnable
//! Python:
//!
//! 1. **Annotation atoms that wouldn't survive runtime evaluation.**
//!    pykrete's atomic type names (`string`, `double`, `timestamp`, …) and
//!    its `DataFrame[X]` / `Array[T]` / `Map[K, V]` annotation shapes
//!    aren't Python builtins, and `DataFrame[X]` leans on
//!    `__class_getitem__` semantics PySpark's `DataFrame` doesn't
//!    implement. Prepending `from __future__ import annotations` defers
//!    every annotation to an unevaluated string, so none of them are
//!    looked up at runtime.
//!
//! 2. **The fluent schema-cast — `<chain>.cast(DataFrame[Schema])`.**
//!    Unlike the above this sits in *expression* position, not an
//!    annotation, so the future import doesn't reach it. A `DataFrame`
//!    has no `.cast` method; the call would raise `AttributeError` at
//!    runtime. The schema-cast is a pykrete-only re-anchoring hint for the
//!    checker — see [`crate::operations`] — and carries no runtime
//!    meaning, so the transpiler removes the `.cast(…)` segment from the
//!    call chain, leaving the receiver in place.
//!
//! User-declared `Schema` classes still need the `Schema` base name bound
//! to *something* at runtime — users import it from a pykrete runtime
//! package or define a no-op base themselves. Shipping that runtime is
//! out of scope for the transpiler.
//!
//! Stripping the schema-cast is the transpiler's one AST-aware step: it
//! parses the source, locates every `.cast(DataFrame[…])` call, and
//! deletes just those slices by byte range. Everything else is copied
//! verbatim — no reformatting, no re-indentation — and a deletion starts
//! at the `.` of `.cast`, never the newline before it, so a `.cast(…)`
//! on its own line in a chain collapses to a blank line rather than
//! pulling the next line up. Line numbers downstream stay stable, which
//! keeps runtime tracebacks pointing at the right `.pyk` line.

use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_body, walk_expr};
use ruff_python_ast::{Expr, ExprCall};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::dataframe::{self, DataFrameAnnotation};

const FUTURE_ANNOTATIONS_IMPORT: &str = "from __future__ import annotations";

/// Transpile `.pyk` source into runnable `.py` source.
///
/// Strips every pykrete-only `.cast(DataFrame[Schema])` schema-cast, then
/// prepends `from __future__ import annotations` (unless already present,
/// in which case no second copy is added).
pub fn transpile(source: &str) -> String {
    let body = strip_schema_casts(source);
    if body.contains(FUTURE_ANNOTATIONS_IMPORT) {
        body
    } else {
        format!("{FUTURE_ANNOTATIONS_IMPORT}\n\n{body}")
    }
}

/// Remove every `<receiver>.cast(DataFrame[Schema])` call from `source`,
/// returning the rewritten source. The receiver is kept; only the
/// `.cast(…)` segment is deleted.
///
/// On unparseable input the source is returned untouched — the transpiler
/// is not a checker, and the user gets a Python syntax error when they
/// run the output, the same as the checker's `D0001`.
fn strip_schema_casts(source: &str) -> String {
    let Ok(parsed) = ruff_python_parser::parse_module(source) else {
        return source.to_string();
    };
    let mut finder = SchemaCastFinder {
        source,
        ranges: Vec::new(),
    };
    walk_body(&mut finder, &parsed.syntax().body);
    if finder.ranges.is_empty() {
        return source.to_string();
    }
    // Delete the slices high-offset-first so the earlier offsets stay
    // valid as we go. Each range is exactly one `.cast(…)` AST node, so
    // the ranges are pairwise non-overlapping (disjoint, or adjacent for
    // a `.cast(…).cast(…)` chain) — a plain reverse sort is enough.
    let mut ranges = finder.ranges;
    ranges.sort_unstable_by_key(|r| std::cmp::Reverse(r.start()));
    let mut out = source.to_string();
    for range in ranges {
        out.replace_range(usize::from(range.start())..usize::from(range.end()), "");
    }
    out
}

/// Source-order walk that collects the byte range of every schema-cast's
/// `.cast(…)` segment.
struct SchemaCastFinder<'a> {
    source: &'a str,
    ranges: Vec<TextRange>,
}

impl<'a> SourceOrderVisitor<'a> for SchemaCastFinder<'a> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Call(call) = expr
            && let Some(range) = schema_cast_strip_range(call, self.source)
        {
            self.ranges.push(range);
        }
        // Keep walking — a schema-cast can appear inside another call's
        // receiver or arguments (`a.cast(DataFrame[A]).cast(DataFrame[B])`).
        walk_expr(self, expr);
    }
}

/// If `call` is pykrete's fluent schema-cast — `<receiver>.cast(DataFrame[Schema])`
/// — return the byte range to delete: the `.cast(…)` slice, from its
/// leading `.` through the call's closing `)`. Returns `None` for every
/// other call, including the ordinary `Column.cast("int")` (whose
/// argument is a type *string*, never a `DataFrame[…]` subscript).
fn schema_cast_strip_range(call: &ExprCall, source: &str) -> Option<TextRange> {
    let attr = call.func.as_attribute_expr()?;
    if attr.attr.id.as_str() != "cast" {
        return None;
    }
    // The schema-cast is always `.cast(DataFrame[BareSchema])` — exactly
    // one positional argument, no keywords. Requiring that precise shape
    // keeps the transpiler from ever deleting an argument it can't
    // account for, and matches what the checker treats as a schema-cast.
    if call.arguments.args.len() != 1 || !call.arguments.keywords.is_empty() {
        return None;
    }
    if !matches!(
        dataframe::recognize(&call.arguments.args[0]),
        Some(DataFrameAnnotation::Typed(_) | DataFrameAnnotation::Derived(_)),
    ) {
        return None;
    }

    // Anchor the deletion at the `.` before `cast`, not the receiver's
    // end: that leaves any newline + indentation that precedes a chained
    // `.cast(…)` untouched, so the line it sat on survives as blank.
    let recv_end = attr.value.range().end();
    let cast_name_start = attr.attr.range().start();
    let gap = source.get(usize::from(recv_end)..usize::from(cast_name_start))?;
    let dot = recv_end + TextSize::try_from(gap.rfind('.')?).ok()?;
    Some(TextRange::new(dot, call.range().end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// True if `src` is valid Python — used to assert transpiled output
    /// is actually runnable, not just textually plausible.
    fn parses(src: &str) -> bool {
        ruff_python_parser::parse_module(src).is_ok()
    }

    // -- future-import behaviour ------------------------------------------

    #[test]
    fn prepends_the_future_annotations_import_when_missing() {
        let output = transpile("class Foo:\n    bar: int\n");
        assert!(output.starts_with("from __future__ import annotations\n"));
        assert!(output.contains("class Foo:\n    bar: int\n"));
    }

    #[test]
    fn leaves_input_unchanged_when_future_import_already_present() {
        let input = "from __future__ import annotations\n\nx = 1\n";
        let output = transpile(input);
        assert_eq!(output, input);
        assert_eq!(output.matches(FUTURE_ANNOTATIONS_IMPORT).count(), 1);
    }

    #[test]
    fn handles_an_empty_input() {
        assert_eq!(transpile(""), "from __future__ import annotations\n\n");
    }

    #[test]
    fn preserves_a_cast_free_source_byte_for_byte() {
        // With no schema-cast to strip, the transpile is identity-plus-
        // prepend: the suffix after the import is exactly the input.
        let input = "class Schema:\n    pass\n\nclass Orders(Schema):\n    x: timestamp\n";
        let output = transpile(input);
        let suffix = &output["from __future__ import annotations\n\n".len()..];
        assert_eq!(suffix, input);
    }

    // -- schema-cast stripping --------------------------------------------

    #[test]
    fn strips_an_inline_schema_cast() {
        let out = transpile("def f(d):\n    return d.cast(DataFrame[Foo]).select(col(\"a\"))\n");
        assert!(!out.contains("cast"), "cast not stripped: {out}");
        assert!(out.contains("return d.select(col(\"a\"))"));
        assert!(parses(&out));
    }

    #[test]
    fn strips_a_trailing_schema_cast() {
        let out = transpile("def f(d):\n    return d.cast(DataFrame[Foo])\n");
        assert!(out.contains("return d\n"));
        assert!(!out.contains("cast"));
        assert!(parses(&out));
    }

    #[test]
    fn leaves_a_column_cast_untouched() {
        // `col("x").cast("int")` is real PySpark — the argument is a type
        // string, not a `DataFrame[…]` subscript, so it must survive.
        let out = transpile("def f(d):\n    return d.select(col(\"x\").cast(\"int\"))\n");
        assert!(out.contains("cast(\"int\")"));
        assert!(parses(&out));
    }

    #[test]
    fn strips_chained_schema_casts() {
        let out = transpile("def f(d):\n    return d.cast(DataFrame[A]).cast(DataFrame[B])\n");
        assert!(!out.contains("cast"));
        assert!(out.contains("return d\n"));
        assert!(parses(&out));
    }

    #[test]
    fn preserves_line_count_when_a_cast_is_on_its_own_line() {
        let src = "def f(d):\n    return (\n        d\n        .cast(DataFrame[Foo])\n        .select(col(\"a\"))\n    )\n";
        let out = transpile(src);
        let body = &out[out.find("def f").expect("def f present")..];
        assert_eq!(
            body.lines().count(),
            src.lines().count(),
            "stripping a stand-alone `.cast` line must not shift line numbers",
        );
        assert!(!body.contains("cast"));
        assert!(parses(&out));
    }

    #[test]
    fn strips_a_cast_to_an_unknown_schema() {
        // The checker flags `DataFrame[Bogus]` (D0020); the transpiler
        // still strips it — the runtime `.cast` would fail regardless.
        let out = transpile("def f(d):\n    return d.cast(DataFrame[Bogus])\n");
        assert!(!out.contains("cast"));
        assert!(parses(&out));
    }

    #[test]
    fn falls_back_to_prepend_only_on_unparseable_input() {
        let src = "def broken(:\n";
        let out = transpile(src);
        assert!(out.starts_with(FUTURE_ANNOTATIONS_IMPORT));
        assert!(out.contains(src));
    }

    #[test]
    fn transpile_is_idempotent_even_with_a_schema_cast() {
        let src = "def f(d):\n    return d.cast(DataFrame[Foo]).select(col(\"a\"))\n";
        let once = transpile(src);
        let twice = transpile(&once);
        assert_eq!(once, twice);
    }
}
