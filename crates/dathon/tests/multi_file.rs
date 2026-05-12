//! Multi-file project analysis. Cross-file `Schema` visibility — when one
//! file declares `class Orders(Schema)` and another file's function uses
//! `DataFrame[Orders]`, dathon should resolve the reference.
//!
//! The minimal v0.1 model: all top-level declarations from every supplied
//! file go into a single resolution scope. No `import` statement parsing,
//! no Python-style per-file scoping rules. Real codebases that span many
//! files just pass them all on the CLI (or `dathon check src/**/*.dpy`
//! via shell glob).

#![allow(non_snake_case)]

use dathon::{CheckResult, check_project};

/// Convenience: build a `ProjectCheckResult` from a list of `(path,
/// source)` pairs and return the per-file results indexed in order.
fn check_project_pairs(pairs: &[(&str, &str)]) -> Vec<CheckResult> {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(p, s)| ((*p).to_string(), (*s).to_string()))
        .collect();
    let project = check_project(&owned);
    project.files.into_iter().map(|f| f.result).collect()
}

// ===========================================================================
// Backward compat: single-file behavior is unchanged
// ===========================================================================

#[test]
fn single_file_project_behaves_like_a_single_file_check() {
    let results = check_project_pairs(&[(
        "f.dpy",
        r#"
class Orders(Schema):
    place_code: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code"))
"#,
    )]);
    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert!(r.diagnostics.is_empty(), "expected clean run");
    assert_eq!(r.schema_count, 1);
}

// ===========================================================================
// Cross-file Schema visibility (the headline feature)
// ===========================================================================

#[test]
fn function_in_file_b_can_reference_a_schema_declared_in_file_a() {
    // schemas.dpy declares Orders. pipeline.dpy uses DataFrame[Orders] in
    // a function signature. With multi-file support, Orders is visible to
    // pipeline.dpy and the function's typed signature resolves cleanly —
    // no D0020 ("unknown schema referenced in DataFrame[…]").
    let results = check_project_pairs(&[
        (
            "schemas.dpy",
            r#"
class Orders(Schema):
    place_code: int
    price: int
"#,
        ),
        (
            "pipeline.dpy",
            r#"
def prepare(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code"), col("price"))
"#,
        ),
    ]);

    // schemas.dpy: one schema, no issues.
    assert_eq!(results[0].schema_count, 1);
    assert!(results[0].diagnostics.is_empty());

    // pipeline.dpy: typed function recognized; the cross-file Orders
    // reference resolves; select args check cleanly against Orders.
    assert_eq!(results[1].typed_function_count, 1);
    assert!(
        results[1].diagnostics.is_empty(),
        "pipeline.dpy should be clean: {:?}",
        results[1]
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn d0030_fires_in_file_b_against_a_schema_declared_in_file_a() {
    // The user typoes a column reference; the diagnostic should reference
    // Orders specifically (from file A) even though the typo is in file B.
    let results = check_project_pairs(&[
        (
            "schemas.dpy",
            r#"
class Orders(Schema):
    place_code: int
    price: int
"#,
        ),
        (
            "pipeline.dpy",
            r#"
def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("priec"))
"#,
        ),
    ]);

    assert!(results[1].has_code("D0030"));
    assert!(
        results[1]
            .diagnostics_with_code("D0030")
            .iter()
            .any(|d| { d.message.contains("priec") && d.message.contains("Orders") })
    );
}

#[test]
fn d0020_still_fires_when_a_schema_is_not_declared_in_any_file() {
    // No file declares MysterySchema; the cross-file lookup also fails;
    // dathon emits the existing D0020 ("Unknown schema 'X' referenced in
    // DataFrame[…]").
    let results = check_project_pairs(&[
        (
            "schemas.dpy",
            r#"
class Orders(Schema):
    place_code: int
"#,
        ),
        (
            "pipeline.dpy",
            r#"
def f(raw: DataFrame[MysterySchema]) -> DataFrame[Orders]:
    return raw
"#,
        ),
    ]);
    assert!(results[1].has_code("D0020"));
}

#[test]
fn d0050_return_type_check_works_across_files() {
    // Function in pipeline.dpy declares `-> DataFrame[Orders]` but returns
    // a Derived schema with only 'place_code'. The mismatch against Orders
    // (declared in schemas.dpy) fires D0050 — proving the cross-file
    // resolution carries all the way through.
    let results = check_project_pairs(&[
        (
            "schemas.dpy",
            r#"
class Orders(Schema):
    place_code: int
    price: int
"#,
        ),
        (
            "pipeline.dpy",
            r#"
def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("place_code"))
"#,
        ),
    ]);
    assert!(results[1].has_code("D0050"));
    assert!(
        results[1]
            .diagnostics_with_code("D0050")
            .iter()
            .any(|d| { d.message.contains("price") })
    );
}

// ===========================================================================
// Cross-file nested struct resolution
// ===========================================================================

#[test]
fn nested_schema_can_reference_a_schema_declared_in_another_file() {
    // Address lives in schemas.dpy; User (in users.dpy) has an `address:
    // Address` field. The cross-file Schema lookup makes the nested
    // resolution succeed and dotted access works through it.
    let results = check_project_pairs(&[
        (
            "schemas.dpy",
            r#"
class Address(Schema):
    street: string
    city: string
"#,
        ),
        (
            "users.dpy",
            r#"
class User(Schema):
    name: string
    address: Address

def f(u: DataFrame[User]) -> DataFrame:
    return u.select(col("address.street"))
"#,
        ),
    ]);

    assert!(
        results[1].diagnostics.is_empty(),
        "users.dpy should be clean across cross-file nested resolution: {:?}",
        results[1]
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>()
    );
}

// ===========================================================================
// Resilience: parse error in one file doesn't break others
// ===========================================================================

#[test]
fn parse_error_in_one_file_does_not_block_analysis_of_other_files() {
    // bad.dpy has invalid Python; good.dpy is clean. Analyzing them
    // together should still produce a usable result for good.dpy.
    let results = check_project_pairs(&[
        (
            "good.dpy",
            r#"
class Orders(Schema):
    x: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("x"))
"#,
        ),
        (
            "bad.dpy",
            r#"
def broken(:
    pass
"#,
        ),
    ]);
    assert!(!results[0].parse_error, "good.dpy should parse");
    assert!(
        results[0].diagnostics.is_empty(),
        "good.dpy should be clean"
    );
    assert!(results[1].parse_error, "bad.dpy should have parse error");
    assert!(results[1].has_code("D0001"));
}
