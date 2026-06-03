---
title: Schemas
description: Declare dataframe shapes as Python classes — atomic types, nested arrays / maps / structs, and TypeScript-style type operators.
---

A schema is a Python class. pykrete reads its type-annotated attributes as the columns of a dataframe.

```python
class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int
```

`Sale` declares four columns. Field name is the column name; field type is the column type. That's the whole idea — everything below is detail.

## Atomic types

| pykrete | Spark / SQL |
|---|---|
| `int` | `int` |
| `long` | `bigint` |
| `short` | `smallint` |
| `byte` | `tinyint` |
| `double` | `double` |
| `decimal(p, s)` | `decimal(p, s)` |
| `string` | `string` |
| `binary` | `binary` |
| `bool` | `boolean` |
| `date` | `date` |
| `timestamp` | `timestamp` |

`decimal` accepts an optional `(precision, scale)`: write `decimal(18, 2)` for money, `decimal(38, 18)` for high precision, `decimal(p)` for a single-arg form (scale defaults to 0, matching Spark SQL), or `decimal` on its own (Spark's default `decimal(10, 0)`). Precision must fit `1..=38` (Spark's cap) and scale must not exceed precision; either violation fires `D0011`.

`numeric` and `dec` are accepted as Spark SQL aliases for `decimal`; the parameterized form (`numeric(18, 2)`, `dec(p, s)`) and the bare form (`numeric`, `dec`) resolve identically. Use whichever your team writes in Spark DDL — pykrete treats them as one type.

`float` is accepted as an alias for `double`. Python's `float` is what PySpark hands to Spark's runtime, which coerces it to `DoubleType` — declaring `amount: float` and `amount: double` produces the same column type, and either is interchangeable with the other under strict mode.

### Case sensitivity

Atomic names are **case-sensitive lowercase** in `.pyk` source: `int`, `string`, `decimal`. `Int`, `STRING`, `Decimal` are rejected (`D0010`). This matches Spark interop — column names already are case-sensitive (see [Why case-sensitive?](#why-case-sensitive)), and pykrete keeps the rule uniform across the type vocabulary.

Two complex-type keywords have legacy spelling carve-outs: both `Array[…]` and `array[…]` are accepted, and both `Map[…, …]` and `map[…, …]` are accepted — exactly those two casings each, not arbitrary case. `ARRAY` / `aRrAy` are rejected. There is no `Struct[…]` subscript form: typed struct columns are declared by typing the nested `Schema` class name as the field type (see [Struct columns](#struct-columns) below), and untyped/opaque structs use bare `Struct` (see [Opaque struct columns](#opaque-struct-columns--struct)). The element types inside `Array[…]` / `Map[…, …]` still follow the strict rule: `Array[Int]` is rejected because `Int` is not the atomic name; write `Array[int]`.

The wider Spark SQL vocabulary (`integer`, `bigint`, `smallint`, `tinyint`, `float`, `real`, `boolean`) is accepted only inside `.cast("…")` strings and string-form UDF return types, where Spark SQL itself is case-insensitive. Inside a Schema class body, stick to the lowercase pykrete names listed above.

## Enum-valued strings — `enum["a", "b", ...]`

A string column whose values are drawn from a fixed vocabulary — order
status, CDC operation kind, run state — is declared as
`enum["v1", "v2", ...]`:

```python
class Order(Schema):
    id: long
    status: enum["pending", "shipped", "delivered", "cancelled"]
```

`status` is still a Spark `string` at runtime; the `enum[...]` annotation
adds a static vocabulary check on string literals that flow into the
column. Comparing against an off-vocabulary value, filling with one, or
writing one in via `withColumn` fires `D0084 enumValueMismatch` at the
literal — with a *did you mean* suggestion when a close match exists:

```
orders.pyk:14:38 - error enumValueMismatch: 'shippd' is not in the enum vocabulary for 'status'. Did you mean 'shipped'?
```

The check fires across every sink-bound site we model: `col("status") ==
"shippd"`, `col("status").isin("pending", "shippd")`,
`.fillna({"status": "shippd"})`, `withColumn("status", lit("shippd"))`,
`F.expr("status = 'shippd'")` (and the SQL `IN (...)` form), and the
branch-form expressions `F.coalesce` / `F.when(...).otherwise(...)` /
`F.nvl` / `F.ifnull` / `F.nullif` when their output flows into an
enum-typed sink.

### Vocabulary semantics

- **Case- and whitespace-sensitive.** `enum["pending"]` rejects
  `"Pending"` and `"pending "` (trailing space). The vocabulary is the
  literal set the schema declares.
- **Full Unicode.** `enum["café", "naïve"]` is fine.
- **Set equality.** Order doesn't matter: `enum["a", "b"]` and
  `enum["b", "a"]` are the same type.

### Nullable enums

Wrap with `Nullable[...]` for an optional enum column. `Nullable[enum[...]]`
is the canonical optional shape:

```python
class Run(Schema):
    id: string
    status: Nullable[enum["RUNNING", "FINISHED", "FAILED", "KILLED", "SCHEDULED"]]
```

`Optional[...]` is accepted as an alias for `Nullable[...]` (same
semantics as elsewhere in the type vocabulary). A literal `None` /
`lit(None)` into a `Nullable[enum[...]]` sink is fine; into a bare
`enum[...]` sink it fires `D0083 nullabilityMismatch` under strict mode.

### Constraint preservation

The enum constraint flows through schema composition and structural
transforms:

- **`Pick` / `Omit`** preserve the constraint on every column carried
  through.
- **`Merge`** preserves the constraint when both sides agree (set-equal
  vocabularies); a non-set-equal collision fires `D0040
  unionSchemaMismatch` rather than silently union or intersect.
- **Aliases and renames** (`F.col("status").alias("s")`) preserve the
  constraint.
- **Per-value aggregations** that emit a value drawn from the input
  column — `first`, `last`, `min`, `max`, `collect_set`, `collect_list`
  — preserve the constraint.
- **Branch-form expressions** (`F.coalesce`, `F.when(...).otherwise(...)`,
  `F.nvl`, `F.ifnull`) preserve the constraint when every branch is
  enum-typed and shares a set-equal vocabulary; otherwise the output
  drops to plain `string` (or fires `D0040` on a non-set-equal
  enum-vs-enum mismatch).

The constraint is dropped on **string-producing operations** — `cast`,
`regexp_replace`, `regexp_extract`, `substring`, `substr`, `lower`,
`upper`, `initcap`, `trim`, `concat`, `concat_ws`, `format_string`, and
the rest of the string-transform family. The result type is plain
`string`; downstream literals are no longer vocabulary-checked. This is
deliberate — the transformed value may or may not still be in the
vocabulary, and a silent "yes" would be a worse signal than no check.

### v1.1 scope — `withColumn(name, lit(...))` literal check, sink only

In v1.1, `withColumn("status", lit("shipped"))` checks the literal
against the `status` sink's vocabulary, but the **output column** drops
the enum constraint to plain `string`. Downstream code that re-uses the
returned frame's `status` column won't see the vocabulary preserved.
The vocabulary check at the literal still fires at the write site,
which is where the bug lives in practice; preservation of the
constraint on the output column is tracked in the polish backlog.

### What you can't do

- **`.cast("enum[...]")`** is rejected (`D0011 invalidColumnType`).
  Schemas are the only way to introduce an enum-typed column —
  casting from arbitrary `string` would require runtime validation
  pykrete deliberately does not perform.
- **Empty vocabulary** (`enum[]`) is rejected — declare at least one
  value, or use plain `string`.

## Optional columns

Wrap a type in `Optional[...]` to mark the column nullable:

```python
class Customer(Schema):
    id: int
    email: Optional[string]
```

For column-existence checks, `Optional[T]` and `T` behave the same. The distinction matters under [strict type-checking](/pykrete/reference/configuration/#typecheckingmode), where a nullable value flowing into a non-nullable slot is flagged.

## Nested types

### Struct columns

A struct column is a field whose type is another `Schema` class:

```python
class Address(Schema):
    street: string
    city: string
    zip: string

class Customer(Schema):
    id: int
    address: Address
```

Dotted column references walk into the struct:

```python
def f(customers: SparkFrame[Customer]) -> SparkFrame:
    return customers.select(F.col("address.city"))
```

A typo on the path — `"address.cty"` — fires `D0030` and names the schema it failed on (`Address`), not the outer one.

### Array columns

```python
class Event(Schema):
    tags: Array[string]
    scores: Array[int]
```

For an array of structs, the element type is another `Schema`:

```python
class LineItem(Schema):
    sku: string
    qty: int

class Order(Schema):
    lines: Array[LineItem]
```

A dotted path pierces the array to its element type:

```python
orders.select(F.col("lines.sku"))   # checked against LineItem.sku
```

### Map columns

```python
class Telemetry(Schema):
    counters: Map[string, int]
    payload: Map[string, string]
```

pykrete doesn't walk into map values — the keys are runtime data, not part of the schema.

### Opaque struct columns — `Struct`

When a column carries a nested struct whose shape isn't worth modeling — third-party telemetry blobs, opaque metadata, anything the rest of the codebase treats as a black box — declare it as bare `Struct`:

```python
class Event(Schema):
    id: int
    payload: Struct
    tags: Array[Struct]
```

`Struct` parses as an opaque composite — the column counts as a struct (so `F.col("payload")` resolves), but pykrete won't try to verify inner-field navigation. `F.col("payload.something")` degrades silently rather than fire `D0030`. The same posture as bare `Array` / `Map` with no parameter: pykrete declines to guess. Use `Struct[…]` syntax doesn't exist — model fields you care about with a nested `Schema` class instead (see [Struct columns](#struct-columns)).

## Type operators

Compose schemas the way TypeScript composes object types, instead of redeclaring columns. The operators are `Pick`, `Omit`, and `Merge`.

### `Pick` — keep some columns

```python
class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int

class SaleSummary(Pick[Sale, "region", "amount"]):
    pass
# SaleSummary == { region: string, amount: int }
```

The kept columns appear in the order listed in the `Pick` arguments — useful when downstream code asserts column order. Naming a column that isn't on the base schema fires `D0030`.

### `Omit` — drop some columns

```python
class SaleLite(Omit[Sale, "product", "quantity"]):
    pass
# SaleLite == { region: string, amount: int }
```

Dropped columns are removed; the surviving columns keep their order from the base schema. Naming a column that isn't on the base schema fires `D0030`.

### `Merge` — combine two or more schemas

```python
class Region(Schema):
    region: string
    manager: string

class SaleWithManager(Merge[Sale, Region]):
    pass
# SaleWithManager has every column of Sale plus every column of Region
```

Use `Merge` to describe the shape of a join result, a `withColumns` extension, or any concatenation of schemas. When two operands declare the same column name, pykrete keeps the **first occurrence** in argument order and silently drops the later ones. (Note: this is the opposite of Python's `{**a, **b}` dict merge, where the later operand wins — pykrete's `Merge` is first-wins, not last-wins.) If you need a later operand's column to survive instead, reorder the arguments, or wrap the earlier operand in `Omit` to drop the column there.

These three operators are the full surface. Operators that appeared in earlier specs (`Join[A, B]`, `GroupBy[S, k]`) were dropped before v0.1 shipped — the join / groupBy result schemas are inferred from the call site instead, so a separate operator wasn't needed.

## Cross-file schemas

Put shared schemas in their own `.pyk` module and import them — pykrete resolves `.pyk` imports the same way Python does:

```python
# schemas.pyk
class Sale(Schema):
    region: string
    amount: int

# sales.pyk
from schemas import Sale

def revenue_by_region(sales: SparkFrame[Sale]) -> SparkFrame:
    return sales.groupBy("region").agg(F.sum("amount").alias("total"))
```

One schema, imported wherever the dataframe shows up. Schema modules need to be `.pyk` (not `.py`) for pykrete to walk them at check time. See the [Quickstart](/pykrete/getting-started/quickstart/) for the gradual-adoption path and the [Cookbook](/pykrete/cookbook/#3-share-schemas-across-files) for the recipe.

## Why case-sensitive?

Column names in pykrete are case-sensitive. This is deliberate: pykrete checks references against the `Schema` class the user wrote, not against a live Spark catalog. The user is the source of truth — if the schema declares `region`, then `Region` is a different name, and a reference to `Region` is genuinely wrong even if Spark would resolve both in some configurations. The same principle drives the case-sensitivity rule on atomic type names above.
