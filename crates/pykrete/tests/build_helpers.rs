//! Integration tests for the pure-logic helpers behind `build.rs`.
//!
//! Migrated from `build.rs::tests` (v1.8 PR-A1 + v1.9 PR-A1 — six +
//! two = eight cases). The inline `#[cfg(test)] mod tests` inside a
//! build script does NOT run under `cargo test --workspace` — build
//! scripts are compiled standalone, and their test modules are dead
//! code from the test harness's perspective. v1.9 PR-A1 R2 review
//! caught this: the arch-I1 `#[should_panic]` tripwire existed in
//! source but never actually fired in CI.
//!
//! Pure-logic helpers live in `crates/pykrete/src/build_helpers.rs`;
//! `build.rs` includes them via `#[path]` at build time and `lib.rs`
//! re-exports the module as `#[doc(hidden)] pub mod build_helpers;`.

use pykrete::build_helpers::{extract_methods, preceded_by_let, strip_line_comments};

#[test]
fn extracts_matches_arm() {
    let src = r#"
        if receiver_is_pandas_inherited
            && matches!(method, "head" | "tail" | "first" | "take")
            && PANDAS_INHERITED_ARMS.contains(&method)
        {
            return Some(receiver);
        }
    "#;
    let mut got = extract_methods(src);
    got.sort();
    assert_eq!(got, vec!["first", "head", "tail", "take"]);
}

#[test]
fn extracts_method_eq_inline() {
    let src = r#"
        if receiver_is_pandas_inherited && method == "assign" {
            return None;
        }
    "#;
    let got = extract_methods(src);
    assert_eq!(got, vec!["assign"]);
}

#[test]
fn extracts_method_eq_multiline() {
    let src = r#"
        if receiver_is_pandas_inherited
            && method == "rename"
            && let Some(dict) = pandas_dict_kwarg(call, "columns")
        {
            return Some(apply_rename_dict(dict, &receiver, ctx));
        }
    "#;
    let got = extract_methods(src);
    assert_eq!(got, vec!["rename"]);
}

#[test]
fn ignores_binding_line() {
    // The `let receiver_is_pandas_inherited = ...` line is the
    // definition of the gate, not an arm. Must not panic, must not
    // collect.
    let src = r#"
        let receiver_is_pandas_inherited = receiver_dialect == Some(Dialect::Pandas);
        if receiver_is_pandas_inherited && method == "melt" {
            return None;
        }
    "#;
    let got = extract_methods(src);
    assert_eq!(got, vec!["melt"]);
}

#[test]
fn negative_space_simulated_new_arm_grows_inventory() {
    // v1.8 spec §2.1 negative-space test: a contributor adding a new
    // arm in `expr.rs` causes the extraction to grow.
    let baseline = r#"
        if receiver_is_pandas_inherited
            && matches!(method, "head" | "tail")
        { return Some(receiver); }
        if receiver_is_pandas_inherited && method == "assign" { return None; }
    "#;
    let added = r#"
        if receiver_is_pandas_inherited
            && matches!(method, "head" | "tail")
        { return Some(receiver); }
        if receiver_is_pandas_inherited && method == "assign" { return None; }
        if receiver_is_pandas_inherited && method == "foobar" { return None; }
    "#;
    let mut base = extract_methods(baseline);
    let mut grew = extract_methods(added);
    base.sort();
    grew.sort();
    assert_eq!(base, vec!["assign", "head", "tail"]);
    assert_eq!(grew, vec!["assign", "foobar", "head", "tail"]);
    assert_eq!(grew.len(), base.len() + 1);
}

#[test]
fn negative_space_simulated_removed_arm_shrinks_inventory() {
    // Mirror of the above — removing an arm shrinks the extraction.
    let with_arm = r#"
        if receiver_is_pandas_inherited && method == "melt" { return None; }
        if receiver_is_pandas_inherited && method == "assign" { return None; }
    "#;
    let without_arm = r#"
        if receiver_is_pandas_inherited && method == "assign" { return None; }
    "#;
    let mut a = extract_methods(with_arm);
    let mut b = extract_methods(without_arm);
    a.sort();
    b.sort();
    assert_eq!(a, vec!["assign", "melt"]);
    assert_eq!(b, vec!["assign"]);
    assert_eq!(a.len(), b.len() + 1);
}

#[test]
fn ignores_method_literals_inside_line_comments() {
    // The tripwire comment in expr.rs documents the build.rs's
    // matched arm shapes — and quotes the patterns. The build.rs
    // must NOT extract method names from inside line comments.
    let src = r#"
        // receiver_is_pandas_inherited && matches!(method, "FAKE_A" | "FAKE_B")
        // receiver_is_pandas_inherited && method == "FAKE_C"
        if receiver_is_pandas_inherited && method == "real" { return None; }
    "#;
    let stripped = strip_line_comments(src);
    let got = extract_methods(&stripped);
    assert_eq!(got, vec!["real"]);
}

#[test]
fn strip_line_comments_preserves_string_literals_with_slashes() {
    let src = r#"let s = "// not a comment";
let real = "v";"#;
    let stripped = strip_line_comments(src);
    assert!(stripped.contains("\"// not a comment\""));
    assert!(stripped.contains("\"v\""));
}

// v1.9 PR-A1 — negative-space tests for the §2.1.1 panic-skip hole.
//
// The v1.8 skip-clause `region.contains('=')` was too loose: any
// future arm shape containing `=` anywhere in the predicate region
// (e.g. `&& let Some(kw) = pandas_kwarg(...)` with no method-name
// predicate) silently skipped the panic AND silently skipped
// inventory extraction. The new skip-clause is tight to the literal
// `let receiver_is_pandas_inherited` binding-line.

#[test]
#[should_panic(expected = "neither `matches!(method, ...)` nor `method == \"...\"`")]
fn negative_space_let_some_kwarg_arm_without_method_predicate_panics() {
    // A future arm shape with `let Some(kw) = ...` in the predicate
    // region but NO `matches!(method, …)` and NO `method == "…"` is
    // invisible to the generator. Under the v1.8 skip-clause, the
    // `=` from `let Some(kw) =` silently suppressed the panic. With
    // the v1.9 tightening, this MUST panic loudly.
    let src = r#"
        if receiver_is_pandas_inherited
            && let Some(kw) = pandas_kwarg(call, "columns")
        {
            return apply_kw(kw);
        }
    "#;
    let _ = extract_methods(src);
}

#[test]
fn negative_space_binding_line_with_let_some_following_still_works() {
    // Mixed case: the binding line is present, AND a real arm with
    // a normal `method == "X"` shape follows. The binding line is
    // skipped; the real arm is extracted; no panic.
    let src = r#"
        let receiver_is_pandas_inherited = receiver_dialect == Some(Dialect::Pandas);
        if receiver_is_pandas_inherited && method == "melt" {
            return None;
        }
    "#;
    let got = extract_methods(src);
    assert_eq!(got, vec!["melt"]);
}

#[test]
fn extracts_full_v17_inventory_from_synthetic_fixture() {
    // Spec §2.1 positive test: feed in a synthetic fixture mirroring
    // the v1.7-HEAD shape of expr.rs and verify the extractor
    // recovers exactly the 9 methods the hand inventory pins.
    let src = r#"
        let receiver_is_pandas_inherited = receiver_dialect == Some(Dialect::Pandas);
        if receiver_is_pandas_inherited
            && matches!(method, "head" | "tail" | "first" | "take")
        { return Some(receiver); }
        if receiver_is_pandas_inherited
            && method == "rename"
            && let Some(dict) = pandas_dict_kwarg(call, "columns")
        { return Some(apply_rename_dict(dict, &receiver, ctx)); }
        if receiver_is_pandas_inherited && method == "assign" {
            return Some(apply_pandas_assign(call));
        }
        if receiver_is_pandas_inherited
            && method == "drop"
            && let Some(list) = pandas_list_kwarg(call, "columns")
        { return Some(apply_pandas_drop_columns(list)); }
        if receiver_is_pandas_inherited
            && method == "drop"
            && pandas_list_kwarg(call, "columns").is_none()
        { return Some(receiver); }
        if receiver_is_pandas_inherited && method == "pivot_table" { return None; }
        if receiver_is_pandas_inherited && method == "melt" { return None; }
    "#;
    let mut got = extract_methods(src);
    got.sort();
    got.dedup();
    assert_eq!(
        got,
        vec![
            "assign",
            "drop",
            "first",
            "head",
            "melt",
            "pivot_table",
            "rename",
            "tail",
            "take",
        ]
    );
    assert_eq!(got.len(), 9);
}

// v1.9 PR-A1 R2 — word-boundary regression for `preceded_by_let`.
//
// The v1.9 PR-A1 v1 impl matched only `prefix.ends_with("let")` (3
// raw bytes), so a future identifier that happened to end in `let`
// (e.g. `inlet`, `outlet`, `goblet`) would silently skip the panic.
// R2 tightening requires the byte BEFORE `let` to be start-of-file,
// whitespace, or one of `{`/`}`/`;`.

#[test]
fn let_at_word_boundary_only_matches_keyword_let() {
    // Real `let` at a word boundary → matches.
    assert!(preceded_by_let("let foo", 4));
    // Newline-separated `let` keyword → matches.
    assert!(preceded_by_let("    let foo", 8));
    // `let` at start of file → matches.
    assert!(preceded_by_let("let", 3));
    // `inlet ` — substring that ends in `let` → MUST NOT match.
    assert!(!preceded_by_let("inlet foo", 6));
    // `outlet ` — another `*let` identifier → MUST NOT match.
    assert!(!preceded_by_let("outlet foo", 7));
    // `goblet ` — yet another → MUST NOT match.
    assert!(!preceded_by_let("goblet foo", 7));
    // Statement-bracket boundaries are also legal word breaks for
    // Rust: `{let`, `;let`, `}let`. Match those.
    assert!(preceded_by_let("{let foo", 5));
    assert!(preceded_by_let(";let foo", 5));
    assert!(preceded_by_let("}let foo", 5));
}

#[test]
#[should_panic(expected = "neither `matches!(method, ...)` nor `method == \"...\"`")]
fn negative_space_inlet_prefix_does_not_bypass_panic() {
    // Simulate a (contrived but possible) source where the gate's
    // identifier is preceded by an `*let` suffix instead of the
    // real `let ` keyword. Under the v1 R1 impl this would have
    // silently skipped; under R2 it MUST panic.
    let src = r#"
        inlet receiver_is_pandas_inherited
            && let Some(kw) = pandas_kwarg(call, "columns")
        {
            return apply_kw(kw);
        }
    "#;
    let _ = extract_methods(src);
}
