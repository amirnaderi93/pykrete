//! Two narrower analyzer gaps: `explode`'s default output column name,
//! and `subset=` keyword-argument column checking.

mod common;

use common::{
    assert_count, assert_does_not_have_code, assert_has_code, assert_message_contains,
    assert_no_diagnostics, check,
};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
    tags: string
";

#[test]
fn explode_without_alias_produces_a_col_column() {
    // `F.explode("tags")` with no `.alias(...)` — Spark names the
    // unnested column `col`, which downstream code can then reference.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(F.explode(\"tags\")).select(col(\"col\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn explode_with_alias_still_uses_the_alias() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.select(F.explode(\"tags\").alias(\"tag\")).select(col(\"tag\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn dropna_subset_bad_column_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.dropna(subset=[\"city\", \"nonexistent\"])
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn fillna_subset_good_columns_pass() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.fillna(0, subset=[\"amount\"])
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn drop_duplicates_subset_bad_column_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.dropDuplicates(subset=[\"madeup\"])
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn na_drop_subset_bad_column_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.na.drop(subset=[\"nonexistent\"])
"
    );
    assert_has_code(&check(&src), "D0030");
}

// ===========================================================================
// fillna(dict) — the dict-literal positional form
// ===========================================================================

#[test]
fn fillna_dict_with_good_keys_passes() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.fillna({{\"amount\": 0, \"city\": \"unknown\"}})
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn fillna_dict_with_bad_key_is_caught() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.fillna({{\"amunt\": 0}})
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "amunt");
}

#[test]
fn fillna_dict_fires_for_every_bad_key_in_a_mixed_dict() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.fillna({{\"amount\": 0, \"nope1\": 0, \"city\": \"x\", \"nope2\": \"y\"}})
"
    );
    let result = check(&src);
    assert_count(&result, "D0030", 2);
    assert_message_contains(&result, "D0030", "nope1");
    assert_message_contains(&result, "D0030", "nope2");
}

#[test]
fn fillna_with_a_non_dict_literal_arg_does_not_false_flag() {
    // A variable holding a dict — we can't see the keys, so we MUST
    // NOT emit a diagnostic. Returning the receiver schema unchanged
    // is the correct conservative behavior.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    cfg = {{\"amount\": 0}}
    return raw.fillna(cfg)
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn na_fill_dict_with_bad_key_is_caught() {
    // Same form as `fillna`, but on the `df.na.fill` route.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.na.fill({{\"amunt\": 0}})
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "amunt");
}

#[test]
fn fillna_dict_does_not_strip_the_chain() {
    // Result schema is still the receiver's, so a follow-up
    // `select("amount")` works and `select("nope")` fires D0030.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    return raw.fillna({{\"amount\": 0}}).select(col(\"amount\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}
