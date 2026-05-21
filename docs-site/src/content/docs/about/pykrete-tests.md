---
title: Real-codebase tests
description: How pykrete is tested against snapshots of Apache Spark and MLflow — what we annotate, what we catch, what gets fixed upstream.
---

[pykrete-tests](https://github.com/amirnaderi93/pykrete-tests) is a separate repo that vendors snapshots of well-known PySpark codebases (Apache Spark, MLflow), adds pykrete annotations the way a real adopter would, and runs `pykrete check` on every push and nightly. It exists for two reasons:

1. **Regression coverage** — catch behavior changes in pykrete's checker as new operations are modeled.
2. **Trust signal** — demonstrate that pykrete keeps real PySpark code diagnostic-free under realistic annotation, not just on the toy examples in `tests/`.

## How it works

Each vendored codebase gets a top-level directory:

```
spark/
├── upstream/        # vendored .pyk files (renamed verbatim from upstream .py)
├── annotated/       # the same files with pykrete annotations added
├── pinned-commit    # the exact upstream SHA we vendored from
├── LICENSE-UPSTREAM # the upstream's license, preserved verbatim
└── RESULTS.md       # findings, including any pykrete gaps surfaced

mlflow/
├── ...
```

`.py` files become `.pyk` by simple rename — pykrete is a strict superset of Python, so the upstream code is unchanged. Annotations live alongside as separate `Schema` classes and typed helper functions, the way a real user adopting pykrete in their codebase would.

CI runs `pykrete check` on every `**/annotated/**/*.pyk` file. pykrete itself is built fresh from `main` each run, so any regression in pykrete shows up here before it gets released.

## What v0.1.6 fixed

The pilot loop surfaced five real pykrete gaps; all of them shipped as fixes in v0.1.6.

| Pilot | File | Gap surfaced | pykrete commit |
|---|---|---|---|
| 1 | Spark `examples/.../basic.py` | `df["X"]` subscript wasn't recognized as a column ref | [`483cc09`](https://github.com/amirnaderi93/pykrete/commit/483cc09) |
| 2 | Spark `tests/.../test_group.py` | GroupedData shortcut aggregates (`g.max("col")`) didn't check args | [`c25fe5c`](https://github.com/amirnaderi93/pykrete/commit/c25fe5c) |
| 3 | MLflow `tests/.../test_spark_datasource_autologging.py` | `intersect`/`subtract`/`exceptAll` weren't modeled (`union` was) | [`d68d1e2`](https://github.com/amirnaderi93/pykrete/commit/d68d1e2) |
| 4 | Spark `tests/.../test_column.py` | Chained Column-on-Column nested-field access (`df.r.X`) skipped | [`0b70d9c`](https://github.com/amirnaderi93/pykrete/commit/0b70d9c) |
| 5 | Spark `examples/.../arrow.py` | Lowercase `groupby` alias not recognized | [`9a49bf6`](https://github.com/amirnaderi93/pykrete/commit/9a49bf6) |

Each fix shipped with regression tests in the pykrete crate, so the same gap can't reopen silently.

## Methodology per pilot

The same five-step loop every time:

1. **Vendor** the upstream file at a pinned commit, preserving the license.
2. **Annotate** — add `Schema` classes and `DataFrame[Schema]` annotations on representative functions. Helpers extract the dataframe-typed cores from test methods that take `self` rather than `DataFrame[X]`, since pykrete only enters body analysis on the latter.
3. **Run** `pykrete check` — should report `0 issues` on the unmodified annotated file (no false positives).
4. **Probe** — plant deliberate typos (`"plcae_code"`, `"vlaue"`, etc.) to verify pykrete actually catches what it should. Any miss is a gap.
5. **Fix** upstream in pykrete, add regression tests, re-run probes, update the per-codebase `RESULTS.md`.

Detailed per-file findings: [Spark RESULTS.md](https://github.com/amirnaderi93/pykrete-tests/blob/main/spark/RESULTS.md), [MLflow RESULTS.md](https://github.com/amirnaderi93/pykrete-tests/blob/main/mlflow/RESULTS.md).
