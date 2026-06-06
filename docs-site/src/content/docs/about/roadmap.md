---
title: Roadmap
description: What's planned, in rough priority order. Tracking the project as it moves.
---

The canonical source is [docs/roadmap.md](https://github.com/amirnaderi93/pykrete/blob/main/docs/roadmap.md) in the repo. This page summarizes it for site visitors.

## Where pykrete is now

The PySpark static checker is **feature-complete** as of the v1.0 release line, with pandas dialect support added in v1.3 (see the [GitHub Releases page](https://github.com/amirnaderi93/pykrete/releases) for the per-release history):

- The full DataFrame operation surface (`select` / `filter` / `join` / `groupBy`+`agg` / `withColumn(s)` / `drop` / `union` / `cube` / `rollup` / `pivot` / `transform` / `cast` / `toDF` / `df.na.*` / set ops / `melt` / `unpivot` / …) with result-schema inference through whole transformation chains.
- Inline SQL (`F.expr`, `selectExpr`, string-`filter`) and raw `spark.sql("SELECT …")` — including `createOrReplaceTempView` + `spark.sql("SELECT … FROM v")` resolution within a file.
- `Window` partition/order key checking.
- Column-**existence** checking (`D0030`) and column-**type** checking — conservative (`D0080`, on by default) and strict (`D0081` / `D0082`, under `typeCheckingMode: strict`).
- Arbitrarily-nested `array` / `map` / `struct` columns — declared, structurally type-checked, and navigated field-by-field (`col("orders.line.sku")`).
- Column method chains — `.isNull` / `.isin` / `.between` / `.like` / `.getField` / `.getItem` / `.withField` / `.dropFields` — recognized and tracked through.
- `F.when` / `F.otherwise` result-type inference and `F.struct` / `F.named_struct` schema construction. Date/time first-arg column checking on ten `F.*` functions. Array higher-order recognizers (`F.transform`, `F.filter`, `F.aggregate`, `F.exists`, `F.forall`).
- `spark.read.<format>(path)` and `spark.table(name)` recognized as opaque sources — re-anchor with `.cast(SparkFrame[Schema])` or a typed variable annotation to resume checking. (`DataFrame[Schema]` is the deprecated alias and also works through v1.x.)
- A `pyspark.sql.functions` result catalog (≈80 functions) and UDF return types.
- Call-site argument checking (`D0051 argumentColumnsMismatch`) closes the function boundary on the input side.
- Generic-inference: multi-TypeVar binding, nested generic shapes, chained class-method calls, and `type[T]` argument binding all dispatch correctly.
- Cross-file imports and shared-schema modules.
- Project-wide duplicate-schema-name detection (`D0072 duplicateSchemaName`, warning), and a performance-pass micro-optimization of the schema-name resolution hot path (with a release-build perf smoke test in CI).

The **LSP server** delivers live diagnostics, hover, completion (column names in bare-string arguments and on chain results), document symbols, go-to-definition, find-references, rename, semantic tokens, and `textDocument/codeAction` quick-fixes for `D0030` "did you mean" suggestions. It embeds a Python language server via an LSP multiplexer.

The **VS Code extension** wraps it; Neovim, Helix, and Emacs setups are documented; a Zed extension is planned.

The **`.pyk` → `.py` transpiler** is complete.

The **in-browser [playground](/pykrete/playground/)** runs pykrete via WebAssembly and now serves the same pykrete features the VS Code extension does for `.pyk` files: live diagnostics, hover on schema and `SparkFrame[X]` references, column-name completion inside `col("…")`, schema completion in `SparkFrame[…]` slots, and go-to-definition on schema references. (`DataFrame[X]` is the deprecated alias and renders the same hover.) The embedded Python engine isn't reachable from the browser yet (queued for a follow-up release).

For the full list of every shipped feature with diagnostics, see the [Operations reference](/pykrete/reference/operations/) and the [GitHub Releases page](https://github.com/amirnaderi93/pykrete/releases).

## Shipped in v1.3 — pandas check-site coverage

`PandasFrame[Schema]` joins `SparkFrame[Schema]` as a **canonical** dataframe-annotation form. `DataFrame[Schema]` is a **deprecated alias** for `SparkFrame[Schema]` — every use fires `D0090 deprecatedDataFrameAlias` (warning) with a quick-fix to the canonical name. The alias stays valid through the v1 line and is **removed in v2.0** so the migration is unhurried.

Six pandas operations dispatch through dialect-specific check sites: column selection (`df[col_list]`), boolean-mask filtering (`df[mask]`), assignment (`df["new"] = expr`), `df.drop`, `df.merge`, and `df.rename`. The bare-subscript widening rule also fires `D0030` on bare `df["typo"]` subscripts in non-method contexts on both `SparkFrame[X]` and `PandasFrame[X]`. Cross-codebase pandas fixtures land for mlflow, feast, and iceberg-python.

## Shipped in v1.4 — depth on pandas

The v1.3 → v1.4 cadence parallels v1.1 → v1.2 on the Spark side: check-site coverage first, type-tracking and donor breadth next.

- **7 new pandas-heavy donors** in pykrete-tests — scikit-learn, statsmodels, pandera, Great Expectations, prophet, seaborn, yfinance — bringing pandas-coverage donor count from 3 to 10. Honest scoping breakdown (see [Real-codebase tests](/about/pykrete-tests/) for the per-donor detail): 3 direct-dispatch (prophet, seaborn, yfinance) against actual upstream library code where pykrete's dispatched-shape recognizers match; 4 canonical-fixture-only (sklearn, statsmodels, pandera, GE) modeling user-pattern fixtures where the upstream code operates above raw pandas dispatch.
- **Pandas type-tracking via `PROBE-TYPE-IS`** (closes [pykrete-tests#14](https://github.com/amirnaderi93/pykrete-tests/issues/14)). The synth wraps `{df}.assign(__probe={df}["x"] + 1)` so off-claim numeric dtype claims on `PandasFrame[X]` parameters fall through to D0081. 21 markers across the 7 new donors (3 per donor, exactly meeting the v1.4 spec §1 floor).
- **Three checker bug closures** (PRE-EXISTING silent-pass paths surfaced by v1.3 audits): registry-call args walk unconditionally so `util(df["typo"])` fires D0030; `inherited_dialect` walks walrus receivers so `(pdf := build()).rename(...)` inherits the assigned-value's dialect; `.transform(helper)` threads the receiver's dialect into the helper's body inference. SemVer-minor under the `tighteningDiagnostics` policy.
- **Config-discovery walk fix**: `pykrete.json` discovery anchors on the input file's parent directory (falling back to CWD when no input resolves to a file), so `pykrete check /abs/path/to/foo.pyk` from any CWD picks up the project's config.
- **Canonical-name migration completion** across docs / design notes / examples for `SparkFrame[X]` vs the deprecated `DataFrame[X]` alias.

For the verification posture and per-donor matrix, see [Real-codebase tests](/about/pykrete-tests/) and [Production readiness → Real-codebase testing](/about/production-readiness/#real-codebase-testing). For the full pandas direction across v1.5+ and v2.0, see [Pandas roadmap](/about/pandas-roadmap/).

### Known limitations (v1.5 trackers)

Two pandas gaps remain open from v1.3 / v1.4 and are tracked for v1.5:

- **`.head()` / `.tail()` / `.first()` on `PandasFrame[X]` end the chain.** These three methods are recognized as Spark terminal methods (chain dies), regardless of the dialect tag on the receiver. In pandas they return a `DataFrame` and are chainable (`pdf.head(10).merge(other, on="id")` is canonical), so typos in operations downstream of pandas `.head()` / `.tail()` / `.first()` currently pass silently. v1.5 dialect-gates the terminal classification.
- **`df.loc[:, "col"]` is not a recognized column-access shape.** The pandas-support spec table previously listed `.loc[:, "status"]` as in scope for v1.3, but no `.loc` recognizer ships — typos in the slice key are silently accepted. Recognizing `.loc[:, "col"]` as a typed column access lands in v1.5; the spec table has been corrected in the meantime.

## Next up

### v1.5+ — pandas breadth + cross-dialect handoffs

- **Cross-dialect handoff annotations**: `.toPandas()` / `.toSpark()` / `pd.DataFrame.from_records(...)` schema propagation. Today these are opaque; v1.5 makes the dialect transition trackable.
- **`df.query("…")` / `df.eval("…")` mini-DSLs**: parse string-fragment column refs the way pykrete parses `selectExpr` SQL today. High signal for production pandas code.
- **Broader pandas method modeling**: `df.pivot_table`, `df.groupby(...).agg(...)`, `df.melt`, `df.stack` / `df.unstack`, `df.reset_index`, `df.set_index`. Currently fall through to opaque.
- **`pd.read_csv(...)` and other pandas I/O entry points**.

### Window-key type tracking

Currently `Window.partitionBy("col")` keys aren't checked against any DataFrame schema. Adding local-binding tracking for Window objects, then resolving keys at the `.over(w)` site, would close the gap.

### Column-expression type tracking

Tracks the type of a Column through chains like `df["a"].cast("int").alias("x")`. Needed for `df[N]` integer-subscript bounds checks, fuller `Column.withField` / `Column.dropFields` against nested struct columns, and warning when a Column's atomic type drives a function that expects a different type.

## Larger structural moves

### Multi-dataframe support

After pandas depth, polars next. The dispatch model lets new libraries plug in without churning the schema model.

### Forking `ty`

Long term, pykrete may fork Astral's [`ty`](https://github.com/astral-sh/ty) (their Rust Python type checker) once it reaches a stable release — a single native stack, replacing the basedpyright multiplexer. Since pykrete's analyzer is already built on `ruff_python_ast` (the AST ty uses), the schema-checking core ports cleanly; the multiplexer is interim scaffolding by design.

### PyCharm support

Deferred well past pandas and polars. VS Code is the only supported editor for now.
