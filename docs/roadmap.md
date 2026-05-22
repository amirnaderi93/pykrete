# Roadmap

What's planned, in rough priority order. A living document — updated as the
project moves.

## Where pykrete is now

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

The **LSP server** delivers live diagnostics, hover, completion (column
names in bare-string arguments and on chain results), document symbols,
go-to-definition, find-references, rename, and semantic tokens, and embeds
a Python language server (an LSP multiplexer — see
[design/multiplexer.md](design/multiplexer.md)). The **VS Code extension**
([../editors/vscode/](../editors/vscode/)) wraps it.

The **`.pyk` → `.py` transpiler** is complete: it prepends
`from __future__ import annotations` (so pykrete's atomic type names and
`DataFrame[X]` annotations don't evaluate at runtime) and strips the
schema-cast `.cast(DataFrame[Schema])` — the one pykrete-only construct in
expression position, which the Python runtime has no `.cast` method for.

## PyCharm support

A JetBrains integration via PyCharm's LSP client. Deferred well past
pandas (v2) and polars (v3) — VS Code is the only supported editor for now.

## Configuration

A `pykrete.json` at (or above) the project root configures both the CLI
and the LSP — `typeCheckingMode` (`off` / `basic` / `standard` /
`strict`), `exclude` (path substrings to skip), and `rules` (per-rule
overrides — `off` / `warning` / `error`, keyed by readable rule name).
For the LSP, `pykrete.json`'s `typeCheckingMode` overrides the editor's
setting; the single value also drives the embedded Python engine.

## Generic-inference extensions

pykrete infers generic-function results for the simplest shape: one type
variable in both a parameter slot `GenericClass[T]` and a return slot
`GenericClass[T]`. Larger patterns, listed so they're not forgotten:

- **Multiple type parameters** — `def join[A, B](left: DataFrame[A], right: DataFrame[B]) -> DataFrame[Joined[A, B]]`.
- **Nested generics** — `List[DataSource[T]]`; the matcher handles one subscript level only.
- **Chained class-method calls** — `builder.with_path("/x").read(SOURCE)`; only direct calls on a class-instance name dispatch through generic inference.
- **Generic methods that aren't `[T] -> G[T]`-shaped** — e.g. `def cast_to[T](self, _: type[T]) -> DataFrame[T]`, where `T` is bound from a value of static type `type[T]`.

## Call-site argument checking

pykrete checks a function's body against its `DataFrame[Schema]`
parameter, and its return value against the declared return type — but
it does **not** yet check the *arguments at a call site*. Passing a
`DataFrame[Refund]` into a function that declares a `DataFrame[Sale]`
parameter goes unflagged.

Planned: a new diagnostic that, at each call to a typed function,
verifies the argument's schema against the parameter's — reporting
missing and extra columns, the way return-type checking
(`returnColumnsMismatch`) already does for the output side. This closes
the function boundary in both directions: what goes in as well as what
comes out.

## Quality-of-life

- **User-facing language reference** ([`language-reference/`](language-reference/))
  — schema syntax, operation reference, error catalog, configuration,
  cookbook. The doc lives but is empty.
- **Performance pass** — benchmark on large codebases; today every
  `pykrete check` reparses the whole project.
- **Duplicate-name detection** across files.
- **Zed extension** — Neovim, Helix and Emacs setups are wired in
  [`editors/`](editors/); Zed needs a dedicated extension.

Already shipped (recorded here for completeness):

- **Packaging.** GitHub Releases with prebuilt binaries for macOS
  (arm64/x64), Linux x64, and a Windows MSI installer; a Homebrew tap
  (`brew install amirnaderi93/pykrete/pykrete`); `cargo install --git`.
  Each release ships through the release workflow automatically.
- **VS Code extension on both registries.** The Visual Studio Marketplace
  (for VS Code) and the Open VSX Registry (for Cursor, VSCodium,
  code-server, Theia). A `.vsix` is also attached to every release for
  side-loading.
- **Editor-agnostic LSP setup docs** for Neovim, Helix and Emacs in
  [`editors/`](editors/).

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

Long term, pykrete may fork Astral's `ty` (their Rust Python type checker)
once it reaches a stable release — a single native stack, replacing the
basedpyright multiplexer. Because pykrete's analyzer is already built on
`ruff_python_ast` (the AST `ty` uses), the schema-checking core ports
cleanly; the multiplexer is interim scaffolding by design.
