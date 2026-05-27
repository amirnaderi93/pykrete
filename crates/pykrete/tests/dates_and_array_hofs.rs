//! Date / time function column-arg checking and array higher-order
//! function recognition.
//!
//! The date helpers (`F.to_date`, `F.to_timestamp`, `F.date_format`,
//! `F.trunc`, `F.next_day`, `F.from_utc_timestamp`, `F.to_utc_timestamp`,
//! `F.from_unixtime`, `F.unix_timestamp`) all take a column as their
//! FIRST positional arg and a format / timezone string as the second
//! (when present). `F.date_trunc(format, col)` reverses the layout. The
//! tests here lock in: a typo on the column slot fires D0030; the format
//! / timezone string is never treated as a column name.
//!
//! The higher-order array functions (`F.transform`, `F.filter`,
//! `F.aggregate`, `F.exists`, `F.forall`) are recognized at the surface
//! level — return types modeled, column ref in the first slot reached,
//! lambda body inferred best-effort.

#![allow(non_snake_case)]

mod common;
use common::*;

fn with_schema(schema: &str, body: &str) -> String {
    format!(
        r#"
{schema}

def f(raw: DataFrame[In]) -> DataFrame:
{body}
"#,
        body = indent(body, 4),
    )
}

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const TIMESTAMP_SCHEMA: &str = "\
class In(Schema):
    ts_str: string
    ts: timestamp
    dt: date
    epoch: long
";

const ARRAY_SCHEMA: &str = "\
class In(Schema):
    nums: Array[int]
    flag: bool
";

// ---------------------------------------------------------------------------
// Date / time — first-arg column ref checking
// ---------------------------------------------------------------------------

#[test]
fn to_date_column_arg_resolved_against_schema() {
    let src = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("d", F.to_date(col("ts_str"), "yyyy-MM-dd"))"#,
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn to_date_typo_fires_D0030() {
    let src = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("d", F.to_date(col("teststr"), "yyyy-MM-dd"))"#,
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "teststr");
    // Crucially — the format literal "yyyy-MM-dd" must NOT be flagged.
    assert_count(&result, "D0030", 1);
}

#[test]
fn to_timestamp_column_arg_resolved() {
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.to_timestamp(col("ts_str"), "yyyy-MM-dd HH:mm:ss"))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.to_timestamp(col("nope"), "yyyy-MM-dd HH:mm:ss"))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn date_format_column_arg_resolved() {
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("s", F.date_format(col("ts"), "yyyy-MM-dd"))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("s", F.date_format(col("nope"), "yyyy-MM-dd"))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn trunc_column_arg_resolved() {
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.trunc(col("dt"), "month"))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.trunc(col("nope"), "month"))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn next_day_column_arg_resolved() {
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("n", F.next_day(col("dt"), "Sunday"))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("n", F.next_day(col("nope"), "Sunday"))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn from_utc_timestamp_column_arg_resolved() {
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.from_utc_timestamp(col("ts"), "UTC"))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.from_utc_timestamp(col("nope"), "UTC"))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn to_utc_timestamp_column_arg_resolved() {
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.to_utc_timestamp(col("ts"), "America/Los_Angeles"))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.to_utc_timestamp(col("nope"), "America/Los_Angeles"))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn from_unixtime_column_arg_resolved() {
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("s", F.from_unixtime(col("epoch"), "yyyy-MM-dd"))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("s", F.from_unixtime(col("nope"), "yyyy-MM-dd"))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn unix_timestamp_column_arg_resolved() {
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("u", F.unix_timestamp(col("ts_str"), "yyyy-MM-dd HH:mm:ss"))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("u", F.unix_timestamp(col("nope"), "yyyy-MM-dd HH:mm:ss"))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn date_trunc_column_arg_in_position_two_resolved() {
    // `F.date_trunc(format, col)` reverses the usual layout — format
    // first, column second.
    let ok = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.date_trunc("month", col("ts")))"#,
    );
    assert_does_not_have_code(&check(&ok), "D0030");

    let bad = with_schema(
        TIMESTAMP_SCHEMA,
        r#"return raw.withColumn("t", F.date_trunc("month", col("nope")))"#,
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

// ---------------------------------------------------------------------------
// Array higher-order functions
// ---------------------------------------------------------------------------

#[test]
fn f_transform_returns_array_of_lambda_body_type() {
    // `F.transform(col("nums"), lambda x: F.lit(1))` → `array<int>`.
    // Chaining `.getItem(0)` peels the array — pykrete must agree that
    // the chained `.getItem(0)` result is int, so an `int`-typed Out
    // column doesn't fire D0080.
    let src = format!(
        "{ARRAY_SCHEMA}
class Out(Schema):
    first: int

def f(raw: DataFrame[In]) -> DataFrame[Out]:
    return raw.select(F.transform(col(\"nums\"), lambda x: F.lit(1)).getItem(0).alias(\"first\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0080");
}

#[test]
fn f_filter_preserves_element_type() {
    // `F.filter(col("nums"), lambda x: x > 0)` keeps `nums`'s element
    // type — so the result is `array<int>`. With `Out.kept: Array[int]`
    // the schema check passes.
    let src = format!(
        "{ARRAY_SCHEMA}
class Out(Schema):
    kept: Array[int]

def f(raw: DataFrame[In]) -> DataFrame[Out]:
    return raw.select(F.filter(col(\"nums\"), lambda x: F.lit(True)).alias(\"kept\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0080");
}

#[test]
fn f_aggregate_returns_lambda_body_type() {
    // `F.aggregate(col("nums"), F.lit(0), lambda acc, x: F.lit(0))` →
    // the merge lambda's body type (int from F.lit(0)). With a column-ref
    // typo, D0030 fires; the literal accumulator must NOT.
    let ok = format!(
        "{ARRAY_SCHEMA}
class Out(Schema):
    total: int

def f(raw: DataFrame[In]) -> DataFrame[Out]:
    return raw.select(F.aggregate(col(\"nums\"), F.lit(0), lambda acc, x: F.lit(0)).alias(\"total\"))
"
    );
    let result = check(&ok);
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0080");

    // Column-ref typo in the first arg slot still fires D0030.
    let bad = format!(
        "{ARRAY_SCHEMA}
def f(raw: DataFrame[In]) -> DataFrame:
    return raw.select(F.aggregate(col(\"nope\"), F.lit(0), lambda acc, x: F.lit(0)).alias(\"total\"))
"
    );
    let result = check(&bad);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn f_exists_returns_bool() {
    let src = format!(
        "{ARRAY_SCHEMA}
class Out(Schema):
    any_positive: bool

def f(raw: DataFrame[In]) -> DataFrame[Out]:
    return raw.select(F.exists(col(\"nums\"), lambda x: F.lit(True)).alias(\"any_positive\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0080");
}

#[test]
fn f_forall_returns_bool() {
    let src = format!(
        "{ARRAY_SCHEMA}
class Out(Schema):
    all_positive: bool

def f(raw: DataFrame[In]) -> DataFrame[Out]:
    return raw.select(F.forall(col(\"nums\"), lambda x: F.lit(True)).alias(\"all_positive\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0080");
}
