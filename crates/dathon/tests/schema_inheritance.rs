//! Schema inheritance — a `Schema` class can extend another `Schema`
//! class to inherit its columns: `class Premium(Orders): tier: string`.
//! The subclass is itself a schema (recognized in `DataFrame[Premium]`),
//! and its column set is the base's columns plus its own.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

#[test]
fn subclass_inherits_base_columns() {
    let src = "\
class Orders(Schema):
    place_code: int
    price: int

class PricedOrders(Orders):
    discount: int

def f(d: DataFrame[PricedOrders]) -> DataFrame:
    return d.select(col(\"place_code\"), col(\"price\"), col(\"discount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn subclass_still_flags_a_column_on_neither_base_nor_subclass() {
    let src = "\
class Orders(Schema):
    place_code: int

class PricedOrders(Orders):
    discount: int

def f(d: DataFrame[PricedOrders]) -> DataFrame:
    return d.select(col(\"nonexistent\"))
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn return_type_check_counts_inherited_columns() {
    // The body must produce the base's columns plus the subclass's own
    // to satisfy `-> DataFrame[Derived]`.
    let src = "\
class Base(Schema):
    a: int
    b: int

class Derived(Base):
    c: int

def f(d: DataFrame[Derived]) -> DataFrame[Derived]:
    return d.select(col(\"a\"), col(\"b\"), col(\"c\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn return_type_check_fails_when_an_inherited_column_is_dropped() {
    // Dropping the inherited `b` leaves the body short of `Derived`.
    let src = "\
class Base(Schema):
    a: int
    b: int

class Derived(Base):
    c: int

def f(d: DataFrame[Derived]) -> DataFrame[Derived]:
    return d.select(col(\"a\"), col(\"c\"))
";
    assert_has_code(&check(src), "D0050");
}

#[test]
fn inheritance_chains_through_multiple_levels() {
    let src = "\
class L1(Schema):
    a: int

class L2(L1):
    b: int

class L3(L2):
    c: int

def f(d: DataFrame[L3]) -> DataFrame:
    return d.select(col(\"a\"), col(\"b\"), col(\"c\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn a_subclass_can_redeclare_an_inherited_column() {
    // Redeclaring `id` in the subclass must not duplicate it or error —
    // `id` and the inherited `name` both still resolve.
    let src = "\
class Base(Schema):
    id: string
    name: string

class Derived(Base):
    id: int

def f(d: DataFrame[Derived]) -> DataFrame:
    return d.select(col(\"id\"), col(\"name\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn extending_a_non_schema_class_is_not_a_schema() {
    // `Plain` has no `Schema` base, so `Derived` does not inherit
    // schema-ness — `DataFrame[Derived]` is an unknown-schema reference.
    let src = "\
class Plain:
    x: int

class Derived(Plain):
    y: int

def f(d: DataFrame[Derived]) -> DataFrame:
    return d
";
    assert_has_code(&check(src), "D0020");
}

#[test]
fn an_inherited_schema_works_as_a_nested_struct() {
    // A subclass used as a struct-typed field navigates into its
    // inherited columns as well as its own.
    let src = "\
class Base(Schema):
    label: string

class Event(Base):
    id: int

class Log(Schema):
    event: Event

def f(d: DataFrame[Log]) -> DataFrame:
    return d.select(col(\"event.label\"), col(\"event.id\"))
";
    assert_no_diagnostics(&check(src));
}
