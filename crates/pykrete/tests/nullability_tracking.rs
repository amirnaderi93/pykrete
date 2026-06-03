//! Pipeline nullability tracking — an outer join leaves the other
//! side's columns null on an unmatched row, so pykrete marks them
//! `Nullable`; `coalesce` / `fillna` / `dropna` / `na.fill` / `na.drop`
//! clear it again. The strict mode flags a nullable column the return
//! type declares non-null (`D0083`); the default mode stays quiet.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check, check_strict};

const SCHEMAS: &str = "\
class Left(Schema):
    id: int
    a: int

class Right(Schema):
    id: int
    b: int

class Joined(Schema):
    id: int
    a: int
    b: int
";

fn left_join(how: &str) -> String {
    format!(
        "{SCHEMAS}
def f(l: SparkFrame[Left], r: SparkFrame[Right]) -> SparkFrame[Joined]:
    return l.join(r, \"id\", how=\"{how}\")
"
    )
}

#[test]
fn left_join_makes_the_right_side_nullable_under_strict() {
    assert_has_code(&check_strict(&left_join("left")), "D0083");
}

#[test]
fn left_join_nullability_is_silent_in_default_mode() {
    assert_no_diagnostics(&check(&left_join("left")));
}

#[test]
fn right_join_makes_the_left_side_nullable_under_strict() {
    assert_has_code(&check_strict(&left_join("right")), "D0083");
}

#[test]
fn outer_join_makes_both_sides_nullable_under_strict() {
    assert_has_code(&check_strict(&left_join("outer")), "D0083");
}

#[test]
fn inner_join_introduces_no_nulls() {
    assert_no_diagnostics(&check_strict(&left_join("inner")));
}

#[test]
fn join_keys_stay_non_nullable() {
    // After a left join the key `id` is still non-null (it is coalesced,
    // present on every row); only the non-key right column `b` is
    // nullable — and here the return type declares `b` as `Optional`.
    let src = "\
class Left(Schema):
    id: int

class Right(Schema):
    id: int
    b: int

class Joined(Schema):
    id: int
    b: Optional[int]

def f(l: SparkFrame[Left], r: SparkFrame[Right]) -> SparkFrame[Joined]:
    return l.join(r, \"id\", how=\"left\")
";
    assert_no_diagnostics(&check_strict(src));
}

/// A left join followed by a null-clearing operation — the strict check
/// must stay quiet, the nullability having been cleared.
fn join_then(tail: &str) -> String {
    format!(
        "\
class Left(Schema):
    id: int

class Right(Schema):
    id: int
    b: int

class Joined(Schema):
    id: int
    b: int

def f(l: SparkFrame[Left], r: SparkFrame[Right]) -> SparkFrame[Joined]:
    return l.join(r, \"id\", how=\"left\"){tail}
"
    )
}

#[test]
fn coalesce_clears_nullability() {
    let src = join_then(".withColumn(\"b\", F.coalesce(col(\"b\"), F.lit(0)))");
    assert_no_diagnostics(&check_strict(&src));
}

#[test]
fn fillna_clears_nullability() {
    assert_no_diagnostics(&check_strict(&join_then(".fillna(0)")));
}

#[test]
fn dropna_clears_nullability() {
    assert_no_diagnostics(&check_strict(&join_then(".dropna()")));
}

#[test]
fn na_fill_clears_nullability() {
    assert_no_diagnostics(&check_strict(&join_then(".na.fill(0)")));
}
