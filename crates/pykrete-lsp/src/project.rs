//! Project-snapshot helper for cross-file LSP analysis.
//!
//! Each LSP request that needs cross-file context (today: diagnostics,
//! hover, definition, completion) calls into [`SnapshotCache`] to get a
//! list of every `.pyk` file pykrete should see — every sibling file
//! under the project root, with the open editor's in-memory contents
//! overriding what's on disk for any URI the client has told us about.
//!
//! The project root is the deepest `pyproject.toml`-bearing directory
//! above the first reachable open document, falling back to that
//! document's parent dir when no `pyproject.toml` is found. This is the
//! same rule [`pykrete::check_project`] uses for resolving absolute
//! imports.
//!
//! Caching strategy — three tiers around a single composite key
//! `(project_root, pykrete_json_fingerprint)`. The project root is
//! derived from the lexicographically-smallest open doc path (sorted so
//! the choice is stable across `HashMap` iteration orders), then the
//! anchor itself is dropped from the key — two different anchors that
//! resolve to the same project root produce identical cache state, so
//! including the anchor would only invalidate spuriously on every
//! `didOpen` / `didClose` with 2+ open files.
//!
//! - HOT — rebuilt from `docs` on every call. The user's editor buffers
//!   never go stale; this is what makes hover-during-typing feel live.
//! - WARM — closed `.pyk` files whose on-disk mtime is in the last
//!   5 minutes. Stat-only sweep gated at most once per second; on stat
//!   diff the file is re-read and the patched entry replaces the old
//!   one. Catches scratchpad / git-checkout activity.
//! - COLD — everything else. Full recursive `read_dir` walk only on
//!   first request, ~every 30 s, or when the project key shifts (open
//!   outside the tracked union, watcher event).
//!
//! Bodies in WARM / COLD are held as `Arc<String>` so the per-request
//! `Vec<(String, String)>` only allocates the outer Vec and clones the
//! body string at the API boundary (downstream callers expect owned
//! `String`s).
//!
//! Hard memory cap (~20 MB): COLD does a two-pass walk. Pass 1
//! enumerates every `.pyk` path under the root with no I/O beyond
//! `read_dir`. Pass 2 reads bodies into Arcs until the cumulative size
//! crosses the cap, then stops reading — but the remaining paths still
//! land in the tracked union with `body = None`. Snapshot assembly does
//! an on-demand `read_to_string` for any `None` entry. This keeps the
//! tracked union complete on big codebases, so `didOpen` on an uncached
//! path is not "outside the union" and does not thrash the cold walk.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use lsp_types::Url;

use pykrete::imports::find_pyproject_root;

/// How often the WARM tier may run its stat sweep.
const WARM_SWEEP_INTERVAL: Duration = Duration::from_secs(1);
/// How often the COLD tier may rewalk the project root.
const COLD_WALK_INTERVAL: Duration = Duration::from_secs(30);
/// An on-disk file with mtime within this window is considered "warm" —
/// stat-checked every WARM_SWEEP_INTERVAL until it cools off.
const WARM_AGE: Duration = Duration::from_secs(5 * 60);
/// Hard ceiling on cached body bytes. The cold walk runs in two
/// passes: pass 1 enumerates every `.pyk` path with no body I/O, pass
/// 2 reads bodies into `Arc`s until the cumulative size crosses this
/// cap, then stops reading. Remaining entries still land in the
/// tracked union with `body = None`; snapshot assembly re-reads those
/// from disk on demand.
const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;

/// Fingerprint for the project root + pykrete.json. Any drift in this
/// tuple invalidates all three tiers atomically. The anchor doc itself
/// is intentionally NOT part of the key — two different anchors under
/// the same project root produce identical cache state, so including
/// the anchor would spuriously invalidate on every `didOpen` /
/// `didClose` once 2+ files are open.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProjectKey {
    project_root: PathBuf,
    /// `(path, mtime)` of the closest `pykrete.json`, if any. `None`
    /// when no config file is present.
    pykrete_json: Option<(PathBuf, SystemTime)>,
}

/// One cached on-disk `.pyk` file. `body` is `None` when the cold walk
/// hit the memory cap before reading this file — snapshot assembly
/// re-reads it from disk on demand. `mtime` is the file's mtime when
/// the entry was created (or `UNIX_EPOCH` for uncached entries, which
/// also keeps them out of the warm sweep).
#[derive(Clone, Debug)]
struct CacheEntry {
    body: Option<Arc<String>>,
    mtime: SystemTime,
}

/// Tiered hot/warm/cold snapshot cache, one instance per LSP session.
#[derive(Debug)]
pub struct SnapshotCache {
    /// `(key, entries)` — both populated together; cleared together on
    /// key drift. Closed-on-disk files only; open docs are overlaid live
    /// at snapshot time.
    state: Option<CachedState>,
    /// Last time the COLD walk ran.
    last_cold_walk: Option<Instant>,
    /// Last time the WARM stat sweep ran.
    last_warm_sweep: Option<Instant>,
    /// Hard cap on cached body bytes during the cold walk. Lifted to a
    /// field so tests can construct a cache with a tiny cap and pin the
    /// two-pass-over-cap behavior without writing 20 MB of fixtures.
    max_body_bytes: usize,
    /// Test-only counter: how many full cold walks have run. Used by
    /// the typing-burst regression to pin the rate-limit guarantee.
    #[cfg(test)]
    cold_walk_count: usize,
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self {
            state: None,
            last_cold_walk: None,
            last_warm_sweep: None,
            max_body_bytes: MAX_BODY_BYTES,
            #[cfg(test)]
            cold_walk_count: 0,
        }
    }
}

#[derive(Debug)]
struct CachedState {
    key: ProjectKey,
    /// Every `.pyk` file the project root walk found, last we looked.
    /// Entries whose `body` is `None` were past the memory cap during
    /// pass 2 — their paths are still tracked (so `didOpen` doesn't
    /// trigger an outside-union invalidate) but the body is re-read on
    /// every snapshot.
    entries: HashMap<PathBuf, CacheEntry>,
    /// Cached `pykrete.json` contents, if any.
    config: pykrete::Config,
    /// Set when this cache build saw a malformed `pykrete.json`. The LSP
    /// layer drains it via [`SnapshotCache::take_pending_warning`] and
    /// pushes one `window/showMessage` + one `window/logMessage` so the
    /// user knows defaults are being used. Cleared after the first drain
    /// within a given cache build — but every cold walk (one per
    /// `COLD_WALK_INTERVAL`, 30 s today) re-populates it unconditionally
    /// when the file is still malformed, so the warning re-fires roughly
    /// every 30 s rather than once-per-mtime. A v1.1 follow-up (item 15
    /// in `docs/design/spark-coverage.md`) will gate re-emission on
    /// mtime drift.
    malformed_warning: Option<MalformedConfig>,
}

/// A `pykrete.json` that couldn't be parsed during the last cold walk.
#[derive(Debug, Clone)]
pub struct MalformedConfig {
    pub path: PathBuf,
    pub error: String,
}

impl SnapshotCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when `path` is already in the cached cold list (so its
    /// project root almost certainly matches what the cache is keyed
    /// on). Callers use this to skip a needless invalidate on didOpen /
    /// didClose for paths already known to the cache.
    pub fn tracks_path(&self, path: &Path) -> bool {
        self.state
            .as_ref()
            .map(|s| s.entries.contains_key(path))
            .unwrap_or(false)
    }

    /// Drop every tier. Wired to the `pykrete/refreshSnapshot` custom
    /// LSP command and to file-watcher events whose path falls outside
    /// the tracked union.
    pub fn invalidate(&mut self) {
        self.state = None;
        self.last_cold_walk = None;
        self.last_warm_sweep = None;
    }

    /// Compute the project snapshot for the current `docs`. Returns
    /// `None` when no open document has a usable filesystem path — the
    /// caller falls back to single-file analysis in that case.
    pub fn snapshot(&mut self, docs: &HashMap<Url, String>) -> Option<Vec<(String, String)>> {
        let key = derive_project_key(docs)?;
        self.refresh(&key);

        let open_by_path: HashMap<PathBuf, &String> = docs
            .iter()
            .filter_map(|(uri, src)| uri.to_file_path().ok().map(|p| (p, src)))
            .collect();

        let state = self.state.as_ref().expect("refresh just populated state");

        // Compose: cold/warm entries (closed files) + hot (open files).
        // Open docs win on path collisions.
        let mut paths: Vec<PathBuf> = state.entries.keys().cloned().collect();
        for path in open_by_path.keys() {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        paths.sort();
        paths.dedup();

        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            let source = if let Some(in_memory) = open_by_path.get(&path) {
                (*in_memory).clone()
            } else if let Some(entry) = state.entries.get(&path) {
                match &entry.body {
                    Some(body) => (**body).clone(),
                    // Past the cap during cold walk — re-read on demand.
                    None => match fs::read_to_string(&path) {
                        Ok(s) => s,
                        Err(_) => continue,
                    },
                }
            } else {
                continue;
            };
            out.push((path.to_string_lossy().into_owned(), source));
        }
        Some(out)
    }

    /// Load the project's `pykrete.json` (cached behind the same key as
    /// the snapshot). Absent or malformed → defaults.
    pub fn config(&mut self, docs: &HashMap<Url, String>) -> pykrete::Config {
        let Some(key) = derive_project_key(docs) else {
            return pykrete::Config::default();
        };
        self.refresh(&key);
        self.state
            .as_ref()
            .map(|s| s.config.clone())
            .unwrap_or_default()
    }

    /// Drain any "this `pykrete.json` was malformed, defaults are in
    /// effect" notification that the most recent cold walk produced.
    /// Surfacing it is the LSP layer's job (`window/showMessage` +
    /// `window/logMessage`). Returns `None` when there's nothing pending,
    /// either because the file is well-formed or because the warning was
    /// already drained for this cache key.
    pub fn take_pending_warning(&mut self) -> Option<MalformedConfig> {
        self.state.as_mut().and_then(|s| s.malformed_warning.take())
    }

    /// Recompute the cold list if needed and run the warm sweep if the
    /// gate has elapsed. After this returns, `self.state` is populated
    /// and consistent with `key`.
    fn refresh(&mut self, key: &ProjectKey) {
        let now = Instant::now();

        // Project-root drift → drop everything, refetch from scratch.
        if self.state.as_ref().map(|s| &s.key) != Some(key) {
            self.invalidate();
        }

        // COLD: first time, or interval elapsed.
        let need_cold_walk = self.state.is_none()
            || self
                .last_cold_walk
                .map(|t| now.duration_since(t) >= COLD_WALK_INTERVAL)
                .unwrap_or(true);
        if need_cold_walk {
            self.run_cold_walk(key);
            self.last_cold_walk = Some(now);
            self.last_warm_sweep = Some(now);
            return;
        }

        // WARM: rate-limited stat sweep over recent-mtime files.
        let need_warm_sweep = self
            .last_warm_sweep
            .map(|t| now.duration_since(t) >= WARM_SWEEP_INTERVAL)
            .unwrap_or(true);
        if need_warm_sweep {
            self.run_warm_sweep();
            self.last_warm_sweep = Some(now);
        }
    }

    fn run_cold_walk(&mut self, key: &ProjectKey) {
        #[cfg(test)]
        {
            self.cold_walk_count += 1;
        }
        // Pass 1: enumerate every `.pyk` path under the project root.
        // No body reads, no sizing — just the path list, so memory is
        // bounded by the count of `.pyk` files even on huge codebases.
        let mut paths: Vec<PathBuf> = Vec::new();
        if collect_pyk_paths(&key.project_root, &mut paths).is_err() {
            paths.clear();
        }
        paths.sort();
        paths.dedup();

        // Pass 2: read bodies into Arcs, accumulating size. Reuse the
        // previous entry where we can (path + mtime match) so the Arc
        // bodies stay shared. Once we cross `MAX_BODY_BYTES`, stop
        // reading — but keep inserting `body = None` entries so the
        // tracked union still contains every `.pyk` path. That avoids
        // an invalidate-loop on `didOpen` for files past the cap.
        let previous = self.state.take().map(|s| s.entries).unwrap_or_default();
        let mut entries: HashMap<PathBuf, CacheEntry> = HashMap::with_capacity(paths.len());
        let mut total_bytes: usize = 0;
        let mut over_cap = false;
        for path in paths {
            if over_cap {
                entries.insert(
                    path,
                    CacheEntry {
                        body: None,
                        mtime: SystemTime::UNIX_EPOCH,
                    },
                );
                continue;
            }
            let mtime = mtime_of(&path).unwrap_or(SystemTime::UNIX_EPOCH);
            if let Some(prev) = previous.get(&path)
                && prev.mtime == mtime
                && let Some(body) = prev.body.as_ref()
            {
                total_bytes = total_bytes.saturating_add(body.len());
                if total_bytes > self.max_body_bytes {
                    over_cap = true;
                    entries.insert(
                        path,
                        CacheEntry {
                            body: None,
                            mtime: SystemTime::UNIX_EPOCH,
                        },
                    );
                    continue;
                }
                entries.insert(path, prev.clone());
                continue;
            }
            let body = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            total_bytes = total_bytes.saturating_add(body.len());
            if total_bytes > self.max_body_bytes {
                over_cap = true;
                entries.insert(
                    path,
                    CacheEntry {
                        body: None,
                        mtime: SystemTime::UNIX_EPOCH,
                    },
                );
                continue;
            }
            entries.insert(
                path,
                CacheEntry {
                    body: Some(Arc::new(body)),
                    mtime,
                },
            );
        }

        let (config, malformed_warning) = load_pykrete_json(&key.project_root);
        self.state = Some(CachedState {
            key: key.clone(),
            entries,
            config,
            malformed_warning,
        });
    }

    fn run_warm_sweep(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let now = SystemTime::now();
        let mut to_refresh: Vec<PathBuf> = Vec::new();
        for (path, entry) in &state.entries {
            // Uncached entries (past the cold-walk cap) are excluded
            // from the warm sweep — their bytes are re-read on demand
            // in `snapshot`, so we don't want to warm them in.
            if entry.body.is_none() {
                continue;
            }
            // Only files that were recently touched on disk; older files
            // sit in the cold tier and get rechecked on the next walk.
            let age = now.duration_since(entry.mtime).unwrap_or(WARM_AGE);
            if age > WARM_AGE {
                continue;
            }
            let Some(current_mtime) = mtime_of(path) else {
                continue;
            };
            if current_mtime != entry.mtime {
                to_refresh.push(path.clone());
            }
        }
        for path in to_refresh {
            let Ok(body) = fs::read_to_string(&path) else {
                continue;
            };
            let mtime = mtime_of(&path).unwrap_or(SystemTime::UNIX_EPOCH);
            state.entries.insert(
                path,
                CacheEntry {
                    body: Some(Arc::new(body)),
                    mtime,
                },
            );
        }
    }
}

fn derive_project_key(docs: &HashMap<Url, String>) -> Option<ProjectKey> {
    // Sort the candidate anchor paths so the pick is stable across
    // `HashMap` iteration orders — otherwise `ProjectKey` would differ
    // between calls on the same project state and the cache would
    // invalidate on every didOpen / didClose with 2+ open files.
    let mut anchor_candidates: Vec<PathBuf> = docs
        .keys()
        .filter_map(|uri| uri.to_file_path().ok())
        .collect();
    anchor_candidates.sort();
    let anchor = anchor_candidates.into_iter().next()?;
    let project_root = find_pyproject_root(&anchor).unwrap_or_else(|| {
        anchor
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });
    let pykrete_json = find_pykrete_json(&project_root)
        .and_then(|path| mtime_of(&path).map(|mtime| (path, mtime)));
    Some(ProjectKey {
        project_root,
        pykrete_json,
    })
}

fn find_pykrete_json(project_root: &Path) -> Option<PathBuf> {
    let mut dir = project_root.to_path_buf();
    loop {
        let candidate = dir.join("pykrete.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Read the project's `pykrete.json` (if any). Returns the parsed
/// config plus, when parsing failed, a [`MalformedConfig`] the caller
/// can surface to the editor. A missing or unreadable file is silent —
/// only a present-but-malformed file produces a warning.
fn load_pykrete_json(project_root: &Path) -> (pykrete::Config, Option<MalformedConfig>) {
    let Some(path) = find_pykrete_json(project_root) else {
        return (pykrete::Config::default(), None);
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return (pykrete::Config::default(), None);
    };
    match pykrete::Config::parse(&content) {
        Ok(cfg) => (cfg, None),
        Err(err) => (
            pykrete::Config::default(),
            Some(MalformedConfig { path, error: err }),
        ),
    }
}

fn mtime_of(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

/// Recursively collect every `.pyk` file under `dir`. Mirrors the CLI's
/// `walk_pyk` so directory mode in the binary and project mode in the
/// LSP agree on what counts as "in the project."
fn collect_pyk_paths(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_pyk_paths(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("pyk") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::thread::sleep;

    fn tmpdir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "pykrete-lsp-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, content: &str) {
        let mut f = File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn docs_with(root: &Path) -> HashMap<Url, String> {
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        let uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs = HashMap::new();
        docs.insert(uri, "class A(Schema):\n    x: int\n".to_string());
        docs
    }

    #[test]
    fn snapshot_returns_none_when_no_open_doc_has_a_file_path() {
        let mut cache = SnapshotCache::new();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(
            Url::parse("untitled:Untitled-1").unwrap(),
            "class X(Schema):\n    a: int\n".to_string(),
        );
        assert!(cache.snapshot(&docs).is_none());
    }

    #[test]
    fn snapshot_includes_sibling_pyk_files_from_disk() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");

        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(
            a_uri.clone(),
            "class A(Schema):\n    x: int  # edited\n".into(),
        );

        let mut cache = SnapshotCache::new();
        let snapshot = cache.snapshot(&docs).expect("snapshot");
        let paths: Vec<&str> = snapshot.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.iter().any(|p| p.ends_with("a.pyk")));
        assert!(paths.iter().any(|p| p.ends_with("b.pyk")));

        let (_, a_source) = snapshot.iter().find(|(p, _)| p.ends_with("a.pyk")).unwrap();
        assert!(
            a_source.contains("edited"),
            "open doc should override disk contents, got {a_source:?}",
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn snapshot_walks_subdirectories() {
        let root = tmpdir();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(
            &root.join("sub").join("b.pyk"),
            "class B(Schema):\n    y: int\n",
        );

        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        let snapshot = cache.snapshot(&docs).expect("snapshot");
        assert!(snapshot.iter().any(|(p, _)| p.ends_with("a.pyk")));
        assert!(snapshot.iter().any(|(p, _)| p.ends_with("b.pyk")));

        std::fs::remove_dir_all(&root).ok();
    }

    /// Repeated `snapshot` calls reuse the same cached entries — the
    /// Arc-backed bodies must not be re-read from disk on every call.
    /// We pin the guarantee by comparing the Arc pointers for a closed
    /// file across two consecutive calls.
    #[test]
    fn snapshot_reuses_cold_arena_across_calls() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");

        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        cache.snapshot(&docs).expect("snapshot 1");
        let first_arc = cache
            .state
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|(p, _)| p.ends_with("b.pyk"))
            .and_then(|(_, e)| e.body.as_ref().map(Arc::clone))
            .unwrap();
        cache.snapshot(&docs).expect("snapshot 2");
        let second_arc = cache
            .state
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|(p, _)| p.ends_with("b.pyk"))
            .and_then(|(_, e)| e.body.as_ref().map(Arc::clone))
            .unwrap();
        assert!(
            Arc::ptr_eq(&first_arc, &second_arc),
            "expected the cached body Arc to be reused on the second snapshot",
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// Editing an open document — represented as a `docs` map mutation
    /// — does not invalidate the cold arena for closed files. The hot
    /// tier is just `docs`; only the entry for the changed URI is
    /// affected, and that entry is overlaid live at snapshot time.
    #[test]
    fn editing_open_doc_does_not_invalidate_other_cold_entries() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");

        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri.clone(), "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        cache.snapshot(&docs).expect("snapshot 1");
        let first_b_arc = cache
            .state
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|(p, _)| p.ends_with("b.pyk"))
            .and_then(|(_, e)| e.body.as_ref().map(Arc::clone))
            .unwrap();

        // Simulate didChange on a.pyk — just mutate the docs map.
        docs.insert(a_uri, "class A(Schema):\n    x: string\n".into());
        cache.snapshot(&docs).expect("snapshot 2");
        let second_b_arc = cache
            .state
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|(p, _)| p.ends_with("b.pyk"))
            .and_then(|(_, e)| e.body.as_ref().map(Arc::clone))
            .unwrap();
        assert!(
            Arc::ptr_eq(&first_b_arc, &second_b_arc),
            "didChange to a should not have re-read b",
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// `invalidate()` drops all tiers — the next snapshot re-runs the
    /// cold walk from scratch, allocating fresh Arcs.
    #[test]
    fn invalidate_drops_all_tiers() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        cache.snapshot(&docs).expect("snapshot 1");
        assert!(cache.state.is_some());

        cache.invalidate();
        assert!(cache.state.is_none());
        assert!(cache.last_cold_walk.is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    /// Touching a closed file on disk and forcing a warm sweep refreshes
    /// just that one entry — sibling files keep their original Arc.
    #[test]
    fn warm_sweep_refreshes_only_touched_files() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");

        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        cache.snapshot(&docs).expect("snapshot 1");
        let first_b_arc = cache
            .state
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|(p, _)| p.ends_with("b.pyk"))
            .and_then(|(_, e)| e.body.as_ref().map(Arc::clone))
            .unwrap();

        // Bump b.pyk on disk. We have to outwait the filesystem's mtime
        // resolution (some FSes are second-granular) so the warm sweep
        // actually sees a difference.
        sleep(Duration::from_millis(1100));
        write(&root.join("b.pyk"), "class B(Schema):\n    y: string\n");

        // Force the warm sweep to run — bypass the 1s gate.
        cache.last_warm_sweep = Some(Instant::now() - Duration::from_secs(2));
        cache.run_warm_sweep();

        let second_b_arc = cache
            .state
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|(p, _)| p.ends_with("b.pyk"))
            .and_then(|(_, e)| e.body.as_ref().map(Arc::clone))
            .unwrap();
        assert!(
            !Arc::ptr_eq(&first_b_arc, &second_b_arc),
            "expected b.pyk to be re-read after the mtime bumped",
        );
        assert!(second_b_arc.contains("string"));

        std::fs::remove_dir_all(&root).ok();
    }

    /// Project-root drift (a different open document anchoring to a
    /// different root) drops every cached entry and walks fresh.
    #[test]
    fn project_root_drift_drops_all_tiers() {
        let root_a = tmpdir();
        let root_b = tmpdir();
        write(&root_a.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root_b.join("b.pyk"), "class B(Schema):\n    y: int\n");

        let a_uri = Url::from_file_path(root_a.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        cache.snapshot(&docs).expect("snapshot 1");
        let key_1 = cache.state.as_ref().unwrap().key.clone();

        // Swap the anchor doc to one under a different project root.
        docs.clear();
        let b_uri = Url::from_file_path(root_b.join("b.pyk")).unwrap();
        docs.insert(b_uri, "class B(Schema):\n    y: int\n".into());
        cache.snapshot(&docs).expect("snapshot 2");
        let key_2 = cache.state.as_ref().unwrap().key.clone();

        assert_ne!(key_1.project_root, key_2.project_root);
        // After drift the cold walk for root_b should be visible.
        assert!(
            cache
                .state
                .as_ref()
                .unwrap()
                .entries
                .keys()
                .any(|p| p.ends_with("b.pyk")),
        );

        std::fs::remove_dir_all(&root_a).ok();
        std::fs::remove_dir_all(&root_b).ok();
    }

    /// Bumping `pykrete.json`'s mtime shifts the project key, which
    /// drops the cache and reloads the config on the next call.
    #[test]
    fn pykrete_json_mtime_change_invalidates_cache() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(
            &root.join("pykrete.json"),
            r#"{"typeCheckingMode": "basic"}"#,
        );
        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        let config_1 = cache.config(&docs);
        assert_eq!(
            config_1.check_mode_override(),
            Some(pykrete::CheckMode::Basic)
        );

        // Wait for mtime resolution, then rewrite with a different mode.
        sleep(Duration::from_millis(1100));
        write(
            &root.join("pykrete.json"),
            r#"{"typeCheckingMode": "strict"}"#,
        );

        let config_2 = cache.config(&docs);
        assert_eq!(
            config_2.check_mode_override(),
            Some(pykrete::CheckMode::Strict),
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn config_reads_pykrete_json_from_the_project_root() {
        let root = tmpdir();
        write(
            &root.join("pykrete.json"),
            r#"{"typeCheckingMode": "strict"}"#,
        );
        let docs = docs_with(&root);

        let mut cache = SnapshotCache::new();
        let config = cache.config(&docs);
        assert_eq!(
            config.check_mode_override(),
            Some(pykrete::CheckMode::Strict)
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// Typing burst: 10 didChange + 10 hover style calls in well under
    /// a second. The cold walk must run at most once — anything more
    /// means the rate limit isn't holding and we'd re-stat the whole
    /// project on every keystroke.
    #[test]
    fn typing_burst_runs_at_most_one_cold_walk() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");
        write(&root.join("c.pyk"), "class C(Schema):\n    z: int\n");

        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri.clone(), "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        for i in 0..10 {
            // Simulate didChange: mutate the in-memory body. Then
            // immediately ask for two snapshots (one diagnostic, one
            // hover-style).
            docs.insert(
                a_uri.clone(),
                format!("class A(Schema):\n    x: int  # edit {i}\n"),
            );
            cache.snapshot(&docs).expect("snapshot diag");
            cache.snapshot(&docs).expect("snapshot hover");
        }
        assert_eq!(
            cache.cold_walk_count, 1,
            "expected exactly one cold walk in a sub-second burst, ran {}",
            cache.cold_walk_count,
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn config_defaults_when_no_pykrete_json_is_present() {
        let root = tmpdir();
        let docs = docs_with(&root);

        let mut cache = SnapshotCache::new();
        let config = cache.config(&docs);
        assert_eq!(config.check_mode_override(), None);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_pykrete_json_falls_back_to_defaults_and_queues_warning() {
        let root = tmpdir();
        write(&root.join("pykrete.json"), r#"{ this is not json"#);
        let docs = docs_with(&root);

        let mut cache = SnapshotCache::new();
        let config = cache.config(&docs);
        // Defaults remain in effect — `typeCheckingMode` override absent.
        assert_eq!(config.check_mode_override(), None);

        let warning = cache
            .take_pending_warning()
            .expect("malformed pykrete.json should queue a warning");
        assert_eq!(warning.path, root.join("pykrete.json"));
        assert!(
            !warning.error.is_empty(),
            "warning error string should carry the parse failure detail"
        );

        // Drain semantics: the warning fires once per cache build.
        assert!(
            cache.take_pending_warning().is_none(),
            "warning should be consumed after the first drain"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn well_formed_pykrete_json_does_not_queue_a_warning() {
        let root = tmpdir();
        write(
            &root.join("pykrete.json"),
            r#"{"typeCheckingMode": "strict"}"#,
        );
        let docs = docs_with(&root);

        let mut cache = SnapshotCache::new();
        let _config = cache.config(&docs);
        assert!(cache.take_pending_warning().is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    /// Two-pass cold walk at the cap boundary: every `.pyk` path lands
    /// in the tracked union — those past the cap as `body = None`, the
    /// snapshot composer fills them via on-demand `read_to_string`.
    /// Without this guarantee, a >cap codebase would silently drop the
    /// excess files from every analysis.
    #[test]
    fn cold_walk_two_pass_keeps_all_paths_in_tracked_union_past_cap() {
        let root = tmpdir();
        // Three files, ~50 bytes each; cap set to 60 bytes so the first
        // file fits in the cache and the second + third spill to None.
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");
        write(&root.join("c.pyk"), "class C(Schema):\n    z: int\n");

        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        cache.max_body_bytes = 30;

        let snapshot = cache.snapshot(&docs).expect("snapshot");
        let paths: Vec<&str> = snapshot.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.iter().any(|p| p.ends_with("a.pyk")));
        assert!(paths.iter().any(|p| p.ends_with("b.pyk")));
        assert!(paths.iter().any(|p| p.ends_with("c.pyk")));

        // Tracked union has all three paths, even though the cache
        // can only hold one body's worth.
        let state = cache.state.as_ref().unwrap();
        assert!(state.entries.contains_key(&root.join("a.pyk")));
        assert!(state.entries.contains_key(&root.join("b.pyk")));
        assert!(state.entries.contains_key(&root.join("c.pyk")));

        // At least one entry must be over-cap (body = None) — exact
        // count depends on which file the iteration order hit first.
        let uncached = state.entries.values().filter(|e| e.body.is_none()).count();
        assert!(
            uncached >= 1,
            "expected at least one entry past the cap, got 0 of {}",
            state.entries.len(),
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// Paired regression for the invalidate-loop: opening a `.pyk` file
    /// whose body is None (past the cold-walk cap) does NOT trigger an
    /// outside-union invalidate, because the path IS in the tracked
    /// union. Without this, `tracks_path` would return false for
    /// uncached paths and the cache would thrash on every `didOpen`.
    #[test]
    fn tracks_path_includes_uncached_entries_past_cap() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");
        write(&root.join("c.pyk"), "class C(Schema):\n    z: int\n");

        let a_uri = Url::from_file_path(root.join("a.pyk")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let mut cache = SnapshotCache::new();
        cache.max_body_bytes = 30;
        cache.snapshot(&docs).expect("snapshot");

        // Every `.pyk` path under root should pass `tracks_path`, even
        // the entries with body = None.
        assert!(cache.tracks_path(&root.join("a.pyk")));
        assert!(cache.tracks_path(&root.join("b.pyk")));
        assert!(cache.tracks_path(&root.join("c.pyk")));

        std::fs::remove_dir_all(&root).ok();
    }

    /// `ProjectKey` derived from the same `docs` map across multiple
    /// builds must compare equal — otherwise `refresh` would think the
    /// project drifted on every call and re-run the cold walk on every
    /// didOpen / didClose with 2+ open files (`HashMap` iteration order
    /// is randomized per-process and per-insert).
    #[test]
    fn project_key_is_deterministic_across_builds() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");
        write(&root.join("c.pyk"), "class C(Schema):\n    z: int\n");

        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(
            Url::from_file_path(root.join("a.pyk")).unwrap(),
            "class A(Schema):\n    x: int\n".into(),
        );
        docs.insert(
            Url::from_file_path(root.join("b.pyk")).unwrap(),
            "class B(Schema):\n    y: int\n".into(),
        );
        docs.insert(
            Url::from_file_path(root.join("c.pyk")).unwrap(),
            "class C(Schema):\n    z: int\n".into(),
        );

        // Re-derive many times; randomized hash order should not change
        // the chosen anchor → resolved project root → key.
        let first = derive_project_key(&docs).expect("key");
        for _ in 0..32 {
            let again = derive_project_key(&docs).expect("key");
            assert_eq!(again, first);
        }

        std::fs::remove_dir_all(&root).ok();
    }

    /// Same project state, different anchor doc → same key. Two files
    /// in the same project shouldn't produce different keys depending
    /// on which one happens to win the anchor pick. Pairs with the
    /// determinism test above to pin the "no spurious invalidation on
    /// didOpen/didClose" guarantee.
    #[test]
    fn project_key_same_for_different_anchors_in_same_project() {
        let root = tmpdir();
        write(&root.join("a.pyk"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.pyk"), "class B(Schema):\n    y: int\n");

        let mut docs_a: HashMap<Url, String> = HashMap::new();
        docs_a.insert(
            Url::from_file_path(root.join("a.pyk")).unwrap(),
            "class A(Schema):\n    x: int\n".into(),
        );
        let mut docs_b: HashMap<Url, String> = HashMap::new();
        docs_b.insert(
            Url::from_file_path(root.join("b.pyk")).unwrap(),
            "class B(Schema):\n    y: int\n".into(),
        );

        let key_a = derive_project_key(&docs_a).expect("key a");
        let key_b = derive_project_key(&docs_b).expect("key b");
        assert_eq!(key_a, key_b);

        std::fs::remove_dir_all(&root).ok();
    }
}
