---
title: Install
description: Get pykrete onto your machine. Homebrew, prebuilt binaries, cargo install, or the Windows MSI — plus the VS Code extension.
---

pykrete is two binaries — `pykrete` (the checker and transpiler) and `pykrete-lsp` (the language server your editor talks to). Both install together; pick whichever method fits your setup.

## CLI

Every install method below puts both `pykrete` and `pykrete-lsp` on your machine.

### Homebrew (macOS / Linux)

```sh
brew install amirnaderi93/pykrete/pykrete
```

This taps the [`amirnaderi93/homebrew-pykrete`](https://github.com/amirnaderi93/homebrew-pykrete) repo and installs both `pykrete` and `pykrete-lsp`. The tap auto-updates on every pykrete release.

### Prebuilt binaries

Every release ships binaries for:

- macOS arm64 and x64
- Linux x64
- Windows (as an MSI installer; see below)

Download from [Releases](https://github.com/amirnaderi93/pykrete/releases) and put the binary on your `PATH`.

### Windows MSI

The `.msi` from the [latest release](https://github.com/amirnaderi93/pykrete/releases/latest) installs `pykrete.exe` and `pykrete-lsp.exe` to `C:\Program Files\pykrete\` and adds it to `PATH`. Restart your shell after installing.

### Cargo

If you have a Rust toolchain:

```sh
cargo install --git https://github.com/amirnaderi93/pykrete pykrete
cargo install --git https://github.com/amirnaderi93/pykrete pykrete-lsp
```

This builds from `main`. For a pinned release, add `--tag` with the
[latest release tag](https://github.com/amirnaderi93/pykrete/releases/latest)
(e.g. `--tag v1.8.0`).

### Verify

```sh
pykrete --version
# pykrete 1.8.0
```

`pykrete-lsp` is a stdio-based language server with no CLI surface — it has no `--version` flag and exits with an error if invoked directly from a terminal. Verify it instead by opening a `.pyk` file in VS Code (or any LSP-capable editor configured per [Other editors](#other-editors)) and watching diagnostics appear.

## VS Code extension

The extension wraps `pykrete-lsp` and lights up `.pyk` files with live diagnostics, hover, completion, go-to-definition, and the rest.

- **VS Code (Visual Studio Marketplace)** — [amirnaderi.pykrete](https://marketplace.visualstudio.com/items?itemName=amirnaderi.pykrete)
- **Cursor / VSCodium / code-server / Theia (Open VSX)** — [amirnaderi.pykrete](https://open-vsx.org/extension/amirnaderi/pykrete)
- **`.vsix`** — attached to every [release](https://github.com/amirnaderi93/pykrete/releases) for offline / sideload installs.

The extension expects `pykrete-lsp` to be on `PATH`. If you installed via Homebrew or the MSI, you're done. If you installed via cargo, make sure `~/.cargo/bin` is on `PATH`.

## Other editors

Neovim, Helix, Emacs and Zed setups talk to `pykrete-lsp` directly. See the editor-specific configs at [editors/](https://github.com/amirnaderi93/pykrete/tree/main/editors) in the repo.

## Configuration

pykrete picks up a `pykrete.json` at (or above) the project root. The defaults are reasonable; you only need a config file to relax type checking or skip directories. See [Configuration](/pykrete/reference/configuration/).

## What's next

Once installed, head to the [Quickstart](/pykrete/getting-started/quickstart/) to annotate a real function in three steps.
