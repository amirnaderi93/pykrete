//! Tests for `.join(...)` and `.crossJoin(...)` — the two-DataFrame
//! schema-concatenation operations.

#![allow(non_snake_case)] // PySpark method names (crossJoin, unionByName) leak into test names.

//!
//! Exercises diagnostic `D0060` (join key missing on one side) and the
//! result-schema inference rules:
//!
//! - For `on="k"` or `on=["k1", "k2"]`, the keys appear once in the result.
//! - For complex on-expressions (anything else), pykrete doesn't analyze the
//!   on-clause but still concatenates left + right field names.
//! - For `crossJoin`, no on-clause; everything is concatenated.

mod common;
use common::*;

// Two schemas with one shared field name, used by most tests below.
const TWO_SCHEMAS: &str = r#"
class A(Schema):
    k: int
    a: int

class B(Schema):
    k: int
    b: int
"#;

// ===========================================================================
// Valid join cases
// ===========================================================================

#[test]
fn join_on_shared_key_is_clean() {
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, on="k")
"#
    ));
    assert_does_not_have_code(&result, "D0060");
}

#[test]
fn join_with_positional_on_argument_works_like_kwarg() {
    // `a.join(b, "k")` — second positional arg is `on`.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, "k")
"#
    ));
    assert_does_not_have_code(&result, "D0060");
}

#[test]
fn join_with_how_argument_does_not_affect_the_key_check() {
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, on="k", how="left")
"#
    ));
    assert_does_not_have_code(&result, "D0060");
}

#[test]
fn join_with_list_of_keys_is_clean_when_all_keys_exist_on_both_sides() {
    let result = check(
        r#"
class A(Schema):
    k1: int
    k2: int
    a: int

class B(Schema):
    k1: int
    k2: int
    b: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, on=["k1", "k2"])
"#,
    );
    assert_does_not_have_code(&result, "D0060");
}

// ===========================================================================
// D0060 — join key missing on a side
// ===========================================================================

#[test]
fn d0060_fires_when_join_key_is_missing_from_left_side() {
    let result = check(
        r#"
class A(Schema):
    a: int

class B(Schema):
    k: int
    b: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, on="k")
"#,
    );
    assert_has_code(&result, "D0060");
    assert_message_contains(&result, "D0060", "k");
    assert_message_contains(&result, "D0060", "left");
}

#[test]
fn d0060_fires_when_join_key_is_missing_from_right_side() {
    let result = check(
        r#"
class A(Schema):
    k: int
    a: int

class B(Schema):
    b: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, on="k")
"#,
    );
    assert_has_code(&result, "D0060");
    assert_message_contains(&result, "D0060", "right");
}

#[test]
fn d0060_fires_for_each_missing_key_in_a_list() {
    // k2 missing from B → one D0060.
    let result = check(
        r#"
class A(Schema):
    k1: int
    k2: int

class B(Schema):
    k1: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, on=["k1", "k2"])
"#,
    );
    assert_count(&result, "D0060", 1);
    assert_message_contains(&result, "D0060", "k2");
}

// ===========================================================================
// Result-schema inference (join + crossJoin)
// ===========================================================================

#[test]
fn join_result_schema_dedups_the_on_key_and_concatenates_the_rest() {
    // After a.join(b, on="k") the result has [k, a, b] — the key once,
    // other columns from both sides. Selecting any of them should be fine;
    // selecting something else should fail.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, on="k").select(col("k"), col("a"), col("b"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn join_result_schema_excludes_columns_that_are_not_in_either_side() {
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, on="k").select(col("nope"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn join_with_complex_on_expression_does_not_fire_when_keys_exist_on_at_least_one_side() {
    // on=col("k") == col("k") is a Compare expression, not a string/list.
    // pykrete can't tell which is left-side vs right-side, so it doesn't fire
    // D0060. Both `k`s exist on both sides → no D0030 either. The result
    // schema is left + right (concat, no dedup for expression-form), so
    // subsequent column references work for fields present in either side.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, col("k") == col("k")).select(col("a"), col("b"))
"#
    ));
    assert_does_not_have_code(&result, "D0060");
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Expression-form on-clause column-existence checks (D0030)
// ===========================================================================

#[test]
fn d0030_fires_for_a_typo_in_an_expression_form_on_clause_left_ref() {
    // `col("typo")` references a name present on NEITHER side → D0030.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, col("typo") == col("k"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

#[test]
fn d0030_fires_for_a_typo_in_an_expression_form_on_clause_right_ref() {
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, col("k") == col("typo_right"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo_right");
}

#[test]
fn d0030_fires_for_each_typo_in_an_expression_form_on_clause() {
    // Both refs are typos → two D0030s.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, col("typo_left") == col("typo_right"))
"#
    ));
    assert_count(&result, "D0030", 2);
}

#[test]
fn expression_form_on_clause_does_not_fire_when_each_name_exists_on_some_side() {
    // `a` only on left, `b` only on right — both valid in the union.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, col("a") == col("b"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0060");
}

#[test]
fn expression_form_on_clause_handles_an_and_of_two_equalities() {
    // `(col("a") == col("b")) & (col("k") == col("k"))` — every ref
    // exists in the union, so no diagnostics.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, (col("a") == col("b")) & (col("k") == col("k")))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0060");
}

#[test]
fn expression_form_on_clause_fires_for_a_typo_inside_an_and_of_equalities() {
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, (col("a") == col("b")) & (col("k") == col("nope")))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn expression_form_on_clause_does_not_dedup_join_key_in_output_schema() {
    // Spark keeps both `k`s when the on-clause is a Column expression
    // (only the string form coalesces). Both sides have `k`, so the
    // result schema has `k` from BOTH — selecting either is fine, and
    // a typo is still caught.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, col("k") == col("k")).select(col("k"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn expression_form_on_clause_propagates_both_sides_columns_to_downstream_select() {
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, col("k") == col("k")).select(col("a"), col("b"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn crossJoin_result_schema_concatenates_both_sides_without_a_key_check() {
    let result = check(
        r#"
class A(Schema):
    a1: int
    a2: int

class B(Schema):
    b1: int
    b2: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.crossJoin(b).select(col("a1"), col("a2"), col("b1"), col("b2"))
"#,
    );
    assert_does_not_have_code(&result, "D0060");
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn crossJoin_result_excludes_columns_present_in_neither_side() {
    let result = check(
        r#"
class A(Schema):
    a1: int

class B(Schema):
    b1: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.crossJoin(b).select(col("c1"))
"#,
    );
    assert_has_code(&result, "D0030");
}

#[test]
fn join_result_schema_feeds_into_return_type_validation() {
    // The declared return is Joined [k, a, b]; after the join, the
    // inferred result is also [k, a, b]. They match — no D0050.
    let result = check(
        r#"
class A(Schema):
    k: int
    a: int

class B(Schema):
    k: int
    b: int

class Joined(Schema):
    k: int
    a: int
    b: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame[Joined]:
    return a.join(b, on="k")
"#,
    );
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn join_result_schema_feeds_into_return_type_mismatch_when_wrong() {
    // Declared return only has [k, a]; actual result has [k, a, b].
    let result = check(
        r#"
class A(Schema):
    k: int
    a: int

class B(Schema):
    k: int
    b: int

class Wrong(Schema):
    k: int
    a: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame[Wrong]:
    return a.join(b, on="k")
"#,
    );
    assert_has_code(&result, "D0050");
    assert_message_contains(&result, "D0050", "b");
}

#[test]
fn expression_form_on_clause_handles_qualified_attribute_refs() {
    // `df1.k == df2.k` — the attribute-access form (no `col(...)`) that
    // production code uses for self-joins and disambiguating shared keys.
    // Both `df1.k` and `df2.k` resolve through the qualified-attribute
    // arm of `collect_col_refs`, hit the union of both schemas, and pass.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(df1: DataFrame[A], df2: DataFrame[B]) -> DataFrame:
    return df1.join(df2, df1.k == df2.k).select(col("a"), col("b"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0060");
}

#[test]
fn expression_form_on_clause_fires_for_a_typo_in_qualified_attribute_ref() {
    // `df1.nope` — `nope` is on neither side, should fire D0030.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(df1: DataFrame[A], df2: DataFrame[B]) -> DataFrame:
    return df1.join(df2, df1.nope == df2.k)
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn expression_form_on_clause_handles_F_col_equality() {
    // `F.col("a") == F.col("b")` — the `F.col` attribute-call form.
    // Both `a` and `b` are in the union → no diagnostic.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, F.col("a") == F.col("b"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0060");
}

#[test]
fn expression_form_on_clause_fires_for_typo_inside_F_col() {
    // Typo through `F.col("nope")` — still caught.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(b, F.col("k") == F.col("nope"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn local_variable_bound_to_join_result_carries_the_combined_schema() {
    // joined = a.join(b, on="k") → joined has [k, a, b].
    // joined.filter(col("b") > 0) is valid.
    let result = check(&format!(
        r#"{TWO_SCHEMAS}

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    joined = a.join(b, on="k")
    return joined.filter(col("b") > 0).select(col("k"), col("a"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0060");
}
