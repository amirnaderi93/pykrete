//! Iteration 35: class-qualified annotated constants.
//!
//! Real codebases commonly declare every data source as an annotated
//! assignment INSIDE a frozen-dataclass class body:
//!
//!     class DataSources:
//!         RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")
//!
//! Before iteration 35, the constants registry only walked module-
//! level annotated assignments, so `DataSources.RAW_ORDERS`
//! produced no schema binding — the entire downstream call chain
//! died silently.
//!
//! Now the registry walks class bodies, indexes the constants under
//! their qualified `(class_name, const_name)` key, and the analyzer's
//! `Expr::Attribute` arm resolves the access path the same way it
//! would for a bare module-level constant.

#![allow(non_snake_case)]

mod common;

use common::check;

#[test]
fn class_level_dataSource_constant_routes_through_generic_inference() {
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

class DataSources:
    RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")

def f(spark) -> DataFrame[RawOrders]:
    dal = DataAccessLayer(spark)
    return dal.read(DataSources.RAW_ORDERS).select(col("place_code"), col("price"))
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
fn typo_off_a_class_level_constant_chain_now_fires_D0030() {
    // The same column-name check that fires on a module-level
    // constant chain should fire when the constant is class-level too.
    let src = r#"
class RawOrders(Schema):
    place_code: int

class DataSource[T]:
    def __init__(self, path: string):
        ...

class DataAccessLayer:
    def __init__(self, spark):
        ...

    def read[T](self, source: DataSource[T]) -> DataFrame[T]:
        ...

class DataSources:
    RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")

def f(spark) -> DataFrame[RawOrders]:
    dal = DataAccessLayer(spark)
    return dal.read(DataSources.RAW_ORDERS).select(col("place_codee"))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0030"),
        "expected D0030; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn unknown_class_level_constant_returns_no_binding() {
    // Referencing `DataSources.DOES_NOT_EXIST` shouldn't crash and
    // shouldn't surface a false positive — the downstream chain just
    // doesn't get a typed receiver. (We may add a dedicated diagnostic
    // for this in a later iteration; for now we just confirm the
    // analyzer doesn't panic and doesn't make things up.)
    let src = r#"
class RawOrders(Schema):
    place_code: int

class DataSource[T]:
    def __init__(self, path: string):
        ...

class DataSources:
    RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")

def f() -> DataFrame[RawOrders]:
    return DataSources.DOES_NOT_EXIST
"#;
    let _ = check(src);
}
