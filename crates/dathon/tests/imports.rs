//! End-to-end tests for iteration 31's per-file scoping and import
//! statements. Exercises the failure modes the new model introduces
//! (missing imports → D0020, malformed clauses → D0070 / D0071) and the
//! happy paths (relative and absolute imports, `as` aliases).

#![allow(non_snake_case)]

use dathon::{CheckResult, check_project};

fn check_pairs(pairs: &[(&str, &str)]) -> Vec<CheckResult> {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(p, s)| ((*p).to_string(), (*s).to_string()))
        .collect();
    check_project(&owned)
        .files
        .into_iter()
        .map(|f| f.result)
        .collect()
}

// ===========================================================================
// Happy path
// ===========================================================================

#[test]
fn from_relative_import_makes_a_schema_visible() {
    let results = check_pairs(&[
        (
            "schemas.dpy",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "pipeline.dpy",
            r#"
from .schemas import Orders

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("x"))
"#,
        ),
    ]);
    assert!(
        results[1].diagnostics.is_empty(),
        "expected clean pipeline.dpy, got {:?}",
        results[1]
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn import_as_alias_renames_a_schema_in_the_importing_file() {
    let results = check_pairs(&[
        (
            "schemas.dpy",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "pipeline.dpy",
            r#"
from .schemas import Orders as MyOrders

def f(raw: DataFrame[MyOrders]) -> DataFrame[MyOrders]:
    return raw.select(col("x"))
"#,
        ),
    ]);
    assert!(
        results[1].diagnostics.is_empty(),
        "alias should rebind the schema name: {:?}",
        results[1]
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}

// ===========================================================================
// Failure modes — the hard switch
// ===========================================================================

#[test]
fn referencing_a_schema_without_importing_it_now_fires_D0020() {
    // The pooled-namespace model accepted this. Iteration 31's strict
    // scoping requires the explicit import.
    let results = check_pairs(&[
        (
            "schemas.dpy",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "pipeline.dpy",
            r#"
def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw
"#,
        ),
    ]);
    assert!(results[1].has_code("D0020"));
}

#[test]
fn importing_a_name_that_does_not_exist_in_the_target_module_fires_D0071() {
    let results = check_pairs(&[
        (
            "schemas.dpy",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "pipeline.dpy",
            r#"
from .schemas import DoesNotExist
"#,
        ),
    ]);
    assert!(results[1].has_code("D0071"));
    assert!(
        results[1]
            .diagnostics_with_code("D0071")
            .iter()
            .any(|d| d.message.contains("DoesNotExist") && d.message.contains(".schemas")),
    );
}

#[test]
fn importing_from_a_module_that_is_not_in_the_project_fires_D0070() {
    let results = check_pairs(&[(
        "pipeline.dpy",
        r#"
from .nonexistent import Orders
"#,
    )]);
    assert!(results[0].has_code("D0070"));
    assert!(
        results[0]
            .diagnostics_with_code("D0070")
            .iter()
            .any(|d| d.message.contains(".nonexistent")),
    );
}
