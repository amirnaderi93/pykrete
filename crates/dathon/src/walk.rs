//! AST walking helpers.
//!
//! Read-only traversals over the parsed `ModModule`. Today we only discover
//! top-level class definitions; this will grow as we need to find more
//! declaration shapes.

use ruff_python_ast::{ModModule, Stmt, StmtClassDef};

#[derive(Debug)]
pub struct DiscoveredClass<'ast> {
    pub def: &'ast StmtClassDef,
}

impl<'ast> DiscoveredClass<'ast> {
    pub fn name(&self) -> &'ast str {
        self.def.name.id.as_str()
    }

    /// True if any base of this class is exactly the bare name `name`.
    /// Subscripts (`Foo[T]`) and attribute access (`mod.Foo`) are not matched —
    /// we can broaden this later if/when import resolution lands.
    pub fn has_base(&self, name: &str) -> bool {
        self.def.bases().iter().any(|expr| {
            expr.as_name_expr()
                .map(|n| n.id.as_str() == name)
                .unwrap_or(false)
        })
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
