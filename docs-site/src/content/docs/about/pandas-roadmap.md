---
title: Pandas roadmap
description: The pandas-specific direction for pykrete — where v1.3 / v1.4 / v1.5 / v1.6 / v1.7 / v1.8 / v1.9 / v1.10 landed, where v1.11+ is going, what v2.0 locks in.
---

This page tracks the pandas-specific direction for pykrete. The umbrella [roadmap](/about/roadmap/) covers the project as a whole; this page is the pandas-focused complement.

## Where we are (v1.10.0)

**Annotation surface**: `PandasFrame[X]` is a canonical parser-level peer of `SparkFrame[X]`. Same `Pick[…]` / `Omit[…]` / `Merge[…]` derived-schema operators. `DataFrame[X]` is a deprecated alias (warning `D0090`, slated for removal in a future pykrete v2.0). v1.5 shipped `pykrete check --report-aliases` — a JSON envelope listing every `DataFrame[X]` annotation site with its resolved dialect and suggested replacement — so projects can quantify the v2.0 migration scope. v1.6 shipped **`pykrete migrate`**: the auto-rewriter binary that walks each `DataFrame[X]` binding's downstream usage, classifies it as Spark / pandas / ambiguous via call-graph adjudication, and rewrites the annotation to the dialect-tagged canonical name (atomic per file, token-preserving). v1.7 flips the migrator default to `--check` — `pykrete migrate src/` now previews verdicts; `--apply` opts into the rewrite. Paired atomically with D0090 strict-mode escalation (shipped v1.6): under `"typeCheckingMode": "strict"` the warning now lands as **error**, but the fix-button ships in the same release. The `--report-aliases` `resolvedDialect` field reports `"spark"` / `"pandas"` / `"ambiguous"`; `aliasReportVersion` is at `"2"`. v1.8 shipped `pykrete check --deprecation-report` — a JSON envelope inventorying every D0090-firing site with its adjudicated dialect and suggested rewrite. v1.9 bumps the envelope to `deprecationReportVersion: "2"` with per-site `migrationStatus` (`pending` / `acknowledged`) driven by a `# pykrete: ack-deprecation` comment marker on the line above the alias annotation, plus a `--ack=<pending|acknowledged>` filter flag. **v1.10 adds `--snapshot=<path>`** — file-write surface for the v2 envelope (atomic write via tempfile-plus-rename, nanosecond-suffixed temp name to avoid concurrent-writer collision, cleanup-on-error guard across every error path) — and **`--fail-on-nonempty`**, a CI gate flag that exits non-zero when the envelope's `sites` array is non-empty, replacing the `jq '.sites | length' | test ... -eq 0` boilerplate adopters were writing by hand. The envelope deliberately ships without `targetVersion` / `removalVersion` / `shipDate` — pykrete tracks per-site migration progress; the user picks the v2.0 ship date.

**Cross-dialect method-mismatch warning `D0091` (surface-completed in v1.10)**: fires when a pandas-spelled method is called on a `SparkFrame[X]` receiver (`sdf.assign(...)`, `sdf.merge(...)`, `sdf.rename(columns=...)`) or a Spark-spelled method on a `PandasFrame[X]` receiver (`pdf.withColumn(...)`, `pdf.selectExpr(...)`). Carries a *use `.x(...)` instead* suggestion for the high-traffic pairs (`withColumn` ↔ `assign`, `withColumnRenamed` ↔ `rename`, `selectExpr` → `eval`, `toPandas` → `copy`; `groupby` → `groupBy`, `merge` → `join`). **v1.9 maturity**: strict-mode escalation (`"typeCheckingMode": "strict"` → error), mirroring the v1.6 D0090 precedent; suggestion drift guard pins the cross-dialect suggestion table at build time; `shape_changes` hint appends "— note arg shape differs" to asymmetric mappings. A bare-attribute inference arm on `Expr::Attribute` catches `pdf.rdd`, `sdf.loc`, `pdf.iloc`, `sdf.toPandas` (bare, no call). **v1.10 surface completion**: `SPARK_DISCRIMINATOR_PROPERTIES` adds `na`, `write`, `writeStream`, `storageLevel` (now 7 entries; closes v1.9 spark-I1); `PANDAS_INHERITED_PROPERTIES` adds `index`, `values`, `shape`, `T` (now 8 entries; closes v1.9 spark-I2) — both via the bare-attribute path. v1.10 PR-D1's 8 new properties are unit-test-covered at v1.10.0; cross-codebase fixture probes filed for v1.11. Carve-outs: deprecated `DataFrame[X]` alias receivers skip the gate (avoid double-warning with D0090); `pivot` and `melt` on Spark receivers don't fire (Spark exposes legitimate same-spelled `groupBy(...).pivot(...)` and 3.4+ positional `melt` surfaces).

**Pandas `stack(level=, dropna=)` literal-form (new in v1.10)**: `pdf.stack(level="month")` (or `level=["month", "year"]`) resolves the string-literal `level` argument against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. Receiver-dialect-gated: only fires on `PandasFrame[X]` receivers (Spark's `stack` is a column-free-function — `pyspark.sql.functions.stack` — not a DataFrame method). Single-literal `level="m"` and list / tuple-of-literals shapes are checked; int / int-list / `None` / non-literal forms fall through to Unknown.

**Pandas `melt` literal-form (new in v1.7)**: `pdf.melt(id_vars=["a", "b"], value_vars=["c", "d"], var_name="variable", value_name="value")` resolves the string-literal arguments to `id_vars` / `value_vars` against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. List-of-literals shapes (`id_vars=["a", "b"]`) and single-literal `id_vars="a"` are also checked. Variable arguments (`id_vars=cols_var`) and the no-arg form fall through to Unknown. The pandas dispatch is gated on `receiver_is_pandas_inherited` so the existing Spark `melt`/`unpivot` arm's behavior on `SparkFrame[X]` receivers is unchanged. Full `melt` output schema-tracking (the long-format schema with `var_name` / `value_name` as columns) is carried forward to v1.11 paired with `unstack` / `groupby.agg`.

**Pandas `pivot_table` literal-form (shipped v1.6)**: `pdf.pivot_table(index="cat", columns="year", values="amount", aggfunc="sum")` resolves the string-literal arguments to `index` / `columns` / `values` / `aggfunc` against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. List-of-literals shapes (`index=["a", "b"]`) are also checked. Variable arguments (`index=col_var`), callable `aggfunc=...` (`aggfunc=np.mean`), and the no-arg form fall through to Unknown. Full `pivot_table` schema-tracking (the wide output schema — variable column values become column names of the result frame) is carried forward to v1.11 paired with the rest of pandas reshape (`unstack` / `groupby.agg`).

**`.take()` dialect-gate closure (new in v1.6)**: pandas `pdf.take([0, 2])` returns a DataFrame and now passes through (`PandasFrame[X]` → `PandasFrame[X]`) instead of dying as a Spark terminal. Closes the last v1.5 deferred dialect-gate alongside `.head` / `.tail` / `.first`.

**`pdf.loc[mask, "col"]` nested-arg D0030 FP closure (new in v1.6)**: v1.5's PR-C `.loc` literal-form arm fired D0030 against the row-mask argument when both row-mask and column-literal arms were present; v1.6 gates the row-mask arm so it falls through to Unknown (deferred per v1.5 spec) while the column-literal arm still fires D0030 on a typo.

**Cross-dialect handoff (new in v1.5)**:

- `df.toPandas()` on a `SparkFrame[X]` receiver re-tags the chain to `PandasFrame[X]`. Inline subexpression receivers (`df.filter(...).toPandas()`) resolve through the same recursive walk Spark chains already use.
- `spark.createDataFrame(pdf)` re-tags `PandasFrame[Y]` back to `SparkFrame[Y]` when either a `schema=` keyword argument resolves through a typed binding, or the call-arg expression types as `PandasFrame[Y]`. With neither schema source present, the call falls through to Unknown — no auto-inference from raw values.
- Round-trip: `spark.createDataFrame(df.toPandas())` preserves the tag end-to-end.
- Pandas `.head()` / `.tail()` / `.first()` are dialect-gated: pandas receivers pass through (`PandasFrame[X]` → `PandasFrame[X]`), Spark receivers stay terminals. `pdf.head(10).merge(other, on="id")` keeps tracking.

**`.loc[:, "col"]` literal-form (shipped v1.5)**: `pdf.loc[:, "col"]` resolves the string-literal column against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. Variable column keys (`pdf.loc[:, col_var]`), column-range slicing (`pdf.loc[:, "a":"b"]`), and `pdf.iloc[...]` still fall through to Unknown — carried forward to v1.11 paired with broader pandas reshape. (v1.6 closed the `pdf.loc[mask, "col"]` D0030 FP on the row-mask side; row-mask schema-tracking still lands with v1.11+.)

**Check sites modeled** (six dispatched operations plus the assign / melt kwarg paths):

- `df[col_list]` — column selection (mirrors `.select`)
- `df[mask]` — boolean-mask filtering (mirrors `.filter`)
- `df["new"] = expr` and `df.assign(new=expr)` — assignment (mirrors `.withColumn`)
- `df.drop(columns=[…])` — column drop
- `df.merge(other, on=…)` — joins (mirrors `.join`, fires D0060 on missing keys)
- `df.rename(columns={…})` — column rename
- `df.melt(...)` / `df.unpivot(...)` — wide-to-long reshape via the shared dispatcher

**§10 widening**: bare `df["typo"]` outside method-call contexts fires `D0030` on both `SparkFrame[X]` and `PandasFrame[X]`. Comprehension `elt` / `ifs`, unrecognized-call arg positions, registry-tracked call args (v1.4 widening), and Spark subscript-assign LHS are all covered. Walrus receivers (`(pdf := build()).rename(...)`) inherit the assigned-value's dialect (v1.4 fix); `.transform(helper)` threads the receiver's dialect into the helper's body inference (v1.4 fix).

**Type tracking (new in v1.4)**: `PROBE-TYPE-IS` on `PandasFrame[X]` is the production-readiness gate. The synth wraps `{df}.assign(__probe={df}["x"] + 1)` (a dispatched pandas op) so off-claim numeric dtype claims fall through to D0081 `nonNumericArithmetic`. 21 markers across the 7 new v1.4 pandas donors (3 per donor, exactly meeting the v1.4 spec §1 floor of ≥3 per donor / ≥21 total).

**Diagnostic dispatch**: every D-code site (D0020 / D0021 / D0030 / D0051 / D0060 / D0081 / D0082 / D0084 / D0090 + `.cast(…)`) renders the user's actual dialect prefix — no silent relabeling.

**Cross-codebase verification** (see [Real-codebase tests](/about/pykrete-tests/) for the matrix):

- 10 of 17 donors carry annotated `PandasFrame[X]` fixtures with paired `probes_negative/` counterparts.
- **3 hybrid** carry-overs from v1.3 (MLflow, Feast, iceberg-python).
- **3 direct-dispatch** (prophet, seaborn, yfinance) — pykrete checks actual upstream library code where dispatched-shape recognizers match real call sites.
- **4 canonical-fixture-only** (scikit-learn, statsmodels, pandera, Great Expectations) — pykrete checks synthesized user-pattern examples inspired by each library's API, since the library code itself operates above raw pandas dispatch (numpy arrays, metric domains).
- 21 positive `PROBE-TYPE-IS` markers (3 per new donor), mix of string / binary atomic families. Temporal and numeric subtype families are correctly excluded from the dispatch path the synth uses (the synth is gated on arithmetic-supported types so the marker is falsifiable; numeric subtypes are out of scope per v1.4 §10 deferral).

## v1.11+ horizons (committed but unscheduled)

- **Full `pivot_table` / `melt` / `stack` schema-tracking** — the wide / long output schemas (variable column values become column names of the result frame for `pivot_table`; `var_name` / `value_name` become columns of the long frame for `melt`; the index-pivoted long frame for `stack`). v1.6 + v1.7 + v1.10 ship literal-form column checking on the inputs; the output-shape models are paired with the rest of pandas reshape in v1.11+.
- **`.loc` non-literal forms**: `.loc[mask, "col"]` (boolean mask) row-key tracking, `.loc[:, "a":"b"]` (column range), and `pdf.iloc[...]` — carried forward to v1.11 paired with broader pandas reshape.
- **`df.query("…")` and `df.eval("…")` mini-DSLs**: parse string-fragment column refs separately. numexpr-influenced syntax, not SQL — separate parser from the path used by `selectExpr`. High signal for production pandas code.
- **Broader pandas method modeling**: `df.groupby(...).agg(...)`, `df.unstack`, `df.reset_index`, `df.set_index` (v1.10 shipped `stack`). Currently fall through to opaque.
- **`--include-py` flag for `pykrete migrate`**: let the migrator walk the multiplexer cohort's `.py` files alongside `.pyk`.
- **`--changed-only` flag** for both `pykrete migrate` and `pykrete check`: walk only files changed against HEAD. Pairs naturally with CI invocations.
- **`--compare-to <snapshot>` for `--deprecation-report`**: consumer-side state model paired with the v1.10 `--snapshot` file-write surface; deferred per v1.9 author-boundary carve-out at `alias_report.rs:446-448`.
- **Pandas multi-index support**: `df.set_index(["a","b"])` produces a structurally-different shape pykrete doesn't model yet.
- **`pd.read_csv(...)` and other I/O entry points** (`pd.read_parquet`, `pd.read_json`, `pd.read_sql`, …): schema inference from file headers / SQL / type-stubs is a separate design surface.
- **Pandas dtype subtypes**: `float32` vs `float64`, `int8/16/32/64`, `Int64` (nullable) vs `int64`. Carve-out from v1.0 spec; revisit if user demand surfaces.
- **Ordered `CategoricalDtype(ordered=True)`, tz-aware `datetime64[ns, tz]`, `timedelta64[ns]` / `IntervalDtype`**: re-deferred from v1.3 §4.
- **Retrofitting pandas `PROBE-TYPE-IS` to the v1.3 hybrid donors** (MLflow, Feast, iceberg-python) — v1.4 / v1.5 / v1.6 / v1.7 / v1.8 / v1.9 / v1.10 deliberately scoped these out; revisit in v1.11+.
- **Cross-codebase fixture probes for v1.10 PR-D1's 8 new D0091 properties** (`na`, `write`, `writeStream`, `storageLevel`, `index`, `values`, `shape`, `T`) — unit-test-covered at v1.10.0; cross-codebase probes filed for the v1.11 batch.

## v2.0 commitments (locked)

- **`DataFrame[X]` alias removed.** v1.3 announced the deprecation; v2.0 is the removal point. Every v1.x release continues to accept the alias with `D0090` warning so users have the entire v1 line to migrate. v1.8 amends the warning text to drop the date commitment ("slated for removal in a future pykrete v2.0") and surfaces `pykrete check --deprecation-report` as the JSON-envelope inventory surface for CI gates. v1.9 layers per-site `migrationStatus` and the `--ack` filter on top so adopters can land "this site is migrated, that one's on-deck, the rest aren't yet" as CI signal.
- **`schemaVersion` JSON output stays at "1"** through the v1 line; v2.0 may bump if the diagnostic shape itself changes.

## What we deliberately don't ship (and why)

- **Pandas runtime validation**: pykrete is a static checker. Validating values at runtime is `pandera`'s job — that's why pandera is on the v1.4 donor list (sibling tool, not competitor).
- **polars support**: separate dialect with separate idioms. Tracked for v1.11+ if user demand surfaces; not gated on pandas work, but realistically follows pandas reshape.
- **Pandas-on-Spark API (`pyspark.pandas`)**: parallel surface to pandas with subtle semantic differences. Modeled only if a real PySpark user requests it.
- **Inferred-schema mode**: pykrete asks users to declare schemas; auto-inference contradicts the "declare your contract" value prop.

## Trust claim trajectory

| Version | Pandas claim | Verifiable? |
|---|---|---|
| v1.3.0 | "check-site coverage for the six pandas dispatched operations" | yes — 19 probes across 3 donors |
| v1.4.0 | "check-site coverage + type-tracking across the dominant pandas stack — 10 donors, 21 new pandas `PROBE-TYPE-IS` markers (3 per new donor), three checker bug closures" | yes — 223 probes total across 17 donors |
| v1.5.0 | "cross-dialect handoff (Spark↔pandas), `.loc[:, "col"]` literal-form, dialect-gated `.head`/`.tail`/`.first`, `--report-aliases` JSON envelope" | yes — 235 probes total across 17 donors |
| v1.6.0 | "`pykrete migrate` auto-rewriter + D0090 strict-mode escalation (paired); pandas `pivot_table` literal-form column checking; `.take()` dialect-gate closure; `pdf.loc[mask, "col"]` D0030 FP fix" | yes — 241 probes total across 17 donors |
| v1.7.0 | "migrator `--check` default + `--apply` opt-in; pandas `df.melt(id_vars=, value_vars=)` literal-form; `dialect_signals` shared module + CI-guard; Spark-D1 audit closure (14 new `SPARK_DISCRIMINATORS`); parse-error surface + CRLF marker normalization on migrate" | yes — 247 probes total across 17 donors |
| v1.8.0 | "`pykrete check --deprecation-report` JSON envelope + D0090 message amend; D0091 `crossDialectMethodMismatch` warning (warning-only this cycle); `build.rs`-generated `PANDAS_INHERITED_ARM_METHODS` inventory; `scripts/changelog-grep.sh` CI gate; D0073 / D0083 cross-codebase probe coverage" | yes — 253 probes total across 17 donors |
| v1.9.0 | "`--deprecation-report` v2 envelope (per-site `migrationStatus` + `--ack` filter); D0091 strict-mode escalation + suggestion drift guard + `shape_changes` hint; D0091 bare-attribute inference arm on `Expr::Attribute`; `PANDAS_INHERITED_ARM_METHODS` tripwire backed by CI-running tests via `build_helpers.rs`; CHANGELOG `text-numeric` gate" | yes — 255 probes total across 17 donors |
| v1.10.0 | "`--deprecation-report --snapshot=<path>` file-write + `--fail-on-nonempty` CI gate; D0091 surface completion (8 new properties: `na`/`write`/`writeStream`/`storageLevel` Spark-side, `index`/`values`/`shape`/`T` pandas-side); pandas `df.stack(level=, dropna=)` literal-form; v1.9 audit-debt closure (ack-marker multi-line + property/method tripwire + release-gate workflow + CHANGELOG grep gate v3 prose scan); §9.2 centralized version bump promoted to standing" | yes — `261 probes` total across `17 donors` |
| v1.11.0 (target) | + "rest of pandas reshape (`unstack` / `groupby.agg` + full `pivot_table` / `melt` / `stack` output schema-tracking); `.loc` non-literal forms + `.iloc`; `.query` / `.eval` mini-DSLs; `--include-py` / `--changed-only` / `--compare-to` migrate flags; cross-codebase fixture probes for the 8 v1.10 D0091 properties" | TBD per spec |
| v2.0.0 (target) | canonical `SparkFrame[X]` / `PandasFrame[X]` only; deprecated alias removed | tag-time grep against repo |
