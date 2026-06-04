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
- **[pandas-support.md](pandas-support.md)** — v1.3 spec for the pandas
  dialect: `PandasFrame[X]` parser surface, the six dispatched
  operations, `D0090 deprecatedDataFrameAlias`, and the v1.4 deferrals.
- **[spark-coverage.md](spark-coverage.md)** — the per-operation matrix
  of what pykrete recognizes on the PySpark surface: every method,
  every recognized argument shape, every result-schema inference rule.
  Source of truth for "does pykrete check this?"
- **[schema-tracking-probes.md](schema-tracking-probes.md)** — the
  cross-codebase trust harness in pykrete-tests: how positive
  (`PROBE-COL-IS`, `PROBE-TYPE-IS`) and negative (`PROBE-EXPECTS`)
  probes pin schema-tracking behavior across donor fixtures.
- **[literal-value-vocabulary.md](literal-value-vocabulary.md)** — how
  pykrete recognizes literal values in column expressions, the
  enum-vocabulary plumbing that powers `D0084 enumValueMismatch`, and
  the broader literal-checking story.
