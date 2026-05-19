//! `Pick` / `Omit` derived schemas — TypeScript-style schema operators.
//! `DataFrame[Pick[Orders, "a", "b"]]` is `Orders` narrowed to those two
//! columns; `DataFrame[Omit[Orders, "x"]]` is `Orders` without `x`.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

#[test]
fn pick_param_resolves_only_the_picked_columns() {
    let src = "\
class Orders(Schema):
    place_code: int
    price: int
    log_date: timestamp

def f(d: DataFrame[Pick[Orders, \"place_code\", \"price\"]]) -> DataFrame:
    return d.select(col(\"place_code\"), col(\"price\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn pick_param_rejects_a_column_that_was_not_picked() {
    // `log_date` exists on Orders but isn't in the picked set.
    let src = "\
class Orders(Schema):
    place_code: int
    price: int
    log_date: timestamp

def f(d: DataFrame[Pick[Orders, \"place_code\", \"price\"]]) -> DataFrame:
    return d.select(col(\"log_date\"))
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn omit_param_drops_the_omitted_column() {
    let src = "\
class Orders(Schema):
    a: int
    b: int
    c: int

def f(d: DataFrame[Omit[Orders, \"c\"]]) -> DataFrame:
    return d.select(col(\"c\"))
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn omit_param_keeps_every_other_column() {
    let src = "\
class Orders(Schema):
    a: int
    b: int
    c: int

def f(d: DataFrame[Omit[Orders, \"c\"]]) -> DataFrame:
    return d.select(col(\"a\"), col(\"b\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn pick_return_type_is_checked_against_the_picked_set() {
    let src = "\
class Orders(Schema):
    a: int
    b: int
    c: int

def f(d: DataFrame[Orders]) -> DataFrame[Pick[Orders, \"a\", \"b\"]]:
    return d.select(col(\"a\"), col(\"b\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn pick_return_type_mismatch_is_flagged() {
    // Body produces [a, c]; the declared return is [a, b].
    let src = "\
class Orders(Schema):
    a: int
    b: int
    c: int

def f(d: DataFrame[Orders]) -> DataFrame[Pick[Orders, \"a\", \"b\"]]:
    return d.select(col(\"a\"), col(\"c\"))
";
    assert_has_code(&check(src), "D0050");
}

#[test]
fn omit_return_type_is_checked() {
    let src = "\
class Orders(Schema):
    a: int
    b: int
    c: int

def f(d: DataFrame[Orders]) -> DataFrame[Omit[Orders, \"c\"]]:
    return d.select(col(\"a\"), col(\"b\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn a_picked_column_that_is_not_on_the_base_schema_is_flagged() {
    let src = "\
class Orders(Schema):
    price: int

def f(d: DataFrame[Pick[Orders, \"priec\"]]) -> DataFrame:
    return d
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn an_unknown_base_schema_in_pick_is_flagged() {
    let src = "\
def f(d: DataFrame[Pick[Bogus, \"a\"]]) -> DataFrame:
    return d
";
    assert_has_code(&check(src), "D0020");
}

#[test]
fn pick_sees_inherited_columns() {
    // `Pick` of a subclass can name columns inherited from its base.
    let src = "\
class Base(Schema):
    a: int
    b: int

class Derived(Base):
    c: int

def f(d: DataFrame[Pick[Derived, \"a\", \"c\"]]) -> DataFrame:
    return d.select(col(\"a\"), col(\"c\"))
";
    assert_no_diagnostics(&check(src));
}
