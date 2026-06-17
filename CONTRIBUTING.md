# Contributing to pykrete

pykrete is a static schema checker for PySpark and pandas, written in Rust. This guide covers everything you need to get set up, run tests, and submit changes. If anything here is unclear or missing, open an issue.

## Getting set up

### Prerequisites

- **Rust ≥ 1.95**. Install via [rustup](https://rustup.rs); your stable toolchain works. CI pins an exact version (currently 1.95.0) so `cargo fmt --check` and `cargo clippy -D warnings` stay reproducible — CI is the authoritative gate.
- **Git**.
- The Rust analyzer extension for your editor (VS Code: `rust-lang.rust-analyzer`).

### Clone and build

```bash
git clone https://github.com/amirnaderi93/pykrete.git
cd pykrete
cargo build
```

The first build pulls Astral's `ruff_python_parser` from GitHub (we depend on it via a pinned git tag — Astral doesn't publish ruff's internal crates to crates.io). Expect ~5–10 minutes on first build. Incremental builds are fast.

### Run the checker

```bash
cargo run -- check examples/schemas.pyk
```

### Run the test suite

```bash
cargo test            # all crates in the workspace
cargo test -p pykrete  # just the checker library + CLI
cargo test -p pykrete-lsp  # just the LSP server
```

All tests should pass on a clean checkout. If any fail on your machine before you've made changes, that's a bug — please open an issue.

### Run the LSP server (manual smoke)

```bash
cargo run -p pykrete-lsp --bin pykrete-lsp
```

stdin/stdout speak LSP JSON-RPC. For actual editor integration, see the README.

## Project layout

```
crates/pykrete/
├── src/
│   ├── main.rs           # CLI shell — calls into the library
│   ├── lib.rs            # `pykrete::check(path, source) -> CheckResult`
│   ├── diagnostics.rs    # Diagnostic types + TS-style formatting
│   ├── schema.rs         # Schema discovery, SchemaView, field resolution
│   ├── types.rs          # ColumnType atoms (Int, String, Date, …)
│   ├── dataframe.rs      # SparkFrame/PandasFrame/DataFrame annotation recognition
│   ├── walk.rs           # Top-level AST walks (classes, functions)
│   ├── registry.rs       # Class + constant registries (for generics)
│   ├── transpiler.rs     # .pyk → .py emit
│   └── operations/       # Body analysis, result-schema inference,
│       ├── mod.rs        #   chain tracking, return-type checks
│       ├── driver.rs     # Top-level statement walker (per function body)
│       ├── expr.rs       # analyze_expr / analyze_method_call dispatch
│       ├── col_refs.rs   # col() reference discovery + F.* allowlist
│       ├── column_methods.rs   # df.<method>(...) handlers
│       ├── column_exprs.rs     # Column-expression result-type inference
│       ├── two_df.rs     # join / union / unionByName / set ops
│       ├── strict_operators.rs # D0081 / D0082 / D0083 type checks
│       ├── context.rs    # BodyContext — bindings, aliases, traces
│       └── shapes.rs     # AST shape recognizers (reader chains, etc.)
└── tests/
    ├── common/mod.rs     # Shared test helpers
    └── *.rs              # Integration tests — one file per feature area

crates/pykrete-lsp/
├── src/
│   ├── main.rs           # LSP binary — thin shell over the library
│   └── lib.rs            # Server loop + diagnostic conversion

docs/                     # Spec, architecture, and design notes
examples/                 # Sample `.pyk` files
```

[`docs/design/architecture.md`](docs/design/architecture.md) is the authoritative reference for how the analyzer is structured. Read it before making non-trivial changes.

## Workflow

pykrete uses a **feature-branch + pull-request** workflow with a strict commit format. All of this exists so the git history stays legible as the project grows and so an external contributor can land changes without needing direct access to `main`.

### Branch naming

Branches are named `<type>/<short-kebab-case>`. `<type>` is one of:

| type | when |
| --- | --- |
| `feat` | new user-visible functionality |
| `fix` | bug fix |
| `docs` | docs only |
| `test` | tests only |
| `refactor` | code reorg with no behavior change |
| `perf` | performance improvement, no behavior change |
| `style` | formatting, whitespace |
| `chore` | tooling, deps, repo plumbing |
| `build` | build system changes |
| `ci` | CI changes |
| `revert` | reverting a prior commit |

Examples: `feat/dotted-column-access`, `fix/d0030-message-formatting`, `docs/contributing-guide`, `chore/bump-ruff`.

### Commit messages — Conventional Commits

Every commit message uses the [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<type>(<optional-scope>): <short imperative subject>

<body — what and why, wrap at ~72 chars>
```

- `<type>` matches the branch types above.
- `<scope>` is optional and names the affected module (`operations`, `schema`, `cli`, etc.).
- Subject is imperative mood ("add", "fix", "rename" — not "added"), under 72 characters, no trailing period.
- Body explains **why** the change is needed; the diff explains the what.

Example:

```
feat(operations): infer schema through dotted col() paths

Previously, col("address.street") failed D0030 because has_field treated
the dotted string as a single flat name. The new resolve_path helper
walks each segment: for non-final segments it looks up the field, asserts
the field's type is a nested Schema, and recurses with the remainder of
the path.

Closes #N
```

If a commit closes an issue, end with `Closes #N` or `Fixes #N`. Co-authored work uses standard `Co-Authored-By:` trailers.

### Version bumps (v1.10+; standing)

Version bumps are **centralized in the cycle-close PR (PR-F)** — workspace `Cargo.toml` and `editors/vscode/package.json` + `package-lock.json` are both bumped once, by PR-F, at the end of the cycle. Per-PR developers do NOT bump versions; an extension-version-guard CI check enforces this. The mechanic exists to eliminate the per-PR-bump rebase ladder that hit v1.7 and v1.8. The v1.9 trial succeeded (zero rebase-ladder collisions across Wave 1); v1.10 promotes the practice to standing. See `docs/design/v1.10-spec.md` §10.2 for the standing rule and the marker-mechanism escape hatch (`.github/centralized-bump-cycle.marker`) for cycles that need per-PR bumps.

### Submitting a change

1. Branch off `main`: `git checkout -b feat/your-change`.
2. Make commits, **one logical change per commit** (small, reviewable diffs).
3. Run `cargo test` and `cargo fmt --all` before pushing.
4. Push the branch: `git push -u origin feat/your-change`.
5. Open a Pull Request on GitHub — via the URL git prints on push, or with `gh pr create`. The [PR template](.github/pull_request_template.md) is filled in for you.
6. CI runs automatically. Make sure it's green before requesting review.
7. Once approved, merge with a **merge commit** (not squash/rebase) so the branch's history is preserved as a side-track in `main`:
   ```bash
   git checkout main && git merge --no-ff feat/your-change
   ```
   (GitHub's "Create a merge commit" option does the same.)

## Code style

- **Formatting**: `cargo fmt --all` before committing. CI enforces this.
- **Idioms**: prefer iterators over indexed loops; prefer `match` over chained `if let`; use `?` for error propagation.
- **Naming**: snake_case for items, PascalCase for types. Test function names read as sentences describing the scenario (`d0030_fires_when_select_references_unknown_column`).
- **Comments**: no comments that just restate the code; add comments where a future reader would ask "why".

## Adding a test

Every new feature should ship with both **unit tests** (inside the module, under `#[cfg(test)]`) and **integration tests** (in `crates/pykrete/tests/<feature>.rs`).

Integration tests use the helpers in `tests/common/mod.rs`:

```rust
mod common;
use common::*;

#[test]
fn my_new_check_fires_on_the_obvious_bad_case() {
    let result = check(r#"
class Orders(Schema):
    place_code: int

def f(raw: SparkFrame[Orders]) -> SparkFrame[Orders]:
    return raw.select(col("typo"))
"#);
    assert_has_code(&result, "D0030");
    assert_message_contains(&result, "D0030", "typo");
}
```

Conventions:
- One test per assertion. Don't combine "checks two things and three other things" — each scenario is its own `#[test]`.
- Name the test as a sentence describing what it verifies.
- Add a brief comment above the test if the scenario isn't self-evident.

## CHANGELOG conventions

When the CHANGELOG quotes a string that comes from the pykrete binary — a stderr warning, a stdout-emitted text, a CLI help fragment — wrap it in a fenced code block labeled `stderr`, `stdout`, or `text`. Example for a stderr warning:

    ```stderr
    pykrete: migrate default is now --check; pass --apply to rewrite in place (v1.7+)
    ```

The CI step `scripts/changelog-grep.sh` (v1.8 PR-A2; closes v1.7 retro rule 6) verifies that any string inside one of those fenced blocks appears in `crates/pykrete/src/`. If it doesn't, CI fails with a `MISMATCH` line naming the offending CHANGELOG entry.

This catches the v1.7-class drift where a binary-emitted string quoted in CHANGELOG silently diverges from the actual code. **Inline single-backtick quotes are NOT checked by the gate** — fence the block (with `stderr` / `stdout` / `text`) to opt in. Other fence labels (`python`, `bash`, `rust`, no label) are ignored.

### Numeric trust-claim verification (`text-numeric` blocks, v1.9+)

For trust-claim numbers — probe counts, fixture counts, test totals, donor counts — wrap them in a `text-numeric` block. Each line is `<number> <key>` where `<key>` is one of the known claims below. The gate runs the live-extract command for the key and fails if the live value differs from the claimed number.

    ```text-numeric
    255 probes
    114 fixtures
    1650 tests
    17 donors
    ```

Known claim keys (defined in `scripts/changelog-grep.sh::numeric_claim_command`):

| Key | Live-extract command |
|---|---|
| `probes` | `python3 ../pykrete-tests/scripts/probes.py extract ../pykrete-tests/cross-codebase \| jq '.probes \| length'` |
| `fixtures` | `find ../pykrete-tests/cross-codebase \( -path '*annotated*' -name '*.pyk' -o -path '*probes_negative*' -name '*.pyk' \) \| wc -l` |
| `tests` | `cargo test --release --workspace 2>&1 \| grep -oE '[0-9]+ passed' \| awk '{s+=$1} END {print s}'` (release-gate workflow memoizes via `PYKRETE_TESTS_COUNT_FILE`) |
| `donors` | `find ../pykrete-tests/cross-codebase -maxdepth 1 -mindepth 1 -type d \| wc -l` |
| `positive` | `... probes.py extract ... \| jq '[.probes[] \| select(.kind != "EXPECTS")] \| length'` |
| `negative` | `... probes.py extract ... \| jq '[.probes[] \| select(.kind == "EXPECTS")] \| length'` |

Unknown keys fail with `MISMATCH: ... unknown numeric-claim key: '<key>'`. To add a new key, extend `numeric_claim_command` in `scripts/changelog-grep.sh` in the same PR that adds the CHANGELOG line using it (mirrors the sibling-arm-grep discipline elsewhere in the codebase).

PR CI runs the gate with `--skip-live-extract` (the `pykrete-tests` sibling repo isn't checked out beside `pykrete` on the GitHub Actions runner). The skip mode still validates fenced-block syntax + the known-key allowlist; only command execution is bypassed. Release-time runs use the dedicated `release-gate.yml` workflow (push to `release/v*`, PR labeled `release-ready`, or manual `workflow_dispatch`) which checks out `pykrete-tests` as a sibling and runs the full gate with live extracts.

This catches the v1.8-class drift where a trust-claim number quoted in CHANGELOG (e.g. "106 fixtures") silently diverges from the live extract (which was 112) — closes v1.8 retro rule 7.

#### Historical pin labels (`text-numeric-historical`, v1.10+)

Once `pykrete-tests` ships a new probe/fixture after a `pykrete` tag, the live counts drift above the pinned trust-claim numbers in the released-CHANGELOG section. The pinned numbers are correct AT THE TAG (release-pinned, immutable) but no longer match live. The release-cycle convention is: at v1.10 release time (PR-F), the v1.9.0 section's `text-numeric` block gets relabeled to `text-numeric-historical`. The gate parses but does NOT live-verify `text-numeric-historical` blocks — they render normally and skip the gate. Digits inside historical blocks are also stripped before the prose scan, so they can't trip gate v3 either.

    ```text-numeric-historical
    255 probes
    114 fixtures
    ```

Mid-cycle, if `pykrete-tests` releases a probe-adding PR before the current cycle's `pykrete` release lands (the v1.9 / v1.10 window saw this with pykrete-tests PR-P1 bumping 255 → 261 probes pre-tag), relabel the prior-release block early — the gate would otherwise fail on the drift.

### Prose-paragraph numeric scan (gate v3, v1.10+)

Gate v3 extends the live-extract verification to **prose paragraphs** — text outside fenced blocks. Any prose digit-sequence followed (after whitespace) by a known claim key is verified against the same live-extract table. This catches the v1.9 PR-F drift class where "183 positive + 72 negative" landed in a paragraph trust-claim, OUTSIDE any fenced block, and the v2 gate missed it.

Concretely: `Cross-codebase coverage lifts to 255 probes (185 positive + 70 negative).` is a prose claim and is gated automatically.

The regex is leading-word-boundary anchored — digits immediately following an identifier prefix (e.g. `0091` inside `D0091 probes`) are rejected so D-code mentions don't trip the gate.

**Escape hatch — single-backtick wrap**: if you need to mention a number that should NOT be auto-verified (a historical number from a prior release, a per-PR contribution count from a prior cycle, a number quoted from external documentation, an inline code example), wrap it in single backticks:

```
The v1.8 pin had `183 positive` probes; v1.9 lifted to 185 positive.
```

The scanner skips matches inside single-backtick spans. The `185 positive` outside the backticks is still verified.

#### Final semantic — when does the gate verify a number?

The full rule, after R2 of v1.10 PR-A2:

- **`[Unreleased]` section + the current release's `text-numeric` (NON-historical) fenced block**: gated against live. PR-F at cycle close updates this block to live counts at tag time.
- **Older `text-numeric-historical` fenced blocks**: skipped by gate. Relabeled from `text-numeric` to `text-numeric-historical` either at the NEXT release's PR-F, or earlier if upstream `pykrete-tests` drifts the live counts forward mid-cycle.
- **Prose**: gated against live unless backtick-wrapped. Backtick-wrap covers historical claims ("the v1.8 pin had `183 positive`"), per-PR contribution counts ("`1 negative` probe each on pandera and delta"), and any other legitimate edge case where a number should NOT chase live.

**Scope boundary**: only `CHANGELOG.md`. README, docs-site prose, and other Markdown surfaces are out of scope (different drift profile).

### File:line cite convention (`scripts/changelog-cite-check.sh`, v1.11+)

CHANGELOG citations may use bare basenames (e.g., `alias_report.rs:446`); the cite-check resolves these via single-match search across `crates/`, `scripts/`, `editors/`, `.github/`. Ambiguous matches fail loudly. Qualified paths (e.g., `crates/pykrete/src/main.rs:972`) are also accepted and resolved literally.

## Filing issues

Pick the matching template when you open a new issue on GitHub (defined under `.github/ISSUE_TEMPLATE/`):

- **Bug**: a reproducible incorrect behavior.
- **Feature**: a new check, operation, or capability.

Include a minimal `.pyk` snippet whenever possible — that's the fastest path to a fix.

## Releases

pykrete follows [semantic versioning](https://semver.org/).

**§9.2 — Centralized version bumps (standing practice as of v1.10, promoted from the v1.9 trial).** Per-PR developers DO NOT bump versions during a cycle. All version bumps — workspace `Cargo.toml`, `editors/vscode/package.json`, lockfiles — happen in the cycle's release PR (conventionally PR-F). This eliminates rebase-ladder collisions across parallel feature PRs (the v1.9 trial recorded zero collisions across Wave 1 and was promoted to standing in v1.10).

The mechanism is a `.github/centralized-bump-cycle.marker` file committed at the start of the cycle (typically by PR-S1) and removed in the release PR. The [`extension-version-guard.yml`](.github/workflows/extension-version-guard.yml) workflow checks the marker presence to decide whether to enforce the per-PR bump rule or wave it through for the cycle. See lines 82–99 of that workflow for the gate logic. The marker mechanism stays in the guard workflow post-cycle as a reusable cycle-trial primitive for future amendments.

**To cut a release**: in the release PR, bump `version` in the workspace `Cargo.toml` and `editors/vscode/package.json`, regenerate lockfiles, remove the `.github/centralized-bump-cycle.marker` file, add a `CHANGELOG.md` entry, commit, merge, then `git tag vX.Y.Z && git push origin vX.Y.Z`. The [release workflow](.github/workflows/release.yml) builds binaries for macOS (arm64/x64) + Linux x64 + Windows (MSI), attaches the `.vsix`, publishes the extension to the Visual Studio Marketplace and Open VSX, and bumps the Homebrew tap — all automatically. See [`packaging/README.md`](packaging/README.md) for the full pipeline.

### During PR-F: Trust-claim sweep checklist

Before opening PR-F, run `bash scripts/trust-claim-sweep-checklist.sh --current-version X.Y.Z` to catch prior-release number leaks across trust-claim surfaces (README, docs-site, editors/vscode/README.md, pykrete-tests/README.md). This is the structural closure for the 5-cycle PR-F-miscount pattern (v1.6/v1.7/v1.8/v1.9/v1.10). The gate also runs automatically in the `release-gate.yml` workflow when the PR is labeled `release-ready`.
