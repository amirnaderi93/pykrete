//! `Window.partitionBy(...).orderBy(...)` — the column names inside a
//! window-spec builder get checked against the schema of the DataFrame
//! the surrounding `.over(...)` is applied to.
//!
//! Only the inline form (`F.row_number().over(Window.partitionBy("…")…)`)
//! is checked: a window spec bound to a separate variable is decoupled
//! from any DataFrame, so pykrete can't tell which schema to check it
//! against. That's an accepted v1 gap.

mod common;

use common::{assert_count, assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
";

#[test]
fn bad_partition_by_key_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumn(\"r\", F.row_number().over(Window.partitionBy(\"nonexistent\")))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn bad_order_by_key_in_chained_spec_is_caught() {
    // `partitionBy` is valid, `orderBy` is not — the bad key lives in the
    // outer link of the builder chain.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumn(\"r\", F.row_number().over(Window.partitionBy(\"city\").orderBy(\"madeup\")))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn bad_partition_by_key_in_chained_spec_is_caught() {
    // The bad key lives in the *inner* link of the chain (`partitionBy`),
    // which sits in the call's `func`, not its arguments.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumn(\"r\", F.row_number().over(Window.partitionBy(\"madeup\").orderBy(\"amount\")))
"
    );
    assert_count(&check(&src), "D0030", 1);
}

#[test]
fn valid_window_spec_keys_pass() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumn(\"r\", F.row_number().over(Window.partitionBy(\"city\").orderBy(\"amount\")))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn window_spec_inside_select_is_checked() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(F.rank().over(Window.partitionBy(\"nope\")).alias(\"rk\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}
