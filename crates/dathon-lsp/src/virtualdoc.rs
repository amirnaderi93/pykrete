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
//! The injected prefix is a fixed number of leading lines, so mapping
//! positions between the real file and the virtual file is a constant
//! line offset — see [`to_child_line`] / [`to_editor_line`]. (If dathon
//! ever gains syntax that isn't valid Python, this upgrades to a real
//! source map; the offset is the degenerate case of that.)
//!
//! One wrinkle: Python requires `from __future__` imports to be the
//! first statement in a file. The preamble would push the user's down,
//! so [`to_virtual`] **hoists** them — collects every `from __future__
//! import …`, coalesces the names onto a single line above the
//! preamble, and blanks the originals (keeping line numbers stable).

/// The preamble injected below the hoisted `__future__` line. Defines
/// dathon's magic names as ordinary Python so the embedded engine
/// resolves them.
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
/// The test `preamble_line_count_matches_constant` keeps
/// [`PREFIX_LINE_COUNT`] honest if this string is edited.
pub const PREAMBLE: &str = "\
from typing import Any as _DathonAny, Generic as _DathonGeneric, TypeVar as _DathonT
_DT = _DathonT('_DT')
string = date = timestamp = double = long = object
class Schema: ...
class DataFrame(_DathonGeneric[_DT]): ...
def col(name: str) -> _DathonAny: ...
";

/// Newline-terminated line count of [`PREAMBLE`].
const PREAMBLE_LINE_COUNT: u32 = 6;

/// Lines the virtual document injects ahead of the user's source: one
/// hoisted `from __future__` line plus the [`PREAMBLE`]. A real
/// 0-indexed line `n` sits at virtual line `n + PREFIX_LINE_COUNT`.
pub const PREFIX_LINE_COUNT: u32 = PREAMBLE_LINE_COUNT + 1;

/// Build the virtual document the child LSP should see for a `.dpy`
/// file with the given real `source`.
///
/// Layout: `[hoisted __future__ line][preamble][source]`. The
/// `__future__` line is empty when the source has no such imports —
/// the prefix is always [`PREFIX_LINE_COUNT`] lines either way.
pub fn to_virtual(source: &str) -> String {
    let (future_line, body) = hoist_future_imports(source);
    let mut out = String::with_capacity(future_line.len() + 1 + PREAMBLE.len() + body.len());
    out.push_str(&future_line);
    out.push('\n');
    out.push_str(PREAMBLE);
    out.push_str(&body);
    out
}

/// Split `source` into a single coalesced `from __future__ import …`
/// line (empty when there are none) and a body with every `__future__`
/// import blanked out — the body keeps the source's line count so the
/// constant-offset position mapping still holds.
fn hoist_future_imports(source: &str) -> (String, String) {
    let mut names: Vec<String> = Vec::new();
    let mut body_lines: Vec<&str> = Vec::new();
    for line in source.lines() {
        match parse_future_import(line) {
            Some(line_names) => {
                for name in line_names {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                body_lines.push(""); // blanked — line numbering preserved
            }
            None => body_lines.push(line),
        }
    }
    let future_line = if names.is_empty() {
        String::new()
    } else {
        format!("from __future__ import {}", names.join(", "))
    };
    (future_line, body_lines.join("\n"))
}

/// If `line` is a `from __future__ import …` statement, return the
/// imported names; otherwise `None`. Handles the single-line forms
/// (`from __future__ import a, b` and `from __future__ import (a, b)`);
/// the rare parenthesized multi-line form is left alone.
fn parse_future_import(line: &str) -> Option<Vec<String>> {
    let rest = line.trim_start().strip_prefix("from __future__ import")?;
    // A separator must follow, or `from __future__ importer` would match.
    if !rest.is_empty() && !rest.starts_with([' ', '\t', '(']) {
        return None;
    }
    let names: Vec<String> = rest
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(String::from)
        .collect();
    (!names.is_empty()).then_some(names)
}

/// The 0-indexed editor line of the first `from __future__` import in
/// `source`, if any. [`to_virtual`] hoists that import into the virtual
/// document's preamble region, so callers that remap positions back
/// (e.g. semantic tokens) need to know where it really lives.
pub fn first_future_import_line(source: &str) -> Option<u32> {
    source
        .lines()
        .position(|line| parse_future_import(line).is_some())
        .map(|index| index as u32)
}

/// Map a real-file 0-indexed line to the virtual document's line.
pub fn to_child_line(real_line: u32) -> u32 {
    real_line + PREFIX_LINE_COUNT
}

/// Map a virtual-document 0-indexed line back to the real file.
///
/// Returns `None` when the line falls inside the injected prefix —
/// the child occasionally reports diagnostics or positions there (e.g.
/// an unused-import warning on the preamble itself); those must be
/// dropped, not shown to the user.
pub fn to_editor_line(virtual_line: u32) -> Option<u32> {
    virtual_line.checked_sub(PREFIX_LINE_COUNT)
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
        assert!(PREAMBLE.ends_with('\n'));
    }

    #[test]
    fn to_virtual_injects_a_fixed_prefix_then_the_source() {
        let v = to_virtual("class Orders(Schema):\n    x: \"int\"\n");
        // Line 0 is the (empty here) `__future__` line; the preamble
        // follows; the source is last.
        assert!(v.starts_with('\n'));
        assert!(v.contains(PREAMBLE));
        assert!(v.contains("class Orders(Schema):"));
        // The prefix is exactly PREFIX_LINE_COUNT lines.
        let prefix: String = v
            .lines()
            .take(PREFIX_LINE_COUNT as usize)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(prefix.lines().count() as u32, PREFIX_LINE_COUNT);
    }

    #[test]
    fn to_virtual_hoists_a_future_import_above_the_preamble() {
        let v = to_virtual("from __future__ import annotations\nclass Orders(Schema): ...\n");
        // The `__future__` import is the very first line — ahead of the
        // preamble — so the engine doesn't flag it as misplaced.
        assert!(v.starts_with("from __future__ import annotations\n"));
        // Its original line is blanked; the class still maps by offset.
        let lines: Vec<&str> = v.lines().collect();
        assert_eq!(lines[PREFIX_LINE_COUNT as usize], "");
        assert_eq!(
            lines[PREFIX_LINE_COUNT as usize + 1],
            "class Orders(Schema): ..."
        );
    }

    #[test]
    fn to_virtual_coalesces_multiple_future_imports_onto_one_line() {
        let v = to_virtual(
            "from __future__ import annotations\nfrom __future__ import division\nx = 1\n",
        );
        assert!(v.starts_with("from __future__ import annotations, division\n"));
        // Both originals blanked → `x = 1` is still at real line 2.
        let lines: Vec<&str> = v.lines().collect();
        assert_eq!(lines[to_child_line(2) as usize], "x = 1");
    }

    #[test]
    fn parse_future_import_recognizes_the_common_forms() {
        assert_eq!(
            parse_future_import("from __future__ import annotations"),
            Some(vec!["annotations".to_string()]),
        );
        assert_eq!(
            parse_future_import("from __future__ import (annotations, division)"),
            Some(vec!["annotations".to_string(), "division".to_string()]),
        );
        // Not a `__future__` import.
        assert_eq!(
            parse_future_import("from pyspark.sql import functions"),
            None
        );
        assert_eq!(parse_future_import("x = 1"), None);
    }

    #[test]
    fn line_mapping_round_trips_for_real_lines() {
        for real in [0u32, 1, 5, 100, 9999] {
            let child = to_child_line(real);
            assert_eq!(child, real + PREFIX_LINE_COUNT);
            assert_eq!(to_editor_line(child), Some(real));
        }
    }

    #[test]
    fn to_editor_line_drops_positions_inside_the_prefix() {
        // Lines 0..PREFIX_LINE_COUNT are the injected prefix — no
        // real-file counterpart.
        for v in 0..PREFIX_LINE_COUNT {
            assert_eq!(to_editor_line(v), None);
        }
        // The first line past the prefix maps to real line 0.
        assert_eq!(to_editor_line(PREFIX_LINE_COUNT), Some(0));
    }
}
