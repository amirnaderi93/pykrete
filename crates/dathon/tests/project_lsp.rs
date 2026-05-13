//! Iteration 38: project-aware LSP entry points.
//!
//! `hover_in_project`, `completions_in_project`, and
//! `definition_in_project` accept a full project snapshot (every
//! `.dpy` file in the project, with open buffers' in-memory content
//! overriding disk) plus a focus path + cursor. Cross-file Schema
//! references that were already understood by `check_project` should
//! now also resolve in hover popups, completion lists, and
//! Cmd-click jumps.

#![allow(non_snake_case)]

use dathon::{completions_in_project, definition_in_project, hover_in_project};

fn project(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(p, s)| ((*p).to_string(), (*s).to_string()))
        .collect()
}

#[test]
fn hover_on_imported_schema_reference_resolves_across_files() {
    // schemas.dpy declares Orders. pipeline.dpy imports it and uses
    // it in a typed signature. Hovering on `Orders` inside
    // `DataFrame[Orders]` in pipeline.dpy must show Orders' fields,
    // even though Orders lives in a sibling file.
    let files = project(&[
        (
            "/proj/schemas.dpy",
            r#"
class Orders(Schema):
    place_code: int
    price: int
"#,
        ),
        (
            "/proj/pipeline.dpy",
            r#"
from .schemas import Orders

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw
"#,
        ),
    ]);
    // Cursor on the `O` of `DataFrame[Orders]` (the second `Orders`
    // in pipeline.dpy — line 4, char "def f(raw: DataFrame[".len() + 1).
    let line = 4;
    let column = "def f(raw: DataFrame[".len() + 1;
    let info = hover_in_project(&files, "/proj/pipeline.dpy", line, column)
        .expect("expected cross-file hover info");
    assert!(info.markdown.contains("Orders"));
    assert!(info.markdown.contains("place_code"));
    assert!(info.markdown.contains("price"));
}

#[test]
fn completion_inside_dataframe_subscript_lists_imported_schemas() {
    let files = project(&[
        (
            "/proj/schemas.dpy",
            r#"
class Orders(Schema):
    x: int

class Returns(Schema):
    y: int
"#,
        ),
        (
            "/proj/pipeline.dpy",
            r#"
from .schemas import Orders, Returns

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw
"#,
        ),
    ]);
    // Cursor on the `O` of `DataFrame[Orders]` slot inside the def
    // line — completion should list both imported schemas.
    let line = 4;
    let column = "def f(raw: DataFrame[".len() + 1;
    let items = completions_in_project(&files, "/proj/pipeline.dpy", line, column);
    let mut names: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["Orders", "Returns"]);
}

#[test]
fn goto_definition_on_imported_schema_returns_span_in_focus_file() {
    let files = project(&[
        (
            "/proj/schemas.dpy",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "/proj/pipeline.dpy",
            r#"
from .schemas import Orders

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw
"#,
        ),
    ]);
    let line = 4;
    let column = "def f(raw: DataFrame[".len() + 1;
    let span = definition_in_project(&files, "/proj/pipeline.dpy", line, column)
        .expect("expected a definition span");
    // The span points at the `Orders` class declaration. Today the
    // span is anchored against the focus file's source coordinates
    // (we don't yet return a cross-file URI), so we just confirm a
    // span was returned at all — it indicates the lookup succeeded.
    assert!(span.start_line > 0);
}
