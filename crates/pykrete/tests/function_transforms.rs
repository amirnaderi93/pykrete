//! Collection-function type transforms — `collect_list(T)` -> `array<T>`,
//! `explode(array<T>)` -> `T`, `map_keys(map<K,V>)` -> `array<K>`, … —
//! so the element type flows through a function call, not just the
//! collection kind.

mod common;

use common::{assert_does_not_have_code, assert_has_code, check};

const IN: &str = "\
class In(Schema):
    name: string
    tags: Array[int]
    score: int
    meta: Map[string, int]
";

#[test]
fn collect_list_wraps_the_element_type() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    scores: Array[int]

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.groupBy(\"name\").agg(F.collect_list(\"score\").alias(\"scores\"))
"
    );
    // `collect_list` of an int column is `array<int>`.
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn collect_list_element_type_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    scores: Array[string]

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.groupBy(\"name\").agg(F.collect_list(\"score\").alias(\"scores\"))
"
    );
    // `array<int>` produced, `array<string>` declared.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn explode_unwraps_the_array_element() {
    let src = format!(
        "{IN}
class Out(Schema):
    t: int

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.select(F.explode(col(\"tags\")).alias(\"t\"))
"
    );
    // `explode` of `array<int>` yields an `int` column.
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn explode_element_type_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    t: string

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.select(F.explode(col(\"tags\")).alias(\"t\"))
"
    );
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn map_keys_yields_an_array_of_the_key_type() {
    let src = format!(
        "{IN}
class Out(Schema):
    ks: Array[string]

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.select(F.map_keys(col(\"meta\")).alias(\"ks\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}

#[test]
fn map_keys_key_type_mismatch_is_caught() {
    let src = format!(
        "{IN}
class Out(Schema):
    ks: Array[int]

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.select(F.map_keys(col(\"meta\")).alias(\"ks\"))
"
    );
    // `meta` is `map<string, int>` — its keys are an `array<string>`.
    assert_has_code(&check(&src), "D0080");
}

#[test]
fn explode_of_map_with_two_arg_alias_emits_both_columns() {
    // `F.explode(map_col).alias("k", "v")` is Spark's tuple-alias form
    // for naming the (key, value) pair an exploded map produces. Both
    // names contribute to the output schema, typed as the map's K and V.
    let src = format!(
        "{IN}
class Out(Schema):
    k: string
    v: int

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.select(F.explode(col(\"meta\")).alias(\"k\", \"v\"))
"
    );
    let result = check(&src);
    assert_does_not_have_code(&result, "D0080");
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn explode_map_dual_alias_downstream_select_resolves_both() {
    // The two columns must be visible to downstream operations.
    let src = format!(
        "{IN}
def f(d: SparkFrame[In]) -> SparkFrame:
    return d.select(F.explode(col(\"meta\")).alias(\"k\", \"v\")).select(col(\"k\"), col(\"v\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn select_explode_alongside_attribute_column_keeps_both_in_schema() {
    // F7: `df.select(F.explode(arr).alias("a"), df.other)` must
    // produce a schema with BOTH `a` and `other`. Before the fix the
    // attribute-access second arg was dropped from the inferred
    // schema, so a downstream ref to `other` false-fired D0030.
    let src = format!(
        "{IN}
def f(d: SparkFrame[In]) -> SparkFrame:
    return d.select(F.explode(col(\"tags\")).alias(\"a\"), d.name).select(col(\"a\"), col(\"name\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn select_explode_alongside_subscript_column_keeps_both_in_schema() {
    let src = format!(
        "{IN}
def f(d: SparkFrame[In]) -> SparkFrame:
    return d.select(F.explode(col(\"tags\")).alias(\"a\"), d[\"name\"]).select(col(\"a\"), col(\"name\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0030");
}

#[test]
fn select_does_not_treat_non_dataframe_attribute_as_column_ref() {
    // F7 follow-up: not every `obj.X` is a DataFrame column. A
    // `helper.foo` attribute where `helper` is NOT in the DF-binding
    // scope must NOT be projected into the output schema as column
    // `foo`. Before the gate, the inferred schema silently grew a
    // bogus `foo` field; a downstream `col(\"foo\")` would falsely
    // type-check.
    let src = format!(
        "{IN}
class Helper:
    foo: int

def f(d: SparkFrame[In], helper: Helper) -> SparkFrame:
    return d.select(helper.foo, col(\"name\")).select(col(\"foo\"))
"
    );
    // `foo` is NOT a column on the receiver schema, so the downstream
    // `col(\"foo\")` must fire D0030 — proof the bogus name never
    // made it into the inferred output schema.
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn split_yields_an_array_of_strings() {
    let src = format!(
        "{IN}
class Out(Schema):
    name: string
    tags: Array[int]
    score: int
    meta: Map[string, int]
    parts: Array[string]

def f(d: SparkFrame[In]) -> SparkFrame[Out]:
    return d.withColumn(\"parts\", F.split(col(\"name\"), \",\"))
"
    );
    assert_does_not_have_code(&check(&src), "D0080");
}
