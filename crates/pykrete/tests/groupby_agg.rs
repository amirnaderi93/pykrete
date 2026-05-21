//! `.groupBy(...).agg(...)` — result-schema inference for grouped
//! aggregation, the trickiest piece of the body checker.
//!
//! Model:
//! - `df.groupBy("k1", "k2")` produces a `SchemaView::Grouped` carrying the
//!   keys plus the underlying schema (needed because agg arguments
//!   reference columns of the original input, not the keys).
//! - `.agg(F.sum("v").alias("total"))` consumes the Grouped state and
//!   produces a `SchemaView::Derived` whose fields are the keys plus
//!   each aggregate argument's output name.
//!
//! `.agg(...)` is also valid on a regular DataFrame (single-group
//! aggregation): no keys, result is just the aliased aggregates.
//!
//! Column references inside agg arguments come from two sources:
//! - `col("X")` — the normal path.
//! - String literals to known aggregate functions like `F.sum("x")` —
//!   recognized for a small list of well-known PySpark aggregate names.

#![allow(non_snake_case)] // PySpark names (groupBy, withColumn) appear in test names.

mod common;
use common::*;

// ===========================================================================
// Happy path
// ===========================================================================

#[test]
fn group_by_then_agg_with_col_ref_is_clean() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.groupBy("place_code").agg(F.sum(col("price")).alias("total"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn group_by_then_agg_with_string_arg_is_clean() {
    // F.sum("price") — string-arg form. Recognized as a column reference
    // to "price" against the underlying Orders schema.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.groupBy("place_code").agg(F.sum("price").alias("total"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn d0030_fires_when_agg_arg_references_an_unknown_column() {
    // "priec" doesn't exist on the underlying schema.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.groupBy("place_code").agg(F.sum("priec").alias("total"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "priec");
}

#[test]
fn d0030_fires_when_group_key_references_an_unknown_column() {
    // The groupBy call itself checks its keys against the receiver, exactly
    // like it did before agg-tracking landed.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.groupBy("not_a_column").agg(F.count("place_code").alias("n"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "not_a_column");
}

// ===========================================================================
// Result-schema inference
// ===========================================================================

#[test]
fn agg_result_schema_is_keys_plus_aliased_aggregates() {
    // Group by place_code, aggregate into sum_price and avg_price.
    // Result: [place_code, sum_price, avg_price]. Subsequent select on
    // those names should succeed.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    grouped = raw.groupBy("place_code").agg(
        F.sum("price").alias("sum_price"),
        F.avg("price").alias("avg_price"),
    )
    return grouped.select(col("place_code"), col("sum_price"), col("avg_price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn agg_result_excludes_columns_that_were_not_aggregated_or_grouped() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    grouped = raw.groupBy("place_code").agg(F.sum("price").alias("total"))
    return grouped.select(col("price"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "price");
    assert_message_contains(&result, "D0030", "place_code"); // in the inferred-schema list
    assert_message_contains(&result, "D0030", "total"); // in the inferred-schema list
}

#[test]
fn agg_with_multiple_keys_produces_keys_in_order() {
    let result = check(
        r#"
class Orders(Schema):
    city: string
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.groupBy("city", "place_code").agg(
        F.sum("price").alias("total")
    ).select(col("city"), col("place_code"), col("total"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn agg_on_a_non_grouped_DataFrame_produces_just_the_aliases() {
    // df.agg(...) without groupBy first — Spark treats this as a single-
    // group aggregation. No keys; result = aliased aggregates.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.agg(F.sum("price").alias("total")).select(col("total"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Return-type validation against agg result
// ===========================================================================

#[test]
fn agg_result_matches_declared_return_schema() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

class Summary(Schema):
    place_code: int
    total: int

def summarize(raw: DataFrame[Orders]) -> DataFrame[Summary]:
    return raw.groupBy("place_code").agg(F.sum("price").alias("total"))
"#,
    );
    assert_does_not_have_code(&result, "D0050");
}

#[test]
fn d0050_fires_when_agg_result_mismatches_declared_return_schema() {
    // Declared Summary has 'place_code, total' but the body produces
    // 'place_code, wrong_name'.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

class Summary(Schema):
    place_code: int
    total: int

def summarize(raw: DataFrame[Orders]) -> DataFrame[Summary]:
    return raw.groupBy("place_code").agg(F.sum("price").alias("wrong_name"))
"#,
    );
    assert_has_code(&result, "D0050");
    assert_message_contains(&result, "D0050", "total");
    assert_message_contains(&result, "D0050", "wrong_name");
}

// ===========================================================================
// Chains that combine groupBy.agg with other operations
// ===========================================================================

#[test]
fn filter_after_groupBy_agg_runs_against_the_agg_result_schema() {
    // The filter is the OUTER call; receiver is the agg result.
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.groupBy("place_code").agg(
        F.sum("price").alias("total")
    ).filter(col("total") > 100)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn filter_after_groupBy_agg_catches_typo_against_the_agg_result_schema() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame:
    return raw.groupBy("place_code").agg(
        F.sum("price").alias("total")
    ).filter(col("missing") > 100)
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "missing");
}

// ===========================================================================
// Lowercase `groupby` alias
//
// PySpark accepts both `df.groupBy(...)` (the documented form) and
// `df.groupby(...)` (a lowercase alias). Real code in the wild — e.g.
// examples/src/main/python/sql/arrow.py — uses lowercase exclusively.
// pykrete used to recognize only the camelCase form; a typo on the
// groupby key with lowercase slipped past silently.
// ===========================================================================

#[test]
fn lowercase_groupby_alias_checks_keys() {
    let result = check(
        r#"
class Sales(Schema):
    region: string
    year: int
    amount: int

def f(df: DataFrame[Sales]) -> None:
    df.groupby("typo").agg(F.sum("amount").alias("total"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

#[test]
fn lowercase_groupby_alias_produces_a_Grouped_view_for_pivot() {
    // Confirm the alias plugs into the same downstream machinery —
    // pivot's argument check should fire just like under groupBy.
    let result = check(
        r#"
class Sales(Schema):
    region: string
    year: int
    amount: int

def f(df: DataFrame[Sales]) -> None:
    df.groupby("year").pivot("typo").sum("amount")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

#[test]
fn lowercase_groupby_alias_plugs_into_shortcut_aggregates() {
    // groupby + GroupedData shortcut (.max here) — the column arg
    // should be checked the same way it is for groupBy.max.
    let result = check(
        r#"
class Sales(Schema):
    region: string
    year: int
    amount: int

def f(df: DataFrame[Sales]) -> None:
    df.groupby("year").max("typo")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

#[test]
fn lowercase_groupby_alias_clean_path_does_not_false_flag() {
    let result = check(
        r#"
class Sales(Schema):
    region: string
    year: int
    amount: int

def f(df: DataFrame[Sales]) -> None:
    df.groupby("region", "year").agg(F.sum("amount").alias("total"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// GroupedData shortcut aggregates — `g.max("col")`, `g.min/sum/mean/avg(...)`
//
// Spark's GroupedData carries shortcut methods that are equivalent to
// `g.agg(F.<method>(col))` but take string args directly. We check those
// args against the underlying schema the same way pivot does, with
// `resolve_path` so dotted nested refs (`"b.c"`) work too.
// ===========================================================================

#[test]
fn groupBy_max_string_arg_is_recognized_against_underlying_schema() {
    let result = check(
        r#"
class KV(Schema):
    key: int
    value: int

def f(df: DataFrame[KV]) -> None:
    df.groupBy("key").max("value")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_max_catches_typo_in_string_arg() {
    let result = check(
        r#"
class KV(Schema):
    key: int
    value: int

def f(df: DataFrame[KV]) -> None:
    df.groupBy("key").max("vlaue")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "vlaue");
    // No did-you-mean assertion: edit distance from "vlaue" to "value" is 2
    // (transposition), past the suggester's threshold. Coverage of the
    // did-you-mean path is in `groupBy_max_catches_typo_in_dotted_nested_path`.
}

#[test]
fn groupBy_min_sum_mean_avg_all_check_string_args() {
    // One typo on each of min, sum, mean, avg. All four should fire D0030.
    for method in &["min", "sum", "mean", "avg"] {
        let src = format!(
            r#"
class KV(Schema):
    key: int
    value: int

def f(df: DataFrame[KV]) -> None:
    df.groupBy("key").{method}("vlaue")
"#
        );
        let result = check(&src);
        assert_has_code(&result, "D0030");
        assert_message_contains(&result, "D0030", "vlaue");
    }
}

#[test]
fn groupBy_max_walks_dotted_nested_paths() {
    // `g.max("b.c")` — same dotted-path resolution as `col("b.c")`.
    let result = check(
        r#"
class C(Schema):
    c: int

class AB(Schema):
    a: string
    b: C

def f(df: DataFrame[AB]) -> None:
    df.groupBy("a").max("b.c")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn groupBy_max_catches_typo_in_dotted_nested_path() {
    // `g.max("b.cc")` — typo on the nested-field name. The error message
    // names the nested schema, not the top-level one.
    let result = check(
        r#"
class C(Schema):
    c: int

class AB(Schema):
    a: string
    b: C

def f(df: DataFrame[AB]) -> None:
    df.groupBy("a").max("b.cc")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "cc");
    assert_message_contains(&result, "D0030", "C"); // schema name in "on ... C"
}

#[test]
fn groupBy_max_on_a_non_grouped_DataFrame_falls_through() {
    // `df.max("col")` without a groupBy first — receiver is not Grouped,
    // so the shortcut handler doesn't fire. (Spark itself would actually
    // error on this at runtime; we just don't claim to check it here.)
    // No false positive.
    let result = check(
        r#"
class KV(Schema):
    key: int
    value: int

def f(df: DataFrame[KV]) -> None:
    df.max("vlaue")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn group_by_an_attribute_access_key_keeps_it_in_the_result_schema() {
    // `groupBy(df.key, ...)` — an attribute-access grouping key, a column
    // carried in from a joined DataFrame, must land in the post-agg
    // schema exactly like a string key. A common real-world pattern;
    // before the fix the key was dropped and the return-type check
    // false-flagged it.
    let result = check(
        r#"
class Left(Schema):
    city: string
    amount: int

class Seg(Schema):
    city: string
    segment_id: int

class Out(Schema):
    segment_id: int
    total: long

def f(left: DataFrame[Left], seg: DataFrame[Seg]) -> DataFrame[Out]:
    return left.join(seg, "city", how="inner").groupBy(seg.segment_id).agg(
        F.sum("amount").alias("total")
    )
"#,
    );
    assert_no_diagnostics(&result);
}
