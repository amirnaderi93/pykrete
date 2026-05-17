//! `df.transform(fn)` — Spark's chaining sugar, equivalent to `fn(df)`.
//! dathon models the result schema from `fn`'s declared return and
//! checks the receiver against `fn`'s declared parameter.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: \"string\"
    amount: \"int\"

class Enriched(Schema):
    city: \"string\"
    amount: \"int\"
    bonus: \"int\"

def enrich(df: DataFrame[Raw]) -> DataFrame[Enriched]:
    return df.withColumn(\"bonus\", col(\"amount\"))

def add_bonus(df):
    return df.withColumn(\"bonus\", col(\"amount\"))
";

#[test]
fn transform_result_schema_is_the_functions_declared_return() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.transform(enrich).select(col(\"bonus\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn bad_column_after_transform_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.transform(enrich).select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn transform_input_schema_mismatch_is_caught() {
    // `enrich` expects DataFrame[Raw]; feeding it a DataFrame[Enriched]
    // (which has an extra `bonus` column) is the wrong-step mistake.
    let src = format!(
        "{SCHEMA}
def f(e: DataFrame[Enriched]) -> DataFrame:
    return e.transform(enrich)
"
    );
    assert_has_code(&check(&src), "D0070");
}

#[test]
fn transform_input_schema_match_is_accepted() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.transform(enrich)
"
    );
    assert_does_not_have_code(&check(&src), "D0070");
}

#[test]
fn undeclared_return_is_inferred_from_the_function_body() {
    // `add_bonus` has no return annotation — its output schema is
    // inferred by walking the body (`withColumn` adds `bonus`).
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.transform(add_bonus).select(col(\"bonus\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn bad_column_after_inferred_transform_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.transform(add_bonus).select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn transform_return_type_flows_into_the_outer_return_check() {
    // `f` declares `-> DataFrame[Raw]` but returns `transform(enrich)`,
    // which is DataFrame[Enriched] — a return mismatch.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame[Raw]:
    return raw.transform(enrich)
"
    );
    assert_has_code(&check(&src), "D0050");
}
