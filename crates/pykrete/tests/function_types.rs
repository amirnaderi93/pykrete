//! Result types of `pyspark.sql.functions` calls. pykrete knows the
//! return type of common functions (`count` -> long, `length` -> int,
//! `lower` -> string, …), so a column produced by one of them is typed
//! and the return-type check (D0080) can see through it.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check, check_strict};

const IN: &str = "\
class In(Schema):
    city: string
    amount: int
    when_date: date
";

#[test]
fn count_result_is_long() {
    // `count` produces a long; declaring the column `string` is a
    // mismatch.
    let src = format!(
        "{IN}
class Out(Schema):
    city: string
    n: string

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.groupBy(\"city\").agg(F.count(\"amount\").alias(\"n\"))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn sum_of_int_widens_to_long_and_accepts_an_int_declaration() {
    // `sum` of an int column is a long; `int` vs `long` is numeric
    // widening, which the conservative check accepts.
    let src = format!(
        "{IN}
class Out(Schema):
    city: string
    total: int

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.groupBy(\"city\").agg(F.sum(\"amount\").alias(\"total\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn length_result_is_int() {
    let src = format!(
        "{IN}
class Out(Schema):
    city: string
    amount: int
    when_date: date
    name_len: string

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.withColumn(\"name_len\", F.length(col(\"city\")))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn lower_result_is_string() {
    let src = format!(
        "{IN}
class Out(Schema):
    city: int
    amount: int
    when_date: date

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.withColumn(\"city\", F.lower(col(\"city\")))
"
    );
    // `city` is overwritten with `F.lower(...)` — a string — but Out
    // declares it `int`.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn year_result_is_int() {
    let src = format!(
        "{IN}
class Out(Schema):
    city: string
    amount: int
    when_date: date
    yr: string

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.withColumn(\"yr\", F.year(col(\"when_date\")))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn function_result_type_flows_into_strict_comparison() {
    // `F.length(col("city"))` is an int; comparing it to the string
    // column `city` is a cross-type comparison.
    let src = format!(
        "{IN}
def f(x: DataFrame[In]) -> DataFrame:
    return x.filter(F.length(col(\"city\")) == col(\"city\"))
"
    );
    assert_has_code(&check_strict(&src), "D0082");
}
