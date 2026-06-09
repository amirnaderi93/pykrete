//! v1.5 PR-E — synthetic-pool soft-cap + warn-and-saturate.
//!
//! Spec §6: pool reaches cap → emit one-shot stderr warning → return
//! sentinel `__pykrete_pool_full__` for further inserts. Pre-cap entries
//! continue to resolve via the existing HashSet lookup arm. LSP keeps
//! running degraded; no panic.
//!
//! Hard-cap-with-panic was explicitly rejected as an LSP-crash regression
//! vs. v1.4's leak-but-live baseline; the soft-cap is the user-level
//! product decision (see PR #116 round-3).

use std::sync::Mutex;

use pykrete::operations::{
    intern_synthetic_for_test, pool_full_warned_for_test, reset_synthetic_pool_for_test,
    synthetic_pool_len, synthetic_pool_sentinel,
};

/// The pool + warned-flag are process-global. Each test resets them
/// upfront; this mutex serializes the resets so tests don't race.
static POOL_TEST_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn pre_fill_then_overshoot_no_panic_and_size_pinned_at_cap() {
    let _guard = POOL_TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    reset_synthetic_pool_for_test();

    let cap = 32usize;
    for i in 0..cap {
        let s = intern_synthetic_for_test(format!("pre_cap_name_{i}"), cap);
        assert_ne!(s, synthetic_pool_sentinel());
    }
    assert_eq!(synthetic_pool_len(), cap);
    assert!(
        !pool_full_warned_for_test(),
        "warning fired before cap reached"
    );

    for i in 0..100 {
        let s = intern_synthetic_for_test(format!("post_cap_name_{i}"), cap);
        assert_eq!(
            s,
            synthetic_pool_sentinel(),
            "post-cap insert did not return the sentinel"
        );
    }

    assert_eq!(
        synthetic_pool_len(),
        cap,
        "pool grew past cap — the soft-cap is supposed to saturate, not insert"
    );
    assert!(
        pool_full_warned_for_test(),
        "one-shot warning never fired after reaching cap"
    );
}

#[test]
fn pre_cap_entries_still_resolve_to_original_static_str() {
    let _guard = POOL_TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    reset_synthetic_pool_for_test();

    let cap = 16usize;
    let original = intern_synthetic_for_test("sum(amount)".to_string(), cap);
    assert_ne!(original, synthetic_pool_sentinel());

    for i in 0..(cap - 1) {
        let _ = intern_synthetic_for_test(format!("filler_{i}"), cap);
    }
    assert_eq!(synthetic_pool_len(), cap);

    // Push past the cap so saturation kicks in.
    let saturated = intern_synthetic_for_test("avg(price)".to_string(), cap);
    assert_eq!(saturated, synthetic_pool_sentinel());

    // The original pre-cap entry must still resolve to its original
    // pointer, not the sentinel — the lookup arm runs before the cap
    // check.
    let re_lookup = intern_synthetic_for_test("sum(amount)".to_string(), cap);
    assert_eq!(
        re_lookup, original,
        "pre-cap entry no longer resolves to its original &'static str"
    );
    assert!(
        std::ptr::eq(re_lookup.as_ptr(), original.as_ptr()),
        "pre-cap re-lookup returned a different allocation than the original leak"
    );
}

#[test]
fn post_cap_insert_returns_named_sentinel() {
    let _guard = POOL_TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
    reset_synthetic_pool_for_test();

    let cap = 4usize;
    for i in 0..cap {
        let _ = intern_synthetic_for_test(format!("n{i}"), cap);
    }

    let s = intern_synthetic_for_test("would_have_leaked".to_string(), cap);
    assert_eq!(s, "__pykrete_pool_full__");
    assert_eq!(s, synthetic_pool_sentinel());

    // The warning is one-shot: a second post-cap insert must not flip
    // the flag a second time (the AtomicBool::swap returns the old
    // value, so a duplicate fire would have stderr-spammed in prod).
    assert!(pool_full_warned_for_test());
    let s2 = intern_synthetic_for_test("would_also_have_leaked".to_string(), cap);
    assert_eq!(s2, synthetic_pool_sentinel());
    assert!(
        pool_full_warned_for_test(),
        "warned-flag was reset between post-cap inserts"
    );
}
