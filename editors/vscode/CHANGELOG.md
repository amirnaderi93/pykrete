# Changelog

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
