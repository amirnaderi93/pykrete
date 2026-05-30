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

- **Reshaping** — `transpose` is unmodeled; `pivot` is column-checked but
  its output schema is data-dependent (deliberate). `melt` / `unpivot`
  now model their output schemas.
- **Streaming** — `readStream` / `writeStream` / `isStreaming` are
  entirely unmodeled.
- **Pandas / Arrow interop** — `toPandas`, `toArrow`, `mapInPandas`,
  `applyInPandas` are opaque by design.
- **Introspection / describe** — `describe`, `summary`, and most
  `stat.*` helpers are unmodeled.

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
| `unionAll` | ✅ | Deprecated alias for `union`; same check |
| `intersect` | ✅ | Same check as union; preserves receiver schema |
| `intersectAll` | ✅ | Same as `intersect` (preserves duplicates) |
| `subtract` | ✅ | Same check as union; preserves receiver schema |
| `exceptAll` | ✅ | Same as `subtract` (preserves duplicates) |

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
| `unpivot` | ✅ | Output schema = ids + variable + value (common type of values columns; nullable if any value is). |
| `melt` | ✅ | Output schema = ids + variable + value (common type of values columns; nullable if any value is). |
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
| `printSchema` | ✅ | Recognized terminal; returns None, chain ends |
| `schema` | ⚠️ | Property, unmodeled |
| `columns` | ⚠️ | Property, unmodeled |
| `dtypes` | ⚠️ | Property, unmodeled |
| `isLocal` | ⚠️ | Unmodeled |
| `isEmpty` | ⚠️ | Unmodeled |
| `explain` | ✅ | Recognized terminal; returns None, chain ends |

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
| `createOrReplaceTempView` | ✅ | Registers the receiver's schema against the view name; within-file resolution |
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
| `spark.sql("…")` | ✅ | Projection columns inferred best-effort; looks up registered tempViews by name and checks column refs (SELECT / WHERE / GROUP BY / ORDER BY / HAVING) against the view's schema |
| `stat.crosstab` / `freqItems` / `approxQuantile` / `corr` / `cov` | ⚠️ | Unmodeled |
| `summary` | ⚠️ | Unmodeled |
| `describe` | ⚠️ | Unmodeled |
| `count` | ✅ | Recognized terminal (returns long) |
| `collect` / `first` / `head` / `take` / `tail` | ✅ | Recognized terminals |
| `show` | ✅ | Recognized terminal (returns None) |
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
| `to_date`, `to_timestamp` | ✅ | ✅ | First-arg column ref checked; format arg ignored |
| `current_date`, `current_timestamp` | ⚠️ | ✅ | No-arg |
| `date_format`, `trunc`, `next_day` | ✅ | ✅ | First-arg column ref checked |
| `date_trunc` | ✅ | ✅ | `(format, col)` — second-arg column ref checked |
| `from_utc_timestamp`, `to_utc_timestamp`, `from_unixtime`, `unix_timestamp` | ✅ | ✅ | First-arg column ref checked; tz / format ignored |
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
| `struct`, `named_struct` | ⚠️ | ✅ | Result struct fields modeled from arg names + types |
| `create_map`, `map_from_arrays`, `map_from_entries`, `map_concat`, `str_to_map`, `transform_keys`, `transform_values`, `map_filter` | ⚠️ | ✅ | → map (K/V untyped) |
| `map_keys`, `map_values` | ✅ | ✅ | → `array<K>` / `array<V>` |
| `map_entries` | ✅ | ✅ | → array |
| `transform`, `filter`, `aggregate`, `exists`, `forall` (higher-order) | ✅ | ✅ | First-arg column ref checked; return type modeled per function — `transform` → `array<lambda body>`, `filter` → input array type, `aggregate` → lambda body (or `finish`), `exists` / `forall` → bool. Lambda body inferred best-effort (literals, `col("y")` against surrounding schema) |
| `zip_with` (higher-order) | ⚠️ | ⚠️ | Unmodeled (two-column form; column refs reached only when written as `col("…")`) |

### Conditional

| Function | Ref | Typed | Notes |
| --- | --- | --- | --- |
| `when`, `otherwise` | ⚠️ | ✅ | Result type inferred from branches (common-type widening; nullable when no `.otherwise()`); column refs inside arguments are reached via the generic walker |
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
| `broadcast` | ⚙️ | ⚙️ | Pass-through: `F.broadcast(df)` carries `df`'s schema |
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
| `.isNull`, `.isNotNull` | ✅ | → bool; chain preserved |
| `.isin` | ✅ | → bool; value-arg column refs checked |
| `.between` | ✅ | → bool; chain preserved |
| `.like`, `.rlike`, `.ilike`, `.contains`, `.startswith`, `.endswith` | ✅ | → bool; chain preserved |
| `.asc`, `.desc` | ⚠️ | Unmodeled |
| `.eqNullSafe` | ⚠️ | Unmodeled |
| `.getField` | ✅ | → nested struct field's type; D0030 on field-name typo |
| `.getItem` | ✅ | → array element / map value type |
| `.withField` | ✅ | → receiver struct with field added or replaced |
| `.dropFields` | ✅ | → receiver struct with fields removed |
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

1. **Chain-after-terminal flag** — terminals are now recognized, so
   the next polish step is to flag a call chained after one (almost
   always a bug) instead of silently returning None.

## Recently closed

- **`melt` / `unpivot` output-schema modeling.** Spark 3.4+'s
  wide-to-long reshape now infers its result schema: the `ids` columns
  are preserved with their declared types and nullability, the variable
  column is `string`, and the value column is the common type across the
  `values` columns (with numeric widening — `int` < `long` < `double` —
  and `Nullable(T)` when any value column is nullable). `values=None` /
  omitted unpivots every non-`ids` column. Typos in `ids` or `values`
  fire `D0030`; heterogeneous value types degrade to Unknown rather than
  fabricate a common type. `unpivot` is treated as an alias of `melt`.
- **Date/time first-arg column checking + array higher-order function
  recognition.** `F.to_date`, `F.to_timestamp`, `F.date_format`,
  `F.trunc`, `F.next_day`, `F.from_utc_timestamp`, `F.to_utc_timestamp`,
  `F.from_unixtime`, and `F.unix_timestamp` now check their FIRST
  positional arg as a column reference (a typo fires `D0030`) while
  treating the second arg as a format / timezone string;
  `F.date_trunc(format, col)` does the same with the column in the
  SECOND position. `date_format` and `from_unixtime` are also now typed
  in the result catalog (→ string). The array higher-order functions
  `F.transform`, `F.filter`, `F.aggregate`, `F.exists`, `F.forall` are
  recognized at the surface — the first-arg column is checked and the
  return type is modeled per function (`transform` → `array<lambda
  body>`, `filter` → input array, `aggregate` → lambda body or `finish`,
  `exists` / `forall` → bool). Lambda bodies are inferred best-effort
  (literals and `col("y")` against the surrounding schema) and fall
  back to Unknown rather than fabricate a type.
- **`when` / `otherwise` result-type inference and `F.struct` /
  `F.named_struct` schema construction.** A
  `F.when(p, v).otherwise(e)` chain now infers its result type as the
  common type of the branches (atomic equality first, then numeric
  widening — `int` < `long` < `double`; heterogeneous branches that
  can't reconcile stay Unknown, never a fabricated type). A chain
  without `.otherwise(...)` is treated as `Nullable(T)` — unmatched
  rows produce null at runtime. Chained predicates
  (`.when(p1, v1).when(p2, v2).otherwise(e)`) walk back to the root
  `F.when` and reconcile across every value branch. `F.struct(c1, c2)`
  now produces a `Struct({a: int, b: string})` whose field names come
  from `.alias("x")` first, then the `col` reference's name, and whose
  field types are each arg's inferred type; `F.named_struct("k1", v1,
  "k2", v2)` uses the string-literal name slots as field names, falling
  back to Unknown when a name slot isn't a literal. Composes with the
  existing `.getField`, so `F.struct(col("a"), col("b")).getField("a")`
  resolves to `a`'s type.
- **`createOrReplaceTempView` + `spark.sql("SELECT … FROM view")`.**
  A chain ending in `.createOrReplaceTempView("name")` registers the
  receiver's schema (Typed or Derived) against the view name in a
  per-file registry. A subsequent `spark.sql("SELECT … FROM name")` in
  the same file looks the view up and checks every column identifier in
  the query — projection, `WHERE`, `GROUP BY`, `ORDER BY`, `HAVING` —
  against the view's schema, firing `D0030` on a typo. The result
  schema is the projection columns when readable, or the view's full
  schema for `SELECT *`. **Scope:** single-table SELECT only (no joins,
  no subqueries), within-file only (cross-file views fall back to the
  pre-tempView best-effort behavior).
- **`Column` method chains.** `.isNull`, `.isNotNull`, `.isin`,
  `.between`, `.like`, `.rlike`, `.ilike`, `.contains`, `.startswith`,
  `.endswith` are now recognized as boolean-returning Column predicates;
  `.getField` resolves the nested struct field's type (and fires D0030
  on a field-name typo); `.getItem` returns the element type of an
  array or the value type of a map; `.withField` / `.dropFields` track
  the receiver's struct shape forward with the field added, replaced,
  or removed. The previous behavior leaked the receiver's whole type
  forward without modeling these methods at all.
- **Set ops, `F.broadcast`, and terminal recognizers** —
  `intersect` / `intersectAll` / `subtract` / `exceptAll` are recognized
  set operations sharing the `union` schema-mismatch check (D0040);
  `unionAll` is wired as a deprecated alias for `union`. `F.broadcast(df)`
  is treated as a pass-through, so chains like
  `df1.join(F.broadcast(df2), "k")` and `F.broadcast(df).select(...)`
  keep their schemas. The nine terminal methods (`count`, `collect`,
  `show`, `printSchema`, `explain`, `first`, `take`, `head`, `tail`) are
  now recognized centrally — the chain dies cleanly, and a future
  "chain-after-terminal" diagnostic has a single seam to attach to.
- **`spark.read.<format>(path)` / `spark.table(name)`** —
  `DataFrameReader` chains (`spark.read.parquet(...)`,
  `spark.read.format(...).load(...)`, `spark.read.schema(...).<format>(...)`)
  and bare `spark.table(...)` are recognized as opaque IO sources. The
  result is still Unknown — the schema is genuinely runtime data — but
  the user re-anchors with `.cast(DataFrame[Schema])` or
  `name: DataFrame[Schema] = spark.read.parquet(...)` and downstream
  column checks resume. Before this change, every codebase outside the
  `dal.read(SOURCE)` pattern lost its chain at line one.

## Post-v1.0 follow-ups

### `match` / `case` bodies (v1.1)

`Stmt::Match` arms are currently NOT walked. `walk_stmt`'s `Match` arm is
an explicit no-op with the rationale: `case` patterns can bind new local
names (`case MyClass(field=x):` binds `x`) that would otherwise look
like undefined symbols to the D0051 / column-ref machinery, since the
walker has no pattern-binding extractor. Adding pattern-binding support
is the prerequisite, after which the walker can descend into subject,
guards, and bodies through here.

Estimated cost: small — a focused pattern walker covering `MatchValue`,
`MatchAs`, `MatchOr`, `MatchClass(positional + keyword)`, `MatchMapping`,
`MatchSequence`, `MatchSingleton`, `MatchStar`. Each maps to a set of
`(name, range)` bindings the walker marks local in the case-body scope
before descending. Sequence patterns can be ignored when starred — the
binding behavior is the same as the unstarred form for our purposes.

### LSP enrichment inside nested function defs (v1.1)

When the body walker descends into a `Stmt::FunctionDef` (or `ClassDef`)
nested inside an outer function body, it processes the nested helper
with its own `BodyContext` and discards the inner-scope `column_refs`,
`local_bindings`, and `call_results` rather than draining them back to
the outer collector. The diagnostic story is correct either way —
column-ref typos, return-type mismatches, and the rest of the D-codes
fire from the inner pass — but the LSP enrichment surfaces (hover,
completion, go-to-definition) won't light up inside the nested helper
because the outer file-level index never sees those inner refs.

Promoting this to feature parity means deciding the merge story for
overlapping name spans (the same identifier text can appear in both the
outer and inner scope with different schemas) and threading a shared
collector through `walk_function_def`. Cost: small-to-medium; the
ordering question (which scope wins on a hover at the boundary?) is the
real work, not the plumbing. Punted to v1.1 because v1.0 promises
correctness, not full IDE polish in every nesting depth.

### v0.1.28 type-vocabulary polish (v1.1)

Surfaced by the v0.1.28 multi-lens review; the round-4 fixes closed
the trust-critical contradictions but five smaller items were
deferred as v1.1 to keep this PR focused.

1. **Tailored `D0011` message on out-of-range decimal cast.**
   `.cast("decimal(40, 2)")` currently fires the generic "Cast target
   '…' is not a recognized Spark type" message. A tailored variant
   ("precision 40 exceeds Spark's cap of 38" / "scale 25 exceeds
   precision 18") would be more actionable. Cost: small — branch the
   diagnostic message in `operations.rs` when the failure shape is
   "valid keyword, invalid args".

2. **Nullable parity on `min` / `max`.** The `mean(decimal)` /
   `sum(decimal)` paths agreed on nullability after v0.1.28, but the
   `min` / `max` paths weren't audited together — verify both `groupBy`
   shortcut and `agg(F.min(...))` form propagate `Nullable(T)`
   identically on nullable inputs.

3. **`numeric` / `dec` in completion + did-you-mean.**
   `COLUMN_TYPE_NAMES` and `COLUMN_TYPE_NAMES_LIST` list `decimal` but
   not its aliases, so LSP completion inside a `name: "<cursor>"`
   annotation never suggests them, and "did you mean" doesn't surface
   them on a near-miss like `dec`. Add them as canonicalised
   completions (lowercase preferred) and to the did-you-mean candidate
   pool.

4. **`schemas.md` documenting the aliases.** The v0.1.28 docs update
   mentions the alias support in the changelog but `schemas.md`'s
   atomic-types table itself doesn't list `numeric` / `dec` as
   alternative spellings of `decimal`. Folding that in keeps the
   docs honest about what's accepted at the schema-annotation
   surface.

5. **Pre-existing `synthetic_name_pool` flake.** A flaky property
   test surfaced during v0.1.26 work; the round-4 review picked it
   up but it's an older issue and out of scope for v0.1.28. Track as
   a standalone hygiene item, separate from the type-vocab work.

### v0.1.29 Spark-coverage minors (v1.1)

Surfaced by the v0.1.29 multi-lens review (PR #59). The blocker and
three important items shipped in v0.1.29; these four minors were
deferred to keep the patch focused.

1. **Explicit-values `pivot(col, ["a", "b"])` overload.** When the
   caller supplies the pivot values inline, the output columns become
   statically knowable — pykrete could materialize a concrete Derived
   schema (`{keys..., "a", "b"}`) instead of bailing Unknown. The
   no-values form (`pivot("col")`) stays Unknown by necessity. Cost:
   small — extend the pivot handler in `operations.rs` to read the
   second positional arg when it's a list literal of string literals.

2. **`posexplode` in agg context lacks a test.** `handle_agg` calls
   `posexplode_fields` for each arg, which produces both `pos: int`
   and `col: <element>` fields, but no test covers
   `.groupBy("k").agg(F.posexplode("tags"))`. The code path is
   exercised only via `.select(F.posexplode(...))`. Cost: trivial —
   add one positive test (both fields resolve) and one negative
   (typo on the input array fires D0030).

3. **`.alias(...)` on `posexplode` lacks a test.** The fall-through
   behavior is documented in the helper's doc-comment ("alias on a
   posexplode is rare; default walker handles it") but is unverified
   by any test. Silent drift on a future Spark rename would go
   unnoticed. Cost: trivial — one test asserting
   `.select(F.posexplode("tags").alias("p"))` doesn't crash or fire
   spurious D-codes.

4. **`F.get(arr, idx)` shares `element_at`'s Map-unwrap arm.** Per
   Spark docs, `get` is array-only — it doesn't accept Map inputs.
   The shared arm in `function_result_type` unwraps `Map(_, V) → V`
   for `get`, which is dead code in practice (the input would have
   been rejected upstream) but cosmetically wrong. Cost: trivial —
   split the arm or guard the Map branch behind a method-name check.

### v0.1.29 PR #59 round-2 review minors (v1.1)

Surfaced by the v0.1.29 round-2 multi-lens re-review (PR #59). All
non-blocking; the round-2 PR shipped with the blocker + 3 importants
closed.

5. **`operations.md` labels for `sampleBy` / `observe`.** The reference
   table needs explicit rows noting that `sampleBy` now col-ref-checks
   its first positional arg and `observe` walks expressions. Without
   the rows, the docs imply they are pure pass-throughs. Cost: trivial.

6. **`completion.rs` `SchemaView::Grouped` rest-pattern.** The
   completion code path still destructures `Grouped` with `..` and
   ignores `after_pivot`. Since completion doesn't currently key
   suggestions on grouping state, this is silent rather than wrong —
   but it's a latent footgun if completion grows pivot-aware
   suggestions. Cost: small — bind the field and assert handling.

7. **`describe([list])` form unchecked.** `check_describe_args` covers
   positional `*cols` and `col(...)` Column forms; the
   `df.describe(["amount", "region"])` list-of-strings form falls
   through to opaque without col-ref checking. Spark accepts both
   forms. Cost: small — branch on `Expr::List` and walk its strings.

### v0.1.30 LSP perf review minors (v1.0.0 prerequisites)

Surfaced by the v0.1.30 multi-lens review (PR #61). Round-1 closed the
blocker (two-pass cold walk + tracked-union invariant) and the two
importants (deterministic `ProjectKey`, `workspace/didChangeWatchedFiles`
dynamic registration); these are tracking items for the v1.0.0 ship.

8. **Miri job in CI for `Schema::fields` unsafe block.** The
   widen-to-`'static` / narrow-back-to-`'ast` pattern in
   `crates/pykrete/src/schema.rs` is sound by three structural
   invariants (no `Clone` derive, `PhantomData<&'ast ()>` invariance,
   private cache) but the borrow checker can't actually prove the
   transmute on its own. Adding a `cargo +nightly miri test` job
   under the MIR interpreter would catch any future use-after-free
   pattern that violates the invariants — particularly if someone
   adds `Clone` or splits the cache out of the struct. Cost: small —
   one nightly job in the existing GitHub Actions workflow, gated to
   `pykrete` crate only (the LSP and wasm crates pull in `tokio` /
   `wasm-bindgen` which Miri can't run).

9. **VS Code `contributes.commands` entry for `pykrete/refreshSnapshot`.**
   The LSP server registers the `pykrete/refreshSnapshot` custom command
   and wires it through to `SnapshotCache::invalidate`, but
   `editors/vscode/package.json contributes.commands` doesn't expose it
   in the Command Palette. The file-watcher (v0.1.30 I3) eliminates
   most of the use cases — staleness collapses to the watcher's debounce
   window for clients that support it — but a manual command is still
   useful for clients without dynamic registration and for diagnosing
   stuck caches in the field. Cost: trivial — one entry in
   `contributes.commands` + a one-line `extension.ts` handler that
   sends the LSP request.

10. **Parsed-module memoization inside `ProjectContext::build`.** Today
    every snapshot rebuild re-parses every `.pyk` body the cache hands
    out, even when neither the path nor the body bytes changed. A
    keyed-on-`(path, content_hash)` parse cache on top of the snapshot
    cache would eliminate the per-keystroke parse cost on closed files
    — the bigger win for project-mode scaling than the snapshot cache
    alone. Cost: medium — the parsed AST borrows from the body string,
    so the cache has to own both, and `ProjectContext`'s lifetime story
    has to widen to accept owned arenas. Mirrored from the PR #61 body.

11. **On-demand read cost on >20 MB codebases.** The round-2 two-pass
    cold walk leaves `body = None` for paths the 20 MB cap evicts;
    snapshot assembly then does a synchronous `fs::read_to_string` for
    each `None` at request time. On codebases whose total `.pyk` body
    bytes exceed the cap, every hover / definition / completion that
    touches an uncached file pays disk I/O on the LSP loop thread.
    Mitigation candidates: (a) make the cap configurable via
    `pykrete.json`, (b) move the on-demand read to a background-prefetch
    pool with a future the loop awaits, (c) keep a small LRU sub-cache
    of recently on-demand-read bodies so the second hover on the same
    file is hot. Cost: small for (a), medium for (b)/(c) — needs an
    async pool wired into the snapshot composition path.

12. **`client/registerCapability` timing vs LSP spec.** The
    `workspace/didChangeWatchedFiles` dynamic registration is sent
    from `initialize_finish` today. The LSP spec is explicit that
    capability registration must follow the client's `initialized`
    notification (distinct from the server's `initialize` response).
    Clients that strictly enforce ordering may drop or error on the
    early registration. Verify the timing against the spec; if
    incorrect, move the registration call to the `initialized`
    notification handler. Cost: trivial — relocating one call site,
    plus a regression test that asserts ordering.

13. **File-watcher glob excludes `pykrete.json`.** The watcher glob
    registered in v0.1.30 I3 is `**/*.pyk`, but `pykrete.json` mtime
    is part of the `ProjectKey` cache key — an external edit to
    `pykrete.json` (e.g. `git checkout`, manual edit, formatter) won't
    fire `workspace/didChangeWatchedFiles`, so the cache holds stale
    config until the 30 s cold-walk window elapses or
    `pykrete/refreshSnapshot` is invoked. Fix: extend the watcher to
    `**/{*.pyk,pykrete.json}` (or register a second pattern); or
    document the gap and require LSP restart / refresh command on
    `pykrete.json` edits. Cost: trivial — one extra glob entry in the
    `DidChangeWatchedFilesRegistrationOptions`.
