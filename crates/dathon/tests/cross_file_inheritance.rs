//! Cross-file schema inheritance — a `class Premium(Orders)` whose base
//! `Orders` is imported from another module. The local-only schema
//! discovery can't see the imported base, so `build_file_scope` promotes
//! such classes once the imports are resolved.

#![allow(non_snake_case)]

use dathon::{CheckResult, check_project};

fn check_project_pairs(pairs: &[(&str, &str)]) -> Vec<CheckResult> {
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

const SCHEMAS: &str = r#"
class Orders(Schema):
    place_code: int
    price: int
"#;

#[test]
fn subclass_inherits_an_imported_base() {
    let results = check_project_pairs(&[
        ("schemas.dpy", SCHEMAS),
        (
            "pipeline.dpy",
            r#"
from .schemas import Orders

class Premium(Orders):
    tier: string

def f(d: DataFrame[Premium]) -> DataFrame:
    return d.select(col("place_code"), col("price"), col("tier"))
"#,
        ),
    ]);
    assert!(
        results[1].diagnostics.is_empty(),
        "expected pipeline.dpy clean, got {:?}",
        results[1].diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>(),
    );
}

#[test]
fn unknown_column_is_still_flagged_with_a_cross_file_base() {
    let results = check_project_pairs(&[
        ("schemas.dpy", SCHEMAS),
        (
            "pipeline.dpy",
            r#"
from .schemas import Orders

class Premium(Orders):
    tier: string

def f(d: DataFrame[Premium]) -> DataFrame:
    return d.select(col("nonexistent"))
"#,
        ),
    ]);
    assert!(results[1].has_code("D0030"));
}

#[test]
fn cross_file_inherited_schema_is_checked_in_return_position() {
    // Body drops the inherited `price` and `tier` — short of `Premium`.
    let results = check_project_pairs(&[
        ("schemas.dpy", SCHEMAS),
        (
            "pipeline.dpy",
            r#"
from .schemas import Orders

class Premium(Orders):
    tier: string

def f(d: DataFrame[Premium]) -> DataFrame[Premium]:
    return d.select(col("place_code"))
"#,
        ),
    ]);
    assert!(results[1].has_code("D0050"));
}

#[test]
fn multi_level_inheritance_from_an_imported_base() {
    // `Mid` extends the imported `Orders`; `Top` extends the local
    // `Mid` — the promotion fixpoint resolves both.
    let results = check_project_pairs(&[
        ("schemas.dpy", SCHEMAS),
        (
            "pipeline.dpy",
            r#"
from .schemas import Orders

class Mid(Orders):
    mid_col: int

class Top(Mid):
    top_col: int

def f(d: DataFrame[Top]) -> DataFrame:
    return d.select(col("place_code"), col("price"), col("mid_col"), col("top_col"))
"#,
        ),
    ]);
    assert!(
        results[1].diagnostics.is_empty(),
        "expected pipeline.dpy clean, got {:?}",
        results[1].diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>(),
    );
}

#[test]
fn subclass_inherits_an_aliased_imported_base() {
    let results = check_project_pairs(&[
        ("schemas.dpy", SCHEMAS),
        (
            "pipeline.dpy",
            r#"
from .schemas import Orders as O

class Premium(O):
    tier: string

def f(d: DataFrame[Premium]) -> DataFrame:
    return d.select(col("place_code"), col("tier"))
"#,
        ),
    ]);
    assert!(
        results[1].diagnostics.is_empty(),
        "expected pipeline.dpy clean, got {:?}",
        results[1].diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>(),
    );
}
