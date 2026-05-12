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

    pub fn has_field(&self, name: &str) -> bool {
        self.fields().iter().any(|f| f.name == name)
    }
}

pub fn discover_schemas<'ast>(classes: &'ast [DiscoveredClass<'ast>]) -> Vec<Schema<'ast>> {
    classes.iter().filter_map(Schema::from_class).collect()
}

// ---------------------------------------------------------------------------
// SchemaView — unified view over declared and derived schemas
// ---------------------------------------------------------------------------

/// Either a user-declared `Schema` class, or a schema *inferred* from the
/// result of an operation chain (e.g. `raw.select("a", "b")` produces a
/// Derived schema with fields `["a", "b"]`).
///
/// All field-existence and field-set comparisons go through this view, so
/// checks (`D0030`, `D0040`, `D0050`) work identically against declared and
/// derived schemas.
#[derive(Debug, Clone)]
pub enum SchemaView<'a> {
    Declared(&'a Schema<'a>),
    Derived(Vec<&'a str>),
}

impl<'a> SchemaView<'a> {
    pub fn has_field(&self, name: &str) -> bool {
        match self {
            Self::Declared(s) => s.has_field(name),
            Self::Derived(fields) => fields.iter().any(|f| *f == name),
        }
    }

    pub fn field_names(&self) -> Vec<&'a str> {
        match self {
            Self::Declared(s) => s.fields().iter().map(|f| f.name).collect(),
            Self::Derived(fields) => fields.clone(),
        }
    }

    /// Human-readable phrase to embed in diagnostics — `schema 'Orders'` for
    /// declared, `inferred schema [a, b]` for derived.
    pub fn display_name(&self) -> String {
        match self {
            Self::Declared(s) => format!("schema '{}'", s.name()),
            Self::Derived(fields) => format!("inferred schema [{}]", fields.join(", ")),
        }
    }
}
