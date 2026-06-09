# v1.5 retrospective

Settled at PR-F prep, prior to the v1.5.0 tag. Per project standing
practice (post-release retro is mandatory after every X.Y.Z tag).
Companion to `v1.4-spec.md` retro section and the (separate)
`docs/design/v1.5-spec.md`. Rules below feed the v1.6 spec authors.

## Cycle shape

- **Theme**: cross-dialect handoff + deferred-promise closure.
- **PRs landed (pykrete-side, this repo)**: 7 implementation PRs
  (PR-A1 / PR-A2 / PR-A3 / PR-B1 / PR-B2 / PR-C / PR-D / PR-E) plus the
  PR-1 spec PR plus PR-F (this PR).
- **PRs landed (pykrete-tests-side)**: PR-G dbt-spark + python-deequ
  negative coverage runs in parallel; non-blocking for the v1.5
  pykrete tag.
- **Audit cycles**: pre-tag 4-audit (architecture, Spark coverage,
  docs sync, pre-launch) + targeted re-audit on PR-F + PR-G per
  v13-rule 6 and v14-rule 11.

## Rules carried in from v1.4 — what worked, what to keep

- **Paste-from-source upstream cites (v14-r1)**: held. Every grammar
  fragment, code anchor, and method-list claim in the v1.5 spec
  cited a source file + line range against a verified commit. PR-B1's
  consumer-surface enumeration in §3.1 caught one missed file
  (`column_methods.rs` Vec construction at multiple sites) before the
  implementation cycle started.
- **Negative-space tests mandatory for new `infer_expr_type` arms
  (v14-r4)**: held. PR-A1, PR-A2, PR-A3, PR-C, PR-B1, PR-B2 each
  shipped its own negative-space block (non-DataFrame receivers,
  Unknown receivers, sibling-arm fall-through). The pattern is now
  load-bearing — when a v1.5 PR landed without one (PR-B2 round 1),
  the reviewer caught it on first read.
- **Probes.py extract, not `grep -c` (v14-r3)**: held. The PR-F
  test-count refresh and the probe-count refresh use
  `python scripts/probes.py extract cross-codebase | jq` (227 total,
  180 positive / 46 negative / 1 file-clean-of). `grep -c PROBE-`
  would have over-counted the multi-marker comment forms.
- **Sibling-arm grep on doc surfaces (v14-r5)**: held but widened.
  PR-F's trust-claim sweep grepped for `production-ready` /
  `reliability` / `trust` / `223 probes` / `83 fixtures` across
  README + docs-site + docs/ and updated all sibling claim-blocks,
  not just the README's. The v1.5 spec's PR-B1 blast-radius
  enumeration was the model: spec authors must list every consumer
  surface, not just the producer file.
- **Single designated extension version bump (v14-r9)**: held. v1.5
  cycle did per-PR patch bumps on the extension; PR-F (this PR) does
  the cycle-close minor bump (`0.2.49 → 0.3.0`).

## New rules from the v1.5 cycle

1. **Spec misnomers carry forward — don't fix mid-cycle.** Spec §2.1
   and §2.2 named `column_exprs::infer_expr_type` as the inference
   site for PR-A1 and PR-A2. Both implementers independently
   discovered the actual sites are `analyze_method_call_inner` +
   `inherited_dialect` (the dual-arm pattern reviewer-verified as
   load-bearing in PR-C). Trying to fix the spec mid-cycle would
   have churned three open PRs. **Rule**: a spec naming misfit
   surfaced mid-cycle becomes a v(N+1) spec amendment in the
   retro, NOT a v(N) spec edit. The misnomer informs v1.6 spec
   authors; the v1.5 PRs still ship to what they actually built.

2. **Dual-arm dispatch is load-bearing for pandas inference,
   document it.** PR-C surfaced that the inference site for nested-
   position `.loc[:, "col"]` requires BOTH the
   `analyze_method_call_inner` arm AND the `inherited_dialect` arm
   to fire — neither alone catches the cases where D0081 / D0082
   fire on the result. The v1.6 spec for any new pandas-dispatched
   inference must call out the dual-arm requirement explicitly, with
   a paste-from-source fragment showing both sites.

3. **Verify CI guard mechanics before citing them.** PR-A1 round-2
   pushback cited the CI guard as per-PR; the actual mechanic is
   since-last-tag. Wrong-pull citations erode reviewer trust faster
   than not citing at all. **Rule**: any "CI will catch this" claim
   in a review thread carries a paste-from-source fragment
   (`.github/workflows/<file>.yml:<lines>`) showing the actual
   trigger and the actual guard expression.

4. **Spec blast-radius defaults to single-file; require explicit
   widening.** Spec §3.1 enumerated PR-B1's blast radius as
   3-file (`col_refs.rs` + `column_methods.rs` +
   `strict_operators.rs`). The actual sweep needed 5 files
   (`expr.rs` + `two_df.rs` also consume the producer tuple type).
   **Rule**: v1.6 spec authors include the sibling-arm grep command
   AND require its output enumeration before committing to the
   blast-radius list. "Verified at spec-write time" is not
   sufficient — verify against the commit the PR will branch off.

5. **Pre-existing false positives get filed, not fixed, in PR-F.**
   PR-C review surfaced a pre-existing false-positive D0030 on
   `pdf.loc` in a nested method-arg position. The PR-F trust-claim
   PR is not the place to fix it — capture in this retro as a
   v1.6 PR-F1-class gate-fix candidate. **Rule**: anything surfaced
   during PR-F review that is (a) pre-existing and (b) not a
   trust-claim integrity blocker gets filed as a v(N+1) candidate,
   not fixed inline.

6. **Atomic pairing on user-facing breaking changes.** Round-2
   TM/user decision moved D0090 strict-mode escalation from v1.5
   to v1.6, paired atomically with `pykrete migrate`. The pairing
   is non-negotiable: no breaking change ships without a one-command
   remediation in the same release. The v1.5 spec §5 records this
   explicitly; v1.6 spec authors must carry it forward (and
   re-confirm at planning-committee time that nothing has changed
   the calculus).

7. **In-flight parallel-codebase PRs go in the trust-claim PR as
   "in flight", not silenced.** PR-G (pykrete-tests dbt-spark +
   python-deequ negative coverage) runs parallel to PR-F. PR-F
   mentions PR-G in the "Coordinated with" section without claiming
   it has landed; the actual landing is on pykrete-tests's release
   line. **Rule**: trust-claim PRs include cross-repo in-flight
   work as a separately-flagged note, never silently bundle.

8. **Spec-PR length is a signal — split if north of ~700 lines.**
   v1.5-spec.md landed at 668 lines and was at the edge of
   reviewable. v1.4's spec was shorter and reviewed faster. v1.6
   spec authors: if the design surface is north of ~700 lines,
   split into a top-level spec + per-pillar deep-dives.

9. **Pre-adoption framing on architecture-audit findings.** PR-E's
   round-2 redesign (graceful degradation vs correctness theater)
   came from re-anchoring on "pykrete is pre-adoption — the
   audit-debt floor is the floor, not a launch blocker." The
   reviewer's correctness-theater alternative (`Arc<str>` pool with
   bounded insertion-ordered eviction) would have widened the PR's
   surface area for no real-user benefit. **Rule**: every
   architecture-audit-driven PR re-confirms its scope against the
   "pre-adoption" framing in the design review; pre-adoption code
   ships the smallest defensible fix, not the most architecturally
   pure.

10. **`probes.py extract` is the canonical probe-count source —
    grep stays banned for counts.** Re-confirms v14-rule 3. PR-F's
    test-count refresh used the structured extract output; sibling
    surfaces (`README.md` reliability block, docs-site splash,
    production-readiness, pykrete-tests page, pandas-roadmap trust
    trajectory) all got the same number from the same source. Any
    "1,312" / "1,312+" / "1,300+" hand-counted references found by
    a sibling-arm grep are explicit grep targets in this cycle
    (v14-rule 5).

## Spec deviations + amendments for v1.6 spec authors

Cited inline in the spec; tagged here for follow-through.

- **Spec §2.1 / §2.2 inference-site naming**. Both arms
  (`analyze_method_call_inner` + `inherited_dialect`) are required;
  `column_exprs::infer_expr_type` was the misnomer. v1.6 spec
  text amendment.
- **Spec §3.1 blast-radius**. 5-file sweep, not 3-file. v1.6 spec
  authors include the sibling-arm grep command in the §-equivalent.
- **Spec §7 extension version-bump strategy**. De-facto pattern is
  per-PR patch bumps culminating in a cycle-close minor bump (v14-
  rule 9). v1.6 spec calls this out explicitly.
- **Pre-existing false-positive D0030 on `pdf.loc` in nested method-
  arg position** (PR-C reviewer): v1.6 PR-F1-class candidate. File
  on issue tracker after v1.5.0 tag if not already.

## Carry-forward to v1.6 planning committee

- **Carry**: trust-claim discipline (atomic per cycle, this PR is
  the template).
- **Carry**: post-release retro is standing practice.
- **Carry**: pre-major-release 4-audit cycle (architecture, Spark
  coverage, docs sync, pre-launch).
- **Carry**: planning-committee protocol (TM + Pragmatist + Strategist
  with reward structure) for major scoping.
- **Carry**: D0090 → error paired with `pykrete migrate` as the
  non-negotiable v1.6 commitment.

## Companion files

- v1.5 spec: `docs/design/v1.5-spec.md`.
- v1.4 retro: not extracted to a standalone file in v1.4; rules carry
  through MEMORY.md and the v1.5 spec.
- Planning-committee protocol: in user MEMORY.md.
