//! Struct column types — declared `Schema` classes compose into
//! `Array[…]` / `Map[…]`, so a column of objects (`Array[Event]`) is a
//! first-class, structurally-checked type.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

#[test]
fn array_of_declared_schema_is_a_valid_type() {
    // Declaring `array<Event>` must not raise an "unknown type" error —
    // `Event` resolves to the declared schema's struct.
    let src = "\
class Event(Schema):
    id: int
    name: string

class In(Schema):
    events: Array[Event]
    amount: int

def f(d: DataFrame[In]) -> DataFrame[In]:
    return d
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn struct_element_mismatch_inside_an_array_is_caught() {
    let src = "\
class Event(Schema):
    id: int

class Other(Schema):
    id: string

class In(Schema):
    events: Array[Event]

class Out(Schema):
    events: Array[Other]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"events\"))
";
    // `events` is `Array[Event]` (a struct with `id: int`); Out declares
    // the struct's `id` as a string.
    assert_has_code(&check(src), "D0080");
}

#[test]
fn matching_struct_element_passes() {
    let src = "\
class Event(Schema):
    id: int
    name: string

class In(Schema):
    events: Array[Event]

class Out(Schema):
    events: Array[Event]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"events\"))
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn nested_struct_field_mismatch_is_caught() {
    // A struct-typed field compared structurally: `rec`'s `label` is a
    // string in In's struct, an int in Out's.
    let src = "\
class RecIn(Schema):
    id: int
    label: string

class RecOut(Schema):
    id: int
    label: int

class In(Schema):
    rec: RecIn

class Out(Schema):
    rec: RecOut

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"rec\"))
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn dotted_access_into_an_array_of_structs_does_not_false_flag() {
    // `events.id` pierces an `Array[Event]` struct — `events` exists,
    // so it must not be flagged.
    let src = "\
class Event(Schema):
    id: int

class In(Schema):
    events: Array[Event]

def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"events.id\"))
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn dotted_access_into_an_atomic_column_is_still_caught() {
    let src = "\
class In(Schema):
    amount: int

def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"amount.foo\"))
";
    // `amount` is an int — `.foo` on it is a genuine mistake.
    assert_has_code(&check(src), "D0030");
}
