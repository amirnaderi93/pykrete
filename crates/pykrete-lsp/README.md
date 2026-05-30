# pykrete-lsp

Language Server Protocol server for pykrete (`.pyk` files). Wraps the analyzer in `crates/pykrete` and speaks LSP over stdio; multiplexes an embedded Python language server so editors get pykrete's schema checks alongside general Python features from one process.

Bundled with the [VS Code extension](https://marketplace.visualstudio.com/items?itemName=amirnaderi.pykrete). Standalone install for other editors:

```sh
cargo install --git https://github.com/amirnaderi93/pykrete pykrete-lsp
```

See the [main repo](https://github.com/amirnaderi93/pykrete) for the editor setup guides and docs site.
