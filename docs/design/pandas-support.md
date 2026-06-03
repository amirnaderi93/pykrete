# Pandas support — `PandasFrame[X]` (v1.3 spec, settled)

**Status**: settled v1.3 spec. Originated as a feasibility tracker
authored 2026-06-03 on `spike/v1.3-pandas`; the first proposed
technique was falsified by a throwaway validation, redesigned, and
the redesigned technique validated by a second throwaway. This
document is the settled spec the v1.3 implementation PR(s) build
against.

**Sibling**: `spark-coverage.md` (the Spark coverage spec is the
reference shape; pandas mirrors it where dtypes align and diverges
where they don't).

---

## 1. Status

- v1.3 spec, settled.
- Open questions Q1–Q10 from the feasibility tracker are all
  resolved below.
- Two validation rounds on `spike/v1.3-pandas` ground the
  implementation cost and shape (see §8 Validation history).
- No production Rust code changes in this PR; this is a spec PR
  the implementation PR(s) cite.

## 2. Headline & framing principle

> Pykrete validates things known at edit time. Pandas dtypes
> declared on a schema and pandas column references in the source
> qualify. Runtime row values do not.

Same bright line as enum constraints (`literal-value-vocabulary.md`)
and PROBE-TYPE-IS (`schema-tracking-probes.md`). Concretely for
pandas:

| Pandas construct | In scope for v1.3? | Why |
|---|---|---|
| `df["status"]` column access against declared schema | Yes | column ref is a literal, schema is known |
| `df.status` column access against declared schema | Yes | same |
| `df.loc[:, "status"]` column access | Yes | literal slice key |
| `df.drop(columns=["status"])` schema transform | Yes | mirrors Spark `.drop("status")` |
| `df.rename(columns={"old": "new"})` | Yes | literal rename map |
| `df.merge(other, on="key")` | Yes | mirrors Spark `.join` |
| Dtype mismatch on `df["new"] = expr` | Yes | same as Spark `withColumn` |
| `df.apply(lambda row: ...)` row-runtime callback | No | row value not knowable at edit time |
| `df.query("status == 'shipped'")` runtime string | No (v1.3) | DSL parsing deferred to v1.4 |
| MultiIndex columns | No | hierarchical-schema work; out of scope |
| `.iloc[3]` row-positional access | No | row position is not a schema concept |
| `df["new"] = some_runtime_python_expr` | Partial | in scope when RHS is a typed-column expression; runtime mutation otherwise out |

The shape of the line is the same as Spark's: pykrete sees the
schema, sees the column references, type-checks where types are
known, leaves everything else alone.

## 3. Syntax

`SparkFrame[X]` is canonical for Spark; `PandasFrame[X]` is canonical
for pandas. `DataFrame[X]` is a deprecated alias for `SparkFrame[X]`
in v1.3 and is **removed in v2.0**.

```python
from pykrete import SparkFrame, PandasFrame, Schema
from pykrete import long, string, decimal, timestamp

class Order(Schema):
    id: long
    status: string
    amount: decimal(18, 2)
    created_at: timestamp

def f(df: SparkFrame[Order]) -> SparkFrame[Order]: ...
def g(df: PandasFrame[Order]) -> PandasFrame[Order]: ...
def h(df: DataFrame[Order])   -> DataFrame[Order]:   ...   # deprecated alias of SparkFrame
```

The subscript shape (`Frame[Schema]`) is identical across both
canonical forms — same parser path, same `Pick[…]` / `Omit[…]` /
`Merge[…]` derived-schema operators, same inline dict shape. The
difference is a parser-level *dialect tag* on the resulting
`TypedSlot`, which drives check-site dispatch.

`SparkFrame[X] | PandasFrame[X]` union annotations are out of scope
for v1.3.

## 4. Dtype mapping (settled)

The full mapping table below incorporates the resolutions of Q1–Q4.

### Signed integers

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `int8` / `Int8` | `ColumnType::Byte` | clean 1:1 |
| `int16` / `Int16` | `ColumnType::Short` | clean 1:1 |
| `int32` / `Int32` | `ColumnType::Int` | clean 1:1 |
| `int64` / `Int64` | `ColumnType::Long` | clean 1:1 — pandas default for `int` columns |

### Unsigned integers — Q1 resolved: widen to next signed

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `uint8`  / `UInt8`  | `ColumnType::Short` | widens; lossy-cast warning emitted at schema parse |
| `uint16` / `UInt16` | `ColumnType::Int`   | widens; lossy-cast warning |
| `uint32` / `UInt32` | `ColumnType::Long`  | widens; lossy-cast warning |
| `uint64` / `UInt64` | `ColumnType::Long`  | widens; lossy-cast warning. `uint64`'s top bit cannot fit in `Long`; user must accept truncation risk or change the schema |

Rationale: TypeScript's `number` type carries no signedness; pykrete
follows the same precedent and widens. Users who want tightness can
declare the schema column as the signed equivalent directly and
self-assert.

### Floats — Q2 resolved: add `ColumnType::Float`

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `float32` / `Float32` | `ColumnType::Float`  | NEW variant. Avoids the lossy `float32 → Double` widening that would silently misrepresent ML feature tables |
| `float64` / `Float64` | `ColumnType::Double` | clean 1:1 — pandas default for floats |

`ColumnType::Float` is the only net-new `ColumnType` variant
required by the v1.3 spec.

### Strings

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `object` (string-typed column) | `ColumnType::String` | legacy default; pykrete trusts the declared schema |
| `string` / `StringDtype` | `ColumnType::String` | modern explicit string (pandas 1.0+) |
| `string[pyarrow]` | `ColumnType::String` | Arrow-backed string; storage detail, not type detail |
| `category` / `CategoricalDtype([...])` | `ColumnType::Enum(vocab)` | resolved via reuse of v1.1 `enum["a", "b", ...]` vocabulary; unordered set-equality. Ordered categoricals deferred to v1.4 |
| `category` with no categories declared | `ColumnType::String` | open-vocabulary categoricals degrade to plain string |

### Booleans

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `bool` / `bool_` | `ColumnType::Bool` | clean 1:1 |
| `boolean` / `BooleanDtype` | `ColumnType::Nullable(Box::new(Bool))` | nullable boolean uses existing `Nullable` |

### Dates and times

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `datetime64[ns]` | `ColumnType::Timestamp` | clean 1:1 at the schema level |
| `datetime64[ns, tz]` | (deferred to v1.4) | Q3 resolved: defer. Spark's tz story is unsettled; v1.3 ships without enforcing tz |
| `timedelta64[ns]` | (deferred to v1.4) | Q4 resolved: defer. Likely arrives as `ColumnType::Interval` then |
| `period` / `PeriodDtype` | (unsupported) | no Spark equivalent; defer indefinitely |
| `date` (object dtype holding `datetime.date`) | `ColumnType::Date` | pandas has no first-class date dtype; pykrete trusts declared schema |

### Other / structured

| Pandas dtype | Mapping | Notes |
|---|---|---|
| `bytes` / `object` holding bytes | `ColumnType::Binary` | parallel to the string case |
| `interval` / `IntervalDtype` | (unsupported, deferred) | no Spark equivalent |
| `Sparse[T]` | mapping of `T` | sparseness is a storage detail; pykrete sees through it |
| MultiIndex columns | (unsupported) | explicit anti-scope |

## 5. Per-call-site dispatch table

Six call-site dispatches and one net-new Subscript shape. The
choice between them is made by reading the dialect tag off the
bound `TypedSlot` and branching internally (see §9 piece (a)).

| Operation | Spark form | Pandas form | Dispatch needed? |
|---|---|---|---|
| Column projection | `df.select(col("x"), col("y"))` | `df[["x", "y"]]` | Yes |
| Boolean filter | `df.filter(col("x") == "y")` | `df[df["x"] == "y"]` | Yes — **net-new Subscript shape**, see below |
| Column add / replace | `df.withColumn("new", expr)` | `df["new"] = expr` or `df.assign(new=expr)` | Yes |
| Drop column | `df.drop("x")` | `df.drop(columns=["x"])` (`df.drop("x")` is row-by-index in pandas) | Yes |
| Join | `df.join(other, on="key")` | `df.merge(other, on="key")` | Yes |
| Rename | `df.withColumnRenamed("a", "b")` | `df.rename(columns={"a": "b"})` | Yes |
| Column reference | `df.x` / `df["x"]` | `df.x` / `df["x"]` | No — shared via piece (b) |

**Net-new shape: `df[boolean_mask]`.** This is the pandas boolean-row
filter and uses bare `Expr::Subscript` whose slice is a column-typed
boolean expression. Disambiguation from the column-projection
Subscript (`df[["x"]]`) and the single-column Subscript (`df["x"]`)
is by inferred type of the slice:

- slice = string literal → column reference (shared with Spark)
- slice = list of string literals → column projection (pandas
  dispatch)
- slice = expression whose type is `Bool` and whose schema matches
  the receiver → boolean-mask filter (pandas dispatch)

Anything else is an error or ignored per existing rules.

## 6. `DataFrame` deprecation policy

- v1.3: `DataFrame[X]` continues to parse and is treated as a
  synonym for `SparkFrame[X]`. Pykrete emits a new deprecation
  diagnostic.
- v2.0: `DataFrame[X]` is removed. The diagnostic becomes a hard
  error.

**Reserved D-code: `D0090` (`deprecatedDataFrameAlias`).** Severity:
warning. Message echoes the source text — if the user wrote
`DataFrame[Order]`, the diagnostic and any hover labels render
`DataFrame[Order]`, not the canonicalized `SparkFrame[Order]`.
(Q7 resolved: respect the source.)

Example:

```
orders.pyk:4:14 - warning deprecatedDataFrameAlias: 'DataFrame[Order]' is a deprecated alias for 'SparkFrame[Order]' and will be removed in pykrete v2.0. Rewrite as 'SparkFrame[Order]'.
```

## 7. Stability surface

From v1.3.0, the following are part of pykrete's stable public
surface and follow the standard deprecation policy:

- The annotation forms `SparkFrame[X]` and `PandasFrame[X]`.
- The `DataFrame[X]` alias is **stable as a deprecated alias** —
  emits a warning, but does not break — until v2.0.
- The per-call-site pandas dispatch table in §5 (the listed six
  operations + the boolean-mask Subscript shape).
- The pandas dtype → `ColumnType` mapping in §4, including the
  new `ColumnType::Float` variant.

Items the v1.3 spec deliberately does *not* freeze:

- The dispatch *factoring* (internal branching vs sibling modules)
  — locked for v1.3 (Q9: internal branching) but a refactor
  toward sibling modules is permitted in any later minor without
  a breakage notice.
- The exact wording of D0090 (may be tuned without a breakage
  notice, per the diagnostic-text policy).

## 8. Validation history

Two throwaway-validation rounds on `spike/v1.3-pandas`. Both are
cited in full so future readers see the rigor.

### Round 1 — FALSIFIED

The initial tracker claimed that column-ref checking on
`PandasFrame[X]` would "fall out for free" with just an
annotation-recognition extension in `dataframe.rs::recognize`.

The throwaway probe constructed a minimal pandas-shaped fixture
(`def f(orders: PandasFrame[Order]): return orders[orders["statuss"] == "shipped"]`)
and ran the existing column-ref pipeline against it. **No D0030
fired.**

Root cause: `collect_col_refs` is only invoked from method-call
sites (`.filter(...)`, `.select(...)`, etc.). The pandas boolean-mask
idiom `orders[orders["statuss"] == "shipped"]` is a bare
`Expr::Subscript` with no enclosing method call, so the column-ref
walker never sees `orders["statuss"]`.

Falsification implication: the "free" technique does not exist.
Pandas requires a dedicated entry point into column-ref checking
from bare Subscript expressions.

### Round 2 — VALIDATED

The redesigned technique (described in §9 below) was implemented
as a throwaway and re-probed. The same fixture now emits D0030
on `"statuss"` and suggests `"status"`. The full workspace test
suite (1174 tests) stayed green; no regressions on Spark fixtures.

Both rounds were thrown away after measurement; no spike-throwaway
code lands in the v1.3 implementation PR(s). The implementation
re-derives the technique against a clean baseline.

## 9. The validated redesign

Two composable pieces, each tightly scoped.

### Piece (a) — annotation recognition

In `crates/pykrete/src/operations/dataframe.rs::recognize`, widen
the matched annotation name set from `{DataFrame}` to
`{SparkFrame, PandasFrame, DataFrame}` and attach a dialect tag
to the resulting `DataFrameAnnotation`:

- `SparkFrame[X]` → `Dialect::Spark`
- `PandasFrame[X]` → `Dialect::Pandas`
- `DataFrame[X]` → `Dialect::Spark` (alias) + flag for D0090

The dialect tag propagates onto the bound `TypedSlot`. This
widens which slots get frame-typed in the first place — the
existing column-ref machinery does not know about pandas until
this piece exists.

Net change: ~4 LOC.

### Piece (b) — bare-Subscript col-ref entry point

In `crates/pykrete/src/operations/expr.rs::analyze_expr`, add an
arm for the `Expr::Subscript` case that:

1. Resolves `subscript.value` via `ctx.lookup`.
2. If the resolved slot is frame-typed (Spark or Pandas) and
   `subscript.slice` is a string literal, invoke the existing
   `report_column_refs` machinery on `(slot, literal)`.

No new types, no new traits, no new D-codes. The existing column-ref
machinery — including the "did you mean" suggestion path — is
reused unchanged.

Net change: ~11 LOC.

### Composability

The two pieces compose cleanly through `ctx.lookup`. Piece (a)
widens *which* slots get a frame type; piece (b) walks bare
Subscript expressions and naturally picks up any frame-typed slot
that piece (a) has produced. Neither piece needs to know about
the other.

### Net cost

~0.25 dev-days for the technique itself (vs. the implied 0 days
in round 1's "falls out for free" claim). The corrected cost
folds into the overall §12 estimate.

## 10. Bonus: Spark-side coverage widening

A side-effect of piece (b) is that bare-Subscript column references
against **`SparkFrame[X]`** are also now checked.

This closes a silent gap on the Spark side: today, an expression
like

```pyk
df.filter(df["statuss"] == "shipped")
```

correctly catches the typo inside `.filter`. But:

```pyk
result = df[df["statuss"] == "shipped"]
```

against a `SparkFrame[X]` did **not** previously catch the typo,
because `collect_col_refs` only fires from method-call sites. Under
v1.3, the bare `df["statuss"]` Subscript is checked regardless of
dialect.

This is intentional widening, not a behavior break. The 1174-test
workspace stayed green under the round-2 throwaway, which is
evidence that no Spark fixture relied on the old silence. We call
the widening out explicitly in the v1.3 changelog so users who do
hit a new diagnostic on previously-tolerated code understand the
source.

## 11. Out of scope (bright-lined)

Explicit defers. v1.3 ships the tight slice; adjacent features
land in v1.4+.

- `.apply(lambda row: …)` and `.apply(lambda col: …, axis=0)` —
  runtime callbacks whose shape isn't statically inferable.
- MultiIndex columns and MultiIndex rows.
- `df.query("...")` and `df.eval("...")` string mini-DSL (Q10
  resolved: v1.4).
- `pd.read_csv(...)` and other I/O entry points (v1.4).
- Cross-dialect handoff via `.toPandas()` (Q8 resolved: v1.4).
- `SparkFrame[X] | PandasFrame[X]` union annotations.
- Copy-vs-view semantics (`SettingWithCopyWarning`) — runtime.
- NumPy structured arrays as DataFrame sources.
- Arrow-backed pandas dtype distinctions beyond the §4 mapping.
- Ordered `CategoricalDtype(ordered=True)` ordering enforcement
  (v1.4 if demand surfaces).
- tz-aware `datetime64[ns, tz]` enforcement (Q3, v1.4).
- `timedelta64[ns]` / `IntervalDtype` (Q4, v1.4).
- `period` / `PeriodDtype` — no Spark equivalent; indefinitely
  deferred.

## 12. Cost estimate

| Piece | Estimate | Notes |
|---|---|---|
| Piece (a): `dataframe.rs::recognize` extension + `Dialect` tag on `TypedSlot` | 0.5 day | per round-2 validation |
| Piece (b): bare-Subscript col-ref entry point in `analyze_expr` | 0.25 day | per round-2 validation; reuses `report_column_refs` |
| Dtype mapping additions (unsigned widening + `ColumnType::Float`) | 1 day | bounded by §4 |
| `df[["x", "y"]]` column-projection dispatch | 0.5 day | mirror of `.select` |
| `df[boolean_mask]` filter dispatch (net-new shape) | 1 day | reuses column-ref + comparison machinery |
| `df["new"] = expr` / `df.assign(...)` schema mutation | 1 day | mirror of `withColumn` |
| `df.drop(columns=[...])` dispatch | 0.5 day | mirror of Spark `.drop` |
| `df.rename(columns={...})` dispatch | 0.5 day | mirror of `withColumnRenamed` |
| `df.merge(other, on=...)` dispatch | 1 day | mirror of `.join` |
| D0090 deprecation diagnostic (rule + tests + diagnostics.md entry) | 0.5 day | the only new D-code in v1.3 |
| Hover / completion surface (`PandasFrame[X]` rendering, dialect-aware hover) | 0.5 day | label-only |
| Cross-codebase pandas fixture (≤200 LOC, upstream-cited dtype claims) | 1 day | per the v1.0 fixture pattern + the rule that dtype claims must cite pandas docs |
| **Total** | **~8.25 days** | well under a typical minor cycle |

Round-1's "0 days" estimate for column-ref checking was wrong by
the cost of piece (b) (0.25 day). The corrected total is 8.25 days.

## 13. v1.3 work plan

1. **Spec PR (this PR)** — settles Q1–Q10, locks syntax, dtype
   mapping, dispatch table, deprecation policy, D-code reservation.
   No production Rust changes.
2. **Implementation PR(s)** — one or more code PRs that implement
   piece (a), piece (b), the six dispatched operations, the
   net-new boolean-mask shape, `ColumnType::Float`, unsigned-int
   widening, and D0090. Each PR cites this spec and the relevant
   §-anchor.
3. **Cross-codebase pandas fixture PR** — a ≤200 LOC, hand-curated
   pandas fixture. Per the cross-codebase rule, every claim about
   pandas dtype behavior must cite a pandas docs source so the
   fixture verifies correctness rather than just "no diagnostic
   fires."
4. **Atomic docs migration PR** — the docs-site reference, tutorials,
   and cookbook update to surface `PandasFrame[X]` alongside
   `SparkFrame[X]`. The `DataFrame[X]` references are not yet
   removed from docs in v1.3 (the alias is still valid); they are
   marked deprecated.

---

## Reference

- Spike branch: `spike/v1.3-pandas` (origin tracker + both
  throwaway-validation rounds).
- Round-1 throwaway: probed the original "free column-ref check"
  claim — falsified. `collect_col_refs` only fires from method
  calls, so bare `Subscript` shapes are never walked.
- Round-2 throwaway: probed the redesigned two-piece technique
  (`dataframe.rs::recognize` extension + `analyze_expr` Subscript
  arm). Validated. 1174 workspace tests stay green; the target
  D0030 fires on the pandas boolean-mask fixture.
- Sibling specs: `spark-coverage.md`, `literal-value-vocabulary.md`,
  `schema-tracking-probes.md`.
- Originating memory: "Phase 2 work plan after v0.1.2 ship" —
  pandas via per-annotation `SparkFrame[X]` / `PandasFrame[X]`
  dispatch was the v1.3 milestone since v0.1.2.
