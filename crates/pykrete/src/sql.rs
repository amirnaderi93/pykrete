//! Column references inside embedded SQL strings.
//!
//! Spark lets a column expression be written as a SQL string —
//! `F.expr("a + b")`, `df.selectExpr("a", "b + 1 as c")`,
//! `df.filter("age > 21")`. pykrete parses those fragments so the column
//! identifiers in them get checked against the dataframe's schema, the
//! same as a `col("…")` reference would.
//!
//! Parsing is best-effort: an unparseable fragment (Spark SQL has
//! syntax `sqlparser` doesn't model) yields no references rather than a
//! spurious error — pykrete stays lenient on SQL it can't read.

use core::ops::ControlFlow;

use sqlparser::ast::{
    BinaryOperator, Expr, ObjectNamePart, Query, SetExpr, Statement, TableFactor, Value,
    visit_expressions,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

/// A `column = 'literal'` / `column IN ('a', 'b', ...)` pair captured
/// from a SQL fragment. `value` is the literal text (single-quote
/// content, byte-for-byte); the byte-offset slice is recovered via
/// `find_literal_offset` against the original fragment because
/// sqlparser does not preserve source spans.
#[derive(Debug, PartialEq, Eq)]
pub struct ColumnLiteralPair {
    pub column: String,
    pub value: String,
}

/// The column identifiers referenced in a SQL expression `fragment`
/// (`"amount + 1"`, `"city > 0"`, `"amount + 1 as bumped"`, …).
///
/// The fragment is parsed as a `SELECT` projection; every bare
/// identifier in expression position is a column reference. Excluded:
/// `AS` aliases (output names, not references), function names (not
/// identifiers in the AST), and table-qualified names like `t.col`
/// (the qualifier is a table pykrete doesn't model). Deduplicated, in
/// first-seen order.
pub fn column_refs(fragment: &str) -> Vec<String> {
    let wrapped = format!("SELECT {fragment}");
    let Ok(statements) = Parser::parse_sql(&GenericDialect {}, &wrapped) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    let _ = visit_expressions(&statements, |expr| {
        if let Expr::Identifier(ident) = expr
            && !names.iter().any(|n| n == &ident.value)
        {
            names.push(ident.value.clone());
        }
        ControlFlow::<()>::Continue(())
    });
    names
}

/// Pairs of `(column, string-literal)` extracted from a SQL fragment's
/// equality (`col = 'X'` / `'X' = col`) and IN (`col IN ('a', 'b')`)
/// shapes. Used by the v1.1 enum-constraint check at `F.expr("…")`
/// sites — the same `D0084` that fires on `col("status") == "X"` is
/// fired here on `F.expr("status = 'X'")` when `status` resolves to
/// an enum-typed column.
///
/// Per the spec's Q7 conservatism, exotic shapes (backticked
/// identifiers, double-quoted literals, computed RHS, qualified
/// identifiers) fall through silently. The covered shapes are the
/// common case: bare identifier on one side, single-quoted literal on
/// the other.
pub fn column_literal_pairs(fragment: &str) -> Vec<ColumnLiteralPair> {
    let wrapped = format!("SELECT {fragment}");
    let Ok(statements) = Parser::parse_sql(&GenericDialect {}, &wrapped) else {
        return Vec::new();
    };
    let mut pairs: Vec<ColumnLiteralPair> = Vec::new();
    let _ = visit_expressions(&statements, |expr| {
        match expr {
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Eq | BinaryOperator::NotEq,
                right,
            } => {
                if let Some((col, lit)) = column_literal(left, right) {
                    pairs.push(ColumnLiteralPair {
                        column: col,
                        value: lit,
                    });
                }
            }
            Expr::InList {
                expr: lhs, list, ..
            } => {
                if let Expr::Identifier(ident) = lhs.as_ref() {
                    for item in list {
                        if let Some(lit) = string_literal_text(item) {
                            pairs.push(ColumnLiteralPair {
                                column: ident.value.clone(),
                                value: lit,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        ControlFlow::<()>::Continue(())
    });
    pairs
}

/// Best-effort byte offset of a quoted literal `'value'` (or `"value"`)
/// within `fragment`. Returns `None` if the exact `'value'` doesn't
/// appear — the caller falls back to anchoring on the whole fragment
/// span, the same way `report_sql_column_refs` does today.
///
/// Q1a byte-for-byte semantics — no escape unfolding here; sqlparser
/// already returns the unescaped text, and we look it up against the
/// raw fragment with the standard single-quote delimiter.
pub fn find_quoted_literal_offset(fragment: &str, value: &str) -> Option<usize> {
    for delim in ['\'', '"'] {
        let mut needle = std::string::String::with_capacity(value.len() + 2);
        needle.push(delim);
        needle.push_str(value);
        needle.push(delim);
        if let Some(offset) = fragment.find(needle.as_str()) {
            return Some(offset);
        }
    }
    None
}

/// If `left`/`right` form a `<ident> <op> <string-lit>` shape (either
/// order), the `(column-name, literal-text)` pair. Bare identifiers
/// only — qualified `t.col` is skipped per Q7 conservatism.
fn column_literal(left: &Expr, right: &Expr) -> Option<(String, String)> {
    if let (Expr::Identifier(ident), Some(lit)) = (left, string_literal_text(right)) {
        return Some((ident.value.clone(), lit));
    }
    if let (Some(lit), Expr::Identifier(ident)) = (string_literal_text(left), right) {
        return Some((ident.value.clone(), lit));
    }
    None
}

/// Unwrap a single-quoted SQL string literal — the only shape covered
/// by Q7. Double-quoted strings (some dialects, Spark included, allow
/// them) fall through; sqlparser models them as identifiers via the
/// GenericDialect, and `column_refs` already handles that flow.
fn string_literal_text(expr: &Expr) -> Option<String> {
    let Expr::Value(value_with_span) = expr else {
        return None;
    };
    match &value_with_span.value {
        Value::SingleQuotedString(s) => Some(s.clone()),
        _ => None,
    }
}

/// The single-table FROM-clause name of a top-level `SELECT … FROM x`
/// query, used to look up a registered tempView.
///
/// Returns `None` when the query has no FROM, more than one FROM table,
/// any JOIN, or anything other than a bare named table — a subquery,
/// a function call, an UNNEST. Aliases (`FROM orders_view o`) are
/// ignored: only the underlying name is returned, since pykrete doesn't
/// model aliasing yet.
///
/// The returned name is an owned `String` because the sqlparser AST it
/// comes from is allocated inside this call and dropped on return.
pub fn single_from_table(query: &str) -> Option<String> {
    let statements = Parser::parse_sql(&GenericDialect {}, query).ok()?;
    let stmt = statements.into_iter().next()?;
    let Statement::Query(q) = stmt else {
        return None;
    };
    select_single_from(&q)
}

fn select_single_from(query: &Query) -> Option<String> {
    let SetExpr::Select(select) = query.body.as_ref() else {
        return None;
    };
    if select.from.len() != 1 {
        return None;
    }
    let table_with_joins = &select.from[0];
    if !table_with_joins.joins.is_empty() {
        return None;
    }
    let TableFactor::Table { name, .. } = &table_with_joins.relation else {
        return None;
    };
    // Single-segment unqualified name only — `db.table` (two parts)
    // would need a catalog model pykrete doesn't have.
    let [ObjectNamePart::Identifier(ident)] = name.0.as_slice() else {
        return None;
    };
    Some(ident.value.clone())
}

/// Every column identifier in a full `SELECT` query — across the
/// projection, the `WHERE` clause, `GROUP BY`, `ORDER BY`, and `HAVING`.
/// Used to check `spark.sql("SELECT a FROM v WHERE b > 0")` against a
/// view schema: every name in the query must exist on the view, not
/// just the projected ones.
///
/// Deduplicated, in first-seen order. Table-qualified names like `t.col`
/// are intentionally skipped — the qualifier names a table pykrete
/// doesn't model, and the bare column would already be checked anyway.
/// An unparseable query yields an empty list (lenient).
pub fn query_column_refs(query: &str) -> Vec<String> {
    let Ok(statements) = Parser::parse_sql(&GenericDialect {}, query) else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    let _ = visit_expressions(&statements, |expr| {
        if let Expr::Identifier(ident) = expr
            && !names.iter().any(|n| n == &ident.value)
        {
            names.push(ident.value.clone());
        }
        ControlFlow::<()>::Continue(())
    });
    names
}

/// The output column names of a top-level `SELECT` query — used to
/// infer the result schema of `spark.sql("…")`.
///
/// Best-effort and deliberately narrow: it reads a plain
/// `SELECT a, b AS c, t.d FROM …`. Anything it can't resolve cleanly —
/// a `WITH` clause, a `*` wildcard, a computed column with no alias —
/// yields `None`, so the caller degrades to an unknown schema (the user
/// then annotates the result) rather than pykrete inventing columns.
///
/// The returned names are slices of `query`, so they carry its lifetime.
pub fn select_projection_columns(query: &str) -> Option<Vec<&str>> {
    let after_select = strip_keyword(query.trim(), "select")?;
    // An optional `DISTINCT` / `ALL` quantifier before the projection.
    let projection_full = strip_keyword(after_select.trim_start(), "distinct")
        .or_else(|| strip_keyword(after_select.trim_start(), "all"))
        .unwrap_or(after_select);
    // The projection runs up to the top-level `FROM` (if any).
    let projection = match find_top_level_keyword(projection_full, "from") {
        Some(i) => &projection_full[..i],
        None => projection_full,
    };
    let mut columns = Vec::new();
    for item in split_top_level_commas(projection) {
        columns.push(projection_item_name(item.trim())?);
    }
    (!columns.is_empty()).then_some(columns)
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// If `s` begins with `keyword` (case-insensitive) followed by
/// whitespace, the remainder after the keyword; otherwise `None`.
fn strip_keyword<'a>(s: &'a str, keyword: &str) -> Option<&'a str> {
    let n = keyword.len();
    if s.len() > n
        && s.is_char_boundary(n)
        && s[..n].eq_ignore_ascii_case(keyword)
        && s[n..].starts_with(char::is_whitespace)
    {
        Some(&s[n..])
    } else {
        None
    }
}

/// Byte index of the first occurrence of `keyword` as a whole word at
/// parenthesis depth 0 and outside any quotes. Used to find the `FROM`
/// that ends the projection and the `AS` that names a column.
fn find_top_level_keyword(s: &str, keyword: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' | b'"' | b'`' => quote = Some(b),
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            _ if depth == 0
                && i + keyword.len() <= bytes.len()
                && s.is_char_boundary(i)
                && s[i..i + keyword.len()].eq_ignore_ascii_case(keyword)
                && (i == 0 || !is_word_byte(bytes[i - 1]))
                && (i + keyword.len() == bytes.len()
                    || !is_word_byte(bytes[i + keyword.len()])) =>
            {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split `s` on commas that sit at parenthesis depth 0 and outside any
/// quotes — so `f(a, b), c` splits into `f(a, b)` and `c`.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// The output name of one projection item: the `AS` alias if present,
/// else the last segment of a bare (dotted) identifier. `None` for a
/// `*` wildcard or a computed expression with no alias.
fn projection_item_name(item: &str) -> Option<&str> {
    if item.is_empty() || item.contains('*') {
        return None;
    }
    if let Some(idx) = find_top_level_keyword(item, "as") {
        let alias = item[idx + 2..].trim().trim_matches(['`', '"', '\'']);
        return (!alias.is_empty() && alias.chars().all(|c| c.is_alphanumeric() || c == '_'))
            .then_some(alias);
    }
    let bare = item.trim_matches(['`', '"']);
    if !bare.is_empty()
        && bare
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return Some(bare.rsplit('.').next().unwrap_or(bare));
    }
    None
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

    #[test]
    fn projection_columns_reads_a_plain_select() {
        assert_eq!(
            select_projection_columns("SELECT amount, city FROM orders"),
            Some(vec!["amount", "city"]),
        );
    }

    #[test]
    fn projection_columns_uses_as_aliases_and_dotted_tails() {
        assert_eq!(
            select_projection_columns("select t.amount + 1 as bumped, o.city from t"),
            Some(vec!["bumped", "city"]),
        );
    }

    #[test]
    fn projection_columns_ignore_commas_inside_function_calls() {
        assert_eq!(
            select_projection_columns("SELECT coalesce(a, b) AS x, c FROM t"),
            Some(vec!["x", "c"]),
        );
    }

    #[test]
    fn projection_columns_bail_on_a_wildcard() {
        assert_eq!(select_projection_columns("SELECT * FROM t"), None);
    }

    #[test]
    fn projection_columns_bail_on_an_unaliased_expression() {
        // A computed column with no `AS` — Spark auto-names it; pykrete
        // can't, so it degrades rather than guess.
        assert_eq!(select_projection_columns("SELECT amount + 1 FROM t"), None);
    }

    #[test]
    fn projection_columns_handle_a_select_with_no_from() {
        assert_eq!(select_projection_columns("SELECT a AS x"), Some(vec!["x"]),);
    }

    #[test]
    fn projection_columns_bail_on_a_non_select() {
        assert_eq!(
            select_projection_columns("WITH c AS (SELECT 1) SELECT * FROM c"),
            None
        );
        assert_eq!(select_projection_columns("not sql at all"), None);
    }

    #[test]
    fn single_from_table_reads_a_bare_named_relation() {
        assert_eq!(
            single_from_table("SELECT a FROM orders_view"),
            Some("orders_view".to_string()),
        );
    }

    #[test]
    fn single_from_table_keeps_the_underlying_name_under_an_alias() {
        // Aliases are ignored — pykrete doesn't model aliasing yet, but
        // the underlying name is what the tempView registry is keyed on.
        assert_eq!(
            single_from_table("SELECT a FROM orders_view AS o"),
            Some("orders_view".to_string()),
        );
    }

    #[test]
    fn single_from_table_bails_on_a_join() {
        assert_eq!(
            single_from_table("SELECT a FROM x JOIN y ON x.k = y.k"),
            None
        );
    }

    #[test]
    fn single_from_table_bails_on_a_subquery() {
        assert_eq!(single_from_table("SELECT a FROM (SELECT 1) AS s"), None,);
    }

    #[test]
    fn single_from_table_bails_on_a_qualified_name() {
        // `db.table` — two segments; pykrete has no catalog model.
        assert_eq!(single_from_table("SELECT a FROM db.orders"), None);
    }

    #[test]
    fn single_from_table_bails_with_no_from() {
        assert_eq!(single_from_table("SELECT 1"), None);
    }

    #[test]
    fn single_from_table_bails_on_an_unparseable_query() {
        assert_eq!(single_from_table("!! not sql at all"), None);
    }

    #[test]
    fn query_column_refs_collects_select_where_group_by_order_by() {
        // Projection, WHERE, GROUP BY, ORDER BY identifiers all picked up,
        // deduplicated in first-seen order. The table-qualified `t.k` in
        // the FROM is skipped (it's a CompoundIdentifier, not Identifier).
        let names = query_column_refs(
            "SELECT region, sum(amount) AS total FROM v \
             WHERE amount > 0 GROUP BY region ORDER BY total",
        );
        assert!(names.iter().any(|n| n == "region"));
        assert!(names.iter().any(|n| n == "amount"));
        assert!(names.iter().any(|n| n == "total"));
    }

    #[test]
    fn query_column_refs_yields_nothing_on_an_unparseable_query() {
        assert!(query_column_refs("!! garbage @@").is_empty());
    }
}
