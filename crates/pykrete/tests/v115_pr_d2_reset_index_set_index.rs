//! v1.15 PR-D2 — `pdf.reset_index(drop=True)` + `pdf.set_index([keys])`
//! arms. **Closes the v1.14 PR-D2 groupby.agg chain-depth carve-out:**
//! `pdf.groupby('k').agg('sum').reset_index(drop=True)` (the canonical
//! pandas idiom) now tracks the Derived envelope two methods deep, so
//! downstream column access resolves against the synthesized fields and
//! a typo fires D0030.
//!
//! Spec: `docs/design/v1.15-spec.md` §5 + §1.iii.
//!
//! Behavior contract:
//!
//! - `reset_index(drop=True)` on a Pandas receiver → Derived with the
//!   receiver's fields (no shape change; pandas drops the index without
//!   promoting it to a column).
//! - `reset_index(drop=False)` / missing kwarg / non-bool-literal →
//!   explicit fall-through to Unknown (v1.16 candidate; needs stateful
//!   index tracking).
//! - `set_index(["k1", "k2", ...])` / `set_index("k")` on a Pandas
//!   receiver → Derived with the receiver's fields MINUS the literal
//!   key names (pandas moves them to the index; they're no longer
//!   accessible via `df["key"]`). Each literal key is validated against
//!   the receiver (D0030 on typo).
//! - `set_index(<bare-name>)` / `set_index(<expr>)` (non-literal) →
//!   explicit fall-through to Unknown.
//! - Spark receivers don't have `reset_index` / `set_index` — D0091
//!   fires via the existing cross-dialect machinery; the synthesis arms
//!   never run.

#![allow(non_snake_case)]

mod common;
use common::*;

// ===========================================================================
// reset_index(drop=True) — positive: chain continues on Declared/Derived
// receiver; downstream column access resolves.
// ===========================================================================

#[test]
fn V115D2_reset_index_drop_true_passthrough_on_declared() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[In]:
    return pdf.reset_index(drop=True)
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V115D2_reset_index_drop_true_on_groupby_agg_chain_resolves_downstream() {
    // The canonical pandas idiom: groupby.agg().reset_index(drop=True).
    // The v1.14 PR-D2 synthesized Derived envelope (keys ++ value-cols)
    // must survive `.reset_index(drop=True)` so downstream column refs
    // resolve.
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
    return pdf.groupby('k').agg('sum').reset_index(drop=True)
";
    assert_does_not_have_code(&check(src), "D0080");
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V115D2_reset_index_drop_true_preserves_synthesized_dtype_override() {
    // Chain `.agg('count').reset_index(drop=True)`: count overrides
    // amount → Long. The Out schema declares amount: long; if the
    // synthesized envelope were lost at reset_index, the downstream
    // return-shape check would degrade silently (Unknown isn't checked).
    // Declaring amount: string here would cross the numeric boundary
    // and fire D0080 ONLY when the synthesized Derived survived.
    let src = "\
class In(Schema):
    k: string
    amount: double

class Out(Schema):
    k: string
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.groupby('k').agg('count').reset_index(drop=True)
";
    assert_has_code(&check(src), "D0080");
}

// ===========================================================================
// reset_index(drop=True) — negative: downstream typo fires D0030 against
// the synthesized envelope.
// ===========================================================================

#[test]
fn V115D2_reset_index_drop_true_downstream_typo_fires_D0030() {
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

#[test]
fn V115D2_reset_index_drop_true_after_agg_downstream_typo_fires_D0030() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg('sum').reset_index(drop=True)
    return result['typo']
";
    assert_has_code(&check(src), "D0030");
}

// ===========================================================================
// reset_index — negative-space: fall-through shapes (drop=False, missing,
// non-bool-literal) return Unknown and don't carry the receiver schema.
// Asserted by: downstream typo does NOT fire D0030 (silent Unknown is
// the v1.x contract for unmodeled shapes).
// ===========================================================================

#[test]
fn V115D2_reset_index_drop_false_falls_through() {
    // drop=False: pandas promotes the index to a NEW column. v1.16
    // candidate. Today: Unknown, no D0030 on downstream typo.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.reset_index(drop=False)
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V115D2_reset_index_missing_kwarg_falls_through() {
    // Missing `drop=` defaults to False in pandas (same fall-through as
    // explicit `drop=False`).
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.reset_index()
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V115D2_reset_index_non_bool_literal_drop_falls_through() {
    // Non-bool-literal (a Name binding) is unresolvable at static-check
    // time → fall-through to Unknown.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In], flag: bool):
    result = pdf.reset_index(drop=flag)
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// set_index — positive: literal keys removed from the synthesized envelope.
// Single-string positional and list-of-strings are both supported.
// ===========================================================================

#[test]
fn V115D2_set_index_single_string_removes_key() {
    let src = "\
class In(Schema):
    k: string
    amount: double

class Out(Schema):
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.set_index('k')
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V115D2_set_index_list_single_key_removes_key() {
    let src = "\
class In(Schema):
    k: string
    amount: double

class Out(Schema):
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.set_index(['k'])
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn V115D2_set_index_list_multi_keys_removes_all_keys() {
    let src = "\
class In(Schema):
    k1: string
    k2: int
    amount: double

class Out(Schema):
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.set_index(['k1', 'k2'])
";
    assert_no_diagnostics(&check(src));
}

// ===========================================================================
// set_index — negative: downstream access to a removed key fires D0030;
// typo on a literal key arg fires D0030 against the receiver.
// ===========================================================================

#[test]
fn V115D2_set_index_then_access_removed_key_fires_D0030() {
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
fn V115D2_set_index_multi_keys_then_access_either_removed_key_fires_D0030() {
    let src = "\
class In(Schema):
    k1: string
    k2: int
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.set_index(['k1', 'k2'])
    return (result['k1'], result['k2'])
";
    // Expect 2 D0030s, one per removed-key access.
    assert_count(&check(src), "D0030", 2);
}

#[test]
fn V115D2_set_index_typo_on_literal_key_fires_D0030() {
    // The literal key arg itself is validated against the receiver
    // (D0030 on typo) before subtraction.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    return pdf.set_index(['typo'])
";
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V115D2_set_index_single_string_typo_fires_D0030() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    return pdf.set_index('typo')
";
    assert_has_code(&check(src), "D0030");
}

// ===========================================================================
// set_index — negative-space: non-literal arg shapes fall through to
// Unknown. Asserted by: downstream typo does NOT fire D0030 (the chain
// degrades silently per the v1.x scope; v1.16 owns expr-eval).
// ===========================================================================

#[test]
fn V115D2_set_index_bare_name_arg_falls_through() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In], key: str):
    result = pdf.set_index(key)
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V115D2_set_index_empty_call_falls_through() {
    // `set_index()` with no args — degenerate but valid Python; arm
    // bails to None.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.set_index()
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V115D2_set_index_dict_arg_falls_through() {
    // A dict-shape arg is non-literal-for-our-parser; fall-through.
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.set_index({'k': 1})
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// Cross-dialect: Spark receivers must NOT trigger synthesis. The existing
// D0091 cross-dialect machinery handles surface-mismatch warnings; the
// synthesis arms here are gated on `receiver_is_pandas_inherited` so a
// Spark receiver flows past without re-shaping its schema.
// ===========================================================================

#[test]
fn V115D2_reset_index_on_spark_receiver_does_not_synthesize() {
    // sdf has no `reset_index`; D0091 fires (cross-dialect warning). We
    // assert the synthesis arms didn't accidentally fire by checking
    // that no spurious downstream D0030 is missed — i.e., the chain dies
    // at Unknown, and a downstream typo against the (lost) schema does
    // NOT raise D0030. (D0091 still fires, which we don't assert here.)
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(sdf: SparkFrame[In]):
    result = sdf.reset_index(drop=True)
    return result['typo']
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn V115D2_set_index_on_spark_receiver_does_not_synthesize() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(sdf: SparkFrame[In]):
    result = sdf.set_index(['k'])
    return result['amount']
";
    // Spark synthesis-arms never run; the chain is Unknown after
    // `set_index`. Downstream `result['amount']` does NOT fire D0030
    // because Unknown can't refute it.
    assert_does_not_have_code(&check(src), "D0030");
}

// ===========================================================================
// Chain composition: set_index → reset_index(drop=True) round-trips
// only loosely — set_index removes keys, reset_index(drop=True) doesn't
// re-add them (because drop=True). Document the current behavior.
// ===========================================================================

#[test]
fn V115D2_set_index_then_reset_index_drop_true_does_not_restore_key() {
    let src = "\
class In(Schema):
    k: string
    amount: double

def f(pdf: PandasFrame[In]):
    result = pdf.set_index(['k']).reset_index(drop=True)
    return result['k']
";
    // `k` removed at set_index and NOT restored by reset_index(drop=True).
    assert_has_code(&check(src), "D0030");
}

#[test]
fn V115D2_groupby_agg_reset_index_drop_true_downstream_column_access() {
    // End-to-end: the canonical pandas idiom plus downstream column
    // access. Every reference resolves against the synthesized envelope.
    let src = "\
class In(Schema):
    k: string
    amount: double
    count_col: int

def f(pdf: PandasFrame[In]):
    result = pdf.groupby('k').agg('sum').reset_index(drop=True)
    return (result['k'], result['amount'], result['count_col'])
";
    assert_no_diagnostics(&check(src));
}
