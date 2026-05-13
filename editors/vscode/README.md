# Dathon VS Code extension

Thin client that launches `dathon-lsp` and routes `.dpy` files to it.

## Develop

```sh
npm install
npm run compile         # one-shot
npm run watch           # rebuild on save
```

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
