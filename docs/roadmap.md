# Roadmap

What's planned, in rough priority order. A living document — updated as the
project moves.

## Where pykrete is now

The **PySpark static checker is feature-complete**:

- The full DataFrame operation surface — `select` / `filter` / `join` /
  `groupBy`+`agg` / `withColumn(s)` / `drop` / `union` / `cube` / `rollup` /
  `pivot` / `transform` / `cast` / `toDF` / `df.na.*` / … — with
  result-schema inference through whole transformation chains.
- Inline SQL (`F.expr`, `selectExpr`, string-`filter`) and raw
  `spark.sql("SELECT …")`.
- `Window` partition/order key checking.
- Column-**existence** checking (`D0030`) and column-**type** checking —
  conservative (`D0080`, on by default) and strict (`D0081`/`D0082`, under
  `typeCheckingMode: strict`).
- Arbitrarily-nested `array` / `map` / `struct` column types — declared,
  structurally type-checked, and navigated field-by-field
  (`col("orders.line.sku")`).
- A `pyspark.sql.functions` result catalog and UDF return types.
- Cross-file imports and shared-schema modules.

The **LSP server** delivers live diagnostics, hover, completion (column
names in bare-string arguments and on chain results), document symbols,
go-to-definition, find-references, rename, and semantic tokens, and embeds
a Python language server (an LSP multiplexer — see
[design/multiplexer.md](design/multiplexer.md)). The **VS Code extension**
([../editors/vscode/](../editors/vscode/)) wraps it.

The **`.pyk` → `.py` transpiler** is complete: it prepends
`from __future__ import annotations` (so pykrete's atomic type names and
`SparkFrame[X]` / `PandasFrame[X]` / `DataFrame[X]` annotations don't
evaluate at runtime) and strips the schema-cast
`.cast(SparkFrame[Schema])` / `.cast(PandasFrame[Schema])` /
`.cast(DataFrame[Schema])` — the one pykrete-only construct in expression
position, which the Python runtime has no `.cast` method for.

**Pandas check-site coverage shipped in v1.3** alongside PySpark: the six
dispatched operations (`df[col_list]` / `df[mask]` / `df["new"] = expr` /
`df.drop` / `df.merge` / `df.rename`), `PandasFrame[X]` annotations, and
the `D0090 deprecatedDataFrameAlias` warning that nudges existing
`DataFrame[X]` users toward the dialect-specific spellings.

**Pandas depth shipped in v1.4**: seven new pandas-heavy donors in
pykrete-tests (scikit-learn, statsmodels, pandera, Great Expectations,
prophet, seaborn, yfinance — 3 direct-dispatch + 4 canonical-fixture-only),
bringing pandas-coverage donor count from 3 to 10; positive
`PROBE-TYPE-IS` coverage on `PandasFrame[X]` (21 markers across the 7 new
donors — 3 per donor, exactly meeting the v1.4 spec §1 floor); and three
checker bug closures (registry-call §10 widening,
`inherited_dialect` walrus receivers, `.transform(helper)` dialect
preservation) that close silent-pass paths surfaced by v1.3 audits.
`pykrete.json` config-discovery now walks from the input file's parent
directory (file-anchored, falling back to CWD) so absolute-path
invocations from outside the project root pick up the config.

**Cross-dialect handoff shipped in v1.5**: `df.toPandas()` re-tags
`SparkFrame[X]` to `PandasFrame[X]`; `spark.createDataFrame(pdf)`
re-tags `PandasFrame[Y]` back to `SparkFrame[Y]` when a `schema=`
keyword argument or a typed call-arg resolves to a known schema; the
round-trip path (`spark.createDataFrame(df.toPandas())`) preserves the
tag end-to-end. Pandas `.head()` / `.tail()` / `.first()` are
dialect-gated as Spark-only terminals so chains downstream of
`pdf.head(10).merge(other, on="id")` keep tracking. The v1.3 promise
of `.loc[:, "col"]` literal-form lands. Two PR-F1-class sibling gates
close (`column_name_arg` ungated arms + `collect_col_refs`
cross-DataFrame routing leak). A new `pykrete check --report-aliases`
flag emits a structured JSON envelope of every `DataFrame[X]`
annotation site with its resolved dialect, so projects can quantify
the v2.0 migration scope before v1.6's `pykrete migrate` ships. The
LSP synthetic-pool gets a soft cap with one-shot warning and
saturation sentinel, closing the v1.4 architecture-audit I4 finding.

**`pykrete migrate` + D0090 strict-mode escalation shipped in v1.6**:
`pykrete migrate` is the auto-rewriter binary for the deprecated
`DataFrame[X]` alias. It walks each `.pyk` file under each input
path, locates every `DataFrame[X]` annotation site via the
`AliasSite` byte-range model, applies call-graph dialect
adjudication to each binding's downstream usage (Spark-only methods
like `withColumn` / `createOrReplaceTempView` / `repartition` →
**Spark**; pandas-only methods like `assign` / `pivot_table` /
`.loc` / `.iloc` / pandas `merge` / `rename(columns=...)` →
**pandas**; both signals → **Ambiguous**; no signal → defaults to
Spark), and rewrites the annotation in place token-preservingly —
atomic per file (sibling temp + rename) so an interrupted run never
leaves half-rewritten source. v1.6 shipped the binary defaulting to
in-place rewrite; v1.7 flipped the default to `--check` (see
below). Ambiguous sites get an idempotent `# pykrete: ambiguous`
marker injected on the line above the unchanged annotation; re-runs
don't accumulate duplicates. Paired atomically with
`D0090 deprecatedDataFrameAlias` escalation: under
`"typeCheckingMode": "strict"` the warning now lands as **error**,
but the fix-button ships in the same release so strict-mode users on
green CI aren't stranded. Non-strict modes keep the warning
unchanged. Pandas `pivot_table(index=, columns=, values=, aggfunc=)`
literal-form column checking ships as the v1.6 pandas reshape
downpayment — string-literal arguments and list-of-literals shapes
resolve against `PandasFrame[X]`'s schema, firing D0030 with a *did
you mean*; variable arguments and callable `aggfunc` fall through.
Two v1.5 deferrals close: `.take()` is now dialect-gated (pandas
`pdf.take([0, 2])` passes through instead of dying as a Spark
terminal), and the `pdf.loc[mask, "col"]` nested-arg D0030 false
positive on the row-mask side closes. Plus audit-debt closure: the
`cross_dialect_handoff_gate` recognizer the v1.5 PR-A1/PR-A2
inference left as a "Keep in sync" comment is extracted to a single
shared site.

**Migrator UX hardening + pandas `melt` literal-form + Spark-D1
audit-debt closure shipped in v1.7**: `pykrete migrate`'s default
mode flips from in-place rewrite to `--check` preview. `--apply`
is the new opt-in for the in-place rewrite. The flip lands two
cycles after the binary first shipped; the v1.6 release notes
explicitly flagged the surface as pre-stable. A first-run on v1.7
with no flag emits a one-line stderr warning so adopters discover
the change without reading release notes. Pandas
`df.melt(id_vars=, value_vars=, var_name=, value_name=)`
literal-form column checking ships as the v1.7 pandas reshape
downpayment — string-literal arguments and list-of-literals shapes
resolve against `PandasFrame[X]`'s schema, firing D0030 with a
*did you mean*; variable arguments and the no-arg form fall
through. The pandas dispatch is gated on the
`receiver_is_pandas_inherited` arm so the existing Spark `melt`
arm's behavior on `SparkFrame[X]` receivers is unchanged. The
v1.6 architecture-audit Important #3 finding (parallel
`PANDAS_ONLY_SIGNALS` / `PANDAS_INHERITED_ARMS` lists with a "Keep
in sync" comment) closes via a shared `dialect_signals` module,
paired with a CI-guard test pinning the `expr.rs` pandas-arm
methods to the shared list. The Spark-D1 audit closure adds 14
Spark-only methods to `SPARK_DISCRIMINATORS` (`selectExpr`,
`freqItems`, `approxQuantile`, `crosstab`, `colRegex`, `summary`,
`mapInPandas`, `mapInArrow`, `writeTo`, `writeStream`, `unpivot`,
`rdd`, `isStreaming`, `sparkSession`) — `corr` / `cov` deliberately
excluded for pandas collision risk (caught at A2 review). Plus two
migrate UX hardenings: parse-error skips now surface on stderr, and
CRLF marker normalization lands for Windows source files. Internal
audit-debt mop-up: the `_source: &str` dead parameter is dropped
from `ambiguous_site_offsets` / `has_ambiguous_in_file`, and the
two-vector lockstep loop in the migrate driver's parse-error filter
collapses to a single-pass filter.

**v2.0 deprecation runway + spark-D2 D0091 + audit-class structural
closures shipped in v1.8**: the v1.8 headline is **`pykrete check
--deprecation-report` makes the v2.0 migration measurable**. A new
JSON envelope (`deprecationReportVersion: "1"`) inventories every
D0090-firing site with its adjudicated dialect and suggested
rewrite — drop it into CI to gate v2.0 readiness without re-parsing
diagnostic text. D0090's message text is amended in lockstep:
drops "and will be removed in pykrete v2.0" for the softer "slated
for removal in a future pykrete v2.0" and names the new
`--deprecation-report` flag inline so users encountering D0090 in
CI output have a one-command path to the migration surface. The
new **`D0091 crossDialectMethodMismatch`** warning fires when a
pandas-only method is called on a Spark receiver (`pdf.withColumn(...)`,
`pdf.selectExpr(...)`) or a Spark-only method on a pandas
receiver (`sdf.assign(...)`, `sdf.merge(...)`), with a *use
`.x(...)` instead* suggestion for the high-traffic pairs
(`withColumn` ↔ `assign`, `withColumnRenamed` ↔ `rename`,
`selectExpr` → `eval`, `toPandas` → `copy`, `groupby` → `groupBy`,
`merge` → `join`). Warning-only this cycle; strict-mode escalation
deferred to v1.9 because the back-compat surface is genuinely
larger than D0090's was. Carve-outs: deprecated `DataFrame[X]`
alias receivers skip the gate (avoid double-warning with D0090);
`pivot` and `melt` on Spark receivers don't fire (Spark exposes
same-spelled `groupBy(...).pivot(...)` and 3.4+ positional `melt`
surfaces). Two audit-class closures land structurally: `build.rs`
auto-generates the `PANDAS_INHERITED_ARM_METHODS` inventory from
`expr.rs` source (the hand-maintained list becomes a tripwire —
within-cycle drift is structurally impossible), and
`scripts/changelog-grep.sh` is a new CI gate that grep-anchors
every CHANGELOG-fenced binary string to `crates/pykrete/src/`.
Cross-codebase probe coverage extends to D0073 / D0083 (2 negative
probes each via pykrete-tests PR-P1 #30) — 247 → 249 → 253 probes
(PR-D1 #28 melt fills 247 → 249 in the v1.7 → v1.8 catalog window;
PR-P1 #30 D0073 / D0083 lifts 249 → 253).

**v2.0 migration plannability + D0091 maturity shipped in v1.9**:
the v1.9 headline is **`pykrete check --deprecation-report` makes
the v2.0 migration plannable**. The envelope bumps to
`deprecationReportVersion: "2"` with per-site `migrationStatus`
(`pending` / `acknowledged`) driven by a `# pykrete: ack-deprecation`
comment marker on the line above the alias annotation — site-level
opt-in, no JSON edit, no separate state file. A new
`--ack=<pending|acknowledged>` filter flag narrows the envelope to
one cohort for CI gating:
`pykrete check --deprecation-report --ack=pending src/` exits
non-zero with the unacked-site inventory; `--ack=acknowledged` runs
the inverse. The envelope deliberately ships without `targetVersion`
/ `removalVersion` / `shipDate` — pykrete tracks per-site migration
progress; the user picks the v2.0 ship date. D0091 matures:
strict-mode escalation (warning → error under `"typeCheckingMode":
"strict"`, mirroring the v1.6 D0090 precedent), a suggestion drift
guard pinning the cross-dialect suggestion table at build time, a
`shape_changes` hint that appends "— note arg shape differs" to
asymmetric mappings (`withColumnRenamed` → `rename`, `assign` →
`withColumn`), and a NEW bare-attribute inference arm on
`Expr::Attribute` that catches `pdf.rdd`, `sdf.loc`, `pdf.iloc`,
`sdf.toPandas` and the rest of the cross-dialect attribute surface
that v1.8's `Expr::Call` path missed (new property tables:
`SPARK_DISCRIMINATOR_PROPERTIES` + `PANDAS_INHERITED_PROPERTIES`).
The v1.8 `build.rs`-generated `PANDAS_INHERITED_ARM_METHODS`
inventory tripwire is now backed by CI-running tests via the
extracted `build_helpers.rs` module — closes the v1.8 retro rule
that the inline `mod tests` block shipped without actually
executing. The CHANGELOG grep gate gains a `text-numeric` label
that live-verifies numeric trust-claims (probes / fixtures / tests
/ donors) against live extracts. Cross-codebase probe coverage adds
2 D0091 negative probes (pandera + delta) via pykrete-tests PR-P1
#32 — 253 → 255 probes.

**v2.0 migration archivability + D0091 surface completion + pandas
`stack` arm + v1.9 audit-debt closure shipped in v1.10**: the v1.10
headline is **`pykrete check --deprecation-report --snapshot=<path>`
makes v2.0 migration archivable in CI**. File-write surface for the
v2 envelope: atomic write via tempfile-plus-rename, nanosecond-
suffixed temp name to avoid concurrent-writer collision, cleanup-
on-error guard across every error path. Compatible with `--ack`;
exit code stays at 0 (gating lives on D1's `--fail-on-nonempty`).
**`pykrete check --deprecation-report --fail-on-nonempty`** (closes
v1.9 arch-I4): CI gate flag exits non-zero when `sites` is non-
empty, replacing the `jq '.sites | length' | test ... -eq 0`
boilerplate adopters were writing by hand. **D0091 surface
completion**: `SPARK_DISCRIMINATOR_PROPERTIES` gains `na`, `write`,
`writeStream`, `storageLevel` (closes v1.9 spark-I1);
`PANDAS_INHERITED_PROPERTIES` gains `index`, `values`, `shape`, `T`
(closes v1.9 spark-I2) — both via the v1.9 bare-attribute path.
**Pandas
`df.stack(level=, dropna=)` literal-form** lands as a NEW inference
arm (continuing the one-reshape-arm-per-cycle cadence from v1.6
`pivot_table` and v1.7 `melt`); receiver-dialect-gated to fire only
on `PandasFrame[X]` receivers because Spark's `stack` is a column-
free-function. **v1.9 architecture audit-debt closes structurally**:
ack-marker multi-line signature support (the v1.9 walker silently
skipped past `def foo(` when the signature ran onto a continuation
line; v1.10 lands an indentation-aware walker + decorator-stack
skip — closes v1.9.1 #2 / v1.9 arch-I1), property / method tripwire
(build-time invariant: `SPARK_DISCRIMINATOR_PROPERTIES ⊂
SPARK_DISCRIMINATORS`; mirror for `PANDAS_INHERITED_PROPERTIES ⊂
PANDAS_ONLY_SIGNALS` — closes v1.9 arch-I3),
`.github/workflows/release-gate.yml` (full CHANGELOG grep gate
including live-extract step against the `pykrete-tests` sibling
checkout — closes v1.9 arch-I2), and CHANGELOG grep gate v3 prose
number scan (extends the v1.9 v2 gate to scan prose paragraphs;
single-backtick-wrapped numbers are the escape hatch — closes v1.9
retro rule 12). **§9.2 centralized version bump promoted to
standing practice** (closes v1.9 retro rule 1): trial verdict
SUCCESS — zero rebase-ladder collisions across Wave 1 of v1.9. The
`.github/centralized-bump-cycle.marker` mechanism was retired in
v1.16 (Cycle-0.1 #210); enforcement now keys on the `chore(release):`
release-PR title. Cross-codebase
probe coverage adds 6 D0091 strict-mode / bare-attribute /
shape-changes probes on `mlflow` / `dbt-spark` / `pandera` / `delta`
(pykrete-tests PR-P1 #34) plus the seaborn `stack(level=)` arm
(pykrete-tests #35) — 255 → 261 probes across 120 fixtures from
`17 donors`.

**v1.10 D0091 PR-D1 cross-codebase carve-out closes + pandas
`unstack` arm + audit-tooling block shipped in v1.11**: pykrete-
tests PR-P1 #39 ships cross-codebase property probes for the 8
v1.10 PR-D1 D0091 properties (`na`, `write`, `writeStream`,
`storageLevel`, `index`, `values`, `shape`, `T`) — the v1.10 carve-
out closes. **Pandas `df.unstack(level=, fill_value=)` literal-
form arm** lands as the v1.11 reshape arm (mirror of v1.10 `stack`;
continuing the one-per-cycle cadence). Receiver-dialect-gated to
fire only on `PandasFrame[X]` receivers; literal `level=` (single
string, list / tuple of strings) validates, int / int-list /
non-literal forms fall through to `Unknown`. The **audit-tooling
block** closes 5 cycles of v1.10 retro rules:
`scripts/trust-claim-sweep-checklist.sh` (docs-vs-history sweep —
17-test self-test suite; closes v1.10 retro rules 1+7),
`scripts/changelog-cite-check.sh` (CHANGELOG `path:LINE` cite
resolution against the working tree; closes v1.10 retro rule 3),
and `.github/workflows/auto-label-release-pr.yml` (auto-applies
`release-ready` label to release PRs; closes v1.10 retro rule 8).
CI-side release-gate label-trigger wiring is tracked for v1.12
(GITHUB_TOKEN cross-workflow trigger limitation — devs run the
sweep checklist locally before opening PR-F). LSP test tempdir-
per-test isolation (sentinel `pykrete.json` boundary; closes v1.10
probe-density audit flake). Walker polish (mixed-indent +
decorator-with-comment edges + tab/space test + counter-semantics
comment; closes v1.10 architecture audit polish items). Cross-
codebase probe coverage climbs 261 → 271 probes across
`130 fixtures` from `17 donors`.

**v1.11 GITHUB_TOKEN calendared promise closes + D0080 cross-
codebase trust coverage + `pivot_table(aggfunc=)` allowlist
shipped in v1.12**: The **auto-label workflow now dispatches
`release-gate.yml` via the `actions.createWorkflowDispatch` API**
after applying the `release-ready` label (PR-A1 #176). This
bypasses GitHub's GITHUB_TOKEN cross-workflow no-trigger rule so
the release-gate fires non-skipped on labeled PR events end-to-
end. Closes the v1.11.0 calendared promise. **Release-gate
runner perf — `cargo test` memoization** via
`PYKRETE_TESTS_COUNT_FILE` env var (PR-A2 #179): v1.11 PR-F's
release-gate had a duplicate `cargo test --release --workspace`
invocation (gate step + workflow step), causing a 34-min cold-
cache stall; the gate step now reads memoized count from the
file (~3 sec). Total release-gate cold-cache runtime drops from
~70min to ~35min. **D0080 returnTypeMismatch cross-codebase
trust coverage** lands (pykrete-tests PR-P1 #42) — the longest-
standing trust gap since v1.6 closed; dialect-on-return remains
a checker carve-out deferred to v1.13. **Pandas
`pivot_table(aggfunc=)` literal-form recognition** (PR-D1 #177):
11-string allowlist for the documented canonical aggfunc strings
(`sum` / `mean` / `count` / `min` / `max` / `median` / `std` /
`var` / `first` / `last` / `nunique`). Recognition is
informational — no diagnostic; result schema unchanged. Primes
v1.13+ aggfunc-driven inference. **Multi-line ack-marker
rationale block** (PR-V1 #178; spec §6.1.4): `# pykrete:
ack-deprecation` (shape b) now extends acknowledgement to the
entire contiguous comment block above the anchor. v1.10's
`marker → non-matching comment → def` reported `pending`; v1.12
reports `acknowledged`. Adopters with ack-deprecation tooling
that depended on v1.10's strict-single-line semantic should
update. **LSP tempdir parent-dir RAII guard** (PR-D2 #180):
`TestDir` struct + `Drop` impl wipes the parent sentinel
directory (including on test panic), closing v1.10 + v1.11
tempdir cleanup debt. Cross-codebase probe coverage climbs
271 → `279 probes` across `138 fixtures` from `17 donors`.

**D0080 dialect-on-return arm + pivot_table aggfunc Derived
schema synthesis + audit-tooling + CI-gate retire shipped in
v1.13**: The v1.13 cycle's headline closes the **longest-
standing checker correctness gap** — 7 cycles since v1.6 — by
landing the D0080 dialect-on-return arm. Functions annotated
`-> SparkFrame[X]` returning a `.toPandas()` chain (or any
expression resolving to a `PandasFrame[…]`) now fire D0080 at
`Severity::Error` with a dedicated dialect-mismatch message
(`Return type mismatch: declared as SparkFrame schema 'X' but the
body produces PandasFrame schema 'X'`).
**Pivot_table aggfunc-driven Derived-schema synthesis** lands as
the first observable aggregate-semantics-informed schema
inference: when `aggfunc=` is a recognized string and `values=`
carries one or more literal columns, the result schema synthesizes
a `Derived` envelope with the named columns at the aggregate-
driven dtype (`count` / `nunique` → int64; `mean` / `std` /
`var` / `median` → float64; `sum` / `min` / `max` / `first` /
`last` → preserve receiver column's type; default → mean →
float64). Multi-values + `columns=` (MultiIndex result) falls
through to Unknown. The **backtick-preservation tripwire**
(PR-A1 #188) prevents the 2-cycle backtick-strip regression class
identified in the v1.12 retro from recurring silently. The
**dispatched-run required-status-check + cancel-self workflow**
(PR-A2 #186) wires the release-gate's dispatched run as a required
status check on `chore(release):` PRs, retiring the v1.12 polling-
step. The **vscode CHANGELOG per-section masking test lock-in**
(PR-V1 #187) pins the v1.12 CHANGELOG-cite carve-out behavior so
future edits can't silently drift. Cross-codebase probe coverage
climbs `279` → `289 probes` across `148 fixtures` from
`17 donors`. The v1.13 cycle also marks the **first live
dispatched-event verification** (per v1.12 retro standing rule
§12) — confirmed end-to-end on PR-F #191. Adopters with code that
incorrectly cross-converts dialects at function boundaries or
that accidentally accessed non-`values=` columns post-
`pivot_table` will see new D0080 / D0030 fires — both flagged
plainly per pre-adoption trust-claim discipline.

**D0080 constructor carve-out closes + `groupby.agg` Derived
synthesis + `--compare-to` snapshot diff + envelope schema v2
provenance pair shipped in v1.14**: The v1.13 honest-silence
carve-out for constructor cases closes — `-> SparkFrame[X]`
returning `pd.DataFrame({...})`, or `-> PandasFrame[X]`
returning `spark.read.parquet(path)`, now fires D0080, and
`spark.createDataFrame(rows, schema=<Schema>)` fires when the
dialect disagrees with the annotation. When both dialect and
column-type mismatches land on the same return, D0080 emits a
single **multi-clause** message (one fire, two clauses) rather
than two stacked fires — CI greps for "two D0080 fires at the
same range" need to adjust. **`groupby.agg` Derived-schema
synthesis** lands as the sibling to v1.13's
`pivot_table(aggfunc=)` arm with a **shared inference helper**
(`count` / `nunique` → int64; `mean` / `std` / `var` / `median`
→ float64; `sum` / `min` / `max` / `first` / `last` preserve);
receiver-dialect-gated to `PandasFrame[X]`. **`pykrete check
--deprecation-report --compare-to=<prior.json>`** ships as the
SIMPLE three-bucket snapshot-diff primitive (`added` /
`removed` / `unchanged`); exit-nonzero on `added`; mutually
exclusive with `--ack` / `--snapshot` / `--fail-on-nonempty`
(gating modes vs delta-reporting mode don't compose). **Envelope
schema v2 provenance pair** — `pykreteSourceCommit` (in-tree
commit hash recorded at report time) + `generatedAt` (ISO-8601
timestamp) — round-trips through `--snapshot` and propagates to
`--compare-to` so CI snapshot artifacts carry author-pinpointing
metadata across release windows. Cross-codebase probe coverage
climbs `289` → `299 probes` across `158 fixtures` from
`17 donors`. Adopters with cross-dialect constructor returns at
function boundaries, or that accidentally accessed non-aggregated
columns post-`groupby.agg`, will see new D0080 / D0030 fires —
both flagged plainly per pre-adoption trust-claim discipline.

**pandas chain-depth extension + synthesis-arm cross-codebase
coverage closure + `resolve_override_ty` primitive + marketing-
table gate v3 shipped in v1.15**: The v1.14 `groupby.agg`
`Derived` envelope now survives one more transform —
`pdf.groupby("k").agg("sum").reset_index(drop=True)` keeps the
synthesized envelope alive (downstream `result["typo"]` fires
D0030 instead of degrading to Unknown), and
`pdf.set_index([literal-keys])` removes the named literal-key
columns from the accessible schema (downstream
`result["literal_key"]` fires D0030 instead of going silent).
The v1.14 synthesis-arm cross-codebase coverage gaps close in
pykrete-tests PR-P1 #50 with positive probes against
`groupby.agg` on real-library fixtures + negative probes against
D0080 constructor returns. The dtype-override family (the
inference table shared between `pivot_table(aggfunc=)` and
`groupby.agg`) consolidates behind the new `resolve_override_ty`
primitive in preparation for the window-aggregation arms (the
`Windowed` lattice variant was evaluated and rejected in v1.16
in favor of direct-chain recognition; deferred to v1.17). The
audit-tooling block gains **marketing-table gate v3** —
`scripts/trust-claim-sweep-checklist.sh` now fires
`MARKETING-TABLE-CLAIM-STALE` on bare `<num> <key>` markdown-
table cells that don't match a current `text-numeric` pin OR a
`text-numeric-historical` block in CHANGELOG. The auto-label
workflow gains native top-level `concurrency:` (PR-A2). Cross-
codebase probe coverage climbs `299` → `305 probes` across
`164 fixtures` from `17 donors`. Adopters who incorrectly
accessed non-aggregate columns after `groupby.agg().reset_index
(drop=True)`, or who accessed literal keys after `set_index
([keys])`, will see new D0030 fires — both flagged plainly per
pre-adoption trust-claim discipline.

**time/window aggregation + dict & callable `groupby.agg` +
honest-silence declines shipped in v1.16**:
`pdf.resample("<rule>").agg("<fn>")` synthesizes a `Derived`
schema at the aggregate-driven dtype (`count` / `nunique` →
Long; `mean` / `std` / `var` / `median` → Double; `sum` /
`min` / `max` / `first` / `last` preserve the receiver
column's dtype), and `pdf.rolling(<n>).agg("<fn>")`
synthesizes all-Double — pandas rolling upcasts numeric
columns to `float64` regardless of the aggregate, so the
`groupby` dtype table does not carry over. `rolling.agg` only
synthesizes when every receiver column is numeric, and its
aggfuncs are restricted to `count` / `sum` / `mean` / `std` /
`var` / `median` / `min` / `max`. Both arms recognize a SINGLE
expression: direct-method `pdf.resample("M").sum()`,
held-intermediate chains, dict / list / callable aggfuncs on
window chains, and non-literal rule / window arguments all
fall through to Unknown. `groupby.agg` gains its dict form
(keeping ONLY the named columns plus group keys — unnamed
receiver columns are dropped, the highest-impact new D0030
source this cycle) and its callable form (`np.mean` / `len`,
which on an all-numeric frame keeps keys plus every non-key
column at Unknown dtype). Four shapes now decline to Unknown where the checker
previously over-claimed: named-aggregation
`groupby(k).agg(out=(col, fn))` — a FALSE POSITIVE through
v1.15 that synthesized a keys-only schema and fired a bogus
`returnColumnsMismatch` on correct code — `rolling` over any
non-numeric column, numeric-restricting aggregation over a
frame carrying a non-numeric column (on `groupby.agg` and
`resample.agg` alike), and `resample(<rule>, on=<col>)`. The
rationale is identical for the first three: pandas 2.x RAISES
(`TypeError` / `DataError`) rather than silently dropping the
column the way pre-2.0 pandas did, and pykrete does not
synthesize a schema for code that errors at runtime. The
fourth applies the same principle to shape rather than dtype —
pandas moves the `on=` column into the resample index, so
keeping it claimed a column the result does not have;
`rolling(<n>, on=<col>)` keeps the column and is deliberately
unaffected. `reset_index(inplace=True)`
and `set_index(inplace=True)` resolve to None, matching
pandas, with D0030 key-existence still validated before the
inplace punt. The audit side ships the
`--expected-failures.json` countdown allowlist and the
roadmap-header drift guard; the §9.2 marker mechanism is
retired in favor of release-PR-title gating. The
`SchemaView::Windowed` lattice variant was evaluated and
REJECTED this cycle in favor of direct-chain recognition; it
is deferred to v1.17. Cross-codebase probe coverage climbs
`305` → `312 probes` across `171 fixtures` from `17 donors`
(pykrete-tests PR-P1 #54) — the window-aggregation arms have
NO cross-codebase probes; they are unit-tested in-crate, with
donor coverage tracked for v1.17. Adopters who accessed
columns dropped by a dict-form `groupby.agg`, or who chained
off an `inplace=True` reset/set result, will see new D0030
fires — both flagged plainly per pre-adoption trust-claim
discipline.

## Next up

### v1.17 — rest of the window surface + remaining pandas reshape + non-literal indexing

The rest of the window-aggregation surface: direct-method
`df.resample("M").sum()` / `df.rolling(7).mean()` (v1.16
recognizes only the `.agg("<str>")` spelling, and the
direct-method form is the more idiomatic one),
held-intermediate chains (`r = df.resample("D")` then
`r.agg(...)` — where the deferred `SchemaView::Windowed`
lattice variant would earn its keep), dict / list / callable
aggfuncs on window chains, non-literal rule and window
arguments, and `rolling.agg("first"/"last"/"nunique")`.
`expanding.agg`, the cumulative-window sibling. Precise
per-output-column modeling for named-aggregation
`groupby(k).agg(out=(col, fn))`, which v1.16 declines to
Unknown. Cross-codebase probes for the v1.16
window-aggregation arms, which no donor exercises yet. Broader
pandas reshape output schemas: `reset_index(drop=False)`
index-as-column promotion, `set_index(<expr>)` non-literal
forms, the list-of-aggfuncs-per-column `groupby.agg` shape,
plus full `pivot_table` multi-aggfunc / `melt` / `stack` /
`unstack` output schema-tracking (v1.10 + v1.11 shipped
`stack` / `unstack` literal-form on the input; v1.13 lands
pivot_table `Derived` synthesis; v1.14 lands `groupby.agg`
single-string `Derived` synthesis; v1.15 lands
`reset_index(drop=True)` + `set_index([literal-keys])`
chain-depth pass-through; v1.16 lands the `resample.agg` /
`rolling.agg` direct chains and the dict / callable
`groupby.agg` forms; the long-format output schemas pair with
the rest of pandas reshape).

`.loc` non-literal forms (`.loc[mask, "col"]` boolean-mask row
keys, `.loc[:, "a":"b"]` column-range slicing) and `pdf.iloc
[...]` integer-position indexing. The `df.query("…")` and
`df.eval("…")` mini-DSLs (numexpr-influenced syntax, separate
parser from the SQL path used by `selectExpr`). `pd.read_csv
(...)` and other pandas I/O entry points if scope allows
(schema inference from file headers / SQL / type-stubs is a
separate design surface). PROBE-TYPE-IS retrofit to the v1.3
hybrid donors (MLflow, Feast, iceberg-python). Canonical-vs-
direct CI gate (I3 from the v1.4 architecture audit).
`--include-py` flag for `pykrete migrate` to walk the
multiplexer cohort's `.py` files alongside `.pyk`, plus a
`--changed-only` flag for both `pykrete migrate` and
`pykrete check` that walks only files changed against HEAD.
**D0030 message rendering on
synthesized grouped-key typo path** — v1.15 fires D0030 against
the synthesized `Derived` envelope but the message text path
still resolves to the pre-synthesis schema in some arms; v1.17
polish. **`as_index=False` / `observed=` / `dropna=` /
`numeric_only=` kwargs-aware groupby** — the `groupby.agg`
chain-depth is still keyword-blind, and `numeric_only=True`
makes pandas drop the non-numeric columns and succeed where
v1.16 declines the whole frame to Unknown.
**`reset_index(level=...)` MultiIndex slice +
`set_index(<mixed-literal>)` asymmetric defense test** — v1.15
narrow-arms the literal-form; v1.17 widens. `pd.DataFrame.
attribute_access` form for D0030 tracking on the pandas
attribute-access surface. **LSP polish formally rescoped to
v2.0.1 / discrete LSP-feature work** per v1.10 spec §10.10, NOT
a v1.x bundle — three cycles (v1.7 / v1.8 / v1.9) of "carry
forward to next minor" was the signal that LSP polish doesn't
fit the v1.x cadence.

## PyCharm support

A JetBrains integration via PyCharm's LSP client. Deferred until after
polars — VS Code is the only supported editor for now.

## Configuration

A `pykrete.json` at (or above) the project root configures both the CLI
and the LSP — `typeCheckingMode` (`off` / `basic` / `standard` /
`strict`), `exclude` (path substrings to skip), and `rules` (per-rule
overrides — `off` / `warning` / `error`, keyed by readable rule name).
For the LSP, `pykrete.json`'s `typeCheckingMode` overrides the editor's
setting; the single value also drives the embedded Python engine.

## Quality-of-life

- **User-facing language reference** ([`language-reference/`](language-reference/))
  — schema syntax, operation reference, error catalog, configuration,
  cookbook. The doc lives but is empty.
- **Zed extension** — Neovim, Helix and Emacs setups are wired in
  [`editors/`](editors/); Zed needs a dedicated extension.

Already shipped (recorded here for completeness):

- **In-browser playground reaches pykrete IDE parity.** The Monaco
  editor at [`/playground`](https://amirnaderi93.github.io/pykrete/playground/)
  now serves the same pykrete capabilities the VS Code extension does
  for `.pyk` files: hover on schema names, `SparkFrame[X]` /
  `PandasFrame[X]` / `DataFrame[X]` references, and chain-bound locals;
  column-name completion inside `col("…")` and schema-name completion
  inside `SparkFrame[…]` / `PandasFrame[…]` / `DataFrame[…]`; and
  go-to-definition on Schema references. Wired through three new `pykrete-wasm` entry
  points (`hover_at`, `complete_at`, `definition_at`) that delegate to
  the same `pykrete::hover` / `pykrete::completions` / `pykrete::definition`
  the LSP server uses, so playground behavior matches a local install.
  Follow-up: the embedded Python language server (the multiplexer's
  half) isn't reachable from the browser yet — Python-side hover,
  parameter info, and imports still need the desktop install.
- **Performance pass.** Project-scope hot paths reviewed and
  micro-optimized: schema-name resolution (previously a linear scan over
  every project-wide schema) is now a `HashMap` index keyed by name on
  the per-function `BodyContext`; the `discover_schemas` fixpoint sweep
  uses a name → class-index table instead of an `O(N²)`
  `iter().position(...)` per (class, base) pair. A `tests/perf.rs`
  smoke test exercises a synthetic 50-file / 1500-schema project and
  asserts the release-build wall-clock stays inside a generous budget
  so an order-of-magnitude regression is caught in CI.
- **Duplicate-name detection across files.** `D0072 duplicateSchemaName`
  warns when the same `class X(Schema)` is declared in more than one
  project file. The alphabetically-earliest declaration is treated as
  the canonical site; every later one gets a warning that names both
  files. Same-file redeclarations don't fire D0072 (different
  concern), and function-name duplicates aren't covered yet.

- **Generic-inference: full coverage of the four extension patterns.**
  Multi-TypeVar binding —
  `def join[A, B](left: SparkFrame[A], right: SparkFrame[B]) -> SparkFrame[Merge[A, B]]`
  binds each TypeVar from its own argument slot and substitutes through
  the return, producing a derived view with the concatenated columns.
  Nested parameter shapes are unwrapped during binding:
  `List[DataSource[T]]`, `Optional[DataSource[T]]`,
  `Dict[str, DataSource[T]]`, and arbitrary re-nesting
  (`List[List[DataSource[T]]]`) all reach the inner `G[T]` shape.
  Chained class-method calls —
  `dal.with_path("/x").read(SOURCE)` — preserve class identity through
  any intermediate method whose return annotation is the class itself
  (`-> "DataAccessLayer"`, `-> DataAccessLayer`, or `-> Self`), so the
  trailing generic call still dispatches. `type[T]`-shaped parameters —
  `def cast_to[T](self, _: type[T]) -> SparkFrame[T]` called as
  `dal.cast_to(Orders)` — bind T from the arg's class identifier
  rather than its runtime value. Incompatible bindings (a list whose
  elements carry different T values, a non-class arg in a `type[T]`
  slot) degrade the offending TypeVar to Unknown rather than fabricate
  a result, keeping the no-false-positive stance.
- **`melt` / `unpivot` output-schema modeling.** Spark 3.4+'s
  wide-to-long reshape (`df.melt(ids, values, variableColumnName,
  valueColumnName)` and its alias `df.unpivot(...)`) now produces a
  modeled result schema: the `ids` columns are preserved with their
  declared types and nullability, the variable column is `string`, and
  the value column carries the common type of the unpivoted `values`
  columns (with numeric widening — `int` < `long` < `double` — and
  `Nullable(T)` when any value column is nullable). `values=None` or
  omitted unpivots every non-`ids` column. Typos in `ids` or `values`
  fire `D0030`; heterogeneous value types degrade to Unknown rather than
  fabricate a common type, so downstream checks stay permissive.
- **Date/time first-arg column checking + array higher-order function
  recognition.** The single-column date helpers — `F.to_date`,
  `F.to_timestamp`, `F.date_format`, `F.trunc`, `F.next_day`,
  `F.from_utc_timestamp`, `F.to_utc_timestamp`, `F.from_unixtime`,
  `F.unix_timestamp`, and the position-2 variant `F.date_trunc(format,
  col)` — now flag `D0030` on a typo in the column slot while the
  format / timezone string is left alone. `date_format` and
  `from_unixtime` joined the typed result catalog (→ string). The array
  higher-order functions `F.transform`, `F.filter`, `F.aggregate`,
  `F.exists`, `F.forall` are recognized — first-arg column ref checked,
  return type modeled per function (`array<lambda body>` for
  `transform`, input array preserved for `filter`, lambda body type for
  `aggregate`, bool for `exists` / `forall`). Lambda bodies are
  inferred best-effort and fall back to Unknown when not traceable.
- **`F.when` / `F.otherwise` result-type inference and `F.struct` /
  `F.named_struct` schema construction.** `F.when(p, v).otherwise(e)`
  chains now infer their result as the common type of the value
  branches (atomic equality, then numeric widening — `int` < `long` <
  `double`); chains without `.otherwise(...)` resolve to `Nullable(T)`
  since unmatched rows yield null. `F.struct(col("a"), col("b"))` now
  produces a `Struct({a: int, b: string})` whose field names come from
  `.alias("x")` first then the column name, and whose types are each
  arg's inferred type; `F.named_struct("k1", v1, "k2", v2)` uses the
  string-literal name slots as field names. Composes with `.getField`,
  so a freshly-constructed struct can be navigated immediately.
- **`createOrReplaceTempView` + `spark.sql("SELECT … FROM view")`
  resolution.** `df.createOrReplaceTempView("v")` registers `df`'s
  schema against the view name in a per-file registry; a subsequent
  `spark.sql("SELECT … FROM v")` in the same file checks every column
  identifier in the query (projection, `WHERE`, `GROUP BY`, `ORDER BY`,
  `HAVING`) against the view's schema, firing `D0030` on a typo, and
  returns either the projected columns or the view's full schema for
  `SELECT *`. Single-table SELECT only, within-file only.
- **`Column` method chain recognition.** `.isNull` / `.isNotNull` /
  `.isin` / `.between` / `.like` / `.rlike` / `.ilike` / `.contains` /
  `.startswith` / `.endswith` are now recognized as boolean-returning
  Column predicates that preserve the chain; `.getField` resolves the
  nested struct field's type and fires `D0030` on a field-name typo;
  `.getItem` returns the array element / map value type;
  `.withField` and `.dropFields` track the receiver's struct shape
  forward with the field added, replaced, or removed.
- **Set ops, `F.broadcast`, and terminal recognizers.** `intersect`,
  `intersectAll`, `subtract`, `exceptAll` are recognized set operations
  sharing the same schema-mismatch check (`D0040`) as `union` /
  `unionByName`; `unionAll` is wired as a deprecated alias for `union`.
  `F.broadcast(df)` is treated as a pass-through, so chains like
  `df1.join(F.broadcast(df2), "k")` keep tracking the schema. The nine
  terminal methods (`count`, `collect`, `show`, `printSchema`, `explain`,
  `first`, `take`, `head`, `tail`) are recognized centrally — the chain
  dies cleanly and a future "chain-after-terminal" diagnostic has a
  single seam to attach to.
- **`spark.read.<format>(path)` / `spark.table(name)` opaque-source
  recognition.** `DataFrameReader` chains (`spark.read.parquet(...)`,
  `spark.read.format(...).load(...)`, `spark.read.schema(...).<format>(...)`)
  and bare `spark.table(...)` are now recognized as opaque IO sources.
  The result is still Unknown — the schema is genuinely runtime data —
  but the user re-anchors the chain with `.cast(SparkFrame[Schema])` or a
  typed variable annotation (`raw: SparkFrame[Schema] = spark.read.parquet(...)`)
  and downstream column checks resume. Closes the headline gap where
  real PySpark codebases lost their chain at line one.
- **Call-site argument checking** (`D0051 argumentColumnsMismatch`) —
  closes the function boundary on the input side. Passing a
  `SparkFrame[Wrong]` into a function that declares `SparkFrame[Right]`
  is now flagged at the call site, with the same missing / extra column
  reporting as `returnColumnsMismatch`. v0.1.8 closes the edge cases:
  local-name shadowing of a top-level function suppresses the check —
  including tuple-unpack (`revenue, _ = …`) and walrus (`(revenue := …)`)
  rebinds; positional-only (`/`) and keyword-only (`*`) parameter
  markers are honored when matching arguments; `*args` / `**kwargs`
  variadics are checked against every call-site argument routed to
  them; and a parameter filled both positionally and by keyword
  (Python's `TypeError`) is diagnosed once, not twice.
- **Packaging.** GitHub Releases with prebuilt binaries for macOS
  (arm64/x64), Linux x64, and a Windows MSI installer; a Homebrew tap
  (`brew install amirnaderi93/pykrete/pykrete`); `cargo install --git`.
  Each release ships through the release workflow automatically.
- **VS Code extension on both registries.** The Visual Studio Marketplace
  (for VS Code) and the Open VSX Registry (for Cursor, VSCodium,
  code-server, Theia). A `.vsix` is also attached to every release for
  side-loading.
- **Editor-agnostic LSP setup docs** for Neovim, Helix and Emacs in
  [`editors/`](editors/).

## Strategic direction

These are larger structural moves, not increments.

### Multi-dataframe support (pandas, polars, …)

Status: **PySpark feature-complete; pandas check-site coverage shipped in
v1.3; polars is next.** Every dataframe library has the same shape — a
value carries a schema, methods narrow or widen it, column names must
exist when referenced.

Priority: **PySpark (done) → pandas check-site (done, v1.3) → pandas
depth + type-tracking (done, v1.4) → cross-dialect handoffs +
deferred-promise closure (done, v1.5) → `pykrete migrate` paired with
D0090 strict-mode escalation + pandas `pivot_table` literal-form +
`.take()` dialect-gate closure (done, v1.6) → migrator UX hardening
(`pykrete migrate` defaults to `--check`) + pandas `melt`
literal-form + `dialect_signals` shared module + Spark-D1 audit
closure (done, v1.7) → v2.0 deprecation runway (`pykrete check
--deprecation-report` JSON envelope + D0090 message amend) +
spark-D2 D0091 cross-dialect mismatch warning + build.rs-generated
inventory + CHANGELOG-binary CI gate (done, v1.8) → v2.0 migration
plannability (`--deprecation-report` v2 envelope with per-site
`migrationStatus` + `--ack` filter) + D0091 maturity (strict-mode
escalation + suggestion drift guard + `shape_changes` hint + new
bare-attribute inference arm) + `text-numeric` CHANGELOG gate
(done, v1.9) → v2.0 migration archivability
(`--deprecation-report --snapshot=<path>` file-write +
`--fail-on-nonempty` CI gate) + D0091 surface completion (8 new
properties — 4 Spark-direction, 4 pandas-direction) + pandas
`df.stack(level=, dropna=)` literal-form + v1.9 audit-debt
closure (done, v1.10) → pandas `df.unstack(level=, fill_value=)`
literal-form + cross-codebase property probes for the 8 v1.10
D0091 properties + audit-tooling block (trust-claim sweep checklist
+ CHANGELOG cite-check + auto-label workflow) (done, v1.11) →
v1.11 GITHUB_TOKEN calendared promise closure (auto-label
workflow dispatches `release-gate.yml` via
`actions.createWorkflowDispatch`) + D0080 returnTypeMismatch
cross-codebase trust coverage (longest-standing trust gap since
v1.6 closed) + pandas `pivot_table(aggfunc=)` 11-string allowlist
recognition (informational; primes v1.13+ aggfunc-driven
inference) + multi-line ack-marker rationale block (spec §6.1.4)
+ release-gate `cargo test` memoization (done, v1.12) →
D0080 dialect-on-return checker arm + broader pandas reshape
(`groupby.agg`) + `.loc` / `.iloc` non-literal forms +
`.query` / `.eval` mini-DSLs + `--include-py` /
`--changed-only` / `--compare-to` migrate flags (v1.13; LSP
polish formally rescoped to v2.0.1 / discrete LSP-feature work
per v1.10 spec §10.10) → polars** → others (DuckDB, Dask, …).

The core type model — `SparkFrame[Schema]` / `PandasFrame[Schema]` /
`DataFrame[Schema]`, the `Schema` class, column checks, return-type
validation — generalizes. The library-specific layer is method dispatch
(`raw.select(col("x"))` vs `raw[["x"]]` vs `raw.select(pl.col("x"))`).
v1.3 shipped the per-annotation dispatch (`SparkFrame[X]` recognizes
Spark shapes, `PandasFrame[X]` recognizes pandas shapes — `df[col_list]`
/ `df[mask]` / `df["new"] = expr` / `df.drop` / `df.merge` / `df.rename`)
and the `D0090` deprecation that nudges callers off the legacy
`DataFrame[X]` alias. v1.4 shipped pandas type-tracking on
`PandasFrame[X]` via the `PROBE-TYPE-IS` synth
(`{df}.assign(__probe={df}["x"] + 1)` — a dispatched op so off-claim
numeric dtypes fall through to D0081), seven new pandas donors with 21
TYPE-IS markers (3 per donor), and three PRE-EXISTING silent-pass checker bug
closures (registry-call args, walrus receivers, `.transform` dialect
preservation). v1.5 shipped cross-dialect handoff (`.toPandas()` →
`PandasFrame[X]`; `spark.createDataFrame(pdf)` → `SparkFrame[Y]` when a
schema source is present), the v1.3 promise of `.loc[:, "col"]`
literal-form, dialect-gated `.head` / `.tail` / `.first` for pandas
chains, two PR-F1-class sibling gates (`column_name_arg` ungated arms +
`collect_col_refs` cross-DataFrame routing), the `--report-aliases` JSON
envelope for v2.0 migration sizing, and the synthetic-pool soft cap
that closes the v1.4 architecture-audit I4 finding. v1.6 ships
`pykrete migrate` — the auto-rewriter binary paired with D0090
strict-mode escalation — plus pandas `pivot_table` literal-form column
checking, the `.take()` dialect-gate closure, the `pdf.loc[mask, "col"]`
nested-arg D0030 FP closure, and the audit-debt `cross_dialect_handoff_gate`
recognizer extraction.

### Forking `ty`

Long term, pykrete may fork Astral's `ty` (their Rust Python type checker)
once it reaches a stable release — a single native stack, replacing the
basedpyright multiplexer. Because pykrete's analyzer is already built on
`ruff_python_ast` (the AST `ty` uses), the schema-checking core ports
cleanly; the multiplexer is interim scaffolding by design.

