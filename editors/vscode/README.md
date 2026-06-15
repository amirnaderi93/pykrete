# pykrete for VS Code

**Static type checking for dataframe schemas — in your editor.**

[![Open VSX](https://img.shields.io/open-vsx/v/amirnaderi/pykrete?label=Open%20VSX)](https://open-vsx.org/extension/amirnaderi/pykrete)

pykrete is a strict superset of Python that adds a type layer for dataframes. This extension brings its schema checks into VS Code (and Cursor, VSCodium, code-server, Theia): live diagnostics, hover, completion, go-to-definition, and quick-fixes on `.pyk` files. New to pykrete? Start with the [documentation](https://amirnaderi93.github.io/pykrete/).

<p align="center">
  <img src="https://raw.githubusercontent.com/amirnaderi93/pykrete/main/editors/vscode/images/showcase-column-typos.png" alt="pykrete catching a misspelled column name in the editor — diagnostic: 'regoin' does not exist on schema 'Sale'. Did you mean 'region'?" width="720">
  <br>
  <sub><em>A column typo, caught at edit time.</em></sub>
</p>

<p align="center">
  <img src="https://raw.githubusercontent.com/amirnaderi93/pykrete/main/editors/vscode/images/showcase-schema-flow.png" alt="pykrete tracks the schema through a transformation chain — hovering the result shows the derived schema (region: string, total: long)." width="720">
  <br>
  <sub><em>The schema flows through every transform — hover the result to see the derived shape.</em></sub>
</p>

## What it gives you

**Column typos, caught as you type.** Every column reference — `col("x")`, `df.x`, `df["x"]`, dotted paths into nested structs, string arguments to functions like `F.sum("x")` — is checked against the schema in scope. A misspelling gets a red underline and a *did you mean*.

**Checks that follow the data.** pykrete tracks the schema through `select`, `filter`, `withColumn`, `drop`, `join`, `groupBy` + `agg`, `pivot`, `union`, and the rest. Reference a column three transforms after it was dropped, and the squiggle lands exactly where you used it.

**Hover to see a schema.** Hover a `SparkFrame[…]` or `PandasFrame[…]` parameter and see its columns without leaving the file. Go-to-definition jumps to the schema declaration. (`DataFrame[…]` is a deprecated alias for `SparkFrame[…]` and renders the same hover.)

**Cross-dialect handoff (v1.5).** `df.toPandas()` re-tags `SparkFrame[X]` to `PandasFrame[X]`, so a downstream `pdf["typo"]` still gets the squiggle. `spark.createDataFrame(pdf)` re-tags back when a `schema=` keyword or a typed call-arg resolves to a known schema. Pandas `.head(10).merge(...)` keeps tracking (dialect-gated terminals), and `pdf.loc[:, "col"]` literal-form lands too.

**`pykrete migrate` + D0090 strict-mode escalation (new in v1.6).** `pykrete migrate src/` rewrites the deprecated `DataFrame[X]` alias to `SparkFrame[X]` or `PandasFrame[X]` based on call-graph dialect adjudication — each binding's downstream usage is inspected for Spark-only versus pandas-only methods, and mixed-dialect bindings get a `# pykrete: ambiguous` marker for hand review. Paired atomically with D0090 escalating from warning to **error** under `"typeCheckingMode": "strict"` — strict-mode projects get the fix-button in the same release as the breaking-change signal. Pandas `pivot_table(index=, columns=, values=, aggfunc=)` literal-form column checking ships too. v1.6 also closes the `.take()` dialect-gate (`pdf.take([0, 2]).merge(...)` keeps tracking) and the `pdf.loc[mask, "col"]` nested-arg false positive.

**Migrator `--check` default + pandas `melt` + Spark-D1 closure (new in v1.7).** v1.6 shipped `pykrete migrate src/` as an in-place rewrite default; v1.7 flips that to `--check` (preview verdicts on stdout; exit 1 if any site needs attention). `--apply` is the new opt-in for the in-place rewrite. A first-run on v1.7 with no flag emits a one-line stderr warning so the change is hard to miss. Pandas `df.melt(id_vars=, value_vars=, var_name=, value_name=)` literal-form column checking ships as the v1.7 reshape downpayment — typo in any string-literal argument fires D0030 with a *did you mean*. The v1.6 architecture-audit Important #3 closes with a shared `dialect_signals` module + a CI-guard test; 14 Spark-only methods (`selectExpr`, `freqItems`, `approxQuantile`, `crosstab`, `colRegex`, `summary`, `mapInPandas`, `mapInArrow`, `writeTo`, `writeStream`, `unpivot`, `rdd`, `isStreaming`, `sparkSession`) get added to the discriminator list — `corr` / `cov` deliberately excluded for pandas collision risk. `pykrete migrate` parse-error skips now surface on stderr; CRLF marker normalization lands for Windows source files. Internal audit-debt mop-up: dead `_source: &str` param dropped from the migrate helpers; two-vector lockstep loop in the migrate driver collapses to single-pass.

**Autocomplete for column names.** Type a column name in a string argument and pykrete completes the ones that actually exist on the dataframe in scope.

**Quick-fixes.** When pykrete flags an unknown column with a *did you mean* suggestion, the lightbulb action swaps in the closest matching name.

**Full Python support, included.** The extension bundles a Python language server, so you also get ordinary Python hover, completion, go-to-definition, find-references, and type diagnostics — for free, in the same extension. Nothing else to install, no `files.associations`, no configuration.

## Requirements

The extension needs the `pykrete-lsp` binary on your `PATH`. Install it via any of:

- **Homebrew** (macOS / Linux): `brew install amirnaderi93/pykrete/pykrete`
- **Windows**: download the MSI from the [latest release](https://github.com/amirnaderi93/pykrete/releases/latest)
- **From source** (Rust ≥ 1.95): `cargo install --git https://github.com/amirnaderi93/pykrete pykrete pykrete-lsp`

Each installs both `pykrete` and `pykrete-lsp`. Homebrew and the MSI put the binary on your `PATH` automatically. Full options in the [install guide](https://amirnaderi93.github.io/pykrete/getting-started/install/).

The bundled Python language server runs on **Node.js**. If `node` isn't on your `PATH`, pykrete's schema features still work fully — only the general Python features are unavailable.

## Install the extension

- **VS Code** — search **pykrete** in the Extensions panel, or run `code --install-extension amirnaderi.pykrete`.
- **Cursor / VSCodium / code-server / Theia** — search **pykrete** in the Extensions panel (served from the Open VSX Registry).
- **Offline / locked-down environments** — every [pykrete release](https://github.com/amirnaderi93/pykrete/releases) attaches a `.vsix`; install it with **Extensions panel → ⋯ → Install from VSIX…**

Open a `.pyk` file and the checks start immediately. Have existing PySpark `.py` files? Rename one to `.pyk` — it's a strict superset of Python, so the file still runs unchanged. The [quickstart](https://amirnaderi93.github.io/pykrete/getting-started/quickstart/) walks through it in five minutes.

## Settings

| Setting | Purpose |
|---|---|
| `pykrete.serverPath` | Path to the `pykrete-lsp` binary. Defaults to discovering it on `PATH` (and the workspace `target/` directory, for contributors). |
| `pykrete.pythonServer.path` | Path to a `basedpyright-langserver` / `pyright-langserver` binary, to use instead of the bundled Python engine. |

Project behavior — type-checking strictness, excluded paths, per-rule severity — is configured with a `pykrete.json` file; see [Configuration](https://amirnaderi93.github.io/pykrete/reference/configuration/).

## Links

- [Documentation](https://amirnaderi93.github.io/pykrete/)
- [Source & issues](https://github.com/amirnaderi93/pykrete)
- [Changelog](CHANGELOG.md)

## Development

To build the extension from source:

```sh
npm install        # also fetches the bundled Python engine (~40 MB)
npm run compile    # one-shot; `npm run watch` rebuilds on save
npx vsce package   # produces a .vsix
```

During development the extension finds `pykrete-lsp` in the workspace's `target/release/` directory — run `cargo build --release -p pykrete-lsp` from the repo root first.

MIT licensed.
