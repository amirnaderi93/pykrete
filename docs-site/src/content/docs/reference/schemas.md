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

### Case sensitivity

Atomic names are **case-sensitive lowercase** in `.pyk` source: `int`, `string`, `decimal`. `Int`, `STRING`, `Decimal` are rejected (`D0010`). This matches Spark interop — column names already are case-sensitive (see [Why case-sensitive?](#why-case-sensitive)), and pykrete keeps the rule uniform across the type vocabulary.

The composite keywords `Array`, `Map`, `Struct` are matched case-insensitively (a legacy compatibility carve-out — the Python annotation form is conventionally `Array[…]` / `Map[…, …]`). The element types inside still follow the strict rule: `Array[Int]` is rejected because `Int` is not the atomic name; write `Array[int]`.

The wider Spark SQL vocabulary (`integer`, `bigint`, `smallint`, `tinyint`, `float`, `real`, `boolean`) is accepted only inside `.cast("…")` strings and string-form UDF return types, where Spark SQL itself is case-insensitive. Inside a Schema class body, stick to the lowercase pykrete names listed above.

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
def f(customers: DataFrame[Customer]) -> DataFrame:
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

Use `Merge` to describe the shape of a join result, a `withColumns` extension, or any concatenation of schemas. Pykrete flags overlapping non-key columns at the operator site; use `Pick` / `Omit` on one of the operands to disambiguate.

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

def revenue_by_region(sales: DataFrame[Sale]) -> DataFrame:
    return sales.groupBy("region").agg(F.sum("amount").alias("total"))
```

One schema, imported wherever the dataframe shows up. Schema modules need to be `.pyk` (not `.py`) for pykrete to walk them at check time. See the [Quickstart](/pykrete/getting-started/quickstart/) for the gradual-adoption path and the [Cookbook](/pykrete/cookbook/#3-share-schemas-across-files) for the recipe.

## Why case-sensitive?

Column names in pykrete are case-sensitive. This is deliberate: pykrete checks references against the `Schema` class the user wrote, not against a live Spark catalog. The user is the source of truth — if the schema declares `region`, then `Region` is a different name, and a reference to `Region` is genuinely wrong even if Spark would resolve both in some configurations. The same principle drives the case-sensitivity rule on atomic type names above.
