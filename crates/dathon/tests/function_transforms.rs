//! Collection-function type transforms — `collect_list(T)` -> `array<T>`,
//! `explode(array<T>)` -> `T`, `map_keys(map<K,V>)` -> `array<K>`, … —
//! so the element type flows through a function call, not just the
//! collection kind.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check};

const IN: &str = "\
class In(Schema):
    name: string
    tags: Array[int]
    score: int
    meta: Map[string, int]
";

#[test]
fn collect_list_wraps_the_element_type() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    scores: Array[int]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.groupBy(\"name\").agg(F.collect_list(\"score\").alias(\"scores\"))
"
    );
    // `collect_list` of an int column is `array<int>`.
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn collect_list_element_type_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    scores: Array[string]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.groupBy(\"name\").agg(F.collect_list(\"score\").alias(\"scores\"))
"
    );
    // `array<int>` produced, `array<string>` declared.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn explode_unwraps_the_array_element() {
    let src = format!(
        "{IN}
class Out(Schema):
    t: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(F.explode(col(\"tags\")).alias(\"t\"))
"
    );
    // `explode` of `array<int>` yields an `int` column.
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn explode_element_type_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    t: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(F.explode(col(\"tags\")).alias(\"t\"))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn map_keys_yields_an_array_of_the_key_type() {
    let src = format!(
        "{IN}
class Out(Schema):
    ks: Array[string]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(F.map_keys(col(\"meta\")).alias(\"ks\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn map_keys_key_type_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    ks: Array[int]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(F.map_keys(col(\"meta\")).alias(\"ks\"))
"
    );
    // `meta` is `map<string, int>` — its keys are an `array<string>`.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn split_yields_an_array_of_strings() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    tags: Array[int]
    score: int
    meta: Map[string, int]
    parts: Array[string]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"parts\", F.split(col(\"name\"), \",\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}
