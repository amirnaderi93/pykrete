//! Function signature recognition — the `SparkFrame[X]` shape.

#![allow(non_snake_case)] // Type names (DataFrame, Schema) appear in test names.

//!
//! Exercises diagnostics:
//! - `D0020` — `SparkFrame[X]` references a schema we don't know about.
//! - `D0021` — the inner `X` in `SparkFrame[X]` is not a bare name.

mod common;
use common::*;

#[test]
fn function_typed_with_known_schema_is_counted_as_a_typed_function() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    pass
"#,
    );
    assert_eq!(result.typed_function_count, 1);
    assert_does_not_have_code(&result, "D0020");
    assert_does_not_have_code(&result, "D0021");
}

#[test]
fn function_with_no_dataframe_annotations_is_not_in_typed_function_count() {
    // No DataFrame parameter or return — pykrete has nothing to check on it,
    // and it's not rendered in the body output.
    let result = check(
        r#"
def helper(z: int) -> int:
    return z
"#,
    );
    assert_eq!(result.typed_function_count, 0);
}

#[test]
fn function_with_bare_DataFrame_is_typed_but_untyped_schema_emits_no_diagnostic() {
    // Bare `DataFrame` (no `[Schema]`) is recognized but doesn't have an
    // attached schema. No diagnostic — it's informational. The function
    // still counts as typed because it touches DataFrame somewhere.
    let result = check(
        r#"
def f(raw: DataFrame) -> SparkFrame:
    pass
"#,
    );
    assert_eq!(result.typed_function_count, 1);
    assert_does_not_have_code(&result, "D0020");
    assert_does_not_have_code(&result, "D0021");
}

#[test]
fn d0020_fires_when_schema_inside_DataFrame_brackets_is_unknown() {
    // Orders is not declared anywhere in the file.
    let result = check(
        r#"
def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    pass
"#,
    );
    assert_has_code(&result, "D0020");
    assert_message_contains(&result, "D0020", "Orders");
}

#[test]
fn d0020_fires_once_per_unknown_schema_reference() {
    // Two slots referencing the same unknown schema → two D0020s.
    let result = check(
        r#"
def f(raw: SparkFrame[Nope]) -> SparkFrame[AlsoNope]:
    pass
"#,
    );
    assert_count(&result, "D0020", 2);
}

#[test]
fn d0021_fires_when_DataFrame_argument_is_subscripted() {
    // `SparkFrame[list[str]]` is a subscript whose inner expression is also
    // a subscript — not a bare schema name.
    let result = check(
        r#"
def f(y: SparkFrame[list[str]]) -> SparkFrame:
    pass
"#,
    );
    assert_has_code(&result, "D0021");
}

#[test]
fn d0021_fires_on_return_annotation_just_like_parameters() {
    // Return-type `SparkFrame[X]` is checked the same way as parameters.
    let result = check(
        r#"
def f(x: DataFrame) -> SparkFrame[list[str]]:
    pass
"#,
    );
    assert_has_code(&result, "D0021");
}

#[test]
fn multiple_typed_parameters_are_all_recognized() {
    let result = check(
        r#"
class A(Schema):
    x: int

class B(Schema):
    y: int

def join_them(left: SparkFrame[A], right: SparkFrame[B]) -> SparkFrame[A]:
    pass
"#,
    );
    assert_eq!(result.typed_function_count, 1);
    assert_does_not_have_code(&result, "D0020");
}

// ------------------------------------------------------------------
// v1.3 PR-E2: CLI render_function must carry the slot's dialect prefix.
// Hover/symbols routed through `dataframe::render_annotation` long ago
// (v1.3 PR-B); these pin the CLI text output to the same surface so a
// `PandasFrame[X]` signature doesn't render as "DataFrame[X]" in the
// text body.

#[test]
fn V13E2_cli_render_pandasframe_keeps_dialect_prefix() {
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: PandasFrame[Orders]) -> PandasFrame[Orders]:
    return df
"#,
    );
    assert!(
        result.body.contains("PandasFrame[Orders]"),
        "CLI body should render PandasFrame[Orders] verbatim; got:\n{}",
        result.body,
    );
    assert!(
        !result.body.contains("DataFrame[Orders]"),
        "CLI body must not collapse PandasFrame to DataFrame; got:\n{}",
        result.body,
    );
}

#[test]
fn V13E2_cli_render_sparkframe_keeps_dialect_prefix() {
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return df
"#,
    );
    assert!(
        result.body.contains("SparkFrame[Orders]"),
        "CLI body should render SparkFrame[Orders] verbatim; got:\n{}",
        result.body,
    );
    // No D0090 — the user wrote SparkFrame, not the deprecated alias.
    assert!(
        !result.body.contains("DataFrame[Orders]"),
        "CLI body must not render SparkFrame as DataFrame; got:\n{}",
        result.body,
    );
}

#[test]
fn V13E2_cli_render_deprecated_alias_preserved() {
    // Per v1.3 §6 Q7, the deprecated `DataFrame[X]` alias renders as the
    // user wrote it — `DataFrame[X]`, not its canonical `SparkFrame[X]`.
    // Pins the alias-preserved surface for the CLI text body.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: DataFrame[Orders]) -> DataFrame[Orders]:
    return df
"#,
    );
    assert!(
        result.body.contains("DataFrame[Orders]"),
        "CLI body should preserve the user-typed DataFrame alias; got:\n{}",
        result.body,
    );
    // D0090 still fires on the slot (alias deprecation) but the rendered
    // surface stays as the user wrote it.
    assert_has_code(&result, "D0090");
}

#[test]
fn V13E2_cli_render_untyped_pandasframe_uses_pandas_prefix() {
    // Architecture-audit nit #11: bare `PandasFrame` (no `[X]`) must
    // render as "PandasFrame  (untyped)", not the prior hardcoded
    // "DataFrame  (untyped)".
    let result = check(
        r#"
def f(df: PandasFrame) -> PandasFrame:
    return df
"#,
    );
    assert!(
        result.body.contains("PandasFrame  (untyped)"),
        "CLI body should render bare PandasFrame as 'PandasFrame  (untyped)'; got:\n{}",
        result.body,
    );
}
