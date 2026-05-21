---
title: Roadmap
description: What's planned, in rough priority order. Tracking the project as it moves.
---

The canonical source is [docs/roadmap.md](https://github.com/amirnaderi93/pykrete/blob/main/docs/roadmap.md) in the repo. This page summarizes it for site visitors.

## Where pykrete is now (v0.1.6)

The PySpark static checker is feature-complete for the v0.1 surface:

- The full DataFrame operation surface (`select` / `filter` / `join` / `groupBy`+`agg` / `withColumn(s)` / `drop` / `union` / `cube` / `rollup` / `pivot` / `transform` / …) with result-schema inference through whole transformation chains.
- Inline SQL (`F.expr`, `selectExpr`, string-`filter`) and raw `spark.sql("SELECT …")`.
- `Window` partition/order key checking.
- Column existence (`D0030`) and column type (`D0080` / `D0081` / `D0082`) checking, including dotted nested-field paths.
- Arbitrarily-nested `array` / `map` / `struct` columns.
- `pyspark.sql.functions` result-type catalog and UDF return types.
- Cross-file imports and shared-schema modules.

The LSP server delivers live diagnostics, hover, completion, document symbols, go-to-definition, find-references, rename, and semantic tokens. The VS Code extension wraps it; Neovim, Helix, Emacs and Zed setups are documented.

The `.pyk` → `.py` transpiler is complete.

## Next up

### pandas support

PySpark is v1; pandas is v2. The core type model — `DataFrame[Schema]`, the `Schema` class, column checks, return-type validation — generalizes. The library-specific layer is method dispatch (`raw.select(col("x"))` vs `raw[["x"]]`). This is the main v0.2 work.

The annotation surface under consideration: `SparkFrame[Schema]` and `PandasFrame[Schema]`, with `DataFrame[Schema]` aliased to `SparkFrame[Schema]` for v0.1.x source compatibility.

### Window-key type tracking

Currently `Window.partitionBy("col")` keys aren't checked against any DataFrame schema. Adding local-binding tracking for Window objects, then resolving keys at the `.over(w)` site, would close the gap.

### Column-expression type tracking

Tracks the type of a Column through chains like `df["a"].cast("int").alias("x")`. Needed for `df[N]` integer-subscript bounds checks, `Column.withField` / `Column.dropFields` against nested struct columns, and warning when a Column's atomic type drives a function that expects a different type.

## Larger structural moves

### Multi-dataframe support

After pandas, polars next. The dispatch model lets new libraries plug in without churning the schema model.

### Forking `ty`

Long term, pykrete may fork Astral's [`ty`](https://github.com/astral-sh/ty) (their Rust Python type checker) once it reaches a stable release — a single native stack, replacing the basedpyright multiplexer. Since pykrete's analyzer is already built on `ruff_python_ast` (the AST ty uses), the schema-checking core ports cleanly; the multiplexer is interim scaffolding by design.

### PyCharm support

Deferred well past pandas and polars. VS Code is the only supported editor for now.
