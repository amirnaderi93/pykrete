---
title: How it works
description: What pykrete is under the hood — the checker, the language server, the transpiler, and how a .pyk file gets checked.
---

The other pages tell you what pykrete does for you. This one is for when you want to know how.

## A strict superset of Python

A `.pyk` file is a Python file. Every valid Python program is a valid pykrete program — pykrete adds meaning, not syntax you'd trip over. The two pieces it reads that plain Python ignores:

- A **`Schema`** class — an ordinary class with type-annotated attributes. Python sees a class; pykrete reads it as the column list of a dataframe.
- A **`SparkFrame[Schema]`** or **`PandasFrame[Schema]`** annotation — an ordinary parameterized type, the same shape as `list[int]`. Python sees a subscripted generic; pykrete reads it as "a Spark (or pandas) dataframe with these columns". (`DataFrame[Schema]` is a deprecated alias for `SparkFrame[Schema]`, accepted through the v1 line and removed in v2.0; uses fire the warning `D0090 deprecatedDataFrameAlias`.)

Because both are valid Python, a `.pyk` file runs unchanged. pykrete is a layer that reads those annotations at edit time and checks the code against them. Nothing it adds survives to runtime.

## Three tools

pykrete ships as two binaries that cover three jobs.

**The checker — `pykrete check`.** Point it at files or a directory; it reports column typos, schema drift, and shape mismatches, in the same `path:line:col` format your other tools use. This is what runs in CI.

**The language server — `pykrete-lsp`.** The same checker, wired to the Language Server Protocol, so the results arrive as you type: diagnostics, hover, completion, go-to-definition, find-references, rename. The VS Code extension wraps it; any LSP-capable editor can use it directly.

**The transpiler — `pykrete transpile`.** Turns a `.pyk` file back into a plain `.py` file. Since `.pyk` is a strict superset, this is nearly a copy — it only has to neutralize the handful of pykrete-only constructs so a stock Python runtime is happy. You rarely need it (Spark workers don't care about file extensions), but it's there when you want the deployed artifact to be unmistakably plain Python.

## How a file gets checked

When pykrete checks a file, it:

1. **Parses it.** pykrete uses [Ruff](https://github.com/astral-sh/ruff)'s Python parser — the same fast, PEP-current parser behind Astral's tooling. A `.pyk` file is Python, so it parses as-is.
2. **Finds the schemas and the typed functions.** Every `Schema` class becomes a known column list. Every function with a `SparkFrame[…]`, `PandasFrame[…]`, or (deprecated) `DataFrame[…]` parameter or return type becomes something to check.
3. **Walks each typed function's body.** This is the core. pykrete follows the dataframe through the function — `select`, `filter`, `withColumn`, `drop`, `groupBy` + `agg`, `join`, `union`, `pivot`, and the rest. Each operation transforms the schema: `drop` removes a column, `withColumnRenamed` renames one, an aggregation collapses many into one. pykrete carries the resulting schema into the next step.
4. **Checks every column reference against the schema in scope at that point.** A name that isn't on the schema is a diagnostic — pointed at the exact reference, with a *did you mean* when something close exists. Because the schema is tracked step by step, a reference to a column that was dropped two transforms earlier is caught where you use it, not where you dropped it.

Column references are checked wherever they appear: `col("x")`, attribute access `df.x`, subscript `df["x"]`, dotted paths into nested structs, the string arguments to functions like `F.sum("x")`, and the identifiers inside embedded SQL — `filter("x > 0")`, `selectExpr(...)`, `spark.sql("SELECT …")`.

Function boundaries are checked too: a `SparkFrame[Schema]` (or `PandasFrame[Schema]`) parameter is the schema the function body is checked against, and what the function declares it returns is verified against what its body actually produces.

## One server, two kinds of help

A schema checker that replaced your normal Python tooling would be a bad trade. `pykrete-lsp` doesn't replace it — it *multiplexes*.

The server embeds a full Python language server alongside pykrete's own analysis. Your editor connects to one server and gets both: pykrete's schema diagnostics, hover, and completion **and** ordinary Python language support — the latter handled by the embedded engine, untouched. pykrete's results are added to what the Python engine produces; nothing the Python engine reports is altered or dropped.

## What the transpiler actually does

`.pyk` → `.py` is nearly an identity transform. Two adjustments:

- It prepends `from __future__ import annotations`, so pykrete's type names and `DataFrame[…]` annotations are never evaluated at runtime — they're just strings as far as Python is concerned.
- It strips the one pykrete-only construct that appears in *expression* position — the schema re-anchor `.cast(DataFrame[Schema])`, which a stock Python runtime has no method for. The removal is surgical: only that call is deleted, line numbers and everything else are preserved byte-for-byte.

## Why these choices

- **Rust**, for a checker fast enough to run on every keystroke.
- **Ruff's parser**, so pykrete didn't spend a year on a Python front-end and stays current with the language. It's also the AST that Astral's `ty` type checker is built on — which keeps a long-term door open (see the [roadmap](/about/roadmap/)).
- **The checker is a library.** The CLI and the language server are both thin shells around the same analysis crate — the editor and the command line can't disagree about what's an error.
- **TypeScript as the design model** — adapted, not copied, for a language where dataframes flow through long transformation pipelines.

## Going deeper

The full architecture — module by module, the diagnostic-code catalog, the multiplexer internals — lives in the repo:

- [`docs/design/architecture.md`](https://github.com/amirnaderi93/pykrete/blob/main/docs/design/architecture.md)
- [`docs/design/multiplexer.md`](https://github.com/amirnaderi93/pykrete/blob/main/docs/design/multiplexer.md)
