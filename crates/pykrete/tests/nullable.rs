//! Nullable columns — `x: Optional[int]` models Spark's per-column
//! nullable flag. Nullability is transparent to the default-mode
//! checks; the strict mode flags a nullable value declared non-null.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check, check_strict};

#[test]
fn optional_field_type_resolves_and_the_column_is_usable() {
    let src = "\
class Orders(Schema):
    place_code: int
    note: Optional[string]

def f(d: DataFrame[Orders]) -> DataFrame:
    return d.select(col(\"place_code\"), col(\"note\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn optional_works_in_an_inline_schema() {
    let src = "\
def f(d: DataFrame[{n: Optional[int]}]) -> DataFrame:
    return d.select(col(\"n\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn optional_can_wrap_a_collection() {
    let src = "\
class Orders(Schema):
    tags: Optional[Array[string]]

def f(d: DataFrame[Orders]) -> DataFrame:
    return d.select(col(\"tags\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn optional_struct_field_still_navigates() {
    // `Optional[Event]` is still a struct for dotted-path navigation.
    let src = "\
class Event(Schema):
    id: int

class In(Schema):
    event: Optional[Event]

def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"event.id\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn nullable_into_non_nullable_is_silent_in_default_mode() {
    // `In.x` is nullable, `Out.x` is not — Spark's nullable flag is
    // loose, so the default mode does not flag the narrowing.
    let src = "\
class In(Schema):
    x: Optional[int]
    y: int

class Out(Schema):
    x: int
    y: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"x\"), col(\"y\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn nullable_into_non_nullable_is_flagged_under_strict() {
    let src = "\
class In(Schema):
    x: Optional[int]
    y: int

class Out(Schema):
    x: int
    y: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"x\"), col(\"y\"))
";
    assert_has_code(&check_strict(src), "D0083");
}

#[test]
fn non_nullable_into_nullable_is_never_flagged() {
    // The widening direction is always sound — even under strict.
    let src = "\
class In(Schema):
    x: int

class Out(Schema):
    x: Optional[int]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.select(col(\"x\"))
";
    assert_no_diagnostics(&check_strict(src));
}

#[test]
fn optional_fields_do_not_break_the_return_column_set_check() {
    // Dropping `b` is still a D0050 even though both columns are
    // `Optional` — nullability doesn't disturb the column-set check.
    let src = "\
class In(Schema):
    a: Optional[int]
    b: Optional[int]

def f(d: DataFrame[In]) -> DataFrame[In]:
    return d.select(col(\"a\"))
";
    assert_has_code(&check(src), "D0050");
}
