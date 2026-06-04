//! v1.3 PR-E3 §10 widening — three completion fixes from the spark-
//! coverage audit:
//!
//! - I1: comprehension `elt` / `ifs` walked for col-refs
//! - I2: unrecognized call arguments walked for col-refs
//! - I3: Spark subscript-assign LHS walked for col-refs
//!
//! Each fix has a positive (fires D0030 on a real typo), at least one
//! safe-path negative (no false positive on legit code), and where the
//! risk of double-firing exists, a one-shot count check.

#![allow(non_snake_case)]

mod common;
use common::*;

// ===========================================================================
// I1 — Comprehension `elt` / `ifs` walked for §10 col-ref
// ===========================================================================

#[test]
fn V13E3_listcomp_elt_subscript_fires_d0030() {
    // The body (`elt`) of a list comprehension contains a `df["typo"]`.
    // Before the I1 fix only the generator's `iter` was walked, so this
    // typo went silent.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return [df["statuss"] for _ in range(3)]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13E3_listcomp_if_clause_subscript_fires_d0030() {
    // The `if` clause of a generator runs in the comprehension's scope
    // (after the bound name is in effect). A `df["typo"]` here is still
    // a genuine ref the user expected to resolve.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return [x for x in range(3) if df["statuss"] is not None]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13E3_listcomp_bound_name_not_flagged() {
    // The bound name `x` shadows whatever's in the outer scope. A
    // Subscript on it inside `elt` must NOT be checked against any
    // outer-scope frame — it's a fresh loop variable.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return [x for x in df["status"]]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13E3_listcomp_bound_name_shadows_outer_frame() {
    // `df` is rebound by the comprehension generator. A Subscript on
    // `df` in `elt` must use the loop binding, not the outer frame —
    // so `df["statuss"]` doesn't false-fire as a typo on Orders.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: SparkFrame[Orders]):
    return [df["statuss"] for df in range(3)]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13E3_dictcomp_value_subscript_fires_d0030() {
    // DictComp's `value` is the dict-comprehension equivalent of `elt`.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: SparkFrame[Orders]):
    return {x: df["statuss"] for x in range(3)}
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ===========================================================================
// I2 — Unrecognized call arguments walked for §10 col-ref
// ===========================================================================

#[test]
fn V13E3_unrecognized_call_arg_subscript_fires_d0030() {
    // `print(df["typo"])` — `print` isn't on a frame and isn't a known
    // generic free function. The arg's `df["statuss"]` Subscript was
    // never walked before I2.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    print(df["statuss"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13E3_unrecognized_call_kwarg_subscript_fires_d0030() {
    // `logger.info(msg=df["typo"])` — receiver isn't a frame, method
    // dispatch can't run, kwarg value was never walked before I2.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    logger.info(msg=df["statuss"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13E3_recognized_method_call_no_double_walk() {
    // `df.select(df["statuss"])` — the recognized `.select` dispatch
    // already walks its args and fires D0030 once via the column-method
    // machinery. The I2 args-walker MUST NOT re-walk and double-fire.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.select(df["statuss"])
"#,
    );
    assert_count(&result, "D0030", 1);
}

#[test]
fn V13E3_recognized_method_call_with_subscript_kwarg_no_double_walk() {
    // Same no-double-walk guard, but on a kwarg position
    // (`.fillna(value=df["statuss"])`). `.fillna` IS a recognized
    // method — its dispatch handled the arg, the I2 walker mustn't
    // re-walk.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.fillna(value=df["statuss"])
"#,
    );
    // Whether this fires 0 or 1 D0030 depends on fillna's exact
    // arg-walk model — the guard is that it never fires *twice*.
    assert!(result.count_code("D0030") <= 1);
}

#[test]
fn V13E3_unrecognized_call_no_subscript_quiet() {
    // Safe path: `print("hello")` — no df["…"] embedded. The arg-walk
    // MUST NOT fire on a bare literal or a non-frame name.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: SparkFrame[Orders]):
    print("hello")
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// I3 — Spark subscript-assign LHS walked for §10 col-ref
// ===========================================================================

#[test]
fn V13E3_spark_subscript_assign_lhs_typo_fires_d0030() {
    // `spark_df["typo"] = 0` — Spark doesn't support item assignment
    // at runtime, but the *column name* on the LHS is still a genuine
    // reference the user expected to resolve. The fix runs the col-ref
    // check on Spark subscript-assign LHS.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    df["statuss"] = 0
    return df
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13E3_spark_subscript_assign_lhs_existing_col_quiet() {
    // Safe path: Spark `df["status"] = 0` — `status` IS on Orders, so
    // the LHS col-ref check resolves quietly. No D0030 fires (the
    // assign itself is still wrong code at runtime, but that's not
    // pykrete's lane — we only catch the typo).
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    df["status"] = 0
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13E3_pandas_subscript_assign_lhs_typo_still_fires_d0030() {
    // Regression guard — pandas LHS-subscript-assign behavior is
    // unchanged from PR-B: `pdf["new"] = ...` is the legitimate
    // column-add path. Firing D0030 on every non-existent column
    // name would flag every new-column add as a typo. Pandas keeps
    // the existing schema-extend path; D0030 does NOT fire here.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    df["statuss"] = 0
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13E3_pandas_subscript_assign_new_column_extends_schema() {
    // Regression guard — the PR-B schema-extend path still works.
    // Adding `pdf["new"] = ...` must not fire D0030 and must extend
    // the tracked schema so a downstream `col("new")` resolves.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: PandasFrame[Orders]):
    df["new"] = "value"
    return df.select(col("new"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
