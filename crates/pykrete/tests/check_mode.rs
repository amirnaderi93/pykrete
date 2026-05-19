//! `CheckMode` — the strictness setting pykrete shares with the embedded
//! Python engine. Drives which diagnostics surface.

use pykrete::CheckMode;

/// A schema with a misspelled column reference — `D0030` at every mode
/// except `off`.
const BAD_COLUMN: &str = "\
class Orders(Schema):
    price: int


def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
    return raw.select(col(\"prce\"))
";

#[test]
fn parse_maps_the_setting_strings() {
    assert_eq!(CheckMode::parse("off"), CheckMode::Off);
    assert_eq!(CheckMode::parse("basic"), CheckMode::Basic);
    assert_eq!(CheckMode::parse("standard"), CheckMode::Standard);
    assert_eq!(CheckMode::parse("strict"), CheckMode::Strict);
    // Unknown / empty falls back to standard.
    assert_eq!(CheckMode::parse("nonsense"), CheckMode::Standard);
    assert_eq!(CheckMode::parse(""), CheckMode::Standard);
}

#[test]
fn as_str_round_trips_with_parse() {
    for mode in [
        CheckMode::Off,
        CheckMode::Basic,
        CheckMode::Standard,
        CheckMode::Strict,
    ] {
        assert_eq!(CheckMode::parse(mode.as_str()), mode);
    }
}

#[test]
fn modes_are_ordered_weakest_to_strongest() {
    assert!(CheckMode::Off < CheckMode::Basic);
    assert!(CheckMode::Basic < CheckMode::Standard);
    assert!(CheckMode::Standard < CheckMode::Strict);
}

#[test]
fn shows_gates_a_diagnostic_by_its_minimum_mode() {
    // A strict-only diagnostic surfaces only at strict.
    assert!(!CheckMode::Standard.shows(CheckMode::Strict));
    assert!(CheckMode::Strict.shows(CheckMode::Strict));
    // A basic diagnostic surfaces at every mode except off.
    assert!(CheckMode::Basic.shows(CheckMode::Basic));
    assert!(CheckMode::Standard.shows(CheckMode::Basic));
    assert!(!CheckMode::Off.shows(CheckMode::Basic));
}

#[test]
fn off_mode_silences_pykretes_diagnostics() {
    let standard = pykrete::check_with_mode("<t>.pyk", BAD_COLUMN, CheckMode::Standard);
    assert!(
        standard.has_code("D0030"),
        "the misspelled column should be an error at standard",
    );

    let off = pykrete::check_with_mode("<t>.pyk", BAD_COLUMN, CheckMode::Off);
    assert!(
        off.diagnostics.is_empty(),
        "off mode must emit nothing, got: {:?}",
        off.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
    );
}

#[test]
fn basic_and_strict_still_show_fundamental_errors() {
    for mode in [CheckMode::Basic, CheckMode::Strict] {
        let result = pykrete::check_with_mode("<t>.pyk", BAD_COLUMN, mode);
        assert!(result.has_code("D0030"), "D0030 must show at {mode:?}");
    }
}
