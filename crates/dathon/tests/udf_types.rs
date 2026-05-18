//! UDF return types. A function registered as a Spark UDF — via an
//! `@udf` / `@pandas_udf` decorator or the functional `udf(f, …)` form —
//! has a known return type, so a column produced by calling it is typed.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check, check_strict};

#[test]
fn decorator_udf_string_return_type_is_used() {
    let src = "\
@udf(\"int\")
def to_score(x):
    return 1

class In(Schema):
    name: string

class Out(Schema):
    name: string
    score: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"score\", to_score(col(\"name\")))
";
    // `to_score` returns int; the `score` column is declared `string`.
    assert_has_code(&check(src), "D0080");
}

#[test]
fn decorator_udf_type_object_return_type_is_used() {
    let src = "\
@udf(returnType=IntegerType())
def to_score(x):
    return 1

class In(Schema):
    name: string

class Out(Schema):
    name: string
    score: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"score\", to_score(col(\"name\")))
";
    // `to_score` returns int, matching the declared `score` column.
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn bare_udf_decorator_defaults_to_string() {
    let src = "\
@udf
def label(x):
    return \"x\"

class In(Schema):
    name: string

class Out(Schema):
    name: string
    tag: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"tag\", label(col(\"name\")))
";
    // A bare `@udf` defaults to a string return; `tag` is declared int.
    assert_has_code(&check(src), "D0080");
}

#[test]
fn functional_udf_form_return_type_is_used() {
    let src = "\
def compute(x):
    return 1

score_udf = udf(compute, \"int\")

class In(Schema):
    name: string

class Out(Schema):
    name: string
    score: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"score\", score_udf(col(\"name\")))
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn udf_result_type_flows_into_a_strict_comparison() {
    let src = "\
@udf(\"int\")
def to_score(x):
    return 1

class In(Schema):
    name: string

def f(d: DataFrame[In]) -> DataFrame:
    return d.filter(to_score(col(\"name\")) == col(\"name\"))
";
    // int UDF result compared to the string column `name`.
    assert_has_code(&check_strict(src), "D0082");
}
