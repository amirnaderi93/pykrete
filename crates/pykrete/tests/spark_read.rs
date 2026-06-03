//! `spark.read.<format>(path)` / `spark.read.format(...).load(...)` /
//! `spark.table(...)` — opaque IO sources.
//!
//! The schema can't be inferred without runtime info, so these calls
//! return Unknown. The chain dies until the user re-anchors with
//! `.cast(SparkFrame[Schema])` or a typed variable annotation, after
//! which downstream column checks resume.
//!
//! These tests pin the three behaviors:
//! - opaque source on its own — chain dies silently (no false positives)
//! - opaque source + `.cast(SparkFrame[X])` — chain re-anchored
//! - opaque source + `name: SparkFrame[X] = ...` — chain re-anchored

#![allow(non_snake_case)]

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Orders(Schema):
    place_code: int
    price: int
";

// ===========================================================================
// `spark.read.<format>(path)` — opaque source
// ===========================================================================

#[test]
fn spark_read_parquet_returns_opaque_source() {
    // Without re-anchor, `raw` is Unknown — column existence isn't
    // checked. This is the permissive "Unknown silently passes" stance.
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw = spark.read.parquet(\"/data/orders\")
    return raw.select(col(\"anything_at_all\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn spark_read_parquet_with_cast_anchors_the_chain() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw = spark.read.parquet(\"/data/orders\").cast(SparkFrame[Orders])
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn spark_read_parquet_with_typed_var_annotation_anchors_the_chain() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw: SparkFrame[Orders] = spark.read.parquet(\"/data/orders\")
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

// ===========================================================================
// Other formats — same code path, one happy + one re-anchor case.
// ===========================================================================

#[test]
fn spark_read_csv_returns_opaque_source() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw = spark.read.csv(\"/data/orders.csv\")
    return raw.select(col(\"anything\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn spark_read_json_with_cast_anchors_the_chain() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw = spark.read.json(\"/data/orders.json\").cast(SparkFrame[Orders])
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

// ===========================================================================
// `spark.read.format("…").load(path)` — builder form, same opaque output.
// ===========================================================================

#[test]
fn spark_read_format_load_returns_opaque_source() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw = spark.read.format(\"parquet\").load(\"/data/orders\")
    return raw.select(col(\"anything\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn spark_read_format_load_with_typed_var_annotation_anchors_the_chain() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw: SparkFrame[Orders] = spark.read.format(\"parquet\").load(\"/data/orders\")
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn spark_read_option_chain_returns_opaque_source() {
    // Chained `.option(...).option(...)` calls before the format method —
    // the reader-builder recursion peels each `option` and lands on
    // `spark.read`, so the whole chain is recognized as an opaque source.
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw = spark.read.option(\"header\", \"true\").option(\"inferSchema\", \"true\").csv(\"/data/orders\")
    return raw.select(col(\"anything\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn spark_read_schema_then_format_returns_opaque_source() {
    // `.schema(s).<format>(...)` form — the schema arg is an arbitrary
    // expression (here a bare string placeholder); the matcher only cares
    // that the receiver of `.parquet(...)` is a recognized builder call.
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    schema = \"orders_schema\"
    raw = spark.read.schema(schema).parquet(\"/data/orders\")
    return raw.select(col(\"anything\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

// ===========================================================================
// `spark.table(name)` — opaque source, same treatment.
// ===========================================================================

#[test]
fn spark_table_returns_opaque_source() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw = spark.table(\"db.orders\")
    return raw.select(col(\"anything\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn spark_table_with_cast_anchors_the_chain() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw = spark.table(\"db.orders\").cast(SparkFrame[Orders])
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn spark_table_with_typed_var_annotation_anchors_the_chain() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> SparkFrame:
    raw: SparkFrame[Orders] = spark.table(\"db.orders\")
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

// ===========================================================================
// Regression guard: the existing `dal.read(SOURCE)` pattern must keep its
// typed return — the new opaque-source matchers must not eat it.
// ===========================================================================

#[test]
fn dal_read_pattern_is_not_intercepted_as_opaque_source() {
    // `DataAccessLayer.read(RAW_ORDERS)` goes through the generic-method
    // substitution path and returns `SparkFrame[Orders]`. If the new
    // opaque-source matchers accidentally swallowed this shape, the chain
    // would silently turn into Unknown and `col("nonexistent")` would pass
    // unchecked. We assert D0030 still fires — proving the typed chain
    // survives.
    let src = "\
class Orders(Schema):
    place_code: int
    price: int

class DataSource[T]:
    def __init__(self, path):
        pass

class DataAccessLayer:
    def read[T](self, source: DataSource[T]) -> SparkFrame[T]:
        pass

RAW_ORDERS: DataSource[Orders] = DataSource(\"/path\")

def f(dal: DataAccessLayer) -> SparkFrame[Orders]:
    raw = dal.read(RAW_ORDERS)
    return raw.select(col(\"nonexistent\"))
";
    assert_has_code(&check(src), "D0030");
}
