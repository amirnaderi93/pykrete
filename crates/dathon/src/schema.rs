//! Schema declarations and field-type resolution.
//!
//! A dathon schema is a Python class whose bases include `Schema`. The fields
//! of the schema are the annotated assignments in the class body
//! (`name: type_expression`). Methods, docstrings, plain assignments without
//! annotations, and nested classes are ignored.

use ruff_python_ast::Expr;

use crate::types::ColumnType;
use crate::walk::DiscoveredClass;

#[derive(Debug)]
pub struct Schema<'ast> {
    pub class: &'ast DiscoveredClass<'ast>,
}

#[derive(Debug)]
pub struct SchemaField<'ast> {
    pub name: &'ast str,
    pub annotation: &'ast Expr,
}

/// Outcome of resolving a single field's annotation. AST-shaped reasons
/// for failure; mapping these to user-facing diagnostics happens at the
/// CLI / driver layer.
#[derive(Debug)]
pub enum FieldResolution<'ast> {
    Resolved(ColumnType),
    UnknownType { name: &'ast str },
    NotABareName,
}

impl<'ast> SchemaField<'ast> {
    pub fn resolve(&self) -> FieldResolution<'ast> {
        match self.annotation {
            Expr::Name(name) => match ColumnType::from_name(name.id.as_str()) {
                Some(ct) => FieldResolution::Resolved(ct),
                None => FieldResolution::UnknownType {
                    name: name.id.as_str(),
                },
            },
            _ => FieldResolution::NotABareName,
        }
    }
}

impl<'ast> Schema<'ast> {
    pub fn from_class(class: &'ast DiscoveredClass<'ast>) -> Option<Self> {
        if class.has_base("Schema") {
            Some(Self { class })
        } else {
            None
        }
    }

    pub fn name(&self) -> &'ast str {
        self.class.name()
    }

    pub fn fields(&self) -> Vec<SchemaField<'ast>> {
        self.class
            .def
            .body
            .iter()
            .filter_map(|stmt| {
                let ann = stmt.as_ann_assign_stmt()?;
                let name = ann.target.as_name_expr()?.id.as_str();
                Some(SchemaField {
                    name,
                    annotation: &ann.annotation,
                })
            })
            .collect()
    }
}

pub fn discover_schemas<'ast>(classes: &'ast [DiscoveredClass<'ast>]) -> Vec<Schema<'ast>> {
    classes.iter().filter_map(Schema::from_class).collect()
}
