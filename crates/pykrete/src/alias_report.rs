//! `pykrete check --report-aliases` — inventory of every deprecated
//! `DataFrame[X]` alias site across a project.
//!
//! v1.5 PR-D shipped visibility: this walker collects sites tagged with
//! the default Spark dialect. v1.6 PR-M3 layers call-graph adjudication
//! (see [`crate::alias_adjudicate`]) on top — the walker still emits
//! every site at the parser-level Spark default, then `adjudicate()`
//! revisits each binding's downstream usage and re-tags it as Spark,
//! Pandas, or ambiguous. The JSON serializer below renders all three
//! discriminators; ambiguous sites are the case where
//! `would_be_replacement` equals the raw source text verbatim (the
//! rewriter is a no-op and the migrator emits `# pykrete: ambiguous`).

use ruff_python_ast::Expr;
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_expr};
use ruff_python_parser::parse_module;
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::dataframe::{self, Dialect};

/// What the v1.6 PR-M3 call-graph walk decided for one binding. Lives
/// here (not in `alias_adjudicate`) so `AliasSite` can carry the typed
/// verdict directly — without this, consumers had to recover the
/// "ambiguous" verdict from a `would_be_replacement.starts_with(...)`
/// text heuristic, which is one schema rename away from silently
/// mis-classifying every site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjudicatedDialect {
    Spark,
    Pandas,
    Ambiguous,
}

/// One reported alias site. `file` / `line` / `column` together form the
/// stable identifier downstream tooling (e.g. v1.6 `pykrete migrate`)
/// keys against; positions are 1-indexed to match the `--format json`
/// diagnostic output and most editor gutters. `range` is the source
/// byte range of the alias expression (`DataFrame` or `DataFrame[X]`),
/// used by the v1.6 PR-M2 in-place rewriter for token-preserving edits.
/// `verdict` is the v1.6 PR-M3 call-graph adjudication verdict; `None`
/// before adjudication runs (v1.5/PR-M1/PR-M2 behavior), `Some(...)`
/// after `alias_adjudicate::apply_verdicts`. The JSON serializer and
/// migrator both branch on this field directly — no text heuristics.
#[derive(Debug, Clone)]
pub struct AliasSite {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub resolved_dialect: Dialect,
    pub would_be_replacement: String,
    pub range: TextRange,
    pub verdict: Option<AdjudicatedDialect>,
    /// Raw source text of the alias annotation (`DataFrame[Sale]`,
    /// `DataFrame`, etc.) — the exact bytes the walker saw. v1.8 PR-V1's
    /// `--deprecation-report` JSON envelope surfaces this so consumers
    /// don't have to reconstruct it from `would_be_replacement` + the
    /// verdict.
    pub raw_text: String,
    /// Name of the binding the alias annotates (parameter name or
    /// ann-assign target). Populated by v1.6 PR-M3's `apply_verdict`
    /// when the adjudicator visits an enclosing function; `None` for
    /// sites the adjudicator didn't visit (module-level annotations,
    /// return slots).
    pub binding_name: Option<String>,
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
                verdict: None,
                raw_text: raw_text.to_string(),
                binding_name: None,
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
/// `resolvedDialect` ranges over `"spark"` / `"pandas"` / `"ambiguous"`
/// after v1.6 PR-M3 adjudication. Branches on the typed `verdict` field;
/// `None` (pre-adjudication, v1.5 behavior) falls back to `"spark"` per
/// spec §5.1 ("v1.5 reports every site as `spark`").
///
/// `aliasReportVersion` is `"2"` as of v1.6: v1 (v1.5) emitted only
/// `"spark"` for `resolvedDialect`; v2 (v1.6+) expands the value set to
/// `{"spark", "pandas", "ambiguous"}`. Per the policy at
/// `main.rs:336-343` ("changing its meaning: breaking — bump"), the
/// value-set expansion bumps the envelope version even though the
/// field name is unchanged. Consumers that switched on `"spark"` only
/// need to handle the new discriminators.
pub fn render_alias_report_json(sites: &[AliasSite]) -> String {
    let aliases: Vec<serde_json::Value> = sites
        .iter()
        .map(|s| {
            let dialect = match s.verdict {
                Some(AdjudicatedDialect::Spark) => "spark",
                Some(AdjudicatedDialect::Pandas) => "pandas",
                Some(AdjudicatedDialect::Ambiguous) => "ambiguous",
                None => "spark",
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
        "aliasReportVersion": "2",
        "aliases": aliases,
    });
    serde_json::to_string_pretty(&payload)
        .expect("alias report JSON is composed of types that always serialize")
}

/// Serialize an in-memory alias inventory to the v1.8 PR-V1
/// `--deprecation-report` envelope. Every alias site is by definition
/// a D0090-firing site, so the filter is structural: every member of
/// `sites` is included. Versioned envelope so downstream CI gates can
/// pin (`deprecationReportVersion: "1"`). The per-site verdict mirrors
/// the alias report's adjudication; `suggestedRewrite` is `null` for
/// ambiguous sites because the migrator leaves them intact for human
/// review.
pub fn render_deprecation_report_json(sites: &[AliasSite]) -> String {
    let mut spark = 0usize;
    let mut pandas = 0usize;
    let mut ambiguous = 0usize;
    let site_values: Vec<serde_json::Value> = sites
        .iter()
        .map(|s| {
            let (verdict_str, suggested) = match s.verdict {
                Some(AdjudicatedDialect::Spark) => {
                    spark += 1;
                    (
                        "spark",
                        serde_json::Value::String(s.would_be_replacement.clone()),
                    )
                }
                Some(AdjudicatedDialect::Pandas) => {
                    pandas += 1;
                    (
                        "pandas",
                        serde_json::Value::String(s.would_be_replacement.clone()),
                    )
                }
                Some(AdjudicatedDialect::Ambiguous) => {
                    ambiguous += 1;
                    ("ambiguous", serde_json::Value::Null)
                }
                None => {
                    spark += 1;
                    (
                        "spark",
                        serde_json::Value::String(s.would_be_replacement.clone()),
                    )
                }
            };
            serde_json::json!({
                "file": s.file,
                "line": s.line,
                "column": s.column,
                "code": "D0090",
                "ruleName": "deprecatedDataFrameAlias",
                "bindingName": s.binding_name,
                "rawAnnotation": s.raw_text,
                "adjudicatedDialect": verdict_str,
                "suggestedRewrite": suggested,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "deprecationReportVersion": "1",
        "sites": site_values,
        "summary": {
            "totalSites": sites.len(),
            "byDialect": {
                "spark": spark,
                "pandas": pandas,
                "ambiguous": ambiguous,
            }
        }
    });
    serde_json::to_string_pretty(&payload)
        .expect("deprecation report JSON is composed of types that always serialize")
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
        assert_eq!(v["aliasReportVersion"], "2");
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
        assert_eq!(v["aliasReportVersion"], "2");
        assert!(v["aliases"].as_array().expect("array").is_empty());
    }
}
