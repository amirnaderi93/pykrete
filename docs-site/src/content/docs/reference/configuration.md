---
title: Configuration
description: pykrete.json reference — typeCheckingMode, exclude, rules.
---

pykrete picks up a `pykrete.json` file at (or above) the project root. The same file configures both the CLI and the LSP server.

A minimal example:

```json
{
  "typeCheckingMode": "standard",
  "exclude": ["target", ".venv", "tests/fixtures"],
  "rules": {
    "unionSchemaMismatch": "warning"
  }
}
```

## `typeCheckingMode`

How aggressively pykrete checks column types.

| Value | What it does |
|---|---|
| `off` | No type checking; column-existence checks (`D0030`) still run. |
| `basic` | `D0080` only — obvious type errors. Permissive. |
| `standard` *(default)* | `D0080` + a stricter pass that catches more mismatches. |
| `strict` | `standard` + `D0081` + nullability (`D0082`). |

The LSP server picks up the same setting. For the bundled VS Code extension, `pykrete.json`'s `typeCheckingMode` overrides the editor's setting, and the single value also drives the embedded Python language server.

## `exclude`

Path substrings to skip. Any file whose path contains one of these is not checked.

```json
{
  "exclude": ["target", ".venv", "node_modules"]
}
```

Useful for excluding generated code, vendored sources, or build directories.

## `rules`

Per-rule severity overrides. Keyed by the readable rule name (not the `D00XX` code).

```json
{
  "rules": {
    "unknownColumn": "error",
    "unionSchemaMismatch": "warning",
    "columnTypeMismatch": "off"
  }
}
```

Possible values: `error`, `warning`, `off`.

Common patterns:

- **Adopting incrementally on a legacy codebase** — set everything to `warning` first, fix the loudest ones, then turn the rules back to `error` once the codebase is clean.
- **Suppressing a noisy rule on a specific style** — turn `columnTypeMismatch` to `warning` if you intentionally feed `int` columns into string-formatting UDFs.

## Where pykrete looks for the file

pykrete walks up from the file being checked, looking for the nearest `pykrete.json`. If none is found, defaults apply:

- `typeCheckingMode`: `standard`
- `exclude`: empty
- `rules`: empty (all rules at their default severity)

You don't *need* a `pykrete.json` to use pykrete. Add one when you want to tune behavior.
