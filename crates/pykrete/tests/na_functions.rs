//! `df.na.fill / df.na.drop / df.na.replace` — the DataFrameNaFunctions
//! methods. Each reshapes rows only, so the schema is preserved and the
//! chain stays alive for downstream checks.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
";

#[test]
fn na_fill_keeps_the_chain_alive() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.na.fill(0).select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn na_drop_does_not_flag_its_how_argument() {
    // `"all"` is the `how` argument of na.drop, not a column name —
    // it must not be mistaken for a `df.drop("col")` reference.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.na.drop(\"all\").select(col(\"amount\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn na_drop_keeps_the_chain_alive() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.na.drop().select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn na_replace_preserves_the_schema() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.na.replace(0, 1).select(col(\"city\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn plain_drop_tolerates_missing_names_per_spark() {
    // Spark's `df.drop(*cols)` silently ignores names not in the schema
    // (per its source / docs). Pykrete v0.1.39 matches that. The `.na`
    // interception path doesn't affect this — what matters is that
    // `raw.drop("nonexistent")` produces no D0030.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.drop(\"nonexistent\")
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn return_type_survives_an_na_call() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame[Raw]:
    return raw.na.fill(0)
"
    );
    assert_does_not_have_code(&check(&src), "D0050");
}
