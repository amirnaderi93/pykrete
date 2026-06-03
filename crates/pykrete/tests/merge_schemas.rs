//! `Merge` derived schemas — `SparkFrame[Merge[OrderInfo, CustomerInfo]]`
//! is a schema with every column of both, the widening counterpart to
//! `Pick` / `Omit`.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

#[test]
fn merge_combines_the_columns_of_two_schemas() {
    let src = "\
class OrderInfo(Schema):
    order_id: int
    amount: int

class CustomerInfo(Schema):
    customer_id: int
    name: string

def f(d: SparkFrame[Merge[OrderInfo, CustomerInfo]]) -> SparkFrame:
    return d.select(col(\"order_id\"), col(\"amount\"), col(\"customer_id\"), col(\"name\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn merge_rejects_a_column_on_neither_schema() {
    let src = "\
class OrderInfo(Schema):
    order_id: int

class CustomerInfo(Schema):
    customer_id: int

def f(d: SparkFrame[Merge[OrderInfo, CustomerInfo]]) -> SparkFrame:
    return d.select(col(\"nonexistent\"))
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn merge_return_type_is_checked_against_the_combined_columns() {
    let src = "\
class A(Schema):
    a: int

class B(Schema):
    b: int

def f(d: SparkFrame[Merge[A, B]]) -> SparkFrame[Merge[A, B]]:
    return d.select(col(\"a\"), col(\"b\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn merge_with_a_shared_column_still_resolves_it() {
    // `city` is on both sides — the merged schema still has it once.
    let src = "\
class L(Schema):
    city: string
    amount: int

class R(Schema):
    city: string
    rating: int

def f(d: SparkFrame[Merge[L, R]]) -> SparkFrame:
    return d.select(col(\"city\"), col(\"amount\"), col(\"rating\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn merge_combines_three_schemas() {
    let src = "\
class A(Schema):
    a: int

class B(Schema):
    b: int

class C(Schema):
    c: int

def f(d: SparkFrame[Merge[A, B, C]]) -> SparkFrame:
    return d.select(col(\"a\"), col(\"b\"), col(\"c\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn merge_with_an_unknown_schema_is_flagged() {
    let src = "\
class A(Schema):
    a: int

def f(d: SparkFrame[Merge[A, Bogus]]) -> SparkFrame:
    return d
";
    assert_has_code(&check(src), "D0020");
}

#[test]
fn merge_composes_with_schema_inheritance() {
    // `Derived` inherits `a` from `Base`; `Merge` sees inherited columns.
    let src = "\
class Base(Schema):
    a: int

class Derived(Base):
    b: int

class Other(Schema):
    c: int

def f(d: SparkFrame[Merge[Derived, Other]]) -> SparkFrame:
    return d.select(col(\"a\"), col(\"b\"), col(\"c\"))
";
    assert_no_diagnostics(&check(src));
}
