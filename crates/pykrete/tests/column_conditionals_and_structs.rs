//! `F.when(...).otherwise(...)` result-type inference and `F.struct` /
//! `F.named_struct` schema construction.
//!
//! Before this file, the result type of a `when` chain fell through to
//! Unknown — column refs inside the branches were still reached, but a
//! `withColumn("flag", F.when(... ).otherwise(...))` downstream
//! type-aware check never saw a concrete type. Same story for
//! `F.struct(...)` — the result was Unknown, so chaining `.getField`
//! onto a freshly constructed struct lost the field types.
//!
//! These tests pin the inference rules down: common-type widening across
//! branches (with numeric widening and explicit nullable handling), and
//! struct field construction from arg names + types.

#![allow(non_snake_case)]

mod common;
use common::*;

const SCHEMAS: &str = r#"
class In(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
"#;

// ===========================================================================
// when / otherwise
// ===========================================================================

#[test]
fn when_otherwise_with_homogeneous_branches_infers_type() {
    // Both branches are strings, no `.otherwise(None)` — the inferred
    // result type is `string`, so `withColumn("label", ...)` writes a
    // string column. Declaring it as `string` in Out → no D0080.
    let src = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    label: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("label", F.when(col("x") > 0, F.lit("yes")).otherwise(F.lit("no")))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn when_otherwise_widens_numerics() {
    // Branches are `int` and `double` — common type widens to `double`.
    // Declaring `result: double` in Out should not trigger D0080.
    let src = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    result: double

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("result", F.when(col("x") > 0, col("int_col")).otherwise(col("double_col")))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn when_chained_multiple_predicates_infers_common_type() {
    // Three-arm chain, all string branches → string result.
    let src = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    bucket: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(
        "bucket",
        F.when(col("x") > 100, F.lit("hi"))
         .when(col("x") > 10, F.lit("mid"))
         .otherwise(F.lit("lo")),
    )
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn when_without_otherwise_is_nullable() {
    // No `.otherwise(...)` — unmatched rows yield null. Declaring the
    // result as `Optional[int]` should pass; declaring it as `int`
    // should fire the strict nullability check (D0083).
    let nullable_ok = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    flag: Optional[int]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("flag", F.when(col("x") > 0, F.lit(1)))
"#
    );
    let result_ok = check_strict(&nullable_ok);
    assert_count(&result_ok, "D0080", 0);
    assert_count(&result_ok, "D0083", 0);
    assert_count(&result_ok, "D0030", 0);

    let nullable_bad = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    flag: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("flag", F.when(col("x") > 0, F.lit(1)))
"#
    );
    let result_bad = check_strict(&nullable_bad);
    assert_count(&result_bad, "D0083", 1);
}

#[test]
fn when_typo_in_otherwise_branch_fires_D0030() {
    // A bad column ref inside the `.otherwise(...)` branch should still
    // fire D0030 — the generic walker reaches the args.
    let src = format!(
        r#"{SCHEMAS}
def f(d: DataFrame[In]) -> DataFrame:
    return d.withColumn(
        "label",
        F.when(col("x") > 0, F.lit("yes")).otherwise(col("nope")),
    )
"#
    );
    let result = check(&src);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}

#[test]
fn when_heterogeneous_branches_degrade_to_unknown() {
    // A string branch and an int branch can't be widened. The result
    // type should be Unknown — not a fabricated `string` or `int` —
    // so a downstream return-type check that declared `result: string`
    // doesn't fire a false-positive D0080. (Conservative: no decision
    // means no diagnostic.)
    let src = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    result: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(
        "result",
        F.when(col("x") > 0, col("name")).otherwise(col("int_col")),
    )
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn when_chained_3_arms_collects_all_branches_heterogeneous() {
    // 3-arm chain with a heterogeneous outermost branch: string, int,
    // int, otherwise int. The outermost arm being a string means common
    // typing fails and the result degrades to Unknown. Declaring
    // `result: int` (or `result: string`) in Out → no D0080, because
    // Unknown is permissive. The point of the test is to PIN that the
    // outermost branch is actually collected — if it were dropped (the
    // old bug), the remaining branches widen to int and a `result: int`
    // declaration would pass for the wrong reason, while a
    // `result: string` declaration would fire a false-positive D0080.
    let src_int = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    result: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(
        "result",
        F.when(col("x") > 100, F.lit("a"))
         .when(col("x") > 10, col("int_col"))
         .otherwise(col("int_col")),
    )
"#
    );
    let result_int = check_strict(&src_int);
    assert_count(&result_int, "D0080", 0);
    assert_count(&result_int, "D0030", 0);

    let src_string = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    result: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(
        "result",
        F.when(col("x") > 100, F.lit("a"))
         .when(col("x") > 10, col("int_col"))
         .otherwise(col("int_col")),
    )
"#
    );
    let result_string = check_strict(&src_string);
    assert_count(&result_string, "D0080", 0);
    assert_count(&result_string, "D0030", 0);
}

#[test]
fn when_chained_3_arms_widens_numerics_across_all_arms() {
    // 3-arm chain across `int`, `long`, `int`, otherwise `double`. The
    // common type should widen to `double`. Pykrete's `types_compatible`
    // treats numerics interchangeably (`D0080` doesn't fire across
    // int/long/double), so the signal for "all branches were collected
    // AND a numeric common type was found" is: declaring `result:
    // string` fires D0080 (numeric vs string), declaring `result:
    // double` does not. If any one arm were dropped the answer would
    // still be numeric and the same diagnostic would fire — so this
    // test is a smoke check; the rigorous pin against the round-1 I1
    // bug is the nullable test below.
    let src_double = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    result: double

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(
        "result",
        F.when(col("x") > 100, col("int_col"))
         .when(col("x") > 10, col("long_col"))
         .when(col("x") > 0, col("int_col"))
         .otherwise(col("double_col")),
    )
"#
    );
    let result_double = check_strict(&src_double);
    assert_count(&result_double, "D0080", 0);
    assert_count(&result_double, "D0030", 0);

    let src_string = format!(
        r#"{SCHEMAS}
class Out(Schema):
    x: int
    int_col: int
    long_col: long
    double_col: double
    name: string
    result: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(
        "result",
        F.when(col("x") > 100, col("int_col"))
         .when(col("x") > 10, col("long_col"))
         .when(col("x") > 0, col("int_col"))
         .otherwise(col("double_col")),
    )
"#
    );
    let result_string = check_strict(&src_string);
    assert_count(&result_string, "D0080", 1);
}

#[test]
fn when_chained_3_arms_outer_branch_nullable_propagates() {
    // The outermost `.when(...)` value is a nullable column. The result
    // type must therefore be `Nullable(string)`, even though the inner
    // branches and `.otherwise` are all non-null string literals.
    // Declaring `result: string` (non-null) → D0083 fires.
    //
    // This is the strongest pin against the round-1 bug: if outer
    // branches were dropped, `nullable_str` would never reach
    // `common_branch_type`, the result would be plain `string`, and
    // D0083 would NOT fire.
    let src = r#"
class In(Schema):
    x: int
    nullable_str: Optional[string]

class Out(Schema):
    x: int
    nullable_str: Optional[string]
    result: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(
        "result",
        F.when(col("x") > 100, col("nullable_str"))
         .when(col("x") > 10, F.lit("x"))
         .otherwise(F.lit("z")),
    )
"#;
    let result = check_strict(src);
    assert_count(&result, "D0083", 1);
}

// ===========================================================================
// F.struct / F.named_struct
// ===========================================================================

const STRUCT_SCHEMAS: &str = r#"
class In(Schema):
    a: int
    b: string

class Bag(Schema):
    a: int
    b: string

class BagRenamed(Schema):
    a: int
    renamed: string

class NamedBag(Schema):
    k1: int
    k2: string
"#;

#[test]
fn f_struct_constructs_struct_from_columns() {
    // `F.struct(col("a"), col("b"))` builds a struct with fields a:int,
    // b:string. Re-anchoring against a schema that declares `bag` as
    // Bag (same shape) should produce no D0080.
    let src = format!(
        r#"{STRUCT_SCHEMAS}
class Out(Schema):
    a: int
    b: string
    bag: Bag

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("bag", F.struct(col("a"), col("b")))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn f_struct_with_alias_uses_alias_as_field_name() {
    // `col("b").alias("renamed")` → field name is "renamed", type from b.
    let src = format!(
        r#"{STRUCT_SCHEMAS}
class Out(Schema):
    a: int
    b: string
    bag: BagRenamed

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("bag", F.struct(col("a"), col("b").alias("renamed")))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn f_named_struct_uses_literal_names() {
    // `F.named_struct("k1", col("a"), "k2", col("b"))` → fields k1:int,
    // k2:string. Matches the NamedBag schema.
    let src = format!(
        r#"{STRUCT_SCHEMAS}
class Out(Schema):
    a: int
    b: string
    bag: NamedBag

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("bag", F.named_struct("k1", col("a"), "k2", col("b")))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn f_struct_then_getField_navigates_into_constructed_struct() {
    // Build a struct from col("a") + col("b"), then `.getField("a")` —
    // the resolved type is `int`. End-to-end proof that struct
    // construction composes with the existing `.getField` modeling.
    let src = format!(
        r#"{STRUCT_SCHEMAS}
class Out(Schema):
    a: int
    b: string
    first: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn("first", F.struct(col("a"), col("b")).getField("a"))
"#
    );
    let result = check_strict(&src);
    assert_count(&result, "D0080", 0);
    assert_count(&result, "D0030", 0);
}

#[test]
fn f_struct_with_unnamed_args_uses_col_index_convention() {
    // `F.struct(F.lit(1), F.lit("y"))` — neither arg has a discoverable
    // name (no alias, not a `col(...)`). Spark synthesizes `col1`,
    // `col2`, ...; pykrete should do the same so `.getField("col1")`
    // resolves and `.getField("")` does NOT. Pin both shapes with the
    // same struct value.
    let src_ok = r#"
class In(Schema):
    a: int

def f(d: DataFrame[In]) -> DataFrame:
    return d.withColumn("first", F.struct(F.lit(1), F.lit("y")).getField("col1"))
"#;
    let result_ok = check_strict(src_ok);
    assert_count(&result_ok, "D0030", 0);

    // `.getField("")` against a struct whose synthetic field names are
    // `col1`, `col2` — empty name doesn't match, so D0030 fires. (The
    // old code minted a `name = ""` field that would have silently
    // matched, hiding the bug.)
    let src_bad = r#"
class In(Schema):
    a: int

def f(d: DataFrame[In]) -> DataFrame:
    return d.withColumn("first", F.struct(F.lit(1), F.lit("y")).getField(""))
"#;
    let result_bad = check_strict(src_bad);
    assert_count(&result_bad, "D0030", 1);
}

#[test]
fn f_struct_typo_in_arg_fires_D0030() {
    // A typo on a column ref inside the struct args still fires D0030
    // — the generic walker reaches into the call.
    let src = format!(
        r#"{STRUCT_SCHEMAS}
def f(d: DataFrame[In]) -> DataFrame:
    return d.withColumn("bag", F.struct(col("a"), col("nope")))
"#
    );
    let result = check(&src);
    assert_count(&result, "D0030", 1);
    assert_message_contains(&result, "D0030", "nope");
}
