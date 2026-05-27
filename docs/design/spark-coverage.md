# PySpark coverage

How pykrete handles every method/function on the Spark public surface.
A living document — update when a method's handling changes. Sources
of truth: [`crates/pykrete/src/operations.rs`](../../crates/pykrete/src/operations.rs)
(method dispatch, `infer_expr_type`, `function_result_type`,
`COLUMN_REF_FUNCTIONS`, `is_pass_through_method`) and the
[integration tests](../../crates/pykrete/tests/).

Legend:

- ✅ **modeled** — schema computed, column refs + types checked
- ⚙️ **pass-through** — schema preserved (correct for non-reshaping ops)
- 🔍 **column-check only** — referenced columns validated, schema unchanged
- ⚠️ **unmodeled, chain dies** — subsequent chain checks don't fire
- 🚫 **intentionally opaque** — returns `Unknown` by design
- ❓ **unclear** — needs investigation

## Summary

Roughly **30 / ~150 `DataFrame` methods** modeled, plus **~25 always-safe
pass-throughs** — the chain-killing surface around them is the headline
gap. **~140 `F.*` helpers** are recognized as column references; **~80**
of those additionally produce a typed result.

The headline gaps:

- **Reshaping** — `melt` / `unpivot` / `transpose` are unmodeled; `pivot`
  is column-checked but its output schema is data-dependent (deliberate).
- **Set operations** — `intersect`, `intersectAll`, `subtract`,
  `exceptAll` aren't recognized (only `union` / `unionByName` are).
- **Streaming** — `readStream` / `writeStream` / `isStreaming` are
  entirely unmodeled.
- **Pandas / Arrow interop** — `toPandas`, `toArrow`, `mapInPandas`,
  `applyInPandas` are opaque by design.
- **Introspection / terminal ops** — `count`, `collect`, `show`,
  `printSchema`, `explain`, `describe`, `summary`, `first`, `take`,
  `head` aren't recognized as terminals, but they are typically the
  last step so the chain-death is moot in practice.

## DataFrame methods

### Projection / column shaping

| Method | State | Notes |
| --- | --- | --- |
| `select` | ✅ | `"*"` expands; output names from alias/col/string |
| `selectExpr` | ✅ | SQL parsed best-effort, idents checked |
| `withColumn` | ✅ | Replaces an existing name or appends |
| `withColumns` | ✅ | Dict literal; Spark 3.3+ |
| `withColumnRenamed` | ✅ | Old name checked, type carried over |
| `withColumnsRenamed` | ✅ | Spark 3.4+, dict of renames |
| `drop` | ✅ | String / col / `df.col` accepted |
| `toDF` | ✅ | Positional rename; types carried by position |
| `alias` | ⚙️ | Schema unchanged |
| `colRegex` | ⚠️ | Unmodeled |
| `withMetadata` | ⚠️ | Unmodeled |

### Filtering

| Method | State | Notes |
| --- | --- | --- |
| `filter` | ✅ | Column expr or SQL string; idents checked |
| `where` | ✅ | Alias of `filter` |
| `dropDuplicates` | ✅ | `subset=` checked |
| `drop_duplicates` | 🔍 | `subset=` checked; schema not re-derived |
| `distinct` | ⚙️ | Pass-through |
| `dropDuplicatesWithinWatermark` | ⚠️ | Unmodeled |

### Joins / set operations

| Method | State | Notes |
| --- | --- | --- |
| `join` | ✅ | Key check (D0060), how= → nullability |
| `crossJoin` | ✅ | Concatenates both schemas |
| `union` | ✅ | Schema-mismatch check (D0040) |
| `unionByName` | ✅ | Same check as union |
| `unionAll` | ⚠️ | Deprecated alias; unmodeled |
| `intersect` | ⚠️ | Unmodeled |
| `intersectAll` | ⚠️ | Unmodeled |
| `subtract` | ⚠️ | Unmodeled |
| `exceptAll` | ⚠️ | Unmodeled |

### Aggregation

| Method | State | Notes |
| --- | --- | --- |
| `groupBy` | ✅ | Returns `Grouped`; agg uses keys + aliases |
| `cube` | ✅ | Same shape as groupBy |
| `rollup` | ✅ | Same shape as groupBy |
| `agg` | ✅ | Result = keys + each arg's alias/col |
| `groupingSets` | ⚠️ | Unmodeled |

### Reshaping (pivot, unpivot, transpose, melt)

| Method | State | Notes |
| --- | --- | --- |
| `pivot` | 🔍 | Column checked; output data-dependent, returns Unknown |
| `unpivot` | ⚠️ | Unmodeled |
| `melt` | ⚠️ | Unmodeled |
| `transpose` | ⚠️ | Unmodeled |

### Sampling / ordering / limits

| Method | State | Notes |
| --- | --- | --- |
| `orderBy` | ⚙️ | Pass-through (key not re-checked) |
| `sort` | ⚙️ | Pass-through |
| `sortWithinPartitions` | ⚙️ | Pass-through |
| `limit` | ⚙️ | Pass-through |
| `offset` | ⚙️ | Pass-through |
| `sample` | ⚙️ | Pass-through |
| `sampleBy` | ⚠️ | Unmodeled |
| `randomSplit` | ⚠️ | Unmodeled (returns a list) |

### Caching / partitioning

| Method | State | Notes |
| --- | --- | --- |
| `cache` | ⚙️ | Pass-through |
| `persist` | ⚙️ | Pass-through |
| `unpersist` | ⚙️ | Pass-through |
| `checkpoint` | ⚙️ | Pass-through |
| `localCheckpoint` | ⚙️ | Pass-through |
| `coalesce` | ⚙️ | Pass-through (partition count) |
| `repartition` | ⚙️ | Pass-through |
| `repartitionByRange` | ⚙️ | Pass-through |
| `hint` | ⚙️ | Pass-through |
| `storageLevel` | ⚠️ | Unmodeled |

### Type / schema introspection

| Method | State | Notes |
| --- | --- | --- |
| `cast(DataFrame[X])` | ✅ | pykrete-only; re-anchors the chain |
| `printSchema` | ⚠️ | Unmodeled (terminal in practice) |
| `schema` | ⚠️ | Property, unmodeled |
| `columns` | ⚠️ | Property, unmodeled |
| `dtypes` | ⚠️ | Property, unmodeled |
| `isLocal` | ⚠️ | Unmodeled |
| `isEmpty` | ⚠️ | Unmodeled |
| `explain` | ⚠️ | Unmodeled (terminal in practice) |

### IO (read/write)

| Method | State | Notes |
| --- | --- | --- |
| `dal.read(SOURCE)` | ✅ | Generic class-method substitution; the intended path |
| `spark.read` / `.parquet` / `.csv` / `.json` / `.orc` / `.text` / `.xml` / `.jdbc` / `.load` | ✅ | Recognized as opaque source; returns Unknown — re-anchor with `.cast(DataFrame[X])` or a typed variable annotation |
| `spark.read.format(...).load(...)` / `.schema(...).<format>(...)` | ✅ | Builder forms recognized; same opaque-source treatment |
| `spark.table` | ✅ | Recognized as opaque source; same re-anchor pattern |
| `write` (`.parquet` / `.csv` / …) | ⚠️ | Unmodeled (terminal in practice) |
| `writeTo` | ⚠️ | Unmodeled |
| `saveAsTable` | ⚠️ | Unmodeled (terminal) |
| `createOrReplaceTempView` | ⚠️ | Unmodeled (terminal) |
| `createTempView` / `createGlobalTempView` | ⚠️ | Unmodeled |
| `registerTempTable` | ⚠️ | Deprecated; unmodeled |

### Streaming

| Method | State | Notes |
| --- | --- | --- |
| `readStream` | ⚠️ | Unmodeled |
| `writeStream` | ⚠️ | Unmodeled |
| `isStreaming` | ⚠️ | Unmodeled |
| `awaitTermination` | ⚠️ | Unmodeled |

### Pandas-on-Spark / Arrow interop

| Method | State | Notes |
| --- | --- | --- |
| `toPandas` | 🚫 | Opaque; not a DataFrame after |
| `toArrow` | 🚫 | Opaque |
| `to_pandas_on_spark` | 🚫 | Opaque |
| `pandas_api` | 🚫 | Opaque |
| `mapInPandas` | 🚫 | UDF-shaped, opaque |
| `applyInPandas` | 🚫 | UDF-shaped, opaque |
| `mapInArrow` | 🚫 | UDF-shaped, opaque |
| `mapPartitions` | 🚫 | RDD-level, opaque |
| `foreach` / `foreachPartition` | ⚠️ | Terminal, unmodeled |

### Other (na, stat, sql, transforms)

| Method | State | Notes |
| --- | --- | --- |
| `na.fill` | ✅ | `subset=` checked; clears nullability |
| `na.drop` | ✅ | `subset=` checked; clears nullability |
| `na.replace` | ✅ | `subset=` checked; preserves nullability |
| `fillna` | ✅ | Same as `na.fill` |
| `dropna` | ✅ | Same as `na.drop` |
| `replace` | ⚙️ | Pass-through |
| `transform` | ✅ | `fn` arg resolved; input + output checked |
| `spark.sql("…")` | ✅ | Projection columns inferred best-effort |
| `stat.crosstab` / `freqItems` / `approxQuantile` / `corr` / `cov` | ⚠️ | Unmodeled |
| `summary` | ⚠️ | Unmodeled |
| `describe` | ⚠️ | Unmodeled |
| `count` | ⚠️ | Returns long, terminal in practice |
| `collect` / `first` / `head` / `take` / `tail` | ⚠️ | Terminal, unmodeled |
| `show` | ⚠️ | Terminal, unmodeled |
| `observe` | ⚠️ | Unmodeled |
| `inputFiles` | ⚠️ | Unmodeled |
| `sameSemantics` / `semanticHash` | ⚠️ | Unmodeled |
| `rdd` | 🚫 | RDD-level, opaque |

## pyspark.sql.functions (F.*)

Every entry below is recognized — its string-literal args are column refs
(D0030 fires on a bad name). The **typed** column says whether the result
column's type is inferred (`infer_expr_type` / `function_result_type`).

### Aggregate

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `sum`, `sumDistinct`, `sum_distinct` | ✅ | ✅ | int/long → long, double → double |
| `avg`, `mean` | ✅ | ✅ | → double |
| `count`, `countDistinct`, `count_distinct`, `approx_count_distinct` | ✅ | ✅ | → long |
| `min`, `max` | ✅ | ✅ | Same type as input |
| `first`, `last`, `first_value`, `last_value` | ✅ | ✅ | Same type as input |
| `max_by`, `min_by` | ✅ | ⚠️ | Listed in COLUMN_REF; type not inferred |
| `collect_list`, `collect_set` | ✅ | ✅ | → `array<T>` |
| `median`, `percentile`, `percentile_approx` | ✅ | ⚠️ | Type not inferred |
| `variance`, `var_pop`, `var_samp`, `stddev`, `stddev_pop`, `stddev_samp` | ✅ | ✅ | → double |
| `skewness`, `kurtosis`, `corr`, `covar_pop`, `covar_samp` | ✅ | ✅ | → double |
| `grouping`, `grouping_id` | ✅ | ⚠️ | Type not inferred |

### Window

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `row_number`, `rank`, `dense_rank`, `ntile` | ✅ | ✅ | → int |
| `percent_rank`, `cume_dist` | ✅ | ✅ | → double |
| `lag`, `lead`, `nth_value` | ✅ | ⚠️ | Type not inferred |

### Math / numeric

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `abs`, `round`, `bround`, `negative`, `positive` | ✅ | ✅ | Input type |
| `ceil`, `ceiling`, `floor` | ✅ | ✅ | → long |
| `sqrt`, `exp`, `expm1`, `ln`, `log`, `log2`, `log10`, `log1p` | ✅ | ✅ | → double |
| `pow`, `power`, `hypot`, `signum`, `factorial` | ✅ | ✅ | → double / long |
| `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` | ✅ | ✅ | → double |
| `degrees`, `radians`, `cbrt` | ✅ | ✅ | → double |
| `rand`, `randn` | ⚠️ | ✅ | No-arg; not in COLUMN_REF list |
| `greatest`, `least`, `nanvl` | ✅ | ✅ | Input type |

### String

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `length`, `char_length`, `character_length` | ✅ | ✅ | → int |
| `lower`, `upper`, `initcap`, `trim`, `ltrim`, `rtrim`, `reverse` | ✅ | ✅ | → string |
| `concat`, `concat_ws` | ✅* | ✅ | concat_ws excluded from COLUMN_REF (mixed args) |
| `substring`, `substring_index`, `lpad`, `rpad`, `repeat`, `translate` | ⚠️ | ✅ | Mixed-arg; not in COLUMN_REF |
| `regexp_replace`, `regexp_extract` | ⚠️ | ✅ | Mixed-arg; not in COLUMN_REF |
| `split` | ⚠️ | ✅ | → `array<string>`; mixed-arg |
| `ascii`, `soundex`, `base64`, `unbase64` | ✅ | ✅ | → string / int |
| `instr`, `locate`, `levenshtein` | ⚠️ | ✅ | Mixed-arg; not in COLUMN_REF |
| `format_string`, `format_number` | ⚠️ | ✅ | Mixed-arg |
| `hex`, `unhex` | ⚠️ | ✅ | → string |
| `like`, `rlike`, `contains`, `startswith`, `endswith` | ⚠️ | ⚠️ | Column methods; unmodeled |

### Date / time

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `year`, `month`, `dayofmonth`, `day`, `dayofweek`, `dayofyear`, `hour`, `minute`, `second`, `weekofyear`, `quarter` | ✅ | ✅ | → int |
| `last_day` | ✅ | ✅ | → date |
| `date_add`, `date_sub`, `add_months` | ✅ | ✅ | → date |
| `datediff`, `months_between` | ✅ | ✅ | → int / double |
| `to_date`, `to_timestamp` | ⚠️ | ✅ | Mixed-arg (format); not in COLUMN_REF |
| `current_date`, `current_timestamp` | ⚠️ | ✅ | No-arg |
| `date_format`, `date_trunc`, `trunc`, `next_day` | ⚠️ | ✅ | Mixed-arg |
| `from_utc_timestamp`, `to_utc_timestamp`, `from_unixtime`, `unix_timestamp` | ⚠️ | ✅ | Mixed-arg |
| `window`, `session_window` | ⚠️ | ⚠️ | Unmodeled |

### Collection (array, map, struct)

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `array`, `array_repeat`, `sequence` | ✅* | ✅ | → `array<T>`; only `array` in COLUMN_REF |
| `array_distinct`, `array_sort`, `sort_array`, `array_union`, `array_except`, `array_intersect`, `array_remove`, `array_compact`, `shuffle`, `slice` | ✅* | ✅ | `array_*` in COLUMN_REF; preserves elem type |
| `array_max`, `array_min` | ✅ | ⚠️ | In COLUMN_REF; result type not inferred |
| `flatten` | ⚠️ | ✅ | Peels one layer |
| `explode`, `explode_outer` | ✅ | ✅ | `array<T>` → T |
| `posexplode`, `posexplode_outer` | ✅ | ⚠️ | Produces two cols; result not modeled |
| `element_at` | ⚠️ | ✅ | Array elem or map value |
| `size` | ✅ | ✅ | → int |
| `arrays_zip` | ⚠️ | ✅ | → `array<struct>` |
| `struct`, `named_struct` | ⚠️ | ⚠️ | Result struct fields not modeled |
| `create_map`, `map_from_arrays`, `map_from_entries`, `map_concat`, `str_to_map`, `transform_keys`, `transform_values`, `map_filter` | ⚠️ | ✅ | → map (K/V untyped) |
| `map_keys`, `map_values` | ✅ | ✅ | → `array<K>` / `array<V>` |
| `map_entries` | ✅ | ✅ | → array |
| `transform`, `filter`, `aggregate`, `zip_with`, `exists`, `forall` (higher-order) | ⚠️ | ⚠️ | Unmodeled |

### Conditional

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `when`, `otherwise` | ⚠️ | ⚠️ | Excluded from COLUMN_REF (mixed args); column refs inside arguments are still collected via recursion |
| `coalesce`, `nvl`, `ifnull` | ✅ | ✅ | Drops nullability |
| `nullif`, `nvl2` | ✅ | ⚠️ | Type not inferred |
| `isnull`, `isnan` | ✅ | ✅ | → bool |

### Hash / id

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `hash`, `xxhash64`, `crc32` | ✅* | ⚠️ | `hash` and `crc32` in COLUMN_REF |
| `md5`, `sha1`, `sha2` | ✅ | ✅ | → string |
| `monotonically_increasing_id` | ✅ | ✅ | → long |
| `spark_partition_id`, `input_file_name` | ✅ | ✅ | → int / string |

### Sort helpers

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `asc`, `asc_nulls_first`, `asc_nulls_last`, `desc`, `desc_nulls_first`, `desc_nulls_last` | ✅ | ⚠️ | In COLUMN_REF; result is a sort spec |

### Misc

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `col`, `column` | ✅ | ✅ | Resolves against the active schema |
| `lit` | ⚠️ | ✅ | Python-literal type; `lit(None)` tracked as nullable |
| `expr` | ⚠️ | ⚠️ | SQL parsed via `sqlparser`; idents D0030-checked |
| `broadcast` | ⚠️ | ⚠️ | Unmodeled (would be pass-through) |
| `udf`, `pandas_udf` | 🚫 | ✅ | UDF return type from decorator / functional form |
| `assert_true`, `raise_error` | ⚠️ | ⚠️ | Unmodeled |
| `bitwiseNOT`, `shiftLeft`, `shiftRight`, `shiftRightUnsigned`, `bitwise_*` | ⚠️ | ⚠️ | Unmodeled |
| `bin`, `conv`, `decode`, `encode` | ⚠️ | ⚠️ | Unmodeled |
| `to_json`, `from_json`, `schema_of_json`, `to_csv`, `from_csv` | ⚠️ | ⚠️ | Unmodeled |
| `version`, `current_user`, `current_database`, `current_schema`, `current_timezone` | ⚠️ | ⚠️ | Unmodeled |

### Column methods (chained on a Column expression)

These are methods on a `Column`, not `F.*` calls — pykrete does not
model them, but column refs inside their arguments are reached by the
generic walker.

| Method | State | Notes |
| --- | --- | --- |
| `.alias` / `.name` | ✅ | Used as output name in `select` / `agg` |
| `.cast` | ✅ | Type follows `from_spark_name`; nullability carried |
| `.over` | ✅ | Passes type through |
| `.isNull`, `.isNotNull` | ⚠️ | Unmodeled |
| `.isin` | ⚠️ | Unmodeled |
| `.between` | ⚠️ | Unmodeled |
| `.like`, `.rlike`, `.ilike`, `.contains`, `.startswith`, `.endswith` | ⚠️ | Unmodeled |
| `.asc`, `.desc` | ⚠️ | Unmodeled |
| `.eqNullSafe` | ⚠️ | Unmodeled |
| `.getField`, `.getItem`, `.dropFields`, `.withField` | ⚠️ | Unmodeled |
| `.substr` | ⚠️ | Unmodeled |

## Window / Spec

| Form | State | Notes |
| --- | --- | --- |
| `Window.partitionBy("k")` | ✅ | Keys checked when the `.over(...)` is applied to a known schema |
| `Window.orderBy("k")` | ✅ | Same |
| `Window.partitionBy(...).orderBy(...)` chain | ✅ | Builder chain walked end-to-end |
| `Window.rowsBetween` / `rangeBetween` | ⚠️ | Unmodeled (no column refs to check) |
| `Window.unboundedPreceding` / `unboundedFollowing` / `currentRow` | ⚠️ | Unmodeled constants |

## Headline gaps (prioritized)

A short list of the unmodeled methods most worth filling in next, ordered
by how commonly real PySpark code uses them.

1. **`intersect` / `subtract` / `intersectAll` / `exceptAll`** — set-shape
   companions to `union`; same schema-mismatch check as `unionByName`.
2. **`Column` method chains — `.isNull`, `.isin`, `.between`,
   `.startswith`, `.contains`, `.like`** — extremely common in `filter` /
   `withColumn`; today recognized only via the generic-walker fallthrough,
   so a typo on a `Column` method goes uncaught.
3. **`when` / `otherwise`** — pivotal in `withColumn` pipelines. Column
   refs in the predicate/branches are reached today, but the result
   type isn't inferred and a bad ref in the `otherwise` branch can be
   masked by the mixed-arg shape.
4. **`F.struct` / `F.named_struct`** — needed to type a struct-valued
   `withColumn` and to chain `getField` / dotted nav onto it.
5. **`melt` / `unpivot`** — Spark 3.4+; small but unmodeled, and the
   output schema is fully determinable from the args.
6. **`broadcast`** — semantically a pass-through; today the chain dies.
7. **`F.to_date` / `to_timestamp` / `date_format`** — already typed, but
   not in `COLUMN_REF_FUNCTIONS`, so a typo on the column arg slips
   through; widen the allowlist carefully (first arg only).
8. **`createOrReplaceTempView`** — terminal in practice but pairs with
   `spark.sql("…")`; recording the registered view name would let
   pykrete check `spark.sql("SELECT … FROM view_name")` queries.
9. **Terminal recognizers (`count`, `collect`, `first`, `show`,
   `printSchema`, `explain`)** — model them as terminal so a chain
   after them is flagged (probably a bug) rather than silently dying.

## Recently closed

- **`spark.read.<format>(path)` / `spark.table(name)`** —
  `DataFrameReader` chains (`spark.read.parquet(...)`,
  `spark.read.format(...).load(...)`, `spark.read.schema(...).<format>(...)`)
  and bare `spark.table(...)` are recognized as opaque IO sources. The
  result is still Unknown — the schema is genuinely runtime data — but
  the user re-anchors with `.cast(DataFrame[Schema])` or
  `name: DataFrame[Schema] = spark.read.parquet(...)` and downstream
  column checks resume. Before this change, every codebase outside the
  `dal.read(SOURCE)` pattern lost its chain at line one.
