//! AST walking helpers.
//!
//! Read-only traversals over the parsed `ModModule`. Today we discover
//! top-level class definitions and top-level function definitions. Will
//! grow as we need to find more declaration shapes (imports, DataSource
//! constants, etc.).

use ruff_python_ast::{ModModule, Stmt, StmtClassDef, StmtFunctionDef};

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
        self.base_names().contains(&name)
    }

    /// The bare-name bases of this class, in declaration order. Subscript
    /// bases (`Generic[T]`) and attribute bases (`mod.Base`) are skipped —
    /// the same limitation as [`DiscoveredClass::has_base`].
    pub fn base_names(&self) -> Vec<&'ast str> {
        self.def
            .bases()
            .iter()
            .filter_map(|expr| expr.as_name_expr().map(|n| n.id.as_str()))
            .collect()
    }
}

#[derive(Debug)]
pub struct DiscoveredFunction<'ast> {
    pub def: &'ast StmtFunctionDef,
}

impl<'ast> DiscoveredFunction<'ast> {
    pub fn name(&self) -> &'ast str {
        self.def.name.id.as_str()
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

pub fn discover_top_level_functions(module: &ModModule) -> Vec<DiscoveredFunction<'_>> {
    module
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::FunctionDef(def) => Some(DiscoveredFunction { def }),
            _ => None,
        })
        .collect()
}
