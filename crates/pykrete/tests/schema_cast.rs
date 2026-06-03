//! `<chain>.cast(SparkFrame[Schema])` — pykrete's fluent schema-cast.
//! Re-anchors a pipeline whose schema pykrete has lost (after a pivot or
//! another un-modeled op) to an explicit schema, so downstream column
//! checks resume. `Column.cast("int")` is untouched.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
    month: string

class Pivoted(Schema):
    city: string
    Jan: int
    Feb: int
";

#[test]
fn cast_re_anchors_an_unknown_chain() {
    // The groupBy/pivot/sum result is unknowable; the cast names it, and
    // the bad column after it is then caught against `Pivoted`.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.groupBy(\"city\").pivot(\"month\").sum(\"amount\").cast(SparkFrame[Pivoted]).select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn cast_lets_a_valid_downstream_column_resolve() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.groupBy(\"city\").pivot(\"month\").sum(\"amount\").cast(SparkFrame[Pivoted]).select(col(\"Jan\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn cast_overrides_the_receiver_schema() {
    // `raw` is SparkFrame[Raw]; the cast forces `Pivoted` regardless, so
    // `Jan` resolves and `amount` (a Raw column) no longer would.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.cast(SparkFrame[Pivoted]).select(col(\"amount\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn cast_to_an_unknown_schema_is_reported() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.cast(SparkFrame[Bogus])
"
    );
    assert_has_code(&check(&src), "D0020");
}

#[test]
fn column_cast_is_left_untouched() {
    // `col("amount").cast("int")` is the ordinary Column.cast — the
    // string argument is a type name, not a schema, and must not trip
    // the schema-cast path.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Raw]) -> SparkFrame:
    return raw.select(col(\"amount\").cast(\"int\"))
"
    );
    assert_no_diagnostics(&check(&src));
}
