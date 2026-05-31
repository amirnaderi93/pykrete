---
title: Diagnostics
description: Every pykrete diagnostic — what triggers it, what it looks like, and how to fix it.
---

Every diagnostic pykrete reports has the same shape:

```
path:line:col - severity rule-name: message
```

For example:

```
sales.pyk:10:18 - error unknownColumn: Column 'regoin' does not exist on schema 'Sale'. Did you mean 'region'?
```

The **rule name** (`unknownColumn`) is what the CLI prints and what the editor shows as the diagnostic code. Each rule also has a stable `D00xx` identifier used internally — both the name and the code are accepted as keys in [`pykrete.json`](/pykrete/reference/configuration/)'s `rules` block.

## Full reference

| Code | Rule name | What it means |
|---|---|---|
| `D0001` | `parseError` | The file isn't valid Python syntax. |
| `D0010` | `unknownColumnType` | A schema field's type isn't a recognized type. |
| `D0011` | `invalidColumnType` | A schema field's type isn't a valid type expression. |
| `D0020` | `unknownSchema` | `DataFrame[X]` names a schema pykrete can't find. |
| `D0021` | `invalidSchemaExpression` | The thing inside `DataFrame[…]` isn't a schema name or a valid operator. |
| `D0030` | `unknownColumn` | A column reference doesn't exist on the schema in scope. |
| `D0040` | `unionSchemaMismatch` | `union` / `intersect` / `subtract` between dataframes whose columns don't match. |
| `D0050` | `returnColumnsMismatch` | A function's returned **columns** differ from its declared return schema. |
| `D0051` | `argumentColumnsMismatch` | A call-site argument's schema differs from the parameter's declared `DataFrame[Schema]`. |
| `D0060` | `missingJoinKey` | A join key isn't present on one side of the join. |
| `D0070` | `unresolvedImport` | An `import` can't be resolved. |
| `D0071` | `unexportedName` | An imported name isn't exported by the module. |
| `D0072` | `duplicateSchemaName` | The same schema name is declared in more than one project file. Warning. |
| `D0073` | `transformInputMismatch` | A `df.transform(fn)` receiver's schema doesn't match `fn`'s declared parameter schema. |
| `D0080` | `returnTypeMismatch` | A returned column's **type** differs from the declared return schema. |
| `D0081` | `nonNumericArithmetic` | Arithmetic on a non-numeric column. Strict mode only. |
| `D0082` | `crossTypeComparison` | A comparison between unrelated types. Strict mode only. |
| `D0083` | `nullabilityMismatch` | A nullable column flows into a slot the return schema declares non-null. Strict mode only. |

## The ones you'll see most

### `unknownColumn` — D0030

A column reference doesn't exist on the schema at that point in the chain. This is the workhorse diagnostic.

It fires on every form of column reference:

- `col("x")`, attribute access `df.x`, subscript `df["x"]`
- Chained nested access — `df.address.city`, `df["address"]["city"]`
- Dotted paths — `col("address.city")`
- String column arguments to functions — `F.sum("x")`, `groupBy("x")`, `drop("x")`, `sort("x")`
- Identifiers inside embedded SQL — `filter("x > 0")`, `selectExpr("x")`, `spark.sql("SELECT x …")`

Because pykrete tracks the schema through each operation, a reference to a column that an earlier `drop` or aggregation removed is caught at the line that uses it.

```
sales.pyk:10:18 - error unknownColumn: Column 'regoin' does not exist on schema 'Sale'. Did you mean 'region'?
```

The message names the failing column and the schema it was checked against, with a *did you mean* when a close match exists.

When a *did you mean* suggestion is attached, the LSP exposes it as a `QuickFix` code action — VS Code surfaces it as a lightbulb on the underlined token, and selecting the action replaces the bad literal with the suggested name. D0030 is the only diagnostic that ships a quick-fix today.

### `unionSchemaMismatch` — D0040

`union`, `unionByName`, `intersect`, `intersectAll`, `subtract`, or `exceptAll` between two dataframes whose column-name sets don't agree.

```
report.pyk:14:12 - error unionSchemaMismatch: union between schema 'Sale' and schema 'Refund': schemas differ. Missing in schema 'Refund': [quantity]; missing in schema 'Sale': [reason].
```

**Fix:** align the two schemas, or switch to the operation that matches your intent.

### `returnColumnsMismatch` — D0050

A function declared `-> DataFrame[Schema]` returns a dataframe whose **column set** doesn't match — a column is missing, or an extra one is present.

```
summary.pyk:8:5 - error returnColumnsMismatch: declared return DataFrame[SaleSummary] expects columns [region, total]; the body produces [region, amount].
```

**Fix:** correct the body, or re-anchor an opaque chain (like `spark.read.parquet(...)`) with `.cast(DataFrame[Schema])` so pykrete knows the shape.

### `argumentColumnsMismatch` — D0051

The mirror of `returnColumnsMismatch`, one frame earlier: a `DataFrame[…]` argument at a call site has a schema that doesn't match the parameter's declared `DataFrame[Schema]`. Same missing / extra column reporting.

```
caller.pyk:13:13 - error argumentColumnsMismatch: Argument schema mismatch for parameter 'sales': expected DataFrame[Sale], got schema 'Refund'. Missing: [amount]; extra: [refund].
```

Arguments whose schema pykrete can't infer (an untyped local, an opaque `spark.read.json(...)` chain) are silently skipped — the checker degrades rather than false-flag.

D0051 also respects Python's calling rules: a local name that rebinds a top-level function shadows it (the call resolves to the local, so the top-level signature isn't checked) — whether the rebind is a plain assignment, a tuple-unpack LHS (`revenue, _ = …`), or a walrus binding (`(revenue := …)`); positional-only (`/`) and keyword-only (`*`) markers are honored when matching arguments to parameters; `*args: DataFrame[Schema]` / `**kwargs: DataFrame[Schema]` variadics are checked against every argument that lands in them; and a parameter that's filled both positionally *and* by keyword (which Python rejects as `TypeError`) is diagnosed once, not twice.

**Fix:** pass a dataframe with the expected shape, or re-anchor the argument with `.cast(DataFrame[Schema])` if the chain's schema was lost upstream.

### `missingJoinKey` — D0060

A column named as a join key doesn't exist on one (or both) sides of the join.

**Fix:** check the key name against both schemas; join keys must exist on each side.

## Type-checking diagnostics

pykrete checks column **types**, not just existence. How much it checks depends on [`typeCheckingMode`](/pykrete/reference/configuration/#typecheckingmode).

**`returnTypeMismatch` — D0080.** On by default. The returned columns match the declared return schema by name, but a column's *type* doesn't — and both types are confidently known and genuinely incompatible. Numeric widening (`int` → `long` → `double`) is accepted; unknown types are left alone. This is the conservative check: it fires only when it's sure.

**Strict mode adds three.** Under `typeCheckingMode: strict`:

- **`nonNumericArithmetic` — D0081.** Arithmetic applied to a column that isn't numeric.
- **`crossTypeComparison` — D0082.** A comparison between two unrelated types — the kind Spark silently coerces rather than rejects.
- **`nullabilityMismatch` — D0083.** A nullable column (or an explicit `lit(None)`) flowing into a slot the return schema declares non-null.

These three are advisory — they catch things that *work* but are usually mistakes. They stay quiet outside strict mode.

## Setup and import diagnostics

These fire before any schema checking — they mean pykrete couldn't read something.

- **`parseError` — D0001.** The file isn't valid Python. The message comes from Ruff's parser.
- **`unknownColumnType` / `invalidColumnType` — D0010 / D0011.** A `Schema` field's type annotation isn't a type pykrete recognizes.
- **`unknownSchema` / `invalidSchemaExpression` — D0020 / D0021.** A `DataFrame[X]` annotation where `X` isn't a known schema, or isn't a schema name / valid operator at all — usually a typo or a missing import.
- **`unresolvedImport` / `unexportedName` — D0070 / D0071.** An `import` that doesn't resolve, or a name the imported module doesn't export.
- **`duplicateSchemaName` — D0072.** The same `class X(Schema)` is declared in more than one file in the project. Pykrete picks one for cross-file resolution (the alphabetically-earliest declaration site), but the ambiguity is usually unintentional — a forgotten old copy, or two teams converging on the same name. Fires as a **warning** at every duplicate past the first, naming both files for context. Same-file redeclarations don't fire D0072 — that's a different concern.

## Changing severity

Any rule can be downgraded to a warning or switched off in `pykrete.json`:

```json
{
  "rules": {
    "unknownColumn": "error",
    "unionSchemaMismatch": "warning",
    "returnTypeMismatch": "off"
  }
}
```

Key by the rule name (or the `D00xx` code — both work). See [Configuration](/pykrete/reference/configuration/).
