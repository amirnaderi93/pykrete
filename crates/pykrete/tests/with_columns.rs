//! `df.withColumns({...})` and `df.withColumnsRenamed({...})` — the
//! dict-form bulk column methods (Spark 3.3+/3.4+).

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
";

#[test]
fn with_columns_adds_the_dict_keys_to_the_schema() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumns({{\"doubled\": col(\"amount\"), \"label\": col(\"city\")}}).select(col(\"doubled\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn with_columns_checks_references_inside_the_value_expressions() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumns({{\"doubled\": col(\"nonexistent\")}})
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn bad_column_after_with_columns_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumns({{\"doubled\": col(\"amount\")}}).select(col(\"nope\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn with_columns_renamed_applies_the_renames() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumnsRenamed({{\"amount\": \"price\"}}).select(col(\"price\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn with_columns_renamed_drops_the_old_name() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumnsRenamed({{\"amount\": \"price\"}}).select(col(\"amount\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn with_columns_renamed_checks_the_old_name_exists() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.withColumnsRenamed({{\"nonexistent\": \"price\"}})
"
    );
    assert_has_code(&check(&src), "D0030");
}
