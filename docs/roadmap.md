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
`SparkFrame[X]` / `PandasFrame[X]` / `DataFrame[X]` annotations don't
evaluate at runtime) and strips the schema-cast
`.cast(SparkFrame[Schema])` / `.cast(PandasFrame[Schema])` /
`.cast(DataFrame[Schema])` — the one pykrete-only construct in expression
position, which the Python runtime has no `.cast` method for.

**Pandas check-site coverage shipped in v1.3** alongside PySpark: the six
dispatched operations (`df[col_list]` / `df[mask]` / `df["new"] = expr` /
`df.drop` / `df.merge` / `df.rename`), `PandasFrame[X]` annotations, and
the `D0090 deprecatedDataFrameAlias` warning that nudges existing
`DataFrame[X]` users toward the dialect-specific spellings.

**Pandas depth shipped in v1.4**: seven new pandas-heavy donors in
pykrete-tests (scikit-learn, statsmodels, pandera, Great Expectations,
prophet, seaborn, yfinance — 3 direct-dispatch + 4 canonical-fixture-only),
bringing pandas-coverage donor count from 3 to 10; positive
`PROBE-TYPE-IS` coverage on `PandasFrame[X]` (21 markers across the 7 new
donors — 3 per donor, exactly meeting the v1.4 spec §1 floor); and three
checker bug closures (registry-call §10 widening,
`inherited_dialect` walrus receivers, `.transform(helper)` dialect
preservation) that close silent-pass paths surfaced by v1.3 audits.
`pykrete.json` config-discovery now walks from the input file's parent
directory (file-anchored, falling back to CWD) so absolute-path
invocations from outside the project root pick up the config.

**Cross-dialect handoff shipped in v1.5**: `df.toPandas()` re-tags
`SparkFrame[X]` to `PandasFrame[X]`; `spark.createDataFrame(pdf)`
re-tags `PandasFrame[Y]` back to `SparkFrame[Y]` when a `schema=`
keyword argument or a typed call-arg resolves to a known schema; the
round-trip path (`spark.createDataFrame(df.toPandas())`) preserves the
tag end-to-end. Pandas `.head()` / `.tail()` / `.first()` are
dialect-gated as Spark-only terminals so chains downstream of
`pdf.head(10).merge(other, on="id")` keep tracking. The v1.3 promise
of `.loc[:, "col"]` literal-form lands. Two PR-F1-class sibling gates
close (`column_name_arg` ungated arms + `collect_col_refs`
cross-DataFrame routing leak). A new `pykrete check --report-aliases`
flag emits a structured JSON envelope of every `DataFrame[X]`
annotation site with its resolved dialect, so projects can quantify
the v2.0 migration scope before v1.6's `pykrete migrate` ships. The
LSP synthetic-pool gets a soft cap with one-shot warning and
saturation sentinel, closing the v1.4 architecture-audit I4 finding.

**`pykrete migrate` + D0090 strict-mode escalation shipped in v1.6**:
`pykrete migrate` is the auto-rewriter binary for the deprecated
`DataFrame[X]` alias. It walks each `.pyk` file under each input
path, locates every `DataFrame[X]` annotation site via the
`AliasSite` byte-range model, applies call-graph dialect
adjudication to each binding's downstream usage (Spark-only methods
like `withColumn` / `createOrReplaceTempView` / `repartition` →
**Spark**; pandas-only methods like `assign` / `pivot_table` /
`.loc` / `.iloc` / pandas `merge` / `rename(columns=...)` →
**pandas**; both signals → **Ambiguous**; no signal → defaults to
Spark), and rewrites the annotation in place token-preservingly —
atomic per file (sibling temp + rename) so an interrupted run never
leaves half-rewritten source. Three modes: `pykrete migrate src/`
rewrites in place; `pykrete migrate --check src/` previews per-site
verdicts to stdout (exit 1 if any site needs attention, 0
otherwise — CI-ready); `pykrete migrate --diff src/` emits a
`patch -p1`-compatible unified diff. Ambiguous sites get an
idempotent `# pykrete: ambiguous` marker injected on the line above
the unchanged annotation; re-runs don't accumulate duplicates.
Paired atomically with `D0090 deprecatedDataFrameAlias` escalation:
under `"typeCheckingMode": "strict"` the warning now lands as
**error**, but the fix-button ships in the same release so
strict-mode users on green CI aren't stranded. Non-strict modes keep
the warning unchanged. Pandas `pivot_table(index=, columns=,
values=, aggfunc=)` literal-form column checking ships as the
v1.6 pandas reshape downpayment — string-literal arguments and
list-of-literals shapes resolve against `PandasFrame[X]`'s schema,
firing D0030 with a *did you mean*; variable arguments and callable
`aggfunc` fall through. Two v1.5 deferrals close: `.take()` is now
dialect-gated (pandas `pdf.take([0, 2])` passes through instead of
dying as a Spark terminal), and the `pdf.loc[mask, "col"]`
nested-arg D0030 false positive on the row-mask side closes. Plus
audit-debt closure: the `cross_dialect_handoff_gate` recognizer the
v1.5 PR-A1/PR-A2 inference left as a "Keep in sync" comment is
extracted to a single shared site.

## Next up

### v1.7 — pandas reshape + `.loc` / `.iloc` non-literal forms + `.query` / `.eval` mini-DSLs

Broader pandas reshape: `melt` / `stack` / `unstack` /
`groupby.agg` / `reset_index` / `set_index`, plus full
`pivot_table` schema-tracking (the wide output schema — variable
column values become column names of the result frame). `.loc`
non-literal forms (`.loc[mask, "col"]` boolean-mask row keys,
`.loc[:, "a":"b"]` column-range slicing) and `pdf.iloc[...]`
integer-position indexing. The `df.query("…")` and `df.eval("…")`
mini-DSLs (numexpr-influenced syntax, separate parser from the SQL
path used by `selectExpr`). `pd.read_csv(...)` and other pandas
I/O entry points if scope allows (schema inference from file
headers / SQL / type-stubs is a separate design surface).
Retrofitting pandas `PROBE-TYPE-IS` to the v1.3 hybrid donors
(MLflow, Feast, iceberg-python). Canonical-vs-direct CI gate (I3
from the v1.4 architecture audit).

## PyCharm support

A JetBrains integration via PyCharm's LSP client. Deferred until after
polars — VS Code is the only supported editor for now.

## Configuration

A `pykrete.json` at (or above) the project root configures both the CLI
and the LSP — `typeCheckingMode` (`off` / `basic` / `standard` /
`strict`), `exclude` (path substrings to skip), and `rules` (per-rule
overrides — `off` / `warning` / `error`, keyed by readable rule name).
For the LSP, `pykrete.json`'s `typeCheckingMode` overrides the editor's
setting; the single value also drives the embedded Python engine.

## Quality-of-life

- **User-facing language reference** ([`language-reference/`](language-reference/))
  — schema syntax, operation reference, error catalog, configuration,
  cookbook. The doc lives but is empty.
- **Zed extension** — Neovim, Helix and Emacs setups are wired in
  [`editors/`](editors/); Zed needs a dedicated extension.

Already shipped (recorded here for completeness):

- **In-browser playground reaches pykrete IDE parity.** The Monaco
  editor at [`/playground`](https://amirnaderi93.github.io/pykrete/playground/)
  now serves the same pykrete capabilities the VS Code extension does
  for `.pyk` files: hover on schema names, `SparkFrame[X]` /
  `PandasFrame[X]` / `DataFrame[X]` references, and chain-bound locals;
  column-name completion inside `col("…")` and schema-name completion
  inside `SparkFrame[…]` / `PandasFrame[…]` / `DataFrame[…]`; and
  go-to-definition on Schema references. Wired through three new `pykrete-wasm` entry
  points (`hover_at`, `complete_at`, `definition_at`) that delegate to
  the same `pykrete::hover` / `pykrete::completions` / `pykrete::definition`
  the LSP server uses, so playground behavior matches a local install.
  Follow-up: the embedded Python language server (the multiplexer's
  half) isn't reachable from the browser yet — Python-side hover,
  parameter info, and imports still need the desktop install.
- **Performance pass.** Project-scope hot paths reviewed and
  micro-optimized: schema-name resolution (previously a linear scan over
  every project-wide schema) is now a `HashMap` index keyed by name on
  the per-function `BodyContext`; the `discover_schemas` fixpoint sweep
  uses a name → class-index table instead of an `O(N²)`
  `iter().position(...)` per (class, base) pair. A `tests/perf.rs`
  smoke test exercises a synthetic 50-file / 1500-schema project and
  asserts the release-build wall-clock stays inside a generous budget
  so an order-of-magnitude regression is caught in CI.
- **Duplicate-name detection across files.** `D0072 duplicateSchemaName`
  warns when the same `class X(Schema)` is declared in more than one
  project file. The alphabetically-earliest declaration is treated as
  the canonical site; every later one gets a warning that names both
  files. Same-file redeclarations don't fire D0072 (different
  concern), and function-name duplicates aren't covered yet.

- **Generic-inference: full coverage of the four extension patterns.**
  Multi-TypeVar binding —
  `def join[A, B](left: SparkFrame[A], right: SparkFrame[B]) -> SparkFrame[Merge[A, B]]`
  binds each TypeVar from its own argument slot and substitutes through
  the return, producing a derived view with the concatenated columns.
  Nested parameter shapes are unwrapped during binding:
  `List[DataSource[T]]`, `Optional[DataSource[T]]`,
  `Dict[str, DataSource[T]]`, and arbitrary re-nesting
  (`List[List[DataSource[T]]]`) all reach the inner `G[T]` shape.
  Chained class-method calls —
  `dal.with_path("/x").read(SOURCE)` — preserve class identity through
  any intermediate method whose return annotation is the class itself
  (`-> "DataAccessLayer"`, `-> DataAccessLayer`, or `-> Self`), so the
  trailing generic call still dispatches. `type[T]`-shaped parameters —
  `def cast_to[T](self, _: type[T]) -> SparkFrame[T]` called as
  `dal.cast_to(Orders)` — bind T from the arg's class identifier
  rather than its runtime value. Incompatible bindings (a list whose
  elements carry different T values, a non-class arg in a `type[T]`
  slot) degrade the offending TypeVar to Unknown rather than fabricate
  a result, keeping the no-false-positive stance.
- **`melt` / `unpivot` output-schema modeling.** Spark 3.4+'s
  wide-to-long reshape (`df.melt(ids, values, variableColumnName,
  valueColumnName)` and its alias `df.unpivot(...)`) now produces a
  modeled result schema: the `ids` columns are preserved with their
  declared types and nullability, the variable column is `string`, and
  the value column carries the common type of the unpivoted `values`
  columns (with numeric widening — `int` < `long` < `double` — and
  `Nullable(T)` when any value column is nullable). `values=None` or
  omitted unpivots every non-`ids` column. Typos in `ids` or `values`
  fire `D0030`; heterogeneous value types degrade to Unknown rather than
  fabricate a common type, so downstream checks stay permissive.
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
  but the user re-anchors the chain with `.cast(SparkFrame[Schema])` or a
  typed variable annotation (`raw: SparkFrame[Schema] = spark.read.parquet(...)`)
  and downstream column checks resume. Closes the headline gap where
  real PySpark codebases lost their chain at line one.
- **Call-site argument checking** (`D0051 argumentColumnsMismatch`) —
  closes the function boundary on the input side. Passing a
  `SparkFrame[Wrong]` into a function that declares `SparkFrame[Right]`
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

Status: **PySpark feature-complete; pandas check-site coverage shipped in
v1.3; polars is next.** Every dataframe library has the same shape — a
value carries a schema, methods narrow or widen it, column names must
exist when referenced.

Priority: **PySpark (done) → pandas check-site (done, v1.3) → pandas
depth + type-tracking (done, v1.4) → cross-dialect handoffs +
deferred-promise closure (done, v1.5) → `pykrete migrate` paired with
D0090 strict-mode escalation + pandas `pivot_table` literal-form +
`.take()` dialect-gate closure (done, v1.6) → broader pandas reshape
+ `.loc` / `.iloc` non-literal forms + `.query` / `.eval` mini-DSLs
(v1.7) → polars** → others (DuckDB, Dask, …).

The core type model — `SparkFrame[Schema]` / `PandasFrame[Schema]` /
`DataFrame[Schema]`, the `Schema` class, column checks, return-type
validation — generalizes. The library-specific layer is method dispatch
(`raw.select(col("x"))` vs `raw[["x"]]` vs `raw.select(pl.col("x"))`).
v1.3 shipped the per-annotation dispatch (`SparkFrame[X]` recognizes
Spark shapes, `PandasFrame[X]` recognizes pandas shapes — `df[col_list]`
/ `df[mask]` / `df["new"] = expr` / `df.drop` / `df.merge` / `df.rename`)
and the `D0090` deprecation that nudges callers off the legacy
`DataFrame[X]` alias. v1.4 shipped pandas type-tracking on
`PandasFrame[X]` via the `PROBE-TYPE-IS` synth
(`{df}.assign(__probe={df}["x"] + 1)` — a dispatched op so off-claim
numeric dtypes fall through to D0081), seven new pandas donors with 21
TYPE-IS markers (3 per donor), and three PRE-EXISTING silent-pass checker bug
closures (registry-call args, walrus receivers, `.transform` dialect
preservation). v1.5 shipped cross-dialect handoff (`.toPandas()` →
`PandasFrame[X]`; `spark.createDataFrame(pdf)` → `SparkFrame[Y]` when a
schema source is present), the v1.3 promise of `.loc[:, "col"]`
literal-form, dialect-gated `.head` / `.tail` / `.first` for pandas
chains, two PR-F1-class sibling gates (`column_name_arg` ungated arms +
`collect_col_refs` cross-DataFrame routing), the `--report-aliases` JSON
envelope for v2.0 migration sizing, and the synthetic-pool soft cap
that closes the v1.4 architecture-audit I4 finding. v1.6 ships
`pykrete migrate` — the auto-rewriter binary paired with D0090
strict-mode escalation — plus pandas `pivot_table` literal-form column
checking, the `.take()` dialect-gate closure, the `pdf.loc[mask, "col"]`
nested-arg D0030 FP closure, and the audit-debt `cross_dialect_handoff_gate`
recognizer extraction.

### Forking `ty`

Long term, pykrete may fork Astral's `ty` (their Rust Python type checker)
once it reaches a stable release — a single native stack, replacing the
basedpyright multiplexer. Because pykrete's analyzer is already built on
`ruff_python_ast` (the AST `ty` uses), the schema-checking core ports
cleanly; the multiplexer is interim scaffolding by design.

