//! Call-graph adjudication for `DataFrame[X]` alias sites.
//!
//! v1.6 PR-M3 ships this on top of PR-M2's rewriter: instead of unconditionally
//! rewriting every `DataFrame[X]` to `SparkFrame[X]`, the migrator first walks
//! each binding's downstream usage and classifies as `spark`, `pandas`, or
//! `ambiguous`. The signal set is intentionally small and conservative —
//! per-spec discriminating methods (`withColumn` / `createOrReplaceTempView`
//! on the Spark side; `assign` / `pivot_table` / `.loc` / `.iloc` / `merge` /
//! `pivot` on the pandas side). A binding with no signal stays Spark — that's
//! the v1.3 default, kept as the no-evidence fallback per spec §2.3.

use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_expr, walk_stmt};
use ruff_python_ast::{Expr, Stmt, StmtFunctionDef};
use ruff_python_parser::parse_module;
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::alias_report::{AdjudicatedDialect, AliasSite};
use crate::dataframe::{self, Dialect};
use crate::dialect_signals::{PANDAS_ONLY_SIGNALS, SPARK_DISCRIMINATORS};

// Round-2 reviewer (I2): `AdjudicatedDialect` lives in `alias_report.rs`
// now so `AliasSite` can carry the typed verdict directly. Re-export
// here so existing `alias_adjudicate::AdjudicatedDialect` call sites
// (tests, internal walkers) keep compiling.
pub use crate::alias_report::AdjudicatedDialect as _AdjudicatedDialect;

/// Apply call-graph adjudication to every site, in place: update
/// `resolved_dialect` and `would_be_replacement` to reflect the
/// downstream-usage classification. Ambiguous sites keep the original
/// `DataFrame[X]` text as `would_be_replacement` (so the rewriter is a
/// no-op) and the marker injection is the migrator's job.
///
/// The walker is per-file: each `(path, source)` is parsed once, every
/// function in the module is visited, and the sites belonging to that
/// file are matched up by byte-range containment. Sites that don't sit
/// inside any function (e.g. a top-level `x: DataFrame[Sales] = ...`)
/// stay Spark — they have no enclosing body to walk.
pub fn adjudicate(files: &[(String, String)], sites: &mut [AliasSite]) {
    for (path, source) in files {
        let Ok(parsed) = parse_module(source) else {
            continue;
        };
        for stmt in &parsed.syntax().body {
            adjudicate_stmt(stmt, source, path, sites);
        }
    }
}

fn adjudicate_stmt(stmt: &Stmt, source: &str, path: &str, sites: &mut [AliasSite]) {
    match stmt {
        Stmt::FunctionDef(func) => {
            adjudicate_function(func, source, path, sites);
            for inner in &func.body {
                adjudicate_stmt(inner, source, path, sites);
            }
        }
        Stmt::ClassDef(cls) => {
            for inner in &cls.body {
                adjudicate_stmt(inner, source, path, sites);
            }
        }
        Stmt::If(s) => {
            for inner in &s.body {
                adjudicate_stmt(inner, source, path, sites);
            }
            for clause in &s.elif_else_clauses {
                for inner in &clause.body {
                    adjudicate_stmt(inner, source, path, sites);
                }
            }
        }
        Stmt::For(s) => {
            for inner in &s.body {
                adjudicate_stmt(inner, source, path, sites);
            }
            for inner in &s.orelse {
                adjudicate_stmt(inner, source, path, sites);
            }
        }
        Stmt::While(s) => {
            for inner in &s.body {
                adjudicate_stmt(inner, source, path, sites);
            }
            for inner in &s.orelse {
                adjudicate_stmt(inner, source, path, sites);
            }
        }
        Stmt::With(s) => {
            for inner in &s.body {
                adjudicate_stmt(inner, source, path, sites);
            }
        }
        Stmt::Try(s) => {
            for inner in &s.body {
                adjudicate_stmt(inner, source, path, sites);
            }
            for handler in &s.handlers {
                let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                for inner in &h.body {
                    adjudicate_stmt(inner, source, path, sites);
                }
            }
            for inner in &s.orelse {
                adjudicate_stmt(inner, source, path, sites);
            }
            for inner in &s.finalbody {
                adjudicate_stmt(inner, source, path, sites);
            }
        }
        _ => {}
    }
}

/// Adjudicate every `DataFrame[X]` site that sits in `func`'s signature
/// (parameter annotation, return annotation) or directly inside its body
/// (an `x: DataFrame[X] = ...` ann-assign). The function's body is
/// walked once and method/attr usage on each binding name is bucketed
/// into spark/pandas/both, then each site's resolved dialect is set.
fn adjudicate_function(func: &StmtFunctionDef, source: &str, path: &str, sites: &mut [AliasSite]) {
    let mut bindings: Vec<(String, TextRange)> = Vec::new();

    for param in &*func.parameters.posonlyargs {
        if let Some(ann) = param.parameter.annotation.as_deref() {
            collect_alias_subexprs(ann, |range| {
                bindings.push((param.parameter.name.id.as_str().to_string(), range));
            });
        }
    }
    for param in &*func.parameters.args {
        if let Some(ann) = param.parameter.annotation.as_deref() {
            collect_alias_subexprs(ann, |range| {
                bindings.push((param.parameter.name.id.as_str().to_string(), range));
            });
        }
    }
    for param in &*func.parameters.kwonlyargs {
        if let Some(ann) = param.parameter.annotation.as_deref() {
            collect_alias_subexprs(ann, |range| {
                bindings.push((param.parameter.name.id.as_str().to_string(), range));
            });
        }
    }
    if let Some(vararg) = func.parameters.vararg.as_deref()
        && let Some(ann) = vararg.annotation.as_deref()
    {
        collect_alias_subexprs(ann, |range| {
            bindings.push((vararg.name.id.as_str().to_string(), range));
        });
    }
    if let Some(kwarg) = func.parameters.kwarg.as_deref()
        && let Some(ann) = kwarg.annotation.as_deref()
    {
        collect_alias_subexprs(ann, |range| {
            bindings.push((kwarg.name.id.as_str().to_string(), range));
        });
    }

    for stmt in &func.body {
        if let Stmt::AnnAssign(ann) = stmt
            && let Some(name) = ann.target.as_name_expr()
        {
            collect_alias_subexprs(&ann.annotation, |range| {
                bindings.push((name.id.as_str().to_string(), range));
            });
        }
    }

    if bindings.is_empty() {
        return;
    }

    let names: Vec<&str> = bindings.iter().map(|(n, _)| n.as_str()).collect();
    let mut visitor = UsageVisitor {
        names: &names,
        usage: vec![UsageBuckets::default(); names.len()],
    };
    for stmt in &func.body {
        visitor.visit_stmt(stmt);
    }

    for (i, (_, range)) in bindings.iter().enumerate() {
        let verdict = visitor.usage[i].verdict();
        apply_verdict(sites, path, *range, source, verdict);
    }
}

#[derive(Default, Clone, Copy)]
struct UsageBuckets {
    spark: bool,
    pandas: bool,
}

impl UsageBuckets {
    fn verdict(self) -> AdjudicatedDialect {
        match (self.spark, self.pandas) {
            (true, true) => AdjudicatedDialect::Ambiguous,
            (false, true) => AdjudicatedDialect::Pandas,
            _ => AdjudicatedDialect::Spark,
        }
    }
}

struct UsageVisitor<'a> {
    names: &'a [&'a str],
    usage: Vec<UsageBuckets>,
}

impl<'a> SourceOrderVisitor<'a> for UsageVisitor<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Call(call) = expr
            && let Expr::Attribute(attr) = call.func.as_ref()
            && let Some(name) = attr.value.as_name_expr()
            && let Some(idx) = self.names.iter().position(|n| *n == name.id.as_str())
        {
            let method = attr.attr.id.as_str();
            if SPARK_DISCRIMINATORS.contains(&method) {
                self.usage[idx].spark = true;
            } else if PANDAS_ONLY_SIGNALS.contains(&method) {
                self.usage[idx].pandas = true;
            }
        }
        if let Expr::Attribute(attr) = expr
            && let Some(name) = attr.value.as_name_expr()
            && let Some(idx) = self.names.iter().position(|n| *n == name.id.as_str())
        {
            let symbol = attr.attr.id.as_str();
            if PANDAS_ONLY_SIGNALS.contains(&symbol) {
                self.usage[idx].pandas = true;
            } else if SPARK_DISCRIMINATORS.contains(&symbol) {
                self.usage[idx].spark = true;
            }
        }
        walk_expr(self, expr);
    }
}

/// Walk an annotation expression and call `f(range)` for every
/// `DataFrame[X]` sub-expression (the alias form). Bare `DataFrame`,
/// `DataFrame[X]`, and nested forms like `Optional[DataFrame[X]]` all
/// route here so adjudication keys against the same byte ranges
/// `alias_report::collect_alias_sites` produced.
fn collect_alias_subexprs<F: FnMut(TextRange)>(expr: &Expr, mut f: F) {
    fn walk<F: FnMut(TextRange)>(expr: &Expr, f: &mut F) {
        if let Some(rec) = dataframe::recognize_with_dialect(expr)
            && rec.is_deprecated_alias
        {
            f(expr.range());
            return;
        }
        if let Expr::Subscript(sub) = expr {
            walk(&sub.slice, f);
        }
    }
    walk(expr, &mut f);
}

fn apply_verdict(
    sites: &mut [AliasSite],
    path: &str,
    site_range: TextRange,
    source: &str,
    verdict: AdjudicatedDialect,
) {
    for s in sites.iter_mut() {
        if s.file != path {
            continue;
        }
        if !contains_range(site_range, s.range) {
            continue;
        }
        let raw = &source[s.range];
        s.verdict = Some(verdict);
        match verdict {
            AdjudicatedDialect::Spark => {
                s.resolved_dialect = Dialect::Spark;
                s.would_be_replacement = dataframe::spark_frame_rewrite(raw);
            }
            AdjudicatedDialect::Pandas => {
                s.resolved_dialect = Dialect::Pandas;
                s.would_be_replacement = pandas_frame_rewrite(raw);
            }
            AdjudicatedDialect::Ambiguous => {
                // Per spec §2.3: ambiguous sites are NOT rewritten. The
                // dialect field stays `Spark` as a placeholder for legacy
                // consumers (LSP hover, transpiler) that still read it;
                // new consumers MUST check `verdict` first. Round-2
                // reviewer (I2) added the typed field so future code
                // doesn't have to dig through `would_be_replacement` text.
                s.resolved_dialect = Dialect::Spark;
                s.would_be_replacement = raw.to_string();
            }
        }
    }
}

fn contains_range(outer: TextRange, inner: TextRange) -> bool {
    outer.start() <= inner.start() && inner.end() <= outer.end()
}

/// Rewrite the `DataFrame` prefix of an annotation source text to
/// `PandasFrame`, mirroring [`dataframe::spark_frame_rewrite`].
pub fn pandas_frame_rewrite(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("DataFrame") {
        format!("PandasFrame{rest}")
    } else {
        raw.to_string()
    }
}

/// Whether any site in `path` was adjudicated as ambiguous — used by the
/// migrator to decide whether `# pykrete: ambiguous` markers need to be
/// emitted alongside the alias rewrite. Round-2 reviewer (I2) replaced
/// the prior text-equality heuristic with the typed `verdict` field;
/// sites without a verdict (pre-adjudication, v1.5 behavior) are never
/// ambiguous.
pub fn has_ambiguous_in_file(sites: &[AliasSite], path: &str, _source: &str) -> bool {
    sites
        .iter()
        .filter(|s| s.file == path)
        .any(|s| s.verdict == Some(AdjudicatedDialect::Ambiguous))
}

/// For the migrator: collect every ambiguous site's start offset in
/// `path`, sorted and deduped. The migrator inserts one `# pykrete:
/// ambiguous` comment per distinct line on the line above the site.
/// Round-2 reviewer (I2): switched from text-heuristic to typed verdict.
pub fn ambiguous_site_offsets(sites: &[AliasSite], path: &str, _source: &str) -> Vec<TextSize> {
    let mut out: Vec<TextSize> = sites
        .iter()
        .filter(|s| s.file == path)
        .filter(|s| s.verdict == Some(AdjudicatedDialect::Ambiguous))
        .map(|s| s.range.start())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias_report::collect_alias_sites;

    fn run(src: &str) -> Vec<AliasSite> {
        let files = vec![("test.pyk".to_string(), src.to_string())];
        let mut sites = collect_alias_sites(&files);
        adjudicate(&files, &mut sites);
        sites
    }

    #[test]
    fn pure_spark_usage_stays_spark() {
        let src = "\
def f(df: DataFrame[Sale]) -> int:
    return df.withColumn('x', 1)
";
        let sites = run(src);
        let param = sites.iter().find(|s| s.line == 1).expect("param site");
        assert_eq!(param.resolved_dialect, Dialect::Spark);
        assert_eq!(param.would_be_replacement, "SparkFrame[Sale]");
    }

    #[test]
    fn pure_pandas_usage_becomes_pandas() {
        let src = "\
def f(df: DataFrame[Sale]) -> int:
    return df.assign(x=1)
";
        let sites = run(src);
        let param = sites.iter().find(|s| s.line == 1).expect("param site");
        assert_eq!(param.resolved_dialect, Dialect::Pandas);
        assert_eq!(param.would_be_replacement, "PandasFrame[Sale]");
    }

    #[test]
    fn mixed_usage_is_ambiguous_and_leaves_text_alone() {
        let src = "\
def f(df: DataFrame[Sale]) -> int:
    df.withColumn('x', 1)
    return df.assign(y=2)
";
        let sites = run(src);
        let param = sites.iter().find(|s| s.line == 1).expect("param site");
        assert_eq!(param.would_be_replacement, "DataFrame[Sale]");
    }

    #[test]
    fn no_usage_defaults_to_spark() {
        let src = "\
def f(df: DataFrame[Sale]) -> int:
    return 1
";
        let sites = run(src);
        let param = sites.iter().find(|s| s.line == 1).expect("param site");
        assert_eq!(param.would_be_replacement, "SparkFrame[Sale]");
    }

    #[test]
    fn loc_attribute_alone_signals_pandas() {
        let src = "\
def f(df: DataFrame[Sale]) -> int:
    return df.loc[0]
";
        let sites = run(src);
        let param = sites.iter().find(|s| s.line == 1).expect("param site");
        assert_eq!(param.resolved_dialect, Dialect::Pandas);
        assert_eq!(param.would_be_replacement, "PandasFrame[Sale]");
    }

    #[test]
    fn local_annassign_is_adjudicated() {
        let src = "\
def f():
    df: DataFrame[Sale] = read_sales()
    return df.iloc[0]
";
        let sites = run(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].would_be_replacement, "PandasFrame[Sale]");
    }

    #[test]
    fn return_slot_with_no_body_reference_stays_spark() {
        let src = "\
def f(x: int) -> DataFrame[Sale]:
    return x
";
        let sites = run(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].would_be_replacement, "SparkFrame[Sale]");
    }

    #[test]
    fn nested_function_body_is_walked() {
        let src = "\
def outer():
    def inner(df: DataFrame[Sale]) -> int:
        return df.assign(x=1)
    return inner
";
        let sites = run(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].would_be_replacement, "PandasFrame[Sale]");
    }

    #[test]
    fn bare_dataframe_with_pandas_usage_becomes_bare_pandasframe() {
        let src = "\
def f(df: DataFrame) -> int:
    return df.assign(x=1)
";
        let sites = run(src);
        let param = sites.iter().find(|s| s.line == 1).expect("param");
        assert_eq!(param.would_be_replacement, "PandasFrame");
    }

    #[test]
    fn ambiguous_helper_finds_marker_target() {
        let src = "\
def f(df: DataFrame[Sale]) -> int:
    df.withColumn('x', 1)
    return df.assign(y=2)
";
        let files = vec![("test.pyk".to_string(), src.to_string())];
        let mut sites = collect_alias_sites(&files);
        adjudicate(&files, &mut sites);
        assert!(has_ambiguous_in_file(&sites, "test.pyk", src));
        assert_eq!(ambiguous_site_offsets(&sites, "test.pyk", src).len(), 1);
    }
}
