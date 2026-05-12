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
    /// An atomic column type from the v0.1 vocabulary.
    Resolved(ColumnType),
    /// A nested struct — the field's type is another declared `Schema`
    /// class in this file. The Spark representation is `StructType`.
    ResolvedNested(&'ast Schema<'ast>),
    /// A bare-name annotation that's neither in the atomic vocabulary
    /// nor a known declared Schema class.
    UnknownType { name: &'ast str },
    /// The annotation isn't a bare name at all (subscript, attribute
    /// access, binary op, …).
    NotABareName,
}

impl<'ast> SchemaField<'ast> {
    /// Resolve the field's annotation against the atomic-type vocabulary
    /// and the set of declared Schema classes.
    ///
    /// The `schemas` parameter is the list of every Schema discovered in
    /// the same source file; nested struct references look the field's
    /// annotation name up there.
    pub fn resolve(&self, schemas: &'ast [Schema<'ast>]) -> FieldResolution<'ast> {
        match self.annotation {
            Expr::Name(name) => {
                let id = name.id.as_str();
                if let Some(ct) = ColumnType::from_name(id) {
                    return FieldResolution::Resolved(ct);
                }
                if let Some(schema) = schemas.iter().find(|s| s.name() == id) {
                    return FieldResolution::ResolvedNested(schema);
                }
                FieldResolution::UnknownType { name: id }
            }
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
/// result of an operation chain.
///
/// `Grouped` is a third state: it's not a DataFrame, but the intermediate
/// value produced by `.groupBy(...)`. It carries the group keys plus the
/// underlying schema, so a subsequent `.agg(...)` can resolve its column
/// references against the original input.
#[derive(Debug, Clone)]
pub enum SchemaView<'a> {
    Declared(&'a Schema<'a>),
    Derived(Vec<&'a str>),
    Grouped {
        keys: Vec<&'a str>,
        underlying: Box<SchemaView<'a>>,
    },
}

impl<'a> SchemaView<'a> {
    pub fn has_field(&self, name: &str) -> bool {
        match self {
            Self::Declared(s) => s.has_field(name),
            Self::Derived(fields) => fields.iter().any(|f| *f == name),
            // GroupedData isn't field-queryable directly. Operations apart
            // from .agg (filter, select, etc.) on a Grouped receiver will
            // collect col-ref diagnostics — that's fine since those calls
            // are invalid in PySpark anyway.
            Self::Grouped { .. } => false,
        }
    }

    pub fn field_names(&self) -> Vec<&'a str> {
        match self {
            Self::Declared(s) => s.fields().iter().map(|f| f.name).collect(),
            Self::Derived(fields) => fields.clone(),
            Self::Grouped { keys, .. } => keys.clone(),
        }
    }

    /// Human-readable phrase to embed in diagnostics.
    pub fn display_name(&self) -> String {
        match self {
            Self::Declared(s) => format!("schema '{}'", s.name()),
            Self::Derived(fields) => format!("inferred schema [{}]", fields.join(", ")),
            Self::Grouped { keys, .. } => {
                format!("grouped data with keys [{}]", keys.join(", "))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests for SchemaView::Derived
//
// SchemaView::Declared needs a parsed Schema (which needs AST data), so it's
// only exercised in integration tests. The Derived variant is pure-data and
// easy to test directly here.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_schema_has_field_returns_true_for_known_columns() {
        let view = SchemaView::Derived(vec!["a", "b", "c"]);
        assert!(view.has_field("a"));
        assert!(view.has_field("b"));
        assert!(view.has_field("c"));
    }

    #[test]
    fn derived_schema_has_field_returns_false_for_unknown_columns() {
        let view = SchemaView::Derived(vec!["a", "b"]);
        assert!(!view.has_field("c"));
        assert!(!view.has_field(""));
    }

    #[test]
    fn derived_schema_has_field_is_case_sensitive() {
        // Column names are matched exactly. Spark itself defaults to
        // case-insensitive matching, but dathon is stricter; this keeps
        // typos like 'PRICE' vs 'price' detectable.
        let view = SchemaView::Derived(vec!["price"]);
        assert!(view.has_field("price"));
        assert!(!view.has_field("Price"));
        assert!(!view.has_field("PRICE"));
    }

    #[test]
    fn derived_schema_field_names_preserves_order() {
        // Column order matters for `union` (vs `unionByName`); preserve the
        // insertion order from the operation that produced this schema.
        let view = SchemaView::Derived(vec!["x", "y", "z"]);
        assert_eq!(view.field_names(), vec!["x", "y", "z"]);
    }

    #[test]
    fn derived_display_name_lists_fields_in_brackets() {
        // The format embedded in D0030 / D0040 messages when the schema
        // doesn't have a user-facing name.
        let view = SchemaView::Derived(vec!["a", "b"]);
        assert_eq!(view.display_name(), "inferred schema [a, b]");
    }

    #[test]
    fn derived_display_name_handles_empty_field_list() {
        // Can happen after `select` with all aliasless complex expressions.
        let view = SchemaView::Derived(vec![]);
        assert_eq!(view.display_name(), "inferred schema []");
    }

    #[test]
    fn grouped_schema_has_field_returns_false_for_all_names() {
        // GroupedData isn't field-queryable; only .agg(...) produces a real
        // DataFrame. has_field uniformly returns false so accidental
        // non-agg operations on a Grouped receiver get caught.
        let underlying = SchemaView::Derived(vec!["k", "a", "b"]);
        let grouped = SchemaView::Grouped {
            keys: vec!["k"],
            underlying: Box::new(underlying),
        };
        assert!(!grouped.has_field("k"));
        assert!(!grouped.has_field("a"));
    }

    #[test]
    fn grouped_field_names_returns_just_the_keys() {
        // Used by the agg result-inference path: keys ∪ aliased aggregates.
        let underlying = SchemaView::Derived(vec!["k1", "k2", "v"]);
        let grouped = SchemaView::Grouped {
            keys: vec!["k1", "k2"],
            underlying: Box::new(underlying),
        };
        assert_eq!(grouped.field_names(), vec!["k1", "k2"]);
    }

    #[test]
    fn grouped_display_name_describes_the_keys() {
        let underlying = SchemaView::Derived(vec!["k", "v"]);
        let grouped = SchemaView::Grouped {
            keys: vec!["k"],
            underlying: Box::new(underlying),
        };
        assert_eq!(grouped.display_name(), "grouped data with keys [k]");
    }
}
