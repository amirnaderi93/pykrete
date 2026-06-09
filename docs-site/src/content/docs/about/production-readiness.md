---
title: Production readiness
description: Stability commitments, false-positive policy, release cadence, and known limitations for production PySpark teams evaluating pykrete.
---

## TL;DR

pykrete is feature-complete for PySpark as of the [v0.1 release line](https://github.com/amirnaderi93/pykrete/releases). v1.3 added pandas check-site coverage via the `PandasFrame[X]` dialect dispatch; v1.4 added depth on pandas — 7 new pandas-heavy donors (scikit-learn, statsmodels, pandera, Great Expectations, prophet, seaborn, yfinance), 21 pandas `PROBE-TYPE-IS` markers proving dtype tracking on `PandasFrame[X]`, and three checker bug closures. v1.5 adds **cross-dialect handoff**: `df.toPandas()` re-tags `SparkFrame[X]` to `PandasFrame[X]`; `spark.createDataFrame(pdf)` re-tags `PandasFrame[Y]` back to `SparkFrame[Y]` when a `schema=` argument or a typed call-arg resolves to a known schema; the round-trip path preserves the tag. Pandas `.head()` / `.tail()` / `.first()` are dialect-gated as Spark-only terminals, so pandas chains downstream of them keep tracking. `.loc[:, "col"]` literal-form lands. Two PR-F1-class sibling gates close (`column_name_arg` ungated arms + `collect_col_refs` cross-DataFrame routing). A new `pykrete check --report-aliases` flag emits a structured JSON envelope of every `DataFrame[X]` annotation site with its resolved dialect, so projects can quantify the v2.0 migration scope before v1.6's `pykrete migrate` ships. A deliberate "degrade to Unknown rather than fabricate" policy keeps the checker honest: when pykrete can't determine a schema or a type with confidence, it stops checking that subtree rather than guessing. A real-codebase integration loop ([pykrete-tests](/about/pykrete-tests/)) catches regressions before they ship.

For the trust posture behind the engineering — why pykrete cannot break a production pipeline, and how each release is validated — see the [Reliability and trust](https://github.com/amirnaderi93/pykrete#reliability-and-trust) section of the README.

## Stability commitments

Once a piece of surface ships in a release, the project commits to backward-compatible behavior on the following:

- **Schema declaration syntax.** `Schema` classes, `Optional[T]` for nullable columns, the `Array` / `Map` / struct-class nested-type forms, and the TypeScript-style schema operators (`Pick`, `Omit`, `Merge`).
- **The dataframe annotation surface.** `SparkFrame[Schema]` and `PandasFrame[Schema]` (canonical, v1.3+), and `DataFrame[Schema]` (deprecated alias for `SparkFrame[Schema]`, stable through v1.x and removed in v2.0). Variable annotations, function parameter and return types, `.cast(SparkFrame[Schema])` re-anchors.
- **Diagnostic codes.** `D0001`, `D0010`, `D0011`, `D0020`, `D0021`, `D0030`, `D0040`, `D0050`, `D0051`, `D0060`, `D0070`, `D0071`, `D0072`, `D0073`, `D0080`, `D0081`, `D0082`, `D0083`, `D0084`, `D0090`. The numeric code and the rule name are part of the contract; the diagnostic message text is not. `D0090 deprecatedDataFrameAlias` is new in v1.3 and warns when `DataFrame[X]` is used as the dialect-less alias for `SparkFrame[X]`; the alias is removed in v2.0.
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

- `spark.read.parquet("s3://...")` returns Unknown until the user re-anchors with `.cast(SparkFrame[Schema])` or a typed variable annotation. The schema is genuinely runtime data; pykrete won't invent one.
- `F.struct(F.lit(1))` falls back to positional names (`col1`, `col2`, …) when no `.alias("x")` is present, rather than fabricating a guessed field name. Heterogeneous value types in `melt` / `unpivot` degrade the value-column type to Unknown rather than picking a "winner".

The same rule applies at the generic-inference layer: a TypeVar bound to incompatible schemas across argument slots stays Unknown. Downstream checks against an Unknown subtree are permissive: no diagnostic fires unless the user re-anchors.

A static checker that cries wolf gets switched off; pykrete prefers to stay quiet when it isn't sure.

## Release cadence

The Spark-coverage closure sprint (v0.1.7 onward, May 2026) ran at multiple releases per week — the finishing pass on the v1.0.0 surface, not a steady-state cadence. v1.0.0 shipped, v1.1/v1.2 added the schema-tracking probe suite and PROBE-TYPE-IS, v1.3 added pandas check-site coverage, v1.4 added pandas depth (7 new donors + pandas type-tracking probes), and v1.5 added cross-dialect handoff between Spark and pandas; the cadence is now a more measured per-feature pace as the focus moves to `pykrete migrate` paired with D0090 strict-mode escalation, broader pandas reshape, the `.query` / `.eval` mini-DSLs, and polars. See the [GitHub Releases page](https://github.com/amirnaderi93/pykrete/releases) for the full per-release history.

## Real-codebase testing

Every release is regression-tested against **93 fixtures from 17 upstream codebases** (46 annotated + 47 deliberately-corrupted under `probes_negative/`). The 10 PySpark donors cover the dominant Spark stack: Apache Spark, Delta Lake, Apache Iceberg ([iceberg-python](https://github.com/apache/iceberg-python)), Apache Hudi, MLflow, Feast, Kedro ([kedro-plugins](https://github.com/kedro-org/kedro-plugins)), [quinn](https://github.com/MrPowers/quinn), [dbt-spark](https://github.com/dbt-labs/dbt-spark), and [python-deequ](https://github.com/awslabs/python-deequ). The 10 pandas-coverage donors split into three honest scoping classes: **3 hybrid** (MLflow, Feast, iceberg-python) carry pandas fixtures on top of their Spark coverage; **3 direct-dispatch** ([prophet](https://github.com/facebook/prophet), [seaborn](https://github.com/mwaskom/seaborn), [yfinance](https://github.com/ranaroussi/yfinance)) annotate the actual upstream library code where pykrete's dispatched-shape recognizers match real call sites; **4 canonical-fixture-only** ([scikit-learn](https://github.com/scikit-learn/scikit-learn), [statsmodels](https://github.com/statsmodels/statsmodels), [pandera](https://github.com/unionai-oss/pandera), [Great Expectations](https://github.com/great-expectations/great_expectations)) ship user-pattern fixtures inspired by each library's API — the upstream code itself operates at numpy / metric layers above raw pandas dispatch, so the fixtures stand in for what a real user writes at the pandas boundary. See [Real-codebase tests](/about/pykrete-tests/) for the methodology and the [pykrete-tests README](https://github.com/amirnaderi93/pykrete-tests#the-donors) for the per-donor matrix. Each push rebuilds pykrete fresh from the pinned source-commit, re-runs `pykrete check` against every fixture, and JSON-diffs the output against the committed golden — any drift fails the build before it gets released.

On top of the golden-diff suite, we run **233 schema-tracking probes** that verify pykrete actually tracks columns through real transforms. The 233 probes cover 92 of the 93 vendored fixtures (the feast `spark_kafka_processor` streaming fixture is annotated but probe-free, since it has no typed-DataFrame slot a probe can anchor to): 180 positive probes across 45 of the 46 annotated fixtures assert columns resolve cleanly after `.select` / `.filter` / `.withColumn` and the pandas analogues, AND that dtype claims on `SparkFrame[X]` / `PandasFrame[X]` parameters survive dispatched chains (24 `PROBE-TYPE-IS` markers across 10 of the 17 donors — the v1.2 Spark side and the v1.4 pandas side, closing pykrete-tests#14); 53 negative probes across 47 deliberately-corrupted fixtures under `probes_negative/` assert specific diagnostics — D0030 `unknownColumn`, D0060 `missingJoinKey`, D0081 `nonNumericArithmetic` (v1.4 widens to subscript-on-name receivers), D0082 `crossTypeComparison` (widened correspondingly), D0084 `enumValueMismatch`, and D0090 `deprecatedDataFrameAlias` — actually fire. Together, the suite verifies four properties on every release: **column resolution + diagnostic firing + Spark type tracking + pandas type tracking**.

We verify enum value vocabularies in 3 of the 17 donors: Delta CDC `_change_type` (`{"insert", "update_preimage", "update_postimage", "delete"}`), Hudi `_hoodie_operation` (`{"I", "-U", "U", "D"}`), and MLflow run status (`{"RUNNING", "FINISHED", "FAILED", "KILLED", "SCHEDULED"}`). Positive probes assert in-vocab literals stay clean across `==` / `.isin` / `withColumn` / `F.expr` / `groupBy` chains; negative probes assert D0084 fires on off-vocab typos.

**New in v1.5: cross-dialect handoff between Spark and pandas, plus deferred-promise closure.** v1.3 shipped check-site dispatch on the six pandas operations; v1.4 closed the type-tracking gap and broadened the pandas donor surface. v1.5 closes the cross-dialect boundary: `df.toPandas()` on a `SparkFrame[X]` receiver re-tags the chain to `PandasFrame[X]`, so a downstream `pdf["typo"]` fires D0030 against `X`; `spark.createDataFrame(pdf)` re-tags `PandasFrame[Y]` back to `SparkFrame[Y]` when either a `schema=` keyword argument or the call-arg expression resolves to a known schema (with neither present, the call falls through to Unknown — no auto-inference from raw values). The round-trip `spark.createDataFrame(df.toPandas())` preserves the tag end-to-end. Pandas `.head()` / `.tail()` / `.first()` are dialect-gated so `pdf.head(10).merge(other, on="id")` keeps tracking. `.loc[:, "col"]` literal-form lands (non-literal forms — boolean-mask row keys, column-range slicing, `.iloc[...]` — fall through and are deferred to v1.6). Two PR-F1-class sibling gates close: `column_name_arg`'s attribute + subscript arms now gate on the receiver being a DataFrame binding (fixes `df.groupBy(bag.x)` where `bag` is a non-DataFrame plain Python dict), and `collect_col_refs` threads the receiver name through to schema-lookup callers so `df.select(df_other["col"])` collects `"col"` against `df_other`'s schema instead of `df`'s. A new `pykrete check --report-aliases` flag emits a JSON envelope of every `DataFrame[X]` annotation site with its resolved dialect, so projects can quantify the v2.0 migration scope before v1.6's `pykrete migrate` ships. The LSP synthetic-pool gets a soft cap with one-shot warning and saturation sentinel, closing the v1.4 architecture-audit I4 finding.

CI fails if any probe asserts the wrong outcome. The probe runner and marker grammar are documented in [`scripts/PROBES.md`](https://github.com/amirnaderi93/pykrete-tests/blob/main/scripts/PROBES.md).

What the v1.5 probe suite does **not** yet verify, all tracked for v1.6+:

- **`pykrete migrate` binary + D0090 strict-mode escalation** (paired, non-negotiable v1.6 commitment). v1.5 ships `--report-aliases` as the visibility slice; v1.6 ships the auto-rewriter framework and lights D0090 as error under strict mode in the same release.
- **`.loc[mask, "col"]` (boolean mask), `.loc[:, "a":"b"]` (column range), `pdf.iloc[...]`** — deferred to v1.6, paired with broader pandas reshape.
- **`df.query("…")` / `df.eval("…")` mini-DSLs.** Own design surface; numexpr-influenced syntax, separate parser from the SQL path used by `selectExpr`.
- **Broader pandas method modeling** (`pivot_table`, `groupby.agg`, `melt`, `stack` / `unstack`, `reset_index`, `set_index`).
- **`pd.read_csv(...)` and other pandas I/O entry points.** Schema inference from file headers / SQL / type-stubs is a separate design surface.
- **`PROBE-TYPE-IS` synth-shape coverage beyond D0081 (Spark side).** The current synth (`{df}.select(col("x") + 1)`) falsifies on non-numeric. D0080 (`returnTypeMismatch`) and D0082 (`crossTypeComparison`) need their own synth shapes; raw-mutation fixtures cover them in the interim.
- Numeric-subtype distinguishability (`int` vs `long` vs `short` arithmetic narrowing). Carried forward from v1.1.
- **withColumn output enum-constraint preservation.** Carried forward from v1.1: the literal is checked against the sink's enum vocabulary, but the constraint drops on the output column — so a downstream `==` comparison against an off-vocab literal on the rewritten column will not fire D0084. Tracker in `docs/design/literal-value-vocabulary.md` polish backlog.

Of the cross-dialect closures shipped in v1.5, only `.toPandas()` (PR-A1) has load-bearing cross-codebase probe coverage today. PR-A2's `createDataFrame(pdf)` Gate (b), PR-A3's pandas `.head/.tail/.first` chain-survival, PR-B1's cross-DataFrame `df.select(df_other["col"])` routing, PR-B2's `column_name_arg` DataFrame-binding gate, and PR-C's `.loc[:, "col"]` literal-form ship with unit-test coverage only; the v1.6 probe batch lands cross-codebase exercise for each.

Gaps closed in earlier releases (`df["X"]` subscript, GroupedData shortcut aggregates, chained nested-field access, `intersect` / `subtract` / `exceptAll`, lowercase `groupby`) all have regression tests in `crates/pykrete/tests/`. They can't reopen silently.

## Known limitations

By design, pykrete does not model:

- **Structured streaming** (`readStream`, `writeStream`, `isStreaming`).
- **RDD-level operations** (`rdd`, `mapPartitions`, `foreach`).
- **Cross-dialect handoffs beyond `.toPandas()` and `spark.createDataFrame(pdf)`.** v1.5 closes the two principal handoff paths (`SparkFrame[X]` → `PandasFrame[X]` via `.toPandas()`, and `PandasFrame[Y]` → `SparkFrame[Y]` via `spark.createDataFrame(pdf)` when a schema source is present). Other cross-dialect interop — `.toArrow`, `mapInPandas`, `pandas_api`, `applyInPandas` — remains opaque by design; these cross into pandas-on-Spark / Arrow tables, not vanilla pandas. See the [roadmap](/about/roadmap/) and the [pandas roadmap](/about/pandas-roadmap/) for the pandas-specific trajectory.

The full unmodeled list, with the rationale for each, is in [Operations → What's not modeled — by design](/reference/operations/#whats-not-modeled--by-design).

## Production deployments

Pykrete is a development-time checker — it does not ship to production hosts and cannot affect a running pipeline. The public, reproducible cross-testing coverage lives in [pykrete-tests](/about/pykrete-tests/), which vendors 93 fixtures (46 annotated + 47 deliberately-corrupted under `probes_negative/`) across 17 donors (10 PySpark + 10 pandas-coverage, three of which are hybrid carry-overs from v1.3); the 233 schema-tracking probes that run on every release cover 92 of those (45 of the 46 annotated + all 47 negative — the feast `spark_kafka_processor` streaming fixture is annotated but probe-free). Each fixture is pinned to a specific upstream commit (see the [per-donor matrix](https://github.com/amirnaderi93/pykrete-tests#the-donors) in the pykrete-tests README) so the coverage is reproducible — anyone can `pip install` the same upstream code pykrete is being tested against. Named external adopter references will be added here as teams give the go-ahead.

See the [Reliability and trust](https://github.com/amirnaderi93/pykrete#reliability-and-trust) section of the README for the full story.
