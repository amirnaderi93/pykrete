//! `F.lit(None)` as a null introducer, and `.cast(...)` carrying
//! nullability through. `F.lit(None).cast("string")` is a nullable
//! string column; `nullable_col.cast(T)` stays nullable.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check, check_strict};

#[test]
fn lit_none_cast_produces_a_nullable_column() {
    let src = "\
class In(Schema):
    id: int

class Out(Schema):
    id: int
    note: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"note\", F.lit(None).cast(\"string\"))
";
    // The body's `note` is nullable; `Out` declares it non-null.
    assert_has_code(&check_strict(src), "D0083");
}

#[test]
fn lit_none_cast_is_silent_in_default_mode() {
    let src = "\
class In(Schema):
    id: int

class Out(Schema):
    id: int
    note: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"note\", F.lit(None).cast(\"string\"))
";
    assert_no_diagnostics(&check(src));
}

#[test]
fn lit_none_cast_into_an_optional_column_is_clean() {
    let src = "\
class In(Schema):
    id: int

class Out(Schema):
    id: int
    note: Optional[string]

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"note\", F.lit(None).cast(\"string\"))
";
    assert_no_diagnostics(&check_strict(src));
}

#[test]
fn cast_preserves_an_input_columns_nullability() {
    // `raw` is nullable; casting it leaves it nullable.
    let src = "\
class In(Schema):
    id: int
    raw: Optional[int]

class Out(Schema):
    id: int
    raw: int

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"raw\", col(\"raw\").cast(\"int\"))
";
    assert_has_code(&check_strict(src), "D0083");
}

#[test]
fn cast_of_a_non_nullable_column_stays_non_nullable() {
    let src = "\
class In(Schema):
    id: int
    n: int

class Out(Schema):
    id: int
    n: string

def f(d: DataFrame[In]) -> DataFrame[Out]:
    return d.withColumn(\"n\", col(\"n\").cast(\"string\"))
";
    assert_no_diagnostics(&check_strict(src));
}
