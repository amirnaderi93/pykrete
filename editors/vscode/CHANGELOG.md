# Changelog


## 0.14.0

Tracks the v1.16.0 pykrete release — **extending pandas reshape modeling
to time/window aggregation**. The bundled `pykrete` and `pykrete-lsp`
binaries gain four NEW pandas inference arms that synthesize a Derived
schema where the chain previously fell through to Unknown:
`df.resample("D").agg("sum")` and `df.rolling(3).agg("mean")` direct
chains, and the dict-form (`df.groupby(k).agg({col: fn})`) and callable
(`df.groupby(k).agg(np.mean)`) shapes of `groupby.agg`. `resample.agg`
follows the aggregate-to-dtype table (count/nunique → Long;
mean/std/var/median → Double; sum/min/max/first/last preserve the
receiver); `rolling.agg` aggregates numeric columns to Double but declines
to Unknown when the frame contains any non-numeric column (honest
silence — pandas 2.x raises on non-numeric rolling aggregation, so pykrete
does not synthesize a schema for code that errors), with aggfuncs
restricted to count/sum/mean/std/var/median/min/max. Dict-form
`groupby.agg` keeps only the named columns plus group keys; callable
`groupby.agg` keeps keys plus all non-key columns at Unknown dtype on
all-numeric frames, and declines to Unknown when any non-key column is
non-numeric (pandas raises on a numeric-restricting callable like
`np.mean` over non-numeric).
Named-aggregation (`groupby(k).agg(out=(col, fn))`) is not yet modeled —
it falls through to Unknown (v1.17). `reset_index(inplace=True)`
and `set_index(inplace=True)` now correctly resolve to None — pandas
returns None for the inplace forms — instead of synthesizing a schema;
column-existence (D0030) is still validated before the inplace punt.
**Adopters whose code accessed columns dropped by a dict-form
`groupby.agg`, or that chained off an `inplace=True` reset/set result,
will see new D0030 / attribute fires** — align downstream code with the
synthesized schema or drop `inplace=True`. The audit side ships the
`--expected-failures.json` allowlist for the trust-claim-sweep gate
(`expiresAfter` countdown + fail-closed reconciliation), the
roadmap-header drift guard, and native `concurrency:` single-flight caps
on `ci.yml` / `wasm.yml` / `extension-version-guard.yml`. The §9.2
centralized-bump chicken-and-egg is retired: bump-enforcement now gates on
the release-PR title (`chore(release):`) rather than a mid-cycle marker
file. Cross-codebase coverage: 305 → 312 probes across 164 → 171 fixtures
from 17 donors. No new D-codes; SemVer-minor under the
`tighteningDiagnostics` policy and the established JSON-additive policy.
Cycle-close minor bump aligns the extension with the v1.16.0 tag per the
version-guard contract. See the
[main CHANGELOG](../../CHANGELOG.md#1160---2026-07-26) for details.

## 0.13.0

Tracks the v1.15.0 pykrete release — **closing 5 audit-debt carve-outs
from v1.14 and extending pandas chain-depth via `reset_index(drop=True)`
+ `set_index([literal-keys])`** so the canonical
`groupby.agg().reset_index(drop=True)` chain continues tracking the
Derived schema 2-methods-deep instead of degrading to Unknown after the
reset. The bundled `pykrete` and `pykrete-lsp` binaries gain two NEW
pandas inference arms: `pdf.groupby("k").agg("sum").reset_index(drop=True)`
now propagates the v1.14 PR-D2 synthesized envelope through the reset
(previously the result degraded to Unknown — downstream `result["typo"]`
was silent), and `pdf.set_index(["k1", "k2"])` now removes the literal
keys from the accessible column set so downstream typos against the
removed keys fire D0030. Kwarg-safety is explicit: `set_index(drop=False)`
or `append=True` falls through to Unknown rather than emitting
false-positive D0030 on the still-accessible keys. Out-of-scope and
explicitly deferred to v1.16: `reset_index(drop=False)` (preserves index
AS new column — stateful tracking needed), `set_index(<expr>)` non-literal
forms (requires expr-eval). The audit side ships **marketing-table gate
v3**: `scripts/trust-claim-sweep-checklist.sh` extends the v1.14
backticked-claim-stale scanner to catch bare `<num> <key>` claims in
markdown-table contexts (the v1.14 architecture audit nit at
`pandas-roadmap.md:103-114` blind-spot), and consolidates the v1.14
`assemble_surfaces()` + `assemble_stale_surfaces()` pair into a single
`collect_surfaces()` helper (v1.14 PR-A1 R2 follow-up DRY-up). The
`auto-label-release-pr.yml` workflow gains a top-level native
`concurrency:` block keyed on PR number with `cancel-in-progress: true`,
closing v1.14 retro rule 2's concurrency-race — dispatched-event
verification was captured on PR-A2 #206 itself (commits 4+5 back-to-back;
commit 4 run pre-empted before being recorded; commit 5 succeeded). The
`resolve_override_ty` 8-line primitive lands as a quiet refactor shared
by `synthesize_pivot_result_from_aggfunc` + `synthesize_groupby_agg_
from_aggfunc` (v1.14 backlog #4 closure); outer iteration scaffolding
stays divergent because pivot is values-only and groupby is
keys-then-non-keys. Cross-codebase coverage: 299 → 305 probes across
158 → 164 fixtures from 17 donors. No new D-codes; SemVer-minor under
the `tighteningDiagnostics` policy and the established alias-report-style
JSON-additive policy. Cycle-close minor bump aligns the extension with
the v1.15.0 tag per the version-guard contract. See the
[main CHANGELOG](../../CHANGELOG.md#1150---2026-06-24) for details.

## 0.12.0

Tracks the v1.14.0 pykrete release — **turning v1.13's `pivot_table`
aggfunc-driven Derived synthesis into the canonical convention via
`groupby.agg`**, closing **D0080 dialect-on-return at constructor
sites** (the v1.13 PR-D1 honest-silence carve-out: `pd.DataFrame(...)`
and `spark.read.<format>(...)` return shapes), and landing the
**5-cycle calendared `--compare-to <snapshot>` user-decision**. The
bundled `pykrete` and `pykrete-lsp` binaries gain a NEW pandas
inference arm for `pdf.groupby("k").agg("sum")`: the result is no
longer Unknown — it synthesizes a `Derived` envelope with
`keys ++ (non-keys at aggregate-driven dtype)` per the v1.13
pivot_table table (`count` / `nunique` → int64; `mean` / `std` /
`var` / `median` → float64; `sum` / `min` / `max` / `first` / `last`
→ preserve receiver column type), so downstream column-refs on the
result that don't match now fire D0030. The narrow-arm covers the
literal-string-aggfunc form; dict / callable / list-of-aggfunc /
multi-aggregate MultiIndex forms are deferred to v1.15 per the
one-arm-per-cycle cadence. The v1.13 D0080 honest-silence carve-out
closes structurally: `inherited_chain_state` gains three constructor-
arm recognizers (`pd.DataFrame(...)` single-level structural,
`spark.read.<format>(...)` two-level structural across any format
identifier, and `spark.createDataFrame(rows, schema=<bare Schema>)`
dialect-only sibling check). Coverage of the adopter shape moves
from ~80% to ~95% — a function annotated `-> SparkFrame[X]` whose
body returns `pd.DataFrame({...})`, or one annotated
`-> PandasFrame[X]` whose body returns `spark.read.parquet(path)`,
now fires D0080. Combined dialect-and-type mismatches at the same
function-return site now emit ONE D0080 with `"; additionally, "`
joining the two clauses instead of two separate diagnostics at the
same range — sets the multi-clause joiner convention for future
D-code consumers. The bundled `pykrete` CLI gains
`pykrete check --deprecation-report --compare-to <snapshot.json>`:
SIMPLE three-bucket diff (`added` / `removed` / `unchanged`) with
the FULL site payload in each bucket, exit-nonzero on
`added.length > 0` for regression-gate CI. Binary `MigrationStatus`
transitions (`pending` → `acknowledged`) surface naturally as a
remove-from-old plus add-to-new pair, so no separate `status_changed`
bucket is needed. The deprecation-report envelope schema bumps to
v2 in lockstep: two new top-level keys (`pykreteSourceCommit` +
`generatedAt`, CLI-captured at snapshot time) ship with every new
report so `--compare-to` round-trips full provenance; pre-v1.14
snapshots remain readable (both keys treated as nullable). On the
audit-tooling side, `trust-claim-sweep-checklist.sh` gains the
`scan_backticked_stale_numbers()` scanner — catches backticked-but-
stale numeric claims that escaped the v1.13 backtick carve-out (the
root cause behind v1.13's docs-sync 8-blocker audit), with v1.4 →
v1.8 historical pins cleaned in `docs-site/src/content/docs/about/
pandas-roadmap.md` in the same PR. The
`.github/workflows/auto-label-release-pr.yml` workflow adds the
`labeled` trigger to `pull_request.types` — investigation revealed
v1.13 retro rule 11's framing was wrong: synchronize-redispatch
worked since v1.12 PR-A1; the actual gap was `labeled` missing from
triggers, so the operator's manual remove+re-add was a no-op reflex
(memory reframed). Cross-codebase coverage: 289 → 299 probes across
148 → 158 fixtures from 17 donors. No new D-codes; SemVer-minor
under the `tighteningDiagnostics` policy and the established alias-
report-style JSON-additive policy. Cycle-close minor bump aligns
the extension with the v1.14.0 tag per the version-guard contract.
See the [main CHANGELOG](../../CHANGELOG.md#1140---2026-06-23) for
details.

## 0.11.0

Tracks the v1.13.0 pykrete release — **closing D0080's dialect-on-
return gap** (the longest-standing 7-cycle correctness hole,
v1.6 → v1.12 silent), turning v1.12's `pivot_table(aggfunc=)`
classifier into observable schema inference, and converting the
dispatched release-gate into a required status check. The
bundled `pykrete` and `pykrete-lsp` binaries gain a NEW D0080
checker arm: `check_return_type` now compares the `Dialect` tag,
so a function annotated `-> SparkFrame[X]` whose body returns a
`.toPandas()` chain fires D0080 with a dedicated dialect-mismatch
message ("declared as SparkFrame schema 'X' but the body produces
PandasFrame schema 'X'"). Honest-silence policy holds for
constructor cases (`pd.DataFrame(...)`,
`spark.read.parquet(...)`); v1.14 extends `inherited_chain_state`
with constructor arms. Pandas `pivot_table(aggfunc=)` schema
inference lands: v1.12 PR-D1's dead-code primer is now consumed,
synthesizing a `Derived` schema with `values=` columns at the
aggregate-driven dtype (`count` / `nunique` → int64; `mean` /
`std` / `var` / `median` → float64; `sum` / `min` / `max` /
`first` / `last` → preserve receiver column type; default → mean
→ float64). Multi-values + `columns=` correctly falls through to
Unknown. FIRST observable aggregate-semantics-informed schema
inference in pykrete; sets the convention for v1.14+
groupby.agg. On the CI side, `release-gate.yml` no longer
triggers on `pull_request` events (only `push: release/v*` +
`workflow_dispatch`); branch-protection now requires
`release-gate-check`, which sits in "Expected — Waiting" state
on PRs until a dispatched run reports SUCCESS. The
backtick-preservation tripwire ships in
`trust-claim-sweep-checklist.sh`, closing the 2-cycle
backtick-strip regression at PR-G v1.11 + v1.12. Cross-codebase
coverage: 279 → 289 probes across 148 fixtures from 17 donors.
No new D-codes, no new annotation forms; SemVer-minor under the
`tighteningDiagnostics` policy and the established
alias-report-style JSON-additive policy. Cycle-close minor bump
aligns the extension with the v1.13.0 tag per the version-guard
policy.

## 0.10.0

Tracks the v1.12.0 pykrete release — **closing the v1.11
calendared GITHUB_TOKEN promise** and shipping **D0080
returnTypeMismatch cross-codebase trust coverage** (the longest-
standing trust gap since v1.6). The bundled `pykrete` and
`pykrete-lsp` binaries gain `pivot_table(aggfunc=)` literal-form
recognition against an 11-string allowlist (`sum` / `mean` /
`count` / `min` / `max` / `median` / `std` / `var` / `first` /
`last` / `nunique`). Recognition is informational: no diagnostic
fires; the result schema is unchanged. The recognition pass
primes v1.13+ aggfunc-driven inference. Multi-line ack-marker
rationale block lands per spec §6.1.4: `# pykrete:
ack-deprecation` (shape b) now extends acknowledgement to the
entire contiguous comment block above the anchor — a behavioral
change versus v1.10, which reported `pending` when the marker
sat on a non-matching comment line above the `def`. Adopters
that depended on the v1.10 strict-single-line semantic should
update. On the CI side, the v1.11 calendared GITHUB_TOKEN
promise closes: the auto-label workflow now dispatches
`release-gate.yml` via the `actions.createWorkflowDispatch` API,
bypassing GitHub's GITHUB_TOKEN cross-workflow no-trigger rule
so the release-gate fires non-skipped on labeled PR events end-
to-end. Release-gate runner perf improves materially: `cargo
test --release --workspace` is memoized via the
`PYKRETE_TESTS_COUNT_FILE` env var (gate step reads the memoized
count rather than re-invoking), dropping total release-gate
cold-cache runtime from ~70min to ~35min. Cross-codebase
coverage: 271 → 279 probes across 138 fixtures from 17 donors.
No new D-codes, no new annotation forms; SemVer-minor under the
`tighteningDiagnostics` policy and the established alias-report-
style JSON-additive policy. Cycle-close minor bump aligns the
extension with the v1.12.0 tag per the version-guard policy.

## 0.9.0

Tracks the v1.11.0 pykrete release — **closing the v1.10 D0091
PR-D1 cross-codebase carve-out** and shipping the pandas
`unstack` literal-form arm + the audit-tooling block. The bundled
`pykrete` and `pykrete-lsp` binaries gain a NEW pandas inference
arm for `df.unstack(level=, fill_value=)` (mirror of v1.10
`stack`, continuing the one-reshape-arm-per-cycle cadence from
v1.6 `pivot_table` and v1.7 `melt`): receiver-dialect-gated to
`PandasFrame[X]`, literal `level=` (single string, list / tuple of
strings) validates against the receiver schema and fires D0030 on
a typo with a *did you mean*; int / int-list / `None` /
non-literal forms fall through to Unknown. The v1.10 PR-D1 carve-
out closes upstream: cross-codebase property probes for the 8 new
v1.10 D0091 properties (`na`, `write`, `writeStream`,
`storageLevel`, `index`, `values`, `shape`, `T`) ship in pykrete-
tests PR-P1 #39 — every property the v1.10 checker added now has
a deliberately-broken cross-codebase fixture verifying it actually
fires. The release also lands the **audit-tooling block** closing
5 cycles of v1.10 retro rules: `trust-claim-sweep-checklist.sh`
(docs-vs-history sweep gate with a 17-test self-test suite),
`changelog-cite-check.sh` (resolves CHANGELOG `path:LINE` cites
against the working tree), and `auto-label-release-pr.yml`
(auto-applies `release-ready` label to release-PR branches). Devs
run the sweep checklist locally before opening PR-F; CI-side
release-gate label-trigger wiring stays tracked for v1.12 (the
default GITHUB_TOKEN doesn't fire downstream workflows on label
changes). LSP test tempdir-per-test isolation lands (sentinel
`pykrete.json` boundary anchored per-test, closing the v1.10
probe-density audit flake under parallel test execution). Walker
polish: mixed-indent + decorator-with-comment edges + tab/space
test + counter-semantics comment. The cross-codebase suite
matures further: 261 → 271 probes across 130 fixtures from 17
donors. No new D-codes, no new annotation forms; SemVer-minor
under the `tighteningDiagnostics` policy and the established
alias-report-style JSON-additive policy. Cycle-close minor bump
aligns the extension with the v1.11.0 tag per the version-guard
contract. See the
[main CHANGELOG](../../CHANGELOG.md#1110---2026-06-17) for details.


## 0.8.0

Tracks the v1.10.0 pykrete release — **`pykrete check
--deprecation-report --snapshot=<path>` makes v2.0 migration
archivable in CI**. The v2 envelope is written to disk via atomic
tempfile-plus-rename (nanosecond-suffixed temp name to avoid
concurrent-writer collision, cleanup-on-error guard across every
error path), so CI can persist a prior-report cache and diff
between releases — v1.8 made migration measurable; v1.9 made it
plannable; v1.10 makes it archivable. **`--fail-on-nonempty`**
exits non-zero when the envelope's `sites` array is non-empty,
replacing the `jq '.sites | length' | test ... -eq 0` boilerplate
adopters were writing by hand; compatible with `--ack` (gates
only on the filtered cohort). **D0091 surface completion**: the
four remaining Spark-direction discriminator properties (`na`,
`write`, `writeStream`, `storageLevel`) and the four
pandas-direction inherited properties (`index`, `values`, `shape`,
`T`) close the v1.9 spark-I1 / spark-I2 firing-site coverage gap
on `Expr::Attribute` — `pdf.na`, `pdf.write`, `sdf.index`,
`sdf.shape` and the rest now fire D0091 via the v1.9 bare-
attribute path. **Pandas `df.stack(level=, dropna=)` literal-
form** lands as a NEW inference arm (continuing the one-reshape-
arm-per-cycle cadence from v1.6 `pivot_table` and v1.7 `melt`);
receiver-dialect-gated to fire only on `PandasFrame[X]` receivers
because Spark's `stack` is a column-free-function
(`pyspark.sql.functions.stack`), not a DataFrame method.
**v1.9 architecture audit-debt closes structurally**: ack-marker
multi-line signature support (the v1.9 walker silently skipped
past `def foo(` when the signature ran onto a continuation line;
v1.10 lands an indentation-aware walker + decorator-stack skip),
property / method tripwire (build-time invariant pinning
`SPARK_DISCRIMINATOR_PROPERTIES ⊂ SPARK_DISCRIMINATORS`; mirror
for `PANDAS_INHERITED_PROPERTIES ⊂ PANDAS_ONLY_SIGNALS`),
release-gate CI workflow, and CHANGELOG grep gate v3 prose
number scan. **§9.2 centralized version bump** trialed in v1.9
is promoted to standing practice — zero rebase-ladder collisions
across the Wave 1 PRs was the SUCCESS signal. The cross-codebase
suite holds at the matured cadence: 6 new D0091 strict-mode /
bare-attribute / shape-changes probes on
`mlflow` / `dbt-spark` / `pandera` / `delta` (pykrete-tests
PR-P1 #34) plus the seaborn `stack(level=)` arm (pykrete-tests
#35). v1.10 PR-D1's 8 new D0091 properties are unit-test-
covered at v1.10.0; cross-codebase fixture probes filed for
v1.11.


## 0.7.0

Tracks the v1.9.0 pykrete release — **`pykrete check
--deprecation-report` makes v2.0 migration plannable**. The
envelope bumps to `deprecationReportVersion: "2"` with per-site
`migrationStatus` (`pending` / `acknowledged`) driven by a
`# pykrete: ack-deprecation` comment marker on the line above the
alias annotation — site-level opt-in, no JSON edit, no separate
state file. A new `--ack=<pending|acknowledged>` filter flag
narrows the envelope to one cohort for CI gating:
`pykrete check --deprecation-report --ack=pending src/` exits
non-zero with the unacked-site inventory; `--ack=acknowledged`
runs the inverse for "did anything regress" checks. The envelope
deliberately ships **without** `targetVersion` / `removalVersion`
/ `shipDate` — pykrete tracks per-site migration progress; the
user picks the v2.0 ship date. **D0091 matures**: strict-mode
escalation lands (warning → error under `"typeCheckingMode":
"strict"`, mirroring the v1.6 D0090 precedent); a suggestion
drift guard pins the cross-dialect suggestion table at build
time so adding a pair on one side without the other fails the
build; `CrossDialectSuggestion` gains a `shape_changes: bool`
field — pairs with asymmetric call-site shape
(`withColumnRenamed` → `rename`, `assign` → `withColumn`) append
"— note arg shape differs" to the suggestion text. A NEW
bare-attribute inference arm on `Expr::Attribute` catches
`pdf.rdd`, `sdf.loc`, `pdf.iloc`, `sdf.toPandas` (bare, no call)
and the rest of the cross-dialect attribute surface that the
v1.8 `Expr::Call` path missed — new property tables
`SPARK_DISCRIMINATOR_PROPERTIES` (3 entries) +
`PANDAS_INHERITED_PROPERTIES` (4 entries) drive the gate. The
v1.8 `build.rs`-generated `PANDAS_INHERITED_ARM_METHODS`
inventory tripwire is now backed by CI-running tests via the
extracted `build_helpers.rs` module — closes the v1.8 retro
rule that the inline `mod tests` block shipped without actually
executing. The CHANGELOG grep gate v2 ships a `text-numeric`
fenced-block label that live-verifies numeric trust-claims
(probes / fixtures / tests / donors) against live extracts.
Cross-codebase probe coverage adds 2 D0091 negative probes
(pandera + delta) via pykrete-tests PR-P1 #32 — 253 → 255
probes. The v1.9 cycle also trial-ran centralized version
bumping per spec §9.2 amendment: PR-F is the only commit that
bumps `Cargo.toml` / `package.json`; per-PR devs deferred to the
release PR via a `.github/centralized-bump-cycle.marker` file
honored by the extension-version-guard workflow.

## 0.6.0

Tracks the v1.8.0 pykrete release — **`pykrete check --deprecation-report`
makes the v2.0 migration measurable**. A new JSON envelope
(`deprecationReportVersion: "1"`) inventories every D0090-firing
site with its adjudicated dialect and suggested rewrite — drop it
into CI to gate v2.0 readiness without re-parsing diagnostic text.
The envelope reuses the v1.5 `--report-aliases` plumbing; the two
flags are mutually exclusive. D0090's message text is amended in
lockstep: drops the date-committal "and will be removed in pykrete
v2.0" framing for the softer "slated for removal in a future
pykrete v2.0" (no ship-date commitment) and names the new
`--deprecation-report` flag inline so users encountering D0090 in
CI output have a one-command path to the migration surface. The
new **`D0091 crossDialectMethodMismatch`** warning fires when a
pandas-only method is called on a Spark receiver
(`pdf.withColumn(...)`, `pdf.selectExpr(...)`) or a Spark-only
method on a pandas receiver (`sdf.assign(...)`, `sdf.merge(...)`),
with a *use `.x(...)` instead* suggestion for the high-traffic
pairs (`withColumn` ↔ `assign`, `withColumnRenamed` ↔ `rename`,
`selectExpr` → `eval`, `toPandas` → `copy`; `groupby` → `groupBy`,
`merge` → `join`). The suggestion is exposed via the LSP
`Diagnostic.suggestion` slot so the editor lights up a quick-fix
on the bulb action. Warning-only this cycle; strict-mode
escalation deferred to v1.9 because the back-compat surface is
genuinely larger than D0090's was. Carve-outs: deprecated
`DataFrame[X]` alias receivers skip the gate (avoid double-warning
with D0090); `pivot` and `melt` on Spark receivers don't fire
(Spark exposes legitimate same-spelled `groupBy(...).pivot(...)`
and 3.4+ positional `melt` surfaces). Two audit-class closures
land structurally: `build.rs` auto-generates the
`PANDAS_INHERITED_ARM_METHODS` inventory from `expr.rs` source
(the hand-maintained list becomes a tripwire — within-cycle drift
is structurally impossible); `scripts/changelog-grep.sh` is a new
CI gate that grep-anchors every CHANGELOG-fenced binary string
(`stderr` / `stdout` / `text` labels) against
`crates/pykrete/src/`. D0073 / D0083 cross-codebase probe coverage
added via pykrete-tests PR-P1 #30 (4 new negative probes — 249 →
253). No new annotation forms; SemVer-minor under the
`tighteningDiagnostics` policy and the established alias-report-
style JSON-additive policy. Cycle-close minor bump aligns the
extension with the v1.8.0 tag per the version-guard contract. See
the [main CHANGELOG](../../CHANGELOG.md#180---2026-06-16) for
details.

## 0.5.0

Tracks the v1.7.0 pykrete release — migrator UX hardening (`pykrete
migrate` defaults to `--check`; `--apply` opts into the in-place
rewrite) + pandas `df.melt(...)` literal-form column checking + the
v1.6 architecture-audit Important #3 closure via a shared
`dialect_signals` module + the Spark-D1 audit closure (14 new
`SPARK_DISCRIMINATORS`). The bundled `pykrete migrate src/` now
runs check-mode by default: it walks every `.pyk` file, applies
call-graph dialect adjudication, and prints per-site verdicts to
stdout (exit 1 if any site needs attention, 0 otherwise). To
rewrite in place, pass `--apply`. A first-run on v1.7 with no flag
emits a one-line stderr warning so adopters discover the change
without reading release notes — any CI invocation that ran
`pykrete migrate src/` and expected an in-place rewrite needs to
switch to `pykrete migrate --apply src/`. Pandas
`df.melt(id_vars=, value_vars=, var_name=, value_name=)`
literal-form column checking ships as the v1.7 reshape downpayment:
string-literal arguments and list-of-literals shapes resolve
against `PandasFrame[X]`'s schema, firing D0030 on a typo with a
*did you mean*. The pandas dispatch is gated on
`receiver_is_pandas_inherited` so the existing Spark
`melt` / `unpivot` arm's behavior on `SparkFrame[X]` receivers is
unchanged. Internal: the v1.6 parallel `PANDAS_ONLY_SIGNALS` /
`PANDAS_INHERITED_ARMS` lists collapse into a shared
`dialect_signals` module with a CI-guard test pinning the `expr.rs`
pandas-arm methods to the shared list. 14 Spark-only methods
(`selectExpr`, `freqItems`, `approxQuantile`, `crosstab`,
`colRegex`, `summary`, `mapInPandas`, `mapInArrow`, `writeTo`,
`writeStream`, `unpivot`, `rdd`, `isStreaming`, `sparkSession`) get
added to `SPARK_DISCRIMINATORS` — `corr` / `cov` deliberately
excluded for pandas collision risk. `pykrete migrate` parse-error
skips now surface on stderr; CRLF marker normalization lands for
Windows source files. No new D-codes, no new annotation forms;
SemVer-minor under the `tighteningDiagnostics` policy.
Cycle-close minor bump aligns the extension with the v1.7.0 tag
per the version-guard contract. See the
[main CHANGELOG](../../CHANGELOG.md#170---2026-06-15) for details.

## 0.4.0

Tracks the v1.6.0 pykrete release — `pykrete migrate` (auto-rewriter
for `DataFrame[X]` → `SparkFrame[X]` / `PandasFrame[X]` with
call-graph dialect adjudication), paired atomically with D0090
strict-mode escalation (under `"typeCheckingMode": "strict"` the
warning lands as **error**, but the fix-button ships in the same
release so strict-mode projects on green v1.4.x/v1.5.x CI aren't
stranded). The bundled `pykrete` binary gains the new `migrate`
subcommand with three modes: `pykrete migrate src/` rewrites in
place (atomic per file, token-preserving); `pykrete migrate --check
src/` previews per-site verdicts to stdout (pipe-friendly, exit 1 if
any site needs attention); `pykrete migrate --diff src/` emits a
`patch -p1`-compatible unified diff. Ambiguous bindings — used with
both Spark-only and pandas-only methods — get an idempotent
`# pykrete: ambiguous` marker injected on the line above the
unchanged annotation; re-runs don't accumulate duplicates. The
`pykrete check --report-aliases` envelope's `resolvedDialect` field
now reports `"pandas"` and `"ambiguous"` in addition to `"spark"`
(v1.5 reported every site as `"spark"` because adjudication wasn't
yet wired). Pandas `pivot_table(index=, columns=, values=,
aggfunc=)` literal-form column checking ships as the v1.6 pandas
reshape downpayment. Two v1.5 deferrals close: `.take()` is now
dialect-gated (pandas `pdf.take([0, 2]).merge(...)` keeps tracking),
and the `pdf.loc[mask, "col"]` nested-arg D0030 FP on the row-mask
side closes. The audit-debt `cross_dialect_handoff_gate` recognizer
the v1.5 PR-A1/PR-A2 inference left as a "Keep in sync" comment is
extracted to a single shared site. No new D-codes, no new annotation
forms; SemVer-minor under the `tighteningDiagnostics` policy.
Version bump aligns the extension with the v1.6.0 cycle-close per
the version-guard contract. See the
[main CHANGELOG](../../CHANGELOG.md#160---2026-06-13) for details.

## 0.3.0

Tracks the v1.5.0 pykrete release — cross-dialect handoff between
Spark and pandas, plus deferred-promise closure. The bundled
`pykrete-lsp` now re-tags `SparkFrame[X]` to `PandasFrame[X]` across
`df.toPandas()`, re-tags `PandasFrame[Y]` back to `SparkFrame[Y]`
across `spark.createDataFrame(pdf)` when a `schema=` argument or a
typed call-arg resolves to a known schema, and the round-trip
`spark.createDataFrame(df.toPandas())` preserves the tag end-to-end.
Pandas `.head()` / `.tail()` / `.first()` are dialect-gated as
Spark-only terminals so pandas chains downstream of them keep
tracking; the v1.3 promise of `.loc[:, "col"]` literal-form lands;
two PR-F1-class sibling gates close (`column_name_arg` ungated arms +
`collect_col_refs` cross-DataFrame routing). The CLI gains a new
`pykrete check --report-aliases` flag that emits a JSON envelope of
every `DataFrame[X]` annotation site with its resolved dialect — so
projects can size the v2.0 migration scope before v1.6's
`pykrete migrate` ships. The LSP synthetic-pool gets a soft cap with
one-shot warning and saturation sentinel, closing the v1.4
architecture-audit I4 finding (the language server keeps running on
adversarial input instead of unbounded `Box::leak` growth). No new
D-codes, no new annotation forms; SemVer-minor under the
`tighteningDiagnostics` policy. Version bump aligns the extension
with the v1.5.0 cycle-close per the version-guard contract. See the
[main CHANGELOG](../../CHANGELOG.md#150---2026-06-09) for details.

## 0.2.47

Tracks the v1.4 PR-F1 pykrete checker fix — the Subscript-on-Name arm
in `infer_expr_type` (added in PR-A) is now gated on the receiver
being a DataFrame binding, mirroring the D0030 sibling arm in
`col_refs.rs`. Before the gate, a plain Python `bag = {"x": 1};
col("name") == bag["x"]` falsely fired D0082 `crossTypeComparison`
because `bag["x"]` was silently typed against the frame's `x` field.
Architecture-audit blocker B1 from the v1.4 pre-tag re-audit. No new
D-codes, no new annotation forms.
## 0.2.46

Tracks the v1.4.0 pykrete release. The bundled `pykrete-lsp` is
bit-identical to v0.2.43–0.2.45 — every checker change in the v1.4
cycle (PR-A Subscript-on-Name `infer_expr_type` arm, PR-B's three
bug closures, PR-D's config-discovery + canonical-name + D0090
spec amendment) already shipped to users via the 0.2.43 / 0.2.44 /
0.2.45 extension releases. This bump tracks the v1.4.0 tag and the
trust-claim migration across the docs surface (pandas-coverage
donor count 3 → 10 in pykrete-tests, with the new donors split
into 3 direct-dispatch + 4 canonical-fixture-only + 3 hybrid
classes). No new D-codes, no new annotation forms.

## 0.2.45

Tracks the v1.4 PR-D pykrete checker fix — `pykrete.json` config
discovery now walks from the input file's parent directory (falling
back to the working directory when no input resolves to a file path),
so `pykrete check /abs/path/to/project/foo.pyk` from any CWD picks
up the project's `pykrete.json`. LSP discovery was already file-
anchored via the project-root resolver; this aligns the CLI. No new
D-codes, no new annotation forms.

## 0.2.44

Tracks the v1.4 PR-B pykrete checker bug closures (spec §4) — three
PRE-EXISTING silent-pass regressions surfaced by v1.3 audits.
(1) Registry-call args (`util(df["typo"])` where `util(x: int)` has
no `DataFrame[X]` slot) are now walked unconditionally, so the
embedded column-typo fires D0030 instead of slipping past the §10
widening gate.
(2) Walrus receivers (`(pdf := build()).rename(...)`) now inherit
the assigned value's dialect, so pandas dispatch fires on
walrus-bound chains.
(3) `.transform(helper)` now threads the receiver's dialect into the
helper's body inference, so pandas-only operations inside the helper
(e.g., `.assign`) dispatch under the correct dialect and the inferred
return schema reaches downstream column references.
No new D-codes, no new annotation forms; SemVer-minor under the
`tighteningDiagnostics` policy.

## 0.2.43

Tracks the v1.4 PR-A pykrete checker change — `infer_expr_type` now
recognizes `df["x"]` (Subscript-on-Name with string-literal slice) as
a scalar column reference, mirroring how `col("x")` is resolved. This
closes the v1.3 gap where pandas arithmetic-on-string expressions
inside `df.assign(__probe=df["x"] + 1)` produced no D0081 warning even
under `typeCheckingMode: "strict"`. No new D-codes, no new annotation
forms; this is SemVer-minor under the `tighteningDiagnostics` policy.

## 0.2.42

Tracks the v1.3.0 pykrete release — pandas dialect support. The
bundled `pykrete-lsp` now recognizes the `PandasFrame[X]` annotation
form alongside `SparkFrame[X]`, dispatches the six pandas operations
(`df[col_list]` / `df[mask]` / `df["new"] = expr` / `df.drop` /
`df.merge` / `df.rename`) through pandas-aware check sites, fires the
new `D0090 deprecatedDataFrameAlias` warning on `DataFrame[X]` (which
is removed in v2.0), and widens `D0030` to bare `df["typo"]`
subscripts in non-method contexts on both `SparkFrame[X]` and
`PandasFrame[X]`. Hover and completion surface the dialect tag.
Quick-fix on D0090 rewrites `DataFrame[X]` to `SparkFrame[X]`. Tracks
v1.3.0: 149 schema-tracking probes across 59 fixtures from 10 real
codebases now verify pandas check-site coverage in addition to
PySpark. See the
[main CHANGELOG](../../CHANGELOG.md#130---2026-06-03) for details.

## 0.2.39

Tracks the v1.2.0 pykrete release — trust-system extension, no
checker behavior change. The pykrete-tests cross-codebase suite
grows to 130 schema-tracking probes (113 positive + 17 negative)
across 47 fixtures from 10 donors, with new `PROBE-TYPE-IS`
type-tracking coverage in 3 donors (quinn, mlflow, python-deequ)
scoped to D0081 via a fixed scope-binding synth shape. The
bundled `pykrete-lsp` binary is **byte-identical to 0.2.38**; this
extension bump exists to track the upstream v1.2.0 release per the
version-guard contract that keeps the marketplace version aligned
with the pykrete-core release line. See the
[main CHANGELOG](../../CHANGELOG.md#120---2026-06-02) for details.

## 0.2.38

Tracks the v1.1.0 pykrete release — enum-constraint check sites land.
The bundled `pykrete-lsp` now emits `D0084 enumValueMismatch` on
off-vocabulary string literals at every site we check: `==` / `!=`
against an enum-typed column, `.isin(...)`, `.fillna({...})`,
`withColumn("col", lit(...))`, `F.expr("col = 'lit'")` (and the SQL
`IN (...)` form), plus the branch-form expressions
`F.coalesce` / `F.when(...).otherwise(...)` / `F.nvl` / `F.ifnull` /
`F.nullif` when their output flows into an enum-typed sink. Quick-fix
suggestions follow the same Levenshtein routine D0030 already uses for
column-name typos. See the
[main CHANGELOG](../../CHANGELOG.md#110---2026-06-02) for details.

## 0.2.37

Tracks the v1.1.0 pykrete release — enum-constraint parser and type
plumbing. The bundled `pykrete-lsp` recognizes the new
`enum["v1", "v2", ...]` atomic type in `Schema` class bodies and
threads the vocabulary through hover, completion, and the
`DataFrame[X]` schema surface so editors show enum-typed columns
alongside the rest of the type vocabulary. No `D0084` emission in this
build — the check sites land in 0.2.38. See the
[main CHANGELOG](../../CHANGELOG.md#110---2026-06-02) for details.

## 0.2.36

Internal test hardening for the bundled `pykrete-lsp` — the integration
test suite now panics loudly when a `pykrete.json` from outside the
test's temp dir would be picked up by accident, so test runs against
real projects can't silently load the wrong config. No editor-visible
behaviour change; the LSP binary is unchanged from 0.2.35.

## 0.2.35

Tracks the v0.1.40 pykrete release — final pre-v1.0.0 docs-only
release wiring the 10-donor / 32-fixture cross-codebase suite into the
main README's "Reliability and trust" section now that every fixture
emits zero diagnostics. No LSP behaviour changes; the bundled
`pykrete-lsp` is unchanged from 0.2.34. See the
[main CHANGELOG](../../CHANGELOG.md#0140---2026-05-31) for details.

## 0.2.34

Tracks the v0.1.39 pykrete release — cross-codebase false-positive
sweep. Closes the last seven false positives surfaced by the v0.1.38
cross-codebase suite: `df.drop(*cols)` and `withColumnsRenamed({…})`
now tolerate missing names (Spark-design behaviour), backtick-quoted
column refs resolve correctly, transient `.alias()` rename helpers
no longer false-fire, `F.posexplode(arr).alias("p", "v")` and
`F.explode(F.map(...)).alias("k", "v")` are recognized as dual-column,
the strict-mode atomic vocabulary accepts `float`, and `getField` on
opaque structs degrades cleanly. See the
[main CHANGELOG](../../CHANGELOG.md#0139---2026-05-31) for details.

## 0.2.33

Skipped — pykrete v0.1.38 was a pykrete-tests-only release wiring in
the cross-codebase goldens, with no extension-facing changes. The
next published extension is 0.2.34 (tracking pykrete v0.1.39).

## 0.2.32

Tracks the v0.1.37 pykrete release — final pre-v1.0.0 polish. Two
false-positive blockers fixed: aliased-DataFrame qualified column
refs (`L = df.alias("L"); col("L.region")`) no longer false-flag on
joins, and `unionByName(other, allowMissingColumns=True)` no longer
fires D0040 on schema-evolution merges. The expression-level walker
now descends into compound expressions so typos inside `if
df.select("typo").count() > 0:` and similar forms are caught.
Transform-input-mismatch gets its own diagnostic code (D0073,
`transformInputMismatch`) instead of inheriting D0070's
`unresolvedImport` name. The malformed-`pykrete.json` warning stops
re-firing every 30 seconds — it now only re-emits when the file's
mtime moves. See the
[main CHANGELOG](../../CHANGELOG.md#0137---2026-05-31) for details.

## 0.2.31

Tracks the v0.1.34 pykrete release — docs honesty pass and a new
"Reliability and trust" README section. No LSP behaviour changes; the
bundled `pykrete-lsp` is unchanged from 0.2.30. See the
[main CHANGELOG](../../CHANGELOG.md#0134---2026-05-31) for details.

## 0.2.30

Tracks the v0.1.33 pykrete release — `pykrete check --format json`
ships, and the per-D-code diagnostic snapshot suite pins every error
message as a reviewable artifact. No editor-visible behaviour change;
the LSP gains the JSON-output stability contract documented in
production-readiness. See the
[main CHANGELOG](../../CHANGELOG.md#0133---2026-05-31) for details.

## 0.2.29

Tracks the v0.1.32 pykrete release — architecture cleanups pass.
Editor-visible signal: pykrete now emits a `window/showMessage` warning
plus a `window/logMessage` detail whenever a `pykrete.json` config
file fails to parse, instead of silently falling back to defaults.
LSP startup is also faster on hosts where the bundled Python engine
search hits an unresponsive PATH candidate. See the
[main CHANGELOG](../../CHANGELOG.md#0132---2026-05-31) for details.

## 0.2.28

Tracks the v0.1.31 pykrete release — internal refactor of the PySpark
operations checker (single 6,000-line file split into nine sibling
modules along its existing section banners). No behaviour change. See
the [main CHANGELOG](../../CHANGELOG.md#0131---2026-05-30) for details.

## 0.2.23 – 0.2.27

Tracked the v0.1.26 → v0.1.30 pykrete releases — Spark coverage
closures (decimal / byte / short / binary atomic types, `melt` /
`unpivot` reconciliation, `drop_duplicates` / `pivot` / `sampleBy` /
`observe`, expression-form join keys + `fillna` dict keys,
control-flow descent + `groupBy` aggregate schema preservation) and
playground polish. The bundled `pykrete-lsp` followed each pykrete
release. See the [main CHANGELOG](../../CHANGELOG.md) for the
per-release breakdown.

## 0.2.22

Marketplace metadata refresh tracking the v0.1.25 docs-only release
(playground tokenizer polish). The bundled `pykrete-lsp` is unchanged
from 0.2.21.

## 0.2.14 – 0.2.21

Tracked the v0.1.16 → v0.1.24 pykrete releases — playground (wasm
analyzer surface + Monaco integration), static PySpark symbol layer
for hover / completion / go-to-definition on `sales.select` / `F.sum`
etc., per-platform `.vsix` packaging refinements, CHANGELOG and docs
maintenance. The bundled `pykrete-lsp` followed each pykrete release.
See the [main CHANGELOG](../../CHANGELOG.md) for the per-release
breakdown.

## 0.2.13

Marketplace listing refresh — README quick-fixes section tightened to
specify the D0030 *did you mean* scope. No LSP behavior changes; the
bundled `pykrete-lsp` is unchanged from 0.2.12.

## 0.2.12

Tracks the v0.1.15 pykrete-lsp release — generic-inference extensions
complete. Multi-TypeVar signatures, nested generic shapes
(`List[DataSource[T]]`, etc.), chained class-method calls preserving
class identity through self-returning intermediaries, and `type[T]`
argument binding. See the
[main CHANGELOG](../../CHANGELOG.md#0115) for the breakdown.

## 0.2.11

Tracks the v0.1.14 pykrete-lsp release — three Spark coverage gaps
closed. `F.when` / `F.otherwise` result-type inference; `F.struct` /
`F.named_struct` schema construction; date/time first-arg column
checking across ten functions; array higher-order function recognizers
(`F.transform`, `F.filter`, `F.aggregate`, `F.exists`, `F.forall`);
`melt` / `unpivot` output-schema modeling. See the
[main CHANGELOG](../../CHANGELOG.md#0114) for the breakdown.

## 0.2.10

Tracks the v0.1.13 pykrete-lsp release — Column method-chain
recognition (`.isNull` / `.isin` / `.between` / `.getField` /
`.withField` / `.dropFields` and friends) and
`createOrReplaceTempView` + `spark.sql` resolution within a file.
Marketplace listing fix: README image URLs absolutized so the
screenshots render correctly on the Visual Studio Marketplace and
Open VSX. See the
[main CHANGELOG](../../CHANGELOG.md#0113) for the breakdown.

## 0.2.9

Tracks the v0.1.12 pykrete-lsp release — chain survival pass closing
three headline gaps. `spark.read.<format>(path)` and
`spark.table(name)` recognized as opaque sources (re-anchor with
`.cast(DataFrame[X])` or a typed variable annotation);
`intersect` / `intersectAll` / `subtract` / `exceptAll` joined `union`
as recognized set operations sharing the `D0040` schema-mismatch
check; `F.broadcast(df)` treated as pass-through;
nine terminal methods (`count` / `collect` / `show` / `printSchema` /
`explain` / `first` / `take` / `head` / `tail`) recognized centrally.
Hover popups for schemas no longer stack a redundant basedpyright
echo; pykrete's content is authoritative. See the
[main CHANGELOG](../../CHANGELOG.md#0112) for the breakdown.

## 0.2.8

Tracks the v0.1.11 pykrete-lsp release — republishes the extension
after v0.1.10's marketplace publish was silently skipped because the
extension `package.json` version wasn't bumped. Ships alongside a
new extension-version-guard CI workflow that fails any future PR
which changes extension-impacting code without bumping
`package.json`, so this class of bug can't recur. Users who reinstall
now receive the v0.1.10 schema-hover format
(fenced Python class block with syntax highlighting). See the
[main CHANGELOG](../../CHANGELOG.md#0111) for the breakdown.

## 0.2.7

Tracks the v0.1.7 pykrete-lsp release. Adds **per-platform `.vsix`
bundling** — the matching `pykrete-lsp` binary is now included inside
the extension package on macOS arm64/x64, Linux x64, and Windows, so
most users install the extension and get the language server in one
step. A universal `.vsix` backs the long tail of other platforms. No
editor-side behavior changes from 0.2.6; the LSP gains `D0051
argumentColumnsMismatch` to close the function boundary on the input
side. See the [main CHANGELOG](../../CHANGELOG.md#017) for the
breakdown.

## 0.2.6

Tracks the v0.1.6 pykrete-lsp release — five real-codebase fixes from
the pykrete-tests integration loop. No editor-side behavior changes;
the LSP gains four new D0030 diagnostic shapes (`df["X"]` subscript,
GroupedData shortcut aggregates, `intersect`/`subtract`/`exceptAll`,
chained nested-field access, lowercase `groupby` alias). See the
[main CHANGELOG](../../CHANGELOG.md#016) for the breakdown.

## 0.2.5

- Temporary displayName change to bypass the Visual Studio Marketplace's
  post-deletion reservation on the name "Pykrete". The new displayName
  is "Pykrete — Static schema checking for Python" — library-agnostic
  so it doesn't pre-commit the brand to PySpark (pandas and polars are
  planned). Will be reverted to plain "Pykrete" once the reservation
  clears.

## 0.2.4

Cancelled before publishing — see 0.2.5.

## 0.2.3

- Republish under the `amirnaderi` publisher and drop the redundant
  `-vscode` suffix from the package name. New marketplace IDs:
  `amirnaderi.pykrete` on both the Visual Studio Marketplace and Open
  VSX. The old `pykrete.pykrete-vscode` listings will be removed.

## 0.2.2

- `.pyk` files now show the pykrete logo in the file explorer (used as
  the language icon when the active icon theme doesn't have one).

## 0.2.1

First marketplace release.

- Adds the marketplace icon and logo.
- Tracks the v0.1.x pykrete-lsp ABI.

Earlier 0.1.x and 0.2.0 development builds were distributed as local
`.vsix` files only.
