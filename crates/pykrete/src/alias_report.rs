//! `pykrete check --report-aliases` — inventory of every deprecated
//! `DataFrame[X]` alias site across a project.
//!
//! v1.5 PR-D ships visibility only: this walker collects sites; the
//! migrator binary that rewrites them lands in v1.6 paired with D0090
//! `warning → error` escalation (spec §5, §9.2). Per spec round-2
//! resolution, `resolvedDialect` is always `"spark"` in v1.5 — the
//! reserved `"ambiguous"` discriminator is v1.6's call-graph
//! adjudication and is not emitted here.

use ruff_python_ast::Expr;
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_expr};
use ruff_python_parser::parse_module;
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{self, Dialect};

/// One reported alias site. `file` / `line` / `column` together form the
/// stable identifier downstream tooling (e.g. v1.6 `pykrete migrate`)
/// keys against; positions are 1-indexed to match the `--format json`
/// diagnostic output and most editor gutters. `range` is the source
/// byte range of the alias expression (`DataFrame` or `DataFrame[X]`),
/// used by the v1.6 PR-M2 in-place rewriter for token-preserving edits.
#[derive(Debug, Clone)]
pub struct AliasSite {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub resolved_dialect: Dialect,
    pub would_be_replacement: String,
    pub range: TextRange,
}

/// Walk every analyzed file's AST and collect every `DataFrame[X]`
/// alias site (including bare `DataFrame` and nested forms like
/// `Dict[str, DataFrame[X]]`). Unparseable files are skipped silently
/// — the diagnostic surface (D0001) is the channel for parse errors,
/// not this report.
pub fn collect_alias_sites(files: &[(String, String)]) -> Vec<AliasSite> {
    let mut out = Vec::new();
    for (path, source) in files {
        let Ok(parsed) = parse_module(source) else {
            continue;
        };
        let line_index = LineIndex::from_source_text(source);
        let mut visitor = AliasVisitor {
            source,
            line_index: &line_index,
            file: path,
            sites: &mut out,
        };
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }
    }
    out
}

struct AliasVisitor<'a> {
    source: &'a str,
    line_index: &'a LineIndex,
    file: &'a str,
    sites: &'a mut Vec<AliasSite>,
}

impl<'a> SourceOrderVisitor<'a> for AliasVisitor<'a> {
    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Some(rec) = dataframe::recognize_with_dialect(expr)
            && rec.is_deprecated_alias
        {
            let range = expr.range();
            let start = self.line_index.line_column(range.start(), self.source);
            let raw_text = &self.source[range];
            self.sites.push(AliasSite {
                file: self.file.to_string(),
                line: start.line.get(),
                column: start.column.get(),
                resolved_dialect: rec.dialect,
                would_be_replacement: dataframe::spark_frame_rewrite(raw_text),
                range,
            });
            // Don't descend into a recognized alias — the inner bare
            // `DataFrame` of `DataFrame[Sales]` would otherwise be
            // re-recognized as a second (untyped) site at the same
            // position. The schema-name leaf isn't a frame annotation
            // on its own, so skipping the subtree loses nothing.
            return;
        }
        walk_expr(self, expr);
    }
}

/// Serialize an in-memory alias inventory to the spec §5.1 JSON shape.
/// `resolvedDialect` is always `"spark"` in v1.5 — the only path that
/// reaches this serializer is `DataFrame[X]` (the deprecated alias),
/// which `dataframe::recognize_with_dialect` always tags as Spark.
pub fn render_alias_report_json(sites: &[AliasSite]) -> String {
    let aliases: Vec<serde_json::Value> = sites
        .iter()
        .map(|s| {
            let dialect = match s.resolved_dialect {
                Dialect::Spark => "spark",
                Dialect::Pandas => "pandas",
            };
            serde_json::json!({
                "file": s.file,
                "line": s.line,
                "column": s.column,
                "resolvedDialect": dialect,
                "wouldBeReplacement": s.would_be_replacement,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "aliasReportVersion": "1",
        "aliases": aliases,
    });
    serde_json::to_string_pretty(&payload)
        .expect("alias report JSON is composed of types that always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(src: &str) -> Vec<AliasSite> {
        collect_alias_sites(&[("test.pyk".to_string(), src.to_string())])
    }

    #[test]
    fn single_dataframe_param_annotation_is_reported() {
        let src = "\
class Sales(Schema):
    id: IntegerType

def f(df: DataFrame[Sales]) -> DataFrame[Sales]:
    return df
";
        let sites = collect(src);
        assert_eq!(sites.len(), 2, "param + return: {sites:?}");
        assert!(sites.iter().all(|s| s.resolved_dialect == Dialect::Spark));
        assert!(
            sites
                .iter()
                .all(|s| s.would_be_replacement == "SparkFrame[Sales]")
        );
    }

    #[test]
    fn sparkframe_x_is_not_reported() {
        let src = "\
def f(df: SparkFrame[Sales]) -> SparkFrame[Sales]:
    return df
";
        assert!(collect(src).is_empty());
    }

    #[test]
    fn pandasframe_x_is_not_reported() {
        let src = "\
def f(df: PandasFrame[Sales]) -> PandasFrame[Sales]:
    return df
";
        assert!(collect(src).is_empty());
    }

    #[test]
    fn no_dataframe_annotations_means_empty_report() {
        let src = "\
def f(x: int) -> int:
    return x
";
        assert!(collect(src).is_empty());
    }

    #[test]
    fn ann_assign_dataframe_alias_is_reported() {
        let src = "\
def f():
    df: DataFrame[Sales] = read_sales()
    return df
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].would_be_replacement, "SparkFrame[Sales]");
    }

    #[test]
    fn nested_dataframe_alias_is_reported() {
        // `Dict[str, DataFrame[Sales]]` — the inner DataFrame[X] still
        // resolves to the Spark alias and the walker still records it.
        let src = "\
def f() -> Dict[str, DataFrame[Sales]]:
    return {}
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1, "nested form: {sites:?}");
        assert_eq!(sites[0].would_be_replacement, "SparkFrame[Sales]");
    }

    #[test]
    fn bare_dataframe_is_reported_as_alias_with_canonical_rewrite() {
        let src = "\
def f(df: DataFrame) -> DataFrame:
    return df
";
        let sites = collect(src);
        assert_eq!(sites.len(), 2);
        assert!(sites.iter().all(|s| s.would_be_replacement == "SparkFrame"));
    }

    #[test]
    fn json_render_has_alias_report_version_and_aliases_array() {
        let src = "def f(df: DataFrame[Sales]) -> int: ...";
        let sites = collect(src);
        let json = render_alias_report_json(&sites);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["aliasReportVersion"], "1");
        let aliases = v["aliases"].as_array().expect("array");
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[0]["resolvedDialect"], "spark");
        assert_eq!(aliases[0]["wouldBeReplacement"], "SparkFrame[Sales]");
        assert_eq!(aliases[0]["file"], "test.pyk");
    }

    #[test]
    fn empty_alias_report_still_has_payload_envelope() {
        let json = render_alias_report_json(&[]);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["aliasReportVersion"], "1");
        assert!(v["aliases"].as_array().expect("array").is_empty());
    }
}
