//! `spark.sql("SELECT …")` — pykrete infers the result schema from the
//! query's projection columns, so the chain stays checkable. A query it
//! can't read cleanly (wildcard, CTE, unaliased expression) degrades to
//! an unknown schema.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

#[test]
fn spark_sql_projection_columns_drive_downstream_checks() {
    let src = "\
def f() -> DataFrame:
    return spark.sql(\"SELECT amount, city FROM orders\").select(col(\"nonexistent\"))
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn spark_sql_projection_column_resolves() {
    let src = "\
def f() -> DataFrame:
    return spark.sql(\"SELECT amount, city FROM orders\").select(col(\"amount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn spark_sql_as_alias_is_the_output_name() {
    let src = "\
def f() -> DataFrame:
    return spark.sql(\"SELECT amount AS total FROM orders\").select(col(\"total\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn spark_sql_alias_replaces_the_original_name() {
    let src = "\
def f() -> DataFrame:
    return spark.sql(\"SELECT amount AS total FROM orders\").select(col(\"amount\"))
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn spark_sql_wildcard_degrades_to_unknown() {
    // `SELECT *` — pykrete can't know the columns, so the chain degrades
    // and the downstream reference is not (mis-)flagged.
    let src = "\
def f() -> DataFrame:
    return spark.sql(\"SELECT * FROM orders\").select(col(\"whatever\"))
";
    assert_does_not_have_code(&check(src), "D0030");
}
