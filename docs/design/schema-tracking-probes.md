# Schema-tracking probes (v1.1 design tracker)

**Status**: planned for v1.1. Sibling of `literal-value-vocabulary.md` —
both are "make the schema layer earn its trust through positive
verification."
**Origin**: 2026-05-31 user direction during the post-v1.0.0 sprint:

> "I want to test that pykrete actually works correctly. For example if
> there is a `.select()`, I want to make sure that after that the schema
> is correctly filtered. And the same for every other scenario. Not just
> 'not raising false positives', but 'correctly tracking the schema
> changes'."

## Framing principle (load-bearing)

The v1.0 cross-codebase suite (10 donors, 32 fixtures) verifies one
thing: pykrete emits zero diagnostics on real working PySpark code. That
is necessary — false positives would torpedo trust — but it is not
sufficient. A checker that emits nothing on every input is vacuously
"passing"; it could be silently doing nothing.

Probes close that gap. A probe is a static assertion embedded in a
fixture that says one of three things:

1. **Name-resolution positive**: "after this transform, pykrete's
   tracked schema must accept this line cleanly — if a diagnostic fires
   here, we lost the schema."
2. **Name-resolution negative**: "after this transform, pykrete's
   tracked schema must reject `<column>` with `D0030` — if it doesn't,
   we silently widened."
3. **Type-shape positive**: "after this transform, the column we name
   has type `<T>` — if pykrete's tracked schema disagrees, we drifted
   the column types."

Probes are the structural answer to the trust pitch. The v1.0 README
says we catch schema bugs at edit time. Probes are how we prove it on
every CI run, against real PySpark donors, without trusting that the
absence of red is the presence of correctness.

**v1.1 scope is honestly narrow**: probes verify **name resolution +
type tracking**. Nullability tracking and exact-output-column-set
verification do not ship in v1.1 — they are filed as v1.2 work below
("Deferred to v1.2") because both demand pykrete-core D-codes that do
not exist today, and synthesizing them would either break the
"no pykrete-core release" headline cost claim or force awkward
encodings that the synthesizer review flagged as hand-wavy. v1.1 ships
the load-bearing pieces (does pykrete still know about column `region`
after a `.select()`? does it know `region` is a `string`?); v1.2
extends to nullability and exact-set output verification.

**Scope bright line, same as `literal-value-vocabulary.md`**: probes
verify *static* schema tracking — what pykrete knows at edit time
about column names and column types after a chain of operations.
Probes do not verify row values, runtime behaviour, or anything that
would require executing user code. Pykrete has no runtime; probes
inherit that constraint.

## Chosen design: inline comment markers (Option A, with one graft from Option B)

We adopt the inline `# PROBE-*` comment-marker design. The probe
vocabulary lives entirely in the cross-codebase repo (markers + harness
extension); pykrete's CLI, JSON schema, and stability surface are
unchanged. We graft one idea from the sibling-TOML proposal: a stable
human `id` field on each marker, so failure output and probe-coverage
reports can name probes by handle rather than by `<file>:<line>`.

### Why this design

1. **Zero coupling to pykrete's release cadence.** v1.0 froze
   `schemaVersion: "1"` for `--format json`. The inline-comment design
   ships the entire v1.1 probe capability without touching that
   contract, without a `pykrete-lsp` notification, and without a
   coordinated `PYKRETE_REF` bump in the cross-codebase CI. The probe
   vocabulary iterates on the cross-codebase repo's timeline.
2. **Co-located with the code it asserts about.** A reviewer reading a
   fixture sees `# PROBE-EXPECTS: D0030 on "product"` directly above
   the line that should fire. No tab-switching to a sibling file, no
   needle-anchor indirection, no mental join. This matches how pytest
   `# noqa:` and TypeScript `// @ts-expect-error` read.
3. **Donor-faithful (with one carve-out).** Markers are plain Python
   comments. They live in the same annotation lane that already adds
   `from pykrete import ...` and schema declarations to the donor file
   — i.e. the lane we already own as annotators, distinct from the
   upstream code we vendor verbatim. Negative probes that need
   deliberately-corrupted column references live in a separate
   `probes_negative/` tree per donor; see "Negative-probe seeding
   methodology" below.
4. **Failure mode is human-legible.** The harness prints `PROBE FAILURE:
   <fixture>:<line> id=<id>  expected: ...  actual: ...` — debuggable
   without `jq` over a 200-line golden diff.

We rejected the sibling-TOML design (Option B) primarily because it
splits the assertion from the code, and because the v1.1 scope does not
need needle-based anchor stability (the inline marker's "target line"
rule either matches loudly or fails loudly — there is no silent-wrong-
target failure mode, since EXPECTS mandates a D-code that must actually
fire). We rejected the helper-call design (Option C) because it expands
pykrete's public surface (`pykrete.testing`), bakes a new D-code
(`D0090`) into the stability contract, and demands ~8-10 days of
checker work for a capability the inline design delivers in ~5-7 days
with no checker change at all. Helper calls remain on the v1.2 table if
probes graduate into a dogfooding/teaching surface; for v1.1 they are
overkill.

### Probe syntax (final shape)

Markers are single-line comments at column 0. **Five kinds total,
split into three line-anchored markers + two file-scoped markers.**
Line-anchored markers always sit on the line *immediately above* the
target line (see Q10 for the exact "target line" resolution rule);
file-scoped markers conventionally live at the top of the annotated
file.

**Line-anchored markers (three kinds):**

```
# PROBE-EXPECTS: <D-code> [id=<handle>] [on "<span-text>"] [match /<regex>/[flags]] [-- <rationale>]
# PROBE-RESOLVES: [id=<handle>] [-- <rationale>]
# PROBE-TYPE-IS: <type-expr> on "<column>" [id=<handle>] [-- <rationale>]
```

**File-scoped markers (two kinds):**

```
# PROBE-FILE-CLEAN-OF: <D-code>[, <D-code>...] [id=<handle>] [-- <rationale>]
# PROBE-FILE-COUNT: <D-code> == <N> [id=<handle>] [-- <rationale>]
```

Total: **5 markers** (3 line-anchored + 2 file-scoped). This is the
honest v1.1 count after the synthesizer-translation review (see
"Deferred to v1.2" below for the two markers we dropped).

Concrete uses:

```python
# PROBE-FILE-CLEAN-OF: D0030, D0050
from pykrete import col, lit
from quinn.schemas import Order

def pipeline(orders: DataFrame[Order]) -> DataFrame[Order]:
    # PROBE-RESOLVES: id=quinn-select-region -- region survives narrow select
    df = orders.select("region", "amount")

    # PROBE-TYPE-IS: double on "amount" id=quinn-amount-type
    df2 = df

    # PROBE-EXPECTS: D0030 id=quinn-select-drops-product on "product"
    return df2.select(col("product"))
```

Rules:

- Markers are line-anchored to a "target line" for `PROBE-EXPECTS`,
  `PROBE-RESOLVES`, and `PROBE-TYPE-IS`. The target line is the
  **first source line of the next Python logical statement** following
  the comment (see Q10). `PROBE-FILE-*` are file-scoped and live at
  the top of the annotated file.
- **All optional arguments (`id=`, `on`, `match`, `-- rationale`) are
  free-floating, order-insensitive named slots.** They can appear in
  any order after the marker's required prefix. The parser keys on the
  slot keyword (`id=`, `on `, `match `, `-- `), not on position.
  Order-insensitivity is part of the stability contract (see "Probes
  stability surface" below).
- `id=<handle>` is optional but recommended. When absent, the harness
  synthesizes one from the fixture path + comment line. When present,
  it shows up in failure output and in the per-donor `PROBES.md` index.
  IDs are per-donor unique (see Q1); the verifier hard-fails on
  collision within a donor.
- `on "<text>"` pins the diagnostic span (for `PROBE-EXPECTS`) or the
  column name (for `PROBE-TYPE-IS`) by source-text match. For
  `PROBE-EXPECTS`, the span text is resolved by slicing the fixture
  text between the diagnostic's `(line, column)..(endLine, endColumn)`
  per Q12. **Dotted accessors are supported** for `PROBE-TYPE-IS`:
  `on "addr.city"` targets the `city` field of a struct column `addr`
  (target line must bind a struct-typed expression). Quoting follows
  shell rules: double-quoted, `\"` escapes.
- `match /<regex>/[flags]` pins the diagnostic message. The flavour is
  **Python `re`** (probes.py is stdlib Python; the regex never leaves
  Python). Supported flags: `i`, `m`, `s`, `x`. Mutually compatible
  with `on`. Caveat: message wording is NOT STABLE per v1.0's JSON
  contract (see "Probes stability surface"); prefer `on` to `match`.
- A line whose stripped form matches
  `^# PROBE-(EXPECTS|RESOLVES|TYPE-IS|FILE-CLEAN-OF|FILE-COUNT)\b`
  but does NOT match the grammar is a hard parse error (`probe parse
  error in <fixture>:<line>`). **Near-miss typo hint**: if the prefix
  after `# PROBE-` does not match the allowlist but is within
  Levenshtein distance ≤ 2 of an allowed kind (e.g. `PROBE-EXPECTSS`,
  `PROBE-RESOLVE`, `PROBE-TYP-IS`), the parser emits a `did you mean
  '<closest>'?` hint as a hard parse error. Anything beyond Levenshtein
  distance 2 from every allowlist entry is silently ignored (not a
  parse error) so that future upstream drift introducing an unrelated
  `# PROBE-FOO` comment in vendored code does not red-fail CI.
- A `PROBE-EXPECTS` whose `<D-code>` is not in `DIAGNOSTIC_CATALOG`
  fails at parse time. The catalog is supplied by the harness itself
  (see "Diagnostic catalog source" below); pykrete-core ships no new
  CLI surface for v1.1.
- `PROBE-TYPE-IS` `<type-expr>` is one of pykrete's atomic type names
  as they appear in `Schema` annotations (`int`, `long`, `double`,
  `string`, `boolean`, `date`, `timestamp`, `binary`, `byte`, `short`,
  `decimal`, `decimal(p, s)`). Composite types: `array<T>` and `Array[T]`
  are both accepted spellings for ergonomics (the verifier normalizes
  both to canonical form before comparison). **Parametric-type
  matching**: `decimal` (unparameterized) matches Spark's default
  `decimal(10, 0)`; `decimal(p, s)` requires an exact precision/scale
  match. Other composites (maps, struct fields beyond the dotted
  accessor case) are deferred to v1.2.

Explicitly excluded from the grammar (kept narrow on purpose):

- No multi-line / block markers. One line, one assertion.
- No comma-separated D-codes in `PROBE-EXPECTS`. Two diagnostics on the
  same line means two stacked `PROBE-EXPECTS` comments (pairing
  algorithm: see Q9).
- No `PROBE-NEXT-LINE` / `PROBE-PREV-LINE` toggles. Target is always
  the next-logical-statement rule from Q10.
- No trailing inline markers (`x = df.col("foo")  # PROBE-EXPECTS:...`).
  Formatters strip them; the target-line rule becomes ambiguous.

### Deferred to v1.2

Two marker kinds appeared in the round-2 grammar draft but are **dropped
from v1.1** after the synthesizer-translation review surfaced that
neither has a clean implementation path within v1.1's "no pykrete-core
release" cost envelope:

- **`PROBE-NULLABLE: <column> = (true|false)`** — pykrete-core has
  `D0083 nullabilityMismatch` already, but the synthesizer rewrite has
  no clean trigger from a single appended expression: nullability
  misuse usually surfaces across multiple operations (a null literal
  assigned to a non-nullable column, a `.dropna()` interaction, etc.)
  and a single-line synthesizer rewrite can't reliably reproduce it
  without false positives or negatives. Path to v1.2: either extend
  D0083's emission sites to cover a direct nullability-assertion
  pattern the synthesizer can target, or land first-class
  schema-trace output (option (b) above). Both are coordinated
  pykrete-core changes that trigger a vendored-catalog refresh.
- **`PROBE-OUTPUT-COLUMNS: [<col>, ...]`** — exact-set match against
  the tracked schema's full column list is awkward to encode via
  existing D-codes (`D0050 returnColumnsMismatch` checks against a
  declared schema, not an inline list literal; no single misuse
  triggers "your declared output set ≠ tracked output set" from
  arbitrary call-sites). Path to v1.2: pykrete-core adds a dedicated
  D-code (number TBD, since D0080 is already taken by
  `returnTypeMismatch`) — e.g. `D0090 outputColumnsAssertion`; the
  synthesizer rewrites `PROBE-OUTPUT-COLUMNS [a, b]` into a synthetic
  assertion that compares the tracked schema against the declared
  list and fires the new code on mismatch.

Both are tracked here so future-us does not relitigate; both require
coordinated pykrete-core releases. Filing them as v1.2 keeps the v1.1
cost claim ("no pykrete-core release needed") honest.

### Diagnostic catalog source (resolves B3)

The harness needs the set of valid D-codes at parse time so
`PROBE-EXPECTS: D0XXX` typos fail loudly. Three options were on the
table; we pick option (b):

- **(a)** Add `pykrete diagnostics --list --format json` subcommand to
  pykrete-core. **Rejected** — pykrete-core has only `check` and
  `transpile` today (`crates/pykrete/src/main.rs`). Adding a CLI
  subcommand creates a new stability surface, forces a coordinated
  pykrete-core release for v1.1, and falsifies the "no pykrete-core
  release needed" cost claim.
- **(b) Chosen**: the cross-codebase repo vendors a checked-in
  `scripts/diagnostic_catalog.json` (list of D-codes + names). A small
  CI job (`scripts/check_catalog_drift.py`) clones the pinned
  `PYKRETE_REF`, parses `crates/pykrete/src/diagnostics.rs` for the
  catalog, and fails CI if the vendored snapshot is out of date. The
  catalog file is regenerated on each `PYKRETE_REF` bump as a one-line
  PR. Cost: ~30 LOC of catalog-scraper plus a CI step; zero changes
  to pykrete-core.
- **(c)** Hardcode the catalog in `probes.py`. Rejected — same content
  as (b) but harder to keep in sync because it's buried in Python
  source.

This design honours the headline "no pykrete-core release needed" claim
without any handwaving about CLI surfaces that don't exist.

#### Diagnostic catalog schema

`scripts/diagnostic_catalog.json` has a fixed schema, versioned
independently of `probesSchemaVersion`. The exact shape:

```json
{
  "catalogSchemaVersion": "1",
  "pykreteSourceCommit": "<full SHA of the pykrete-core ref this was scraped from>",
  "diagnostics": [
    {"code": "D0030", "ruleName": "unknownColumn", "severity": "error", "stable": true},
    {"code": "D0050", "ruleName": "returnColumnsMismatch", "severity": "error", "stable": true},
    {"code": "D0083", "ruleName": "nullabilityMismatch", "severity": "error", "stable": true}
  ]
}
```

Field semantics:

- **`catalogSchemaVersion`** — STRING, REQUIRED. Follows the v1.0 JSON
  contract convention (semver-aligned major). `"1"` for v1.1 ship. A
  bump signals a breaking change to the catalog JSON shape itself
  (e.g. renaming `ruleName` to `name`); additive fields do not bump.
- **`pykreteSourceCommit`** — STRING, REQUIRED. Full 40-character SHA
  of the pykrete-core commit the catalog was scraped from. The drift
  checker compares this against `PYKRETE_REF` in CI; mismatch fails
  loudly with the two SHAs printed. Pins which pykrete-core version
  the vendored catalog was synced against, so a stale catalog cannot
  silently accept D-codes that don't exist in the running binary.
- **`diagnostics`** — ARRAY, REQUIRED.
  - **`code`** — STRING, REQUIRED. The D-code (e.g. `"D0030"`).
  - **`ruleName`** — STRING, REQUIRED. The rule's short name
    (e.g. `"unknownColumn"`).
  - **`severity`** — STRING, REQUIRED. One of `"error"`, `"warning"`,
    `"info"`. Used by the harness for sanity-checking probes
    (e.g. `PROBE-EXPECTS` against an info-severity code is fine; a
    `PROBE-FILE-CLEAN-OF` against an info-severity code is fine but
    surfaced in the per-donor report).
  - **`stable`** — BOOLEAN, REQUIRED. `true` means the D-code is part
    of the v1.0 stability commitment — i.e. probes targeting it are
    safe across pykrete-core minor releases. `false` means the code is
    experimental and may be renamed or removed without a major bump;
    probes targeting an unstable code emit an authoring warning at
    parse time (not a parse error). This field lets fixture authors
    make informed decisions about which codes are safe to lean on.

The schema is documented in `cross-codebase/README.md` so downstream
fixture authors can read the vendored catalog directly.

#### D-code lifecycle (out-of-bump drift policy)

The drift checker as initially proposed only catches *in-bump
staleness*: a `PYKRETE_REF` bump landed without regenerating the
catalog. It does NOT catch the more insidious case — pykrete-core
ships a minor release adding `D0079` (non-breaking per the v1.0
contract), pykrete-tests CI keeps running against the OLD vendored
catalog but a NEW pykrete-core binary, and a probe that *should*
expect D0079 passes vacuously because the harness has no way to know
the new code exists.

The fix is two-layered:

1. **Scheduled out-of-bump drift workflow** (cross-codebase repo).
   `.github/workflows/catalog-drift-watch.yml` runs weekly on a cron
   trigger. It clones `pykrete-core@main` (not the pinned
   `PYKRETE_REF`), scrapes its `DIAGNOSTIC_CATALOG`, and diffs against
   the vendored `diagnostic_catalog.json`. Any additions or removals
   open a pull request that carries the refreshed catalog (originally
   spec'd as a tracking issue; amended to PR-mode during pykrete-tests
   PR #5 because the refresh is mechanical and a one-step
   reviewable-and-mergeable PR is stronger trust signal than an issue
   that just documents drift). This catches the "main grew a D-code,
   our pin is behind" case before it festers into a bad release.

2. **SemVer policy on pykrete-core's side** (codified here as a
   commitment the pykrete-core release process honours):
   - **Adding a new D-code is non-breaking** in a minor release, but
     triggers a vendored-catalog refresh PR in cross-codebase within
     one minor cycle. The scheduled drift workflow surfaces the
     mismatch; cross-codebase reviewers cut the refresh PR.
   - **Renaming a D-code is breaking** — requires a major bump on
     pykrete-core. Probes naming the old code break loudly.
   - **Removing a D-code requires a deprecation cycle**: when
     pykrete-core decides D0030 is going away (replaced, merged into
     another rule, no longer applicable), it MUST stay emittable in a
     vendored-catalog-compatible response for one minor cycle. I.e.
     D0030 deprecated in pykrete-core v1.2.0 must still be emittable
     through v1.3.0; only in v1.4.0 can the code stop firing entirely
     (or a major bump removes it). This gives cross-codebase fixtures
     a release-cycle window to migrate `PROBE-EXPECTS: D0030` to the
     replacement code without a forced lockstep release.

This policy is restated in pykrete-core's `RELEASING.md` as part of
the v1.0 stability commitment (release-notes checklist item: "did
any D-code change kind? deprecations require the one-cycle window").
The cross-codebase repo's `check_catalog_drift.py` and the scheduled
workflow are the enforcement teeth.

### Golden format (unchanged)

The `.golden.json` format does not change. `--format json` output stays
exactly as v1.0 froze it: `{schemaVersion: "1", diagnostics, summary}`,
no new top-level `probes` array, no new per-diagnostic fields. Probes
are an assertion *layer* over the existing JSON, not a new payload in
it.

A fixture passes iff (a) the normalized JSON diff against
`.golden.json` is clean AND (b) every probe in the file is satisfied.
Both checks run on every CI invocation; both report independently.

For `PROBE-TYPE-IS`, the verifier needs the type pykrete tracks for
a named column at the target line. The `--format json` payload today
carries diagnostics but not the tracked schema. Two implementation
options:

- **(a) Synthesizer-style probe-to-diagnostic translation**: the
  harness rewrites `PROBE-TYPE-IS double on "amount"` into a synthetic
  line appended to a scratch copy of the fixture that triggers an
  existing type-mismatch D-code iff `amount`'s tracked type ≠ `double`.
  Candidate D-codes for the synthesizer to target (impl PR picks the
  cleanest one; all already exist in pykrete-core's stable catalog):
  - **D0082 `crossTypeComparison`** — fires on comparison between
    columns of incompatible types. Rewrite shape:
    `_ = (df.amount == lit(0.0))` triggers D0082 iff `df.amount`'s
    tracked type is not numeric-compatible with `double`.
  - **D0081 `nonNumericArithmetic`** — fires on arithmetic with a
    non-numeric column. Useful for the inverse case (probe expects
    `string`, synthesizer rewrites to arithmetic that fires D0081 iff
    the column IS numeric).
  - **D0080 `returnTypeMismatch`** — fires on a transform whose
    declared return schema disagrees with the inferred output schema.
    Useful when the column appears as a return-position expression.
  The impl PR picks per-type. Pure JSON-output assertion; zero
  pykrete-core change. Works for every atomic and parametric type in
  the v1.1 grammar; verified for the dotted-accessor case
  (`on "addr.city"`).
- **(b) Add a `--emit-schema-trace` CLI flag** to `pykrete check`
  emitting per-binding-site schema info on stderr or as a side-channel
  JSON file. First-class but requires a pykrete-core release and
  schema-stability discussion for the new payload.

**Chosen: (a) for v1.1**. The synthesizer rewrite has clean code
expansions using existing stable D-codes (D0080-D0082 cover the
expressible cases). The "no pykrete-core release" claim stays honest.
If the rewrite proves infeasible for a specific type-checking corner
case during seeding, the affected `PROBE-TYPE-IS` instances are
dropped from the v1.1 seeding pass (not the grammar) and re-attempted
in v1.2 alongside the deferred NULLABLE / OUTPUT-COLUMNS work.

Failing output (per fixture):

```
PROBE FAILURE: cross-codebase/quinn/annotated/quinn/functions.pyk
  comment line 86  target line 87  id=quinn-select-drops-product
    PROBE-EXPECTS D0030 on "product"
    expected: D0030 with span text "product"
    actual:   no diagnostic on line 87

  comment line 141 target line 142  id=quinn-withColumn-resolves
    PROBE-RESOLVES
    expected: no diagnostic on line 142
    actual:   D0030 [resolveColumn] "unknown column 'region'" at 142:18-142:24

  comment line N/A  file-scoped  PROBE-FILE-COUNT D0083 == 2
    expected: 2
    actual:   3 (lines 55, 89, 201)
```

Both the comment line (where the marker lives) and the target line
(where the assertion applies) are printed; ambiguity is intentionally
removed.

### CLI changes (none for pykrete-core)

`pykrete check --format json` is unchanged. `schemaVersion` stays
`"1"`. No new subcommand, no new flag, no new D-code, no new public
module on pykrete-core. `pykrete-lsp` and the VS Code extension see
nothing new.

One deferred convenience (v1.2 candidate, not v1.1 scope): a
`pykrete probes <file>` debug subcommand that parses markers and prints
them as a table for local authoring. If demand is real, it lives in
`scripts/probes.py` as a `--report` mode of the harness rather than as
pykrete-core surface.

### Runner changes (cross-codebase repo)

All changes live in the cross-codebase repo. Pykrete-core itself is
untouched for v1.1 probes.

1. **New: `scripts/probes.py`** (~300 LOC, stdlib-only, Python 3.10+;
   floor confirmed against cross-codebase repo CI matrix).
   - `extract(fixture_path) -> list[Probe]`: extracts via
     `tokenize.tokenize` filtered to COMMENT tokens (NOT a regex scan
     of file bytes — see Q11), regex-matches each comment against the
     marker grammar, attaches target line via the next-logical-
     statement rule (Q10), synthesizes IDs for un-tagged markers,
     returns typed records. Hard-fails on malformed markers and on
     unknown D-codes (catalog from `diagnostic_catalog.json`).
   - `verify(fixture_path, normalized_actual_json) -> list[ProbeFailure]`:
     - `EXPECTS`: scan `diagnostics[]` for `(file == fixture,
       line == target_line, code == expected_code)`. If `on "..."`
       present, slice fixture text by the diagnostic's
       `(line, column)..(endLine, endColumn)` per Q12 and compare.
       If `match /.../` present, regex the `message`. Stacked
       EXPECTS use the pairing algorithm from Q9.
     - `RESOLVES`: assert no diagnostic has `line == target_line` in
       the fixture.
     - `TYPE-IS`: implemented via the probe-to-diagnostic translation
       described in "Golden format" above (synthetic D0080-D0082 target
       on a scratch fixture copy). Individual probe instances drop out
       cleanly if the rewrite cannot encode them; the marker grammar
       stays.
     - `FILE-CLEAN-OF`: assert no diagnostic in the file carries any
       listed code.
     - `FILE-COUNT`: count diagnostics with that code in the file,
       compare to N.
   - Path normalization: both sides (probes.py target + diagnostics
     `file` field) reduce to `Path(p).resolve().relative_to(REPO_ROOT)
     .as_posix()` before equality (Q14). Mismatch is a hard error,
     not silent zero-match.
   - Exits 0 if all satisfied, 1 with the failure block otherwise.

2. **New: `scripts/diagnostic_catalog.json`** (vendored from
   pykrete-core; see "Diagnostic catalog source" above).

3. **New: `scripts/check_catalog_drift.py`** (~80 LOC). Clones pinned
   `PYKRETE_REF`, parses `crates/pykrete/src/diagnostics.rs`, fails CI
   if `diagnostic_catalog.json` is stale. Run in the cross-codebase
   workflow.

4. **Edit `scripts/golden.sh`** (~20 LOC).
   - After the existing `actual_norm=$(...)` capture in `check` mode:
     ```bash
     if ! python3 "$REPO_ROOT/scripts/probes.py" verify "$fixture" <<< "$actual_norm"; then
       fails=$((fails + 1))
     fi
     ```
   - In `generate` mode: run `probes.py --lint-only` (parse-check
     without verifying) so authoring errors fail the regen step.
   - New mode `golden.sh probes-report` emits per-donor `PROBES.md`.

5. **New: `tests/test_probes.py`** (~250-400 LOC, pytest). Realistic
   budget for: grammar parsing (all 5 marker kinds, all error paths
   including in-docstring suppression, the silent-skip rule for far-
   distance unknown `PROBE-*` prefixes, and the Levenshtein-≤-2 "did
   you mean" hint); span matching against canned JSON + synthetic
   fixture with UTF-8 multi-byte characters; ID synthesis + collision
   handling; path-normalization edge cases (relative vs absolute);
   stacked-EXPECTS pairing (Q9); target-line resolution on chained
   calls / decorated defs / blank lines (Q10); free-floating named-arg
   order-insensitivity (id= / on / match / -- can appear in any order).
   Runs as a pre-flight CI step before the golden suite (<2s).

6. **Edit `.github/workflows/cross-codebase.yml`**:
   - Add `Probe coverage guard` step. **For v1.1 ship: informational
     only — prints density numbers, does not fail the build.** The
     guard becomes release-blocking in v1.2 once we have one release-
     cycle of authoring experience and the seed curve stabilizes.
     This resolves the contradiction between PR-body / doc step 4 /
     doc Q8 (see Q16 for the grace-window policy).
   - Add `Catalog drift check` step (runs `check_catalog_drift.py`).
     Blocking from v1.1 ship — the catalog is the source of truth
     for `PROBE-EXPECTS` parse validation. Covers in-bump staleness
     (the `PYKRETE_REF` pin moved but the catalog wasn't regenerated).

7. **New: `.github/workflows/catalog-drift-watch.yml`** — scheduled
   weekly workflow that catches the *out-of-bump* drift case (pykrete-
   core `main` grew a new D-code, our pin is behind so CI hasn't
   noticed). Diffs `diagnostic_catalog.json` against a fresh scrape of
   `pykrete-core@main` and opens a refresh pull request on mismatch
   (PR carries the regenerated catalog; one-step reviewable + mergeable).
   Not release-blocking; it surfaces drift early so the next
   `PYKRETE_REF` bump comes pre-warned. See "D-code lifecycle" above.

8. **Per-donor `cross-codebase/<donor>/PROBES.md`** (auto-generated by
   `golden.sh probes-report`). Lists every probe with file, line, kind,
   id, expectation. Committed to the repo, regenerated by CI on each
   merge to keep diffs reviewable. Treated as informational — its
   format is not part of the marker stability contract (see "Probes
   stability surface" below).

## Probes stability surface

Once 32 fixtures encode the marker vocabulary, every word in the
grammar is a corpus-wide migration. This section parallels the v1.0
JSON output stability contract in
[`about/production-readiness.md`](../../docs-site/src/content/docs/about/production-readiness.md#json-output-stability-contract)
and defines the analogous commitments for probes.

### Probes schema version (single source of truth)

The probes layer carries an explicit version handle:
`probesSchemaVersion: "1"`. **Single source of truth: a module-level
constant `PROBES_SCHEMA_VERSION = "1"` at the top of
`scripts/probes.py`.** Every other location that references it —
generated `PROBES.md` headers, `cross-codebase/README.md`, the
per-donor `probes_negative/README.md`, harness failure output — reads
from `scripts.probes.PROBES_SCHEMA_VERSION` or, for non-Python
contexts, gets templated in by the `golden.sh probes-report`
generator. Hand-editing the version in any documentation file is an
authoring error; CI greps for stale literal `"probesSchemaVersion:
"<n>"` values and fails on mismatch.

The version bumps on breaking changes to the marker grammar (see the
"Marker grammar" subsection below for what counts as breaking).

**Marker grammar — STABLE.** Once the v1.1 spec lands, these names
are commitments:

- **Marker kind names — STABLE.** Renaming `PROBE-EXPECTS` →
  `PROBE-FIRES` (or similar) requires a `probesSchemaVersion` bump and
  a corpus-wide migration. Codified in `tests/test_probes.py`.
- **Argument syntax for existing kinds — STABLE.** Changing
  `on "text"` to `on:text` is breaking.
- **Optional-argument order — STABLE as order-insensitive.** The
  `id=`, `on`, `match`, and `-- rationale` slots are free-floating
  named arguments. The parser accepts them in any order; changing to
  positional-only or fixed-order is breaking. Tests in
  `test_probes.py` exercise every permutation of present slots.
- **Adding a new marker kind — NON-BREAKING.** New `PROBE-*` kinds
  (e.g. `PROBE-NULLABLE` or `PROBE-OUTPUT-COLUMNS` when they land in
  v1.2) are additive; `probesSchemaVersion` stays `"1"`.
- **Adding a new optional argument to an existing kind — NON-BREAKING.**
  As long as existing fixtures parse unchanged.
- **Removing a marker kind — BREAKING.** Requires a
  `probesSchemaVersion` bump.

**Verifier semantics — STABLE.** Pinning down once for all:

- **`PROBE-RESOLVES` semantics — STABLE.** "No diagnostic of any code
  fires on the target line." Tag/expression is documentation only; the
  verifier does not parse it. (Resolves B1: the verifier is
  precisely-defined and the marker has exactly one shape.)
- **`PROBE-EXPECTS` pairing — STABLE.** Bipartite-match-distinct
  (Q9); each probe matches a distinct diagnostic. Changing to first-
  match-wins is breaking.
- **`on "..."` column-unit — STABLE.** 1-indexed UTF-8 character
  units (Q12). Switching to bytes or UTF-16 is breaking and amends the
  v1.0 JSON contract (see Q12 follow-up).
- **Path normalization — STABLE.** Relative-to-`REPO_ROOT` POSIX form
  (Q14). Switching to absolute paths is breaking.

**Authoring API — CONTRACT for fixture authors.**

- `scripts/probes.py extract / verify` function signatures are stable
  within `probesSchemaVersion: "1"`. Fixture-author tooling can import
  them.
- `--lint-only` and `--report` modes are stable surface.

**Per-donor `PROBES.md` format — NOT STABLE.** It is informational; its
shape may evolve per release without a `probesSchemaVersion` bump.
Downstream tools that want machine-readable probe inventories should
call `scripts/probes.py extract` directly.

**Diagnostic message text in `match /.../` — NOT STABLE.** v1.0
explicitly says message wording can change in minor pykrete releases.
A `match` probe may break on any pykrete-core minor release. Authors
should prefer `on` over `match`; `match` exists for the rare case
where span text alone is ambiguous.

### Python dataclass shapes (impl-PR target)

Pinning the in-Python representation so the impl-PR review has a
concrete target. These shapes are part of the **authoring API
contract** (stable within `probesSchemaVersion: "1"`):

```python
from dataclasses import dataclass
from typing import Literal, Optional

ProbeKind = Literal[
    "EXPECTS",      # PROBE-EXPECTS
    "RESOLVES",     # PROBE-RESOLVES
    "TYPE-IS",      # PROBE-TYPE-IS
    "FILE-CLEAN-OF", # PROBE-FILE-CLEAN-OF
    "FILE-COUNT",   # PROBE-FILE-COUNT
]

@dataclass(frozen=True)
class Probe:
    kind: ProbeKind
    fixture_path: str            # POSIX, relative to REPO_ROOT
    comment_line: int            # 1-indexed; the line the # PROBE-... lives on
    target_line: Optional[int]   # 1-indexed; None for FILE-* kinds
    id: str                      # always set (synthesized if user omitted id=)
    rationale: Optional[str]     # the -- <rationale> slot, if present
    # Kind-specific payload (one of):
    expected_code: Optional[str]      # EXPECTS, FILE-CLEAN-OF (single code form), FILE-COUNT
    expected_codes: Optional[tuple[str, ...]]  # FILE-CLEAN-OF (multi-code form)
    expected_count: Optional[int]     # FILE-COUNT
    span_text: Optional[str]          # EXPECTS, TYPE-IS (on "..." slot)
    match_regex: Optional[str]        # EXPECTS only
    match_flags: Optional[str]        # EXPECTS only
    type_expr: Optional[str]          # TYPE-IS only

@dataclass(frozen=True)
class ProbeFailure:
    probe: Probe
    expected: str                # human-readable, e.g. 'D0030 with span text "product"'
    actual: str                  # human-readable, e.g. 'no diagnostic on line 87'
```

These types are importable from `scripts.probes`. Fixture-author
tooling and `tests/test_probes.py` consume them. Adding fields to
either dataclass is non-breaking if `@dataclass(frozen=True)` and the
new field has a default; renaming or removing a field is breaking
(bumps `probesSchemaVersion`).

## Negative-probe seeding methodology (resolves donor-faithful collision)

Round-1 review surfaced that `PROBE-EXPECTS: D0030 on "missing_col"`
requires either inventing a typo in donor code (collides with the
donor-faithful principle) or finding one already in upstream code
(limits coverage). Three options were on the table; we pick (a):

- **(a) Chosen**: add a third tree per donor:
  `cross-codebase/<donor>/probes_negative/`. These are pykrete fixtures
  built from the donor's schemas + a *small* synthetic transform that
  deliberately references columns not in the schema, with
  `PROBE-EXPECTS` markers asserting the expected diagnostic. The
  donor-faithful `upstream/` and `annotated/` trees stay byte-identical
  to upstream / annotation-only. Negative probes live in their own
  tree, clearly labelled, with their own README explaining that the
  code in `probes_negative/` is intentionally broken pykrete-fixture-
  shaped — not donor-faithful.
- **(b)** Allow inline negative probes only where the underlying donor
  code already has a real-world typo. Rejected — limits coverage to
  whatever the upstream maintainers happened to leave on the floor;
  doesn't let us cover diagnostic codes systematically.
- **(c)** Mark some donors positive-only and put all negative probes
  in a separate global tree. Rejected — divorces the synthetic negative
  fixtures from the donor's schema/idiom, which is what makes them
  trust-bearing.

Trade-off accepted by (a): a small amount of synthetic code per donor.
Mitigated by keeping each `probes_negative/<file>.pyk` short (~10-20
LOC) and labelling it clearly. Counted toward the per-donor probe
minimums separately from the annotated/ tree (informational metric for
v1.1; see Q16).

### `probes_negative/` harness contract

The `probes_negative/` tree is a first-class fixture lane. Pinning
its contract so impl-PR review has a clear target:

- **Discovery glob**: `cross-codebase/<donor>/probes_negative/**/*.pyk`.
  Discovered by `golden.sh check` the same way `annotated/` fixtures
  are; treated identically by the runner except for the per-fixture
  golden expectation.
- **`.golden.json` policy**: each `probes_negative/` fixture has a
  paired `.golden.json` whose `diagnostics[]` array is **NON-EMPTY**.
  This is the inverse of the `annotated/` contract (which expects
  `diagnostics: []`). A `probes_negative/` fixture whose golden is
  empty is a hard authoring error (the harness fails the regen step
  with `probes_negative fixture must emit at least one diagnostic`).
- **Catalog-drift coverage**: `probes_negative/` fixtures count
  toward the `check_catalog_drift.py` reachability check — every
  D-code referenced in the vendored catalog SHOULD have at least one
  `probes_negative/` fixture that exercises it. Coverage is
  informational in v1.1; becomes a target metric in v1.2 alongside
  the coverage-guard ratchet (Q16).
- **Naming convention**: `cross-codebase/<donor>/probes_negative/<topic>.pyk`
  where `<topic>` describes what's being verified (e.g.
  `select_drops_column.pyk`, `withColumn_typo.pyk`,
  `join_ambiguous.pyk`). The donor's `probes_negative/README.md`
  lists every file with a one-line summary.
- **No mixing with `annotated/`**: a fixture either lives entirely in
  `annotated/` (donor-faithful, expects zero diagnostics) or entirely
  in `probes_negative/` (synthetic, expects ≥1 diagnostic). No fixture
  straddles.

**v1.0 trust-claim language carve-out**: the v1.0 README's
"zero diagnostics on real PySpark code" line gets a one-line update
in the v1.1 release notes to: *"zero diagnostics on donor-faithful
`annotated/` fixtures; expected diagnostics on `probes_negative/`
fixtures."* The carve-out is honest — it's the same trust claim, just
specifying which tree is which. Without the carve-out, a reader could
misinterpret the v1.1 CI summary (which will show non-zero
diagnostics from `probes_negative/`) as a regression.

### Future tree stability

The per-donor tree layout (`upstream/`, `annotated/`,
`probes_negative/`) is a contract for v1.1, but it's **append-only,
not closed**. Adding a future tree (e.g. `probes_perf/` for
performance-regression fixtures, `probes_lsp/` for LSP-specific
behaviour) is a non-breaking change — existing fixtures and probes
stay valid. The harness's discovery loop is `glob` over a documented
list of tree names; adding a new name to the list does not migrate
or invalidate anything. This is filed here so the v1.2+ planning
process knows the door is open.

## Open design questions (settle before any code lands)

The first 8 questions stay from round 1; Q9-Q16 were surfaced by the
round-2 multi-lens review; Q17-Q19 were added in round 3 after the
synthesizer-translation review and the NULLABLE/OUTPUT-COLUMNS scope-
narrowing decision. All must be settled before the impl PR lands.
"Settled" here means "the spec or a follow-up PR has a written
decision, not just a recommendation."

**Load-bearing flagged with ★** — these encode commitments that 32
fixtures will materialize, so changing them later is a corpus-wide
migration. Q9, Q10, Q11, **Q12**, and Q14 are the load-bearing ones;
Q12 in particular got buried in earlier rewrites and is re-surfaced
here as the column-unit decision that drives every `on "..."` match.

1. **ID uniqueness scope.** Per-file or per-donor? Recommendation:
   per-donor (lets the per-donor `PROBES.md` use IDs as primary key).
   Failure output qualifies with the fixture path for global clarity.

2. **`PROBE-FILE-COUNT: D0030 == 0` vs `PROBE-FILE-CLEAN-OF: D0030`.**
   Both express the same constraint. Recommendation: allow both, prefer
   COUNT when the historical context is "there used to be N, now there
   should be 0" (the COUNT form documents the delta).

3. **Span-match Unicode/whitespace policy.** The cross-codebase corpus
   is upstream OSS PySpark — UTF-8 LF, no tabs. Recommendation: assume
   UTF-8 LF, fail loudly on CRLF or tab-expanded text in the spanned
   region. Document in `cross-codebase/README.md`. (Q12 separately pins
   the column-unit semantics.)

4. **Probe targets the last line of a file (no next-logical-
   statement).** Recommendation: parse-time hard error (`probe targets
   nonexistent statement after line N`).

5. **Stacked probes on the same target line.** Pairing algorithm is
   Q9 (newly surfaced); the doc-step-5 recommendation ("verifier ANDs
   them") is too informal — Q9 makes it precise.

6. **Donor file containing a pre-existing comment starting with
   `# PROBE`.** Recommendation: parse only the strict-allowlist
   prefixes (see syntax rules above); other `# PROBE-FOO` comments
   skip-not-error. Donor-sync script still greps for `^# PROBE-` and
   surfaces matches in the sync report so the annotator confirms
   intent. No silent CI red-fail from upstream drift.

7. **Multi-file probes.** A probe in `pipeline.pyk` cannot today
   reference schema defined in `schemas.pyk`. Recommendation: out of
   scope for v1.1. Today's `golden.sh` processes one fixture at a
   time; the only multi-file fixtures live in pykrete's insta catalog
   (D0071/D0072). Revisit in v1.2 when the first real cross-file
   cross-codebase fixture lands.

8. **Probe density threshold per release.** v1.1 ships with the
   coverage guard *informational only*. The guard becomes release-
   blocking in v1.2 once we have one release-cycle of authoring
   experience. v1.1 still tracks corpus-wide probe count as a CI
   summary metric; commits to "non-decreasing" via the ratchet in Q16.

9. ★ **Stacked-EXPECTS pairing algorithm.** When two `PROBE-EXPECTS`
   markers stack above one line and pykrete fires two D0030
   diagnostics, how do we pair probe ↔ diagnostic? Options:
   bipartite-match-distinct (each probe must match a distinct
   diagnostic; recommended), first-match-wins (order-dependent;
   brittle), each-against-the-set (would let one diagnostic satisfy
   two probes; loses precision). **Must settle before impl.**

10. ★ **Target-line resolution rule.** Literal next-line vs AST-resolved
    next logical statement; behaviour on decorators, multi-line
    chained calls, blank lines, comments. Recommendation: AST re-
    parse on the fixture; target line = first source line of the next
    logical statement after the comment. Blank-only and comment-only
    lines are skipped. Markers immediately above a decorator attach to
    the decorated def, not the decorator. **Must settle before impl
    because target_line is encoded in every probe.**

11. ★ **In-string probe extraction.** Regex scan over file bytes will
    match inside docstrings and triple-quoted SQL strings.
    Recommendation: use `tokenize.tokenize` and filter to COMMENT
    tokens only. Lock with a test case (docstring containing the
    string `# PROBE-EXPECTS: D0030` must NOT count as a probe).
    **Must settle before impl.**

12. ★ **Column-unit semantics for span match (load-bearing).** Today's renderer
    (`ruff_source_file`) uses UTF-8 character units; the v1.0 JSON
    contract does not pin the unit. Recommendation: 1-indexed UTF-8
    character units. Add a one-line amendment to
    `about/production-readiness.md` pinning this for the JSON
    contract too — it's the same column field — and bump no schema
    version (it's a clarification, not a change). **Must settle
    before impl; once 32 fixtures encode it via `on "..."` matching,
    changing the unit is a corpus-wide migration.**

13. **Probe-syntax stability contract.** Covered above in "Probes
    stability surface". Open question is whether `probesSchemaVersion`
    lives in `scripts/probes.py` only, or also in every generated
    `PROBES.md` header. Recommendation: both, so a PROBES.md committed
    in a future release is self-describing.

14. ★ **Path-normalization protocol.** Fixture path equality discipline
    (relative vs absolute) to avoid silent zero-match on RESOLVES.
    Recommendation: both sides reduce to `Path(p).resolve()
    .relative_to(REPO_ROOT).as_posix()` before equality; mismatch is a
    hard error. **Must settle before impl** — silent zero-match makes
    every RESOLVES pass vacuously, which is the worst failure mode for
    a trust-gap-closing feature.

15. **Negative-probe seeding policy.** Resolved above in "Negative-
    probe seeding methodology": option (a), separate
    `probes_negative/` tree per donor. Open follow-up: how does the
    coverage guard count `probes_negative/` files toward the per-donor
    minimum? Recommendation: count separately so a donor must have
    both positive and negative coverage to clear the v1.2 threshold.

16. **Coverage-guard grace for new fixtures/donors.** When a brand-new
    donor lands with zero probes, does the guard red-fail immediately?
    Recommendation: ratchet on corpus-wide probe count (build fails if
    probe count drops vs the prior `main` commit, *not* if absolute
    coverage drops below a static threshold). New fixtures land with
    `PROBES: pending` in the donor README and get one release cycle
    to grow probes before counting toward release-blocking thresholds.
    Combined with Q8: ratcheting is informational in v1.1, becomes
    release-blocking in v1.2.

17. **Out-of-bump catalog drift surfacing.** The scheduled
    `catalog-drift-watch.yml` workflow catches `pykrete-core@main`
    growing a D-code before our `PYKRETE_REF` pin moves. Original spec
    proposed a GitHub issue; amended during pykrete-tests PR #5 to a
    pull request (peter-evans/create-pull-request) carrying the
    regenerated catalog. Rationale: PR is reviewable + mergeable in
    one step; the catalog refresh is mechanical and benefits from
    auto-PR; trust signal is stronger ("here's the proposed change,
    review and land") vs an issue that merely documents drift.
    Gating the bump PR is overkill; the bump PR's own CI runs the
    (non-scheduled) drift checker which will catch any actual
    staleness. The auto-PR uses a stable `catalog-drift/auto` branch
    so successive weekly runs force-update one PR rather than
    accumulating one open PR per cron — keeps the queue tidy without
    needing a separate auto-close workflow.

18. **SemVer policy for D-code renames/removals within a minor.**
    Pinned above in "D-code lifecycle": renames are breaking
    (major bump on pykrete-core); removals require one-cycle
    deprecation (D0030 deprecated in v1.2.0 stays emittable through
    v1.3.0). Open follow-up: where does the deprecation get codified
    in pykrete-core's source — a `#[deprecated]` attribute on the
    catalog entry, a parallel `DEPRECATED_CODES` list, or comment-only?
    Recommendation: structured (a `deprecated: bool` field added to
    `DIAGNOSTIC_CATALOG` entries, surfaced into
    `diagnostic_catalog.json` so the harness can warn on
    `PROBE-EXPECTS` against a deprecated code). Decision lives in
    pykrete-core's first relevant release; tracked here so the harness
    knows what to expect when the field appears.

19. **v1.0 trust-claim language migration for v1.1 release notes.**
    The v1.0 README's "zero diagnostics on real PySpark code"
    line predates the `probes_negative/` tree. Once v1.1 ships,
    the CI summary will show non-zero diagnostics from
    `probes_negative/` fixtures, which a reader could misinterpret
    as regression. Recommendation: the v1.1 release notes amend the
    trust-claim to *"zero diagnostics on donor-faithful `annotated/`
    fixtures; expected diagnostics on `probes_negative/` fixtures."*
    The amendment is in the release-notes draft, the
    `cross-codebase/README.md`, and the public-facing docs-site
    Production Readiness page. Open follow-up: does v1.1 also amend
    the v1.0 README itself (retroactive clarification), or only land
    the new phrasing going forward? Recommendation: amend the README
    as a one-liner in the v1.1 ship PR — the v1.0 phrasing is
    technically still true (`annotated/` is "the real PySpark code"),
    but the precision improves trust.

## Out of scope (kept explicit; mirrors `literal-value-vocabulary.md`)

Probes are static schema-tracking assertions. The following are
deliberately excluded; they would require either a runtime or
capabilities pykrete has decided not to ship:

- **Row-value assertions.** Probes do not assert that a column contains
  specific values, or that a filter produced N rows. Pykrete has no
  runtime; row values are unknowable at edit time.
- **Cross-file schema flow.** A probe in file A cannot assert about
  bindings in file B. v1.1 limits probes to the file they live in.
- **Longitudinal "schema-after-N-transforms" assertions** as a
  primitive. If you need to assert the schema after a 5-step chain,
  you write a `PROBE-RESOLVES` or `PROBE-TYPE-IS` on a line that
  references the post-chain binding — there is no `PROBE-SCHEMA-AT:
  <line>` form in v1.1.
- **Behavioural assertions on pykrete-the-runtime** (the transpiled
  Python). Probes assert what pykrete-the-checker sees, not what the
  transpiled `.py` does at runtime. Runtime correctness is donor-test
  territory, not probe territory.
- **Nullability tracking and exact-output-column-set verification.**
  Deferred to v1.2 — see "Deferred to v1.2" above. v1.1 ships name
  resolution + type tracking only.
- **Generic Python assertion DSL.** Probes are the five markers above
  and nothing else. The grammar is closed for v1.1. New marker kinds
  (including the v1.2 NULLABLE / OUTPUT-COLUMNS revival) require a
  spec PR.

## v1.1 work plan

### Spec PR (this document)

Land this file plus the open-question resolutions above. Multi-lens
review on the spec, same pattern as v0.1.x:

1. **Correctness lens.** Does the grammar express every assertion the
   v1.1 cross-codebase suite actually needs? Walk the 32 fixtures and
   sketch at least one probe per fixture; flag any that the grammar
   cannot express.
2. **Adversarial lens.** Failure modes the design glosses over —
   ambiguous spans, donor-file collision with `# PROBE-FOO`, stacked
   probes interacting badly, transpiler interaction with the marker
   comments, synthesizer-rewrite approach (a) breaking down for some
   `PROBE-TYPE-IS` corner case.
3. **Schema-stability lens.** Confirm the design genuinely does not
   touch `--format json` output for the EXPECTS/RESOLVES/TYPE-IS/
   FILE-* markers, that the synthesizer-rewrite approach for TYPE-IS
   keeps the JSON contract intact, and that the deferred NULLABLE /
   OUTPUT-COLUMNS markers do not leak forward-references into v1.1
   stability surface. The grep of `pykrete-lsp` and `pykrete-vscode`
   for any reliance on the `diagnostics[]` shape stays informational
   only — the contract does not change.

### Implementation PRs (in order)

1. **`scripts/probes.py` + `diagnostic_catalog.json` +
   `check_catalog_drift.py` + unit tests** (cross-codebase repo).
   Lands first, in isolation, so the grammar is reviewable before any
   fixture work.
2. **Wire into `golden.sh check` + CI workflow** (cross-codebase
   repo). Lands second; at this point the system is shippable but
   asserts nothing yet (no fixture has probes; coverage guard
   informational).
3. **Seed probes across the 32 existing fixtures + bootstrap
   `probes_negative/` trees per donor** (cross-codebase repo). At
   minimum one `PROBE-RESOLVES` per fixture, plus one
   `PROBE-EXPECTS`-driven `probes_negative/` fixture per donor.
   Target: ≥50 probes across the corpus, ≥3 per donor (positive +
   negative combined). Each probe accompanied by a one-line rationale
   in the `-- <rationale>` slot for review-pass legibility.
4. **(v1.2)** Flip the coverage guard from informational to
   release-blocking once authoring experience confirms the thresholds.
5. **Docs**: author-facing section in `cross-codebase/README.md`
   (probe vocabulary, when to use which kind, examples drawn from
   the seeded corpus, link to the probes stability surface); release-
   notes blurb tied to the pykrete-tests v1.1 cut.

No pykrete-core PR is required for any v1.1 marker
(EXPECTS / RESOLVES / TYPE-IS / FILE-*). TYPE-IS lands via the
synthesizer-rewrite approach (option (a) under "Golden format") using
existing D-codes (D0080-D0082) with no pykrete-core change. NULLABLE
and OUTPUT-COLUMNS are deferred to v1.2 along with their coordinated
pykrete-core extensions (D0083 emission-site work for NULLABLE; a new
D-code for OUTPUT-COLUMNS); see "Deferred to v1.2" above.

### Cross-codebase fixture migration (the 32 backfill)

The backfill is the load-bearing piece — it converts probes from "a
spec we shipped" into "a capability we use." Plan:

- **Bulk happy-path pass** (~half day). For each of the 32 fixtures,
  add a `PROBE-RESOLVES id=<donor>-<short>` immediately above one
  representative DataFrame binding in the file. Hits the
  ≥1-per-fixture minimum, gives the coverage guard something to
  enforce, costs almost nothing per fixture.
- **Negative-probe pass per donor** (~1 day). For each donor, write
  3-5 fixtures in `probes_negative/` that deliberately reference
  columns the schema does not declare. Add `PROBE-EXPECTS: D0030 on
  "..."` for each. These are the probes that earn the trust claim —
  they prove the checker fires where it should on real PySpark
  shapes. Donor-faithful tree stays untouched.
- **Schema-narrowing pass on `.select()` chains** (~half day). For
  every fixture with a `.select(subset...)` followed by further
  operations, add a `PROBE-RESOLVES` (kept column) on a line after the
  select that references one of the kept columns. This is the exact
  pattern the user named in the framing quote — proves a kept column
  survives the narrow, without requiring exact-output-set verification
  (deferred with OUTPUT-COLUMNS to v1.2).
- **Type pass** (~half day). Add `PROBE-TYPE-IS` on selected bindings
  using the synthesizer rewrite (D0080-D0082 target). Focused on bindings
  where the column type is non-obvious — e.g. after a `cast()`, after
  arithmetic, after `withColumn(... lit(0).cast("long"))`. Individual
  probe instances drop out of the seeding pass cleanly if the
  synthesizer can't encode them; the marker grammar stays for v1.2
  to extend.

Total backfill: ~2.5 days, parallel-izable with the harness PRs.

### Impl-PR coverage backlog (not blocking spec approval)

These items emerged from spec round-3 review but were judged better-
suited to impl-PR-time verification than another spec round. Tracking
here so they're not lost when impl starts.

- [ ] **catalogSchemaVersion edge cases** — what happens when impl
  encounters `catalogSchemaVersion: "2"`? Hard fail with named error,
  or skip-with-warning? Decide at impl PR for `scripts/probes.py`.
- [ ] **D-code deprecation enforcement teeth** — the spec says
  renames/removes follow SemVer-major and need a deprecation window.
  Impl needs to actually emit a warning when a probe references a
  D-code marked `deprecatedIn`. Decide warning channel (stderr,
  golden, both) at impl.
- [ ] **Scheduled GHA failure modes** — pykrete-core main red, network
  failure mid-pull, pykrete-core ships a patch between weekly runs.
  Each path needs a defined behavior (no-op, alert, auto-PR). Decide
  at the GHA wiring PR.
- [ ] **Trust-claim migration atomicity** — the README +
  production-readiness.md update from "zero on annotated/" to "zero on
  annotated/, expected on probes_negative/" must land in the same PR
  as the first probes_negative fixtures, not before (or trust claim
  becomes a lie). Coordinate at the seed-probes PR.
- [ ] **Append-only tree invariants** — adding sibling trees
  (`probes_perf/`, `probes_smoke/`) is non-breaking. Spec should state
  this is the convention; impl should encode the convention in the
  runner's tree-discovery logic.
- [ ] **Per-atomic-type feasibility table for PROBE-TYPE-IS** — the
  synthesizer "construct a synthetic expression that fires D0080-D0082"
  is plausible for numeric/string/bool, but what about complex types
  (struct/array/map)? Table at impl PR mapping each `atomic_type →
  which D-code fires → synthetic expression shape`. Cases with no
  working synthesis become impl-time errors with a defined message.
- [ ] **Synthesizer harness flow diagram or pseudocode** — the spec
  describes synthesis abstractly. Impl PR for `scripts/probes.py`
  should include a pseudocode block in its design doc showing the
  marker → synthesized `.pyk` → checker invocation → diagnostic
  capture → assertion flow.
- [ ] **PROBE-TYPE-IS numeric-subtype distinguishability** — v1.1 rejects numeric-subtype assertions (int/long/short/byte/double/float/decimal) at parse time because the synthesizer can only fire family-level D-codes (D0080-D0082). v1.2 requires either schema-trace output or new pykrete-core D-codes for numeric-subtype-mismatch.
- [ ] **golden.sh discovery widening for probes_negative/** — current `golden.sh check` only walks `*/annotated/*`. PR #3c should widen to also walk `*/probes_negative/*` so the release-blocking gate covers the negative tree. Currently the negative tree is exercised only via probes_ci.sh, not golden.sh. Note: when widened, the existing normalize step at `golden.sh:36` (which strips a `cross-codebase/` prefix) will need adjustment to match the runtime's donor-anchored relpath format used by `_fixture_relpath`. Either adjust normalize OR re-normalize negative goldens to match the walker output format.

## Cost estimate

| Phase | Effort |
|---|---|
| Grammar + parser (`probes.py extract` + unit tests + tokenize plumbing) | 1 day |
| Verifier + harness wiring (`probes.py verify`, `golden.sh` edits) | 1 day |
| Catalog vendor + drift check (`diagnostic_catalog.json`, `check_catalog_drift.py`, scheduled out-of-bump workflow) | 0.5 day |
| Seed probes across 32 fixtures + bootstrap `probes_negative/` | 1.5 days |
| Coverage guard (informational) + CI + `PROBES.md` generator | 0.5 day |
| Type pass (synthesizer-rewrite translation for `PROBE-TYPE-IS`) | 0.5 day |
| Docs + release notes + stability surface + buffer | 1 day |

**Total: 5-7 days** (one focused engineer-week including review
iteration). Reverts to the original v1.1 estimate after the TM scope-
narrowing decision to drop NULLABLE and OUTPUT-COLUMNS to v1.2: with
fewer marker kinds, no coordinated pykrete-core release, and a
synthesizer rewrite that targets only D-codes already in v1.0 stable
catalog (D0080-D0082), the budget tightens back to the original
headline.

**Release delta**: ships as **pykrete-tests v1.1.0**. Pykrete-core
does NOT need a release for any v1.1 marker
(EXPECTS / RESOLVES / TYPE-IS / FILE-*). All five markers land within
pykrete-tests; the synthesizer rewrite targets only D-codes that
already exist in the v1.0 stability commitment (D0080-D0082). When
pykrete cuts its next release for unrelated work (e.g. the literal-
value-vocabulary work), the cross-codebase repo bumps `PYKRETE_REF`
and regenerates `diagnostic_catalog.json` as usual. The deferred
NULLABLE / OUTPUT-COLUMNS markers will pull in coordinated pykrete-
core changes (D0083 emission-site work for NULLABLE; a new D-code for
OUTPUT-COLUMNS — number TBD since D0080 is already taken) when they
revive in v1.2; that cost is filed against v1.2, not v1.1.

## Related

- [[feedback_cross_codebase_must_verify_correctness]] — the
  user-supplied gap-statement this design answers. The framing
  principle and scope bright-line come from there.
- [`literal-value-vocabulary.md`](./literal-value-vocabulary.md) — the
  sibling v1.1 tracker. Both features answer the same question
  ("how does the schema layer earn its trust?") from two directions:
  enum literals extend *what* pykrete checks; probes prove *that*
  pykrete checks correctly. The two trackers should ship in the same
  v1.1 cycle, with this one landing first (zero pykrete-core risk for
  the EXPECTS / RESOLVES / FILE-* markers).
- [[feedback_trust_is_core_value_prop]] — the underlying discipline.
  Probes are the structural enforcement of "delay over a bad launch":
  the v1.0 cross-codebase suite earned the launch; the v1.1 probes
  earn the *next* launch by proving the launch wasn't vacuous.
- [[project_pre_release_audit_cycle.md]] — the 3-agent pre-release
  audit pattern. The probe-coverage guard becomes a fourth signal the
  pre-release audit should sample (probe density, per-donor minimums,
  unclaimed-fixture count, `probes_negative/` parity).
- [`about/production-readiness.md`](../../docs-site/src/content/docs/about/production-readiness.md#json-output-stability-contract)
  — the v1.0 JSON output stability contract. Q12 (column-unit
  semantics) adds a one-line clarification to that contract; the
  Probes stability surface section above parallels its structure.
