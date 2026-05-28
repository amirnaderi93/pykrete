---
title: Roadmap
description: What's planned, in rough priority order. Tracking the project as it moves.
---

The canonical source is [docs/roadmap.md](https://github.com/amirnaderi93/pykrete/blob/main/docs/roadmap.md) in the repo. This page summarizes it for site visitors.

## Where pykrete is now

The PySpark static checker is **feature-complete** as of [v0.1.15](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.15):

- The full DataFrame operation surface (`select` / `filter` / `join` / `groupBy`+`agg` / `withColumn(s)` / `drop` / `union` / `cube` / `rollup` / `pivot` / `transform` / `cast` / `toDF` / `df.na.*` / set ops / `melt` / `unpivot` / …) with result-schema inference through whole transformation chains.
- Inline SQL (`F.expr`, `selectExpr`, string-`filter`) and raw `spark.sql("SELECT …")` — including `createOrReplaceTempView` + `spark.sql("SELECT … FROM v")` resolution within a file.
- `Window` partition/order key checking.
- Column-**existence** checking (`D0030`) and column-**type** checking — conservative (`D0080`, on by default) and strict (`D0081` / `D0082`, under `typeCheckingMode: strict`).
- Arbitrarily-nested `array` / `map` / `struct` columns — declared, structurally type-checked, and navigated field-by-field (`col("orders.line.sku")`).
- Column method chains — `.isNull` / `.isin` / `.between` / `.like` / `.getField` / `.getItem` / `.withField` / `.dropFields` — recognized and tracked through.
- `F.when` / `F.otherwise` result-type inference and `F.struct` / `F.named_struct` schema construction. Date/time first-arg column checking on ten `F.*` functions. Array higher-order recognizers (`F.transform`, `F.filter`, `F.aggregate`, `F.exists`, `F.forall`).
- `spark.read.<format>(path)` and `spark.table(name)` recognized as opaque sources — re-anchor with `.cast(DataFrame[Schema])` or a typed variable annotation to resume checking.
- A `pyspark.sql.functions` result catalog (≈80 functions) and UDF return types.
- Call-site argument checking (`D0051 argumentColumnsMismatch`) closes the function boundary on the input side.
- Generic-inference: multi-TypeVar binding, nested generic shapes, chained class-method calls, and `type[T]` argument binding all dispatch correctly.
- Cross-file imports and shared-schema modules.
- Project-wide duplicate-schema-name detection (`D0072 duplicateSchemaName`, warning), and a performance-pass micro-optimization of the schema-name resolution hot path (with a release-build perf smoke test in CI).

The **LSP server** delivers live diagnostics, hover, completion (column names in bare-string arguments and on chain results), document symbols, go-to-definition, find-references, rename, semantic tokens, and `textDocument/codeAction` quick-fixes for `D0030` "did you mean" suggestions. It embeds a Python language server via an LSP multiplexer.

The **VS Code extension** wraps it; Neovim, Helix, and Emacs setups are documented; a Zed extension is planned.

The **`.pyk` → `.py` transpiler** is complete.

For the full list of every shipped feature with diagnostics, see the [Operations reference](/pykrete/reference/operations/) and the [GitHub Releases page](https://github.com/amirnaderi93/pykrete/releases).

## Next up

### pandas support

PySpark is v1; pandas is v2. The core type model — `DataFrame[Schema]`, the `Schema` class, column checks, return-type validation — generalizes. The library-specific layer is method dispatch (`raw.select(col("x"))` vs `raw[["x"]]`). This is the main v0.2 work.

The annotation surface under consideration: `SparkFrame[Schema]` and `PandasFrame[Schema]`, with `DataFrame[Schema]` aliased to `SparkFrame[Schema]` for v0.1.x source compatibility.

### Window-key type tracking

Currently `Window.partitionBy("col")` keys aren't checked against any DataFrame schema. Adding local-binding tracking for Window objects, then resolving keys at the `.over(w)` site, would close the gap.

### Column-expression type tracking

Tracks the type of a Column through chains like `df["a"].cast("int").alias("x")`. Needed for `df[N]` integer-subscript bounds checks, fuller `Column.withField` / `Column.dropFields` against nested struct columns, and warning when a Column's atomic type drives a function that expects a different type.

## Larger structural moves

### Multi-dataframe support

After pandas, polars next. The dispatch model lets new libraries plug in without churning the schema model.

### Forking `ty`

Long term, pykrete may fork Astral's [`ty`](https://github.com/astral-sh/ty) (their Rust Python type checker) once it reaches a stable release — a single native stack, replacing the basedpyright multiplexer. Since pykrete's analyzer is already built on `ruff_python_ast` (the AST ty uses), the schema-checking core ports cleanly; the multiplexer is interim scaffolding by design.

### PyCharm support

Deferred well past pandas and polars. VS Code is the only supported editor for now.
