# Packaging & releasing pykrete

How a pykrete release is cut and how each distribution channel is fed.

## Cutting a release

1. Bump `version` in the workspace `Cargo.toml` (`[workspace.package]`),
   run `cargo build` so `Cargo.lock` picks it up, and add a `CHANGELOG.md`
   entry.
2. Commit, merge to `main`.
3. Tag and push:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
4. The [`release.yml`](../.github/workflows/release.yml) workflow builds
   release binaries for macOS (arm64 + x64) and Linux (x64), packages each
   as a tarball, and publishes them — with `.sha256` checksum files — on a
   GitHub Release for the tag.

## Channels

### Prebuilt binaries

Produced automatically by `release.yml`. Each tarball contains the
`pykrete` and `pykrete-lsp` binaries plus `README.md` and `LICENSE`.

### Homebrew

The formula lives in [`homebrew/pykrete.rb`](homebrew/pykrete.rb). It
installs the prebuilt binaries, so it has no Rust build dependency.

One-time tap setup: create a GitHub repository named `homebrew-pykrete`
and copy `homebrew/pykrete.rb` into it as `Formula/pykrete.rb`. Users then:

```bash
brew install amirnaderi93/pykrete/pykrete
```

After every release, update `version` and the three `sha256` values in the
formula. The hashes are the first field of the `*.tar.gz.sha256` files
attached to the GitHub Release.

### cargo install

pykrete depends on ruff's parser crates via a pinned git revision, and
Astral does not publish those to crates.io — so pykrete itself cannot be
published to crates.io. Installing from the git source works regardless:

```bash
cargo install --git https://github.com/amirnaderi93/pykrete pykrete
cargo install --git https://github.com/amirnaderi93/pykrete pykrete-lsp
```

### VS Code Marketplace

The extension lives in [`../editors/vscode/`](../editors/vscode/) and is
already marketplace-ready (`publisher: pykrete`). To publish:

1. Register the `pykrete` publisher at <https://marketplace.visualstudio.com/manage>.
2. From `editors/vscode/`:
   ```bash
   npm install
   npx vsce publish
   ```

A JetBrains/PyCharm plugin is planned and will use the JetBrains
Marketplace once it exists.
