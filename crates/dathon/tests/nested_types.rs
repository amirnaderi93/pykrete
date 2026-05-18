//! Recursive collection types — `array<T>` / `map<K, V>` carry their
//! element types, nested arbitrarily. A column declared `array<int>`
//! flows through the pipeline keeping that type, and a mismatch against
//! a declared schema is caught.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check};

const IN: &str = "\
class In(Schema):
    tags: Array[int]
    grid: Array[Array[int]]
    meta: Map[string, int]
";

#[test]
fn array_element_type_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    tags: Array[string]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"tags\"))
"
    );
    // `tags` is `array<int>`; Out declares `array<string>`.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn matching_array_element_type_passes() {
    let src = format!(
        "{IN}
class Out(Schema):
    tags: Array[int]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"tags\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn numeric_widening_applies_inside_collections() {
    // `array<int>` vs `array<long>` — element-wise numeric widening,
    // accepted by the conservative check.
    let src = format!(
        "{IN}
class Out(Schema):
    tags: Array[long]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"tags\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn deeply_nested_array_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    grid: Array[Array[string]]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"grid\"))
"
    );
    // `grid` is `array<array<int>>`; Out nests `string` at the leaf.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn map_value_type_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    meta: Map[string, string]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"meta\"))
"
    );
    // `meta` is `map<string, int>`; Out declares the value `string`.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn unknown_element_type_is_permissive() {
    // A bare `array` declaration has an unknown element type — it must
    // not clash with a concrete `array<int>`.
    let src = format!(
        "{IN}
class Out(Schema):
    tags: Array

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"tags\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}
