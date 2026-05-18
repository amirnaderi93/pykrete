//! Two narrower analyzer gaps: `explode`'s default output column name,
//! and `subset=` keyword-argument column checking.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
    tags: string
";

#[test]
fn explode_without_alias_produces_a_col_column() {
    // `F.explode("tags")` with no `.alias(...)` — Spark names the
    // unnested column `col`, which downstream code can then reference.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(F.explode(\"tags\")).select(col(\"col\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn explode_with_alias_still_uses_the_alias() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(F.explode(\"tags\").alias(\"tag\")).select(col(\"tag\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn dropna_subset_bad_column_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.dropna(subset=[\"city\", \"nonexistent\"])
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn fillna_subset_good_columns_pass() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.fillna(0, subset=[\"amount\"])
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn drop_duplicates_subset_bad_column_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.dropDuplicates(subset=[\"madeup\"])
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn na_drop_subset_bad_column_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.na.drop(subset=[\"nonexistent\"])
"
    );
    assert_has_code(&check(&src), "D0030");
}
