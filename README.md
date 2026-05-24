<p align="center"><img src="docs/assets/logo.svg" width="150" alt="pykrete logo"></p>

<h1 align="center">pykrete</h1>

<p align="center">
  <strong>Python dataframes, done right.</strong><br>
  Static type checking for dataframe schemas.<br>
  <sub>PySpark today (feature-complete); pandas and polars next.</sub>
</p>

<p align="center">
  <a href="https://github.com/amirnaderi93/pykrete/actions/workflows/ci.yml"><img src="https://github.com/amirnaderi93/pykrete/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/amirnaderi93/pykrete-tests/actions/workflows/check.yml"><img src="https://github.com/amirnaderi93/pykrete-tests/actions/workflows/check.yml/badge.svg" alt="pykrete-tests"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
</p>

<p align="center">
  <a href="https://amirnaderi93.github.io/pykrete/"><strong>Docs</strong></a> ·
  <a href="https://amirnaderi93.github.io/pykrete/getting-started/install/">Install</a> ·
  <a href="https://amirnaderi93.github.io/pykrete/getting-started/quickstart/">Quickstart</a> ·
  <a href="docs/roadmap.md">Roadmap</a>
</p>

---

## The bug that reaches production

Rename a column. Mistype it once, three transforms downstream. Nothing stops you — not the interpreter, not the linter, not your tests, unless one happens to assert on that exact name. The job runs, returns an empty dataframe or a column of nulls, and you find out hours later in a scheduled run or a dashboard that quietly went blank.

pykrete catches it before that. `sales.pyk` below is a strict superset of Python — the `Schema` class and the `DataFrame[Sale]` annotation are ordinary Python the runtime ignores; pykrete reads them as types.

```python
class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int

def revenue_by_region(sales: DataFrame[Sale]) -> DataFrame:
    return sales.groupBy("regoin").agg(F.sum("amount").alias("total"))
```

```console
$ pykrete check sales.pyk
sales.pyk:8:26 - error unknownColumn: Column 'regoin' does not exist on schema 'Sale'. Did you mean 'region'?
```

The CLI runs in CI; with the [VS Code extension](#editor-integration), the same error appears as a red squiggle under `regoin` as you type — no command needed.

Annotate a dataframe with its schema — a `Schema` class plus a `DataFrame[Sale]` parameter — and pykrete checks every column you touch through the whole transformation chain. It's TypeScript's idea, applied to dataframes: a type layer the runtime ignores, a checker that runs at edit time. If you've used [Pandera](https://pandera.readthedocs.io/), this is its edit-time counterpart — Pandera validates dataframes when your job runs; pykrete checks them before it does.

Atomic types (`string`, `int`, `long`, `double`, `bool`, `date`, `timestamp`) and nested arrays / maps / structs are in the [Schemas reference](https://amirnaderi93.github.io/pykrete/reference/schemas/). The [full showcase](https://amirnaderi93.github.io/pykrete/) walks through autocomplete, hover, schema flow, and the rest.

## What you get

- **Typos caught as you type** — every column reference, against the schema in scope, with a *did you mean*.
- **Checks that follow the data** — `select` / `filter` / `withColumn` / `drop` / `join` / `groupBy` + `agg` / `pivot` / `union` and the rest; a reference to a column three transforms after it was dropped is still caught.
- **Schema visibility** — hover a `DataFrame[…]` parameter to see its columns; go-to-definition jumps to the schema.
- **Column-name autocomplete** in string arguments.
- **Inline SQL checked too** — identifiers inside `filter("…")`, `selectExpr(...)`, `spark.sql("SELECT …")`.
- **Zero runtime cost** — `.pyk` is a strict superset of Python; the deployed job is ordinary Python.

## Quickstart

Install — macOS / Linux:

```bash
brew install amirnaderi93/pykrete/pykrete
```

Windows: the [latest release](https://github.com/amirnaderi93/pykrete/releases/latest) ships an MSI installer. Other options — prebuilt binaries, `cargo install` — are in the [install guide](https://amirnaderi93.github.io/pykrete/getting-started/install/).

Then convert one file — rename `sales.py` to `sales.pyk` (it still runs unchanged), add a `Schema` class, annotate one function with `DataFrame[Schema]`, and check it:

```bash
pykrete check sales.pyk
```

The rest of your repo stays plain Python — pykrete only checks the functions you've annotated. The [quickstart](https://amirnaderi93.github.io/pykrete/getting-started/quickstart/) walks through it in five minutes.

## Editor integration

The **VS Code extension** gives you live diagnostics, hover, completion, and go-to-definition on `.pyk` files. Search **pykrete** in the Extensions panel — it's on the [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=amirnaderi.pykrete) (VS Code) and the [Open VSX Registry](https://open-vsx.org/extension/amirnaderi/pykrete) (Cursor, VSCodium, code-server, Theia).

For Neovim, Helix, Emacs, and other LSP clients, see [docs/editors/](docs/editors/).

## Documentation

Full documentation — schema reference, the diagnostic catalog, how it works, the roadmap — is at **[amirnaderi93.github.io/pykrete](https://amirnaderi93.github.io/pykrete/)**.

PySpark is supported today; pandas and polars are next. See the [roadmap](docs/roadmap.md).

## Repository layout

- [`crates/`](crates/) — the Rust workspace: `pykrete` (checker + CLI) and `pykrete-lsp` (language server).
- [`editors/vscode/`](editors/vscode/) — the VS Code extension.
- [`docs/`](docs/) — design docs, the roadmap, and the source for the docs site.
- [`docs-site/`](docs-site/) — the Astro + Starlight documentation site.
- [`examples/`](examples/) — sample `.pyk` files.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow — feature branches, Conventional Commits, CI, and the pull-request process.

## License

MIT. See [LICENSE](LICENSE).

## Built with Claude

pykrete is developed with [Claude Code](https://claude.com/claude-code), Anthropic's agentic coding tool.
