# Schema-tracking probes (v1.1 design tracker)

**Status**: planned for v1.1. Sibling of `literal-value-vocabulary.md` —
both are "make the schema layer earn its trust through positive
verification."
**Origin**: 2026-05-30 user direction during the post-v1.0.0 sprint:

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
fixture that says one of two things:

1. **Positive**: "after this transform, pykrete's tracked schema should
   resolve `<column>` cleanly — if it doesn't, we lost the schema."
2. **Negative**: "after this transform, pykrete's tracked schema should
   reject `<column>` with D0030 — if it doesn't, we silently widened."

Probes are the structural answer to the trust pitch. The v1.0 README
says we catch schema bugs at edit time. Probes are how we prove it on
every CI run, against real PySpark donors, without trusting that the
absence of red is the presence of correctness.

**Scope bright line, same as `literal-value-vocabulary.md`**: probes
verify *static* schema tracking — what pykrete knows at edit time about
column names, column types, and nullability after a chain of operations.
Probes do not verify row values, runtime behavior, or anything that
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
3. **Donor-faithful.** Markers are plain Python comments. They live in
   the same annotation lane that already adds `from pykrete import ...`
   and schema declarations to the donor file — i.e. the lane we already
   own as annotators, distinct from the upstream code we vendor
   verbatim. Donor re-sync touches the underlying code; probe comments
   travel with our annotation layer.
4. **Failure mode is human-legible.** The harness prints `PROBE FAILURE:
   <fixture>:<line> id=<id>  expected: ...  actual: ...` — debuggable
   without `jq` over a 200-line golden diff.

We rejected the sibling-TOML design (Option B) primarily because it
splits the assertion from the code, and because the v1.1 scope does not
need needle-based anchor stability (the inline marker's "line+1" rule
either matches loudly or fails loudly — there is no silent-wrong-target
failure mode, since EXPECTS mandates a D-code that must actually fire).
We rejected the helper-call design (Option C) because it expands
pykrete's public surface (`pykrete.testing`), bakes a new D-code
(`D0090`) into the stability contract, and demands ~8-10 days of
checker work for a capability the inline design delivers in ~5 days
with no checker change at all. Helper calls remain on the v1.2 table if
probes graduate into a dogfooding/teaching surface; for v1.1 they are
overkill.

### Probe syntax (final shape)

Markers are single-line comments at column 0, always on the line
*immediately above* the target line. Four kinds, covering exactly what
the v1.1 cross-codebase suite needs to assert:

```
# PROBE-EXPECTS: <D-code> [id=<handle>] [on "<span-text>"] [match /<regex>/[flags]]
# PROBE-RESOLVES: <tag> [id=<handle>]
# PROBE-FILE-CLEAN-OF: <D-code>[, <D-code>...]
# PROBE-FILE-COUNT: <D-code> == <N>
```

Concrete uses:

```python
# PROBE-FILE-CLEAN-OF: D0030, D0050
from pykrete import col, lit
from quinn.schemas import Order

def pipeline(orders: DataFrame[Order]) -> DataFrame[Order]:
    # PROBE-RESOLVES: id=quinn-select-region  region survives narrow select
    df = orders.select("region", "amount")

    # PROBE-EXPECTS: D0030 id=quinn-select-drops-product on "product"
    return df.select(col("product"))
```

Rules:

- Markers are line-anchored. `PROBE-EXPECTS` and `PROBE-RESOLVES` assert
  about the line *immediately below* the comment. `PROBE-FILE-*` are
  file-scoped and conventionally live at the top of the annotated file.
- `id=<handle>` is optional but recommended. When absent, the harness
  synthesizes one from the fixture path + line. When present, it shows
  up in failure output and in the per-donor `PROBES.md` index. IDs are
  per-file unique, not globally unique; failure output qualifies with
  the fixture path.
- `on "<text>"` pins the diagnostic span by source-text match
  (resolved by slicing the fixture bytes between the diagnostic's
  `(line, column)..(endLine, endColumn)`). Quoting follows shell rules:
  double-quoted, `\"` escapes.
- `match /<regex>/[flags]` pins the diagnostic message. Flags reuse
  the Rust `regex` crate syntax. Mutually compatible with `on`.
- A line whose stripped form matches `^# PROBE-` but does NOT match
  the grammar is a hard parse error (`probe parse error in
  <fixture>:<line>`). Catches typos like `# PROBE-EXPECT:` (missing S).
- A `PROBE-EXPECTS` whose `<D-code>` is not in `DIAGNOSTIC_CATALOG`
  fails at parse time. The harness reads the catalog once at startup
  via `pykrete diagnostics --list --format json` (existing surface).

Explicitly excluded from the grammar (kept narrow on purpose):

- No multi-line / block markers. One line, one assertion.
- No comma-separated D-codes in `PROBE-EXPECTS`. Two diagnostics on the
  same line means two stacked `PROBE-EXPECTS` comments.
- No `PROBE-NEXT-LINE` / `PROBE-PREV-LINE` toggles. Target is always
  line+1 for EXPECTS/RESOLVES.
- No trailing inline markers (`x = df.col("foo")  # PROBE-EXPECTS:...`).
  Formatters strip them; the line+1 rule becomes ambiguous.

### Golden format (unchanged)

The `.golden.json` format does not change. `--format json` output stays
exactly as v1.0 froze it: `{schemaVersion: "1", diagnostics, summary}`,
no new top-level `probes` array, no new per-diagnostic fields. Probes
are an assertion *layer* over the existing JSON, not a new payload in
it.

A fixture passes iff (a) the normalized JSON diff against
`.golden.json` is clean AND (b) every probe in the file is satisfied.
Both checks run on every CI invocation; both report independently.

Failing output (per fixture):

```
PROBE FAILURE: cross-codebase/quinn/annotated/quinn/functions.pyk
  line 87  id=quinn-select-drops-product  PROBE-EXPECTS D0030 on "product"
    expected: D0030 with span text "product"
    actual:   no diagnostic on line 88

  line 142 id=quinn-withColumn-resolves   PROBE-RESOLVES "col(region)"
    expected: clean
    actual:   D0030 [resolveColumn] "unknown column 'region'" at 143:18-143:24

  file     PROBE-FILE-COUNT D0083 == 2
    expected: 2
    actual:   3 (lines 55, 89, 201)
```

### CLI changes (none)

`pykrete check --format json` is unchanged. `schemaVersion` stays
`"1"`. No new subcommand, no new flag, no new D-code, no new public
module. `pykrete-lsp` and the VS Code extension see nothing new.

One deferred convenience (v1.2 candidate, not v1.1 scope): a
`pykrete probes <file>` debug subcommand that parses markers and prints
them as a table for local authoring. Ship only if fixture authors ask.

### Runner changes (cross-codebase repo)

All changes live in the cross-codebase repo. Pykrete itself is
untouched for v1.1 probes.

1. **New: `scripts/probes.py`** (~200 LOC, stdlib-only, Python 3.10+).
   - `extract(fixture_path) -> list[Probe]`: line-by-line scan,
     regex-match `^# PROBE-`, parse per grammar, attach
     `target_line = comment_line + 1` for EXPECTS/RESOLVES, synthesize
     IDs for un-tagged markers, return typed records. Hard-fails on
     malformed markers and on unknown D-codes.
   - `verify(fixture_path, normalized_actual_json) -> list[ProbeFailure]`:
     - `EXPECTS`: scan `diagnostics[]` for `(file == fixture,
       line == target_line, code == expected_code)`. If `on "..."`
       present, slice fixture bytes by the diagnostic's
       `(line, column)..(endLine, endColumn)` and compare. If
       `match /.../` present, regex the `message`. Fail if no match.
     - `RESOLVES`: assert no diagnostic has `line == target_line`.
       Tag echoed in failure text.
     - `FILE-CLEAN-OF`: assert no diagnostic in the file carries any
       listed code.
     - `FILE-COUNT`: count diagnostics with that code in the file,
       compare to N.
   - Exits 0 if all satisfied, 1 with the failure block otherwise.

2. **Edit `scripts/golden.sh`** (~15 LOC).
   - After the existing `actual_norm=$(...)` capture in `check` mode:
     ```bash
     if ! python3 "$REPO_ROOT/scripts/probes.py" verify "$fixture" <<< "$actual_norm"; then
       fails=$((fails + 1))
     fi
     ```
   - In `generate` mode: run `probes.py --lint-only` (parse-check
     without verifying) so authoring errors fail the regen step.
   - New mode `golden.sh probes-report` emits per-donor `PROBES.md`.

3. **New: `tests/test_probes.py`** (~80 LOC, pytest).
   - Grammar parsing (all 4 kinds, all error paths).
   - Span matching against canned JSON + synthetic fixture.
   - ID synthesis + collision handling.
   - Runs as a pre-flight CI step before the golden suite (<1s).

4. **Edit `.github/workflows/cross-codebase.yml`**:
   - Add `Probe coverage guard` step: fail if any
     `*/annotated/**/*.pyk` fixture contains zero probe markers.
     Mirrors the spirit of pykrete's
     `every_diagnostic_code_has_a_fixture` guard. Threshold:
     `>= 1 probe per annotated fixture`. Tightenable per release.
   - Add per-donor minimum (start at 3, grow per release). Prevents
     a donor from joining the suite with a single throwaway probe.

5. **Per-donor `cross-codebase/<donor>/PROBES.md`** (auto-generated by
   `golden.sh probes-report`). Lists every probe with file, line, kind,
   id, expectation. Used by code review to spot probe-density drops
   when fixtures land or get edited.

## Open design questions (settle in the spec PR before any code)

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
   region. Document in `cross-codebase/README.md`.

4. **What happens when probe targets the last line of a file (no
   line+1)?** Recommendation: parse-time hard error (`probe targets
   nonexistent line N+1`).

5. **Stacked probes on the same target line** (two `PROBE-EXPECTS`
   comments above one line of code). Recommendation: allow; the
   verifier ANDs them. Both diagnostics must fire on the target.

6. **Donor file containing a pre-existing comment starting with
   `# PROBE`** (some upstream lint config). Recommendation: preflight
   check in the donor-sync script greps for `^# PROBE-` before
   annotation; fail-loud namespace collision detection. Vanishingly
   unlikely in PySpark corpora but cheap to guard.

7. **Multi-file probes**. A probe in `pipeline.pyk` cannot today
   reference schema defined in `schemas.pyk`. Recommendation: out of
   scope for v1.1. Today's `golden.sh` processes one fixture at a
   time; the only multi-file fixtures live in pykrete's insta catalog
   (D0071/D0072). Revisit in v1.2 when the first real cross-file
   cross-codebase fixture lands.

8. **Probe density threshold per release.** v1.1 lands with "≥1 probe
   per annotated fixture" and "≥3 per donor". Where does v1.2 set
   the bar? Recommendation: track corpus-wide probe count as a CI
   summary metric; commit to "non-decreasing" but settle the next
   absolute bar after a release of authoring experience.

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
  you write a `PROBE-RESOLVES` on a line that references the post-chain
  binding — there is no `PROBE-SCHEMA-AT: <line>` form in v1.1. (A
  schema-emit channel could be added in v1.2 if needed; would require
  the JSON-channel discussion we declined for v1.1.)
- **Behavioral assertions on pykrete-the-runtime** (the transpiled
  Python). Probes assert what pykrete-the-checker sees, not what the
  transpiled `.py` does at runtime. Runtime correctness is donor-test
  territory, not probe territory.
- **Generic Python assertion DSL.** Probes are the four markers above
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
   ambiguous spans, donor-file collision with `# PROBE`, stacked
   probes interacting badly, transpiler interaction with the marker
   comments.
3. **Schema-stability lens.** Confirm the design genuinely does not
   touch `--format json` output. The grep of `pykrete-lsp` and
   `pykrete-vscode` for any reliance on the `diagnostics[]` shape
   stays informational only — the contract does not change.

### Implementation PRs (in order)

1. **`scripts/probes.py` + unit tests** (cross-codebase repo). Lands
   first, in isolation, so the grammar is reviewable before any
   fixture work.
2. **Wire into `golden.sh check` + CI workflow** (cross-codebase
   repo). Lands second; at this point the system is shippable but
   asserts nothing yet (no fixture has probes; coverage guard not yet
   active).
3. **Seed probes across the 32 existing fixtures** (cross-codebase
   repo). At minimum one `PROBE-RESOLVES` per fixture (happy-path
   assertion that the binding resolves cleanly), plus
   `PROBE-EXPECTS` for any fixture we deliberately built to fire a
   diagnostic. Target: ≥50 probes across the corpus, ≥3 per donor.
4. **Activate the coverage guard** (cross-codebase repo). Once
   seeding completes, flip the workflow step from informational to
   blocking.
5. **Docs**: author-facing section in `cross-codebase/README.md`
   (probe vocabulary, when to use which kind, examples drawn from
   the seeded corpus); release-notes blurb tied to the pykrete-tests
   v1.1 cut.

No pykrete-core PR is required. No `--format json` change. No
coordinated `PYKRETE_REF` bump. The cross-codebase repo iterates
independently.

### Cross-codebase fixture migration (the 32 backfill)

The backfill is the load-bearing piece — it converts probes from "a
spec we shipped" into "a capability we use." Plan:

- **Bulk happy-path pass** (~half day). For each of the 32 fixtures,
  add a `PROBE-RESOLVES: id=<donor>-<short>  happy path` immediately
  above one representative DataFrame binding in the file. Hits the
  ≥1-per-fixture minimum, gives the coverage guard something to
  enforce, costs almost nothing per fixture.
- **Negative-probe pass per donor** (~1 day). For each donor, identify
  3-5 columns the donor's own code references that are *not* in the
  declared schema (off-by-one typos, dropped columns after a select).
  Add `PROBE-EXPECTS: D0030 on "..."` for each. These are the probes
  that earn the trust claim — they prove the checker fires where it
  should on real PySpark.
- **Schema-narrowing pass on `.select()` chains** (~1 day). For every
  fixture with a `.select(subset...)` followed by further operations,
  add a paired `PROBE-RESOLVES` (kept column) + `PROBE-EXPECTS: D0030`
  (dropped column) at the post-select binding site. This is the
  exact pattern the user named in the framing quote.

Total backfill: ~2.5 days, parallel-izable with the harness PRs.

## Cost estimate

| Phase | Effort |
|---|---|
| Grammar + parser (`probes.py extract` + unit tests) | 1 day |
| Verifier + harness wiring (`probes.py verify`, `golden.sh` edits) | 1 day |
| Seed probes across 32 fixtures (≥1 per, ≥3 per donor) | 1 day |
| Coverage guard + CI + `PROBES.md` generator | 0.5 day |
| Negative-probe + select-narrowing passes (real trust work) | 1.5 days |
| Docs + release notes + buffer | 1 day |

**Total: 5-7 days** (one focused engineer-week including review
iteration). Cheaper than the literal-value-vocabulary tracker
(~5-9 days) because nothing touches pykrete's type system.

**Release delta**: ships as **pykrete-tests v1.1.0**. Pykrete itself
does not need a release for this feature (the trust capability lives
entirely in the cross-codebase repo). When pykrete cuts its next
release for unrelated work (e.g. the literal-value-vocabulary work),
the cross-codebase repo bumps `PYKRETE_REF` as usual; no coordination
required.

## Related

- [[feedback_cross_codebase_must_verify_correctness]] — the
  user-supplied gap-statement this design answers. The framing
  principle and scope bright-line come from there.
- [`literal-value-vocabulary.md`](./literal-value-vocabulary.md) — the
  sibling v1.1 tracker. Both features answer the same question
  ("how does the schema layer earn its trust?") from two directions:
  enum literals extend *what* pykrete checks; probes prove *that*
  pykrete checks correctly. The two trackers should ship in the same
  v1.1 cycle, with this one landing first (zero pykrete-core risk).
- [[feedback_trust_is_core_value_prop]] — the underlying discipline.
  Probes are the structural enforcement of "delay over a bad launch":
  the v1.0 cross-codebase suite earned the launch; the v1.1 probes
  earn the *next* launch by proving the launch wasn't vacuous.
- [[project_pre_release_audit_cycle.md]] — the 3-agent pre-release
  audit pattern. The probe-coverage guard becomes a fourth signal the
  pre-release audit should sample (probe density, per-donor minimums,
  unclaimed-fixture count).
