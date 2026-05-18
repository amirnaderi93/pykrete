//! Schema declarations and field-type resolution.
//!
//! A dathon schema is a Python class whose bases include `Schema`. The fields
//! of the schema are the annotated assignments in the class body
//! (`name: type_expression`). Methods, docstrings, plain assignments without
//! annotations, and nested classes are ignored.

use ruff_python_ast::Expr;
use ruff_text_size::{Ranged, TextRange};

use crate::types::{COLUMN_TYPE_NAMES, ColumnType, StructField};
use crate::walk::DiscoveredClass;

/// Recursion ceiling for resolving a (possibly self-referential) schema
/// into a nested [`ColumnType`]. Spark types are finite — a cyclic
/// schema is a user error; this just stops the resolver looping.
const MAX_TYPE_DEPTH: usize = 24;

#[derive(Debug)]
pub struct Schema<'ast> {
    pub class: &'ast DiscoveredClass<'ast>,
    /// Ancestor classes this schema inherits fields from, nearest base
    /// first — resolved from the class hierarchy at discovery. Empty for
    /// a schema that extends only the bare `Schema` marker; a subclass
    /// `class Premium(Orders)` carries `[Orders]` here, so `fields()`
    /// surfaces `Orders`' columns too. Same-file bases only — a base
    /// imported from another module is not resolved.
    pub bases: Vec<&'ast DiscoveredClass<'ast>>,
    /// Set when this schema was pulled in via `from X import Y as Z` —
    /// the importing file should see it under the local name `Z`, even
    /// though its actual class declaration is `class Y(Schema)`. `None`
    /// for schemas declared locally (which use the class name verbatim).
    pub alias: Option<&'ast str>,
    /// Index of the project file this schema was declared in. Used by
    /// go-to-definition to point at the right file when the schema was
    /// imported from another module. `0` in single-file contexts (the
    /// only file) and until [`crate::ProjectContext`] tags it.
    pub file_index: usize,
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
    /// Field types are written as ordinary Python type annotations: a
    /// bare name for an atomic type (`int`, `double`, …) or a referenced
    /// `Schema` class (`address: Address`), and a subscript for a
    /// collection (`Array[int]`, `Map[string, Event]`).
    ///
    /// The `schemas` parameter is the list of every Schema discovered in
    /// the same source file; nested struct references look the field's
    /// annotation name up there.
    pub fn resolve(&self, schemas: &'ast [Schema<'ast>]) -> FieldResolution<'ast> {
        match self.annotation {
            // A bare name — an atomic type, or a nested `Schema` class.
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
            // A subscript — `Array[…]` / `Map[…, …]`.
            Expr::Subscript(_) => match resolve_annotation_type(self.annotation, schemas, 0) {
                Some(ct) => FieldResolution::Resolved(ct),
                None => FieldResolution::NotABareName,
            },
            _ => FieldResolution::NotABareName,
        }
    }
}

impl<'ast> Schema<'ast> {
    /// The name this schema should resolve under in the current scope —
    /// the alias if one is set, otherwise the class's declared name.
    pub fn name(&self) -> &'ast str {
        self.alias.unwrap_or_else(|| self.class.name())
    }

    /// The class's declared name verbatim, ignoring any import alias.
    /// Used for hover / go-to-definition where we want to show what the
    /// schema is actually called in its source file.
    pub fn declared_name(&self) -> &'ast str {
        self.class.name()
    }

    /// Every column of this schema — inherited columns first (farthest
    /// ancestor to nearest), then this class's own. A column redeclared
    /// in a subclass overrides the inherited one: it keeps the inherited
    /// position but takes the subclass's annotation.
    pub fn fields(&self) -> Vec<SchemaField<'ast>> {
        let mut fields: Vec<SchemaField<'ast>> = Vec::new();
        for base in self.bases.iter().rev() {
            for field in class_body_fields(base) {
                push_or_override(&mut fields, field);
            }
        }
        for field in class_body_fields(self.class) {
            push_or_override(&mut fields, field);
        }
        fields
    }

    pub fn has_field(&self, name: &str) -> bool {
        self.fields().iter().any(|f| f.name == name)
    }

    /// The full [`ColumnType`] of field `name`, resolved recursively —
    /// nested `array` / `map` element types and struct fields included.
    /// `None` for an unknown field or one whose type doesn't resolve.
    pub fn field_type(&self, name: &str, schemas: &'ast [Schema<'ast>]) -> Option<ColumnType> {
        self.fields()
            .iter()
            .find(|f| f.name == name)
            .and_then(|f| resolve_field_column_type(f, schemas, 0))
    }
}

/// Resolve a schema field's annotation to its full [`ColumnType`].
fn resolve_field_column_type(
    field: &SchemaField<'_>,
    schemas: &[Schema<'_>],
    depth: usize,
) -> Option<ColumnType> {
    resolve_annotation_type(field.annotation, schemas, depth)
}

/// Resolve a type-annotation expression to a [`ColumnType`], descending
/// recursively through `Array[…]` / `Map[…, …]` subscripts and into
/// referenced `Schema` classes.
fn resolve_annotation_type(
    expr: &Expr,
    schemas: &[Schema<'_>],
    depth: usize,
) -> Option<ColumnType> {
    match expr {
        // A bare name: an atomic type, or a referenced `Schema` class.
        Expr::Name(n) => {
            let id = n.id.as_str();
            if let Some(ct) = ColumnType::from_name(id) {
                return Some(ct);
            }
            schemas
                .iter()
                .find(|sc| sc.name() == id)
                .map(|sc| schema_as_struct(sc, schemas, depth))
        }
        // `Array[T]` / `Map[K, V]`.
        Expr::Subscript(sub) => {
            let base = sub.value.as_name_expr()?.id.as_str();
            match base {
                "Array" | "array" => Some(ColumnType::Array(Some(Box::new(
                    resolve_annotation_type(&sub.slice, schemas, depth)?,
                )))),
                // `Optional[T]` — a nullable column.
                "Optional" => Some(ColumnType::Nullable(Box::new(resolve_annotation_type(
                    &sub.slice, schemas, depth,
                )?))),
                "Map" | "map" => {
                    let tuple = sub.slice.as_tuple_expr()?;
                    let [key, value] = tuple.elts.as_slice() else {
                        return None;
                    };
                    Some(ColumnType::Map(
                        Some(Box::new(resolve_annotation_type(key, schemas, depth)?)),
                        Some(Box::new(resolve_annotation_type(value, schemas, depth)?)),
                    ))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Resolve a declared `Schema` class to a [`ColumnType::Struct`] — each
/// field resolved recursively. `depth` guards against a self-referential
/// schema (Spark types are finite; a cycle is a user error, and beyond
/// the ceiling the struct is truncated rather than looping forever).
fn schema_as_struct(schema: &Schema<'_>, schemas: &[Schema<'_>], depth: usize) -> ColumnType {
    if depth >= MAX_TYPE_DEPTH {
        return ColumnType::Struct(Vec::new());
    }
    let fields = schema
        .fields()
        .iter()
        .map(|f| StructField {
            name: f.name.to_string(),
            ty: resolve_field_column_type(f, schemas, depth + 1),
        })
        .collect();
    ColumnType::Struct(fields)
}

/// The `name: type` annotated assignments in a class body, in order.
/// Methods, plain assignments, docstrings and nested classes are skipped.
fn class_body_fields<'ast>(class: &'ast DiscoveredClass<'ast>) -> Vec<SchemaField<'ast>> {
    class
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

/// Append `field`, or — if a field of the same name is already present —
/// overwrite it in place. A subclass redeclaration wins over the
/// inherited column while keeping that column's position.
fn push_or_override<'ast>(fields: &mut Vec<SchemaField<'ast>>, field: SchemaField<'ast>) {
    match fields.iter_mut().find(|f| f.name == field.name) {
        Some(existing) => *existing = field,
        None => fields.push(field),
    }
}

/// The ancestor classes a schema inherits fields from — every class
/// reachable through its base list, nearest base first. The bare
/// `Schema` marker has no class declaration and simply isn't found; a
/// base imported from another module isn't found either (cross-file
/// inheritance is not resolved yet). Guards against an inheritance cycle.
fn resolve_schema_bases<'ast>(
    class: &'ast DiscoveredClass<'ast>,
    classes: &'ast [DiscoveredClass<'ast>],
) -> Vec<&'ast DiscoveredClass<'ast>> {
    let mut chain: Vec<&'ast DiscoveredClass<'ast>> = Vec::new();
    let mut frontier: Vec<&'ast str> = class.base_names();
    let mut steps = 0;
    while let Some(base) = frontier.pop() {
        steps += 1;
        if steps > MAX_TYPE_DEPTH {
            break;
        }
        let Some(base_class) = classes.iter().find(|c| c.name() == base) else {
            continue;
        };
        if chain.iter().any(|c| std::ptr::eq(*c, base_class)) {
            continue;
        }
        chain.push(base_class);
        frontier.extend(base_class.base_names());
    }
    chain
}

/// Discover every `Schema` class in `classes`.
///
/// A class is a schema if one of its bases is the bare `Schema` marker,
/// or another class that is itself a schema — `class Premium(Orders)`
/// inherits Schema-ness, to any depth. The set is resolved as a fixpoint
/// over the class list. Each resulting [`Schema`] carries its ancestor
/// chain in [`Schema::bases`], so `fields()` surfaces inherited columns.
pub fn discover_schemas<'ast>(classes: &'ast [DiscoveredClass<'ast>]) -> Vec<Schema<'ast>> {
    let mut is_schema: Vec<bool> = classes.iter().map(|c| c.has_base("Schema")).collect();
    loop {
        let mut changed = false;
        for i in 0..classes.len() {
            if is_schema[i] {
                continue;
            }
            let extends_a_schema = classes[i].base_names().iter().any(|&base| {
                classes
                    .iter()
                    .position(|c| c.name() == base)
                    .is_some_and(|j| is_schema[j])
            });
            if extends_a_schema {
                is_schema[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    classes
        .iter()
        .enumerate()
        .filter(|&(i, _)| is_schema[i])
        .map(|(_, class)| Schema {
            class,
            bases: resolve_schema_bases(class, classes),
            alias: None,
            file_index: 0,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// SchemaView — unified view over declared and derived schemas
// ---------------------------------------------------------------------------

/// One column of an inferred ([`SchemaView::Derived`]) schema: its name
/// and, when dathon could work it out, its atomic type.
///
/// `ty` is `None` when the type couldn't be inferred (a function result
/// dathon doesn't model, a column off another Derived schema, …). An
/// unknown type is treated permissively — like TypeScript `any` — and is
/// never itself the source of a type error.
#[derive(Debug, Clone)]
pub struct DerivedField<'a> {
    pub name: &'a str,
    pub ty: Option<ColumnType>,
}

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
    Derived(Vec<DerivedField<'a>>),
    Grouped {
        keys: Vec<&'a str>,
        underlying: Box<SchemaView<'a>>,
    },
}

impl<'a> SchemaView<'a> {
    /// Build a Derived view from column names whose types aren't (yet)
    /// inferred — every field gets `ty: None`.
    pub fn derived_untyped(names: Vec<&'a str>) -> Self {
        Self::Derived(
            names
                .into_iter()
                .map(|name| DerivedField { name, ty: None })
                .collect(),
        )
    }

    pub fn has_field(&self, name: &str) -> bool {
        match self {
            Self::Declared(s) => s.has_field(name),
            Self::Derived(fields) => fields.iter().any(|f| f.name == name),
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
            Self::Derived(fields) => fields.iter().map(|f| f.name).collect(),
            Self::Grouped { keys, .. } => keys.clone(),
        }
    }

    /// The atomic type of column `name` on this view, if known. `None`
    /// for an unknown column, an un-inferred Derived field, or a field
    /// whose declared type is itself a nested struct.
    ///
    /// `name` may be a dotted path (`"address.zipcode"`); on a declared
    /// schema each non-final segment is walked through its nested
    /// struct, and the leaf segment's atomic type is returned.
    pub fn field_type(&self, name: &str, schemas: &'a [Schema<'a>]) -> Option<ColumnType> {
        match self {
            Self::Declared(s) => {
                if let Some((head, rest)) = name.split_once('.') {
                    // Walk into the nested struct named by `head`.
                    let nested = s.fields().iter().find(|f| f.name == head).and_then(|f| {
                        match f.resolve(schemas) {
                            FieldResolution::ResolvedNested(n) => Some(n),
                            _ => None,
                        }
                    })?;
                    return Self::Declared(nested).field_type(rest, schemas);
                }
                s.field_type(name, schemas)
            }
            Self::Derived(fields) => {
                fields.iter().find(|f| f.name == name).and_then(|f| f.ty.clone())
            }
            Self::Grouped { underlying, .. } => underlying.field_type(name, schemas),
        }
    }

    /// Every column of this view as a [`DerivedField`] — its name paired
    /// with its type (where known). The shared basis for operations that
    /// carry columns through: `drop`, `withColumn`, `select("*")`, etc.
    pub fn typed_fields(&self, schemas: &'a [Schema<'a>]) -> Vec<DerivedField<'a>> {
        self.field_names()
            .into_iter()
            .map(|name| DerivedField {
                name,
                ty: self.field_type(name, schemas),
            })
            .collect()
    }

    /// Human-readable phrase to embed in diagnostics.
    pub fn display_name(&self) -> String {
        match self {
            Self::Declared(s) => format!("schema '{}'", s.name()),
            Self::Derived(fields) => {
                let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
                format!("inferred schema [{}]", names.join(", "))
            }
            Self::Grouped { keys, .. } => {
                format!("grouped data with keys [{}]", keys.join(", "))
            }
        }
    }
}

/// Outcome of resolving a possibly-dotted column path against a schema.
///
/// `Resolved` means every segment matched. `Missing { field, on }` means
/// `field` is the segment that didn't match; `on` is the `SchemaView`
/// being searched (`Some` for a top-level miss — used for the message
/// and the "did you mean" suggestion), or `None` for a miss inside a
/// nested `struct`, where there's no `SchemaView` to point at.
#[derive(Debug)]
pub enum FieldPathResult<'a> {
    Resolved,
    Missing {
        field: &'a str,
        on: Option<SchemaView<'a>>,
    },
}

/// Walk a possibly-dotted column path through a schema and its nested
/// types — `struct` fields, and the element type of an `array` (a dotted
/// access pierces the array, as in Spark: `events.id` where
/// `events: array<struct<id:int>>`).
///
/// `Resolved` if every segment matched. `Missing` names the failed
/// segment. A path dathon can't verify — through an unknown type, or an
/// `array` of unknown element / a `map` — degrades to `Resolved` rather
/// than risking a false positive.
pub fn resolve_path<'a>(
    view: &SchemaView<'a>,
    path: &'a str,
    schemas: &'a [Schema<'a>],
) -> FieldPathResult<'a> {
    let segments: Vec<&str> = path.split('.').collect();
    let mut current = view.clone();

    for (i, &segment) in segments.iter().enumerate() {
        let is_last = i + 1 == segments.len();
        if !current.has_field(segment) {
            return FieldPathResult::Missing {
                field: segment,
                on: Some(current),
            };
        }
        if is_last {
            return FieldPathResult::Resolved;
        }
        // Prefer descending as a bare-name nested `Schema` — that keeps a
        // named schema to point at in any later diagnostic.
        let nested = match &current {
            SchemaView::Declared(s) => s
                .fields()
                .iter()
                .find(|f| f.name == segment)
                .and_then(|f| match f.resolve(schemas) {
                    FieldResolution::ResolvedNested(nested) => Some(nested),
                    _ => None,
                }),
            _ => None,
        };
        if let Some(nested) = nested {
            current = SchemaView::Declared(nested);
            continue;
        }
        // Otherwise descend through the field's resolved `ColumnType` —
        // `array<struct<…>>`, a string-form `struct<…>`, etc.
        return match current.field_type(segment, schemas) {
            // Descending past an atomic column is a genuine mistake —
            // point the diagnostic at that column.
            Some(ty) if !ty.is_composite() => FieldPathResult::Missing {
                field: segment,
                on: Some(current),
            },
            Some(ty) => resolve_in_type(&ty, &segments[i + 1..]),
            // The field exists but its type is unknown — can't verify the
            // rest of the path; degrade rather than false-flag.
            None => FieldPathResult::Resolved,
        };
    }
    FieldPathResult::Resolved
}

/// Walk the remaining dotted segments through a resolved composite
/// [`ColumnType`].
fn resolve_in_type<'a>(ty: &ColumnType, segments: &[&'a str]) -> FieldPathResult<'a> {
    let Some((&seg, rest)) = segments.split_first() else {
        return FieldPathResult::Resolved;
    };
    match ty {
        // A dotted access on an array pierces to the element type.
        ColumnType::Array(Some(elem)) => resolve_in_type(elem, segments),
        ColumnType::Struct(fields) => match fields.iter().find(|f| f.name == seg) {
            None => FieldPathResult::Missing {
                field: seg,
                on: None,
            },
            Some(_) if rest.is_empty() => FieldPathResult::Resolved,
            Some(field) => match &field.ty {
                Some(inner) if inner.is_composite() => resolve_in_type(inner, rest),
                // The path continues past an atomic struct field.
                Some(_) => FieldPathResult::Missing {
                    field: seg,
                    on: None,
                },
                // An untyped struct field — can't verify further.
                None => FieldPathResult::Resolved,
            },
        },
        // A nullable column navigates as its underlying type.
        ColumnType::Nullable(inner) => resolve_in_type(inner, segments),
        // An array of unknown element, or a map — can't navigate further;
        // degrade rather than false-flag.
        ColumnType::Array(None) | ColumnType::Map(..) => FieldPathResult::Resolved,
        // An atomic column — a dotted access on it is a genuine mistake.
        _ => FieldPathResult::Missing {
            field: seg,
            on: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Pick / Omit / Merge — derived-schema type operators
// ---------------------------------------------------------------------------

/// A derived-schema operator inside a `DataFrame[…]` annotation:
/// `Pick[S, "a", …]`, `Omit[S, "a", …]`, or `Merge[A, B, …]`.
#[derive(Clone, Copy, PartialEq)]
enum DerivedOp {
    /// `Pick[S, "a", "b"]` — keep only the named columns.
    Pick,
    /// `Omit[S, "a", "b"]` — keep every column except the named ones.
    Omit,
    /// `Merge[A, B, …]` — every column of two or more schemas combined.
    Merge,
}

/// The operator and raw subscript arguments of a derived-schema
/// expression. The arguments are the tuple elements — or the single
/// element for a one-argument subscript. `None` if `expr` isn't a
/// `Pick` / `Omit` / `Merge` subscript.
fn derived_parts(expr: &Expr) -> Option<(DerivedOp, &[Expr])> {
    let sub = expr.as_subscript_expr()?;
    let op = match sub.value.as_name_expr()?.id.as_str() {
        "Pick" => DerivedOp::Pick,
        "Omit" => DerivedOp::Omit,
        "Merge" => DerivedOp::Merge,
        _ => return None,
    };
    let args: &[Expr] = match sub.slice.as_ref() {
        Expr::Tuple(tuple) => &tuple.elts,
        single => std::slice::from_ref(single),
    };
    Some((op, args))
}

/// The column-name string of an inline-schema dict key — a bare name
/// (`{checkin: date}`) or a string literal (`{"checkin": date}`).
fn dict_key_name(key: Option<&Expr>) -> Option<&str> {
    match key? {
        Expr::Name(n) => Some(n.id.as_str()),
        Expr::StringLiteral(s) => Some(s.value.to_str()),
        _ => None,
    }
}

/// Resolve a derived-schema expression to a `SchemaView::Derived`: a
/// `Pick` / `Omit` / `Merge` subscript, or an inline structural schema
/// `{col: type, …}`.
///
/// `Pick` keeps the named columns in the order listed; `Omit` keeps every
/// other column in the base schema's order; `Merge` concatenates the
/// columns of every named schema, de-duplicated by name (first occurrence
/// wins); an inline dict becomes a column per `name: type` pair. Columns
/// / schemas / types that don't resolve are skipped or left untyped —
/// [`derived_schema_errors`] reports them. `None` if `expr` isn't a
/// derived-schema expression, or nothing in it resolved.
pub fn resolve_derived_schema<'a>(
    expr: &'a Expr,
    schemas: &'a [Schema<'a>],
) -> Option<SchemaView<'a>> {
    // `DataFrame[{col: type, …}]` — an inline structural schema. Each
    // `name: type` pair becomes a column; an unresolvable type is left
    // `None` (permissive) — `derived_schema_errors` flags it.
    if let Some(dict) = expr.as_dict_expr() {
        let fields = dict
            .items
            .iter()
            .filter_map(|item| {
                Some(DerivedField {
                    name: dict_key_name(item.key.as_ref())?,
                    ty: resolve_annotation_type(&item.value, schemas, 0),
                })
            })
            .collect();
        return Some(SchemaView::Derived(fields));
    }
    let (op, args) = derived_parts(expr)?;
    match op {
        DerivedOp::Pick | DerivedOp::Omit => {
            let (base, col_exprs) = args.split_first()?;
            let base_name = base.as_name_expr()?.id.as_str();
            let schema = schemas.iter().find(|s| s.name() == base_name)?;
            let cols: Vec<&str> = col_exprs
                .iter()
                .filter_map(|e| e.as_string_literal_expr().map(|s| s.value.to_str()))
                .collect();
            if cols.is_empty() {
                return None;
            }
            let all = SchemaView::Declared(schema).typed_fields(schemas);
            let fields: Vec<DerivedField<'a>> = if op == DerivedOp::Pick {
                cols.iter()
                    .filter_map(|c| all.iter().find(|f| f.name == *c).cloned())
                    .collect()
            } else {
                all.into_iter().filter(|f| !cols.contains(&f.name)).collect()
            };
            Some(SchemaView::Derived(fields))
        }
        DerivedOp::Merge => {
            let mut fields: Vec<DerivedField<'a>> = Vec::new();
            let mut resolved_any = false;
            for arg in args {
                let Some(name) = arg.as_name_expr() else {
                    continue;
                };
                let Some(schema) = schemas.iter().find(|s| s.name() == name.id.as_str()) else {
                    continue;
                };
                resolved_any = true;
                for field in SchemaView::Declared(schema).typed_fields(schemas) {
                    if !fields.iter().any(|f| f.name == field.name) {
                        fields.push(field);
                    }
                }
            }
            resolved_any.then_some(SchemaView::Derived(fields))
        }
    }
}

/// Validation errors for a derived-schema expression — each a
/// `(code, message, range)` triple ready to become a `Diagnostic`: a
/// `Pick` / `Omit` column not on its base schema (`D0030`), an unknown
/// base or merged schema (`D0020`), and an unresolvable type in an
/// inline structural schema (`D0010` / `D0011`). Empty if `expr` isn't
/// a derived-schema expression or is error-free.
pub fn derived_schema_errors<'a>(
    expr: &'a Expr,
    schemas: &'a [Schema<'a>],
    source: &str,
) -> Vec<(&'static str, String, TextRange)> {
    let mut errors: Vec<(&'static str, String, TextRange)> = Vec::new();
    // `DataFrame[{col: type, …}]` — every value must resolve to a type;
    // reuse `SchemaField::resolve` so the messages match a class field's.
    if let Some(dict) = expr.as_dict_expr() {
        for item in &dict.items {
            let Some(name) = dict_key_name(item.key.as_ref()) else {
                continue;
            };
            let field = SchemaField {
                name,
                annotation: &item.value,
            };
            match field.resolve(schemas) {
                FieldResolution::Resolved(_) | FieldResolution::ResolvedNested(_) => {}
                FieldResolution::UnknownType { name: ty } => errors.push((
                    "D0010",
                    format!("Unknown column type '{ty}'. Expected one of: {COLUMN_TYPE_NAMES}."),
                    item.value.range(),
                )),
                FieldResolution::NotABareName => errors.push((
                    "D0011",
                    format!(
                        "Column type '{}' is not a recognized dathon type. \
                         Use a bare atomic ({COLUMN_TYPE_NAMES}), a declared Schema, \
                         or an `Array[…]` / `Map[…, …]` collection.",
                        &source[item.value.range()],
                    ),
                    item.value.range(),
                )),
            }
        }
        return errors;
    }
    let Some((op, args)) = derived_parts(expr) else {
        return errors;
    };
    let unknown_schema = |name: &str, range: TextRange| {
        (
            "D0020",
            format!("Unknown schema '{name}'. Declare it as a class extending Schema."),
            range,
        )
    };
    match op {
        DerivedOp::Pick | DerivedOp::Omit => {
            let Some((base, col_exprs)) = args.split_first() else {
                return errors;
            };
            let Some(base_name) = base.as_name_expr().map(|n| n.id.as_str()) else {
                return errors;
            };
            match schemas.iter().find(|s| s.name() == base_name) {
                None => errors.push(unknown_schema(base_name, base.range())),
                Some(schema) => {
                    for e in col_exprs {
                        if let Some(lit) = e.as_string_literal_expr() {
                            let col = lit.value.to_str();
                            if !schema.has_field(col) {
                                errors.push((
                                    "D0030",
                                    format!(
                                        "Column '{col}' does not exist on schema '{base_name}'."
                                    ),
                                    lit.range(),
                                ));
                            }
                        }
                    }
                }
            }
        }
        DerivedOp::Merge => {
            for arg in args {
                if let Some(n) = arg.as_name_expr() {
                    let name = n.id.as_str();
                    if !schemas.iter().any(|s| s.name() == name) {
                        errors.push(unknown_schema(name, arg.range()));
                    }
                }
            }
        }
    }
    errors
}

// ---------------------------------------------------------------------------
// "Did you mean?" suggestions
// ---------------------------------------------------------------------------

/// Find the closest field name on `view` to `target` by Levenshtein
/// distance. Returns `None` if no candidate is within edit-distance
/// threshold — the threshold is `max(1, target.len() / 3)` so a single
/// typo on a 3-character field qualifies and two typos on a 6-character
/// one does, but completely unrelated names don't.
///
/// Powers the "Did you mean 'X'?" hint on `D0030` diagnostics and the
/// `textDocument/codeAction` quick-fix that replaces the bad literal.
pub fn suggest_field_name(target: &str, view: &SchemaView<'_>) -> Option<String> {
    let candidates = view.field_names();
    let threshold = std::cmp::max(1, target.len() / 3);
    let mut best: Option<(&str, usize)> = None;
    for candidate in candidates {
        let d = levenshtein(target, candidate);
        if d > threshold {
            continue;
        }
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((candidate, d));
        }
    }
    best.map(|(name, _)| name.to_string())
}

/// Plain Levenshtein distance between `a` and `b`. Used by
/// [`suggest_field_name`] to rank column candidates.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let n = a_chars.len();
    let m = b_chars.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
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
        let view = SchemaView::derived_untyped(vec!["a", "b", "c"]);
        assert!(view.has_field("a"));
        assert!(view.has_field("b"));
        assert!(view.has_field("c"));
    }

    #[test]
    fn derived_field_type_returns_the_inferred_type() {
        // A Derived view carries a type per column; `field_type` reads
        // it back. Unknown columns and un-inferred fields give `None`.
        let view = SchemaView::Derived(vec![
            DerivedField {
                name: "amount",
                ty: Some(ColumnType::Int),
            },
            DerivedField {
                name: "label",
                ty: None,
            },
        ]);
        assert_eq!(view.field_type("amount", &[]), Some(ColumnType::Int));
        assert_eq!(view.field_type("label", &[]), None);
        assert_eq!(view.field_type("missing", &[]), None);
    }

    #[test]
    fn derived_untyped_leaves_every_field_type_unknown() {
        let view = SchemaView::derived_untyped(vec!["a", "b"]);
        assert_eq!(view.field_type("a", &[]), None);
        assert_eq!(view.field_type("b", &[]), None);
    }

    #[test]
    fn levenshtein_handles_empty_and_simple_edits() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("price", "priec"), 2);
        assert_eq!(levenshtein("price", "prce"), 1);
    }

    #[test]
    fn suggest_field_name_returns_closest_within_threshold() {
        let view = SchemaView::derived_untyped(vec!["price", "place_code", "quantity"]);
        // priec → price (transposition, distance 2 ≤ floor(5/3)=1 fails;
        // but ceil-style max(1, 5/3) gives 1, so 2 is over threshold).
        // Use a one-character typo so it's clearly within threshold.
        assert_eq!(suggest_field_name("prce", &view), Some("price".to_string()));
    }

    #[test]
    fn suggest_field_name_returns_none_when_nothing_is_close() {
        let view = SchemaView::derived_untyped(vec!["price", "place_code"]);
        assert_eq!(suggest_field_name("totally_unrelated_name", &view), None);
    }

    #[test]
    fn derived_schema_has_field_returns_false_for_unknown_columns() {
        let view = SchemaView::derived_untyped(vec!["a", "b"]);
        assert!(!view.has_field("c"));
        assert!(!view.has_field(""));
    }

    #[test]
    fn derived_schema_has_field_is_case_sensitive() {
        // Column names are matched exactly. Spark itself defaults to
        // case-insensitive matching, but dathon is stricter; this keeps
        // typos like 'PRICE' vs 'price' detectable.
        let view = SchemaView::derived_untyped(vec!["price"]);
        assert!(view.has_field("price"));
        assert!(!view.has_field("Price"));
        assert!(!view.has_field("PRICE"));
    }

    #[test]
    fn derived_schema_field_names_preserves_order() {
        // Column order matters for `union` (vs `unionByName`); preserve the
        // insertion order from the operation that produced this schema.
        let view = SchemaView::derived_untyped(vec!["x", "y", "z"]);
        assert_eq!(view.field_names(), vec!["x", "y", "z"]);
    }

    #[test]
    fn derived_display_name_lists_fields_in_brackets() {
        // The format embedded in D0030 / D0040 messages when the schema
        // doesn't have a user-facing name.
        let view = SchemaView::derived_untyped(vec!["a", "b"]);
        assert_eq!(view.display_name(), "inferred schema [a, b]");
    }

    #[test]
    fn derived_display_name_handles_empty_field_list() {
        // Can happen after `select` with all aliasless complex expressions.
        let view = SchemaView::derived_untyped(vec![]);
        assert_eq!(view.display_name(), "inferred schema []");
    }

    #[test]
    fn grouped_schema_has_field_returns_false_for_all_names() {
        // GroupedData isn't field-queryable; only .agg(...) produces a real
        // DataFrame. has_field uniformly returns false so accidental
        // non-agg operations on a Grouped receiver get caught.
        let underlying = SchemaView::derived_untyped(vec!["k", "a", "b"]);
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
        let underlying = SchemaView::derived_untyped(vec!["k1", "k2", "v"]);
        let grouped = SchemaView::Grouped {
            keys: vec!["k1", "k2"],
            underlying: Box::new(underlying),
        };
        assert_eq!(grouped.field_names(), vec!["k1", "k2"]);
    }

    #[test]
    fn grouped_display_name_describes_the_keys() {
        let underlying = SchemaView::derived_untyped(vec!["k", "v"]);
        let grouped = SchemaView::Grouped {
            keys: vec!["k"],
            underlying: Box::new(underlying),
        };
        assert_eq!(grouped.display_name(), "grouped data with keys [k]");
    }
}
