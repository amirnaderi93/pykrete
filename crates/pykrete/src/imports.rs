//! Import-statement parsing and per-file scoping.
//!
//! Each `.pyk` file gets its own resolution scope — schemas / functions
//! declared in OTHER files are only visible after an explicit
//! `from X import Y` statement. This module parses those imports and
//! resolves the module path to a sibling file.
//!
//! v0.1 supports:
//!
//! - `from .schemas import Orders` — relative import, same dir
//! - `from ..pkg.schemas import Orders` — relative import, parent dir(s)
//! - `from pkg.schemas import Orders` — absolute import, resolved
//!   against the project root (`pyproject.toml`-anchored)
//! - `as` aliases on any of the above
//!
//! Out of scope (deferred to a follow-up):
//!
//! - `import X` and qualified access (`X.Y`) — needs attribute-access
//!   tracking on module names.
//! - `from X import *` — wildcard imports are diagnosed as
//!   "unsupported" rather than expanded.

use std::path::{Path, PathBuf};

use ruff_python_ast::{ModModule, Stmt};
use ruff_text_size::TextRange;

/// One `from X import Y [as Z]` clause, parsed out of a file's AST.
///
/// `local_name` is the name visible inside the importing file (so `Z` if
/// `as Z` is present, else `Y`). `source_name` is the name in the source
/// module. `module` is the module path verbatim from the source — the
/// caller resolves it to a file path via [`resolve_module_to_file`].
#[derive(Debug, Clone)]
pub struct ImportedName<'a> {
    pub local_name: &'a str,
    pub source_name: &'a str,
    pub module: ModulePath,
    /// Source range of the imported-name token, for diagnostic anchoring.
    pub range: TextRange,
}

/// A module path the way it appears in an `import`-from clause.
///
/// `level` is the number of leading dots: 0 for absolute (`pkg.x`), 1
/// for same-dir relative (`.x`), 2 for parent-dir relative (`..x`), etc.
/// `segments` is the dotted tail. `from .schemas import X` →
/// `ModulePath { level: 1, segments: ["schemas"] }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePath {
    pub level: u32,
    pub segments: Vec<String>,
}

impl ModulePath {
    /// Resolve this module path against `importing_file` (the path of
    /// the file that contains the `from … import …` clause) and
    /// `project_root` (the directory containing `pyproject.toml`, or
    /// the longest common ancestor of the input files when no
    /// `pyproject.toml` is found).
    ///
    /// Returns the absolute `.pyk` path of the source module. The
    /// caller then matches it against the list of files pykrete is
    /// checking.
    pub fn resolve(&self, importing_file: &Path, project_root: &Path) -> Option<PathBuf> {
        let base: PathBuf = if self.level == 0 {
            project_root.to_path_buf()
        } else {
            // For relative imports, start from the importing file's
            // parent dir and walk up `level - 1` directories. Level 1
            // means "same dir as the importing file"; level 2 means
            // "parent dir"; and so on.
            let mut dir = importing_file.parent()?.to_path_buf();
            for _ in 1..self.level {
                dir = dir.parent()?.to_path_buf();
            }
            dir
        };
        let mut path = base;
        for segment in &self.segments {
            path.push(segment);
        }
        path.set_extension("pyk");
        Some(path)
    }
}

/// Walk a parsed module and return every `from … import …` clause we
/// can recognize as a possible cross-file schema/function reference.
/// `import X` and `from X import *` are not produced here — they're
/// diagnosed separately at the analysis layer.
pub fn parse_imports(module: &ModModule) -> Vec<ImportedName<'_>> {
    let mut out = Vec::new();
    for stmt in &module.body {
        let Stmt::ImportFrom(import) = stmt else {
            continue;
        };
        let module_path = ModulePath {
            level: import.level,
            segments: import
                .module
                .as_ref()
                .map(|m| m.id.as_str().split('.').map(String::from).collect())
                .unwrap_or_default(),
        };
        for alias in &import.names {
            // Skip `from X import *` — we record it as a no-op and let
            // the analysis layer surface a diagnostic if it wants to.
            if alias.name.id.as_str() == "*" {
                continue;
            }
            let source_name = alias.name.id.as_str();
            let local_name = alias
                .asname
                .as_ref()
                .map(|n| n.id.as_str())
                .unwrap_or(source_name);
            out.push(ImportedName {
                local_name,
                source_name,
                module: module_path.clone(),
                range: alias.range,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Project root discovery
// ---------------------------------------------------------------------------

/// Walk up from `start` looking for a directory that contains
/// `pyproject.toml`. Returns the deepest match, or `None` if the file
/// system root is reached without finding one.
///
/// Used by `check_project` to anchor absolute imports.
pub fn find_pyproject_root(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        if dir.join("pyproject.toml").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Fall back when no `pyproject.toml` is found: the longest common
/// ancestor directory of every input file. This keeps absolute imports
/// working in test fixtures and ad-hoc scripts.
pub fn longest_common_ancestor<I, P>(paths: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut paths = paths.into_iter();
    let first = paths.next()?;
    let mut ancestor: PathBuf = first.as_ref().parent()?.to_path_buf();
    for path in paths {
        let parent = path.as_ref().parent()?;
        ancestor = common_prefix(&ancestor, parent);
    }
    Some(ancestor)
}

fn common_prefix(a: &Path, b: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for (sa, sb) in a.components().zip(b.components()) {
        if sa == sb {
            out.push(sa.as_os_str());
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn module_path_resolves_a_relative_import_to_a_sibling_file() {
        let importing = Path::new("/project/src/pipeline.pyk");
        let root = Path::new("/project");
        let module = ModulePath {
            level: 1,
            segments: vec!["schemas".to_string()],
        };
        let resolved = module.resolve(importing, root).unwrap();
        assert_eq!(resolved, PathBuf::from("/project/src/schemas.pyk"));
    }

    #[test]
    fn module_path_resolves_an_absolute_import_against_the_project_root() {
        let importing = Path::new("/project/src/pipeline.pyk");
        let root = Path::new("/project");
        let module = ModulePath {
            level: 0,
            segments: vec!["src".to_string(), "schemas".to_string()],
        };
        let resolved = module.resolve(importing, root).unwrap();
        assert_eq!(resolved, PathBuf::from("/project/src/schemas.pyk"));
    }

    #[test]
    fn module_path_walks_up_for_double_dot_relative_imports() {
        let importing = Path::new("/project/src/jobs/pipeline.pyk");
        let root = Path::new("/project");
        let module = ModulePath {
            level: 2,
            segments: vec!["schemas".to_string()],
        };
        let resolved = module.resolve(importing, root).unwrap();
        assert_eq!(resolved, PathBuf::from("/project/src/schemas.pyk"));
    }

    #[test]
    fn longest_common_ancestor_of_two_sibling_files_is_their_shared_dir() {
        let ancestor =
            longest_common_ancestor(["/project/src/a.pyk", "/project/src/b.pyk"]).unwrap();
        assert_eq!(ancestor, PathBuf::from("/project/src"));
    }

    #[test]
    fn longest_common_ancestor_of_cousins_is_the_first_shared_dir() {
        let ancestor = longest_common_ancestor([
            "/project/src/jobs/pipeline.pyk",
            "/project/src/utils/schemas.pyk",
        ])
        .unwrap();
        assert_eq!(ancestor, PathBuf::from("/project/src"));
    }
}
