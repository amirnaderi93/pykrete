//! Body-level column reference checks across every supported operation.

#![allow(non_snake_case)] // PySpark method names (groupBy, withColumn, etc.) leak into test names.

//!
//! Exercises `D0030` — column doesn't exist on the receiver's schema —
//! across `select`, `filter` / `where`, `drop`, `dropDuplicates`,
//! `groupBy`, `withColumn`, and `withColumnRenamed`.
//!
//! All tests use a small Orders schema with two fields: `place_code: int`
//! and `price: int`.

mod common;
use common::*;

/// Wraps the snippet under test in a standard preamble that defines an
/// `Orders(Schema)` with two fields and a function that takes
/// `DataFrame[Orders]` and returns `DataFrame[Orders]`. The function body
/// is whatever the caller supplies.
fn with_orders(body: &str) -> String {
    format!(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
{body}
"#,
        body = indent(body, 4)
    )
}

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

// ===========================================================================
// select
// ===========================================================================

#[test]
fn select_with_known_columns_is_clean() {
    let result = check(&with_orders(r#"return raw.select(col("place_code"), col("price"))"#));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_when_select_references_an_unknown_column() {
    let result = check(&with_orders(r#"return raw.select(col("priec"))"#));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "priec");
}

#[test]
fn select_accepts_bare_string_column_names() {
    // PySpark allows `df.select("col_name")` as shorthand for col("col_name").
    let result = check(&with_orders(r#"return raw.select("place_code", "price")"#));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_on_bare_string_with_a_typo() {
    let result = check(&with_orders(r#"return raw.select("place_code", "no_such")"#));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "no_such");
}

#[test]
fn select_with_alias_does_not_flag_the_alias_target() {
    // The "pc" is the OUTPUT name, not a column we're reading from raw.
    let result = check(&with_orders(r#"return raw.select(col("place_code").alias("pc"))"#));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_on_nested_col_in_complex_select_argument() {
    // (col("a") + col("missing")).cast("int").alias("z")
    // The `missing` deep inside should still be caught.
    let result = check(&with_orders(
        r#"return raw.select((col("place_code") * col("missing")).cast("int").alias("z"))"#,
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "missing");
}

// ===========================================================================
// filter / where
// ===========================================================================

#[test]
fn filter_with_known_columns_is_clean() {
    let result = check(&with_orders(r#"return raw.filter(col("price") > 0)"#));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_when_filter_references_an_unknown_column() {
    let result = check(&with_orders(r#"return raw.filter(col("nope") > 0)"#));
    assert_has_code(&result, "D0030");
}

#[test]
fn filter_recognizes_columns_through_boolean_and_comparison_operators() {
    // The recursive col-ref walker should descend through &, |, comparisons
    // and method-chain calls to find the nested col("ghost") reference.
    let result = check(&with_orders(
        r#"return raw.filter((col("price") > 0) & (col("ghost").isNotNull()))"#,
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "ghost");
}

#[test]
fn filter_string_literals_are_values_not_columns() {
    // "confirmed" is a value being compared against, not a column reference.
    // It must NOT trigger D0030 even though "confirmed" isn't a column of
    // Orders.
    let result = check(&with_orders(r#"return raw.filter(col("place_code") == "confirmed")"#));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn where_behaves_identically_to_filter() {
    // `where` is just an alias for `filter` in PySpark.
    let result = check(&with_orders(r#"return raw.where(col("missing") > 0)"#));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "missing");
}

// ===========================================================================
// drop, dropDuplicates
// ===========================================================================

#[test]
fn drop_with_known_columns_is_clean() {
    let result = check(&with_orders(r#"return raw.drop("price")"#));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_when_drop_references_an_unknown_column() {
    let result = check(&with_orders(r#"return raw.drop("nope")"#));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn drop_accepts_col_references_in_addition_to_bare_strings() {
    let result = check(&with_orders(r#"return raw.drop(col("place_code"), "price")"#));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_on_typo_inside_dropDuplicates_list_argument() {
    // The list literal is unpacked; each string element is treated as a
    // column name.
    let result = check(&with_orders(
        r#"return raw.dropDuplicates(["place_code", "wrong"])"#,
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "wrong");
}

// ===========================================================================
// groupBy
// ===========================================================================

#[test]
fn groupBy_with_known_columns_is_clean() {
    // groupBy returns a GroupedData so the result schema isn't tracked,
    // but the args are still checked for column existence.
    let result = check(&with_orders(r#"raw.groupBy("place_code")"#));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_when_groupBy_references_an_unknown_column() {
    let result = check(&with_orders(r#"raw.groupBy("place_code", col("invented"))"#));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "invented");
}

// ===========================================================================
// withColumn
// ===========================================================================

#[test]
fn withColumn_does_not_flag_its_new_column_name() {
    // The first arg is the NEW column name being introduced — it shouldn't
    // be checked for existence.
    let result = check(&with_orders(
        r#"return raw.withColumn("brand_new_field", col("price"))"#,
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_on_typo_inside_withColumn_expression_argument() {
    let result = check(&with_orders(
        r#"return raw.withColumn("new", col("priec") * 2)"#,
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "priec");
}

// ===========================================================================
// withColumnRenamed
// ===========================================================================

#[test]
fn withColumnRenamed_with_existing_old_name_is_clean() {
    let result = check(&with_orders(
        r#"return raw.withColumnRenamed("place_code", "code")"#,
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_when_withColumnRenamed_old_name_does_not_exist() {
    let result = check(&with_orders(
        r#"return raw.withColumnRenamed("place_cdoe", "code")"#,
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "place_cdoe");
}

#[test]
fn withColumnRenamed_does_not_flag_its_new_name() {
    // Only the OLD name needs to exist; the NEW name is being introduced.
    // "totally_new_name" is intentionally not in Orders and should not flag.
    let result = check(&with_orders(
        r#"return raw.withColumnRenamed("place_code", "totally_new_name")"#,
    ));
    assert_does_not_have_code(&result, "D0030");
}
