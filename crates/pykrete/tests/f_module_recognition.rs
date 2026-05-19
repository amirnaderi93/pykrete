//! Iteration 37: broadened `F.*` column-reference recognition.
//!
//! Before this iteration the COLUMN_REF_FUNCTIONS allowlist held about
//! 18 aggregate names. Real PySpark uses dozens of column-y helpers
//! (date math, string ops, math, null handling, window helpers,
//! arrays). The allowlist now spans roughly 100 entries; every
//! position where a string literal is legal must mean "column name"
//! so we don't introduce false positives on functions that take
//! value-shaped strings (`lit`, `date_format`, `regexp_replace`, …).

mod common;

use common::check;

#[test]
fn F_add_months_string_arg_is_checked_as_column_name() {
    let src = r#"
class Bookings(Schema):
    checkin: date
    place_code: int

def f(raw: DataFrame[Bookings]) -> DataFrame[Bookings]:
    return raw.withColumn("checkin", F.add_months("chekin", 1))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0030"),
        "expected D0030 for `chekin` typo; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn F_lower_string_arg_is_checked_as_column_name() {
    let src = r#"
class Bookings(Schema):
    city: string

def f(raw: DataFrame[Bookings]) -> DataFrame[Bookings]:
    return raw.withColumn("city", F.lower("cityy"))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0030"),
        "expected D0030 for `cityy` typo; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn F_round_string_arg_is_checked_as_column_name() {
    // Round takes (col, int) — the int arg falls through to the
    // default walker; the string arg is the column.
    let src = r#"
class Orders(Schema):
    price: double
    place_code: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.withColumn("price", F.round("priec", 2))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0030"),
        "expected D0030 for `priec` typo; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn F_coalesce_multiple_string_args_all_checked() {
    let src = r#"
class Orders(Schema):
    price: double
    discount_price: double

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.withColumn("price", F.coalesce("price", "discontu_price"))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0030"),
        "expected D0030 for `discontu_price` typo; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn F_col_recognized_alongside_bare_col() {
    // Iteration 19 had `col("x")` (top-level) but not `F.col("x")`.
    // Iteration 37 puts `col` on the F-allowlist so both forms work.
    let src = r#"
class Orders(Schema):
    price: double

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(F.col("priec"))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0030"),
        "expected D0030 for `priec` typo via F.col(); got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn F_lit_value_arg_is_NOT_treated_as_column() {
    // `lit` is the canonical value-string function — explicitly NOT on
    // the allowlist. Passing a literal value that happens to match a
    // non-existent column name should NOT fire a column-ref
    // diagnostic.
    let src = r#"
class Orders(Schema):
    price: double

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.withColumn("price", F.lit("not_a_column"))
"#;
    let result = check(src);
    assert!(
        !result.has_code("D0030"),
        "did not expect D0030 from F.lit(...) value arg; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn F_date_format_format_string_NOT_treated_as_column() {
    // `date_format(col, "format_str")` — the format string isn't a
    // column. Deliberately omitted from the allowlist so we don't
    // misread the format as a column reference.
    let src = r#"
class Bookings(Schema):
    checkin: date

def f(raw: DataFrame[Bookings]) -> DataFrame[Bookings]:
    return raw.withColumn("checkin_str", F.date_format("checkin", "yyyy-MM-dd"))
"#;
    let result = check(src);
    assert!(
        !result.has_code("D0030"),
        "did not expect D0030 from F.date_format's format-string arg; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}
