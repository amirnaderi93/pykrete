use super::col_refs::col_reference;
use super::context::{BodyContext, TypeCtx};
use super::strict_operators::select_output_name;

use ruff_python_ast::{Expr, ExprCall, Number};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextRange};

use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::SchemaView;
use crate::types::{ColumnType, StructField};

// ---------------------------------------------------------------------------
// Column-expression type inference
// ---------------------------------------------------------------------------

// `TypeCtx` moved to `context.rs` so `BodyContext::type_ctx` can return it
// without a cross-section reference back into this file.

/// Infer the atomic type of a column expression evaluated against
/// `schema`. `None` means "couldn't determine" — a function result
/// pykrete doesn't model, a column off an un-inferred schema, an
/// unmodeled literal. `None` is permissive: it is never itself a type
/// error, only the absence of information.
/// True if `expr` is `lit(None)` / `F.lit(None)` — an explicit null
/// literal. An untyped null is rarely useful on its own; it is usually
/// `.cast(...)` to a concrete type, and the cast carries the type while
/// this carries the nullability.
fn expr_is_null_literal(expr: &Expr) -> bool {
    let Some(call) = expr.as_call_expr() else {
        return false;
    };
    let fname = match call.func.as_ref() {
        Expr::Name(n) => n.id.as_str(),
        Expr::Attribute(a) => a.attr.id.as_str(),
        _ => return false,
    };
    fname == "lit"
        && call
            .arguments
            .args
            .first()
            .is_some_and(|a| a.is_none_literal_expr())
}

pub(super) fn infer_expr_type<'a>(
    expr: &Expr,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> Option<ColumnType> {
    // `col("x")` / `column("x")` — the column's declared/inferred type.
    if let Some((name, _)) = col_reference(expr) {
        return schema.field_type(name, tcx.schemas);
    }
    // `df["x"]` (Subscript-on-Name, string-literal slice) — the pandas
    // scalar column-ref idiom. v1.4 spec §3c fork (a): treat the
    // subscript as a column reference and look the name up on `schema`.
    // Gated on the receiver Name being a DataFrame-bound binding in the
    // current body (mirrors the D0030 sibling arm in `col_refs.rs`); a
    // plain Python `bag["k"]` against a dict/list local has no DF
    // binding and falls through, so D0081 / D0082 don't false-fire on
    // it. Other receiver shapes (`Expr::Attribute`, `Expr::Call`,
    // nested `Expr::Subscript`) and non-literal slices fall through.
    if let Expr::Subscript(sub) = expr
        && let Some(name) = sub.value.as_name_expr()
        && body.lookup(name.id.as_str()).is_some()
        && let Some(lit) = sub.slice.as_string_literal_expr()
    {
        return schema.field_type(lit.value.to_str(), tcx.schemas);
    }
    match expr {
        Expr::Call(call) => {
            if let Some(attr) = call.func.as_attribute_expr() {
                match attr.attr.id.as_str() {
                    // `<expr>.alias("y")` / `.name("y")` — type unchanged.
                    // `<windowed>.over(w)` likewise carries the windowed
                    // expression's type through.
                    "alias" | "name" | "over" => {
                        return infer_expr_type(&attr.value, schema, tcx, body);
                    }
                    // `<expr>.cast("int")` / `.cast(IntegerType())`.
                    // A cast carries nullability through: a nullable
                    // input — or a `lit(None)` — casts to a nullable
                    // column of the target type.
                    "cast" => {
                        let target = call
                            .arguments
                            .args
                            .first()
                            .and_then(crate::registry::spark_type_from_expr)?;
                        let nullable = expr_is_null_literal(&attr.value)
                            || infer_expr_type(&attr.value, schema, tcx, body)
                                .is_some_and(|t| t.is_nullable());
                        return Some(if nullable {
                            ColumnType::Nullable(Box::new(target))
                        } else {
                            target
                        });
                    }
                    // Boolean-returning Column predicates. Every one of
                    // these reduces a column expression to a boolean —
                    // `col("x").isNull()`, `.isin(...)`, `.between(lo, hi)`,
                    // `.like("…")`, `.rlike("…")`, `.ilike("…")`,
                    // `.contains("…")`, `.startswith("…")`, `.endswith("…")`.
                    // The receiver's column refs are reached by the
                    // generic walker; here we just publish the result type.
                    "isNull" | "isNotNull" | "isin" | "between" | "like" | "rlike" | "ilike"
                    | "contains" | "startswith" | "endswith" => {
                        return Some(ColumnType::Bool);
                    }
                    // `<struct-col>.getField("name")` — the nested
                    // field's type. Falls through (None) when the
                    // receiver isn't a known struct or the name isn't a
                    // string literal (computed access), staying
                    // permissive.
                    "getField" => {
                        let recv_ty = infer_expr_type(&attr.value, schema, tcx, body)?;
                        let field_name = call
                            .arguments
                            .args
                            .first()
                            .and_then(|a| a.as_string_literal_expr())?
                            .value
                            .to_str();
                        return struct_field_type(&recv_ty, field_name);
                    }
                    // `<array-col>.getItem(0)` → element type;
                    // `<map-col>.getItem("k")` → value type. Index /
                    // key shape doesn't matter to the result type.
                    "getItem" => {
                        let recv_ty = infer_expr_type(&attr.value, schema, tcx, body)?;
                        return collection_element_type(&recv_ty);
                    }
                    // `<struct-col>.withField("name", expr)` returns a
                    // struct column with `name` added (or replaced) and
                    // its type set to `expr`'s inferred type. Carries
                    // the receiver's struct shape forward — `.withField`
                    // is how PySpark code threads a new field into a
                    // nested struct without losing the rest.
                    "withField" => {
                        let recv_ty = infer_expr_type(&attr.value, schema, tcx, body)?;
                        let field_name = call
                            .arguments
                            .args
                            .first()
                            .and_then(|a| a.as_string_literal_expr())?
                            .value
                            .to_str();
                        let value_ty = call
                            .arguments
                            .args
                            .get(1)
                            .and_then(|v| infer_expr_type(v, schema, tcx, body));
                        return Some(struct_with_field(&recv_ty, field_name, value_ty));
                    }
                    // `<struct-col>.dropFields("a", "b", …)` returns a
                    // struct column with those fields removed. Names
                    // that aren't string literals are ignored — pykrete
                    // can't verify them and falls back to returning the
                    // receiver's struct unchanged.
                    "dropFields" => {
                        let recv_ty = infer_expr_type(&attr.value, schema, tcx, body)?;
                        let drop: Vec<&str> = call
                            .arguments
                            .args
                            .iter()
                            .filter_map(|a| a.as_string_literal_expr().map(|s| s.value.to_str()))
                            .collect();
                        return Some(struct_drop_fields(&recv_ty, &drop));
                    }
                    // `F.when(p, v).otherwise(e)` — common-type widen the
                    // branch values; if any branch is Nullable, the
                    // result is Nullable. `.otherwise()` is only valid
                    // chained on a `when`, so the helper validates the
                    // receiver shape and returns None on anything else.
                    "otherwise" => {
                        let else_ty = call
                            .arguments
                            .args
                            .first()
                            .and_then(|a| infer_expr_type(a, schema, tcx, body));
                        return when_chain_result_type(
                            &attr.value,
                            Some(else_ty),
                            schema,
                            tcx,
                            body,
                        );
                    }
                    // A `.when(...)` chained on an earlier `when` — model
                    // it identically to a fresh `F.when(...)` chain that
                    // has no `.otherwise()` (so the result is nullable).
                    // Standalone `F.when(p, v)` falls into the `Name` /
                    // `Attribute` arm below.
                    "when" if is_when_chain(&attr.value) => {
                        return when_chain_result_type(expr, None, schema, tcx, body);
                    }
                    _ => {}
                }
            }
            let fname = match call.func.as_ref() {
                Expr::Name(n) => n.id.as_str(),
                Expr::Attribute(a) => a.attr.id.as_str(),
                _ => return None,
            };
            // `F.lit(value)` — the literal's own type.
            if fname == "lit" {
                return call.arguments.args.first().and_then(python_literal_type);
            }
            // `F.when(p, v)` without a trailing `.otherwise()` — the
            // result is Nullable(common-type-of-value-branches) because
            // unmatched rows produce null.
            if fname == "when" {
                return when_chain_result_type(expr, None, schema, tcx, body);
            }
            // `F.struct(...)` / `F.named_struct("k1", v1, ...)` — build a
            // struct type from the args' inferred names and types.
            if fname == "struct" {
                return Some(infer_struct_type(call, schema, tcx, body));
            }
            if fname == "named_struct" {
                return infer_named_struct_type(call, schema, tcx, body);
            }
            // A call to a user-defined UDF — its declared return type.
            if let Some(udf_ty) = tcx.registry.find_udf(fname) {
                return Some(udf_ty);
            }
            // Array higher-order functions — `F.transform(col, fn)` /
            // `F.filter` / `F.aggregate` / `F.exists` / `F.forall`. Pykrete
            // doesn't fully evaluate the lambda body; the return type is
            // modeled per-function with a best-effort lambda-body type
            // heuristic that falls back to Unknown rather than fabricating.
            if matches!(
                fname,
                "transform" | "filter" | "aggregate" | "exists" | "forall"
            ) {
                return array_hof_result_type(fname, call, schema, tcx, body);
            }
            // Branch-form null-coalescing (`coalesce` / `nvl` /
            // `ifnull`) — Q9 output preservation. All branches'
            // inferred types are reconciled: enum-typed branches with
            // set-equal vocabularies preserve the constraint; any
            // plain-string branch drops it; non-set-equal enum
            // vocabularies degrade to None (the firing of D0040 lives
            // on the schema-merge surface, not here). The downstream
            // `coalesce_branch_result` returns the carried-through
            // type or falls back to the first-arg behaviour.
            if matches!(fname, "coalesce" | "nvl" | "ifnull") {
                return coalesce_branch_result(call, schema, tcx, body);
            }
            // Any other recognized `pyspark.sql.functions` call — look its
            // result type up in the catalog, resolving the first argument
            // (a column name or expression) for the input-dependent ones.
            let first_arg = call
                .arguments
                .args
                .first()
                .and_then(|a| select_arg_type(a, schema, tcx, body));
            function_result_type(fname, first_arg)
        }
        // A bare Python literal in column position acts as `lit(...)`.
        _ => python_literal_type(expr),
    }
}

/// The type of a `select` output column, given its argument. A bare
/// string literal is a column *name* there (not a string value), so it
/// is resolved against the receiver before falling back to
/// [`infer_expr_type`].
pub(super) fn select_arg_type<'a>(
    arg: &Expr,
    recv: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> Option<ColumnType> {
    if let Some(s) = arg.as_string_literal_expr() {
        return recv.field_type(s.value.to_str(), tcx.schemas);
    }
    infer_expr_type(arg, recv, tcx, body)
}

/// The result [`ColumnType`] of a `pyspark.sql.functions` call, given
/// the function name and (for input-dependent functions) the type of
/// its first argument. `None` for functions pykrete doesn't model or
/// whose result isn't an atomic type (`collect_list` → array, …) —
/// permissive, never a type error.
pub(super) fn function_result_type(
    name: &str,
    first_arg: Option<ColumnType>,
) -> Option<ColumnType> {
    use ColumnType::{
        Array, Bool, Byte, Date, Double, Float, Int, Long, Map, Short, String, Timestamp,
    };
    // Functions with a fixed result type, regardless of input.
    match name {
        "count"
        | "countDistinct"
        | "count_distinct"
        | "count_if"
        | "approx_count_distinct"
        | "unix_timestamp"
        | "monotonically_increasing_id"
        | "factorial" => return Some(Long),
        "stddev" | "stddev_pop" | "stddev_samp" | "variance" | "var_pop" | "var_samp"
        | "skewness" | "kurtosis" | "corr" | "covar_pop" | "covar_samp" | "percent_rank"
        | "cume_dist" | "rand" | "randn" | "sqrt" | "exp" | "expm1" | "ln" | "log" | "log2"
        | "log10" | "log1p" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2"
        | "sinh" | "cosh" | "tanh" | "degrees" | "radians" | "cbrt" | "pow" | "power" | "hypot"
        | "signum" | "months_between" | "try_divide" => return Some(Double),
        "length" | "char_length" | "character_length" | "ascii" | "instr" | "locate"
        | "levenshtein" | "year" | "month" | "dayofmonth" | "day" | "dayofweek" | "dayofyear"
        | "hour" | "minute" | "second" | "weekofyear" | "quarter" | "datediff" | "date_diff"
        | "unix_date" | "row_number" | "rank" | "dense_rank" | "ntile" | "spark_partition_id"
        | "size" => return Some(Int),
        "lower" | "upper" | "initcap" | "trim" | "ltrim" | "rtrim" | "reverse" | "concat_ws"
        | "substring" | "substring_index" | "regexp_replace" | "regexp_extract" | "lpad"
        | "rpad" | "translate" | "repeat" | "soundex" | "base64" | "format_string"
        | "format_number" | "hex" | "sha1" | "sha2" | "md5" | "date_format" | "from_unixtime" => {
            return Some(String);
        }
        "to_date" | "current_date" | "last_day" | "next_day" | "date_add" | "date_sub"
        | "add_months" | "trunc" => return Some(Date),
        "to_timestamp" | "current_timestamp" | "date_trunc" | "from_utc_timestamp"
        | "to_utc_timestamp" => return Some(Timestamp),
        "isnull" | "isnan" => return Some(Bool),
        "split" => return Some(Array(Some(Box::new(String)))),
        "create_map" | "map_from_arrays" | "map_from_entries" | "map_concat" | "str_to_map"
        | "transform_keys" | "transform_values" | "map_filter" => {
            return Some(Map(None, None));
        }
        // Result not an atomic type pykrete models (an array of structs).
        "arrays_zip" | "map_entries" => return Some(Array(None)),
        _ => {}
    }
    // Functions whose result type depends on the first argument.
    match name {
        // Null-coalescing — the result is non-null when *any* argument
        // is. Conservatively drop nullability (this only under-reports).
        // Note: `coalesce`/`nvl`/`ifnull` already short-circuit via
        // `coalesce_branch_result` in `infer_expr_type`, so this arm is
        // only reached on schema paths that bypass the call-site dispatch
        // (kept for symmetry / future call sites).
        "coalesce" | "nvl" | "ifnull" => first_arg.map(|t| t.base().clone()),
        // `nullif(a, b)` returns NULL when `a == b`, else `a`. Per spec
        // Q9a it is idempotent over enum constraints: the result type
        // is `Nullable(typeof a)`, preserving the enum vocabulary so
        // downstream check sites still see the constraint.
        "nullif" => first_arg.map(|t| ColumnType::Nullable(Box::new(t.base().clone()))),
        "min" | "max" | "first" | "last" | "first_value" | "last_value" | "any_value"
        | "greatest" | "least" | "nanvl" | "abs" | "round" | "bround" | "negative" | "positive" => {
            first_arg
        }
        "ceil" | "ceiling" | "floor" => Some(Long),
        // `sum` widens any integral input (byte/short/int/long) to long;
        // double stays double; float widens to double (matching
        // Spark's float-collapse convention — see `from_type_constructor`);
        // a decimal stays decimal (precision-growth intentionally
        // collapsed — see `aggregate_output_type`).
        //
        // Exhaustive: every `ColumnType` variant is listed so a future
        // variant addition is a compile error here rather than a silent
        // `_ => None` regression (the §9 exhaustiveness gate).
        "sum" | "sumDistinct" | "sum_distinct" => match first_arg.as_ref().map(ColumnType::base) {
            Some(Byte | Short | Int | Long) => Some(Long),
            Some(Float | Double) => Some(Double),
            Some(ColumnType::Decimal { .. }) => Some(ColumnType::DEFAULT_DECIMAL),
            Some(
                String
                | ColumnType::Enum(_)
                | ColumnType::Binary
                | Bool
                | Date
                | Timestamp
                | Array(_)
                | Map(..)
                | ColumnType::Struct(_),
            ) => None,
            // `Nullable` is stripped by `.base()` above, so this branch
            // is unreachable in practice — listed for compile-time
            // exhaustiveness only.
            Some(ColumnType::Nullable(_)) => None,
            None => None,
        },
        // `mean` / `avg` mirrors the groupBy shortcut path: a decimal
        // stays decimal (precision-growth collapsed); any other numeric
        // input promotes to double (including float). Non-numeric input
        // (string, bool, date, binary, …) yields `None` on both surfaces
        // — Spark rejects such an average at runtime, so neither path
        // pins a type. Exhaustive arms for §9 gate.
        "avg" | "mean" => match first_arg.as_ref().map(ColumnType::base) {
            Some(ColumnType::Decimal { .. }) => Some(ColumnType::DEFAULT_DECIMAL),
            Some(Byte | Short | Int | Long | Float | Double) => Some(Double),
            Some(
                String
                | ColumnType::Enum(_)
                | ColumnType::Binary
                | Bool
                | Date
                | Timestamp
                | Array(_)
                | Map(..)
                | ColumnType::Struct(_),
            ) => None,
            Some(ColumnType::Nullable(_)) => None,
            None => None,
        },
        // Collection constructors — wrap the input as the element type.
        "collect_list" | "collect_set" | "array_agg" | "array" | "array_repeat" | "sequence" => {
            Some(Array(first_arg.map(Box::new)))
        }
        // Array → array of the same element type.
        "array_distinct" | "array_sort" | "sort_array" | "array_union" | "array_except"
        | "array_intersect" | "array_remove" | "array_compact" | "shuffle" | "slice" => {
            match first_arg {
                Some(array @ Array(_)) => Some(array),
                _ => Some(Array(None)),
            }
        }
        // `flatten` peels one array layer: `array<array<T>>` → `array<T>`.
        "flatten" => match first_arg {
            Some(Array(Some(inner))) if inner.is_composite() => Some(*inner),
            _ => Some(Array(None)),
        },
        // `explode` unwraps an array to its element type. (On a map it
        // yields two columns — not a single type — so it's left `None`.)
        "explode" | "explode_outer" => match first_arg {
            Some(Array(elem)) => elem.map(|b| *b),
            _ => None,
        },
        // `element_at` indexes into a collection — array element or map
        // value type. `get(array, index)` (Spark 3.4+) is the array-only
        // sibling that returns null on out-of-bounds instead of erroring;
        // the result type is the same shape.
        "element_at" | "get" => match first_arg {
            Some(Array(elem)) => elem.map(|b| *b),
            Some(Map(_, value)) => value.map(|b| *b),
            _ => None,
        },
        // `map_keys` / `map_values` → an array of the key / value type.
        "map_keys" => match first_arg {
            Some(Map(key, _)) => Some(Array(key)),
            _ => Some(Array(None)),
        },
        "map_values" => match first_arg {
            Some(Map(_, value)) => Some(Array(value)),
            _ => Some(Array(None)),
        },
        _ => None,
    }
}

/// Best-effort return-type inference for a lambda `lambda x: <body>`
/// passed to an array higher-order function. `schema` is the surrounding
/// DataFrame schema, so `lambda x: col("y")` against `y: int` resolves
/// to int. The lambda parameters are not bound — `lambda x: x + 1`
/// can't be evaluated without modeling `x` itself, which is out of scope
/// for v0.1; the heuristic falls back to None in that case (Unknown).
fn lambda_body_type<'a>(
    arg: Option<&Expr>,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> Option<ColumnType> {
    let lam = arg?.as_lambda_expr()?;
    // A `col("y")` against the surrounding schema, or a bare Python
    // literal. References to the lambda parameter aren't traced —
    // pykrete doesn't bind lambda params yet.
    infer_expr_type(&lam.body, schema, tcx, body)
}

/// The result [`ColumnType`] of an array higher-order function:
/// `F.transform(col, fn)`, `F.filter(col, fn)`, `F.aggregate(col, zero,
/// fn, [finish])`, `F.exists(col, fn)`, `F.forall(col, fn)`.
///
/// Lambda bodies aren't fully evaluated. The heuristic captures the
/// common shapes (`lambda x: 1`, `lambda x: col("y")`) and falls back
/// to a permissive default — `array<unknown>` for `transform`, the
/// input array type for `filter`, Unknown for `aggregate` — when the
/// body's type can't be read.
fn array_hof_result_type<'a>(
    name: &str,
    call: &ExprCall,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> Option<ColumnType> {
    use ColumnType::{Array, Bool};
    match name {
        // Boolean predicates — always bool, regardless of lambda body.
        "exists" | "forall" => Some(Bool),
        // `F.filter(col, fn)` preserves the element type of `col`.
        "filter" => {
            let input_ty = call
                .arguments
                .args
                .first()
                .and_then(|a| select_arg_type(a, schema, tcx, body));
            match input_ty {
                Some(arr @ Array(_)) => Some(arr),
                _ => Some(Array(None)),
            }
        }
        // `F.transform(col, fn)` → array of fn's return type.
        "transform" => {
            let body_ty = lambda_body_type(call.arguments.args.get(1), schema, tcx, body);
            Some(Array(body_ty.map(Box::new)))
        }
        // `F.aggregate(col, zero, fn, [finish])` → fn's return type
        // (or `finish`'s if provided). The arg layout is fixed:
        // index 2 is the merge lambda, index 3 the optional finalizer.
        "aggregate" => {
            let finish_ty = lambda_body_type(call.arguments.args.get(3), schema, tcx, body);
            if finish_ty.is_some() {
                return finish_ty;
            }
            lambda_body_type(call.arguments.args.get(2), schema, tcx, body)
        }
        _ => None,
    }
}

/// True if `expr` is a `when`-chain expression: a chain of `.when(p, v)`
/// calls rooted at a `F.when(...)` (or bare `when(...)`). Used to
/// distinguish a chained `.when` (`F.when(p, v).when(p2, v2)`) from an
/// unrelated `.when(...)` method on some other receiver.
fn is_when_chain(expr: &Expr) -> bool {
    let Some(call) = expr.as_call_expr() else {
        return false;
    };
    match call.func.as_ref() {
        // Bare `when(...)` — root.
        Expr::Name(n) => n.id.as_str() == "when",
        // `X.when(...)` — either the root `F.when(...)` (or
        // `pyspark.sql.functions.when(...)`, etc.) or a `.when` chained
        // on an earlier `.when`. If the receiver is itself a `.when(...)`
        // call, peel and recurse; otherwise this IS the root.
        Expr::Attribute(a) if a.attr.id.as_str() == "when" => match a.value.as_call_expr() {
            Some(inner)
                if matches!(
                    inner.func.as_ref(),
                    Expr::Attribute(ia) if ia.attr.id.as_str() == "when",
                ) =>
            {
                is_when_chain(&a.value)
            }
            _ => true,
        },
        _ => false,
    }
}

/// The result type of a `F.when(p, v)[.when(p2, v2)…][.otherwise(e)]`
/// chain. Walks the chain back to the root `F.when(...)`, collects every
/// branch value's type (and the `.otherwise` value's type if `else_ty`
/// is `Some`), and returns the common type — or `None` when the branches
/// don't reconcile (no false positive on downstream type-aware checks).
///
/// Nullability: any nullable branch — or the absence of an `.otherwise`
/// (modeled by passing `else_ty = None`) — wraps the result in
/// `Nullable(...)`. An explicit `.otherwise(F.lit(None))` carries
/// `Nullable` through the same path.
fn when_chain_result_type<'a>(
    chain: &Expr,
    else_ty: Option<Option<ColumnType>>,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> Option<ColumnType> {
    let mut branch_types: Vec<Option<ColumnType>> = Vec::new();
    let no_otherwise = else_ty.is_none();
    if let Some(t) = else_ty {
        branch_types.push(t);
    }
    let mut cursor = chain;
    loop {
        let call = cursor.as_call_expr()?;
        let fname = match call.func.as_ref() {
            Expr::Name(n) => n.id.as_str(),
            Expr::Attribute(a) => a.attr.id.as_str(),
            _ => return None,
        };
        if fname != "when" {
            return None;
        }
        let value_ty = call
            .arguments
            .args
            .get(1)
            .and_then(|v| infer_expr_type(v, schema, tcx, body));
        branch_types.push(value_ty);
        // `.when(p, v)` chained on an earlier `.when(...)` — peel one
        // layer and keep walking. A `Name("when")` callee (bare `when`)
        // or an attribute callee whose receiver isn't itself a `when`
        // chain (`F.when`) is the root.
        if let Some(attr) = call.func.as_attribute_expr()
            && is_when_chain(&attr.value)
        {
            cursor = &attr.value;
            continue;
        }
        break;
    }
    let common = common_branch_type(&branch_types)?;
    let any_nullable = branch_types
        .iter()
        .any(|t| t.as_ref().is_some_and(ColumnType::is_nullable));
    Some(if no_otherwise || any_nullable {
        ColumnType::Nullable(Box::new(common.base().clone()))
    } else {
        common
    })
}

/// The common type of a list of `when`/`otherwise` branch types.
///
/// Rules: if every branch shares the same base (Nullable-stripped) type,
/// that type. Numeric branches widen — `int` < `long` < `double`. Any
/// branch with `None` is treated as "unknown but compatible" — it doesn't
/// derail inference, but it can't pin the type down on its own. Returns
/// `None` when the branches are heterogeneous in a way pykrete can't
/// reconcile, so downstream checks stay permissive instead of firing on
/// a fabricated common type.
pub(super) fn common_branch_type(branches: &[Option<ColumnType>]) -> Option<ColumnType> {
    let mut acc: Option<ColumnType> = None;
    let mut saw_known = false;
    for ty in branches {
        let Some(ty) = ty else { continue };
        saw_known = true;
        let base = ty.base().clone();
        acc = Some(match acc {
            None => base,
            Some(prev) => widen_pair(&prev, &base)?,
        });
    }
    if saw_known { acc } else { None }
}

/// Widen two atomic types: equal → that type, numerics widen to the
/// widest (`int` < `long` < `double`). Everything else → `None`.
/// Composite types compare structurally — same-shape arrays/maps/structs
/// keep their shape; differing composites can't be reconciled here.
fn widen_pair(a: &ColumnType, b: &ColumnType) -> Option<ColumnType> {
    if a == b {
        return Some(a.clone());
    }
    // Set-equal enum vocabularies (declaration order differs) — Q4 / Q9
    // reconcile through `enum_vocab_eq`, not derived `PartialEq`.
    if ColumnType::enum_vocab_eq(a, b) {
        return Some(a.clone());
    }
    // Enum vs plain string in branch position — Q9 drops the
    // constraint; the reconciled type is plain `string`.
    if matches!(
        (a.base(), b.base()),
        (ColumnType::Enum(_), ColumnType::String) | (ColumnType::String, ColumnType::Enum(_))
    ) {
        return Some(ColumnType::String);
    }
    // Per-variant widening classifier. Exhaustive (no `_` arm) so a
    // future `ColumnType` variant is a compile error here — the §9 gate.
    // Returns the ladder slot for the widening cases pykrete models
    // today; non-modeled variants are explicit `None`.
    //
    // Scope is intentionally conservative: only the `(Int|Long, Double)`
    // ↔ Double and `(Int, Long)` ↔ Long pairs widen, plus Float against
    // those three (Spark collapses mixed-integral-with-Float to Double,
    // matching the `sum`/`avg` discipline). Byte/Short/Decimal stay
    // outside the ladder until a separate widening design lands.
    enum WideSlot {
        Int,
        Long,
        FloatOrDouble,
        Outside,
    }
    fn widen_slot(t: &ColumnType) -> WideSlot {
        match t {
            ColumnType::Int => WideSlot::Int,
            ColumnType::Long => WideSlot::Long,
            ColumnType::Float | ColumnType::Double => WideSlot::FloatOrDouble,
            ColumnType::Byte
            | ColumnType::Short
            | ColumnType::Decimal { .. }
            | ColumnType::String
            | ColumnType::Enum(_)
            | ColumnType::Binary
            | ColumnType::Bool
            | ColumnType::Date
            | ColumnType::Timestamp
            | ColumnType::Array(_)
            | ColumnType::Map(..)
            | ColumnType::Struct(_) => WideSlot::Outside,
            ColumnType::Nullable(inner) => widen_slot(inner),
        }
    }
    match (widen_slot(a), widen_slot(b)) {
        (WideSlot::FloatOrDouble, WideSlot::Int | WideSlot::Long)
        | (WideSlot::Int | WideSlot::Long, WideSlot::FloatOrDouble) => Some(ColumnType::Double),
        (WideSlot::Long, WideSlot::Int) | (WideSlot::Int, WideSlot::Long) => Some(ColumnType::Long),
        (WideSlot::Int, WideSlot::Int) => Some(ColumnType::Int),
        (WideSlot::Long, WideSlot::Long) => Some(ColumnType::Long),
        // Float+Float short-circuits via `a == b` above; this arm covers
        // Float+Double (and the Nullable-asymmetric cases that classify
        // into FloatOrDouble on both sides) — collapse to Double.
        (WideSlot::FloatOrDouble, WideSlot::FloatOrDouble) => Some(ColumnType::Double),
        (WideSlot::Outside, _) | (_, WideSlot::Outside) => None,
    }
}

/// Build the result type of `F.struct(arg1, arg2, ...)` — a `Struct`
/// whose fields are the args' inferred names and types. The name of each
/// field comes from the same logic that names columns in `select` /
/// `withColumn`: an explicit `.alias("x")` first, otherwise the `col`
/// reference's name. Args without a discoverable name fall back to
/// Spark's positional convention — `col1`, `col2`, … (1-indexed) — so a
/// downstream `.getField("col1")` resolves cleanly and we never mint a
/// bogus empty-name field that `.getField("")` could match against.
fn infer_struct_type<'a>(
    call: &ExprCall,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> ColumnType {
    let fields: Vec<StructField> = call
        .arguments
        .args
        .iter()
        .enumerate()
        .map(|(i, arg)| {
            let name = select_output_name(arg, None)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("col{}", i + 1));
            let ty = select_arg_type(arg, schema, tcx, body);
            StructField { name, ty }
        })
        .collect();
    ColumnType::Struct(fields)
}

/// Build the result type of `F.named_struct("k1", v1, "k2", v2, ...)` —
/// the names are string-literal positional args at even indices, the
/// values are the column expressions at odd indices. If a name slot
/// isn't a string literal pykrete can't resolve it statically, so the
/// whole call degrades to `None` (Unknown) rather than mint a struct
/// with fabricated names.
fn infer_named_struct_type<'a>(
    call: &ExprCall,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> Option<ColumnType> {
    let mut fields: Vec<StructField> = Vec::new();
    let mut iter = call.arguments.args.iter();
    while let Some(name_arg) = iter.next() {
        let name = name_arg
            .as_string_literal_expr()?
            .value
            .to_str()
            .to_string();
        let value_arg = iter.next()?;
        let ty = select_arg_type(value_arg, schema, tcx, body);
        fields.push(StructField { name, ty });
    }
    Some(ColumnType::Struct(fields))
}

/// The pykrete type of a Python literal used as a column value.
/// The type of a struct field — what `<col>.getField(name)` returns.
/// Walks through `Nullable` so an `Optional[Address]` is still
/// navigable. `None` when the receiver isn't a struct, or when the
/// field name doesn't appear, or when the field's type isn't known.
fn struct_field_type(recv_ty: &ColumnType, field: &str) -> Option<ColumnType> {
    match recv_ty {
        ColumnType::Struct(fields) => fields
            .iter()
            .find(|f| f.name == field)
            .and_then(|f| f.ty.clone()),
        ColumnType::Nullable(inner) => struct_field_type(inner, field),
        _ => None,
    }
}

/// The element / value type of an `array` or `map` — what
/// `<col>.getItem(...)` returns. `None` when the receiver isn't a
/// collection or the inner type is unknown.
fn collection_element_type(recv_ty: &ColumnType) -> Option<ColumnType> {
    match recv_ty {
        ColumnType::Array(Some(elem)) => Some(*elem.clone()),
        ColumnType::Map(_, Some(value)) => Some(*value.clone()),
        ColumnType::Nullable(inner) => collection_element_type(inner),
        _ => None,
    }
}

/// Build the result of `<col>.withField(name, value)` — the receiver's
/// struct with `name` added (or its type replaced if already present).
/// If the receiver isn't a struct (Spark would error at runtime), keep
/// the type as-is rather than mint a fictional shape.
fn struct_with_field(recv_ty: &ColumnType, name: &str, value_ty: Option<ColumnType>) -> ColumnType {
    match recv_ty {
        ColumnType::Struct(fields) => {
            let mut out = fields.clone();
            if let Some(existing) = out.iter_mut().find(|f| f.name == name) {
                existing.ty = value_ty;
            } else {
                out.push(StructField {
                    name: name.to_string(),
                    ty: value_ty,
                });
            }
            ColumnType::Struct(out)
        }
        ColumnType::Nullable(inner) => {
            ColumnType::Nullable(Box::new(struct_with_field(inner, name, value_ty)))
        }
        other => other.clone(),
    }
}

/// Build the result of `<col>.dropFields("a", "b", …)` — the
/// receiver's struct with those names removed. Non-struct receivers
/// fall through unchanged (a Spark runtime error, not pykrete's to
/// flag here).
fn struct_drop_fields(recv_ty: &ColumnType, drop: &[&str]) -> ColumnType {
    match recv_ty {
        ColumnType::Struct(fields) => ColumnType::Struct(
            fields
                .iter()
                .filter(|f| !drop.contains(&f.name.as_str()))
                .cloned()
                .collect(),
        ),
        ColumnType::Nullable(inner) => {
            ColumnType::Nullable(Box::new(struct_drop_fields(inner, drop)))
        }
        other => other.clone(),
    }
}

/// Emit `D0030` when `<col>.getField("name")` is called against a
/// known struct that doesn't have `name`. A non-struct receiver
/// (Spark would error at runtime, but pykrete can't always tell) is
/// silenced rather than risk a false positive.
pub(super) fn report_get_field_typo(
    recv_ty: &ColumnType,
    lit: &ruff_python_ast::ExprStringLiteral,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let fields = match recv_ty {
        // An empty fields vec is F4's opaque-Struct sentinel — the user
        // declared `Struct` without specifying the shape. Any field
        // access is permissive; pykrete declines to fire D0030 without
        // knowing what's inside.
        ColumnType::Struct(fields) if !fields.is_empty() => fields,
        ColumnType::Nullable(inner) => {
            return report_get_field_typo(inner, lit, source, line_index, diagnostics);
        }
        _ => return,
    };
    let name = lit.value.to_str();
    if fields.iter().any(|f| f.name == name) {
        return;
    }
    let candidates: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    let suggestion = closest_name(name, &candidates);
    let mut message = format!("Field '{name}' does not exist on the nested struct.");
    if let Some(s) = &suggestion {
        message.push_str(&format!(" Did you mean '{s}'?"));
    }
    diagnostics.push(
        Diagnostic::at_range(
            Severity::Error,
            "D0030",
            message,
            lit.range(),
            source,
            line_index,
        )
        .with_suggestion(suggestion),
    );
}

/// Find the closest match to `target` among `candidates` by Levenshtein
/// distance, with the same `max(1, len/3)` threshold used elsewhere for
/// "Did you mean?" suggestions. Returns `None` if no candidate is close
/// enough.
fn closest_name(target: &str, candidates: &[&str]) -> Option<String> {
    let threshold = std::cmp::max(1, target.len() / 3);
    let mut best: Option<(&str, usize)> = None;
    for &candidate in candidates {
        let d = crate::schema::levenshtein(target, candidate);
        if d > threshold {
            continue;
        }
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((candidate, d));
        }
    }
    best.map(|(name, _)| name.to_string())
}

/// Output type of `coalesce(c1, c2, ...)` / `nvl(c1, c2)` / `ifnull(c1,
/// c2)` per Q9: every branch's type is inferred and reconciled. If
/// every branch is enum-typed and the vocabularies are set-equal, the
/// output preserves the constraint. If any branch is plain `string`,
/// the constraint drops and the output is plain `string`. Non-set-equal
/// enum vocabularies degrade to `None`; D0040 is emitted at the call
/// site by [`report_branch_form_enum_conflicts`] (the
/// schema-merge surface and the branch surface share the code).
///
/// Behavior change vs PR-B round 1: the type seed is the first non-`None`
/// inferred branch (rather than only the very first arg). This keeps a
/// reconciled enum constraint flowing through `coalesce(unknown,
/// col(enum), ...)` instead of dropping it when the leading arg is a
/// constant or otherwise un-typed.
pub(super) fn coalesce_branch_result<'a>(
    call: &ExprCall,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> Option<ColumnType> {
    let branch_types: Vec<Option<ColumnType>> = call
        .arguments
        .args
        .iter()
        .map(|a| select_arg_type(a, schema, tcx, body))
        .collect();
    let mut iter = branch_types.iter().flatten();
    let first = iter.next()?.base().clone();
    let mut all_enum = matches!(first, ColumnType::Enum(_));
    let mut any_plain_string = matches!(first, ColumnType::String);
    let mut vocabs_set_equal = true;
    for ty in iter {
        let base = ty.base();
        match base {
            ColumnType::Enum(_) => {
                if !ColumnType::enum_vocab_eq(&first, base) {
                    vocabs_set_equal = false;
                }
                if !matches!(first, ColumnType::Enum(_)) {
                    all_enum = false;
                }
            }
            ColumnType::String => {
                any_plain_string = true;
                all_enum = false;
            }
            _ => {
                all_enum = false;
            }
        }
    }
    if any_plain_string && matches!(first, ColumnType::Enum(_)) {
        return Some(ColumnType::String);
    }
    if all_enum && !vocabs_set_equal {
        return None;
    }
    Some(first)
}

/// Per spec Q9: when every branch of a `coalesce` / `nvl` / `ifnull` /
/// `when`-chain expression resolves to an enum-typed column but the
/// vocabularies are NOT set-equal, fire `D0040`. Symmetric with the
/// `Merge[A, B]` shared-column enum mismatch on the schema-merge surface
/// (same diagnostic code, distinct anchor — here the call expression).
///
/// Mixed enum-and-plain-`string` branches are silent: the user
/// explicitly mixed a `string` source in, so the constraint is
/// intentionally dropped to plain `string` (no D0040). Set-equal vocabs
/// are also silent; the constraint is preserved.
pub(super) fn report_branch_form_enum_conflicts<'a>(
    expr: &Expr,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(call) = expr.as_call_expr() else {
        return;
    };
    let fname = match call.func.as_ref() {
        Expr::Name(n) => n.id.as_str(),
        Expr::Attribute(a) => a.attr.id.as_str(),
        _ => return,
    };
    let branch_types: Vec<Option<ColumnType>> = if matches!(fname, "coalesce" | "nvl" | "ifnull") {
        call.arguments
            .args
            .iter()
            .map(|a| select_arg_type(a, schema, tcx, body))
            .collect()
    } else if fname == "when" || (fname == "otherwise" && is_when_chain_attr_value(call)) {
        collect_when_chain_branch_types(expr, schema, tcx, body)
    } else {
        return;
    };
    fire_d0040_if_disjoint_enums(&branch_types, expr.range(), source, line_index, diagnostics);
}

/// True for an `.otherwise(...)` call whose receiver is itself a
/// `when`-chain — i.e. a real `F.when(...).otherwise(...)` shape rather
/// than an unrelated `.otherwise` method on some other object.
fn is_when_chain_attr_value(call: &ExprCall) -> bool {
    let Some(attr) = call.func.as_attribute_expr() else {
        return false;
    };
    is_when_chain(&attr.value)
}

/// Collect each value-branch's inferred type from a
/// `F.when(p, v).when(p2, v2)…[.otherwise(e)]` chain. Walks the chain
/// from the outermost call down to the root `when`, gathering every
/// value-position expression's type.
fn collect_when_chain_branch_types<'a>(
    chain: &Expr,
    schema: &SchemaView<'a>,
    tcx: TypeCtx<'a>,
    body: &BodyContext<'a>,
) -> Vec<Option<ColumnType>> {
    let mut out: Vec<Option<ColumnType>> = Vec::new();
    let mut cursor = chain;
    loop {
        let Some(call) = cursor.as_call_expr() else {
            return out;
        };
        let fname = match call.func.as_ref() {
            Expr::Name(n) => n.id.as_str(),
            Expr::Attribute(a) => a.attr.id.as_str(),
            _ => return out,
        };
        if fname == "otherwise" {
            if let Some(arg) = call.arguments.args.first() {
                out.push(infer_expr_type(arg, schema, tcx, body));
            }
            let Some(attr) = call.func.as_attribute_expr() else {
                return out;
            };
            cursor = &attr.value;
            continue;
        }
        if fname != "when" {
            return out;
        }
        if let Some(v) = call.arguments.args.get(1) {
            out.push(infer_expr_type(v, schema, tcx, body));
        }
        if let Some(attr) = call.func.as_attribute_expr() {
            cursor = &attr.value;
            continue;
        }
        return out;
    }
}

/// If every branch type is enum-typed and the vocabularies are NOT
/// set-equal pairwise, push a single `D0040` anchored on `range`. Mixed
/// enum-vs-plain-string is silent (intentional drop). One diagnostic
/// per call expression is enough — the user's fix is to reconcile the
/// branches, not to chase per-pair messages.
fn fire_d0040_if_disjoint_enums(
    branch_types: &[Option<ColumnType>],
    range: TextRange,
    source: &str,
    line_index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let knowns: Vec<&ColumnType> = branch_types.iter().flatten().collect();
    if knowns.len() < 2 {
        return;
    }
    if !knowns
        .iter()
        .all(|t| matches!(t.base(), ColumnType::Enum(_)))
    {
        return;
    }
    let first = knowns[0].base();
    if knowns
        .iter()
        .all(|t| ColumnType::enum_vocab_eq(first, t.base()))
    {
        return;
    }
    // Spec Q9: one D0040 per top-level branch form. `report_branch_form_enum_conflicts`
    // runs at every Expr::Call descent, so a nested chain like
    // `when(p1, a).when(p2, b).otherwise(c)` would re-match on the inner
    // `.when(p2, b)` sub-chain after firing on the outer `.otherwise(...)`.
    // Both share the same start position (chains anchor on their root call),
    // so dedupe by start line/column.
    let start = line_index.line_column(range.start(), source);
    let (start_line, start_col) = (start.line.get(), start.column.get());
    if diagnostics
        .iter()
        .any(|d| d.code == "D0040" && d.line == start_line && d.column == start_col)
    {
        return;
    }
    let message = "Branch-form expression reconciles enum-typed branches with non-set-equal \
         vocabularies. Reconcile by widening one side's vocabulary to match the \
         other, or cast a branch to plain string to drop the constraint."
        .to_string();
    diagnostics.push(Diagnostic::at_range(
        Severity::Error,
        "D0040",
        message,
        range,
        source,
        line_index,
    ));
}

fn python_literal_type(expr: &Expr) -> Option<ColumnType> {
    match expr {
        Expr::StringLiteral(_) => Some(ColumnType::String),
        Expr::BooleanLiteral(_) => Some(ColumnType::Bool),
        Expr::NumberLiteral(n) => match n.value {
            Number::Int(_) => Some(ColumnType::Int),
            Number::Float(_) => Some(ColumnType::Double),
            Number::Complex { .. } => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // widen_pair's same-slot arm previously collapsed every matched pair
    // to Double, hiding behind the `a == b` short-circuit and the live
    // caller's `.base().clone()` normalization. Calling widen_pair with
    // a Nullable on one side bypasses both guards and pins the per-slot
    // disposition directly — Int stays Int, Long stays Long.
    #[test]
    fn widen_pair_same_slot_preserves_int_not_double() {
        let nint = ColumnType::Nullable(Box::new(ColumnType::Int));
        assert_eq!(widen_pair(&nint, &ColumnType::Int), Some(ColumnType::Int));
        let nlong = ColumnType::Nullable(Box::new(ColumnType::Long));
        assert_eq!(
            widen_pair(&nlong, &ColumnType::Long),
            Some(ColumnType::Long)
        );
        // Float+Double still collapses to Double — the only non-short-
        // circuit FloatOrDouble pair lands on the explicit arm.
        assert_eq!(
            widen_pair(&ColumnType::Float, &ColumnType::Double),
            Some(ColumnType::Double),
        );
    }
}
