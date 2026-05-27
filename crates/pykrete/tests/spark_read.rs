//! `spark.read.<format>(path)` / `spark.read.format(...).load(...)` /
//! `spark.table(...)` — opaque IO sources.
//!
//! The schema can't be inferred without runtime info, so these calls
//! return Unknown. The chain dies until the user re-anchors with
//! `.cast(DataFrame[Schema])` or a typed variable annotation, after
//! which downstream column checks resume.
//!
//! These tests pin the three behaviors:
//! - opaque source on its own — chain dies silently (no false positives)
//! - opaque source + `.cast(DataFrame[X])` — chain re-anchored
//! - opaque source + `name: DataFrame[X] = ...` — chain re-anchored

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
def f(spark) -> DataFrame:
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
def f(spark) -> DataFrame:
    raw = spark.read.parquet(\"/data/orders\").cast(DataFrame[Orders])
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn spark_read_parquet_with_typed_var_annotation_anchors_the_chain() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> DataFrame:
    raw: DataFrame[Orders] = spark.read.parquet(\"/data/orders\")
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
def f(spark) -> DataFrame:
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
def f(spark) -> DataFrame:
    raw = spark.read.json(\"/data/orders.json\").cast(DataFrame[Orders])
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
def f(spark) -> DataFrame:
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
def f(spark) -> DataFrame:
    raw: DataFrame[Orders] = spark.read.format(\"parquet\").load(\"/data/orders\")
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

// ===========================================================================
// `spark.table(name)` — opaque source, same treatment.
// ===========================================================================

#[test]
fn spark_table_returns_opaque_source() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> DataFrame:
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
def f(spark) -> DataFrame:
    raw = spark.table(\"db.orders\").cast(DataFrame[Orders])
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn spark_table_with_typed_var_annotation_anchors_the_chain() {
    let src = format!(
        "{SCHEMA}
def f(spark) -> DataFrame:
    raw: DataFrame[Orders] = spark.table(\"db.orders\")
    return raw.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}
