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
pub const COLUMN_TYPE_NAMES: &str = "int, long, double, string, bool, date, timestamp";
