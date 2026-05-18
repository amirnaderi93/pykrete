//! `df.selectExpr("…", "…")` — its arguments are SQL expression strings.
//! dathon models the *output schema* (the result column names) so a
//! transformation chain doesn't die at `.selectExpr(...)`. Checking the
//! column references inside the SQL is a separate follow-up.

mod common;

use common::{assert_has_code, assert_no_diagnostics, check};

const SCHEMA: &str = "\
class Raw(Schema):
    city: string
    amount: int
";

#[test]
fn select_expr_keeps_the_chain_alive_for_downstream_checks() {
    // Before: `selectExpr` collapsed the schema to unknown and the bad
    // ref below passed silently. Now the chain survives and it's caught.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    a = raw.selectExpr(\"city\", \"amount + 1 as bumped\")
    return a.select(col(\"nonexistent\"))
"
    );
    assert_has_code(&check(&src), "D0030");
}

#[test]
fn select_expr_aliased_output_column_resolves() {
    // `amount + 1 as bumped` produces a column named `bumped`.
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    a = raw.selectExpr(\"city\", \"amount + 1 as bumped\")
    return a.select(col(\"bumped\"))
"
    );
    assert_no_diagnostics(&check(&src));
}

#[test]
fn select_expr_star_expands_to_the_receiver_columns() {
    let src = format!(
        "{SCHEMA}
def f(raw: DataFrame[Raw]) -> DataFrame:
    a = raw.selectExpr(\"*\")
    return a.select(col(\"amount\"))
"
    );
    assert_no_diagnostics(&check(&src));
}
