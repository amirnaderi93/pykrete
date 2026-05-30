//! Regression coverage for v0.1.28's expanded type vocabulary:
//! `decimal(p, s)`, `byte`, `short`, and `binary`. Spark calls these
//! atomic, pykrete now recognises them everywhere a name resolves to
//! a [`pykrete::types::ColumnType`].

mod common;

use common::{
    assert_does_not_have_code, assert_has_code, assert_message_contains, assert_no_diagnostics,
    check,
};

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
fn cast_to_a_typo_is_flagged_with_d0011() {
    // A typo in the `.cast("...")` target was previously silently
    // accepted — pykrete couldn't pin the result, but never warned
    // either. v0.1.28 surfaces a `D0011` (invalidColumnType) on the
    // string-literal target. The message reproduces the bad target so
    // the user sees what didn't parse.
    let src = "\
class Sale(Schema):
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\").cast(\"decimial(18,2)\").alias(\"amount\"))
";
    let result = check(src);
    assert_has_code(&result, "D0011");
    assert_message_contains(&result, "D0011", "decimial(18,2)");
}

#[test]
fn cast_with_recognized_target_does_not_fire_d0011() {
    // Negative pair for the above — a legitimate cast (parameterized
    // decimal, byte, etc.) must NOT fire D0011, only the typo arm does.
    let src = "\
class Sale(Schema):
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(
        col(\"amount\").cast(\"decimal(18, 2)\").alias(\"d\"),
        col(\"amount\").cast(\"byte\").alias(\"b\"),
    )
";
    assert_does_not_have_code(&check(src), "D0011");
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
    // Positive shape: the whole pipeline is clean (no spurious D-codes).
    // Type pinning that decimal-vs-double don't agree is covered by the
    // unit tests on `aggregate_output_type` and `function_result_type`
    // (which the permissive numeric-numeric `types_compatible` rule
    // would otherwise hide at this integration layer).
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
    assert_no_diagnostics(&check(src));
}

#[test]
fn group_by_sum_of_decimal_flagged_when_declared_non_numeric() {
    // Companion to the above: when the declared output column type is
    // categorically wrong for `sum(decimal)` (string, not numeric), the
    // return-type check DOES surface D0080. This pins that the column
    // type is actually being inferred and threaded through, not silently
    // dropped.
    let src = "\
class Raw(Schema):
    city: string
    amount: decimal(18, 2)

class Totals(Schema):
    city: string
    total: string

def f(raw: DataFrame[Raw]) -> DataFrame[Totals]:
    return raw.groupBy(\"city\").sum(\"amount\").select(col(\"city\"), col(\"sum(amount)\").alias(\"total\"))
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn agg_mean_of_decimal_matches_groupby_mean_shortcut() {
    // The two surfaces for `mean(decimal)` must agree — the previous
    // implementation routed `F.mean(...)` through a fixed-Double branch
    // while `groupBy.mean(...)` kept the decimal, so the same input
    // produced different types depending on which API the user reached
    // for. Both pipelines below must accept a `decimal` output column.
    let shortcut = "\
class Raw(Schema):
    city: string
    amount: decimal(18, 2)

class Avg(Schema):
    city: string
    average: decimal

def f(raw: DataFrame[Raw]) -> DataFrame[Avg]:
    return raw.groupBy(\"city\").mean(\"amount\").select(col(\"city\"), col(\"mean(amount)\").alias(\"average\"))
";
    let agg = "\
class Raw(Schema):
    city: string
    amount: decimal(18, 2)

class Avg(Schema):
    city: string
    average: decimal

def f(raw: DataFrame[Raw]) -> DataFrame[Avg]:
    return raw.groupBy(\"city\").agg(F.mean(col(\"amount\")).alias(\"average\"))
";
    assert_no_diagnostics(&check(shortcut));
    assert_no_diagnostics(&check(agg));
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

#[test]
fn decimal_precision_above_spark_cap_is_rejected() {
    // Spark caps `DECIMAL` precision at 38. pykrete used to silently
    // accept `decimal(39, 0)` (and any precision up to u8::MAX); v0.1.28
    // rejects it on every surface — schema annotations and cast strings.
    let in_schema = "\
class Bad(Schema):
    amount: decimal(39, 0)
";
    assert_has_code(&check(in_schema), "D0011");
    let in_cast = "\
class Sale(Schema):
    amount: int

def f(s: DataFrame[Sale]) -> DataFrame:
    return s.select(col(\"amount\").cast(\"decimal(39, 0)\").alias(\"a\"))
";
    assert_has_code(&check(in_cast), "D0011");
}

#[test]
fn decimal_single_arg_defaults_scale_to_zero() {
    // `decimal(p)` is Spark SQL shorthand for `decimal(p, 0)`. pykrete
    // used to reject the single-arg form outright; v0.1.28 accepts it
    // with scale defaulted to 0 — same behavior as Spark.
    let src = "\
class Order(Schema):
    qty: decimal(10)

def f(o: DataFrame[Order]) -> DataFrame:
    return o.select(col(\"qty\"))
";
    assert_no_diagnostics(&check(src));
}
