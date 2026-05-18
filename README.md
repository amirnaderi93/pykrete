# dathon

[![pipeline status](https://gitlab.com/amir.naderi93/dathon/badges/main/pipeline.svg)](https://gitlab.com/amir.naderi93/dathon/-/commits/main)
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
- [examples/](examples/) — sample `.dpy` files for poking at the checker.

## Initial target

PySpark. The author's production PySpark codebase is the real-world testing yardstick for v0.1.

## Usage

### Static checker

```bash
dathon check examples/schemas.dpy          # single file
dathon check schemas.dpy pipeline.dpy      # multi-file; cross-file Schema visibility
dathon check src/*.dpy                     # shell glob

dathon transpile examples/schemas.dpy      # emit runnable Python to stdout
dathon transpile examples/schemas.dpy > out.py
```

### Editor integration (LSP)

`dathon-lsp` is a Language Server Protocol server — live diagnostics,
hover, completion, document symbols, and go-to-definition over stdio.

It is also an **LSP multiplexer**: it embeds a Python language server
(basedpyright) as a child process and merges its responses with dathon's
schema-aware results, so a single server delivers both full Python
support and dathon's checks. See [docs/design/multiplexer.md](docs/design/multiplexer.md).

The **VS Code extension** ([editors/vscode/](editors/vscode/)) wraps this —
it launches `dathon-lsp`, bundles the Python engine, and routes `.dpy`
files to it. It is distributed as a local `.vsix` for now; marketplace
publishing is pending.

For other editors, point the LSP client at the `dathon-lsp` binary (after
`cargo build --release`, at `target/release/dathon-lsp`).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow — feature branches, Conventional Commits, CI, and the MR-based merge process.

The [roadmap](docs/roadmap.md) lays out what's planned after v0.1.

## License

MIT. See [LICENSE](LICENSE).
