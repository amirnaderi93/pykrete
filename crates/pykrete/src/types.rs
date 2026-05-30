//! Column types — pykrete's type system.
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
    /// 8-bit signed integer — Spark `ByteType`.
    Byte,
    /// 16-bit signed integer — Spark `ShortType`.
    Short,
    /// Fixed-precision decimal — Spark `DecimalType(precision, scale)`.
    /// The bare `decimal` keyword (in a schema annotation, cast target,
    /// or type constructor) resolves to Spark's default `decimal(10, 0)`
    /// — see [`Self::DEFAULT_DECIMAL`]. Precision-growth on aggregation
    /// is intentionally not modeled in v1.0 — `sum(decimal(p, s))`
    /// reduces to the default, matching the simpler-rules-than-Spark
    /// stance the checker takes elsewhere.
    Decimal {
        precision: u8,
        scale: u8,
    },
    String,
    /// Raw byte array — Spark `BinaryType`.
    Binary,
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
    /// Spark's default for the unparameterized `DECIMAL` SQL type. Pykrete
    /// uses this for the bare `decimal` keyword (no `(p, s)` args) and
    /// for aggregation outputs where precision-growth is collapsed.
    pub const DEFAULT_DECIMAL: Self = Self::Decimal {
        precision: 10,
        scale: 0,
    };

    /// Spark caps decimal precision at 38 (`Decimal128`'s upper bound).
    pub const MAX_DECIMAL_PRECISION: u8 = 38;

    /// Parse a pykrete type annotation — an atomic name, or a nested
    /// `array<…>` / `map<…>`. Atomic names are the strict pykrete
    /// vocabulary (`int`, `long`, …, `decimal`, `numeric`, `dec`) —
    /// case-sensitive lowercase. The `array` / `map` keywords are
    /// case-insensitive (legacy contract; written as `Array[…]` /
    /// `Map[…, …]` in Python annotations).
    pub fn from_name(name: &str) -> Option<Self> {
        parse_type_expr(name, &atomic_strict, Case::Strict)
    }

    /// Parse a Spark SQL type name — like [`from_name`](Self::from_name)
    /// but with Spark's wider atomic vocabulary (`bigint`, `integer`,
    /// `float`, …) and case-insensitive on every keyword (`DECIMAL`,
    /// `NUMERIC`, `Dec` all work; Spark SQL itself is case-insensitive
    /// on type names). Used for `.cast("…")` targets and string-form UDF
    /// return types.
    pub fn from_spark_name(name: &str) -> Option<Self> {
        parse_type_expr(name, &atomic_lenient, Case::Lenient)
    }

    /// True when `name` is a legitimate Spark type that pykrete doesn't
    /// model yet — `varchar(n)`, `char(n)`, `interval`, `timestamp_ntz`,
    /// `void`, `null`. The cast-typo check uses this to avoid false-
    /// rejecting code that worked before v0.1.28: a `.cast("varchar(100)")`
    /// or `.cast("timestamp_ntz")` is real Spark, just not something
    /// pykrete pins a type on. Returning `true` means "skip the D0011
    /// emission, fall through silently."
    ///
    /// Case-insensitive — Spark SQL itself doesn't distinguish
    /// `VARCHAR` from `varchar`. Garbage like `interval(5)` or
    /// `void(0)` is rejected because the underlying types take no
    /// parenthesised argument; `varchar(n)` / `char(n)` accept a
    /// single positive `n`.
    ///
    /// See `pyspark.sql.types` for the canonical list. Modeling these
    /// (VARCHAR/CHAR width, IntervalType, TimestampNTZType) is tracked
    /// for v1.1 polish — until then, recognizing-but-not-pinning is the
    /// honest stance.
    pub fn is_unmodeled_spark_type(name: &str) -> bool {
        let trimmed = name.trim();
        // Parameterised forms: only `varchar(n)` / `char(n)` are real,
        // with `n` a positive integer.
        if let Some(open) = trimmed.find('(') {
            let base = trimmed[..open].trim();
            if !matches!(base.to_ascii_lowercase().as_str(), "varchar" | "char") {
                return is_unmodeled_interval_compound(trimmed);
            }
            let Some(close) = trimmed.rfind(')') else {
                return false;
            };
            if close <= open {
                return false;
            }
            let inner = trimmed[open + 1..close].trim();
            let trailing = trimmed[close + 1..].trim();
            return trailing.is_empty()
                && inner.chars().all(|c| c.is_ascii_digit())
                && !inner.is_empty()
                && inner.parse::<u32>().map(|n| n > 0).unwrap_or(false);
        }
        // Bare forms — `varchar`, `char`, `interval`, `timestamp_ntz`,
        // `void`, `null`. The compound `interval … to …` shapes (with
        // or without precision args) are handled separately.
        matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "varchar" | "char" | "interval" | "timestamp_ntz" | "void" | "null"
        ) || is_unmodeled_interval_compound(trimmed)
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
            "ByteType" => Some(Self::Byte),
            "ShortType" => Some(Self::Short),
            "DecimalType" => Some(Self::DEFAULT_DECIMAL),
            "StringType" => Some(Self::String),
            "BinaryType" => Some(Self::Binary),
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
            Self::Byte => "Byte",
            Self::Short => "Short",
            Self::Decimal { .. } => "Decimal",
            Self::String => "String",
            Self::Binary => "Binary",
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

/// How the type parser handles keyword case. Strict matches lowercase
/// only (`decimal`, `numeric`, `dec`); lenient accepts any casing
/// (Spark SQL itself is case-insensitive on type names).
#[derive(Clone, Copy)]
pub(crate) enum Case {
    Strict,
    Lenient,
}

impl Case {
    fn matches(self, word: &str, target: &str) -> bool {
        match self {
            Self::Strict => word == target,
            Self::Lenient => word.eq_ignore_ascii_case(target),
        }
    }
}

/// Parse a complete type string — the whole input must be consumed.
///
/// `leaf` resolves a leaf identifier (one that isn't an `array` / `map`
/// / `struct` keyword). [`from_name`](ColumnType::from_name) passes an
/// atomic-only resolver; [`crate::schema`] passes one that also resolves
/// declared `Schema` class names to a [`ColumnType::Struct`]. `case`
/// controls keyword case-sensitivity — `Strict` matches lowercase only,
/// `Lenient` accepts any casing (Spark SQL convention).
pub(crate) fn parse_type_expr<F: Fn(&str) -> Option<ColumnType>>(
    s: &str,
    leaf: &F,
    case: Case,
) -> Option<ColumnType> {
    let (ty, rest) = parse_type(s.trim(), leaf, case)?;
    rest.trim().is_empty().then_some(ty)
}

/// Parse one type expression off the front of `s`, returning it and the
/// unconsumed remainder. Recursive for `array<…>` / `map<…>` /
/// `struct<…>`.
///
/// The `array` / `map` / `struct` keywords are always matched
/// case-insensitively (legacy contract — pykrete annotations write
/// them as `Array[…]` / `Map[…, …]`); `decimal` / `numeric` / `dec`
/// follow `case`.
fn parse_type<'s, F: Fn(&str) -> Option<ColumnType>>(
    s: &'s str,
    leaf: &F,
    case: Case,
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
    if ["decimal", "numeric", "dec"]
        .iter()
        .any(|kw| case.matches(word, kw))
    {
        // `numeric` and `dec` are Spark SQL aliases for `decimal` —
        // same parameterized form, same default. Route them through
        // the decimal arm so casts like `.cast("numeric(18, 2)")` and
        // `.cast("dec")` resolve identically.
        let Some(inner) = after.strip_prefix('(') else {
            return Some((ColumnType::DEFAULT_DECIMAL, after));
        };
        let (precision, rest) = parse_u8(inner)?;
        let rest = rest.trim_start();
        // Single-arg form `decimal(p)` defaults scale to 0 — matches
        // Spark SQL's `DECIMAL(p)` shorthand.
        let (scale, rest) = if let Some(after_comma) = rest.strip_prefix(',') {
            let (scale, rest) = parse_u8(after_comma)?;
            (scale, rest)
        } else {
            (0u8, rest)
        };
        let decimal = validate_decimal_args(precision, scale)?;
        let rest = rest.trim_start().strip_prefix(')')?;
        return Some((decimal, rest));
    }
    // `array` / `map` / `struct` are always case-insensitive — the
    // pykrete annotation surface writes them as `Array[…]` / `Map[…]`
    // and Spark SQL has its own casing conventions.
    if word.eq_ignore_ascii_case("array") {
        let Some(inner) = after.strip_prefix('<') else {
            return Some((ColumnType::Array(None), after));
        };
        let (elem, rest) = parse_type(inner, leaf, case)?;
        let rest = rest.trim_start().strip_prefix('>')?;
        return Some((ColumnType::Array(Some(Box::new(elem))), rest));
    }
    if word.eq_ignore_ascii_case("map") {
        let Some(inner) = after.strip_prefix('<') else {
            return Some((ColumnType::Map(None, None), after));
        };
        let (key, rest) = parse_type(inner, leaf, case)?;
        let rest = rest.trim_start().strip_prefix(',')?;
        let (value, rest) = parse_type(rest, leaf, case)?;
        let rest = rest.trim_start().strip_prefix('>')?;
        return Some((
            ColumnType::Map(Some(Box::new(key)), Some(Box::new(value))),
            rest,
        ));
    }
    if word.eq_ignore_ascii_case("struct") {
        let inner = after.strip_prefix('<')?;
        return parse_struct_fields(inner, leaf, case);
    }
    Some((leaf(word)?, after))
}

/// Bounds-check a `decimal(p, s)` argument pair against Spark's caps —
/// `1..=38` for precision, `scale <= precision`. Returns the validated
/// [`ColumnType::Decimal`] or `None` if either bound is violated.
/// Shared by the type-string parser ([`parse_type`]) and the schema
/// annotation parser ([`crate::schema::resolve_annotation_type`]) — one
/// source of truth keeps the rules from drifting apart.
pub(crate) fn validate_decimal_args(precision: u8, scale: u8) -> Option<ColumnType> {
    if precision == 0 || precision > ColumnType::MAX_DECIMAL_PRECISION || scale > precision {
        return None;
    }
    Some(ColumnType::Decimal { precision, scale })
}

/// Parse a `u8` integer off the front of `s` (after leading whitespace),
/// returning the value and the remainder. Used for `decimal(p, s)` —
/// Spark caps both at 38, so `u8` is plenty.
fn parse_u8(s: &str) -> Option<(u8, &str)> {
    let s = s.trim_start();
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let value = s[..end].parse().ok()?;
    Some((value, &s[end..]))
}

/// Parse a `struct<…>` field list — `name: type`, comma-separated — off
/// the front of `s` (positioned just after the opening `<`), up to the
/// closing `>`.
fn parse_struct_fields<'s, F: Fn(&str) -> Option<ColumnType>>(
    s: &'s str,
    leaf: &F,
    case: Case,
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
        let (ty, after) = parse_type(rest, leaf, case)?;
        fields.push(StructField { name, ty: Some(ty) });
        rest = after.trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after.trim_start();
        }
    }
}

/// Spark's `INTERVAL` types come in compound forms like
/// `interval year to month` and `interval day to second`, optionally
/// with precision args (`interval day(3) to second(6)`). This helper
/// recognises the small fixed set of `<unit> [to <unit>]` patterns
/// Spark accepts — case-insensitive, whitespace and precision args
/// stripped before comparison.
fn is_unmodeled_interval_compound(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    // Strip precision args like `day(3)` → `day` so the canonical-
    // pattern match below doesn't have to enumerate every digit combo.
    let mut stripped = String::with_capacity(lowered.len());
    let mut chars = lowered.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '(' {
            let mut digits = String::new();
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == ')' {
                    closed = true;
                    break;
                }
                digits.push(inner);
            }
            // A non-numeric or unclosed paren isn't a real Spark
            // interval precision arg — bail.
            if !closed
                || digits.trim().is_empty()
                || !digits.trim().chars().all(|c| c.is_ascii_digit())
            {
                return false;
            }
            continue;
        }
        stripped.push(c);
    }
    let normalized: String = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    matches!(
        normalized.as_str(),
        "interval year"
            | "interval year to month"
            | "interval month"
            | "interval day"
            | "interval day to hour"
            | "interval day to minute"
            | "interval day to second"
            | "interval hour"
            | "interval hour to minute"
            | "interval hour to second"
            | "interval minute"
            | "interval minute to second"
            | "interval second"
    )
}

/// The strict pykrete atomic vocabulary — case-sensitive lowercase.
fn atomic_strict(name: &str) -> Option<ColumnType> {
    match name {
        "int" => Some(ColumnType::Int),
        "long" => Some(ColumnType::Long),
        "double" => Some(ColumnType::Double),
        "byte" => Some(ColumnType::Byte),
        "short" => Some(ColumnType::Short),
        "string" => Some(ColumnType::String),
        "binary" => Some(ColumnType::Binary),
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
        "byte" | "tinyint" => Some(ColumnType::Byte),
        "short" | "smallint" => Some(ColumnType::Short),
        "string" => Some(ColumnType::String),
        "binary" => Some(ColumnType::Binary),
        "boolean" | "bool" => Some(ColumnType::Bool),
        "date" => Some(ColumnType::Date),
        "timestamp" => Some(ColumnType::Timestamp),
        _ => None,
    }
}

impl fmt::Display for ColumnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decimal { precision, scale } => write!(f, "Decimal({precision}, {scale})"),
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
/// `.pyk` file. Used inside error messages. `decimal` accepts an optional
/// `(precision, scale)` — `decimal` alone resolves to Spark's default
/// `decimal(10, 0)`.
pub const COLUMN_TYPE_NAMES: &str = "int, long, double, byte, short, decimal, decimal(p, s), \
    string, binary, bool, date, timestamp, Array, Map";

/// Same vocabulary as [`COLUMN_TYPE_NAMES`] but as a slice — fed to the
/// completion engine when the cursor sits inside a `name: "<cursor>"`
/// string-literal annotation in a Schema class body.
pub const COLUMN_TYPE_NAMES_LIST: &[&str] = &[
    "int",
    "long",
    "double",
    "byte",
    "short",
    "decimal",
    "decimal(p, s)",
    "string",
    "binary",
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
        assert_eq!(
            ColumnType::from_name("array<string>"),
            Some(array(ColumnType::String))
        );
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
        assert_eq!(
            ColumnType::from_name("array"),
            Some(ColumnType::Array(None))
        );
        assert_eq!(
            ColumnType::from_name("map"),
            Some(ColumnType::Map(None, None))
        );
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
        assert_eq!(
            ColumnType::from_spark_name("bigint"),
            Some(ColumnType::Long)
        );
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
            "int",
            "long",
            "double",
            "byte",
            "short",
            "decimal",
            // The parameterized form must be surfaced so users discover it
            // in error messages.
            "decimal(p, s)",
            "string",
            "binary",
            "bool",
            "date",
            "timestamp",
            "Array",
            "Map",
        ] {
            assert!(
                COLUMN_TYPE_NAMES.contains(name),
                "COLUMN_TYPE_NAMES should list '{name}'",
            );
            assert!(
                COLUMN_TYPE_NAMES_LIST.contains(&name),
                "COLUMN_TYPE_NAMES_LIST should list '{name}'",
            );
        }
    }

    #[test]
    fn from_name_recognizes_byte_short_decimal_binary() {
        assert_eq!(ColumnType::from_name("byte"), Some(ColumnType::Byte));
        assert_eq!(ColumnType::from_name("short"), Some(ColumnType::Short));
        assert_eq!(ColumnType::from_name("binary"), Some(ColumnType::Binary));
        // Bare `decimal` resolves to Spark's default `decimal(10, 0)`.
        assert_eq!(
            ColumnType::from_name("decimal"),
            Some(ColumnType::DEFAULT_DECIMAL)
        );
        assert_eq!(
            ColumnType::from_name("decimal(18, 2)"),
            Some(ColumnType::Decimal {
                precision: 18,
                scale: 2,
            })
        );
        // Whitespace inside the parens is tolerated.
        assert_eq!(
            ColumnType::from_name("decimal( 38 , 18 )"),
            Some(ColumnType::Decimal {
                precision: 38,
                scale: 18,
            })
        );
        // Single-arg form defaults scale to 0 (Spark's `DECIMAL(p)`).
        assert_eq!(
            ColumnType::from_name("decimal(18)"),
            Some(ColumnType::Decimal {
                precision: 18,
                scale: 0,
            })
        );
    }

    #[test]
    fn from_name_rejects_malformed_decimal() {
        // Garbage args, typo in keyword, unbalanced parens.
        assert_eq!(ColumnType::from_name("decimal(18,)"), None);
        assert_eq!(ColumnType::from_name("decimal(a, b)"), None);
        assert_eq!(ColumnType::from_name("decimal(18, 2"), None);
        assert_eq!(ColumnType::from_name("decimial(18, 2)"), None);
        // Precision must be in `1..=38` (Spark's `Decimal128` cap).
        assert_eq!(ColumnType::from_name("decimal(0, 0)"), None);
        assert_eq!(ColumnType::from_name("decimal(39, 0)"), None);
        // Scale must not exceed precision.
        assert_eq!(ColumnType::from_name("decimal(5, 6)"), None);
    }

    #[test]
    fn from_spark_name_accepts_byte_short_decimal_binary() {
        assert_eq!(ColumnType::from_spark_name("byte"), Some(ColumnType::Byte));
        assert_eq!(
            ColumnType::from_spark_name("tinyint"),
            Some(ColumnType::Byte)
        );
        assert_eq!(
            ColumnType::from_spark_name("short"),
            Some(ColumnType::Short)
        );
        assert_eq!(
            ColumnType::from_spark_name("smallint"),
            Some(ColumnType::Short)
        );
        assert_eq!(
            ColumnType::from_spark_name("binary"),
            Some(ColumnType::Binary)
        );
        assert_eq!(
            ColumnType::from_spark_name("decimal(18, 2)"),
            Some(ColumnType::Decimal {
                precision: 18,
                scale: 2,
            })
        );
        // Case-insensitive on the keyword.
        assert_eq!(
            ColumnType::from_spark_name("DECIMAL(10, 0)"),
            Some(ColumnType::DEFAULT_DECIMAL),
        );
    }

    #[test]
    fn from_type_constructor_accepts_byte_short_decimal_binary() {
        assert_eq!(
            ColumnType::from_type_constructor("ByteType"),
            Some(ColumnType::Byte)
        );
        assert_eq!(
            ColumnType::from_type_constructor("ShortType"),
            Some(ColumnType::Short)
        );
        assert_eq!(
            ColumnType::from_type_constructor("DecimalType"),
            Some(ColumnType::DEFAULT_DECIMAL),
        );
        assert_eq!(
            ColumnType::from_type_constructor("BinaryType"),
            Some(ColumnType::Binary)
        );
    }

    #[test]
    fn display_decimal_carries_precision_and_scale() {
        assert_eq!(
            format!(
                "{}",
                ColumnType::Decimal {
                    precision: 18,
                    scale: 2,
                }
            ),
            "Decimal(18, 2)",
        );
        assert_eq!(format!("{}", ColumnType::DEFAULT_DECIMAL), "Decimal(10, 0)");
        assert_eq!(format!("{}", ColumnType::Byte), "Byte");
        assert_eq!(format!("{}", ColumnType::Short), "Short");
        assert_eq!(format!("{}", ColumnType::Binary), "Binary");
    }

    #[test]
    fn numeric_and_dec_are_aliases_for_decimal_on_every_surface() {
        // `from_name` keeps the lowercase-only contract — same as `int`
        // vs `Int`. Uppercase / mixed-case is reserved for the Spark
        // cast path, where Spark SQL itself accepts any casing.
        for name in ["numeric", "dec"] {
            assert_eq!(
                ColumnType::from_name(name),
                Some(ColumnType::DEFAULT_DECIMAL),
                "{name} should resolve to default decimal",
            );
            assert_eq!(
                ColumnType::from_spark_name(name),
                Some(ColumnType::DEFAULT_DECIMAL),
                "{name} should resolve to default decimal on the cast path too",
            );
        }
        // Parameterized aliases on the strict path.
        assert_eq!(
            ColumnType::from_name("numeric(18, 2)"),
            Some(ColumnType::Decimal {
                precision: 18,
                scale: 2,
            })
        );
        assert_eq!(
            ColumnType::from_name("dec(18, 2)"),
            Some(ColumnType::Decimal {
                precision: 18,
                scale: 2,
            })
        );
        // Spark cast path stays case-insensitive on the aliases too —
        // matches Spark SQL's behaviour on type names.
        for name in ["NUMERIC", "Dec", "DECIMAL(18, 2)", "Numeric(10)"] {
            assert!(
                ColumnType::from_spark_name(name).is_some(),
                "{name} should resolve on the Spark cast path",
            );
        }
        assert_eq!(
            ColumnType::from_spark_name("numeric(18, 2)"),
            Some(ColumnType::Decimal {
                precision: 18,
                scale: 2,
            })
        );
        assert_eq!(
            ColumnType::from_spark_name("dec(10)"),
            Some(ColumnType::Decimal {
                precision: 10,
                scale: 0,
            })
        );
    }

    #[test]
    fn from_name_keeps_case_sensitivity_on_decimal_aliases() {
        // The strict-vocabulary surface (`from_name`, used by Schema
        // annotations) is case-sensitive — `Int` and `Decimal` are
        // rejected; the alias forms `numeric` / `dec` follow the same
        // rule. Lifting case-sensitivity here would contradict the
        // long-standing `from_name("Int") == None` contract.
        for name in ["NUMERIC", "Dec", "DECIMAL", "Decimal(18, 2)", "DEC(10)"] {
            assert_eq!(
                ColumnType::from_name(name),
                None,
                "{name} should be rejected by the strict-case-sensitive surface",
            );
        }
    }

    #[test]
    fn is_unmodeled_spark_type_recognises_real_but_unpinned_targets() {
        // Trust-critical: these are legitimate Spark types, just not
        // ones pykrete pins a type on yet. The cast-typo emission has to
        // skip them to avoid regressing v0.1.27 behavior.
        for name in [
            "varchar(100)",
            "varchar(1)",
            "VARCHAR(100)",
            "char(10)",
            "char(1)",
            "CHAR(10)",
            "interval",
            "INTERVAL",
            "interval year",
            "interval year to month",
            "interval day to second",
            "interval hour to minute",
            // Round-4 fix: compound interval with precision args and
            // mixed casing — `INTERVAL DAY(3) TO SECOND(6)` is real
            // Spark SQL, the matcher must accept it.
            "INTERVAL DAY(3) TO SECOND(6)",
            "interval day(3) to second(6)",
            "interval day to second",
            "timestamp_ntz",
            "TIMESTAMP_NTZ",
            "void",
            "null",
            "VOID",
            "NULL",
        ] {
            assert!(
                ColumnType::is_unmodeled_spark_type(name),
                "{name} should be recognised as a real-but-unmodeled Spark type",
            );
        }
        // Negative pairs: typos and parenthesised garbage must not slip
        // through the allowlist. Round-4 fix: `interval(5)`, `void(0)`,
        // `null(0)` were previously false-allowed because the matcher
        // split on `(` and matched the bare keyword — these underlying
        // types take no parenthesised argument.
        for name in [
            "decimial(18,2)",
            "intger",
            "varhar(10)",
            "intervals",
            "interval(5)",
            "void(0)",
            "null(0)",
            "timestamp_ntz(3)",
            "varchar(0)",
            "varchar(-1)",
            "varchar()",
            "char(0)",
        ] {
            assert!(
                !ColumnType::is_unmodeled_spark_type(name),
                "{name} should NOT be on the unmodeled allowlist",
            );
        }
    }
}
