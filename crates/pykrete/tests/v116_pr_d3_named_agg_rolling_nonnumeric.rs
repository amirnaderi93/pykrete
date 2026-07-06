//! v1.16 PR-D3 — honest-silence for two audit-flagged false positives on
//! CORRECT pandas code, plus the inplace=<non-literal> multiplexer-guard fold.
//!
//! Two independent pre-tag audits (pandas-coverage + architecture) each built
//! the release binary and live-verified these FPs. The contract pykrete sells
//! is "no false positives on correct code"; each fix falls through to Unknown
//! (honest silence) rather than synthesizing a schema known to be wrong.
//!
//! - **named-aggregation** `df.groupby(k).agg(total=("c","sum"), ...)` — the
//!   modern flat-output groupby idiom passes outputs as KEYWORD args, so the
//!   positional-only aggfunc classifier saw `Absent` and the Pandas arm fell
//!   into the Spark keys-only loop (positional args only), dropping every
//!   named output → a keys-only schema that false-fired D0050 ("Missing in
//!   body") and D0030 (downstream access). The Pandas arm now returns Unknown
//!   for every shape the string/dict/callable recognizers don't handle.
//! - **rolling non-numeric** `pdf.rolling(N).agg("mean")` on a frame with a
//!   non-numeric column — pandas DROPS non-numeric columns, but synthesis
//!   forced every column to Double, so a correct return schema omitting them
//!   false-fired D0050 "extra in body" and a string column got a misleading
//!   Double. Synthesis now runs ONLY when every receiver column is numeric.
//! - **inplace=<non-literal>** — `reset_index(drop=True, inplace=flag)` /
//!   `set_index([...], inplace=flag)` resolve (pandas-stubs) to
//!   `DataFrame | None`; the v1.16 PR-D2 guard only caught the `inplace=True`
//!   literal, so a non-literal flag still synthesized a Derived, overriding
//!   the engine. Now suppressed for any non-literal inplace.
//!
//! Each "no diagnostic" gate is load-bearing: it FIRES the FP on the pre-fix
//! binary. Each "still fires" regression proves the recognized-shape paths
//! (single-string agg, all-numeric rolling, literal-False inplace) are intact.

#![allow(non_snake_case)]

mod common;
use common::*;

// ===========================================================================
// Fix 1 — named-aggregation groupby.agg. The BLOCKER: keyword-arg outputs
// dropped by the positional-only keys-only loop. Now honest-silence Unknown.
// ===========================================================================

#[test]
fn V116D3_named_agg_returns_unknown_no_diagnostics() {
    // Auditor gate #1. Pre-fix: keys-only {region} vs declared {region, total,
    // avg} → D0050 "Missing in body: [avg, total]". Post-fix: Unknown accepts
    // the declared return; no D0050/D0080.
    let src = "\
class In(Schema):
    region: string
    sales: double
    units: int

class Agg(Schema):
    region: string
    total: double
    avg: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Agg]:
    return pdf.groupby('region').agg(total=('sales', 'sum'), avg=('units', 'mean'))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V116D3_named_agg_held_valid_output_access_no_D0030() {
    // Auditor gate #2. Pre-fix: `g` is keys-only {region}, so `g['total']` →
    // D0030 "does not exist on inferred schema [region]". Post-fix: `g` is
    // Unknown → the named output resolves permissively.
    let src = "\
class In(Schema):
    region: string
    sales: double

def f(pdf: PandasFrame[In]):
    g = pdf.groupby('region').agg(total=('sales', 'sum'))
    return g['total']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D3_named_agg_held_arbitrary_key_silent_proves_unknown() {
    // Non-vacuous companion to gate #2: an arbitrary key is ALSO silent,
    // proving the receiver is genuinely Unknown (permissive to every key) and
    // not some concrete schema that happens to contain the named output.
    let src = "\
class In(Schema):
    region: string
    sales: double

def f(pdf: PandasFrame[In]):
    g = pdf.groupby('region').agg(total=('sales', 'sum'))
    return g['anything_at_all']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D3_named_agg_no_D0080_against_wrong_dtype() {
    // Honest silence extends to dtype: since the chain is Unknown, declaring
    // `total` as string (it would be a numeric sum) does NOT false-fire D0080.
    // pykrete claims no schema it can't justify for named aggregation.
    let src = "\
class In(Schema):
    region: string
    sales: double

class Agg(Schema):
    region: string
    total: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Agg]:
    return pdf.groupby('region').agg(total=('sales', 'sum'))
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn V116D3_single_string_agg_typo_still_fires_D0030() {
    // Auditor gate #4 (regression). The single-string arm is untouched:
    // `.agg('sum')` synthesizes {region, sales} and a downstream typo fires
    // D0030. Proves Fix 1 narrowed ONLY the previously-unhandled shapes.
    // Held-intermediate form (like every D1/D2 downstream-typo test): an inline
    // subscript on a groupby-agg chain is a separate pre-existing gap unrelated
    // to Fix 1, which touches only the agg-shape dispatch, not subscript
    // resolution.
    let src = "\
class In(Schema):
    region: string
    sales: double

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('region').agg('sum')
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

// ===========================================================================
// Fix 2 — rolling.agg on a non-numeric-containing frame. pandas DROPS
// non-numeric columns; all-Double synthesis now runs only when every column
// is numeric, else honest-silence Unknown.
// ===========================================================================

#[test]
fn V116D3_rolling_non_numeric_drops_no_false_diagnostic() {
    // Auditor gate #3. Pre-fix: synthesized {name: double, val: double} vs
    // declared {val} → D0050 "extra in body: [name]", and `name` got a
    // misleading Double. Post-fix: `name` is non-numeric → Unknown → no
    // D0050, no D0080.
    let src = "\
class In(Schema):
    name: string
    val: double

class Out(Schema):
    val: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg('mean')
";
    assert_does_not_have_code(&check(src), "D0050");
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn V116D3_rolling_non_numeric_downstream_typo_silent() {
    // Non-vacuous: with a non-numeric column present the chain is Unknown, so
    // even a downstream typo is silent (proves Unknown, not a concrete schema).
    let src = "\
class In(Schema):
    name: string
    val: double

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(3).agg('mean')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D3_rolling_all_numeric_still_synthesizes_typo_fires_D0030() {
    // Regression: an all-numeric frame STILL synthesizes the all-Double
    // envelope, so a downstream typo fires D0030 (the numeric modeling holds).
    let src = "\
class In(Schema):
    a: double
    b: long

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(3).agg('mean')
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D3_rolling_all_numeric_out_matches_no_D0080() {
    // Regression positive: all-numeric frame → all-Double synthesis, so `b:
    // long` upcasts to double and the declared all-double Out matches (no
    // D0080). Load-bearing against a fix that over-suppressed the numeric case.
    let src = "\
class In(Schema):
    a: double
    b: long

class Out(Schema):
    a: double
    b: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg('mean')
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn V116D3_rolling_all_numeric_declared_string_still_fires_D0080() {
    // Regression FIRE: numeric-only synthesis produces Double, so declaring
    // `a: string` crosses the numeric boundary → D0080. Proves synthesis ran
    // for the all-numeric case (a no-op Unknown would not fire).
    let src = "\
class In(Schema):
    a: double
    b: long

class Out(Schema):
    a: string
    b: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg('mean')
";
    assert_has_code(&check(src), "D0080");
}

// ===========================================================================
// Optional fold — reset_index/set_index inplace=<non-literal>. The stubs
// overload resolves a non-literal flag to `DataFrame | None`; synthesis is
// suppressed. Literal inplace=False still returns the frame → still synthesizes.
// ===========================================================================

#[test]
fn V116D3_reset_index_inplace_non_literal_no_synthesis() {
    // Non-literal `inplace=flag` → Unknown (engine's DataFrame|None stands);
    // a downstream typo does NOT fire, proving no synthesis.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In], flag: bool):
    result = pdf.reset_index(drop=True, inplace=flag)
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D3_reset_index_inplace_literal_false_still_synthesizes() {
    // Boundary lock: literal `inplace=False` DOES return the frame, so
    // reset_index(drop=True, inplace=False) still synthesizes and the same
    // downstream typo fires D0030. Proves the fold suppresses only non-literal.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.reset_index(drop=True, inplace=False)
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D3_set_index_inplace_non_literal_no_synthesis() {
    // Non-literal `inplace=flag` on set_index → Unknown, so a valid key is not
    // stripped from a synthesized schema — `result['k']` stays silent.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In], flag: bool):
    result = pdf.set_index(['k'], inplace=flag)
    return result['k']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D3_set_index_inplace_non_literal_typo_still_fires_D0030() {
    // report-before-punt preserved: the guard sits after the key-existence
    // check, so a typoed key fires D0030 even with a non-literal inplace.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In], flag: bool):
    return pdf.set_index(['typo'], inplace=flag)
";
    assert_has_code(&check(src), "D0030");
}
