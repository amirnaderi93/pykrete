# Changelog

All notable changes to pykrete are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and pykrete adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
fixture per D-code (17 codes total), runs the checker, and snapshots
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

[Unreleased]: https://github.com/amirnaderi93/pykrete/compare/v0.1.37...HEAD
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
