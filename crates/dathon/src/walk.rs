//! AST walking helpers.
//!
//! For now we only walk for top-level class definitions. These will later be
//! recognized as Schema declarations once we add the type system.

use ruff_python_ast::{Expr, ModModule, Stmt, StmtClassDef};

#[derive(Debug)]
pub struct DiscoveredClass<'ast> {
    pub def: &'ast StmtClassDef,
}

impl<'ast> DiscoveredClass<'ast> {
    pub fn name(&self) -> &str {
        self.def.name.id.as_str()
    }

    /// Best-effort textual rendering of base classes. Complex bases
    /// (subscripts, attribute access, etc.) render as `?` for now.
    pub fn base_names(&self) -> Vec<String> {
        self.def
            .bases()
            .iter()
            .map(|expr| match expr {
                Expr::Name(name) => name.id.as_str().to_string(),
                _ => "?".to_string(),
            })
            .collect()
    }
}

pub fn discover_top_level_classes(module: &ModModule) -> Vec<DiscoveredClass<'_>> {
    module
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(def) => Some(DiscoveredClass { def }),
            _ => None,
        })
        .collect()
}
