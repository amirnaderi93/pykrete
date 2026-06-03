//! Dotted column access through nested struct schemas.
//!
//! `col("address.street")` walks the path through nested `Schema` classes:
//! non-final segments must resolve to a nested-schema field; the final
//! segment is checked with the usual `has_field` against whichever schema
//! the walk lands on.
//!
//! Diagnostic refinement: when a path fails mid-way, the `D0030` message
//! points at the **failed segment** and the **schema we were searching at
//! that point**, not the whole dotted string against the outer schema.
//! For `col("address.missing")` against a `User` schema where `address`
//! is a nested `Address`, the diagnostic says `Column 'missing' does not
//! exist on schema 'Address'` — the user can see exactly where to look.

#![allow(non_snake_case)]

mod common;
use common::*;

const NESTED_SCHEMAS: &str = r#"
class Address(Schema):
    street: string
    city: string

class User(Schema):
    name: string
    address: Address
"#;

// ===========================================================================
// Happy path
// ===========================================================================

#[test]
fn dotted_path_resolves_through_a_single_level_of_nesting() {
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: SparkFrame[User]) -> SparkFrame[User]:
    return u.select(col("address.street"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn dotted_path_resolves_through_multiple_levels() {
    let result = check(
        r#"
class Inner(Schema):
    leaf: int

class Middle(Schema):
    inner: Inner

class Outer(Schema):
    middle: Middle

def f(o: SparkFrame[Outer]) -> SparkFrame:
    return o.select(col("middle.inner.leaf"))
"#,
    );
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn top_level_field_access_still_works_when_field_is_a_nested_struct() {
    // No dot in the path; just the top-level `address` (which is itself
    // a nested struct field). Should resolve as a normal flat lookup.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: SparkFrame[User]) -> SparkFrame:
    return u.select(col("address"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn dotted_path_works_inside_filter_and_withColumn_too() {
    // Same resolution rule applies in any context where col() refs are
    // checked. Just verify the wiring.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: SparkFrame[User]) -> SparkFrame[User]:
    return u.filter(col("address.city") == "Tehran").withColumn("c", col("address.street"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Failure: diagnostic pinpoints the failed segment and the nested schema
// ===========================================================================

#[test]
fn d0030_on_failed_inner_segment_names_the_nested_schema() {
    // `address` exists as a nested Address, but `street` is mis-typed
    // as `streetz`. The diagnostic should reference Address, not User —
    // that's where the missing field actually is.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: SparkFrame[User]) -> SparkFrame:
    return u.select(col("address.streetz"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "streetz");
    assert_message_contains(&result, "D0030", "Address");
}

#[test]
fn d0030_on_failed_outer_segment_names_the_outer_schema() {
    // `nope` doesn't exist on User at all. Diagnostic reports against User.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: SparkFrame[User]) -> SparkFrame:
    return u.select(col("nope.street"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "nope");
    assert_message_contains(&result, "D0030", "User");
}

#[test]
fn d0030_when_intermediate_segment_is_not_a_nested_schema() {
    // `name` exists as a top-level field but it's `string`, not a nested
    // schema. Trying to descend into it should fail.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}

def f(u: SparkFrame[User]) -> SparkFrame:
    return u.select(col("name.street"))
"#
    ));
    assert_has_code(&result, "D0030");
    // The failure is on `name` — we couldn't descend into a non-nested field.
    assert_message_contains(&result, "D0030", "name");
}

#[test]
fn deep_dotted_path_fails_at_the_first_missing_segment() {
    // Deep nesting; we mistype the deepest leaf. The diagnostic should
    // point at the innermost schema where the failure happened.
    let result = check(
        r#"
class Inner(Schema):
    leaf: int

class Middle(Schema):
    inner: Inner

class Outer(Schema):
    middle: Middle

def f(o: SparkFrame[Outer]) -> SparkFrame:
    return o.select(col("middle.inner.wrong"))
"#,
    );
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "wrong");
    assert_message_contains(&result, "D0030", "Inner");
}

// ===========================================================================
// Chained Column-on-Column nested-field access — `df.r.X`, `df["r"].X`,
// `df.r["X"]`, `df["r"]["X"]`. Spark resolves these by first lifting
// `r` to a Column (the nested struct) and then accessing `X` on it.
// Pykrete used to record only the first step ("r") and skip the second
// — pilot 4 surfaced this on real PySpark column-method usage.
// ===========================================================================

const NESTED_LR: &str = r#"
class R(Schema):
    a: int
    b: string

class LR(Schema):
    l: int
    r: R
"#;

#[test]
fn chained_attribute_access_into_nested_struct_is_clean_when_valid() {
    let result = check(&format!(
        r#"{NESTED_LR}
def f(df: SparkFrame[LR]) -> SparkFrame:
    return df.select(df.r.a)
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn chained_attribute_access_catches_typo_on_nested_field() {
    let result = check(&format!(
        r#"{NESTED_LR}
def f(df: SparkFrame[LR]) -> SparkFrame:
    return df.select(df.r.typo)
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
    // The diagnostic names the nested schema R, not the outer LR.
    assert_message_contains(&result, "D0030", "R");
}

#[test]
fn attribute_then_subscript_into_nested_struct_catches_typo() {
    // df.r["typo"] — attr then subscript with string literal.
    let result = check(&format!(
        r#"{NESTED_LR}
def f(df: SparkFrame[LR]) -> SparkFrame:
    return df.select(df.r["typo"])
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
    assert_message_contains(&result, "D0030", "R");
}

#[test]
fn subscript_then_attribute_into_nested_struct_catches_typo() {
    // df["r"].typo — subscript then attr.
    let result = check(&format!(
        r#"{NESTED_LR}
def f(df: SparkFrame[LR]) -> SparkFrame:
    return df.select(df["r"].typo)
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
    assert_message_contains(&result, "D0030", "R");
}

#[test]
fn double_subscript_into_nested_struct_catches_typo() {
    // df["r"]["typo"] — subscript chain.
    let result = check(&format!(
        r#"{NESTED_LR}
def f(df: SparkFrame[LR]) -> SparkFrame:
    return df.select(df["r"]["typo"])
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
    assert_message_contains(&result, "D0030", "R");
}

#[test]
fn chained_access_through_unknown_top_level_still_flags_outer_typo() {
    // The single-step arm fires on the outer column 'noexist'; the
    // chained walker shouldn't double-fire because the first step
    // didn't even resolve. (One diagnostic, not two.)
    let result = check(&format!(
        r#"{NESTED_LR}
def f(df: SparkFrame[LR]) -> SparkFrame:
    return df.select(df.noexist.field)
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "noexist");
}

#[test]
fn chained_access_inside_filter_is_caught() {
    // df.r.typo inside a predicate — needs the chained walker to
    // descend into BinOp/Compare branches.
    let result = check(&format!(
        r#"{NESTED_LR}
def f(df: SparkFrame[LR]) -> SparkFrame:
    return df.filter(df.r.typo == 0)
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}

#[test]
fn chained_access_through_non_nested_field_does_not_false_flag() {
    // df.l is an atomic int column; df.l.something has no schema to
    // walk into. We must not flag "something" — pykrete can't verify
    // anything about it, but the int.something pattern is also
    // nonsense at runtime, so silence here is fine (consistent with
    // resolve_path's degrade-rather-than-false-flag stance).
    let result = check(&format!(
        r#"{NESTED_LR}
def f(df: SparkFrame[LR]) -> SparkFrame:
    return df.select(df.l.something)
"#
    ));
    // The "l" first step is fine. The chained walker should bail at
    // step 2 because l is atomic — no false D0030 on "something".
    assert_does_not_have_code(&result, "D0030");
}

// ===========================================================================
// Unit-test the `resolve_path` helper directly
// ===========================================================================

mod resolve_path_unit {
    use pykrete::schema::{FieldPathResult, SchemaView, resolve_path};

    #[test]
    fn derived_schema_dotted_path_degrades_gracefully() {
        // Derived schemas (results of select / agg / etc.) don't carry
        // nested-struct info. A dotted path whose first segment *is* a
        // column degrades to `Resolved` — pykrete can't verify the rest,
        // but must not falsely flag the existing column.
        let view = SchemaView::derived_untyped(vec!["a", "b"]);
        assert!(matches!(
            resolve_path(&view, "a.b", &[]),
            FieldPathResult::Resolved
        ));
        // …but a first segment that doesn't exist is still a miss.
        assert!(matches!(
            resolve_path(&view, "missing.b", &[]),
            FieldPathResult::Missing { .. }
        ));
    }

    #[test]
    fn derived_schema_with_flat_path_resolves_normally() {
        let view = SchemaView::derived_untyped(vec!["a", "b"]);
        let result = resolve_path(&view, "a", &[]);
        assert!(matches!(result, FieldPathResult::Resolved));
    }
}

// ===========================================================================
// Spark-style backtick-quoted column refs — `col("`info`")` is `info`,
// `col("`a.b`")` is a column literally named `a.b`, and
// `col("`info`.`name`")` is nested-struct access.
// ===========================================================================

#[test]
fn backtick_wrapped_simple_name_resolves() {
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}
def f(u: SparkFrame[User]) -> SparkFrame:
    return u.select(col("`name`"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn backtick_wrapped_typo_still_fires_d0030() {
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}
def f(u: SparkFrame[User]) -> SparkFrame:
    return u.select(col("`naem`"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "naem");
}

#[test]
fn backtick_wrapped_nested_struct_access_resolves() {
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}
def f(u: SparkFrame[User]) -> SparkFrame:
    return u.select(col("`address`.`street`"))
"#
    ));
    assert_does_not_have_code(&result, "D0030");
}

#[test]
fn backtick_wrapping_literal_dot_treated_as_one_segment() {
    // ``col("`a.b`")`` is Spark's canonical way to address a column
    // whose ACTUAL name contains a dot — it's NOT struct walk
    // `a → b`. Pykrete's split_backtick_aware unit tests pin the
    // segmentation; this end-to-end check confirms that against a
    // schema with no `a.b` column, the diagnostic names the literal
    // `a.b`, not the bogus `a` segment. The split is what matters; this
    // test just guards the wiring through resolve_path / report_column_refs.
    let result = check(&format!(
        r#"{NESTED_SCHEMAS}
def f(u: SparkFrame[User]) -> SparkFrame:
    return u.select(col("`a.b`"))
"#
    ));
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "a.b");
}

mod split_backtick_aware_unit {
    use pykrete::schema::split_backtick_aware;

    #[test]
    fn plain_path_splits_on_dots() {
        assert_eq!(split_backtick_aware("a.b.c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn no_dot_returns_single_segment() {
        assert_eq!(split_backtick_aware("info"), vec!["info"]);
    }

    #[test]
    fn backticks_around_simple_name_are_stripped() {
        assert_eq!(split_backtick_aware("`info`"), vec!["info"]);
    }

    #[test]
    fn backticks_around_dotted_name_are_kept_as_single_segment() {
        assert_eq!(split_backtick_aware("`a.b`"), vec!["a.b"]);
    }

    #[test]
    fn dotted_backtick_segments_split_correctly() {
        assert_eq!(split_backtick_aware("`info`.`name`"), vec!["info", "name"]);
    }

    #[test]
    fn unmatched_backtick_degrades_to_plain_split() {
        assert_eq!(split_backtick_aware("`oops.field"), vec!["`oops", "field"]);
    }
}
