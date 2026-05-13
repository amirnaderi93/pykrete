# Dathon VS Code extension

Thin client that launches `dathon-lsp` and routes `.dpy` files to it.

## Two ways to use it

### Default — dathon-only

Install the extension. Open a `.dpy` file. You get dathon's dataframe-specific
features: diagnostics, hover on `class X(Schema)` / `DataFrame[X]` /
`col("foo")`, completion inside `col("…")` / `df.` / `DataFrame[…]`,
go-to-definition, "Did you mean?" quick-fixes.

Standard Python features (general syntax highlighting, std-lib completion,
references, rename, formatting, type errors on `def f(`, etc.) are NOT
provided in this mode — dathon-lsp only knows about dataframe-specific
positions and stays silent on everything else.

### Recommended — co-activation with a Python LSP

Because `.dpy` is a strict superset of Python, the same files work as
input to any Python language server. Pair this extension with a Python
LSP and you get **every Python feature on `.dpy` files** plus dathon's
schema checks layered on top.

1. **Install a Python LSP extension**, in order of preference:

   - **[basedpyright](https://marketplace.visualstudio.com/items?itemName=detachhead.basedpyright)**
     (recommended) — free, MIT, no telemetry, type-checker.
   - **[Pylance](https://marketplace.visualstudio.com/items?itemName=ms-python.vscode-pylance)**
     — Microsoft's, fastest type-checker, requires the Python extension.
   - **[ruff-lsp](https://marketplace.visualstudio.com/items?itemName=charliermarsh.ruff)**
     — minimal, lint-focused; pair with one of the above if you want
     type checking.

2. **Tell VS Code that `.dpy` files are Python**. Add to your user
   `settings.json`:

   ```json
   "files.associations": {
     "*.dpy": "python"
   }
   ```

3. **Drop `python_stubs/dathon.py` into your project root** (or anywhere
   on the Python module-search path). dathon's magic names — `Schema`,
   `DataFrame`, `col`, `string`, `date`, `timestamp`, `double`, `long` —
   aren't real Python identifiers; the stub provides Pylance-friendly
   declarations for them so the type-checker doesn't flag every Schema
   class as "undefined name." dathon itself ignores the import.

4. **Add a one-line import at the top of every `.dpy` file**:

   ```python
   from dathon import Schema, DataFrame, col, string, date, timestamp, double
   ```

   (Cherry-pick names per file as needed.) Examples are in
   [`examples/pipelines/`](../../examples/pipelines/) — the
   schemas.dpy / dal.dpy / example_job.dpy fixtures all use the
   pattern.

5. Open a `.dpy` file. Both servers run side-by-side:

   - The Python LSP handles general Python features.
   - dathon-lsp handles `DataFrame[X]`, `col("…")`, Schema classes,
     return-type validation, "Did you mean?" suggestions.

   VS Code merges their responses automatically.

If your Python LSP still complains about specific dathon idioms after
the stub is in place, the most common knobs are:

- **`from pyspark.sql.functions import col` shows red** — PySpark isn't
  installed in the active Python interpreter. `pip install pyspark` in
  the project's venv, or silence with `"reportMissingImports": "none"`
  (basedpyright) for `.dpy` files.
- **`DataFrame[X]` triggers "Unknown" warnings** — the stub's
  `DataFrame` is a generic, so basedpyright's stricter rules like
  `reportUnknownArgumentType` may complain. Add `"reportUnknownArgumentType": "none"`
  in your project's basedpyright config to silence.

## Develop

```sh
npm install
npm run compile         # one-shot
npm run watch           # rebuild on save
```

## Package + install locally

```sh
npx vsce package --allow-missing-repository
code --install-extension dathon-vscode-0.1.0.vsix
```

To pick up code changes after reinstall, reload the VS Code window
(`Developer: Reload Window`).

## Server discovery

The extension looks for the `dathon-lsp` binary in this order:

1. `dathon.serverPath` setting, if set.
2. `<workspace>/target/release/dathon-lsp`.
3. `<workspace>/target/debug/dathon-lsp`.
4. `dathon-lsp` on `PATH`.

Run `cargo build --release -p dathon-lsp` from the workspace root before
opening a `.dpy` file.
