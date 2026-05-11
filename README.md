# dathon

A strict superset of Python that adds static schema checking for dataframes. Inspired by TypeScript's relationship to JavaScript.

**Status:** pre-implementation. Designing v0.1.

## What it does

- Define schemas as Python classes.
- Annotate dataframes with their schema: `DataFrame[MySchema]`.
- Catch column-name typos, schema drift, and shape mismatches at check time, not in production.
- Transpile to plain Python — runtime cost is zero.

## Project layout

- [docs/v0.1-spec.md](docs/v0.1-spec.md) — the contract for the first usable version.
- [docs/language-reference/](docs/language-reference/) — user-facing reference (grows as features land).
- [docs/design/](docs/design/) — internal design and implementation docs (grows as we build).
- [examples/](examples/) — sample `.dpy` files for poking at the checker.

## Initial target

PySpark. The author's production PySpark codebase is the real-world testing yardstick for v0.1.

## License

TBD.
