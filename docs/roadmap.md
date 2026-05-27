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

- **Date/time first-arg column checking + array higher-order function
  recognition.** The single-column date helpers — `F.to_date`,
  `F.to_timestamp`, `F.date_format`, `F.trunc`, `F.next_day`,
  `F.from_utc_timestamp`, `F.to_utc_timestamp`, `F.from_unixtime`,
  `F.unix_timestamp`, and the position-2 variant `F.date_trunc(format,
  col)` — now flag `D0030` on a typo in the column slot while the
  format / timezone string is left alone. `date_format` and
  `from_unixtime` joined the typed result catalog (→ string). The array
  higher-order functions `F.transform`, `F.filter`, `F.aggregate`,
  `F.exists`, `F.forall` are recognized — first-arg column ref checked,
  return type modeled per function (`array<lambda body>` for
  `transform`, input array preserved for `filter`, lambda body type for
  `aggregate`, bool for `exists` / `forall`). Lambda bodies are
  inferred best-effort and fall back to Unknown when not traceable.
- **`F.when` / `F.otherwise` result-type inference and `F.struct` /
  `F.named_struct` schema construction.** `F.when(p, v).otherwise(e)`
  chains now infer their result as the common type of the value
  branches (atomic equality, then numeric widening — `int` < `long` <
  `double`); chains without `.otherwise(...)` resolve to `Nullable(T)`
  since unmatched rows yield null. `F.struct(col("a"), col("b"))` now
  produces a `Struct({a: int, b: string})` whose field names come from
  `.alias("x")` first then the column name, and whose types are each
  arg's inferred type; `F.named_struct("k1", v1, "k2", v2)` uses the
  string-literal name slots as field names. Composes with `.getField`,
  so a freshly-constructed struct can be navigated immediately.
- **`createOrReplaceTempView` + `spark.sql("SELECT … FROM view")`
  resolution.** `df.createOrReplaceTempView("v")` registers `df`'s
  schema against the view name in a per-file registry; a subsequent
  `spark.sql("SELECT … FROM v")` in the same file checks every column
  identifier in the query (projection, `WHERE`, `GROUP BY`, `ORDER BY`,
  `HAVING`) against the view's schema, firing `D0030` on a typo, and
  returns either the projected columns or the view's full schema for
  `SELECT *`. Single-table SELECT only, within-file only.
- **`Column` method chain recognition.** `.isNull` / `.isNotNull` /
  `.isin` / `.between` / `.like` / `.rlike` / `.ilike` / `.contains` /
  `.startswith` / `.endswith` are now recognized as boolean-returning
  Column predicates that preserve the chain; `.getField` resolves the
  nested struct field's type and fires `D0030` on a field-name typo;
  `.getItem` returns the array element / map value type;
  `.withField` and `.dropFields` track the receiver's struct shape
  forward with the field added, replaced, or removed.
- **Set ops, `F.broadcast`, and terminal recognizers.** `intersect`,
  `intersectAll`, `subtract`, `exceptAll` are recognized set operations
  sharing the same schema-mismatch check (`D0040`) as `union` /
  `unionByName`; `unionAll` is wired as a deprecated alias for `union`.
  `F.broadcast(df)` is treated as a pass-through, so chains like
  `df1.join(F.broadcast(df2), "k")` keep tracking the schema. The nine
  terminal methods (`count`, `collect`, `show`, `printSchema`, `explain`,
  `first`, `take`, `head`, `tail`) are recognized centrally — the chain
  dies cleanly and a future "chain-after-terminal" diagnostic has a
  single seam to attach to.
- **`spark.read.<format>(path)` / `spark.table(name)` opaque-source
  recognition.** `DataFrameReader` chains (`spark.read.parquet(...)`,
  `spark.read.format(...).load(...)`, `spark.read.schema(...).<format>(...)`)
  and bare `spark.table(...)` are now recognized as opaque IO sources.
  The result is still Unknown — the schema is genuinely runtime data —
  but the user re-anchors the chain with `.cast(DataFrame[Schema])` or a
  typed variable annotation (`raw: DataFrame[Schema] = spark.read.parquet(...)`)
  and downstream column checks resume. Closes the headline gap where
  real PySpark codebases lost their chain at line one.
- **Call-site argument checking** (`D0051 argumentColumnsMismatch`) —
  closes the function boundary on the input side. Passing a
  `DataFrame[Wrong]` into a function that declares `DataFrame[Right]`
  is now flagged at the call site, with the same missing / extra column
  reporting as `returnColumnsMismatch`. v0.1.8 closes the edge cases:
  local-name shadowing of a top-level function suppresses the check —
  including tuple-unpack (`revenue, _ = …`) and walrus (`(revenue := …)`)
  rebinds; positional-only (`/`) and keyword-only (`*`) parameter
  markers are honored when matching arguments; `*args` / `**kwargs`
  variadics are checked against every call-site argument routed to
  them; and a parameter filled both positionally and by keyword
  (Python's `TypeError`) is diagnosed once, not twice.
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

