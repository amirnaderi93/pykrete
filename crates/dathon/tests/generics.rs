//! Generic-function inference. The motivating use case is the DataSource
//! pattern, where a `dal.read(SOURCE_CONST)` call should infer
//! `DataFrame[Schema]` from the type of `SOURCE_CONST` flowing through a
//! generic method like `def read[T](source: G[T]) -> DataFrame[T]`.
//!
//! What dathon recognizes in v0.1:
//!
//! - Top-level annotated constants of the form `NAME: GenericClass[Schema] = …`.
//!   These bind `NAME` as a value carrying that schema, regardless of the
//!   outer generic class name. Used by both the simple-pattern path (direct
//!   use of the constant) and the generic-inference path (passing the
//!   constant to a generic method).
//! - Non-Schema classes (e.g. `DataAccessLayer`) are discovered and their
//!   method signatures captured. Function parameters typed as a known class
//!   bind that parameter as a class instance.
//! - When the body has `instance.method(args)` and `method` is generic with
//!   one type parameter T appearing in both a parameter slot of shape
//!   `Generic[T]` and the return slot `Generic[T]`, dathon binds T from
//!   the matching argument's schema and substitutes through the return.

#![allow(non_snake_case)] // DataFrame, DataSource, etc. appear in test names.

mod common;
use common::*;

/// A standard preamble for the dal-read pattern: a `RawOrders` schema, a
/// generic `DataSource[T]` class, and a `DataAccessLayer` with a generic
/// `read` method. The test then writes a body inside `f` to exercise it.
fn with_dal(body: &str) -> String {
    format!(
        r#"
class RawOrders(Schema):
    place_code: int
    price: int

class DataSource[T]:
    def __init__(self, path):
        pass

class DataAccessLayer:
    def read[T](self, source: DataSource[T]) -> DataFrame[T]:
        pass

RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")

def f(dal: DataAccessLayer) -> DataFrame[RawOrders]:
{indented}
"#,
        indented = indent(body, 4),
    )
}

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ===========================================================================
// Direct constant resolution — RAW_ORDERS used as a DataFrame-carrying value
// ===========================================================================

#[test]
fn top_level_typed_constant_resolves_to_its_inner_schema() {
    // RAW_ORDERS: DataSource[RawOrders] — when referenced as a value, it
    // resolves to the RawOrders schema. (We treat any GenericClass[Schema]
    // typed constant as schema-carrying; the outer class name is decorative.)
    //
    // Here we use it as the receiver of .select(...) — weird in practice,
    // but a clean test of constant resolution.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

RAW_ORDERS: DataSource[Orders] = DataSource("/path")

def f() -> DataFrame[Orders]:
    return RAW_ORDERS.select(col("place_code"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn d0030_fires_on_unknown_column_against_a_constant_resolved_schema() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

RAW_ORDERS: DataSource[Orders] = DataSource("/path")

def f() -> DataFrame[Orders]:
    return RAW_ORDERS.select(col("nope"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

// ===========================================================================
// Full generic-method substitution — dal.read(SOURCE) pattern
// ===========================================================================

#[test]
fn dal_read_of_typed_constant_returns_dataframe_of_the_constants_schema() {
    // The headline test. `dal.read(RAW_ORDERS)` should infer
    // `DataFrame[RawOrders]` end-to-end. Returning that value directly
    // satisfies the function's declared `-> DataFrame[RawOrders]` return
    // — no D0030, no D0050.
    let result = check(&with_dal(
        r#"
return dal.read(RAW_ORDERS)
"#,
    ));
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn dal_read_followed_by_select_of_all_fields_satisfies_the_declared_return() {
    // Chained `.select(col("place_code"), col("price"))` keeps the full
    // RawOrders shape. No D0030 (both columns exist) and no D0050 (the
    // body's final schema still matches RawOrders).
    let result = check(&with_dal(
        r#"
return dal.read(RAW_ORDERS).select(col("place_code"), col("price"))
"#,
    ));
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn dal_read_inferred_schema_catches_a_downstream_typo() {
    // Same call, but the downstream select uses a column that ISN'T in
    // RawOrders. The diagnostic should reference RawOrders specifically,
    // proving the generic substitution actually wired the schema through.
    let result = check(&with_dal(
        r#"
return dal.read(RAW_ORDERS).select(col("madeup"))
"#,
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "madeup");
    assert_message_contains(&result, "D0030", "RawOrders");
}

#[test]
fn dal_read_result_can_be_bound_to_a_local_and_used_downstream() {
    let result = check(&with_dal(
        r#"
raw = dal.read(RAW_ORDERS)
return raw.filter(col("price") > 0).select(col("place_code"), col("price"))
"#,
    ));
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn dal_read_result_with_wrong_declared_return_fires_D0050() {
    // Function declares `-> DataFrame[Other]` but the inferred result of
    // `dal.read(RAW_ORDERS)` is DataFrame[RawOrders]. Mismatch.
    let result = check(
        r#"
class RawOrders(Schema):
    place_code: int
    price: int

class Other(Schema):
    x: int

class DataSource[T]:
    def __init__(self, path): pass

class DataAccessLayer:
    def read[T](self, source: DataSource[T]) -> DataFrame[T]: pass

RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")

def f(dal: DataAccessLayer) -> DataFrame[Other]:
    return dal.read(RAW_ORDERS)
"#,
    );
    assert_has_code(&result, "D0050");
}

// ===========================================================================
// Negative space: things that shouldn't be confused with constants
// ===========================================================================

#[test]
fn untyped_top_level_assignment_is_not_treated_as_a_schema_carrier() {
    // No annotation on the assignment — dathon doesn't record it as a
    // typed constant, and the body's `whatever.select(...)` returns no
    // schema info (silent pass-through).
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

whatever = some_loader()

def f() -> DataFrame[Orders]:
    return whatever.select(col("place_code"))
"#,
    );
    // The receiver `whatever` is unbound; analyze_expr returns None, and
    // the select call goes unchecked. No D0030 either way.
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn non_generic_method_call_on_class_instance_returns_no_inference() {
    // `dal.read` here is a non-generic method (no [T] type params); we
    // don't infer its return type at all. The body silently passes.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

class DataAccessLayer:
    def read(self, path: str) -> DataFrame:
        pass

def f(dal: DataAccessLayer) -> DataFrame[Orders]:
    return dal.read("/some/path")
"#,
    );
    // No D0050 fires because the actual return type is unknown — we don't
    // infer non-generic methods.
    assert_does_not_have_code(&result, "D0050");
}
