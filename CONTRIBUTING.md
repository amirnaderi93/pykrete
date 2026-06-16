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

### Version bumps (v1.9+)

For the v1.9 cycle, version bumps are **centralized in the cycle-close PR (PR-F)** — workspace `Cargo.toml` and `editors/vscode/package.json` + `package-lock.json` are both bumped once, by PR-F, at the end of the cycle. Per-PR developers do NOT bump versions; an extension-version-guard CI check enforces this. The mechanic exists to eliminate the per-PR-bump rebase ladder that hit v1.7 and v1.8. See `docs/design/v1.9-spec.md` §9.2 for the trial details and revert path; v1.10 will keep or revert based on the trial outcome.

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

## Filing issues

Pick the matching template when you open a new issue on GitHub (defined under `.github/ISSUE_TEMPLATE/`):

- **Bug**: a reproducible incorrect behavior.
- **Feature**: a new check, operation, or capability.

Include a minimal `.pyk` snippet whenever possible — that's the fastest path to a fix.

## Releases

pykrete follows [semantic versioning](https://semver.org/). To cut a release: bump `version` in the workspace `Cargo.toml`, add a `CHANGELOG.md` entry, commit, then `git tag vX.Y.Z && git push origin vX.Y.Z`. The [release workflow](.github/workflows/release.yml) builds binaries for macOS (arm64/x64) + Linux x64 + Windows (MSI), attaches the `.vsix`, publishes the extension to the Visual Studio Marketplace and Open VSX, and bumps the Homebrew tap — all automatically. See [`packaging/README.md`](packaging/README.md) for the full pipeline.
