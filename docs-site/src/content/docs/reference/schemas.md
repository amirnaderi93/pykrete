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

`decimal` accepts an optional `(precision, scale)`: write `decimal(18, 2)` for money, `decimal(38, 18)` for high precision, or `decimal` on its own (precision and scale unspecified — pykrete checks the kind only).

The pykrete name is what you write in `.pyk`; the Spark column is the type in the actual dataframe. Names are case-sensitive.

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
from typing import List

class Event(Schema):
    tags: List[string]
    scores: List[int]
```

For an array of structs, the element type is another `Schema`:

```python
class LineItem(Schema):
    sku: string
    qty: int

class Order(Schema):
    lines: List[LineItem]
```

A dotted path pierces the array to its element type:

```python
orders.select(F.col("lines.sku"))   # checked against LineItem.sku
```

### Map columns

```python
from typing import Dict

class Telemetry(Schema):
    counters: Dict[string, int]
    payload: Dict[string, string]
```

pykrete doesn't walk into map values — the keys are runtime data, not part of the schema.

## Type operators

Compose schemas the way TypeScript composes object types, instead of redeclaring columns.

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

### `Omit` — drop some columns

```python
class SaleLite(Omit[Sale, "product", "quantity"]):
    pass
# SaleLite == { region: string, amount: int }
```

### `Join` — combine two schemas

```python
class Region(Schema):
    region: string
    manager: string

class SaleWithManager(Join[Sale, Region]):
    pass
# SaleWithManager has every column of Sale plus every column of Region
```

### `GroupBy` — a grouped result

```python
class SaleByRegion(GroupBy[Sale, "region"]):
    pass
# 'region' is the grouping key
```

Mirrors the shape PySpark's `groupBy` produces; used in `DataFrame[GroupBy[Sale, "region"]]` annotations.

## Cross-file schemas

Put shared schemas in their own module and import them — pykrete resolves `.pyk` imports the same way Python does:

```python
# schemas.py  (or schemas.pyk)
class Sale(Schema):
    region: string
    amount: int

# sales.pyk
from schemas import Sale

def revenue_by_region(sales: DataFrame[Sale]) -> DataFrame:
    return sales.groupBy("region").agg(F.sum("amount").alias("total"))
```

One schema, imported wherever the dataframe shows up. See the [Quickstart](/pykrete/getting-started/quickstart/) for the gradual-adoption path.
