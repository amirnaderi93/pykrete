//! End-to-end tests for iteration 31's per-file scoping and import
//! statements. Exercises the failure modes the new model introduces
//! (missing imports → D0020, malformed clauses → D0070 / D0071) and the
//! happy paths (relative and absolute imports, `as` aliases).

#![allow(non_snake_case)]

use pykrete::{CheckResult, check_project};

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
            "schemas.pyk",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "pipeline.pyk",
            r#"
from .schemas import Orders

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col("x"))
"#,
        ),
    ]);
    assert!(
        results[1].diagnostics.is_empty(),
        "expected clean pipeline.pyk, got {:?}",
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
            "schemas.pyk",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "pipeline.pyk",
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
            "schemas.pyk",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "pipeline.pyk",
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
            "schemas.pyk",
            r#"
class Orders(Schema):
    x: int
"#,
        ),
        (
            "pipeline.pyk",
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
fn importing_an_external_module_is_silent_in_pykrete() {
    // Iteration 40: imports whose target isn't a `.pyk` file in the
    // project are external Python imports — `from pyspark.sql.functions
    // import col`, `from datetime import datetime`, `from pykrete import
    // Schema`, etc. pykrete doesn't try to validate them; that's the
    // embedded Python engine's job. We just skip them so they don't
    // flood the diagnostic stream.
    let results = check_pairs(&[(
        "pipeline.pyk",
        r#"
from .nonexistent import Orders
from pyspark.sql.functions import col
from pykrete import Schema
"#,
    )]);
    assert!(
        !results[0].has_code("D0070"),
        "expected no D0070 from external imports; got {:?}",
        results[0]
            .diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>(),
    );
}

#[test]
fn malformed_relative_import_with_too_many_dots_still_fires_D0070() {
    // The other half of the D0070 surface: a path resolution failure
    // (too many leading dots so the import walks above the project
    // root) IS a real bug, and we keep emitting D0070 for it.
    let results = check_pairs(&[(
        "pkg/pipeline.pyk",
        r#"
from ......way_too_many_dots import X
"#,
    )]);
    assert!(
        results[0].has_code("D0070"),
        "expected D0070 for over-deep relative import; got {:?}",
        results[0]
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>(),
    );
}
