//! v1.14 PR-D2 — `pdf.groupby(keys).agg("<aggfunc>")` narrow-arm
//! aggregate-semantics-informed schema inference. **FIRST observable
//! `groupby.agg` schema inference in pykrete; canonical consumer of the
//! v1.13 §13 aggregate-semantics convention.**
//!
//! Spec: `docs/design/v1.14-spec.md` §1.iii.
//!
//! Behavior contract: when the receiver is `PandasFrame[X]` (so
//! `SchemaView::Grouped { dialect: Pandas, ... }`) AND the first
//! positional arg to `.agg(...)` is a single string literal on the
//! v1.13 allowlist, the result `SchemaView` is synthesized as
//! `keys ++ (underlying \ keys)` with each non-key column's dtype
//! overridden per the v1.13 §5.1.1 aggregate-to-dtype table:
//!
//! - `"count"` / `"nunique"`              → Long
//! - `"mean"` / `"std"` / `"var"` /
//!   `"median"`                            → Double
//! - `"sum"` / `"min"` / `"max"` /
//!   `"first"` / `"last"`                  → preserve receiver column type
//!
//! Spark-side `sdf.groupBy("k").agg(F.sum("x"))` flows through the
//! pre-v1.14 column-expression arm unchanged — the new arm is gated on
//! `dialect == Pandas`.
//!
//! Observation surface: declare the return-type schema with a dtype
//! that crosses the numeric/non-numeric boundary (D0080 will fire on
//! mismatch). Positive tests assert no D0080 when Out matches the
//! synthesized type; negative tests assert D0080 on mismatch.

#![allow(non_snake_case)]

mod common;
use common::*;

const IN_BASE: &str = "\
class In(Schema):
    k: string
    amount: double
    count_col: int
";

// ===========================================================================
// 11 positive tests — one per allowlist string. Each declares Out with
// the EXPECTED synthesized dtype on the value column; D0080 must NOT
// fire.
// ===========================================================================

#[test]
fn V114D2_count_synthesizes_long() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: long
    count_col: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('count')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_nunique_synthesizes_long() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: long
    count_col: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('nunique')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_mean_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('mean')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_std_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('std')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_var_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('var')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_median_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('median')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_sum_preserves_receiver_types() {
    // `amount: double` stays double; `count_col: int` stays int.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('sum')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_min_preserves_receiver_types() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('min')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_max_preserves_receiver_types() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('max')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_first_preserves_receiver_types() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('first')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_last_preserves_receiver_types() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('last')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ===========================================================================
// Multi-key + dtype-cross D0080 fires
// ===========================================================================

#[test]
fn V114D2_multi_string_keys_synthesized_in_order() {
    let src = "\
class In(Schema):
    k1: string
    k2: int
    amount: double

class Out(Schema):
    k1: string
    k2: int
    amount: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby(['k1', 'k2']).agg('count')
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn V114D2_mean_declared_as_string_fires_D0080() {
    // mean overrides amount: double → double; declaring it string crosses
    // the numeric/non-numeric boundary → D0080 fires.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string
    count_col: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('mean')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V114D2_count_declared_as_string_fires_D0080() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string
    count_col: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('count')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V114D2_sum_preserve_string_recv_declared_as_long_fires_D0080() {
    // amount is double; sum preserves → expected double. Declaring it as
    // string crosses the boundary.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string
    count_col: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('sum')
"
    );
    assert_has_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// Positive synthesis proof — one D0080-fire per dtype-override family that
// previously only had a single representative in the negative cohort. Pairs
// with the `_synthesizes_long` / `_synthesizes_double` / `_preserves_*`
// positive tests above to lock in that each aggfunc's synthesis arm is
// actually wired (a no-op `None` synthesizer would silently pass the
// positive tests).
// ---------------------------------------------------------------------------

#[test]
fn V114D2_nunique_declared_as_string_fires_D0080() {
    // nunique → Long override (sibling to count). Declaring amount as
    // string crosses the boundary; D0080 must fire to prove synthesis.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string
    count_col: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('nunique')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V114D2_std_declared_as_string_fires_D0080() {
    // std → Double override (sibling to mean).
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string
    count_col: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('std')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V114D2_min_declared_as_string_fires_D0080() {
    // min → Preserve (sibling to sum). amount: double preserved as double;
    // declaring it as string crosses the numeric/non-numeric boundary.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string
    count_col: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('min')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V114D2_multi_key_count_declared_as_string_fires_D0080() {
    // Multi-key synthesis path proof: keys k1, k2 are emitted first, then
    // amount is overridden to Long by count. Declaring amount: string
    // crosses the boundary — D0080 fires on the multi-key path.
    let src = "\
class In(Schema):
    k1: string
    k2: int
    amount: double

class Out(Schema):
    k1: string
    k2: int
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby(['k1', 'k2']).agg('count')
";
    assert_has_code(&check(src), "D0080");
}

// ===========================================================================
// Negative fall-through tests — Unknown result, NOT silently-wrong schema.
// Per spec §1.iii.3, dict-aggfunc, callable-aggfunc, list-of-aggfunc,
// and out-of-allowlist string all fall through. The observation is:
// the chain's result schema is Unknown, so downstream column-ref typos
// don't fire D0080 (the receiver schema isn't tied to a Schema class
// the checker can validate against).
// ===========================================================================

#[test]
fn V114D2_out_of_allowlist_string_falls_through_to_unknown() {
    // 'wat' is not on the allowlist → fall-through to Unknown. With
    // Unknown, downstream column-ref checks on `amount` against the
    // declared `Out` schema can't be made; D0080 must NOT fire (because
    // the agg result isn't a synthesized schema to compare).
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('wat')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_list_of_aggfunc_falls_through_to_unknown() {
    // CRITICAL guard per v1.13 PR-D2 R2 Q2 catch: list-of-aggfunc must
    // NOT silently produce a flat schema. MultiIndex-on-columns is
    // out-of-scope per spec §1.iii.3 — fall through to Unknown.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string
    count_col: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg(['sum', 'mean'])
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// NOTE: dict-aggfunc (`.agg({"c": "sum"})`) and callable-aggfunc
// (`.agg(np.mean)`) NO LONGER fall through as of v1.16 PR-D2 — they
// synthesize a Derived schema (dict keeps only the named columns at their
// per-column dtype; callable keeps all non-key columns at Unknown dtype).
// Their coverage lives in `v116_pr_d2_groupby_dict_callable_inplace.rs`.
// list-of-aggfunc (above) remains a fall-through (MultiIndex columns,
// deferred to v1.17).

// ---------------------------------------------------------------------------
// Spec §1.iii.6 asymmetric dialect-crossover defense — pandas dialect with
// a Spark-shape arg (column-expression) and Spark dialect with a pandas-
// shape arg (literal string) must BOTH fall through to Unknown rather than
// silently producing a synthesized schema. v1.5 retro rule 4 / v1.6 rule 8.
// ---------------------------------------------------------------------------

#[test]
fn V114D2_pandas_dialect_with_spark_shape_arg_falls_through() {
    // Pandas-grouped receiver but the arg is a Spark-style column
    // expression, not an allowlist literal string. Must fall through to
    // Unknown — D0080 must NOT fire on the declared Out schema.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    k: string
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg(F.sum(col('amount')))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V114D2_spark_dialect_with_pandas_shape_arg_falls_through() {
    // Spark-grouped receiver with a pandas-style string-literal aggfunc.
    // The new pandas arm is gated on `dialect == Pandas`; the Spark
    // dialect path does NOT consume string-literal args, so this must
    // fall through (no D0080 on the declared Out schema) and not invoke
    // the new synthesis.
    let src = "\
class Orders(Schema):
    place_code: int
    price: int

class Out(Schema):
    place_code: int
    price: string

def f(raw: SparkFrame[Orders]) -> SparkFrame[Out]:
    return raw.groupBy('place_code').agg('sum')
";
    assert_does_not_have_code(&check(src), "D0080");
}

// ===========================================================================
// Spark-side regression tests — the v1.0 column-expression arm must
// remain unchanged. Spark `.groupBy(col).agg(F.sum("x"))` flows through
// the pre-v1.14 path because the Grouped variant carries
// `dialect: Spark`.
// ===========================================================================

#[test]
fn V114D2_spark_groupBy_agg_F_sum_unchanged() {
    let src = "\
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.groupBy('place_code').agg(F.sum(col('price')).alias('total'))
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V114D2_spark_groupBy_agg_F_count_unchanged() {
    let src = "\
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.groupBy('place_code').agg(F.count(col('price')).alias('n'))
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V114D2_spark_multi_key_groupBy_agg_F_mean_unchanged() {
    let src = "\
class Orders(Schema):
    region: string
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.groupBy('region', 'place_code').agg(F.mean(col('price')).alias('avg_price'))
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V114D2_spark_groupBy_pivot_agg_unchanged_after_pivot_unknown() {
    // `groupBy(...).pivot(...).agg(...)` still yields Unknown — the
    // after_pivot branch dies cleanly per the v1.0 behavior. No D0030
    // on `amount` (it's a real column).
    let src = "\
class Sales(Schema):
    region: string
    year: int
    amount: double

def f(raw: SparkFrame[Sales]) -> SparkFrame:
    return raw.groupBy('region').pivot('year').agg(F.sum('amount'))
";
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// Existing-behavior preservation: count/sum/mean shortcut aggregates
// still work on the Spark-grouped path (uses grouped_aggregate_schema,
// unrelated to the new pandas arm).
// ===========================================================================

#[test]
fn V114D2_spark_grouped_count_shortcut_unchanged() {
    let src = "\
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.groupBy('place_code').count()
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V114D2_spark_grouped_sum_shortcut_unchanged() {
    let src = "\
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame:
    return raw.groupBy('place_code').sum('price')
";
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// Pandas-side D0030 still fires on a typo'd key, even with the new arm.
// ===========================================================================

#[test]
fn V114D2_pandas_groupby_typo_in_key_fires_D0030() {
    let src = format!(
        "{IN_BASE}
def f(pdf: PandasFrame[In]):
    pdf.groupby('typo').agg('sum')
"
    );
    assert_has_code(&check(&src), "D0030");
}
