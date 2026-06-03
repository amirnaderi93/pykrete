//! `df.X` attribute access as a column reference — the third recognized
//! form alongside `col("X")` and bare strings.
//!
//! Rule: an attribute access `name.attr` is treated as a column reference
//! to `attr` when `name` is bound to a DataFrame in the current body
//! context (a function parameter typed `SparkFrame[Schema]`, or a local
//! variable bound via assignment or `: SparkFrame[Schema]` annotation).
//!
//! Things that look like `name.attr` but where `name` is NOT a DataFrame
//! (e.g. `F.add_months`, `datetime.now`) are deliberately NOT treated as
//! column references — context-bound names are the discriminator.

#![allow(non_snake_case)] // DataFrame appears in test names.

mod common;
use common::*;

// ===========================================================================
// Happy path
// ===========================================================================

#[test]
fn df_X_is_recognized_as_a_column_reference_in_select() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.select(raw.place_code, raw.price)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_when_df_X_references_an_unknown_column() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.select(raw.priec)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "priec");
}

#[test]
fn df_X_can_chain_alias_and_cast() {
    // raw.price.cast("int").alias("p_int") — the col-ref discovery walks
    // through the .cast and .alias layers to find raw.price, the underlying
    // column reference.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.select(raw.price.cast("int").alias("p_int"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn df_X_with_typo_under_cast_alias_is_still_caught() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.select(raw.misspelled.cast("int").alias("x"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "misspelled");
}

#[test]
fn df_X_works_inside_filter_expressions() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.filter((raw.price > 0) & (raw.place_code.isNotNull()))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn df_X_typo_inside_filter_is_caught() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.filter(raw.nonexistent > 0)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nonexistent");
}

#[test]
fn df_X_works_inside_withColumn_expression() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.withColumn("doubled", raw.price * 2)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Negative space: things that look like df.X but aren't
// ===========================================================================

#[test]
fn module_attribute_access_is_not_treated_as_a_column_ref() {
    // `F.add_months(...)` — F is not bound as a DataFrame in scope.
    // The walker should NOT collect "add_months" as a column reference.
    // If it did, the filter below would emit a spurious D0030 against
    // "add_months" not being in Orders.
    let result = check(
        r#"
class Orders(Schema):
    price: int
    log_date: timestamp

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.filter(raw.log_date > F.add_months(lit(NOW), -1))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn attribute_access_on_unrelated_name_is_not_collected() {
    // `config.threshold` — config is not in scope as a DataFrame.
    // The walker descends but doesn't collect anything. The filter then
    // sees no col refs at all from that side of the comparison.
    let result = check(
        r#"
class Orders(Schema):
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.filter(raw.price > config.threshold)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// df.X with multiple DataFrames in scope (joins)
// ===========================================================================

#[test]
fn df_X_works_with_multiple_typed_dataframes_in_scope() {
    // Both `left` and `right` are DataFrame params; references to either
    // are caught as col refs and checked against the receiver's schema.
    let result = check(
        r#"
class A(Schema):
    k: int
    a: int

class B(Schema):
    k: int
    b: int

def f(left: SparkFrame[A], right: SparkFrame[B]) -> SparkFrame:
    return left.join(right, on="k").select(left.a, right.b)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn df_X_typo_with_multiple_dataframes_is_caught_against_the_receivers_schema() {
    // The receiver is the join's combined schema [k, a, b]. left.zzz is a
    // ref to "zzz" — not in the combined schema. Fires D0030.
    let result = check(
        r#"
class A(Schema):
    k: int
    a: int

class B(Schema):
    k: int
    b: int

def f(left: SparkFrame[A], right: SparkFrame[B]) -> SparkFrame:
    return left.join(right, on="k").select(left.zzz)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "zzz");
}
