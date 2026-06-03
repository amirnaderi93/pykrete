//! Iteration 38: project-aware LSP entry points.
//!
//! `hover_in_project`, `completions_in_project`, and
//! `definition_in_project` accept a full project snapshot (every
//! `.pyk` file in the project, with open buffers' in-memory content
//! overriding disk) plus a focus path + cursor. Cross-file Schema
//! references that were already understood by `check_project` should
//! now also resolve in hover popups, completion lists, and
//! Cmd-click jumps.

#![allow(non_snake_case)]

use pykrete::{completions_in_project, definition_in_project, hover_in_project};

fn project(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(p, s)| ((*p).to_string(), (*s).to_string()))
        .collect()
}

#[test]
fn hover_on_imported_schema_reference_resolves_across_files() {
    // schemas.pyk declares Orders. pipeline.pyk imports it and uses
    // it in a typed signature. Hovering on `Orders` inside
    // `SparkFrame[Orders]` in pipeline.pyk must show Orders' fields,
    // even though Orders lives in a sibling file.
    let files = project(&[
        (
            "/proj/schemas.pyk",
            r#"
class Orders(Schema):
    place_code: int
    price: int
"#,
        ),
        (
            "/proj/pipeline.pyk",
            r#"
from .schemas import Orders

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw
"#,
        ),
    ]);
    // Cursor on the `O` of `SparkFrame[Orders]` (the second `Orders`
    // in pipeline.pyk — line 4, char "def f(raw: SparkFrame[".len() + 1).
    let line = 4;
    let column = "def f(raw: SparkFrame[".len() + 1;
    let info = hover_in_project(&files, "/proj/pipeline.pyk", line, column)
        .expect("expected cross-file hover info");
    assert!(info.markdown.contains("Orders"));
    assert!(info.markdown.contains("place_code"));
    assert!(info.markdown.contains("price"));
}

#[test]
fn completion_inside_dataframe_subscript_lists_imported_schemas() {
    let files = project(&[
        (
            "/proj/schemas.pyk",
            r#"
class Orders(Schema):
    x: int

class Returns(Schema):
    y: int
"#,
        ),
        (
            "/proj/pipeline.pyk",
            r#"
from .schemas import Orders, Returns

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw
"#,
        ),
    ]);
    // Cursor on the `O` of `SparkFrame[Orders]` slot inside the def
    // line — completion should list both imported schemas.
    let line = 4;
    let column = "def f(raw: SparkFrame[".len() + 1;
    let items = completions_in_project(&files, "/proj/pipeline.pyk", line, column);
    let mut names: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["Orders", "Returns"]);
}

#[test]
fn goto_definition_on_imported_schema_jumps_to_the_declaring_file() {
    let files = project(&[
        (
            "/proj/schemas.pyk",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "/proj/pipeline.pyk",
            r#"
from .schemas import Orders

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw
"#,
        ),
    ]);
    let line = 4;
    let column = "def f(raw: SparkFrame[".len() + 1;
    let (path, span) = definition_in_project(&files, "/proj/pipeline.pyk", line, column)
        .expect("expected a definition");
    // Resolves across files: the `Orders` class is declared in
    // schemas.pyk, so the definition points there — not at the focus
    // file — at `class Orders` on line 2.
    assert_eq!(path, "/proj/schemas.pyk");
    assert_eq!(span.start_line, 2);
    assert_eq!(span.start_column, "class ".len() + 1);
}

#[test]
fn goto_definition_on_column_ref_jumps_to_the_imported_schemas_field() {
    // The schema is imported, not declared locally. Clicking the
    // column literal must jump to the field in the *schemas* file —
    // regression test for the column ref landing at a stray position
    // in the focus file (its byte range read against the wrong text).
    let files = project(&[
        (
            "/proj/schemas.pyk",
            r#"
class Orders(Schema):
    place_code: int
    price: int
"#,
        ),
        (
            "/proj/pipeline.pyk",
            r#"
from .schemas import Orders

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("price"))
"#,
        ),
    ]);
    // Cursor inside `col("price")` on line 5 of pipeline.pyk.
    let line = 5;
    let column = "    return raw.select(col(\"pr".len() + 1;
    let (path, span) = definition_in_project(&files, "/proj/pipeline.pyk", line, column)
        .expect("expected a definition");
    // `price` is declared on line 4 of schemas.pyk.
    assert_eq!(path, "/proj/schemas.pyk");
    assert_eq!(span.start_line, 4);
    assert_eq!(span.start_column, "    ".len() + 1);
}

#[test]
fn goto_definition_on_import_module_jumps_to_the_module_file() {
    // Clicking the module name of `from .schemas import …` jumps to
    // the schemas file itself.
    let files = project(&[
        (
            "/proj/schemas.pyk",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "/proj/pipeline.pyk",
            r#"
from .schemas import Orders

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw
"#,
        ),
    ]);
    // Cursor inside `schemas` of `from .schemas import Orders` (line 2).
    let line = 2;
    let column = "from .sch".len() + 1;
    let (path, span) = definition_in_project(&files, "/proj/pipeline.pyk", line, column)
        .expect("expected a definition");
    assert_eq!(path, "/proj/schemas.pyk");
    // Jumps to the top of the module file.
    assert_eq!(span.start_line, 1);
}
