//! Field-by-field navigation into nested data — `col("events.payload.id")`
//! is checked segment by segment, piercing `array<struct>` as Spark does.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check};

const SCHEMAS: &str = "\
class Line(Schema):
    sku: string
    qty: int

class Order(Schema):
    id: int
    line: Line

class In(Schema):
    orders: Array[Order]
    meta: Map[string, int]
";

#[test]
fn valid_nested_field_through_array_of_structs_resolves() {
    let src = format!(
        "{SCHEMAS}
def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"orders.id\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn typo_in_a_nested_field_is_caught() {
    let src = format!(
        "{SCHEMAS}
def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"orders.nonexistent\"))
"
    );
    // `nonexistent` is not a field of the `Order` struct.
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn deep_nested_path_resolves() {
    let src = format!(
        "{SCHEMAS}
def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"orders.line.sku\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn typo_in_a_deep_nested_field_is_caught() {
    let src = format!(
        "{SCHEMAS}
def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"orders.line.maddeup\"))
"
    );
    // `maddeup` is not a field of the deeply-nested `Line` struct.
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn dotted_access_past_an_atomic_nested_field_is_caught() {
    let src = format!(
        "{SCHEMAS}
def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"orders.id.something\"))
"
    );
    // `orders.id` is an int — `.something` on it is a mistake.
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn map_access_degrades_without_false_flagging() {
    // pykrete doesn't model dotted access into a map — it must not
    // false-flag an existing map column.
    let src = format!(
        "{SCHEMAS}
def f(d: DataFrame[In]) -> DataFrame:
    return d.select(col(\"meta.whatever\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}
