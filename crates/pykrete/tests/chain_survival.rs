//! Operations that reshape rows but not columns must keep a
//! transformation chain alive — otherwise a bad column reference
//! *after* one of them passes silently (the "cascade degradation"
//! found when checking real-world pipelines).

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
";

#[test]
fn fillna_keeps_the_chain_alive() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.fillna(0).select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn replace_keeps_the_chain_alive() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.replace(0, 1).select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn fillna_does_not_flag_its_own_arguments() {
    // `0` is a fill value, `["amount"]` a subset — not column-name
    // positions to be checked, and the schema is unchanged.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.fillna(0).select(col(\"amount\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn to_df_renames_every_column() {
    // `toDF("x", "y")` discards the old names; `city` no longer exists.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.toDF(\"x\", \"y\").select(col(\"city\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn to_df_renamed_column_resolves() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.toDF(\"x\", \"y\").select(col(\"x\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn to_df_with_no_args_keeps_the_receiver_schema() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.toDF().select(col(\"city\"))
"
    );
    assert_no_diagnostics(&check(&src));
}
