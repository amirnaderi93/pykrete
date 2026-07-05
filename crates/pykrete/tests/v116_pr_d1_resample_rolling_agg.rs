//! v1.16 PR-D1 — `pdf.resample("<rule>").agg("<aggfunc>")` +
//! `pdf.rolling(<N>).agg("<aggfunc>")` direct-chain synthesis (Option B).
//! **First observable resample/rolling schema inference in pykrete; THIRD
//! consumer of the v1.15 `resolve_override_ty` aggregate-to-dtype
//! primitive.**
//!
//! Spec: `docs/design/v1.16-spec.md` §1.ii (Option B — no
//! `SchemaView::Windowed` lattice variant).
//!
//! Behavior contract: when the `.agg(...)` receiver expression is a
//! `<base>.resample(<literal>)` / `<base>.rolling(<literal>)` call, `<base>`
//! is Pandas-dialect, and the aggfunc is a single allowlisted string, the
//! result `SchemaView` is synthesized directly as EVERY receiver column at
//! the aggregate-driven dtype (no key columns — the resample DatetimeIndex
//! and rolling window are both un-modeled per the v1.4 §5 index carve-out):
//!
//! - `"count"` / `"nunique"`             → Long
//! - `"mean"` / `"std"` / `"var"` /
//!   `"median"`                           → Double
//! - `"sum"` / `"min"` / `"max"` /
//!   `"first"` / `"last"`                 → preserve receiver column type
//!
//! Observation surface mirrors the v1.14 groupby.agg battery: declare the
//! return-type schema and let D0080 fire on a dtype mismatch, or bind the
//! chain and let a downstream typo fire D0030. Per spec §1.ii.5 each
//! dtype-override family ships a paired positive (no D0080) + FIRE (D0080
//! fires) test, with the FIRE test load-bearing — a no-op `None`
//! synthesizer would pass the positives vacuously but fail every FIRE.
//!
//! Fall-through (honest silence → Unknown): held-intermediate
//! (`r = df.resample("D"); r.agg(...)`), non-literal rule/window,
//! dict/list/callable/out-of-allowlist aggfunc, and Spark receivers all
//! return Unknown — asserted by "no D0080 against a would-mismatch Out"
//! and "no D0030 on a downstream typo".

#![allow(non_snake_case)]

mod common;
use common::*;

const IN_BASE: &str = "\
class In(Schema):
    sales: double
    units: int
";

// ===========================================================================
// resample.agg — positive: correct override dtype, no D0080. Each pairs
// with a FIRE test below to prove the synthesis is wired (not a no-op).
// ===========================================================================

#[test]
fn V116D1_resample_count_synthesizes_long() {
    // count → Long on every value column (sales: double → long, units: int
    // → long). No key exemption: resample models no index column.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: long
    units: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg('count')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_resample_mean_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: double
    units: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('1H').agg('mean')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_resample_sum_preserves_receiver_types() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg('sum')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ===========================================================================
// resample.agg — FIRE per dtype-override family (§1.ii.5, load-bearing).
// A declared `string` crosses the numeric/non-numeric boundary against the
// synthesized dtype; D0080 must fire, proving the arm produced a concrete
// schema (a no-op Unknown would NOT fire).
// ===========================================================================

#[test]
fn V116D1_resample_count_declared_string_fires_D0080() {
    // count → Long; declaring sales: string → D0080 (Long-override family).
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg('count')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V116D1_resample_mean_declared_string_fires_D0080() {
    // mean → Double; declaring sales: string → D0080 (Double-override).
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg('mean')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V116D1_resample_sum_declared_string_fires_D0080() {
    // sum → preserve (sales stays double); declaring string → D0080. Proves
    // the preserve arm ran rather than degrading to Unknown.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg('sum')
"
    );
    assert_has_code(&check(&src), "D0080");
}

// ===========================================================================
// resample.agg — PROBE-RESOLVES + downstream D0030 on the synthesized
// envelope.
// ===========================================================================

#[test]
fn V116D1_resample_downstream_valid_column_resolves() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    result = pdf.resample('D').agg('sum')
    return result['sales']
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V116D1_resample_downstream_typo_fires_D0030() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    result = pdf.resample('D').agg('sum')
    return result['salez']
";
    assert_has_code(&check(src), "D0030");
}

// ===========================================================================
// rolling.agg — positive: rolling ALWAYS upcasts every column to Double
// (empirically verified against pandas 2.3.3 — NaN from incomplete leading
// windows + float64 kernels), regardless of aggfunc OR input dtype. Unlike
// resample/groupby, `resolve_override_ty` does NOT apply. No D0080 against
// an all-Double Out.
// ===========================================================================

#[test]
fn V116D1_rolling_count_synthesizes_double() {
    // count → Double on rolling (it's Long on resample/groupby). sales:
    // double, units: int → both Double.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: double
    units: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg('count')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_rolling_mean_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: double
    units: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg('mean')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_rolling_sum_synthesizes_double() {
    // sum does NOT preserve on rolling (it does on resample/groupby):
    // rolling upcasts units: int → Double too.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: double
    units: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(2).agg('sum')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_rolling_upcasts_string_column_to_double() {
    // THE correctness lock for the R2 rolling-dtype fix. rolling upcasts
    // EVERY column to float64 — even a string `label`, even for 'sum'
    // (which PRESERVES on resample/groupby). No key exemption, no preserve.
    // Load-bearing against the R1 `resolve_override_ty`-reuse bug: a
    // preserve arm leaves `label` string → string-vs-declared-double fires
    // D0080 (the numeric-blind comparator only flags numeric-vs-non-numeric,
    // so this string column is the one dtype the D0080 check CAN observe).
    let src = "\
class In(Schema):
    label: string
    value: double

class Out(Schema):
    label: double
    value: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(2).agg('sum')
";
    assert_does_not_have_code(&check(src), "D0080");
}

// ===========================================================================
// rolling.agg — FIRE (§1.ii.5, load-bearing). rolling collapses every
// family to Double, so the fired column (declared string vs the synthesized
// Double) is the discriminator, not the aggfunc. Each of count/sum/mean
// still proves its own arm produced a concrete schema (a no-op Unknown
// would not fire). Non-fired columns are declared `double` (the truth).
// ===========================================================================

#[test]
fn V116D1_rolling_count_declared_string_fires_D0080() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg('count')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V116D1_rolling_mean_declared_string_fires_D0080() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg('mean')
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn V116D1_rolling_sum_declared_string_fires_D0080() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(2).agg('sum')
"
    );
    assert_has_code(&check(&src), "D0080");
}

// ===========================================================================
// rolling.agg — PROBE-RESOLVES (all columns present) + downstream D0030.
// ===========================================================================

#[test]
fn V116D1_rolling_keeps_all_columns_downstream_resolves() {
    // All receiver columns survive the rolling window (same-length output,
    // no row/column reduction) and stay name-tracked for D0030. `region` is
    // a string: pandas `rolling.agg('sum')` actually DataErrors on a string
    // column — type-aware rolling (declining non-numeric columns) is a v1.17
    // gap; v1.16 permissively models it as Double, so it still resolves.
    let src = "\
class In(Schema):
    sales: double
    units: int
    region: string

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(2).agg('sum')
    return (result['sales'], result['units'], result['region'])
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V116D1_rolling_downstream_typo_fires_D0030() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(3).agg('mean')
    return result['unitz']
";
    assert_has_code(&check(src), "D0030");
}

// ===========================================================================
// Fall-through / honest-silence → Unknown (spec §1.ii.2). Asserted by "no
// D0080 against a would-mismatch Out" (a fired arm would synthesize numeric
// ≠ the declared string) and "no D0030 on a downstream typo".
// ===========================================================================

#[test]
fn V116D1_resample_out_of_allowlist_string_falls_through() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg('wat')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_resample_dict_aggfunc_falls_through() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg({{'sales': 'sum'}})
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_resample_list_aggfunc_falls_through() {
    // List-of-aggfunc → MultiIndex-on-columns, out-of-scope per §1.ii.2.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg(['sum', 'mean'])
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_rolling_callable_aggfunc_falls_through() {
    let src = format!(
        "{IN_BASE}
import numpy as np

class Out(Schema):
    sales: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg(np.sum)
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_resample_non_literal_rule_falls_through() {
    // The rule is a variable, not a literal — a dynamic window pykrete
    // can't key on. Unknown result → downstream typo does NOT fire D0030.
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In], rule: str):
    result = pdf.resample(rule).agg('sum')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D1_rolling_non_literal_window_falls_through() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In], n: int):
    result = pdf.rolling(n).agg('sum')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D1_held_intermediate_resample_falls_through() {
    // `r = df.resample("D"); r.agg(...)` split across statements — the
    // `.agg` receiver is a bare Name, not the resample call, so the chain
    // recognizer doesn't match. Deferred to v1.17 (SchemaView::Windowed).
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    r = pdf.resample('D')
    result = r.agg('sum')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D1_held_intermediate_rolling_falls_through() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    r = pdf.rolling(3)
    result = r.agg('mean')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D1_spark_receiver_resample_falls_through() {
    // resample/rolling are pandas-only idioms; a Spark receiver falls
    // through (dialect gate). A fired arm would synthesize numeric columns
    // and mismatch the declared string Out — no D0080 proves no synthesis.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: string

def f(sdf: SparkFrame[In]) -> SparkFrame[Out]:
    return sdf.resample('D').agg('sum')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_spark_receiver_rolling_falls_through() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: string

def f(sdf: SparkFrame[In]) -> SparkFrame[Out]:
    return sdf.rolling(3).agg('sum')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ===========================================================================
// R2 allowlist split — first/last/nunique AttributeError on a `Rolling`
// object (they exist on Resampler/GroupBy). rolling drops them → Unknown;
// resample keeps the full allowlist → synthesizes. The paired rolling-falls-
// through + resample-still-synthesizes tests lock the split in both
// directions (load-bearing: the resample side fires D0030, proving synthesis
// happened, so a shared-allowlist regression would flip one side).
// ===========================================================================

#[test]
fn V116D1_rolling_first_falls_through_not_valid_on_rolling() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(3).agg('first')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D1_rolling_last_falls_through_not_valid_on_rolling() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(3).agg('last')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D1_rolling_nunique_falls_through_not_valid_on_rolling() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    result = pdf.rolling(3).agg('nunique')
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D1_resample_first_still_synthesizes_downstream_typo_fires_D0030() {
    // Contrast to rolling: 'first' IS valid on a Resampler — resample keeps
    // the full allowlist, synthesizes, and a downstream typo fires D0030.
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    result = pdf.resample('D').agg('first')
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D1_resample_nunique_still_synthesizes_downstream_typo_fires_D0030() {
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    result = pdf.resample('D').agg('nunique')
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

// ===========================================================================
// R2 symmetric fall-through matrix — dict/list/callable now covered on BOTH
// arms (R1 had dict/list on resample and callable on rolling only).
// ===========================================================================

#[test]
fn V116D1_resample_callable_aggfunc_falls_through() {
    let src = format!(
        "{IN_BASE}
import numpy as np

class Out(Schema):
    sales: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.resample('D').agg(np.sum)
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_rolling_dict_aggfunc_falls_through() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg({{'sales': 'sum'}})
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn V116D1_rolling_list_aggfunc_falls_through() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    sales: string
    units: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.rolling(3).agg(['sum', 'mean'])
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ===========================================================================
// R2 single-walk guarantee (reviewer Q3). A diagnostic-emitting base under a
// window chain whose synthesis returns Unknown must NOT double-emit.
// ===========================================================================

#[test]
fn V116D1_diagnostic_emitting_base_fires_exactly_once() {
    // `set_index(['sales', 'units', 'typo'])` removes BOTH real columns → an
    // empty envelope → rolling synthesis returns None (Unknown). It also
    // fires D0030 on the 'typo' key. `handle_window_agg_chain` walks the base
    // exactly once and returns Some(None), so the outer receiver-resolution
    // guard never re-walks it. Pre-R2 (single SchemaView return) this fell
    // through and re-walked the base → D0030 fired twice.
    let src = "\
class In(Schema):
    sales: double
    units: int

def f(pdf: PandasFrame[In]):
    return pdf.set_index(['sales', 'units', 'typo']).rolling(2).agg('sum')
";
    assert_count(&check(src), "D0030", 1);
}
