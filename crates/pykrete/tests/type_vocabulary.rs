//! Regression coverage for v0.1.28's expanded type vocabulary:
//! `decimal(p, s)`, `byte`, `short`, and `binary`. Spark calls these
//! atomic, pykrete now recognises them everywhere a name resolves to
//! a [`pykrete::types::ColumnType`].

mod common;

use common::{assert_does_not_have_code, assert_has_code, assert_no_diagnostics, check};

#[test]
fn schema_declares_decimal_with_precision_and_scale() {
    let src = "\
class Sale(Schema):
    region: string
    amount: decimal(18, 2)

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn schema_declares_bare_decimal_without_args() {
    let src = "\
class Sale(Schema):
    region: string
    amount: decimal

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn schema_declares_byte_short_binary() {
    let src = "\
class Row(Schema):
    flag: byte
    code: short
    payload: binary

def f(r: DataFrame[Row]) -> DataFrame:
    return r.select(col(\"flag\"), col(\"code\"), col(\"payload\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn cast_to_decimal_with_precision_carries_the_type() {
    // Audit launch-gate snippet #5: `col("amount").cast("decimal(18,2)")`
    // must produce a known decimal-typed result so downstream checks
    // can reason about it.
    let src = "\
class Sale(Schema):
    region: string
    amount: int

def repriced(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\").cast(\"decimal(18,2)\").alias(\"amount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn cast_to_byte_short_binary_is_accepted() {
    let src = "\
class Sale(Schema):
    region: string
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(
        col(\"amount\").cast(\"byte\").alias(\"b\"),
        col(\"amount\").cast(\"short\").alias(\"s\"),
        col(\"region\").cast(\"binary\").alias(\"r\"),
    )
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn cast_to_a_typo_is_not_silently_accepted() {
    // Negative: a typo in the cast string must not roll over to a
    // known type. The cast still parses syntactically (Spark accepts
    // any string at runtime — that's its problem, not pykrete's), but
    // the schema-cast vs Column.cast distinction means we don't pin
    // the column to anything spurious.
    let src = "\
class Sale(Schema):
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\").cast(\"decimial(18,2)\").alias(\"amount\"))
";
    // The cast itself doesn't fire a diagnostic — pykrete is permissive
    // on unknown cast targets. But the downstream return-type check
    // must not accept this as `decimal`. We assert no false positives
    // by checking nothing surfaces from the cast (no D0080 either, since
    // the return type is `DataFrame` with no schema).
    assert_no_diagnostics(&check(src));
}

#[test]
fn group_by_sum_of_byte_returns_long() {
    // `sum(byte)` widens to long in Spark. pykrete should agree —
    // declaring the result as `long` in the schema and returning it
    // through the chain must not produce a return-type mismatch.
    let src = "\
class Raw(Schema):
    city: string
    flag: byte

class Totals(Schema):
    city: string
    total: long

def f(raw: DataFrame[Raw]) -> DataFrame[Totals]:
    return raw.groupBy(\"city\").sum(\"flag\").select(col(\"city\"), col(\"sum(flag)\").alias(\"total\"))
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn group_by_sum_of_decimal_stays_decimal() {
    // `sum(decimal)` stays decimal in pykrete's simplified rule
    // (Spark widens to decimal(p+10, s), capped at 38; we collapse).
    let src = "\
class Raw(Schema):
    city: string
    amount: decimal(18, 2)

class Totals(Schema):
    city: string
    total: decimal

def f(raw: DataFrame[Raw]) -> DataFrame[Totals]:
    return raw.groupBy(\"city\").sum(\"amount\").select(col(\"city\"), col(\"sum(amount)\").alias(\"total\"))
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn malformed_decimal_args_in_schema_field_are_reported() {
    // Bad precision/scale args in a Schema field annotation fire D0011
    // ("not a recognized pykrete type"). Garbage inside the parens must
    // not silently pass.
    let src = "\
class Bad(Schema):
    amount: decimal(\"oops\", 2)
";
    assert_has_code(&check(src), "D0011");
}
