//! Column-type-level checking (conservative). pykrete infers the atomic
//! type of each result column and, when a function declares a return
//! schema, checks that columns shared with the body have compatible
//! types — emitting D0080 on a confident mismatch.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check};

const SCHEMAS: &str = "\
class In(Schema):
    amount: int
    city: string
";

#[test]
fn return_type_column_type_mismatch_is_caught() {
    // `amount` is declared `string` in Out, but the body produces the
    // `int` column straight off In.
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: string
    city: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
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
    amount: int
    city: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
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
    amount: string
    city: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
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
    amount: int
    city: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.select(col(\"amount\").cast(\"string\"), col(\"city\"))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn numeric_widening_is_not_flagged() {
    // Body produces `int`, schema declares `long` — both numeric, so
    // the conservative check accepts it (pykrete infers int/long
    // imprecisely).
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: long
    city: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.select(col(\"amount\"), col(\"city\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn unknown_column_type_is_not_flagged() {
    // `total` comes from `F.sum(...)`, whose type pykrete doesn't infer —
    // an unknown type is permissive, never a mismatch.
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: int
    city: string
    total: long

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.withColumn(\"total\", F.sum(\"amount\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn types_survive_a_group_by_agg() {
    // The group key keeps its type through `groupBy(...).agg(...)`.
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    city: int
    total: long

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.groupBy(\"city\").agg(F.sum(\"amount\").alias(\"total\"))
"
    );
    // `city` is a string key, declared `int` in Out.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn types_survive_a_join() {
    let src = "\
class L(Schema):
    id: int
    amount: int

class R(Schema):
    id: int
    label: string

class Out(Schema):
    id: int
    amount: string
    label: string

def f(l: SparkFrame[L], r: SparkFrame[R]) -> SparkFrame[Out]:
    return l.join(r, on=\"id\")
";
    // `amount` comes through the join as `int`, declared `string`.
    assert_has_code(&check(src), "D0080");
}

#[test]
fn types_survive_to_df() {
    let src = "\
class In(Schema):
    a: int
    b: string

class Out(Schema):
    x: string
    y: string

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.toDF(\"x\", \"y\")
";
    // `toDF` renames positionally — `x` takes `a`'s int type.
    assert_has_code(&check(src), "D0080");
}

#[test]
fn types_survive_with_columns() {
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: int
    city: string
    doubled: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.withColumns({{\"doubled\": col(\"amount\")}})
"
    );
    // `doubled` is `col(\"amount\")` — an int — declared `string`.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn nested_struct_field_type_is_resolved_through_the_dotted_path() {
    // `col("address.zipcode")` resolves through the nested `Address`
    // struct; `zipcode` is an int, declared `string` in Out.
    let src = "\
class Address(Schema):
    zipcode: int
    street: string

class Person(Schema):
    name: string
    address: Address

class Out(Schema):
    name: string
    zip: string

def f(p: SparkFrame[Person]) -> SparkFrame[Out]:
    return p.select(col(\"name\"), col(\"address.zipcode\").alias(\"zip\"))
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn nested_struct_field_type_matching_declaration_passes() {
    let src = "\
class Address(Schema):
    zipcode: int
    street: string

class Person(Schema):
    name: string
    address: Address

class Out(Schema):
    name: string
    zip: int

def f(p: SparkFrame[Person]) -> SparkFrame[Out]:
    return p.select(col(\"name\"), col(\"address.zipcode\").alias(\"zip\"))
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn lit_value_type_flows_into_the_return_check() {
    // `withColumn("city", F.lit(1))` overwrites `city` with an int —
    // but Out declares it `string`.
    let src = format!(
        "{SCHEMAS}
class Out(Schema):
    amount: int
    city: string

def f(x: SparkFrame[In]) -> SparkFrame[Out]:
    return x.withColumn(\"city\", F.lit(1))
"
    );
    assert_has_code(&check(&src), "D0080");
}
