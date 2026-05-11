# Design & Implementation Notes

Internal design documentation. Anything an external contributor needs to understand the codebase.

**Status:** empty. Will be populated as we build.

## Planned sections

- **Architecture overview** — high-level component diagram (CLI, parser, type checker, transpiler, diagnostics).
- **Parser layer** — how `ruff_python_parser` is integrated and what we add on top.
- **Type system** — schemas, columns, dataframes, generics, structural typing, `Any`/`Unknown`.
- **Operation semantics** — per-operation rules, derived from Spark's Catalyst analyzer.
- **Inference engine** — bidirectional checking, how annotations and inference interact.
- **Diagnostics** — error code catalog, formatting, source positions.
- **Transpiler** — what gets stripped, what gets lowered, the (mostly trivial) `.dpy` → `.py` pipeline.
- **Testing strategy** — unit tests, Catalyst-as-oracle integration tests, golden-file tests on a real-world codebase.
