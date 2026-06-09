---
title: Pandas roadmap
description: The pandas-specific direction for pykrete — where v1.3 / v1.4 / v1.5 landed, where v1.6+ is going, what v2.0 locks in.
---

This page tracks the pandas-specific direction for pykrete. The umbrella [roadmap](/about/roadmap/) covers the project as a whole; this page is the pandas-focused complement.

## Where we are (v1.5.0)

**Annotation surface**: `PandasFrame[X]` is a canonical parser-level peer of `SparkFrame[X]`. Same `Pick[…]` / `Omit[…]` / `Merge[…]` derived-schema operators. `DataFrame[X]` is a deprecated alias (warning `D0090`, removed in v2.0). v1.5 ships `pykrete check --report-aliases` — a JSON envelope listing every `DataFrame[X]` annotation site with its resolved dialect and suggested replacement — so projects can quantify the v2.0 migration scope before v1.6's `pykrete migrate` ships. v1.5 reports every site as `spark` / `SparkFrame[X]`; v1.6 introduces call-graph dialect adjudication that distinguishes `spark` / `pandas` / ambiguous. The envelope carries its own `aliasReportVersion: "1"` so the report format can evolve independently of the diagnostic JSON contract.

**Cross-dialect handoff (new in v1.5)**:

- `df.toPandas()` on a `SparkFrame[X]` receiver re-tags the chain to `PandasFrame[X]`. Inline subexpression receivers (`df.filter(...).toPandas()`) resolve through the same recursive walk Spark chains already use.
- `spark.createDataFrame(pdf)` re-tags `PandasFrame[Y]` back to `SparkFrame[Y]` when either a `schema=` keyword argument resolves through a typed binding, or the call-arg expression types as `PandasFrame[Y]`. With neither schema source present, the call falls through to Unknown — no auto-inference from raw values.
- Round-trip: `spark.createDataFrame(df.toPandas())` preserves the tag end-to-end.
- Pandas `.head()` / `.tail()` / `.first()` are dialect-gated: pandas receivers pass through (`PandasFrame[X]` → `PandasFrame[X]`), Spark receivers stay terminals. `pdf.head(10).merge(other, on="id")` keeps tracking.

**`.loc[:, "col"]` literal-form (new in v1.5)**: `pdf.loc[:, "col"]` resolves the string-literal column against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. Variable column keys (`pdf.loc[:, col_var]`), boolean-mask row keys (`pdf.loc[mask, "col"]`), column-range slicing (`pdf.loc[:, "a":"b"]`), and `pdf.iloc[...]` fall through to Unknown — deferred to v1.6 paired with broader pandas reshape.

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

## v1.6+ horizons (committed but unscheduled)

- **`pykrete migrate` binary + D0090 strict-mode escalation** (paired, non-negotiable v1.6 commitment). v1.5 ships `--report-aliases` as the visibility slice; v1.6 ships the auto-rewriter framework and lights D0090 as error under strict mode in the same release. Per "trust over hype, delay over bad launch", strict-mode users must not see a silent escalation without a fix-button in the same release.
- **`.loc` non-literal forms**: `.loc[mask, "col"]` (boolean mask), `.loc[:, "a":"b"]` (column range), and `pdf.iloc[...]` — all deferred to v1.6, paired with broader pandas reshape.
- **`df.query("…")` and `df.eval("…")` mini-DSLs**: parse string-fragment column refs separately. numexpr-influenced syntax, not SQL — separate parser from the path used by `selectExpr`. High signal for production pandas code.
- **Broader pandas method modeling**: `df.pivot_table`, `df.groupby(...).agg(...)`, `df.stack` / `df.unstack`, `df.reset_index`, `df.set_index`. Currently fall through to opaque.
- **Pandas multi-index support**: `df.set_index(["a","b"])` produces a structurally-different shape pykrete doesn't model yet.
- **`pd.read_csv(...)` and other I/O entry points** (`pd.read_parquet`, `pd.read_json`, `pd.read_sql`, …): schema inference from file headers / SQL / type-stubs is a separate design surface.
- **Pandas dtype subtypes**: `float32` vs `float64`, `int8/16/32/64`, `Int64` (nullable) vs `int64`. Carve-out from v1.0 spec; revisit if user demand surfaces.
- **Ordered `CategoricalDtype(ordered=True)`, tz-aware `datetime64[ns, tz]`, `timedelta64[ns]` / `IntervalDtype`**: re-deferred from v1.3 §4.
- **Retrofitting pandas `PROBE-TYPE-IS` to the v1.3 hybrid donors** (MLflow, Feast, iceberg-python) — v1.4 / v1.5 deliberately scoped these out; revisit in v1.6.

## v2.0 commitments (locked)

- **`DataFrame[X]` alias removed.** v1.3 announced the deprecation; v2.0 is the removal point. Every v1.x release continues to accept the alias with `D0090` warning so users have the entire v1 line to migrate.
- **`schemaVersion` JSON output stays at "1"** through the v1 line; v2.0 may bump if the diagnostic shape itself changes.

## What we deliberately don't ship (and why)

- **Pandas runtime validation**: pykrete is a static checker. Validating values at runtime is `pandera`'s job — that's why pandera is on the v1.4 donor list (sibling tool, not competitor).
- **polars support**: separate dialect with separate idioms. Tracked for v1.6+ if user demand surfaces; not gated on pandas work, but realistically follows pandas reshape.
- **Pandas-on-Spark API (`pyspark.pandas`)**: parallel surface to pandas with subtle semantic differences. Modeled only if a real PySpark user requests it.
- **Inferred-schema mode**: pykrete asks users to declare schemas; auto-inference contradicts the "declare your contract" value prop.

## Trust claim trajectory

| Version | Pandas claim | Verifiable? |
|---|---|---|
| v1.3.0 | "check-site coverage for the six pandas dispatched operations" | yes — 19 probes across 3 donors |
| v1.4.0 | "check-site coverage + type-tracking across the dominant pandas stack — 10 donors, 21 new pandas `PROBE-TYPE-IS` markers (3 per new donor), three checker bug closures" | yes — 223 probes total across 17 donors |
| v1.5.0 | "cross-dialect handoff (Spark↔pandas), `.loc[:, "col"]` literal-form, dialect-gated `.head`/`.tail`/`.first`, `--report-aliases` JSON envelope" | yes — 235 probes total across 17 donors |
| v1.6.0 (target) | + "`pykrete migrate` auto-rewriter + D0090 strict-mode escalation (paired); plus pandas reshape or `.query` / `.eval` string-fragment DSLs (committee scoping)" | TBD per spec |
| v2.0.0 (target) | canonical `SparkFrame[X]` / `PandasFrame[X]` only; deprecated alias removed | tag-time grep against repo |
