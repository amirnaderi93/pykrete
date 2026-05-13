# Roadmap

What's planned beyond v0.1, in rough priority order. This document is a living plan; it's updated as the project moves.

## After v0.1

Items the v0.1 spec defers, in the order we'll likely tackle them after the v0.1 release tag:

### Real import-statement support

Iteration 31 shipped strict per-file scoping driven by `from X import Y [as Z]` clauses (relative `from .X import Y`, absolute `from pkg.X import Y`, and `as` aliases). Project root is detected via `pyproject.toml` (longest-common-ancestor fallback). `dathon check` accepts directory paths and walks them recursively for `.dpy` files. Missing imports surface as `D0020`; unresolved module paths as `D0070`; names not exported by a module as `D0071`. Still pending:

- `import X` and qualified access (`X.Y`) — currently parsed but `X.Y` references don't resolve.
- Wildcard imports (`from X import *`) — parsed but no-op; consider emitting a warning or expanding.
- Duplicate-name detection / warnings across files.
- Incremental rechecking — today every `dathon check` re-reads and reparses every file in the project.

### LSP — features beyond skeleton + hover

Iterations 24–32 shipped the LSP skeleton (live diagnostics via `textDocument/publishDiagnostics`), hover (Schema declarations, typed function declarations, Schema references in annotations, `col("foo")` literals, and `x = raw.select(...)` local-variable bindings on both their LHS and uses elsewhere), document symbols, go-to-definition for Schema references in `DataFrame[X]` / nested-struct annotations and for `col("foo")` literals, completion at three surfaces (`col("…")`, `df.` for typed params *and* local-variable bindings, `DataFrame[…]`), and "Did you mean?" suggestions on `D0030` — surfaced both in the diagnostic message and as a `textDocument/codeAction` quick-fix that replaces the literal in place. Still pending:

- **Completion / hover on chain results** — `raw.select(...).<>` doesn't yet trigger column-name completion. Needs the same logic that backs `x.<>` for locals, but applied to the smallest enclosing call's result schema rather than a name lookup.
- **Find references**, **rename**, **semantic tokens** as further iterations.

### VS Code extension

Shipped (iteration 27) — TypeScript wrapper at [editors/vscode/](../editors/vscode/) that launches `dathon-lsp` and routes `.dpy` files to it. Currently distributed as a local `.vsix` (`npx vsce package`); marketplace publishing is still pending.

### Editor-agnostic LSP docs

Setup snippets for Neovim, Helix, Zed, Emacs to plug `dathon-lsp` into their LSP clients.

## Generic-inference extensions

Iteration 21 introduced generic-function inference for the simplest shape: a single type variable `T` appearing in both a parameter slot `GenericClass[T]` and a return slot `GenericClass[T]`. Real-world generic patterns often want more. Listed here so they're not forgotten:

### Multiple type parameters

```python
def join[A, B](left: DataFrame[A], right: DataFrame[B]) -> DataFrame[Joined[A, B]]: ...
```

Today: only one type variable per generic-method call is bound. Two-param methods don't infer at all.

### Nested generics

```python
def lift[T](xs: List[DataSource[T]]) -> List[DataFrame[T]]: ...
```

Today: dathon's matcher only handles one level of subscript (`G[T]`). Nested forms (`List[DataSource[T]]`) aren't recognized.

### Chained class-method calls

```python
return builder.with_path("/x").read[T](RAW_ORDERS)
```

Today: only direct method calls on a class-instance name are dispatched through the generic-inference path. A method call whose receiver is itself a call result (the `builder.with_path(...)` here) isn't treated as a class instance, so the outer `.read(...)` is skipped.

### Class-level constants

```python
class DataSources:
    RAW_ORDERS: DataSource[RawOrders] = DataSource("/path")
```

Today: only **module-level** annotated constants are tracked in the constants registry. Class-attribute constants (the more common pattern in the user's real codebase) aren't yet picked up. Need to walk class bodies for `AnnAssign` and qualify the names (`DataSources.RAW_ORDERS`).

### Generic methods that aren't `[T] -> G[T]`-shaped

```python
def cast_to[T](self, _: type[T]) -> DataFrame[T]: ...
```

Methods that take a *type* as a value-level argument (e.g. `df.cast_to(RawOrders)`) need a different inference path — the type variable is bound from a value whose static type is `type[T]`, not from a `G[T]`-shaped slot.

## Quality-of-life items

- **Better error messages with hints** — "Did you mean 'X'?" suggestions on `D0030` via Levenshtein distance over the schema's field names.
- **`dathon.json` config file** — strictness modes, file/dir excludes, per-rule severity overrides.
- **`cargo install` packaging + Homebrew tap** — easier local install than `cargo run --`.
- **Performance pass** — benchmark on large codebases; explore parallel file checks once multi-file support lands.

## Beyond v0.1 — strategic direction

These are larger structural moves, not iterations. They shape what dathon becomes once the v0.1 surface is solid.

### Full Python LSP feature parity

Because `.dpy` is a strict superset of Python, the LSP should offer **everything a regular Python LSP does** — syntax highlighting, completions on standard library symbols, references, go-to-definition for non-dataframe code, formatting, etc. dathon-specific checks (`DataFrame[X]`, `col(...)`, `Schema` classes) sit on top of that base. Today, opening a `.dpy` in VS Code gives dathon's dataframe diagnostics but nothing for regular Python — that's a worse experience than just writing `.py`.

Direction: don't reimplement a Python LSP. Piggyback on an existing one (Pylance, basedpyright, pyright, ruff-lsp). The VS Code extension can either:

- **Co-activation**: require users to install a Python extension alongside dathon, route `.dpy` files through both, let the Python extension handle general features while dathon-lsp handles dataframe-specific ones. Cheap.
- **Multiplex inside dathon-lsp**: embed/proxy an existing Python LSP behind dathon's stdio interface and merge responses. "Right" but probably overkill for the size of this project.

Co-activation is the v0.1.x target; multiplexing is a follow-up if friction shows up.

### Multi-dataframe support (pandas, polars, …)

PySpark is the v0.1 target, but every popular dataframe library has the same fundamental shape: a value carries a schema, methods narrow or widen that schema, column names must exist on the schema when referenced. Real data engineering work mixes Spark with pandas in the same job (the [example `example_job.py`](design-notes.md) does `.toPandas()` → process → `spark.createDataFrame()`); polars is rising fast. Schema checking is valuable for *every* dataframe library.

Priority order: **PySpark → pandas → polars** → others (DuckDB, Dask, …) as they show up.

The core type model — `DataFrame[Schema]`, `Schema` class, column reference checks, return-type validation — should generalize across libraries. The library-specific layer handles method dispatch:

- Spark: `raw.select(col("x"))`, `raw.filter(col("a") > 0)`, `raw.groupBy(...).agg(...)`.
- pandas: `raw[["x"]]`, `raw.loc[raw.a > 0]`, `raw.groupby(...).agg(...)`.
- polars: `raw.select(pl.col("x"))`, `raw.filter(pl.col("a") > 0)`, `raw.group_by(...).agg(...)`.

This argues for moving toward a plugin/dispatch model for method handling before too much PySpark-specific code accumulates in `operations.rs`. Probably the right time to do this is **before** pandas support lands, while the surface is still small.
