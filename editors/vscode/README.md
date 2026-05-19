# Pykrete VS Code extension

Thin client that launches `pykrete-lsp` and routes `.pyk` files to it.

## What you get

Install the extension, open a `.pyk` file. pykrete-lsp gives you:

- **pykrete's schema features** — diagnostics, hover on `class X(Schema)` /
  `DataFrame[X]` / `col("foo")`, completion inside `col("…")` / `df.` /
  `DataFrame[…]`, go-to-definition, "Did you mean?" quick-fixes.
- **general Python features** — hover, completion, go-to-definition,
  find-all-references, and signature help for ordinary Python code,
  plus Python type diagnostics.

There is **nothing else to install or configure** — no separate Python
LSP extension, no `files.associations`, no import stub, no import line
in your `.pyk` files.

## How it works

pykrete-lsp is a **multiplexer**: it embeds a Python language server
([basedpyright](https://github.com/detachhead/basedpyright), bundled
with this extension) as a child process, and presents a single merged
language server to VS Code. Schema-aware answers come from pykrete;
general Python answers come from the embedded engine; pykrete-lsp merges
the two.

The bundled engine runs on **Node.js** — if `node` isn't on your `PATH`,
pykrete-lsp logs it and runs **pykrete-only**: the schema features still
work, the general Python features are simply absent. Nothing breaks.

To embed your own Python language server instead of the bundled one,
set `pykrete.pythonServer.path` to a `basedpyright-langserver` /
`pyright-langserver` binary.

## Develop

```sh
npm install
npm run compile         # one-shot
npm run watch           # rebuild on save
```

`npm install` also fetches the bundled basedpyright (~40 MB) into
`node_modules`; it ships inside the packaged `.vsix`.

## Package + install locally

```sh
npx vsce package --allow-missing-repository
code --install-extension pykrete-vscode-0.1.0.vsix
```

To pick up code changes after reinstall, reload the VS Code window
(`Developer: Reload Window`).

## Server discovery

The extension looks for the `pykrete-lsp` binary in this order:

1. `pykrete.serverPath` setting, if set.
2. `<workspace>/target/release/pykrete-lsp`.
3. `<workspace>/target/debug/pykrete-lsp`.
4. `pykrete-lsp` on `PATH`.

Run `cargo build --release -p pykrete-lsp` from the workspace root before
opening a `.pyk` file.

The embedded Python engine is resolved separately:

1. `pykrete.pythonServer.path` setting, if set.
2. The basedpyright bundled in this extension (needs Node.js on `PATH`).
3. `basedpyright-langserver` / `pyright-langserver` on `PATH`.
4. None found — pykrete-lsp runs pykrete-only.
