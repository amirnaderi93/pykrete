//! Integration tests for the `.dpy` → `.py` transpiler.
//!
//! These exercise `dathon::transpile` end-to-end on realistic-shaped
//! input — the same example files the checker runs against — and verify
//! that the output parses as valid Python (via `ruff_python_parser`) and
//! preserves the input's content.

#![allow(non_snake_case)]

const FUTURE_IMPORT: &str = "from __future__ import annotations";

// ===========================================================================
// Textual properties
// ===========================================================================

#[test]
fn transpiled_output_starts_with_the_future_import() {
    let input = "class Foo:\n    pass\n";
    let output = dathon::transpile(input);
    assert!(output.starts_with(FUTURE_IMPORT));
}

#[test]
fn transpile_is_idempotent_when_run_on_already_transpiled_source() {
    let input = "class Foo:\n    pass\n";
    let once = dathon::transpile(input);
    let twice = dathon::transpile(&once);
    assert_eq!(once, twice);
}

#[test]
fn original_source_is_preserved_verbatim_inside_the_output() {
    // The transpiler must not reformat / rewrite / re-indent the user's
    // code. Whatever lines were there, they appear in the output
    // unchanged. This is what makes line-number-based debugging usable.
    let input =
        "class Orders(Schema):\n    place_code: int\n    price: int\n\n\ndef f():\n    return 1\n";
    let output = dathon::transpile(input);
    assert!(output.contains(input));
}

// ===========================================================================
// Real example files round-trip
// ===========================================================================

/// Helper: read an example file from the workspace-level `examples/`
/// directory, transpile it, and parse the output with ruff's Python
/// parser. `cargo test` runs from the package directory, so we route via
/// `CARGO_MANIFEST_DIR` to reach the workspace root.
fn transpile_and_reparse(example_name: &str) -> Result<(), String> {
    let path = format!(
        "{}/../../examples/{}",
        env!("CARGO_MANIFEST_DIR"),
        example_name
    );
    let source =
        std::fs::read_to_string(&path).map_err(|e| format!("failed to read {path}: {e}"))?;
    let transpiled = dathon::transpile(&source);
    ruff_python_parser::parse_module(&transpiled)
        .map(|_| ())
        .map_err(|e| format!("transpiled output did not parse: {e:?}"))
}

#[test]
fn schemas_dpy_transpiles_to_parseable_python() {
    // The big example file. The transpiled output must be valid Python
    // source — which it should be, since dathon syntax IS valid Python.
    transpile_and_reparse("schemas.dpy").expect("schemas.dpy");
}

#[test]
fn nested_dpy_transpiles_to_parseable_python() {
    transpile_and_reparse("nested.dpy").expect("nested.dpy");
}

#[test]
fn generic_read_dpy_transpiles_to_parseable_python() {
    transpile_and_reparse("generic_read.dpy").expect("generic_read.dpy");
}

#[test]
fn orders_example_dpy_transpiles_to_parseable_python() {
    transpile_and_reparse("orders_example.dpy").expect("orders_example.dpy");
}

#[test]
fn events_example_dpy_transpiles_to_parseable_python() {
    transpile_and_reparse("events_example.dpy").expect("events_example.dpy");
}

// ===========================================================================
// Schema-cast stripping
// ===========================================================================

#[test]
fn fluent_schema_cast_is_stripped_from_the_chain() {
    // `<chain>.cast(DataFrame[Schema])` is a dathon-only re-anchoring hint
    // — `DataFrame` has no `.cast` method — so the transpiler removes the
    // `.cast(…)` segment and leaves the receiver wired straight through.
    let src = "\
class Raw(Schema):
    city: string

class Pivoted(Schema):
    city: string

def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.cast(DataFrame[Pivoted]).select(col(\"city\"))
";
    let output = dathon::transpile(src);
    assert!(
        !output.contains(".cast("),
        "schema-cast should be stripped, got:\n{output}",
    );
    assert!(output.contains("raw.select(col(\"city\"))"));
    ruff_python_parser::parse_module(&output).expect("transpiled output must parse");
}

#[test]
fn column_cast_survives_the_transpile() {
    // The ordinary `Column.cast("int")` is real PySpark and must not be
    // touched — only the `DataFrame[…]`-argument form is dathon-only.
    let src = "def f(raw):\n    return raw.select(col(\"amount\").cast(\"int\"))\n";
    let output = dathon::transpile(src);
    assert!(output.contains("cast(\"int\")"));
    ruff_python_parser::parse_module(&output).expect("transpiled output must parse");
}
