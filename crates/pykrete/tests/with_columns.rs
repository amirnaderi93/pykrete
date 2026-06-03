//! `df.withColumns({...})` and `df.withColumnsRenamed({...})` — the
//! dict-form bulk column methods (Spark 3.3+/3.4+).

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
";

#[test]
fn with_columns_adds_the_dict_keys_to_the_schema() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumns({{\"doubled\": col(\"amount\"), \"label\": col(\"city\")}}).select(col(\"doubled\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn with_columns_checks_references_inside_the_value_expressions() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumns({{\"doubled\": col(\"nonexistent\")}})
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn bad_column_after_with_columns_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumns({{\"doubled\": col(\"amount\")}}).select(col(\"nope\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn with_columns_renamed_applies_the_renames() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumnsRenamed({{\"amount\": \"price\"}}).select(col(\"price\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn with_columns_renamed_drops_the_old_name() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumnsRenamed({{\"amount\": \"price\"}}).select(col(\"amount\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn with_columns_renamed_silently_tolerates_missing_source_name() {
    // PySpark's `withColumnsRenamed` silently ignores dict keys that
    // aren't in the receiver schema (same design as `df.drop`). Firing
    // D0030 would flag working production code as broken.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumnsRenamed({{\"nonexistent\": \"price\"}})
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn with_columns_renamed_silently_tolerates_mix_of_known_and_unknown() {
    // Both keys present in one dict — known one is renamed, missing
    // one is ignored; neither fires D0030.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.withColumnsRenamed({{\"amount\": \"price\", \"nonexistent\": \"x\"}})
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}
