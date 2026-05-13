//! Project-snapshot helper for cross-file LSP analysis.
//!
//! Each LSP request that needs cross-file context (today: diagnostics)
//! calls [`build_project_snapshot`] to produce a list of every `.dpy`
//! file dathon should see — every sibling file under the project root,
//! with the open editor's in-memory contents overriding what's on disk
//! for any URI the client has told us about.
//!
//! The project root is the deepest `pyproject.toml`-bearing directory
//! above the first reachable open document, falling back to that
//! document's parent dir when no `pyproject.toml` is found. This is the
//! same rule [`dathon::check_project`] uses for resolving absolute
//! imports.
//!
//! Out of scope for v0.1: file watching. We re-scan the project on
//! every analysis pass, which is fine at typical dathon project sizes
//! (a few dozen `.dpy` files at most) and avoids the complexity of
//! maintaining an inotify/fsevents watch.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use lsp_types::Url;

use dathon::imports::find_pyproject_root;

/// Build a `(path, source)` snapshot of the entire project for the
/// currently-open editor session.
///
/// Returns `None` if no open document has a usable filesystem path —
/// the caller falls back to single-file analysis in that case.
pub fn build_project_snapshot(docs: &HashMap<Url, String>) -> Option<Vec<(String, String)>> {
    let anchor = docs
        .keys()
        .find_map(|uri| uri.to_file_path().ok())
        .map(PathBuf::from)?;

    let project_root = find_pyproject_root(&anchor).unwrap_or_else(|| {
        anchor
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });

    let open_by_path: HashMap<PathBuf, &String> = docs
        .iter()
        .filter_map(|(uri, src)| uri.to_file_path().ok().map(|p| (p, src)))
        .collect();

    let mut dpy_paths: Vec<PathBuf> = Vec::new();
    if collect_dpy_paths(&project_root, &mut dpy_paths).is_err() {
        // If the walk fails (e.g. permissions), fall back to just the
        // open files so the user at least gets single-file analysis.
        dpy_paths.clear();
    }
    // Always include every open file, even if it sits outside the
    // discovered project root — for example, a one-off `.dpy` opened
    // by absolute path with no `pyproject.toml` nearby.
    for path in open_by_path.keys() {
        if !dpy_paths.contains(path) {
            dpy_paths.push(path.clone());
        }
    }
    dpy_paths.sort();
    dpy_paths.dedup();

    let mut out = Vec::with_capacity(dpy_paths.len());
    for path in dpy_paths {
        let source = if let Some(in_memory) = open_by_path.get(&path) {
            (*in_memory).clone()
        } else {
            match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            }
        };
        out.push((path.to_string_lossy().into_owned(), source));
    }
    Some(out)
}

/// Recursively collect every `.dpy` file under `dir`. Mirrors the CLI's
/// `walk_dpy` so directory mode in the binary and project mode in the
/// LSP agree on what counts as "in the project."
fn collect_dpy_paths(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_dpy_paths(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("dpy") {
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

    fn tmpdir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "dathon-lsp-test-{}-{}",
            std::process::id(),
            // Cheap unique suffix; the test deletes its dir on cleanup.
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

    #[test]
    fn snapshot_returns_none_when_no_open_doc_has_a_file_path() {
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(
            Url::parse("untitled:Untitled-1").unwrap(),
            "class X(Schema):\n    a: int\n".to_string(),
        );
        assert!(build_project_snapshot(&docs).is_none());
    }

    #[test]
    fn snapshot_includes_sibling_dpy_files_from_disk() {
        let root = tmpdir();
        write(&root.join("a.dpy"), "class A(Schema):\n    x: int\n");
        write(&root.join("b.dpy"), "class B(Schema):\n    y: int\n");

        let a_uri = Url::from_file_path(root.join("a.dpy")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(
            a_uri.clone(),
            "class A(Schema):\n    x: int  # edited\n".into(),
        );

        let snapshot = build_project_snapshot(&docs).expect("snapshot");
        let paths: Vec<&str> = snapshot.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.iter().any(|p| p.ends_with("a.dpy")));
        assert!(paths.iter().any(|p| p.ends_with("b.dpy")));

        let (_, a_source) = snapshot.iter().find(|(p, _)| p.ends_with("a.dpy")).unwrap();
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
        write(&root.join("a.dpy"), "class A(Schema):\n    x: int\n");
        write(
            &root.join("sub").join("b.dpy"),
            "class B(Schema):\n    y: int\n",
        );

        let a_uri = Url::from_file_path(root.join("a.dpy")).unwrap();
        let mut docs: HashMap<Url, String> = HashMap::new();
        docs.insert(a_uri, "class A(Schema):\n    x: int\n".into());

        let snapshot = build_project_snapshot(&docs).expect("snapshot");
        assert!(snapshot.iter().any(|(p, _)| p.ends_with("a.dpy")));
        assert!(snapshot.iter().any(|(p, _)| p.ends_with("b.dpy")));

        std::fs::remove_dir_all(&root).ok();
    }
}
