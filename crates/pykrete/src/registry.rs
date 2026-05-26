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
    Decorator, Expr, ModModule, Stmt, StmtAnnAssign, StmtClassDef, StmtFunctionDef, TypeParam,
    TypeParams,
};
use ruff_text_size::TextRange;

use crate::types::ColumnType;

/// One method declared on a class. Captures everything needed for generic
/// substitution: the method's own type parameters (PEP 695 `def m[T]`),
/// each positional parameter's name and annotation, and the return-type
/// annotation.
#[derive(Debug, Clone)]
pub struct MethodInfo<'a> {
    pub name: &'a str,
    pub type_params: Vec<&'a str>,
    pub params: Vec<MethodParam<'a>>,
    pub return_annotation: Option<&'a Expr>,
    pub range: TextRange,
    /// The function's own definition node. Carried so a caller can walk
    /// the body — used to *infer* a transform function's output schema
    /// when its return type isn't declared.
    pub def: &'a StmtFunctionDef,
}

#[derive(Debug, Clone)]
pub struct MethodParam<'a> {
    pub name: &'a str,
    pub annotation: Option<&'a Expr>,
    pub kind: ParamKind,
}

/// Which segment of the parameter list a [`MethodParam`] belongs to. D0051
/// uses this to refuse matches Python itself would reject — a keyword arg
/// against a positional-only slot, or a positional arg sliding past `*`
/// into a kw-only slot — and to route variadics (`*args` / `**kwargs`) to
/// every remaining call-site arg of the right flavor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// Before a `/` marker — positional only.
    PositionalOnly,
    /// Regular — can be passed positionally or by keyword.
    Regular,
    /// After a `*` or `*args` marker — keyword only.
    KeywordOnly,
    /// `*args` itself — soaks up trailing positional arguments.
    VarPositional,
    /// `**kwargs` itself — soaks up trailing keyword arguments.
    VarKeyword,
}

/// Any top-level class definition. Schema-derived classes are also
/// represented here, alongside non-Schema classes like `DataAccessLayer`.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct ConstantInfo<'a> {
    pub name: &'a str,
    /// The outer generic class name as written, e.g. `"DataSource"`.
    pub generic_class: &'a str,
    /// The inner schema-class name as written, e.g. `"RawOrders"`.
    pub schema_name: &'a str,
    pub range: TextRange,
}

#[derive(Debug, Clone)]
pub struct Registry<'a> {
    pub classes: HashMap<&'a str, ClassInfo<'a>>,
    pub constants: HashMap<&'a str, ConstantInfo<'a>>,
    /// Class-qualified annotated assignments — `DataSources.RAW_ORDERS`
    /// in a frozen-dataclass registry. Keyed by `(class_name, const_name)`
    /// so the analyzer can resolve `Attribute(Name("DataSources"), "RAW_ORDERS")`
    /// the same way it resolves a bare module-level constant.
    pub class_constants: HashMap<(&'a str, &'a str), ConstantInfo<'a>>,
    /// Top-level `def` functions, keyed by name. Captured as [`MethodInfo`]
    /// (the same signature shape used for class methods — a free function
    /// is just a method with no `self`). Used to resolve the function
    /// argument of `df.transform(some_function)`: its declared parameter
    /// and return annotations tell us the input schema to check against
    /// and the result schema the transform produces.
    pub functions: HashMap<&'a str, MethodInfo<'a>>,
    /// User-defined functions registered as Spark UDFs — via an `@udf` /
    /// `@pandas_udf` decorator or the functional `name = udf(f, …)` form.
    /// Maps the UDF's name to the column type it returns, so a call like
    /// `my_udf(col("x"))` can be typed.
    pub udfs: HashMap<&'a str, ColumnType>,
}

impl<'a> Registry<'a> {
    pub fn build(module: &'a ModModule) -> Self {
        let mut classes = HashMap::new();
        let mut constants = HashMap::new();
        let mut class_constants = HashMap::new();
        let mut functions = HashMap::new();
        let mut udfs = HashMap::new();

        for stmt in &module.body {
            match stmt {
                Stmt::FunctionDef(def) => {
                    let info = build_method_info(def);
                    functions.insert(info.name, info);
                    // A `@udf` / `@pandas_udf` decorator registers the
                    // function as a Spark UDF with a known return type.
                    for dec in &def.decorator_list {
                        if let Some(rt) = udf_decorator_return_type(dec) {
                            udfs.insert(def.name.id.as_str(), rt);
                            break;
                        }
                    }
                }
                // Functional UDF form — `name = udf(func, "int")`.
                Stmt::Assign(a) => {
                    if let Some(rt) = functional_udf_return_type(&a.value) {
                        for target in &a.targets {
                            if let Some(n) = target.as_name_expr() {
                                udfs.insert(n.id.as_str(), rt.clone());
                            }
                        }
                    }
                }
                Stmt::ClassDef(def) => {
                    let info = build_class_info(def);
                    let class_name = info.name;
                    classes.insert(info.name, info);
                    // Walk the class body for annotated constants too —
                    // real codebases declare data sources inside a
                    // `@dataclass class DataSources:` body, not at module
                    // top level.
                    for body_stmt in &def.body {
                        if let Stmt::AnnAssign(ann) = body_stmt
                            && let Some(const_info) = build_constant_info(ann)
                        {
                            class_constants.insert((class_name, const_info.name), const_info);
                        }
                    }
                }
                Stmt::AnnAssign(ann) => {
                    if let Some(info) = build_constant_info(ann) {
                        constants.insert(info.name, info);
                    }
                }
                _ => {}
            }
        }

        Self {
            classes,
            constants,
            class_constants,
            functions,
            udfs,
        }
    }

    pub fn find_class(&self, name: &str) -> Option<&ClassInfo<'a>> {
        self.classes.get(name)
    }

    pub fn find_constant(&self, name: &str) -> Option<&ConstantInfo<'a>> {
        self.constants.get(name)
    }

    /// Look up a top-level `def` function by name. Used to resolve the
    /// function argument of `df.transform(...)`.
    pub fn find_function(&self, name: &str) -> Option<&MethodInfo<'a>> {
        self.functions.get(name)
    }

    /// The column type a registered UDF returns, if `name` is one.
    pub fn find_udf(&self, name: &str) -> Option<ColumnType> {
        self.udfs.get(name).cloned()
    }

    /// Look up a class-qualified annotated constant — `ClassName.NAME`.
    /// HashMap lookup against a `(&str, &str)` tuple key doesn't borrow
    /// cleanly against the stored `&'a str` keys, so we scan linearly.
    /// The class-constants map is tiny in practice (one entry per
    /// declared data source).
    pub fn find_class_constant(
        &self,
        class_name: &str,
        const_name: &str,
    ) -> Option<&ConstantInfo<'a>> {
        self.class_constants
            .iter()
            .find(|((c, n), _)| *c == class_name && *n == const_name)
            .map(|(_, info)| info)
    }
}

/// The simple-name callee of an expression — `f` in `f(...)`, or the
/// attribute `m` in `obj.m(...)` / a bare `obj.m`.
fn callee_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(n) => Some(n.id.as_str()),
        Expr::Attribute(a) => Some(a.attr.id.as_str()),
        _ => None,
    }
}

/// Resolve a Spark type written as a string literal (`"int"`) or a
/// type-object constructor call (`IntegerType()`) to a [`ColumnType`].
pub fn spark_type_from_expr(expr: &Expr) -> Option<ColumnType> {
    match expr {
        Expr::StringLiteral(s) => ColumnType::from_spark_name(s.value.to_str()),
        Expr::Call(call) => ColumnType::from_type_constructor(callee_name(&call.func)?),
        _ => None,
    }
}

/// If `decorator` is an `@udf` / `@pandas_udf` decorator, the column
/// type it declares for the decorated function's result. A bare `@udf`
/// (or `@udf()` with no return type) defaults to string — Spark's
/// default UDF return type.
fn udf_decorator_return_type(decorator: &Decorator) -> Option<ColumnType> {
    match &decorator.expression {
        // `@udf` / `@F.udf` — bare, not called.
        Expr::Name(_) | Expr::Attribute(_) => {
            matches!(callee_name(&decorator.expression)?, "udf" | "pandas_udf")
                .then_some(ColumnType::String)
        }
        // `@udf("int")` / `@udf(returnType=IntegerType())` / `@udf()`.
        Expr::Call(call) => {
            if !matches!(callee_name(&call.func)?, "udf" | "pandas_udf") {
                return None;
            }
            for kw in &call.arguments.keywords {
                if kw
                    .arg
                    .as_ref()
                    .is_some_and(|n| n.id.as_str() == "returnType")
                {
                    return spark_type_from_expr(&kw.value);
                }
            }
            // As a decorator, the first positional argument is the
            // return type; absent one, Spark defaults to string.
            Some(
                call.arguments
                    .args
                    .first()
                    .and_then(spark_type_from_expr)
                    .unwrap_or(ColumnType::String),
            )
        }
        _ => None,
    }
}

/// The return type of a functional-form UDF — `udf(func, "int")`. Unlike
/// the decorator form, the return type is the *second* positional
/// argument (the first is the wrapped function), or the `returnType`
/// keyword. Returns `None` if `value` isn't a `udf(...)` call.
fn functional_udf_return_type(value: &Expr) -> Option<ColumnType> {
    let Expr::Call(call) = value else {
        return None;
    };
    if !matches!(callee_name(&call.func)?, "udf" | "pandas_udf") {
        return None;
    }
    for kw in &call.arguments.keywords {
        if kw
            .arg
            .as_ref()
            .is_some_and(|n| n.id.as_str() == "returnType")
        {
            return spark_type_from_expr(&kw.value);
        }
    }
    Some(
        call.arguments
            .args
            .get(1)
            .and_then(spark_type_from_expr)
            .unwrap_or(ColumnType::String),
    )
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
    for pwd in &parameters.posonlyargs {
        let p = &pwd.parameter;
        params.push(MethodParam {
            name: p.name.id.as_str(),
            annotation: p.annotation.as_deref(),
            kind: ParamKind::PositionalOnly,
        });
    }
    for pwd in &parameters.args {
        let p = &pwd.parameter;
        params.push(MethodParam {
            name: p.name.id.as_str(),
            annotation: p.annotation.as_deref(),
            kind: ParamKind::Regular,
        });
    }
    if let Some(vararg) = parameters.vararg.as_deref() {
        params.push(MethodParam {
            name: vararg.name.id.as_str(),
            annotation: vararg.annotation.as_deref(),
            kind: ParamKind::VarPositional,
        });
    }
    for pwd in &parameters.kwonlyargs {
        let p = &pwd.parameter;
        params.push(MethodParam {
            name: p.name.id.as_str(),
            annotation: p.annotation.as_deref(),
            kind: ParamKind::KeywordOnly,
        });
    }
    if let Some(kwarg) = parameters.kwarg.as_deref() {
        params.push(MethodParam {
            name: kwarg.name.id.as_str(),
            annotation: kwarg.annotation.as_deref(),
            kind: ParamKind::VarKeyword,
        });
    }

    MethodInfo {
        name,
        type_params,
        params,
        return_annotation: def.returns.as_deref(),
        range: def.range,
        def,
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
