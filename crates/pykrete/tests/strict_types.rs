//! Strict-mode operator type checks. pykrete flags type combinations
//! Spark *coerces* rather than rejects — legal but usually a mistake.
//! Because the coercion is legal, these are advisory: they surface only
//! under `typeCheckingMode: "strict"`, never in the default mode.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check, check_strict};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
    amount2: int
    when_date: date
";

#[test]
fn string_in_arithmetic_is_flagged_under_strict() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumn(\"x\", col(\"city\") + col(\"amount\"))
"
    );
    assert_has_code(&check_strict(&src), "D0081");
}

#[test]
fn string_in_arithmetic_is_silent_in_default_mode() {
    // The advisory check must not fire outside strict mode.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumn(\"x\", col(\"city\") + col(\"amount\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0081");
}

#[test]
fn numeric_arithmetic_is_not_flagged() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumn(\"x\", col(\"amount\") + col(\"amount2\"))
"
    );
    assert_does_not_have_code(&check_strict(&src), "D0081");
}

#[test]
fn cross_type_comparison_is_flagged_under_strict() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.filter(col(\"amount\") == col(\"city\"))
"
    );
    assert_has_code(&check_strict(&src), "D0082");
}

#[test]
fn cross_type_comparison_is_silent_in_default_mode() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.filter(col(\"amount\") == col(\"city\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0082");
}

#[test]
fn date_versus_string_comparison_is_idiomatic_and_accepted() {
    // `col("date") > "2024-01-01"` is idiomatic Spark — the string is
    // cast to a date. Strict mode must not flag it.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.filter(col(\"when_date\") > \"2024-01-01\")
"
    );
    assert_does_not_have_code(&check_strict(&src), "D0082");
}

#[test]
fn same_family_comparison_is_not_flagged() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.filter(col(\"amount\") > col(\"amount2\"))
"
    );
    assert_does_not_have_code(&check_strict(&src), "D0082");
}

#[test]
fn nested_string_arithmetic_is_reached() {
    // The bad BinOp is nested inside an `F.lit`-style call argument.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumn(\"x\", F.abs(col(\"city\") * col(\"amount\")))
"
    );
    assert_has_code(&check_strict(&src), "D0081");
}
