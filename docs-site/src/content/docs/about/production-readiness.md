---
title: Production readiness
description: Stability commitments, false-positive policy, release cadence, and known limitations for production PySpark teams evaluating pykrete.
---

## TL;DR

pykrete is feature-complete for PySpark as of the [v0.1 release line](https://github.com/amirnaderi93/pykrete/releases). A deliberate "degrade to Unknown rather than fabricate" policy keeps the checker honest: when pykrete can't determine a schema or a type with confidence, it stops checking that subtree rather than guessing. A real-codebase integration loop ([pykrete-tests](/about/pykrete-tests/)) catches regressions before they ship.

For the trust posture behind the engineering — why pykrete cannot break a production pipeline, and how each release is validated — see the [Reliability and trust](https://github.com/amirnaderi93/pykrete#reliability-and-trust) section of the README.

## Stability commitments

Once a piece of surface ships in a release, the project commits to backward-compatible behavior on the following:

- **Schema declaration syntax.** `Schema` classes, `Optional[T]` for nullable columns, the `Array` / `Map` / struct-class nested-type forms, and the TypeScript-style schema operators (`Pick`, `Omit`, `Merge`).
- **The `DataFrame[Schema]` annotation surface.** Variable annotations, function parameter and return types, `.cast(DataFrame[Schema])` re-anchors.
- **Diagnostic codes.** `D0001`, `D0010`, `D0011`, `D0020`, `D0021`, `D0030`, `D0040`, `D0050`, `D0051`, `D0060`, `D0070`, `D0071`, `D0072`, `D0073`, `D0080`, `D0081`, `D0082`, `D0083`, `D0084`. The numeric code and the rule name are part of the contract; the diagnostic message text is not.
- **`pykrete.json` keys.** `typeCheckingMode`, `exclude`, `rules`. New keys may be added; existing ones won't change shape.
- **The CLI's machine-readable output** (`pykrete check --format json`) and exit codes. Shipped in [v0.1.33](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.33); the JSON schema becomes a stability contract at v1.0.0 (breaking changes after that point require a SemVer major bump). Exit codes are also part of the contract: `0` when no diagnostics, `1` when any diagnostic fires (error _or_ warning — matches the text format and lets CI scripts react uniformly to warnings like `D0072 duplicateSchemaName`). A future `--max-severity` flag may let consumers customize this; tracked in `docs/design/spark-coverage.md`.

### JSON output stability contract

The `--format json` payload carries an explicit `schemaVersion` field (currently `"1"`). Consumers pin to that; the pykrete `version` is informational. The contract covers:

- **JSON field names — STABLE.** Renaming a field requires a SemVer-major bump and a `schemaVersion` bump.
- **JSON field types — STABLE.** Changing a string to an integer (or similar) requires a SemVer-major bump.
- **JSON field semantics — STABLE.** Changing what a field means requires a SemVer-major bump.
- **D-code identity — STABLE.** `D0030` will always mean `unknownColumn`; codes are never reassigned.
- **Diagnostic message wording — NOT STABLE.** Rewording for clarity is a SemVer-minor change. Consumers should match on `code` / `ruleName` / `severity`, not on message text.
- **Adding a new top-level or per-diagnostic field — NON-BREAKING.** Consumers must accept unknown fields. `schemaVersion` stays at `"1"`.
- **Adding a new severity — NON-BREAKING.** Consumers must handle unknown severities gracefully (a sensible default is to treat unknown as `error`). `schemaVersion` stays at `"1"`.
- **Adding a new D-code — NON-BREAKING.** Consumers must handle unknown codes gracefully. `schemaVersion` stays at `"1"`.

Bumping `schemaVersion` to `"2"` only happens alongside a SemVer-major pykrete release.

What may still change without notice:

- The internal LSP wire protocol with the embedded Python engine (today's multiplexer is interim — see the [roadmap](/about/roadmap/#forking-ty)).
- The wasm API surface (`pykrete-wasm`): shipped in v0.1.16 and consumed by the in-browser [playground](/playground/). The current export shape (`check_source`, `hover_at`, `complete_at`, `definition_at`) is stable in spirit until v1.0.0 and becomes part of the SemVer contract from v1.0 onward. The crate is a single-file analyzer wrapper, not a general-purpose embedding library — multi-file / cross-import support stays a CLI / LSP capability.

## False-positive policy

**No false positives.** When pykrete can't determine a schema or a column's type with confidence, it degrades that subtree to Unknown rather than guess. Two concrete examples from the v0.1 surface:

- `spark.read.parquet("s3://...")` returns Unknown until the user re-anchors with `.cast(DataFrame[Schema])` or a typed variable annotation. The schema is genuinely runtime data; pykrete won't invent one.
- `F.struct(F.lit(1))` falls back to positional names (`col1`, `col2`, …) when no `.alias("x")` is present, rather than fabricating a guessed field name. Heterogeneous value types in `melt` / `unpivot` degrade the value-column type to Unknown rather than picking a "winner".

The same rule applies at the generic-inference layer: a TypeVar bound to incompatible schemas across argument slots stays Unknown. Downstream checks against an Unknown subtree are permissive: no diagnostic fires unless the user re-anchors.

A static checker that cries wolf gets switched off; pykrete prefers to stay quiet when it isn't sure.

## Release cadence

The Spark-coverage closure sprint (v0.1.7 onward, May 2026) ran at multiple releases per week — the finishing pass on the v1.0.0 surface, not a steady-state cadence. Expect a more measured pace once v1.0.0 ships and focus shifts to pandas / polars. See the [GitHub Releases page](https://github.com/amirnaderi93/pykrete/releases) for the full per-release history.

## Real-codebase testing

Every release is regression-tested against **47 fixtures from 10 upstream codebases** (35 annotated + 12 deliberately-corrupted under `probes_negative/`) — Apache Spark, Delta Lake, Apache Iceberg ([iceberg-python](https://github.com/apache/iceberg-python)), Apache Hudi, MLflow, Feast, Kedro ([kedro-plugins](https://github.com/kedro-org/kedro-plugins)), [quinn](https://github.com/MrPowers/quinn), [dbt-spark](https://github.com/dbt-labs/dbt-spark), and [python-deequ](https://github.com/awslabs/python-deequ). See [Real-codebase tests](/about/pykrete-tests/) for the methodology and the [pykrete-tests README](https://github.com/amirnaderi93/pykrete-tests#the-donors) for the per-donor matrix. Each push rebuilds pykrete fresh from `main`, re-runs `pykrete check` against every fixture, and JSON-diffs the output against the committed golden — any drift fails the build before it gets released.

On top of the golden-diff suite, we run **130 schema-tracking probes** that verify pykrete actually tracks columns through real transforms. The 130 probes cover 46 of the 47 vendored fixtures (the feast `spark_kafka_processor` streaming fixture is annotated but probe-free, since it has no typed-DataFrame slot a probe can anchor to): 113 positive probes across 34 of the 35 annotated fixtures assert columns resolve cleanly after `.select` / `.filter` / `.withColumn`; 17 negative probes across all 12 deliberately-corrupted fixtures under `probes_negative/` assert specific diagnostics — D0030 `unknownColumn`, D0081 `nonNumericArithmetic`, D0082 `crossTypeComparison`, and D0084 `enumValueMismatch` — actually fire. Together, the suite verifies three properties on every release: **column resolution + diagnostic firing + type tracking (scoped to D0081 via the `PROBE-TYPE-IS` synth shape in v1.2)**.

We verify enum value vocabularies in 3 of the 10 donors: Delta CDC `_change_type` (`{"insert", "update_preimage", "update_postimage", "delete"}`), Hudi `_hoodie_operation` (`{"I", "-U", "U", "D"}`), and MLflow run status (`{"RUNNING", "FINISHED", "FAILED", "KILLED", "SCHEDULED"}`). Positive probes assert in-vocab literals stay clean across `==` / `.isin` / `withColumn` / `F.expr` / `groupBy` chains; negative probes assert D0084 fires on off-vocab typos.

**New in v1.2: type-tracking coverage in 3 of the 10 donors** — quinn, MLflow, and python-deequ — via the `PROBE-TYPE-IS` synthesizer. The v1.1 reservation declared the marker syntax but the runner couldn't anchor the synthesized type assertion to the live local scope; under v1.2 the synth wraps the assertion in `{df}.select(...)`, binding `col(...)` to the typed DataFrame in scope so off-claim markers fire D0081. The synth shape is scoped to D0081 in v1.2 — D0080 (`returnTypeMismatch`) and D0082 (`crossTypeComparison`) keep their falsifiability through raw-mutation fixtures in the test suite (`V12CrossCodebaseMarkerMutationTests`) until v1.3 brings them under the synth gate. A new CI gate, `V12FalsifiabilityCoverageGuard`, mutates the claimed type on every `PROBE-TYPE-IS` marker and verifies D0081 fires, so the new coverage can't silently regress.

CI fails if any probe asserts the wrong outcome. The probe runner and marker grammar are documented in [`scripts/PROBES.md`](https://github.com/amirnaderi93/pykrete-tests/blob/main/scripts/PROBES.md).

What the v1.2 probe suite does **not** yet verify, all tracked for v1.3:

- **`PROBE-TYPE-IS` synth-shape coverage beyond D0081.** The current synth (`{df}.select(col("x") + 1)`) falsifies on non-numeric. D0080 and D0082 need their own synth shapes; raw-mutation fixtures cover them in the interim.
- Numeric-subtype distinguishability (`int` vs `long` vs `short` arithmetic narrowing). Carried forward from v1.1.
- **withColumn output enum-constraint preservation.** Carried forward from v1.1: the literal is checked against the sink's enum vocabulary, but the constraint drops on the output column — so a downstream `==` comparison against an off-vocab literal on the rewritten column will not fire D0084. Tracker in `docs/design/literal-value-vocabulary.md` polish backlog.

Gaps closed in earlier releases (`df["X"]` subscript, GroupedData shortcut aggregates, chained nested-field access, `intersect` / `subtract` / `exceptAll`, lowercase `groupby`) all have regression tests in `crates/pykrete/tests/`. They can't reopen silently.

## Known limitations

By design, pykrete does not model:

- **Structured streaming** (`readStream`, `writeStream`, `isStreaming`).
- **RDD-level operations** (`rdd`, `mapPartitions`, `foreach`).
- **Pandas-on-Spark and Arrow conversions** (`toPandas`, `toArrow`, `mapInPandas`, `pandas_api`). Pandas support is on the [roadmap](/about/roadmap/) as its own typed surface.

The full unmodeled list, with the rationale for each, is in [Operations → What's not modeled — by design](/reference/operations/#whats-not-modeled--by-design).

## Production deployments

Pykrete is a development-time checker — it does not ship to production hosts and cannot affect a running pipeline. The public, reproducible cross-testing coverage lives in [pykrete-tests](/about/pykrete-tests/), which vendors 47 fixtures (35 annotated + 12 deliberately-corrupted under `probes_negative/`) across the 10 donors listed above; the 130 schema-tracking probes that run on every release cover 46 of those (34 annotated + 12 negative — the feast `spark_kafka_processor` streaming fixture is annotated but probe-free). Each fixture is pinned to a specific upstream commit (see the [per-donor matrix](https://github.com/amirnaderi93/pykrete-tests#the-donors) in the pykrete-tests README) so the coverage is reproducible — anyone can `pip install` the same upstream code pykrete is being tested against. Named external adopter references will be added here as teams give the go-ahead.

See the [Reliability and trust](https://github.com/amirnaderi93/pykrete#reliability-and-trust) section of the README for the full story.
