# Editor setup

pykrete ships a Language Server Protocol server, `pykrete-lsp`, that speaks
LSP over stdio: live diagnostics, hover, completion, document symbols, and
go-to-definition for `.pyk` files. The [VS Code extension](../../editors/vscode/)
wraps it for you; this page covers every other editor.

## Prerequisites

Install the `pykrete-lsp` binary, then point your editor at it. The
[install guide](https://amirnaderi93.github.io/pykrete/getting-started/install/)
has the full menu; the short version, in order of preference:

1. **Homebrew** (macOS / Linux):

   ```sh
   brew install amirnaderi93/pykrete/pykrete
   ```

2. **Windows MSI** — download from the
   [latest release](https://github.com/amirnaderi93/pykrete/releases/latest).
   The installer drops `pykrete.exe` and `pykrete-lsp.exe` into
   `C:\Program Files\pykrete\` and adds them to `PATH`.

3. **`cargo install`** — anywhere with a Rust toolchain:

   ```sh
   cargo install --git https://github.com/amirnaderi93/pykrete pykrete
   cargo install --git https://github.com/amirnaderi93/pykrete pykrete-lsp
   ```

   For a pinned release add `--tag` with the latest release tag
   (e.g. `--tag v1.3.0`).

4. **Prebuilt binaries** — every release attaches per-platform archives
   for macOS arm64/x64 and Linux x64; extract and put the binaries on
   your `PATH`.

Each method installs both `pykrete` (the checker / transpiler) and
`pykrete-lsp` (the language server).

Optional: install [basedpyright](https://github.com/DetachHead/basedpyright)
and put `basedpyright-langserver` on `PATH`. `pykrete-lsp` embeds it to
add general Python language features alongside pykrete's schema checks.
pykrete's own checks work whether or not it is present.

`pykrete-lsp` takes no arguments — point your LSP client's command at it
and route `.pyk` files to it.

> Contributing? Build from source with `cargo build --release` from
> the repo root; the server lands at `target/release/pykrete-lsp`. Use
> an absolute path in the editor configs below (or symlink it onto
> `PATH`). See [CONTRIBUTING.md](../../CONTRIBUTING.md).

## Neovim

Neovim 0.11+ (built-in LSP config):

```lua
vim.filetype.add({ extension = { pyk = "pyk" } })

vim.lsp.config("pykrete", {
  cmd = { "pykrete-lsp" },
  filetypes = { "pyk" },
  root_markers = { "pykrete.json", ".git" },
})
vim.lsp.enable("pykrete")
```

On older Neovim with `nvim-lspconfig`, define the same `cmd` / `filetypes` /
`root_dir` via a custom server entry.

## Helix

In `~/.config/helix/languages.toml` — reuses Python's tree-sitter grammar
for highlighting:

```toml
[language-server.pykrete-lsp]
command = "pykrete-lsp"

[[language]]
name = "pyk"
scope = "source.python"
file-types = ["pyk"]
roots = ["pykrete.json"]
comment-tokens = ["#"]
indent = { tab-width = 4, unit = "    " }
grammar = "python"
language-servers = ["pykrete-lsp"]
```

Run `hx --grammar fetch && hx --grammar build` once if the Python grammar
is not already built.

## Emacs

With Eglot (built into Emacs 29+):

```elisp
(define-derived-mode pyk-mode python-mode "Pyk"
  "Major mode for pykrete .pyk files.")
(add-to-list 'auto-mode-alist '("\\.pyk\\'" . pyk-mode))

(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs '(pyk-mode . ("pykrete-lsp"))))
```

`M-x eglot` in a `.pyk` buffer starts the server.

## Zed

Zed only attaches language servers through extensions, so a dedicated
pykrete Zed extension is needed; one is planned. Until it lands, use the
VS Code extension or one of the editors above.

## Other LSP clients

Any LSP client works: launch `pykrete-lsp`, communicate over stdio, and
send it `.pyk` documents. Use `pykrete.json` at the project root as the
root marker and for [strictness configuration](../v0.1-spec.md#10-strictness-modes).
