//! v1.7 PR-D1 — pandas `melt(id_vars=, value_vars=, var_name=, value_name=)`
//! literal-form schema synthesis.
//!
//! Spec: `docs/design/v1.7-spec.md` §4. Pandas's `melt` uses kwargs
//! (`id_vars` / `value_vars` / `var_name` / `value_name`) whereas Spark's
//! existing `apply_melt` arm at `column_methods.rs:526` uses positional
//! lists + `variableColumnName=` / `valueColumnName=` kwargs. The pandas
//! arm sits BEFORE the Spark `melt | unpivot` arm at `expr.rs:1145`,
//! gated on `receiver_is_pandas_inherited`; Spark receivers continue to
//! route to `apply_melt` unchanged (regression guard).
//!
//! Output schema = `id_vars + [var_name, value_name]`. id_vars retain
//! their declared types; var_name is string; value_name is the common
//! type of the value_vars columns (Unknown if they disagree).

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V17D1_literal_kwargs_resolves_cleanly:
//
// Positive — the load-bearing literal-form happy path. id_vars + value_vars
// both present as list literals, every name exists on the receiver schema.
// Expected: no D-code; the call resolves cleanly.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_literal_kwargs_resolves_cleanly() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["id"], value_vars=["a", "b"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_custom_var_value_names_propagate_to_chain:
//
// Positive — custom `var_name=` / `value_name=` produce the named columns
// on the output schema. The chain `.melt(...).select("metric", "amount")`
// resolves cleanly because the synthesized schema carries `metric` /
// `amount`. Vacuity check at V17D1_default_var_value_names below.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_custom_var_value_names_propagate_to_chain() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["id"], value_vars=["a", "b"], var_name="metric", value_name="amount").select("id", "metric", "amount")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_default_var_value_names_propagate_to_chain:
//
// Positive — without `var_name=` / `value_name=`, defaults `"variable"` /
// `"value"` ride the synthesized schema. A follow-up `.select("variable",
// "value")` resolves cleanly. The vacuity guard for
// V17D1_custom_var_value_names_propagate_to_chain.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_default_var_value_names_propagate_to_chain() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["id"], value_vars=["a", "b"]).select("id", "variable", "value")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_id_vars_typo_fires_D0030:
//
// Negative — typo'd id_vars name fires D0030 with "did you mean".
// ---------------------------------------------------------------------------

#[test]
fn V17D1_id_vars_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["typo"], value_vars=["a", "b"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V17D1_value_vars_typo_fires_D0030:
//
// Negative — typo'd value_vars element fires D0030 with "did you mean".
// Vacuity guard for the id_vars test: each kwarg list is independently
// validated.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_value_vars_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["id"], value_vars=["a", "typo"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V17D1_string_literal_id_vars_form:
//
// Positive — `id_vars="id"` (bare string literal, not a list) is accepted
// and validated. Pandas accepts both shapes.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_string_literal_id_vars_form() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars="id", value_vars=["a", "b"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_string_literal_id_vars_typo_fires_D0030:
//
// Negative — single-string-literal id_vars typo fires. Vacuity guard for
// the cleanly-resolving case.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_string_literal_id_vars_typo_fires_D0030() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars="typo", value_vars=["a", "b"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V17D1_missing_id_vars_falls_through:
//
// Negative-space (per spec §4.3) — missing `id_vars=` kwarg → arm doesn't
// fire (literal-only scope; pandas runtime catches the required-kwarg
// shape). No D-code is emitted; specifically no spurious D0030 on the
// `value_vars` list since the arm short-circuits.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_missing_id_vars_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(value_vars=["a", "b"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_missing_value_vars_falls_through:
//
// Negative-space — missing `value_vars=` mirrors the missing-id_vars case.
// Both kwargs are required for the literal arm to fire.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_missing_value_vars_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["id"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_non_literal_var_name_falls_through:
//
// Negative-space (per spec §4.3) — non-literal `var_name=` (a Name
// reference) → arm falls through silently. Vacuity check: id_vars +
// value_vars are literal-and-valid; only the arm's gating on var_name's
// shape is exercised.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_non_literal_var_name_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales], name_var):
    return pdf.melt(id_vars=["id"], value_vars=["a", "b"], var_name=name_var)
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_non_literal_id_vars_falls_through:
//
// Negative-space — variable `id_vars` (a Name) → arm falls through, no
// diagnostic on the literal `value_vars` either (the arm short-circuits
// on the first non-literal kwarg, leaving runtime to catch).
// ---------------------------------------------------------------------------

#[test]
fn V17D1_non_literal_id_vars_falls_through() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales], cols_var):
    return pdf.melt(id_vars=cols_var, value_vars=["a", "b"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_chain_rename_after_melt:
//
// Positive — chained `.rename(columns={...})` on the melted schema
// resolves cleanly. The synthesized output schema flows through the
// rename arm. Vacuity check: if the arm returned None (no schema) the
// rename's column-ref check would still pass quietly; but the renamed
// column appearing in a later .select would die. The deeper check is
// the V17D1_chain_select_invalid_column_after_melt test below.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_chain_rename_after_melt() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["id"], value_vars=["a", "b"]).rename(columns={"variable": "metric"})
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_chain_select_invalid_column_after_melt:
//
// Negative — selecting a column NOT in the synthesized output schema
// fires D0030. The melted schema = [id, variable, value]; "a" is no
// longer present (it was unpivoted into a row). Sharp test that the
// synthesized schema replaces the receiver schema, not appends to it.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_chain_select_invalid_column_after_melt() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["id"], value_vars=["a", "b"]).select("a")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "a");
}

// ---------------------------------------------------------------------------
// V17D1_spark_receiver_routes_to_existing_arm:
//
// Sibling-arm regression guard (per spec §4 sibling-arm decision). The
// pandas arm is gated on `receiver_is_pandas_inherited`; a Spark
// receiver passes through to the existing `apply_melt` arm. Pandas's
// `id_vars=` kwarg is unknown to Spark's `apply_melt` (which takes
// positional ids/values + variableColumnName/valueColumnName), so the
// call falls back to the receiver schema unchanged. Expected: no
// D-code from the pandas arm (the gate misses) AND no D-code from the
// Spark arm on the unrecognized kwargs.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_spark_receiver_routes_to_existing_arm() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(sdf: SparkFrame[Sales]):
    return sdf.melt(id_vars=["id"], value_vars=["a", "b"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_spark_melt_positional_form_unchanged:
//
// Sibling-arm regression guard — the existing Spark `apply_melt` arm,
// invoked via positional column-list args, continues to validate the
// `ids` and `values` lists against the receiver schema. A typo'd Spark
// positional `values` element fires D0030. If the pandas arm leaked
// into the Spark path, this could regress.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_spark_melt_positional_form_unchanged() {
    let result = check(
        r#"
class Sales(Schema):
    id: string
    a: int
    b: int

def f(sdf: SparkFrame[Sales]):
    return sdf.melt(["id"], ["a", "typo"], "metric", "amount")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// V17D1_unbound_receiver_falls_through:
//
// Negative-space — unbound receiver (`helper`): no DataFrame binding,
// no dialect tag. The receiver-resolve step in
// `analyze_method_call_inner` would bail at the `analyze_expr` call
// (no schema), but BEFORE that, `receiver_is_pandas_inherited` is
// also false. No D-code. Vacuity check via a bogus name.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_unbound_receiver_falls_through() {
    let result = check(
        r#"
def f(helper):
    return helper.melt(id_vars=["anything"], value_vars=["bogus"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V17D1_list_form_multiple_id_vars:
//
// Positive — list-form `id_vars=["id1", "id2"]`. Each element is
// validated; the synthesized output schema carries both id columns
// followed by [variable, value]. Vacuity guard: a chained
// `.select("id1", "id2", "variable", "value")` resolves cleanly.
// ---------------------------------------------------------------------------

#[test]
fn V17D1_list_form_multiple_id_vars() {
    let result = check(
        r#"
class Sales(Schema):
    id1: string
    id2: string
    a: int
    b: int

def f(pdf: PandasFrame[Sales]):
    return pdf.melt(id_vars=["id1", "id2"], value_vars=["a", "b"]).select("id1", "id2", "variable", "value")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
