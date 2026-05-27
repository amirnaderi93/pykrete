//! `F.broadcast(df)` is a join hint that tells Spark to broadcast the
//! dataframe to every executor. The schema is unchanged — it's a pure
//! pass-through. Before this iteration the chain died at
//! `F.broadcast(...)` because pykrete didn't recognize the call.

#![allow(non_snake_case)] // `F.broadcast` leaks into test names.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_message_contains, check};

#[test]
fn F_broadcast_preserves_schema_for_downstream_select() {
    // `F.broadcast(df).select(col("x"))` — the chain should keep the
    // receiver's schema and resolve `x` against it.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return F.broadcast(raw).select(col("place_code"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn F_broadcast_catches_typo_in_downstream_select() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return F.broadcast(raw).select(col("plcae_code"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "plcae_code");
}

#[test]
fn F_broadcast_in_join_still_checks_keys() {
    // `df1.join(F.broadcast(df2), "k")` — the broadcast wrap is
    // transparent; the join key check fires off the unwrapped schema.
    let result = check(
        r#"
class A(Schema):
    k: int
    a: int

class B(Schema):
    k: int
    b: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(F.broadcast(b), on="k")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0060");
}

#[test]
fn F_broadcast_in_join_with_missing_key_still_fires_D0060() {
    let result = check(
        r#"
class A(Schema):
    k: int
    a: int

class B(Schema):
    b: int

def f(a: DataFrame[A], b: DataFrame[B]) -> DataFrame:
    return a.join(F.broadcast(b), on="k")
"#,
    );
    assert_has_code(&result, "D0060");
}

#[test]
fn bare_broadcast_without_F_module_also_works() {
    // `from pyspark.sql.functions import broadcast` is common; the bare
    // `broadcast(df)` form lands at `analyze_method_call` with an
    // arbitrary receiver — pykrete recognizes it by method name.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return functions.broadcast(raw).select(col("place_code"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
