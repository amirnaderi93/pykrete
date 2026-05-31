# Literal value vocabulary (v1.1 design tracker)

**Status**: deferred from v1.0.0; planned for v1.1.
**Origin**: 2026-05-31 user proposal during pre-v1.0.0 sprint.

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
    #                                       ^^^^^^^ D00XX: typo? did you mean "pending"
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
| Enum literal `status == "pendig"` (typo) | ✅ Yes | literal vs literal — both known at edit time |
| Enum literal in `.isin("a", "b")` | ✅ Yes | same |
| Enum literal in `.fillna({"status": "open"})` | ✅ Yes | same |
| `amount > 0` runtime row value | ❌ No | row value not knowable at edit time |
| `date BETWEEN '2024-01-01' AND today()` | ❌ No | row value not knowable |
| `name MATCHES /\d+/` runtime regex on row | ❌ No | same |
| NOT NULL on row | ❌ No | same |

Pykrete is a checker, not a validation library. The enum case is the
one shape where the constraint and the data the constraint applies to
are both source literals — uniquely suited to compile-time checking.

## Syntax sketch (to settle in the spec PR)

Three forms under consideration:

```python
# A: subscript on a built-in enum keyword
class Order(Schema):
    status: enum["pending", "shipped", "delivered", "cancelled"]

# B: parametric form mirroring decimal(p, s)
class Order(Schema):
    status: enum("pending", "shipped", "delivered", "cancelled")

# C: separate decorator-like annotation
class Order(Schema):
    status: string
    _status_values = ["pending", "shipped", "delivered", "cancelled"]
```

Form A reads cleanest and matches existing pykrete syntax patterns
(`decimal[18, 2]`, `Array[T]`, `Map[K, V]`). Form B has a subtle
parsing problem: PEP 526 annotations only accept expressions, and a
call `enum(...)` is a call expression that gets evaluated at runtime
(which pykrete is fine with, but readers might confuse with actual
runtime enum constructors). Form C splits the declaration in two,
which violates the "schema is a single declaration" ergonomics.

**Default recommendation: Form A.** Settle in the spec PR.

## Open design questions (settle in the spec PR before any code)

1. **Unification on `withColumn("status", lit("c"))` where "c" is not
   in the enum.** Two options:
   - (a) Fire a diagnostic (the user violated the declared constraint).
   - (b) Widen the output type to plain String, lose the constraint.

   Recommend (a). The whole point of declaring the enum is that
   off-enum values are bugs. Silent widening would defeat the purpose.

2. **String-producing operations drop the constraint.** `.substr(...)`,
   `.regexp_replace(...)`, `.concat(...)`, `.lower()/.upper()` etc.
   produce strings that may or may not be in the enum. Result type:
   plain String. Document explicitly so users aren't surprised when
   chaining a transform "loses" the enum.

3. **Cast from arbitrary String to enum.** `.cast("enum[...]")`-style
   form not allowed; the only way to get an enum-typed column is via
   the schema declaration. Cast from `string` → `enum[...]` would
   require runtime validation, which is out of scope. Document.

4. **Schema operators carry the constraint.** `Pick[Order, "status"]`,
   `Omit[Order, "id"]`, `Merge[A, B]` all preserve the enum constraint
   on carried-through fields. Test explicitly.

5. **Aggregations and the constraint.** `first/last/min/max/collect_set`
   over an enum column produce a value (or set) that's still within
   the enum vocabulary — output type preserves the constraint.
   `groupBy("status").count()` produces a schema with `status` (still
   enum-constrained) + `count: long`. `sum/mean/avg` are nonsensical
   over a string column — pykrete already fires; nothing new here.

6. **`F.lit("...")` inside other expressions.** `F.lit("active")` going
   into an enum-typed sink — checked. `F.lit("active")` standalone
   without a sink — opaque, no enum context to check against. Fine.

7. **`F.expr("status = 'actiev'")` SQL fragment.** Pykrete's SQL parser
   in `sql.rs` already handles bare identifier column refs; should it
   also check string-literal RHS against an enum constraint? Probably
   yes (consistency), but adds parser surface. Settle in the spec PR.

8. **Implication for the JSON output contract.** New D-code (D0084 or
   the next available). New JSON field on diagnostics? Probably not —
   the message + suggestion fields already carry the "did you mean"
   payload. Add to the stability D-code list before v1.1.0 freezes.

## Cost estimate (rough, settle precisely in the spec PR)

- Schema parser extension: ~1 day
- Type machinery carry-through (every `ColumnType::String` pattern
  match site needs to handle the optional value-set): ~2-3 days
- Check sites (`==` literal, `.isin`, `.fillna` dict, `lit` into enum
  sink, the F.expr SQL parser): ~1-2 days
- Tests + docs + diagnostic catalog + snapshot: ~1-2 days
- Cross-codebase fixtures specifically targeting enum patterns: ~1 day

**Total: ~5-9 days.** Cheap relative to the value caught, not cheap
absolutely. Type-system surface is the riskiest part — pattern-match
exhaustiveness across the analyzer needs a careful sweep.

## What we explicitly will NOT do (until / unless pykrete grows a runtime)

- Numeric `min`/`max` on column values
- Date / timestamp range constraints
- Regex pattern constraints on column values
- NOT NULL at row level (`Nullable[T]` already handles the type layer)
- Foreign-key / referential-integrity checks
- Multi-column row-level invariants

These all require row-by-row evaluation at runtime. Pykrete is a
development-time checker; adopting them would either degrade us into a
no-op-at-runtime validation library (the worst of both worlds) or
require a runtime component we have decided not to ship.

## v1.1 work plan

1. **Spec PR first.** This file plus a settled syntax decision (Form A
   default), unification rules per the 8 questions above, the new
   D-code reservation. No code.
2. **Multi-lens review on the spec.** Same pattern that's been
   catching things in v0.1.x: correctness lens against the framing
   principle, adversarial lens hunting for edge cases the principle
   doesn't cover, schema-stability lens against the JSON contract.
3. **Implementation PR after spec is settled.**
4. **Cross-codebase fixtures.** Find 2-3 donor codebases that
   explicitly model enum-shaped string columns (Hudi
   `_hoodie_operation`, Delta CDC `_change_type`, MLflow run states
   are candidates) and write fixtures that exercise the new D-code.
5. **README update.** Add a one-paragraph entry in the "Reliability
   and trust" section noting the literal-value vocabulary as a v1.1
   capability that ships with the same audit-cycle rigor.

## Related

- [[feedback_trust_is_core_value_prop]] — the trust-first principle
  this feature reinforces (catches silent-empty-DataFrame bugs).
- The Spark coverage v1.1 follow-up section in this same directory
  (`spark-coverage.md`) — this tracker is a sibling.
