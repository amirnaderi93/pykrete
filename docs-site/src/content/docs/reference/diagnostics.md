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
| `D0020` | `unknownSchema` | `SparkFrame[X]` / `PandasFrame[X]` / `DataFrame[X]` names a schema pykrete can't find. |
| `D0021` | `invalidSchemaExpression` | The thing inside `SparkFrame[…]` / `PandasFrame[…]` / `DataFrame[…]` isn't a schema name or a valid operator. |
| `D0030` | `unknownColumn` | A column reference doesn't exist on the schema in scope. |
| `D0040` | `unionSchemaMismatch` | `union` / `intersect` / `subtract` between dataframes whose columns don't match. |
| `D0050` | `returnColumnsMismatch` | A function's returned **columns** differ from its declared return schema. |
| `D0051` | `argumentColumnsMismatch` | A call-site argument's schema differs from the parameter's declared `SparkFrame[Schema]` (or `PandasFrame[Schema]`). |
| `D0060` | `missingJoinKey` | A join key isn't present on one side of the join. |
| `D0070` | `unresolvedImport` | An `import` can't be resolved. |
| `D0071` | `unexportedName` | An imported name isn't exported by the module. |
| `D0072` | `duplicateSchemaName` | The same schema name is declared in more than one project file. Warning. |
| `D0073` | `transformInputMismatch` | A `df.transform(fn)` receiver's schema doesn't match `fn`'s declared parameter schema. |
| `D0080` | `returnTypeMismatch` | A returned column's **type** differs from the declared return schema. |
| `D0081` | `nonNumericArithmetic` | Arithmetic on a non-numeric column. Strict mode only. |
| `D0082` | `crossTypeComparison` | A comparison between unrelated types. Strict mode only. |
| `D0083` | `nullabilityMismatch` | A nullable column flows into a slot the return schema declares non-null. Strict mode only. |
| `D0084` | `enumValueMismatch` | A string literal compared against, or written into, a column declared `enum[...]` is not in the column's vocabulary. |
| `D0090` | `deprecatedDataFrameAlias` | `DataFrame[X]` is used instead of the dialect-specific `SparkFrame[X]` / `PandasFrame[X]`. Warning. |

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

A function declared `-> SparkFrame[Schema]` returns a dataframe whose **column set** doesn't match — a column is missing, or an extra one is present.

```
summary.pyk:8:5 - error returnColumnsMismatch: declared return SparkFrame[SaleSummary] expects columns [region, total]; the body produces [region, amount].
```

**Fix:** correct the body, or re-anchor an opaque chain (like `spark.read.parquet(...)`) with `.cast(SparkFrame[Schema])` so pykrete knows the shape.

### `argumentColumnsMismatch` — D0051

The mirror of `returnColumnsMismatch`, one frame earlier: a `SparkFrame[…]` argument at a call site has a schema that doesn't match the parameter's declared `SparkFrame[Schema]`. Same missing / extra column reporting.

```
caller.pyk:13:13 - error argumentColumnsMismatch: Argument schema mismatch for parameter 'sales': expected SparkFrame[Sale], got schema 'Refund'. Missing: [amount]; extra: [refund].
```

Arguments whose schema pykrete can't infer (an untyped local, an opaque `spark.read.json(...)` chain) are silently skipped — the checker degrades rather than false-flag.

D0051 also respects Python's calling rules: a local name that rebinds a top-level function shadows it (the call resolves to the local, so the top-level signature isn't checked) — whether the rebind is a plain assignment, a tuple-unpack LHS (`revenue, _ = …`), or a walrus binding (`(revenue := …)`); positional-only (`/`) and keyword-only (`*`) markers are honored when matching arguments to parameters; `*args: SparkFrame[Schema]` / `**kwargs: SparkFrame[Schema]` variadics are checked against every argument that lands in them; and a parameter that's filled both positionally *and* by keyword (which Python rejects as `TypeError`) is diagnosed once, not twice.

**Fix:** pass a dataframe with the expected shape, or re-anchor the argument with `.cast(SparkFrame[Schema])` if the chain's schema was lost upstream.

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

### `enumValueMismatch` — D0084

A string literal compared against, or written into, a column declared
`enum["v1", "v2", ...]` (see [Enum-valued
strings](/reference/schemas/#enum-valued-strings--enuma-b-)) is not in
the column's vocabulary.

```pyk
class Order(Schema):
    id: long
    status: enum["pending", "shipped", "delivered", "cancelled"]

def stale(orders: SparkFrame[Order]) -> SparkFrame[Order]:
    return orders.filter(col("status") == "shippd")
    #                                       ^^^^^^^ D0084
```

```
orders.pyk:6:39 - error enumValueMismatch: 'shippd' is not in the enum vocabulary for 'status'. Did you mean 'shipped'?
```

Fires at every sink-bound site we check: `==` / `!=` against a string
literal, `.isin(...)`, `.fillna({...})`, `withColumn(name, lit(...))`,
`F.expr("col = 'lit'")` (and the SQL `IN (...)` form), and the
branch-form expressions `F.coalesce` / `F.when(...).otherwise(...)` /
`F.nvl` / `F.ifnull` / `F.nullif` when their output flows into an
enum-typed sink.

**Severity.** Error in every check mode — `basic`, `standard`, and
`strict`. Unlike D0081 / D0082 / D0083, this isn't an advisory; an
off-vocabulary literal is an unambiguous bug. Downgradable to warning
or off in `pykrete.json` like any other rule.

**Suggestion behavior.** When a close match exists in the vocabulary
the message carries a *did you mean*. The suggestion uses Levenshtein
distance — the same routine D0030 uses for column-name typos —
and ties are broken by Unicode code-point order (Rust `str::cmp`) so
the same input always yields the same suggestion.

## Setup and import diagnostics

These fire before any schema checking — they mean pykrete couldn't read something.

- **`parseError` — D0001.** The file isn't valid Python. The message comes from Ruff's parser.
- **`unknownColumnType` / `invalidColumnType` — D0010 / D0011.** A `Schema` field's type annotation isn't a type pykrete recognizes.
- **`unknownSchema` / `invalidSchemaExpression` — D0020 / D0021.** A `SparkFrame[X]` / `PandasFrame[X]` / `DataFrame[X]` annotation where `X` isn't a known schema, or isn't a schema name / valid operator at all — usually a typo or a missing import.
- **`unresolvedImport` / `unexportedName` — D0070 / D0071.** An `import` that doesn't resolve, or a name the imported module doesn't export.
- **`duplicateSchemaName` — D0072.** The same `class X(Schema)` is declared in more than one file in the project. Pykrete picks one for cross-file resolution (the alphabetically-earliest declaration site), but the ambiguity is usually unintentional — a forgotten old copy, or two teams converging on the same name. Fires as a **warning** at every duplicate past the first, naming both files for context. Same-file redeclarations don't fire D0072 — that's a different concern.
- **`transformInputMismatch` — D0073.** A `df.transform(fn)` call where the receiver `df`'s schema doesn't match `fn`'s declared `SparkFrame[Schema]` parameter. Spark's `.transform` is just a fluent-style apply — `df.transform(fn)` is `fn(df)` — so this is the same kind of shape check as D0051, surfaced at the call site where Spark would silently pass the wrong frame through. The message names the function, the expected schema, and the missing / extra columns.

  ```
  pipeline.pyk:12:8 - error transformInputMismatch: transform('add_total') expects a DataFrame matching schema 'Sale', but the receiver (schema 'Refund') does not. Missing: [amount]; extra: [refund].
  ```

  **Fix:** call `transform` on a frame whose schema matches `fn`'s parameter, or anchor an opaque chain with `.cast(SparkFrame[Schema])` so pykrete can see the shape.

### `deprecatedDataFrameAlias` — D0090

`DataFrame[X]` is the v1.0–v1.2 spelling. v1.3 introduces dialect-specific annotations — `SparkFrame[X]` for PySpark code and `PandasFrame[X]` for pandas code — and `DataFrame` now fires D0090 as a warning at every use.

D0090 fires at two annotation positions:

1. **Subscripted alias** — `DataFrame[X]` in any frame-annotation slot.
2. **Bare alias** — `DataFrame` (no subscript) in any frame-annotation slot. The bare form is itself the deprecated alias; pykrete treats it as `SparkFrame` (untyped) and fires D0090 per slot, exactly as for the subscripted form.

```pyk
class Sale(Schema):
    region: string
    amount: int

def revenue(sales: DataFrame[Sale]) -> DataFrame[Sale]:
    #                ^^^^^^^^^^^^^^^                   D0090
    #                                  ^^^^^^^^^^^^^^^  D0090
    return sales
```

```
sales.pyk:5:20 - warning deprecatedDataFrameAlias: 'DataFrame[Sale]' is a deprecated alias for 'SparkFrame[Sale]' and will be removed in pykrete v2.0. Rewrite as 'SparkFrame[Sale]'.
sales.pyk:5:40 - warning deprecatedDataFrameAlias: 'DataFrame[Sale]' is a deprecated alias for 'SparkFrame[Sale]' and will be removed in pykrete v2.0. Rewrite as 'SparkFrame[Sale]'.
```

The bare form fires the same way, with the message naming `DataFrame` / `SparkFrame` instead of the subscripted spelling:

```pyk
def passthrough(df: DataFrame) -> DataFrame:
    #               ^^^^^^^^^                D0090
    #                                 ^^^^^^^^^  D0090
    return df
```

```
sales.pyk:1:21 - warning deprecatedDataFrameAlias: 'DataFrame' is a deprecated alias for 'SparkFrame' and will be removed in pykrete v2.0. Rewrite as 'SparkFrame'.
sales.pyk:1:35 - warning deprecatedDataFrameAlias: 'DataFrame' is a deprecated alias for 'SparkFrame' and will be removed in pykrete v2.0. Rewrite as 'SparkFrame'.
```

Every `DataFrame` annotation fires — parameter, return, and any `.cast(DataFrame[X])` re-anchors all emit the warning independently. A function with two `DataFrame[Sale]` slots gets two warnings, as above.

**Severity.** Warning, not error — existing `DataFrame[X]` code keeps checking exactly as it did. The runtime is unaffected (the transpiler still strips `.cast(DataFrame[Schema])` re-anchors the same way it strips `.cast(SparkFrame[Schema])`). v1.6 will pair this diagnostic with the `pykrete migrate` auto-rewriter and escalate D0090 to error under strict mode in the same release — the breaking-change signal and the fix-button land together, so strict-mode users never see a silent escalation without a one-command remediation.

**Why dispatch matters.** v1.3 dispatches the six pandas operations — `df[col_list]` / `df[mask]` / `df["new"] = expr` / `df.drop` / `df.merge` / `df.rename` — based on which annotation the dataframe carries. A `PandasFrame[Sale]` parameter recognizes `sales[["region", "amount"]]` as a column-list select; a `SparkFrame[Sale]` parameter would flag the same code as a column-name typo (Spark doesn't subscript with a list). The dialect-specific annotation lets the check site say what it means. v1.5 extends dispatch across the dialect boundary: `df.toPandas()` re-tags `SparkFrame[X]` to `PandasFrame[X]`, and `spark.createDataFrame(pdf)` re-tags back when a schema source is present, so chains spanning both dialects stay checked.

**Sizing the migration.** `pykrete check --report-aliases` (v1.5+) emits a structured JSON envelope listing every `DataFrame[X]` annotation site in analyzed user code (function signatures, variable annotations, return types, cast targets) with its resolved dialect (`spark` or `pandas`) and the suggested replacement (`SparkFrame[X]` or `PandasFrame[X]`). The envelope carries its own `aliasReportVersion: "1"` so the report format can evolve independently of the diagnostic JSON contract. Pipe the report to your own tooling to quantify the v2.0 migration scope before v1.6's `pykrete migrate` ships.

```bash
pykrete check --report-aliases src/ > aliases.json
```

**Fix.** Rename `DataFrame[X]` to `SparkFrame[X]` for PySpark code or `PandasFrame[X]` for pandas code. The import path (`pyspark.sql.DataFrame` vs `pandas.DataFrame`) is unchanged; only the pykrete annotation slot is renamed.

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
