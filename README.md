# dathon

[![pipeline status](https://gitlab.com/amir.naderi93/dathon/badges/main/pipeline.svg)](https://gitlab.com/amir.naderi93/dathon/-/commits/main)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A strict superset of Python that adds static schema checking for dataframes. Inspired by TypeScript's relationship to JavaScript.

**Status:** v0.1 in progress.

## What it does

- Define schemas as Python classes.
- Annotate dataframes with their schema: `DataFrame[MySchema]`.
- Catch column-name typos, schema drift, and shape mismatches at check time, not in production.
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

`dathon-lsp` is a Language Server Protocol server. It speaks LSP over stdio
and pushes live diagnostics to any LSP-compatible editor as you type.
Iteration 24 ships the skeleton (diagnostics only); hover, document symbols,
and go-to-definition land in subsequent iterations.

For now, hook it up manually via your editor's LSP config — point the
client at the `dathon-lsp` binary (after `cargo build --release`, the path
is `target/release/dathon-lsp`). A VS Code extension wrapping this is on
the roadmap.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow — feature branches, Conventional Commits, CI, and the MR-based merge process.

The [roadmap](docs/roadmap.md) lays out what's planned after v0.1.

## License

MIT. See [LICENSE](LICENSE).
