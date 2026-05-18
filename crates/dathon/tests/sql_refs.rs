//! Column references *inside* embedded SQL strings get checked against
//! the dataframe schema, the same as a `col("…")` reference would.
//!
//! Three Spark surfaces accept a SQL fragment in lieu of a `Column`:
//! `F.expr("…")`, `df.selectExpr("…")`, and the string form of
//! `df.filter("…")` / `df.where("…")`. dathon parses the fragment
//! best-effort and emits `D0030` for any identifier the schema doesn't
//! have.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
";

#[test]
fn select_expr_bad_column_ref_in_sql_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.selectExpr(\"city\", \"nonexistent + 1 as bumped\")
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn select_expr_valid_column_refs_in_sql_pass() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.selectExpr(\"city\", \"amount + 1 as bumped\")
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn filter_string_predicate_bad_ref_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.filter(\"madeup > 21\")
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn filter_string_predicate_valid_ref_passes() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.filter(\"amount > 21 and city = 'x'\")
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn where_string_predicate_bad_ref_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.where(\"missing = 1\")
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn f_expr_bad_ref_inside_select_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(F.expr(\"nonexistent * 2\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn f_expr_valid_ref_inside_select_passes() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(F.expr(\"amount * 2\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn f_expr_bad_ref_inside_filter_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.filter(F.expr(\"madeup > 0\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn unparseable_sql_fragment_yields_no_diagnostic() {
    // Spark SQL has syntax `sqlparser` doesn't model; an unparseable
    // fragment must not produce a spurious column error.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.selectExpr(\"!! not sql @@\")
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn sql_string_function_call_does_not_flag_the_function_name() {
    // `length` is a SQL function, not a column — it must not be reported.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.selectExpr(\"length(city) as n\")
"
    );
    assert_no_diagnostics(&check(&src));
}
