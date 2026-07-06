//! v1.16 PR-D2 — dict-form + callable-form `pdf.groupby(keys).agg(...)` and
//! the inplace multiplexer-guards on `reset_index` / `set_index`.
//!
//! Spec: `docs/design/v1.16-spec.md` §1.ii.3 (dict/callable) + §1.iii.3
//! (inplace guards) + §1.iii.5.
//!
//! Behavior contract:
//!
//! - **dict-form** `.agg({"c": "sum", ...})` — synthesize `Derived` = group
//!   keys ++ the named columns at their per-column aggregate dtype (via the
//!   v1.13 aggregate-to-dtype table); columns absent from the dict are
//!   DROPPED (pandas dict-agg keeps only the named columns). Each dict key
//!   is validated against the receiver (D0030 on a typo). A `**spread` /
//!   non-literal key or a per-column list value (`{"c": ["sum","mean"]}`,
//!   MultiIndex) falls through to Unknown.
//! - **callable-form** `.agg(np.mean)` / `.agg(len)` — the callable applies
//!   to ALL non-key columns. The output column set is knowable (keys ++
//!   non-key columns) but the dtype is NOT (a bare callable has no allowlist
//!   entry). Design decision (§1.ii.3): the columns EXIST at Unknown dtype
//!   so downstream `result["typo"]` still fires D0030, while pykrete never
//!   claims a dtype it can't justify (no D0080 on a bare callable's cols).
//! - **list-of-aggfunc** `.agg(["sum","mean"])` — MultiIndex columns the
//!   flat `Derived` model can't represent; DEFERRED to v1.17. Explicit
//!   fall-through to Unknown, asserted to produce no silent-wrong-schema.
//! - **inplace guards** — pandas-stubs type `reset_index(..., inplace=True)`
//!   and `set_index(..., inplace=True)` as `-> None`; the v1.15 arms wrongly
//!   synthesized a `Derived`, OVERRIDING the engine's correct `None`. The
//!   guards fall through on `inplace=True` (append-don't-change). The
//!   `set_index` guard sits AFTER the D0030 key-existence check so a typoed
//!   key still fires, and BEFORE synthesis so no schema is produced.
//! - **mixed-literal** `set_index(["k", some_var])` — the `parse_string_list`
//!   short-circuit falls the whole arg through to Unknown (no partial
//!   validation).

#![allow(non_snake_case)]

mod common;
use common::*;

// ===========================================================================
// dict-form — PROBE-RESOLVES: Out matches the synthesized per-column dtypes,
// unlisted columns dropped. No diagnostics.
// ===========================================================================

#[test]
fn V116D2_dict_sum_preserves_dtype() {
    // sum preserves amount: double; count_col is NOT in the dict → dropped.
    let src = "\
class In(Schema):
    k: string
    amount: double
    count_col: int

class Out(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg({'amount': 'sum'})
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V116D2_dict_count_overrides_long() {
    let src = "\
class In(Schema):
    k: string
    amount: double
    count_col: int

class Out(Schema):
    k: string
    amount: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg({'amount': 'count'})
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V116D2_dict_mean_overrides_double() {
    // mean overrides count_col: int → double; amount dropped (not in dict).
    let src = "\
class In(Schema):
    k: string
    amount: double
    count_col: int

class Out(Schema):
    k: string
    count_col: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg({'count_col': 'mean'})
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V116D2_dict_multi_col_per_column_dtype() {
    // Each dict column gets its OWN aggregate dtype: sum preserves amount
    // (double), mean overrides count_col (int → double).
    let src = "\
class In(Schema):
    k: string
    amount: double
    count_col: int

class Out(Schema):
    k: string
    amount: double
    count_col: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg({'amount': 'sum', 'count_col': 'mean'})
";
    assert_no_diagnostics(&check(src));
}

// ===========================================================================
// dict-form — D0080-fire per dtype family (synthesis proof, v1.14 retro §8).
// Declaring the value column with a type that crosses the numeric boundary
// fires D0080 ONLY when the per-column synthesis actually ran.
// ===========================================================================

#[test]
fn V116D2_dict_sum_preserve_declared_string_fires_D0080() {
    // Preserve family: sum keeps amount: double; declaring it string crosses
    // the numeric/non-numeric boundary → D0080.
    let src = "\
class In(Schema):
    k: string
    amount: double

class Out(Schema):
    k: string
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg({'amount': 'sum'})
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn V116D2_dict_count_long_declared_string_fires_D0080() {
    // Long family: count overrides amount → long; declared string → D0080.
    let src = "\
class In(Schema):
    k: string
    amount: double

class Out(Schema):
    k: string
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg({'amount': 'count'})
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn V116D2_dict_mean_double_declared_string_fires_D0080() {
    // Double family: mean overrides amount → double; declared string → D0080.
    let src = "\
class In(Schema):
    k: string
    amount: double

class Out(Schema):
    k: string
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg({'amount': 'mean'})
";
    assert_has_code(&check(src), "D0080");
}

// ===========================================================================
// dict-form — drops unlisted columns.
// ===========================================================================

#[test]
fn V116D2_dict_drops_unlisted_column_downstream_typo_fires_D0030() {
    // count_col is not in the dict → dropped from the synthesized schema;
    // a downstream access fires D0030.
    let src = "\
class In(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg({'amount': 'sum'})
    return result['count_col']
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D2_dict_drops_unlisted_column_return_shape_fires_D0050() {
    // Return declares count_col but dict-agg dropped it → column-set
    // mismatch (D0050), proving the drop.
    let src = "\
class In(Schema):
    k: string
    amount: double
    count_col: int

class Out(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg({'amount': 'sum'})
";
    assert_has_code(&check(src), "D0050");
}

// ===========================================================================
// dict-form — key validation + downstream resolution + nested-list defer.
// ===========================================================================

#[test]
fn V116D2_dict_typo_key_fires_D0030() {
    // A dict key naming a nonexistent column fires D0030 (keys are validated
    // against the receiver before synthesis).
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    return pdf.groupby('k').agg({'typo': 'sum'})
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D2_dict_downstream_valid_access_resolves() {
    // Group key + a dict column both survive; valid downstream access is
    // clean (no D0030).
    let src = "\
class In(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg({'amount': 'sum'})
    return (result['k'], result['amount'])
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D2_dict_per_column_list_value_falls_through() {
    // `{"amount": ["sum","mean"]}` → per-column MultiIndex; the flat model
    // can't represent it → fall through to Unknown (no synthesis, so a
    // downstream typo does NOT fire).
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg({'amount': ['sum', 'mean']})
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D2_dict_typo_key_with_list_value_still_fires_D0030() {
    // A typoed dict KEY must fire D0030 even when its VALUE is a deferred
    // MultiIndex list — key existence is validated before the shape-punt.
    // `result['nope']` does NOT fire (the list value punts synthesis →
    // Unknown), so exactly ONE D0030 proves both: key validated AND no
    // synthesis. Mirrors the set_index inplace guard's report-before-punt.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg({'typo': ['sum', 'mean']})
    return result['nope']
";
    assert_count(&check(src), "D0030", 1);
}

// ===========================================================================
// callable-form — columns exist at Unknown dtype: downstream typos catchable
// (D0030), but no dtype claimed (no D0080 on the aggregated columns).
// ===========================================================================

#[test]
fn V116D2_callable_np_mean_columns_exist_resolve() {
    let src = "\
import numpy as np

class In(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg(np.mean)
    return (result['k'], result['amount'], result['count_col'])
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D2_callable_len_bare_name_columns_exist() {
    // Bare-Name builtin callable — `len` applies to all non-key columns.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg(len)
    return result['amount']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D2_callable_downstream_typo_fires_D0030() {
    // Load-bearing: the callable synthesizes a column SET, so a typo on the
    // result fires D0030 (the value of "columns exist, dtype Unknown").
    let src = "\
import numpy as np

class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg(np.mean)
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D2_callable_len_downstream_typo_fires_D0030() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg(len)
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D2_callable_dtype_unknown_no_D0080() {
    // A bare callable claims NO dtype: every non-key column is Unknown, so
    // declaring them `string` does NOT fire D0080 (Unknown is permissive).
    // Contrast: `.agg('mean')` would override amount → double and fire
    // D0080 against this same Out. Proves the callable path is honest.
    let src = "\
import numpy as np

class In(Schema):
    k: string
    amount: double
    count_col: int

class Out(Schema):
    k: string
    amount: string
    count_col: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg(np.mean)
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn V116D2_callable_nonnumeric_honest_silence_no_D0050() {
    // v1.16 PR-D4 SUPERSEDES the old over-approximation pin. A bare callable
    // is opaque: `np.mean` RAISES on the non-numeric `name` column in pandas
    // 2.x (nuisance-column dropping was removed in 2.0), while `len` would
    // keep it — undecidable statically. So the callable arm now declines the
    // whole chain to Unknown (honest silence) whenever any non-key column is
    // non-numeric. Synthesized {k, amount, name} used to false-fire D0050
    // "extra in body: [name]" against this correctly-narrowed `Out`; Unknown
    // now accepts it. PINNED (flipped) so the honest-silence contract can't
    // silently regress. Uniform with the D4 string-arm and D3 rolling gates.
    let src = "\
import numpy as np

class In(Schema):
    k: string
    amount: double
    name: string

class Out(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg(np.mean)
";
    assert_does_not_have_code(&check(src), "D0050");
}

// ===========================================================================
// list-of-aggfunc — DEFERRED (MultiIndex columns, v1.17). Explicit
// fall-through to Unknown; no silent-wrong-schema.
// ===========================================================================

#[test]
fn V116D2_list_of_aggfunc_falls_through_downstream_typo_no_D0030() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg(['sum', 'mean'])
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D2_list_of_aggfunc_no_silent_wrong_schema_no_D0080() {
    // The chain stays Unknown, so a mismatched return type does NOT fire —
    // the deferral must not masquerade as a concrete (wrong) schema.
    let src = "\
class In(Schema):
    k: string
    amount: double

class Out(Schema):
    k: string
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg(['sum', 'mean'])
";
    assert_does_not_have_code(&check(src), "D0080");
}

// ===========================================================================
// inplace guards — reset_index. `reset_index(drop=True, inplace=True)` binds
// None in pandas; pykrete must fall through, NOT synthesize (append-don't-
// change). Paired with the non-inplace positive that DOES synthesize.
// ===========================================================================

#[test]
fn V116D2_reset_index_drop_true_inplace_true_no_synthesis() {
    // inplace=True → Unknown (engine's None stands). A downstream typo does
    // NOT fire, proving no synthesis.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.reset_index(drop=True, inplace=True)
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D2_reset_index_drop_true_without_inplace_synthesizes() {
    // Paired positive: WITHOUT inplace, reset_index(drop=True) synthesizes,
    // so the same downstream typo DOES fire. Confirms the guard is what
    // suppresses synthesis above.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.reset_index(drop=True)
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

// ===========================================================================
// inplace guards — set_index. The guard sits AFTER the D0030 key-existence
// check (typo still fires) and BEFORE synthesis (no schema produced).
// ===========================================================================

#[test]
fn V116D2_set_index_typo_inplace_true_still_fires_D0030() {
    // A typoed key must still fire D0030 regardless of inplace — the guard
    // is placed after report_column_refs.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    return pdf.set_index(['typo'], inplace=True)
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D2_set_index_valid_key_inplace_true_no_synthesis() {
    // inplace=True with a VALID key: no synthesis, so the key is NOT dropped
    // from a synthesized schema — `result['k']` does NOT fire D0030 (the
    // chain is Unknown). If the guard were missing, synthesis would strip
    // `k` and this access would wrongly fire.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.set_index(['k'], inplace=True)
    return result['k']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D2_set_index_valid_key_no_inplace_synthesizes() {
    // Paired positive: WITHOUT inplace, set_index(['k']) synthesizes and
    // strips `k`, so `result['k']` DOES fire D0030. Confirms the guard is
    // what suppresses synthesis above.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.set_index(['k'])
    return result['k']
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V116D2_set_index_single_string_inplace_true_no_synthesis() {
    // Single-string positional key + inplace=True → no synthesis (Unknown).
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.set_index('k', inplace=True)
    return result['k']
";
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// mixed-literal set_index (spec §1.iii.5) — a list mixing a string literal
// and a non-literal (`["k", some_var]`) short-circuits `parse_string_list`
// and falls the whole arg through to Unknown. No synthesis, no partial
// validation.
// ===========================================================================

#[test]
fn V116D2_set_index_mixed_literal_falls_through() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In], some_var: str):
    result = pdf.set_index(['k', some_var])
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V116D2_set_index_mixed_literal_no_partial_validation() {
    // The literal member (`'typo'`) is NOT validated when a sibling is
    // non-literal — the whole list must be literal to enter the arm.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In], some_var: str):
    return pdf.set_index([some_var, 'typo'])
";
    assert_does_not_have_code(&check(src), "D0030");
}
