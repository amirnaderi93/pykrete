//! Integration tests for the `.pyk` → `.py` transpiler.
//!
//! These exercise `pykrete::transpile` end-to-end on realistic-shaped
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
    let output = pykrete::transpile(input);
    assert!(output.starts_with(FUTURE_IMPORT));
}

#[test]
fn transpile_is_idempotent_when_run_on_already_transpiled_source() {
    let input = "class Foo:\n    pass\n";
    let once = pykrete::transpile(input);
    let twice = pykrete::transpile(&once);
    assert_eq!(once, twice);
}

#[test]
fn original_source_is_preserved_verbatim_inside_the_output() {
    // The transpiler must not reformat / rewrite / re-indent the user's
    // code. Whatever lines were there, they appear in the output
    // unchanged. This is what makes line-number-based debugging usable.
    let input =
        "class Orders(Schema):\n    place_code: int\n    price: int\n\n\ndef f():\n    return 1\n";
    let output = pykrete::transpile(input);
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
    let transpiled = pykrete::transpile(&source);
    ruff_python_parser::parse_module(&transpiled)
        .map(|_| ())
        .map_err(|e| format!("transpiled output did not parse: {e:?}"))
}

#[test]
fn schemas_pyk_transpiles_to_parseable_python() {
    // The big example file. The transpiled output must be valid Python
    // source — which it should be, since pykrete syntax IS valid Python.
    transpile_and_reparse("schemas.pyk").expect("schemas.pyk");
}

#[test]
fn nested_pyk_transpiles_to_parseable_python() {
    transpile_and_reparse("nested.pyk").expect("nested.pyk");
}

#[test]
fn generic_read_pyk_transpiles_to_parseable_python() {
    transpile_and_reparse("generic_read.pyk").expect("generic_read.pyk");
}

#[test]
fn orders_pyk_transpiles_to_parseable_python() {
    transpile_and_reparse("orders.pyk").expect("orders.pyk");
}

// ===========================================================================
// Schema-cast stripping
// ===========================================================================

#[test]
fn fluent_schema_cast_is_stripped_from_the_chain() {
    // `<chain>.cast(SparkFrame[Schema])` is a pykrete-only re-anchoring hint
    // — `DataFrame` has no `.cast` method — so the transpiler removes the
    // `.cast(…)` segment and leaves the receiver wired straight through.
    let src = "\
class Raw(Schema):
    city: string

class Pivoted(Schema):
    city: string

def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.cast(SparkFrame[Pivoted]).select(col(\"city\"))
";
    let output = pykrete::transpile(src);
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
    // touched — only the `SparkFrame[…]`-argument form is pykrete-only.
    let src = "def f(raw):\n    return raw.select(col(\"amount\").cast(\"int\"))\n";
    let output = pykrete::transpile(src);
    assert!(output.contains("cast(\"int\")"));
    ruff_python_parser::parse_module(&output).expect("transpiled output must parse");
}
