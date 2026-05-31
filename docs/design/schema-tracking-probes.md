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
3. **Schema-shape positive**: "after this transform, the column we name
   has type `<T>` / is nullable / the schema's exact column set is
   `<...>` — if pykrete's tracked schema disagrees, we drifted."

Probes are the structural answer to the trust pitch. The v1.0 README
says we catch schema bugs at edit time. Probes are how we prove it on
every CI run, against real PySpark donors, without trusting that the
absence of red is the presence of correctness.

**Scope bright line, same as `literal-value-vocabulary.md`**: probes
verify *static* schema tracking — what pykrete knows at edit time about
column names, column types, and nullability after a chain of operations.
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
checker work for a capability the inline design delivers in ~6-8 days
with no checker change at all. Helper calls remain on the v1.2 table if
probes graduate into a dogfooding/teaching surface; for v1.1 they are
overkill.

### Probe syntax (final shape)

Markers are single-line comments at column 0, always on the line
*immediately above* the target line (see Q10 for the exact "target
line" resolution rule). Five kinds, covering exactly what the v1.1
cross-codebase suite needs to assert:

```
# PROBE-EXPECTS: <D-code> [id=<handle>] [on "<span-text>"] [match /<regex>/[flags]]
# PROBE-RESOLVES: [id=<handle>] [-- <rationale>]
# PROBE-TYPE-IS: <column> :: <type-expr> [id=<handle>] [-- <rationale>]
# PROBE-NULLABLE: <column> = (true|false) [id=<handle>] [-- <rationale>]
# PROBE-OUTPUT-COLUMNS: [<col>, <col>, ...] [id=<handle>] [-- <rationale>]
# PROBE-FILE-CLEAN-OF: <D-code>[, <D-code>...]
# PROBE-FILE-COUNT: <D-code> == <N>
```

Concrete uses:

```python
# PROBE-FILE-CLEAN-OF: D0030, D0050
from pykrete import col, lit
from quinn.schemas import Order

def pipeline(orders: DataFrame[Order]) -> DataFrame[Order]:
    # PROBE-RESOLVES: id=quinn-select-region -- region survives narrow select
    df = orders.select("region", "amount")

    # PROBE-OUTPUT-COLUMNS: [region, amount] id=quinn-select-shape
    # PROBE-TYPE-IS: amount :: double id=quinn-amount-type
    # PROBE-NULLABLE: region = false id=quinn-region-nonnull
    df2 = df

    # PROBE-EXPECTS: D0030 id=quinn-select-drops-product on "product"
    return df2.select(col("product"))
```

Rules:

- Markers are line-anchored to a "target line". For `PROBE-EXPECTS`,
  `PROBE-RESOLVES`, `PROBE-TYPE-IS`, `PROBE-NULLABLE`, and
  `PROBE-OUTPUT-COLUMNS`, the target line is the **first source line of
  the next Python logical statement** following the comment (see Q10).
  `PROBE-FILE-*` are file-scoped and conventionally live at the top of
  the annotated file.
- `id=<handle>` is optional but recommended. When absent, the harness
  synthesizes one from the fixture path + comment line. When present,
  it shows up in failure output and in the per-donor `PROBES.md` index.
  IDs are per-donor unique (see Q1); the verifier hard-fails on
  collision within a donor.
- `on "<text>"` pins the diagnostic span by source-text match
  (resolved by slicing the fixture text between the diagnostic's
  `(line, column)..(endLine, endColumn)` — column unit defined in Q12).
  Quoting follows shell rules: double-quoted, `\"` escapes.
- `match /<regex>/[flags]` pins the diagnostic message. The flavour is
  **Python `re`** (probes.py is stdlib Python; the regex never leaves
  Python). Supported flags: `i`, `m`, `s`, `x`. Mutually compatible
  with `on`. Caveat: message wording is NOT STABLE per v1.0's JSON
  contract (see "Probes stability surface"); prefer `on` to `match`.
- A line whose stripped form matches
  `^# PROBE-(EXPECTS|RESOLVES|TYPE-IS|NULLABLE|OUTPUT-COLUMNS|FILE-CLEAN-OF|FILE-COUNT)\b`
  but does NOT match the grammar is a hard parse error (`probe parse
  error in <fixture>:<line>`). A line that matches `^# PROBE-` with
  any other suffix is silently ignored (not a parse error) so that
  future upstream drift introducing an unrelated `# PROBE-FOO` comment
  in vendored code does not red-fail CI.
- A `PROBE-EXPECTS` whose `<D-code>` is not in `DIAGNOSTIC_CATALOG`
  fails at parse time. The catalog is supplied by the harness itself
  (see "Diagnostic catalog source" below); pykrete-core ships no new
  CLI surface for v1.1.
- `PROBE-TYPE-IS` `<type-expr>` is one of pykrete's atomic type names
  as they appear in `Schema` annotations (`int`, `long`, `double`,
  `string`, `boolean`, `date`, `timestamp`, `binary`, `byte`, `short`,
  `decimal`, `decimal(p, s)`). Composite types are out of v1.1 scope.
- `PROBE-OUTPUT-COLUMNS` asserts an **ordered, exact match** against
  the schema pykrete tracks at the target line. Subset-only or
  any-order modes are deferred to a future spec PR.

Explicitly excluded from the grammar (kept narrow on purpose):

- No multi-line / block markers. One line, one assertion.
- No comma-separated D-codes in `PROBE-EXPECTS`. Two diagnostics on the
  same line means two stacked `PROBE-EXPECTS` comments (pairing
  algorithm: see Q9).
- No `PROBE-NEXT-LINE` / `PROBE-PREV-LINE` toggles. Target is always
  the next-logical-statement rule from Q10.
- No trailing inline markers (`x = df.col("foo")  # PROBE-EXPECTS:...`).
  Formatters strip them; the target-line rule becomes ambiguous.

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

### Golden format (unchanged)

The `.golden.json` format does not change. `--format json` output stays
exactly as v1.0 froze it: `{schemaVersion: "1", diagnostics, summary}`,
no new top-level `probes` array, no new per-diagnostic fields. Probes
are an assertion *layer* over the existing JSON, not a new payload in
it.

A fixture passes iff (a) the normalized JSON diff against
`.golden.json` is clean AND (b) every probe in the file is satisfied.
Both checks run on every CI invocation; both report independently.

For `PROBE-TYPE-IS`, `PROBE-NULLABLE`, and `PROBE-OUTPUT-COLUMNS`,
the verifier needs the schema pykrete tracks at the target line.
The `--format json` payload today carries diagnostics but not the
tracked schema. Two implementation options:

- **(a) Synthesizer-style probe-to-diagnostic translation**: the
  harness rewrites `PROBE-TYPE-IS amount :: long` (when the truth is
  `double`) into a synthetic `lit(0).cast("long") + df.amount` line
  appended to a scratch copy of the fixture, then asserts the
  resulting type-mismatch D-code fires. Pure JSON-output assertion;
  zero pykrete-core change. Awkward for `PROBE-OUTPUT-COLUMNS`.
- **(b) Add a `--emit-schema-trace` CLI flag** to `pykrete check`
  emitting per-binding-site schema info on stderr or as a side-channel
  JSON file. First-class but requires a pykrete-core release and
  schema-stability discussion for the new payload.

Recommendation: **(a) for v1.1**, scoped to `PROBE-TYPE-IS` and
`PROBE-NULLABLE`; defer `PROBE-OUTPUT-COLUMNS` to v1.2 if (a) proves
infeasible for the ordered exact-match case. This keeps the "no
pykrete-core release" claim intact, at the cost of some authoring
gymnastics for the type/nullability probes. If the awkwardness is
real once we hit the seeding pass, we revisit (b) before the v1.1
cut, accepting the coordinated release.

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
     - `TYPE-IS`, `NULLABLE`, `OUTPUT-COLUMNS`: implemented via the
       probe-to-diagnostic translation described in "Golden format"
       above; fall back to "v1.1 deferred" if (a) proves unworkable.
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
   budget for: grammar parsing (all 7 marker kinds, all error paths
   including in-docstring suppression and the silent-skip rule for
   unknown PROBE-* prefixes); span matching against canned JSON +
   synthetic fixture with UTF-8 multi-byte characters; ID synthesis +
   collision handling; path-normalization edge cases (relative vs
   absolute); stacked-EXPECTS pairing (Q9); target-line resolution
   on chained calls / decorated defs / blank lines (Q10).
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
     for `PROBE-EXPECTS` parse validation.

7. **Per-donor `cross-codebase/<donor>/PROBES.md`** (auto-generated by
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

The probes layer carries an explicit version handle:
`probesSchemaVersion: "1"`. It lives at the top of
`scripts/probes.py` and in `cross-codebase/README.md`. It bumps on
breaking changes to the marker grammar.

**Marker grammar — STABLE.** Once the v1.1 spec lands, these names
are commitments:

- **Marker kind names — STABLE.** Renaming `PROBE-EXPECTS` →
  `PROBE-FIRES` (or similar) requires a `probesSchemaVersion` bump and
  a corpus-wide migration. Codified in `tests/test_probes.py`.
- **Argument syntax for existing kinds — STABLE.** Changing
  `on "text"` to `on:text` or moving `id=` to a positional slot is
  breaking.
- **Adding a new marker kind — NON-BREAKING.** New `PROBE-*` kinds
  (e.g. `PROBE-PARTITIONED-BY` if that ever lands) are additive;
  `probesSchemaVersion` stays `"1"`.
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

## Open design questions (settle before any code lands)

The first 8 questions stay from round 1; Q9-Q16 were surfaced by the
multi-lens review and must be settled before the impl PR lands.
"Settled" here means "the spec or a follow-up PR has a written
decision, not just a recommendation."

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

9. **Stacked-EXPECTS pairing algorithm.** When two `PROBE-EXPECTS`
   markers stack above one line and pykrete fires two D0030
   diagnostics, how do we pair probe ↔ diagnostic? Options:
   bipartite-match-distinct (each probe must match a distinct
   diagnostic; recommended), first-match-wins (order-dependent;
   brittle), each-against-the-set (would let one diagnostic satisfy
   two probes; loses precision). **Must settle before impl.**

10. **Target-line resolution rule.** Literal next-line vs AST-resolved
    next logical statement; behaviour on decorators, multi-line
    chained calls, blank lines, comments. Recommendation: AST re-
    parse on the fixture; target line = first source line of the next
    logical statement after the comment. Blank-only and comment-only
    lines are skipped. Markers immediately above a decorator attach to
    the decorated def, not the decorator. **Must settle before impl
    because target_line is encoded in every probe.**

11. **In-string probe extraction.** Regex scan over file bytes will
    match inside docstrings and triple-quoted SQL strings.
    Recommendation: use `tokenize.tokenize` and filter to COMMENT
    tokens only. Lock with a test case (docstring containing the
    string `# PROBE-EXPECTS: D0030` must NOT count as a probe).
    **Must settle before impl.**

12. **Column-unit semantics for span match.** Today's renderer
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

14. **Path-normalization protocol.** Fixture path equality discipline
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
  you write a `PROBE-RESOLVES` or `PROBE-OUTPUT-COLUMNS` on a line
  that references the post-chain binding — there is no
  `PROBE-SCHEMA-AT: <line>` form in v1.1.
- **Behavioural assertions on pykrete-the-runtime** (the transpiled
  Python). Probes assert what pykrete-the-checker sees, not what the
  transpiled `.py` does at runtime. Runtime correctness is donor-test
  territory, not probe territory.
- **Generic Python assertion DSL.** Probes are the seven markers above
  and nothing else. The grammar is closed. New marker kinds require a
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
   comments, schema-trace approach (a) breaking down for
   `PROBE-OUTPUT-COLUMNS`.
3. **Schema-stability lens.** Confirm the design genuinely does not
   touch `--format json` output for the EXPECTS/RESOLVES/FILE-*
   markers, and that the schema-trace approach for TYPE-IS/NULLABLE
   keeps the JSON contract intact (or surfaces the coordinated release
   clearly if it doesn't). The grep of `pykrete-lsp` and
   `pykrete-vscode` for any reliance on the `diagnostics[]` shape
   stays informational only — the contract does not change.

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

No pykrete-core PR is required for the EXPECTS/RESOLVES/FILE-*
markers. The TYPE-IS/NULLABLE/OUTPUT-COLUMNS markers may require
pykrete-core surface (option (b) under "Golden format") if option (a)
proves infeasible during seeding; that decision is folded into the
seeding-pass PR review, not pre-decided here.

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
- **Schema-narrowing pass on `.select()` chains** (~1 day). For every
  fixture with a `.select(subset...)` followed by further operations,
  add a paired `PROBE-RESOLVES` (kept column) + `PROBE-OUTPUT-COLUMNS
  [keptA, keptB]` at the post-select binding site. This is the exact
  pattern the user named in the framing quote.
- **Type/nullability pass** (~half day, if option (a) holds). Add
  `PROBE-TYPE-IS` / `PROBE-NULLABLE` on selected bindings using the
  synthesizer rewrite. If awkwardness is high, defer to v1.2 and
  reframe v1.1 probe scope to "names + shape" without type/nullability
  guarantees — the Probes stability surface section then drops the
  `TYPE-IS` / `NULLABLE` markers cleanly via additive removal.

Total backfill: ~3 days, parallel-izable with the harness PRs.

## Cost estimate

| Phase | Effort |
|---|---|
| Grammar + parser (`probes.py extract` + unit tests + tokenize plumbing) | 1.5 days |
| Verifier + harness wiring (`probes.py verify`, `golden.sh` edits) | 1.5 days |
| Catalog vendor + drift check (`diagnostic_catalog.json`, `check_catalog_drift.py`) | 0.5 day |
| Seed probes across 32 fixtures + bootstrap `probes_negative/` | 1.5 days |
| Coverage guard (informational) + CI + `PROBES.md` generator | 0.5 day |
| Type/nullability pass (option (a) translation) | 1 day |
| Docs + release notes + stability surface + buffer | 1 day |

**Total: 6-8 days** (one focused engineer-week-plus including review
iteration). Slightly more than the original 5-7 day estimate to honour
the expanded grammar (TYPE-IS / NULLABLE / OUTPUT-COLUMNS), the
catalog-drift infrastructure, and the realistic test-budget bump.

**Release delta**: ships as **pykrete-tests v1.1.0**. Pykrete-core
does not need a release for the EXPECTS / RESOLVES / FILE-* markers.
TYPE-IS / NULLABLE / OUTPUT-COLUMNS land via the synthesizer rewrite
(option (a)) and stay within pykrete-tests; if option (b) becomes
necessary during seeding, a coordinated pykrete-core release is folded
in and the cost above bumps by ~1 day for the CLI surface. When
pykrete cuts its next release for unrelated work (e.g. the literal-
value-vocabulary work), the cross-codebase repo bumps `PYKRETE_REF`
and regenerates `diagnostic_catalog.json` as usual.

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
