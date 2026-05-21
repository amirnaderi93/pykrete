---
title: Quickstart
description: From a plain Python PySpark file to a pykrete-checked one in three steps.
---

The shortest path from "I have a PySpark codebase" to "pykrete is catching my typos". Five minutes, no project rewrite.

## 1. Rename one file `.py` → `.pyk`

Pick a function whose dataframe schema you actually know. Rename its file:

```sh
mv orders.py orders.pyk
```

Nothing else changes. `.pyk` is a strict superset of Python — every valid Python file is also valid pykrete. At this point `pykrete check orders.pyk` will run and report `0 issues`, because there are no `Schema` annotations yet to check anything against.

## 2. Add a `Schema` class

Declare the columns of the dataframe you're working with. Field names and types match what Spark sees in the source:

```python
# orders.pyk
class Order(Schema):
    place_code: int
    status: string
    amount: int
```

Atomic types are `int`, `long`, `string`, `double`, `bool`, `date`, `timestamp`. Nested types are arrays, maps, and structs — declared by referencing another `Schema` class as a field type. See [Schemas](/pykrete/reference/schemas/) for the full reference.

## 3. Annotate one function's parameter

```python
# orders.pyk
def average_basket(orders: DataFrame[Order]) -> DataFrame:
    return (
        orders
        .filter(F.col("status") == "paid")
        .groupBy("place_code")
        .agg(F.avg("amount").alias("avg_amount"))
    )
```

That's the whole investment: one `Schema` class, one `DataFrame[Order]` annotation.

Run the check:

```sh
$ pykrete check orders.pyk
orders.pyk: parsed OK — 1 schema(s), 1 typed function(s), 0 issue(s)
```

## 4. Make a typo on purpose

Change `"place_code"` to `"plcae_code"` and re-run:

```sh
$ pykrete check orders.pyk
orders.pyk:6:18 error D0030 unknownColumn:
    Column 'plcae_code' does not exist on schema 'Order'.
    Did you mean 'place_code'?
```

The diagnostic includes the location, the rule name, the failing column, the schema it was checked against, and a did-you-mean. If you have the [VS Code extension](/pykrete/getting-started/install/#vs-code-extension) installed, this same diagnostic appears as a red squiggle under `plcae_code` as you type.

Revert the typo. You now have one checked function.

## What gets checked

The annotation propagates through whole chains:

- After `.filter(...)`, the schema is still `Order` — filter doesn't change columns.
- After `.groupBy("place_code")`, pykrete tracks that the group key is `place_code` and the underlying schema is `Order`.
- After `.agg(F.avg("amount").alias("avg_amount"))`, the result schema is `place_code: int, avg_amount: double`. A downstream `.filter(F.col("amount") > 0)` would now fail — `amount` was aggregated away.

The same flow applies to `select`, `withColumn`, `drop`, `join`, `union`, `intersect`, `cube`, `rollup`, `pivot`, `transform`, and the rest. See [Diagnostics](/pykrete/reference/diagnostics/) for what gets caught.

## 5. Gradual adoption

You don't need to convert the whole repo. Run pykrete on the one file you've annotated:

```sh
pykrete check orders.pyk
```

When `Order` becomes useful elsewhere, move it to a shared module — `schemas.py` or whatever — and import it. Other files can stay `.py`; pykrete only enters body analysis for functions whose signature has a `DataFrame[X]` slot.

When you're ready, run pykrete against the whole repo:

```sh
pykrete check .
```

It walks `.pyk` files and reports the same `parsed OK — N schema(s), M typed function(s), K issue(s)` summary per file. Unannotated `.py` files are skipped.

## What's next

- [Schemas](/pykrete/reference/schemas/) — the full schema syntax including nested arrays / maps / structs and the TypeScript-style type operators (`Pick`, `Omit`, `Join`, `GroupBy`).
- [Diagnostics](/pykrete/reference/diagnostics/) — every rule with example and fix.
- [Real-codebase tests](/pykrete/about/pykrete-tests/) — see what pykrete catches on real annotated PySpark code.
