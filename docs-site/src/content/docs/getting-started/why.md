---
title: Why pykrete
description: The case for static schema checking on PySpark codebases — and why pykrete looks the way it does.
---

PySpark dataframes have a schema. The schema is real — every row in a DataFrame has the same columns, the same types, the same nullability. But Python doesn't see it. `df.select("price")` is just a string passed to a method; mistyping it as `"prcie"` produces no syntax error, no IDE warning, no test failure unless the test happens to assert on the wrong column name explicitly.

What you get instead is an empty DataFrame in production, or a `Column 'prcie' does not exist` from Spark's analyzer the first time a query plan actually runs — by which point the bad code is already merged, deployed, or part of a scheduled job that ran at 2am.

The shape of this problem is identical to JavaScript's. Object keys are strings; mistyping them is silent. TypeScript fixed it by adding a separate type layer that the runtime ignores, with a checker that runs at edit time. pykrete does the same thing for dataframes.

## The shape of the solution

```python
class Order(Schema):
    place_code: int
    status: string
    amount: int

def total_per_place(orders: DataFrame[Order]) -> DataFrame:
    return orders.groupBy("place_code").agg(F.sum("amount").alias("total"))
```

Three things happen here, none of which exist in plain Python:

1. **`Schema`** declares a dataframe's columns and types. Python evaluates it as a class with type-annotated attributes; pykrete reads it as a schema.
2. **`DataFrame[Order]`** is a parameterized type — same shape as TypeScript's `Array<Order>`. It says "this parameter is a DataFrame whose columns are exactly those of `Order`".
3. **The body** is checked against `Order`. A typo on `"place_code"` would fire `D0030 unknownColumn` at the call site. A typo on `"amount"` would fire it inside the `F.sum(...)` call. A `select("status").filter("payed > 0")` after the `agg` would catch `payed` against the *post-agg* schema (`place_code` and `total`), not the original `Order`.

`.pyk` is a strict superset of Python — every valid Python file is also valid pykrete. The new syntax (`Schema`, `DataFrame[X]`) evaluates at runtime to plain Python classes and annotations; the checker is the only thing that uses them as types. `pykrete transpile` strips back to `.py` if you want to deploy without the `.pyk` extension visible — though you usually don't have to; Spark workers don't care about file extensions, only about the Python they get.

## What this is and isn't

It **is** a static checker, an LSP, a VS Code extension, and a thin transpiler. It catches column-name typos, schema drift through transformation chains, mismatched schemas at `union` / `unionByName` / `intersect`, column-type errors at function boundaries, and shape mismatches between what a function declares it returns and what its body actually produces.

It **isn't** a runtime validator, a query planner, a Spark client, or a replacement for tests. The deployed code is plain Python that runs on the same JVM-backed Spark. pykrete is a separate layer that sits at edit time, like TypeScript at edit time.

## Why "strict superset"

Same reason TypeScript is one: gradual adoption. Annotate the one function whose dataframe you actually understand, ship it, run pykrete only on that file at first. The other 200 files in the repo can stay `.py` and untouched. When `Order` becomes useful elsewhere, move it to a shared module and add `DataFrame[Order]` annotations to the next function. The checker scales with the annotations you've added; un-annotated code is just Python.

## Why now

PySpark codebases have grown faster than their type discipline. Every team has a wiki page somewhere documenting "the canonical columns of the orders dataset" — a thing that the language should know but doesn't. The pieces to fix that are now off-the-shelf:

- A fast Python parser (ruff's, reused).
- A flexible LSP protocol so the checker becomes diagnostics in your editor, not a build step.
- Cross-editor reach via VS Code, Cursor, Open VSX, and a dedicated LSP that any editor with LSP support can use.

pykrete is what falls out when you assemble those pieces around a `DataFrame[Schema]` type and a Spark-aware schema model.
