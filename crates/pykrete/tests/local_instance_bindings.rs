//! Iteration 34: class-instance bindings discovered from local
//! assignments.
//!
//! Before this iteration, the canonical data-access pattern
//!
//!     data_access = DataAccessLayer(spark)
//!     df = data_access.read(SOME_SOURCE)
//!
//! lost all type information after the second line — `data_access` was
//! only ever populated as an instance binding when it appeared as a
//! function parameter. Now we also recognize the `name = ClassName(...)`
//! call form and the `name: ClassName = ...` annotated form, so the
//! generic-inference path picks up where the user left off.

#![allow(non_snake_case)]

mod common;

use common::check;

#[test]
fn x_eq_ClassName_call_lets_method_chains_keep_their_type() {
    let src = r#"
class RawOrders(Schema):
    place_code: int
    price: int

class DataSource[T]:
    def __init__(self, path: string):
        ...

class DataAccessLayer:
    def __init__(self, spark):
        ...

    def read[T](self, source: DataSource[T]) -> DataFrame[T]:
        ...

RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")

def f(spark) -> DataFrame[RawOrders]:
    dal = DataAccessLayer(spark)
    return dal.read(RAW_ORDERS).select(col("place_code"), col("price"))
"#;
    let result = check(src);
    assert!(
        result.diagnostics.is_empty(),
        "expected clean run; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn typo_after_local_dal_assignment_now_fires_D0030() {
    // The smoking-gun bug this iteration targets: with iteration 34
    // landed, a typo'd column reference on a `data_access = DAL(spark)`
    // → `read(...)` chain surfaces, where it used to slide silently.
    let src = r#"
class RawOrders(Schema):
    place_code: int
    price: int

class DataSource[T]:
    def __init__(self, path: string):
        ...

class DataAccessLayer:
    def __init__(self, spark):
        ...

    def read[T](self, source: DataSource[T]) -> DataFrame[T]:
        ...

RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")

def f(spark) -> DataFrame[RawOrders]:
    dal = DataAccessLayer(spark)
    return dal.read(RAW_ORDERS).select(col("priec"))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0030"),
        "expected D0030 for typo'd column; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn annotated_local_instance_assignment_also_binds() {
    // `dal: DataAccessLayer = DataAccessLayer(spark)` — annotation
    // drives the instance binding even if the RHS happens to not be
    // an obvious constructor call.
    let src = r#"
class RawOrders(Schema):
    x: int

class DataSource[T]:
    def __init__(self, path: string):
        ...

class DataAccessLayer:
    def __init__(self, spark):
        ...

    def read[T](self, source: DataSource[T]) -> DataFrame[T]:
        ...

RAW: DataSource[RawOrders] = DataSource("/path")

def f(spark) -> DataFrame[RawOrders]:
    dal: DataAccessLayer = DataAccessLayer(spark)
    return dal.read(RAW)
"#;
    let result = check(src);
    assert!(
        result.diagnostics.is_empty(),
        "expected clean run; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}
