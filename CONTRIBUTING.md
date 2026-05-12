# Contributing to dathon

dathon is a static schema checker for PySpark, written in Rust. This guide covers everything you need to get set up, run tests, and submit changes. If anything here is unclear or missing, open an issue.

## Getting set up

### Prerequisites

- **Rust ≥ 1.95**. Install via [rustup](https://rustup.rs). The toolchain version is pinned in `rust-toolchain.toml`; cargo will use it automatically once rustup is installed.
- **Git**.
- The Rust analyzer extension for your editor (VS Code: `rust-lang.rust-analyzer`).

### Clone and build

```bash
git clone https://gitlab.com/amir.naderi93/dathon.git
cd dathon
cargo build
```

The first build pulls Astral's `ruff_python_parser` from GitHub (we depend on it via a pinned git tag — Astral doesn't publish ruff's internal crates to crates.io). Expect ~5–10 minutes on first build. Incremental builds are fast.

### Run the checker

```bash
cargo run -- check examples/schemas.dpy
```

### Run the test suite

```bash
cargo test
```

All tests should pass on a clean checkout. If any fail on your machine before you've made changes, that's a bug — please open an issue.

## Project layout

```
crates/dathon/
├── src/
│   ├── main.rs           # CLI shell — calls into the library
│   ├── lib.rs            # `dathon::check(path, source) -> CheckResult`
│   ├── diagnostics.rs    # Diagnostic types + TS-style formatting
│   ├── schema.rs         # Schema discovery, SchemaView, field resolution
│   ├── types.rs          # ColumnType atoms (Int, String, Date, …)
│   ├── dataframe.rs      # DataFrame[Schema] annotation recognition
│   ├── walk.rs           # Top-level AST walks (classes, functions)
│   ├── registry.rs       # Class + constant registries (for generics)
│   └── operations.rs     # Body analysis, result-schema inference,
│                         #   chain tracking, return-type checks
└── tests/
    ├── common/mod.rs     # Shared test helpers
    └── *.rs              # Integration tests — one file per feature area

docs/                     # Spec + architecture + design notes
examples/                 # Sample `.dpy` files
```

[`docs/design/architecture.md`](docs/design/architecture.md) is the authoritative reference for how the analyzer is structured. Read it before making non-trivial changes.

## Workflow

dathon uses a **feature-branch + Merge-Request** workflow with a strict commit format. All of this exists so the git history stays legible as the project grows and so an external contributor can land changes without needing direct access to `main`.

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

### Submitting a change

1. Branch off `main`: `git checkout -b feat/your-change`.
2. Make commits, **one logical change per commit** (small, reviewable diffs).
3. Run `cargo test` and `cargo fmt --all` before pushing.
4. Push the branch: `git push -u origin feat/your-change`.
5. Open a Merge Request on GitLab — either via the URL printed by git on push, or via `git push -o merge_request.create=true ...`. Fill out the [MR template](.gitlab/merge_request_templates/Default.md).
6. CI runs automatically. Make sure it's green before requesting review.
7. Once approved, merge with **no fast-forward** so the branch's history is preserved as a side-track in `main`:
   ```bash
   git checkout main && git merge --no-ff feat/your-change
   ```
   (The GitLab UI defaults to this for non-trivial branches.)

## Code style

- **Formatting**: `cargo fmt --all` before committing. CI enforces this.
- **Idioms**: prefer iterators over indexed loops; prefer `match` over chained `if let`; use `?` for error propagation.
- **Naming**: snake_case for items, PascalCase for types. Test function names read as sentences describing the scenario (`d0030_fires_when_select_references_unknown_column`).
- **Comments**: no comments that just restate the code; add comments where a future reader would ask "why".

## Adding a test

Every new feature should ship with both **unit tests** (inside the module, under `#[cfg(test)]`) and **integration tests** (in `crates/dathon/tests/<feature>.rs`).

Integration tests use the helpers in `tests/common/mod.rs`:

```rust
mod common;
use common::*;

#[test]
fn my_new_check_fires_on_the_obvious_bad_case() {
    let result = check(r#"
class Orders(Schema):
    place_code: int

def f(raw: DataFrame[Orders]) -> DataFrame[Orders]:
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

## Filing issues

Use the appropriate template under `.gitlab/issue_templates/`:

- **Bug**: a reproducible incorrect behavior.
- **Feature**: a new check, operation, or capability.

Include a minimal `.dpy` snippet whenever possible — that's the fastest path to a fix.

## Releases

When v0.1 ships we'll tag `v0.1.0`. Going forward, releases follow [semantic versioning](https://semver.org/) with notes in `CHANGELOG.md`.
