//! `df["X"]` subscript access as a column reference — the sibling of
//! `df.X` (see `attribute_columns.rs`).
//!
//! Rule: a subscript `name["lit"]` is treated as a column reference to
//! `lit` when `name` is bound to a DataFrame in the current body context
//! and the slice is a string literal. Computed subscripts
//! (`df[some_expr()]`) and subscripts on a non-DataFrame name fall
//! through to the default walker.

#![allow(non_snake_case)] // DataFrame appears in test names.

mod common;
use common::*;

// ===========================================================================
// Happy path — same shapes as attribute_columns, swapped to subscript form
// ===========================================================================

#[test]
fn df_X_subscript_is_recognized_as_a_column_reference_in_select() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.select(raw["place_code"], raw["price"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_when_df_X_subscript_references_an_unknown_column() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.select(raw["priec"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "priec");
}

#[test]
fn df_X_subscript_can_chain_alias_and_cast() {
    // raw["price"].cast("int").alias("p_int") — the col-ref discovery
    // walks through the .cast and .alias layers to find raw["price"], the
    // underlying column reference.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.select(raw["price"].cast("int").alias("p_int"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn df_X_subscript_with_typo_under_cast_alias_is_still_caught() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.select(raw["misspelled"].cast("int").alias("x"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "misspelled");
}

#[test]
fn df_X_subscript_works_inside_filter_expressions() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.filter((raw["price"] > 0) & (raw["place_code"] > 100))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn df_X_subscript_typo_inside_filter_is_caught() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.filter(raw["nonexistent"] > 0)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nonexistent");
}

#[test]
fn df_X_subscript_works_inside_withColumn_expression() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.withColumn("doubled", raw["price"] * 2)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Subscript form in arg-position recognizers (column_name_arg)
// ===========================================================================

#[test]
fn df_X_subscript_is_recognized_as_a_groupBy_key() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.groupBy(raw["place_code"]).agg(F.sum("price").alias("total"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn df_X_subscript_drop_removes_the_named_column() {
    // raw.drop(raw["price"]) should return a schema without price. If the
    // subscript wasn't recognized as a column name, drop would be a no-op
    // and "price" would still be present (no false positive though).
    // The proper assertion: a downstream reference to the dropped column
    // fires D0030 against the post-drop schema.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.drop(raw["price"]).select(col("price"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "price");
}

// ===========================================================================
// Negative space — things that look like df["X"] but aren't column refs
// ===========================================================================

#[test]
fn subscript_on_unrelated_name_is_not_treated_as_a_column_ref() {
    // `config["threshold"]` — config is not bound as a DataFrame.
    // The walker descends but doesn't collect anything.
    let result = check(
        r#"
class Orders(Schema):
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.filter(raw["price"] > config["threshold"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn computed_subscript_on_df_falls_through() {
    // `raw[some_var]` — slice isn't a string literal, so it can't be
    // resolved to a column name at static check time. Walker descends
    // without collecting; no false positives.
    let result = check(
        r#"
class Orders(Schema):
    price: int

def f(raw: SparkFrame[Orders], colname) -> SparkFrame:
    return raw.select(raw[colname])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
