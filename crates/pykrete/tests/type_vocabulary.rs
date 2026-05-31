//! Regression coverage for v0.1.28's expanded type vocabulary:
//! `decimal(p, s)`, `byte`, `short`, and `binary`. Spark calls these
//! atomic, pykrete now recognises them everywhere a name resolves to
//! a [`pykrete::types::ColumnType`].

mod common;

use common::{
    assert_does_not_have_code, assert_has_code, assert_message_contains, assert_no_diagnostics,
    check,
};

#[test]
fn schema_declares_decimal_with_precision_and_scale() {
    let src = "\
class Sale(Schema):
    region: string
    amount: decimal(18, 2)

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn schema_declares_bare_decimal_without_args() {
    let src = "\
class Sale(Schema):
    region: string
    amount: decimal

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn schema_declares_byte_short_binary() {
    let src = "\
class Row(Schema):
    flag: byte
    code: short
    payload: binary

def f(r: DataFrame[Row]) -> DataFrame:
    return r.select(col(\"flag\"), col(\"code\"), col(\"payload\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn cast_to_decimal_with_precision_carries_the_type() {
    // Audit launch-gate snippet #5: `col("amount").cast("decimal(18,2)")`
    // must produce a known decimal-typed result so downstream checks
    // can reason about it.
    let src = "\
class Sale(Schema):
    region: string
    amount: int

def repriced(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\").cast(\"decimal(18,2)\").alias(\"amount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn cast_to_byte_short_binary_is_accepted() {
    let src = "\
class Sale(Schema):
    region: string
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(
        col(\"amount\").cast(\"byte\").alias(\"b\"),
        col(\"amount\").cast(\"short\").alias(\"s\"),
        col(\"region\").cast(\"binary\").alias(\"r\"),
    )
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn cast_to_a_typo_is_flagged_with_d0011() {
    // A typo in the `.cast("...")` target was previously silently
    // accepted — pykrete couldn't pin the result, but never warned
    // either. v0.1.28 surfaces a `D0011` (invalidColumnType) on the
    // string-literal target. The message reproduces the bad target so
    // the user sees what didn't parse.
    let src = "\
class Sale(Schema):
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\").cast(\"decimial(18,2)\").alias(\"amount\"))
";
    let result = check(src);
    assert_has_code(&result, "D0011");
    assert_message_contains(&result, "D0011", "decimial(18,2)");
}

#[test]
fn cast_with_recognized_target_does_not_fire_d0011() {
    // Negative pair for the above — a legitimate cast (parameterized
    // decimal, byte, etc.) must NOT fire D0011, only the typo arm does.
    let src = "\
class Sale(Schema):
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(
        col(\"amount\").cast(\"decimal(18, 2)\").alias(\"d\"),
        col(\"amount\").cast(\"byte\").alias(\"b\"),
    )
";
    assert_does_not_have_code(&check(src), "D0011");
}

#[test]
fn group_by_sum_of_byte_returns_long() {
    // `sum(byte)` widens to long in Spark. pykrete should agree —
    // declaring the result as `long` in the schema and returning it
    // through the chain must not produce a return-type mismatch.
    let src = "\
class Raw(Schema):
    city: string
    flag: byte

class Totals(Schema):
    city: string
    total: long

def f(raw: DataFrame[Raw]) -> DataFrame[Totals]:
    return raw.groupBy(\"city\").sum(\"flag\").select(col(\"city\"), col(\"sum(flag)\").alias(\"total\"))
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn group_by_sum_of_decimal_stays_decimal() {
    // `sum(decimal)` stays decimal in pykrete's simplified rule
    // (Spark widens to decimal(p+10, s), capped at 38; we collapse).
    // Positive shape: the whole pipeline is clean (no spurious D-codes).
    // Type pinning that decimal-vs-double don't agree is covered by the
    // unit tests on `aggregate_output_type` and `function_result_type`
    // (which the permissive numeric-numeric `types_compatible` rule
    // would otherwise hide at this integration layer).
    let src = "\
class Raw(Schema):
    city: string
    amount: decimal(18, 2)

class Totals(Schema):
    city: string
    total: decimal

def f(raw: DataFrame[Raw]) -> DataFrame[Totals]:
    return raw.groupBy(\"city\").sum(\"amount\").select(col(\"city\"), col(\"sum(amount)\").alias(\"total\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn group_by_sum_of_decimal_flagged_when_declared_non_numeric() {
    // Companion to the above: when the declared output column type is
    // categorically wrong for `sum(decimal)` (string, not numeric), the
    // return-type check DOES surface D0080. This pins that the column
    // type is actually being inferred and threaded through, not silently
    // dropped.
    let src = "\
class Raw(Schema):
    city: string
    amount: decimal(18, 2)

class Totals(Schema):
    city: string
    total: string

def f(raw: DataFrame[Raw]) -> DataFrame[Totals]:
    return raw.groupBy(\"city\").sum(\"amount\").select(col(\"city\"), col(\"sum(amount)\").alias(\"total\"))
";
    assert_has_code(&check(src), "D0080");
}

#[test]
fn agg_mean_of_decimal_matches_groupby_mean_shortcut() {
    // The two surfaces for `mean(decimal)` must agree — the previous
    // implementation routed `F.mean(...)` through a fixed-Double branch
    // while `groupBy.mean(...)` kept the decimal, so the same input
    // produced different types depending on which API the user reached
    // for. Both pipelines below must accept a `decimal` output column.
    let shortcut = "\
class Raw(Schema):
    city: string
    amount: decimal(18, 2)

class Avg(Schema):
    city: string
    average: decimal

def f(raw: DataFrame[Raw]) -> DataFrame[Avg]:
    return raw.groupBy(\"city\").mean(\"amount\").select(col(\"city\"), col(\"mean(amount)\").alias(\"average\"))
";
    let agg = "\
class Raw(Schema):
    city: string
    amount: decimal(18, 2)

class Avg(Schema):
    city: string
    average: decimal

def f(raw: DataFrame[Raw]) -> DataFrame[Avg]:
    return raw.groupBy(\"city\").agg(F.mean(col(\"amount\")).alias(\"average\"))
";
    assert_no_diagnostics(&check(shortcut));
    assert_no_diagnostics(&check(agg));
}

#[test]
fn malformed_decimal_args_in_schema_field_are_reported() {
    // Bad precision/scale args in a Schema field annotation fire D0011
    // ("not a recognized pykrete type"). Garbage inside the parens must
    // not silently pass.
    let src = "\
class Bad(Schema):
    amount: decimal(\"oops\", 2)
";
    assert_has_code(&check(src), "D0011");
}

#[test]
fn decimal_precision_above_spark_cap_is_rejected() {
    // Spark caps `DECIMAL` precision at 38. pykrete used to silently
    // accept `decimal(39, 0)` (and any precision up to u8::MAX); v0.1.28
    // rejects it on every surface — schema annotations and cast strings.
    let in_schema = "\
class Bad(Schema):
    amount: decimal(39, 0)
";
    assert_has_code(&check(in_schema), "D0011");
    let in_cast = "\
class Sale(Schema):
    amount: int

def f(s: DataFrame[Sale]) -> DataFrame:
    return s.select(col(\"amount\").cast(\"decimal(39, 0)\").alias(\"a\"))
";
    assert_has_code(&check(in_cast), "D0011");
}

#[test]
fn decimal_single_arg_defaults_scale_to_zero() {
    // `decimal(p)` is Spark SQL shorthand for `decimal(p, 0)`. pykrete
    // used to reject the single-arg form outright; v0.1.28 accepts it
    // with scale defaulted to 0 — same behavior as Spark.
    let src = "\
class Order(Schema):
    qty: decimal(10)

def f(o: DataFrame[Order]) -> DataFrame:
    return o.select(col(\"qty\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn cast_to_unmodeled_spark_types_does_not_fire_d0011() {
    // Round-3 multi-lens-review fix: the new D0011 cast-typo emission
    // must not false-reject legitimate Spark types that pykrete simply
    // hasn't modeled yet. Code that worked in v0.1.27 (no D0011 because
    // the cast was silently swallowed) must still pass in v0.1.28 — the
    // allowlist threads that needle.
    let src = "\
class Sale(Schema):
    region: string
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(
        col(\"region\").cast(\"varchar(100)\").alias(\"v\"),
        col(\"region\").cast(\"char(10)\").alias(\"c\"),
        col(\"amount\").cast(\"timestamp_ntz\").alias(\"t\"),
        col(\"amount\").cast(\"interval\").alias(\"i\"),
        col(\"amount\").cast(\"interval year to month\").alias(\"iym\"),
        col(\"amount\").cast(\"void\").alias(\"vd\"),
        col(\"amount\").cast(\"null\").alias(\"nl\"),
    )
";
    assert_does_not_have_code(&check(src), "D0011");
}

#[test]
fn cast_to_numeric_and_dec_are_accepted_as_decimal_aliases() {
    // `numeric` and `dec` are Spark SQL aliases for `decimal` — same
    // parameterized form. v0.1.28 routes them through the decimal arm so
    // a typo-checker doesn't false-reject perfectly valid SQL.
    let src = "\
class Sale(Schema):
    amount: int

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(
        col(\"amount\").cast(\"numeric(18, 2)\").alias(\"n\"),
        col(\"amount\").cast(\"dec(10, 0)\").alias(\"d\"),
        col(\"amount\").cast(\"numeric\").alias(\"nb\"),
        col(\"amount\").cast(\"dec\").alias(\"db\"),
    )
";
    assert_does_not_have_code(&check(src), "D0011");
}

#[test]
fn schema_field_accepts_numeric_and_dec_aliases() {
    // Same alias treatment in schema field annotations — `amount:
    // numeric(18, 2)` is a legal Spark surface form.
    let src = "\
class Sale(Schema):
    region: string
    amount: numeric(18, 2)
    qty: dec(10)

def f(sales: DataFrame[Sale]) -> DataFrame:
    return sales.select(col(\"amount\"), col(\"qty\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn cast_typo_still_fires_d0011_even_with_allowlist() {
    // Allowlist negative pair — the bonafide typos `decimial` (for
    // decimal) and `intger` (for int) match neither the recognized set
    // nor the unmodeled-Spark allowlist, so D0011 must still fire. Pins
    // the allowlist hasn't accidentally over-broadened.
    for typo in ["decimial(18,2)", "intger"] {
        let src = format!(
            "class Sale(Schema):
    amount: int

def f(s: DataFrame[Sale]) -> DataFrame:
    return s.select(col(\"amount\").cast(\"{typo}\").alias(\"a\"))
"
        );
        assert_has_code(&check(&src), "D0011");
    }
}

#[test]
fn agg_mean_of_string_does_not_pin_a_double_on_either_path() {
    // Multi-lens-review finding (Important #1): the `F.mean(string_col)`
    // path used to short-circuit to `Double`, while
    // `groupBy.mean(string_col)` returned None. The same input would type
    // differently depending on which surface the user used. With both
    // paths now returning None, declaring the output as `string`
    // (Spark would reject the aggregate at runtime, but pykrete is
    // permissive on Unknown) leaves the pipeline clean — and crucially,
    // there's no false D0080 from a wrongly-pinned Double.
    let shortcut = "\
class Raw(Schema):
    city: string
    label: string

class Out(Schema):
    city: string
    label: string

def f(raw: DataFrame[Raw]) -> DataFrame[Out]:
    return raw.groupBy(\"city\").mean(\"label\").select(col(\"city\"), col(\"mean(label)\").alias(\"label\"))
";
    let agg = "\
class Raw(Schema):
    city: string
    label: string

class Out(Schema):
    city: string
    label: string

def f(raw: DataFrame[Raw]) -> DataFrame[Out]:
    return raw.groupBy(\"city\").agg(F.mean(col(\"label\")).alias(\"label\"))
";
    assert_does_not_have_code(&check(shortcut), "D0080");
    assert_does_not_have_code(&check(agg), "D0080");
}

// ---------------------------------------------------------------------------
// Round-4 multi-lens review fixes
// ---------------------------------------------------------------------------

#[test]
fn round4_schema_field_decimal_aliases_stay_case_sensitive() {
    // Round-4 fix: `from_name` (the strict-vocabulary surface that
    // backs schema annotations) is case-sensitive — `Int` is rejected,
    // and the alias forms `numeric` / `dec` follow the same rule. A
    // mixed-case `Numeric(18, 2)` in a schema annotation must surface
    // as D0010/D0011, not silently resolve.
    for typo in ["Numeric(18, 2)", "DECIMAL(18, 2)", "Dec(10)", "DEC"] {
        let src = format!(
            "class Sale(Schema):
    amount: {typo}
"
        );
        let result = check(&src);
        let any_unknown_code = result
            .diagnostics
            .iter()
            .any(|d| d.code == "D0010" || d.code == "D0011");
        assert!(
            any_unknown_code,
            "{typo} on the schema-annotation surface should fire D0010 or D0011, got {:?}",
            result
                .diagnostics
                .iter()
                .map(|d| d.code)
                .collect::<Vec<_>>(),
        );
    }
}

#[test]
fn round4_cast_path_decimal_aliases_stay_case_insensitive() {
    // Round-4 companion: the Spark cast path stays lenient — Spark SQL
    // itself accepts `DECIMAL`, `NUMERIC`, `Dec` interchangeably, so
    // pykrete's typo-checker must too.
    let src = "\
class Sale(Schema):
    amount: int

def f(s: DataFrame[Sale]) -> DataFrame:
    return s.select(
        col(\"amount\").cast(\"NUMERIC(18, 2)\").alias(\"n\"),
        col(\"amount\").cast(\"Dec(10, 0)\").alias(\"d\"),
        col(\"amount\").cast(\"DECIMAL\").alias(\"db\"),
    )
";
    assert_does_not_have_code(&check(src), "D0011");
}

#[test]
fn round4_schema_decimal_above_cap_fires_d0011_from_unified_validator() {
    // Round-4 fix: `validate_decimal_args` is the single source of
    // truth for `(p, s)` bounds — `precision > 38`, `scale > precision`,
    // and `precision == 0` are rejected from both the cast path and
    // the schema-annotation path. These cases land in the schema
    // surface specifically, exercising the unified validator.
    for ann in ["decimal(40, 2)", "decimal(18, 25)", "decimal(0, 0)"] {
        let src = format!(
            "class Bad(Schema):
    amount: {ann}
"
        );
        assert_has_code(&check(&src), "D0011");
    }
}

#[test]
fn round4_schema_decimal_at_max_precision_passes() {
    // Boundary check: the unified validator must allow `decimal(38, 18)`
    // — the max-valid case on both surfaces.
    let src = "\
class Sale(Schema):
    amount: decimal(38, 18)

def f(s: DataFrame[Sale]) -> DataFrame:
    return s.select(col(\"amount\").cast(\"decimal(38, 18)\").alias(\"a\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn round4_cast_to_paren_garbage_on_bare_unmodeled_types_fires_d0011() {
    // Round-4 fix: `is_unmodeled_spark_type` used to false-allow
    // `interval(5)`, `void(0)`, `null(0)` etc. because the matcher
    // split on `(` and only checked the bare keyword. These types
    // don't take parenthesised args in Spark SQL; the cast-typo
    // emission must fire.
    for typo in [
        "interval(5)",
        "void(0)",
        "null(0)",
        "timestamp_ntz(3)",
        "varchar(0)",
        "varchar(-1)",
        "char(0)",
    ] {
        let src = format!(
            "class Sale(Schema):
    amount: int

def f(s: DataFrame[Sale]) -> DataFrame:
    return s.select(col(\"amount\").cast(\"{typo}\").alias(\"a\"))
"
        );
        assert_has_code(&check(&src), "D0011");
    }
}

#[test]
fn schema_declares_float_alias_for_double() {
    // `float` is Python's float, which PySpark maps to Spark's
    // DoubleType (FloatType vs DoubleType is a coercion the runtime
    // hides). Pykrete accepts `float` everywhere `double` is accepted.
    let src = "\
class Sale(Schema):
    region: string
    amount: float

def f(s: DataFrame[Sale]) -> DataFrame:
    return s.select(col(\"amount\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn array_of_float_resolves() {
    let src = "\
class Row(Schema):
    values: Array[float]

def f(r: DataFrame[Row]) -> DataFrame:
    return r.select(col(\"values\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn float_is_interchangeable_with_double_under_strict_mode() {
    // A column declared `float` flowing into a slot declared `double`
    // (and vice-versa) is one type — strict mode must NOT flag it.
    let src = "\
class In(Schema):
    a: float

class Out(Schema):
    a: double

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d
";
    let result = common::check_strict(src);
    assert_does_not_have_code(&result, "D0080");
}

#[test]
fn schema_declares_bare_struct_as_opaque_struct() {
    // `field: Struct` is the opaque-struct sentinel — the user is
    // declaring a nested struct they haven't fully modeled. Pykrete
    // accepts it without firing D0010 and treats the column as a
    // composite whose inner navigation degrades silently.
    let src = "\
class Row(Schema):
    id: int
    payload: Struct

def f(r: DataFrame[Row]) -> DataFrame:
    return r.select(col(\"id\"), col(\"payload\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn opaque_struct_field_inside_array_is_clean() {
    let src = "\
class Row(Schema):
    events: Array[Struct]

def f(r: DataFrame[Row]) -> DataFrame:
    return r.select(col(\"events\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn opaque_struct_dotted_access_degrades_silently() {
    // Without inner-field info, pykrete can't verify the nested
    // access — but it must not false-flag (same posture as
    // `Array(None)` / `Map(..)`).
    let src = "\
class Row(Schema):
    payload: Struct

def f(r: DataFrame[Row]) -> DataFrame:
    return r.select(col(\"payload.something\"))
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn opaque_struct_flowing_into_typed_return_is_not_a_d0080() {
    // The pass-through-with-refinement pattern: a function accepts a
    // schema whose nested struct is opaque (`Struct`) and refines it
    // on output by declaring a typed nested schema. The body produces
    // an opaque-Struct value for `addr`; the declared return type has
    // a typed `Addr` struct for `addr`. Pykrete must NOT fire D0080
    // — an opaque struct flowing into a typed one is permissive, the
    // same posture as opaque-array elements and unknown column types.
    let src = "\
class Addr(Schema):
    city: string
    zip: string

class In(Schema):
    id: int
    addr: Struct

class Out(Schema):
    id: int
    addr: Addr

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d
";
    assert_does_not_have_code(&check(src), "D0080");
}

#[test]
fn get_field_on_opaque_struct_does_not_fire_d0030() {
    // `col(\"payload\").getField(\"anything\")` against an opaque
    // `Struct` — pykrete can't know the inner shape, so any field
    // access is permissive.
    let src = "\
class Row(Schema):
    id: int
    payload: Struct

def f(r: DataFrame[Row]) -> DataFrame:
    return r.select(col(\"payload\").getField(\"anything\"))
";
    assert_does_not_have_code(&check(src), "D0030");
}

#[test]
fn round4_cast_to_compound_interval_with_precision_does_not_fire_d0011() {
    // Round-4 fix: `INTERVAL DAY(3) TO SECOND(6)` is real Spark SQL —
    // a compound interval with per-unit precision args. The matcher
    // used to false-reject it (the case-sensitive base lookup and the
    // exact-string interval table both missed). With case-normalisation
    // and precision-arg stripping it now matches.
    let src = "\
class Sale(Schema):
    amount: int

def f(s: DataFrame[Sale]) -> DataFrame:
    return s.select(
        col(\"amount\").cast(\"INTERVAL DAY(3) TO SECOND(6)\").alias(\"a\"),
        col(\"amount\").cast(\"interval day(3) to second(6)\").alias(\"b\"),
        col(\"amount\").cast(\"INTERVAL YEAR TO MONTH\").alias(\"c\"),
        col(\"amount\").cast(\"VARCHAR(100)\").alias(\"d\"),
    )
";
    assert_does_not_have_code(&check(src), "D0011");
}
