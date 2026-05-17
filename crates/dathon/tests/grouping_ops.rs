//! Grouping/projection operations: `select("*")`, `cube`, `rollup`,
//! and `pivot`.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: \"string\"
    amount: \"int\"
    month: \"string\"
";

#[test]
fn select_star_is_not_flagged_as_a_missing_column() {
    // Regression: `"*"` is the all-columns wildcard, not a literal
    // column name — it must not trip a D0030.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(\"*\")
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn select_star_expands_to_the_receiver_columns() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(\"*\").select(col(\"amount\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn bad_column_after_select_star_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(\"*\").select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn cube_groups_like_group_by() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.cube(\"city\").agg(F.sum(\"amount\").alias(\"total\")).select(col(\"total\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn bad_cube_key_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.cube(\"nonexistent\").agg(F.sum(\"amount\").alias(\"total\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn rollup_groups_like_group_by() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.rollup(\"city\").agg(F.sum(\"amount\").alias(\"total\")).select(col(\"city\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn bad_pivot_column_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.groupBy(\"city\").pivot(\"nonexistent\").sum(\"amount\")
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn valid_pivot_column_is_accepted() {
    // The pivoted result schema is unknowable, but the pivot column
    // itself resolves cleanly — no D0030.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.groupBy(\"city\").pivot(\"month\").sum(\"amount\")
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}
