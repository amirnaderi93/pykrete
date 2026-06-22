//! v1.14 PR-D3 — `--compare-to <snapshot>` SIMPLE diff-only output.
//!
//! Implements the diff document shape pinned at v1.14-spec.md §1.i.1
//! (forward-binding amendment, user signoff 2026-06-22 Option A). The
//! consumer surface that closes the 5-cycle defer cascade running from
//! v1.10 (carve-out §5.5) through v1.13 (§12 calendared user-decision).
//!
//! Shape:
//!
//! ```json
//! {
//!   "snapshot_a": {"sha": <git-sha|null>, "timestamp": <iso8601|null>},
//!   "snapshot_b": {"sha": <git-sha|null>, "timestamp": <iso8601|null>},
//!   "diff": {
//!     "added":     [<full site payload>],
//!     "removed":   [<full site payload>],
//!     "unchanged": [<full site payload>]
//!   }
//! }
//! ```
//!
//! Keying = `(file, line)` per v1.13-spec §12 Option A. The binary
//! `MigrationStatus` (`Pending` / `Acknowledged`) surfaces transitions
//! as remove-from-old + add-to-new — no separate `status_changed`
//! bucket (spec §1.i.1).
//!
//! `snapshot_a` provenance (`sha` / `timestamp`) is extracted from the
//! prior snapshot file's top-level keys when present (`pykreteSourceCommit`
//! and `generatedAt`); absent → `null`. The current deprecation-report
//! envelope does not emit those keys, so a snapshot taken pre-v1.14
//! produces `null` provenance — the diff still computes correctly.
//! `snapshot_b` provenance is supplied by the caller (the CLI captures
//! live git SHA + system clock).

use std::collections::BTreeMap;

/// Provenance pair for one side of the diff. Both fields are optional —
/// `None` when the source snapshot doesn't carry the value (legacy
/// envelopes) or when the current-run capture failed (no git available,
/// detached HEAD that `git rev-parse` could still answer for, etc.).
#[derive(Debug, Clone, Default)]
pub struct SnapshotProvenance {
    pub sha: Option<String>,
    pub timestamp: Option<String>,
}

/// Parsed view of a prior snapshot file. Holds the per-site JSON
/// payloads keyed by `(file, line)` so the diff can lift them verbatim
/// into the output buckets (self-contained snapshot semantics per spec
/// §1.i.1 — every bucket carries the full site payload, not just
/// `SiteId`).
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub provenance: SnapshotProvenance,
    /// Per-site payload in source order. Diff dedup happens against
    /// `(file, line)` keys; preserving the original Vec lets the
    /// `removed` bucket emit in the prior snapshot's order rather than
    /// alphabetical-by-key.
    pub sites: Vec<SnapshotSite>,
}

#[derive(Debug, Clone)]
pub struct SnapshotSite {
    pub file: String,
    pub line: u64,
    /// Full per-site object as it appeared in the snapshot's `sites`
    /// array. The diff emits this verbatim into `unchanged` / `removed`
    /// without restructuring (and into `added` from the current run).
    pub payload: serde_json::Value,
}

/// Parse a `--deprecation-report` v2 snapshot file (string contents
/// already read off disk). Returns a structured `Snapshot` ready for
/// diffing.
///
/// Errors surface clear, single-line messages the CLI can pass to
/// stderr verbatim. Recognized failure modes:
/// - Top-level JSON parse error.
/// - Top-level value is not an object.
/// - `sites` missing, or present but not an array.
/// - A site entry is not an object, missing `file`, or missing/non-integer
///   `line`.
///
/// Top-level provenance keys (`pykreteSourceCommit`, `generatedAt`) are
/// optional. Non-string values for either field are tolerated as
/// `None` rather than failing the whole parse — the diff still
/// computes; the consumer just loses the provenance side-channel.
pub fn parse_snapshot(content: &str) -> Result<Snapshot, String> {
    let root: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("snapshot is not valid JSON: {e}"))?;
    let obj = root
        .as_object()
        .ok_or_else(|| "snapshot top-level value is not a JSON object".to_string())?;

    let provenance = SnapshotProvenance {
        sha: obj
            .get("pykreteSourceCommit")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        timestamp: obj
            .get("generatedAt")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    };

    let sites_val = obj
        .get("sites")
        .ok_or_else(|| "snapshot missing required field 'sites'".to_string())?;
    let sites_arr = sites_val
        .as_array()
        .ok_or_else(|| "snapshot field 'sites' is not an array".to_string())?;

    let mut sites = Vec::with_capacity(sites_arr.len());
    for (idx, raw) in sites_arr.iter().enumerate() {
        let entry = raw
            .as_object()
            .ok_or_else(|| format!("snapshot site #{idx} is not a JSON object"))?;
        let file = entry
            .get("file")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("snapshot site #{idx} missing required string 'file'"))?
            .to_owned();
        let line = entry
            .get("line")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("snapshot site #{idx} missing required integer 'line'"))?;
        sites.push(SnapshotSite {
            file,
            line,
            payload: raw.clone(),
        });
    }
    Ok(Snapshot { provenance, sites })
}

/// Build a `Snapshot` view from an in-memory v2 envelope (already
/// rendered JSON string, typically the current run's
/// `render_deprecation_report_json` output). Convenience wrapper that
/// re-uses `parse_snapshot` so the current run shares the exact same
/// keying + payload-lift path as the prior snapshot — no duplicated
/// extraction logic.
///
/// Caller is expected to pass the canonical envelope text; if the
/// current run drifted from `render_deprecation_report_json`'s shape
/// this would surface as the same parse errors as a malformed file.
pub fn snapshot_from_current_envelope(envelope_json: &str) -> Result<Snapshot, String> {
    parse_snapshot(envelope_json)
}

/// Compute the three-bucket diff document for `snapshot_a` (prior) and
/// `snapshot_b` (current). `provenance_b` is supplied separately
/// because the current-run envelope doesn't carry git-sha / timestamp
/// fields; the CLI captures them live and threads them in here.
///
/// `(file, line)` is the dedup key per v1.13-spec §12 Option A.
/// Self-contained semantics: every bucket carries the full site
/// payload. `unchanged` uses the CURRENT run's payload (so a
/// `MigrationStatus` flip surfaces in the right bucket via the
/// remove/add pattern — never in `unchanged`).
pub fn render_diff(
    snapshot_a: &Snapshot,
    snapshot_b: &Snapshot,
    provenance_b: &SnapshotProvenance,
) -> String {
    let mut by_key_a: BTreeMap<(String, u64), &SnapshotSite> = BTreeMap::new();
    for site in &snapshot_a.sites {
        by_key_a.insert((site.file.clone(), site.line), site);
    }
    let mut by_key_b: BTreeMap<(String, u64), &SnapshotSite> = BTreeMap::new();
    for site in &snapshot_b.sites {
        by_key_b.insert((site.file.clone(), site.line), site);
    }

    let mut added: Vec<serde_json::Value> = Vec::new();
    let mut unchanged: Vec<serde_json::Value> = Vec::new();
    for site in &snapshot_b.sites {
        let key = (site.file.clone(), site.line);
        if let Some(prior) = by_key_a.get(&key) {
            // Same SiteId in both → check payload equality. A
            // MigrationStatus flip (or any other field change) at the
            // same (file, line) re-emits the site as remove-from-old +
            // add-to-new, mirroring the spec's "binary status surfaces
            // transitions via remove/add" rule.
            if prior.payload == site.payload {
                unchanged.push(site.payload.clone());
            } else {
                added.push(site.payload.clone());
            }
        } else {
            added.push(site.payload.clone());
        }
    }

    let mut removed: Vec<serde_json::Value> = Vec::new();
    for site in &snapshot_a.sites {
        let key = (site.file.clone(), site.line);
        match by_key_b.get(&key) {
            Some(current) if current.payload == site.payload => {
                // Already counted in `unchanged`.
            }
            Some(_) => {
                // Payload changed → the OLD payload belongs in
                // `removed`; the new one is already in `added`.
                removed.push(site.payload.clone());
            }
            None => removed.push(site.payload.clone()),
        }
    }

    let payload = serde_json::json!({
        "snapshot_a": provenance_value(&snapshot_a.provenance),
        "snapshot_b": provenance_value(provenance_b),
        "diff": {
            "added": added,
            "removed": removed,
            "unchanged": unchanged,
        }
    });
    serde_json::to_string_pretty(&payload)
        .expect("diff JSON is composed of types that always serialize")
}

fn provenance_value(p: &SnapshotProvenance) -> serde_json::Value {
    serde_json::json!({
        "sha": p.sha.as_deref().map_or(serde_json::Value::Null, |s| serde_json::Value::String(s.to_owned())),
        "timestamp": p.timestamp.as_deref().map_or(serde_json::Value::Null, |s| serde_json::Value::String(s.to_owned())),
    })
}

/// Count of sites in the `added` bucket — the CLI uses this to decide
/// exit code (nonzero when `added > 0` per spec §1.i.1, "regression
/// signal — new sites have appeared since the prior snapshot"). Plain
/// `removed` and `unchanged` are healthy steady-state / migration
/// progress and stay exit 0.
pub fn added_count(snapshot_a: &Snapshot, snapshot_b: &Snapshot) -> usize {
    let mut by_key_a: BTreeMap<(&str, u64), &SnapshotSite> = BTreeMap::new();
    for site in &snapshot_a.sites {
        by_key_a.insert((site.file.as_str(), site.line), site);
    }
    let mut count = 0usize;
    for site in &snapshot_b.sites {
        let key = (site.file.as_str(), site.line);
        match by_key_a.get(&key) {
            Some(prior) if prior.payload == site.payload => {}
            _ => count += 1,
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(file: &str, line: u64, status: &str) -> serde_json::Value {
        serde_json::json!({
            "file": file,
            "line": line,
            "column": 1,
            "code": "D0090",
            "ruleName": "deprecatedDataFrameAlias",
            "migrationStatus": status,
        })
    }

    fn envelope(sites: Vec<serde_json::Value>) -> String {
        let payload = serde_json::json!({
            "deprecationReportVersion": "2",
            "sites": sites,
            "summary": {"totalSites": 0, "byDialect": {"spark": 0, "pandas": 0, "ambiguous": 0}}
        });
        serde_json::to_string_pretty(&payload).unwrap()
    }

    #[test]
    fn parse_empty_envelope_yields_no_sites() {
        let snap = parse_snapshot(&envelope(vec![])).unwrap();
        assert!(snap.sites.is_empty());
        assert!(snap.provenance.sha.is_none());
        assert!(snap.provenance.timestamp.is_none());
    }

    #[test]
    fn parse_provenance_when_present() {
        let src = serde_json::json!({
            "deprecationReportVersion": "2",
            "pykreteSourceCommit": "abc123",
            "generatedAt": "2026-06-22T00:00:00Z",
            "sites": [],
            "summary": {"totalSites": 0, "byDialect": {"spark": 0, "pandas": 0, "ambiguous": 0}}
        })
        .to_string();
        let snap = parse_snapshot(&src).unwrap();
        assert_eq!(snap.provenance.sha.as_deref(), Some("abc123"));
        assert_eq!(
            snap.provenance.timestamp.as_deref(),
            Some("2026-06-22T00:00:00Z")
        );
    }

    #[test]
    fn parse_malformed_json_errors_cleanly() {
        let err = parse_snapshot("{not json").unwrap_err();
        assert!(err.contains("not valid JSON"), "msg: {err}");
    }

    #[test]
    fn parse_non_object_top_level_errors_cleanly() {
        let err = parse_snapshot("[1, 2, 3]").unwrap_err();
        assert!(err.contains("not a JSON object"), "msg: {err}");
    }

    #[test]
    fn parse_missing_sites_errors_cleanly() {
        let err = parse_snapshot(r#"{"deprecationReportVersion":"2"}"#).unwrap_err();
        assert!(err.contains("missing required field 'sites'"), "msg: {err}");
    }

    #[test]
    fn parse_sites_not_array_errors_cleanly() {
        let err = parse_snapshot(r#"{"deprecationReportVersion":"2","sites":42}"#).unwrap_err();
        assert!(err.contains("not an array"), "msg: {err}");
    }

    #[test]
    fn parse_site_missing_file_errors_cleanly() {
        let src = envelope(vec![serde_json::json!({"line": 1})]);
        let err = parse_snapshot(&src).unwrap_err();
        assert!(err.contains("missing required string 'file'"), "msg: {err}");
    }

    #[test]
    fn parse_site_missing_line_errors_cleanly() {
        let src = envelope(vec![serde_json::json!({"file": "a.pyk"})]);
        let err = parse_snapshot(&src).unwrap_err();
        assert!(
            err.contains("missing required integer 'line'"),
            "msg: {err}"
        );
    }

    #[test]
    fn diff_identical_snapshots_all_unchanged() {
        let a = parse_snapshot(&envelope(vec![site("a.pyk", 5, "pending")])).unwrap();
        let b = parse_snapshot(&envelope(vec![site("a.pyk", 5, "pending")])).unwrap();
        let out = render_diff(&a, &b, &SnapshotProvenance::default());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 0);
        assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 0);
        assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn diff_empty_a_full_b_all_added() {
        let a = parse_snapshot(&envelope(vec![])).unwrap();
        let b = parse_snapshot(&envelope(vec![
            site("a.pyk", 5, "pending"),
            site("b.pyk", 10, "pending"),
        ]))
        .unwrap();
        let out = render_diff(&a, &b, &SnapshotProvenance::default());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 2);
        assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 0);
        assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn diff_full_a_empty_b_all_removed() {
        let a = parse_snapshot(&envelope(vec![
            site("a.pyk", 5, "pending"),
            site("b.pyk", 10, "pending"),
        ]))
        .unwrap();
        let b = parse_snapshot(&envelope(vec![])).unwrap();
        let out = render_diff(&a, &b, &SnapshotProvenance::default());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 0);
        assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 2);
        assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn diff_status_flip_surfaces_as_remove_plus_add() {
        let a = parse_snapshot(&envelope(vec![site("a.pyk", 5, "pending")])).unwrap();
        let b = parse_snapshot(&envelope(vec![site("a.pyk", 5, "acknowledged")])).unwrap();
        let out = render_diff(&a, &b, &SnapshotProvenance::default());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 1);
        assert_eq!(
            v["diff"]["added"][0]["migrationStatus"], "acknowledged",
            "added carries the NEW payload"
        );
        assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 1);
        assert_eq!(
            v["diff"]["removed"][0]["migrationStatus"], "pending",
            "removed carries the OLD payload"
        );
        assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn diff_mixed_buckets() {
        let a = parse_snapshot(&envelope(vec![
            site("a.pyk", 5, "pending"),
            site("b.pyk", 10, "pending"),
        ]))
        .unwrap();
        let b = parse_snapshot(&envelope(vec![
            site("a.pyk", 5, "pending"),
            site("c.pyk", 7, "pending"),
        ]))
        .unwrap();
        let out = render_diff(&a, &b, &SnapshotProvenance::default());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["diff"]["added"].as_array().unwrap().len(), 1);
        assert_eq!(v["diff"]["added"][0]["file"], "c.pyk");
        assert_eq!(v["diff"]["removed"].as_array().unwrap().len(), 1);
        assert_eq!(v["diff"]["removed"][0]["file"], "b.pyk");
        assert_eq!(v["diff"]["unchanged"].as_array().unwrap().len(), 1);
        assert_eq!(v["diff"]["unchanged"][0]["file"], "a.pyk");
    }

    #[test]
    fn diff_envelope_carries_provenance_for_both_sides() {
        let a_src = serde_json::json!({
            "deprecationReportVersion": "2",
            "pykreteSourceCommit": "old-sha",
            "generatedAt": "2026-06-01T00:00:00Z",
            "sites": [],
            "summary": {"totalSites": 0, "byDialect": {"spark": 0, "pandas": 0, "ambiguous": 0}}
        })
        .to_string();
        let a = parse_snapshot(&a_src).unwrap();
        let b = parse_snapshot(&envelope(vec![])).unwrap();
        let prov_b = SnapshotProvenance {
            sha: Some("new-sha".into()),
            timestamp: Some("2026-06-22T12:00:00Z".into()),
        };
        let out = render_diff(&a, &b, &prov_b);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["snapshot_a"]["sha"], "old-sha");
        assert_eq!(v["snapshot_a"]["timestamp"], "2026-06-01T00:00:00Z");
        assert_eq!(v["snapshot_b"]["sha"], "new-sha");
        assert_eq!(v["snapshot_b"]["timestamp"], "2026-06-22T12:00:00Z");
    }

    #[test]
    fn diff_envelope_emits_null_provenance_when_missing() {
        let a = parse_snapshot(&envelope(vec![])).unwrap();
        let b = parse_snapshot(&envelope(vec![])).unwrap();
        let out = render_diff(&a, &b, &SnapshotProvenance::default());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["snapshot_a"]["sha"].is_null());
        assert!(v["snapshot_a"]["timestamp"].is_null());
        assert!(v["snapshot_b"]["sha"].is_null());
        assert!(v["snapshot_b"]["timestamp"].is_null());
    }

    #[test]
    fn added_count_matches_diff_bucket() {
        let a = parse_snapshot(&envelope(vec![site("a.pyk", 5, "pending")])).unwrap();
        let b = parse_snapshot(&envelope(vec![
            site("a.pyk", 5, "pending"),
            site("b.pyk", 10, "pending"),
        ]))
        .unwrap();
        assert_eq!(added_count(&a, &b), 1);
    }

    #[test]
    fn added_count_counts_payload_flips() {
        // (file, line) same but payload differs → counts as an add for
        // exit-code purposes, mirroring `render_diff`'s remove + add
        // surfacing of MigrationStatus flips.
        let a = parse_snapshot(&envelope(vec![site("a.pyk", 5, "pending")])).unwrap();
        let b = parse_snapshot(&envelope(vec![site("a.pyk", 5, "acknowledged")])).unwrap();
        assert_eq!(added_count(&a, &b), 1);
    }

    #[test]
    fn added_count_zero_when_only_removals() {
        let a = parse_snapshot(&envelope(vec![
            site("a.pyk", 5, "pending"),
            site("b.pyk", 10, "pending"),
        ]))
        .unwrap();
        let b = parse_snapshot(&envelope(vec![site("a.pyk", 5, "pending")])).unwrap();
        assert_eq!(added_count(&a, &b), 0);
    }

    #[test]
    fn snapshot_from_current_envelope_round_trips() {
        let env_text = envelope(vec![site("a.pyk", 1, "pending")]);
        let snap = snapshot_from_current_envelope(&env_text).unwrap();
        assert_eq!(snap.sites.len(), 1);
        assert_eq!(snap.sites[0].file, "a.pyk");
        assert_eq!(snap.sites[0].line, 1);
    }
}
