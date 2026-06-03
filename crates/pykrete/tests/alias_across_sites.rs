//! `df.alias("L"); col("L.region")` — the canonical SQL-style alias
//! pattern — must resolve at EVERY column-checking site, not just on a
//! join's on-clause. Round-1 of the v0.1.37 false-positive sweep wired
//! the alias resolver into `check_join_keys` only, so the most common
//! shape of the pattern (post-join `select` / `filter` / `withColumn`
//! / `groupBy` referencing the aliased side) still false-fired D0030.
//! These tests pin the round-2 lift of the alias-resolution helper
//! across every site that goes through `report_column_refs`.

#![allow(non_snake_case)] // Aliases (`L`, `R`) leak into test names.

mod common;
use common::*;

const SALE_SCHEMA: &str = r#"
class Sale(Schema):
    region: string
    amount: int
"#;

#[test]
fn alias_select_aliased_col_does_not_false_flag() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: SparkFrame[Sale]) -> SparkFrame:
    L = raw.alias("L")
    return L.select(col("L.region"), col("L.amount"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn alias_filter_aliased_col_does_not_false_flag() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: SparkFrame[Sale]) -> SparkFrame:
    L = raw.alias("L")
    return L.filter(col("L.amount") > 0)
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn alias_withColumn_aliased_col_does_not_false_flag() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: SparkFrame[Sale]) -> SparkFrame:
    L = raw.alias("L")
    return L.withColumn("doubled", col("L.amount") * 2)
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn alias_groupBy_aliased_col_does_not_false_flag() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: SparkFrame[Sale]) -> SparkFrame:
    L = raw.alias("L")
    return L.groupBy(col("L.region")).count()
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn alias_select_typo_in_suffix_still_fires_d0030() {
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: SparkFrame[Sale]) -> SparkFrame:
    L = raw.alias("L")
    return L.select(col("L.regoin"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn alias_select_unknown_alias_prefix_falls_through_to_nested_struct_resolver() {
    // With aliases in scope, a dotted name whose prefix does NOT match
    // any registered alias must defer to the nested-struct resolver
    // (so legitimate `addr.city`-style accesses survive). When that
    // resolver also can't find the field, D0030 fires through the
    // generic "Column X does not exist" path — not the "alias not in
    // scope" path. Pre-fix this branch hijacked the ref and false-
    // fired on nested-struct accesses; see
    // `nested_struct_accessor_survives_alias_in_scope` below.
    let result = check(&format!(
        r#"{SALE_SCHEMA}

def f(raw: SparkFrame[Sale]) -> SparkFrame:
    L = raw.alias("L")
    return L.select(col("BAD.region"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "BAD");
}

// --- B-R3 regression coverage --------------------------------------
//
// Round 2 of v0.1.37 lifted `try_resolve_alias_ref` across every
// column-checking site so the `df.alias("L"); col("L.x")` pattern would
// resolve uniformly. The lift introduced a regression: the helper's
// "no matching alias prefix" branch unconditionally claimed the ref
// and false-fired D0030 whenever ANY alias was in scope — which
// hijacked legitimate nested-struct accessors like `addr.city`.
// Round 3 narrows the helper to defer in that case. The two tests
// below pin the canonical regression shape and the trickier
// disambiguation case where a struct field NAME collides with an
// alias NAME.

const CUSTOMER_NESTED_SCHEMA: &str = r#"
class Addr(Schema):
    city: string
    zip: string

class Customer(Schema):
    name: string
    addr: Addr

class Order(Schema):
    id: int
    amount: int
"#;

#[test]
fn nested_struct_accessor_survives_alias_in_scope() {
    // `X = orders.alias("X")` registers an alias, but the receiver of
    // the `.select` is `df`, which has an `addr: Addr` nested struct.
    // `col("addr.city")` must resolve via the nested-struct resolver,
    // NOT be hijacked as an unknown-alias D0030.
    let result = check(&format!(
        r#"{CUSTOMER_NESTED_SCHEMA}

def f(df: SparkFrame[Customer], orders: SparkFrame[Order]) -> SparkFrame:
    X = orders.alias("X")
    return df.select(col("addr.city"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

const COLLISION_SCHEMA: &str = r#"
class Inner(Schema):
    y: string

class Outer(Schema):
    X: Inner

class Order(Schema):
    id: int
"#;

#[test]
fn nested_struct_field_name_colliding_with_registered_alias_resolves_on_receiver() {
    // Disambiguation case: a nested-struct field is literally named
    // `X`, AND an unrelated alias `X = orders.alias("X")` is in scope.
    // The receiver is `df: SparkFrame[Outer]`, so `col("X.y")` MUST
    // resolve as the nested-struct accessor on `df` (Outer.X.y), NOT
    // as the aliased `orders` schema (which has no `y` field and
    // would otherwise produce a misleading D0030 about `y`).
    //
    // Decision: at sites where we know the receiver (the central
    // `report_column_refs` path), we do receiver-first disambiguation
    // — if `resolve_path` on the receiver succeeds, we trust that
    // interpretation and never consult the alias table. Only when the
    // receiver can't resolve the path do we fall back to alias
    // routing. This keeps the canonical `df.alias("L"); col("L.x")`
    // pattern working (the alias name `L` isn't a field of the
    // aliased df itself, so the receiver path fails and alias routing
    // takes over) AND avoids hijacking legitimate nested-struct
    // accesses whose prefix happens to collide with an alias name.
    let result = check(&format!(
        r#"{COLLISION_SCHEMA}

def f(df: SparkFrame[Outer], orders: SparkFrame[Order]) -> SparkFrame:
    X = orders.alias("X")
    return df.select(col("X.y"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}
