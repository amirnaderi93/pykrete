//! `createOrReplaceTempView` + `spark.sql("SELECT … FROM view")` —
//! pykrete records a view's schema against its registered name and
//! later resolves column references in `spark.sql(...)` queries that
//! select from it.
//!
//! Scope of this PR (locked at v0.1.13):
//! - **Single-table SELECT only** — no JOINs, no subqueries.
//! - Aliased tables (`FROM v AS x`) resolve via the underlying name.
//! - **Within-file only** — a tempView created in file A is invisible
//!   to file B's `spark.sql(...)`.

#![allow(non_snake_case)]

mod common;

use common::{assert_count, assert_message_contains, assert_no_diagnostics, check};
use pykrete::{CheckResult, check_project};

const SCHEMA: &str = "\
class Orders(Schema):
    region: string
    amount: int
";

fn check_project_pairs(pairs: &[(&str, &str)]) -> Vec<CheckResult> {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(p, s)| ((*p).to_string(), (*s).to_string()))
        .collect();
    let project = check_project(&owned);
    project.files.into_iter().map(|f| f.result).collect()
}

#[test]
fn createOrReplaceTempView_registers_view_for_within_file_resolution() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT region, amount FROM orders_view\")
"
    );
    // Clean: the view's columns match the query, every projection
    // identifier resolves on the registered Orders schema.
    assert_no_diagnostics(&check(&src));
}

#[test]
fn spark_sql_select_columns_validates_against_view_schema() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    result = spark.sql(\"SELECT region, amount FROM orders_view\")
    return result.select(col(\"region\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn spark_sql_select_typo_fires_D0030_against_view() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT regoin FROM orders_view\")
"
    );
    let result = check(&src);
    assert_count(&result, "D0030", 1);
    // The message names the view's schema so the user can tell pykrete
    // resolved the FROM clause against a real schema.
    assert_message_contains(&result, "D0030", "Orders");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn spark_sql_where_clause_idents_checked_against_view() {
    // Clean — WHERE refs `amount` which exists on Orders.
    let clean = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT region FROM orders_view WHERE amount > 0\")
"
    );
    assert_no_diagnostics(&check(&clean));

    // Typo in WHERE — `amout` doesn't exist on Orders. D0030 fires.
    let typo = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT region FROM orders_view WHERE amout > 0\")
"
    );
    let result = check(&typo);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "amout");
}

#[test]
fn spark_sql_unknown_view_falls_back_silently() {
    // No tempView is registered under `unregistered_view`. pykrete
    // falls back to the pre-tempView behavior — infer the result
    // schema from the projection, no column-existence checks.
    let src = format!(
        "{SCHEMA}
def f() -> DataFrame:
    return spark.sql(\"SELECT x FROM unregistered_view\")
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn createOrReplaceTempView_on_opaque_chain_does_not_register() {
    // `spark.read.parquet(...)` returns Unknown — the receiver of
    // `.createOrReplaceTempView(...)` has no known schema, so no
    // registration happens. The subsequent `spark.sql(...)` falls
    // back silently (no D0030 even though "x" wouldn't exist).
    let src = "\
def f(spark) -> DataFrame:
    spark.read.parquet(\"/data/orders\").createOrReplaceTempView(\"v\")
    return spark.sql(\"SELECT x FROM v\")
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn multiple_temp_views_in_same_file() {
    // Two views registered under different names against different
    // schemas. Each subsequent spark.sql resolves against the right
    // view — a column that exists on one schema but not the other
    // only fires when queried against the wrong view.
    let src = "\
class Orders(Schema):
    region: string
    amount: int

class Products(Schema):
    sku: string
    price: int

def f(orders: DataFrame[Orders], products: DataFrame[Products]) -> DataFrame:
    orders.createOrReplaceTempView(\"orders_view\")
    products.createOrReplaceTempView(\"products_view\")
    # region exists on Orders, not Products — clean against orders_view.
    a = spark.sql(\"SELECT region FROM orders_view\")
    # sku exists on Products, not Orders — clean against products_view.
    b = spark.sql(\"SELECT sku FROM products_view\")
    # region against products_view — D0030 fires.
    c = spark.sql(\"SELECT region FROM products_view\")
    return c
";
    let result = check(src);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "Products");
}

#[test]
fn spark_sql_select_star_returns_view_schema_and_supports_followups() {
    // `SELECT *` against a registered view yields the view's full
    // schema, so a downstream `.select(col("region"))` is checked
    // cleanly and a typo on a view column fires D0030 just like a
    // direct `spark.sql("SELECT region FROM …")` chain would.
    let clean = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT * FROM orders_view\").select(col(\"region\"))
"
    );
    assert_no_diagnostics(&check(&clean));

    let typo = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT * FROM orders_view\").select(col(\"regoin\"))
"
    );
    let r = check(&typo);
    assert_count(&r, "D0030", 1);
    assert_message_contains(&r, "D0030", "regoin");
}

#[test]
fn spark_sql_join_or_subquery_falls_back_silently() {
    // Per the v0.1.13 scope cut, multi-table FROM (JOINs, subqueries,
    // qualified `db.table` names) bypass tempView resolution. The
    // query parses, but `single_from_table` returns None, so pykrete
    // degrades to the unknown-result-schema path — no D0030 even when
    // the projection names a column the view doesn't have.
    let with_join = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT nonexistent FROM orders_view JOIN other ON orders_view.amount = other.amount\")
"
    );
    assert!(
        !check(&with_join).has_code("D0030"),
        "JOIN should fall back silently — no D0030 expected"
    );

    let with_subquery = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT nonexistent FROM (SELECT * FROM orders_view) x\")
"
    );
    assert!(
        !check(&with_subquery).has_code("D0030"),
        "subquery FROM should fall back silently — no D0030 expected"
    );

    let qualified = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> DataFrame:
    raw.createOrReplaceTempView(\"orders_view\")
    return spark.sql(\"SELECT nonexistent FROM db.orders_view\")
"
    );
    assert!(
        !check(&qualified).has_code("D0030"),
        "qualified `db.table` FROM should fall back silently — no D0030 expected"
    );
}

#[test]
fn tempview_registered_in_one_function_is_visible_to_another_in_the_same_file() {
    // The file-scoped TempViewRegistry is shared across every typed
    // function in the file. A view registered in `register` is visible
    // to the `spark.sql(...)` call in `consume`, and a typo'd column
    // there still fires D0030 against the originally-registered schema.
    let src = format!(
        "{SCHEMA}
def register(raw: DataFrame[Orders]) -> None:
    raw.createOrReplaceTempView(\"orders_view\")
    return None

def consume() -> DataFrame:
    return spark.sql(\"SELECT regoin FROM orders_view\")
"
    );
    let r = check(&src);
    assert_count(&r, "D0030", 1);
    assert_message_contains(&r, "D0030", "regoin");
    assert_message_contains(&r, "D0030", "Orders");
}

#[test]
fn re_registering_a_view_replaces_the_earlier_schema() {
    // `df1.createOrReplaceTempView("v")` then `df2.createOrReplaceTempView("v")`
    // — the second registration wins (matching Spark's
    // CREATE OR REPLACE semantics). A subsequent `spark.sql` resolves
    // against the second schema, so a column that exists only on the
    // SECOND schema is clean and one that exists only on the FIRST
    // fires D0030.
    let clean_after_replace = "\
class Orders(Schema):
    region: string
    amount: int

class Products(Schema):
    sku: string
    price: int

def f(orders: DataFrame[Orders], products: DataFrame[Products]) -> DataFrame:
    orders.createOrReplaceTempView(\"v\")
    products.createOrReplaceTempView(\"v\")
    # `sku` exists on Products (the second registration), not Orders.
    return spark.sql(\"SELECT sku FROM v\")
";
    assert_no_diagnostics(&check(clean_after_replace));

    let stale_after_replace = "\
class Orders(Schema):
    region: string
    amount: int

class Products(Schema):
    sku: string
    price: int

def f(orders: DataFrame[Orders], products: DataFrame[Products]) -> DataFrame:
    orders.createOrReplaceTempView(\"v\")
    products.createOrReplaceTempView(\"v\")
    # `region` only existed on the FIRST schema (Orders); after the
    # replace it's gone — D0030 fires against Products.
    return spark.sql(\"SELECT region FROM v\")
";
    let r = check(stale_after_replace);
    assert_count(&r, "D0030", 1);
    assert_message_contains(&r, "D0030", "Products");
    assert_message_contains(&r, "D0030", "region");
}

#[test]
fn tempview_is_within_file_only() {
    // File A registers a view; file B references it via spark.sql.
    // Cross-file registration is deliberately not supported, so file
    // B sees the view as unregistered and falls back silently — no
    // D0030 on the bogus `nonexistent` column.
    let results = check_project_pairs(&[
        (
            "a.pyk",
            "\
class Orders(Schema):
    region: string
    amount: int

def register(raw: DataFrame[Orders]) -> None:
    raw.createOrReplaceTempView(\"orders_view\")
    return None
",
        ),
        (
            "b.pyk",
            "\
def consume() -> DataFrame:
    return spark.sql(\"SELECT nonexistent FROM orders_view\")
",
        ),
    ]);
    // File A: clean — the registration call itself doesn't error.
    assert_no_diagnostics(&results[0]);
    // File B: unregistered-view fallback path → no D0030.
    assert!(
        !results[1].has_code("D0030"),
        "cross-file resolution should not happen, but D0030 fired"
    );
}
