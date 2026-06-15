---
title: Roadmap
description: What's planned, in rough priority order. Tracking the project as it moves.
---

The canonical source is [docs/roadmap.md](https://github.com/amirnaderi93/pykrete/blob/main/docs/roadmap.md) in the repo. This page summarizes it for site visitors.

## Where pykrete is now

The PySpark static checker is **feature-complete** as of the v1.0 release line, with pandas dialect support added in v1.3 (see the [GitHub Releases page](https://github.com/amirnaderi93/pykrete/releases) for the per-release history):

- The full DataFrame operation surface (`select` / `filter` / `join` / `groupBy`+`agg` / `withColumn(s)` / `drop` / `union` / `cube` / `rollup` / `pivot` / `transform` / `cast` / `toDF` / `df.na.*` / set ops / `melt` / `unpivot` / …) with result-schema inference through whole transformation chains.
- Inline SQL (`F.expr`, `selectExpr`, string-`filter`) and raw `spark.sql("SELECT …")` — including `createOrReplaceTempView` + `spark.sql("SELECT … FROM v")` resolution within a file.
- `Window` partition/order key checking.
- Column-**existence** checking (`D0030`) and column-**type** checking — conservative (`D0080`, on by default) and strict (`D0081` / `D0082`, under `typeCheckingMode: strict`).
- Arbitrarily-nested `array` / `map` / `struct` columns — declared, structurally type-checked, and navigated field-by-field (`col("orders.line.sku")`).
- Column method chains — `.isNull` / `.isin` / `.between` / `.like` / `.getField` / `.getItem` / `.withField` / `.dropFields` — recognized and tracked through.
- `F.when` / `F.otherwise` result-type inference and `F.struct` / `F.named_struct` schema construction. Date/time first-arg column checking on ten `F.*` functions. Array higher-order recognizers (`F.transform`, `F.filter`, `F.aggregate`, `F.exists`, `F.forall`).
- `spark.read.<format>(path)` and `spark.table(name)` recognized as opaque sources — re-anchor with `.cast(SparkFrame[Schema])` or a typed variable annotation to resume checking. (`DataFrame[Schema]` is the deprecated alias and also works through v1.x.)
- A `pyspark.sql.functions` result catalog (≈80 functions) and UDF return types.
- Call-site argument checking (`D0051 argumentColumnsMismatch`) closes the function boundary on the input side.
- Generic-inference: multi-TypeVar binding, nested generic shapes, chained class-method calls, and `type[T]` argument binding all dispatch correctly.
- Cross-file imports and shared-schema modules.
- Project-wide duplicate-schema-name detection (`D0072 duplicateSchemaName`, warning), and a performance-pass micro-optimization of the schema-name resolution hot path (with a release-build perf smoke test in CI).

The **LSP server** delivers live diagnostics, hover, completion (column names in bare-string arguments and on chain results), document symbols, go-to-definition, find-references, rename, semantic tokens, and `textDocument/codeAction` quick-fixes for `D0030` "did you mean" suggestions. It embeds a Python language server via an LSP multiplexer.

The **VS Code extension** wraps it; Neovim, Helix, and Emacs setups are documented; a Zed extension is planned.

The **`.pyk` → `.py` transpiler** is complete.

The **in-browser [playground](/pykrete/playground/)** runs pykrete via WebAssembly and now serves the same pykrete features the VS Code extension does for `.pyk` files: live diagnostics, hover on schema and `SparkFrame[X]` references, column-name completion inside `col("…")`, schema completion in `SparkFrame[…]` slots, and go-to-definition on schema references. (`DataFrame[X]` is the deprecated alias and renders the same hover.) The embedded Python engine isn't reachable from the browser yet (queued for a follow-up release).

For the full list of every shipped feature with diagnostics, see the [Operations reference](/pykrete/reference/operations/) and the [GitHub Releases page](https://github.com/amirnaderi93/pykrete/releases).

## Shipped in v1.3 — pandas check-site coverage

`PandasFrame[Schema]` joins `SparkFrame[Schema]` as a **canonical** dataframe-annotation form. `DataFrame[Schema]` is a **deprecated alias** for `SparkFrame[Schema]` — every use fires `D0090 deprecatedDataFrameAlias` (warning) with a quick-fix to the canonical name. The alias stays valid through the v1 line and is **removed in v2.0** so the migration is unhurried.

Six pandas operations dispatch through dialect-specific check sites: column selection (`df[col_list]`), boolean-mask filtering (`df[mask]`), assignment (`df["new"] = expr`), `df.drop`, `df.merge`, and `df.rename`. The bare-subscript widening rule also fires `D0030` on bare `df["typo"]` subscripts in non-method contexts on both `SparkFrame[X]` and `PandasFrame[X]`. Cross-codebase pandas fixtures land for mlflow, feast, and iceberg-python.

## Shipped in v1.4 — depth on pandas

The v1.3 → v1.4 cadence parallels v1.1 → v1.2 on the Spark side: check-site coverage first, type-tracking and donor breadth next.

- **7 new pandas-heavy donors** in pykrete-tests — scikit-learn, statsmodels, pandera, Great Expectations, prophet, seaborn, yfinance — bringing pandas-coverage donor count from 3 to 10. Honest scoping breakdown (see [Real-codebase tests](/about/pykrete-tests/) for the per-donor detail): 3 direct-dispatch (prophet, seaborn, yfinance) against actual upstream library code where pykrete's dispatched-shape recognizers match; 4 canonical-fixture-only (sklearn, statsmodels, pandera, GE) modeling user-pattern fixtures where the upstream code operates above raw pandas dispatch.
- **Pandas type-tracking via `PROBE-TYPE-IS`** (closes [pykrete-tests#14](https://github.com/amirnaderi93/pykrete-tests/issues/14)). The synth wraps `{df}.assign(__probe={df}["x"] + 1)` so off-claim numeric dtype claims on `PandasFrame[X]` parameters fall through to D0081. 21 markers across the 7 new donors (3 per donor, exactly meeting the v1.4 spec §1 floor).
- **Three checker bug closures** (PRE-EXISTING silent-pass paths surfaced by v1.3 audits): registry-call args walk unconditionally so `util(df["typo"])` fires D0030; `inherited_dialect` walks walrus receivers so `(pdf := build()).rename(...)` inherits the assigned-value's dialect; `.transform(helper)` threads the receiver's dialect into the helper's body inference. SemVer-minor under the `tighteningDiagnostics` policy.
- **Config-discovery walk fix**: `pykrete.json` discovery anchors on the input file's parent directory (falling back to CWD when no input resolves to a file), so `pykrete check /abs/path/to/foo.pyk` from any CWD picks up the project's config.
- **Canonical-name migration completion** across docs / design notes / examples for `SparkFrame[X]` vs the deprecated `DataFrame[X]` alias.

For the verification posture and per-donor matrix, see [Real-codebase tests](/about/pykrete-tests/) and [Production readiness → Real-codebase testing](/about/production-readiness/#real-codebase-testing). For the full pandas direction across v1.7+ and v2.0, see [Pandas roadmap](/about/pandas-roadmap/).

## Shipped in v1.5 — cross-dialect handoff + deferred-promise closure

The v1.5 cycle's headline is **cross-dialect handoff**: pykrete now tracks schema across the Spark↔pandas boundary at the same depth it tracks within a single dialect.

- **`.toPandas()` re-tags `SparkFrame[X]` to `PandasFrame[X]`** (PR-A1), so a downstream `pdf["typo"]` chain fires D0030 against `X`. Inline subexpression receivers (`df.filter(...).toPandas()`) resolve through the same recursive `infer_expr_type` walk Spark chains already use.
- **`spark.createDataFrame(pdf)` re-tags `PandasFrame[Y]` back to `SparkFrame[Y]`** (PR-A2) when either a `schema=` keyword argument resolves through a typed binding, or the call-arg expression types as `PandasFrame[Y]`. With neither present, the call falls through to Unknown — no auto-inference from raw values. The round-trip `spark.createDataFrame(df.toPandas())` preserves the tag end-to-end.
- **`.head()` / `.tail()` / `.first()` dialect-gated** (PR-A3): pandas receivers pass through (`PandasFrame[X]` → `PandasFrame[X]`), Spark receivers stay terminals. `pdf.head(10).merge(other, on="id")` keeps tracking.
- **`.loc[:, "col"]` literal-form lands** (PR-C). Variable column keys, boolean-mask row keys, column-range slicing, and `.iloc[...]` fall through to Unknown — deferred to v1.7 paired with broader pandas reshape (v1.6 closed the `pdf.loc[mask, "col"]` D0030 FP on the row-mask side).
- **Two PR-F1-class sibling gates close** (PR-B1 + PR-B2). `column_name_arg`'s attribute + subscript arms now gate on the receiver being a DataFrame binding; `collect_col_refs` threads the receiver name through to schema-lookup callers so cross-DataFrame subscript routing lands on the correct schema.
- **`pykrete check --report-aliases`** (PR-D): new invocation-only flag emits a structured JSON envelope listing every `DataFrame[X]` annotation site with its resolved dialect and suggested replacement. v1.5 reported every site as `spark` / `SparkFrame[X]`; v1.6 lit the call-graph dialect adjudicator into the same envelope so the field now distinguishes `spark` / `pandas` / ambiguous. Does not rewrite source — projects pipe the report to their own tooling to size the v2.0 migration scope (v1.6 ships `pykrete migrate` for the rewrite step). The envelope carries its own `aliasReportVersion` (v1.5 shipped `"1"`; v1.6 bumps to `"2"` to signal the `resolvedDialect` value-set expansion) so the report format evolves independently of the diagnostic JSON contract.
- **Synthetic-pool soft cap with warn-and-saturate sentinel** (PR-E): closes the v1.4 architecture-audit I4 finding. The LSP keeps running on adversarial input instead of unbounded `Box::leak` growth.

D0090 stayed at `warning` everywhere in v1.5; the severity escalation under strict mode landed in v1.6 paired atomically with `pykrete migrate` so strict-mode users get a fix-button at the same release as the breaking-change signal.

For the verification posture and per-donor matrix, see [Real-codebase tests](/about/pykrete-tests/) and [Production readiness → Real-codebase testing](/about/production-readiness/#real-codebase-testing). For the full pandas direction across v1.7+ and v2.0, see [Pandas roadmap](/about/pandas-roadmap/).

## Shipped in v1.6 — `pykrete migrate` + D0090 strict-mode escalation + pandas `pivot_table`

The v1.6 cycle's headline is **`pykrete migrate`**: the auto-rewriter binary for `DataFrame[X]` → `SparkFrame[X]` / `PandasFrame[X]`, paired atomically with `D0090 deprecatedDataFrameAlias` strict-mode escalation so strict-mode users on green v1.4.x/v1.5.x CI aren't stranded.

- **`pykrete migrate` CLI** (PR-M1 + PR-M2 + PR-M3). Three modes: `pykrete migrate src/` rewrites in place; `pykrete migrate --check src/` previews per-site verdicts to **stdout** (pipe-friendly) and exits 1 if any site needs attention or is ambiguous, 0 otherwise — drop it into CI to gate merges; `pykrete migrate --diff src/` emits a `patch -p1`-compatible unified diff. The walker traverses `.pyk` files under each input path, locates every `DataFrame[X]` site via the `AliasSite` byte-range model, and rewrites token-preservingly. The in-place rewrite is **atomic per file** — pykrete writes to a sibling temp file and renames, so an interrupted run never leaves a half-rewritten source. See [cookbook recipe 6](/cookbook/#6-migrate-dataframex-to-the-v20-dialect-tagged-names).
- **Call-graph dialect adjudication** (PR-M3). Each `DataFrame[X]` binding's downstream usage is inspected for dialect-discriminating method signals: Spark-only (`withColumn` / `withColumns` / `createOrReplaceTempView` / `repartition` / SparkSession constructors and the rest of the Spark surface) versus pandas-only (`assign` / `pivot_table` / `.loc` / `.iloc` / pandas `merge` / `rename(columns=...)` / and the pandas dispatched surface). Only Spark signals → **Spark**; only pandas → **pandas**; both → **Ambiguous** (rewrite skipped, `# pykrete: ambiguous` marker injected on the line above, idempotent on re-runs); no signal → defaults to Spark.
- **D0090 strict-mode escalation** (PR-M3). Under `"typeCheckingMode": "strict"` in `pykrete.json`, D0090 lands as **error** instead of warning. Non-strict modes (`off` / `basic` / `standard`) keep the warning unchanged. The escalation ships in the same release as `pykrete migrate`, per "trust over hype, delay over bad launch".
- **`pykrete check --report-aliases` `resolvedDialect`** field now reports `"pandas"` and `"ambiguous"` discriminators in addition to `"spark"`. v1.5 reported every site as `"spark"` because adjudication wasn't yet wired; v1.6 lights the call-graph adjudicator into the same envelope. `aliasReportVersion` bumps from `"1"` to `"2"` to signal the value-set expansion to consumers.
- **Pandas `pivot_table(index=, columns=, values=, aggfunc=)` literal-form** (PR-D1): the v1.6 pandas reshape downpayment. String-literal arguments and list-of-literals shapes resolve against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. Variable arguments (`index=col_var`), callable `aggfunc` (`aggfunc=np.mean`), and the no-arg form fall through to Unknown. Full `pivot_table` schema-tracking (the wide output schema) is deferred to v1.7 paired with broader pandas reshape.
- **`.take()` dialect-gate** (PR-A2): the last v1.5 deferred dialect-gate closes. pandas `pdf.take([0, 2])` returns a DataFrame and now passes through (`PandasFrame[X]` → `PandasFrame[X]`) instead of dying as a Spark terminal.
- **`pdf.loc[mask, "col"]` nested-arg D0030 FP fix** (PR-A2): v1.5's PR-C `.loc` literal-form arm fired D0030 against the row-mask argument when both row-mask and column-literal arms were present; v1.6 gates the row-mask arm so it falls through to Unknown (deferred per v1.5 spec) while the column-literal arm still fires D0030 on a typo.
- **`cross_dialect_handoff_gate` recognizer extracted** (PR-A1): the v1.5 architecture-audit "Keep in sync" comment between the `.toPandas()` and `spark.createDataFrame(pdf)` arms gets replaced by a single shared recognizer. No behavior change — audit-debt closure.

For the verification posture and per-donor matrix, see [Real-codebase tests](/about/pykrete-tests/) and [Production readiness → Real-codebase testing](/about/production-readiness/#real-codebase-testing). For the full pandas direction across v1.7+ and v2.0, see [Pandas roadmap](/about/pandas-roadmap/).

## Shipped in v1.7 — migrator `--check` default + pandas `melt` + `dialect_signals` + Spark-D1 audit closure

The v1.7 cycle hardens the v1.6 migrator surface, ships the pandas reshape downpayment for `melt`, and closes the v1.6 architecture-audit Important #3 finding.

- **`pykrete migrate` default mode flips to `--check`** (PR-M1). `pykrete migrate src/` now runs check-mode (preview verdicts on stdout, exit 1 if any site needs attention, 0 otherwise). `--apply` is the new opt-in for the in-place rewrite. The flip lands two cycles after the binary first shipped; the v1.6 release notes explicitly flagged the CLI surface as pre-stable. A first-run on v1.7 with no flag emits a one-line stderr warning so adopters discover the change without reading release notes. **Adopter callout**: any CI invocation that ran `pykrete migrate src/` expecting in-place rewrite needs `pykrete migrate --apply src/`.
- **Pandas `df.melt(id_vars=, value_vars=, var_name=, value_name=)` literal-form** (PR-D1; spec §4). String-literal arguments and list-of-literals shapes resolve against `PandasFrame[X]`'s schema, firing D0030 on a typo with a *did you mean*. Variable arguments (`id_vars=cols_var`) and the no-arg form fall through to Unknown. The pandas dispatch is gated on `receiver_is_pandas_inherited`, so the existing Spark `melt`/`unpivot` arm's behavior on `SparkFrame[X]` receivers is unchanged. Full `melt` output schema-tracking (the long-format schema with `var_name` / `value_name` as columns) is deferred to v1.8.
- **`dialect_signals` shared module** (PR-A1; closes v1.6 architecture-audit Important #3). The v1.6 cycle left `PANDAS_ONLY_SIGNALS` (binding-classification) and `PANDAS_INHERITED_ARMS` (expr-side dispatched arms) as parallel lists in two files with a "Keep in sync" comment between them. v1.7 extracts both into a single `crates/pykrete/src/dialect_signals.rs` module. PR-A2 added `SPARK_DISCRIMINATORS` to the same module — the Spark-side companion populated with 14 new Spark-only methods (`selectExpr`, `freqItems`, `approxQuantile`, `crosstab`, `colRegex`, `summary`, `mapInPandas`, `mapInArrow`, `writeTo`, `writeStream`, `unpivot`, `rdd`, `isStreaming`, `sparkSession`). `corr` / `cov` were considered and deliberately excluded for pandas collision risk (caught at A2 review).
- **CI-guard test pinning `expr.rs` pandas-arm methods to `PANDAS_INHERITED_ARMS`** (PR-A1). Asserts the methods dispatched by the `receiver_is_pandas_inherited` arm in `operations/expr.rs` are exactly the methods in `PANDAS_INHERITED_ARMS`. Honest scoping: catches "added to one list, forgot the other" (parallel-edit drift), does NOT catch "added a wholly new dispatched arm and updated neither list" (omitted-edit drift). Failure mode #2 is a v1.8 candidate.
- **`pykrete migrate` parse-error surface** (PR-M1). Files that fail to parse are skipped (existing behavior); v1.7 reports each skipped file on stderr with the parse error so adopters can see why a file didn't get migrated.
- **CRLF marker normalization in `# pykrete: ambiguous` insertions** (PR-M1). On Windows-style CRLF source files, the v1.6 marker inserter mixed LF (the marker itself) with surrounding CRLF runs. v1.7 detects the line-ending convention and emits the marker with the matching ending.
- **Audit-debt mop-up** (PR-A2). `_source: &str` dead parameter dropped from `ambiguous_site_offsets` / `has_ambiguous_in_file`; two-vector lockstep loop in the migrate driver's parse-error filter collapsed to single-pass.

For the verification posture and per-donor matrix, see [Real-codebase tests](/about/pykrete-tests/) and [Production readiness → Real-codebase testing](/about/production-readiness/#real-codebase-testing). For the full pandas direction across v1.8+ and v2.0, see [Pandas roadmap](/about/pandas-roadmap/).

## Next up

### v1.8 — broader pandas reshape + LSP polish + `--include-py` migrate flag + spark-D2 D-code

Now that `melt` literal-form, the migrator `--check` default, and the audit-debt closure are out the door, v1.8's focus is on the rest of the pandas reshape surface, an LSP polish block, and the spark-D2 cross-dialect mismatch D-code.

- **Pandas reshape**: `stack` / `unstack`, `groupby.agg`, `reset_index`, `set_index`, plus full `pivot_table` and `melt` output schema-tracking (the wide / long output schemas — variable column values become column names of the result frame; `var_name` / `value_name` become columns of the long frame).
- **`.loc` non-literal forms and `.iloc`**: `.loc[mask, "col"]` (boolean mask), `.loc[:, "a":"b"]` (column range), and `pdf.iloc[...]`.
- **`df.query("…")` / `df.eval("…")` mini-DSLs**: parse string-fragment column refs separately. numexpr-influenced syntax, not SQL.
- **`pd.read_csv(...)` and other pandas I/O entry points**: schema inference from file headers / SQL / type-stubs as a separate design surface.
- **`--include-py` flag for `pykrete migrate`**: let the migrator walk `.py` files in the multiplexer cohort alongside `.pyk`.
- **New D-code for cross-dialect method mismatch (spark-D2)**: fire a diagnostic when a pandas-only method is called on `SparkFrame[X]` (or vice versa) instead of the silent fall-through.
- **D0073 / D0083 cross-codebase probes**: extend the v1.7 D0040 / D0050 / D0051 negative-probe sweep to the remaining un-probed D-codes.
- **LSP polish block**: visitor name-shadowing M3 round-2, hover_timeout flake, col-ref helper consolidation, suggester threshold.
- **CI-guard for the omitted-edit drift class**: extend the v1.7 CI-guard to catch new arms that get added without updating either list.
- **Retrofitting pandas `PROBE-TYPE-IS` to the v1.3 hybrid donors** (MLflow, Feast, iceberg-python).
- **Canonical-vs-direct CI gate (I3)** from the v1.4 architecture audit.

### Window-key type tracking

Currently `Window.partitionBy("col")` keys aren't checked against any DataFrame schema. Adding local-binding tracking for Window objects, then resolving keys at the `.over(w)` site, would close the gap.

### Column-expression type tracking

Tracks the type of a Column through chains like `df["a"].cast("int").alias("x")`. Needed for `df[N]` integer-subscript bounds checks, fuller `Column.withField` / `Column.dropFields` against nested struct columns, and warning when a Column's atomic type drives a function that expects a different type.

## Larger structural moves

### Multi-dataframe support

After pandas depth, polars next. The dispatch model lets new libraries plug in without churning the schema model.

### Forking `ty`

Long term, pykrete may fork Astral's [`ty`](https://github.com/astral-sh/ty) (their Rust Python type checker) once it reaches a stable release — a single native stack, replacing the basedpyright multiplexer. Since pykrete's analyzer is already built on `ruff_python_ast` (the AST ty uses), the schema-checking core ports cleanly; the multiplexer is interim scaffolding by design.

### PyCharm support

Deferred well past pandas and polars. VS Code is the only supported editor for now.
