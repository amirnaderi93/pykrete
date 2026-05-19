//! Iteration 36: chain-preserving method allowlist.
//!
//! PySpark has a small family of methods that don't alter the
//! receiver's schema — caching hints (`persist`, `cache`, `unpersist`,
//! `checkpoint`), partitioning hints (`coalesce`, `repartition`,
//! `repartitionByRange`, `hint`), ordering (`sort`, `orderBy`,
//! `sortWithinPartitions`), sampling (`sample`, `limit`, `distinct`),
//! and aliasing (`alias`). Before this iteration, encountering any of
//! these in the middle of a chain broke schema tracking — the
//! analyzer fell through to `None` because the method name wasn't on
//! any allowlist.
//!
//! Now they pass the receiver's `SchemaView` through unchanged, so a
//! chain like `raw.persist().filter(col("x"))` keeps lighting up
//! column-reference checks on every operation.

#![allow(non_snake_case)] // Test names embed diagnostic codes (D0030).

mod common;

use common::check;

#[test]
fn persist_in_the_middle_of_a_chain_does_not_break_tracking() {
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.persist().select(col("place_code"), col("price"))
"#;
    let result = check(src);
    assert!(
        result.diagnostics.is_empty(),
        "expected clean run; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn typo_after_a_persist_now_fires_D0030() {
    // The whole point of the pass-through allowlist: a column typo
    // that appears AFTER a `.persist()` (or any other no-op) used to
    // be invisible. Now it fires the same way it does on a bare chain.
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.persist().select(col("priec"))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0030"),
        "expected D0030; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn stacked_pass_through_methods_compose() {
    // A common real-world pattern: cache + repartition + orderBy chained
    // together for a warmup. Schema tracking must survive every link.
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.cache().repartition(8).orderBy("price").select(col("place_code"), col("price"))
"#;
    let result = check(src);
    assert!(
        result.diagnostics.is_empty(),
        "expected clean run; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn return_type_check_still_fires_through_pass_throughs() {
    // The return statement carries the chain's final schema. If
    // pass-throughs preserve it correctly, a `-> DataFrame[X]`
    // mismatch should still fire.
    let src = r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.cache().select(col("place_code"))
"#;
    let result = check(src);
    assert!(
        result.has_code("D0050"),
        "expected D0050 (place_code-only chain misses `price`); got {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}
