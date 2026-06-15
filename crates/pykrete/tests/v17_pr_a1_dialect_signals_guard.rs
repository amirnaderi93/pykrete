//! v1.7 PR-A1 — `dialect_signals` cross-consistency CI guard.
//!
//! The v1.6 architecture audit (Important #3) flagged a drift hazard:
//! three separate sources of truth for "what is a pandas-side
//! method" (`PANDAS_DISCRIMINATORS` in `alias_adjudicate.rs`,
//! `SPARK_DISCRIMINATORS` in the same file, and inline `matches!`
//! arms guarded by `receiver_is_pandas_inherited` in
//! `operations/expr.rs`). Adding a new pandas-dispatched arm in
//! `expr.rs` did not force the discriminator lists to update, and
//! vice versa. PR-A1 collected the lists into the shared
//! `dialect_signals` module; this test enforces that every method
//! named in an `expr.rs` pandas-dispatched arm lives in EITHER
//! [`PANDAS_ONLY_SIGNALS`] OR [`PANDAS_INHERITED_ARMS`] — never
//! neither.
//!
//! The v1.7 expr.rs arm inventory is hardcoded here. If a future
//! contributor adds an `if receiver_is_pandas_inherited && ... method ...`
//! arm without updating the shared lists, they're expected to update
//! this inventory too — at which point the membership assertions
//! below fail loudly, naming the offending method. This isn't perfect
//! drift coverage (a contributor could remove a method from expr.rs
//! without touching the inventory), but it catches the actual
//! audit-flagged failure mode: "added pandas arm, forgot to update
//! vocabulary list".

use pykrete::{PANDAS_INHERITED_ARMS, PANDAS_ONLY_SIGNALS};

/// Every method name appearing in an `expr.rs` arm gated by
/// `receiver_is_pandas_inherited`. Verified at v1.7 PR-A1 against
/// `operations/expr.rs` lines 874 (head/tail/first/take pass-through),
/// 1067 (rename), 1078 (assign), 1095 (drop columns kwarg), 1114
/// (drop positional), 1130 (pivot_table).
const PANDAS_INHERITED_ARM_METHODS: &[&str] = &[
    "head",
    "tail",
    "first",
    "take",
    "rename",
    "assign",
    "drop",
    "pivot_table",
];

/// Every method in the `expr.rs` pandas-arm inventory above must
/// appear in either [`PANDAS_ONLY_SIGNALS`] or [`PANDAS_INHERITED_ARMS`].
/// The failure mode this catches: a contributor adds a new
/// `receiver_is_pandas_inherited && method == "M"` arm to `expr.rs`,
/// updates this inventory, but forgets to add `M` to either list.
#[test]
fn V17PRA1_every_arm_method_is_listed() {
    for method in PANDAS_INHERITED_ARM_METHODS {
        let in_only = PANDAS_ONLY_SIGNALS.contains(method);
        let in_inherited = PANDAS_INHERITED_ARMS.contains(method);
        assert!(
            in_only || in_inherited,
            "expr.rs uses method '{method}' in a pandas-dispatched arm, but \
             '{method}' is in neither PANDAS_ONLY_SIGNALS nor \
             PANDAS_INHERITED_ARMS. Add it to one (or the test inventory \
             above is stale and should be updated to match expr.rs)."
        );
    }
}

/// Disjointness — a method either tags the binding (pandas-only name)
/// or shares the dispatch surface (inherited from Spark). It can't do
/// both; that's the semantic split documented in
/// `crates/pykrete/src/dialect_signals.rs`. If `head` ever leaks into
/// `PANDAS_ONLY_SIGNALS`, every Spark codebase calling `df.head()`
/// suddenly classifies as Pandas (false-positive D0090 adjudication);
/// if `assign` ever leaks into `PANDAS_INHERITED_ARMS`, the meaning of
/// the list — "shared-with-Spark" — is corrupted.
#[test]
fn V17PRA1_only_signals_and_inherited_arms_are_disjoint() {
    for method in PANDAS_ONLY_SIGNALS {
        assert!(
            !PANDAS_INHERITED_ARMS.contains(method),
            "'{method}' appears in both PANDAS_ONLY_SIGNALS and \
             PANDAS_INHERITED_ARMS. The two lists must be disjoint per \
             the dialect_signals module doc — pandas-only names tag \
             bindings; shared-with-Spark names route dispatch arms."
        );
    }
}

/// Sanity pin: the inventory list above is what the v1.7 PR-A1
/// CI-guard considers "complete". A contributor adding a new arm in
/// `expr.rs` updates this number too; a contributor who forgets is
/// the failure mode this whole file exists to catch. Pinning the
/// length surfaces the "you added an arm, did you update the
/// inventory?" question in code review.
#[test]
fn V17PRA1_inventory_has_eight_methods() {
    assert_eq!(
        PANDAS_INHERITED_ARM_METHODS.len(),
        8,
        "expr.rs pandas-arm inventory pinned at 8 methods as of v1.7 \
         PR-A1; if you're seeing this fail you've added/removed an arm \
         in operations/expr.rs and the inventory above needs the same \
         update."
    );
}

/// Regression guard for the load-bearing case: `head` MUST live in
/// `PANDAS_INHERITED_ARMS` only. The v1.5 PR-A3 / v1.6 PR-A2 schema
/// preservation through `pdf.head().assign(...)` depends on the
/// expr.rs arm at line 874 firing — which references the list. If
/// `head` accidentally moves to `PANDAS_ONLY_SIGNALS` the dispatch
/// classifies Spark `df.head()` as Pandas and the schema mutates
/// silently.
#[test]
fn V17PRA1_head_is_only_in_inherited_arms() {
    assert!(
        PANDAS_INHERITED_ARMS.contains(&"head"),
        "head must remain in PANDAS_INHERITED_ARMS — the expr.rs arm \
         depends on it for the schema pass-through.",
    );
    assert!(
        !PANDAS_ONLY_SIGNALS.contains(&"head"),
        "head must NOT be in PANDAS_ONLY_SIGNALS — Spark `df.head()` \
         exists too; classifying on `head` would false-positive D0090 \
         on every Spark codebase.",
    );
}

/// Regression guard for the other load-bearing case: `assign` MUST
/// live in `PANDAS_ONLY_SIGNALS` only. The adjudicator at
/// `alias_adjudicate.rs:222`/`:231` walks downstream usage and tags a
/// binding as Pandas if it sees a pandas-only signal. `assign` is the
/// canonical example.
#[test]
fn V17PRA1_assign_is_only_in_pandas_only_signals() {
    assert!(
        PANDAS_ONLY_SIGNALS.contains(&"assign"),
        "assign must remain in PANDAS_ONLY_SIGNALS — the call-graph \
         adjudicator relies on it to classify pandas bindings.",
    );
    assert!(
        !PANDAS_INHERITED_ARMS.contains(&"assign"),
        "assign must NOT be in PANDAS_INHERITED_ARMS — it has no Spark \
         counterpart; the list is for shared-with-Spark names only.",
    );
}
