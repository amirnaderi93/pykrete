---
title: Why pykrete
description: The case for static schema checking on dataframe code — the bug it removes, and why pykrete looks the way it does.
---

## The bug

A dataframe has a schema — every row has the same columns, the same types. The schema is real. Python just can't see it.

`df.select("amount")` is a method call with a string argument. Mistype it `"amuont"` and nothing reacts: not the interpreter, not your linter, not your tests unless one happens to assert on that exact name. The mistake travels — into review, into `main`, into a scheduled job. The first thing that notices is Spark itself, the first time the query plan actually runs, and by then the answer is already wrong: an empty dataframe, a column of nulls, a number nobody can explain.

Multiply that by every column reference in a pipeline that's been refactored four times by three people, and you have the normal state of a dataframe codebase: correct by habit and memory, not by anything a machine checks.

## The fix has a familiar shape

JavaScript had the same problem with object keys — strings, mistyped silently. TypeScript fixed it by adding a type layer the runtime ignores and a checker that runs while you type. pykrete does that for dataframes.

You describe a dataframe's columns once, as a class:

```python
class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int
```

You annotate the dataframes that have that shape:

```python
def revenue_by_region(sales: SparkFrame[Sale]) -> DataFrame:
    return sales.groupBy("region").agg(F.sum("amount").alias("total"))
```

And pykrete checks every column you touch against the schema in scope — `groupBy("regoin")` is flagged, `F.sum("amuont")` is flagged, and a reference to a column that an earlier `drop` removed is flagged at the line that uses it. The checks follow the data through the whole chain, because each operation transforms the schema and pykrete tracks the result.

`.pyk` is a strict superset of Python. Every valid Python file is valid pykrete; the schema class and the `SparkFrame[Sale]` annotation are ordinary Python that the interpreter is happy to ignore. The file runs unchanged. pykrete is the layer that reads those annotations at edit time — nothing it adds reaches runtime.

## Adopt it one file at a time

You do not convert a codebase to use pykrete. You convert a file.

Rename `sales.py` to `sales.pyk` — it still runs exactly as before. Add one `Schema` class and one `SparkFrame[…]` annotation (or `PandasFrame[…]` for pandas) to the function whose dataframe you actually understand. That function is now checked. The other two hundred files stay `.py` and untouched; pykrete only analyzes functions you've annotated.

This is the whole reason pykrete is a *superset* and not a new language. Adoption scales with the annotations you've added, and stops costing you anything the moment you stop adding them. There is no all-or-nothing migration, no flag day.

## What it is, and what it isn't

It **is** a static checker (`pykrete check`), a language server (`pykrete-lsp`) that puts the same checks in your editor as diagnostics, hover, and completion, and a thin transpiler back to `.py`. It catches column-name typos, schema drift through transformation chains, mismatched schemas at `union` / `intersect`, wrong join keys, and shape mismatches between what a function declares and what it returns.

It **isn't** a runtime validator, a query planner, or a replacement for tests. It doesn't run your job or touch your cluster. The deployed code is plain Python on the same Spark as before. pykrete sits one layer up, at edit time — exactly where TypeScript sits relative to JavaScript.

## Where it's going

PySpark is feature-complete; pandas check-site coverage shipped in v1.3, type-tracking in v1.4, cross-dialect handoff in v1.5, the `pykrete migrate` rewriter in v1.6, migrator UX hardening + pandas `melt` in v1.7, the v2.0 deprecation runway (`pykrete check --deprecation-report` + D0091 cross-dialect mismatch warning) in v1.8, v2.0 migration plannability (`--deprecation-report` v2 envelope with per-site `migrationStatus` + `--ack` filter + D0091 maturity) in v1.9, v2.0 migration archivability (`--deprecation-report --snapshot=<path>` file-write + `--fail-on-nonempty` CI gate + D0091 8-property surface completion + pandas `df.stack(level=, dropna=)` literal-form) in v1.10, pandas `df.unstack(level=, fill_value=)` literal-form + cross-codebase property probes for the v1.10 D0091 8-property surface + audit-tooling block (trust-claim sweep checklist, CHANGELOG cite-check, auto-label workflow) in v1.11, the v1.11 calendared GITHUB_TOKEN promise closure (auto-label workflow now dispatches `release-gate.yml` via `actions.createWorkflowDispatch`) + D0080 returnTypeMismatch cross-codebase coverage (closing the longest-standing trust gap since v1.6) + pandas `pivot_table(aggfunc=)` 11-string allowlist recognition (priming v1.13+ aggfunc-driven inference) + multi-line ack-marker rationale block (spec §6.1.4) in v1.12, the D0080 dialect-on-return checker arm (closing the longest-standing 7-cycle correctness gap) + `pivot_table(aggfunc=)` Derived-schema synthesis (first observable aggregate-semantics-informed schema inference) + backtick-preservation tripwire + dispatched-run required-status-check in v1.13, and the D0080 constructor carve-out closure (cross-dialect constructor returns now fire; multi-clause format when both dialect + column-type mismatches land on the same return) + `groupby.agg` Derived synthesis (sibling to v1.13's `pivot_table(aggfunc=)`; shared inference helper) + `--compare-to` SIMPLE three-bucket snapshot diff (exit-nonzero on `added`; mutex with `--ack` / `--snapshot` / `--fail-on-nonempty`) + envelope schema v2 provenance pair (`pykreteSourceCommit` + `generatedAt`) in v1.14, and pandas chain-depth extension through `groupby.agg().reset_index(drop=True)` + `set_index([literal-keys])` + synthesis-arm cross-codebase coverage closure (pykrete-tests PR-P1 #50) + `resolve_override_ty` primitive (dtype-override family consolidation, which the v1.16 window arms became the third consumer of) + marketing-table gate v3 (audit-tooling fence-vs-claim discipline) in v1.15, and time/window aggregation — `resample.agg` + `rolling.agg` direct chains plus the dict and callable forms of `groupby.agg` — with honest-silence declines wherever pandas 2.x raises (named-aggregation, `rolling` over a non-numeric column, numeric-restricting aggregation over a non-numeric column) and `inplace=` guards on `reset_index` / `set_index` in v1.16. Broader pandas reshape (`reset_index(drop=False)` / `set_index(<expr>)` non-literal forms / `expanding.agg` / the rest of the window surface, including the direct-method `df.resample("M").sum()` spelling) is next, then polars. The [roadmap](/about/roadmap/) has the detail.

Ready to try it? [Install](/getting-started/install/) takes a minute; the [quickstart](/getting-started/quickstart/) gets a real function under checking in five.
