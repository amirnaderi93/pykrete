use super::context::BodyContext;

use ruff_python_ast::Expr;
use ruff_text_size::{Ranged, TextRange};

// ---------------------------------------------------------------------------
// col() reference discovery
// ---------------------------------------------------------------------------

pub(super) fn col_reference(expr: &Expr) -> Option<(&str, TextRange)> {
    let call = expr.as_call_expr()?;
    let func = call.func.as_name_expr()?;
    if func.id.as_str() != "col" {
        return None;
    }
    let arg = call.arguments.args.first()?;
    let lit = arg.as_string_literal_expr()?;
    Some((lit.value.to_str(), lit.range()))
}

/// PySpark functions where every positional **string-literal** argument
/// is a column name (mixed args with int literals or column expressions
/// are fine — those don't match the string-literal arm). Used so that
/// `F.sum("price")`, `F.add_months("checkin", 1)`, `F.lower("city")`,
/// `F.coalesce("a", "b")`, etc. are recognized as column references and
/// checked against the surrounding schema.
///
/// Iteration 37 widened this from the aggregate-only list to cover the
/// rest of the column-y subset of `pyspark.sql.functions`. The rule
/// for adding a function: every position where a string literal is
/// LEGAL must mean "column name." Functions that take a value-shaped
/// string literal in any position are deliberately omitted to avoid
/// false positives:
///
/// - `lit("default")`, `expr("a > 1")` — string is a value / SQL.
/// - `date_format(col, "yyyy-MM-dd")` — second string is a format.
/// - `regexp_replace`, `regexp_extract`, `split`, `to_date`,
///   `to_timestamp`, `from_utc_timestamp`, `from_unixtime`,
///   `unix_timestamp`, `date_trunc`, `trunc`, `next_day`, `lpad`,
///   `rpad`, `translate`, `locate`, `instr`, `concat_ws`, `format_string`,
///   `substring_index`, `cast`, `when` — mixed.
const COLUMN_REF_FUNCTIONS: &[&str] = &[
    // Aggregation — single column or all-column args.
    "sum",
    "avg",
    "mean",
    "max",
    "min",
    "count",
    "countDistinct",
    "approx_count_distinct",
    "median",
    "percentile",
    "percentile_approx",
    "var_pop",
    "var_samp",
    "variance",
    "stddev",
    "stddev_pop",
    "stddev_samp",
    "first",
    "first_value",
    "last",
    "last_value",
    "max_by",
    "min_by",
    "collect_list",
    "collect_set",
    "skewness",
    "kurtosis",
    "corr",
    "covar_pop",
    "covar_samp",
    "grouping",
    // Spark 3.4+ additions — every positional column-name string is a ref.
    "any_value",
    "array_agg",
    "count_if",
    "try_divide",
    "date_diff",
    "unix_date",
    "get",
    // Window
    "row_number",
    "rank",
    "dense_rank",
    "percent_rank",
    "cume_dist",
    "ntile",
    "lag",
    "lead",
    "nth_value",
    // Window-spec builders — `Window.partitionBy("city").orderBy("amount")`.
    // Every string arg is a column name; the spec is checked against the
    // schema of the DataFrame the surrounding `.over(...)` is applied to.
    // (`orderBy` is also a DataFrame method, but that form is a method
    // call routed through `analyze_method_call`, never seen here.)
    "partitionBy",
    "orderBy",
    // Date / time — single-column extractors and arithmetic helpers
    // where any non-column arg is an int (not a string).
    "year",
    "month",
    "day",
    "dayofmonth",
    "dayofweek",
    "dayofyear",
    "hour",
    "minute",
    "second",
    "weekofyear",
    "quarter",
    "last_day",
    "date_add",
    "date_sub",
    "add_months",
    "months_between",
    "datediff",
    // String — single-column or all-column-arg helpers
    "length",
    "char_length",
    "character_length",
    "lower",
    "upper",
    "initcap",
    "trim",
    "ltrim",
    "rtrim",
    "reverse",
    "ascii",
    "soundex",
    "base64",
    "unbase64",
    "concat",
    // Math — single-column or column + int helpers
    "abs",
    "ceil",
    "ceiling",
    "floor",
    "round",
    "bround",
    "sqrt",
    "exp",
    "ln",
    "log",
    "log2",
    "log10",
    "log1p",
    "expm1",
    "pow",
    "power",
    "signum",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "degrees",
    "radians",
    "factorial",
    "hypot",
    "negative",
    "positive",
    // Null handling — every string arg is a column name.
    "isnan",
    "isnull",
    "coalesce",
    "nanvl",
    "nullif",
    "ifnull",
    "nvl",
    "nvl2",
    "least",
    "greatest",
    // Hash / misc column-y — every string is a column.
    "hash",
    "md5",
    "sha1",
    "sha2",
    "crc32",
    "monotonically_increasing_id",
    "spark_partition_id",
    "input_file_name",
    "asc",
    "asc_nulls_first",
    "asc_nulls_last",
    "desc",
    "desc_nulls_first",
    "desc_nulls_last",
    "col",
    "column",
    "size",
    "sort_array",
    "array",
    "array_distinct",
    "array_except",
    "array_intersect",
    "array_union",
    "array_max",
    "array_min",
    "array_sort",
    "explode",
    "explode_outer",
    "posexplode",
    "posexplode_outer",
    "map_keys",
    "map_values",
    "map_entries",
];

/// Date/time helpers shaped `F.fn(col, format)` — the FIRST positional
/// arg is the column reference; remaining positional args are formats /
/// timezones / values, NOT column names. Listing them in the generic
/// `COLUMN_REF_FUNCTIONS` allowlist would false-positive on those format
/// strings; this list narrows the rule to position 0 only. Non-string
/// args after the first still descend through the generic walker so that
/// `col("…")` inside is reached normally.
const FIRST_ARG_COLUMN_FUNCTIONS: &[&str] = &[
    "to_date",
    "to_timestamp",
    "date_format",
    "trunc",
    "next_day",
    "from_utc_timestamp",
    "to_utc_timestamp",
    "from_unixtime",
    "unix_timestamp",
];

/// `F.date_trunc(format, col)` reverses the usual layout — the format is
/// the first arg, the column is the SECOND. A list-of-one is overkill,
/// but keeping the pattern symmetric with `FIRST_ARG_COLUMN_FUNCTIONS`
/// makes adding any future position-2 entries trivial.
const SECOND_ARG_COLUMN_FUNCTIONS: &[&str] = &["date_trunc"];

/// Array higher-order functions — `F.transform(col, fn)` and friends.
/// The FIRST positional arg is the column (an array); subsequent args
/// are lambdas / accumulator literals which aren't column references.
/// Pykrete doesn't model the lambda's parameter binding, so column refs
/// like `col("y")` inside the lambda body that AREN'T the lambda's own
/// parameter still resolve against the surrounding schema via the
/// default walker. `zip_with(left, right, fn)` is two-column and not
/// listed here — its column args descend through the generic walker.
const ARRAY_HOF_FUNCTIONS: &[&str] = &["transform", "filter", "aggregate", "exists", "forall"];

pub(super) fn collect_col_refs<'a>(
    expr: &'a Expr,
    ctx: &BodyContext<'a>,
    out: &mut Vec<(Option<&'a str>, &'a str, TextRange)>,
) {
    if let Some((name, range)) = col_reference(expr) {
        out.push((None, name, range));
        return;
    }
    // `df.X` attribute access — recognized as a column reference to `X`
    // when `df` is a Name bound to a DataFrame in the current scope. The
    // receiver Name is captured in the first tuple slot so consumers can
    // route the lookup to THIS df's schema rather than the surrounding
    // method-chain's (closes the v1.5 I1 cross-DataFrame leak — e.g.
    // `df.select(df_other.x)` checks `x` on `df_other`, not `df`).
    //
    // Importantly, this filters out things like `F.add_months(...)` —
    // `F` is not in `ctx`, so the attribute is left for the default walker
    // to descend into, and `add_months` is not collected.
    //
    // v1.6 PR-A2 — `loc`/`iloc` are pandas indexer accessors, NOT column
    // names. Without this gate, a nested arg like
    // `pdf.assign(bag=pdf.loc[:, "x"])` walks the subscript-fall-through
    // into `pdf.loc` (Attribute) and false-fires D0030 on `loc`. The
    // literal-form `pdf.loc[:, "x"]` arm in `column_exprs.rs:82-88`
    // handles the inference; this arm needs to skip the accessor names
    // outright. Spec §3.2.
    if let Some(attr) = expr.as_attribute_expr()
        && let Some(name) = attr.value.as_name_expr()
        && ctx.lookup(name.id.as_str()).is_some()
        && !matches!(attr.attr.id.as_str(), "loc" | "iloc")
    {
        out.push((
            Some(name.id.as_str()),
            attr.attr.id.as_str(),
            attr.attr.range,
        ));
        return;
    }
    // `df["X"]` subscript access — the sibling of `df.X`. Real PySpark code
    // uses this ubiquitously (`df["age"]`, `df["name"]`), and a typo in the
    // string slot should be a D0030 just like a typo on `df.X` or
    // `col("X")`. The receiver name must be bound in the current scope
    // (same ctx discriminator as the attribute arm) and the slice must be
    // a string literal — computed subscripts fall through to the default
    // walker. Receiver-Name captured for cross-DataFrame routing as above.
    if let Some(sub) = expr.as_subscript_expr()
        && let Some(name) = sub.value.as_name_expr()
        && ctx.lookup(name.id.as_str()).is_some()
        && let Some(s) = sub.slice.as_string_literal_expr()
    {
        out.push((Some(name.id.as_str()), s.value.to_str(), s.range()));
        return;
    }
    // Recognize `F.sum("x")` and similar — for the listed function names,
    // every string-literal positional arg is a column reference. Non-string
    // args are walked normally.
    if let Some(call) = expr.as_call_expr() {
        let func_name = match call.func.as_ref() {
            Expr::Name(n) => Some(n.id.as_str()),
            Expr::Attribute(a) => Some(a.attr.id.as_str()),
            _ => None,
        };
        // Position-restricted recognizers — date/time and array HOFs
        // where only ONE positional slot carries a column reference. The
        // remaining args (format strings, timezones, lambdas) descend
        // through the default walker so embedded `col("…")` references
        // are still reached.
        if let Some(name) = func_name {
            let position = if FIRST_ARG_COLUMN_FUNCTIONS.contains(&name)
                || ARRAY_HOF_FUNCTIONS.contains(&name)
            {
                Some(0)
            } else if SECOND_ARG_COLUMN_FUNCTIONS.contains(&name) {
                Some(1)
            } else {
                None
            };
            if let Some(col_idx) = position {
                for (i, arg) in call.arguments.args.iter().enumerate() {
                    if i == col_idx {
                        if let Some(s) = arg.as_string_literal_expr() {
                            out.push((None, s.value.to_str(), s.range()));
                        } else {
                            collect_col_refs(arg, ctx, out);
                        }
                    } else {
                        // Other args may carry nested column references
                        // (e.g. an `F.col(...)` inside a lambda body),
                        // but bare string literals are values/formats
                        // and must NOT be treated as column names.
                        if arg.as_string_literal_expr().is_none() {
                            collect_col_refs(arg, ctx, out);
                        }
                    }
                }
                for kw in &call.arguments.keywords {
                    if kw.value.as_string_literal_expr().is_none() {
                        collect_col_refs(&kw.value, ctx, out);
                    }
                }
                collect_col_refs(&call.func, ctx, out);
                return;
            }
        }
        if let Some(name) = func_name
            && COLUMN_REF_FUNCTIONS.contains(&name)
        {
            for arg in &call.arguments.args {
                if let Some(s) = arg.as_string_literal_expr() {
                    out.push((None, s.value.to_str(), s.range()));
                } else {
                    collect_col_refs(arg, ctx, out);
                }
            }
            for kw in &call.arguments.keywords {
                collect_col_refs(&kw.value, ctx, out);
            }
            // Descend into the callee too, so an *earlier* link in a
            // builder chain is still reached — e.g. the `partitionBy`
            // in `Window.partitionBy("city").orderBy("amount")`, which
            // lives in this call's `func`, not its arguments.
            collect_col_refs(&call.func, ctx, out);
            return;
        }
    }
    match expr {
        Expr::Call(c) => {
            collect_col_refs(&c.func, ctx, out);
            for arg in &c.arguments.args {
                collect_col_refs(arg, ctx, out);
            }
            for kw in &c.arguments.keywords {
                collect_col_refs(&kw.value, ctx, out);
            }
        }
        Expr::Attribute(a) => collect_col_refs(&a.value, ctx, out),
        Expr::Subscript(s) => {
            collect_col_refs(&s.value, ctx, out);
            collect_col_refs(&s.slice, ctx, out);
        }
        Expr::BinOp(b) => {
            collect_col_refs(&b.left, ctx, out);
            collect_col_refs(&b.right, ctx, out);
        }
        Expr::UnaryOp(u) => collect_col_refs(&u.operand, ctx, out),
        Expr::Compare(c) => {
            collect_col_refs(&c.left, ctx, out);
            for cmp in &c.comparators {
                collect_col_refs(cmp, ctx, out);
            }
        }
        Expr::BoolOp(b) => {
            for v in &b.values {
                collect_col_refs(v, ctx, out);
            }
        }
        Expr::If(if_exp) => {
            collect_col_refs(&if_exp.test, ctx, out);
            collect_col_refs(&if_exp.body, ctx, out);
            collect_col_refs(&if_exp.orelse, ctx, out);
        }
        Expr::Tuple(t) => {
            for e in &t.elts {
                collect_col_refs(e, ctx, out);
            }
        }
        Expr::List(l) => {
            for e in &l.elts {
                collect_col_refs(e, ctx, out);
            }
        }
        Expr::Starred(s) => collect_col_refs(&s.value, ctx, out),
        _ => {}
    }
}
