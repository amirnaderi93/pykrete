# pykrete

[![CI](https://github.com/amirnaderi93/pykrete/actions/workflows/ci.yml/badge.svg)](https://github.com/amirnaderi93/pykrete/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A strict superset of Python that adds static schema checking for dataframes. Inspired by TypeScript's relationship to JavaScript.

**Status:** actively developed. The PySpark static checker, the LSP server, and the VS Code extension all work end-to-end.

## What it does

- Define schemas as Python classes — including arbitrarily-nested `array` / `map` / `struct` columns.
- Annotate dataframes with their schema: `DataFrame[MySchema]`.
- Catch column-name typos, schema drift, shape mismatches, and column-type mismatches at check time, not in production — across whole transformation chains, inline SQL, and nested-field access.
- Live diagnostics, hover, completion, and go-to-definition in the editor.
- Transpile to plain Python — runtime cost is zero.

## Project layout

- [docs/v0.1-spec.md](docs/v0.1-spec.md) — the contract for the first usable version.
- [docs/language-reference/](docs/language-reference/) — user-facing reference (grows as features land).
- [docs/design/](docs/design/) — internal design and implementation docs (grows as we build).
- [examples/](examples/) — sample `.pyk` files for poking at the checker.

## Initial target

PySpark. The author's production PySpark codebase is the real-world testing yardstick for v0.1.

## Usage

### Static checker

```bash
pykrete check examples/schemas.pyk          # single file
pykrete check schemas.pyk pipeline.pyk      # multi-file; cross-file Schema visibility
pykrete check src/*.pyk                     # shell glob

pykrete transpile examples/schemas.pyk      # emit runnable Python to stdout
pykrete transpile examples/schemas.pyk > out.py
```

### Editor integration (LSP)

`pykrete-lsp` is a Language Server Protocol server — live diagnostics,
hover, completion, document symbols, and go-to-definition over stdio.

It is also an **LSP multiplexer**: it embeds a Python language server
(basedpyright) as a child process and merges its responses with pykrete's
schema-aware results, so a single server delivers both full Python
support and pykrete's checks. See [docs/design/multiplexer.md](docs/design/multiplexer.md).

The **VS Code extension** ([editors/vscode/](editors/vscode/)) wraps this —
it launches `pykrete-lsp`, bundles the Python engine, and routes `.pyk`
files to it. It is distributed as a local `.vsix` for now; marketplace
publishing is pending.

For other editors, point the LSP client at the `pykrete-lsp` binary (after
`cargo build --release`, at `target/release/pykrete-lsp`).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow — feature branches, Conventional Commits, CI, and the MR-based merge process.

The [roadmap](docs/roadmap.md) lays out what's planned after v0.1.

## License

MIT. See [LICENSE](LICENSE).
