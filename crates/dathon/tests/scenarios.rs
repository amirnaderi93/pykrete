//! End-to-end realistic scenarios that exercise multiple features at once.
//!
//! Each test is a small, plausible-looking PySpark module written in dathon
//! syntax. The intent is to verify that features compose correctly — not to
//! re-test each feature in isolation (that's the job of the other test
//! files).

mod common;
use common::*;

#[test]
fn a_clean_etl_pipeline_produces_no_diagnostics() {
    // Read a raw schema, project + cast into a normalized schema, return it.
    // Models the typical "prepare_X" function pattern.
    let result = check(r#"
class RawOrders(Schema):
    ProductCode: int
    UnitPrice: double
    CreatedAt: timestamp

class Orders(Schema):
    place_code: int
    price: int
    log_date: timestamp

def prepare_orders(raw: DataFrame[RawOrders]) -> DataFrame[Orders]:
    return raw.select(
        col("ProductCode").alias("place_code"),
        col("UnitPrice").cast("int").alias("price"),
        col("CreatedAt").alias("log_date"),
    )
"#);
    assert_no_diagnostics(&result);
    assert_eq!(result.schema_count, 2);
    assert_eq!(result.typed_function_count, 1);
}

#[test]
fn a_realistic_pipeline_with_an_intermediate_variable_catches_typo() {
    // Standard pattern: filter raw → keep, project → return.
    // The typo `priec` in the projection should be flagged against the
    // (still Orders) schema after the filter.
    let result = check(r#"
class Orders(Schema):
    place_code: int
    price: int

def prepare(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    filtered = raw.filter(col("price") > 0)
    return filtered.select(col("place_code"), col("priec"))
"#);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "priec");
}

#[test]
fn a_unionByName_between_two_prepare_functions_outputs() {
    // Two prepare-style functions return the same schema; their outputs
    // can be unioned cleanly.
    let result = check(r#"
class Common(Schema):
    id: int
    name: string

def combine(a: DataFrame[Common], b: DataFrame[Common]) -> DataFrame[Common]:
    return a.unionByName(b)
"#);
    assert_no_diagnostics(&result);
}

#[test]
fn a_file_with_multiple_issues_reports_all_of_them() {
    // One schema has an unknown type, one function references an unknown
    // schema, one function has a column-name typo, and one has a return
    // type mismatch. dathon should surface all four independently.
    let result = check(r#"
class Orders(Schema):
    place_code: int
    price: int
    weird: WeirdType

class RawOrders(Schema):
    ProductCode: int

def has_typo(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code"), col("priec"))

def has_unknown_schema(x: DataFrame[NoSuchSchema]) -> DataFrame[Orders]:
    pass

def has_return_mismatch(raw: DataFrame[RawOrders]) -> DataFrame[Orders]:
    return raw.select(col("ProductCode"))
"#);
    assert_has_code(&result, "D0010"); // weird: WeirdType
    assert_has_code(&result, "D0020"); // DataFrame[NoSuchSchema]
    assert_has_code(&result, "D0030"); // priec typo
    assert_has_code(&result, "D0050"); // RawOrders body vs Orders annotation
}

#[test]
fn a_long_chain_of_operations_propagates_schemas_correctly() {
    // raw -> filter (preserves) -> withColumn (adds discount) -> select
    // (projects to place_code, discount) -> filter (still ok)
    // No diagnostic expected.
    let result = check(r#"
class Orders(Schema):
    place_code: int
    price: int

def pipeline(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return (
        raw
        .filter(col("price") > 0)
        .withColumn("discount", col("price") * 1)
        .select(col("place_code"), col("discount"))
        .filter(col("discount") < 1000)
    )
"#);
    // The chained select projects to `discount` and `place_code`. The
    // outer filter then references `discount`, which is in scope.
    // No D0030 expected. The return schema is [place_code, discount],
    // which doesn't match Orders [place_code, price] — so D0050 fires.
    assert_does_not_have_code(&result, "D0030");
    assert_has_code(&result, "D0050");
}

#[test]
fn a_function_with_only_an_untyped_DataFrame_param_is_not_checked() {
    // No DataFrame[Schema] anywhere — the body isn't checked even when
    // there are obvious col() references that would be wrong if there
    // were a schema. This is the v0.1 "untyped DataFrame escape hatch".
    let result = check(r#"
def f(raw: DataFrame) -> DataFrame:
    return raw.select(col("anything"), col("else"))
"#);
    assert_does_not_have_code(&result, "D0030");
    assert_eq!(result.typed_function_count, 1);
}
