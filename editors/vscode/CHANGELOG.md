# Changelog


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
