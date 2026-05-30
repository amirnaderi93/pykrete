---
title: Production readiness
description: Stability commitments, false-positive policy, release cadence, and known limitations for production PySpark teams evaluating pykrete.
---

## TL;DR

pykrete [v0.1.15](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.15) is feature-complete for PySpark. A deliberate "degrade to Unknown rather than fabricate" policy keeps the checker honest: when pykrete can't determine a schema or a type with confidence, it stops checking that subtree rather than guessing. A real-codebase integration loop ([pykrete-tests](/pykrete/about/pykrete-tests/)) catches regressions before they ship.

## Stability commitments

Once a piece of surface ships in a release, the project commits to backward-compatible behavior on the following:

- **Schema declaration syntax.** `Schema` classes, the `array` / `map` / `struct` / `Nullable` constructors, the TypeScript-style operators (`Pick`, `Omit`, `Join`, `GroupBy`, `Merge`).
- **The `DataFrame[Schema]` annotation surface.** Variable annotations, function parameter and return types, `.cast(DataFrame[Schema])` re-anchors.
- **Diagnostic codes.** `D0030`, `D0040`, `D0050`, `D0051`, `D0060`, `D0080`, `D0081`, `D0082`. The numeric code and the rule name are part of the contract; the diagnostic message text is not.
- **`pykrete.json` keys.** `typeCheckingMode`, `exclude`, `rules`. New keys may be added; existing ones won't change shape.
- **The CLI's machine-readable output** (`pykrete check --format json`) and exit codes. Shipped in [v0.1.33](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.33); the JSON schema becomes a stability contract at v1.0.0 (breaking changes after that point require a SemVer major bump).

What may still change without notice:

- The internal LSP wire protocol with the embedded Python engine (today's multiplexer is interim — see the [roadmap](/pykrete/about/roadmap/#forking-ty)).
- The wasm API surface — not yet shipped.
- Internal type representations exposed by `--debug` flags.

## False-positive policy

**No false positives.** When pykrete can't determine a schema or a column's type with confidence, it degrades that subtree to Unknown rather than guess. Two concrete examples from the v0.1 surface:

- `spark.read.parquet("s3://...")` returns Unknown until the user re-anchors with `.cast(DataFrame[Schema])` or a typed variable annotation. The schema is genuinely runtime data; pykrete won't invent one.
- `F.struct(F.lit(1))` falls back to positional names (`col1`, `col2`, …) when no `.alias("x")` is present, rather than fabricating a guessed field name. Heterogeneous value types in `melt` / `unpivot` degrade the value-column type to Unknown rather than picking a "winner".

The same rule applies at the generic-inference layer: a TypeVar bound to incompatible schemas across argument slots stays Unknown. Downstream checks against an Unknown subtree are permissive: no diagnostic fires unless the user re-anchors.

A static checker that cries wolf gets switched off; pykrete prefers to stay quiet when it isn't sure.

## Release cadence

Nine releases in 48 hours (v0.1.7 → v0.1.15, late May 2026) — the finishing pass on the Spark-coverage closure sprint, not a steady-state cadence. Expect a more measured pace once v1.0.0 ships and focus shifts to pandas / polars. See the [GitHub Releases page](https://github.com/amirnaderi93/pykrete/releases) for the full history.

## Real-codebase testing

Every release is regression-tested against vendored snapshots of Apache Spark and MLflow — see [Real-codebase tests](/pykrete/about/pykrete-tests/) for the methodology. CI on every push and nightly runs `pykrete check` against the annotated snapshots; pykrete is rebuilt fresh from `main` each run, so any regression surfaces before it gets released.

Gaps closed in earlier releases (`df["X"]` subscript, GroupedData shortcut aggregates, chained nested-field access, `intersect` / `subtract` / `exceptAll`, lowercase `groupby`) all have regression tests in `crates/pykrete/tests/`. They can't reopen silently.

## Known limitations

By design, pykrete does not model:

- **Structured streaming** (`readStream`, `writeStream`, `isStreaming`).
- **RDD-level operations** (`rdd`, `mapPartitions`, `foreach`).
- **Pandas-on-Spark and Arrow conversions** (`toPandas`, `toArrow`, `mapInPandas`, `pandas_api`). Pandas support is on the [roadmap](/pykrete/about/roadmap/) as its own typed surface.

The full unmodeled list, with the rationale for each, is in [Operations → What's not modeled — by design](/pykrete/reference/operations/#whats-not-modeled--by-design).

## Production deployments

Currently being trialed inside production data engineering teams. We'll add named references here as adopters give the go-ahead.
