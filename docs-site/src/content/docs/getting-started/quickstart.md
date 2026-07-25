---
title: Quickstart
description: From a plain Python dataframe file to a pykrete-checked one — in three steps, five minutes, no project rewrite.
---

You have a PySpark codebase and pykrete [installed](/getting-started/install/). Here's the shortest path to it catching your first typo.

## 1. Rename one file

Pick a file with a function whose dataframe you understand. Rename it:

```sh
mv sales.py sales.pyk
```

That's a real change and a safe one. `.pyk` is a strict superset of Python — the file still parses, still runs, still does exactly what it did. `pykrete check sales.pyk` already works; it just has nothing to check yet.

## 2. Declare a schema

Add a class describing the columns of the dataframe the function works with:

```python
class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int
```

Field name is column name; field type is column type. The atomic types are `int`, `long`, `string`, `double`, `bool`, `date`, `timestamp`; columns can also be arrays, maps, and nested structs — see [Schemas](/reference/schemas/).

## 3. Annotate the function

Add the schema to a parameter with `SparkFrame[Sale]` (or `PandasFrame[Sale]` for a pandas dataframe):

```python
def revenue_by_region(sales: SparkFrame[Sale]) -> DataFrame:
    return (
        sales
        .filter(F.col("quantity") > 0)
        .groupBy("region")
        .agg(F.sum("amount").alias("total"))
    )
```

That's the whole investment — one class, one annotation. Run the check:

```sh
$ pykrete check sales.pyk
sales.pyk: parsed OK — 1 schema(s), 1 typed function(s), 0 issue(s)
```

## 4. Break it on purpose

Change `"region"` to `"regoin"` and run again:

```sh
$ pykrete check sales.pyk
sales.pyk:10:18 - error unknownColumn: Column 'regoin' does not exist on schema 'Sale'. Did you mean 'region'?
```

There it is — location, severity, rule name, the bad column, the schema it was checked against, and a suggestion. With the [VS Code extension](/getting-started/install/#vs-code-extension), this is a red underline under `regoin` as you type, no command needed.

Change it back. You now have one checked function.

## What pykrete is checking

The annotation doesn't just check the one `groupBy`. It follows the chain:

- `.filter(...)` keeps the schema `Sale` — a filter changes rows, not columns.
- `.groupBy("region")` produces a grouped view keyed on `region`.
- `.agg(F.sum("amount").alias("total"))` produces a new schema: `region` and `total`.

So a `.select(F.col("amount"))` tacked on the end would be flagged — `amount` was aggregated away two steps earlier. The error points at the line that uses the missing column, not the line that removed it. The same tracking covers `select`, `withColumn`, `drop`, `join`, `union`, `pivot`, and the rest.

## Grow it at your own pace

One file is a complete, useful state — leave it there as long as you like.

When the `Sale` schema would help elsewhere, move it to a shared module and import it; other files can stay `.py`. pykrete only enters a function when its signature has a `SparkFrame[…]` or `PandasFrame[…]` slot, so unannotated code costs nothing.

When you want the whole project checked at once:

```sh
pykrete check .
```

It walks every `.pyk` file and prints the same `parsed OK — N schema(s), M typed function(s), K issue(s)` summary per file. That line is what you put in CI.

For CI scripts that need to parse the results, `pykrete check --format json .` emits a single JSON object on stdout (`{"version": ..., "diagnostics": [...], "summary": {...}}`) with the same `0` / `1` exit code. The schema becomes a stability contract at v1.0.0.

## Next

- [Schemas](/reference/schemas/) — nested arrays / maps / structs, and the `Pick` / `Omit` / `Merge` operators.
- [Diagnostics](/reference/diagnostics/) — every rule, with examples.
- [How it works](/about/how-it-works/) — what's happening under the hood.
