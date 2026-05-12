//! `.dpy` → `.py` transpiler.
//!
//! Most of a `.dpy` file is already valid Python — that's a load-bearing
//! design property (a "strict superset" — see the v0.1 spec). The
//! transpiler exists to bridge the few places where `.dpy` source uses
//! dathon-specific atoms that wouldn't survive Python's runtime evaluation
//! of annotations.
//!
//! The shape of the gap:
//!
//! - dathon's atomic type names — `string`, `double`, `timestamp`, `date`,
//!   `long`, `bool` — aren't Python builtins. If Python evaluates an
//!   annotation like `x: timestamp`, it raises `NameError`. With
//!   `from __future__ import annotations`, all annotations are treated as
//!   strings and never evaluated at runtime, so dathon's atomic names
//!   simply don't get looked up.
//!
//! - `DataFrame[X]`, `DataSource[X]`, and other `Generic[Schema]` shapes
//!   use `__class_getitem__` semantics. Some classes (e.g. PySpark's
//!   `DataFrame`) don't implement that, so evaluating these at runtime
//!   would raise `TypeError`. Same fix: `from __future__ import
//!   annotations` defers evaluation to strings.
//!
//! - User-declared `Schema` classes need the `Schema` base name to be
//!   bound to *something* at runtime. We don't inject this — users
//!   either import it from a future `dathon` runtime package or define a
//!   no-op base class themselves. The transpiler's job is to deal with
//!   the annotation-evaluation gap, not to ship a runtime.
//!
//! All of the above is covered by a single textual transformation:
//! prepend `from __future__ import annotations` if not already present.
//! No AST walking, no source reshaping. The output is the input plus
//! one line.
//!
//! Future iterations may extend this if/when dathon introduces syntax
//! that ISN'T valid Python (e.g. inline schema blocks). For now,
//! identity-plus-future-import is enough.

const FUTURE_ANNOTATIONS_IMPORT: &str = "from __future__ import annotations";

/// Transpile `.dpy` source into runnable `.py` source.
///
/// Returns the input with `from __future__ import annotations` prepended,
/// unless that import is already present (in which case the input is
/// returned unchanged).
pub fn transpile(source: &str) -> String {
    if source.contains(FUTURE_ANNOTATIONS_IMPORT) {
        return source.to_string();
    }
    format!("{FUTURE_ANNOTATIONS_IMPORT}\n\n{source}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepends_the_future_annotations_import_when_missing() {
        let input = "class Foo:\n    bar: int\n";
        let output = transpile(input);
        assert!(output.starts_with("from __future__ import annotations\n"));
        assert!(output.contains("class Foo:\n    bar: int\n"));
    }

    #[test]
    fn leaves_input_unchanged_when_future_import_already_present() {
        let input = "from __future__ import annotations\n\nx = 1\n";
        let output = transpile(input);
        assert_eq!(output, input);
        // No duplication.
        assert_eq!(output.matches(FUTURE_ANNOTATIONS_IMPORT).count(), 1);
    }

    #[test]
    fn handles_an_empty_input() {
        let output = transpile("");
        assert_eq!(output, "from __future__ import annotations\n\n");
    }

    #[test]
    fn preserves_the_original_source_byte_for_byte() {
        // After stripping the prepended line, the rest of the output must
        // be exactly the input. This is the "identity-plus-prepend"
        // property the rest of the transpiler design rests on.
        let input = "class Schema:\n    pass\n\nclass Orders(Schema):\n    x: timestamp\n";
        let output = transpile(input);
        let suffix = &output["from __future__ import annotations\n\n".len()..];
        assert_eq!(suffix, input);
    }
}
