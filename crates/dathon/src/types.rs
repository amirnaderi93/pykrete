//! Column types — the atoms of dathon's type system.
//!
//! For v0.1 we support a fixed vocabulary of Spark-aligned atomic types.
//! Subscripted forms (`list[str]`, `Optional[int]`) and nested schemas are
//! deferred to later iterations.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    Int,
    Long,
    Double,
    String,
    Bool,
    Date,
    Timestamp,
    /// A Spark `ArrayType` column. The element type isn't tracked — v1
    /// distinguishes collection-vs-atomic, not `array<int>` vs
    /// `array<string>`.
    Array,
    /// A Spark `MapType` column. Key/value types aren't tracked.
    Map,
}

impl ColumnType {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "int" => Some(Self::Int),
            "long" => Some(Self::Long),
            "double" => Some(Self::Double),
            "string" => Some(Self::String),
            "bool" => Some(Self::Bool),
            "date" => Some(Self::Date),
            "timestamp" => Some(Self::Timestamp),
            "array" => Some(Self::Array),
            "map" => Some(Self::Map),
            _ => {
                // `array<…>` / `map<…>` — the element and key/value types
                // aren't modeled in v1, only the collection kind.
                match name.split_once('<').map(|(base, _)| base.trim_end())? {
                    "array" => Some(Self::Array),
                    "map" => Some(Self::Map),
                    _ => None,
                }
            }
        }
    }

    /// Map a Spark SQL type name to a [`ColumnType`]. Spark's vocabulary
    /// is wider than dathon's annotation vocabulary — it has `bigint`,
    /// `integer`, `float`, … — so this is its own mapping, used for
    /// `.cast("…")` targets and string-form UDF return types.
    pub fn from_spark_name(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        // `array<…>` / `map<…>` — match on the collection kind.
        if name.starts_with("array") {
            return Some(Self::Array);
        }
        if name.starts_with("map") {
            return Some(Self::Map);
        }
        match name.as_str() {
            "int" | "integer" => Some(Self::Int),
            "long" | "bigint" => Some(Self::Long),
            "double" | "float" | "real" => Some(Self::Double),
            "string" => Some(Self::String),
            "boolean" | "bool" => Some(Self::Bool),
            "date" => Some(Self::Date),
            "timestamp" => Some(Self::Timestamp),
            _ => None,
        }
    }

    /// Map a Spark type-object constructor name (`IntegerType`,
    /// `StringType`, …) to a [`ColumnType`]. Used where a Spark type is
    /// written as `IntegerType()` rather than the string `"int"`.
    pub fn from_type_constructor(name: &str) -> Option<Self> {
        match name {
            "IntegerType" => Some(Self::Int),
            "LongType" => Some(Self::Long),
            "DoubleType" | "FloatType" => Some(Self::Double),
            "StringType" => Some(Self::String),
            "BooleanType" => Some(Self::Bool),
            "DateType" => Some(Self::Date),
            "TimestampType" => Some(Self::Timestamp),
            "ArrayType" => Some(Self::Array),
            "MapType" => Some(Self::Map),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Int => "Int",
            Self::Long => "Long",
            Self::Double => "Double",
            Self::String => "String",
            Self::Bool => "Bool",
            Self::Date => "Date",
            Self::Timestamp => "Timestamp",
            Self::Array => "Array",
            Self::Map => "Map",
        }
    }
}

impl fmt::Display for ColumnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Comma-separated list of the source-form names users can write in a
/// `.dpy` file. Used inside error messages.
pub const COLUMN_TYPE_NAMES: &str =
    "int, long, double, string, bool, date, timestamp, array, map";

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
    "array",
    "map",
];

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_name_recognizes_all_v0_1_atomic_types() {
        // Every name in the v0.1 vocabulary should resolve.
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
    fn from_name_recognizes_array_and_map_collections() {
        // Bare and parameterized forms both resolve; the element /
        // key-value types inside `<…>` are not modeled in v1.
        assert_eq!(ColumnType::from_name("array"), Some(ColumnType::Array));
        assert_eq!(ColumnType::from_name("map"), Some(ColumnType::Map));
        assert_eq!(
            ColumnType::from_name("array<string>"),
            Some(ColumnType::Array)
        );
        assert_eq!(
            ColumnType::from_name("map<string, int>"),
            Some(ColumnType::Map)
        );
        // A parameterized atomic is still nonsense.
        assert_eq!(ColumnType::from_name("int<x>"), None);
    }

    #[test]
    fn from_name_rejects_unknown_types() {
        // Anything outside the vocabulary is None — caller treats it as an error.
        assert_eq!(ColumnType::from_name("WeirdType"), None);
        assert_eq!(ColumnType::from_name(""), None);
        // Case-sensitive: capitalised forms are rejected (we only accept lowercase).
        assert_eq!(ColumnType::from_name("Int"), None);
        assert_eq!(ColumnType::from_name("INT"), None);
        // Spark distinguishes Float (32-bit) and Double (64-bit); we only
        // accept `double` in v0.1, not `float`.
        assert_eq!(ColumnType::from_name("float"), None);
    }

    #[test]
    fn as_str_produces_canonical_capitalized_names() {
        // What gets rendered in body output and in diagnostic messages.
        assert_eq!(ColumnType::Int.as_str(), "Int");
        assert_eq!(ColumnType::Long.as_str(), "Long");
        assert_eq!(ColumnType::Double.as_str(), "Double");
        assert_eq!(ColumnType::String.as_str(), "String");
        assert_eq!(ColumnType::Bool.as_str(), "Bool");
        assert_eq!(ColumnType::Date.as_str(), "Date");
        assert_eq!(ColumnType::Timestamp.as_str(), "Timestamp");
    }

    #[test]
    fn display_matches_as_str() {
        // The `Display` impl just goes through `as_str` — sanity-check that.
        assert_eq!(format!("{}", ColumnType::Int), "Int");
        assert_eq!(format!("{}", ColumnType::Timestamp), "Timestamp");
    }

    #[test]
    fn column_type_names_constant_lists_every_atomic_type() {
        // The constant embedded in user-facing error messages. Keep it in
        // sync with from_name. If you add a new type, update this and the
        // constant.
        for name in [
            "int",
            "long",
            "double",
            "string",
            "bool",
            "date",
            "timestamp",
        ] {
            assert!(
                COLUMN_TYPE_NAMES.contains(name),
                "COLUMN_TYPE_NAMES should list '{name}'",
            );
        }
    }
}
