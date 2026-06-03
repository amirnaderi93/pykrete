//! `df.melt(...)` and its alias `df.unpivot(...)` — Spark 3.4+ wide-to-long
//! reshaping. The output schema is fully determinable from the args:
//! `ids` columns are preserved, the variable column is `string`, the
//! value column is the common type of the unpivoted columns
//! (`Nullable(T)` if any of them is). Typos in `ids` or `values` fire
//! `D0030`.

#![allow(non_snake_case)]

mod common;
use common::*;

const WIDE_SCHEMA: &str = r#"
class In(Schema):
    id: int
    q1_sales: int
    q2_sales: int
    q3_sales: int
"#;

// ===========================================================================
// melt — output schema name + type pinning
// ===========================================================================

#[test]
fn melt_with_full_keyword_args_infers_schema() {
    // id (int) preserved; quarter:string; sales:int (common of three ints).
    // Declaring Out with exactly those names + types should fire neither
    // D0080 (type mismatch) nor D0030 (name typo).
    let src = format!(
        r#"{WIDE_SCHEMA}
class Out(Schema):
    id: int
    quarter: string
    sales: int

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(
        ids=["id"],
        values=["q1_sales", "q2_sales", "q3_sales"],
        variableColumnName="quarter",
        valueColumnName="sales",
    )
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
    assert_count(&result, "D0050", 0);
    assert_count(&result, "D0083", 0);
}

#[test]
fn melt_with_positional_args_infers_schema() {
    let src = format!(
        r#"{WIDE_SCHEMA}
class Out(Schema):
    id: int
    quarter: string
    sales: int

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(["id"], ["q1_sales", "q2_sales", "q3_sales"], "quarter", "sales")
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
    assert_count(&result, "D0050", 0);
    assert_count(&result, "D0083", 0);
}

#[test]
fn melt_default_variable_and_value_names() {
    // Omit variableColumnName / valueColumnName — Spark's defaults are
    // "variable" and "value". An Out that uses those names lines up;
    // an Out that uses "quarter"/"sales" would fire D0050 / D0030 on
    // the downstream `.select`, so we pin via select.
    let src = format!(
        r#"{WIDE_SCHEMA}
def f(d: SparkFrame[In]) -> SparkFrame:
    return d.melt(ids=["id"], values=["q1_sales", "q2_sales", "q3_sales"]).select(
        col("id"), col("variable"), col("value")
    )
"#
    );
    let result = check(&src);
    assert_count(&result, "D0030", 0);

    // The negative side: selecting "quarter" or "sales" (the *non*-default
    // names) against the default-named melt output should fire D0030.
    let src_bad = format!(
        r#"{WIDE_SCHEMA}
def f(d: SparkFrame[In]) -> SparkFrame:
    return d.melt(ids=["id"], values=["q1_sales", "q2_sales", "q3_sales"]).select(col("quarter"))
"#
    );
    let result_bad = check(&src_bad);
    assert_count(&result_bad, "D0030", 1);
    assert_message_contains(&result_bad, "D0030", "quarter");
}

#[test]
fn melt_with_values_none_unpivots_all_non_ids() {
    // `values=None` → unpivot every non-`ids` column. The "id" is the
    // only id, so q1_sales, q2_sales, q3_sales feed the value column.
    // Their common type is int. Same schema as the explicit-list case.
    let src = format!(
        r#"{WIDE_SCHEMA}
class Out(Schema):
    id: int
    quarter: string
    sales: int

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(ids=["id"], values=None, variableColumnName="quarter", valueColumnName="sales")
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
    assert_count(&result, "D0050", 0);
    assert_count(&result, "D0083", 0);

    // Same expectation when `values` is omitted entirely (keyword-only call).
    let src_omitted = format!(
        r#"{WIDE_SCHEMA}
class Out(Schema):
    id: int
    quarter: string
    sales: int

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(ids=["id"], variableColumnName="quarter", valueColumnName="sales")
"#
    );
    let result_omitted = check_strict(&src_omitted);
    assert_count(&result_omitted, "D0080", 0);
    assert_count(&result_omitted, "D0030", 0);
    assert_count(&result_omitted, "D0050", 0);
    assert_count(&result_omitted, "D0083", 0);
}

#[test]
fn melt_typo_in_ids_fires_D0030() {
    let src = format!(
        r#"{WIDE_SCHEMA}
def f(d: SparkFrame[In]) -> SparkFrame:
    return d.melt(ids=["i_d"], values=["q1_sales", "q2_sales", "q3_sales"])
"#
    );
    let result = check(&src);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "i_d");
}

#[test]
fn melt_typo_in_values_fires_D0030() {
    let src = format!(
        r#"{WIDE_SCHEMA}
def f(d: SparkFrame[In]) -> SparkFrame:
    return d.melt(ids=["id"], values=["q1_sales", "q2_zales", "q3_sales"])
"#
    );
    let result = check(&src);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "q2_zales");
}

#[test]
fn melt_heterogeneous_values_degrades_to_unknown() {
    // int and string can't widen to a common type — value column type
    // becomes Unknown. A downstream Out declaring `sales: string` (or
    // any concrete type) should not fire D0080 — Unknown is permissive.
    let src = r#"
class In(Schema):
    id: int
    s: string
    n: int

class Out(Schema):
    id: int
    quarter: string
    sales: string

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(ids=["id"], values=["s", "n"], variableColumnName="quarter", valueColumnName="sales")
"#;
    let result = check_strict(src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
    assert_count(&result, "D0050", 0);
}

#[test]
fn melt_numeric_widening_in_values() {
    // int + long + double → double. Declaring `sales: double` passes;
    // declaring it as `string` should fire D0080.
    let src_ok = r#"
class In(Schema):
    id: int
    a: int
    b: long
    c: double

class Out(Schema):
    id: int
    quarter: string
    sales: double

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(ids=["id"], values=["a", "b", "c"], variableColumnName="quarter", valueColumnName="sales")
"#;
    let result_ok = check_strict(src_ok);
    assert_count(&result_ok, "D0080", 0);
    assert_count(&result_ok, "D0030", 0);
    assert_count(&result_ok, "D0050", 0);

    let src_bad = r#"
class In(Schema):
    id: int
    a: int
    b: long
    c: double

class Out(Schema):
    id: int
    quarter: string
    sales: string

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(ids=["id"], values=["a", "b", "c"], variableColumnName="quarter", valueColumnName="sales")
"#;
    let result_bad = check_strict(src_bad);
    assert_count(&result_bad, "D0080", 1);
}

#[test]
fn melt_nullable_value_propagates() {
    // One of the value columns is `Optional[int]` — the value column
    // becomes `Nullable(int)`. Declaring `sales: int` (non-null) should
    // fire D0083; declaring `sales: Optional[int]` should not.
    let src_strict = r#"
class In(Schema):
    id: int
    a: int
    b: Optional[int]

class Out(Schema):
    id: int
    quarter: string
    sales: int

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(ids=["id"], values=["a", "b"], variableColumnName="quarter", valueColumnName="sales")
"#;
    let result_strict = check_strict(src_strict);
    assert_count(&result_strict, "D0083", 1);

    let src_ok = r#"
class In(Schema):
    id: int
    a: int
    b: Optional[int]

class Out(Schema):
    id: int
    quarter: string
    sales: Optional[int]

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.melt(ids=["id"], values=["a", "b"], variableColumnName="quarter", valueColumnName="sales")
"#;
    let result_ok = check_strict(src_ok);
    assert_count(&result_ok, "D0083", 0);
    assert_count(&result_ok, "D0080", 0);
}

#[test]
fn unpivot_is_alias_for_melt() {
    // Same call shape, same expected output schema, same diagnostics.
    let src = format!(
        r#"{WIDE_SCHEMA}
class Out(Schema):
    id: int
    quarter: string
    sales: int

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.unpivot(
        ids=["id"],
        values=["q1_sales", "q2_sales", "q3_sales"],
        variableColumnName="quarter",
        valueColumnName="sales",
    )
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
    assert_count(&result, "D0050", 0);
    assert_count(&result, "D0083", 0);

    // Typo in unpivot's ids should still fire D0030.
    let src_bad = format!(
        r#"{WIDE_SCHEMA}
def f(d: SparkFrame[In]) -> SparkFrame:
    return d.unpivot(ids=["i_d"], values=["q1_sales", "q2_sales", "q3_sales"])
"#
    );
    let result_bad = check(&src_bad);
    assert_count(&result_bad, "D0030", 1);
    assert_message_contains(&result_bad, "D0030", "i_d");
}
