//! File-level registries of discovered classes and top-level typed constants.
//!
//! Iteration 21 introduces these to support **generic-function inference**.
//! The motivating use case is the DataSource pattern:
//!
//! ```text
//! class DataSource[T]: ...
//! class DataAccessLayer:
//!     def read[T](self, source: DataSource[T]) -> DataFrame[T]: ...
//!
//! RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")
//!
//! def f(dal: DataAccessLayer) -> DataFrame[RawOrders]:
//!     return dal.read(RAW_ORDERS)
//! ```
//!
//! To make `dal.read(RAW_ORDERS)` resolve to `DataFrame[RawOrders]`, we need:
//!
//! - a **class registry** mapping `DataAccessLayer` → its methods (so we can
//!   look up `read`'s signature);
//! - generic-type parameter info on each method (so we know `T` is a binder
//!   in `read[T]`);
//! - a **constant registry** mapping top-level annotated assignments like
//!   `RAW_ORDERS: DataSource[RawOrders]` → the schema name they carry.
//!
//! This module owns the data structures; the substitution logic that uses
//! them lives in `operations.rs`.

use std::collections::HashMap;

use ruff_python_ast::{
    Expr, ModModule, Stmt, StmtAnnAssign, StmtClassDef, StmtFunctionDef, TypeParam, TypeParams,
};
use ruff_text_size::TextRange;

/// One method declared on a class. Captures everything needed for generic
/// substitution: the method's own type parameters (PEP 695 `def m[T]`),
/// each positional parameter's name and annotation, and the return-type
/// annotation.
#[derive(Debug)]
pub struct MethodInfo<'a> {
    pub name: &'a str,
    pub type_params: Vec<&'a str>,
    pub params: Vec<MethodParam<'a>>,
    pub return_annotation: Option<&'a Expr>,
    pub range: TextRange,
}

#[derive(Debug)]
pub struct MethodParam<'a> {
    pub name: &'a str,
    pub annotation: Option<&'a Expr>,
}

/// Any top-level class definition. Schema-derived classes are also
/// represented here, alongside non-Schema classes like `DataAccessLayer`.
#[derive(Debug)]
pub struct ClassInfo<'a> {
    pub name: &'a str,
    pub type_params: Vec<&'a str>,
    pub methods: HashMap<&'a str, MethodInfo<'a>>,
}

/// A top-level annotated assignment like
/// `RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")`.
///
/// Only the simple shape `name: GenericClass[SchemaName] = …` is captured —
/// other forms (no subscript, non-bare inner name) are skipped.
#[derive(Debug)]
pub struct ConstantInfo<'a> {
    pub name: &'a str,
    /// The outer generic class name as written, e.g. `"DataSource"`.
    pub generic_class: &'a str,
    /// The inner schema-class name as written, e.g. `"RawOrders"`.
    pub schema_name: &'a str,
    pub range: TextRange,
}

#[derive(Debug)]
pub struct Registry<'a> {
    pub classes: HashMap<&'a str, ClassInfo<'a>>,
    pub constants: HashMap<&'a str, ConstantInfo<'a>>,
}

impl<'a> Registry<'a> {
    pub fn build(module: &'a ModModule) -> Self {
        let mut classes = HashMap::new();
        let mut constants = HashMap::new();

        for stmt in &module.body {
            match stmt {
                Stmt::ClassDef(def) => {
                    let info = build_class_info(def);
                    classes.insert(info.name, info);
                }
                Stmt::AnnAssign(ann) => {
                    if let Some(info) = build_constant_info(ann) {
                        constants.insert(info.name, info);
                    }
                }
                _ => {}
            }
        }

        Self { classes, constants }
    }

    pub fn find_class(&self, name: &str) -> Option<&ClassInfo<'a>> {
        self.classes.get(name)
    }

    pub fn find_constant(&self, name: &str) -> Option<&ConstantInfo<'a>> {
        self.constants.get(name)
    }
}

fn build_class_info(def: &StmtClassDef) -> ClassInfo<'_> {
    let name = def.name.id.as_str();
    let type_params = extract_type_params(def.type_params.as_deref());
    let mut methods = HashMap::new();

    for stmt in &def.body {
        if let Stmt::FunctionDef(fn_def) = stmt {
            let info = build_method_info(fn_def);
            methods.insert(info.name, info);
        }
    }

    ClassInfo {
        name,
        type_params,
        methods,
    }
}

fn build_method_info(def: &StmtFunctionDef) -> MethodInfo<'_> {
    let name = def.name.id.as_str();
    let type_params = extract_type_params(def.type_params.as_deref());

    let parameters = &def.parameters;
    let mut params: Vec<MethodParam<'_>> = Vec::new();
    for pwd in parameters
        .posonlyargs
        .iter()
        .chain(&parameters.args)
        .chain(&parameters.kwonlyargs)
    {
        let p = &pwd.parameter;
        params.push(MethodParam {
            name: p.name.id.as_str(),
            annotation: p.annotation.as_deref(),
        });
    }

    MethodInfo {
        name,
        type_params,
        params,
        return_annotation: def.returns.as_deref(),
        range: def.range,
    }
}

fn extract_type_params(type_params: Option<&TypeParams>) -> Vec<&str> {
    let Some(tp) = type_params else {
        return Vec::new();
    };
    let mut result = Vec::new();
    // Only the simple `TypeVar` form (`def f[T]`) is captured; `TypeVarTuple`
    // and `ParamSpec` aren't needed for v0.1.
    for p in &tp.type_params {
        if let TypeParam::TypeVar(tv) = p {
            result.push(tv.name.id.as_str());
        }
    }
    result
}

fn build_constant_info(ann: &StmtAnnAssign) -> Option<ConstantInfo<'_>> {
    let name = ann.target.as_name_expr()?.id.as_str();
    let subscript = ann.annotation.as_subscript_expr()?;
    let generic_class = subscript.value.as_name_expr()?.id.as_str();
    let schema_name = subscript.slice.as_name_expr()?.id.as_str();
    Some(ConstantInfo {
        name,
        generic_class,
        schema_name,
        range: ann.range,
    })
}
