# Changelog

All notable changes to pykrete are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and pykrete adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/amirnaderi93/pykrete/compare/v0.1.22...HEAD
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
