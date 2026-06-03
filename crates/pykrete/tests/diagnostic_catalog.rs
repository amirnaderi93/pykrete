//! Per-`D00xx`-code snapshot test. The canonical "what pykrete says when
//! D-code <X> fires" reference: one fixture per code, the rendered
//! diagnostics for that code captured as an `insta` snapshot. A wording
//! change fails the snapshot test until explicitly accepted with
//! `cargo insta accept`.
//!
//! Adding a new code to [`pykrete::diagnostics::DIAGNOSTIC_CATALOG`]
//! without adding a fixture here fails [`every_diagnostic_code_has_a_fixture`].

use std::collections::HashSet;
use std::path::PathBuf;

use pykrete::diagnostics::{CheckMode, DIAGNOSTIC_CATALOG, Diagnostic};

/// A fixture for one diagnostic code. Most are single-file; D0071 and
/// D0072 need a 2-file project so the import / duplicate-schema check has
/// something to point across.
struct Fixture {
    code: &'static str,
    files: &'static [&'static str],
    mode: CheckMode,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        code: "D0001",
        files: &["d0001_parse_error.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0010",
        files: &["d0010_unknown_column_type.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0011",
        files: &["d0011_invalid_column_type.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0020",
        files: &["d0020_unknown_schema.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0021",
        files: &["d0021_invalid_schema_expression.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0030",
        files: &["d0030_unknown_column.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0040",
        files: &["d0040_union_schema_mismatch.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0050",
        files: &["d0050_return_columns_mismatch.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0051",
        files: &["d0051_argument_columns_mismatch.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0060",
        files: &["d0060_missing_join_key.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0070",
        files: &["d0070_unresolved_import.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0071",
        files: &[
            "d0071_unexported_name/schemas.pyk",
            "d0071_unexported_name/pipeline.pyk",
        ],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0072",
        files: &[
            "d0072_duplicate_schema_name/a.pyk",
            "d0072_duplicate_schema_name/b.pyk",
        ],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0073",
        files: &["d0073_transform_input_mismatch.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0080",
        files: &["d0080_return_type_mismatch.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0081",
        files: &["d0081_non_numeric_arithmetic.pyk"],
        mode: CheckMode::Strict,
    },
    Fixture {
        code: "D0082",
        files: &["d0082_cross_type_comparison.pyk"],
        mode: CheckMode::Strict,
    },
    Fixture {
        code: "D0083",
        files: &["d0083_nullability_mismatch.pyk"],
        mode: CheckMode::Strict,
    },
    Fixture {
        code: "D0084",
        files: &["d0084_enum_value_mismatch.pyk"],
        mode: CheckMode::Standard,
    },
    Fixture {
        code: "D0090",
        files: &["d0090_deprecated_dataframe_alias.pyk"],
        mode: CheckMode::Standard,
    },
];

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("diagnostic_catalog")
}

/// Codes reserved in `DIAGNOSTIC_CATALOG` whose check sites haven't
/// landed yet — the catalog entry pins the code identity (stability
/// surface) ahead of the emission PR. Each entry MUST be retired the
/// same release the emission lands; the coverage guard checks an entry
/// is still in the catalog so a typo can't silently strand it.
const RESERVED_CODES_WITHOUT_FIXTURES: &[&str] = &[];

/// Coverage guard. Adding a new code to `DIAGNOSTIC_CATALOG` requires
/// adding a `FIXTURES` entry; removing a code requires dropping the
/// fixture. Either way, the catalog and fixtures stay in sync. Codes
/// listed in [`RESERVED_CODES_WITHOUT_FIXTURES`] are exempt — they're
/// the code-identity-reserved slots whose emission PR is staged to
/// follow.
#[test]
fn every_diagnostic_code_has_a_fixture() {
    let known: HashSet<&str> = DIAGNOSTIC_CATALOG.iter().map(|(c, _)| *c).collect();
    let snapshotted: HashSet<&str> = FIXTURES.iter().map(|f| f.code).collect();
    let reserved: HashSet<&str> = RESERVED_CODES_WITHOUT_FIXTURES.iter().copied().collect();
    let missing: Vec<&str> = known
        .difference(&snapshotted)
        .copied()
        .filter(|c| !reserved.contains(c))
        .collect();
    let unexpected: Vec<&str> = snapshotted.difference(&known).copied().collect();
    let stale_reserved: Vec<&str> = reserved.difference(&known).copied().collect();
    assert!(
        missing.is_empty(),
        "DIAGNOSTIC_CATALOG codes without a catalog fixture: {missing:?}"
    );
    assert!(
        unexpected.is_empty(),
        "FIXTURES entries for codes not in DIAGNOSTIC_CATALOG: {unexpected:?}"
    );
    assert!(
        stale_reserved.is_empty(),
        "RESERVED_CODES_WITHOUT_FIXTURES entries not in DIAGNOSTIC_CATALOG: {stale_reserved:?}"
    );
}

/// Run the checker on `fixture` and return only the diagnostics for the
/// fixture's target code. Each diagnostic is rendered with the fixture-
/// relative path so the snapshot is stable across check-out directories.
fn capture(fixture: &Fixture) -> String {
    let dir = fixture_dir();
    let sources: Vec<(String, String)> = fixture
        .files
        .iter()
        .map(|rel| {
            let path = dir.join(rel);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            // Relative path keeps the snapshot independent of the
            // checkout location.
            ((*rel).to_string(), source)
        })
        .collect();

    let project = pykrete::check_project_with_mode(&sources, fixture.mode);

    let mut lines: Vec<String> = Vec::new();
    for file in &project.files {
        for d in &file.result.diagnostics {
            if d.code != fixture.code {
                continue;
            }
            lines.push(render_one(file.path.as_str(), d));
        }
    }
    if lines.is_empty() {
        panic!(
            "fixture for {} produced no {} diagnostics; got {:?}",
            fixture.code,
            fixture.code,
            project
                .files
                .iter()
                .flat_map(|f| f.result.diagnostics.iter().map(|d| d.code))
                .collect::<Vec<_>>()
        );
    }
    lines.join("\n")
}

fn render_one(path: &str, d: &Diagnostic) -> String {
    let suggestion = d
        .suggestion
        .as_ref()
        .map(|s| format!("  suggestion: {s}"))
        .unwrap_or_default();
    format!(
        "{path}:{line}:{col}-{eline}:{ecol} {severity} {code} {rule_name}: {message}{suggestion}",
        path = path,
        line = d.line,
        col = d.column,
        eline = d.end_line,
        ecol = d.end_column,
        severity = d.severity_label(),
        code = d.code,
        rule_name = pykrete::diagnostics::rule_name(d.code),
        message = d.message,
        suggestion = suggestion,
    )
}

/// One snapshot per fixture, named after the D-code. Test names are
/// generated by a `macro_rules!` so the catalog list above stays the
/// single source of truth — adding a fixture only requires adding to
/// `FIXTURES`.
macro_rules! snapshot_case {
    ($name:ident, $code:literal) => {
        #[test]
        fn $name() {
            let fixture = FIXTURES
                .iter()
                .find(|f| f.code == $code)
                .expect(concat!("missing FIXTURES entry for ", $code));
            let captured = capture(fixture);
            insta::assert_snapshot!($code, captured);
        }
    };
}

snapshot_case!(d0001, "D0001");
snapshot_case!(d0010, "D0010");
snapshot_case!(d0011, "D0011");
snapshot_case!(d0020, "D0020");
snapshot_case!(d0021, "D0021");
snapshot_case!(d0030, "D0030");
snapshot_case!(d0040, "D0040");
snapshot_case!(d0050, "D0050");
snapshot_case!(d0051, "D0051");
snapshot_case!(d0060, "D0060");
snapshot_case!(d0070, "D0070");
snapshot_case!(d0071, "D0071");
snapshot_case!(d0072, "D0072");
snapshot_case!(d0073, "D0073");
snapshot_case!(d0080, "D0080");
snapshot_case!(d0081, "D0081");
snapshot_case!(d0082, "D0082");
snapshot_case!(d0083, "D0083");
snapshot_case!(d0084, "D0084");
snapshot_case!(d0090, "D0090");
