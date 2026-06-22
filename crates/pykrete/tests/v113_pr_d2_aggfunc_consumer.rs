//! v1.13 PR-D2 — `classify_pivot_table_aggfunc` consumer. Aggfunc-driven
//! schema inference for `pdf.pivot_table(aggfunc=...)`. **FIRST observable
//! aggregate-semantics-informed schema inference in pykrete; canonical
//! convention for v1.14+ aggregate arms per spec §13.**
//!
//! Spec: `docs/design/v1.13-spec.md` §5.
//!
//! Behavior contract: the v1.12 PR-D1 recognition pass returns one of
//! Absent / AllowlistedString(s) / AllowlistedStringList / FellThrough.
//! v1.13 PR-D2 wires the consumer at `crates/pykrete/src/operations/expr.rs`
//! (the `receiver_is_pandas_inherited && method == "pivot_table"` arm) so
//! the result schema synthesizes the `values=` columns at the aggregate-
//! driven dtype per spec §5.1.1:
//!
//! - `"count"` / `"nunique"`            → Long
//! - `"mean"` / `"std"` / `"var"`       → Double
//!   `"median"`                          → Double
//! - `"sum"` / `"min"` / `"max"` /      → preserve receiver column type
//!   `"first"` / `"last"`
//!
//! Multi-aggregate (`AllowlistedStringList`) is deferred to v1.14+ per
//! spec §5.1.4; it falls through to the pre-v1.13 Unknown behavior.
//! Non-literal / callable / out-of-allowlist aggfunc forms continue to
//! fall through unchanged from v1.12.
//!
//! Observation surface: declare the return-type schema with a dtype that
//! crosses the numeric/non-numeric boundary against the synthesized type.
//! `check_return_type` collapses numeric-to-numeric mismatches as
//! permissive widening (`is_numeric`), so e.g. `Long` vs `Int` won't
//! trigger D0080 — the positive tests assert no D0080 with the EXPECTED
//! type, the dtype-cross tests assert D0080 fires when the declared type
//! crosses the numeric/non-numeric boundary against the synthesized one.

#![allow(non_snake_case)]

mod common;
use common::*;

const IN_BASE: &str = "\
class In(Schema):
    cat: string
    year: int
    amount: double
";

// ===========================================================================
// 11 positive tests — one per allowlist string. Each declares Out with
// the EXPECTED synthesized dtype on the values column; D0080 must NOT
// fire.
// ===========================================================================

// ---------------------------------------------------------------------------
// V113D2_count_synthesizes_long
//
// aggfunc='count' on `amount: double` → synthesized `amount: long`.
// `long` is numeric; declaring Out as `long` matches exactly.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_count_synthesizes_long() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='amount', index='cat', columns='year', aggfunc='count')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_nunique_synthesizes_long
// ---------------------------------------------------------------------------

#[test]
fn V113D2_nunique_synthesizes_long() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: long

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='amount', index='cat', aggfunc='nunique')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_mean_synthesizes_double
// ---------------------------------------------------------------------------

#[test]
fn V113D2_mean_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    year: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='year', index='cat', aggfunc='mean')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_std_synthesizes_double
// ---------------------------------------------------------------------------

#[test]
fn V113D2_std_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    year: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='year', index='cat', aggfunc='std')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_var_synthesizes_double
// ---------------------------------------------------------------------------

#[test]
fn V113D2_var_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    year: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='year', index='cat', aggfunc='var')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_median_synthesizes_double
// ---------------------------------------------------------------------------

#[test]
fn V113D2_median_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    year: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='year', index='cat', aggfunc='median')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_sum_preserves_receiver_type
//
// aggfunc='sum' on `amount: double` → preserved as `amount: double`.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_sum_preserves_receiver_type() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='amount', index='cat', aggfunc='sum')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_min_preserves_receiver_type
//
// aggfunc='min' on a string column (`cat: string`) preserves as string.
// Sharp test — pure preserve, no override.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_min_preserves_string_receiver() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    cat: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='cat', index='year', aggfunc='min')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_max_preserves_receiver_type
// ---------------------------------------------------------------------------

#[test]
fn V113D2_max_preserves_string_receiver() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    cat: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='cat', index='year', aggfunc='max')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_first_preserves_receiver_type
// ---------------------------------------------------------------------------

#[test]
fn V113D2_first_preserves_string_receiver() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    cat: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='cat', index='year', aggfunc='first')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_last_preserves_receiver_type
// ---------------------------------------------------------------------------

#[test]
fn V113D2_last_preserves_string_receiver() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    cat: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='cat', index='year', aggfunc='last')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ===========================================================================
// Dtype-crossing observation tests — these prove the synthesized schema
// is REAL (not Unknown), by declaring a return type whose values-column
// dtype CROSSES the numeric/non-numeric boundary against the synthesized
// one. D0080 MUST fire.
// ===========================================================================

#[test]
fn V113D2_count_declared_as_string_fires_D0080() {
    // count → Long (numeric); declaring string mismatches the numeric
    // family → D0080 fires.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='amount', index='cat', columns='year', aggfunc='count')
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "amount");
}

#[test]
fn V113D2_mean_declared_as_string_fires_D0080() {
    // mean → Double (numeric); declaring string → D0080.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    year: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='year', index='cat', aggfunc='mean')
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "year");
}

#[test]
fn V113D2_sum_declared_as_string_fires_D0080() {
    // sum preserves receiver `amount: double` (numeric); declaring string
    // mismatches → D0080.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='amount', index='cat', aggfunc='sum')
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "amount");
}

#[test]
fn V113D2_first_on_string_declared_as_int_fires_D0080() {
    // first preserves receiver `cat: string` (non-numeric); declaring
    // int mismatches the family → D0080. Sharp test of preserve-type
    // when receiver is non-numeric.
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    cat: int

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='cat', index='year', aggfunc='first')
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0080");
    assert_message_contains(&result, "D0080", "cat");
}

// ---------------------------------------------------------------------------
// V113D2_default_aggfunc_is_mean
//
// Pandas's documented default when `aggfunc=` is omitted is `mean`. The
// consumer arm treats Absent as if `aggfunc='mean'` were passed; values
// columns synthesize as Double. Declaring `year: string` (non-numeric)
// against synthesized `year: Double` (numeric) → D0080 fires.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_default_aggfunc_is_mean_declared_as_string_fires_D0080() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    year: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='year', index='cat')
"
    );
    assert_has_code(&check(&src), "D0080");
}

// ===========================================================================
// Negative-space tests (spec §5.1.6) — fall-through paths MUST stay
// Unknown so downstream observability is permissive.
// ===========================================================================

// ---------------------------------------------------------------------------
// V113D2_mixed_list_falls_through_to_unknown
//
// aggfunc=['sum', 'mean'] → multi-aggregate produces a MultiIndex on
// columns that pykrete's schema lattice doesn't model. Per spec §5.1.4,
// v1.13 PR-D2 falls through to Unknown. Declaring `Out` with a
// dtype-crossing schema (`amount: string` vs receiver's double) MUST
// NOT fire D0080 — Unknown is permissive.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_mixed_list_falls_through_to_unknown() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='amount', index='cat', aggfunc=['sum', 'mean'])
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_non_literal_aggfunc_falls_through_to_unknown
//
// aggfunc=variable → dynamic value, classifier returns FellThrough; arm
// returns Unknown. Dtype-crossing declared schema MUST NOT fire D0080.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_non_literal_aggfunc_falls_through_to_unknown() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    fn = 'sum'
    return pdf.pivot_table(values='amount', index='cat', aggfunc=fn)
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_callable_aggfunc_falls_through_to_unknown
//
// aggfunc=np.sum → callable, classifier returns FellThrough; arm returns
// Unknown.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_callable_aggfunc_falls_through_to_unknown() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='amount', index='cat', aggfunc=np.sum)
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ===========================================================================
// Sibling-arm regression guards
// ===========================================================================

// ---------------------------------------------------------------------------
// V113D2_typo_in_values_still_fires_D0030
//
// The v1.6 PR-D1 / v1.12 PR-D1 D0030 col-ref validation continues to
// fire independently of the new schema synthesis. A typo'd `values=`
// name on a recognized aggfunc must still produce D0030.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_typo_in_values_still_fires_D0030() {
    let result = check(
        r#"
class In(Schema):
    cat: string
    year: int
    amount: double

def f(pdf: PandasFrame[In]):
    return pdf.pivot_table(values='amout', index='cat', columns='year', aggfunc='sum')
"#,
    );
    assert_has_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V113D2_out_of_allowlist_string_falls_through_to_unknown
//
// 'prod' is real pandas but not in the v1.12 allowlist. Classifier
// returns FellThrough; arm returns Unknown. Dtype-crossing declared
// schema MUST NOT fire D0080 — fall-through preserved unchanged from
// v1.12.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_out_of_allowlist_string_falls_through_to_unknown() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='amount', index='cat', aggfunc='prod')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_default_aggfunc_is_mean_synthesizes_double
//
// Positive pairing for the Absent → mean → Double default. Declaring
// the values column as `double` matches the synthesized type exactly;
// no D0080 fires. Pairs with
// `V113D2_default_aggfunc_is_mean_declared_as_string_fires_D0080`
// above.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_default_aggfunc_is_mean_synthesizes_double() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    year: double

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values='year', index='cat')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

// ---------------------------------------------------------------------------
// V113D2_index_arg_is_not_a_synthesized_column_fires_D0030
//
// Spec §5.1.2: `index=` arguments are NOT modeled as result columns.
// pandas places `index=` values in the result DataFrame's INDEX, not
// as columns. Subscripting `result["date"]` (where `date` was the
// `index=`) must fire D0030 — synthesized schema covers `values=`
// only. Sharp guard against silencing legitimate D0030 fires on
// result-index access.
// ---------------------------------------------------------------------------

#[test]
fn V113D2_index_arg_is_not_a_synthesized_column_fires_D0030() {
    let result = check(
        r#"
class In(Schema):
    date: string
    product: string
    sales: double

def f(pdf: PandasFrame[In]):
    pivoted = pdf.pivot_table(values='sales', index='date', columns='product', aggfunc='sum')
    return pivoted["date"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "date");
}

// ---------------------------------------------------------------------------
// V113D2_multi_values_plus_columns_falls_through_to_unknown
//
// Spec §5.1.4 (extended): `values=['x','y']` AND `columns=` non-empty
// produces a MultiIndex-on-columns shape pandas emits as
// `(value_col, column_value)` tuples. pykrete's schema lattice doesn't
// model that; fall through to Unknown. Declared dtype-crossing schema
// MUST NOT fire D0080 — Unknown is permissive. Documents the carve-out
// AND prevents silent wrong-schema synthesis (flat `[x, y]` would be
// incorrect for the MultiIndex case).
// ---------------------------------------------------------------------------

#[test]
fn V113D2_multi_values_plus_columns_falls_through_to_unknown() {
    let src = format!(
        "{IN_BASE}
class Out(Schema):
    amount: string
    year: string

def f(pdf: PandasFrame[In]) -> PandasFrame[Out]:
    return pdf.pivot_table(values=['amount', 'year'], index='cat', columns='cat', aggfunc='sum')
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}
