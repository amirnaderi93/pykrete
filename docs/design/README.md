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

## Cycle specs

One per minor release, written before the cycle starts and amended at
cycle close. Each carries the scope decisions, the settled design
forks, the per-PR briefs, and the deferral set for the following cycle.

- [v1.4](v1.4-spec.md) — pandas depth: 7 new donors, pandas type-tracking
- [v1.5](v1.5-spec.md) — cross-dialect handoff, `.loc` literal-form
- [v1.6](v1.6-spec.md) — `pykrete migrate`, `pivot_table` literal-form
- [v1.7](v1.7-spec.md) — migrator `--check` default, `melt` literal-form
- [v1.8](v1.8-spec.md) — v2.0 deprecation runway, `D0091`
- [v1.9](v1.9-spec.md) — migration plannability, `D0091` maturity
- [v1.10](v1.10-spec.md) — migration archivability, `stack` literal-form
- [v1.11](v1.11-spec.md) — `unstack` literal-form, audit-tooling block
- [v1.12](v1.12-spec.md) — D0080 cross-codebase, `pivot_table(aggfunc=)` allowlist
- [v1.13](v1.13-spec.md) — D0080 dialect-on-return, aggfunc Derived synthesis
- [v1.14](v1.14-spec.md) — D0080 constructor arms, `groupby.agg` synthesis
- [v1.15](v1.15-spec.md) — pandas chain-depth, `resolve_override_ty`
- [v1.16](v1.16-spec.md) — time/window aggregation, dict + callable `groupby.agg`

`v15_retro.md` holds the v1.15 cycle retrospective.
