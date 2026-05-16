# Dathon VS Code extension

Thin client that launches `dathon-lsp` and routes `.dpy` files to it.

## What you get

Install the extension, open a `.dpy` file. dathon-lsp gives you:

- **dathon's schema features** — diagnostics, hover on `class X(Schema)` /
  `DataFrame[X]` / `col("foo")`, completion inside `col("…")` / `df.` /
  `DataFrame[…]`, go-to-definition, "Did you mean?" quick-fixes.
- **general Python features** — hover, completion, go-to-definition,
  find-all-references, and signature help for ordinary Python code,
  plus Python type diagnostics.

There is **nothing else to install or configure** — no separate Python
LSP extension, no `files.associations`, no import stub, no import line
in your `.dpy` files.

## How it works

dathon-lsp is a **multiplexer**: it embeds a Python language server
([basedpyright](https://github.com/detachhead/basedpyright), bundled
with this extension) as a child process, and presents a single merged
language server to VS Code. Schema-aware answers come from dathon;
general Python answers come from the embedded engine; dathon-lsp merges
the two.

The bundled engine runs on **Node.js** — if `node` isn't on your `PATH`,
dathon-lsp logs it and runs **dathon-only**: the schema features still
work, the general Python features are simply absent. Nothing breaks.

To embed your own Python language server instead of the bundled one,
set `dathon.pythonServer.path` to a `basedpyright-langserver` /
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
code --install-extension dathon-vscode-0.1.0.vsix
```

To pick up code changes after reinstall, reload the VS Code window
(`Developer: Reload Window`).

## Server discovery

The extension looks for the `dathon-lsp` binary in this order:

1. `dathon.serverPath` setting, if set.
2. `<workspace>/target/release/dathon-lsp`.
3. `<workspace>/target/debug/dathon-lsp`.
4. `dathon-lsp` on `PATH`.

Run `cargo build --release -p dathon-lsp` from the workspace root before
opening a `.dpy` file.

The embedded Python engine is resolved separately:

1. `dathon.pythonServer.path` setting, if set.
2. The basedpyright bundled in this extension (needs Node.js on `PATH`).
3. `basedpyright-langserver` / `pyright-langserver` on `PATH`.
4. None found — dathon-lsp runs dathon-only.
