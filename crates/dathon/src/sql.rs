//! Column references inside embedded SQL strings.
//!
//! Spark lets a column expression be written as a SQL string —
//! `F.expr("a + b")`, `df.selectExpr("a", "b + 1 as c")`,
//! `df.filter("age > 21")`. dathon parses those fragments so the column
//! identifiers in them get checked against the dataframe's schema, the
//! same as a `col("…")` reference would.
//!
//! Parsing is best-effort: an unparseable fragment (Spark SQL has
//! syntax `sqlparser` doesn't model) yields no references rather than a
//! spurious error — dathon stays lenient on SQL it can't read.

use core::ops::ControlFlow;

use sqlparser::ast::{Expr, visit_expressions};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

/// The column identifiers referenced in a SQL expression `fragment`
/// (`"amount + 1"`, `"city > 0"`, `"amount + 1 as bumped"`, …).
///
/// The fragment is parsed as a `SELECT` projection; every bare
/// identifier in expression position is a column reference. Excluded:
/// `AS` aliases (output names, not references), function names (not
/// identifiers in the AST), and table-qualified names like `t.col`
/// (the qualifier is a table dathon doesn't model). Deduplicated, in
/// first-seen order.
pub fn column_refs(fragment: &str) -> Vec<String> {
    let wrapped = format!("SELECT {fragment}");
    let Ok(statements) = Parser::parse_sql(&GenericDialect {}, &wrapped) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    let _ = visit_expressions(&statements, |expr| {
        if let Expr::Identifier(ident) = expr {
            if !names.iter().any(|n| n == &ident.value) {
                names.push(ident.value.clone());
            }
        }
        ControlFlow::<()>::Continue(())
    });
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_identifiers_from_an_arithmetic_expression() {
        assert_eq!(column_refs("amount + base * 2"), ["amount", "base"]);
    }

    #[test]
    fn ignores_the_as_alias_and_function_names() {
        // `bumped` is an output name; `length` is a function.
        assert_eq!(column_refs("length(city) as bumped"), ["city"]);
    }

    #[test]
    fn handles_a_boolean_predicate() {
        assert_eq!(column_refs("age > 21 and city = 'x'"), ["age", "city"]);
    }

    #[test]
    fn deduplicates_repeated_references() {
        assert_eq!(column_refs("amount + amount"), ["amount"]);
    }

    #[test]
    fn unparseable_fragment_yields_nothing() {
        assert!(column_refs("!! not sql @@").is_empty());
    }

    #[test]
    fn star_has_no_column_references() {
        assert!(column_refs("*").is_empty());
    }
}
