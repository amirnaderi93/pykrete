# Editor setup

pykrete ships a Language Server Protocol server, `pykrete-lsp`, that speaks
LSP over stdio: live diagnostics, hover, completion, document symbols, and
go-to-definition for `.pyk` files. The [VS Code extension](../../editors/vscode/)
wraps it for you; this page covers every other editor.

## Prerequisites

1. Install the binaries — see the [README](../../README.md#install). After
   `cargo build --release` the server is at `target/release/pykrete-lsp`;
   put it on your `PATH` (or use an absolute path in the configs below).
2. Optional: install [basedpyright](https://github.com/DetachHead/basedpyright)
   and put `basedpyright-langserver` on `PATH`. `pykrete-lsp` embeds it to
   add general Python language features alongside pykrete's schema checks.
   pykrete's own checks work whether or not it is present.

`pykrete-lsp` takes no arguments — point your LSP client's command at it
and route `.pyk` files to it.

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
