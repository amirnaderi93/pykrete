//! Array / map column types. dathon tracks a column as a collection
//! (vs. an atomic) — the element / key-value types inside `<…>` aren't
//! modeled in v1, but `array`-vs-`int` mistakes are caught.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check, check_strict};

const IN: &str = "\
class In(Schema):
    name: string
    score: int
";

#[test]
fn collect_list_result_is_an_array() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    scores: Array[int]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.groupBy(\"name\").agg(F.collect_list(\"score\").alias(\"scores\"))
"
    );
    // `collect_list` produces an array, matching the declared column.
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn array_column_declared_but_atomic_produced_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    scores: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.groupBy(\"name\").agg(F.collect_list(\"score\").alias(\"scores\"))
"
    );
    // The body produces an array; Out declares `scores` a string.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn atomic_produced_where_array_declared_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    score: int
    tag: Array

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"tag\", F.lower(col(\"name\")))
"
    );
    // `tag` is declared an array but `F.lower(...)` is a string.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn arithmetic_on_an_array_is_flagged_under_strict() {
    let src = format!(
        "{IN}
def f(d: DataFrame[In]) -> DataFrame:
    return d.withColumn(\"x\", F.array(col(\"score\")) + F.lit(1))
"
    );
    assert_has_code(&check_strict(&src), "D0081");
}

#[test]
fn comparing_an_array_to_an_atomic_is_flagged_under_strict() {
    let src = format!(
        "{IN}
def f(d: DataFrame[In]) -> DataFrame:
    return d.filter(F.array(col(\"score\")) == col(\"score\"))
"
    );
    assert_has_code(&check_strict(&src), "D0082");
}
