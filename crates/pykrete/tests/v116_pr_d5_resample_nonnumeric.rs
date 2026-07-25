//! v1.16 PR-D5 — honest-silence for `resample.agg` when an arithmetic
//! aggregate meets a non-numeric column. The sibling-arm sweep D3 (rolling,
//! named-agg) and D4 (groupby string + callable) missed: `resample.agg` was
//! left mapping EVERY receiver column through `resolve_override_ty`
//! unconditionally.
//!
//! ## The audit-flagged false positive
//! `df.resample("D").agg("mean")` on `{region: string, amount: double}`
//! returning the same schema false-fired D0080 "column 'region' is declared
//! String in schema 'OutSchema', but the body produces Double" — synthesis
//! forced the string column to the mean-override dtype.
//!
//! ## Per-aggfunc raises-vs-keeps evidence
//! Verified directly against a `DatetimeIndex` frame with a string column on
//! **pandas 2.2.3** (numpy 1.26.4) and **pandas 3.0.5** (numpy 2.5.1). Both
//! majors agree; only the message text differs.
//!
//! | aggfunc                | non-numeric col                        | pykrete model      |
//! |------------------------|----------------------------------------|--------------------|
//! | mean/std/var/median    | RAISES `TypeError`                     | decline → Unknown  |
//! | sum/min/max/first/last | KEEPS (dtype preserved)                | keep, receiver ty  |
//! | count/nunique          | KEEPS (→ int64)                        | keep, Long         |
//! | bool (any arith agg)   | KEEPS + upcast → float64               | keep → Double      |
//!
//! pandas 2.0 removed nuisance-column dropping for `Resampler` reductions
//! exactly as it did for groupby, so the drop needs an explicit
//! `numeric_only=True`. Modeling a precise drop would pronounce CLEAN code
//! that raises at runtime, so — matching the merged D3/D4 gates — the whole
//! chain declines to Unknown instead.
//!
//! Unlike the groupby arms there is NO key exemption: resample models no key
//! columns (the DatetimeIndex is un-modeled per the v1.4 §5 carve-out), so
//! the gate covers ALL receiver columns.
//!
//! ## `resample(..., on="col")`
//! pandas moves `col` OUT of the columns and INTO the resample index
//! (`.resample("D", on="ts").agg("sum")` → columns `[region, amount]`,
//! `index.name == "ts"`, both majors). Keeping `ts` false-fired D0050 "extra
//! in body: [ts]" against a correct return schema, so the chain declines.
//! `rolling(<n>, on="col")` KEEPS the column and is deliberately untouched.
//!
//! Every "no diagnostic" gate here is load-bearing: it FIRES on the pre-D5
//! binary. Every KEEP gate is paired with a direction-check proving the
//! non-numeric column genuinely SURVIVES rather than the arm having gone
//! silently Unknown.

#![allow(non_snake_case)]

mod common;
use common::*;

const MIXED: &str = "\
class Mixed(Schema):
    region: string
    amount: double
";

// ===========================================================================
// DROP set — mean/std/var/median RAISE on a non-numeric column → honest
// silence. The headline FP: `region` forced to Double, D0080 against a
// return schema that correctly declares it String.
// ===========================================================================

fn drop_set_src(aggfunc: &str) -> String {
    format!(
        "{MIXED}
class OutSchema(Schema):
    region: string
    amount: double

def r(df: PandasFrame[Mixed]) -> PandasFrame[OutSchema]:
    return df.resample('D').agg('{aggfunc}')
"
    )
}

#[test]
fn V116D5_resample_mean_mixed_nonnumeric_no_D0080() {
    // The exact reviewer repro. Pre-D5: D0080 "column 'region' is declared
    // String ... but the body produces Double".
    assert_does_not_have_code(&check(&drop_set_src("mean")), "D0080");
}

#[test]
fn V116D5_resample_std_mixed_nonnumeric_no_D0080() {
    assert_does_not_have_code(&check(&drop_set_src("std")), "D0080");
}

#[test]
fn V116D5_resample_var_mixed_nonnumeric_no_D0080() {
    assert_does_not_have_code(&check(&drop_set_src("var")), "D0080");
}

#[test]
fn V116D5_resample_median_mixed_nonnumeric_no_D0080() {
    assert_does_not_have_code(&check(&drop_set_src("median")), "D0080");
}

#[test]
fn V116D5_resample_mean_mixed_narrowed_out_no_D0050() {
    // The D4-shaped sibling FP: honest silence also accepts a return schema
    // that OMITS the non-numeric column (some users write the
    // `numeric_only=True` shape). Unknown is permissive in both directions.
    let src = format!(
        "{MIXED}
class OutSchema(Schema):
    amount: double

def r(df: PandasFrame[Mixed]) -> PandasFrame[OutSchema]:
    return df.resample('D').agg('mean')
"
    );
    assert_does_not_have_code(&check(&src), "D0050");
}

#[test]
fn V116D5_resample_mean_mixed_downstream_typo_silent() {
    // Non-vacuous: with a non-numeric column present the chain is genuinely
    // Unknown, so even a downstream typo is silent. Proves we declined rather
    // than shipping a precise-drop schema (which would still validate column
    // existence — and would bless code that raises at runtime).
    let src = format!(
        "{MIXED}
def r(df: PandasFrame[Mixed]):
    result = df.resample('D').agg('mean')
    return result['typo']
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

// ===========================================================================
// KEEP set — sum/min/max/first/last/count/nunique genuinely keep the
// non-numeric column on a `Resampler`. Each is paired with a direction-check
// (omitting the column fires D0050) so the "no diagnostic" assertion can't
// pass vacuously through an over-broad decline.
// ===========================================================================

fn keep_set_src(aggfunc: &str, region_ty: &str, amount_ty: &str) -> String {
    format!(
        "{MIXED}
class OutSchema(Schema):
    region: {region_ty}
    amount: {amount_ty}

def r(df: PandasFrame[Mixed]) -> PandasFrame[OutSchema]:
    return df.resample('D').agg('{aggfunc}')
"
    )
}

fn keep_set_omitted_src(aggfunc: &str) -> String {
    format!(
        "{MIXED}
class OutSchema(Schema):
    amount: double

def r(df: PandasFrame[Mixed]) -> PandasFrame[OutSchema]:
    return df.resample('D').agg('{aggfunc}')
"
    )
}

#[test]
fn V116D5_resample_sum_keeps_nonnumeric_no_D0080() {
    // sum preserves receiver dtypes: region stays String, amount stays Double.
    assert_does_not_have_code(&check(&keep_set_src("sum", "string", "double")), "D0080");
}

#[test]
fn V116D5_resample_min_keeps_nonnumeric_no_D0080() {
    assert_does_not_have_code(&check(&keep_set_src("min", "string", "double")), "D0080");
}

#[test]
fn V116D5_resample_max_keeps_nonnumeric_no_D0080() {
    assert_does_not_have_code(&check(&keep_set_src("max", "string", "double")), "D0080");
}

#[test]
fn V116D5_resample_first_keeps_nonnumeric_no_D0080() {
    assert_does_not_have_code(&check(&keep_set_src("first", "string", "double")), "D0080");
}

#[test]
fn V116D5_resample_last_keeps_nonnumeric_no_D0080() {
    assert_does_not_have_code(&check(&keep_set_src("last", "string", "double")), "D0080");
}

#[test]
fn V116D5_resample_count_keeps_nonnumeric_no_D0080() {
    // count → Long on BOTH columns (pandas: int64 for the string column too).
    assert_does_not_have_code(&check(&keep_set_src("count", "long", "long")), "D0080");
}

#[test]
fn V116D5_resample_nunique_keeps_nonnumeric_no_D0080() {
    assert_does_not_have_code(&check(&keep_set_src("nunique", "long", "long")), "D0080");
}

#[test]
fn V116D5_resample_sum_omitting_nonnumeric_fires_D0050() {
    // Direction-check for the whole KEEP set: `region` genuinely SURVIVES, so
    // a return schema that omits it is WRONG and must still be caught. This is
    // what makes the seven no-D0080 gates above non-vacuous — an over-broad
    // decline would silence this too.
    assert_has_code(&check(&keep_set_omitted_src("sum")), "D0050");
}

#[test]
fn V116D5_resample_count_omitting_nonnumeric_fires_D0050() {
    assert_has_code(&check(&keep_set_omitted_src("count")), "D0050");
}

#[test]
fn V116D5_resample_sum_keeps_nonnumeric_wrong_dtype_fires_D0080() {
    // Second direction-check: the KEEP set still TYPES the surviving column.
    // sum preserves String; declaring `region: double` crosses the boundary.
    assert_has_code(&check(&keep_set_src("sum", "double", "double")), "D0080");
}

#[test]
fn V116D5_resample_sum_mixed_downstream_typo_fires_D0030() {
    // The KEEP set produces a concrete envelope, not Unknown.
    let src = format!(
        "{MIXED}
def r(df: PandasFrame[Mixed]):
    result = df.resample('D').agg('sum')
    return result['typo']
"
    );
    assert_has_code(&check(&src), "D0030");
}

// ===========================================================================
// ALL-NUMERIC regression — the v1.16 PR-D1 resample feature must still work.
// The guard narrowed ONLY the non-numeric case.
// ===========================================================================

const ALL_NUM: &str = "\
class NumIn(Schema):
    sales: double
    units: int
";

#[test]
fn V116D5_resample_mean_all_numeric_still_synthesizes_no_D0080() {
    let src = format!(
        "{ALL_NUM}
class Out(Schema):
    sales: double
    units: double

def r(df: PandasFrame[NumIn]) -> PandasFrame[Out]:
    return df.resample('D').agg('mean')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D5_resample_mean_all_numeric_declared_mismatch_fires_D0080() {
    // Load-bearing against an over-broad fix: mean → Double, so declaring
    // `sales: string` must still fire. A no-op Unknown would NOT fire.
    let src = format!(
        "{ALL_NUM}
class Out(Schema):
    sales: string
    units: double

def r(df: PandasFrame[NumIn]) -> PandasFrame[Out]:
    return df.resample('D').agg('mean')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V116D5_resample_mean_all_numeric_typo_fires_D0030() {
    let src = format!(
        "{ALL_NUM}
def r(df: PandasFrame[NumIn]):
    result = df.resample('D').agg('mean')
    return result['typo']
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn V116D5_resample_mean_bool_column_still_synthesizes_fires_D0080() {
    // `bool` is numeric for aggregation: pandas KEEPS it and upcasts to
    // float64 (verified both majors), so the frame must NOT decline. Declaring
    // `flag: string` crosses the numeric boundary → D0080. This fires ONLY if
    // bool is numeric here; were it treated non-numeric the arm would decline
    // to Unknown and stay silent.
    let src = "\
class In(Schema):
    amount: double
    flag: bool

class Out(Schema):
    amount: double
    flag: string

def r(df: PandasFrame[In]) -> PandasFrame[Out]:
    return df.resample('D').agg('mean')
";
    assert_has_code(&check(src), "D0080");
}

// ===========================================================================
// Nullable arm — `Nullable(inner)` recurses through `is_numeric_dtype`, so
// Optional[double] synthesizes and Optional[string] declines.
// ===========================================================================

#[test]
fn V116D5_resample_mean_nullable_double_still_synthesizes_fires_D0080() {
    let src = "\
class In(Schema):
    amount: Optional[double]
    units: int

class Out(Schema):
    amount: string
    units: double

def r(df: PandasFrame[In]) -> PandasFrame[Out]:
    return df.resample('D').agg('mean')
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn V116D5_resample_mean_nullable_string_declines_no_D0080() {
    // Optional[string] is non-numeric through the Nullable recursion → decline.
    let src = "\
class In(Schema):
    region: Optional[string]
    amount: double

class Out(Schema):
    region: Optional[string]
    amount: double

def r(df: PandasFrame[In]) -> PandasFrame[Out]:
    return df.resample('D').agg('mean')
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn V116D5_resample_mean_nullable_string_downstream_typo_silent() {
    // Non-vacuous companion: the Optional[string] frame is genuinely Unknown.
    let src = "\
class In(Schema):
    region: Optional[string]
    amount: double

def r(df: PandasFrame[In]):
    result = df.resample('D').agg('mean')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// `resample(..., on="col")` — pandas moves `col` into the index, so keeping
// it false-fired D0050 "extra in body: [ts]" against a CORRECT return schema.
// ===========================================================================

const WITH_TS: &str = "\
class WithTs(Schema):
    ts: timestamp
    region: string
    amount: double
";

#[test]
fn V116D5_resample_on_kwarg_correct_schema_no_D0050() {
    // The STEP-4 FP: `ts` becomes the index, so a return schema omitting it is
    // CORRECT. Pre-D5: D0050 "extra in body: [ts]".
    let src = format!(
        "{WITH_TS}
class OutNoTs(Schema):
    region: string
    amount: double

def r(df: PandasFrame[WithTs]) -> PandasFrame[OutNoTs]:
    return df.resample('D', on='ts').agg('sum')
"
    );
    assert_does_not_have_code(&check(&src), "D0050");
}

#[test]
fn V116D5_resample_on_kwarg_declines_downstream_typo_silent() {
    // Non-vacuous: the `on=` chain is genuinely Unknown, not some narrowed
    // concrete schema. (Precisely dropping `ts` would be new modeling — a
    // feature, not an FP fix; honest silence is the smaller claim.)
    let src = format!(
        "{WITH_TS}
def r(df: PandasFrame[WithTs]):
    result = df.resample('D', on='ts').agg('sum')
    return result['typo']
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn V116D5_resample_without_on_kwarg_still_synthesizes_D0030() {
    // Scoping pin: the decline keys on `on=` specifically. The SAME frame
    // without it still synthesizes a concrete envelope.
    let src = format!(
        "{WITH_TS}
def r(df: PandasFrame[WithTs]):
    result = df.resample('D').agg('sum')
    return result['typo']
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn V116D5_resample_on_kwarg_base_diagnostic_emitted_once() {
    // R2 single-walk guarantee. The `on=` decline returns BEFORE the base is
    // walked, so the caller's normal receiver-resolution path walks it exactly
    // once — same shape as the non-literal-rule and out-of-allowlist-aggfunc
    // early returns. Declining AFTER the walk instead would double-emit.
    let src = format!(
        "{WITH_TS}
def r(df: PandasFrame[WithTs]):
    return df[['nope']].resample('D', on='ts').agg('sum')
"
    );
    assert_count(&check(&src), "D0030", 1);
}

#[test]
fn V116D5_resample_nonnumeric_base_diagnostic_emitted_once() {
    // Companion for the non-numeric gate, which declines AFTER the walk (it
    // needs the receiver's dtypes) and so relies on the `Some(None)`
    // single-walk contract rather than the early return.
    let src = format!(
        "{MIXED}
def r(df: PandasFrame[Mixed]):
    return df[['nope']].resample('D').agg('mean')
"
    );
    assert_count(&check(&src), "D0030", 1);
}

#[test]
fn V116D5_rolling_on_kwarg_unaffected_still_synthesizes_D0030() {
    // `rolling(<n>, on="col")` KEEPS `col` as a column (verified both majors),
    // so it must NOT inherit the resample decline. An all-numeric frame still
    // synthesizes through the rolling arm.
    let src = "\
class In(Schema):
    qty: int
    amount: double

def r(df: PandasFrame[In]):
    result = df.rolling(2, on='qty').agg('mean')
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}
