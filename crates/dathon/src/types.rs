//! Column types — dathon's type system.
//!
//! The vocabulary is the Spark-aligned atomic types plus the composite
//! `array<T>` and `map<K, V>`, which nest arbitrarily — `array<int>`,
//! `map<string, array<int>>`, and so on. Struct element types compose
//! in too (see [`crate::schema`]).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnType {
    Int,
    Long,
    Double,
    String,
    Bool,
    Date,
    Timestamp,
    /// A Spark `ArrayType`. The element type is carried when known;
    /// `None` means the element type couldn't be determined.
    Array(Option<Box<ColumnType>>),
    /// A Spark `MapType`. Key and value types are carried when known.
    Map(Option<Box<ColumnType>>, Option<Box<ColumnType>>),
    /// A Spark `StructType` — an ordered list of named, typed fields.
    /// Covers both an inline `struct<…>` and a reference to a declared
    /// `Schema` class. Compared structurally (field order matters, as
    /// in Spark).
    Struct(Vec<StructField>),
    /// A nullable column — written `Optional[T]`. Wraps the underlying
    /// type; mirrors Spark's per-column `nullable` flag. Nullability is
    /// transparent to the conservative checks (`Nullable(T)` behaves as
    /// `T`); the strict mode flags a nullable value declared non-null.
    Nullable(Box<ColumnType>),
}

/// One field of a [`ColumnType::Struct`] — its name and type. `ty` is
/// `None` when the field's type couldn't be resolved (permissive, like
/// any unknown type).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructField {
    pub name: String,
    pub ty: Option<ColumnType>,
}

impl ColumnType {
    /// Parse a dathon type annotation — an atomic name, or a nested
    /// `array<…>` / `map<…>`. Atomic names are the strict dathon
    /// vocabulary (`int`, `long`, …); `array` / `map` keywords are
    /// case-insensitive.
    pub fn from_name(name: &str) -> Option<Self> {
        parse_type_expr(name, &atomic_strict)
    }

    /// Parse a Spark SQL type name — like [`from_name`](Self::from_name)
    /// but with Spark's wider atomic vocabulary (`bigint`, `integer`,
    /// `float`, …). Used for `.cast("…")` targets and string-form UDF
    /// return types.
    pub fn from_spark_name(name: &str) -> Option<Self> {
        parse_type_expr(name, &atomic_lenient)
    }

    /// Map a Spark type-object constructor name (`IntegerType`,
    /// `ArrayType`, …) to a [`ColumnType`]. Element types of an
    /// `ArrayType(…)` / `MapType(…)` aren't recovered here (the name
    /// alone doesn't carry them) — the caller fills those in from the
    /// constructor's arguments.
    pub fn from_type_constructor(name: &str) -> Option<Self> {
        match name {
            "IntegerType" => Some(Self::Int),
            "LongType" => Some(Self::Long),
            "DoubleType" | "FloatType" => Some(Self::Double),
            "StringType" => Some(Self::String),
            "BooleanType" => Some(Self::Bool),
            "DateType" => Some(Self::Date),
            "TimestampType" => Some(Self::Timestamp),
            "ArrayType" => Some(Self::Array(None)),
            "MapType" => Some(Self::Map(None, None)),
            _ => None,
        }
    }

    /// Whether this is a composite type — `array`, `map`, or `struct` —
    /// as opposed to an atomic (`int`, `string`, …). A `Nullable`
    /// wrapper is transparent: `Optional[Array[int]]` is still composite.
    pub fn is_composite(&self) -> bool {
        match self {
            Self::Array(_) | Self::Map(..) | Self::Struct(_) => true,
            Self::Nullable(inner) => inner.is_composite(),
            _ => false,
        }
    }

    /// True if this is a `Nullable(…)` — an `Optional[T]` column.
    pub fn is_nullable(&self) -> bool {
        matches!(self, Self::Nullable(_))
    }

    /// The underlying type with any `Nullable` wrapper peeled off.
    pub fn base(&self) -> &ColumnType {
        match self {
            Self::Nullable(inner) => inner.base(),
            other => other,
        }
    }

    /// The bare kind name — the atomic name, or `array` / `map` without
    /// their element types. For the full nested rendering use `Display`.
    /// A `Nullable` wrapper is peeled — it isn't itself a "kind".
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Int => "Int",
            Self::Long => "Long",
            Self::Double => "Double",
            Self::String => "String",
            Self::Bool => "Bool",
            Self::Date => "Date",
            Self::Timestamp => "Timestamp",
            Self::Array(_) => "array",
            Self::Map(..) => "map",
            Self::Struct(_) => "struct",
            Self::Nullable(inner) => inner.as_str(),
        }
    }
}

/// Parse a complete type string — the whole input must be consumed.
///
/// `leaf` resolves a leaf identifier (one that isn't an `array` / `map`
/// / `struct` keyword). [`from_name`](ColumnType::from_name) passes an
/// atomic-only resolver; [`crate::schema`] passes one that also resolves
/// declared `Schema` class names to a [`ColumnType::Struct`].
pub(crate) fn parse_type_expr<F: Fn(&str) -> Option<ColumnType>>(
    s: &str,
    leaf: &F,
) -> Option<ColumnType> {
    let (ty, rest) = parse_type(s.trim(), leaf)?;
    rest.trim().is_empty().then_some(ty)
}

/// Parse one type expression off the front of `s`, returning it and the
/// unconsumed remainder. Recursive for `array<…>` / `map<…>` /
/// `struct<…>`.
fn parse_type<'s, F: Fn(&str) -> Option<ColumnType>>(
    s: &'s str,
    leaf: &F,
) -> Option<(ColumnType, &'s str)> {
    let s = s.trim_start();
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let word = &s[..end];
    let after = s[end..].trim_start();
    match word.to_ascii_lowercase().as_str() {
        "array" => {
            let Some(inner) = after.strip_prefix('<') else {
                return Some((ColumnType::Array(None), after));
            };
            let (elem, rest) = parse_type(inner, leaf)?;
            let rest = rest.trim_start().strip_prefix('>')?;
            Some((ColumnType::Array(Some(Box::new(elem))), rest))
        }
        "map" => {
            let Some(inner) = after.strip_prefix('<') else {
                return Some((ColumnType::Map(None, None), after));
            };
            let (key, rest) = parse_type(inner, leaf)?;
            let rest = rest.trim_start().strip_prefix(',')?;
            let (value, rest) = parse_type(rest, leaf)?;
            let rest = rest.trim_start().strip_prefix('>')?;
            Some((
                ColumnType::Map(Some(Box::new(key)), Some(Box::new(value))),
                rest,
            ))
        }
        "struct" => {
            let inner = after.strip_prefix('<')?;
            parse_struct_fields(inner, leaf)
        }
        _ => Some((leaf(word)?, after)),
    }
}

/// Parse a `struct<…>` field list — `name: type`, comma-separated — off
/// the front of `s` (positioned just after the opening `<`), up to the
/// closing `>`.
fn parse_struct_fields<'s, F: Fn(&str) -> Option<ColumnType>>(
    s: &'s str,
    leaf: &F,
) -> Option<(ColumnType, &'s str)> {
    let mut fields: Vec<StructField> = Vec::new();
    let mut rest = s.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix('>') {
            return Some((ColumnType::Struct(fields), after));
        }
        // `name : type`
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        let name = rest[..end].to_string();
        rest = rest[end..].trim_start().strip_prefix(':')?;
        let (ty, after) = parse_type(rest, leaf)?;
        fields.push(StructField { name, ty: Some(ty) });
        rest = after.trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after.trim_start();
        }
    }
}

/// The strict dathon atomic vocabulary — case-sensitive lowercase.
fn atomic_strict(name: &str) -> Option<ColumnType> {
    match name {
        "int" => Some(ColumnType::Int),
        "long" => Some(ColumnType::Long),
        "double" => Some(ColumnType::Double),
        "string" => Some(ColumnType::String),
        "bool" => Some(ColumnType::Bool),
        "date" => Some(ColumnType::Date),
        "timestamp" => Some(ColumnType::Timestamp),
        _ => None,
    }
}

/// Spark's wider atomic vocabulary, case-insensitive.
fn atomic_lenient(name: &str) -> Option<ColumnType> {
    match name.to_ascii_lowercase().as_str() {
        "int" | "integer" => Some(ColumnType::Int),
        "long" | "bigint" => Some(ColumnType::Long),
        "double" | "float" | "real" => Some(ColumnType::Double),
        "string" => Some(ColumnType::String),
        "boolean" | "bool" => Some(ColumnType::Bool),
        "date" => Some(ColumnType::Date),
        "timestamp" => Some(ColumnType::Timestamp),
        _ => None,
    }
}

impl fmt::Display for ColumnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Array(Some(elem)) => write!(f, "array<{elem}>"),
            Self::Array(None) => f.write_str("array"),
            Self::Map(Some(key), Some(value)) => write!(f, "map<{key}, {value}>"),
            Self::Map(..) => f.write_str("map"),
            Self::Struct(fields) => {
                f.write_str("struct<")?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}: ", field.name)?;
                    match &field.ty {
                        Some(ty) => write!(f, "{ty}")?,
                        None => f.write_str("?")?,
                    }
                }
                f.write_str(">")
            }
            Self::Nullable(inner) => write!(f, "{inner}?"),
            atomic => f.write_str(atomic.as_str()),
        }
    }
}

/// Comma-separated list of the source-form names users can write in a
/// `.dpy` file. Used inside error messages.
pub const COLUMN_TYPE_NAMES: &str =
    "int, long, double, string, bool, date, timestamp, Array, Map";

/// Same vocabulary as [`COLUMN_TYPE_NAMES`] but as a slice — fed to the
/// completion engine when the cursor sits inside a `name: "<cursor>"`
/// string-literal annotation in a Schema class body.
pub const COLUMN_TYPE_NAMES_LIST: &[&str] = &[
    "int",
    "long",
    "double",
    "string",
    "bool",
    "date",
    "timestamp",
    "Array",
    "Map",
];

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn array(elem: ColumnType) -> ColumnType {
        ColumnType::Array(Some(Box::new(elem)))
    }
    fn map(key: ColumnType, value: ColumnType) -> ColumnType {
        ColumnType::Map(Some(Box::new(key)), Some(Box::new(value)))
    }

    #[test]
    fn from_name_recognizes_all_v0_1_atomic_types() {
        assert_eq!(ColumnType::from_name("int"), Some(ColumnType::Int));
        assert_eq!(ColumnType::from_name("long"), Some(ColumnType::Long));
        assert_eq!(ColumnType::from_name("double"), Some(ColumnType::Double));
        assert_eq!(ColumnType::from_name("string"), Some(ColumnType::String));
        assert_eq!(ColumnType::from_name("bool"), Some(ColumnType::Bool));
        assert_eq!(ColumnType::from_name("date"), Some(ColumnType::Date));
        assert_eq!(
            ColumnType::from_name("timestamp"),
            Some(ColumnType::Timestamp)
        );
    }

    #[test]
    fn from_name_parses_nested_array_and_map_element_types() {
        assert_eq!(ColumnType::from_name("array<string>"), Some(array(ColumnType::String)));
        assert_eq!(
            ColumnType::from_name("map<string, int>"),
            Some(map(ColumnType::String, ColumnType::Int)),
        );
        // Arbitrary nesting.
        assert_eq!(
            ColumnType::from_name("array<map<string, array<int>>>"),
            Some(array(map(ColumnType::String, array(ColumnType::Int)))),
        );
        // Bare `array` / `map` — element types unknown.
        assert_eq!(ColumnType::from_name("array"), Some(ColumnType::Array(None)));
        assert_eq!(ColumnType::from_name("map"), Some(ColumnType::Map(None, None)));
    }

    #[test]
    fn from_name_rejects_malformed_and_unknown_types() {
        assert_eq!(ColumnType::from_name("WeirdType"), None);
        assert_eq!(ColumnType::from_name(""), None);
        // Atomic names are case-sensitive.
        assert_eq!(ColumnType::from_name("Int"), None);
        assert_eq!(ColumnType::from_name("float"), None);
        // A parameterized atomic, and unbalanced / malformed nesting.
        assert_eq!(ColumnType::from_name("int<x>"), None);
        assert_eq!(ColumnType::from_name("array<int"), None);
        assert_eq!(ColumnType::from_name("array<>"), None);
        assert_eq!(ColumnType::from_name("map<string>"), None);
    }

    #[test]
    fn from_spark_name_accepts_the_wider_vocabulary_and_nesting() {
        assert_eq!(ColumnType::from_spark_name("bigint"), Some(ColumnType::Long));
        assert_eq!(
            ColumnType::from_spark_name("array<integer>"),
            Some(array(ColumnType::Int)),
        );
    }

    #[test]
    fn display_renders_nested_types() {
        assert_eq!(format!("{}", ColumnType::Int), "Int");
        assert_eq!(format!("{}", array(ColumnType::String)), "array<String>");
        assert_eq!(
            format!("{}", map(ColumnType::String, array(ColumnType::Int))),
            "map<String, array<Int>>",
        );
    }

    #[test]
    fn column_type_names_constant_lists_every_type_name() {
        for name in [
            "int", "long", "double", "string", "bool", "date", "timestamp", "Array", "Map",
        ] {
            assert!(
                COLUMN_TYPE_NAMES.contains(name),
                "COLUMN_TYPE_NAMES should list '{name}'",
            );
        }
    }
}
