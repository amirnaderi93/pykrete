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

/// v1.9 PR-V1 — per-site migration tracking for the
/// `--deprecation-report` v2 envelope. The pykrete binary emits only
/// `Pending` (no marker) and `Acknowledged` (line-above
/// `# pykrete: ack-deprecation` marker). `Resolved` is a value reserved
/// in the schema for stateful delta tracking by external consumers: a
/// consumer that retains the prior report can mark sites that no longer
/// appear in the current envelope as `resolved`. The binary itself
/// never emits `Resolved`; it only reports the state it currently
/// sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStatus {
    Pending,
    Acknowledged,
}

impl MigrationStatus {
    fn as_str(self) -> &'static str {
        match self {
            MigrationStatus::Pending => "pending",
            MigrationStatus::Acknowledged => "acknowledged",
        }
    }
}

/// v1.9 PR-V1 — `--ack=<value>` filter for the deprecation envelope.
/// `resolved` is rejected at parse time per spec §4.7: pykrete never
/// emits sites with that status (it's a consumer-side stateful-delta
/// sentinel), so accepting the value here would only mask user
/// confusion (`--ack=resolved` always returns an empty array, which
/// looks the same as "no migration work left"). Out-of-domain → exit 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckFilter {
    Pending,
    Acknowledged,
}

impl AckFilter {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(AckFilter::Pending),
            "acknowledged" => Some(AckFilter::Acknowledged),
            _ => None,
        }
    }

    /// True when `status` survives this filter. Public so the CLI
    /// (v1.10 PR-D1 `--fail-on-nonempty`) can compute the post-filter
    /// site count without re-rendering or re-parsing the JSON envelope.
    pub fn matches(self, status: MigrationStatus) -> bool {
        matches!(
            (self, status),
            (AckFilter::Pending, MigrationStatus::Pending)
                | (AckFilter::Acknowledged, MigrationStatus::Acknowledged)
        )
    }
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
    /// v1.9 PR-V1 — per-site migration state for the v2
    /// `--deprecation-report` envelope. `Acknowledged` when a
    /// `# pykrete: ack-deprecation` comment sits on the line immediately
    /// above this site (mirroring the v1.6 ambiguous-marker mechanic);
    /// `Pending` otherwise. Set by the walker at site-collection time.
    pub migration_status: MigrationStatus,
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
            let migration_status = if line_above_has_ack_marker(self.source, range.start().into()) {
                MigrationStatus::Acknowledged
            } else {
                MigrationStatus::Pending
            };
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
                migration_status,
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

/// v1.9 PR-V1 + v1.10 PR-A1 + v1.12 PR-V1 — mirrors the v1.6
/// `# pykrete: ambiguous` line-above lookup. Returns `true` when the
/// marker `# pykrete: ack-deprecation` sits on any line of the
/// contiguous comment block directly above the enclosing
/// `def`/`async def` (walking past any decorator stack), or, for
/// module-level annotations with no enclosing def, directly above the
/// annotation itself. Case-sensitive; leading whitespace tolerated;
/// trailing whitespace tolerated. The annotation's offset is the byte
/// position of the `DataFrame` token inside `DataFrame[X]` (or the
/// bare `DataFrame` annotation).
///
/// v1.10 PR-A1 closes v1.9 arch-I1: pre-fix, the lookup checked the
/// line immediately above the annotation's byte offset, which on a
/// multi-line signature
/// (`def f(\n    a: DataFrame[X],\n) -> int:`) landed on the `def f(`
/// continuation line instead of the user-intended marker line above
/// `def f(`. The walk now anchors at the enclosing `def` line so the
/// "line above" answer is invariant under signature line breaks.
///
/// The "enclosing" check is indentation-aware: the candidate `def`
/// must sit at strictly less leading whitespace than the annotation
/// line. Without this, a module-level annotation that appears below
/// an earlier sibling `def` would mis-anchor at that sibling and
/// regress the v1.9 module-level "line directly above annotation"
/// behavior. Module-level annotations (zero leading whitespace) never
/// find an enclosing def regardless of what `def`s precede them.
///
/// The walk also steps past any contiguous `@decorator` lines sitting
/// directly above the `def`, so
/// `# pykrete: ack-deprecation\n@cached\ndef f(a: DataFrame[X]) ...`
/// reads as acknowledged. Only single-line decorators are recognized;
/// multi-line decorator calls require the marker on the line directly
/// above the `def`.
///
/// v1.12 PR-V1 — multi-line rationale block (shape (b)). The marker
/// no longer needs to sit on the line directly above the anchor; it
/// can appear on any line of the contiguous `#`-prefixed comment
/// block above the anchor:
///
/// ```text
/// # pykrete: ack-deprecation
/// # rationale: legacy code, will migrate in Q3
/// x: DataFrame[X] = ...
/// ```
///
/// Walk stops at the first non-comment line (blank line, code line,
/// or anything else). The marker may appear anywhere in the
/// contiguous block — top, middle, or bottom — to acknowledge the
/// site. An empty-comment line (`#` alone, or `#` followed by
/// whitespace) preserves contiguity since it remains `#`-prefixed
/// after trim.
fn line_above_has_ack_marker(source: &str, offset: usize) -> bool {
    const MARKER: &str = "# pykrete: ack-deprecation";
    if source.is_empty() {
        return false;
    }
    let bytes = source.as_bytes();
    let upper = offset.min(bytes.len());
    let annot_line_start = bytes[..upper]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let annot_indent = leading_indent(source, annot_line_start);
    let def_anchor = find_enclosing_def_line_start(source, annot_line_start, annot_indent);
    let anchor_start = match def_anchor {
        Some(def_start) => walk_past_decorators(source, def_start),
        None => annot_line_start,
    };
    let mut cursor = anchor_start;
    while cursor > 0 {
        let prev_line_end = cursor - 1;
        let prev_line_start = bytes[..prev_line_end]
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let line = &source[prev_line_start..prev_line_end];
        let trimmed = line.trim();
        if trimmed == MARKER {
            return true;
        }
        if !trimmed.starts_with('#') {
            return false;
        }
        cursor = prev_line_start;
    }
    false
}

/// Walk backwards from `from_line_start` (a line's first-byte offset)
/// looking for the nearest line whose trimmed content matches
/// `def NAME(` or `async def NAME(` AND whose leading indentation is
/// strictly less than `annot_indent` — i.e. the candidate def actually
/// encloses the annotation. Returns the line's first-byte offset, or
/// `None` when no enclosing def exists (module-level annotations, or
/// annotations whose only preceding `def` is at equal/greater indent
/// and therefore sibling/inner rather than enclosing).
fn find_enclosing_def_line_start(
    source: &str,
    from_line_start: usize,
    annot_indent: usize,
) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut line_start = from_line_start;
    loop {
        let line_end = source[line_start..]
            .find('\n')
            .map(|p| line_start + p)
            .unwrap_or(source.len());
        let line = &source[line_start..line_end];
        if is_def_signature_line(line) {
            let candidate_indent = leading_indent(source, line_start);
            // The annot is on the def line itself (single-line
            // signature: `def f(df: DataFrame[X]) ...`) → trivially
            // enclosing. Otherwise the def must sit at strictly less
            // indent than the annotation line to qualify as enclosing
            // (a same-indent or deeper def is a sibling/inner, not an
            // ancestor).
            if line_start == from_line_start || candidate_indent < annot_indent {
                return Some(line_start);
            }
        }
        if line_start == 0 {
            return None;
        }
        let prev_line_end = line_start - 1;
        let prev_line_start = bytes[..prev_line_end]
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        line_start = prev_line_start;
    }
}

/// Given the first-byte offset of a `def`/`async def` line, walk upward
/// past any contiguous decorator lines (lines whose first non-whitespace
/// character is `@`) and return the topmost line's first-byte offset.
/// The marker check then asks "what's the line above the decorator
/// stack?" instead of "what's the line above the def?", so
///
/// ```text
/// # pykrete: ack-deprecation
/// @decorator
/// def f(a: DataFrame[X]) -> int: ...
/// ```
///
/// reads as acknowledged. Only single-line decorators (`@deco`,
/// `@deco(...)`) are recognized — multi-line decorator calls
/// (`@deco(\n    arg,\n)`) are rare and treated as non-decorator lines,
/// so the marker must sit on the line directly above the `def` in that
/// case.
fn walk_past_decorators(source: &str, def_line_start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut anchor = def_line_start;
    while anchor > 0 {
        let prev_line_end = anchor - 1;
        let prev_line_start = bytes[..prev_line_end]
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let prev_line = &source[prev_line_start..prev_line_end];
        if prev_line.trim_start().starts_with('@') {
            anchor = prev_line_start;
        } else {
            break;
        }
    }
    anchor
}

/// Count of leading ASCII space/tab bytes on the line beginning at
/// `line_start`. Tabs count as 1 byte each — pykrete doesn't model
/// tab-stop width because the indentation-aware comparison only needs
/// the relative ordering "annotation is deeper than candidate def,"
/// which holds under any consistent tab/space treatment as long as the
/// source file is itself self-consistent.
fn leading_indent(source: &str, line_start: usize) -> usize {
    source[line_start..]
        .bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count()
}

/// True when `line` (no trailing newline) reads as the opening line of
/// a `def NAME(` or `async def NAME(` signature. Allows leading
/// whitespace and any content after the open paren — the line need not
/// contain the matching close paren, so multi-line param lists qualify.
fn is_def_signature_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("async ")
        .map(str::trim_start)
        .unwrap_or(trimmed);
    let Some(after_def_keyword) = rest.strip_prefix("def ") else {
        return false;
    };
    let after_name = after_def_keyword.trim_start();
    let mut chars = after_name.char_indices();
    let Some((_, first)) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    let mut last_name_end = first.len_utf8();
    for (i, c) in chars {
        if c.is_alphanumeric() || c == '_' {
            last_name_end = i + c.len_utf8();
            continue;
        }
        break;
    }
    after_name[last_name_end..].trim_start().starts_with('(')
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

/// Serialize an in-memory alias inventory to the v1.9 PR-V1
/// `--deprecation-report` v2 envelope. Every alias site is a D0090-
/// firing site, so the structural filter is "every member of `sites`."
/// `ack_filter` (the `--ack=<value>` CLI flag) narrows further:
/// `Some(AckFilter::Pending)` keeps only `migration_status == "pending"`,
/// `Some(AckFilter::Acknowledged)` keeps only `migration_status ==
/// "acknowledged"`, and `None` keeps all. `summary` counts reflect the
/// filtered set so consumers consume `summary.totalSites` directly.
///
/// v2 schema additions over v1 (v1.8 PR-V1):
/// - `deprecationReportVersion` bumped `"1"` → `"2"`.
/// - Per-site `migrationStatus`: `"pending"` (default — alias still in
///   source, no marker) or `"acknowledged"` (line-above marker present).
///   The pykrete binary never emits `"resolved"`; that value is
///   reserved in the schema for stateful delta tracking by external
///   consumers maintaining a prior-report cache.
///
/// Per spec §4.5 carve-out: NO `target_version` field. NO calendar
/// quarter. NO date marker. The envelope is a planning instrument;
/// the ship-date is product fork, not committee fork.
///
/// v1.14 PR-D3 addition: emits top-level `pykreteSourceCommit` and
/// `generatedAt` keys (`null` when `provenance` carries `None` for the
/// respective field). These round-trip through `--compare-to` as
/// `snapshot_a` provenance, closing the spec promise that v1.14+
/// snapshots preserve their source-of-truth context across diffs.
pub fn render_deprecation_report_json(
    sites: &[AliasSite],
    ack_filter: Option<AckFilter>,
    provenance: &crate::compare_to::SnapshotProvenance,
) -> String {
    // Counters are post-filter: .map() runs after .filter(), so spark/pandas/ambiguous
    // reflect the --ack-filtered envelope, not the raw site set. Intentional.
    let mut spark = 0usize;
    let mut pandas = 0usize;
    let mut ambiguous = 0usize;
    let site_values: Vec<serde_json::Value> = sites
        .iter()
        .filter(|s| match ack_filter {
            Some(f) => f.matches(s.migration_status),
            None => true,
        })
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
                "migrationStatus": s.migration_status.as_str(),
            })
        })
        .collect();
    let total = site_values.len();
    let sha_value = provenance
        .sha
        .as_deref()
        .map_or(serde_json::Value::Null, |s| {
            serde_json::Value::String(s.to_owned())
        });
    let ts_value = provenance
        .timestamp
        .as_deref()
        .map_or(serde_json::Value::Null, |s| {
            serde_json::Value::String(s.to_owned())
        });
    let payload = serde_json::json!({
        "deprecationReportVersion": "2",
        "pykreteSourceCommit": sha_value,
        "generatedAt": ts_value,
        "sites": site_values,
        "summary": {
            "totalSites": total,
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

    // v1.9 PR-V1 — migration_status walker behavior

    #[test]
    fn no_ack_marker_means_pending() {
        let src = "def f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    #[test]
    fn ack_marker_on_line_above_means_acknowledged() {
        let src = "# pykrete: ack-deprecation\ndef f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn ack_marker_two_lines_above_is_pending() {
        let src = "# pykrete: ack-deprecation\n\ndef f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    #[test]
    fn ack_filter_parse_accepts_pending_and_acknowledged_only() {
        assert_eq!(AckFilter::parse("pending"), Some(AckFilter::Pending));
        assert_eq!(
            AckFilter::parse("acknowledged"),
            Some(AckFilter::Acknowledged)
        );
        assert_eq!(AckFilter::parse("resolved"), None);
        assert_eq!(AckFilter::parse("garbage"), None);
        assert_eq!(AckFilter::parse(""), None);
    }

    #[test]
    fn deprecation_report_v2_envelope_has_version_2() {
        let src = "def f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        let json = render_deprecation_report_json(
            &sites,
            None,
            &crate::compare_to::SnapshotProvenance::default(),
        );
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["deprecationReportVersion"], "2");
    }

    #[test]
    fn deprecation_report_v2_per_site_has_migration_status() {
        let src = "def f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        let json = render_deprecation_report_json(
            &sites,
            None,
            &crate::compare_to::SnapshotProvenance::default(),
        );
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["sites"][0]["migrationStatus"], "pending");
    }

    #[test]
    fn deprecation_report_v2_emits_provenance_keys_when_present() {
        // v1.14 PR-D3 BLOCKER #1 fix: envelope ALWAYS emits both
        // top-level provenance keys; when the caller threads non-None
        // values, those values surface verbatim.
        let src = "def f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        let prov = crate::compare_to::SnapshotProvenance {
            sha: Some("deadbeef".to_owned()),
            timestamp: Some("2026-06-23T12:00:00Z".to_owned()),
        };
        let json = render_deprecation_report_json(&sites, None, &prov);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["pykreteSourceCommit"], "deadbeef");
        assert_eq!(v["generatedAt"], "2026-06-23T12:00:00Z");
    }

    #[test]
    fn deprecation_report_v2_emits_null_provenance_when_absent() {
        // Default provenance (both fields None) → top-level keys are
        // STILL present but explicitly null. Consumers can rely on the
        // keys existing regardless of capture success.
        let src = "def f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        let json = render_deprecation_report_json(
            &sites,
            None,
            &crate::compare_to::SnapshotProvenance::default(),
        );
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let obj = v.as_object().expect("envelope is an object");
        assert!(
            obj.contains_key("pykreteSourceCommit"),
            "top-level key MUST be present even when null"
        );
        assert!(
            obj.contains_key("generatedAt"),
            "top-level key MUST be present even when null"
        );
        assert!(v["pykreteSourceCommit"].is_null());
        assert!(v["generatedAt"].is_null());
    }

    // v1.10 PR-A1 — multi-line ack-marker walker (arch-I1 closure).

    #[test]
    fn v110_pra1_single_line_def_marker_above_is_acknowledged() {
        // Regression guard for v1.9 single-line behavior.
        let src = "# pykrete: ack-deprecation\ndef f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_multi_line_def_marker_above_def_is_acknowledged() {
        let src = "\
# pykrete: ack-deprecation
def f(
    a: DataFrame[Sale],
) -> int:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1, "param: {sites:?}");
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_multi_line_def_marker_inside_signature_is_pending() {
        // Marker on the line above the annotation (NOT above `def f(`)
        // must NOT acknowledge — the marker contract is "line above the
        // def line."
        let src = "\
def f(
# pykrete: ack-deprecation
    a: DataFrame[Sale],
) -> int:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    #[test]
    fn v110_pra1_multi_line_def_multi_line_annotation_is_acknowledged() {
        let src = "\
# pykrete: ack-deprecation
def f(
    a: DataFrame[
        LongSchemaName
    ],
) -> int:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1, "param: {sites:?}");
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_module_level_annotation_marker_above_is_acknowledged() {
        // Regression guard for v1.9 module-level behavior — when there
        // is no enclosing def, the walker falls back to "line above the
        // annotation."
        let src = "\
# pykrete: ack-deprecation
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_marker_above_contiguous_comment_is_acknowledged() {
        // v1.10 PR-A1 originally pinned this case as `Pending` ("the
        // walk terminates at the def line"). v1.12 PR-V1 ships the
        // rationale-block walker (shape (b)): the marker can sit
        // anywhere in the contiguous `#`-prefixed comment block above
        // the anchor. The unrelated comment line no longer blocks
        // recognition, so this case is now Acknowledged.
        let src = "\
# pykrete: ack-deprecation
# unrelated comment
def f(
    a: DataFrame[Sale],
) -> int:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_multi_line_return_type_acknowledged() {
        let src = "\
# pykrete: ack-deprecation
def f(a: int) -> \\
        DataFrame[Sale]:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1, "return: {sites:?}");
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_async_def_multi_line_marker_above_is_acknowledged() {
        let src = "\
# pykrete: ack-deprecation
async def f(
    a: DataFrame[Sale],
) -> int:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1, "async: {sites:?}");
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_marker_with_trailing_whitespace_is_acknowledged() {
        let src = "# pykrete: ack-deprecation   \ndef f(df: DataFrame[Sale]) -> int: ...\n";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_no_marker_anywhere_is_pending() {
        let src = "\
def f(
    a: DataFrame[Sale],
) -> int:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    // v1.10 PR-A1 R2 — indentation-aware enclosing-def walk + decorator
    // walk-past. The R1 walker was indentation-blind and regressed v1.9
    // module-level behavior when an earlier sibling `def` preceded a
    // module-level annotation (Cases J/K below). The decorator support
    // addresses the spec-silent but real adopter case where the marker
    // sits above an `@decorator` stack on top of the `def`.

    #[test]
    fn v110_pra1_case_j_single_preceding_def_module_level_annotation_acknowledged() {
        // Case J: one earlier function defined above a module-level
        // annotation. The indentation-blind R1 walker mis-anchored at
        // `def earlier_fn(`, found start-of-file above it, and reported
        // `pending`. With the indentation check, the module-level
        // annotation (col 0) cannot be enclosed by any def (also col 0
        // or deeper), so the walker falls through to "line above the
        // annotation."
        let src = "\
def earlier_fn():
    return 1

# pykrete: ack-deprecation
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_case_k_multiple_preceding_defs_module_level_annotation_acknowledged() {
        // Case K: several earlier functions above a module-level
        // annotation. Same regression class as Case J, with more
        // candidate def lines to mis-anchor on.
        let src = "\
def first():
    return 1

def second():
    return 2

def third():
    return 3

# pykrete: ack-deprecation
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_module_level_annotation_with_no_marker_is_pending() {
        // Indentation-aware fallthrough must NOT silently acknowledge
        // module-level annotations — only when the line directly above
        // is the marker.
        let src = "\
def earlier_fn():
    return 1

x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    #[test]
    fn v110_pra1_sibling_def_does_not_anchor_later_def_annotation() {
        // Two sibling top-level defs. Marker above the second def
        // must acknowledge the second def's annotation; the walker
        // must not anchor at `def first(` and treat the marker as
        // "two defs above."
        let src = "\
def first(a: int) -> int:
    return a

# pykrete: ack-deprecation
def second(df: DataFrame[Sale]) -> int:
    return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_decorator_simple_marker_above_decorator_is_acknowledged() {
        // Decorator support: marker above a single `@deco` line above a
        // `def` reads as acknowledged.
        let src = "\
# pykrete: ack-deprecation
@decorator
def f(df: DataFrame[Sale]) -> int:
    return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_decorator_with_args_marker_above_is_acknowledged() {
        let src = "\
# pykrete: ack-deprecation
@decorator(arg=1)
def f(df: DataFrame[Sale]) -> int:
    return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_decorator_stack_marker_above_top_is_acknowledged() {
        // Multiple decorators on the def — marker above the topmost
        // decorator anchors correctly.
        let src = "\
# pykrete: ack-deprecation
@outer
@inner
def f(df: DataFrame[Sale]) -> int:
    return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_decorator_marker_between_decorator_and_def_still_acknowledges() {
        // The walker steps past decorators only when they sit directly
        // above the `def` — if a marker interrupts the decorator/def
        // adjacency, the walker stops at the def line and the marker
        // is read as "line directly above the def." This case stays
        // acknowledged for v1.9 compatibility: pre-PR, v1.9 anchored
        // at the def line and saw the marker on the line above.
        let src = "\
@decorator
# pykrete: ack-deprecation
def f(df: DataFrame[Sale]) -> int:
    return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_pra1_decorator_multi_line_def_marker_above_decorator_is_acknowledged() {
        // Decorator + multi-line def both involved.
        let src = "\
# pykrete: ack-deprecation
@decorator
def f(
    a: DataFrame[Sale],
) -> int:
    return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn deprecation_report_v2_filters_summary_matches_filtered_sites() {
        // Two sites, one acknowledged via marker. Filter to pending →
        // summary.totalSites == 1.
        let src = "\
# pykrete: ack-deprecation
def a(df: DataFrame[Sale]) -> int: ...

def b(df: DataFrame[Sale]) -> int: ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 2);
        let json = render_deprecation_report_json(
            &sites,
            Some(AckFilter::Pending),
            &crate::compare_to::SnapshotProvenance::default(),
        );
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["sites"].as_array().unwrap().len(), 1);
        assert_eq!(v["summary"]["totalSites"], 1);
    }

    // v1.10 PR-G — class-method + nested-def walker coverage. PR-A1
    // tests covered top-level `def` / `async def`; these pin the
    // walker on indented `def`s inside a `class:` body and on `def`s
    // nested inside another function.

    #[test]
    fn v110_prg_class_method_marker_above_def_is_acknowledged() {
        let src = "\
class Repo:
    # pykrete: ack-deprecation
    def fetch(self, df: DataFrame[Sale]) -> int:
        return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_prg_class_method_marker_above_decorator_stack_is_acknowledged() {
        let src = "\
class Repo:
    # pykrete: ack-deprecation
    @staticmethod
    @classmethod
    def fetch(df: DataFrame[Sale]) -> int:
        return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v110_prg_nested_def_marker_above_inner_def_is_acknowledged() {
        let src = "\
def outer():
    # pykrete: ack-deprecation
    def inner(x: DataFrame[Sale]) -> int:
        return 0
    return inner
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    // v1.11 PR-D2 — walker polish: regression guards for the v1.10
    // architecture-audit edge cases (mixed tab/space indent + decorator
    // stack with intervening comment) and a documentation-by-test of
    // the deliberate raw-char-count `leading_indent` semantics. No
    // walker logic changes: the suite pins current behavior so the next
    // refactor can't silently shift the edges.

    #[test]
    fn v111_prd2_mixed_indent_tab_def_marker_above_acknowledges() {
        // Tab-indented inner def with a tab-indented marker on the line
        // immediately above it. The annotation is on the def signature
        // line itself, so `find_enclosing_def_line_start` takes the
        // trivially-enclosing branch (line 290) without needing the
        // indent comparison. Result: acknowledged.
        let src = "\
def outer():
\tdef inner_a():
\t\tpass
\t# pykrete: ack-deprecation
\tdef tab_indented(y: DataFrame[Y]): pass
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1, "sites: {sites:?}");
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v111_prd2_mixed_indent_space_marker_above_tab_def_acknowledges() {
        // Tab-indented inner def with a SPACE-indented marker on the
        // line immediately above. The marker text `.trim()`s to the
        // exact marker string regardless of leading-whitespace flavor,
        // so the marker check still matches and the site is
        // acknowledged. Documents the deliberate indent-agnostic
        // matching: pykrete pins on text content, not whitespace
        // shape. The contract is "what does the line above the anchor
        // (post-decorator-walk) say after trimming"; with no
        // decorators here the anchor is the def line itself.
        let src = "\
def outer():
\tdef inner_a():
\t\tpass
    # pykrete: ack-deprecation
\tdef tab_indented(y: DataFrame[Y]): pass
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1, "sites: {sites:?}");
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v111_prd2_comment_between_decorators_break_at_comment_documented() {
        // Decorator stack with an unrelated `# some unrelated comment`
        // between `@outer_decorator` and `@inner_decorator`. The
        // walker's `walk_past_decorators` stops at the first non-`@`
        // line above the def — the comment — and anchors at
        // `@inner_decorator`'s line. The marker is two lines above the
        // outer decorator, which is three lines above the anchor's
        // "line above," so the walker reads the marker as NOT directly
        // above the anchor and reports pending. Documents the
        // deliberate carve-out: walk_past_decorators is contiguous-
        // decorator-lines-only, not "any-non-def line."
        let src = "\
# pykrete: ack-deprecation
@outer_decorator
# some unrelated comment
@inner_decorator
def f(x: DataFrame[X]): pass
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1, "sites: {sites:?}");
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    // v1.12 PR-V1 — multi-line ack-marker shape (b) rationale block.
    // Per spec §6.1.2: the marker may sit on any line of the contiguous
    // `#`-prefixed comment block directly above the anchor (def, post-
    // decorator-walk; or annotation line for module-level). The walk
    // stops at the first non-comment line (blank line, code line,
    // decorator — anything not starting with `#` after trim).

    #[test]
    fn v112_prv1_marker_then_rationale_acknowledges() {
        // 2-line block: marker on top, rationale below.
        let src = "\
# pykrete: ack-deprecation
# rationale: legacy code, will migrate in Q3
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v112_prv1_rationale_then_marker_acknowledges() {
        // 2-line block: rationale on top, marker below.
        let src = "\
# rationale: legacy code, will migrate in Q3
# pykrete: ack-deprecation
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v112_prv1_marker_middle_of_block_acknowledges() {
        // 3-line block: marker on the middle line.
        let src = "\
# Tracking issue: https://github.com/team/project/issues/123
# pykrete: ack-deprecation
# Owner: @amir
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v112_prv1_three_line_block_with_no_marker_is_pending() {
        // 3-line block: no marker anywhere → pending.
        let src = "\
# Tracking issue: https://github.com/team/project/issues/123
# Owner: @amir
# Migration scheduled: 2026-Q4
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    #[test]
    fn v112_prv1_blank_line_breaks_contiguity_pending() {
        // Marker, then blank line, then annotation → blank line
        // breaks the contiguous comment-walk; pending.
        let src = "\
# pykrete: ack-deprecation

x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    #[test]
    fn v112_prv1_non_comment_breaks_contiguity_pending() {
        // Marker, then bare statement (non-comment, non-decorator),
        // then annotation → walker stops at the bare statement;
        // pending.
        let src = "\
# pykrete: ack-deprecation
y = 1
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Pending);
    }

    #[test]
    fn v112_prv1_empty_comment_preserves_contiguity() {
        // An "empty" comment line (`#` alone, or `#` + whitespace)
        // is still `#`-prefixed after `trim_start`, so the walker
        // treats it as part of the contiguous block. Documents the
        // choice: an empty comment line does NOT break contiguity.
        let src = "\
# pykrete: ack-deprecation
#
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v112_prv1_no_rationale_marker_only_acknowledges() {
        // Regression guard: shape (a) single-line marker (no
        // rationale block) still acknowledges, both for module-level
        // and def-anchored sites.
        let src = "\
# pykrete: ack-deprecation
x: DataFrame[Sale] = read_sales()
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v112_prv1_multi_line_def_still_acknowledges() {
        // Regression guard for v1.10 PR-A1 shape (a) — multi-line def
        // signature with single-line marker still works unchanged.
        let src = "\
# pykrete: ack-deprecation
def f(
    a: DataFrame[Sale],
) -> int:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v112_prv1_rationale_block_above_multi_line_def_acknowledges() {
        // Shape (a) + shape (b) composed: rationale block above a
        // multi-line def signature. The walker first anchors at the
        // def line (post-decorator-walk; here no decorators), then
        // walks UP through the contiguous comment block looking for
        // the marker.
        let src = "\
# rationale: legacy code, will migrate in Q3
# pykrete: ack-deprecation
def f(
    a: DataFrame[Sale],
) -> int:
    ...
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v112_prv1_rationale_block_above_decorator_acknowledges() {
        // Decorator + shape (b): walker anchors at the decorator
        // stack (post-decorator-walk), then walks UP through the
        // contiguous comment block above the topmost decorator.
        let src = "\
# rationale: legacy code, will migrate in Q3
# pykrete: ack-deprecation
@decorator
def f(df: DataFrame[Sale]) -> int:
    return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v112_prv1_decorator_between_marker_and_def_acknowledges() {
        // Marker above a decorator that sits above the def.
        // `walk_past_decorators` consumes the `@` line, anchoring
        // the comment-walk at the decorator's line. The walker then
        // walks UP and finds the marker immediately above —
        // acknowledged. (This is the shape (b) extension of the
        // v1.10 walk-past-decorator behavior.)
        let src = "\
# pykrete: ack-deprecation
@decorator
def f(df: DataFrame[Sale]) -> int:
    return 0
";
        let sites = collect(src);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].migration_status, MigrationStatus::Acknowledged);
    }

    #[test]
    fn v111_prd2_leading_indent_tab_vs_space_raw_char_count() {
        // `leading_indent` counts raw whitespace bytes — tab = 1,
        // 4 spaces = 4. The walker's enclosing-def check compares this
        // count for relative ordering ("annotation deeper than def"),
        // which is internally consistent for tab-only or space-only
        // files. Mixed-indent files where a tab-indented def sits at
        // 1-char indent and a space-indented annotation sits at
        // 4-char indent will report different indents (1 vs 4), even
        // though both visually nest one level deep. This is a
        // deliberate choice per v1.10 architecture-audit finding #9:
        // tab-width is editor-configured (2/4/8 are all common), so
        // raw-char counting is content-agnostic.
        assert_eq!(leading_indent("\tdef x", 0), 1);
        assert_eq!(leading_indent("    def x", 0), 4);
        assert_eq!(leading_indent("\t\tdef x", 0), 2);
        assert_eq!(leading_indent("def x", 0), 0);
    }
}
