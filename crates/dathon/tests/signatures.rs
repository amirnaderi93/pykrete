//! Function signature recognition — the `DataFrame[X]` shape.

#![allow(non_snake_case)] // Type names (DataFrame, Schema) appear in test names.

//!
//! Exercises diagnostics:
//! - `D0020` — `DataFrame[X]` references a schema we don't know about.
//! - `D0021` — the inner `X` in `DataFrame[X]` is not a bare name.

mod common;
use common::*;

#[test]
fn function_typed_with_known_schema_is_counted_as_a_typed_function() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    pass
"#,
    );
    assert_eq!(result.typed_function_count, 1);
    assert_does_not_have_code(&result, "D0020");
    assert_does_not_have_code(&result, "D0021");
}

#[test]
fn function_with_no_dataframe_annotations_is_not_in_typed_function_count() {
    // No DataFrame parameter or return — dathon has nothing to check on it,
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
def f(raw: DataFrame) -> DataFrame:
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
def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
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
def f(raw: DataFrame[Nope]) -> DataFrame[AlsoNope]:
    pass
"#,
    );
    assert_count(&result, "D0020", 2);
}

#[test]
fn d0021_fires_when_DataFrame_argument_is_subscripted() {
    // `DataFrame[list[str]]` is a subscript whose inner expression is also
    // a subscript — not a bare schema name.
    let result = check(
        r#"
def f(y: DataFrame[list[str]]) -> DataFrame:
    pass
"#,
    );
    assert_has_code(&result, "D0021");
}

#[test]
fn d0021_fires_on_return_annotation_just_like_parameters() {
    // Return-type `DataFrame[X]` is checked the same way as parameters.
    let result = check(
        r#"
def f(x: DataFrame) -> DataFrame[list[str]]:
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

def join_them(left: DataFrame[A], right: DataFrame[B]) -> DataFrame[A]:
    pass
"#,
    );
    assert_eq!(result.typed_function_count, 1);
    assert_does_not_have_code(&result, "D0020");
}
