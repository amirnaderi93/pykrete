---
title: Schemas
description: Declare dataframe shapes as Python classes — atomic types, nested arrays / maps / structs, and TypeScript-style type operators.
---

A schema is a Python class. pykrete reads its annotated attributes as the columns of a dataframe.

```python
class Order(Schema):
    place_code: int
    price: int
    status: string
```

`Order` declares three columns. Field name is the column name; field type is the column type.

## Atomic types

| Pykrete | Spark / SQL |
|---|---|
| `int` | `int` |
| `long` | `bigint` |
| `string` | `string` |
| `double` | `double` |
| `float` | `float` |
| `bool` | `boolean` |
| `date` | `date` |
| `timestamp` | `timestamp` |
| `decimal` | `decimal(p, s)` |
| `bytes` | `binary` |

The Pykrete name is what you write in `.pyk`; the Spark column is what's stored in the actual dataframe. Atomic types are case-sensitive.

## Optional columns

Wrap any atomic type in `Optional[...]` to mark it nullable:

```python
class User(Schema):
    id: int
    email: Optional[string]
```

The checker treats `Optional[T]` and `T` interchangeably for column-existence checks; the distinction matters for [strict type-checking mode](/pykrete/reference/configuration/#typecheckingmode).

## Nested types

### Struct columns

A struct column is declared by referencing another `Schema` class as a field type:

```python
class Address(Schema):
    street: string
    city: string
    zip: string

class User(Schema):
    id: int
    address: Address
```

Dotted access through the nested struct works in column refs:

```python
def f(users: DataFrame[User]) -> DataFrame:
    return users.select(F.col("address.city"))
```

A typo on the dotted path (`"address.cty"`) fires `D0030` and names the failing schema (`Address`).

### Array columns

```python
from typing import List

class Order(Schema):
    line_skus: List[string]
    tags: List[int]
```

Arrays of structs work via a nested Schema reference:

```python
class LineItem(Schema):
    sku: string
    qty: int

class Order(Schema):
    lines: List[LineItem]
```

A dotted path into an array's element type pierces the array:

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

The checker doesn't walk into map values (key names are runtime data, not part of the schema).

## TypeScript-style type operators

Compose schemas the way TypeScript composes object types:

### `Pick`

```python
class User(Schema):
    id: int
    email: string
    name: string

class UserSummary(Pick[User, "id", "name"]):
    pass
# UserSummary == { id: int, name: string }
```

### `Omit`

```python
class UserPublic(Omit[User, "email"]):
    pass
# UserPublic == { id: int, name: string }
```

### `Join`

```python
class Address(Schema):
    user_id: int
    city: string

class UserWithCity(Join[User, Address]):
    pass
# UserWithCity == { id: int, email: string, name: string, user_id: int, city: string }
```

### `GroupBy`

```python
class Aggregated(GroupBy[User, "name"]):
    pass
# Aggregated has 'name' as a grouping key
```

Mirrors PySpark's groupBy result type. Used by `DataFrame[GroupBy[Schema, key]]` annotations.

## Cross-file schemas

Move shared schemas to a dedicated module. Importing the schema class brings the type into scope; pykrete walks `.pyk` imports the same way Python does:

```python
# schemas.py (or schemas.pyk)
class Order(Schema):
    place_code: int
    price: int

# orders.pyk
from schemas import Order

def total_per_place(orders: DataFrame[Order]) -> DataFrame:
    return orders.groupBy("place_code").agg(F.sum("price").alias("total"))
```

See the [Quickstart](/pykrete/getting-started/quickstart/) for the gradual-adoption path.
