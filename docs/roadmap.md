# Roadmap

What's planned, in rough priority order. A living document — updated as the
project moves.

## Where dathon is now

The **PySpark static checker is feature-complete**:

- The full DataFrame operation surface — `select` / `filter` / `join` /
  `groupBy`+`agg` / `withColumn(s)` / `drop` / `union` / `cube` / `rollup` /
  `pivot` / `transform` / `cast` / `toDF` / `df.na.*` / … — with
  result-schema inference through whole transformation chains.
- Inline SQL (`F.expr`, `selectExpr`, string-`filter`) and raw
  `spark.sql("SELECT …")`.
- `Window` partition/order key checking.
- Column-**existence** checking (`D0030`) and column-**type** checking —
  conservative (`D0080`, on by default) and strict (`D0081`/`D0082`, under
  `typeCheckingMode: strict`).
- Arbitrarily-nested `array` / `map` / `struct` column types — declared,
  structurally type-checked, and navigated field-by-field
  (`col("orders.line.sku")`).
- A `pyspark.sql.functions` result catalog and UDF return types.
- Cross-file imports and shared-schema modules.

The **LSP server** delivers live diagnostics, hover, completion, document
symbols, and go-to-definition, and embeds a Python language server (an
LSP multiplexer — see [design/multiplexer.md](design/multiplexer.md)). The
**VS Code extension** ([../editors/vscode/](../editors/vscode/)) wraps it.

## Next: editor features

- **Autocomplete completeness** — column-name completion in bare-string
  arguments (`.select("…")`, `.groupBy("…")`, `.join(on="…")`,
  `.drop("…")`, `.withColumnRenamed("…")`), and on chain results
  (`raw.select(...).<cursor>`).
- **Find references**, **rename**, **semantic tokens** — broader LSP
  coverage.

## Transpiler

`.dpy` → `.py` is nearly an identity transform — it prepends
`from __future__ import annotations` so dathon's atomic type names and
`DataFrame[X]` annotations don't evaluate at runtime. Remaining: strip the
dathon-only constructs the Python runtime doesn't have — notably the
fluent schema-cast `.cast(DataFrame[Schema])`, which the transpiler must
remove from the call chain.

## PyCharm support

A JetBrains integration via PyCharm's LSP client. Deferred well past
pandas (v2) and polars (v3) — VS Code is the only supported editor for now.

## Strictness configuration

`typeCheckingMode` (`off` / `basic` / `standard` / `strict`) is wired and
drives both dathon's checks and the embedded Python engine. A fuller
`dathon.json` — file/dir excludes, per-rule severity overrides — is a
follow-up.

## Generic-inference extensions

dathon infers generic-function results for the simplest shape: one type
variable in both a parameter slot `GenericClass[T]` and a return slot
`GenericClass[T]`. Larger patterns, listed so they're not forgotten:

- **Multiple type parameters** — `def join[A, B](left: DataFrame[A], right: DataFrame[B]) -> DataFrame[Joined[A, B]]`.
- **Nested generics** — `List[DataSource[T]]`; the matcher handles one subscript level only.
- **Chained class-method calls** — `builder.with_path("/x").read(SOURCE)`; only direct calls on a class-instance name dispatch through generic inference.
- **Generic methods that aren't `[T] -> G[T]`-shaped** — e.g. `def cast_to[T](self, _: type[T]) -> DataFrame[T]`, where `T` is bound from a value of static type `type[T]`.

## Quality-of-life

- **Packaging** — `cargo install` + a Homebrew tap; marketplace publishing
  for the VS Code extension (distributed as a local `.vsix` today).
- **Editor-agnostic LSP docs** — setup snippets for Neovim, Helix, Zed,
  Emacs.
- **Performance pass** — benchmark on large codebases; today every
  `dathon check` reparses the whole project.
- **Duplicate-name detection** across files.

## Strategic direction

These are larger structural moves, not increments.

### Multi-dataframe support (pandas, polars, …)

PySpark is the v1 target, but every dataframe library has the same shape:
a value carries a schema, methods narrow or widen it, column names must
exist when referenced. Schema checking is valuable for every one.

Priority: **PySpark → pandas → polars** → others (DuckDB, Dask, …).

The core type model — `DataFrame[Schema]`, the `Schema` class, column
checks, return-type validation — generalizes. The library-specific layer
is method dispatch (`raw.select(col("x"))` vs `raw[["x"]]` vs
`raw.select(pl.col("x"))`). This argues for a plugin/dispatch model for
operation handling **before** pandas support accumulates more
PySpark-specific code in `operations`.

### Forking `ty`

Long term, dathon may fork Astral's `ty` (their Rust Python type checker)
once it reaches a stable release — a single native stack, replacing the
basedpyright multiplexer. Because dathon's analyzer is already built on
`ruff_python_ast` (the AST `ty` uses), the schema-checking core ports
cleanly; the multiplexer is interim scaffolding by design.
