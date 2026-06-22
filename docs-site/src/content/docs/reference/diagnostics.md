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
| `D0080` | `returnTypeMismatch` | A returned column's **type** differs from the declared return schema. OR the declared dialect (`SparkFrame[X]` / `PandasFrame[X]`) differs from the dialect inferred from the body (new in v1.13). |
| `D0081` | `nonNumericArithmetic` | Arithmetic on a non-numeric column. Strict mode only. |
| `D0082` | `crossTypeComparison` | A comparison between unrelated types. Strict mode only. |
| `D0083` | `nullabilityMismatch` | A nullable column flows into a slot the return schema declares non-null. Strict mode only. |
| `D0084` | `enumValueMismatch` | A string literal compared against, or written into, a column declared `enum[...]` is not in the column's vocabulary. |
| `D0090` | `deprecatedDataFrameAlias` | `DataFrame[X]` is used instead of the dialect-specific `SparkFrame[X]` / `PandasFrame[X]`. Warning. |
| `D0091` | `crossDialectMethodMismatch` | A pandas-only method or attribute is accessed on a `SparkFrame[X]` receiver, or a Spark-only one on a `PandasFrame[X]` receiver. Warning; error in strict mode (v1.9+). |

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

**`returnTypeMismatch` — D0080.** On by default; emitted at `error` severity. The returned columns match the declared return schema by name, but a column's *type* doesn't — and both types are confidently known and genuinely incompatible. Numeric widening (`int` → `long` → `double`) is accepted; unknown types are left alone. The check is conservative on the column-type arm: it stays silent when either side's type is Unknown.

**New in v1.13: dialect mismatch.** When a function is annotated `-> SparkFrame[X]` but the body returns a `.toPandas()` chain (or any other expression that resolves to a `PandasFrame[…]`), D0080 fires with the message: `Return type mismatch: declared as SparkFrame schema 'X' but the body produces PandasFrame schema 'X'.` Honest-silence carve-out: constructor cases (`pd.DataFrame(...)`, `spark.read.parquet(...)`) where the body dialect is unknown don't fire (no fabrication). v1.10's bare-attribute D0091 carve-out for the deprecated `DataFrame[X]` annotation does NOT apply here — the return-type annotation IS the adjudication site. **Adopters with code that incorrectly cross-converts dialects at function boundaries will see new D0080 fires.** Fix: align the annotation with the body, OR rewrite the body to match the annotation.

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
sales.pyk:5:20 - warning deprecatedDataFrameAlias: 'DataFrame[Sale]' is a deprecated alias for 'SparkFrame[Sale]', slated for removal in a future pykrete v2.0. Rewrite as 'SparkFrame[Sale]', or run `pykrete check --deprecation-report` to inventory remaining sites.
sales.pyk:5:40 - warning deprecatedDataFrameAlias: 'DataFrame[Sale]' is a deprecated alias for 'SparkFrame[Sale]', slated for removal in a future pykrete v2.0. Rewrite as 'SparkFrame[Sale]', or run `pykrete check --deprecation-report` to inventory remaining sites.
```

The bare form fires the same way, with the message naming `DataFrame` / `SparkFrame` instead of the subscripted spelling:

```pyk
def passthrough(df: DataFrame) -> DataFrame:
    #               ^^^^^^^^^                D0090
    #                                 ^^^^^^^^^  D0090
    return df
```

```
sales.pyk:1:21 - warning deprecatedDataFrameAlias: 'DataFrame' is a deprecated alias for 'SparkFrame', slated for removal in a future pykrete v2.0. Rewrite as 'SparkFrame', or run `pykrete check --deprecation-report` to inventory remaining sites.
sales.pyk:1:35 - warning deprecatedDataFrameAlias: 'DataFrame' is a deprecated alias for 'SparkFrame', slated for removal in a future pykrete v2.0. Rewrite as 'SparkFrame', or run `pykrete check --deprecation-report` to inventory remaining sites.
```

Every `DataFrame` annotation fires — parameter, return, and any `.cast(DataFrame[X])` re-anchors all emit the warning independently. A function with two `DataFrame[Sale]` slots gets two warnings, as above.

**Severity.** Warning under `off` / `basic` / `standard` modes — existing `DataFrame[X]` code keeps checking exactly as it did. Under `"typeCheckingMode": "strict"` (v1.6+), D0090 lands as **error**. The pairing is atomic: `pykrete migrate` ships in the same release, so a strict-mode project that turns red on upgrade can be rewritten with one command. The runtime is unaffected (the transpiler still strips `.cast(DataFrame[Schema])` re-anchors the same way it strips `.cast(SparkFrame[Schema])`).

**Why dispatch matters.** v1.3 dispatches the six pandas operations — `df[col_list]` / `df[mask]` / `df["new"] = expr` / `df.drop` / `df.merge` / `df.rename` — based on which annotation the dataframe carries. A `PandasFrame[Sale]` parameter recognizes `sales[["region", "amount"]]` as a column-list select; a `SparkFrame[Sale]` parameter would flag the same code as a column-name typo (Spark doesn't subscript with a list). The dialect-specific annotation lets the check site say what it means. v1.5 extends dispatch across the dialect boundary: `df.toPandas()` re-tags `SparkFrame[X]` to `PandasFrame[X]`, and `spark.createDataFrame(pdf)` re-tags back when a schema source is present, so chains spanning both dialects stay checked.

**Sizing the migration.** `pykrete check --report-aliases` (v1.5+) emits a structured JSON envelope listing every `DataFrame[X]` annotation site in analyzed user code (function signatures, variable annotations, return types, cast targets) with its resolved dialect and suggested replacement. As of v1.6, the envelope's `resolvedDialect` field reports `spark` / `pandas` / `ambiguous` via call-graph dialect adjudication: each binding's downstream usage is inspected for Spark-only methods (`withColumn`, `createOrReplaceTempView`, `repartition`, …) versus pandas-only methods (`assign`, `pivot_table`, `.loc`, `.iloc`, `merge`, …). Both signals → `ambiguous`; only one → that dialect; no discriminating signal → defaults to `spark`. The envelope carries its own `aliasReportVersion` so the report format can evolve independently of the diagnostic JSON contract: v1.5 shipped `"1"` (value set: `{"spark"}`); v1.6 bumps to `"2"` for the value-set expansion to `{"spark", "pandas", "ambiguous"}`. Consumers that switched on `"spark"` only need to handle the new discriminators.

```bash
pykrete check --report-aliases src/ > aliases.json
```

The flag is invocation-only: it suppresses normal diagnostic output and always exits 0, since the report is informational rather than a diagnostic.

**Inventorying for CI gates (v1.8+).** `pykrete check --deprecation-report` is the v1.8 sibling envelope, purpose-built for v2.0 readiness gating. It emits the same per-site shape `--report-aliases` does (file, line, column, binding name, raw annotation, adjudicated dialect, suggested rewrite) plus an explicit `code: "D0090"` / `ruleName: "deprecatedDataFrameAlias"` on every site and a `summary: {totalSites, byDialect: {spark, pandas, ambiguous}}` block, so a CI step can decide whether to block a merge without re-parsing diagnostic text. The flag is mutually exclusive with `--report-aliases`; passing both exits 2 with a usage error.

**v1.9 — v2 envelope: `migrationStatus` + `--ack` filter.** Starting in v1.9, `deprecationReportVersion` bumps from `"1"` to `"2"`. Each per-site record gains `migrationStatus: "pending" | "acknowledged"` driven by a `# pykrete: ack-deprecation` comment marker on the line above the alias annotation — site-level opt-in, no JSON edit, no separate state file. A new `--ack=<pending|acknowledged>` filter flag narrows the envelope to one cohort so CI can gate site-by-site:

```bash
# Fail CI on any unacked D0090 site:
pykrete check --deprecation-report --ack=pending src/ > pending.json
test "$(jq '.summary.totalSites' < pending.json)" -eq 0

# Inverse: catch regressions where a site flipped acked → pending:
pykrete check --deprecation-report --ack=acknowledged src/ > acked.json
```

To mark a site acknowledged, drop the comment marker on the line above the annotation:

```pyk
# pykrete: ack-deprecation
def revenue(sales: DataFrame[Sale]) -> DataFrame[Sale]:
    ...
```

The envelope deliberately ships **without** `targetVersion` / `removalVersion` / `shipDate` fields: pykrete tracks per-site migration progress; the user picks the v2.0 ship date.

```bash
pykrete check --deprecation-report src/ > deprecation.json
# In CI, fail the build if the inventory is non-empty:
test "$(jq '.summary.totalSites' < deprecation.json)" -eq 0
```

Like `--report-aliases`, the flag is invocation-only — diagnostic output is suppressed and the command always exits 0 (the report is informational; gate on the JSON, not the exit code). The report honors no `rules` suppression — the inventory is the inventory, regardless of whether D0090 is silenced in `pykrete.json`.

**Fix — automated.** `pykrete migrate --apply src/` performs the rewrite. In v1.6, `--apply` was the implicit default; v1.7 flips the default to dry-run, so `--apply` is required to write. It rewrites every Spark-adjudicated site to `SparkFrame[X]`, every pandas-adjudicated site to `PandasFrame[X]`, and leaves ambiguous sites unchanged with an idempotent `# pykrete: ambiguous` marker on the line above so the user can adjudicate by hand. The rewrite is token-preserving (the only byte change in non-ambiguous lines is the `DataFrame` prefix) and atomic per file (sibling temp + rename, so an interrupted run never leaves half-rewritten source). In v1.6 the default mode of `pykrete migrate src/` was the in-place rewrite; v1.7 flips that to `--check` — `pykrete migrate src/` now previews per-site verdicts to stdout and exits 1 if any site needs attention. `--apply` is the new opt-in for the in-place rewrite, and `--diff src/` emits a `patch -p1`-compatible unified diff. A first-run on v1.7 with no flag emits a one-line stderr warning so adopters discover the change without reading release notes. See [cookbook recipe 6](/cookbook/#6-migrate-dataframex-to-the-v20-dialect-tagged-names) for the full workflow.

**Fix — manual.** Rename `DataFrame[X]` to `SparkFrame[X]` for PySpark code or `PandasFrame[X]` for pandas code. The import path (`pyspark.sql.DataFrame` vs `pandas.DataFrame`) is unchanged; only the pykrete annotation slot is renamed.

### `crossDialectMethodMismatch` — D0091

A method whose vocabulary belongs to one dialect is being called on a receiver tagged as the other dialect. Pandas-only methods called on `SparkFrame[X]` receivers (`sdf.assign(...)`, `sdf.merge(...)`, `sdf.rename(columns=...)`), and Spark-only methods called on `PandasFrame[X]` receivers (`pdf.withColumn(...)`, `pdf.selectExpr(...)`, `pdf.createOrReplaceTempView(...)`), both fire D0091 as a **warning** starting in v1.8. **In v1.9, D0091 escalates to error under `"typeCheckingMode": "strict"`** (mirroring the v1.6 D0090 precedent). Non-strict modes keep the warning unchanged.

```pyk
class Sale(Schema):
    region: string
    amount: int

def revenue(sales: PandasFrame[Sale]) -> PandasFrame[Sale]:
    return sales.withColumn("total", sales["amount"] * 2)
    #            ^^^^^^^^^^                                D0091
```

```
sales.pyk:5:18 - warning crossDialectMethodMismatch: 'withColumn' is a Spark-only DataFrame method but the receiver is a PandasFrame. Use '.assign(...)' instead.
```

**Suggestions.** D0091 carries a *use `.x(...)` instead* hint for the high-traffic cross-dialect pairs:

| Receiver dialect | Method called | Suggested replacement | Shape changes |
|---|---|---|---|
| `PandasFrame[X]` | `withColumn`, `withColumns` | `assign` | yes (kwarg vs positional) |
| `PandasFrame[X]` | `withColumnRenamed`, `withColumnsRenamed` | `rename` | yes (dict vs positional) |
| `PandasFrame[X]` | `selectExpr` | `eval` | yes |
| `PandasFrame[X]` | `toPandas` | `copy` | no |
| `SparkFrame[X]` | `assign` | `withColumn` | yes (positional vs kwarg) |
| `SparkFrame[X]` | `rename` | `withColumnRenamed` | yes (positional vs dict) |
| `SparkFrame[X]` | `groupby` | `groupBy` | no |
| `SparkFrame[X]` | `merge` | `join` | yes |

**`shape_changes` hint (v1.9).** Pairs whose call-site argument shape differs across dialects append "— note arg shape differs" to the suggestion text. For example, `withColumnRenamed("old", "new")` (Spark, two positionals) maps to `rename(columns={"old": "new"})` (pandas, kwarg with a dict) — pykrete still suggests the cross-dialect name, but the hint tells adopters that a name swap isn't enough on its own. The pair table above marks which pairs carry the hint. A suggestion-drift guard test pins the table at build time so adding a pair on one side without the other fails the build.

**Bare-attribute path (v1.9, surface-completed v1.10).** D0091 also fires on bare attribute access (no call), catching `pdf.rdd`, `sdf.loc`, `pdf.iloc`, `sdf.toPandas` and other cross-dialect attribute surfaces that the v1.8 `Expr::Call` path missed. The check is driven by two property tables: `SPARK_DISCRIMINATOR_PROPERTIES` — `rdd`, `isStreaming`, `sparkSession`, plus v1.10+ entries `na`, `write`, `writeStream`, `storageLevel` (7 entries) — and `PANDAS_INHERITED_PROPERTIES` — `loc`, `iloc`, `at`, `iat`, plus v1.10+ entries `index`, `values`, `shape`, `T` (8 entries). The bare-attribute path inherits the same carve-outs as the call path: untagged receivers skip the gate; deprecated `DataFrame[X]` alias receivers skip to avoid double-warning with D0090. v1.10 PR-D1's 8 new properties are unit-test-covered at v1.10.0; v1.11 ships the matching cross-codebase property probes (pykrete-tests PR-P1 #39 — closes the v1.10 PR-D1 carve-out). **New in v1.12**: D0080 `returnTypeMismatch` (a separate D-code, not D0091) gets its own cross-codebase trust coverage via pykrete-tests PR-P1 #42 — the longest-standing trust gap since v1.6 closed.

Methods without a clean cross-dialect equivalent (`mapInPandas`, `freqItems`, `pivot_table`, …) render a bare mismatch note without a suggestion. The suggestion field is also exposed via the LSP `Diagnostic.suggestion` slot, so editors that support `textDocument/codeAction` can light up a quick-fix.

**Carve-outs.** D0091 fires only on adjudicated receivers (`SparkFrame[X]` / `PandasFrame[X]`). Untagged bindings (parameters without a frame annotation) skip the gate. The deprecated `DataFrame[X]` alias also skips, so D0090 and D0091 don't double-fire on the same line — the v2.0 migration narrative is "adjudicate, then enforce". Two pandas-discriminator method names — `pivot` and `melt` — are excluded from the Spark-receiver direction because Spark exposes legitimate same-spelled surfaces (`groupBy(...).pivot(...)`, Spark 3.4+ positional `df.melt(ids, values, ...)`); firing on those would false-positive idiomatic Spark code. The pandas-direction check has no equivalent carve-out — every Spark discriminator is genuinely absent from the pandas DataFrame surface.

**Back-compat preservation.** Pre-v1.8, `pdf.withColumn(...)` typechecked silently as Spark — the existing un-gated `column_method_shape` arm still handles the call, schema flows through unchanged. D0091 is informational warning **alongside** the existing behavior in non-strict modes, not a replacement. Adopters who want the strict-mode escalation (v1.9) softened can downgrade D0091 to `warning` or `off` in `pykrete.json`'s `rules` block (`{"rules": {"crossDialectMethodMismatch": "warning"}}`).

**Fix.** Replace the method call with the dialect-appropriate spelling from the table above. If the receiver is genuinely the wrong dialect (the call won't work at runtime in the called library), fix the upstream chain — `.toPandas()` to convert a Spark receiver to pandas, `spark.createDataFrame(pdf)` to go the other way (v1.5+ cross-dialect handoff).

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
