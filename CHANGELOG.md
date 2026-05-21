# Changelog

All notable changes to pykrete are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/), and pykrete adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.4]

### Changed

- VS Code extension displayName temporarily set to
  "Pykrete — Static schema checking for PySpark" to bypass the Visual
  Studio Marketplace's post-deletion reservation on the name "Pykrete".
  Will be reverted to plain "Pykrete" once the reservation clears.

No checker, LSP, or transpiler behavior changed from 0.1.3.

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

[Unreleased]: https://github.com/amirnaderi93/pykrete/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/amirnaderi93/pykrete/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/amirnaderi93/pykrete/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/amirnaderi93/pykrete/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/amirnaderi93/pykrete/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/amirnaderi93/pykrete/releases/tag/v0.1.0
