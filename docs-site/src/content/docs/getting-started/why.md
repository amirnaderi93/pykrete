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
def revenue_by_region(sales: DataFrame[Sale]) -> DataFrame:
    return sales.groupBy("region").agg(F.sum("amount").alias("total"))
```

And pykrete checks every column you touch against the schema in scope — `groupBy("regoin")` is flagged, `F.sum("amuont")` is flagged, and a reference to a column that an earlier `drop` removed is flagged at the line that uses it. The checks follow the data through the whole chain, because each operation transforms the schema and pykrete tracks the result.

`.pyk` is a strict superset of Python. Every valid Python file is valid pykrete; the schema class and the `DataFrame[Sale]` annotation are ordinary Python that the interpreter is happy to ignore. The file runs unchanged. pykrete is the layer that reads those annotations at edit time — nothing it adds reaches runtime.

## Adopt it one file at a time

You do not convert a codebase to use pykrete. You convert a file.

Rename `sales.py` to `sales.pyk` — it still runs exactly as before. Add one `Schema` class and one `DataFrame[…]` annotation to the function whose dataframe you actually understand. That function is now checked. The other two hundred files stay `.py` and untouched; pykrete only analyzes functions you've annotated.

This is the whole reason pykrete is a *superset* and not a new language. Adoption scales with the annotations you've added, and stops costing you anything the moment you stop adding them. There is no all-or-nothing migration, no flag day.

## What it is, and what it isn't

It **is** a static checker (`pykrete check`), a language server (`pykrete-lsp`) that puts the same checks in your editor as diagnostics, hover, and completion, and a thin transpiler back to `.py`. It catches column-name typos, schema drift through transformation chains, mismatched schemas at `union` / `intersect`, wrong join keys, and shape mismatches between what a function declares and what it returns.

It **isn't** a runtime validator, a query planner, or a replacement for tests. It doesn't run your job or touch your cluster. The deployed code is plain Python on the same Spark as before. pykrete sits one layer up, at edit time — exactly where TypeScript sits relative to JavaScript.

## Where it's going

PySpark is supported today. Every dataframe library has the same shape — a value carries a schema, operations narrow or widen it, column names must exist when referenced — so pandas and polars are next. The [roadmap](/pykrete/about/roadmap/) has the detail.

Ready to try it? [Install](/pykrete/getting-started/install/) takes a minute; the [quickstart](/pykrete/getting-started/quickstart/) gets a real function under checking in five.
