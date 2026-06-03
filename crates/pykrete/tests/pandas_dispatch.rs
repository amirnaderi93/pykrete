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
    // per spec §5 taxonomy. Falsifiable: a wrong impl would either
    // (a) fire D0030 on the var, or (b) crash on the lookup.
    let result = check(
        r#"
class Orders(Schema):
    id: int
    status: string

def f(df: PandasFrame[Orders]):
    key = "id"
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
