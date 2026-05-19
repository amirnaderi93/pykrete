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
/// (the qualifier is a table pykrete doesn't model). Deduplicated, in
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
                && (i + keyword.len() == bytes.len() || !is_word_byte(bytes[i + keyword.len()])) =>
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
    if !bare.is_empty() && bare.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
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
        assert_eq!(
            select_projection_columns("SELECT a AS x"),
            Some(vec!["x"]),
        );
    }

    #[test]
    fn projection_columns_bail_on_a_non_select() {
        assert_eq!(select_projection_columns("WITH c AS (SELECT 1) SELECT * FROM c"), None);
        assert_eq!(select_projection_columns("not sql at all"), None);
    }
}
