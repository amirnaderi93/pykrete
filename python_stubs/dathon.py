"""Pylance / basedpyright companion stubs for dathon.

dathon treats `Schema`, `DataFrame`, `col`, `string`, `date`, `timestamp`,
`double`, `long` as magic names — they don't have to be imported for
dathon's checker to recognize them. But your companion Python LSP
(Pylance / basedpyright / pyright / ruff-lsp) doesn't know that, so it
flags every Schema declaration and column-type annotation as an
undefined name.

This module exists to make the Python LSP happy. Drop it anywhere on
the Python module-search path (the project root usually works) and add
to the top of every `.dpy` file::

    from dathon import Schema, DataFrame, col, string, date, timestamp, double, long

dathon's analyzer silently ignores this import — module paths that
don't resolve to a project `.dpy` file are treated as external Python
imports and skipped. So you get full Python LSP type-checking AND
dathon's dataframe checks, with no duplication.

The runtime semantics here are deliberate placeholders. If you want
the transpiled `.py` output to actually run (after `dathon transpile
foo.dpy > foo.py`), you'll need to substitute these names with real
PySpark imports — that's a separate workflow from the LSP/editor
story.
"""

from __future__ import annotations

from datetime import date as _date, datetime as _datetime
from typing import Any, Generic, TypeVar

__all__ = [
    "Schema",
    "DataFrame",
    "col",
    "string",
    "date",
    "timestamp",
    "double",
    "long",
]


class Schema:
    """Marker base class for dathon schema declarations.

    Subclasses describe a dataframe row's structure as annotated
    attributes::

        class Orders(Schema):
            place_code: int
            price: double
    """


# Column type aliases. These map dathon's PySpark-flavoured vocabulary
# onto Python types so Pylance accepts them as field annotations.
string = str
date = _date
timestamp = _datetime
double = float
long = int


_T = TypeVar("_T")


class DataFrame(Generic[_T]):
    """Generic dataframe shape used in typed function signatures::

        def prepare_orders(raw: DataFrame[RawOrders]) -> DataFrame[Orders]: ...
    """


def col(name: str) -> Any:
    """Column-reference function — `col("foo")` names a column by string.

    Returns `Any` so the LSP doesn't try to type-check operations on
    the returned value. dathon's checker validates the column name
    against the active schema regardless of this function's signature.
    """
    return name
