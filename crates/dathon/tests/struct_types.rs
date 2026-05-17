//! Struct column types — declared `Schema` classes and inline
//! `struct<…>` compose into `array` / `map`, so a column of objects
//! (`array<Event>`) is a first-class, structurally-checked type.

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

#[test]
fn array_of_declared_schema_is_a_valid_type() {
    // Declaring `array<Event>` must not raise an "unknown type" error —
    // `Event` resolves to the declared schema's struct.
    let src = "\
class Event(Schema):
    id: \"int\"
    name: \"string\"

class In(Schema):
    events: \"array<Event>\"
    amount: \"int\"

def f(d: DataFrame[In]) -> DataFrame[In]:
    return d
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn struct_element_mismatch_inside_an_array_is_caught() {
    let src = "\
class Event(Schema):
    id: \"int\"

class Other(Schema):
    id: \"string\"

class In(Schema):
    events: \"array<Event>\"

class Out(Schema):
    events: \"array<Other>\"

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"events\"))
";
    // `events` is `array<struct<id:int>>`; Out declares the struct's
    // `id` as a string.
    assert_has_code(&check(src), "D0080");
}

#[test]
fn matching_struct_element_passes() {
    let src = "\
class Event(Schema):
    id: \"int\"
    name: \"string\"

class In(Schema):
    events: \"array<Event>\"

class Out(Schema):
    events: \"array<Event>\"

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"events\"))
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn inline_struct_type_is_accepted_and_checked() {
    let src = "\
class In(Schema):
    rec: \"struct<id: int, label: string>\"

class Out(Schema):
    rec: \"struct<id: int, label: int>\"

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"rec\"))
";
    // The inline struct's `label` field is string in In, int in Out.
    assert_has_code(&check(src), "D0080");
}

#[test]
fn dotted_access_into_an_array_of_structs_does_not_false_flag() {
    // `events.id` pierces an `array<struct>` — dathon can't yet verify
    // the nested field, but `events` exists, so it must not be flagged.
    let src = "\
class Event(Schema):
    id: \"int\"

class In(Schema):
    events: \"array<Event>\"

def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"events.id\"))
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn dotted_access_into_an_atomic_column_is_still_caught() {
    let src = "\
class In(Schema):
    amount: \"int\"

def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"amount.foo\"))
";
    // `amount` is an int — `.foo` on it is a genuine mistake.
    assert_has_code(&check(src), "D0030");
}
