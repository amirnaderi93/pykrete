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

pykrete is a strict superset of Python that adds a type layer for dataframes. Define a schema as a class, annotate a dataframe with `DataFrame[Schema]`, and pykrete checks every column you touch — at edit time, before your job runs. The runtime is plain Python; the type layer never executes. It's the same idea TypeScript brings to JavaScript, applied to dataframe code.

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
- **Schema visibility** — hover a `DataFrame[…]` parameter to see its columns; go-to-definition jumps to the schema.
- **TypeScript-style schema composition** — `Pick`, `Omit`, and `Merge` build derived shapes without redeclaring columns.
- **Column-name autocomplete** in string arguments.
- **Inline SQL checked too** — identifiers inside `filter("…")`, `selectExpr(...)`, `spark.sql("SELECT …")`.
- **Zero runtime cost** — `.pyk` is a strict superset of Python; the deployed job is ordinary Python.

## Reliability and trust

Pykrete is a development-time checker, not a runtime dependency. Your `.pyk` files transpile to plain Python — Spark runs the same Python it always did. **Pykrete cannot break a production pipeline because pykrete is not in the production pipeline.**

What this means in practice:

- **Static analysis runs in your editor, pre-commit hook, or CI.** The `pykrete` binary never ships to production hosts.
- **The transpile step is small and well-defined.** It prepends `from __future__ import annotations` and strips the pykrete-only `.cast(DataFrame[Schema])` re-anchor calls — these exist purely to help the checker; PySpark's `DataFrame` has no `.cast` method, so the call would `AttributeError` at runtime regardless. Everything else is copied verbatim, line numbers preserved. Run `pykrete transpile path/to/file.pyk` and diff against the source to see exactly what changed.
- **Adopting pykrete is reversible.** Run `pykrete transpile` once to bake the `.pyk` → `.py` rewrite into your repo, commit the result, and you've vendored your way off pykrete. No runtime dependency to remove (there isn't one), no binary on production hosts (there never was one).

### How we earn confidence

We hold pykrete to PySpark's standard because that's the standard that matters:

- **Cross-tested against the real PySpark and pandas stack on every release.** The [pykrete-tests](https://github.com/amirnaderi93/pykrete-tests) repo vendors **59 fixtures from 10 upstream codebases** (38 annotated + 21 deliberately-corrupted under `probes_negative/`) — Apache Spark itself, Delta Lake, Apache Iceberg ([iceberg-python](https://github.com/apache/iceberg-python)), Apache Hudi, MLflow, Feast, Kedro ([kedro-plugins](https://github.com/kedro-org/kedro-plugins)), [quinn](https://github.com/MrPowers/quinn), [dbt-spark](https://github.com/dbt-labs/dbt-spark), and [python-deequ](https://github.com/awslabs/python-deequ). Each donor's PySpark code is preserved verbatim under `upstream/`, paired with an `annotated/` companion that adds the pykrete schema declarations an adopter would write; pykrete runs against the annotated form. Each push rebuilds pykrete from the catalog-pinned source commit (`scripts/diagnostic_catalog.json`'s `pykreteSourceCommit`), re-runs `pykrete check` against every fixture, and JSON-diffs the output against the committed golden — a new false positive on real Spark code blocks the release before the tag goes out.
- **Schema tracking is verified, not assumed.** On top of the golden-diff suite, we run **149 schema-tracking probes** that assert pykrete is actually tracking columns through real transforms. The probes cover 58 of the 59 vendored fixtures (the feast `spark_kafka_processor` streaming fixture is annotated but probe-free, since it has no typed-DataFrame slot a probe can anchor to): 122 positive probes across 37 of the 38 annotated fixtures verify columns resolve cleanly after `.select` / `.filter` / `.withColumn` and the pandas analogues; 27 negative probes across all 21 deliberately-corrupted fixtures verify specific diagnostics — D0030 `unknownColumn`, D0081 `nonNumericArithmetic`, D0082 `crossTypeComparison`, D0084 `enumValueMismatch`, and D0090 `deprecatedDataFrameAlias` — actually fire. Together, these verify three properties on every release: **column resolution + diagnostic firing + Spark type tracking (scoped to D0081 via `PROBE-TYPE-IS` synth, shipped in v1.2)**. We verify enum value vocabularies in 3 of 10 donors: Delta CDC `_change_type` (`{"insert", "update_preimage", "update_postimage", "delete"}`), Hudi `_hoodie_operation` (`{"I", "-U", "U", "D"}`), and MLflow run status (`{"RUNNING", "FINISHED", "FAILED", "KILLED", "SCHEDULED"}`). **New in v1.3**: pandas dialect support — `PandasFrame[X]` / `SparkFrame[X]` / `DataFrame[X]` annotations dispatch to dialect-specific check sites, with pandas check-site coverage exercised by new cross-codebase fixtures across mlflow, feast, and iceberg-python. Honest scoping: v1.3 ships pandas **check-site coverage** (column-ref recognition, the six dispatched operations `df[col_list]` / `df[mask]` / `df["new"] = expr` / `df.drop` / `df.merge` / `df.rename`, and D0090 deprecation); positive **type-tracking** verification for pandas via `PROBE-TYPE-IS` lands in v1.4 (parallel to how v1.2 added Spark type-tracking after v1.1 introduced Spark column tracking — tracker: [pykrete-tests#14](https://github.com/amirnaderi93/pykrete-tests/issues/14)). D0080 `returnTypeMismatch` and D0082 `crossTypeComparison` keep their falsifiability through raw-mutation fixtures in the test suite until their synth shapes ship. Numeric-subtype distinguishability (`int` vs `long` vs `short`) and `withColumn` output enum-constraint preservation are carried forward in the polish backlog. CI fails if any probe asserts the wrong outcome. See the [pykrete-tests README](https://github.com/amirnaderi93/pykrete-tests#schema-tracking-probes-v13) and [`scripts/PROBES.md`](https://github.com/amirnaderi93/pykrete-tests/blob/main/scripts/PROBES.md) for the methodology; per-donor matrix and pinned commits live in the [donors table](https://github.com/amirnaderi93/pykrete-tests#the-donors).
- **1,252 tests in CI across the analyzer, LSP, and wasm crates.** Every release has to pass the full suite, plus per-D-code snapshot tests that pin every error message — wording drifts fail the build until explicitly accepted.
- **JSON output is a stability contract from v1.0.0.** Field names, types, semantics, and D-code identity will not change without a SemVer-major bump and a corresponding `schemaVersion` bump. See [Production readiness → JSON output stability contract](https://amirnaderi93.github.io/pykrete/about/production-readiness/#json-output-stability-contract).
- **No-false-positives policy.** When pykrete cannot determine a schema or a type with confidence, it stops checking that subtree rather than guessing. A static checker that cries wolf gets switched off.
- **Pre-major-release audit cycle.** Every X.0.0 bump runs three independent fresh-eyes audits (architecture, Spark coverage, docs sync) before the tag. Findings ship in the release notes.

If pykrete is wrong on your code, [open an issue](https://github.com/amirnaderi93/pykrete/issues) — false positives are triaged ahead of everything else.

## Editor integration

The **VS Code extension** gives you live diagnostics, hover, completion, and go-to-definition on `.pyk` files. Search **pykrete** in the Extensions panel — it's on the [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=amirnaderi.pykrete) (VS Code) and the [Open VSX Registry](https://open-vsx.org/extension/amirnaderi/pykrete) (Cursor, VSCodium, code-server, Theia).

For Neovim, Helix, Emacs, and other LSP clients, see [docs/editors/](docs/editors/).

## Documentation

Full documentation at **[amirnaderi93.github.io/pykrete](https://amirnaderi93.github.io/pykrete/)** — schema reference, diagnostic catalog, how it works, the roadmap.

Today: PySpark (feature-complete) plus pandas check-site coverage shipped in v1.3. Next: pandas type-tracking probe parity (v1.4) and polars — see the [roadmap](docs/roadmap.md).

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
