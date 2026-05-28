# Changelog

All notable changes to pykrete are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and pykrete adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/amirnaderi93/pykrete/compare/v0.1.15...HEAD
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
