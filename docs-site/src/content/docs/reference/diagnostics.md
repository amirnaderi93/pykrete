---
title: Diagnostics
description: Every pykrete diagnostic code, with what triggers it and how to fix.
---

Diagnostic codes are stable across releases. The format is always:

```
file.pyk:LINE:COL  severity  CODE  rule-name: message
```

Most are errors; a few default to warnings. The severity of each is configurable in [`pykrete.json`](/pykrete/reference/configuration/).

## D0030 — `unknownColumn`

A column reference doesn't exist on the schema in scope at that point in the chain.

**Triggers on:**

- `col("typo")` / `df.typo` / `df["typo"]`
- Chained nested access — `df.r.typo`, `df["r"].typo`, `df.r["typo"]`, `df["r"]["typo"]`
- Bare-string column arguments to F-functions — `F.sum("typo")`, `F.first("typo")`, …
- Inline SQL identifiers — `df.filter("typo > 0")`, `df.selectExpr("typo")`, `spark.sql("SELECT typo FROM …")`
- DataFrame method args expecting column names — `groupBy("typo")`, `drop("typo")`, `withColumnRenamed("typo", "new")`, `sort("typo")`, …

**Fix:** correct the column name, or add it to the schema if it's actually expected.

```
orders.pyk:9:18 error D0030 unknownColumn:
    Column 'plcae_code' does not exist on schema 'Order'.
    Did you mean 'place_code'?
```

The diagnostic includes a did-you-mean when there's a close match in the schema.

## D0040 — `unionSchemaMismatch`

`union` / `unionByName` / `intersect` / `intersectAll` / `subtract` / `exceptAll` between two dataframes whose schemas don't agree on the column-name set.

**Fix:** align the schemas, or use the appropriate method for the actual intent (`unionByName` if names match but order doesn't).

```
report.pyk:12:5 error D0040 unionSchemaMismatch:
    union between schema 'Orders' and schema 'Returns': schemas differ.
    Missing in schema 'Returns': [status]; missing in schema 'Orders': [reason].
```

## D0050 — `returnSchemaMismatch`

A function declared `-> DataFrame[Schema]` returns a dataframe whose inferred schema doesn't match.

**Fix:** add a `.cast(DataFrame[Schema])` if the chain ends with an opaque step (like `spark.read.json(...)`), or correct the body.

```
ingest.pyk:14:5 error D0050 returnSchemaMismatch:
    Declared return is DataFrame[Order] (columns: place_code, price);
    inferred return is DataFrame (columns: place_code, total).
    'price' is missing; 'total' is unexpected.
```

## D0080 / D0081 / D0082 — column type checking

Conservative type checks (`D0080`, on by default) catch obvious type errors. Strict checks (`D0081`, `D0082`) catch nullability mismatches and require `typeCheckingMode: strict` in [`pykrete.json`](/pykrete/reference/configuration/#typecheckingmode).

| Code | Rule | Severity | Triggers on |
|---|---|---|---|
| D0080 | `columnTypeMismatch` | error | `F.upper(col("price"))` where `price: int` — string-expecting function called with an int column. |
| D0081 | `strictColumnTypeMismatch` | error in strict mode | Stricter cross-type combinations the basic mode lets through. |
| D0082 | `nullabilityMismatch` | error in strict mode | Operations that produce a nullable result fed into a non-null sink. |

## D0001 — `parseError`

The file isn't valid Python syntax. pykrete uses ruff's parser, so the message matches ruff's.

## D0020 — `unknownSchema`

A `DataFrame[X]` annotation references a class `X` that pykrete can't find — typo in the schema name, missing import, or not annotated with `class X(Schema)`.

## D0021 — `notASchema`

A `DataFrame[X]` annotation where `X` is not actually a `Schema` subclass.

## Configuring severity

Any diagnostic can be downgraded to a warning or turned off entirely in `pykrete.json`:

```json
{
  "rules": {
    "unknownColumn": "error",
    "unionSchemaMismatch": "warning",
    "columnTypeMismatch": "off"
  }
}
```

Use the rule name (the part after the code), not the code itself.

See [Configuration](/pykrete/reference/configuration/) for the full file format.
