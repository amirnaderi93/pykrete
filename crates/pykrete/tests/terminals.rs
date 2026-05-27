//! Terminal DataFrame methods — `count`, `collect`, `show`,
//! `printSchema`, `explain`, `first`, `take`, `head`, `tail`. They're
//! typically the last call in a chain and return something other than a
//! DataFrame. Recognized centrally so the chain dies cleanly (no
//! diagnostic about an "unknown method") and so a future
//! "chain-after-terminal" check has a single seam to attach to.

#![allow(non_snake_case)] // PySpark method names leak into test names.

mod common;

use common::{assert_does_not_have_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Orders(Schema):
    place_code: int
    price: int
";

#[test]
fn count_on_a_typed_dataframe_is_recognized() {
    // `df.count()` returns a long, not a DataFrame. The chain dies here;
    // the call itself is a clean, recognized terminal — no diagnostic.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> int:
    return raw.count()
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn collect_show_explain_first_take_head_tail_printSchema_all_recognized() {
    // Each terminal called directly on a typed DataFrame produces no
    // pykrete diagnostic — they're recognized, they just don't keep the
    // DataFrame chain alive.
    for method in &[
        "collect",
        "show",
        "printSchema",
        "explain",
        "first",
        "head",
        "take",
        "tail",
    ] {
        let src = format!(
            "{SCHEMA}
def f(raw: DataFrame[Orders]) -> None:
    raw.{method}()
"
        );
        let result = check(&src);
        // We only assert that no D0030 fires off the terminal itself.
        // The body of the function may not satisfy other diagnostics
        // (e.g. return-type), but that's not what this test guards.
        assert_does_not_have_code(&result, "D0030");
    }
}

#[test]
fn count_result_is_not_a_dataframe() {
    // Confirm `df.count()` is NOT treated as a DataFrame: a subsequent
    // `.select(col("nofield"))` chained off it would, if pykrete still
    // tracked the schema, fire D0030. We expect NO D0030 — the chain
    // died at `.count()`, so anything after it is unmodeled. This
    // documents the deferred behavior; v0.1.16 may instead flag the
    // chained call as a likely bug.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> None:
    raw.count().select(col(\"nofield\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn terminals_do_not_break_argument_checking_on_the_receiver() {
    // `.show(20)` / `.take(5)` take an integer arg. The integer isn't a
    // column-name string, so no D0030 false positive should fire.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Orders]) -> None:
    raw.show(20)
    raw.take(5)
    raw.head(3)
    raw.tail(2)
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0030");
}
