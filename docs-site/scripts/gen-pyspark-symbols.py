#!/usr/bin/env python3
"""Generate `public/pyspark-symbols.json` from live pyspark introspection.

Walks the public surface of pyspark's DataFrame, Column, GroupedData,
functions module, and Window class; emits `{name, signature, doc}` per
symbol. The Monaco playground reads the JSON at runtime to drive
autocomplete and hover.

Run manually when bumping the pinned pyspark version. CI does not
run this — the generated JSON is committed.
"""

from __future__ import annotations

import inspect
import json
import sys
from pathlib import Path
from typing import Any


def _doc_summary(obj: Any) -> str:
    """First paragraph of the docstring, trimmed."""
    doc = inspect.getdoc(obj) or ""
    if not doc:
        return ""
    para = doc.split("\n\n", 1)[0].strip()
    para = " ".join(line.strip() for line in para.splitlines() if line.strip())
    if len(para) > 400:
        para = para[:397] + "..."
    return para


def _signature(obj: Any) -> str:
    try:
        sig = inspect.signature(obj)
    except (TypeError, ValueError):
        return "(...)"
    return str(sig)


def _collect_class(cls: type) -> list[dict[str, str]]:
    out: list[dict[str, str]] = []
    seen: set[str] = set()
    for name in dir(cls):
        if name.startswith("_"):
            continue
        if name in seen:
            continue
        seen.add(name)
        try:
            attr = inspect.getattr_static(cls, name)
        except AttributeError:
            continue
        # Skip nested classes; we only want methods and properties.
        if inspect.isclass(attr):
            continue
        try:
            # Resolve through the class so descriptors (property,
            # classmethod, staticmethod) yield the underlying callable
            # for signature/doc extraction.
            resolved = getattr(cls, name)
        except Exception as err:  # noqa: BLE001
            print(f"  warn: {cls.__name__}.{name}: getattr failed: {err}", file=sys.stderr)
            continue
        try:
            if isinstance(attr, property):
                doc = _doc_summary(attr.fget) if attr.fget else ""
                out.append({"name": name, "signature": "", "doc": doc})
            elif callable(resolved):
                out.append(
                    {
                        "name": name,
                        "signature": _signature(resolved),
                        "doc": _doc_summary(resolved),
                    }
                )
            elif isinstance(resolved, (int, float, str, bytes, bool)):
                # Primitive class-level constants (e.g. `Window.currentRow`,
                # `Window.unboundedPreceding`) — `_doc_summary` would walk
                # to `int.__doc__` and emit `int([x]) -> integer …`. Emit
                # an empty doc instead; the symbol name is the signal.
                out.append({"name": name, "signature": "", "doc": ""})
            else:
                out.append({"name": name, "signature": "", "doc": _doc_summary(resolved)})
        except Exception as err:  # noqa: BLE001
            print(f"  warn: {cls.__name__}.{name}: introspection failed: {err}", file=sys.stderr)
            continue
    out.sort(key=lambda item: item["name"])
    return out


def _collect_module(mod: Any) -> list[dict[str, str]]:
    out: list[dict[str, str]] = []
    mod_name = mod.__name__
    for name in dir(mod):
        if name.startswith("_"):
            continue
        try:
            obj = getattr(mod, name)
        except Exception as err:  # noqa: BLE001
            print(f"  warn: {mod.__name__}.{name}: getattr failed: {err}", file=sys.stderr)
            continue
        if not callable(obj):
            continue
        # Skip classes re-exported into the functions module (Column,
        # WindowSpec, etc.) — we want function-style entries here.
        if inspect.isclass(obj):
            continue
        # Skip names re-exported from other modules (typing helpers like
        # `Callable`, `Dict`, `Iterable`; `datetime`, etc.) — we want
        # only the real `pyspark.sql.functions` surface.
        obj_mod = getattr(obj, "__module__", None)
        if obj_mod is not None and obj_mod != mod_name:
            continue
        try:
            out.append(
                {
                    "name": name,
                    "signature": _signature(obj),
                    "doc": _doc_summary(obj),
                }
            )
        except Exception as err:  # noqa: BLE001
            print(f"  warn: {mod.__name__}.{name}: introspection failed: {err}", file=sys.stderr)
            continue
    out.sort(key=lambda item: item["name"])
    return out


def main() -> int:
    try:
        import pyspark  # noqa: F401
        from pyspark.sql.column import Column
        from pyspark.sql.dataframe import DataFrame
        from pyspark.sql import functions as F
        from pyspark.sql.group import GroupedData
        from pyspark.sql.window import Window
    except ImportError as err:
        print(
            f"[gen-pyspark-symbols] pyspark not importable ({err}); "
            f"skipping symbol-table regeneration. The playground will fall "
            f"back to the existing committed JSON (if any).",
            file=sys.stderr,
        )
        return 0

    print(f"[gen-pyspark-symbols] pyspark {pyspark.__version__}")

    payload = {
        "_meta": {
            "pyspark_version": pyspark.__version__,
        },
        "DataFrame": _collect_class(DataFrame),
        "Column": _collect_class(Column),
        "GroupedData": _collect_class(GroupedData),
        "Window": _collect_class(Window),
        "functions": _collect_module(F),
    }

    out_path = Path(__file__).resolve().parent.parent / "public" / "pyspark-symbols.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, indent=2, sort_keys=False) + "\n")
    counts = {k: len(v) for k, v in payload.items() if isinstance(v, list)}
    print(f"[gen-pyspark-symbols] wrote {out_path}: {counts}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
