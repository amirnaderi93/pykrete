# Architecture

How dathon is organized. dathon is a static schema checker for PySpark —
`.dpy` files are a strict superset of Python.

## Workspace

Two crates in one Cargo workspace:

- **`dathon`** ([`crates/dathon/`](../../crates/dathon/)) — the checker. A
  library (the analysis is a library so editors can embed it) plus the
  CLI binary `dathon`.
- **`dathon-lsp`** ([`crates/dathon-lsp/`](../../crates/dathon-lsp/)) — the
  Language Server. An LSP *multiplexer*: it wraps the `dathon` library and
  an embedded Python language server, so the editor talks to one server
  and gets both schema checking and full Python support. See
  [multiplexer.md](multiplexer.md).

## Pipeline

The checker is no longer a single linear pass — it parses, builds
registries, then runs a recursive body analysis with schema inference and
type checking.

```
.dpy source
  → ruff_python_parser           (Python AST — ModModule)
  → walk                         (discover top-level classes + functions)
  → schema / dataframe / registry (Schema classes, DataFrame[X] slots,
                                   classes/methods/functions/UDFs/constants)
  → operations                   (per-function body analysis: operation
                                   modeling, result-schema inference,
                                   column-existence + column-type checks)
  → diagnostics                  (TypeScript-style: path:line:col - sev code: msg)
```

Cross-file resolution (imports, shared schemas) happens in `lib`, which
pools declarations across the project before analyzing each file.

## Modules in `dathon`

### `diagnostics`

`Diagnostic` — severity, code, message, eagerly-computed line/column.
`CheckMode` (`off` / `basic` / `standard` / `strict`) mirrors the embedded
Python engine's `typeCheckingMode`; each diagnostic carries a `min_mode`
and the driver keeps only those the active mode surfaces. `format()` emits
the TypeScript-style line `path:line:col - severity code: message`.

### `walk`

Read-only AST discovery — `discover_top_level_classes`,
`discover_top_level_functions`.

### `schema`

Schema classes (a Python class whose bases include `Schema`) and field
resolution. `SchemaField::resolve` produces a `FieldResolution`;
`field_type` resolves a field's full [`ColumnType`], recursively — nested
`array` / `map` element types and `struct` fields included, descending
into referenced `Schema` classes (depth-guarded against a cyclic schema).

`SchemaView` is the unified view used during analysis — `Declared(&Schema)`,
`Derived(Vec<DerivedField>)` (a schema inferred from an operation, each
field carrying its name and inferred type), or `Grouped { keys, underlying }`
(a post-`groupBy` intermediate).

`resolve_path` walks a dotted column path — `col("orders.line.sku")` — through
nested structs, piercing `array<struct>` as Spark does. `suggest_field_name`
powers the "did you mean?" hints.

### `types`

`ColumnType` — dathon's type system. The atomic vocabulary (`int`, `long`,
`double`, `string`, `bool`, `date`, `timestamp`) plus the composites
`Array`, `Map`, and `Struct`, which nest arbitrarily. Schema fields are
written as ordinary Python type annotations — a bare name for an atomic
type or a referenced `Schema` class, a subscript for a collection
(`Array[int]`, `Map[string, Event]`) — which `schema` resolves off the
AST. `from_spark_name` parses the string form Spark's `.cast("…")` and
UDF `returnType` still use.

### `registry`

Per-file registries built before body analysis: classes and their methods
(with PEP 695 generic type parameters), top-level functions, annotated
constants (`NAME: DataSource[Schema] = …`, including class-body
constants), and UDFs (`@udf` / `@pandas_udf` decorators and the functional
`udf(f, …)` form, mapped to their return type).

### `dataframe`

Recognizes `DataFrame[X]` annotations on function signatures —
`Untyped` / `Typed("Foo")` / `NonBareName` — and the typed parameter /
return slots.

### `imports`

`from X import Y [as Z]` resolution — relative and absolute paths, `as`
aliases — anchored at the project root (`pyproject.toml`, with a
longest-common-ancestor fallback).

### `operations`

The heart of the checker — body analysis with result-schema inference, so
chained calls and local bindings carry their schemas forward.

`BodyContext` resolves names to schemas (typed parameters, `x = …`
bindings, annotated assignments, top-level constants, class instances).
`analyze_expr` recursively walks an expression, returns a `SchemaView`
when it evaluates to a DataFrame, and along the way checks every
recognized call against the receiver's schema.

**Operations modeled** — `select`, `filter`/`where`, `withColumn`,
`withColumns`, `withColumnRenamed`/`withColumnsRenamed`, `drop`,
`dropDuplicates`, `dropna`, `groupBy`/`cube`/`rollup` + `agg`, `pivot`,
`join`/`crossJoin`/`union`/`unionByName`, `selectExpr`, `transform`,
`toDF`, the `df.na.*` family, `Window` key checking, the schema-cast
`.cast(DataFrame[Schema])`, and a wide pass-through set
(`persist`/`cache`/`orderBy`/`limit`/…).

**Column-type inference** — `infer_expr_type` works out the type of a
column expression (`col(...)`, `.cast(...)`, `F.lit(...)`, literals, and a
`pyspark.sql.functions` result catalog with type transforms like
`collect_list(T) → array<T>` and `explode(array<T>) → T`). Result builders
carry inferred types into `Derived` schemas.

**Embedded SQL** — column references inside `F.expr("…")`, `selectExpr("…")`,
and string-form `filter("…")` are parsed (see `sql`) and checked.

### `sql`

Best-effort parsing of the SQL strings PySpark embeds — column references
inside `F.expr`/`selectExpr`/string-`filter` (via `sqlparser`), and the
projection columns of a `spark.sql("SELECT …")` query.

### `hover` / `completion` / `symbols`

Position-aware backends for the LSP — hover, completions, and document
symbols. Each takes a parsed module and a `(line, column)` and returns the
LSP payload.

### `transpiler`

`.dpy` → `.py`. `.dpy` is a strict superset of Python, so this is nearly
an identity transform — it prepends `from __future__ import annotations`
(so dathon's atomic type names and `DataFrame[X]` annotations don't
evaluate at runtime) and strips the schema-cast `.cast(DataFrame[Schema])`
— the one dathon-only construct in *expression* position, which the
Python runtime has no `.cast` method for. Stripping is AST-located but
byte-surgical: only the `.cast(…)` slice is deleted, line numbers are
preserved, everything else is copied verbatim.

### `lib`

The driver. `check` / `check_project` parse, build the pooled cross-file
registries, run `analyze_module` per file, and filter diagnostics by
`CheckMode`. Also the entry points the LSP layer calls for hover,
completion, definition, and symbols.

### `main`

CLI — `dathon check <files…>` and `dathon transpile <file>`.

## Diagnostic codes

| Code | Meaning |
| --- | --- |
| `D0001` | Parse error. |
| `D0010` / `D0011` | Unknown column type / type is not a bare name. |
| `D0020` / `D0021` | Unknown schema in `DataFrame[…]` / schema is not a bare name. |
| `D0030` | Column does not exist on the schema. |
| `D0040` | `union` / `unionByName` schema mismatch. |
| `D0050` | Return type mismatch — column *set* differs from the declared schema. |
| `D0060` | Join key missing on one side. |
| `D0070` / `D0071` | Unresolved import / name not exported by a module. |
| `D0080` | Return type mismatch — a column's *type* differs (conservative; on by default). |
| `D0081` / `D0082` | Arithmetic on a non-numeric column / comparison of unrelated types (advisory; `typeCheckingMode: strict` only). |

## Column-type checking

dathon checks both column *existence* (`D0030`) and column *types*.
Type checking is split by strictness:

- **Conservative** (default) — `D0080` flags a declared-return type
  mismatch only when both types are confidently known and genuinely
  incompatible. Unknown types are permissive; numeric widening
  (`int`/`long`/`double`) is accepted.
- **Strict** (`typeCheckingMode: strict`) — `D0081`/`D0082` additionally
  flag type combinations Spark *coerces* rather than rejects. They are
  emitted at `min_mode: Strict`, so the driver surfaces them only in
  strict mode.

## Multi-file analysis

`check_project` parses every file, pools every Schema / class / constant /
function (visible per-file through `from X import Y`), and analyzes each
file against the pooled view. Column references, generic-method
substitution, and return-type checks all resolve cross-file.

## External dependencies

Git-pinned to `astral-sh/ruff` at tag `0.15.12`: `ruff_python_parser`,
`ruff_python_ast`, `ruff_source_file`, `ruff_text_size`. Plus `sqlparser`
(embedded-SQL parsing) and, for `dathon-lsp`, `lsp-server` / `lsp-types` /
`crossbeam-channel`.

## Decisions in effect

- **Language: Rust.**
- **Parser: ruff's** — saves a year of front-end work, tracks PEPs, and is
  the same AST Astral's `ty` is built on (so the analyzer ports cleanly if
  dathon ever forks `ty`).
- **`.dpy` is a strict superset of Python** — every `.dpy` file parses
  with ruff's Python parser as-is.
- **The checker is a library** — the CLI and the LSP both embed it.
- **TypeScript is the design north star** — adapted, not copied, for a
  pipeline-heavy, coercion-happy language.

See [v0.1-spec.md](../v0.1-spec.md) for the original user-facing contract.
