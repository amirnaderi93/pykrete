<p align="center"><img src="docs/assets/logo.svg" width="150" alt="pykrete logo"></p>

<h1 align="center">pykrete</h1>

<p align="center">
  <strong>Python dataframes, done right.</strong><br>
  Static type checking for dataframe schemas.
</p>

<p align="center">
  <a href="https://github.com/amirnaderi93/pykrete/actions/workflows/ci.yml"><img src="https://github.com/amirnaderi93/pykrete/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/amirnaderi93/pykrete-tests/actions/workflows/cross-codebase.yml"><img src="https://github.com/amirnaderi93/pykrete-tests/actions/workflows/cross-codebase.yml/badge.svg" alt="pykrete-tests cross-codebase"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
</p>

<p align="center">
  <a href="https://amirnaderi93.github.io/pykrete/"><strong>Docs</strong></a> ·
  <a href="https://amirnaderi93.github.io/pykrete/getting-started/install/">Install</a> ·
  <a href="https://amirnaderi93.github.io/pykrete/getting-started/quickstart/">Quickstart</a> ·
  <a href="docs/roadmap.md">Roadmap</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

pykrete is a strict superset of Python that adds a type layer for dataframes. Define a schema as a class, annotate a dataframe with `SparkFrame[Schema]` (or `PandasFrame[Schema]`), and pykrete checks every column you touch — at edit time, before your job runs. The runtime is plain Python; the type layer never executes. It's the same idea TypeScript brings to JavaScript, applied to dataframe code. (`DataFrame[Schema]` still works as a deprecated alias — it emits D0090; prefer the dialect-specific form.)

<p align="center">
  <img src="docs/assets/showcase-column-typos.png" alt="pykrete catching a misspelled column name in the editor — diagnostic: 'regoin' does not exist on schema 'Sale'. Did you mean 'region'?" width="720">
  <br>
  <sub><em>A column typo, caught at edit time.</em></sub>
</p>

<p align="center">
  <img src="docs/assets/showcase-schema-flow.png" alt="pykrete tracks the schema through a transformation chain — hovering the result shows the derived schema (region: string, total: long)." width="720">
  <br>
  <sub><em>The schema flows through every transform — hover the result to see the derived shape.</em></sub>
</p>

## Install

macOS / Linux:

```bash
brew install amirnaderi93/pykrete/pykrete
```

Windows: the [latest release](https://github.com/amirnaderi93/pykrete/releases/latest) ships an MSI installer. Other options — prebuilt binaries, `cargo install` — are in the [install guide](https://amirnaderi93.github.io/pykrete/getting-started/install/).

Each install gives you two binaries: `pykrete` (the checker, for CLI and CI) and `pykrete-lsp` (the language server for your editor).

## What you get

- **Typos caught as you type** — every column reference, against the schema in scope, with a _did you mean_.
- **Checks that follow the data** — `select` / `filter` / `withColumn` / `drop` / `join` / `groupBy` + `agg` / `pivot` / `union` and the rest; a reference to a column three transforms after it was dropped is still caught.
- **Schema visibility** — hover a `SparkFrame[…]` or `PandasFrame[…]` parameter to see its columns; go-to-definition jumps to the schema.
- **TypeScript-style schema composition** — `Pick`, `Omit`, and `Merge` build derived shapes without redeclaring columns.
- **Column-name autocomplete** in string arguments.
- **Inline SQL checked too** — identifiers inside `filter("…")`, `selectExpr(...)`, `spark.sql("SELECT …")`.
- **Zero runtime cost** — `.pyk` is a strict superset of Python; the deployed job is ordinary Python.

## Reliability and trust

Pykrete is a development-time checker, not a runtime dependency. Your `.pyk` files transpile to plain Python — Spark runs the same Python it always did. **Pykrete cannot break a production pipeline because pykrete is not in the production pipeline.**

What this means in practice:

- **Static analysis runs in your editor, pre-commit hook, or CI.** The `pykrete` binary never ships to production hosts.
- **The transpile step is small and well-defined.** It prepends `from __future__ import annotations` and strips the pykrete-only `.cast(SparkFrame[Schema])` re-anchor calls — these exist purely to help the checker; PySpark's `DataFrame` has no `.cast` method, so the call would `AttributeError` at runtime regardless. Everything else is copied verbatim, line numbers preserved. Run `pykrete transpile path/to/file.pyk` and diff against the source to see exactly what changed.
- **Adopting pykrete is reversible.** Run `pykrete transpile` once to bake the `.pyk` → `.py` rewrite into your repo, commit the result, and you've vendored your way off pykrete. No runtime dependency to remove (there isn't one), no binary on production hosts (there never was one).

### How we earn confidence

We hold pykrete to PySpark's standard because that's the standard that matters:

- **Cross-tested against the real PySpark and pandas stack on every release.** The [pykrete-tests](https://github.com/amirnaderi93/pykrete-tests) repo vendors **`120 fixtures` from `17 donors`** (49 annotated + 71 deliberately-corrupted under `probes_negative/`). The 10 PySpark donors — Apache Spark itself, Delta Lake, Apache Iceberg ([iceberg-python](https://github.com/apache/iceberg-python)), Apache Hudi, MLflow, Feast, Kedro ([kedro-plugins](https://github.com/kedro-org/kedro-plugins)), [quinn](https://github.com/MrPowers/quinn), [dbt-spark](https://github.com/dbt-labs/dbt-spark), and [python-deequ](https://github.com/awslabs/python-deequ) — cover the dominant Spark stack. The 10 pandas-coverage donors split into three honest scoping classes: **3 hybrid** (MLflow, Feast, iceberg-python) carry pandas fixtures on top of their existing Spark coverage; **3 direct-dispatch** ([prophet](https://github.com/facebook/prophet), [seaborn](https://github.com/mwaskom/seaborn), [yfinance](https://github.com/ranaroussi/yfinance)) annotate the actual upstream library code where pykrete's dispatched-shape recognizers match real call sites; **4 canonical-fixture-only** ([scikit-learn](https://github.com/scikit-learn/scikit-learn), [statsmodels](https://github.com/statsmodels/statsmodels), [pandera](https://github.com/unionai-oss/pandera), [Great Expectations](https://github.com/great-expectations/great_expectations)) ship user-pattern fixtures inspired by each library's API — the upstream code itself operates at numpy / metric layers above raw pandas dispatch, so the fixtures stand in for what a real user writes at the pandas boundary. Each donor's PySpark or pandas code is preserved verbatim under `upstream/`, paired with an `annotated/` companion that adds the pykrete schema declarations an adopter would write; pykrete runs against the annotated form. Each push rebuilds pykrete from the catalog-pinned source commit (`scripts/diagnostic_catalog.json`'s `pykreteSourceCommit`), re-runs `pykrete check` against every fixture, and JSON-diffs the output against the committed golden — a new false positive on real Spark or pandas code blocks the release before the tag goes out.
- **Schema tracking is verified, not assumed.** On top of the golden-diff suite, we run **`261 probes`** that assert pykrete is actually tracking columns through real transforms. The probes cover the probe-anchored fixtures of the 120 vendored (a small number of streaming or import-only fixtures are annotated but probe-free, since they have no typed-DataFrame slot a probe can anchor to): `186 positive` probes across 49 annotated fixtures verify columns resolve cleanly after `.select` / `.filter` / `.withColumn` and the pandas analogues, AND that dtype claims on `SparkFrame[X]` / `PandasFrame[X]` parameters survive dispatched chains (24 `PROBE-TYPE-IS` markers across 10 of the `17 donors` — the v1.2 Spark side and the v1.4 pandas side, closing pykrete-tests#14); `75 negative` probes across 71 deliberately-corrupted fixtures verify specific diagnostics — D0030 `unknownColumn`, D0040 / D0050 / D0051 (cross-codebase coverage added in v1.7), D0060 `missingJoinKey`, D0073 `transformInputMismatch` (cross-codebase coverage added in v1.8 per pykrete-tests PR-P1 #30), D0081 `nonNumericArithmetic` (v1.4 widens it to subscript-on-name receivers), D0082 `crossTypeComparison` (v1.4 widens correspondingly), D0083 `nullabilityMismatch` (cross-codebase coverage added in v1.8 per pykrete-tests PR-P1 #30), D0084 `enumValueMismatch`, D0090 `deprecatedDataFrameAlias`, and D0091 `crossDialectMethodMismatch` (cross-codebase coverage added in v1.9 per pykrete-tests PR-P1 #32 on pandera + delta, extended in v1.10 PR-P1 with D0091 strict-mode escalation + bare-attribute + shape-changes probes on `mlflow` / `dbt-spark` / `pandera` / `delta`) — actually fire. Together, these verify four properties on every release: **column resolution + diagnostic firing + Spark type tracking + pandas type tracking**. We verify enum value vocabularies in 3 of `17 donors`: Delta CDC `_change_type` (`{"insert", "update_preimage", "update_postimage", "delete"}`), Hudi `_hoodie_operation` (`{"I", "-U", "U", "D"}`), and MLflow run status (`{"RUNNING", "FINISHED", "FAILED", "KILLED", "SCHEDULED"}`). **New in v1.10**: `pykrete check --deprecation-report --snapshot=<path>` writes the v2 envelope to disk so CI can persist a prior-report cache and diff between releases (atomic write, exact-keys allowlist on the persisted file); `--fail-on-nonempty` exits non-zero when the envelope's `sites` array is non-empty, replacing the `jq | test` boilerplate adopters were writing by hand. D0091 surface completes: `SPARK_DISCRIMINATOR_PROPERTIES` adds `na`, `write`, `writeStream`, `storageLevel`; `PANDAS_INHERITED_PROPERTIES` adds `index`, `values`, `shape`, `T` — both via the v1.9 bare-attribute path. The pandas `df.stack(level=, dropna=)` literal-form arm lands (continuing the one-reshape-arm-per-cycle cadence from v1.6 `pivot_table` and v1.7 `melt`). v1.10 PR-D1's 8 new D0091 properties (`na`, `write`, `writeStream`, `storageLevel`, `index`, `values`, `shape`, `T`) are unit-test-covered at v1.10.0; cross-codebase fixture probes filed for v1.11. D0080 `returnTypeMismatch` and D0082 `crossTypeComparison` keep their falsifiability through raw-mutation fixtures in the test suite until dedicated synth shapes ship. Numeric-subtype distinguishability (`int` vs `long` vs `short`) and `withColumn` output enum-constraint preservation are carried forward in the polish backlog. CI fails if any probe asserts the wrong outcome. See the [pykrete-tests README](https://github.com/amirnaderi93/pykrete-tests#schema-tracking-probes-v14) and [`scripts/PROBES.md`](https://github.com/amirnaderi93/pykrete-tests/blob/main/scripts/PROBES.md) for the methodology; per-donor matrix and pinned commits live in the [donors table](https://github.com/amirnaderi93/pykrete-tests#the-donors).
- **`1738 tests` in CI across the analyzer, LSP, and wasm crates.** Every release has to pass the full suite, plus per-D-code snapshot tests that pin every error message — wording drifts fail the build until explicitly accepted.
- **JSON output is a stability contract from v1.0.0.** Field names, types, semantics, and D-code identity will not change without a SemVer-major bump and a corresponding `schemaVersion` bump. See [Production readiness → JSON output stability contract](https://amirnaderi93.github.io/pykrete/about/production-readiness/#json-output-stability-contract).
- **No-false-positives policy.** When pykrete cannot determine a schema or a type with confidence, it stops checking that subtree rather than guessing. A static checker that cries wolf gets switched off.
- **Pre-major-release audit cycle.** Every X.0.0 bump runs three independent fresh-eyes audits (architecture, Spark coverage, docs sync) before the tag. Findings ship in the release notes.
- **One-command v2.0 migration, plannable AND archivable in CI.** With v1.7, **v2.0 is one command away**: `pykrete migrate src/` previews; `pykrete migrate --apply src/` rewrites. With v1.8, **v2.0 readiness is measurable**: `pykrete check --deprecation-report src/` emits a JSON envelope listing every D0090-firing site with its adjudicated dialect and suggested rewrite. With v1.9, **v2.0 migration is plannable**: the envelope bumps to `deprecationReportVersion: "2"` with per-site `migrationStatus` (`pending` / `acknowledged`) driven by a `# pykrete: ack-deprecation` comment marker, plus a `--ack=<pending|acknowledged>` filter that narrows the envelope to one cohort for CI gating. With v1.10, **v2.0 migration is archivable**: `pykrete check --deprecation-report --snapshot=<path>` writes the v2 envelope to disk so CI can persist a prior-report cache and diff between releases, and `--fail-on-nonempty` exits non-zero when `sites` is non-empty — drop `pykrete check --deprecation-report --fail-on-nonempty src/` into CI to fail on any unacked site without the `jq | test` boilerplate. The envelope deliberately ships without `targetVersion` / `removalVersion` / `shipDate` — pykrete tracks per-site migration progress; the user picks the v2.0 ship date. `DataFrame[X]` is a deprecated alias slated for removal in a future pykrete v2.0. v1.6 shipped `pykrete migrate` — it walks each binding's downstream usage, classifies it as Spark / pandas / ambiguous via call-graph adjudication, and rewrites the annotation in place to the dialect-tagged canonical name (atomic per file, token-preserving). v1.7 flips the default to `--check` (preview verdicts on stdout) and adds `--apply` as the opt-in for the in-place rewrite. Under `"typeCheckingMode": "strict"`, D0090 escalates from warning to error so strict-mode projects gate on the migration; v1.9 lands the same strict-mode escalation for D0091 `crossDialectMethodMismatch`. Non-strict modes keep the warning unchanged. See [Migrating to v2.0](https://amirnaderi93.github.io/pykrete/cookbook/#6-migrate-dataframex-to-the-v20-dialect-tagged-names) for the cookbook recipe.

If pykrete is wrong on your code, [open an issue](https://github.com/amirnaderi93/pykrete/issues) — false positives are triaged ahead of everything else.

## Editor integration

The **VS Code extension** gives you live diagnostics, hover, completion, and go-to-definition on `.pyk` files. Search **pykrete** in the Extensions panel — it's on the [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=amirnaderi.pykrete) (VS Code) and the [Open VSX Registry](https://open-vsx.org/extension/amirnaderi/pykrete) (Cursor, VSCodium, code-server, Theia).

For Neovim, Helix, Emacs, and other LSP clients, see [docs/editors/](docs/editors/).

## Documentation

Full documentation at **[amirnaderi93.github.io/pykrete](https://amirnaderi93.github.io/pykrete/)** — schema reference, diagnostic catalog, how it works, the roadmap.

Today: PySpark (feature-complete), pandas check-site coverage (v1.3), pandas depth (v1.4), cross-dialect handoff between Spark and pandas (v1.5 — `.toPandas()` / `spark.createDataFrame(pdf)`, dialect-gated `.head` / `.tail` / `.first`, `.loc[:, "col"]` literal-form, and a `--report-aliases` JSON envelope for sizing the v2.0 `DataFrame[X]` migration), the `pykrete migrate` auto-rewriter with call-graph dialect adjudication + D0090 strict-mode escalation + pandas `pivot_table` literal-form column checking + `.take()` dialect-gate closure (v1.6), the v1.7 migrator UX hardening (`pykrete migrate` defaults to `--check`; `--apply` opts into the rewrite) + pandas `df.melt(...)` literal-form + `dialect_signals` shared module + Spark-D1 audit-debt closure (14 new `SPARK_DISCRIMINATORS`), the v1.8 v2.0-readiness surface (`pykrete check --deprecation-report` JSON envelope + D0090 message amend + new `D0091 crossDialectMethodMismatch` warning + `build.rs`-generated inventory + CHANGELOG-binary CI gate), the v1.9 v2.0-plannability surface (`--deprecation-report` v2 envelope with per-site `migrationStatus` + `--ack` filter + D0091 strict-mode escalation + D0091 bare-attribute inference arm + `text-numeric` CHANGELOG gate), and the v1.10 v2.0-archivability surface (`--snapshot=<path>` file-write + `--fail-on-nonempty` CI gate + D0091 8-property surface completion (`na` / `write` / `writeStream` / `storageLevel` on the Spark side; `index` / `values` / `shape` / `T` on the pandas side) + pandas `df.stack(level=, dropna=)` literal-form). Next: full `pivot_table` / `melt` output schema-tracking + broader pandas reshape (`unstack` / `groupby.agg` / `reset_index` / `set_index`), `.loc` non-literal forms + `.iloc`, the `.query` / `.eval` mini-DSLs, pandas I/O entry points, an `--include-py` flag for `pykrete migrate`, a `--changed-only` flag, and polars — see the [roadmap](docs/roadmap.md).

## Repository layout

- [`crates/`](crates/) — the Rust workspace: `pykrete` (checker + CLI) and `pykrete-lsp` (language server).
- [`editors/vscode/`](editors/vscode/) — the VS Code extension.
- [`docs/`](docs/) — design docs, the roadmap, and the source for the docs site.
- [`docs-site/`](docs-site/) — the Astro + Starlight documentation site.
- [`examples/`](examples/) — sample `.pyk` files.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow — feature branches, Conventional Commits, CI, and the pull-request process.

## License

MIT. See [LICENSE](LICENSE).
