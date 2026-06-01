# Literal value vocabulary — enum constraints (v1.1 spec)

**Status**: settled spec — implementation PR may pick up.
**Origin**: 2026-05-31 user proposal during the pre-v1.0.0 sprint;
promoted from design tracker on 2026-06-01 after the 8 open questions
were resolved.
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

## Settled questions (8 of 8 resolved)

Each question below was identified during the design tracker phase.
Each now has a concrete decision an implementation PR can encode
without further design discussion.

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

### Q2. String-producing operations drop the constraint.

**Decision: explicitly documented; result type is plain `string`.** The
following operations, all of which produce strings that may or may not
be in the enum, **drop the enum constraint** from the result type:

- `.substr(start, length)`
- `.regexp_replace(pattern, replacement)`
- `.regexp_extract(pattern, idx)`
- `.concat(...)`, `F.concat(...)`, `F.concat_ws(...)`
- `.lower()`, `F.lower(col)`
- `.upper()`, `F.upper(col)`
- `.translate(matching, replace)`
- `.trim()`, `.ltrim()`, `.rtrim()`, `F.trim(col)` etc.
- `.lpad(len, pad)`, `.rpad(len, pad)`
- `.replace(pat, repl)`, `F.replace(col, ...)`
- `.cast("string")` (cast TO string from any other type)
- `F.format_string(...)`, `F.format_number(...)`
- `.split(pat, limit)` (returns `array<string>` — array element drops constraint)
- Any other string-returning function not explicitly enumerated below

Documented in `docs-site/src/content/docs/reference/types.md` (added
alongside the implementation PR) so users aren't surprised when
chaining a transform "loses" the enum constraint.

Implementation note for the impl PR: the result type of these ops is
`ColumnType::String` (no constraint carry-through); this is the
default fall-through path — only operations explicitly listed in Q5
preserve the constraint.

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
  - If `A.status` is `enum[X, Y]` and `B` does not declare `status`,
    the merged column carries `enum[X, Y]`.
  - If `A.status` and `B.status` both declare `enum[...]` with
    **identical** value sets, the merged column carries that set.
  - If `A.status` and `B.status` declare **different** value sets,
    fire `D0040 unionSchemaMismatch` (the existing code for this
    class of conflict) with a message that names both value sets.
    Do NOT silently union or intersect the sets — the user did not
    sign up for either interpretation; surface the conflict.
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
| `first(col)` | Yes | `enum[...]` (same as input) |
| `last(col)` | Yes | `enum[...]` |
| `min(col)` | Yes | `enum[...]` (lexicographic min, still in vocabulary) |
| `max(col)` | Yes | `enum[...]` |
| `collect_set(col)` | Yes | `array<enum[...]>` |
| `collect_list(col)` | Yes | `array<enum[...]>` |
| `count(col)` | N/A | `long` (returns count, never the value) |
| `count_distinct(col)` | N/A | `long` |
| `countDistinct(col)` | N/A | `long` |
| `sum(col)` | N/A | already fires `D0081 nonNumericArithmetic` on string |
| `avg(col)` / `mean(col)` | N/A | already fires `D0081` on string |
| `stddev(col)` / `variance(col)` etc. | N/A | already fires `D0081` on string |

The "carries" cases are operations that emit a value drawn from the
input column — by construction, the output value is still in the
input vocabulary. The "N/A" cases either produce a numeric (count) or
are already rejected by existing rules on string columns.

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
  `DIAGNOSTIC_CATALOG` in `crates/pykrete/src/diagnostics.rs`
  at the position immediately after the current
  `("D0083", "nullabilityMismatch")` entry (line 198).

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

    # 5. Off-enum in `F.expr("...")` SQL fragment.
    e = d.filter(F.expr("region = 'eu-centarl'"))
    #                              ^^^^^^^^^^^^ D0084: 'eu-centarl' is not in
    #                                          the enum vocabulary for
    #                                          'region'. Did you mean
    #                                          'eu-central'?

    return e
```

Diagnostic rendering (CLI form, matching v1.0 contract):

```
orders.pyk:15:38 - error enumValueMismatch: 'pendig' is not in the enum vocabulary for 'status'. Did you mean 'pending'?
orders.pyk:22:52 - error enumValueMismatch: 'shippd' is not in the enum vocabulary for 'status'. Did you mean 'shipped'?
orders.pyk:30:35 - error enumValueMismatch: 'delivered_typo' is not in the enum vocabulary for 'status'. Did you mean 'delivered'?
orders.pyk:36:29 - error enumValueMismatch: 'unkown' is not in the enum vocabulary for 'status'.
orders.pyk:42:34 - error enumValueMismatch: 'eu-centarl' is not in the enum vocabulary for 'region'. Did you mean 'eu-central'?
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
  match against the enum vocabulary when one exists within the same
  distance threshold pykrete uses for `D0030`.
- The "Did you mean" pattern in the message text echoes the
  suggestion, when present. Same convention as `D0030`.

**NOT STABLE (per the v1.0 message-text carve-out):**

- The exact message wording (`'pendig' is not in the enum vocabulary
  for 'status'. Did you mean 'pending'?`) is not stable — pykrete
  reserves the right to refine the phrasing in minor releases.
- The Levenshtein threshold itself is an implementation detail; the
  contract is "suggest when a close match exists", not a fixed
  numeric distance.

## Cost estimate (refined from the design tracker)

Re-estimated now that the call sites are mapped. Lines reference
`crates/pykrete/src/`.

| Slice | Days | Notes |
|---|---|---|
| Schema parser extension (`types.rs::from_name`, ~2 sites) | 1 | Adds `enum[...]` recognition in the type-expression parser. Value-set carried on a new `ColumnType` variant or as a sidecar field on `String` — impl PR picks. |
| `ColumnType` representation update | 0.5 | One variant change in `types.rs:11`. Touches 17 grep-matched pattern-match sites; most flow through unchanged via base-type fallthrough. |
| Type-machinery carry-through pattern-match audit | 1.5 | Pattern-match exhaustiveness sweep across `operations/` (10 files, ~17 `ColumnType::String` match sites). Most call sites already flow through unchanged (column refs, schema-operators). Targeted edits: the string-producing ops listed in Q2 (drop), the aggregations in Q5 (preserve). |
| Check sites: `==` literal RHS | 0.5 | Hook into `operations/strict_operators.rs` near line 122 (already handles cross-type comparison). |
| Check sites: `.isin(...)` | 0.5 | Single addition in `operations/column_methods.rs`. |
| Check sites: `.fillna({...})` dict values | 0.5 | `operations/column_methods.rs` fillna branch. |
| Check sites: `lit(...)` into enum sink (`withColumn`, `.fill`, etc.) | 0.5 | Carry sink context through; check at the sink. |
| `sql.rs` parser extension for `F.expr` | 1 | Extends `column_refs` walk to capture `<ident> = '<lit>'` and `IN (...)` shapes. |
| Schema operators carry-through (`Pick`/`Omit`/`Merge`) + `D0040` on conflict | 0.5 | Hook in `schema.rs:768+`. |
| Diagnostic catalog entry + snapshot test | 0.25 | One line in `DIAGNOSTIC_CATALOG`, one snapshot fixture. |
| Tests + diagnostic catalog + snapshots | 1 | One snapshot fixture per worked-example call site + per Q4 case + per Q5 carry-through case. |
| Docs (types reference + diagnostics page) | 0.5 | New section in types.md, new D0084 entry in diagnostics.md. |
| Cross-codebase fixtures (donor enums) | 1 | Hudi `_hoodie_operation`, Delta CDC `_change_type`, MLflow run states. |

**Total: ~9 days.** Slightly higher than the original 5-9 day estimate
because the SQL-parser extension (Q7) and the Pick/Omit/Merge
unification rules (Q4) are concrete spec items now rather than
hand-waved as "consistency requires it."

## Multi-lens review plan

Mirroring the pattern set by the probes spec PR (#71). When this spec
PR is in review, the reviewers should apply at least three lenses:

1. **Correctness lens** — does Form A actually parse cleanly in PEP
   526 annotation position? Does the carry-through table in Q5 match
   Spark's actual aggregation semantics? Are the Q2 string-producing
   ops listed exhaustively? Does the SQL-parser extension in Q7 cover
   the common shapes without false positives on backtick/quote
   variants?
2. **Adversarial lens** — hunt for edge cases the framing principle
   doesn't cover. What about `enum[]` (empty vocabulary)? `enum["a",
   "a"]` (duplicates)? Unicode literals? `enum["true", "false"]`
   colliding with boolean coercion? `Nullable[enum[...]]` interactions?
   Chained `.cast(...)` round-trip from `enum[...]` → `string` →
   should `.cast("enum[...]")` work? (Answer per Q3: no.)
3. **Schema-stability lens** — does this addition preserve the v1.0
   JSON contract? Is `D0084` the right slot? Should the catalog entry
   land at end-of-list or maintain numeric ordering? Are the
   stability commitments correctly scoped (code identity stable;
   message wording not stable)?

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

## Open questions remaining for the impl PR

All **design** questions are settled. The following are **impl-time**
choices the implementation PR makes — they don't affect the user-
facing surface and don't need spec-level review:

1. **Where to store the value-set in the AST**: as a new
   `ColumnType::Enum { values: Vec<String> }` variant, or as an
   optional sidecar field `String(Option<EnumConstraint>)`. The
   reviewer-side rule: whichever yields fewer churned pattern-match
   sites in `operations/` (count grep hits before deciding).
2. **How to render in hover text**: full vocabulary inline (cluttery
   for big enums), or truncated with "...". Recommendation: truncate
   to 5 entries with `...` if more, full list available via
   `pykrete check --explain` (if/when that lands; not a v1.1
   requirement).
3. **Levenshtein distance threshold for the `D0084` suggestion**:
   reuse the same threshold as `D0030`. Impl PR confirms the constant
   is shared, not duplicated.
4. **Whether to short-circuit on `enum[]` (empty vocabulary)**:
   either reject at parse time (`D0011 invalidColumnType` — "enum
   vocabulary must be non-empty") or allow and treat every literal
   as off-enum. Recommendation: reject at parse time; empty enums
   have no use case and silently rejecting every assignment is
   worse than a clear up-front error.
5. **Duplicate vocabulary entries (`enum["a", "a"]`)**: reject at
   parse time with a clear message via `D0011`. Same rationale as
   empty vocabulary.

These are all impl-PR judgment calls; none change the spec.

## v1.1 work plan

1. **This spec PR.** Settle the 8 questions, lock Form A, reserve
   `D0084`. No code.
2. **Multi-lens review** on this spec (correctness / adversarial /
   schema-stability — see above).
3. **Implementation PR** in pykrete-core. Covers schema parser,
   type machinery, check sites, SQL parser extension, snapshot
   tests, diagnostic catalog entry.
4. **Catalog drift refresh** in `pykrete-tests` (mechanical PR from
   the scheduled `catalog-drift-watch.yml` workflow). Vendors the
   refreshed `diagnostic_catalog.json` with `D0084` included.
5. **Cross-codebase fixtures.** Find 2-3 donor codebases that
   explicitly model enum-shaped string columns (Hudi
   `_hoodie_operation`, Delta CDC `_change_type`, MLflow run states
   are candidates) and write fixtures that exercise `D0084` —
   both positive (clean code that should not fire) and negative
   (synthetic typos in `probes_negative/` that must fire).
6. **README + Production Readiness update.** Add a one-paragraph
   entry in the "Reliability and trust" section noting enum-value
   checking as a v1.1 capability shipping with the same audit-cycle
   rigor as v1.0. Add `D0084` to the diagnostic catalog table on the
   docs site.

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
