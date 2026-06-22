//! v1.14 PR-D1 — D0080 dialect-on-return constructor arms.
//!
//! Spec: `docs/design/v1.14-spec.md` §1.ii.
//!
//! v1.13 PR-D1 plumbed D0080 dialect-on-return through the chain walker
//! (`inherited_chain_state`), catching mismatches where the body's chain
//! bottoms out at a tagged Name (parameter, walrus, `.toPandas()` flip,
//! `<X>.createDataFrame(...)` flip). It stayed honest-silent on the
//! constructor case: `pd.DataFrame(...)` and `spark.read.<format>(...)`
//! bottom out at a literal `Name("pd")` / `Name("spark")` that pykrete has
//! no frame binding for.
//!
//! PR-D1 (v1.14) adds three structural recognizer arms:
//!
//!   * Arm 1 — `pd.DataFrame(...)` → `(Some(Pandas), false)`.
//!   * Arm 2 — `spark.read.<any_format>(...)` → `(Some(Spark), false)`
//!     (two-level structural; any `<format>` identifier matches).
//!   * Arm 3 — `<X>.createDataFrame(rows, schema=<bare Schema name>)` →
//!     `(Some(Spark), false)`. Dialect-only extension to the v1.5 PR-A2
//!     gate; the view-side resolver is intentionally NOT widened.
//!
//! Per spec §1.ii.2, `Name("pd")` / `Name("spark")` are HEURISTIC literal-
//! name matches, NOT symbol-table lookups. Aliased imports
//! (`import pandas as p; p.DataFrame(...)`) fall through to the v1.13
//! baseline silent-pass behavior — same shape as v1.6's structural-match
//! rules for `pivot_table` / `stack` / `unstack`.

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// Arm 1 — `pd.DataFrame(...)`
// ---------------------------------------------------------------------------

#[test]
fn V114D1_arm1_spark_decl_returns_pd_dataframe_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f() -> SparkFrame[Orders]:
    return pd.DataFrame({"id": [1, 2], "status": ["a", "b"]})
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
    assert_message_contains(&result, "D0080", "unresolved PandasFrame value");
}

#[test]
fn V114D1_arm1_spark_decl_returns_pd_dataframe_kwarg_form_fires_d0080() {
    // `pd.DataFrame(data=...)` — the kwarg form of the same constructor.
    // The structural recognizer doesn't care about argument shape; the
    // dialect tag fires off the func shape alone.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(payload: dict) -> SparkFrame[Orders]:
    return pd.DataFrame(data=payload)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as SparkFrame");
}

#[test]
fn V114D1_arm1_pandas_decl_returns_pd_dataframe_silent() {
    // Same-dialect case: `-> PandasFrame[X]: return pd.DataFrame(...)` is
    // the canonical pandas constructor idiom — D0080 must stay silent.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f() -> PandasFrame[Orders]:
    return pd.DataFrame({"id": [1, 2], "status": ["a", "b"]})
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

// ---------------------------------------------------------------------------
// Arm 2 — `spark.read.<format>(...)`
// ---------------------------------------------------------------------------

#[test]
fn V114D1_arm2_pandas_decl_returns_spark_read_parquet_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(path: str) -> PandasFrame[Orders]:
    return spark.read.parquet(path)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
    assert_message_contains(&result, "D0080", "unresolved SparkFrame value");
}

#[test]
fn V114D1_arm2_pandas_decl_returns_spark_read_json_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(path: str) -> PandasFrame[Orders]:
    return spark.read.json(path)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
}

#[test]
fn V114D1_arm2_pandas_decl_returns_spark_read_csv_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(path: str) -> PandasFrame[Orders]:
    return spark.read.csv(path)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
}

#[test]
fn V114D1_arm2_pandas_decl_returns_spark_read_orc_fires_d0080() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(path: str) -> PandasFrame[Orders]:
    return spark.read.orc(path)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
}

#[test]
fn V114D1_arm2_spark_read_then_chain_fires_d0080() {
    // `spark.read.parquet(path).filter(...)` — the outer `.filter()` is
    // dialect-preserving (no flip arm), so the chain walker descends to
    // the bottom-most `.parquet(...)` call and fires Arm 2.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(path: str) -> PandasFrame[Orders]:
    return spark.read.parquet(path).filter(col("id") > 0)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
}

#[test]
fn V114D1_arm2_spark_decl_returns_spark_read_silent() {
    // Same-dialect case: `-> SparkFrame[X]: return spark.read.parquet(...)`
    // is the canonical Spark reader idiom — D0080 must stay silent.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(path: str) -> SparkFrame[Orders]:
    return spark.read.parquet(path)
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

// ---------------------------------------------------------------------------
// Arm 3 — `<X>.createDataFrame(rows, schema=<bare Schema name>)`
// ---------------------------------------------------------------------------

#[test]
fn V114D1_arm3_pandas_decl_returns_create_dataframe_python_literal_fires_d0080() {
    // The Python-literal-rows form: `spark.createDataFrame([(1,2)],
    // schema=Orders)`. The `schema=` kwarg's value is a bare Schema class
    // Name, not a Spark-tagged frame binding — the v1.5 PR-A2 gate stays
    // silent. Arm 3 fires structurally on the `schema=<Name>` shape.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(spark: SparkSession) -> PandasFrame[Orders]:
    return spark.createDataFrame([(1, "a"), (2, "b")], schema=Orders)
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "declared as PandasFrame");
    assert_message_contains(&result, "D0080", "unresolved SparkFrame value");
}

#[test]
fn V114D1_arm3_spark_decl_returns_create_dataframe_python_literal_silent() {
    // Same-dialect case: Spark-decl + Python-literal-rows construction is
    // the canonical idiom — D0080 must stay silent.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(spark: SparkSession) -> SparkFrame[Orders]:
    return spark.createDataFrame([(1, "a"), (2, "b")], schema=Orders)
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

// ---------------------------------------------------------------------------
// Negative-space — literal-name guards (spec §1.ii.2)
// ---------------------------------------------------------------------------

#[test]
fn V114D1_aliased_pandas_import_arm1_does_not_fire() {
    // `import pandas as p; p.DataFrame(...)` — `p` is not the heuristic
    // literal `pd`, so Arm 1's structural recognizer falls through to the
    // v1.13 baseline silent-pass. Documented as accepted limitation per
    // spec §1.ii.2/§1.ii.3.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f() -> SparkFrame[Orders]:
    return p.DataFrame({"id": [1, 2], "status": ["a", "b"]})
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

#[test]
fn V114D1_aliased_spark_session_arm2_does_not_fire() {
    // `from pyspark.sql import SparkSession as ss; ss.read.parquet(...)`
    // — `ss` is not the heuristic literal `spark`, so Arm 2's two-level
    // structural recognizer falls through. v1.13 baseline holds.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(path: str) -> PandasFrame[Orders]:
    return ss.read.parquet(path)
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

#[test]
fn V114D1_opaque_helper_unbound_function_silent() {
    // `return some_helper()` — opaque function call, dialect unknown per
    // honest-silence. Regression guard: the new constructor arms don't
    // overreach into the generic Call fall-through.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f() -> SparkFrame[Orders]:
    return some_helper()
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}

// ---------------------------------------------------------------------------
// Regression — pre-existing v1.13 PR-D1 arms still fire
// ---------------------------------------------------------------------------

#[test]
fn V114D1_regression_topandas_chain_still_fires() {
    // v1.13 PR-D1's `.toPandas()` arm — must still fire post-PR. Ordering
    // sanity: the new constructor arms slot in after the existing
    // `createDataFrame` and `toPandas` flip arms.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return sdf.toPandas()
"#,
    );
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "body produces PandasFrame");
}

#[test]
fn V114D1_regression_same_dialect_param_silent() {
    // v1.13 PR-D1 negative-space — same-dialect parameter pass-through
    // must stay silent post-PR.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(sdf: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return sdf
"#,
    );
    assert_does_not_have_code(&result, "D0080");
}
