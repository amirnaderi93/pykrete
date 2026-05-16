//! Virtual-document transform for the LSP multiplexer.
//!
//! `.dpy` files are valid Python, but dathon's magic names (`Schema`,
//! `DataFrame`, `col`) aren't defined anywhere the embedded Python
//! engine can see. So before forwarding a document to the child LSP,
//! dathon-lsp prepends a fixed **preamble** that defines those names.
//!
//! The user's real `.dpy` file keeps zero imports; the virtual document
//! the child analyzes is fully resolvable Python.
//!
//! Because the preamble is a fixed block of leading lines, mapping
//! positions between the real file and the virtual file is a constant
//! line offset — see [`to_child_line`] / [`to_editor_line`]. (If dathon
//! ever gains syntax that isn't valid Python, this upgrades to a real
//! source map; the offset is the degenerate case of that.)

/// The preamble injected at the top of every virtual document. Defines
/// dathon's magic names as ordinary Python so the embedded engine
/// resolves them. Kept on a single logical block so the line count is
/// stable and easy to reason about.
///
/// Two groups of names:
/// - `Schema` / `DataFrame` / `col` — dathon's magic identifiers.
/// - `string` / `date` / `timestamp` / `double` / `long` — dathon's
///   column-type keywords. They appear as *string literals* in schema
///   bodies (`EventDate: "date"`), which the embedded engine reads as
///   forward-reference type annotations; aliasing each to `object`
///   makes it resolve them cleanly instead of flagging them as
///   undefined. (`int` / `bool` are Python builtins and need no alias;
///   `object` rather than `Any` keeps the engine's no-explicit-`Any`
///   rule quiet on schema bodies.)
///
/// Every line here counts toward [`PREAMBLE_LINE_COUNT`]; if you edit
/// this string, the test `preamble_line_count_matches_constant` keeps
/// the constant honest.
pub const PREAMBLE: &str = "\
from typing import Any as _DathonAny, Generic as _DathonGeneric, TypeVar as _DathonT
_DT = _DathonT('_DT')
string = date = timestamp = double = long = object
class Schema: ...
class DataFrame(_DathonGeneric[_DT]): ...
def col(name: str) -> _DathonAny: ...
";

/// Number of newline-terminated lines in [`PREAMBLE`]. The virtual
/// document is `PREAMBLE` followed by the original source, so a real
/// 0-indexed line `n` sits at virtual line `n + PREAMBLE_LINE_COUNT`.
pub const PREAMBLE_LINE_COUNT: u32 = 6;

/// Build the virtual document the child LSP should see for a `.dpy`
/// file with the given real `source`.
pub fn to_virtual(source: &str) -> String {
    let mut out = String::with_capacity(PREAMBLE.len() + source.len());
    out.push_str(PREAMBLE);
    out.push_str(source);
    out
}

/// Map a real-file 0-indexed line to the virtual document's line.
pub fn to_child_line(real_line: u32) -> u32 {
    real_line + PREAMBLE_LINE_COUNT
}

/// Map a virtual-document 0-indexed line back to the real file.
///
/// Returns `None` when the line falls inside the injected preamble —
/// the child occasionally reports diagnostics or positions there (e.g.
/// an unused-import warning on the preamble itself); those must be
/// dropped, not shown to the user.
pub fn to_editor_line(virtual_line: u32) -> Option<u32> {
    virtual_line.checked_sub(PREAMBLE_LINE_COUNT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preamble_line_count_matches_constant() {
        let actual = PREAMBLE.lines().count() as u32;
        assert_eq!(
            actual, PREAMBLE_LINE_COUNT,
            "PREAMBLE_LINE_COUNT ({PREAMBLE_LINE_COUNT}) is out of sync with \
             the preamble's actual line count ({actual}) — update the constant",
        );
    }

    #[test]
    fn preamble_ends_with_newline_so_source_starts_on_its_own_line() {
        // The original source is concatenated directly after the
        // preamble; the preamble must end with `\n` or the first line
        // of user code would be glued onto the last preamble line.
        assert!(PREAMBLE.ends_with('\n'));
    }

    #[test]
    fn to_virtual_prepends_the_preamble_verbatim() {
        let v = to_virtual("class Orders(Schema):\n    x: \"int\"\n");
        assert!(v.starts_with(PREAMBLE));
        assert!(v.ends_with("class Orders(Schema):\n    x: \"int\"\n"));
    }

    #[test]
    fn line_mapping_round_trips_for_real_lines() {
        for real in [0u32, 1, 5, 100, 9999] {
            let child = to_child_line(real);
            assert_eq!(child, real + PREAMBLE_LINE_COUNT);
            assert_eq!(to_editor_line(child), Some(real));
        }
    }

    #[test]
    fn to_editor_line_drops_positions_inside_the_preamble() {
        // Lines 0..PREAMBLE_LINE_COUNT are the injected preamble — no
        // real-file counterpart.
        for v in 0..PREAMBLE_LINE_COUNT {
            assert_eq!(to_editor_line(v), None);
        }
        // The first line past the preamble maps to real line 0.
        assert_eq!(to_editor_line(PREAMBLE_LINE_COUNT), Some(0));
    }
}
