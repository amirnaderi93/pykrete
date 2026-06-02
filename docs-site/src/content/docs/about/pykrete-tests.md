---
title: Real-codebase tests
description: How pykrete is tested against 47 fixtures and 130 schema-tracking probes from 10 upstream codebases — what we cover, what the goldens and probes verify, what's deliberately out of scope.
---

[pykrete-tests](https://github.com/amirnaderi93/pykrete-tests) is a separate repo that vendors fixtures from 10 widely-used PySpark codebases, adds pykrete annotations the way a real adopter would, and on every push runs two release-blocking suites against pykrete built fresh from `main`: a golden-diff suite (JSON output of `pykrete check` against a frozen snapshot) and a 130-probe schema-tracking suite (inline `# PROBE-*` assertions that columns survive transforms, that specific diagnostics fire on deliberately-corrupted fixtures, and — new in v1.2 — that pykrete tracks types through transformations on a scoped 3-donor subset). It exists for two reasons:

1. **Regression coverage** — catch behavior changes in pykrete's checker as new operations are modeled.
2. **Trust signal** — demonstrate that pykrete keeps real PySpark code diagnostic-free under realistic annotation, *and* that the absence of diagnostics actually means the schema tracker is doing its job.

## The donors

47 fixtures (35 annotated + 12 deliberately-corrupted under `probes_negative/`) across 10 donors, all Apache 2.0:

| donor | upstream | annotated | probes_negative |
|---|---|---:|---:|
| **spark** | [apache/spark](https://github.com/apache/spark) | 8 | 2 |
| **delta** | [delta-io/delta](https://github.com/delta-io/delta) | 4 | 2 |
| **kedro-plugins** | [kedro-org/kedro-plugins](https://github.com/kedro-org/kedro-plugins) | 3 | 1 |
| **iceberg-python** | [apache/iceberg-python](https://github.com/apache/iceberg-python) | 2 | 0 |
| **hudi** | [apache/hudi](https://github.com/apache/hudi) | 3 | 1 |
| **mlflow** | [mlflow/mlflow](https://github.com/mlflow/mlflow) | 5 | 3 |
| **feast** | [feast-dev/feast](https://github.com/feast-dev/feast) | 3 | 1 |
| **quinn** | [MrPowers/quinn](https://github.com/MrPowers/quinn) | 3 | 2 |
| **dbt-spark** | [dbt-labs/dbt-spark](https://github.com/dbt-labs/dbt-spark) | 2 | 0 |
| **python-deequ** | [awslabs/python-deequ](https://github.com/awslabs/python-deequ) | 2 | 0 |

Three donors — delta, hudi, and mlflow — carry **v1.1 enum value vocabulary** fixtures: Delta CDC `_change_type` (`{"insert", "update_preimage", "update_postimage", "delete"}`), Hudi `_hoodie_operation` (`{"I", "-U", "U", "D"}`), and MLflow run status (`{"RUNNING", "FINISHED", "FAILED", "KILLED", "SCHEDULED"}`). Each ships an annotated fixture demonstrating in-vocab usage and a `probes_negative/` counterpart asserting D0084 `enumValueMismatch` fires on off-vocab typos.

Every annotated fixture currently emits zero diagnostics against the released binary; the golden snapshots for the annotated tree are all empty-diagnostic arrays after the v0.1.39 false-positive sweep cleared the six v0.1.38-baseline fixtures (seven underlying findings total) that the cross-codebase suite had surfaced. The `probes_negative/` fixtures are deliberately broken — each one's `.golden.json` carries the exact diagnostics pykrete must fire, and the golden-diff suite verifies they fire on every release. The donor table with pinned commits and per-donor coverage rationale — what each codebase exercises, why it earned a slot — lives in the [pykrete-tests README](https://github.com/amirnaderi93/pykrete-tests#the-donors).

## Schema-tracking probes

On top of golden-diff, every release runs **130 schema-tracking probes**:

- **113 positive probes** across 34 of the 35 annotated fixtures assert columns survive `.select` / `.filter` / `.withColumn` and similar narrow transforms. These probes prove that the absence of a diagnostic isn't a silent miss — pykrete genuinely tracked the column through the chain. The feast `spark_kafka_processor` streaming fixture is annotated but probe-free, since it has no typed-DataFrame slot a probe can anchor to.
- **17 negative probes** across all 12 deliberately-corrupted fixtures assert specific diagnostics fire: D0030 (`unknownColumn`), D0081 (`nonNumericArithmetic`), D0082 (`crossTypeComparison`), and D0084 (`enumValueMismatch`). Without these, a silently-passing checker would satisfy every annotated probe vacuously.
- **Enum value vocabulary verification** in 3 of the 10 donors — Delta CDC `_change_type`, Hudi `_hoodie_operation`, and MLflow run status. Positive probes assert in-vocab literals stay clean across `==` / `.isin` / `withColumn` / `F.expr` / `groupBy` chains; negative probes assert D0084 fires when an off-vocab typo is used in a comparison or fill operation.
- **`PROBE-TYPE-IS` type-tracking coverage** in 3 of the 10 donors — quinn, MLflow, and python-deequ — new in v1.2. Each donor ships at least one type-tracking assertion through `.select` / `.withColumn` / `.filter` chains. The synth wraps the assertion in `{df}.select(...)`, binding `col(...)` against the typed DataFrame in scope so off-claim markers fire D0081. The synth shape is scoped to D0081 in v1.2; D0080 (`unknownAttribute`) and D0082 (`crossTypeComparison`) are covered by raw-mutation fixtures (`V12CrossCodebaseMarkerMutationTests`) until v1.3 brings them under the synth gate. A CI gate, `V12FalsifiabilityCoverageGuard`, mutates the claimed type on every `PROBE-TYPE-IS` marker and verifies D0081 fires.

Together, the suite verifies three properties on every release: **column resolution + diagnostic firing + type tracking (scoped to D0081 via `PROBE-TYPE-IS` synth in v1.2)**.

Probes are inline `# PROBE-*` comment markers in `.pyk` fixtures, parsed by `scripts/probes.py` and verified against `pykrete check --format json` output. The marker grammar, placement convention, and `catalog-drift-watch` workflow that keeps `PROBE-EXPECTS` D-codes in sync with upstream are documented in [`scripts/PROBES.md`](https://github.com/amirnaderi93/pykrete-tests/blob/main/scripts/PROBES.md). CI fails if any probe asserts the wrong outcome.

What the v1.2 probe suite does *not* yet verify, all tracked for v1.3:

- **`PROBE-TYPE-IS` synth-shape coverage beyond D0081.** The v1.2 synth shape (`{df}.select(col("x") + 1)`) falsifies on non-numeric. D0080 (`unknownAttribute`) and D0082 (`crossTypeComparison`) need their own synth shapes; the raw-mutation suite covers them until then.
- Numeric-subtype distinguishability (`int` vs `long` vs `double` arithmetic narrowing). Carried forward from v1.1.
- **withColumn output enum-constraint preservation.** Carried forward from v1.1: the literal is checked against the sink's enum vocabulary, but the constraint drops on the output column — so a downstream `==` against an off-vocab literal on the rewritten column will not fire D0084. Tracker in `docs/design/literal-value-vocabulary.md` polish backlog.

## How it works

Each donor lives under `cross-codebase/<donor>/`:

```
cross-codebase/<donor>/
├── upstream/<orig-path>/<file>.pyk           # verbatim upstream Python, .py → .pyk renamed
├── annotated/<orig-path>/<file>.pyk          # same file with Schema + typed helpers added
├── annotated/<orig-path>/<file>.golden.json  # frozen pykrete check JSON output (empty diagnostics)
├── probes_negative/<file>.pyk                # deliberately-corrupted fixture (optional, per-donor)
├── probes_negative/<file>.golden.json        # frozen JSON with the expected diagnostics
├── LICENSE-UPSTREAM
└── pinned-commit
```

The `.py` → `.pyk` rename is zero-behavior-change — pykrete is a strict superset. Annotations live in the `annotated/` companion: `Schema` classes plus typed helper functions extracting dataframe-typed cores of upstream methods (since pykrete only enters body analysis on `DataFrame[X]` signatures). `probes_negative/` fixtures are smaller, single-purpose corruptions of the annotated patterns — they exist to prove pykrete actually catches regressions, not just that it doesn't false-positive.

CI runs `bash scripts/golden.sh check` (both trees) and `bash scripts/probes_ci.sh` on every push. pykrete is built fresh from `main` each run; any golden-diff drift or probe failure fails the build before the regression gets released.

## What this suite does NOT cover

Real-world donor code doesn't exercise pykrete's full surface area. ~30 individual `F.*` functions, `melt` / `cube` / `rollup`, Schema arithmetic operators, and the v0.1.28+ atomic-type aliases (`byte`, `short`, `decimal(p, s)`, `binary`) aren't represented by any donor fixture. Those features are covered by synthetic unit tests in [`crates/pykrete/tests/`](https://github.com/amirnaderi93/pykrete/tree/main/crates/pykrete/tests). The two tiers complement each other: real-world donors prove pykrete keeps working on production patterns; synthetic unit tests prove each feature surface behaves to spec.

## What the pilot loop surfaced

The first pass through the vendored codebases surfaced five real pykrete gaps; all of them shipped as fixes in earlier releases:

| Pilot | File | Gap surfaced | pykrete commit |
|---|---|---|---|
| 1 | Spark `examples/.../basic.py` | `df["X"]` subscript wasn't recognized as a column ref | [`483cc09`](https://github.com/amirnaderi93/pykrete/commit/483cc09) |
| 2 | Spark `tests/.../test_group.py` | GroupedData shortcut aggregates (`g.max("col")`) didn't check args | [`c25fe5c`](https://github.com/amirnaderi93/pykrete/commit/c25fe5c) |
| 3 | MLflow `tests/.../test_spark_datasource_autologging.py` | `intersect`/`subtract`/`exceptAll` weren't modeled (`union` was) | [`d68d1e2`](https://github.com/amirnaderi93/pykrete/commit/d68d1e2) |
| 4 | Spark `tests/.../test_column.py` | Chained Column-on-Column nested-field access (`df.r.X`) skipped | [`0b70d9c`](https://github.com/amirnaderi93/pykrete/commit/0b70d9c) |
| 5 | Spark `examples/.../arrow.py` | Lowercase `groupby` alias not recognized | [`9a49bf6`](https://github.com/amirnaderi93/pykrete/commit/9a49bf6) |

Each fix shipped with regression tests in the pykrete crate, so the same gap can't reopen silently. Every Spark coverage gap surfaced in pykrete-tests (or against real production PySpark codebases) since then has followed the same pattern: a regression test in `crates/pykrete/tests/` plus a checked-in fix. See the [CHANGELOG](https://github.com/amirnaderi93/pykrete/blob/main/CHANGELOG.md) for the per-release breakdown.

## Methodology per donor

The same loop every time:

1. **Vendor** the upstream file at a pinned commit, preserving the license.
2. **Annotate** — add `Schema` classes and `DataFrame[Schema]` annotations on representative functions. Helpers extract the dataframe-typed cores from test methods that take `self` rather than `DataFrame[X]`, since pykrete only enters body analysis on the latter.
3. **Generate the golden** — `bash scripts/golden.sh generate <pykrete-binary>` writes the current JSON output as `<file>.golden.json`.
4. **Review the golden** — if a fixture has non-empty diagnostics, decide: is it a planted probe (typo to verify the checker fires), an upstream type-vocabulary gap, or a pykrete false positive worth tracking?
5. **CI freezes the contract.** From here on, any pykrete behavior change shows up as a golden diff in the PR.

Per-donor pinned commits, the cross-codebase contract, and the `update-pinned-commit` procedure live in the [pykrete-tests README](https://github.com/amirnaderi93/pykrete-tests#updating-donors).
