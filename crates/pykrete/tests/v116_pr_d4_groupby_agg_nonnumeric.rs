//! v1.16 PR-D4 — uniform honest-silence for `groupby.agg` (string + callable
//! arms) when an arithmetic aggregate meets a non-numeric column, plus the
//! D3-reviewer bool/Nullable folds. Completes the non-numeric handling that
//! D3 started for rolling/named-agg.
//!
//! ## The audit-flagged false positive
//! `pdf.groupby("region").agg("mean")` on `{region:string, amount:double,
//! name:string}` returning `{region, amount}` false-fired D0050 "extra in
//! body: [name]" — synthesis kept `name`, but the return schema (correctly)
//! omitted it. The identical FP hit the callable arm (`.agg(np.mean)`).
//!
//! ## Why honest-silence, not precise-drop
//! The brief assumed `.agg("mean")` "silently drops" the non-numeric column.
//! Empirically (pandas 2.2.2, whatsnew-2.0 confirmed) it does NOT: pandas 2.0
//! REMOVED the silent nuisance-column drop, so `.agg("mean")` on a frame with
//! a string column RAISES `TypeError: agg function failed` — the drop needs an
//! explicit `numeric_only=True`. Modeling a precise drop would synthesize
//! `{region, amount}` and pronounce code CLEAN that crashes at runtime — a
//! false negative on broken code. So, matching the merged D3 rolling gate
//! (rolling ALSO raises `DataError` on non-numeric), an arithmetic aggregate
//! (mean/std/var/median → Double) meeting a non-numeric non-key column
//! DECLINES the whole chain to Unknown (honest silence). All three arms
//! (string, callable, rolling) are now uniform: no FP, no blessed crash.
//!
//! ## Per-aggfunc drops-vs-keeps evidence (pandas 2.x `.agg("<fn>")`)
//! | aggfunc              | non-numeric col | pykrete model        |
//! |----------------------|-----------------|----------------------|
//! | mean/std/var/median  | RAISES          | decline → Unknown    |
//! | sum/min/max/first/last | KEEPS (preserve) | keep, receiver ty  |
//! | count/nunique        | KEEPS (→ int64) | keep, Long           |
//! | bool (any arith agg) | KEEPS + upcast  | keep → Double        |
//!
//! Each "no diagnostic" gate is load-bearing: it FIRES the FP on the pre-D4
//! binary. Each KEEP gate proves the non-numeric column genuinely SURVIVES.

#![allow(non_snake_case)]

mod common;
use common::*;

const MIXED: &str = "\
class In(Schema):
    region: string
    amount: double
    name: string
";

// ===========================================================================
// DROP set — arithmetic aggregates (mean/std/var/median) RAISE on a
// non-numeric non-key column in pandas 2.x → honest-silence Unknown. The FP
// gate: synthesized {region, amount, name} vs declared {region, amount} used
// to fire D0050 "extra in body: [name]".
// ===========================================================================

fn drop_set_src(aggfunc: &str) -> String {
    format!(
        "{MIXED}
class Out(Schema):
    region: string
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('region').agg('{aggfunc}')
"
    )
}

#[test]
fn V116D4_agg_mean_mixed_nonnumeric_no_D0050() {
    // Headline FP for a "groupby.agg completion" release. Pre-D4: D0050
    // "extra in body: [name]". Post-D4: Unknown accepts the narrowed return.
    assert_does_not_have_code(&check(&drop_set_src("mean")), "D0050");
}

#[test]
fn V116D4_agg_std_mixed_nonnumeric_no_D0050() {
    assert_does_not_have_code(&check(&drop_set_src("std")), "D0050");
}

#[test]
fn V116D4_agg_var_mixed_nonnumeric_no_D0050() {
    assert_does_not_have_code(&check(&drop_set_src("var")), "D0050");
}

#[test]
fn V116D4_agg_median_mixed_nonnumeric_no_D0050() {
    assert_does_not_have_code(&check(&drop_set_src("median")), "D0050");
}

#[test]
fn V116D4_agg_mean_mixed_wrong_dtype_no_D0080() {
    // Honest silence extends to dtype: the chain is Unknown, so declaring
    // `amount` as string does NOT false-fire D0080 (Unknown is permissive).
    let src = "\
class In(Schema):
    region: string
    amount: double
    name: string

class Out(Schema):
    region: string
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('region').agg('mean')
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn V116D4_agg_mean_mixed_downstream_typo_silent() {
    // Non-vacuous: with a non-numeric column present the chain is genuinely
    // Unknown, so even a downstream typo is silent — proving Unknown, not some
    // concrete drop-schema that would still validate column existence.
    let src = "\
class In(Schema):
    region: string
    amount: double
    name: string

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('region').agg('mean')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D4_agg_mean_mixed_dropped_col_access_silent() {
    // Distinguishes honest-silence from precise-drop: under a concrete
    // {region, amount} drop-schema, `result['name']` would fire D0030. Under
    // Unknown it is silent. Proves we did NOT ship precise-drop (which would
    // bless code that raises at runtime).
    let src = "\
class In(Schema):
    region: string
    amount: double
    name: string

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('region').agg('mean')
    return result['name']
";
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// DROP set regression — an ALL-NUMERIC frame still synthesizes the arithmetic
// override (guard narrowed ONLY the non-numeric case).
// ===========================================================================

const ALL_NUM: &str = "\
class In(Schema):
    region: string
    amount: double
    cnt: long
";

#[test]
fn V116D4_agg_mean_all_numeric_still_synthesizes_typo_D0030() {
    let src = format!(
        "{ALL_NUM}
def f(pdf: PandasFrame[In]):
    result = pdf.groupby('region').agg('mean')
    return result['typo']
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn V116D4_agg_mean_all_numeric_out_matches_no_D0080() {
    // Load-bearing against a fix that over-suppresses the numeric case: `cnt:
    // long` upcasts to double under mean and the all-double Out matches.
    let src = format!(
        "{ALL_NUM}
class Out(Schema):
    region: string
    amount: double
    cnt: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('region').agg('mean')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D4_agg_mean_all_numeric_declared_string_fires_D0080() {
    // Proves synthesis ran (a no-op Unknown would not fire): mean → Double,
    // declaring `amount: string` crosses the numeric boundary → D0080.
    let src = format!(
        "{ALL_NUM}
class Out(Schema):
    region: string
    amount: string
    cnt: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('region').agg('mean')
"
    );
    assert_has_code(&check(&src), "D0080");
}

// ===========================================================================
// KEEP set — sum/min/max/first/last (preserve) + count/nunique (Long)
// genuinely KEEP the non-numeric column in pandas 2.x. One positive per
// aggfunc proving `name` SURVIVES (clean return WITH name).
// ===========================================================================

fn keep_preserve_src(aggfunc: &str) -> String {
    // preserve-arm aggfuncs keep receiver dtype: name stays string.
    format!(
        "{MIXED}
class Out(Schema):
    region: string
    amount: double
    name: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('region').agg('{aggfunc}')
"
    )
}

#[test]
fn V116D4_agg_sum_keeps_nonnumeric_clean() {
    assert_no_diagnostics(&check(&keep_preserve_src("sum")));
}

#[test]
fn V116D4_agg_min_keeps_nonnumeric_clean() {
    assert_no_diagnostics(&check(&keep_preserve_src("min")));
}

#[test]
fn V116D4_agg_max_keeps_nonnumeric_clean() {
    assert_no_diagnostics(&check(&keep_preserve_src("max")));
}

#[test]
fn V116D4_agg_first_keeps_nonnumeric_clean() {
    assert_no_diagnostics(&check(&keep_preserve_src("first")));
}

#[test]
fn V116D4_agg_last_keeps_nonnumeric_clean() {
    assert_no_diagnostics(&check(&keep_preserve_src("last")));
}

fn keep_long_src(aggfunc: &str) -> String {
    // count/nunique count non-null/distinct → Long for every kept column.
    format!(
        "{MIXED}
class Out(Schema):
    region: string
    amount: long
    name: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('region').agg('{aggfunc}')
"
    )
}

#[test]
fn V116D4_agg_count_keeps_nonnumeric_clean() {
    assert_no_diagnostics(&check(&keep_long_src("count")));
}

#[test]
fn V116D4_agg_nunique_keeps_nonnumeric_clean() {
    assert_no_diagnostics(&check(&keep_long_src("nunique")));
}

#[test]
fn V116D4_agg_sum_omit_nonnumeric_fires_D0050() {
    // Direction check: synthesis KEEPS `name`, so a return that omits it makes
    // `name` "extra in body". Proves the KEEP set is non-vacuous — the clean
    // tests above pass because `name` is present, not because synthesis punted.
    let src = "\
class In(Schema):
    region: string
    amount: double
    name: string

class Out(Schema):
    region: string
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('region').agg('sum')
";
    assert_has_code(&check(src), "D0050");
    assert_message_contains(&check(src), "D0050", "extra in body: [name]");
}

// ===========================================================================
// Callable arm — opaque; honest-silence when any non-key column is
// non-numeric, all-numeric keeps the superset synthesis. (The pinned
// counterpart V116D2_callable_nonnumeric_honest_silence_no_D0050 lives in the
// D2 file; these cover the all-numeric survivor path.)
// ===========================================================================

#[test]
fn V116D4_callable_mean_all_numeric_typo_fires_D0030() {
    // All non-key columns numeric → synthesize the superset at Unknown dtype;
    // a downstream typo still fires D0030 (columns exist, dtype unclaimed).
    let src = format!(
        "import numpy as np

{ALL_NUM}
def f(pdf: PandasFrame[In]):
    result = pdf.groupby('region').agg(np.mean)
    return result['typo']
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn V116D4_callable_mean_mixed_downstream_typo_silent() {
    // Non-vacuous companion: with a non-numeric column the callable chain is
    // Unknown, so a downstream typo is silent (proves honest-silence, not a
    // concrete schema).
    let src = "\
import numpy as np

class In(Schema):
    region: string
    amount: double
    name: string

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('region').agg(np.mean)
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// Fix 3 — bool is kept + upcast to float64 under an arithmetic aggregate
// (groupby + rolling), and the Nullable(inner) recursion arm.
// ===========================================================================

#[test]
fn V116D4_agg_mean_bool_nonkey_declared_string_fires_D0080() {
    // Non-vacuous pin for the Bool arm added to `is_numeric_dtype`, in the
    // GROUPBY arm — mirrors the rolling
    // `V116D4_rolling_bool_upcasts_double_declared_string_fires_D0080` below.
    // bool is numeric for aggregation (True→1.0): the frame does NOT decline,
    // `flag` synthesizes to Double, so declaring `flag: string` crosses the
    // numeric boundary → D0080. This fires ONLY if bool is numeric here — were
    // bool treated non-numeric the arm would decline to Unknown and D0080
    // would NOT fire (a clean `flag: double` Out would pass either way, so it
    // can't pin bool-is-numeric; this mismatch can).
    let src = "\
class In(Schema):
    region: string
    amount: double
    flag: bool

class Out(Schema):
    region: string
    amount: double
    flag: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('region').agg('mean')
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn V116D4_rolling_bool_upcasts_double_declared_string_fires_D0080() {
    // rolling keeps + upcasts bool to float64. Declaring `flag: string`
    // crosses the numeric boundary → D0080 fires, proving the all-Double
    // synthesis ran (bool did not decline the chain to Unknown).
    let src = "\
class In(Schema):
    a: double
    flag: bool

class Out(Schema):
    a: string
    flag: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg('mean')
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn V116D4_rolling_nullable_double_synthesizes_typo_D0030() {
    // Nullable(inner) recursion: Optional[double] is numeric, so the all-Double
    // synthesis runs and a downstream typo fires D0030.
    let src = "\
class In(Schema):
    a: Optional[double]
    b: long

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(3).agg('mean')
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D4_rolling_nullable_string_declines_typo_silent() {
    // Nullable(String) recurses to non-numeric → the chain declines to Unknown,
    // so even a downstream typo is silent (proves the recursion arm rejects a
    // wrapped non-numeric, not just a bare one).
    let src = "\
class In(Schema):
    name: Optional[string]
    b: long

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(3).agg('mean')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}
