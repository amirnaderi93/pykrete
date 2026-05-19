//! Result-schema inference, local variable bindings, chained-call receivers,
//! `union` / `unionByName` checks, and return-type validation.

#![allow(non_snake_case)] // PySpark method names (unionByName) leak into test names.

//!
//! Exercises diagnostics:
//! - `D0030` — used here in cases where the receiver's schema is *inferred*
//!   rather than directly declared.
//! - `D0040` — `unionByName` / `union` between two DataFrames with
//!   different field-name sets.
//! - `D0050` — function body's return value's schema doesn't match the
//!   declared return type.

mod common;
use common::*;

// ===========================================================================
// Chained-call receivers
// ===========================================================================

#[test]
fn chained_select_after_filter_is_clean_when_columns_are_valid() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.filter(col("price") > 0).select(col("place_code"), col("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn chained_select_after_filter_catches_typo_in_outer_call() {
    // The outer `.select(col("madeup"))` runs against the filter's output
    // schema (which is Orders since filter is schema-preserving).
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.filter(col("price") > 0).select(col("madeup"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "madeup");
}

#[test]
fn chained_filter_after_select_runs_against_the_selects_derived_schema() {
    // `raw.select(col("place_code"))` produces an inferred schema with just
    // `place_code`. The outer `.filter(col("price") > 0)` should fail
    // because `price` is no longer in scope.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code")).filter(col("price") > 0)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "price");
}

// ===========================================================================
// Local variable bindings
// ===========================================================================

#[test]
fn local_bound_to_filter_preserves_the_input_schema() {
    // filter is schema-preserving, so the local has the same fields as
    // the parameter. The subsequent select on a known column is fine.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    filtered = raw.filter(col("price") > 0)
    return filtered.select(col("place_code"), col("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn local_bound_to_select_carries_the_narrowed_schema() {
    // `partial` has only `place_code` after the first select. Asking for
    // `price` on it should fail — that field was projected away.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    partial = raw.select(col("place_code"))
    return partial.select(col("price"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "inferred schema [place_code]");
}

#[test]
fn local_bound_to_withColumnRenamed_carries_the_renamed_field() {
    // After the rename, `code` exists; `place_code` doesn't.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    renamed = raw.withColumnRenamed("place_code", "code")
    return renamed.select(col("code"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn after_withColumnRenamed_the_old_name_is_gone() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    renamed = raw.withColumnRenamed("place_code", "code")
    return renamed.select(col("place_code"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "place_code");
}

#[test]
fn after_withColumn_the_new_field_is_part_of_the_schema() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    extended = raw.withColumn("doubled", col("price") * 2)
    return extended.select(col("doubled"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn after_drop_the_dropped_column_is_no_longer_in_scope() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    trimmed = raw.drop("price")
    return trimmed.select(col("price"))
"#,
    );
    assert_has_code(&result, "D0030");
}

// ===========================================================================
// D0040 — union / unionByName schema-set matching
// ===========================================================================

#[test]
fn unionByName_with_identical_schemas_emits_no_diagnostic() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(a: DataFrame[Orders], b: DataFrame[Orders]) -> DataFrame[Orders]:
    return a.unionByName(b)
"#,
    );
    assert_does_not_have_code(&result, "D0040");
}

#[test]
fn d0040_fires_when_unionByName_schemas_differ() {
    let result = check(
        r#"
class A(Schema):
    x: int
    y: int

class B(Schema):
    x: int
    z: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame[A]:
    return a.unionByName(b)
"#,
    );
    assert_has_code(&result, "D0040");
    // The diagnostic message should mention both sides' diff.
    assert_message_contains(&result, "D0040", "y");
    assert_message_contains(&result, "D0040", "z");
}

#[test]
fn d0040_fires_for_plain_union_just_like_unionByName() {
    // For v0.1 the two are treated the same — name-set equality.
    let result = check(
        r#"
class A(Schema):
    x: int

class B(Schema):
    y: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame[A]:
    return a.union(b)
"#,
    );
    assert_has_code(&result, "D0040");
}

// ===========================================================================
// D0050 — return-type validation
// ===========================================================================

#[test]
fn return_with_matching_schema_is_clean() {
    // raw is Orders, we return all of it — declared return is Orders.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code"), col("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn d0050_fires_when_body_returns_fewer_columns_than_declared() {
    // We return only `place_code` but the declared return Orders has both.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code"))
"#,
    );
    assert_has_code(&result, "D0050");
    assert_message_contains(&result, "D0050", "price");
}

#[test]
fn d0050_fires_when_body_introduces_an_extra_column() {
    // withColumn adds `bonus`, which Orders doesn't have.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.withColumn("bonus", col("price") * 2)
"#,
    );
    assert_has_code(&result, "D0050");
    assert_message_contains(&result, "D0050", "bonus");
}

#[test]
fn d0050_does_not_fire_for_functions_without_a_typed_return_annotation() {
    // No declared return → nothing to validate against. The function is
    // still typed (DataFrame[X] param) but return is not checked.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.select(col("place_code"))
"#,
    );
    assert_does_not_have_code(&result, "D0050");
}
