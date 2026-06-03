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

### Union annotations (Q5 resolved by deferral; cross-referenced from §11)

`SparkFrame[X] | PandasFrame[X]` union annotations are out of scope
for v1.3. The settled v1.3 behavior on such an annotation is
**quiet ignore**:

- Piece (a) does not commit a dialect tag when the annotation is a
  union including multiple frame dialects.
- Piece (b)'s `ctx.lookup` returns no frame-typed slot for that
  binding, so no column-ref check fires.

Rationale: pykrete v1.3 is per-annotation single-dialect. Cross-
dialect union annotations are a real production shape but require
additional design (which dispatch table applies? both? error on
incompatibilities?). Deferring is honest; silently picking one
dialect would be misleading. v1.4 is the candidate release for
explicit cross-dialect union handling. Adding support in v1.4 is a
SemVer-minor extension (no v1.3 behavior changes meaning, the union
case simply starts being checked).

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
| `uint8`  / `UInt8`  | `ColumnType::Short` | widens; lossy-cast warning emitted at schema parse (reuses existing widening diagnostic; no new D-code) |
| `uint16` / `UInt16` | `ColumnType::Int`   | widens; lossy-cast warning (reuses existing widening diagnostic) |
| `uint32` / `UInt32` | `ColumnType::Long`  | widens; lossy-cast warning (reuses existing widening diagnostic) |
| `uint64` / `UInt64` | `ColumnType::Long`  | widens; lossy-cast warning (reuses existing widening diagnostic). `uint64`'s top bit cannot fit in `Long`; user must accept truncation risk or change the schema |

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
| `category` / `CategoricalDtype([...])` | `ColumnType::Enum(vocab)` | (Q6 resolved) reuse of v1.1 `enum["a", "b", ...]` vocabulary; unordered set-equality. `ordered=True` categoricals deferred to v1.4 |
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

### §4 SemVer extension policy

The mapping table is part of the stable surface (§7). Future
changes follow:

- **Adding a new dtype mapping** post-v1.3.0 (e.g., adding
  `uint128 → Long` or `datetime64[ns, tz] → Timestamp`) is
  **SemVer-minor**. New mappings can land in any minor release.
- **Remapping an existing dtype** to a different `ColumnType` is
  **SemVer-major**. Users have built schemas against the v1.3.0
  mappings.
- **Removing a mapping** is **SemVer-major** with the standard
  deprecation cycle (warning in a minor release before removal in
  the major).

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
is by inferred type of the slice.

### Subscript-slice taxonomy (v1.3 settled)

The bare-`Expr::Subscript` shape has many concrete forms in real
production code. v1.3 takes an explicit per-form position:

| Slice shape | Example | v1.3 behavior |
|---|---|---|
| String literal | `df["col"]` | Piece (b) fires col-ref check → D0030 on unknown column |
| List of string literals | `df[["a", "b"]]` | Dispatched as `select(["a", "b"])`; col-ref check fires on EACH literal |
| `Bool`-typed expression | `df[df["x"] == "y"]` | Boolean-mask filter (pandas dispatch); piece (b) descends the inner subscript naturally |
| Name variable | `df[some_var]` | Opaque slice; no check. Constant-folding attempts deferred to v1.4 |
| Integer literal | `df[0]` | Pandas iloc-style row positional; v1.3 does not support row-positional access; quiet ignore |
| Slice object | `df[:5]` | Row slicing; v1.3 ignores |
| String `BinOp` | `df["a" + "b"]` | Constant-foldable but v1.3 does not fold; quiet ignore |
| Chained Subscript | `df["a"]["b"]` | Outer Subscript fires col-ref check on `"a"`; result type is non-frame, so inner `["b"]` does not fire |

"Quiet ignore" means no diagnostic and no result-type assignment;
the existing analyzer state is preserved unchanged. Forms not listed
above fall through to existing rules and are not v1.3's
responsibility.

### Result-type divergence: Spark vs pandas

The col-ref check that piece (b) performs is dialect-agnostic — the
same D0030 fires for unknown columns on either dialect. But the
**result type** that the analyzer threads forward differs:

- `SparkFrame[A]` receiver: `df["x"]` yields a `Column` reference
  (Spark semantics).
- `PandasFrame[A]` receiver: `df["x"]` yields a `Series` reference
  (pandas semantics).

Piece (b) is responsible for col-ref checking. Result-type
assignment for the Subscript expression itself is dialect-aware and
is the responsibility of the surrounding analyzer pass.

### §5 SemVer extension policy

- **Adding a new dispatched operation** post-v1.3.0 (e.g., adding
  `.melt`, `.pivot`, `.groupby().agg(...)`) is **SemVer-minor**.
  New operations expand checked coverage; they do not change the
  meaning of existing code.
- **Changing existing dispatch behavior** (e.g., flipping
  `df[["a", "b"]]` from col-projection dispatch to something else)
  is **SemVer-major**.
- **Tightening an existing dispatch site** (adding a new firing
  position to an already-dispatched site that previously missed a
  case) follows the SemVer-minor `tighteningDiagnostics` policy
  documented in §10.

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

### Rules-config interaction

D0090 is subject to the standard rules-config override the way every
other D-code is: users can set `"D0090": "off"` in their pykrete
config and suppress the warning entirely. This is **intentional and
allowed**. Deprecation warnings are informative-only; users who
choose to silence the signal still face the v2.0 hard break.

(Per existing rules-config semantics, all D-codes are subject to
user override. D0090 is no exception.)

### Hover and completion surface

The echo-source-text policy (Q7) applies to hover and completion
labels too: hover renders what the user wrote (`DataFrame[Order]`),
not the canonicalized form (`SparkFrame[Order]`). The wording of
the hover/completion label is part of the message-text surface and
is not stable across minor releases (see §7).

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

See §4 and §5 for the SemVer extension policies on dtype mappings
and dispatched operations respectively. The summary: adding
mappings and adding operations is minor; remapping or changing
behavior is major.

Items the v1.3 spec deliberately does *not* freeze:

- The dispatch *factoring* (internal branching vs sibling modules)
  — locked for v1.3 (Q9: internal branching) but a refactor
  toward sibling modules is permitted in any later minor without
  a breakage notice.
- The exact wording of D0090 (may be tuned without a breakage
  notice, per the diagnostic-text policy).

### JSON output contract and message-text policy

Per the v1.0 JSON stability contract:

- D0090's **identity** (the code `D0090`, the rule name
  `deprecatedDataFrameAlias`, the JSON output shape) is stable from
  v1.3.0 onward.
- D0090's **message wording** is NOT stable. The exact text may be
  retuned in any minor release (e.g., to read "DataFrame[X] is
  deprecated; use SparkFrame[X]" or any other clearer variant).
- The **echo-source-text policy** from Q7 — D0090 messages echo
  what the user wrote (`DataFrame[Order]`, not the canonicalized
  `SparkFrame[Order]`) — is part of the message-text surface. The
  SHAPE (echoing user source) is stable; the surrounding wording
  is not.

### Ordered `CategoricalDtype` characterization

§4 reuses the v1.1 `enum[...]` vocabulary for pandas `category`
columns. `ordered=True` is **NOT supported in v1.3** and ordered
categoricals degrade to unordered set-equality.

- **Adding ordered-category support post-v1.3.0** is **SemVer-minor**
  (extension via new syntax — for example, `enum_ordered[...]` or
  `enum[..., ordered=True]`). Unordered enums stay unordered;
  ordered arrives as a distinct, opt-in syntax.
- **Changing existing unordered enum semantics** (e.g., flipping
  `enum["a", "b"]` to mean ordered) is **SemVer-major**.

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
suite at the spike commit (1174 tests then; see the spike branch
for the exact SHA — this count will drift as the workspace grows)
stayed green; no regressions on Spark fixtures.

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

#### Receiver-shape bound

Piece (b) fires **only** when `subscript.value` is an `Expr::Name`
directly bound to a frame-typed slot in the analyzer's `ctx`. The
following receiver shapes are deferred to v1.4 and are quiet-ignored
in v1.3:

| Receiver shape | Example | v1.3 behavior |
|---|---|---|
| `Expr::Name` | `df["col"]` | Piece (b) fires |
| `Expr::Attribute` | `obj.df["col"]` | Out of scope; quiet ignore |
| `Expr::Call` | `get_df()["col"]` | Out of scope; quiet ignore |
| `Expr::IfExp` | `(df1 if cond else df2)["col"]` | Out of scope; quiet ignore |

The Name-only bound keeps piece (b) under the ~11 LOC budget and
matches how the existing column-ref pipeline already treats
receivers. Rebinding behavior follows naturally from `ctx.lookup`:
if a `Name` is rebound to a non-frame type, the lookup returns a
non-frame slot and the col-ref check skips.

#### `ColumnType::Float` exhaustiveness gate

The v1.3 implementation PR(s) MUST include an exhaustiveness sweep:
every site that matches on `ColumnType` must explicitly handle the
new `ColumnType::Float` variant (either preserve it, document a
deliberate widening to `Double`, or document a deliberate
fallthrough). This is the same trap the v1.1 `ColumnType::Enum`
PR-A hit and is called out here so the impl PR can't silently
collapse `Float` into a `_` arm.

The sweep is verified by `cargo check --workspace` after declaring
`Float` as non-default. The PR description must list every site
touched and the per-site disposition.

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

This is intentional widening, not a behavior break — the same
D0030 identity is reused, not a new D-code.

### Changelog commitment (required for v1.3.0 release notes)

The v1.3 CHANGELOG MUST carry a dedicated **"Tightened coverage"**
bullet that:

- Names D0030 explicitly as the diagnostic involved.
- Lists the new firing positions enabled by piece (b) (bare
  `Expr::Subscript` against any frame-typed Name receiver).
- Includes a minimal before/after example showing a v1.2-silent
  shape that v1.3 catches.
- Cross-references this §10.

The internal evidence ("1174 workspace tests stayed green") is
internal-fixtures-only and is not a substitute for external
characterization. External users with brittle CI may see new
diagnostics on existing code.

### SemVer-minor `tighteningDiagnostics` policy

Adding new firing positions to an existing D-code that was
previously silent at those positions is **SemVer-minor** under
pykrete's standard policy. The contract pykrete makes is:

- A v1.x → v1.(x+1) upgrade may emit new diagnostics on code that
  v1.x silently accepted, where the new diagnostics use **existing**
  D-code identities. Users with brittle CI may need to triage these.
- A v1.x → v1.(x+1) upgrade will not change the meaning or shape of
  the JSON output for diagnostics that were already firing.
- A v1.x → v1.(x+1) upgrade will not introduce a new D-code that
  changes the severity envelope of existing diagnostics.

The §10 widening lands squarely in the first bullet: existing D0030
identity, new firing positions, no JSON shape change. The release
notes acknowledge this is **intentional + a good catch** but worth
flagging.

### External-codebase implications

Any v1.0/v1.1/v1.2 user with a bare `df["typo"]` subscript outside
a method-call context will newly see D0030 fire when they upgrade
to v1.3. The widening is a net win for correctness (it closes a
silent gap) but the release notes call it out so users aren't
surprised.

## 11. Out of scope (bright-lined)

Explicit defers. v1.3 ships the tight slice; adjacent features
land in v1.4+. The boolean-mask disambiguation rules in §5 already
narrow the in-scope Subscript shapes; this section is the explicit
defer-list for everything outside that boundary.

- `.apply(lambda row: …)` and `.apply(lambda col: …, axis=0)` —
  runtime callbacks whose shape isn't statically inferable.
- MultiIndex columns and MultiIndex rows.
- `df.query("...")` and `df.eval("...")` string mini-DSL (Q10
  resolved: v1.4).
- `pd.read_csv(...)` and other I/O entry points (v1.4).
- Cross-dialect handoff via `.toPandas()` (Q8 resolved: v1.4).
- Cross-dialect union annotations like `SparkFrame[X] | PandasFrame[X]`
  — v1.3 quietly skips dialect tagging on union annotations (see
  §3); v1.4 is the candidate release for explicit support.
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
   §-anchor. The PR(s) MUST include the `ColumnType::Float`
   exhaustiveness sweep mandated in §9 piece (b), with a per-site
   disposition list in the PR description.
3. **Cross-codebase pandas fixture PR** — a ≤200 LOC, hand-curated
   pandas fixture. Per the cross-codebase rule (rule 4a:
   upstream-cited), every claim about pandas dtype behavior must
   cite a pandas docs source. The fixture set MUST include both
   positive PROBE-RESOLVES (real pandas usage that should pass
   under v1.3) AND negative fixtures under `probes_negative/` that
   exercise:
   - D0030 firing on bare `df["typo"]` subscripts against
     `PandasFrame[X]`,
   - D0030 firing on the same shape against `SparkFrame[X]` (the
     §10 widening),
   - D0090 firing on `DataFrame[X]` alias use.
   Donor candidates: scikit-learn and statsmodels fixtures, both of
   which use pandas heavily. The cross-codebase suite verifies
   correctness, not just absence of false positives.
4. **Atomic docs migration PR** — the docs-site reference, tutorials,
   and cookbook update to surface `PandasFrame[X]` alongside
   `SparkFrame[X]`. The `DataFrame[X]` references are not yet
   removed from docs in v1.3 (the alias is still valid); they are
   marked deprecated.
5. **Atomic trust-claim migration** (required release-blocker for
   the v1.3.0 tag). Following the same pattern as the v1.2 PR-D
   trust-claim migration: coordinate updates to README "Reliability
   and trust", docs-site `production-readiness.md`, docs-site
   `pykrete-tests.md`, the splash page, and the pykrete-tests
   README in a single PR (or a tightly coordinated pair) so the
   headline claim — "we check pandas DataFrames in addition to
   PySpark" — lands with the impl. No silent deployment window
   between the impl-ship commit and the copy update. The v1.3.0
   tag does not fly until the trust-claim copy is in place.

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
