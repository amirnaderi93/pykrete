# Design & Implementation Notes

Internal design documentation — what an external contributor needs to
understand the codebase.

## Documents

- **[architecture.md](architecture.md)** — how the checker and the LSP
  server are organized: the pipeline, every module, the diagnostic codes,
  the type system, multi-file analysis.
- **[multiplexer.md](multiplexer.md)** — the LSP multiplexer: how
  `pykrete-lsp` embeds a Python language server, the virtual-document
  transform, message routing.

## Still to write

- **Operation semantics** — per-operation rules, derived from Spark's
  Catalyst analyzer.
- **Testing strategy** — unit tests, integration tests, and real-world testing
  against a real-world codebase.
