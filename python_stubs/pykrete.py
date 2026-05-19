"""Pylance / basedpyright companion stubs for pykrete.

Iteration 41 moved the column-type vocabulary into **string literals**
(`EventDate: "timestamp"` instead of `EventDate: timestamp`), so the only
pykrete identifiers users still write are `Schema`, `DataFrame`, and
`col`. This stub provides Pylance-friendly declarations for those.

Drop this file anywhere on the Python module-search path (the project
root usually works) and add at the top of each `.pyk` file::

    from pykrete import Schema, DataFrame, col

pykrete's analyzer silently ignores this import — module paths that
don't resolve to a project `.pyk` file are treated as external Python
imports and skipped. So you get full Python LSP type-checking AND
pykrete's dataframe checks, with no duplication.

Iteration 42 will retire this file entirely — the VS Code extension
will bundle a typeshed extension that makes these names globally
available without any import, just like TypeScript's built-in `string`
and `Promise`.
"""

from __future__ import annotations

from typing import Any, Generic, TypeVar

__all__ = ["Schema", "DataFrame", "col"]


class Schema:
    """Marker base class for pykrete schema declarations.

    Subclasses describe a dataframe row's structure as annotated
    attributes whose annotations are **string literals naming a column
    type**::

        class Orders(Schema):
            place_code: "int"
            price: "double"

    pykrete recognizes the strings as column-type names from the
    vocabulary `int`, `long`, `double`, `string`, `bool`, `date`,
    `timestamp`. Nested-struct fields keep using bare names —
    `address: Address` — so the type-checker can still resolve them.
    """


_T = TypeVar("_T")


class DataFrame(Generic[_T]):
    """Generic dataframe shape used in typed function signatures::

        def prepare_orders(raw: DataFrame[RawOrders]) -> DataFrame[Orders]: ...
    """


def col(name: str) -> Any:
    """Column-reference function — `col("foo")` names a column by string.

    Returns `Any` so the LSP doesn't try to type-check operations on
    the returned value. pykrete's checker validates the column name
    against the active schema regardless of this function's signature.
    """
    return name
