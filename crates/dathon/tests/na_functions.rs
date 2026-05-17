//! `df.na.fill / df.na.drop / df.na.replace` — the DataFrameNaFunctions
//! methods. Each reshapes rows only, so the schema is preserved and the
//! chain stays alive for downstream checks.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: \"string\"
    amount: \"int\"
";

#[test]
fn na_fill_keeps_the_chain_alive() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
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
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.na.drop(\"all\").select(col(\"amount\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn na_drop_keeps_the_chain_alive() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.na.drop().select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn na_replace_preserves_the_schema() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.na.replace(0, 1).select(col(\"city\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn plain_drop_still_checks_column_names() {
    // The `.na` interception must not weaken the ordinary `df.drop`
    // column check.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.drop(\"nonexistent\")
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn return_type_survives_an_na_call() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame[Raw]:
    return raw.na.fill(0)
"
    );
    assert_does_not_have_code(&check(&src), "D0050");
}
