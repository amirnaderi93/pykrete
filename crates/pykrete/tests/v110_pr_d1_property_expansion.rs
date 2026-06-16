//! v1.10 PR-D1 — D0091 surface completion: property expansion.
//!
//! Closes v1.9 spark-audit findings spark-I1 + spark-I2. v1.9 PR-D2
//! shipped the bare-attribute D0091 arm with 3 Spark + 4 pandas
//! discriminator-property entries; v1.10 PR-D1 expands the discriminator
//! coverage:
//!
//! - Spark direction: adds `na`, `write`, `writeStream`, `storageLevel`
//!   to [`pykrete::SPARK_DISCRIMINATOR_PROPERTIES`]. A PandasFrame
//!   receiver accessing one of these fires D0091.
//! - Pandas direction: adds `index`, `values`, `shape`, `T` to
//!   [`pykrete::PANDAS_INHERITED_PROPERTIES`]. A SparkFrame receiver
//!   accessing one of these fires D0091.
//!
//! Each new entry was collision-audited against the opposing dialect's
//! DataFrame surface (see PR body); paired positive + negative tests
//! per v1.6 retro rule 8.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Spark-direction (v1.9 spark-I1) — Pandas receiver, Spark-only property.
// Each new property: positive (cross-dialect → D0091) + negative
// (correct dialect → silent).
// ---------------------------------------------------------------------------

#[test]
fn V110D1_pandas_na_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.na
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "na");
    assert_message_contains(&result, "D0091", "PandasFrame");
}

#[test]
fn V110D1_spark_na_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.na
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V110D1_pandas_write_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.write
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "write");
    assert_message_contains(&result, "D0091", "PandasFrame");
}

#[test]
fn V110D1_spark_write_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.write
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V110D1_pandas_writeStream_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.writeStream
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "writeStream");
}

#[test]
fn V110D1_spark_writeStream_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.writeStream
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V110D1_pandas_storageLevel_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.storageLevel
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "storageLevel");
}

#[test]
fn V110D1_spark_storageLevel_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.storageLevel
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Pandas-direction (v1.9 spark-I2) — Spark receiver, pandas-only property.
// Each new property: positive + negative.
// ---------------------------------------------------------------------------

#[test]
fn V110D1_spark_index_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.index
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "index");
    assert_message_contains(&result, "D0091", "SparkFrame");
}

#[test]
fn V110D1_pandas_index_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.index
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V110D1_spark_values_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.values
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "values");
}

#[test]
fn V110D1_pandas_values_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.values
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V110D1_spark_shape_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.shape
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "shape");
}

#[test]
fn V110D1_pandas_shape_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.shape
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

#[test]
fn V110D1_spark_T_fires_d0091() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(sdf: SparkFrame[Sale]):
    return sdf.T
"#,
    );
    assert_has_code(&result, "D0091");
    assert_message_contains(&result, "D0091", "T");
}

#[test]
fn V110D1_pandas_T_does_not_fire() {
    let result = check(
        r#"
class Sale(Schema):
    region: string

def f(pdf: PandasFrame[Sale]):
    return pdf.T
"#,
    );
    assert_does_not_have_code(&result, "D0091");
}

// ---------------------------------------------------------------------------
// Disjointness — the new entries do NOT leak into `PANDAS_INHERITED_ARMS`
// (shared-with-Spark method names). Regression guard mirroring the v1.9
// PR-D2 `V19D2_property_tables_are_disjoint_from_inherited_arms` test —
// the new v1.10 entries pass the same disjointness invariant.
// ---------------------------------------------------------------------------

#[test]
fn V110D1_new_property_entries_disjoint_from_inherited_arms() {
    use pykrete::{
        PANDAS_INHERITED_ARMS, PANDAS_INHERITED_PROPERTIES, SPARK_DISCRIMINATOR_PROPERTIES,
    };
    for new in &["na", "write", "writeStream", "storageLevel"] {
        assert!(
            SPARK_DISCRIMINATOR_PROPERTIES.contains(new),
            "expected new v1.10 entry {new} in SPARK_DISCRIMINATOR_PROPERTIES"
        );
        assert!(
            !PANDAS_INHERITED_ARMS.contains(new),
            "'{new}' leaked into PANDAS_INHERITED_ARMS — would over-fire \
             on a bare `df.{new}` reference"
        );
    }
    for new in &["index", "values", "shape", "T"] {
        assert!(
            PANDAS_INHERITED_PROPERTIES.contains(new),
            "expected new v1.10 entry {new} in PANDAS_INHERITED_PROPERTIES"
        );
        assert!(
            !PANDAS_INHERITED_ARMS.contains(new),
            "'{new}' leaked into PANDAS_INHERITED_ARMS"
        );
    }
}
