# Pandas support — `PandasFrame[X]` (v1.3 design tracker)

**Status**: feasibility-prototype phase; v1.3 candidate spec. Open
questions below are intentionally open — the v1.3 spec PR settles
them. This tracker collects the framing, the evidence from the
spike at `spikes/v1.3-pandas/`, the dtype-mapping research, and
the open questions.

**Origin**: 2026-06-03, after v1.2.0 ship. Authored as a feasibility
spike per the post-mortem rule that synthesizer/generator features
need a feasibility prototype before the spec PR locks syntax.
PROBE-TYPE-IS (v1.1) hit the inverse trap — spec was settled
assuming the synthesizer would work, then surprised by the
scope-binding gap. v1.3 starts from a verified technique.

**Sibling**: `spark-coverage.md` (the Spark coverage spec is the
reference shape; pandas mirrors it where dtypes align and
diverges where they don't).

## The pitch

Typed pandas DataFrames in pykrete. A schema author declares
column names and dtypes once:

```python
class Order(Schema):
    id: long
    status: string
    amount: decimal(18, 2)
    created_at: timestamp
```

…then annotates a function:

```python
def shipped_amounts(orders: PandasFrame[Order]) -> PandasFrame[Order]:
    return orders[orders["status"] == "shipped"]
```

…and pykrete catches the same class of bugs it catches for Spark
today: typos in column references (`orders["statuss"]`), references
to columns dropped by an earlier step, type mismatches, off-enum
literal comparisons.

The value prop is identical to Spark, aimed at the segment of
pykrete's audience that lives in pandas (notebooks, ML pipelines,
ETL using pandas instead of Spark, polars-curious teams that still
ship pandas in prod).

## Framing principle (load-bearing)

> Pykrete validates things known at edit time. Pandas dtypes
> declared on a schema and pandas column references in the source
> qualify. Runtime row values do not.

Same bright line as enum constraints (`literal-value-vocabulary.md`)
and PROBE-TYPE-IS (`schema-tracking-probes.md`). Concretely:

| Pandas construct | In scope for v1.3? | Why |
|---|---|---|
| `df["status"]` column access against declared schema | Yes | column ref is a literal, schema is known |
| `df.status` column access against declared schema | Yes | same |
| `df.loc[:, "status"]` column access | Yes | same — literal slice key |
| `df.drop(columns=["status"])` schema transform | Yes | mirrors Spark `.drop("status")` |
| `df.rename(columns={"old": "new"})` | Yes | literal rename map |
| `df.merge(other, on="key")` | Yes | mirrors Spark `.join` |
| Dtype mismatch on `withColumn`-style assignment | Yes | same as Spark |
| `df.apply(lambda row: ...)` row-runtime callback | No | row value not knowable at edit time |
| `df.query("status == 'shipped'")` runtime string | Partial — see Open Q | the literal in the query string is known, but parsing it adds a SQL-like surface |
| MultiIndex columns (`df[("a", "b")]`) | No (v1.3) | hierarchical-schema work; see Anti-scope |
| `.iloc[3]` row-positional access | No | row position not a schema concept |
| `df["new"] = some_python_expr` runtime row mutation | Partial | the dtype of `some_python_expr` is rarely knowable; mutation is in scope only when RHS is a typed-column expression |

The shape of the line is the same as Spark's: pykrete sees the
schema, sees the column references, type-checks where types are
known, leaves everything else alone.

## Syntax (provisional — v1.3 spec PR locks)

```python
# v1.3 introduces SparkFrame as the explicit name and PandasFrame
# as its parallel. DataFrame is grandfathered for one major as a
# synonym for SparkFrame (see Open Q on alias lifetime).
def f(df: SparkFrame[Order]) -> SparkFrame[Order]: ...
def g(df: PandasFrame[Order]) -> PandasFrame[Order]: ...
def h(df: DataFrame[Order]) -> DataFrame[Order]: ...   # alias of SparkFrame, deprecated
```

The subscript shape (`Frame[Schema]`) is identical across both
forms — same parser path, same `Pick[…]` / `Omit[…]` /
`Merge[…]` derived-schema operators, same inline dict shape.
The difference is the *parser-level frame tag* on the resulting
`TypedSlot`, which drives check-site dispatch.

### Cross-dialect interaction (provisional)

A function whose param is `SparkFrame[X]` and whose return is
`PandasFrame[X]` (or vice versa) is the cross-dialect handoff —
typically a `.toPandas()` boundary. v1.3 should recognize this
shape and check the schema match, but the dialect transition
itself is checker-silent (it's a deliberate user action).

`SparkFrame[X] | PandasFrame[X]` union annotations are explicitly
out of scope for v1.3 — see Open Q on union-typed frame
annotations.

## Pandas-dtype → pykrete `ColumnType` mapping (research output)

Pandas has more dtype variety than Spark: a NumPy-derived legacy
set (`int8`, `int16`, `int32`, `int64`, `uint8`–`uint64`,
`float32`, `float64`, `object`, `bool_`, `datetime64[ns]`,
`timedelta64[ns]`), plus the pandas-native ExtensionArray dtypes
(`Int8`–`Int64`, `UInt8`–`UInt64`, `Float32`/`Float64`,
`string`/`StringDtype`, `boolean`/`BooleanDtype`,
`category`/`CategoricalDtype`, `period`/`PeriodDtype`,
`interval`/`IntervalDtype`), plus the Arrow-backed dtypes
(`int64[pyarrow]`, `string[pyarrow]`, etc., from pandas 2.0+).

The table below is the v1.3 starting point. The "Mapping" column
lists the proposed `ColumnType` variant; "Notes" flags semantic
gaps the spec PR needs to settle.

### Signed integers

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `int8` / `Int8` | `ColumnType::Byte` | clean 1:1 |
| `int16` / `Int16` | `ColumnType::Short` | clean 1:1 |
| `int32` / `Int32` | `ColumnType::Int` | clean 1:1 |
| `int64` / `Int64` | `ColumnType::Long` | clean 1:1 — this is the pandas default for `int` columns |

### Unsigned integers — GAP

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `uint8` / `UInt8` | (none) | Spark has no unsigned types. v1.3 options: widen to next signed (`uint8`→`Short`), reject as unsupported, or add `ColumnType::U8/U16/U32/U64`. The TS-north-star reading: TS numbers don't carry signedness, so we should widen — but pandas users will see truncation surprises. **Open question.** |
| `uint16` / `UInt16` | (none) | same |
| `uint32` / `UInt32` | (none) | same |
| `uint64` / `UInt64` | (none) | same — and there's no signed type wide enough to hold all `uint64` values |

### Floats

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `float32` / `Float32` | (gap — Spark has FloatType, pykrete doesn't expose it) | v1.0 pykrete collapses `float` to `ColumnType::Double` (= Spark `DoubleType`). Pandas users expect `float32` precision. **Open question:** add `ColumnType::Float` or widen and document the lossy mapping? |
| `float64` / `Float64` | `ColumnType::Double` | clean 1:1 — pandas default for floats |

### Strings

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `object` (string-typed column) | `ColumnType::String` | the legacy default; pandas stores strings as Python objects. Lossy: `object` can hold non-string Python objects. Pykrete will trust the declared schema. |
| `string` / `StringDtype` | `ColumnType::String` | the modern explicit string type (pandas 1.0+). Same mapping. |
| `string[pyarrow]` | `ColumnType::String` | Arrow-backed string. Same mapping; the storage difference doesn't affect pykrete's edit-time checks. |
| `category` / `CategoricalDtype` (with explicit categories) | `ColumnType::Enum(vocab)` | clean fit: pandas `CategoricalDtype(["a", "b", "c"])` is exactly pykrete's `enum["a", "b", "c"]`. The v1.3 spec PR should pin whether the schema author writes `enum[…]` or `category[…]` — argument for both: `enum` matches the v1.1 vocabulary; `category` matches pandas idiom. **Open question.** |
| `category` (no categories declared) | `ColumnType::String` | open-vocabulary categoricals degrade to plain string. |

### Booleans

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `bool` / `bool_` | `ColumnType::Bool` | clean 1:1 — but pandas `bool` cannot hold NaN |
| `boolean` / `BooleanDtype` | `ColumnType::Nullable(Box::new(Bool))` | the nullable boolean (introduced for `<NA>` support). Pykrete already models nullability via `ColumnType::Nullable` — clean fit. |

### Dates and times

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `datetime64[ns]` | `ColumnType::Timestamp` | clean 1:1 (pandas's microsecond/nanosecond precision is finer than Spark, but at the schema level the type is the same) |
| `datetime64[ns, tz]` | `ColumnType::Timestamp` | timezone-aware. Spark's `TimestampType` is tz-naive; pykrete doesn't carry tz today. v1.3 can match on type but won't enforce tz. **Open question:** add tz to `ColumnType::Timestamp` or defer? |
| `Timestamp` (scalar, not dtype) | n/a | this is a value, not a column dtype |
| `timedelta64[ns]` | (none) | GAP — Spark has `DayTimeIntervalType`/`YearMonthIntervalType` but pykrete v1.2 doesn't expose either. **Open question:** add `ColumnType::Interval` or defer? |
| `period` / `PeriodDtype` | (none) | GAP — no Spark equivalent. v1.3 candidate: defer / mark unsupported. |
| `date` (object dtype holding `datetime.date`) | `ColumnType::Date` | pandas doesn't have a first-class date dtype; the convention is `datetime64` truncated. Schema author declares `date` → pykrete trusts it. |

### Other / structured

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `bytes` / `object` holding bytes | `ColumnType::Binary` | parallel to the string case |
| `interval` / `IntervalDtype` | (none) | GAP — no Spark equivalent. Defer. |
| Sparse dtypes (`Sparse[int64]`, etc.) | (storage detail, transparent) | The logical type is the underlying; sparseness is a storage optimization. Pykrete sees through it. |
| NumPy structured arrays (heterogeneous record dtype) | `ColumnType::Struct(...)` | clean fit in principle. **Open question:** rare in practice — defer to a follow-up? |
| MultiIndex columns | (none) | hierarchical column schema. Explicit anti-scope for v1.3. |

### Dtype mapping summary

- **Clean 1:1** (no spec work needed): `int8`/`Int8`, `int16`/`Int16`, `int32`/`Int32`, `int64`/`Int64`, `float64`/`Float64`, `bool`, `object`-string, `string`/`StringDtype`, `string[pyarrow]`, `category` (with categories), `datetime64[ns]`, `bytes`/binary.
- **Clean fit via existing variant** (no new variants, just mapping work): `boolean`/`BooleanDtype` → `Nullable(Bool)`, `category` → `Enum(vocab)`, `Sparse[T]` → `T`.
- **GAPs requiring spec decisions**:
  1. Unsigned ints (`uint8`–`uint64`) — widen, reject, or add variants?
  2. `float32` — add `ColumnType::Float` or widen lossy to `Double`?
  3. tz-aware `datetime64[ns, tz]` — add tz to `Timestamp` or defer?
  4. `timedelta64[ns]` — add `Interval` variant or defer?
  5. `period`, `interval` — explicit defer expected.
  6. `category` syntax — `enum[…]` (matches v1.1) vs `category[…]` (matches pandas idiom)?

Five of six gaps are tractable in v1.3 with a single new variant or a documented widening. The sixth (category syntax) is a naming call.

## Dispatch shape

Per the original roadmap memory: dispatch is per-annotation,
parser-level. The flow:

1. `dataframe.rs::recognize` is extended from matching the literal name `DataFrame` to matching the set `{SparkFrame, PandasFrame, DataFrame}`. The returned `DataFrameAnnotation` carries a new `Dialect` tag (`Spark | Pandas`); `DataFrame` resolves to `Spark` (the alias).
2. `TypedSlot` carries the dialect through to the check sites.
3. Each check site (column-ref walker, schema transform walker, join walker, etc.) reads the dialect off the bound slot and dispatches:
   - `df.x` and `df["x"]` column access — both work for pandas; today both work for Spark too. **Likely fully shared** — no dispatch needed at the column-ref walker.
   - `df.select(col("x"))` — Spark only. Pandas equivalent is `df[["x"]]`. Dispatch needed.
   - `df.filter(col("x") == "y")` — Spark only. Pandas equivalent is `df[df["x"] == "y"]`. Dispatch needed.
   - `df.withColumn("new", expr)` — Spark only. Pandas equivalent is `df["new"] = expr` or `df.assign(new=expr)`. Dispatch needed.
   - `df.drop("x")` — both, but pandas `df.drop("x")` drops a row by index; the column-drop form is `df.drop(columns=["x"])`. Dispatch needed.
   - `df.rename(columns={...})` — both share this.
   - `df.merge(...)` / `df.join(...)` — pandas only. Spark uses `df.join(...)` (different signature). Dispatch needed.
4. Where dispatch is needed, the existing operation modules under `crates/pykrete/src/operations/` either grow a dialect parameter and branch internally, or split into `operations/spark/` and `operations/pandas/` siblings sharing a common column-ref core. **Open question:** which factoring is cleaner.

The encouraging finding from the spike: column-ref resolution
(`col_refs.rs`) already handles `Subscript` shape uniformly,
which means `df["status"]` against a `PandasFrame[Order]` slot
should fall out of an annotation-recognition extension alone —
no new walker, no new D-code. That's the cheapest possible
proof-of-life.

## Anti-scope (v1.3 will NOT do)

These are explicit defers. v1.3 ships a tight slice; adjacent
features come in v1.4+.

- `.apply(lambda row: …)` row-runtime dispatch — the lambda's input is a `Series` whose shape isn't statically inferable from the declared schema in the general case. Defer.
- `.apply(lambda col: …, axis=0)` column-runtime dispatch — same.
- MultiIndex columns — hierarchical column schemas need a new schema vocabulary; out of scope for v1.3.
- MultiIndex on rows — the row index is irrelevant to column-name/dtype checking.
- pandas `.eval("status == 'shipped'")` and `.query("…")` — these embed a mini-DSL. The literal in the string is a known value but parsing it requires a pandas-eval surface analogous to Spark's `F.expr`. Defer to v1.4.
- Copy-vs-view semantics (`df["a"] = …` after slicing emits a `SettingWithCopyWarning`) — runtime behavior, not edit-time checkable.
- NumPy structured arrays as DataFrame sources — rare; defer.
- Arrow-backed pandas dtype distinctions beyond what's already in the mapping table — defer.
- Cross-dialect union annotations (`SparkFrame[X] | PandasFrame[X]`) — defer.

## Open questions (settle in v1.3 spec PR)

1. **Unsigned integer dtypes.** Widen, reject, or add variants? Recommendation from this spike: widen to the next signed type with a one-time warning when the schema is parsed, and document the lossy mapping. Aligns with TS (no unsigned numbers).
2. **`float32` precision.** Add `ColumnType::Float` or widen to `Double`? Recommendation: add `Float`. The cost is one new variant; the benefit is honest types for pandas's `float32` columns, which are common in ML feature tables.
3. **Timezone-aware timestamps.** Add tz to `ColumnType::Timestamp` or defer? Recommendation: defer — Spark itself has tz handling muddled, and the schema-level check (the column is a timestamp) succeeds without tz. Revisit when Spark's tz story stabilizes.
4. **`timedelta64[ns]` / interval.** Add `ColumnType::Interval` or defer? Recommendation: defer to v1.4 — usage is concentrated in time-series analytics; v1.3 ships without it and adds a clean defer message.
5. **`category` vocabulary syntax.** `enum[…]` (matches v1.1) vs `category[…]` (matches pandas)? Recommendation: `enum[…]` — pykrete's vocabulary is the canonical name; the pandas idiom is the storage detail. But this is the most product-shaped question on the list and the user (PM) should weigh in.
6. **Alias lifetime for `DataFrame`.** "One major" was the locked decision. Concretely: deprecated in v1.3, removed in v2.0? Or "one major from introduction" meaning removed in v3.0? Default reading: removed in v2.0. Spec PR confirms.
7. **`DataFrame = SparkFrame` alias mechanics.** Surface in diagnostics — does the diagnostic say "SparkFrame[Order]" or "DataFrame[Order]" when the source wrote `DataFrame`? Recommendation: echo what the source wrote (no rewriting), matching pykrete's "respect the source" stance elsewhere.
8. **Cross-dialect handoff check.** A function `def f(df: SparkFrame[X]) -> PandasFrame[X]: return df.toPandas()` — does pykrete recognize `.toPandas()` as the dialect-transition operator and check the schema match? Recommendation: yes, but as a v1.4 follow-up; v1.3 should at least not regress (no false-positive diagnostics on the cross-dialect signature).
9. **Operations factoring.** When dispatch is needed inside an operation module, branch internally on dialect, or split into sibling modules? Recommendation: branch internally for v1.3 (cheaper, less churn); split once a third dialect (polars) is on the roadmap.
10. **`df.query(...)` and `df.eval(...)` string mini-DSL.** In or out? Recommendation: out for v1.3, in for v1.4 — same shape as `F.expr` for Spark.

## Cost estimate

Rough days, assuming the spec PR settles the 10 open questions
cleanly and the implementation PR ships the conservative slice
(branch-internally dispatch, no `Float` / `Interval` variants —
those are spec-decision-deferred):

| Piece | Estimate | Notes |
|---|---|---|
| Parser: extend `dataframe.rs::recognize` to `{SparkFrame, PandasFrame, DataFrame}` + dialect tag on `TypedSlot` | 0.5 day | localized change; existing tests stay green |
| Schema dtype mapping additions (unsigned widening + `Float` if approved + `Interval` if approved) | 1 day | bounded by approved scope |
| Column-ref check site (column-existence + did-you-mean for `df["x"]` against `PandasFrame[X]`) | 0.5 day | falls out of recognition extension; verify no regression on Spark |
| `df.drop(columns=[…])` dispatch | 0.5 day | mirror of Spark `.drop` |
| `df.rename(columns={…})` dispatch | 0.5 day | mirror of Spark `withColumnRenamed` |
| `df.merge(other, on=…)` dispatch | 1 day | join check-site, similar to Spark `.join` |
| `df[df["x"] == "y"]` boolean-mask filter recognition | 1 day | new shape; reuses column-ref + comparison machinery |
| `df["new"] = expr` / `df.assign(new=expr)` schema mutation | 1 day | mirror of `withColumn` |
| Hover / completion surface (`PandasFrame[X]` rendered in symbol tree, hover labels) | 0.5 day | label-only update |
| Minimal cross-codebase pandas fixture (≤200 LOC, hand-curated from sklearn or seaborn examples) | 1 day | per the v1.0 fixture pattern |
| Diagnostics-doc update for any new error code (or confirmation no new codes are needed) | 0.5 day | docs-site + diagnostics.rs |
| **Total** | **~8 days** | well under a typical minor cycle |

The encouraging finding: most of the cost is in the call-site
modules, not the type system. The type system carries pandas
with at most two new variants. The dispatch is per-call-site,
case-by-case, and most call sites are direct mirrors of their
Spark equivalents.

## Related

- `spark-coverage.md` — the Spark coverage spec. Pandas mirrors this shape; the dispatch table above is the diff.
- `literal-value-vocabulary.md` — the `enum[…]` framing principle that pandas categorical columns map onto.
- `schema-tracking-probes.md` — the framing-principle sibling; same edit-time bright line.
- Original roadmap memory ("Phase 2 work plan after v0.1.2 ship") — pandas was the v1.3 milestone; this tracker is the concrete shape.
- Spike branch `spike/v1.3-pandas/` and `spikes/v1.3-pandas/REPORT.md` — the evidence this tracker rests on.
