//! Inline structural schemas — `DataFrame[{checkin: date, n: int}]`.
//! An anonymous schema written as a dict literal right in the
//! annotation, no `class` declaration needed.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

#[test]
fn inline_schema_param_resolves_its_columns() {
    let src = "\
def f(d: DataFrame[{a: int, b: string}]) -> DataFrame:
    return d.select(col(\"a\"), col(\"b\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn inline_schema_param_rejects_an_unknown_column() {
    let src = "\
def f(d: DataFrame[{a: int}]) -> DataFrame:
    return d.select(col(\"nonexistent\"))
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn inline_schema_in_return_position_is_checked() {
    let src = "\
def f(d: DataFrame[{a: int, b: int}]) -> DataFrame[{a: int, b: int}]:
    return d.select(col(\"a\"), col(\"b\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn inline_schema_return_mismatch_is_flagged() {
    let src = "\
def f(d: DataFrame[{a: int, b: int}]) -> DataFrame[{a: int, b: int}]:
    return d.select(col(\"a\"))
";
    assert_has_code(&check(src), "D0050");
}

#[test]
fn inline_schema_with_an_unknown_type_is_flagged() {
    let src = "\
def f(d: DataFrame[{n: weirdtype}]) -> DataFrame:
    return d
";
    assert_has_code(&check(src), "D0010");
}

#[test]
fn inline_schema_supports_collection_types() {
    let src = "\
def f(d: DataFrame[{tags: Array[string], n: int}]) -> DataFrame:
    return d.select(col(\"tags\"), col(\"n\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn inline_schema_supports_a_nested_schema_value() {
    // A declared `Schema` class as an inline-schema column type — the
    // dotted path navigates into it.
    let src = "\
class Event(Schema):
    id: int

def f(d: DataFrame[{event: Event, n: int}]) -> DataFrame:
    return d.select(col(\"event.id\"), col(\"n\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn inline_schema_accepts_string_literal_keys() {
    // `{"a": int}` is accepted alongside the bare-name form `{a: int}`.
    let src = "\
def f(d: DataFrame[{\"a\": int, \"b\": string}]) -> DataFrame:
    return d.select(col(\"a\"), col(\"b\"))
";
    assert_no_diagnostics(&check(src));
}
