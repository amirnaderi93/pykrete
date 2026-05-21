# Changelog

## 0.2.6

Tracks the v0.1.6 pykrete-lsp release — five real-codebase fixes from
the pykrete-tests integration loop. No editor-side behavior changes;
the LSP gains four new D0030 diagnostic shapes (`df["X"]` subscript,
GroupedData shortcut aggregates, `intersect`/`subtract`/`exceptAll`,
chained nested-field access, lowercase `groupby` alias). See the
[main CHANGELOG](../../CHANGELOG.md#016) for the breakdown.

## 0.2.5

- Temporary displayName change to bypass the Visual Studio Marketplace's
  post-deletion reservation on the name "Pykrete". The new displayName
  is "Pykrete — Static schema checking for Python" — library-agnostic
  so it doesn't pre-commit the brand to PySpark (pandas and polars are
  planned). Will be reverted to plain "Pykrete" once the reservation
  clears.

## 0.2.4

Cancelled before publishing — see 0.2.5.

## 0.2.3

- Republish under the `amirnaderi` publisher and drop the redundant
  `-vscode` suffix from the package name. New marketplace IDs:
  `amirnaderi.pykrete` on both the Visual Studio Marketplace and Open
  VSX. The old `pykrete.pykrete-vscode` listings will be removed.

## 0.2.2

- `.pyk` files now show the pykrete logo in the file explorer (used as
  the language icon when the active icon theme doesn't have one).

## 0.2.1

First marketplace release.

- Adds the marketplace icon and logo.
- Tracks the v0.1.x pykrete-lsp ABI.

Earlier 0.1.x and 0.2.0 development builds were distributed as local
`.vsix` files only.
