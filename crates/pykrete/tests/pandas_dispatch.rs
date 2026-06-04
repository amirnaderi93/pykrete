//! Integration tests for the v1.3 pandas-spec piece (b) bare-Subscript
//! col-ref entry point + the six dispatched operations (PR-B).
//!
//! Covers:
//! - Piece (b) (spec §9): `df["col"]` / `df[["a", "b"]]` col-ref check
//!   on PandasFrame[X] AND SparkFrame[X] (the §10 bonus widening).
//! - The §5 Subscript-slice taxonomy: variable, integer, slice,
//!   boolean-mask, chained, Attribute-receiver — quiet-ignored.
//! - The six dispatched operations: column projection, boolean filter,
//!   column add/replace, drop columns, join/merge, rename.
//!
//! Spec: `docs/design/pandas-support.md` §5, §9, §10.

#![allow(non_snake_case)]

mod common;
use common::*;

// ===========================================================================
// V13B — Piece (b) bare-Subscript col-ref entry point
// ===========================================================================

#[test]
fn V13B_subscript_string_literal_on_pandas_fires_d0030_on_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df["statuss"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13B_subscript_string_literal_on_pandas_passes_on_known_column() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df["status"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_subscript_string_literal_on_spark_fires_d0030_on_typo() {
    // Spec §10 — the bonus widening. Bare `df["typo"]` against a Spark
    // receiver, OUTSIDE any `.filter(...)` / `.select(...)` call,
    // previously went unchecked; piece (b) tightens that gap.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    result = df["statuss"]
    return result
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13B_subscript_list_of_string_literals_fires_d0030_per_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[["ide", "statuss"]]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "ide");
    assert_message_contains(&result, "D0030", "statuss");
    assert_count(&result, "D0030", 2);
}

#[test]
fn V13B_subscript_list_with_one_typo_and_one_good_fires_one_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[["id", "statuss"]]
"#,
    );
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13B_subscript_variable_quiet_ignored() {
    // `df[some_var]` — opaque slice; v1.3 does not fold. Quiet ignore
    // per spec §5 taxonomy. Falsifiable with `key = "statuss"` (a real
    // typo): a wrong impl that constant-folded the variable would
    // fire D0030; the right impl leaves it alone.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    key = "statuss"
    return df[key]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_subscript_integer_quiet_ignored() {
    // `df[0]` — pandas iloc-style row positional. v1.3 ignores.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[0]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_subscript_slice_quiet_ignored() {
    // `df[:5]` — row slicing. v1.3 ignores.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[:5]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_boolean_mask_inner_subscript_fires_outer_quiet() {
    // `df[df["typo"] == "x"]` — the inner `df["typo"]` fires piece (b)
    // (Name receiver, string-literal slice → D0030). The outer
    // Subscript is a boolean-mask shape and emits no extra diagnostic.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[df["statuss"] == "shipped"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
    assert_count(&result, "D0030", 1);
}

#[test]
fn V13B_attribute_receiver_subscript_quiet_ignored() {
    // `obj.df["x"]` — Attribute receiver, NOT a Name. Spec §9 receiver-
    // shape bound: piece (b) skips, no D0030 even on a real typo.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

class Holder:
    df: object

def f(holder: Holder):
    return holder.df["statuss"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_chained_subscript_inner_subscript_descended() {
    // `df["a"]["b"]` — outer Subscript fires col-ref on "a" (Name
    // receiver). Inner ["b"] skips because its receiver is the outer
    // Subscript expression, not an Expr::Name. Spec §5 taxonomy.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df["statuss"]["x"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
    assert_count(&result, "D0030", 1);
}

#[test]
fn V13B_module_scope_subscript_quiet_ignored() {
    // `df["x"]` at module scope — ctx.lookup returns None for an
    // un-bound name. Quiet ignore — no D-code fires for "not in scope".
    let result = check(
        r#"
unbound["status"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// V13B — Per-call-site dispatch table (§5)
// ===========================================================================

// 1. Column projection: Spark `select` / pandas `df[["a", "b"]]`

#[test]
fn V13B_dispatch_select_spark_fires_d0030_on_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.select(col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13B_dispatch_select_pandas_list_subscript_fires_d0030_on_typo() {
    // Pandas's equivalent of `.select` — handled by piece (b)'s
    // List-of-literals arm.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[["statuss"]]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// 2. Boolean filter: Spark `.filter(expr)` / pandas `df[mask]`

#[test]
fn V13B_dispatch_filter_spark_fires_d0030_on_inner_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.filter(col("statuss") == "shipped")
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13B_dispatch_filter_pandas_boolean_mask_fires_inner_d0030_only() {
    // Pandas boolean-mask filter `df[df["status"] == "shipped"]` —
    // inner subscript fires col-ref check; outer shape recognized but
    // emits nothing extra.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[df["status"] == "shipped"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// 3. Column add/replace: Spark `withColumn` / pandas `df["x"] = expr`

#[test]
fn V13B_dispatch_with_column_spark_fires_d0030_on_value_col_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.withColumn("new", col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13B_dispatch_pandas_subscript_assign_enum_sink_fires_d0084() {
    // `df["status"] = "BOGUS"` where status is enum-typed. The
    // pandas-side mirror of withColumn's enum-sink check (D0084).
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: enum["shipped", "pending", "cancelled"]

def f(df: PandasFrame[Orders]):
    df["status"] = "BOGUS"
    return df
"#,
    );
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "BOGUS");
}

#[test]
fn V13B_dispatch_pandas_subscript_assign_in_vocab_quiet() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: enum["shipped", "pending", "cancelled"]

def f(df: PandasFrame[Orders]):
    df["status"] = "shipped"
    return df
"#,
    );
    assert_does_not_have_code(&result, "D0084");
}

// 4. Drop column: Spark `.drop(...)` / pandas `.drop(columns=[...])`

#[test]
fn V13B_dispatch_drop_spark_drops_named_column_from_schema() {
    // Spark `.drop("status")` silently tolerates missing names (matching
    // PySpark's runtime); the test verifies the schema is updated so a
    // subsequent reference to the dropped column fires D0030.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.drop("status").select(col("status"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "status");
}

#[test]
fn V13B_dispatch_drop_pandas_columns_kwarg_fires_d0030_on_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.drop(columns=["statuss"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13B_dispatch_drop_pandas_columns_kwarg_good_quiet() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.drop(columns=["status"])
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// 5. Join: Spark `.join(other, on=…)` / pandas `.merge(other, on=…)`

#[test]
fn V13B_dispatch_join_spark_fires_d0060_on_missing_key() {
    // Spark join's `on="statuss"` fires D0060 (the join-key existence
    // code) when the key is missing on either side.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

class Refunds(Schema):
    id: int

def f(o: SparkFrame[Orders], r: SparkFrame[Refunds]):
    return o.join(r, on="statuss")
"#,
    );
    assert_has_code(&result, "D0060");
}

#[test]
fn V13B_dispatch_merge_pandas_fires_d0060_on_missing_key() {
    // Pandas `.merge(...)` routes through the same dispatch as Spark
    // `.join(...)` — same D0060 fires on a missing key. This is the
    // load-bearing dispatch-table proof for the join row.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

class Refunds(Schema):
    id: int

def f(o: PandasFrame[Orders], r: PandasFrame[Refunds]):
    return o.merge(r, on="statuss")
"#,
    );
    assert_has_code(&result, "D0060");
}

#[test]
fn V13B_dispatch_merge_pandas_good_key_quiet() {
    // The dispatch-under-test fires D0060 on a missing join key —
    // assert no D0060 (and no D0030) when the key is real, so a
    // regression that routed merge through the wrong dispatch (e.g.
    // back to a column-method check that would fire D0030 on the
    // string `"id"`) would still get caught.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

class Refunds(Schema):
    id: int

def f(o: PandasFrame[Orders], r: PandasFrame[Refunds]):
    return o.merge(r, on="id")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
    assert_does_not_have_code(&result, "D0060");
}

// 6. Rename: Spark `.withColumnRenamed(...)` / pandas
//    `.rename(columns={…})`. Both shapes are tolerant of missing
//    source names (per Spark `.withColumnsRenamed` and pandas
//    `.rename` semantics) — the rename map is a soft transform.

#[test]
fn V13B_dispatch_rename_pandas_columns_kwarg_preserves_schema_shape() {
    // The schema shape change carries through: the renamed column
    // should be accessible after the rename, and the original name
    // should not.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    renamed = df.rename(columns={"status": "state"})
    return renamed.select(col("state"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_dispatch_rename_pandas_typo_after_rename_fires_d0030() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    renamed = df.rename(columns={"status": "state"})
    return renamed.select(col("status"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "status");
}

// ===========================================================================
// V13B — assign (pandas withColumn equivalent)
// ===========================================================================

#[test]
fn V13B_dispatch_assign_pandas_fires_d0030_on_value_col_typo() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.assign(amount=col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

#[test]
fn V13B_dispatch_assign_pandas_extends_schema() {
    // After df.assign(new=...), the new column should be accessible.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: PandasFrame[Orders]):
    extended = df.assign(amount=col("id"))
    return extended.select(col("amount"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_dispatch_assign_pandas_fires_d0030_on_subscript_ref_col_typo() {
    // Round-2 minor: parallel to the `col("statuss")` test, but with
    // the idiomatic pandas `df["statuss"]` shape on the assign kwarg's
    // RHS. Locks down the col-ref descent inside an assign kwarg
    // value.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.assign(amount=df["statuss"])
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "statuss");
}

// ===========================================================================
// V13B — Round 2: drop positional silent-corruption guard (BLOCKER 1)
// ===========================================================================

#[test]
fn V13B_pdf_drop_positional_does_not_fire_d0030() {
    // Spec §5: pandas `pdf.drop("x")` (positional) is row-by-label,
    // NOT column drop. Round 1 fell through to Spark's column-drop
    // dispatch, wrongly firing D0030 AND silently erasing the column
    // from the tracked schema. Round 2 quiet-ignores per §5 (row
    // operations are out of v1.3 scope).
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.drop("statuss")
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_pdf_drop_positional_does_not_corrupt_schema() {
    // The data-corruption half of BLOCKER 1: a subsequent reference
    // to the (not-actually-dropped) column must still resolve. If the
    // Spark column-drop dispatch erased the column from the tracked
    // schema, `col("status")` on the chain result would false-fire
    // D0030.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df.drop("status").select(col("status"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_sdf_drop_positional_fires_d0030_on_typo() {
    // Spark's `.drop("x")` is the column-drop dispatch; the Spark side
    // shouldn't regress. Spark `.drop` SILENTLY TOLERATES missing
    // names, so the typo on the drop call itself doesn't fire D0030 —
    // but the chained `col("status")` does, because the (claimed-
    // dropped) name was tolerated, leaving the schema unchanged.
    // (Round 1 covered this via the `V13B_dispatch_drop_spark_drops_named_column_from_schema`
    // test; this one pins the bare-typo behavior so the round-2 gate
    // can't silently lose Spark coverage.)
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.drop("statuss").select(col("statuss"))
"#,
    );
    assert_has_code(&result, "D0030");
}

// ===========================================================================
// V13B — Round 2: List-projection result type (IMPORTANT 1)
// ===========================================================================

#[test]
fn V13B_subscript_list_projection_chained_select_preserves_schema() {
    // `df[["a"]]` returns a Derived schema containing just "a"; the
    // chained `.select(col("a"))` resolves against that projected
    // schema. A Round-1 impl that returned None would lose schema
    // fidelity and false-fire D0030 on the legitimate access.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[["id"]].select(col("id"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_subscript_list_projection_chained_select_fires_d0030_on_unprojected() {
    // The other side of IMPORTANT 1: a chained access to a column that
    // was NOT in the projection must fire D0030. Confirms the result
    // schema is the projection, not the receiver's full schema.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    return df[["id"]].select(col("status"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "status");
}

// ===========================================================================
// V13B — Round 2: df["new"] = expr schema-extend (IMPORTANT 2)
// ===========================================================================

#[test]
fn V13B_pdf_subscript_assign_extends_schema() {
    // After `pdf["new"] = expr`, a subsequent `col("new")` must
    // resolve — Round 1 only fired the enum-sink check and never
    // extended the schema, so a chained access would false-fire D0030.
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: PandasFrame[Orders]):
    df["new"] = "value"
    return df.select(col("new"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// V13B — Round 2: Spark-gate tests for pandas-only dispatches (IMPORTANT 4)
// ===========================================================================
//
// PR-B routes `rename`, `assign`, `drop(columns=...)`, and `merge` to
// pandas-side handlers only when the receiver isn't Spark-tagged. The
// round-1 PR description asked the reviewer to eyeball this gate;
// memory `feedback_cross_codebase_must_verify_correctness` rejects
// eyeballed gates. These tests pin each gate against a SparkFrame
// receiver so an inverted-boolean regression would surface.

#[test]
fn V13B_sdf_rename_does_not_route_to_pandas_dispatch() {
    // `sdf.rename(columns={"status": "state"})` on a SparkFrame must
    // NOT route through pandas's `apply_rename_dict` — Spark's
    // `.rename` is not a column-rename method (Spark uses
    // `.withColumnRenamed`). Falsifier: with the pandas dispatch
    // fired, `renamed` would carry `{id, state}` and the chained
    // `col("status")` would fire D0030 (status was renamed away).
    // With the gate, the chain returns Unknown — no schema mutation,
    // no D0030. An inverted boolean (pandas dispatch fired on Spark)
    // would surface as the spurious D0030 below.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    renamed = df.rename(columns={"status": "state"})
    return renamed.select(col("status"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_sdf_assign_method_call_on_sparkframe_does_not_route_to_pandas() {
    // `sdf.assign(...)` on a SparkFrame must NOT route through
    // pandas's `apply_pandas_assign` — Spark doesn't have `.assign`.
    // Falsifier: pandas dispatch walks the kwarg value's col-refs and
    // fires D0030 on a typo. With the gate, the kwarg value isn't
    // walked by the assign dispatch — no D0030 on the typo. (This
    // mirrors `V13B_dispatch_assign_pandas_fires_d0030_on_value_col_typo`
    // on PandasFrame; the SparkFrame counterpart must NOT fire.)
    let result = check(
        r#"
class Orders(Schema):
    id: int

def f(df: SparkFrame[Orders]):
    return df.assign(amount=col("statuss"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_sdf_drop_columns_kwarg_on_sparkframe_does_not_route_to_pandas() {
    // `sdf.drop(columns=[...])` on a SparkFrame must NOT route to
    // pandas's `apply_pandas_drop_columns` (which would fire D0030 on
    // a missing name). Spark's `.drop(...)` accepts kwargs only as
    // positional; `columns=` isn't a Spark keyword. Falling through to
    // the Spark column-method dispatch is the correct behavior:
    // Spark's `.drop` tolerates missing names silently.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.drop(columns=["statuss"])
"#,
    );
    // The pandas dispatch would have fired D0030 on the typo;
    // the gate prevents that.
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13B_sdf_merge_does_not_route_to_pandas_join_dispatch() {
    // `sdf.merge(...)` on a SparkFrame must NOT route to the
    // join-dispatch (`two_df_method`) — Spark frames use `.join`.
    // A misspelled `.merge` is wrong code, not a join. Round-1 PR-B
    // explicitly skips Join when method == "merge" and the receiver
    // is Spark-named, so no D0060 fires on the bogus key.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

class Refunds(Schema):
    id: int

def f(o: SparkFrame[Orders], r: SparkFrame[Refunds]):
    return o.merge(r, on="statuss")
"#,
    );
    // Pandas merge would fire D0060 on "statuss"; the gate
    // prevents the routing.
    assert_does_not_have_code(&result, "D0060");
}

// ===========================================================================
// V13E1 — v1.3.0 followup #1: dispatch-correctness fixes
//   Fix 1: pandas .assign(kw=...) enum-sink check (D0084)
//   Fix 2: chain-receiver gate inversion (silent schema mutation on
//          `df.cache().rename(columns={…})`)
//   Fix 3: bind_df dialect preservation across self-rebind through a
//          pandas op (`pdf = pdf.merge(...)`)
// ===========================================================================

#[test]
fn V13E1_pdf_assign_enum_sink_off_vocab_fires_d0084() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: enum["shipped", "pending", "cancelled"]

def f(pdf: PandasFrame[Orders]):
    return pdf.assign(status="BOGUS")
"#,
    );
    assert_has_code(&result, "D0084");
    assert_message_contains(&result, "D0084", "BOGUS");
}

#[test]
fn V13E1_pdf_assign_enum_sink_in_vocab_quiet() {
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: enum["shipped", "pending", "cancelled"]

def f(pdf: PandasFrame[Orders]):
    return pdf.assign(status="shipped")
"#,
    );
    assert_does_not_have_code(&result, "D0084");
}

#[test]
fn V13E1_sdf_assign_method_not_dispatched() {
    // Spark frame with `.assign(status="BOGUS")` must NOT fire D0084
    // via the pandas dispatch path — Spark frames have no `.assign`,
    // and pre-fix the dispatch gate (`!receiver_is_spark_named`)
    // already excluded SparkFrame receivers from .assign routing. After
    // Fix 2 the gate is `receiver_is_pandas_named`, which preserves the
    // same exclusion. (Spark's column-method shape table doesn't list
    // `.assign`, so nothing else here fires D0084 either.)
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: enum["shipped", "pending", "cancelled"]

def f(sdf: SparkFrame[Orders]):
    return sdf.assign(status="BOGUS")
"#,
    );
    assert_does_not_have_code(&result, "D0084");
}

#[test]
fn V13E1_chain_spark_cache_rename_does_not_mutate_schema() {
    // Pre-fix (`!receiver_is_spark_named`): the chain receiver
    // `df.cache()` had no dialect, so `.rename(columns={…})` would
    // dispatch as pandas and silently rename `status` → `state` on the
    // tracked schema. Then `col("status")` would D0030.
    // Post-fix (`receiver_is_pandas_named`): chain receivers fall
    // through; the rename is unrecognized; the original Spark schema
    // is preserved; `col("status")` still resolves.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: SparkFrame[Orders]):
    return df.cache().rename(columns={"status": "state"}).select(col("status"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13E1_chain_pandas_named_rename_still_dispatches() {
    // Positive case unaffected: when the receiver IS a pandas-tagged
    // Name, `.rename(columns={…})` still mutates the schema. Accessing
    // the old name after the rename fires D0030.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(pdf: PandasFrame[Orders]):
    out = pdf.rename(columns={"status": "state"})
    return out["status"]
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "status");
}

#[test]
fn V13E1_chain_unrecognized_receiver_rename_quiet() {
    // `something_else.rename(columns=...)` where the receiver has no
    // dialect tag (here a function call result whose source type isn't
    // a tagged Name) — pandas dispatch must NOT fire, and nothing else
    // here should either. No D0030 on the rename argument.
    let result = check(
        r#"
def f():
    something_else = make_thing()
    return something_else.rename(columns={"x": "y"})
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13E1_pdf_self_reassign_through_pandas_op_preserves_dispatch() {
    // Pre-fix: `pdf = pdf.merge(other, on="id")` calls bind_df, which
    // unconditionally removed pdf's pandas dialect tag. Then
    // `pdf["new"] = "value"` (which dispatches only on the Pandas tag)
    // silently did nothing, and a later `col("new")` D0030'd.
    // Post-fix: bind_df receives Some(Pandas) inherited from the RHS
    // receiver (pdf is pandas-tagged), preserving dispatch on the
    // rebound name.
    let result = check(
        r#"
class Orders(Schema):
    id: int

class Refunds(Schema):
    id: int

def f(pdf: PandasFrame[Orders], other: PandasFrame[Refunds]):
    pdf = pdf.merge(other, on="id")
    pdf["new"] = "value"
    return pdf["new"]
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn V13E1_pdf_to_local_name_assignment_unchanged_for_truly_opaque() {
    // `result = some_unrelated_call()` — the RHS has no dialect-tagged
    // receiver. Post-fix bind_df receives None, and `result` doesn't
    // accidentally inherit a stale dialect from anything. A later
    // `result.assign(status="BOGUS")` must NOT fire D0084 (no pandas
    // dispatch on an untagged Name).
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: enum["shipped", "pending", "cancelled"]

def f():
    result = some_unrelated_call()
    return result.assign(status="BOGUS")
"#,
    );
    assert_does_not_have_code(&result, "D0084");
}
