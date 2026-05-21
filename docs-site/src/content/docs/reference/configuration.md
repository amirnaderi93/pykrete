---
title: Configuration
description: The pykrete.json reference — typeCheckingMode, exclude, and per-rule severity overrides.
---

pykrete reads a `pykrete.json` file from the project root, or the nearest ancestor directory. The same file configures the CLI and the language server, so the editor and your CI agree.

You don't need one to get started — the defaults are sensible. Add a `pykrete.json` when you want to tune behavior.

```json
{
  "typeCheckingMode": "standard",
  "exclude": ["target", ".venv"],
  "rules": {
    "unionSchemaMismatch": "warning"
  }
}
```

## `typeCheckingMode`

How far pykrete goes when checking column **types**. Column **existence** checking (`unknownColumn` and friends) runs regardless of this setting.

| Value | Behavior |
|---|---|
| `off` | No type checking. Existence checks still run. |
| `basic` | Minimal type checking. |
| `standard` *(default)* | Conservative type checking — `returnTypeMismatch` fires only when two types are confidently known and genuinely incompatible. |
| `strict` | Everything in `standard`, plus the advisory checks: `nonNumericArithmetic`, `crossTypeComparison`, `nullabilityMismatch`. |

The language server reads the same value, and a `pykrete.json` `typeCheckingMode` takes precedence over the editor's own setting. See [Diagnostics](/pykrete/reference/diagnostics/#type-checking-diagnostics) for what each level surfaces.

## `exclude`

Path substrings to skip. A file whose path contains any of these is not checked.

```json
{
  "exclude": ["target", ".venv", "generated"]
}
```

Useful for build output, virtual environments, and vendored or generated code.

## `rules`

Per-rule severity overrides — turn a rule into a warning, or off entirely.

```json
{
  "rules": {
    "unknownColumn": "error",
    "unionSchemaMismatch": "warning",
    "returnTypeMismatch": "off"
  }
}
```

Each value is `error`, `warning`, or `off`. Keys are [rule names](/pykrete/reference/diagnostics/#full-reference) — the `D00xx` code works too.

A common adoption pattern on an existing codebase: set the noisy rules to `warning` first, clear them at your own pace, then promote them back to `error` once the project is clean.

## Where pykrete looks

Starting from the file being checked, pykrete walks up the directory tree and uses the first `pykrete.json` it finds. With none, the defaults apply:

- `typeCheckingMode`: `standard`
- `exclude`: empty
- `rules`: empty — every rule at its default severity
