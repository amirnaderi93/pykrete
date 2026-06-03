//! v0.1.29 Spark "important"-tier audit follow-ups:
//!
//! 1. High-traffic Spark 3.4+ `F.*` functions — `try_divide`, `any_value`,
//!    `array_agg`, `unix_date`, `get`, `count_if`, `date_diff` — get
//!    column-ref checking and a modeled result type.
//! 2. `F.posexplode` / `posexplode_outer` expand to **two** columns
//!    (`pos: int`, `col: <element>`) — previously pykrete named only
//!    one (`col`) and lost track of `pos`.
//! 3. `df.summary` / `df.describe` are opaque (chain dies cleanly,
//!    re-anchor with `.cast`); `df.observe(...)` is a pass-through.
//! 4. `groupBy(...).pivot(...).agg(F.sum("col"))` still checks `col`
//!    against the pre-pivot schema, but the post-`.agg` schema is
//!    Unknown — pivot values are runtime data.
//! 5. `dropDuplicates` / `drop_duplicates` are unified: both names check
//!    column args AND preserve the receiver's schema.
//! 6. `sampleBy` joins `sample` as a pass-through.

#![allow(non_snake_case)]

mod common;
use common::*;

const SCHEMA: &str = "\
class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int
    tags: Array[string]
    when_date: date
    end_date: date
";

// ---------------------------------------------------------------------------
// 1. High-traffic F.* functions
// ---------------------------------------------------------------------------

#[test]
fn F_try_divide_checks_column_args() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.withColumn(\"ratio\", F.try_divide(\"amount\", \"quantity\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn F_try_divide_typos_fire_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.withColumn(\"ratio\", F.try_divide(\"amunt\", \"quantity\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "amunt");
}

#[test]
fn F_try_divide_result_is_double() {
    let src = format!(
        "{SCHEMA}
class Out(Schema):
    region: string
    product: string
    amount: int
    quantity: int
    tags: Array[string]
    when_date: date
    end_date: date
    ratio: string

def f(raw: SparkFrame[Sale]) -> SparkFrame[Out]:
    return raw.withColumn(\"ratio\", F.try_divide(col(\"amount\"), col(\"quantity\")))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn F_any_value_passthrough_type() {
    let src = format!(
        "{SCHEMA}
class Out(Schema):
    region: string
    one_amount: string

def f(raw: SparkFrame[Sale]) -> SparkFrame[Out]:
    return raw.groupBy(\"region\").agg(F.any_value(\"amount\").alias(\"one_amount\"))
"
    );
    // `any_value(amount: int)` is an int; declaring it string is a mismatch.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn F_array_agg_wraps_input_type() {
    let src = format!(
        "{SCHEMA}
class Out(Schema):
    region: string
    products: string

def f(raw: SparkFrame[Sale]) -> SparkFrame[Out]:
    return raw.groupBy(\"region\").agg(F.array_agg(\"product\").alias(\"products\"))
"
    );
    // `array_agg(product: string)` is array<string>; vs declared string → mismatch.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn F_unix_date_result_is_int() {
    let src = format!(
        "{SCHEMA}
class Out(Schema):
    region: string
    product: string
    amount: int
    quantity: int
    tags: Array[string]
    when_date: date
    end_date: date
    udate: string

def f(raw: SparkFrame[Sale]) -> SparkFrame[Out]:
    return raw.withColumn(\"udate\", F.unix_date(col(\"when_date\")))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn F_count_if_result_is_long() {
    let src = format!(
        "{SCHEMA}
class Out(Schema):
    region: string
    big: string

def f(raw: SparkFrame[Sale]) -> SparkFrame[Out]:
    return raw.groupBy(\"region\").agg(F.count_if(col(\"amount\") > 100).alias(\"big\"))
"
    );
    // count_if returns long; declared string → mismatch.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn F_date_diff_result_is_int() {
    let src = format!(
        "{SCHEMA}
class Out(Schema):
    region: string
    product: string
    amount: int
    quantity: int
    tags: Array[string]
    when_date: date
    end_date: date
    days: string

def f(raw: SparkFrame[Sale]) -> SparkFrame[Out]:
    return raw.withColumn(\"days\", F.date_diff(col(\"end_date\"), col(\"when_date\")))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn F_get_returns_element_type() {
    let src = format!(
        "{SCHEMA}
class Out(Schema):
    region: string
    product: string
    amount: int
    quantity: int
    tags: Array[string]
    when_date: date
    end_date: date
    first_tag: int

def f(raw: SparkFrame[Sale]) -> SparkFrame[Out]:
    return raw.withColumn(\"first_tag\", F.get(col(\"tags\"), 0))
"
    );
    // tags: list[string] → F.get returns string; declared int → mismatch.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn F_count_if_typo_inside_predicate_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").agg(F.count_if(col(\"amunt\") > 100).alias(\"big\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "amunt");
}

// ---------------------------------------------------------------------------
// 2. posexplode / posexplode_outer expand to two columns
// ---------------------------------------------------------------------------

#[test]
fn posexplode_in_select_produces_pos_and_col() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.select(F.posexplode(\"tags\")).select(col(\"pos\"), col(\"col\"))
"
    );
    // Both `pos` and `col` should resolve on the post-posexplode schema.
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn posexplode_outer_in_select_produces_pos_and_col() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.select(F.posexplode_outer(\"tags\")).select(col(\"pos\"), col(\"col\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn posexplode_only_pos_and_col_resolve_others_fire_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.select(F.posexplode(\"tags\")).select(col(\"region\"))
"
    );
    // After the posexplode-only select, the schema is just {pos, col};
    // `region` is gone.
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn posexplode_unknown_array_arg_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.select(F.posexplode(\"taggs\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "taggs");
}

// ---------------------------------------------------------------------------
// 3. summary / describe (opaque) and observe (pass-through)
// ---------------------------------------------------------------------------

#[test]
fn summary_makes_chain_opaque() {
    // After `summary()` the chain's schema is gone, so a typo downstream
    // is NOT flagged — same contract as any opaque op.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.summary().select(col(\"nope\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn describe_makes_chain_opaque() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.describe(\"amount\").select(col(\"nope\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn observe_is_pass_through() {
    // `observe()` returns the receiver unchanged — a downstream typo
    // still resolves against the original schema.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.observe(\"metrics\", F.count(col(\"amount\")).alias(\"n\")).select(col(\"region\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn observe_pass_through_catches_downstream_typo() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.observe(\"metrics\", F.count(col(\"amount\")).alias(\"n\")).select(col(\"regoin\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// 4. groupBy.pivot.agg checks input against pre-pivot schema
// ---------------------------------------------------------------------------

#[test]
fn groupBy_pivot_agg_checks_amount_against_pre_pivot_schema() {
    // `amount` exists on Sale; the chain should be clean — pre-v0.1.29
    // the pivot killed the chain so the agg's arg went unchecked, but
    // also no D0030 was raised on the agg's column. Either way the
    // *positive* case is unchanged: no D0030.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").agg(F.sum(\"amount\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn groupBy_pivot_agg_catches_typo_in_agg_argument() {
    // The negative case: a typo in `agg`'s column ref should fire D0030
    // — the new behavior, since pre-v0.1.29 the chain died at `.pivot()`.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").agg(F.sum(\"amunt\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "amunt");
}

#[test]
fn groupBy_pivot_unknown_pivot_column_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"produkt\").agg(F.sum(\"amount\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "produkt");
}

#[test]
fn groupBy_pivot_agg_post_chain_is_unknown() {
    // After the pivot.agg the result schema is data-dependent — pykrete
    // returns Unknown, so a downstream typo against the post-agg schema
    // is NOT flagged. This guards the deliberate degradation.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").agg(F.sum(\"amount\")).select(col(\"nope\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

// The shorthand dispatch sites for `.count()` and `.max/.min/.sum/.mean/.avg`
// must honor the same `after_pivot` contract as `.agg(...)`: column args
// against the pre-pivot schema still get checked, but the output schema
// is Unknown — pivot columns are runtime data.

#[test]
fn groupBy_pivot_count_post_chain_is_unknown() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").count().select(col(\"anything\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn groupBy_pivot_max_post_chain_is_unknown() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").max(\"amount\").filter(col(\"amount\") > 100)
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn groupBy_pivot_sum_post_chain_is_unknown() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").sum(\"amount\").select(col(\"nope\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn groupBy_pivot_min_post_chain_is_unknown() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").min(\"amount\").select(col(\"nope\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn groupBy_pivot_mean_post_chain_is_unknown() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").mean(\"amount\").select(col(\"nope\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn groupBy_pivot_avg_post_chain_is_unknown() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").avg(\"amount\").select(col(\"nope\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn groupBy_pivot_max_typo_in_agg_arg_fires_D0030() {
    // The pre-pivot column check must still fire — `typo` doesn't exist
    // on Sale, so the user's typo on the aggregate column is caught.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").max(\"typo\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

#[test]
fn groupBy_pivot_sum_typo_in_agg_arg_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.groupBy(\"region\").pivot(\"product\").sum(\"typo\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// ---------------------------------------------------------------------------
// 5. dropDuplicates / drop_duplicates parity
// ---------------------------------------------------------------------------

#[test]
fn drop_duplicates_snake_case_preserves_schema() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.drop_duplicates([\"region\"]).select(col(\"region\"), col(\"amount\"))
"
    );
    // Pre-v0.1.29: snake_case was column-check-only and dropped the
    // schema, so the downstream select went unchecked but also wasn't
    // a false positive. Now it preserves the schema: the positive case
    // stays clean.
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn drop_duplicates_snake_case_catches_downstream_typo() {
    // The schema-preservation rebuild: a typo AFTER drop_duplicates now
    // fires D0030 (it didn't pre-v0.1.29 — schema was lost).
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.drop_duplicates([\"region\"]).select(col(\"regoin\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn drop_duplicates_snake_case_catches_subset_typo() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.drop_duplicates(subset=[\"regoin\"])
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

#[test]
fn drop_duplicates_snake_case_catches_positional_typo() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.drop_duplicates([\"regoin\"])
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// ---------------------------------------------------------------------------
// 6. sampleBy pass-through
// ---------------------------------------------------------------------------

#[test]
fn sampleBy_preserves_schema() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.sampleBy(\"region\", {{\"x\": 0.5}}).select(col(\"region\"), col(\"amount\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn sampleBy_catches_downstream_typo() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.sampleBy(\"region\", {{\"x\": 0.5}}).select(col(\"regoin\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "regoin");
}

// The first positional arg to `sampleBy` IS a column reference and
// must be checked against the receiver — pre-fix it slipped through
// the pass-through bucket unchecked.

#[test]
fn sampleBy_typo_in_col_arg_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.sampleBy(\"typo\", {{\"us\": 0.5}})
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

#[test]
fn sampleBy_col_form_first_arg_resolved() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.sampleBy(col(\"region\"), {{\"us\": 0.5}})
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn sampleBy_dict_string_keys_are_values_not_column_refs() {
    // The dict keys are stratum values (literal cells of the column),
    // NOT column names — they must not be column-ref-checked.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.sampleBy(\"region\", {{\"us\": 0.5, \"eu\": 0.3}})
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

// `describe(*cols)` — explicit-column form should column-ref-check
// each positional arg against the receiver; the output schema is left
// Unknown (data-dependent statistics table).

#[test]
fn describe_explicit_cols_no_typo_clean() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.describe(\"amount\", \"region\")
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn describe_explicit_cols_typo_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.describe(\"typo\")
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

// `observe(name, *exprs)` — first arg is a metric name (NOT a column
// ref); subsequent args are expressions whose embedded column refs
// must be checked.

#[test]
fn observe_metric_name_not_treated_as_column_ref() {
    // `"metric_name"` is a literal label, not a column ref — it must
    // NOT fire D0030 even though no column by that name exists.
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.observe(\"metric_name\", F.sum(\"amount\").alias(\"total\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn observe_expr_arg_typo_fires_D0030() {
    let src = format!(
        "{SCHEMA}
def f(raw: SparkFrame[Sale]) -> SparkFrame:
    return raw.observe(\"metric_name\", F.sum(\"typo\").alias(\"total\"))
"
    );
    let result = check(&src);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}
