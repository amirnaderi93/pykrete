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

// ------------------------------------------------------------------
// v1.3 PR-E4: D0020 / D0021 diagnostic messages must echo the slot's
// actual dialect prefix rather than the hardcoded "DataFrame" surface.
// Mirrors the PR-E2 fix shape applied to D0051.

#[test]
fn V13E4_d0020_message_uses_param_actual_dialect() {
    // A `PandasFrame[Mystery]` slot with no `Mystery` schema declared
    // must say "referenced in PandasFrame[Mystery]" — not "DataFrame[…]".
    let result = check(
        r#"
def f(df: PandasFrame[Mystery]) -> PandasFrame[Mystery]:
    return df
"#,
    );
    assert_has_code(&result, "D0020");
    assert_message_contains(&result, "D0020", "PandasFrame[Mystery]");
    let has_dataframe_surface = result
        .diagnostics_with_code("D0020")
        .iter()
        .any(|d| d.message.contains("DataFrame"));
    assert!(
        !has_dataframe_surface,
        "D0020 on a PandasFrame slot must not mention DataFrame; got:\n  {}",
        result
            .diagnostics_with_code("D0020")
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n  "),
    );
}

#[test]
fn V13E4_d0021_message_uses_param_actual_dialect() {
    // A `PandasFrame[<complex>]` slot — message must lead with
    // "PandasFrame schema must be a bare name", not "DataFrame".
    let result = check(
        r#"
def f(df: PandasFrame[list[str]]) -> PandasFrame:
    return df
"#,
    );
    assert_has_code(&result, "D0021");
    assert_message_contains(&result, "D0021", "PandasFrame schema must be a bare name");
    let has_dataframe_surface = result
        .diagnostics_with_code("D0021")
        .iter()
        .any(|d| d.message.starts_with("DataFrame schema"));
    assert!(
        !has_dataframe_surface,
        "D0021 on a PandasFrame slot must not lead with 'DataFrame schema'; got:\n  {}",
        result
            .diagnostics_with_code("D0021")
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n  "),
    );
}

#[test]
fn V13E4_d0020_ann_assign_pandas_frame_uses_pandas_prefix() {
    // Round-2 PR-E4 fix: positive coverage for driver.rs's ann-assign
    // D0020 emit site (the `x: PandasFrame[Mystery] = …` shape). The
    // dispatched dialect must surface as `PandasFrame`, not `DataFrame`.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: PandasFrame[Orders]) -> PandasFrame:
    x: PandasFrame[Mystery] = raw
    return x
"#,
    );
    assert_has_code(&result, "D0020");
    assert_message_contains(&result, "D0020", "PandasFrame[Mystery]");
    let has_dataframe_surface = result
        .diagnostics_with_code("D0020")
        .iter()
        .any(|d| d.message.contains("DataFrame"));
    assert!(
        !has_dataframe_surface,
        "D0020 on a PandasFrame ann-assign must not mention DataFrame; got:\n  {}",
        result
            .diagnostics_with_code("D0020")
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n  "),
    );
}

#[test]
fn V13E4_d0021_ann_assign_pandas_frame_uses_pandas_prefix() {
    // Round-2 PR-E4 fix: positive coverage for driver.rs's ann-assign
    // D0021 emit site (the `x: PandasFrame[list[str]] = …` shape).
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: PandasFrame[Orders]) -> PandasFrame:
    x: PandasFrame[list[str]] = raw
    return x
"#,
    );
    assert_has_code(&result, "D0021");
    assert_message_contains(&result, "D0021", "PandasFrame schema must be a bare name");
    let has_dataframe_surface = result
        .diagnostics_with_code("D0021")
        .iter()
        .any(|d| d.message.starts_with("DataFrame schema"));
    assert!(
        !has_dataframe_surface,
        "D0021 on a PandasFrame ann-assign must not lead with 'DataFrame schema'; got:\n  {}",
        result
            .diagnostics_with_code("D0021")
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n  "),
    );
}

#[test]
fn V13E4_d0020_d0021_deprecated_alias_preserved() {
    // Per spec §6 Q7, the deprecated `DataFrame[X]` alias renders as
    // the user typed it. Both D0020 and D0021 must keep the `DataFrame`
    // surface when the slot was declared as the alias.
    let d0020 = check(
        r#"
def f(df: DataFrame[Mystery]) -> DataFrame[Mystery]:
    return df
"#,
    );
    assert_has_code(&d0020, "D0020");
    assert_message_contains(&d0020, "D0020", "DataFrame[Mystery]");

    let d0021 = check(
        r#"
def f(df: DataFrame[list[str]]) -> DataFrame:
    return df
"#,
    );
    assert_has_code(&d0021, "D0021");
    assert_message_contains(&d0021, "D0021", "DataFrame schema must be a bare name");
}
