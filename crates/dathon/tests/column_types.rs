//! Column-type-level checking (conservative). dathon infers the atomic
//! type of each result column and, when a function declares a return
//! schema, checks that columns shared with the body have compatible
//! types — emitting D0080 on a confident mismatch.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check};

const SCHEMAS: &str = "\
class In(Schema):
    amount: \"int\"
    city: \"string\"
";

#[test]
fn return_type_column_type_mismatch_is_caught() {
    // `amount` is declared `string` in Out, but the body produces the
    // `int` column straight off In.
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: \"string\"
    city: \"string\"

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.select(col(\"amount\"), col(\"city\"))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn matching_column_types_pass() {
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: \"int\"
    city: \"string\"

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.select(col(\"amount\"), col(\"city\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn cast_aligns_the_column_type() {
    // `.cast("string")` makes the body's `amount` a string — matching
    // the declared Out schema.
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: \"string\"
    city: \"string\"

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.select(col(\"amount\").cast(\"string\"), col(\"city\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn cast_to_the_wrong_type_is_caught() {
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: \"int\"
    city: \"string\"

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.select(col(\"amount\").cast(\"string\"), col(\"city\"))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn numeric_widening_is_not_flagged() {
    // Body produces `int`, schema declares `long` — both numeric, so
    // the conservative check accepts it (dathon infers int/long
    // imprecisely).
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: \"long\"
    city: \"string\"

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.select(col(\"amount\"), col(\"city\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn unknown_column_type_is_not_flagged() {
    // `total` comes from `F.sum(...)`, whose type dathon doesn't infer —
    // an unknown type is permissive, never a mismatch.
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: \"int\"
    city: \"string\"
    total: \"long\"

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.withColumn(\"total\", F.sum(\"amount\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn lit_value_type_flows_into_the_return_check() {
    // `withColumn("city", F.lit(1))` overwrites `city` with an int —
    // but Out declares it `string`.
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: \"int\"
    city: \"string\"

def f(x: DataFrame[In]) -> DataFrame[Out]:
    return x.withColumn(\"city\", F.lit(1))
"
    );
    assert_has_code(&check(&src), "D0080");
}
