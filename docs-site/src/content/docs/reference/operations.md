---
title: Operations
description: Every PySpark operation pykrete recognizes — what it checks, what it carries forward, and where chains end.
---

This page is the answer to "does pykrete model the Spark operation I care about?". For each operation, it tells you whether pykrete tracks the schema through it, validates the column names you pass, or steps out of the way and lets the chain end. Read it the way you'd read a language reference: scan the table, drill into the section that matters.

Every operation is marked with one of five status tags. They describe what you'll observe in the editor, not what's happening internally.

## Legend

| Tag | Meaning |
| --- | --- |
| **modeled** | pykrete computes the output schema and checks every column reference and (where applicable) type. The next call in the chain is fully checked too. |
| **pass-through** | pykrete carries the receiver's schema forward unchanged. Correct for ops that don't reshape (`cache`, `orderBy`, `limit`, ...). The chain keeps flowing. |
| **column-check only** | pykrete checks the column names you pass but doesn't re-derive the output schema. Typos still fire [`unknownColumn`](/pykrete/reference/diagnostics/#unknowncolumn--d0030); chains after this point may degrade. |
| **unmodeled** | pykrete doesn't understand the call. Column references inside the arguments may still be caught, but the chain after this point loses its schema. |
| **opaque** | Intentionally returns an unknown type — usually because the result genuinely depends on runtime data (UDF outputs, pandas conversions, RDD ops). Re-anchor with [`.cast(SparkFrame[X])`](#cast--the-re-anchor-primitive) if you want checking to resume. |

If you see a method below tagged something other than **modeled**, the operation itself still works at runtime — pykrete just won't catch typos *past* that point in the chain. Re-anchoring with `.cast(SparkFrame[X])` or a typed local annotation restores checking.

The rest of this page walks each operation category. For the workhorse methods there's a short worked example; for the long tail, a table suffices.

## Projection / column shaping

The headline operations — these are what makes a typo fireable.

```python
class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int

def f(sales: SparkFrame[Sale]) -> DataFrame:
    return (
        sales
        .select("region", "amount")          # modeled — output is {region, amount}
        .withColumn("doubled", F.col("amount") * 2)   # modeled — adds 'doubled: int'
        .drop("region")                       # modeled — output is {amount, doubled}
    )
```

A typo anywhere in that chain — `.select("regoin", ...)`, `.drop("amunt")` — fires [`unknownColumn`](/pykrete/reference/diagnostics/#unknowncolumn--d0030) against the schema at that point in the chain, not the original `Sale`.

| Method | Status | Notes |
| --- | --- | --- |
| `select` | modeled | `"*"` expands; output names come from `.alias()`, `F.col(...)`, or the string literal. |
| `selectExpr` | modeled | SQL parsed best-effort; identifiers checked. |
| `withColumn` | modeled | Replaces an existing column or appends a new one. |
| `withColumns` | modeled | Dict literal of name → expression (Spark 3.3+). |
| `withColumnRenamed` | modeled | Old name checked; type carried over to the new name. |
| `withColumnsRenamed` | modeled | Dict of renames (Spark 3.4+). |
| `drop` | modeled | String, `Column`, and `df.col` forms accepted. |
| `toDF` | modeled | Positional rename — types carried by position. |
| `alias` | pass-through | DataFrame alias for self-joins; schema unchanged. |
| `colRegex` | unmodeled | Returns an opaque column. |
| `withMetadata` | unmodeled | Metadata edits aren't tracked. |

## Filtering

| Method | Status | Notes |
| --- | --- | --- |
| `filter` | modeled | Column expression or SQL string — identifiers checked against the schema. |
| `where` | modeled | Alias of `filter`. |
| `dropDuplicates` | modeled | `subset=` keys checked; schema preserved. |
| `drop_duplicates` | modeled | Snake-case alias of `dropDuplicates` — same checks, same schema preservation. |
| `distinct` | pass-through | No columns named, no shape change. |
| `dropDuplicatesWithinWatermark` | unmodeled | Streaming-only. |

```python
def adults(people: SparkFrame[Person]) -> DataFrame:
    return people.filter(F.col("age") >= 18)   # 'age' checked against Person
```

SQL strings count too — `people.filter("age >= 18")` parses the predicate and checks each identifier.

## Joins / set operations

```python
class Sale(Schema):
    region: string
    amount: int

class Region(Schema):
    region: string
    manager: string

def with_manager(sales: SparkFrame[Sale], regions: SparkFrame[Region]) -> DataFrame:
    return sales.join(regions, "region", how="inner")
```

A wrong key — `.join(regions, "regoin")` — fires [`missingJoinKey`](/pykrete/reference/diagnostics/#missingjoinkey--d0060). A `union` between two dataframes whose columns don't agree fires [`unionSchemaMismatch`](/pykrete/reference/diagnostics/#unionschemamismatch--d0040).

| Method | Status | Notes |
| --- | --- | --- |
| `join` | modeled | Join keys checked (D0060); `how=` controls nullability of the right side. |
| `crossJoin` | modeled | Concatenates both schemas. |
| `union` | modeled | Schema-mismatch check (D0040). |
| `unionByName` | modeled | Same check as `union`, name-aligned. |
| `unionAll` | modeled | Deprecated alias for `union`; same check. |
| `intersect` | modeled | Same check as `union`; preserves the receiver's schema. |
| `intersectAll` | modeled | Like `intersect` but preserves duplicates. |
| `subtract` | modeled | Same check as `union`; preserves the receiver's schema. |
| `exceptAll` | modeled | Like `subtract` but preserves duplicates. |

## Aggregation

```python
class Sale(Schema):
    region: string
    amount: int

def revenue_by_region(sales: SparkFrame[Sale]) -> DataFrame:
    return (
        sales
        .groupBy("region")
        .agg(F.sum("amount").alias("total"))
    )
# Output schema: { region: string, total: long }
```

The result schema is the grouping keys plus each aggregation, named by `.alias(...)` or by the column being aggregated. Drop the alias and `total` would be named `sum(amount)` — both forms are tracked.

| Method | Status | Notes |
| --- | --- | --- |
| `groupBy` | modeled | Returns a grouped view; `agg` builds the output from keys + alias names. |
| `cube` | modeled | Same shape as `groupBy`. |
| `rollup` | modeled | Same shape as `groupBy`. |
| `agg` | modeled | Output = grouping keys + each aggregation's alias or referenced column. |
| `groupingSets` | unmodeled | Output schema not tracked. |

## Reshaping

| Method | Status | Notes |
| --- | --- | --- |
| `pivot` | column-check only | Pivot column checked; a follow-up `.agg(...)` still checks its column references against the pre-pivot schema, but the post-`.agg` output columns depend on runtime data, so the schema becomes opaque. |
| `melt` | modeled | `ids` and `values` column refs checked against the receiver; output schema is `ids + [variableColumnName: string, valueColumnName: T]`, where `T` is the common type across the `values` columns (or all non-`id` columns when `values` is omitted/`None`). `Nullable(T)` if any branch is nullable. Defaults: `variable` / `value`. Falls back to the receiver schema when `ids`/`values` aren't list literals of strings. |
| `unpivot` | modeled | Spark 3.4+ alias of `melt` — same shape and checks. |
| `transpose` | unmodeled | Spark 4.0+; unmodeled. |

`pivot` is the deliberate compromise here — its column names depend on the data, so pykrete checks what it can (the pivot key) and steps out of the way for the result. Use `.cast(SparkFrame[PivotedSchema])` when you're ready to resume checking on the pivoted output.

## Sampling / ordering / limits

All of these change rows or row order, never columns. The schema flows straight through.

| Method | Status | Notes |
| --- | --- | --- |
| `orderBy` | pass-through | Schema preserved (key not re-checked). |
| `sort` | pass-through | Same. |
| `sortWithinPartitions` | pass-through | Same. |
| `limit` | pass-through | Same. |
| `offset` | pass-through | Same. |
| `sample` | pass-through | Same. |
| `sampleBy` | pass-through | Stratified sampling — same schema-shape contract as `sample`. |
| `randomSplit` | unmodeled | Returns a list of frames — special-cased shape. |

## Caching / partitioning

Spark execution hints. None of them reshape data — schemas flow through.

| Method | Status | Notes |
| --- | --- | --- |
| `cache` | pass-through | |
| `persist` | pass-through | |
| `unpersist` | pass-through | |
| `checkpoint` | pass-through | |
| `localCheckpoint` | pass-through | |
| `coalesce` | pass-through | Partition count, not the column function. |
| `repartition` | pass-through | |
| `repartitionByRange` | pass-through | |
| `hint` | pass-through | |
| `storageLevel` | unmodeled | Property, not a schema-shaping op. |

## Type / schema introspection

### `.cast` — the re-anchor primitive

`.cast(SparkFrame[X])` is pykrete-specific. It tells the checker "treat this dataframe as having schema `X` from here on". It's how you bring an opaque chain back under checking:

```python
class Sale(Schema):
    region: string
    amount: int

def f(spark) -> DataFrame:
    return (
        spark.read.parquet("s3://...")          # opaque source → schema unknown
        .cast(SparkFrame[Sale])                   # re-anchored: schema = Sale
        .select("region", "amount")              # checked against Sale
    )
```

Equivalent forms work too — a typed local annotation does the same job:

```python
sales: SparkFrame[Sale] = spark.read.parquet("s3://...")
sales.select("region", "amount")   # checked
```

Use `.cast(SparkFrame[X])` after any operation tagged **opaque** or **unmodeled** to resume checking. It's not a runtime cast — at runtime it's an identity no-op.

### Other introspection methods

| Method | Status | Notes |
| --- | --- | --- |
| `cast(SparkFrame[X])` | modeled | See above. Re-anchors the chain. |
| `printSchema` | modeled | Recognized terminal — returns None; the chain ends. |
| `explain` | modeled | Recognized terminal — returns None; the chain ends. |
| `schema` | unmodeled | Property. |
| `columns` | unmodeled | Property. |
| `dtypes` | unmodeled | Property. |
| `isLocal` | unmodeled | Property. |
| `isEmpty` | unmodeled | Property. |

## IO (read / write / table / views)

Reading from external storage is intentionally opaque — pykrete can't see the parquet's actual schema. The expected pattern is `.cast(SparkFrame[X])` or a typed local annotation right after the read:

```python
class Sale(Schema):
    region: string
    amount: int

# Re-anchor pattern (preferred):
sales: SparkFrame[Sale] = spark.read.parquet("s3://sales/")
sales.select("region")           # checked

# Or chained:
spark.read.parquet("s3://sales/").cast(SparkFrame[Sale]).select("region")
```

`createOrReplaceTempView` is special — it registers the chain's schema against the view name, and a later `spark.sql("SELECT … FROM name")` in the same file resolves identifiers against that schema:

```python
sales.createOrReplaceTempView("sales_view")
spark.sql("SELECT region FROM sales_view WHERE amount > 0")   # checked
spark.sql("SELECT regoin FROM sales_view")
# error unknownColumn: Column 'regoin' does not exist on schema 'Sale'. Did you mean 'region'?
```

| Method | Status | Notes |
| --- | --- | --- |
| `dal.read(SOURCE)` | modeled | Generic class-method substitution; the schema-aware path. |
| `spark.read.parquet` / `.csv` / `.json` / `.orc` / `.text` / `.xml` / `.jdbc` / `.load` | opaque | Returns unknown — re-anchor with `.cast(SparkFrame[X])`. |
| `spark.read.format(...).load(...)` | opaque | Same. |
| `spark.read.schema(...).<format>(...)` | opaque | Same. |
| `spark.table` | opaque | Same. |
| `createOrReplaceTempView` | modeled | Registers receiver's schema; resolved by `spark.sql` in the same file. |
| `spark.sql("SELECT … FROM view")` | modeled | Single-table SELECT, within-file. Checks identifiers in SELECT / WHERE / GROUP BY / ORDER BY / HAVING. |
| `write` (`.parquet` / `.csv` / ...) | unmodeled | Terminal in practice. |
| `writeTo` | unmodeled | |
| `saveAsTable` | unmodeled | Terminal. |
| `createTempView` / `createGlobalTempView` | unmodeled | Use `createOrReplaceTempView` for view-based checking. |
| `registerTempTable` | unmodeled | Deprecated. |

`spark.sql` is single-table-SELECT only — joins, subqueries, and cross-file views fall back to best-effort behavior. If you need richer SQL checking, lean on the dataframe API instead.

### Reader-receiver heuristic

The `.read.<format>(…)` recognition is structural: pykrete matches any chain of the form `<X>.read.<format>(…)` (and the equivalent `<X>.read.format(...).load(...)` / `<X>.read.schema(...).<format>(...)` builder shapes) without verifying that `<X>` is a `SparkSession`. In practice `<X>` is `spark`, `ss`, `sess`, …; we deliberately don't pin the receiver name. The trade-off: a non-Spark API that happens to expose a `.read.<format>(...)` shape (e.g. an in-house loader) is also matched and yields **opaque** instead of the loader's real return type. If your own type-checker tooling agrees the loader returns a `DataFrame`, re-anchor with `.cast(SparkFrame[X])` — same workaround as a genuine `spark.read`. Tracked for a follow-up if the false-positive rate ever bites.

## Streaming

Structured streaming is a runtime concern — pykrete is a static schema checker, so this surface is out of scope by design.

| Method | Status | Notes |
| --- | --- | --- |
| `readStream` | unmodeled | |
| `writeStream` | unmodeled | |
| `isStreaming` | unmodeled | |
| `awaitTermination` | unmodeled | |

## Pandas-on-Spark / Arrow interop

Most of these cross out of the dataframe world (Arrow tables, UDF outputs, RDD ops), so pykrete returns an opaque type rather than guess. `.toPandas()` is the exception — v1.5 re-tags it as a dialect transition.

| Method | Status | Notes |
| --- | --- | --- |
| `toPandas` | modeled | v1.5+. `SparkFrame[X]` → `PandasFrame[X]` — the chain keeps tracking on the pandas side. Receiver must be DataFrame-bound and Spark-dialect; non-DataFrame helpers and Unknown receivers fall through. `arrow=True` and other kwargs are ignored, not gated. |
| `toArrow` | opaque | Result is an Arrow table. |
| `to_pandas_on_spark` | opaque | |
| `pandas_api` | opaque | |
| `mapInPandas` | opaque | UDF-shaped. |
| `applyInPandas` | opaque | UDF-shaped. |
| `mapInArrow` | opaque | UDF-shaped. |
| `mapPartitions` | opaque | RDD-level. |
| `foreach` / `foreachPartition` | unmodeled | Terminal. |

### `spark.createDataFrame(pdf)` — the pandas → Spark direction

The reverse handoff lives on the `SparkSession`, not the dataframe. `spark.createDataFrame(pdf)` re-tags `PandasFrame[Y]` back to `SparkFrame[Y]` when either (a) a `schema=` keyword argument resolves through a typed binding to `DataFrame[X]` / `SparkFrame[X]`, or (b) the call-arg expression types as `PandasFrame[Y]` via the recursive resolver. With neither schema source present, the call falls through to Unknown — pykrete won't auto-infer a schema from raw values. The round-trip `spark.createDataFrame(df.toPandas())` preserves the tag through the `.toPandas` arm above.

```python
def round_trip(sales: SparkFrame[Sale], spark) -> SparkFrame[Sale]:
    pdf = sales.toPandas()                    # pdf is PandasFrame[Sale]
    return spark.createDataFrame(pdf)         # back to SparkFrame[Sale]
```

## Other (na, stat, transform, terminals)

```python
class Sale(Schema):
    region: string
    amount: Optional[int]

def with_defaults(sales: SparkFrame[Sale]) -> DataFrame:
    return sales.fillna({"amount": 0})   # 'amount' checked; nullability cleared
```

`fillna` / `dropna` clear nullability on the affected columns. `replace` doesn't — it's value substitution, not null handling. `transform` resolves the function argument and checks its input + output against the surrounding schema.

| Method | Status | Notes |
| --- | --- | --- |
| `na.fill` | modeled | `subset=` checked; clears nullability. |
| `na.drop` | modeled | Same. |
| `na.replace` | modeled | `subset=` checked; preserves nullability. |
| `fillna` | modeled | Same as `na.fill`. |
| `dropna` | modeled | Same as `na.drop`. |
| `replace` | pass-through | Value-substitution; schema unchanged. |
| `transform` | modeled | `fn` argument resolved; input + output checked. |
| `count` | modeled | Recognized terminal (returns `long`). |
| `collect` | modeled | Recognized terminal on Spark (no pandas equivalent). |
| `take` | modeled | v1.6+. Spark receivers: recognized terminal. Pandas receivers: pass-through (`pdf.take([0, 2])` returns a row-sliced DataFrame, chain keeps tracking on `PandasFrame[X]`). |
| `first` / `head` / `tail` | modeled | v1.5+. Spark receivers: recognized terminals. Pandas receivers: pass-through (chain keeps tracking on `PandasFrame[X]`). |
| `show` | modeled | Recognized terminal (returns None). |
| `stat.crosstab` / `freqItems` / `approxQuantile` / `corr` / `cov` | unmodeled | |
| `summary` | opaque | Returns a statistics table whose schema depends on the receiver's numeric subset. Re-anchor with `.cast(SparkFrame[X])`. |
| `describe` | opaque | Same as `summary`. |
| `observe` | pass-through | Observability hook — returns the receiver unchanged. |
| `inputFiles` | unmodeled | |
| `sameSemantics` / `semanticHash` | unmodeled | |
| `rdd` | opaque | RDD-level. |

Terminal methods on Spark receivers — `count`, `collect`, `first`, `head`, `take`, `tail`, `show`, `printSchema`, `explain` — are recognized as "the chain ends here". They return scalars, lists, `Row`, or `None`, not dataframes. On pandas receivers, `first`, `head`, and `tail` are pass-through (they return a `DataFrame`); `count` deliberately stays terminal (it returns a per-column Series).

## Column functions (`F.*`)

Functions in `pyspark.sql.functions` show up inside the operations above — `df.select(F.upper("name"))`, `df.agg(F.sum("amount"))`. pykrete recognizes about 140 of them. Two things to know:

1. **Column refs are always checked.** A string-literal argument to a recognized `F.*` function is treated as a column reference. `F.sum("amunt")` fires [`unknownColumn`](/pykrete/reference/diagnostics/#unknowncolumn--d0030) the same way `df.select("amunt")` does.
2. **Result types are inferred for ~80 of them** — enough to power [`returnTypeMismatch`](/pykrete/reference/diagnostics/#type-checking-diagnostics) and downstream `.cast(...)` / arithmetic checks. The rest produce a column whose type is unknown until you re-anchor it — the chain keeps flowing, but downstream type checks against that column can't fire.

A spot-check of the families:

| Family | Examples | Column refs | Result type |
| --- | --- | --- | --- |
| Aggregate | `sum`, `avg`, `count`, `min`, `max`, `collect_list`, `stddev` | checked | inferred (e.g. `sum(int) → long`, `avg(*) → double`, `collect_list(T) → array<T>`) |
| Window | `row_number`, `rank`, `dense_rank`, `lag`, `lead` | checked | inferred (rank ops → int) |
| Math | `abs`, `round`, `sqrt`, `log`, `pow`, `sin`, `floor` | checked | inferred |
| String | `length`, `lower`, `upper`, `trim`, `concat`, `regexp_replace`, `split` | mostly checked | inferred |
| Date/time | `year`, `month`, `to_date`, `date_format`, `date_add`, `datediff`, `date_trunc` | first-arg checked | inferred |
| Collection | `array`, `explode`, `posexplode`, `size`, `array_distinct`, `map_keys` | checked | inferred (`posexplode` expands to `{pos: int, col: T}` inside `select` / `agg`) |
| Higher-order | `transform`, `filter`, `aggregate`, `exists`, `forall` | first-arg checked | inferred (lambda body resolved best-effort) |
| Conditional | `when` / `otherwise`, `coalesce`, `isnull` | walked | `when/otherwise` inferred from branches; `coalesce` drops nullability |
| Struct | `struct`, `named_struct` | walked | inferred — field names from `.alias()` / `F.col(...)` / literal name slots |
| Hash / id | `md5`, `sha1`, `sha2`, `monotonically_increasing_id` | checked | inferred |
| Sort helpers | `asc`, `desc`, `asc_nulls_first`, ... | checked | sort spec |

A handful of misc functions — `bin`, `conv`, `decode`, `encode`, `to_json`, `from_json`, `assert_true` — are unmodeled. `expr(...)` is partially modeled: its SQL string is parsed for column references (so typos still fire), but the result type isn't tracked.

Spark 3.4+ additions covered: `try_divide` (→ `double`), `any_value` (passthrough), `array_agg` (wraps as `array<T>`), `count_if` (→ `long`), `date_diff` (→ `int`), `unix_date` (→ `int`), `get` (array element type — the null-on-out-of-bounds sibling of `element_at`).

### `F.broadcast` — join optimization hint

`F.broadcast(df)` is a pass-through: it tells Spark to broadcast the dataframe in a join, but pykrete carries the wrapped frame's schema through unchanged. It's the only `F.*` function that operates on a whole dataframe rather than column expressions.

```python
class Sale(Schema):
    region: string
    amount: int

class Region(Schema):
    region: string
    manager: string

def with_manager(sales: SparkFrame[Sale], regions: SparkFrame[Region]) -> DataFrame:
    return sales.join(F.broadcast(regions), "region", how="inner")   # 'region' checked on both sides
```

| Function | Status | Notes |
| --- | --- | --- |
| `F.broadcast` | pass-through | Wraps a dataframe for broadcast-join hinting; receiver's schema preserved. |

### Column methods (`.alias`, `.cast`, `.isNull`, ...)

These are methods on a `Column` expression rather than `F.*` calls. The headline ones are all recognized:

| Method | Status | Notes |
| --- | --- | --- |
| `.alias` / `.name` | modeled | Used as the output column name in `select` / `agg`. |
| `.cast` | modeled | Result type follows the target name; nullability carried. |
| `.over` | modeled | Type passes through. |
| `.isNull` / `.isNotNull` | modeled | → bool. |
| `.isin` | modeled | → bool; value-arg column refs checked. |
| `.between` | modeled | → bool. |
| `.like` / `.rlike` / `.ilike` / `.contains` / `.startswith` / `.endswith` | modeled | → bool. |
| `.getField` | modeled | Resolves the nested struct field's type; D0030 on a field-name typo. |
| `.getItem` | modeled | → array element type or map value type. |
| `.withField` | modeled | → receiver struct with the field added or replaced. |
| `.dropFields` | modeled | → receiver struct with fields removed. |
| `.asc` / `.desc` | unmodeled | Sort spec; not currently a chain target. |
| `.eqNullSafe` | unmodeled | |
| `.substr` | unmodeled | |

### Window specs

| Form | Status | Notes |
| --- | --- | --- |
| `Window.partitionBy("k")` | modeled | Keys checked when `.over(...)` is applied to a known schema. |
| `Window.orderBy("k")` | modeled | Same. |
| `Window.partitionBy(...).orderBy(...)` chain | modeled | Builder walked end-to-end. |
| `Window.rowsBetween` / `rangeBetween` | unmodeled | No column refs to check. |
| `Window.unboundedPreceding` / `unboundedFollowing` / `currentRow` | unmodeled | Constants. |

## Pandas dispatch

Added in v1.3 (column-reference recognition) and completed in v1.4 (positive type-tracking parity), pykrete supports a pandas check-site dialect alongside the PySpark surface above. Annotate a parameter with `PandasFrame[Schema]` and pykrete switches to the pandas operation shapes — same column-reference checking, different syntax.

```python
import pandas as pd

class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int

def revenue(sales: PandasFrame[Sale]) -> pd.DataFrame:
    return (
        sales[sales["amount"] > 0]                 # filter via df[mask]
        [["region", "amount"]]                     # select via df[col_list]
    )
```

A typo anywhere — `sales["amunt"]`, `sales[["regoin", "amount"]]` — fires `unknownColumn` against `Sale` the same way PySpark column refs do. The dialect is the only thing that changes; the diagnostic story is identical.

### Dialect-gated `.head` / `.tail` / `.first`

PySpark recognizes `.head()`, `.tail()`, `.first()`, and `.take()` as chain-ending terminals (they return a `Row` / list of `Row`s, not a `DataFrame`). In pandas, the same four names return a sliced `DataFrame` — `pdf.head(10).merge(other, on="id")` and `pdf.take([0, 2]).merge(other, on="id")` are canonical pandas code. v1.5 dialect-gated `.head` / `.tail` / `.first`; v1.6 closes the same gate on `.take()`. Pandas receivers (`PandasFrame[X]`) pass through unchanged; Spark receivers stay terminals. Pandas `count()` deliberately stays terminal (it returns a per-column Series).

```python
def first_n(orders: PandasFrame[Order], other: PandasFrame[OtherSchema]) -> pd.DataFrame:
    return orders.head(100).merge(other, on="id")   # 'id' checked against Order and OtherSchema
```

### `.loc[:, "col"]` literal-form (v1.5, v1.6 nested-arg FP closure)

`pdf.loc[:, "col"]` resolves the string-literal column against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. Only the literal form lands in v1.5; variable column keys (`pdf.loc[:, col_var]`), boolean-mask row-key tracking (`pdf.loc[mask, "col"]`), column-range slicing (`pdf.loc[:, "a":"b"]`), and `pdf.iloc[...]` fall through to Unknown and are deferred to v1.9 paired with broader pandas reshape. v1.6 closes the `pdf.loc[mask, "col"]` nested-arg D0030 false positive on the row-mask side: the row-mask now falls through to Unknown (deferred per spec) while the column-literal arm still fires D0030 on a typo.

### `.pivot_table(index=, columns=, values=, aggfunc=)` literal-form (v1.6)

`pdf.pivot_table(index="cat", columns="year", values="amount", aggfunc="sum")` resolves the string-literal arguments to `index` / `columns` / `values` / `aggfunc` against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. List-of-literals shapes (`index=["a", "b"]`) are also checked. Variable arguments (`index=col_var`), callable `aggfunc` (`aggfunc=np.mean`), and the no-arg form fall through to Unknown. Full `pivot_table` schema-tracking (the wide output schema — variable column values become column names of the result frame) is deferred to v1.9 paired with broader pandas reshape (`stack` / `unstack` / `groupby.agg`).

### `.melt(id_vars=, value_vars=, var_name=, value_name=)` literal-form (v1.7)

`pdf.melt(id_vars=["a", "b"], value_vars=["c", "d"], var_name="variable", value_name="value")` resolves the string-literal arguments to `id_vars` / `value_vars` against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. List-of-literals shapes (`id_vars=["a", "b"]`) and single-literal `id_vars="a"` are also checked. Variable arguments (`id_vars=cols_var`) and the no-arg form fall through to Unknown. The pandas dispatch is gated on `receiver_is_pandas_inherited`, so the existing Spark `melt`/`unpivot` arm's behavior on `SparkFrame[X]` receivers is unchanged. Full `melt` output schema-tracking (the long-format schema with `var_name` / `value_name` as columns) is deferred to v1.9 paired with `stack` / `unstack` / `groupby.agg`.

### Six dispatched operations

These are the operations that read or write *via the dataframe itself*, not via a method call — Spark and pandas spell them differently, so v1.3 dispatches them on the annotation:

| Operation | PySpark form | pandas form |
| --- | --- | --- |
| Select columns | `df.select("region", "amount")` | `df[["region", "amount"]]` |
| Filter rows | `df.filter(F.col("amount") > 0)` | `df[df["amount"] > 0]` |
| Add / replace column | `df.withColumn("doubled", F.col("amount") * 2)` | `df["doubled"] = df["amount"] * 2` |
| Drop columns | `df.drop("region")` | `df.drop(columns=["region"])` |
| Join | `df.join(other, "region", "inner")` | `df.merge(other, on="region", how="inner")` |
| Rename | `df.withColumnRenamed("region", "country")` | `df.rename(columns={"region": "country"})` |

Each one reports column refs against the schema in scope; the output schema flows forward the same way it does in PySpark chains.

`df.assign(new=expr)` is the kwarg form of `df["new"] = expr` and dispatches identically — same widening, same column-ref checking on `expr`.

### Widening for `df["new"] = expr`

A column assignment of the form `df["new"] = expr` widens the schema with a new column whose type follows from `expr` — same inference path that powers `withColumn`. Reassigning an existing column replaces it; the runtime semantics are pandas's, but pykrete tracks the post-assignment shape so chained reads stay checked.

```python
def annotate(sales: PandasFrame[Sale]) -> pd.DataFrame:
    sales["total"] = sales["amount"] * sales["quantity"]    # widens schema with 'total: int'
    return sales[["region", "total"]]                        # checked against widened shape
```

### What pandas check-site coverage means

v1.3 shipped **column-reference recognition** for pandas — the six dispatched operations above plus the D0090 deprecation. Positive **type-tracking** verification for pandas (the `PROBE-TYPE-IS` parity that PySpark got in v1.2) shipped in v1.4; the [pykrete-tests#14](https://github.com/amirnaderi93/pykrete-tests/issues/14) tracker closed with that release.

For the PySpark-only operations on this page (joins / aggregations / windows / IO), `PandasFrame[X]` chains fall back to **opaque** — pykrete doesn't model pandas's `groupby` / `agg` / `read_parquet` / window surface yet. Re-anchor with `.cast(PandasFrame[X])` when needed.

## What's not modeled — by design

Some of the surface is intentionally outside pykrete's reach. These aren't gaps to fill — they're runtime concerns, not schema concerns:

- **Structured streaming** (`readStream`, `writeStream`, `isStreaming`). pykrete is a static checker against declared schemas; streaming state is a runtime construct.
- **Arrow conversions, pandas-on-Spark, and UDF-shaped pandas interop** (`toArrow`, `mapInPandas`, `applyInPandas`, `mapInArrow`, `pandas_api`, ...). The result isn't a vanilla dataframe anymore. pandas check-site coverage shipped in v1.3 as its own typed surface (`PandasFrame[X]`) — see the [Pandas dispatch](#pandas-dispatch) section above. v1.5 added the `.toPandas()` and `spark.createDataFrame(pdf)` cross-dialect handoff (re-tagging `SparkFrame[X]` ↔ `PandasFrame[X]` at those two seams); v1.6 added the `.take()` pandas dialect-gate and `pivot_table` literal-form. Polars is tracked for v1.8+ on the [roadmap](/pykrete/about/roadmap/).
- **RDD-level operations** (`rdd`, `mapPartitions`, `foreach`). These drop below the dataframe abstraction by design.
- **Runtime introspection** (`describe`, `summary`, `stat.*`). These return shape-of-data summaries, not schemas.
- **UDF internals**. The decorator's return type is honored, but the body is opaque.

For all of these, the chain ends or becomes opaque at the call site. Downstream code that needs to be checked can resume with [`.cast(SparkFrame[X])`](#cast--the-re-anchor-primitive) or a typed local annotation.

## See also

- [Schemas](/pykrete/reference/schemas/) — how to declare the shapes the operations above check against.
- [Diagnostics](/pykrete/reference/diagnostics/) — the full list of errors, including [`D0030 unknownColumn`](/pykrete/reference/diagnostics/#unknowncolumn--d0030) (the workhorse), [`D0040 unionSchemaMismatch`](/pykrete/reference/diagnostics/#unionschemamismatch--d0040), and [`D0060 missingJoinKey`](/pykrete/reference/diagnostics/#missingjoinkey--d0060).
- [Configuration](/pykrete/reference/configuration/) — turn individual rules into warnings or off.
