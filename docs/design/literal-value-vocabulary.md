# Literal value vocabulary — enum constraints (v1.1 spec)

**Status**: settled spec — implementation PR may pick up.
**Origin**: 2026-05-31 user proposal during the pre-v1.0.0 sprint;
promoted from design tracker on 2026-06-01 after the 8 open questions
were resolved. Tightened on 2026-06-01 (round 2 of the multi-lens
synthesis review) to add Q9 (branch-form expressions), promote four
"impl-PR judgment calls" to spec-level decisions, and settle eleven
adjacent gaps surfaced by the correctness / adversarial /
schema-stability lenses.
**Sibling**: `schema-tracking-probes.md` — both are "make the schema
layer earn its trust through positive verification." This spec adds the
literal-value vocabulary; probes verify what the schema layer already
tracks. They compose: probes can target `D0084` once this lands.

## The pitch

A schema author should be able to declare the allowed value set for an
enum-shaped string column, and pykrete should catch typos and unknown
values in literal comparisons, assignments, and `isin` clauses at edit
time.

```python
class Order(Schema):
    id: long
    status: enum["pending", "shipped", "delivered", "cancelled"]
```

```python
def stale(orders: DataFrame[Order]) -> DataFrame[Order]:
    return orders.filter(col("status") == "pendig")
    #                                       ^^^^^^^ D0084: 'pendig' is not in the
    #                                              enum vocabulary for 'status'.
    #                                              Did you mean 'pending'?
```

The class of bug this catches: `df.filter(col("status") == "actiev")`
silently returns an empty DataFrame in production. Everything appears
to work until someone notices the metric is zero. It is exactly the
silent-bug class pykrete was built to catch.

## Framing principle (load-bearing)

> Pykrete validates things known at edit time. Enum literals qualify
> (the constraint is on the literal value present in the source).
> Runtime row values do not (we'd need a runtime to see them).

This is the bright line. It draws a permanent boundary against the
inevitable "what about min/max, regex, date ranges, NOT NULL, foreign
keys, ..." requests:

| Constraint | In scope? | Why |
|---|---|---|
| Enum literal `status == "pendig"` (typo) | Yes | literal vs literal — both known at edit time |
| Enum literal in `.isin("a", "b")` | Yes | same |
| Enum literal in `.fillna({"status": "open"})` | Yes | same |
| Enum literal in `withColumn("status", lit("c"))` | Yes | same |
| Enum literal in `F.expr("status = 'actiev'")` | Yes | SQL string literal, parsed by `sql.rs` |
| `amount > 0` runtime row value | No | row value not knowable at edit time |
| `date BETWEEN '2024-01-01' AND today()` | No | row value not knowable |
| `name MATCHES /\d+/` runtime regex on row | No | same |
| NOT NULL on row | No | same |

Pykrete is a checker, not a validation library. The enum case is the
one shape where the constraint and the data the constraint applies to
are both source literals — uniquely suited to compile-time checking.

## Chosen syntax: Form A — `enum["a", "b", ...]` (LOCKED)

Three forms were on the table during the design phase:

```python
# A: subscript on a built-in enum keyword (CHOSEN)
class Order(Schema):
    status: enum["pending", "shipped", "delivered", "cancelled"]

# B: parametric form mirroring decimal(p, s) (REJECTED)
class Order(Schema):
    status: enum("pending", "shipped", "delivered", "cancelled")

# C: separate decorator-like annotation (REJECTED)
class Order(Schema):
    status: string
    _status_values = ["pending", "shipped", "delivered", "cancelled"]
```

**Form A is locked for v1.1.** Rationale:

- Mirrors every other parametric pykrete type — `decimal[18, 2]`,
  `Array[T]`, `Map[K, V]`. One mental model for "type with parameters".
- PEP 526 annotation lane only accepts expressions; a subscript on a
  name is the cleanest expression that reads as a type.
- Form B's call-shape `enum(...)` confuses readers familiar with
  Python's stdlib `enum` module — they expect a runtime enum class,
  but pykrete `enum[...]` is a static type annotation, never
  evaluated at runtime.
- Form C splits the declaration in two, which violates the
  "schema is a single declaration" ergonomics the rest of pykrete
  commits to.

The reserved keyword `enum` is lowercase, matching `string`, `int`,
`long`, `decimal`, etc. Capital `Enum` (Python stdlib) is unaffected.

## Settled questions (9 of 9 resolved)

Each question below was identified during the design tracker phase.
Each now has a concrete decision an implementation PR can encode
without further design discussion.

> See also the [**Spec-level decisions promoted from round-1
> impl-PR list**](#spec-level-decisions-promoted-from-round-1-impl-pr-list)
> section near the end of this document — it captures four
> previously-parked judgment calls (empty enum, duplicate entries,
> hover rendering, Levenshtein threshold) that decide user-visible
> behaviour and so are settled here.

### Q1. Unification on `withColumn("status", lit("c"))` where `"c"` is off-enum.

**Decision: fire `D0084` diagnostic.** Off-enum literal assignment is a
bug — the whole point of declaring the enum is to catch off-enum
values. Silent widening (the alternative) would defeat the purpose;
the user explicitly opted in to enum constraints by writing
`enum[...]` in the schema.

Concrete shape:

```python
class Order(Schema):
    status: enum["pending", "shipped"]

def f(orders: DataFrame[Order]) -> DataFrame[Order]:
    return orders.withColumn("status", lit("delivered"))
    #                                   ^^^^^^^^^^^^^^ D0084: 'delivered' is not
    #                                                  in the enum vocabulary
    #                                                  for 'status'. Did you
    #                                                  mean 'pending', 'shipped'?
```

Note: if the user genuinely needs to widen, they can change the schema
declaration (the schema is the source of truth). There is no per-call
"trust me, widen here" escape hatch in v1.1 (filed as a v1.2 candidate
only if real usage demands it).

#### Q1a. String comparison semantics for enum membership.

**Decision: byte-exact, case-sensitive, no normalization.** This is
consistent with pykrete's case-sensitive column-name policy (we check
against the user's declared schema, not a normalized form).

- **Case**: `enum["pending"]` rejects `"Pending"` and `"PENDING"`. The
  user is comparing against literals they wrote themselves; if they
  want case-insensitive membership, they `.lower()` first (which
  drops the constraint per Q2 — by design).
- **Unicode**: full Unicode allowed in the vocabulary. `enum["café"]`
  accepts the literal `"café"` (UTF-8 byte-for-byte match). No NFC /
  NFD normalization: composed and decomposed forms of the same
  grapheme are distinct values.
- **Whitespace**: literal-preserving. `enum["pending "]` (trailing
  space) is a distinct value from `enum["pending"]`. No auto-trim.
  Users who want trim-then-compare wrap with `.trim()`, which drops
  the constraint per Q2.

#### Q1b. Column-to-column assignments and equality.

The literal RHS case in Q1 is the headline shape; column-to-column
shapes also fire:

- `df.withColumn("status", col("other"))` where `other` is plain
  `string` (no enum constraint): **fire `D0084`** with message
  "assigning unconstrained string into enum-typed sink 'status'."
  Same code, slightly different message body.
- `df.withColumn("status", col("other_enum"))` where both columns are
  enum-typed: same rules as Q4 Merge — identical value sets pass
  silently, **non-identical sets fire `D0040`** (do not silently
  union or intersect; the user signed up for neither interpretation).
- `df.filter(col("a") == col("b"))` where `a` and `b` are enum-typed
  with **disjoint** vocabularies: the comparison is provably false at
  edit time. Fire by extending the existing `D0082 crossTypeComparison`
  logic — no new D-code. The message body distinguishes
  "non-overlapping enum vocabularies" from the generic cross-type
  case, but the code is `D0082`.

#### Q1c. Precedence when `D0030`, `D0082`, and `D0084` could both fire.

`col("enum_field") == col("int_field")` could in principle fire both
`D0082` (cross-type) and `D0084` (off-enum literal); add an unknown
column reference and `D0030` could also fire. **Lock the full
precedence chain: `D0030` > `D0082` > `D0084`.**

- `D0030 unknownColumn` is the first failure — without a resolved
  column, the enum constraint is moot (we don't know the column's
  type at all yet).
- `D0082 crossTypeComparison` is next — once the column resolves, a
  cross-type comparison is the more fundamental error; the off-enum
  sub-check only runs once both sides are confirmed string-typed (or
  enum-typed).
- `D0084 enumValueMismatch` only runs once the column resolves and
  the types are compatible.

The chain reflects which failure mode prevents the next from being
meaningfully checked. Same precedence applies to `col("enum_field")
== True` and similar boolean / numeric coercion collisions: the type
error fires first; the enum sub-check never runs.

### Q2. String-producing operations drop the constraint.

**Decision: any string-returning operation outside the explicit
preserve-list (below) drops the enum constraint; the result type is
plain `string`.** The source-of-truth list of string-returning
functions lives in `crates/pykrete/src/operations/column_exprs.rs`
(the `match` arm that returns `Some(String)` for the string-producing
operations). At time of writing (2026-06-01, commit `4bbb9a1`) the
list is:

> `lower`, `upper`, `initcap`, `trim`, `ltrim`, `rtrim`, `reverse`,
> `concat_ws`, `substring`, `substring_index`, `regexp_replace`,
> `regexp_extract`, `lpad`, `rpad`, `translate`, `repeat`, `soundex`,
> `base64`, `format_string`, `format_number`, `hex`, `sha1`, `sha2`,
> `md5`, `date_format`, `from_unixtime`

Plus the column-method analogues `.substr`, `.cast("string")`,
`.concat`, `F.concat`, `F.replace`, and `.split` (which returns
`Array[string]` — the element type loses the constraint). All of
these produce strings that may or may not be in the enum vocabulary;
the constraint cannot be carried through soundly.

The impl PR does NOT maintain a duplicate spec-side list. The single
authoritative list is the `match` arm in `column_exprs.rs` (the arm
that returns `Some(String)` for string-producing operations); the
spec references that arm by name, not by line number, so it stays
correct across refactors. The impl PR is responsible for ensuring
every entry in that arm participates in the "drop the constraint"
path. New string-returning ops added to that arm in later patches
automatically inherit the drop behaviour — no spec update required.

**Preserve-side ops (explicit list, NOT in the drop path):**

- `.alias(name)` / `F.col(...).alias(...)` — rename, not transform.
  Output keeps the input's enum constraint verbatim.
- The Q5 aggregation entries that say "Carries enum constraint?: Yes"
  — `first`, `last`, `min`, `max`, `collect_set`, `collect_list`.
- The Q9 branch-form entries that propagate the constraint when all
  branches agree.
- `greatest(col_a, col_b)` / `least(col_a, col_b)` over enum inputs:
  **drop the constraint.** Output type is plain `string`. Rationale:
  string ordering is lexicographic, not semantic; "greatest of
  pending/shipped" has no meaningful enum interpretation. A future
  v1.2 may revisit if a real use case appears; for v1.1 it is a
  drop to avoid a confusing "this value is in the enum because we
  said so" surface.

Documented in `docs-site/src/content/docs/reference/types.md` (added
alongside the implementation PR) so users aren't surprised when
chaining a transform "loses" the enum constraint.

Implementation note for the impl PR: the result type of drop-path ops
is `ColumnType::String` (no constraint carry-through); this is the
default fall-through path — only operations explicitly listed in the
preserve-side list above, Q5, and Q9 preserve the constraint.

### Q3. Cast from arbitrary `string` to enum.

**Decision: disallowed.** `.cast("enum[...]")` and any other cast-form
producing an enum-typed column are not allowed. The only way to get an
enum-typed column is via the schema declaration. Cast from plain
`string` → enum would require runtime validation — out of scope per
the framing principle.

Concrete shape: pykrete's type parser (`types.rs::from_name`) refuses
the `enum[...]` form when invoked from a cast context. The
implementation PR routes through a single `parse_cast_target` entry
point that rejects `enum[...]` with a clear message:

```
.cast("enum[...]") is not allowed — only schema declarations can
produce enum-typed columns. To widen, change the schema; to narrow
from a string, restructure the pipeline.
```

This is a hard reject, not a `D0084` — it's a fundamentally different
class of error (forbidden cast target, not off-enum value). The impl
PR uses the existing `D0011 invalidColumnType` code for this case; no
new D-code is needed.

### Q4. Schema operators carry the constraint.

**Decision: `Pick`, `Omit`, `Merge` preserve the enum constraint on
every carried-through field.**

- `Pick[Order, "status"]` produces a one-column schema with `status`
  still enum-constrained.
- `Omit[Order, "id"]` produces every-column-except-`id`, with the
  `status` enum constraint preserved.
- `Merge[A, B]`:
  - **Set-equality semantics**: two enum vocabularies are equal iff
    they contain the same elements as sets. Declaration order does
    not matter: `enum["a", "b"]` and `enum["b", "a"]` are identical.
  - If `A.status` is `enum[X, Y]` and `B` does not declare `status`,
    the merged column carries `enum[X, Y]`.
  - If `A.status` and `B.status` both declare `enum[...]` with
    **set-equal** value sets, the merged column carries that set.
  - If `A.status` and `B.status` declare **non-set-equal** value
    sets (one is a subset, or they overlap partially, or they're
    disjoint), fire `D0040 unionSchemaMismatch` (the existing code
    for this class of conflict) with a message that names both
    value sets. Do NOT silently union or intersect the sets — the
    user did not sign up for either interpretation; surface the
    conflict and let them widen one side explicitly.
  - If one side declares `enum[...]` and the other plain `string`,
    fire `D0040` with the same conflict message. The user must
    reconcile in the source schema.

Tested explicitly in the impl PR's snapshot suite — at least one
fixture per case above.

### Q5. Aggregations and the constraint.

**Decision: the aggregation result type is determined per-operation.**

Each Spark aggregation function maps to a specific carry-through
behaviour:

| Operation | Carries enum constraint? | Result type |
|---|---|---|
| `first(col)` | Yes | `enum[...]` (same as input; `Nullable` wrapper preserved if input is `Nullable[enum[...]]`) |
| `last(col)` | Yes | `enum[...]` (Nullable wrapper preserved) |
| `min(col)` | Yes | `enum[...]` (lexicographic min, still in vocabulary; Nullable wrapper preserved) |
| `max(col)` | Yes | `enum[...]` (Nullable wrapper preserved) |
| `collect_set(col)` | Yes | `Array[enum[...]]` |
| `collect_list(col)` | Yes | `Array[enum[...]]` |
| `count(col)` | N/A | `long` (returns count, never the value) |
| `count_distinct(col)` | N/A | `long` |
| `countDistinct(col)` | N/A | `long` |
| `approx_count_distinct(col)` | N/A | `long` |
| `sum(col)` | N/A | already fires `D0081 nonNumericArithmetic` on string |
| `avg(col)` / `mean(col)` | N/A | already fires `D0081` on string |
| `stddev(col)` / `variance(col)` etc. | N/A | already fires `D0081` on string |

The "carries" cases are operations that emit a value drawn from the
input column — by construction, the output value is still in the
input vocabulary. The "N/A" cases either produce a numeric (count) or
are already rejected by existing rules on string columns.

`count_distinct` and `countDistinct` are aliases — Spark exposes both
spellings of the same function. They route through a single code path
in the impl PR (no double-coverage); both names are listed in the
table only for completeness against the Spark surface.

`groupBy("status").count()` produces a schema with `status` (still
enum-constrained, since the group key passes through) plus `count:
long`. Tested.

### Q6. `F.lit("...")` inside other expressions.

**Decision: checked against the enum-typed sink; standalone is opaque.**

Two distinct cases:

- **Sink-bound**: `F.lit("active")` flowing into a position with a
  known enum-typed sink — `withColumn("status", lit("active"))`,
  `.fillna({"status": lit("active")})`, `(col("status") ==
  lit("active"))`. The checker resolves the sink's enum vocabulary
  and validates the literal against it. Off-enum: fire `D0084`.
- **Standalone**: `F.lit("active")` outside any enum-typed sink
  context (e.g. assigned to a variable, returned from a helper) has
  no enum context to check against. The literal's type is plain
  `string`. If the variable is later threaded into an enum sink, the
  check fires at the sink (the literal's source location is reported
  as the offending span via the existing diagnostic-span machinery).

This mirrors how the existing `D0030` (unknownColumn) flows — the
check fires where the constraint is known, not where the literal
first appears.

### Q7. `F.expr("status = 'actiev'")` SQL fragment.

**Decision: yes, the SQL parser at `sql.rs` checks string-literal RHS
against an enum constraint when the LHS resolves to an enum column.**

Consistency wins here — `col("status") == "actiev"` and
`F.expr("status = 'actiev'")` are user-equivalent; firing on the
former but not the latter would be a confusing gap.

Implementation surface (concrete for the impl PR):

- Today's `sql.rs::column_refs` extracts bare-identifier column
  references from SQL fragments. The impl PR extends the parser
  walk to also capture binary-comparison shape `<ident> = '<lit>'`
  (and `IN ('a', 'b', ...)` for the SQL equivalent of `.isin`).
- For each captured `(column, string-literal)` pair, the checker
  looks up `column`'s type via the existing schema-tracking
  machinery. If it resolves to `enum[...]`, validate the literal
  against the vocabulary; fire `D0084` on mismatch.
- Span: the diagnostic span is the **substring within the SQL
  fragment**, computed from the offset of the string literal in
  the original Python source (the fragment's start offset plus
  the literal's offset within the fragment). The existing
  `sql.rs` already does offset arithmetic for `D0030` in SQL
  fragments; the impl PR reuses the same routine.

Edge case (documented for the impl PR): SQL identifiers and string
literals can be quoted differently (backticked idents,
single-quoted literals). The impl PR handles the common case
(`status = 'value'`) and falls through silently on exotic shapes
(e.g. `` `status` = "value" ``) rather than risking false positives
from imperfect SQL parsing. The impl PR documents which shapes are
covered in the snapshot suite.

**Silent fall-through is an acceptable v1.1 limitation.** False
negatives in SQL parsing are acceptable because enum-vocabulary
checking is opt-in (the schema author chose to declare `enum[...]`);
a missed off-vocabulary literal inside a backticked-ident SQL
fragment defaults to the v1.0 behaviour (no check) without
regressing anything. The set of covered SQL shapes can be broadened
in later minor versions without breaking the v1.1 contract.

### Q8. JSON output contract / new D-code.

**Decision: reserve `D0084 enumValueMismatch`. No new JSON fields.**

The new code:

- **Code**: `D0084`
- **Rule name**: `enumValueMismatch`
- **Severity**: `Error`
- **Min mode**: `CheckMode::Basic` (this is a real bug, not an
  advisory)
- **Message form**: `'<value>' is not in the enum vocabulary for
  '<column>'. Did you mean '<suggestion>'?` (suggestion clause
  omitted when no close match found via Levenshtein search)
- **Suggestion field** (existing `Diagnostic::suggestion` slot):
  populated with the best Levenshtein match, reusing the same
  routine `D0030` uses for column-name typos.
- **Source-of-truth registration**: appended at the end of
  `DIAGNOSTIC_CATALOG` in `crates/pykrete/src/diagnostics.rs`,
  immediately after the current `("D0083", "nullabilityMismatch")`
  entry.

Severity-across-modes: `D0084` fires at `error` severity in all check
modes (`basic`, `standard`, `strict`). There is no per-mode downgrade
to `warning`; the off-enum-literal class of bug is unambiguous.

**No new JSON top-level fields, no new per-diagnostic fields.** The
existing `code`, `message`, `suggestion`, line/column span — all
already part of the v1.0-frozen `schemaVersion: "1"` contract — are
sufficient.

Stability commitment for v1.1:

- The **code identity** `D0084` is stable per the v1.0 SemVer policy
  (renames are breaking; removals require one-cycle deprecation).
- The **rule name** `enumValueMismatch` is stable per the same policy.
- The **suggestion convention** — populate `suggestion` with the best
  Levenshtein match — is stable.
- The **exact message wording** is NOT stable (consistent with v1.0's
  message-text-not-stable policy in `production-readiness.md`).

`D0084` is added to the v1.1 stability D-code list in the release
notes; `pykrete-tests` vendors the refreshed `diagnostic_catalog.json`
in the same release cycle via the existing drift-watch workflow
(`catalog-drift-watch.yml`) described in
`schema-tracking-probes.md`.

### Q9. Branch-form expressions (`coalesce`, `when/otherwise`, `nvl`, `nullif`, `ifnull`).

**Decision: branch-form expressions check every literal arg against
the surrounding enum constraint AND preserve the constraint on the
output iff every branch preserves it.** This is the load-bearing
silent-bug catch the framing principle exists to defend. A
"drop-constraint-on-branch-form" path would let
`coalesce(col("status"), lit("shippd"))` slip through as plain
`string`, defeating the whole feature.

Concrete rule, in two parts:

1. **Literal checking at the call site.** Every `lit("...")`
   (string-literal) argument inside a branch-form expression whose
   output flows into an enum-typed sink is checked against the
   sink's vocabulary. Off-enum literal → fire `D0084` on the
   literal's source span. If the branch-form expression is not in
   a sink-bound position (e.g. assigned to a variable), the check
   defers to the eventual sink-bound site per Q6.
2. **Output constraint propagation.** Branch-form output vocabulary
   follows the same rule as Q4 Merge — no silent union or
   intersection across branches. Concretely:
   - If every branch is enum-typed AND every branch's vocabulary is
     set-equal to every other's, the output carries that vocabulary.
   - If any branch resolves to plain `string` (e.g.
     `coalesce(col("enum_col"), col("plain_string_col"))`), the
     output drops the constraint; the result type is plain `string`.
   - If branches are enum-typed but their vocabularies are NOT
     set-equal (one is a subset, partial overlap, or disjoint), fire
     `D0040 unionSchemaMismatch` — the same code Q4 Merge reuses for
     vocabulary conflict. Do not silently union or intersect; surface
     the conflict and let the schema author reconcile explicitly.

   For literal-checking purposes (part 1 above), a string-literal
   branch in a sink-bound branch-form expression inherits the sink's
   enum type — the same vocabulary used to decide preservation in
   part 2 is also the vocabulary the per-branch literal is checked
   against. A standalone branch-form expression not in a sink-bound
   position follows the Q6 deferred-check rule (the literal's check
   fires at the eventual sink site).

Operations covered (the exhaustive list — the impl PR hooks into
`crates/pykrete/src/operations/column_exprs.rs` for the standalone
functions and `crates/pykrete/src/operations/column_methods.rs` for
the `.when(...)` / `.otherwise(...)` methods):

| Operation | Literal-arg check | Output preserves constraint? |
|---|---|---|
| `F.coalesce(c1, c2, ..., cN)` | yes, per literal arg | iff every branch is enum-typed AND all branches share a set-equal vocabulary (else: drop if any branch is plain `string`; `D0040` if branches are enum-typed with non-set-equal vocabularies, per Q4 Merge) |
| `F.nvl(c1, c2)` | yes (same shape as `coalesce(c1, c2)`) | same rule as `coalesce` (Q4 Merge applies on non-set-equal branch vocabularies) |
| `F.ifnull(c1, c2)` | yes (alias of `nvl`) | same rule as `coalesce` |
| `F.nullif(c1, c2)` | yes — the second arg's literal is checked against the first arg's enum (the comparison can never match if off-enum, but it's still a bug) | preserve the first arg's constraint; output is `Nullable[enum[...]]` (the function can return null). Wrapping is idempotent — if the first arg is already `Nullable[enum[...]]`, no double-wrap occurs |
| `F.when(cond, value).when(...).otherwise(value)` chains | yes, per literal arg in every `value` position | iff every `value` is enum-typed AND all values share a set-equal vocabulary (else: drop or `D0040` per the `coalesce` rule above) |
| `.when(...).otherwise(...)` column-method form | same as `F.when` | same as `F.when` |
| `F.lit(...)` standalone | covered by Q6 (sink-bound check) | n/a |
| `F.greatest(c1, c2, ...)`, `F.least(...)` | per Q2 preserve-side note: **drop the constraint** | no (lexicographic ordering has no enum meaning) |

Worked example of the silent-bug shape this catches:

```python
class Order(Schema):
    id: long
    status: enum["pending", "shipped", "delivered", "cancelled"]


def fix_nulls(orders: DataFrame[Order]) -> DataFrame[Order]:
    # Fires D0084 on 'shippd' — coalesce flows into the 'status' sink
    # via withColumn, so the surrounding enum constraint is applied to
    # every literal branch.
    return orders.withColumn(
        "status", F.coalesce(col("status"), F.lit("shippd"))
    )
    #                                              ^^^^^^^^ D0084: 'shippd'
    #                                                      is not in the enum
    #                                                      vocabulary for
    #                                                      'status'. Did you
    #                                                      mean 'shipped'?
```

If both branches resolve to enum-typed columns with set-equal
vocabularies (e.g. both are `enum["pending", "shipped", ...]`), the
output type is the same enum and downstream uses continue to check
against the vocabulary. If branches' vocabularies are non-set-equal,
`D0040` fires — branch-form output vocabulary follows the same rule
as Q4 Merge.

### Q9a. `Nullable[enum[...]]` interactions.

**Decision: `Nullable` is an orthogonal wrapper around `enum[...]`;
the enum vocabulary applies to non-null values, and null is accepted
or rejected per the existing `D0083 nullabilityMismatch` rule.**

Concrete cases:

- `status: Nullable[enum["a", "b"]]` — `lit(None)` accepted; `lit("a")`
  and `lit("b")` accepted; `lit("c")` fires `D0084`.
- `status: enum["a", "b"]` (non-nullable) — `lit(None)` fires `D0083`
  (not `D0084`); null isn't an enum-vocabulary question.
- `first(col("nullable_enum_status"))` — preserves the
  `Nullable[enum[...]]` wrapper (the aggregation may return null if
  no rows; the value, when present, is in the vocabulary).
- `coalesce(col("nullable_enum_status"), F.lit("a"))` — output is
  `enum["a", "b"]` (non-nullable; `coalesce` strips the null
  possibility), constraint preserved iff `"a"` is in the vocabulary
  (otherwise `D0084` fires on `"a"`).
- `min(col("nullable_enum_status"))` / `max(...)` — same: preserve
  `Nullable[enum[...]]`.

The constraint and the nullability are independent: `Nullable[enum[X,
Y]]` is the type "either null or a value in {X, Y}", not "null is in
the vocabulary."

## Worked example

A complete `.pyk` file demonstrating five distinct call sites where
`D0084` fires. Each diagnostic shows the rendered editor surface,
matching pykrete's standard format.

```python
from pykrete import Schema, DataFrame, col, lit
from pyspark.sql import functions as F


class Order(Schema):
    id: long
    status: enum["pending", "shipped", "delivered", "cancelled"]
    region: enum["us-east", "us-west", "eu-central"]


def stale_orders(orders: DataFrame[Order]) -> DataFrame[Order]:
    # 1. Typo in `==` RHS — D0084 fires on the literal.
    a = orders.filter(col("status") == "pendig")
    #                                    ^^^^^^^ D0084: 'pendig' is not in the
    #                                            enum vocabulary for 'status'.
    #                                            Did you mean 'pending'?

    # 2. Typo in `.isin(...)` — D0084 fires on the off-enum entry.
    b = orders.filter(col("status").isin("pending", "shippd"))
    #                                                ^^^^^^^^ D0084: 'shippd' is
    #                                                        not in the enum
    #                                                        vocabulary for
    #                                                        'status'. Did you
    #                                                        mean 'shipped'?

    # 3. Off-enum in `withColumn(..., lit())`.
    c = b.withColumn("status", lit("delivered_typo"))
    #                              ^^^^^^^^^^^^^^^^ D0084: 'delivered_typo' is
    #                                              not in the enum vocabulary
    #                                              for 'status'. Did you mean
    #                                              'delivered'?

    # 4. Off-enum in `.fillna({...})`.
    d = c.fillna({"status": "unkown"})
    #                       ^^^^^^^^ D0084: 'unkown' is not in the enum
    #                                vocabulary for 'status'. (no close match)

    # 5. Off-enum in `F.expr("...")` SQL fragment with `=`.
    e = d.filter(F.expr("region = 'eu-centarl'"))
    #                              ^^^^^^^^^^^^ D0084: 'eu-centarl' is not in
    #                                          the enum vocabulary for
    #                                          'region'. Did you mean
    #                                          'eu-central'?

    # 6. Off-enum in `F.expr("...")` SQL IN clause (Q7 IN-form).
    f = e.filter(F.expr("region IN ('us-east', 'us-wast')"))
    #                                          ^^^^^^^^^ D0084: 'us-wast' is
    #                                                   not in the enum
    #                                                   vocabulary for
    #                                                   'region'. Did you mean
    #                                                   'us-west'?

    # 7. Off-enum literal inside a `coalesce(...)` branch flowing into
    #    the enum-typed `status` sink (Q9 branch-form rule).
    g = f.withColumn("status", F.coalesce(col("status"), F.lit("shippd")))
    #                                                          ^^^^^^^^ D0084:
    #                                                          'shippd' is not
    #                                                          in the enum
    #                                                          vocabulary for
    #                                                          'status'. Did
    #                                                          you mean
    #                                                          'shipped'?

    return g
```

Diagnostic rendering (CLI form, matching v1.0 contract):

```
orders.pyk:15:38 - error enumValueMismatch: 'pendig' is not in the enum vocabulary for 'status'. Did you mean 'pending'?
orders.pyk:22:52 - error enumValueMismatch: 'shippd' is not in the enum vocabulary for 'status'. Did you mean 'shipped'?
orders.pyk:30:35 - error enumValueMismatch: 'delivered_typo' is not in the enum vocabulary for 'status'. Did you mean 'delivered'?
orders.pyk:36:29 - error enumValueMismatch: 'unkown' is not in the enum vocabulary for 'status'.
orders.pyk:42:34 - error enumValueMismatch: 'eu-centarl' is not in the enum vocabulary for 'region'. Did you mean 'eu-central'?
orders.pyk:48:46 - error enumValueMismatch: 'us-wast' is not in the enum vocabulary for 'region'. Did you mean 'us-west'?
orders.pyk:55:60 - error enumValueMismatch: 'shippd' is not in the enum vocabulary for 'status'. Did you mean 'shipped'?
```

## Stability surface (v1.1 commitments)

This section parallels the v1.0 JSON output stability contract in
`docs-site/src/content/docs/about/production-readiness.md` and the
probes-spec stability section in `schema-tracking-probes.md`.

**Form A syntax — STABLE.**

- The keyword `enum` (lowercase) as a parametric type with
  `enum["a", "b", ...]` subscript form is the stable v1.1 surface.
- Adding an entry to the vocabulary in a schema is non-breaking for
  downstream consumers of the schema (existing assignments still
  type-check).
- Removing an entry is a schema-level breaking change (existing
  assignments may newly fire `D0084`), governed by the consumer's
  release process, not pykrete's.
- Changing the keyword (`enum` → `oneof`, etc.) or the bracket form
  (subscript → call) is breaking on pykrete's side and requires a
  major bump per the v1.0 SemVer policy.

**`D0084` D-code — STABLE.**

- The code identity `D0084` is stable. Renames require a major bump;
  removals require one-cycle deprecation per `schema-tracking-probes.md`'s
  D-code lifecycle policy.
- The rule name `enumValueMismatch` is stable.
- `D0084` appears in pykrete's `DIAGNOSTIC_CATALOG` and in the v1.1
  release's vendored `diagnostic_catalog.json` for the cross-codebase
  probes harness; once landed, removal follows the deprecation cycle.

**Suggestion convention — STABLE.**

- The `suggestion` field is populated with the closest Levenshtein
  match against the enum vocabulary when one exists within the
  shared distance threshold pykrete uses for `D0030`. That threshold
  is defined once in `crates/pykrete/src/schema.rs` alongside the
  existing `D0030` unknown-column suggestion logic
  (`std::cmp::max(1, target.len() / 3)`) and reused by `D0084`; the
  impl PR must not duplicate the constant. If `D0030`'s threshold
  changes in a later release, `D0084`'s changes with it — they move
  together by construction.
- **Tiebreaker on equidistant candidates: Unicode code-point order
  (Rust `str::cmp`).** When two or more vocabulary entries are at the
  same Levenshtein distance from the off-enum literal, the suggestion
  is the entry that sorts first under Rust's `str::cmp` (Unicode
  code-point order). This ordering is locale-independent and
  deterministic across machines — locale-aware collation would make
  suggestions vary by platform, which is incompatible with the
  stability contract. Chosen for determinism (independent of
  vocabulary declaration order, so reformatting the schema cannot
  shift the suggestion).
- The "Did you mean" pattern in the message text echoes the
  suggestion, when present. Same convention as `D0030`.

**Hover / completion / schema-export rendering — STABLE in contract,
threshold may shift.**

- The enum vocabulary IS surfaced in hover text for any `enum[...]`-typed
  column. Removing it from hover would silently regress the v1.1 trust
  surface; consumers may rely on the vocabulary being visible.
- Default render policy: full vocabulary inline if the enum has 5 or
  fewer entries (`enum["a", "b", "c"]`); truncated with a tail
  `, ... (N more)` suffix above that (`enum["a", "b", "c", "d", "e",
  ... (3 more)]`). The truncation threshold (5) MAY shift in minor
  versions if user feedback suggests a different cap reads better;
  the **contract that the vocabulary appears in hover is fixed**, the
  exact cut-off is not.
- Same render policy applies to completion item details and to
  schema-export sites where pykrete prints the column type.
- Snapshot tests that capture the exact hover render are at
  **message-level** (not stable); they may need refresh when the
  truncation threshold (5) is retuned. The contract guarantees the
  vocabulary appears, not the exact byte sequence of the rendered
  string.

**NOT STABLE (per the v1.0 message-text carve-out):**

- The exact message wording (`'pendig' is not in the enum vocabulary
  for 'status'. Did you mean 'pending'?`) is not stable — pykrete
  reserves the right to refine the phrasing in minor releases.
- The Levenshtein numeric threshold itself is not a stability contract
  — only the "share with `D0030`" relationship is. If `D0030`'s
  threshold is retuned, both codes move together; consumers should
  not pin to a specific edit distance.
- The truncation threshold for hover rendering (5 entries) is not
  pinned; the contract is that the vocabulary appears, not the exact
  cut-off.

## Cost estimate (refined from the design tracker)

Re-estimated now that the call sites are mapped. Lines reference
`crates/pykrete/src/`.

| Slice | Days | Notes |
|---|---|---|
| Schema parser extension (`types.rs::from_name`) | 1 | Adds `enum[...]` recognition in the type-expression parser, plus empty-vocabulary and duplicate-entry rejection via `D0011`. |
| `ColumnType` representation update | 0.5 | One variant change in `types.rs`. Cost depends on whether the impl PR picks the variant or sidecar route (variant is the light recommendation); pattern-match churn flows through fallthrough for sites that don't care about the constraint. |
| Type-machinery carry-through pattern-match audit | 1.5 | Pattern-match sweep across `operations/`. Most call sites already flow through unchanged (column refs, schema-operators). Targeted edits: the string-producing ops enumerated in the `column_exprs.rs` string-returning match arm (drop), the aggregations in Q5 (preserve), the branch-form expressions in Q9 (check + propagate). |
| Check sites: `==` literal RHS + `D0082` precedence | 0.5 | Hook into `operations/strict_operators.rs` (already handles cross-type comparison). The Q1c precedence rule means `D0082` short-circuits the enum sub-check; no new code paths needed for the collision case. |
| Check sites: `.isin(...)` | 0.5 | Single addition in `operations/column_methods.rs`. |
| Check sites: `.fillna({...})` dict values | 0.5 | `operations/column_methods.rs` fillna branch. |
| Check sites: `lit(...)` into enum sink (`withColumn`, `.fill`, etc.) | 0.5 | Carry sink context through; check at the sink. |
| Q9 branch-form expressions (`coalesce` / `when` / `nvl` / `nullif` / `ifnull`) | 1 | Hook into `operations/column_exprs.rs` for the function form and `operations/column_methods.rs` for `.when(...).otherwise(...)`. Per-arg literal check + output constraint propagation. |
| `sql.rs` parser extension for `F.expr` | 1 | Extends `column_refs` walk to capture `<ident> = '<lit>'` and `IN (...)` shapes. |
| Schema operators carry-through (`Pick`/`Omit`/`Merge`) + `D0040` on conflict | 0.5 | Hook in `schema.rs` (existing operator implementation site). Set-equality semantics per Q4. |
| Hover / completion / schema-export render policy | 0.5 | Truncate-to-5 default render; reused across hover, completion-item details, schema-export. |
| Diagnostic catalog entry + snapshot test | 0.25 | One line in `DIAGNOSTIC_CATALOG`, one snapshot fixture. |
| Tests + diagnostic catalog + snapshots | 1 | One snapshot fixture per worked-example call site + per Q4 case + per Q5 carry-through case + per Q9 branch-form case. |
| Docs (types reference + diagnostics page) | 0.5 | New section in types.md, new D0084 entry in diagnostics.md. |
| Trust-surface docs (production-readiness.md, README.md) | 0.25 | Release-blocker per the v1.1 work plan step 6. |
| Cross-codebase fixtures (donor enums) | 1 | Hudi `_hoodie_operation`, Delta CDC `_change_type`, MLflow run states. |

**Total: ~11 days.** Higher than the round-1 9-day estimate because
Q9 (branch-form expressions) is now a first-class line item rather
than a silent fall-through hole, and the trust-surface docs are a
named release-blocker rather than a sub-bullet.

## Multi-lens review plan

Mirroring the pattern set by the probes spec PR (#71). The round-1
synthesis review (2026-06-01) applied all three lenses and surfaced
two blockers + eleven important + nine minor items; the round-2
revision in this PR addressed them (Q9 added, four impl-PR judgment
calls promoted, Q1a/Q1b/Q1c/Q9a/Q4 set-equality settled). The
round-2 synthesis review then surfaced one further blocker (Q9
output-vocabulary contradiction) and five important items (worked
example #7 internal breakage, Q6/Q9 bridge sentence, D0030
precedence, Unicode-code-point tiebreaker pin, brittle line-number
citations), all closed in round 3. The plan below is preserved for
follow-up reviewers verifying the round-3 tightening:

1. **Correctness lens** — does Form A actually parse cleanly in PEP
   526 annotation position? Does the carry-through table in Q5 match
   Spark's actual aggregation semantics? Is Q2's source-of-truth
   reference to the string-returning `match` arm in `column_exprs.rs`
   correctly pointing at every constraint-dropping op? Does the
   SQL-parser extension in Q7 cover the common shapes without false
   positives on backtick/quote variants? Does Q9's branch-form table
   cover every Spark conditional/null-handling op?
2. **Adversarial lens** — hunt for edge cases the framing principle
   doesn't cover. Round 2 settled: `enum[]` (rejected via `D0011`),
   `enum["a", "a"]` (rejected via `D0011`), Unicode/whitespace/case
   sensitivity (Q1a, byte-exact), `enum["true", "false"]` colliding
   with boolean coercion (Q1c, `D0082` wins), `Nullable[enum[...]]`
   interactions (Q9a), column-to-column assignment and cross-column
   compare (Q1b), branch-form silent-drop (Q9). New round-2 probes:
   chained branch-forms (`coalesce(when(...).otherwise(lit), lit)`)
   — should compose cleanly via Q9's "iff every branch preserves"
   rule; the impl PR confirms.
3. **Schema-stability lens** — does this addition preserve the v1.0
   JSON contract? Is `D0084` the right slot? Round 2 added explicit
   stability commitments for hover rendering (contract stable;
   threshold may shift), Levenshtein threshold sharing with `D0030`,
   and the lexicographic-tiebreaker rule. Reviewer confirms these
   are correctly scoped (no over-pinning of implementation details
   that the impl PR may need to retune).

Reviewers should explicitly call out which lens(es) they applied. If
all three return "no blocker," the spec is ready for the impl PR.

## What we explicitly will NOT do

Same bright line as the design tracker. These all require row-by-row
evaluation at runtime; pykrete is a development-time checker:

- Numeric `min`/`max` on column values
- Date / timestamp range constraints
- Regex pattern constraints on column values
- NOT NULL at row level (`Nullable[T]` already handles the type layer)
- Foreign-key / referential-integrity checks
- Multi-column row-level invariants
- Runtime validation of `string`-to-`enum` casts (per Q3)

These are out of scope **permanently** under the framing principle, not
just deferred. Adopting them would either degrade pykrete into a
no-op-at-runtime validation library (the worst of both worlds) or
require a runtime component pykrete has decided not to ship.

## Spec-level decisions promoted from round-1 impl-PR list

Round 1 of the synthesis review (2026-06-01) noted that four items
previously parked as "impl-PR judgment calls" actually decide
user-visible behaviour. They are settled here, not left to the impl
PR:

- **Empty vocabulary `enum[]`**: **rejected at parse time** via the
  existing `D0011 invalidColumnType` code, with a message clarifying
  that an enum's vocabulary must be non-empty. Rationale: an empty
  enum has no use case; every assignment would fire `D0084`
  vacuously, which is worse than a clear up-front error.
- **Duplicate vocabulary entries `enum["a", "a"]`**: **rejected at
  parse time** via `D0011` with a message that names the duplicated
  value. Same rationale; a duplicate offers no expressive power and
  most likely signals a copy-paste bug.
- **Reuse of `D0011` for both empty-enum and duplicate-entries is a
  deliberate code-reuse choice**, not a stability constraint. Both
  cases are "the parser refused to construct a valid enum type" — the
  same conceptual failure mode `D0011 invalidColumnType` already
  models. A future v1.2 may split these into dedicated codes
  (`D008X enumEmpty`, `D008Y enumDuplicate`) for finer diagnostic
  routing; that's an additive change under the v1.0 SemVer policy
  (new codes are non-breaking), not a v1.1 commitment.
- **Hover / completion / schema-export rendering**: settled in the
  Stability surface section above. The vocabulary appears in hover
  for every `enum[...]`-typed column; the default truncation
  threshold is 5 entries (renders the full list inline below the
  cap, appends a `, ... (N more)` tail above it). The contract is
  STABLE; the exact cut-off is not.
- **Levenshtein distance threshold for `D0084` suggestions**: shared
  with `D0030` via the existing constant in
  `crates/pykrete/src/schema.rs`, defined alongside the `D0030`
  unknown-column suggestion logic. Impl PR confirms the constant is
  referenced, not duplicated. Tiebreaker on equidistant candidates is
  **Unicode code-point order (Rust `str::cmp`)**, per the Stability
  surface section above — locale-independent and deterministic.

## Open questions remaining for the impl PR

All **design** questions are settled. The only outstanding choice is
genuinely impl-internal:

1. **Where to store the value-set in the AST**: as a new
   `ColumnType::Enum { values: Vec<String> }` variant, or as an
   optional sidecar field on `ColumnType::String`. **Light recommendation:
   the variant.** A dedicated variant gets compiler-enforced
   exhaustiveness across the existing `match ColumnType` sites, which
   matters when adding carry-through behaviour. The sidecar form is
   defensible if the pattern-match churn proves expensive in practice
   — count grep hits before committing. This is the only call that
   genuinely doesn't surface to users; the impl PR picks.

## v1.1 work plan

1. **This spec PR.** Settle the 9 questions (Q1-Q9), lock Form A,
   reserve `D0084`. No code.
2. **Multi-lens review** on this spec (correctness / adversarial /
   schema-stability — see above).
3. **Implementation PR** in pykrete-core. Covers schema parser,
   type machinery, check sites (including the Q9 branch-form
   expressions), SQL parser extension, snapshot tests, diagnostic
   catalog entry.
4. **Catalog drift refresh** in `pykrete-tests` (mechanical PR from
   the scheduled `catalog-drift-watch.yml` workflow). Vendors the
   refreshed `diagnostic_catalog.json` with `D0084` included.
5. **Cross-codebase fixtures.** Find 2-3 donor codebases that
   explicitly model enum-shaped string columns (Hudi
   `_hoodie_operation`, Delta CDC `_change_type`, MLflow run states
   are candidates) and write fixtures that exercise `D0084` —
   both positive (clean code that should not fire) and negative
   (synthetic typos in `probes_negative/` that must fire).
6. **RELEASE-BLOCKER: trust-surface docs update.** Before the v1.1.0
   tag, both of the following MUST land atomically with the impl PR
   (not a follow-up):
   - `docs-site/src/content/docs/about/production-readiness.md` —
     add `D0084` to the stable-D-code enumeration in the "Diagnostic
     codes" bullet, and add a "Reliability and trust" paragraph
     noting enum-value verification as a v1.1 capability.
   - `README.md` — add enum-value checking to the headline feature
     list with the same rigor language used for v1.0 D-codes.

   These are NOT cosmetic. Pykrete sells trust; advertising a feature
   in the live release without updating the stable-code surface is
   the "ship undelivered promises" antipattern the trust-first
   principle exists to prevent. This step is named (not a bullet
   under another step) and gates the tag.

## Migration story for existing schemas

Real codebases will have `status: string` declarations today. The
migration to `status: enum["pending", "shipped", ...]` is intentionally
a narrowing change:

- Existing assignments `withColumn("status", lit("pending"))` continue
  to type-check (the literal is in the new vocabulary).
- Existing assignments with off-vocabulary literals NEWLY fire
  `D0084`. This is the desired outcome — those are the bugs the
  feature is designed to catch.
- The migration is per-column, not all-or-nothing. A schema can mix
  `string` and `enum[...]` fields freely; only declared-enum columns
  participate in vocabulary checking.
- Recommended rollout for production codebases: narrow one column at
  a time, run the checker, fix the surfaced sites, commit. Pykrete's
  edit-time loop makes this a tractable Tuesday-afternoon exercise
  rather than a multi-day migration.

The reverse migration (`enum[...]` → `string`) is unconstrained on
pykrete's side — widening the type drops the constraint and existing
code continues to check. The schema's downstream consumers are the
ones who care; that's a consumer-side release-process question, not
a pykrete one.

## v1.1-polish backlog (post-PR-A follow-ups)

Five non-blocking minor items surfaced during PR-A review rounds. None
force a PR-B/C/D retrofit; addressing them as a single polish PR (or
bundled into PR-D's atomic doc migration) keeps the rough edges from
piling up.

- [ ] **Composite-wrapper error threading**: `Array[enum["a","a"]]`,
  `Map[..., enum["a","a"]]`, `Array[Optional[enum["a","a"]]]` all
  collapse to generic D0011 — `resolve_annotation_type`'s recursive
  `?` swallows the specific `Invalid`. Only top-level + single
  `Optional` wrapper preserves the duplicate-value cite today.
- [ ] **`EnumParseError::NonStringLiteral` payload**: variant carries
  no payload, so D0011 cannot cite the offending entry. Asymmetric
  with `Duplicate(String)`. Add the offending-token text to the
  variant + message.
- [ ] **`Optional[Optional[enum[...]]]` rejection**: `parse_enum_annotation`
  silently flattens double-wrapped `Optional` into `Nullable(Nullable(Enum))`.
  Functionally harmless; structurally suspicious. Reject in
  `resolve_annotation_type` with a clear message.
- [ ] **Pick/Omit assertions via `enum_vocab_eq`**: round-2 vacuity
  tests use derived `==` which happens to work because Vec order
  is preserved by construction. Switch to `enum_vocab_eq` to
  assert the spec-level set-equality contract instead of the
  implementation detail.
- [ ] **Enum-in-aggregates regression test**: behavior is correct
  (`aggregates_on_non_numeric_input_agree_between_paths` falls into
  `_ => None` for Enum) but not locked in by a regression test. Add
  one that explicitly mentions `ColumnType::Enum` in the rejection set.

### PR-B round-3 polish additions

- [ ] **Extended `Nullable[enum]` integration coverage**: round-2
  Nullable peel is covered on `==`, `withColumn(lit)`, `fillna(dict)`,
  and `F.expr` equality. Add positive tests for the remaining shapes:
  `.isin(...)`, `.fillna(scalar)`, `withColumns(dict)`, and branch-form
  reconciliation (`coalesce` / `when().otherwise()`) where one or more
  branches carry `Nullable(Enum)` wrappers.
- [ ] **fillna / withColumn unknown-column precedence test**:
  pin down that `D0030` (unknown column) wins over `D0084` when both
  could fire on `fillna({"unknown": "lit"})` or `withColumn("unknown",
  lit("x"))` against an enum-typed schema. Today the precedence is
  implicit in the walker order; a regression test would lock it in.
- [ ] **Q1b cross-column compare**: `col(enum_a) == col(enum_b)` with
  disjoint vocabularies — should fire D0040 (or a paired diagnostic)
  by symmetry with the branch-form Q9 case. Out of PR-B scope; needs
  spec decision on whether the same code or a distinct one is right.
- [ ] **`col_reference` shape coverage**: PR-B only resolves
  `col("name")` / `column("name")` to an enum-typed column. `df.name`
  attribute access, `df["name"]` subscript, and SQL-style aliases all
  bypass the enum lookup today — pre-existing limitation, but a test
  pinning down "no diagnostic, by design" would prevent silent
  regressions when those resolution paths grow enum awareness.
- [ ] **D0040 cross-call dedupe**: round-3 dedupes by start
  line/column when the same `Expr::Call` chain is visited from outer
  and inner descents. Approach is correct (chains anchor on their
  root call, so spans share a start) but the check scans the full
  diagnostics buffer linearly per emission. Re-evaluate if profiling
  surfaces it on large files; a per-pass HashSet would be O(1) per
  emission.

## Related

- `feedback_trust_is_core_value_prop` — the trust-first principle this
  feature reinforces (catches silent-empty-DataFrame bugs).
- `schema-tracking-probes.md` — sibling v1.1 spec; once `D0084` lands,
  `PROBE-EXPECTS: D0084` becomes a probe target the cross-codebase
  fixtures use.
- `spark-coverage.md` — the v1.1 follow-up section names enum
  constraints as a Spark-coverage extension; this spec is its
  detailed shape.
- v1.0 JSON output contract — `docs-site/src/content/docs/about/production-readiness.md`.
