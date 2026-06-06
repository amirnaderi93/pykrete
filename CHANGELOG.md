# Changelog

All notable changes to pykrete are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and pykrete adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.4.0] - 2026-06-07

Fourth minor release on the v1.0 line. The headline change is **depth
on pandas**: seven new pandas-heavy donors in pykrete-tests bring
pandas-coverage donor count from 3 to 10, positive `PROBE-TYPE-IS`
coverage on `PandasFrame[X]` lands across the new donors (closing
pykrete-tests#14), and three PRE-EXISTING silent-pass checker bug paths
surfaced by v1.3 audits are closed. The `pykrete.json` config-discovery
walk now anchors on the input file's parent directory (file-anchored
with CWD fallback), so absolute-path invocations from outside the
project root pick up the project's config. No new D-codes, no new
annotation forms; SemVer-minor under the `tighteningDiagnostics`
policy.

### Added

- **Pandas `PROBE-TYPE-IS` coverage on `PandasFrame[X]`** (closes
  pykrete-tests#14). The probe synth wraps
  `{df}.assign(__probe={df}["x"] + 1)` — a dispatched pandas op so
  off-claim numeric dtypes fall through to D0081
  `nonNumericArithmetic`. The pykrete-tests recognizer was widened to
  accept `PandasFrame[X]` / `SparkFrame[X]` annotations in
  `_first_dataframe_param`; pykrete's `infer_expr_type` learned an
  `Expr::Subscript(Name)` arm so the synthesized arithmetic actually
  reaches the D0081 dispatch. **39 markers across the 7 new donors**
  (≥5 per donor; spec §1 floor was ≥3 per donor / ≥21 total).
- **Seven new pandas-heavy donors in pykrete-tests** —
  scikit-learn, statsmodels, pandera, Great Expectations, prophet,
  seaborn, yfinance. Donor count 10 → 17; pandas-coverage donor count
  3 → 10. Honest scoping breakdown:
    - **3 direct-dispatch** (prophet, seaborn, yfinance): the
      `annotated/<libname>/...` fixtures track the actual upstream
      library code, with `PandasFrame[X]` annotations added and the
      call sites matched against pykrete's v1.3 dispatched-shape
      recognizers (string-literal subscripts in `prophet/forecaster.py`,
      dict-literal `rename(columns=…)` in `seaborn/categorical.py`,
      `df["new"] = expr` / `df.rename(columns={…})` /
      `df.merge(...)` in `yfinance/utils.py`).
    - **4 canonical-fixture-only** (scikit-learn, statsmodels,
      pandera, Great Expectations): the `annotated/canonical/...`
      fixtures model how a user idiomatically wields the library at
      the pandas boundary. The upstream code itself rarely uses
      pykrete-dispatched shapes (sklearn / statsmodels operate on
      numpy arrays internally; pandera / GE operate at metric /
      domain layers above raw pandas).
- **Three checker bug closures** (PR-B; v1.4 spec §4 — all
  PRE-EXISTING silent-pass paths surfaced by v1.3 audits):
    - **Registry-call §10 widening edge case.** `util(df["typo"])`
      where `util(x: int)` has no `DataFrame[X]`-typed param now
      walks the args unconditionally and fires D0030 on the embedded
      typo. Previously the registry-call gate short-circuited first
      and the typo slipped past.
    - **`inherited_dialect` walrus / Named receivers.**
      `(pdf := build()).rename(...)` now inherits the assigned value's
      dialect, so pandas dispatch fires on walrus-bound chains.
      Previously walrus receivers fell through to the Spark default.
    - **`.transform(helper)` dialect preservation.** The receiver's
      dialect is now threaded into the helper's body inference, so
      pandas-only operations inside the helper (e.g., `.assign`)
      dispatch under the correct dialect and the inferred return
      schema reaches downstream column references. Previously the
      helper's `PandasFrame[X]` parameter dropped its dialect tag at
      the bind site.
- **`pykrete.json` config-discovery walk** (PR-D; closes
  pykrete#98). The discovery now walks from the input file's parent
  directory (falling back to the working directory when no input
  resolves to a file path), so `pykrete check
  /abs/path/to/project/foo.pyk` from any CWD picks up the project's
  `pykrete.json`. LSP discovery was already file-anchored via the
  project-root resolver; this aligns the CLI.
- **Probe count: 149 → 223 (+74).** Pandas type-tracking and the
  seven new pandas donors drive the increase: 39 new
  `PROBE-TYPE-IS` markers + new `PROBE-RESOLVES` / `PROBE-EXPECTS`
  coverage across the new donors' positive and negative fixtures.
- **Fixture count: 59 → 83 (+24).** 46 annotated (was 38) + 37
  `probes_negative/` (was 21). Donor count 10 → 17.

### Changed

- **Trust-claim surfaces** swept end-to-end for v1.4 reality:
  README "Reliability and trust", docs-site
  `about/production-readiness`, `about/pykrete-tests`, the splash
  page, and the docs-site / canonical roadmaps all refresh to 223
  probes across 83 fixtures from 17 donors; pandas check-site +
  type-tracking coverage in 10 of 17 donors split into 3 hybrid + 3
  direct-dispatch + 4 canonical-fixture-only classes; the v1.5+
  deferral list (cross-dialect handoffs, `.query` / `.eval` mini-DSLs,
  broader pandas method modeling, I/O entry points) re-stated under
  "what we do not yet verify".
- **New docs-site page**:
  `docs-site/src/content/docs/about/pandas-roadmap.md` tracks the
  pandas-specific direction across v1.3 / v1.4 / v1.5+ / v2.0 as a
  complement to the umbrella roadmap.
- **Canonical-name migration completion** (PR-D-canonical; closes
  pykrete#97). All in-repo `docs/` design notes, language-reference
  pages, editor-integration notes, and the canonical roadmap use
  `SparkFrame[X]` / `PandasFrame[X]` as the current annotation form
  in examples; references that describe `DataFrame[X]` as the
  deprecated alias remain (the file documents the alias's existence).
- **2 goldens refreshed** to absorb v1.4's D0081 / D0082 tightening
  on already-corrupted negative inputs
  (`mlflow/probes_negative/withColumn_arith_on_string.pyk` adds a
  D0081 at L21; `spark/probes_negative/cross_type_comparison.pyk`
  adds a D0082 at L21). The 81 unchanged fixtures across the 17
  donors confirm v1.4's checker work didn't introduce silent
  positive-fixture regressions.

### Verified properties (cumulative)

The trust suite verifies, on every release:

- **Column resolution** through the Spark v1.0 surface plus the
  pandas analogues — 180 positive probes across 45 annotated
  fixtures.
- **Diagnostic firing** on broken fixtures — 43 negative probes
  across 37 `probes_negative/` fixtures pinning D0030
  `unknownColumn`, D0060 `missingJoinKey`, D0081
  `nonNumericArithmetic` (v1.4 widened to subscript-on-name
  receivers), D0082 `crossTypeComparison` (widened correspondingly),
  D0084 `enumValueMismatch`, and D0090 `deprecatedDataFrameAlias`.
- **Spark type tracking** through transformations, scoped to D0081
  via the `PROBE-TYPE-IS` synth-shape path (shipped v1.2), with
  raw-mutation coverage on D0080 / D0082 until follow-up synth
  shapes ship.
- **Pandas type tracking** through dispatched chains (new in v1.4)
  on `PandasFrame[X]`, scoped to D0081 via the assign-arithmetic
  synth — 39 markers across 7 donors.

### Deferred (v1.5+ trackers)

- **Cross-dialect handoff annotations** (`SparkFrame[X] →
  PandasFrame[X]` across `.toPandas()` and friends). v1.4 covers
  depth on annotated frames, not boundary recognition.
- **`df.query("…")` / `df.eval("…")` mini-DSLs.** Own design surface;
  parse string-fragment column refs separately.
- **Broader pandas method modeling** (`pivot_table`,
  `groupby.agg`, `melt`, `stack` / `unstack`, `reset_index`,
  `set_index`).
- **`pd.read_csv(...)` and other pandas I/O entry points.**
- **Retrofitting pandas `PROBE-TYPE-IS` to the v1.3 hybrid donors**
  (MLflow, Feast, iceberg-python) — v1.4 deliberately scoped these
  out per spec §1.

### Coordinated with

- pykrete-tests: PR-A through PR-E of the v1.4 cycle ship the
  cross-codebase pandas donor expansion (7 new donors), the
  `PROBE-TYPE-IS` recognizer widening, the catalog pin bump, and
  the mass golden refresh.

### Compatibility

- **`DataFrame[X]` source-compatible through v1.x.** Every
  existing `DataFrame[X]` annotation continues to type-check; the
  warning is informational and downgradable to off via the standard
  `pykrete.json` `rules` block. Removal is committed for v2.0.
- **JSON output contract.** `schemaVersion` stays at `"1"`. No new
  D-codes; no existing diagnostic's shape or semantics changed. The
  v1.4 widening on D0081 / D0082 is SemVer-minor under the
  `tighteningDiagnostics` policy (new firing positions for existing
  D-code identities).

## [1.3.0] - 2026-06-03

Third minor release on the v1.0 line. The headline change is **pandas
dialect support**: `PandasFrame[X]` joins `SparkFrame[X]` as a
canonical dataframe-annotation form, with `DataFrame[X]` demoted to a
deprecated alias (warning `D0090`, removed in v2.0). Six pandas
operations dispatch through dialect-specific check sites, the §10
widening fires `D0030` on bare `df["typo"]` subscripts in
non-method contexts (both Spark and pandas), and three donor
fixtures land cross-codebase pandas coverage. Cross-codebase CI on
pykrete-tests is now pinned to the catalog's `pykreteSourceCommit`
(no more `PYKRETE_REF: main` silent drift), and 48 goldens were
mass-refreshed to absorb the new `D0090` warnings on existing
`DataFrame[X]` annotations.

### Added

- **`PandasFrame[X]` annotation surface.** The pandas dialect is a
  parser-level peer of `SparkFrame[X]`: same `Pick[…]` / `Omit[…]` /
  `Merge[…]` derived-schema operators, same inline dict shape, same
  `Schema` class declarations. The dialect tag on the resulting
  `TypedSlot` drives check-site dispatch — a `df: PandasFrame[X]`
  parameter routes through the pandas-shape operations, a
  `SparkFrame[X]` parameter routes through Spark's. Spec settled in
  [`docs/design/pandas-support.md`](docs/design/pandas-support.md).
- **Six dispatched pandas operations.** Column selection
  (`df[col_list]` mirroring `.select`), boolean-mask filtering
  (`df[mask]` mirroring `.filter`), assignment (`df["new"] = expr`
  mirroring `withColumn`), drop (`df.drop(columns=[…])`), merge
  (`df.merge(other, on=…)` mirroring `.join`), and rename
  (`df.rename(columns={…})`). Each one mirrors its Spark cousin's
  schema-tracking behavior; runtime semantics are pandas's.
- **`D0090 deprecatedDataFrameAlias`.** Stable, warning severity.
  Fires on every `DataFrame[X]` annotation, with a quick-fix
  suggesting `SparkFrame[X]`. The alias remains valid in v1.3 for
  source compatibility; it is **removed in v2.0** — projects should
  migrate to `SparkFrame[X]` (or `PandasFrame[X]` where applicable)
  before the next major.
- **`ColumnType::Float` variant** (SemVer-minor: new variant on the
  public type enum). Pandas's `float32` / `float64` distinction
  required the new variant; existing Spark `double` mappings are
  unchanged. Exhaustive-match sites were swept per the spec § 9
  piece (b) requirement.
- **§10 widening: bare `df["typo"]` fires D0030 in non-method
  contexts.** Previously a bare `df["typo"]` subscript outside a
  method-call context was silently accepted on `SparkFrame[X]`
  (also future-`PandasFrame[X]`); v1.3 widens D0030 to fire at the
  same position. Before: `x = df["typo"]` was silent on
  `SparkFrame[Sale]` even when `typo` was not in `Sale`. After:
  same line fires `D0030 unknownColumn` on `"typo"` with a *did you
  mean* against the schema. Existing D-code identity, new firing
  positions — policy: SemVer-minor `tighteningDiagnostics`. Users
  with brittle CI may see new D0030 fires on previously-silent
  code; the change is a net win for correctness.
- **3 new cross-codebase pandas fixtures.** mlflow, feast, and
  iceberg-python each contribute an annotated `PandasFrame[X]`
  fixture exercising the six dispatched operations, paired with
  `probes_negative/` counterparts asserting D0030 on bare
  `df["typo"]` subscripts and D0090 on the deprecated
  `DataFrame[X]` alias.
- **Probe count: 130 → 149 (+19).** Pandas check-site coverage adds
  9 positive probes (column resolution across the new operations)
  and 10 negative probes (D0030 + D0090 on probes_negative shapes)
  across the three new donor fixtures.
- **Fixture count: 47 → 59 (+12).** 38 annotated (was 35) + 21
  `probes_negative/` (was 12). Donor count stays at 10.

### Changed

- **DataFrame[X] is now a deprecated alias for SparkFrame[X].**
  Every existing `DataFrame[X]` annotation in the wild fires
  `D0090` as a warning starting in v1.3, with a quick-fix to the
  canonical `SparkFrame[X]`. Removal is committed for v2.0; the
  v1 line keeps the alias working so the migration is unhurried.
- **Trust-claim surfaces.** README "Reliability and trust",
  docs-site `about/production-readiness`, `about/pykrete-tests`,
  the splash page, and the pykrete-tests README all refresh to the
  v1.3 reality: 149 probes across 59 fixtures from 10 donors;
  pandas check-site coverage in 3 of 10 donors; D0090 in the
  D-code list; honest scoping that pandas **positive type-tracking**
  via `PROBE-TYPE-IS` is deferred to v1.4 (parallel to how v1.2
  added Spark type-tracking after v1.1 introduced Spark column
  tracking) — tracker [pykrete-tests#14](https://github.com/amirnaderi93/pykrete-tests/issues/14).
- **pykrete-tests CI hardening.** `cross-codebase.yml` now reads
  `PYKRETE_REF` from `scripts/diagnostic_catalog.json`'s
  `pykreteSourceCommit` pin (matching `probes.yml`), removing the
  silent-drift class where `PYKRETE_REF: main` could disagree with
  the catalog pin and surface unrelated regressions on every cron
  run. The exit-mask block in the same file mirrors `probes.yml`'s
  `set +e` pattern so the friendly `::error::` annotation lands on
  pipeline failure (previously `bash -e` on the pipeline boundary
  could skip it).
- **48 goldens refreshed.** The mass refresh absorbs the new D0090
  warnings on every existing `DataFrame[X]`-annotated fixture in
  pykrete-tests. The change set is mechanical: D0090 additions
  plus warningCount bumps; non-D0090 diagnostics on the same
  fixtures are preserved. Two pre-existing strict-mode
  `probes_negative/` goldens (mlflow `withColumn_arith_on_string`
  for D0081, spark `cross_type_comparison` for D0082) drop their
  strict-mode entries because `golden.sh` invokes pykrete from the
  repo root, so the sibling `pykrete.json` (`typeCheckingMode:
  "strict"`) is not discovered — this matches a pre-existing v1.2
  tracker (`pykrete.json` config discovery on absolute paths) and
  is NOT a regression introduced by v1.3. Strict-mode coverage on
  those fixtures remains enforced by `probes_ci.sh`, which stages
  the fixture's full directory into a tempdir and invokes pykrete
  from there — the `pykrete.json` is picked up and `D0081` /
  `D0082` continue to fire under PROBE-EXPECTS verification.

### Verified properties (cumulative)

The trust suite verifies, on every release:

- **Column resolution** through `.select` / `.filter` /
  `.withColumn` / `.drop` / `.join` / `.groupBy` and the rest of
  the Spark v1.0 surface, plus the pandas analogues `df[col_list]`
  / `df[mask]` / `df["new"] = expr` / `df.drop` / `df.merge` /
  `df.rename` — 122 positive probes across 37 annotated fixtures.
- **Diagnostic firing** on broken fixtures — 27 negative probes
  across all 21 `probes_negative/` fixtures pinning D0030
  `unknownColumn`, D0060 `missingJoinKey`, D0081
  `nonNumericArithmetic`, D0082 `crossTypeComparison`, D0084
  `enumValueMismatch`, and D0090 `deprecatedDataFrameAlias`.
- **Spark type tracking** through transformations, scoped to D0081
  via the `PROBE-TYPE-IS` synth-shape path (shipped v1.2), with
  raw-mutation coverage on D0080 / D0082 until follow-up synth
  shapes ship.

### Deferred (v1.4 trackers)

- **Positive `PROBE-TYPE-IS` coverage on `PandasFrame[X]`.** v1.3
  ships pandas check sites; positive type-tracking probes on
  pandas annotations land in v1.4, parallel to how v1.2 added
  Spark type-tracking after v1.1 introduced Spark column tracking.
  Tracker: [pykrete-tests#14](https://github.com/amirnaderi93/pykrete-tests/issues/14).
- **Cross-dialect handoff annotations** (`SparkFrame[X] →
  PandasFrame[X]` across `.toPandas()` and friends). Spec § 11
  defers to v1.4.
- **`df.query("...")` / `df.eval("...")` mini-DSL.** Spec § 11
  defers to v1.4.

### Coordinated with

- pykrete-tests: PR-A through PR-D of the v1.3 cycle ship the
  cross-codebase pandas fixtures, the probes runner extensions,
  the catalog pin alignment, the cross-codebase exit-mask
  hardening, and the mass golden refresh.

### Compatibility

- **`DataFrame[X]` source-compatible through v1.x.** Every existing
  `DataFrame[X]` annotation continues to type-check; the warning
  is informational and downgradable to off via the standard
  `pykrete.json` `rules` block. Removal is committed for v2.0.
- **JSON output contract.** `schemaVersion` stays at `"1"`. Adding
  `D0090` is a non-breaking change per the v1.0 stability contract
  (consumers must accept unknown D-codes). No existing diagnostic's
  shape or semantics changed.

## [1.2.0] - 2026-06-02

Second minor release on the v1.0 line. The headline change is a
trust-system extension, not a checker behavior change: the
**`PROBE-TYPE-IS` synthesizer** now binds to live local scopes, which
turns the v1.1-reserved-but-silent type-tracking marker class into a
working release-blocking gate. Three donor fixtures pick up
`PROBE-TYPE-IS` coverage in this release (quinn, mlflow,
python-deequ), the cross-codebase suite grows to **130 probes (113
positive + 17 negative)**, and a new CI gate plus two test classes
pin the new coverage so it can't silently regress. The pykrete-core
Rust workspace is bit-identical to v1.1.0 — every change in this
release lives in pykrete-tests and the docs surface.

### Added

- **`PROBE-TYPE-IS` scope-binding fix.** The v1.1 reservation
  declared the marker syntax but the runner couldn't anchor the
  synthesized type assertion to the live local scope where the
  probed DataFrame lived — so off-claim markers stayed silent
  instead of failing. v1.2 fixes the synthesizer to wrap the
  assertion in `{df}.select(...)`, which binds `col(...)` against
  the typed DataFrame in scope. Off-claim markers now fire D0081
  `nonNumericArithmetic` (the scoped failure mode the synth shape
  surfaces) — the marker is finally falsifiable. The grammar and
  the synth shape are documented in
  [`docs/design/schema-tracking-probes.md`](https://github.com/amirnaderi93/pykrete/blob/main/docs/design/schema-tracking-probes.md).
- **3 new `PROBE-TYPE-IS` markers across donor fixtures.** quinn
  (`column_helpers.pyk`), mlflow
  (`spark_autologging_intro.pyk`), and python-deequ
  (`basic_example.pyk`) each pick up at least one type-tracking
  assertion through `.select` / `.withColumn` / `.filter` chains.
  All three were silent under v1.1 (the runner couldn't bind the
  assertion); under v1.2 they're release-blocking — falsify any
  one and CI fails.
- **`V12FalsifiabilityCoverageGuard` CI gate** in pykrete-tests.
  The gate scans every `PROBE-TYPE-IS` marker in the suite, runs
  the synthesized assertion against a mutated `Schema` (the
  claimed type swapped for a non-matching one), and verifies
  D0081 fires. Any marker that stays silent under mutation fails
  the gate before the release tag.
- **`V12PerDCodeFalsifiabilityTests`** in pykrete-tests, asserting
  per-D-code that the synth shape actually surfaces the diagnostic
  the marker claims it does. Scoped to D0081 in v1.2 (the synth
  shape `{df}.select(col("x") + 1)` falsifies on non-numeric);
  D0080 / D0082 falsifiability lives in the raw-mutation
  `V12CrossCodebaseMarkerMutationTests` instead until v1.3's
  synth-shape expansion lands.
- **`V12CrossCodebaseMarkerMutationTests`** in pykrete-tests. For
  the D-codes that don't have a dedicated synth-shape gate yet
  (D0080, D0082), the test class mutates the upstream fixture
  directly — swaps an `int` for a `string`, an arithmetic operator
  for a comparison — and verifies the corresponding diagnostic
  fires. Closes the falsifiability gap on D0080 / D0082 until
  v1.3 brings them under the synth gate.

### Changed

- **Probe count: 127 → 130 (positive 110 → 113; negative
  unchanged at 17).** The three new `PROBE-TYPE-IS` markers each
  ship a positive assertion. Fixture count, donor count, negative
  count, and D-code coverage by the negative probes are unchanged
  from v1.1: 47 fixtures (35 annotated + 12 negative), 10 donors,
  46-of-47 covered (the feast `spark_kafka_processor` streaming
  fixture is annotated but probe-free), D0030 / D0081 / D0082 /
  D0084 firing on negative probes.
- **Behavior change for `PROBE-TYPE-IS` markers.** Any marker
  authored under v1.1 was silent — the runner couldn't anchor it.
  Under v1.2, the same markers fire D0081 if the claimed type is
  wrong. Authors who landed `PROBE-TYPE-IS` lines on v1.1
  assuming "no diagnostic means correct" should re-verify against
  the v1.2 runner. None of the in-tree v1.1 markers were wrong;
  the three new v1.2 markers verify clean.
- **Trust-claim surfaces.** README "Reliability and trust",
  docs-site `about/production-readiness`, `about/pykrete-tests`,
  and the index splash all refresh to the v1.2 reality: 130
  probes across 47 fixtures, 10 donors, 3-of-10 with enum
  vocabulary verification, **3-of-10 with `PROBE-TYPE-IS`
  type-tracking coverage** scoped to D0081 via the synth-shape
  path. Honest scoping noted on all four surfaces: numeric
  subtype distinguishability and withColumn output enum
  preservation remain in the v1.1 polish backlog; D0080 / D0082
  type-tracking lives in the raw-mutation suite until v1.3's
  synth-shape expansion.

### Verified properties (cumulative)

The trust suite now verifies, on every release:

- **Column resolution** through `.select` / `.filter` /
  `.withColumn` / `.drop` / `.join` / `.groupBy` and the rest of
  the v1.0 surface — 113 positive probes across 34 annotated
  fixtures.
- **Diagnostic firing** on broken fixtures — 17 negative probes
  across all 12 `probes_negative/` fixtures pinning D0030
  `unknownColumn`, D0081 `nonNumericArithmetic`, D0082
  `crossTypeComparison`, and D0084 `enumValueMismatch`.
- **Type tracking** through transformations, scoped to D0081 in
  v1.2 via the `PROBE-TYPE-IS` synth-shape path, with raw-mutation
  coverage on D0080 / D0082 until v1.3 brings those D-codes under
  the synth gate.

### Known limitations (v1.3 trackers)

- **`PROBE-TYPE-IS` synth-shape coverage scoped to D0081.** The
  current synth shape (`{df}.select(col("x") + 1)`) falsifies on
  non-numeric. D0080 (`returnTypeMismatch`) and D0082
  (`crossTypeComparison`) need their own synth shapes — tracked
  in
  [`docs/design/schema-tracking-probes.md`](https://github.com/amirnaderi93/pykrete/blob/main/docs/design/schema-tracking-probes.md)
  for v1.3.
- **Path drift between marker source and synth host.** The synth
  writes its assertion into a temp file adjacent to the fixture;
  if the fixture path changes between probe parsing and synth
  execution (rare, but possible under symlinked working trees),
  the assertion misses. v1.3 tracker.
- **Two latent golden mismatches** surfaced by the pre-tag
  docs-sync audit (one in `delta`, one in `mlflow`) where the
  committed golden disagrees with the current binary's output on
  a non-blocking detail. Quarantined for v1.3 — they don't affect
  the release-blocking probe suite, but the goldens should
  converge before the next tag.
- **withColumn output enum-preservation.** Carried forward from
  v1.1: `withColumn("status", lit("shipped"))` checks the literal
  against the sink's enum vocabulary but drops the constraint on
  the output column. Tracker in
  [`docs/design/literal-value-vocabulary.md`](https://github.com/amirnaderi93/pykrete/blob/main/docs/design/literal-value-vocabulary.md)
  polish backlog.
- **Numeric-subtype distinguishability.** Carried forward from
  v1.1: probes still don't distinguish `int` / `long` / `short`
  on column reads.

### Coordinated with

- pykrete-tests last tag: v1.1.0. A v1.2.0 tag of pykrete-tests
  follows separately if the new probes + gates + test classes
  warrant the bump; the existing v1.1.0 vendor still works
  against this pykrete release because the pykrete-core binary
  is bit-identical to v1.1.0.

### Compatibility

- **No pykrete-core binary change.** The Rust workspace
  (`pykrete` + `pykrete-lsp` + `pykrete-wasm`) is bit-identical
  to v1.1.0. The version bump exists to align the public
  pykrete-tests trust posture, the docs surface, and the VS Code
  extension's bundled-binary version pointer with the upstream
  pykrete release line.

## [1.1.0] - 2026-06-02

First minor release after the v1.0.0 ship. Two trust-system additions
land together: **enum constraints** on string-typed columns
(`status: enum["pending", "shipped", ...]`) and the
**schema-tracking probe suite** that pins both the new feature and
the existing v1.0 checks against 47 real-codebase fixtures.

### Added

- **`enum["v1", "v2", ...]` atomic type** (literal-value vocabulary —
  Form A). Declared in a `Schema` class, the type adds a static
  vocabulary check on string literals that flow into the column.
  Order-independent set equality, case- and whitespace-sensitive
  values, full Unicode. `Nullable[enum[...]]` is the canonical
  optional shape. The constraint flows through `Pick`, `Omit`,
  `Merge` (with `D0040` on non-set-equal vocabulary collisions),
  through aliases and renames, through per-value aggregations
  (`first`, `last`, `min`, `max`, `collect_set`, `collect_list`),
  and through branch-form expressions when every branch shares a
  set-equal vocabulary. String-producing operations (`cast`,
  `regexp_replace`, `substr`, `lower`, `upper`, `concat`, …) drop
  the constraint to plain `string`. Documented in
  [`reference/schemas`](https://pykrete.dev/reference/schemas/#enum-valued-strings--enuma-b-).
- **`D0084 enumValueMismatch`** — fires when a string literal
  compared against, or written into, an enum-typed column is not in
  the column's vocabulary. Error severity in every check mode
  (`basic`, `standard`, `strict`); downgradable to warning or off
  per the standard `pykrete.json` rules block. Suggestions reuse
  D0030's Levenshtein routine; ties break on Unicode code-point
  order (Rust `str::cmp`) so the same input always yields the same
  suggestion. Documented in
  [`reference/diagnostics`](https://pykrete.dev/reference/diagnostics/#enumvaluemismatch--d0084).
- **Check sites covered.** `D0084` fires across `col(...) ==
  "lit"` / `col(...) != "lit"`, `col(...).isin("lit", ...)`,
  `.fillna({"col": "lit"})`, `withColumn("col", lit("lit"))`,
  `F.expr("col = 'lit'")` and the SQL `IN ('a', 'b')` form, and
  the branch-form expressions `F.coalesce` /
  `F.when(...).otherwise(...)` / `F.nvl` / `F.ifnull` / `F.nullif`
  when their output flows into an enum-typed sink.
- **Schema-tracking probe suite** vendored in
  [pykrete-tests](https://github.com/amirnaderi93/pykrete-tests).
  127 probes (110 positive + 17 negative) across 47 fixtures (35
  annotated + 12 deliberately-corrupted under `probes_negative/`)
  from the same 10 donors the v1.0 golden-diff suite uses: Apache
  Spark, Delta Lake, Apache Iceberg, Apache Hudi, MLflow, Feast,
  Kedro, quinn, dbt-spark, python-deequ. Probes cover 46 of the 47
  fixtures (the feast `spark_kafka_processor` streaming fixture is
  annotated but probe-free — no typed-DataFrame slot to anchor on).
  Positive probes assert columns resolve cleanly after `.select` /
  `.filter` / `.withColumn` chains; negative probes assert specific
  diagnostics fire on broken fixtures. Probe coverage spans D0030 /
  D0081 / D0082 / D0084; the remaining 15 D-codes in the catalog
  are covered by unit-test snapshots only. See the
  [pykrete-tests README](https://github.com/amirnaderi93/pykrete-tests#schema-tracking-probes-v11)
  and
  [`scripts/PROBES.md`](https://github.com/amirnaderi93/pykrete-tests/blob/main/scripts/PROBES.md).
- **Enum value vocabulary verification in 3 of 10 donors.** Delta
  CDC `_change_type` (`{"insert", "update_preimage",
  "update_postimage", "delete"}`), Hudi `_hoodie_operation`
  (`{"I", "-U", "U", "D"}` — per `HoodieOperation.java`, not the
  `WriteOperationType` values), and MLflow run status (`{"RUNNING",
  "FINISHED", "FAILED", "KILLED", "SCHEDULED"}`). Positive probes
  assert in-vocab literals stay clean across `==` / `.isin` /
  `withColumn` / `F.expr` / `groupBy` chains; negative probes
  assert `D0084` fires on off-vocab typos.

### Changed

- **README "Reliability and trust" section** now states the v1.1
  probe coverage alongside the v1.0 golden-diff coverage: 47
  fixtures total (35 annotated + 12 negative), 127 probes (110
  positive + 17 negative), enum vocabulary verification in 3 of 10
  donors. Already shipped in
  [#77](https://github.com/amirnaderi93/pykrete/pull/77).
- **`docs-site` reference + splash sweep**: `reference/schemas`
  documents the new `enum[...]` type and its preservation /
  drop-side rules; `reference/diagnostics` adds the `D0084` row to
  the catalog and a prose entry under the type-checking section;
  the index splash refreshes the trust line from "32 annotated
  snapshots" to "127 schema-tracking probes across 47 fixtures from
  10 real PySpark codebases" so every surface agrees with the
  README and production-readiness pages.

### Known limitations (v1.2 trackers)

- **`withColumn` output drops the enum constraint.** In v1.1
  `withColumn("status", lit("shipped"))` checks the literal against
  the sink's vocabulary, but the **output column** drops to plain
  `string` — downstream code re-using the returned frame's `status`
  column won't see the constraint preserved. The check at the
  literal still fires at the write site, where the bug lives.
  Tracker:
  [`docs/design/literal-value-vocabulary.md`](https://github.com/amirnaderi93/pykrete/blob/main/docs/design/literal-value-vocabulary.md)
  v1.1-polish backlog.
- **Type-tracking probe class deferred.** The
  `PROBE-TYPE-IS` synthesizer (assert a column's runtime type
  through transforms) is scoped out of v1.1 — the schema-tracking
  probe runner can't bind synthesized type assertions to live local
  scopes yet. Tracker:
  [`docs/design/schema-tracking-probes.md`](https://github.com/amirnaderi93/pykrete/blob/main/docs/design/schema-tracking-probes.md).
- **Numeric subtype distinguishability.** Probes don't yet
  distinguish `int` from `long` from `short` on column reads.
  v1.2 tracker.

### Coordinated with

- pykrete-tests v1.1.0: vendors the 47 fixtures + 127 probes +
  refreshed `diagnostic_catalog.json` with the new `D0084` entry.

## [1.0.0] - 2026-05-31

First stable release. Ten months of pre-1.0 hardening converge into a
SemVer-major commitment on the public-facing surfaces: the JSON output
contract (`schemaVersion: "1"`), the D-code catalog (18 codes —
D0001 / D0010 / D0011 / D0020 / D0021 / D0030 / D0040 / D0050 / D0051 /
D0060 / D0070 / D0071 / D0072 / D0073 / D0080 / D0081 / D0082 /
D0083), the `pykrete check` CLI surface (`--format text|json`, exit
codes, `-V` / `-h` / `-v`), the LSP wire protocol, and the wasm
playground API surface shipped in v0.1.16.

### Added

- **JSON output contract** (`pykrete check --format json`) becomes
  a stability commitment at v1.0. The wire format carries
  `schemaVersion: "1"`; any breaking change post-v1.0 requires a
  SemVer-major bump and a `schemaVersion: "2"`. Consumers pin to
  `schemaVersion`, not pykrete `version`. Documented end-to-end in
  [`about/production-readiness`](https://pykrete.dev/about/production-readiness/).
- **Per-D-code diagnostic catalog snapshot suite** locks in every
  rendered diagnostic message as a reviewable artifact. Adding a
  new D-code now forces an accompanying fixture; wording changes
  fail the snapshot test until explicitly accepted.
- **No-false-positives policy** as a release gate: a regression on
  real Spark code blocks the tag before it goes out.
- **`reference/diagnostics` catalog** in the docs site as the
  canonical user-facing catalog of every D-code we emit, paired
  with the rule names accepted in `pykrete.json`.
- **Reliability and trust** section in the README states what
  pykrete ships against, what it doesn't, and the cross-codebase
  testing methodology that keeps it honest.

### Changed

- **Cross-codebase suite** ([pykrete-tests](https://github.com/amirnaderi93/pykrete-tests))
  vendors 32 annotated fixtures from 10 upstream codebases — Apache
  Spark, Delta Lake, Apache Iceberg (iceberg-python), Apache Hudi,
  MLflow, Feast, Kedro (kedro-plugins), quinn, dbt-spark,
  python-deequ — pinned to specific upstream commits. Every push
  rebuilds pykrete fresh from `main`, re-runs `pykrete check`
  against every fixture, and JSON-diffs against the committed
  goldens. The whole suite emits zero diagnostics against v1.0.0.
- **VS Code extension** ships as `amirnaderi.pykrete` on the
  Visual Studio Marketplace and Open VSX, per-platform `.vsix`
  binaries on macOS arm64/x64, Linux x64, Windows, plus a
  universal fallback.
- **Playground** runs the real `pykrete-wasm` checker in the
  browser with a static PySpark symbol table for hover / completion
  / go-to-definition on Spark types alongside pykrete's own schema
  surface.

### Notes

The pre-1.0 cycle's full per-release breakdown lives in the
[0.1.x history below](#0140---2026-05-31) — every check, every
hardening fix, every cross-codebase false-positive sweep that fed
into v1.0.

## [0.1.40] - 2026-05-31

Final pre-v1.0.0 release. Docs-only: wires the 10-donor cross-codebase
suite into the main README's "Reliability and trust" section now that
every fixture in the suite emits zero diagnostics against the v0.1.39
binary. After this lands, v1.0.0 is the SemVer bump plus tag — no new
work.

### Changed

- **README "Reliability and trust" section** names all 10 donor
  codebases (apache/spark, delta-io/delta, apache/iceberg-python,
  apache/hudi, mlflow/mlflow, feast-dev/feast,
  kedro-org/kedro-plugins, MrPowers/quinn, dbt-labs/dbt-spark,
  awslabs/python-deequ), links the per-donor matrix in the
  pykrete-tests README, and states the now-true claim: every push
  golden-diffs `pykrete check` against 32 annotated fixtures across
  those 10 codebases, and regressions block release. CI badge points
  at `cross-codebase.yml`.
- **docs-site "Real-codebase tests" page** rewritten around the
  10-donor / 32-fixture / all-clean reality. No more v0.1.37-baseline
  or known-finding language: the suite has been clean against the
  current binary since the v0.1.39 coordinated golden refresh.
- **production-readiness "Real-codebase testing" + "Production
  deployments" sections** updated to the same all-clean framing and
  cross-link the donor matrix.

### Coordinated with

- pykrete-tests v0.1.40 (PR amirnaderi93/pykrete-tests#3, merged):
  refreshed the six v0.1.37-baseline goldens (which together carried
  seven findings) that v0.1.39 cleaned up, plus the two restored
  pilots, bringing the suite to 32 fixtures all emitting
  `diagnostics: []` against v0.1.39 and v0.1.40.

### Versions

- Workspace `Cargo.toml`: 0.1.39 → 0.1.40
- VS Code extension: 0.2.34 → 0.2.35

## [0.1.39] - 2026-05-31

Penultimate pre-v1.0.0 release. Closes the last seven false-positive
findings (across six affected fixtures) surfaced by the v0.1.38
cross-codebase suite — a ten-donor sweep of
real OSS PySpark fixtures (apache/spark, mlflow, hudi, iceberg-python,
delta, kedro-plugins, feast, dbt-spark, quinn, python-deequ). Every
one of those goldens now emits zero diagnostics, which is the
v1.0.0 bar. Skipped 0.1.38 in this repo: that version was
pykrete-tests-only, wiring in the goldens that surfaced this work.

### Fixed

- **F1: `df.drop(*cols)` silently tolerates names not in the schema.**
  Spark's `drop` is designed to ignore unknown names — its source
  reads "ignored if schema doesn't contain column name(s)". Pykrete
  used to fire `D0030`, telling users their working production code
  was broken. The new `tolerates_missing_column_names(method)`
  classifier routes `drop`'s name refs through an LSP-only path that
  records them for hover / jump-to-definition but suppresses the
  diagnostic. `drop("region", "missing")` is clean on both names.
- **F2: `df.withColumnsRenamed({"missing": "new_name"})` silently
  tolerates dict keys not in the schema.** Same Spark-design rationale
  as `drop`. `apply_with_columns_renamed` now uses the same LSP-only
  recording path. A mixed dict — known + missing keys — is clean
  end-to-end.
- **F3: Backtick-quoted column refs resolve correctly.** Spark allows
  `` `name` `` to escape identifiers that contain special chars; a
  backtick-wrapped column like `F.col("`info`")` used to false-fire
  `D0030`. Added `split_backtick_aware` in `schema.rs`: it segments
  on `.` outside backticks AND strips backticks from each segment,
  so `` "`info`" `` → `info`, `` "`info`.`name`" `` →
  `["info", "name"]`, `` "`a.b`" `` → one literal segment `a.b`.
  `resolve_path` and `split_qualified` both go through it. A typo
  inside backticks still fires `D0030`.
- **F4: Bare `Struct` is an opaque-struct atomic type.** Real
  production codebases carry nested structs whose shape the user
  hasn't fully modeled (third-party telemetry blobs, opaque
  metadata). `field: Struct` now parses as
  `ColumnType::Struct(vec![])` — the column counts as composite, so
  `F.col("field")` resolves, but inner-field navigation
  (`F.col("field.x")`) degrades silently rather than fire `D0030`.
  Same posture as bare `Array` / `Map` without an element/key-value
  parameter. Added to `COLUMN_TYPE_NAMES` and documented in
  `schemas.md`.
- **F5: `float` is an alias for `double`.** PySpark passes Python
  floats to Spark's runtime, which always coerces them to
  `DoubleType` — `IntegerType` / `FloatType` / `DoubleType` all
  receive float-typed values as Double. Mixed `field: float` and
  `field: double` in the same codebase used to fire `D0010` on the
  former. Strict atomics now resolve `float` to
  `ColumnType::Double`; nested `Array[float]` works; strict mode
  treats `float` and `double` as interchangeable.
- **F6: `F.explode(map_col).alias("k", "v")` emits both columns.**
  Spark's two-arg alias on an exploded map is the canonical way to
  name the (key, value) pair. Pykrete used to take only the first
  alias and drop the second, leaving a one-column derived schema and
  false-firing downstream refs to the value column. New
  `explode_map_aliased_fields` helper, wired into both the
  `apply_column_method("select")` and the `groupBy(...).agg(...)`
  arg-walking paths. Single-arg alias on a map explode (which Spark
  errors at runtime) is left to the existing `select_output_name`
  one-name path.
- **F7: `df.select(F.explode(arr).alias("a"), df.other)` keeps both
  columns in the inferred schema.** `select_output_name` used to
  recognise `col("X")` / `F.col("X")` / `"X"` / `.alias("X")` but
  NOT the attribute / subscript shapes (`df.X`, `df["X"]`) — even
  though `collect_col_refs` already accepted them as column
  references. The mismatch meant the select walked all the way
  through but the second arg's contribution was silently dropped from
  the derived schema, so a downstream `col("other")` false-fired
  `D0030`. `select_output_name` now recognises both shapes, returning
  the attribute name / subscript-literal as the output column name.

### Internal

- `check_column_method_args` takes the method name so it can route
  drop's refs through the silent-skip path.
- New `record_column_refs_tolerating_missing(refs, schema, ctx)`:
  LSP-records each ref without emitting `D0030`.
- New `split_backtick_aware(path)` in `schema.rs`: shared by
  `resolve_path` and `split_qualified`. Unit-tested directly.
- New `explode_map_aliased_fields(arg, recv, tcx)` in
  `strict_operators.rs`: alongside `posexplode_fields`, recognises
  the explode-of-map dual-alias shape.
- `select_output_name` extended with two new arms for the attribute
  and subscript receiver patterns.

### Coordinated golden refresh (pykrete-tests)

Six of the v0.1.38 cross-codebase goldens were captured with the
v0.1.37-baseline false-positive diagnostics (seven findings total
across those six fixtures). After v0.1.39 lands and the binary is
rebuilt, a coordinated PR on pykrete-tests refreshes those six
goldens to empty-diagnostic arrays. The remaining 24 stay unchanged.
The full 32-fixture sweep then carries a single shared expectation:
zero diagnostics, which is the v1.0.0 bar.

## [0.1.37] - 2026-05-31

Final pre-v1.0.0 polish. Two false-positive blockers + four important
items surfaced by the v0.1.35 pre-launch re-audit. Skipped 0.1.35 and
0.1.36 to align the workspace version with the audit's recommendation.

The blockers are trust-killers: pykrete was telling users their
working production code was broken. A senior Spark engineer trialing
pykrete on real code hit one within minutes. Under the project's
"trust > speed, default to delay" principle, these had to land before
v1.0.0.

### Fixed (blockers)

- **B1: Aliased-DataFrame qualified column refs no longer
  false-flag (every site, not just join).** The canonical
  join-disambiguation pattern — `L = left.alias("L"); R =
  right.alias("R"); L.join(R, col("L.region") == col("R.region"))` —
  used to fire `D0030` because the prefixed name `"L.region"` doesn't
  literally exist on either schema. Pykrete now tracks SQL-style
  aliases via `BodyContext.df_aliases` and resolves alias-qualified
  column refs through to the underlying schema at EVERY column-check
  site: `select`, `filter`, `withColumn`, `groupBy`, the join-on
  clause, and the rest of the `report_column_refs` callers. (Round 1
  of the v0.1.37 fix only wired the resolver into the join-on path,
  leaving the most common shape — `L = raw.alias("L");
  L.select(col("L.region"))` — still false-firing. Round 2 lifted the
  resolver into a shared helper so every site honors it uniformly,
  but that lift ITSELF shipped a regression of exactly the class the
  v0.1.37 cycle was meant to eliminate: when ANY alias was in scope,
  the helper hijacked every alias-shaped name and false-fired on
  legitimate nested-struct accessors like `col("addr.city")`. Round 3
  narrows the helper to defer when the prefix doesn't match any
  registered alias, AND adds receiver-first disambiguation in
  `report_column_refs` so nested-struct fields whose names collide
  with a registered alias name still resolve on the receiver schema.)
  A typo on the suffix (`col("L.regoin")`) still fires `D0030`; a
  prefix that doesn't match any alias AND doesn't resolve on the
  receiver fires `D0030` on the full column name.
- **B2: `unionByName(other, allowMissingColumns=True)` no longer
  false-flags (positional form too).** The kwarg PySpark added for
  schema-evolution merges used to be ignored — `D0040` fired whenever
  the two sides differed in column set, the exact case the kwarg
  exists to permit. Pykrete now reads `allowMissingColumns` in BOTH
  forms PySpark accepts: the named kwarg (`allowMissingColumns=True`)
  AND the positional second arg (`unionByName(other, True)`). A
  literal `True` suppresses `D0040` and returns the union of both
  schemas (all columns from both sides); a literal `False` or absent
  flag keeps the strict-match default; a non-literal value (variable,
  expression) falls through conservatively (suppress, on the
  under-checking-over-false-positives principle). (Round 1 only
  honored the kwarg form; round 2 added the positional path.)

### Fixed (importants)

- **I1: Expression-level walker now descends into compound expression
  nodes.** v0.1.26 closed the statement-level blind spot; v0.1.37
  closes the expression-level analog. `analyze_expr` now descends
  into `Expr::Compare`, `Expr::BoolOp`, `Expr::UnaryOp`,
  `Expr::BinOp`, `Expr::Tuple`, `Expr::List`, `Expr::Set`,
  `Expr::Dict`, `Expr::If`, `Expr::Starred`, `Expr::Subscript`,
  `Expr::FString` (each interpolation's expression), `Expr::ListComp`
  / `Expr::SetComp` / `Expr::DictComp` / `Expr::Generator` (each
  generator's `iter` source) so an embedded method call
  (`df.select("typo").count() > 0`, `df.select("typo") is not None`,
  `[df.select("typo")]`, `f"{df.select('typo').count()}"`, `[x for x
  in df.select("typo")]`) still gets its column refs checked. Before
  this, the test expression of an `if` and the right side of many
  other compound forms were silent misses. Round-2 added the
  Subscript / FString / comprehension arms; `Expr::Lambda` is
  deliberately deferred (its body sits inside a new parameter scope
  the analyzer doesn't track yet — analyzing it without that scope
  would false-fire on the lambda parameter name). Tracked as a
  v1.0.1 follow-up.
- **I4: D0070 split into D0070 (`unresolvedImport`) and D0073
  (`transformInputMismatch`).** The transform-input-mismatch check
  was reusing D0070's code and inheriting its `ruleName`
  (`unresolvedImport`) — a label that didn't describe what the check
  actually does. With v1.0.0's JSON output stability contract about
  to land, this had to be split before users started pinning CI
  suppressions to the wrong name. D0073 is the new code for
  transform input mismatches; D0070 retains its original meaning
  (unresolved relative-import path).
- **Architecture: missed `to_string_lossy()` site.** v0.1.32 swept
  LSP paths from `to_string_lossy()` to `to_str()?` but missed
  `project.rs:229`. Now matches the sister sites in `lib.rs` —
  non-UTF8 paths are skipped instead of round-tripped through a lossy
  string that masks the mismatch.
- **Architecture: malformed-`pykrete.json` warning no longer re-fires
  every 30s (watermark survives cache invalidation).** The cold-walk
  loop used to re-populate the `window/showMessage` warning
  unconditionally as long as the file stayed malformed, producing a
  toast-every-30-seconds loop on a chronically broken config.
  `SnapshotCache` now tracks `last_warned_pykrete_json_mtime` and
  suppresses the warning when the mtime is unchanged. Editing the
  file (even if it stays broken) surfaces a fresh warning; fixing it
  clears the watermark so the next break re-warns immediately. Round
  1 still reset the watermark inside `SnapshotCache::invalidate()`,
  which the LSP layer calls on every `workspace/didChangeWatchedFiles`
  notification — so any `.pyk` save re-armed the toast loop after the
  next cold walk. Round 2 stopped clearing the watermark on
  invalidate; it's keyed by `pykrete.json` mtime alone, and
  `gate_malformed_warning` already handles the "fixed → break again"
  path on its own.

### Added

- **D0073 `transformInputMismatch`** — new diagnostic code split from
  D0070. Fires when `df.transform(fn)` is given a DataFrame whose
  schema doesn't match `fn`'s declared parameter schema. Same wording
  as before, new code + rule name.

### Changed

- **`docs/design/spark-coverage.md`** — refreshed the
  `drop_duplicates`, `sampleBy`, `summary`, `describe`, `observe`,
  and `posexplode` rows to reflect their actual v0.1.29 modeling
  (previously showed pre-v0.1.29 status); migrated three
  `operations.rs` filename references to `operations/<sibling>.rs`
  after the v0.1.31 split; added v0.1.37 deferrals section.
- **`CONTRIBUTING.md`** — project-layout tree now shows the
  9-sibling split under `operations/` (was the pre-split single
  `operations.rs`).
- **`docs-site/.../about/production-readiness.md`** — removed the
  mention of a `--debug` flag the CLI doesn't have (only `-V` / `-h`
  / `-v` / `--format` exist on `pykrete check`).

### Known limitations (v1.0 release notes)

Two patterns still produce silent misses; surfaced here so first-day
users hit a documented limitation rather than a surprise. Both are
deferred to v1.0.1 per the audit recommendation.

- **Window spec bound to a local variable.** `w =
  Window.partitionBy("typo"); df.over(w)` doesn't catch the typo —
  the Window spec is bound to a name, so the column ref isn't checked
  against the receiver of the eventual `.over(...)` call. Inline
  usage (`df.over(Window.partitionBy("typo"))`) is checked normally.
  Tracker: `docs/design/spark-coverage.md` item 17.
- **`.orderBy` / `.sort` / `.write.partitionBy` column-ref checks.**
  These methods are currently routed through `is_pass_through_method`
  so the receiver schema flows through correctly, but their string
  arguments aren't column-ref-checked. A typo in
  `df.orderBy("regoin")` is a silent miss. Tracker:
  `docs/design/spark-coverage.md` item 18.

### Verification

`cargo test --workspace` (1,056 passing — round 3 added two new
regression tests pinning the nested-struct survival fix on top of
round 2's across-sites alias coverage, positional `unionByName`
cases, watermark-survives-invalidate test, and walker-descent tests),
`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cd docs-site && npm run build`,
`cd editors/vscode && npm run compile` all green. Launch-gate
snippets verified manually: aliased-DataFrame join (B1) AND
post-join `L.select(col("L.region"))` (round-2 B1 broadening) AND
nested-struct `col("addr.city")` survival when an unrelated alias is
in scope (round-3 regression fix), `unionByName(other, True)`
positional (round-2 B2 broadening), `if df.select("typo")`
compare-form (I1). Two snippets (window-as-local, .orderBy column
checks) confirmed as documented limitations.

Round 3 also replaced the sleep-based
`malformed_pykrete_json_rewarns_when_file_is_edited` test with a
deterministic `filetime::set_file_mtime` bump (no more 1.1 s sleep,
no more CI flake risk on coarse filesystem mtime resolution), and
added a D0073 prose entry under "Setup and import diagnostics" in
`docs-site/.../reference/diagnostics.md` so users hitting the new
code from a CI error get a description without leaving for the
catalog snapshot.

## [0.1.34] - 2026-05-31

Leg 9 of 10 in the v1.0.0 hardening sprint — a docs-only release that
closes the remaining doc-vs-code drifts surfaced by the pre-launch
audit and adds a new **Reliability and trust** section to the README.

### Added

- **README "Reliability and trust" section.** New section explaining
  that pykrete is a development-time checker (never ships to
  production hosts; cannot affect a running pipeline), and how each
  release earns confidence: cross-testing against Apache Spark /
  MLflow / an internal production codebase, 1,018-test CI suite,
  per-D-code snapshot tests, JSON output stability contract from
  v1.0.0, no-false-positives policy, pre-major-release audit cycle.
- **Per-crate READMEs** for `pykrete`, `pykrete-lsp`, `pykrete-wasm`.
  Two- to three-sentence stubs pointing at the main repo for install,
  usage, and docs.
- **Schema reference — case sensitivity section.** Documents that
  atomic names (`int`, `string`, `decimal`, …) are case-sensitive
  lowercase in `.pyk` source, the `Array` / `Map` / `Struct` keywords
  are case-insensitive (legacy compatibility carve-out), and the
  wider Spark SQL vocabulary (`integer`, `bigint`, `tinyint`,
  `float`, `real`, `boolean`) is accepted only inside `.cast("…")`
  strings — folds the v0.1.28 round-4 review's `from_name`
  case-sensitivity nuance into the user-facing docs.
- **`numeric` / `dec` aliases documented** in the atomic types
  section — Spark SQL aliases for `decimal`, parameterised and bare
  forms resolve identically.
- **Three new CHANGELOG link-refs** (v0.1.31, v0.1.32, v0.1.33,
  v0.1.34) plus the `Unreleased` compare base bumped to v0.1.34.

### Changed

- **Schema reference — full rewrite.** Dropped `List[T]` / `Dict[K, V]`
  examples in favour of the Spark-aligned `Array[T]` / `Map[K, V]`
  forms pykrete actually uses. Dropped the `Join[A, B]` and
  `GroupBy[S, k]` operator sections — these appeared in early v0.1
  specs but never shipped; the actual operator surface is `Pick`,
  `Omit`, `Merge`. Aligned the cross-file example on `.pyk` (a `.py`
  schema module isn't walked at check time). Tightened the `Pick` /
  `Omit` / `Merge` descriptions against `schema.rs`.
- **Production-readiness sweep.** Dropped non-existent `Join` /
  `GroupBy` operators from the stability commitments list. Promoted
  the wasm API surface from "not yet shipped" to "shipped in v0.1.16;
  current shape stable until v1.0, frozen per SemVer from v1.0
  onward" — single-file analyzer wrapper, scope explicitly noted.
  Replaced the now-stale v0.1.7 → v0.1.15 release-cadence story with
  a release-line-neutral phrasing. Linked to the new README
  Reliability and trust section from both the TL;DR and the
  Production deployments section.
- **Playground install copy.** Replaced the misleading
  `pip install pykrete` (pykrete is a Rust binary, not on PyPI) with
  the real install paths (Homebrew, cargo, Windows MSI, VS Code
  marketplace links). Tightened the "no general Python type
  checking" copy to reflect the static PySpark-symbol hover /
  completion / go-to-definition layer that's been part of the
  playground since v0.1.21.
- **VS Code marketplace CHANGELOG backfill.** Collapsed the
  v0.2.14 – v0.2.21 and v0.2.23 – v0.2.27 gaps into two summary
  stubs pointing at the main CHANGELOG, plus fresh entries for
  v0.2.30 (tracks v0.1.33) and v0.2.31 (tracks v0.1.34). Marketplace
  shoppers no longer see a broken-looking version history.
- **Stale version-number sweep.** Replaced `v0.1.15` references in
  the install guide, roadmap, pykrete-tests, and production-readiness
  pages with current versions or release-line-neutral phrasing.
- **README operator list.** Added an explicit "TypeScript-style
  schema composition — `Pick`, `Omit`, and `Merge`" bullet to the
  "What you get" list.

### Verification

`cargo test --workspace` (1,018 passing), `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cd docs-site && npm run build`,
`cd editors/vscode && npm run compile` all green.

## [0.1.33] - 2026-05-31

Leg 8 of 10 in the v1.0.0 hardening sprint. Two items, both surfaced by
the pre-launch audit: deliver the long-promised JSON CLI output, and
add a per-D-code snapshot test that doubles as the canonical "what
pykrete says when D00xx fires" reference.

**`pykrete check --format json`.** Production-readiness has been
promising machine-readable output since v0.1.15; this release delivers
it. New `--format <text|json>` flag on `pykrete check` — default
`text` is byte-identical to today's CLI behaviour, `json` emits one
object on stdout with the shape:

```json
{
  "schemaVersion": "1",
  "version": "0.1.33",
  "diagnostics": [
    {
      "file": "path/to/file.pyk",
      "line": 12, "column": 8,
      "endLine": 12, "endColumn": 14,
      "code": "D0030",
      "ruleName": "unknownColumn",
      "severity": "error",
      "source": "pykrete",
      "message": "Column 'regoin' does not exist on schema 'Sale'. Did you mean 'region'?",
      "suggestion": "region",
      "relatedInformation": []
    }
  ],
  "summary": {
    "filesChecked": 12,
    "errorCount": 1,
    "warningCount": 0
  }
}
```

`schemaVersion` versions the wire format itself (consumers pin to
that, not the pykrete `version`); `source` always equals `"pykrete"`
so CI aggregators like reviewdog can disambiguate from other linters;
`suggestion` carries the structured "did you mean…" replacement that
LSP already round-trips for quick-fixes, or `null` when the
diagnostic doesn't have one. Positions are 1-indexed (matching the
existing `text` output); pykrete-lsp re-indexes to 0-indexed on the
wire per the LSP spec, so tools consuming the CLI's JSON directly
should not.

Exit codes are unchanged: `0` when no diagnostics, `1` when any
diagnostic fires (error _or_ warning). This is deliberate — it
matches today's text-format behaviour and lets CI scripts react
uniformly to warnings like `D0072 duplicateSchemaName` without
having to parse the summary block. Consumers that want "errors only"
semantics will be able to opt in via a future `--max-severity` flag
(tracked in `docs/design/spark-coverage.md` as a v1.1 follow-up).
The JSON schema and exit codes become a **stability contract at
v1.0.0** — any breaking change post-v1.0 requires a SemVer-major
bump (and a `schemaVersion` bump to `"2"`); see `Production
readiness → JSON output stability contract` for the full scope of
what's covered. Until v1.0 it's a `0.x` API and may still shift if a
real adopter surfaces a fork in the design.

**Diagnostic catalog snapshot test.** New
`crates/pykrete/tests/diagnostic_catalog.rs` builds a minimal `.pyk`
fixture per D-code (18 codes total), runs the checker, and snapshots
the rendered diagnostic with `insta`. A coverage assertion at the top
iterates `pykrete::diagnostics::DIAGNOSTIC_CATALOG` and fails if any
code lacks a fixture — adding a new D-code now forces an accompanying
snapshot. Wording changes fail the snapshot test until explicitly
accepted with `cargo insta accept`, locking in the message text as a
reviewable artifact rather than something that drifts silently. The
`rule_name` lookup itself moved from an inline `match` to iterating
`DIAGNOSTIC_CATALOG`, so the catalog list is the single source of
truth for both the runtime mapping and the test.

**`DIAGNOSTIC_CATALOG` is now a public const.** Exposed from
`pykrete::diagnostics` so the catalog test (and future tooling) can
iterate every known code without going through the private `match`.
Public API surface: a `&[(&str, &str)]` of `(code, rule_name)` pairs.
Adding a code to this list is what makes the snapshot test require a
fixture.

## [0.1.32] - 2026-05-31

Architecture-cleanups pass — leg 7 of the v1.0.0 hardening sprint.
Bundles the eight Important items the architecture audit surfaced
against v0.1.30 with the cosmetic minors from the three-lens review of
v0.1.31's `operations.rs` split. No user-visible behaviour change in
the checker; one new editor-visible signal (malformed `pykrete.json`
warnings, below).

**LSP discovery via the `which` crate.** `pykrete-lsp`'s Python-engine
discovery used to test each candidate (`basedpyright-langserver`,
`pyright-langserver`) for PATH presence by spawning it with `--version`
and waiting on exit. That blocked LSP startup on any candidate whose
`--version` didn't terminate promptly, and ran whatever binary
happened to share the name. v0.1.32 routes the check through
`which::which(...)` — a single PATH scan, zero spawns. Added a test
that pins the negative result for a definitely-nonexistent program
name (a guarantee the old probe path couldn't pin: it would still
spawn and fall through).

**Path → URL plumbing.** The LSP layer's path/URL coordinate
translation went through `Path::to_string_lossy()` at three sites and
fell back to the focus URI when `Url::from_file_path` failed at the
goto-definition site. Both were quietly wrong: `to_string_lossy`
corrupts non-UTF-8 paths (silently producing a non-roundtrippable
string), and the focus-URI fallback teleported cross-file
goto-definition to the wrong file. v0.1.32 swaps `to_string_lossy()` →
`to_str()` (the snapshot cache already keys paths as `String`, so
non-UTF-8 paths could never have been in the snapshot anyway — bailing
to `None` matches the real lookup result), and drops the wrong
fallback in favour of an LSP-spec `null` response — returning `null`
from goto-definition when the path isn't valid UTF-8 is the spec-correct
behaviour, whereas the prior fallback teleported the user to the focus
file's location. Two tests pin the change: one valid-UTF-8 non-ASCII
URI round-trips through goto-definition cleanly, and (on Unix only)
a path containing a raw `0xFF` byte short-circuits to `None` at the
strict `to_str()?` site instead of taking the lossy fallback.

**Malformed `pykrete.json` is no longer silent.** A `pykrete.json` that
couldn't be parsed used to fall back to `Config::default()` with no
log, notification, or signal — the user sat with stale rules in effect
and never learned why. v0.1.32 makes the snapshot cache capture the
parse-error detail at cold-walk time and surface it on the next
diagnostic publish: one `window/showMessage` warning ("malformed
pykrete.json at <path> — using defaults") plus one `window/logMessage`
with the parse error for the LSP output channel. The cache drains the
warning once per build (so a still-malformed file on every typing
keystroke produces exactly one notification per build), but every cold
walk — one per 30 s today — re-populates the warning unconditionally
when the file is still malformed, so in practice the notification
re-fires roughly every 30 s. Suppressing re-emission until the
`pykrete.json` mtime drifts is tracked as item 15 in
`docs/design/spark-coverage.md` for v1.1. (Honesty correction: the
initial v0.1.32 wording on this entry said "re-fires when mtime drifts
and the next cold walk re-reads" — that overclaimed; the actual gate
is the cold-walk interval, not mtime.)

**Hover renderer cleanup.** `hover.rs` had ~29
`writeln!(md, ...).unwrap()` calls on String writes that are
infallible in practice. Replaced with the explicit
`let _ = writeln!(...)` shape — same semantics, less stamping, makes
the "infallible to a String" intent read at a glance.

**`BodyContext` docstring tightened.** The four `RefCell` fields on
`BodyContext` (`local_names`, `column_refs`, `local_bindings`,
`call_results`) carry interior-mutability state that the analysis pass
appends to through `&self`. The audit flagged the panic-on-aliased-
borrow risk. v0.1.32 documents the actual invariant: every
`borrow_mut` is confined to the eight `record_*` / `take_*` /
`mark_local` / `is_locally_bound` helpers on the same `impl`, each of
which holds its borrow only for the duration of the call. A
borrow-conflict panic is impossible as long as that locality holds.
(`grep` on the rest of `operations/` confirms zero direct borrows.)

**Reader-receiver heuristic documented.** `is_dataframe_reader_expr`
recognises `<X>.read.<format>(…)` structurally without verifying that
`<X>` is a `SparkSession` — codebases bind the session as `spark`,
`ss`, `sess`, etc. The trade-off: a non-Spark API exposing a similar
`.read.<format>(...)` shape (an in-house loader, say) also matches and
yields opaque instead of the loader's real return type. Workaround is
identical to a genuine `spark.read`: re-anchor with
`.cast(DataFrame[X])`. Documented in `reference/operations.md`'s IO
section. Tightening the receiver check requires plumbing binding
context through `shapes.rs` (currently `&Expr`-only) — deferred as a
v1.1 follow-up since the workaround is the same.

**Visibility tightening (cosmetic).** Two `pub(super) fn` helpers in
`operations/shapes.rs` (`is_dataframe_reader_expr`,
`is_dataframe_reader_format`) had no cross-module callers post-split;
demoted to private `fn`. 22 `pub fn` impl methods on the
`pub(crate) struct BodyContext` switched to `pub(crate) fn` to make
the actual visibility ceiling read at the call site. Added a comment
explaining the `unreachable!()` arm in `column_methods.rs`'s
chained-field-access walker (outer match arm already pinned the
variant set; inner re-match cannot see any other).

**Audit items previously delivered.** `Schema::fields()` memoization
(v0.1.30 PR #61) and wasm `word_range_at` two-pass scan (v0.1.30 PR
\#61) are confirmed still in place. No code change here.

## [0.1.31] - 2026-05-30

Architecture-performance hardening, leg 6 of the v1.0.0 sprint. Pure
module reorganization, no user-visible behaviour change.

**`operations.rs` split.** The PySpark operations checker — analyzer,
`BodyContext`, statement walker, `analyze_expr` dispatch, column-method
checking + result inference, type-inference engine, strict-mode
operator checks, two-DataFrame methods, and `col(...)` reference
discovery — has lived in a single 6,000-line file. The architecture
audit flagged it as a blocker for parallel-PR development: every
analyzer touch recompiles the whole `pykrete` crate, every analyzer PR
diffs against the same file, and visibility-by-default leaked internal
types just so `hover.rs` / `completion.rs` could see them. v0.1.31
splits the file along its existing nine section banners into sibling
files under `crates/pykrete/src/operations/`:

- `shapes.rs` — `column_method_shape`, `two_df_method`, terminal
  recognizers, `spark.read.*` opaque-source recognizer.
- `context.rs` — `BodyContext`, `ColumnRefTrace`,
  `LocalBindingTrace`, `CallResultTrace`, `TypeCtx`, the synthetic-name
  intern pool.
- `driver.rs` — `check_function_body`, `walk_body`, `walk_stmt`,
  `handle_ann_assign`, `check_return_type`.
- `expr.rs` — `analyze_expr`, `analyze_method_call`, the
  `transform`/`agg`/`groupBy` shortcuts, generic-method routing.
- `column_methods.rs` — `check_column_method_args`,
  `apply_with_columns`, `apply_melt`, fillna-dict / subset-kwarg
  checks.
- `column_exprs.rs` — `infer_expr_type`, `function_result_type`, the
  `when`/`struct`/`getField` shape-inference engine.
- `strict_operators.rs` — `report_expr_type_errors` (D0081 / D0082),
  `apply_column_method`, `apply_select_expr`, SQL-fragment-reference
  reporting.
- `two_df.rs` — `union`, `unionByName`, `join`, `crossJoin`,
  `apply_concat`, nullability strip.
- `col_refs.rs` — `col(...)` / `df.X` / `df["X"]` / `F.<fn>("x")`
  reference discovery.

All 985 tests pass unchanged; visibility tightened on the public
surface (`BodyContext` and its trace types are now `pub(crate)` rather
than `pub`); cross-section helpers default to `pub(super)` so each
sibling exposes only what the others need.

## [0.1.30] - 2026-05-30

Architecture-performance hardening, leg 5 of the v1.0.0 sprint. Three
independent hot-path wins, no user-visible behaviour change.

**LSP snapshot cache.** Every cross-file LSP request — diagnostics on
`didChange`, hover, definition, completion — previously walked the
project root and re-read every closed `.pyk` file from disk. Now a
tiered cache keyed on `(project_root, pykrete.json mtime)` holds
closed-file bodies as `Arc<String>`: a HOT tier reads open editor
buffers live, WARM stat-checks recently-touched closed files at most
once a second, and COLD does a full `read_dir` walk only on first
request, every 30 s, or on project-key drift /
`didChangeWatchedFiles` / didOpen for a path outside the tracked
union. A 20 MB body cap engages a two-pass cold walk: pass 1
enumerates every `.pyk` path with no body I/O, pass 2 reads bodies
into `Arc`s until the cap is reached and then leaves remaining entries
with `body = None`. Snapshot assembly does an on-demand
`read_to_string` for any `None` entry, so the tracked union stays
complete on big codebases and `didOpen` outside the union doesn't
thrash the cold walk. A new `pykrete/refreshSnapshot` custom LSP
command drops every tier — the escape hatch for the 30 s cold-walk
staleness window when no file watcher is wired.

**`Schema::fields` memoization.** The schema-field inheritance walk +
override merge ran on every diagnostic, hover, completion, and symbol
pass — for a file with ~20 schemas, hundreds of redundant walks per
request. Cached behind a per-instance `OnceLock`; the return type
shifts from `Vec<…>` to `&[…]` so callers see the cached slice
directly. Uses `OnceLock` (not `OnceCell`) to keep `Schema` `Sync` for
future parallel checker passes.

**`word_range_at` O(line) scan.** The wasm playground hover path was
building a `Vec<(line, col, char)>` over the entire source file just
to find the identifier under the cursor. Rewritten as a two-pass char
scan that buffers only the current line — no whole-source allocation.

## [0.1.29]

Spark coverage hardening, part 4 — the "important"-tier audit follow-ups.
Six high-traffic gaps from the pre-launch audit are now closed.

**High-traffic `F.*` functions.** Seven Spark 3.4–3.5 additions —
`F.try_divide`, `F.any_value`, `F.array_agg`, `F.count_if`,
`F.date_diff`, `F.unix_date`, `F.get` — get column-ref checking and a
modeled result type. `F.try_divide(a, b)` returns `double`,
`F.any_value(col)` passes through the input type, `F.array_agg(col)`
wraps the input as `array<T>`, `F.count_if(predicate)` returns `long`,
`F.date_diff(end, start)` and `F.unix_date(date)` return `int`, and
`F.get(array, i)` returns the element type (same shape as
`F.element_at` on arrays — `get` is just the null-on-out-of-bounds
sibling). Typos in any of these now fire `D0030`, and the result
flows into downstream `.cast(...)` / return-type checks.

**`posexplode` / `posexplode_outer`** now expand to **two** output
columns — `pos: int` and `col: <element-type>` — when used inside
`select` or `agg`. Previously pykrete named only `col` (matching the
`explode` special case), which silently lost `pos`; a follow-up
`.select(col("pos"))` mis-reported the column as missing.

**`df.summary` / `df.describe` / `df.observe`** are modeled. `summary`
and `describe` produce a statistics table whose schema depends on the
receiver's numeric subset (data-dependent), so pykrete treats them as
opaque — the chain dies cleanly and the user re-anchors with
`.cast(DataFrame[X])`. `observe(...)` is an observability hook that
returns the receiver unchanged, so it's a pass-through: a downstream
typo still resolves against the original schema.

**`groupBy(...).pivot(...).agg(...)`** now checks the agg's column
references against the pre-pivot schema. Previously the pivot killed
the chain, so a typo like `agg(F.sum("amunt"))` went silent. The
post-`.agg` result schema is Unknown — pivot's output columns depend
on the runtime pivot values pykrete can't see — but the input check
covers the common bug. The same termination contract now applies to
the shorthand aggregate paths: `groupBy(...).pivot(...).count()` and
`.{sum,max,min,mean,avg}(col)` check the input column against the
pre-pivot schema, then return Unknown so the chain dies cleanly
instead of synthesizing a wrong concrete schema (which would have
fired false-positive `D0030` on legitimate pivot-value column
references downstream).

**`dropDuplicates` / `drop_duplicates` parity.** Both names are Spark
aliases for the same method; pykrete previously modeled the camelCase
form as schema-preserving and the snake_case form as column-check-only
(silently losing the schema downstream). Both names now route to the
same handler: arguments are checked AND the schema flows through.

**`sampleBy`** is schema-preserving like `sample`, and its first
positional arg — the stratification column — is now column-ref
checked against the receiver. A typo (`sampleBy("regoin", ...)`)
fires `D0030`; the `col(...)` expression form is recognized too. The
fractions dict's keys are stratum *values*, not column names, and are
correctly left alone. `randomSplit` remains unmodeled — it returns
`list[DataFrame]`, a shape pykrete can't yet thread through tuple
unpacking; documented as a v1.1 gap.

**`describe(*cols)`** now column-ref-checks its positional string
arguments against the receiver before bailing Unknown. A typo like
`describe("amunt")` fires `D0030`. The result schema stays Unknown
(it's a data-dependent statistics table). `summary` is unchanged —
its arguments are statistic-name strings (`"mean"`, `"50%"`), not
column names.

**`observe(name, *exprs)`** now walks the expression args for
embedded column references. The first arg is a metric label (a
string literal, not a column ref) and is correctly skipped; embedded
column refs in the remaining args (`F.sum("typo").alias("total")`)
fire `D0030`. The receiver's schema continues to flow through
unchanged.

## [0.1.28]

Spark coverage hardening, part 3 — the last v1.0.0 audit blocker:
pykrete's type vocabulary now includes Spark's `DecimalType`,
`ByteType`, `ShortType`, and `BinaryType`.

`decimal(p, s)` is now a first-class atomic. Write `amount:
decimal(18, 2)` in a Schema, or `col("amount").cast("decimal(18,2)")`
in a chain, and pykrete tracks the type through downstream column
references and return-type checks. Production money columns —
the canonical case from the audit — finally type as decimal
instead of degrading to Unknown.

`byte`, `short`, and `binary` join the atomic set too. `byte`
and `short` are full integers under the type-family rules
(numeric, sum-widens-to-long matching Spark); `binary` is
opaque (no arithmetic, no string comparison — strict-mode
operator checks treat it like a collection).

Aggregate widening is honest but simplified. `sum`/`mean` of
a decimal stays a (bare) decimal in pykrete's model; Spark
widens precision and scale (`decimal(p+10, s)` for sum,
`decimal(p+4, s+4)` for mean, both capped at 38), and the
precision-growth refinement is parked as a v1.1 polish item.
`sum(byte)` and `sum(short)` widen to `long`, matching Spark.

The `mean(decimal)` and `avg(decimal)` rule now applies to
both the `groupBy.mean(col)` shortcut and `F.mean(col)` inside
`.agg(...)` — previously the function-form path short-circuited
to `Double` regardless of input, while the shortcut path
correctly kept the decimal. Both surfaces now agree on
**every** input: `mean`/`avg` of a numeric input promotes to
`double` (decimal stays decimal), and `mean`/`avg`/`sum` of a
non-numeric column (string, bool, date, binary) pins no result
type on either path — Spark rejects those aggregates at
runtime, so the previous "function form pins a wrong Double"
behaviour was an actively misleading signal.

`decimal(p)` (single-arg form) is accepted with scale defaulted
to 0, matching Spark SQL's `DECIMAL(p)` shorthand. Precision is
validated against Spark's cap (`1..=38`) and scale must not
exceed precision; violating either fires `D0011`.

`.cast("...")` with a target string that isn't a recognized
Spark type (e.g. `decimial(18,2)`) is now flagged with
`D0011` — previously the typo was silently swallowed (no type
pinned, no warning either). The type-constructor form
(`.cast(IntegerType())`) and any computed expression stay
permissive. The typo check skips known-Spark-but-unmodeled
types (`varchar(n)`, `char(n)`, `interval` and its compound
forms, `timestamp_ntz`, `void`, `null`) so legitimate Spark
casts that pykrete simply doesn't pin a type on yet aren't
false-rejected.

`numeric` and `dec` are accepted as aliases for `decimal` end-
to-end — Spark SQL treats them as synonyms, so `amount:
numeric(18, 2)` in a Schema and `.cast("dec(10)")` in a chain
both resolve identically to the corresponding `decimal`. The
strict schema-annotation surface keeps its long-standing case-
sensitivity contract (`Int` is rejected, and the alias forms
follow the same rule — `Numeric(18, 2)` in a class body fires
`D0011`); the Spark cast path stays case-insensitive
(`NUMERIC`, `Dec`, `DECIMAL` all resolve), matching Spark SQL's
own behaviour on type names.

Decimal bounds validation (`1..=38` precision, `scale <=
precision`) is now driven by a single shared helper
(`validate_decimal_args`), so the same rules fire from both the
cast-string parser and the schema-annotation parser. Previously
the two paths each carried their own bounds check; the
extraction protects against drift on future tweaks.

The cast-typo allowlist for unmodeled Spark types now handles
parenthesised forms more tightly: `varchar(n)` and `char(n)`
require a positive integer `n`, and bare-only types
(`interval`, `void`, `null`, `timestamp_ntz`) reject paren
forms outright — `.cast("interval(5)")` and `.cast("void(0)")`
now fire `D0011` instead of being silently allowed. The
compound `INTERVAL <unit> TO <unit>` form is normalised case-
insensitively and accepts per-unit precision args (`INTERVAL
DAY(3) TO SECOND(6)`), so real Spark compound-interval casts
aren't false-rejected.

`schemas.md` updated to list the real v0.1.28 atomic set —
`float` and `bytes` (which were never recognised) are out;
`decimal(p, s)`, `byte`, `short`, and `binary` are in.

## [0.1.27]

Spark coverage hardening, part 2. Three more pre-v1.0.0 audit
blockers closed, plus four small follow-ups from the v0.1.26 review.

Expression-form join keys are now checked. Pre-v0.1.27 a
`df.join(other, col("a") == col("b"))` clause landed in the
"complex expression — give up" bucket, so a typo in either side
slipped past D0030. The analyzer now walks the on-expression for
every column reference (recursing through boolean ops and
comparisons, so an `AND` of two equalities is covered) and validates
each against the union of both sides — names missing from BOTH
fire D0030. The output schema for expression-form joins also
matches Spark now: both join-key columns are kept (the string/list
form still coalesces, as before).

`fillna({"col": value, ...})` (and `na.fill({"col": ..., ...})`)
now checks the dict's literal keys against the receiver's schema —
the most common production form of `fillna`, where a typo previously
went undetected. Non-dict-literal first args (a scalar value, a
variable holding a dict) fall through silently rather than false-
flag.

`melt` and `unpivot` (Spark 3.4+) are documented correctly. The
reference operations table listed both as "unmodeled" while the
analyzer has modeled them since v0.1.21 — both column-existence
checks on `ids`/`values` and the long-form output schema (`ids +
[variableColumnName: string, valueColumnName: T]` with nullability
propagation). Docs now match code.

Four follow-ups from PR #56 review:

- CHANGELOG correction: the v0.1.26 entry claimed `Stmt::Match`
  bodies are walked and that synthetic-name interning is per-
  `BodyContext`. Both were wrong — `Stmt::Match` is deferred until
  pattern binding is modeled (and is now tracked in code as a
  v1.1 follow-up), and synthetic-name interning is process-wide
  via `OnceLock<Mutex<HashSet<&'static str>>>`. The original
  v0.1.26 entry is left intact per Keep-a-Changelog convention.
- `grouped_count_schema` now exercised by a test where `count` is
  itself a grouping key: `df.groupBy("count").count()` keeps a
  single `count` column (the synthetic shadows the user field).
- `Stmt::FunctionDef` and `Stmt::ClassDef` nested inside a function
  body are now walked, so column references inside a nested helper
  are still checked.
- `Stmt::Match` deferral now carries a tracker comment linking to
  the v1.1 plan note in `docs/design/spark-coverage.md`.

## [0.1.26]

Spark coverage hardening, part 1 of the pre-v1.0.0 sprint. Two blocker-
grade gaps in the analyzer closed: a body-walker blind spot inside
control-flow statements, and a `GroupedData` schema regression on the
aggregate shortcuts.

The body walker now descends into every `if`/`elif`/`else`, `for`,
`while`, `with`, `try`/`except`/`finally`, and `match` block — pre-
v0.1.26 it only checked top-level `Assign`/`AnnAssign`/`Return`/`Expr`
statements, so a typo like `if debug: return raw.select("regoin")`
never reached the col-ref checker. Loop targets, `with ... as x:`
bindings, and `except E as e:` exception bindings are now marked as
local names (so D0051's local-rebind detection works inside nested
blocks), and the `with`-as binding additionally records its schema
when the context expression resolves to a DataFrame. `AugAssign`
(`x += expr`) is also walked.

On the GroupedData front, `groupBy(keys).count()` is no longer treated
as a chain-killing terminal — it now produces a `{keys..., count: long}`
schema so the routine `.filter(col("count") > 10)` follow-up gets
checked. `df.count()` (the actual terminal) is unchanged. The
`groupBy.sum()` / `.max()` / `.min()` / `.mean()` / `.avg()` shortcuts
now return `{keys..., method(col)...}` instead of `None`, mirroring
Spark's auto-generated output column names. Type derivation follows
pyspark's rules: `sum` widens `int` → `long`, preserves `long` and
`double`; `mean`/`avg` always return `double`; `max`/`min` preserve
the input type. Synthetic names are interned per-`BodyContext` so
they can flow through the `DerivedField` chain at the source
lifetime.

## [0.1.25]

Playground polish. Monaco's bundled Python tokenizer is context-free
and matches identifiers against a single flat list that mixes
real keywords (`def`, `class`, `return`) with Python builtins
(`filter`, `map`, `sum`, `len`, `type`, `print`, `range`, `zip`,
`iter`, `next`, etc.) — so a method call like
`sales.filter(col("amount") < 1000)` rendered `filter` in the
keyword color while sibling chains like `sales.select(...)` or
`sales.groupBy(...)` rendered their methods in the regular identifier
color. The asymmetry was distracting and the user asked for it fixed.

This release ships a Monarch tokenizer override that copies Monaco's
v0.52.2 Python grammar verbatim, then adds one rule: a `.` followed by
an identifier pushes into a dedicated `property` state where the
identifier always tokenizes as a plain `identifier`, never matched
against `@keywords`. Top-level uses (e.g. `len(my_list)`) keep their
existing color — only post-dot identifiers change. The override is
installed in `beforeMount`, before any editor model is created, so
existing buffers pick it up immediately. Everything else about
Monaco's Python tokenizing — strings, f-strings, triple-quoted
strings, numbers, hex, decorators, comments — is byte-identical to
upstream. Docs-only release.

## [0.1.24]

Hotfix. v0.1.23 mirrored Monaco's color rules onto
`#playground-overflow-root` but missed the layout rules — Monaco's
`suggest.css` scopes `display: flex`, padding, and white-space on
`.monaco-editor .suggest-widget .monaco-list .monaco-list-row` under
the same `.monaco-editor` ancestor it scopes everything else under, so
the rebased rows existed in DOM but collapsed to zero height and the
popup still rendered as an empty dark rectangle. Fix gives the
body-level overflow host the `monaco-editor` class itself, so
Monaco's own theme variables (written onto
`.monaco-editor, .monaco-diff-editor, .monaco-component`) and every
ancestor-scoped widget rule (suggest layout, hover styling,
symbolIcon colors) fire naturally for the widgets that mount inside
it. Drops the manual `--vscode-*` forwarding and the mirror rules
v0.1.22 and v0.1.23 added — Monaco's own stylesheet now does the
work. Docs-only release.

## [0.1.23]

Hotfix. v0.1.22 fixed the suggest popup's background fill, but the list
items inside the popup still rendered invisible — the dark frame and
footer painted correctly, the focused row's blue highlight bar was
there, but no method names, no codicons, no docs preview. Same root
cause as v0.1.22, one layer deeper: Monaco scopes its
`suggest.css` row/label/icon-label rules and its
`symbolIcons.css` codicon rules under `.monaco-editor`, and the
overflow-widgets host node sits outside that subtree, so the rules
never fire on the rebased widget. Forwards the rest of the `vs-dark`
palette onto `#playground-overflow-root` (foreground, list selection,
the `symbolIcon.*Foreground` family) and mirrors the row label
and `.codicon.codicon-symbol-*` rules under
`#playground-overflow-root` so the widget paints with the right
colors. Docs-only release.

## [0.1.22]

Hotfix. v0.1.21 reparented Monaco's hover/suggest/parameter-hint
widgets onto a body-level host node to clear Starlight's right-hand
TOC sidebar — verified working — but the widgets came out with no
background fill because Monaco only writes its `--vscode-*` theme
variables onto the `.monaco-editor, .monaco-diff-editor,
.monaco-component` selector, and the new host node sits outside that
subtree. The widgets fell back to transparent and text showed through
the editor source behind them. Fix forwards the `vs-dark` palette
values for hover/suggest/parameter-hint/list/textLink/textCodeBlock
onto `#playground-overflow-root` so Monaco's own
`hover.css` / `suggest.css` rules resolve there too, plus a
belt-and-suspenders explicit rule directly on the widget classes.
Docs-only release; no checker/LSP/extension behavior changed.

## [0.1.21]

Playground rebuild — drops `pyright-browser`, ships a static pyspark
symbol table. v0.1.20's `@typefox/pyright-browser` 1.1.299 turned out
to ship without typeshed fallback files, so even `int` was reported
"not defined" and `sales.<dot>` showed only generic Python keywords;
the worker also dragged a few MB of stale-fork bundle. The
playground now reads a build-time-generated symbol table
(`docs-site/scripts/gen-pyspark-symbols.py` against pyspark 3.5.1)
for DataFrame / Column / GroupedData / `F.*` / Window hover and
completion, dispatched on a per-source DataFrame-name heuristic.
Pykrete's hover/completion/definition still run first and own the
schema-related surface; the Spark table only fires when pykrete
returns nothing. Hover-popup z-index fix in v0.1.20 didn't actually
land — Starlight's `.main-pane` sets `isolation: isolate`, which
trapped Monaco's widget root. Two-part fix: hoist Monaco's overflow
widget container to a body-level `#playground-overflow-root` (via
`overflowWidgetsDomNode`) and z-index 9999 on that node so it clears
every Starlight chrome layer. Bundle: Playground.js shrinks from
~80 KB to 29 KB; pyspark-symbols.json adds 192 KB of static asset
loaded lazily on `/playground` only. Docs-only; no checker behavior
changed.

## [0.1.20]

Hotfix on v0.1.19's playground polish. Pyright was still firing
"Expected no type arguments for class DataFrame" on `DataFrame[Sale]`
because `@typefox/pyright-browser` 1.1.299 handles
`class Foo(Generic[T])` less robustly than current pyright — switched
to explicit `__class_getitem__` on DataFrame, which works in every
pyright version. PREAMBLE_LINES auto-recomputes. Also bumps the
hover popup's z-index above Starlight's right-hand TOC sidebar
(superseded by the more comprehensive fix in v0.1.21). Docs-only
release; no checker behavior changed.

## [0.1.19]

Playground implicit imports. A hidden 27-line preamble is prepended
to whatever pyright sees, declaring `Schema`, `DataFrame[T]`, `col`,
`lit`, `F` (with `__getattr__` open-class so any method resolves to
`Any`), and lowercase pykrete type aliases (`string`, `double`,
`long`, `short`, `byte`, `binary`, `date`, `timestamp`, `decimal`).
All five LSP boundaries (`didOpen`, `didChange`, diagnostics, hover,
completion, definition) handle the offset arithmetic in
`pyrightClient.ts`. Result: playground users can write code without
any import lines, pyright stops complaining about
Schema/DataFrame/Sale/col/F, and real Python errors (`1 + "foo"`)
are still caught. Pykrete's analyzer path unchanged. 825 workspace
tests (no change since v0.1.18 — playground-only release).

## [0.1.18]

The playground becomes a full Python + pykrete IDE.
`@typefox/pyright-browser` runs in a Web Worker behind a hand-rolled
LSP client (`pyrightClient.ts` on `vscode-jsonrpc/browser`) and is
multiplexed alongside pykrete using the same rules as the VS Code
extension's `pykrete-lsp/multiplex.rs`: diagnostics union, hover
stacked with `---` rule (pykrete first; schema-hover suppresses
pyright's contribution), completion pykrete-first then pyright with
explicit LSP→Monaco kind mapping. Editor polish:
`quickSuggestions.strings: true` so completions appear inside string
literals (pykrete's whole completion surface lives in strings),
`fixedOverflowWidgets: true` so hover popups extend outside the
playground container, a Cmd/Ctrl+S no-op so the browser's "Save
Page As" dialog doesn't fire inside the editor, 8 px top/bottom
padding, and the underscore removed from trigger characters
(was firing mid-identifier). Docs-only release; no checker behavior
changed.

## [0.1.17]

The playground becomes an IDE for pykrete features.
**`pykrete-wasm`** grows three new exports — `hover_at(source, line,
column)` for schemas / columns / function signatures,
`complete_at(source, line, column)` for column-name completions
inside `col("...")`, bare-string args to `groupBy` / `select` /
`agg` / `F.sum`, and schema-name completions inside `DataFrame[...]`
annotations, and `definition_at(source, line, column)` for jump-to-
schema. Each is a thin wrapper around the existing pykrete-side
analyzer functions — no duplicated logic. Same `catch_unwind` +
`console_error_panic_hook` panic-safety pattern as `check_source`.
Monaco wiring in `Playground.tsx` registers
`HoverProvider`/`CompletionItemProvider`/`DefinitionProvider`. Docs-
only; no Rust checker behavior changed.

## [0.1.16]

The launch-readiness release. Closes the last Spark coverage gaps,
ships the WASM playground, and brings the project to a state ready
for public posting. **CLI polish** — `pykrete --version`, `--help`,
and quieter `check` output by default (`-v` restores the full dump);
the first install-verify step no longer fails. **Perf pass** —
`discover_schemas` fixpoint uses a name→idx table instead of a
linear scan; 29% wall-clock reduction on a 100-file / 3000-schema
synthetic, fenced by a perf smoke test. **`D0072
duplicateSchemaName`** — warns when the same schema name is declared
in multiple files within a project. **Column `.dropFields("typo")`**
— fires `D0030` with did-you-mean, symmetric to `.getField`.
**WASM playground** — new `pykrete-wasm` crate (wasm32 build
pipeline, wasm-bindgen API, npm wrapper at
`docs-site/pykrete-wasm-pkg/`), Monaco editor at `/playground` with
live diagnostics (debounced 300 ms), three pre-loaded snippets,
click-to-jump diagnostic list, lazy-loaded so other pages pay
nothing, `catch_unwind` panic-safety + synthetic `D9999
internalError` fallback. **Marketing readiness** — production-
readiness page on the docs site, every stale `v0.1.6` version
reference across docs fixed, quick-fixes claim accurately scoped
against the LSP implementation, new cookbook page with 5 realistic
recipes (adoption, `spark.read` re-anchor, cross-file schemas,
function signature validation, `pykrete.json` configuration).
806 workspace tests (+31 since v0.1.15). See the
[v0.1.16 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.16).

## [0.1.15]

Generic-inference extensions complete — all four roadmap items shipped.
Multi-TypeVar signatures (`def join[A, B](left: DataFrame[A], right:
DataFrame[B]) -> DataFrame[Merge[A, B]]`) now bind each TypeVar from its
own argument slot and substitute through the return type. Nested
parameter shapes (`List[DataSource[T]]`, `Optional[DataSource[T]]`,
`Dict[str, DataSource[T]]`, and arbitrary re-nesting like
`List[List[DataSource[T]]]`) are unwrapped during binding. Chained
class-method calls — `dal.with_path("/x").read(SOURCE)` — preserve class
identity through self-returning intermediaries (`-> "ClassName"`,
`-> ClassName`, `-> Self`) so the trailing generic call still
dispatches. `type[T]`-shaped parameters — `def cast_to[T](self, _:
type[T]) -> DataFrame[T]` called as `dal.cast_to(Orders)` — bind T from
the arg's class identifier rather than its runtime value. Incompatible
bindings (a list whose elements carry different T values, a non-class
arg in a `type[T]` slot) degrade the offending TypeVar to Unknown
rather than fabricate a result. 775 workspace tests (+22 since v0.1.14).
See the [v0.1.15 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.15).

## [0.1.14]

Three Spark coverage gaps closed plus the docs site grew the
user-facing operations reference. `F.when(p, v).otherwise(e)` chains
now infer their result as the common type of the value branches
(atomic equality, then numeric widening — `int` < `long` < `double`);
chains without `.otherwise(...)` resolve to `Nullable(T)`. `F.struct`
and `F.named_struct` produce a `Struct({...})` whose field names come
from `.alias("x")` first then the column name (or the string-literal
slots for `named_struct`), composing with `.getField` so a freshly-
constructed struct can be navigated immediately. Date/time first-arg
column checking landed on ten functions (`to_date`, `to_timestamp`,
`date_format`, `trunc`, `next_day`, `from_utc_timestamp`,
`to_utc_timestamp`, `from_unixtime`, `unix_timestamp`, `date_trunc`)
with the format/timezone string args left alone. Array higher-order
function recognizers — `F.transform`, `F.filter`, `F.aggregate`,
`F.exists`, `F.forall` — check the first-arg column ref and model the
return type per function. `melt` / `unpivot` output-schema inference:
ids preserved with type and nullability, the variable column is
`string`, the value column carries the common type of the unpivoted
source columns with nullable propagation. The docs site grew a
[user-facing operations reference](https://amirnaderi93.github.io/pykrete/reference/operations/)
covering every modeled DataFrame method and `F.*` function. See the
[v0.1.14 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.14).

## [0.1.13]

Two Spark coverage gaps closed plus a sharper release workflow.
**Column method chain recognition** — `.isNull` / `.isNotNull` /
`.isin` / `.between` / `.like` / `.rlike` / `.ilike` / `.contains` /
`.startswith` / `.endswith` are now recognized as boolean-returning
Column predicates that preserve the chain; `.getField` resolves the
nested struct field's type and fires `D0030` with a "did you mean"
on a field-name typo; `.getItem` returns the array element / map
value type; `.withField` and `.dropFields` track the receiver's
struct shape forward with the field added, replaced, or removed.
**`createOrReplaceTempView` + `spark.sql` resolution** —
`df.createOrReplaceTempView("v")` registers `df`'s schema against
the view name in a per-file registry; a subsequent
`spark.sql("SELECT … FROM v")` in the same file checks every column
identifier in the query (projection, `WHERE`, `GROUP BY`, `ORDER BY`,
`HAVING`) against the view's schema. Single-table SELECT only,
within-file only. Marketplace README image URLs absolutized; the
extension-version-guard CI now compares against the last release tag
rather than the PR diff so subsequent feature PRs in a cycle pass
cleanly after the first ext bump. 712 tests (+31 since v0.1.12). See
the [v0.1.13 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.13).

## [0.1.12]

Chain survival pass — closes three headline gaps where real PySpark
codebases lost their schema chain at line one. `spark.read.<format>(path)`
and `spark.table(name)` are now recognized as **opaque sources**; the
result schema is still Unknown (the schema is genuinely runtime data),
but the chain keeps tracking and the user re-anchors with
`.cast(DataFrame[Schema])` or a typed variable annotation
(`raw: DataFrame[Schema] = spark.read.parquet(...)`) to resume
downstream column checks. `intersect` / `intersectAll` / `subtract` /
`exceptAll` joined `union` / `unionByName` as recognized set operations
sharing the same schema-mismatch check (`D0040`); `unionAll` is wired
as a deprecated alias. `F.broadcast(df)` is treated as pass-through, so
chains like `df1.join(F.broadcast(df2), "k")` keep tracking the schema.
Nine terminal methods (`count`, `collect`, `show`, `printSchema`,
`explain`, `first`, `take`, `head`, `tail`) are recognized centrally —
the chain dies cleanly at the right method. Hover popups for schemas
no longer stack the redundant basedpyright "(class) X" echo; pykrete's
content is authoritative. See the [v0.1.12 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.12).

## [0.1.11]

Republishes the extension after v0.1.10's marketplace publish was
silently skipped — the extension's `package.json` version was
unchanged, so `vsce` and `ovsx` left the v0.1.7-era `.vsix` in place
even though the release rebuilt and attached a fresh one. Users who
reinstalled were still served the old bundled binary and missed the
new schema-hover format. With `package.json` now at 0.2.8, this tag
republishes on both marketplaces. Ships alongside a new
extension-version-guard CI workflow that fails any future PR which
changes extension-impacting code without bumping `package.json` —
closes this entire class of bug. See the
[v0.1.11 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.11).

## [0.1.10]

Schema hovers now render as a fenced Python class block with syntax
highlighting — VS Code's markdown renderer applies its Python
highlighter so type names are colored, and the layout reads as the
actual class definition the schema is. Colons align across fields.
Replaces the prior bulleted-list format. See the
[v0.1.10 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.10).

## [0.1.9]

Hotfix for a hover regression introduced when the per-platform
`.vsix` started bundling `pykrete-lsp` + `basedpyright` in v0.1.7.
The multiplexer fans hover requests out to both engines and waits
for the child's reply before merging — when basedpyright didn't
respond (common on `.pyk`-only workspaces where it reports "No
source files found"), the pending request hung forever and the
editor never got a hover popup. Adds a 2-second timeout backstop on
every fanned-out request. When the child stays silent, pykrete's
standalone result is sent to the editor unchanged. Diagnostics,
definition, references, rename, and completion all benefit from the
same backstop. 655 tests (up from 650). See the
[v0.1.9 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.9).

## [0.1.8]

Correctness sweep on `D0051 argumentColumnsMismatch` — six edge
cases tightened. Local-name shadowing of a top-level function now
suppresses the check (plain assignment, tuple-unpack
`revenue, _ = …`, and walrus `(revenue := …)` rebinds). Positional-
only (`/`) and keyword-only (`*`) parameter markers are honored when
matching arguments. `*args` and `**kwargs` variadics with
`DataFrame[Schema]` annotations are checked against every call-site
argument routed to them. A parameter filled both positionally and
by keyword (Python's `TypeError`) is diagnosed once, not twice.
650 workspace tests (+8 in `call_site_args.rs`). See the
[v0.1.8 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.8).

## [0.1.7]

Two features. **`D0051 argumentColumnsMismatch`** closes the
function boundary on the input side — passing a `DataFrame[Wrong]`
into a function declaring `DataFrame[Right]` is now flagged at the
call site, with the same missing / extra column reporting as
`returnColumnsMismatch`. **Per-platform `.vsix` bundling** for the
VS Code extension — the extension and the matching `pykrete-lsp`
binary now install in one step on macOS arm64/x64, Linux x64, and
Windows; a universal `.vsix` backs the long tail of other
platforms. See the [v0.1.7 release](https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.7).

## [0.1.6]

Five fixes batched from the
[pykrete-tests](https://github.com/amirnaderi93/pykrete-tests) loop —
annotating real PySpark codebases (Apache Spark + MLflow) and patching
every gap that surfaced. CI on every push and nightly is wired up there
to keep the runs green.

### Added

- **`df["X"]` subscript** recognized as a column reference alongside
  `col("X")` and `df.X`. Typo in `df["aeg"]` is now caught the same way
  `df.aeg` is. Surfaced by pilot 1 (Spark `basic.py`).
- **GroupedData shortcut aggregates** check their string column args.
  `g.max("col")` / `g.min(...)` / `g.sum(...)` / `g.mean(...)` /
  `g.avg(...)` now route through the same `resolve_path` machinery as
  pivot — dotted nested refs (`g.max("b.c")`) work too. Surfaced by
  pilot 2 (Spark `test_group.py`).
- **`intersect` / `intersectAll` / `subtract` / `exceptAll`** modeled
  as set ops alongside `union` / `unionByName`. Downstream
  `select(col("typo"))` after one of these is now checked; the
  schema-mismatch diagnostic names the actual method. Surfaced by
  pilot 3 (MLflow `test_spark_datasource_autologging.py`).
- **Chained Column-on-Column nested-field access** — `df.r.X` /
  `df["r"].X` / `df.r["X"]` / `df["r"]["X"]` — checked against the
  nested struct schema, not just the top-level column. The diagnostic
  for `df.r.typo` names schema `R`, not the outer schema. Method
  calls (`df["r"].withField(...)`) are correctly distinguished from
  field accesses so the method name isn't flagged. Surfaced by
  pilot 4 (Spark `test_column.py`).
- **Lowercase `df.groupby(...)` alias** recognized identically to
  `df.groupBy(...)`. Doc-tutorial code (e.g. `examples/.../arrow.py`)
  uses the lowercase form exclusively; typos there used to slip past.
  Surfaced by pilot 5 (Spark `arrow.py`).

### Changed

- Workspace and VS Code extension version bumps. No release-channel or
  packaging changes; the publishing pipeline from 0.1.5 ships
  unchanged.

## [0.1.5]

### Changed

- VS Code extension displayName temporarily set to
  "Pykrete — Static schema checking for Python" (library-agnostic — the
  brand stays open for pandas and polars support, planned in
  [the roadmap](docs/roadmap.md)). The change bypasses the Visual Studio
  Marketplace's post-deletion reservation on the name "Pykrete". Will be
  reverted to plain "Pykrete" once the reservation clears.

No checker, LSP, or transpiler behavior changed from 0.1.3.

## [0.1.4]

Cancelled before publishing — superseded by 0.1.5.

## [0.1.3]

### Changed

- VS Code extension republished under the **`amirnaderi`** publisher
  with the redundant `-vscode` suffix dropped from the package name.
  New IDs:
  [`amirnaderi.pykrete`](https://marketplace.visualstudio.com/items?itemName=amirnaderi.pykrete)
  on the Visual Studio Marketplace and Open VSX. The old
  `pykrete.pykrete-vscode` listings are scheduled for removal.

No checker, LSP, or transpiler behavior changed from 0.1.2.

## [0.1.2]

### Added

- **VS Code extension on two registries.** Every release publishes the
  extension to the [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=pykrete.pykrete-vscode)
  (VS Code) and the [Open VSX Registry](https://open-vsx.org/extension/pykrete/pykrete-vscode)
  (Cursor, VSCodium, code-server, Theia). Search **pykrete** in the
  extensions panel.
- **`.vsix` attached to every release** for offline / sideload installs.
- **File icon for `.pyk` files** in the VS Code explorer.
- **Brand assets** — logo at the top of the README, file icons, and a
  proper pykrete icon in Windows' Add/Remove Programs entry for the MSI.

### Changed

- No checker, LSP, or transpiler behavior changed from 0.1.1; this
  release is brand, packaging, and distribution-channel work.

## [0.1.1]

### Added

- **Windows MSI installer** — every release now publishes a `.msi` that
  installs `pykrete` and `pykrete-lsp` and adds them to `PATH`.

### Changed

- Release builds and the Homebrew tap update are fully automated; the
  obsolete pre-multiplexer Pylance stub was removed. No checker, LSP, or
  transpiler behavior changed from 0.1.0.

## [0.1.0]

First usable release — see [docs/v0.1-spec.md](docs/v0.1-spec.md) for the
full contract.

### Added

- **Static schema checker** (`pykrete check`) for PySpark code: column-name
  typos, missing columns after rename/drop, mismatched schemas at union,
  wrong join keys, and shape mismatches at function boundaries.
- **Schemas as Python classes**, including nested `array` / `map` / `struct`
  columns, and TypeScript-style type operators (`Pick`, `Omit`, `Join`,
  `GroupBy`).
- **`DataFrame[Schema]` annotations** checked across whole transformation
  chains, inline SQL, and nested-field access.
- **Transpiler** (`pykrete transpile`) — emits plain Python; runtime cost
  is zero.
- **Language server** (`pykrete-lsp`) — diagnostics, hover, completion,
  document symbols, and go-to-definition over stdio. It multiplexes an
  embedded Python language server (basedpyright) so one server delivers
  both pykrete's schema checks and general Python features.
- **VS Code extension** wrapping `pykrete-lsp`.
- **Multi-file analysis** via imported typed declarations.
- **`pykrete.json`** project configuration with non-strict / strict modes.

[Unreleased]: https://github.com/amirnaderi93/pykrete/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/amirnaderi93/pykrete/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/amirnaderi93/pykrete/compare/v0.1.40...v1.0.0
[0.1.40]: https://github.com/amirnaderi93/pykrete/compare/v0.1.39...v0.1.40
[0.1.39]: https://github.com/amirnaderi93/pykrete/compare/v0.1.37...v0.1.39
[0.1.37]: https://github.com/amirnaderi93/pykrete/compare/v0.1.34...v0.1.37
[0.1.34]: https://github.com/amirnaderi93/pykrete/compare/v0.1.33...v0.1.34
[0.1.33]: https://github.com/amirnaderi93/pykrete/compare/v0.1.32...v0.1.33
[0.1.32]: https://github.com/amirnaderi93/pykrete/compare/v0.1.31...v0.1.32
[0.1.31]: https://github.com/amirnaderi93/pykrete/compare/v0.1.30...v0.1.31
[0.1.30]: https://github.com/amirnaderi93/pykrete/compare/v0.1.29...v0.1.30
[0.1.29]: https://github.com/amirnaderi93/pykrete/compare/v0.1.28...v0.1.29
[0.1.28]: https://github.com/amirnaderi93/pykrete/compare/v0.1.27...v0.1.28
[0.1.27]: https://github.com/amirnaderi93/pykrete/compare/v0.1.26...v0.1.27
[0.1.26]: https://github.com/amirnaderi93/pykrete/compare/v0.1.25...v0.1.26
[0.1.25]: https://github.com/amirnaderi93/pykrete/compare/v0.1.24...v0.1.25
[0.1.24]: https://github.com/amirnaderi93/pykrete/compare/v0.1.23...v0.1.24
[0.1.23]: https://github.com/amirnaderi93/pykrete/compare/v0.1.22...v0.1.23
[0.1.22]: https://github.com/amirnaderi93/pykrete/compare/v0.1.21...v0.1.22
[0.1.21]: https://github.com/amirnaderi93/pykrete/compare/v0.1.20...v0.1.21
[0.1.20]: https://github.com/amirnaderi93/pykrete/compare/v0.1.19...v0.1.20
[0.1.19]: https://github.com/amirnaderi93/pykrete/compare/v0.1.18...v0.1.19
[0.1.18]: https://github.com/amirnaderi93/pykrete/compare/v0.1.17...v0.1.18
[0.1.17]: https://github.com/amirnaderi93/pykrete/compare/v0.1.16...v0.1.17
[0.1.16]: https://github.com/amirnaderi93/pykrete/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/amirnaderi93/pykrete/compare/v0.1.14...v0.1.15
[0.1.14]: https://github.com/amirnaderi93/pykrete/compare/v0.1.13...v0.1.14
[0.1.13]: https://github.com/amirnaderi93/pykrete/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/amirnaderi93/pykrete/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/amirnaderi93/pykrete/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/amirnaderi93/pykrete/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/amirnaderi93/pykrete/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/amirnaderi93/pykrete/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/amirnaderi93/pykrete/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/amirnaderi93/pykrete/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/amirnaderi93/pykrete/compare/v0.1.3...v0.1.5
[0.1.4]: https://github.com/amirnaderi93/pykrete/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/amirnaderi93/pykrete/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/amirnaderi93/pykrete/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/amirnaderi93/pykrete/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.0
