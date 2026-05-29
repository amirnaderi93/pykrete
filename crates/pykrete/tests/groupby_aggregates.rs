//! Schema preservation across the `groupBy.<agg>()` shortcuts —
//! `.count()`, `.sum()`, `.max()`, `.min()`, `.mean()`, `.avg()`.
//!
//! Pre-v0.1.26:
//! - `groupBy(keys).count()` was treated as terminal (same code path as
//!   `df.count()` → long), so a follow-up `.filter(col("count") > 10)`
//!   was unchecked.
//! - `groupBy(keys).sum("c")` (and friends) checked their string args
//!   for D0030 but then returned `None`, killing the chain.
//!
//! Spark's actual output: `{keys..., count}` for `.count()` and
//! `{keys..., sum(c), …}` for `.sum("c", …)` (Spark always names the
//! synthetic columns `<method>(col)`).

#![allow(non_snake_case)]

mod common;
use common::*;

const SCHEMA: &str = "\
class Sale(Schema):
    region: string
    amount: int
    price: double
";

// ---------------------------------------------------------------------------
// .count()
// ---------------------------------------------------------------------------

#[test]
fn groupBy_count_then_filter_against_count_column_is_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").count().filter(col(\"count\") > 10)
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_count_then_select_against_keys_and_count_is_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").count().select(col(\"region\"), col(\"count\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_count_then_select_unknown_column_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").count().select(col(\"nope\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn df_count_remains_terminal() {
    // `df.count()` (not on a GroupedData) is still terminal — returns a
    // long. A chained `.select(...)` after it produces no D0030 (the
    // chain died at count). This guards against accidentally making
    // count non-terminal in every case.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> None:
    raw.count().select(col(\"nofield\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// .sum() / .max() / .min() / .mean() / .avg()
// ---------------------------------------------------------------------------

#[test]
fn groupBy_sum_then_filter_against_sum_column_is_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").sum(\"amount\").filter(col(\"sum(amount)\") > 100)
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_sum_multi_then_select_against_each_synthetic_is_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").sum(\"amount\", \"price\").select(
        col(\"region\"), col(\"sum(amount)\"), col(\"sum(price)\")
    )
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_sum_with_unknown_input_column_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").sum(\"nope\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn groupBy_sum_then_select_unknown_synthetic_fires_D0030() {
    // `sum(amount)` exists; `sum(nope)` does not (it was never produced).
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").sum(\"amount\").select(col(\"sum(nope)\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "sum(nope)");
}

#[test]
fn groupBy_max_then_filter_against_max_column_is_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").max(\"amount\").filter(col(\"max(amount)\") > 50)
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_max_with_unknown_input_column_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").max(\"nope\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn groupBy_min_then_select_synthetic_is_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").min(\"amount\").select(col(\"min(amount)\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_min_with_unknown_input_column_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").min(\"nope\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
}

#[test]
fn groupBy_mean_then_filter_against_avg_synthetic_is_clean() {
    // pyspark names mean's output `avg(col)` regardless of which alias
    // the user wrote — `.mean(...)` and `.avg(...)` are the same call.
    // We use the method name the user wrote (`mean`) for the synthetic.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").mean(\"amount\").filter(col(\"mean(amount)\") > 5)
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_mean_with_unknown_input_column_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").mean(\"nope\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
}

#[test]
fn groupBy_avg_then_select_synthetic_is_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").avg(\"amount\").select(col(\"avg(amount)\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_avg_with_unknown_input_column_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").avg(\"nope\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// Multi-key & nested follow-ups
// ---------------------------------------------------------------------------

#[test]
fn groupBy_multi_keys_count_preserves_all_keys() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\", \"amount\").count().select(
        col(\"region\"), col(\"amount\"), col(\"count\")
    )
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_count_when_user_column_named_count_drops_original() {
    // The schema already has a `count` column; `.count()` produces a new
    // long-typed `count` and Spark's behavior is that the synthetic
    // shadows the original. The chain after the .count() should see ONE
    // `count` field — assert that by referencing `count` and no D0030.
    // (Note: the original `count: string` is gone, so the type is now
    // long, but we only assert "the chain stays clean and doesn't
    // double-report `count`".)
    let src = r#"
class Hits(Schema):
    page: string
    count: string

def f(raw: DataFrame[Hits]) -> DataFrame:
    return raw.groupBy("page").count().select(col("page"), col("count"))
"#;
    let result = check(src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_count_when_grouping_key_is_named_count_keeps_one_field() {
    // `groupBy("count")` would normally add `count` to the result keys;
    // `.count()` then synthesizes a second `count` column. Spark
    // shadows the original — pykrete's `grouped_count_schema` drops the
    // key before pushing the synthetic so the result has exactly one
    // `count` field. (This exercises the retain-then-push path that
    // the previous test couldn't reach, since `count` wasn't a key
    // there.) Selecting `count` and nothing else is fine; selecting
    // an extra column would D0030.
    let src = r#"
class Hits(Schema):
    count: string
    page: string

def f(raw: DataFrame[Hits]) -> DataFrame:
    return raw.groupBy("count").count().select(col("count"))
"#;
    let result = check(src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_count_when_grouping_key_is_named_count_other_columns_are_dropped() {
    // After `raw.groupBy("count").count()` the result is just
    // {count: long} — the user's `page` field is gone, so selecting it
    // must fire D0030.
    let src = r#"
class Hits(Schema):
    count: string
    page: string

def f(raw: DataFrame[Hits]) -> DataFrame:
    return raw.groupBy("count").count().select(col("page"))
"#;
    let result = check(src);
    assert_has_code(&result, "D0030");
}

#[test]
fn groupBy_sum_chained_into_withColumn_is_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").sum(\"amount\").withColumn(
        \"doubled\", col(\"sum(amount)\") * 2
    )
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// Synthetic-name intern-pool boundedness (B2 regression)
// ---------------------------------------------------------------------------

#[test]
fn synthetic_name_pool_is_bounded_under_repeated_analysis() {
    // Pre-fix: each fresh `BodyContext` ran its own dedup set, so
    // re-checking the same source N times leaked N copies of every
    // synthetic name. With the global pool, the second pass is a no-op
    // for the leak set — `(method, column)` pairs are deduped across
    // every analysis call in the process.
    //
    // The assertion is "growth between iteration 1 and iteration N is
    // zero" rather than "pool size equals K" — this stays robust if
    // another test in the suite already populated the pool.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Sale]) -> DataFrame:
    return raw.groupBy(\"region\").sum(\"amount\", \"price\").select(
        col(\"region\"), col(\"sum(amount)\"), col(\"sum(price)\")
    )
"
    );
    // Prime the pool with one pass first so any "first-time" entries
    // from this source land in the snapshot.
    let _ = check(&src);
    let before = pykrete::operations::synthetic_pool_len();
    for _ in 0..1_000 {
        let _ = check(&src);
    }
    let after = pykrete::operations::synthetic_pool_len();
    assert_eq!(
        before, after,
        "synthetic name pool grew from {before} to {after} across 1000 passes — \
         the per-call intern was supposed to be deduped globally",
    );
}
