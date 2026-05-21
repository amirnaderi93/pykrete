<p align="center"><img src="docs/assets/logo.svg" width="160" alt="pykrete logo"></p>

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

PySpark. pandas and polars support is planned — see the [roadmap](docs/roadmap.md).

## Install

pykrete ships two binaries: `pykrete` (the CLI checker/transpiler) and
`pykrete-lsp` (the editor language server).

**Homebrew** (macOS / Linux):

```bash
brew install amirnaderi93/pykrete/pykrete
```

**Prebuilt binaries** — download the tarball for your platform from the
[latest release](https://github.com/amirnaderi93/pykrete/releases/latest)
and put `pykrete` and `pykrete-lsp` on your `PATH`.

**From source** with Cargo (Rust ≥ 1.95):

```bash
cargo install --git https://github.com/amirnaderi93/pykrete pykrete
cargo install --git https://github.com/amirnaderi93/pykrete pykrete-lsp
```

pykrete depends on ruff's parser via a pinned git revision, which Astral
does not publish to crates.io — so installation is via Homebrew, a
prebuilt binary, or `cargo install --git`, not `cargo install pykrete`.

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
files to it.

**VS Code, Cursor, VSCodium, code-server, Theia** — search **pykrete**
in the extensions panel. Each release is published to both the
[Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=amirnaderi.pykrete)
(VS Code's default source) and the
[Open VSX Registry](https://open-vsx.org/extension/amirnaderi/pykrete)
(the default for the others).

If you can't reach either registry, every release also attaches a
`pykrete-vscode-vX.Y.Z.vsix` you can side-load:

```bash
code --install-extension pykrete-vscode-vX.Y.Z.vsix
```

Or inside the editor: **Extensions panel → ⋯ menu → Install from VSIX…**

For Neovim, Helix, Emacs, and other LSP clients, see
[docs/editors/](docs/editors/).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow — feature branches, Conventional Commits, CI, and the pull-request review process.

The [roadmap](docs/roadmap.md) lays out what's planned after v0.1.

## License

MIT. See [LICENSE](LICENSE).

## Built with Claude

pykrete is developed with [Claude Code](https://claude.com/claude-code), Anthropic's agentic coding tool.
