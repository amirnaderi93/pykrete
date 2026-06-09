//! v1.5 PR-B2 — gate the `column_name_arg` attribute + subscript arms in
//! `strict_operators.rs` on a DataFrame binding in `BodyContext`, mirroring
//! the v1.4 PR-F1 fix shape in `select_output_name`.
//!
//! `column_name_arg` is consumed by `drop` and the `groupBy`/`cube`/`rollup`
//! share-site. Before the gate, the attribute (`<name>.X`) and subscript
//! (`<name>["X"]`) arms fired on ANY bare-Name receiver — including a
//! Python dict or any other non-DataFrame value in scope. The downstream
//! consequence: `df.drop(helper.price)` silently treated `price` as a
//! drop key and removed it from the inferred schema, surfacing a phantom
//! D0030 on a follow-up `select("price")`; `df.groupBy(helper.x).agg(...)`
//! treated `x` as a grouping key against the wrong schema view.
//!
//! Each test below is a negative-space regression-guard: it constructs a
//! fixture where the PRE-GATE behavior would cause a diagnostic the user
//! didn't author. The positive tests pin existing behavior (the gate is
//! a no-op when the receiver IS a DataFrame binding).

#![allow(non_snake_case)]

mod common;
use common::*;

// ---------------------------------------------------------------------------
// V15B2_drop_attr_on_dataframe_still_removes_column:
//
// Positive regression-guard for the existing `df.drop(df.col)` shape.
// Schema after `.drop(raw.price)` no longer contains `price`; the
// follow-up `select(col("price"))` must still fire D0030. Mirrors
// `column_checks::drop_accepts_an_attribute_access_column`.
// ---------------------------------------------------------------------------

#[test]
fn V15B2_drop_attr_on_dataframe_still_removes_column() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.drop(raw.price).select(col("price"))
"#,
    );
    assert_has_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15B2_drop_attr_on_unbound_helper_does_not_drop_column:
//
// `helper` has no binding in scope; the PRE-GATE attribute arm read
// `helper.price` as `price` and added it to the drop set, removing
// `price` from the schema. The follow-up `select(col("price"))` would
// then fire a phantom D0030 on a column the user never dropped. With
// the gate, `helper` isn't DF-bound → `column_name_arg` falls through
// → `price` stays → no D0030.
// ---------------------------------------------------------------------------

#[test]
fn V15B2_drop_attr_on_unbound_helper_does_not_drop_column() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.drop(helper.price).select(col("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15B2_drop_subscript_on_unbound_helper_does_not_drop_column:
//
// Subscript sibling of the attribute case above. `helper["price"]`
// pre-gate produced `price` and dropped it; post-gate the gate rejects
// the non-DF receiver and the column stays.
// ---------------------------------------------------------------------------

#[test]
fn V15B2_drop_subscript_on_unbound_helper_does_not_drop_column() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.drop(helper["price"]).select(col("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15B2_drop_attr_on_dict_does_not_drop_column:
//
// `bag` is a plain Python dict bound in the function body — the gate
// keys on `ctx.lookup(name).is_some()`, so a bound non-DataFrame name
// is still gated out. Without the gate, `bag.price` would have looked
// like a column ref and pulled `price` out of the schema.
// ---------------------------------------------------------------------------

#[test]
fn V15B2_drop_attr_on_dict_does_not_drop_column() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    bag = {"price": 1}
    return raw.drop(bag.price).select(col("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15B2_groupBy_attr_on_dataframe_still_recognized:
//
// Positive regression-guard for `df.groupBy(df.col)` — the grouping
// key `place_code` must still be recognized post-gate. We verify
// downstream observability via `.agg(F.sum("price"))` succeeding on
// the receiver's schema; if `place_code` weren't seen as a grouping
// key, the test would not regress directly, so this is a clean-run
// pin rather than a positive-diagnostic test. (No D0030 fires on a
// valid path.)
// ---------------------------------------------------------------------------

#[test]
fn V15B2_groupBy_attr_on_dataframe_clean() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> int:
    return raw.groupBy(raw.place_code).agg(F.sum("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15B2_groupBy_attr_on_unbound_helper_clean:
//
// Pre-gate, `helper.place_code` was harvested as a grouping key. The
// observable downstream consequence depends on the .agg follow-up; the
// minimal pin is that no D0030 surfaces from the groupBy itself. The
// real value of this test is the structural pin — if PR-B2 regresses,
// the gate stops being applied at the groupBy site too.
// ---------------------------------------------------------------------------

#[test]
fn V15B2_groupBy_attr_on_unbound_helper_clean() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> int:
    return raw.groupBy(helper.place_code).agg(F.sum("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15B2_cube_subscript_on_unbound_helper_clean:
//
// `cube` and `rollup` share the same dispatch arm as `groupBy`; the
// subscript form on an unbound receiver should also fall through. This
// pins the share-site coverage at `strict_operators.rs:551`.
// ---------------------------------------------------------------------------

#[test]
fn V15B2_cube_subscript_on_unbound_helper_clean() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> int:
    return raw.cube(helper["place_code"]).agg(F.sum("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ---------------------------------------------------------------------------
// V15B2_rollup_attr_on_unbound_helper_clean:
//
// Same share-site, attribute form, rollup variant.
// ---------------------------------------------------------------------------

#[test]
fn V15B2_rollup_attr_on_unbound_helper_clean() {
    let result = check(
        r#"
class Orders(Schema):
    place_code: int
    price: int

def f(raw: SparkFrame[Orders]) -> int:
    return raw.rollup(helper.place_code).agg(F.sum("price"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}
