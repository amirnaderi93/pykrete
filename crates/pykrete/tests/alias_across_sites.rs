//! `df.alias("L"); col("L.region")` — the canonical SQL-style alias
//! pattern — must resolve at EVERY column-checking site, not just on a
//! join's on-clause. Round-1 of the v0.1.37 false-positive sweep wired
//! the alias resolver into `check_join_keys` only, so the most common
//! shape of the pattern (post-join `select` / `filter` / `withColumn`
//! / `groupBy` referencing the aliased side) still false-fired D0030.
//! These tests pin the round-2 lift of the alias-resolution helper
//! across every site that goes through `report_column_refs`.

#![allow(non_snake_case)] // Aliases (`L`, `R`) leak into test names.

mod common;
use common::*;

const SALE_SCHEMA: &str = r#"
class Sale(Schema):
    region: string
    amount: int
"#;

#[test]
fn alias_select_aliased_col_does_not_false_flag() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: DataFrame[Sale]) -> DataFrame:
    L = raw.alias("L")
    return L.select(col("L.region"), col("L.amount"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn alias_filter_aliased_col_does_not_false_flag() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: DataFrame[Sale]) -> DataFrame:
    L = raw.alias("L")
    return L.filter(col("L.amount") > 0)
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn alias_withColumn_aliased_col_does_not_false_flag() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: DataFrame[Sale]) -> DataFrame:
    L = raw.alias("L")
    return L.withColumn("doubled", col("L.amount") * 2)
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn alias_groupBy_aliased_col_does_not_false_flag() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: DataFrame[Sale]) -> DataFrame:
    L = raw.alias("L")
    return L.groupBy(col("L.region")).count()
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn alias_select_typo_in_suffix_still_fires_d0030() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: DataFrame[Sale]) -> DataFrame:
    L = raw.alias("L")
    return L.select(col("L.regoin"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn alias_select_unknown_alias_prefix_fires_d0030_on_prefix() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: DataFrame[Sale]) -> DataFrame:
    L = raw.alias("L")
    return L.select(col("BAD.region"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "BAD");
    assert_message_contains(&result, "D0030", "not in scope");
}
