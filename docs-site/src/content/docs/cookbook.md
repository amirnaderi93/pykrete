---
title: Cookbook
description: Realistic recipes for production PySpark teams adopting pykrete — beyond the quickstart's one-function example.
---

The [Quickstart](/pykrete/getting-started/quickstart/) gets you to one checked function. This page covers the next questions: how do I bring pykrete into an existing repo, share schemas across files, re-anchor when the schema is lost, check function signatures, and tune behavior with `pykrete.json`.

The recipes use a small running cast — `Sale`, `Order`, `Refund` — so you can read the page top-to-bottom or dip into a single recipe.

```python
class Sale(Schema):
    region: string
    product: string
    amount: int
    quantity: int
```

## 1. Introduce pykrete to an existing PySpark repo

**Scenario.** You have a working PySpark project. You want to start catching column-name typos without rewriting anything.

**Steps.**

1. [Install pykrete](/pykrete/getting-started/install/).
2. Pick one file whose dataframes you understand. Rename it from `.py` to `.pyk`.
3. Declare a schema for the dataframe the file's main function takes. Annotate the parameter with `SparkFrame[Sale]` (or `PandasFrame[Sale]` if it's a pandas dataframe).
4. Run `pykrete check sales.pyk`. Nothing else in the repo is checked yet.

```python
# sales.pyk — PySpark
class Sale(Schema):
    region: string
    amount: int

def revenue_by_region(sales: SparkFrame[Sale]) -> DataFrame:
    return sales.groupBy("region").agg(F.sum("amount").alias("total"))
```

The same recipe with a pandas dataframe — different annotation, same column-reference story on the operations pykrete dispatches:

```python
# sales.pyk — pandas
import pandas as pd

class Sale(Schema):
    region: string
    amount: int

def positive_sales(sales: PandasFrame[Sale]) -> pd.DataFrame:
    return sales[sales["amount"] > 0][["region", "amount"]]
```

**What you get.** Existence checks ([`D0030`](/pykrete/reference/diagnostics/#unknowncolumn--d0030)) on the column references in the [six dispatched pandas operations](/pykrete/reference/operations/#pandas-dispatch) — `df[col_list]` / `df[mask]` / `df["new"] = expr` / `df.drop` / `df.merge` / `df.rename`. `df.assign(new=expr)` is the kwarg form of `df["new"] = expr` and dispatches identically. Other pandas surface (`.groupby`, `.agg`, `.read_parquet`, window ops) currently falls back to **opaque** in v1.3 — re-anchor with `.cast(PandasFrame[X])` when you need checking to resume. PySpark column references are checked on every operation pykrete models, not just dispatched ones — the per-operation matrix is in the [operations reference](/pykrete/reference/operations/).

The other `.py` files in the project remain unchanged and unchecked — `.py` and `.pyk` coexist in the same repo, and `.pyk` is a strict superset of Python, so the file still runs.

**Pitfall.** pykrete only enters a function when its signature has a `SparkFrame[…]` or `PandasFrame[…]` slot. Untyped helper functions in the same file aren't checked — that's by design, but it surprises people who expect whole-file coverage from the rename alone. (`DataFrame[…]` still works as a deprecated alias and emits [D0090](/pykrete/reference/diagnostics/#deprecateddataframealias--d0090).)

## 2. Re-anchor an opaque `spark.read.*` chain

**Scenario.** Your pipeline starts with `spark.read.parquet(...)`. pykrete can't know what's in the parquet, so downstream column references aren't checked. You want to bring the chain back under checking.

**Steps.**

1. Declare the schema the read produces.
2. Re-anchor the read with `.cast(SparkFrame[Sale])`, or assign to a typed local.
3. Everything after the re-anchor is checked against `Sale`.

```python
class Sale(Schema):
    region: string
    amount: int

def load_and_summarize(spark) -> DataFrame:
    return (
        spark.read.parquet("s3://sales/")    # opaque — schema unknown
        .cast(SparkFrame[Sale])              # re-anchored
        .select("region", "amount")          # checked against Sale
        .groupBy("region")
        .agg(F.sum("amount").alias("total"))
    )
```

Equivalent — a typed local does the same job:

```python
sales: SparkFrame[Sale] = spark.read.parquet("s3://sales/")
sales.select("region", "amount")
```

**What you get.** Every column reference after the re-anchor fires [`D0030`](/pykrete/reference/diagnostics/#unknowncolumn--d0030) on a typo. `.cast(SparkFrame[Sale])` is a static annotation only — at runtime it's an identity no-op.

**Pitfall.** Re-anchor right at the boundary. Anything written between the opaque source and the `.cast(...)` is unchecked. See [`.cast` in the operations reference](/pykrete/reference/operations/#cast--the-re-anchor-primitive).

## 3. Share schemas across files

**Scenario.** Two files use the `Sale` schema. You want it defined in one place.

**Steps.**

1. Put shared schemas in a `schemas.pyk` module at the project root (or any importable location).
2. Import them with normal Python `from … import …` syntax.
3. pykrete resolves `.pyk` imports the same way Python does.

```python
# schemas.pyk
class Sale(Schema):
    region: string
    amount: int

class Refund(Schema):
    region: string
    refund: int
```

```python
# revenue.pyk
from schemas import Sale

def revenue_by_region(sales: SparkFrame[Sale]) -> DataFrame:
    return sales.groupBy("region").agg(F.sum("amount").alias("total"))
```

```python
# refunds.pyk
from schemas import Refund

def total_refunds(refunds: SparkFrame[Refund]) -> DataFrame:
    return refunds.groupBy("region").agg(F.sum("refund").alias("total"))
```

**What you get.** One source of truth for `Sale`. Edit a column there and every file that imports it re-checks against the new shape. An unresolved import fires [`D0070 unresolvedImport`](/pykrete/reference/diagnostics/); importing a name that isn't exported fires [`D0071 unexportedName`](/pykrete/reference/diagnostics/).

**Pitfall.** A `.py` file can't import a schema from a `.pyk` file at check time — pykrete only walks `.pyk`. Schema modules should be `.pyk` if any `.pyk` file imports from them.

## 4. Check a function's signature at the call site

**Scenario.** You have `def summarize(sales: SparkFrame[Sale]) -> SparkFrame[SaleSummary]`. You want pykrete to enforce that callers actually pass a `Sale`-shaped dataframe, and that the body produces a `SaleSummary`-shaped one.

**Steps.**

1. Annotate both the parameter and the return.
2. Call the function from another typed location — a caller with its own `SparkFrame[…]` annotation, or from a chain whose schema pykrete can infer.

```python
class Sale(Schema):
    region: string
    amount: int

class SaleSummary(Schema):
    region: string
    total: long

def summarize(sales: SparkFrame[Sale]) -> SparkFrame[SaleSummary]:
    return sales.groupBy("region").agg(F.sum("amount").alias("total"))

def report(refunds: SparkFrame[Refund]) -> DataFrame:
    return summarize(refunds)   # mismatch
```

**What you get.** Two checks for the price of one annotation:

- The body's output is compared to `SaleSummary`. A drift fires [`D0050 returnColumnsMismatch`](/pykrete/reference/diagnostics/) or [`D0080 returnTypeMismatch`](/pykrete/reference/diagnostics/#type-checking-diagnostics).
- Each call site whose argument has a known schema is compared to `Sale`. A mismatch fires [`D0051 argumentColumnsMismatch`](/pykrete/reference/diagnostics/) with a *missing / extra* breakdown.

**Pitfall.** Arguments whose schema pykrete can't infer (an untyped local, an opaque `spark.read.parquet(...)` chain that isn't re-anchored) are silently skipped — the checker degrades rather than false-flag. Re-anchor the caller's argument with `.cast(SparkFrame[Sale])` if you want D0051 to fire there.

## 5. Configure `pykrete.json` for an existing codebase

**Scenario.** You're rolling pykrete out across a large project. The strict checks are noisy on legacy code, but you want the workhorse rules at full strength on a new subdirectory.

**Steps.**

1. Add a `pykrete.json` at the project root. Exclude generated and vendored paths. Downgrade noisy rules to warnings while you clear them.
2. Add a second `pykrete.json` inside the new subdirectory with stricter settings. pykrete uses the nearest config walking up from the file being checked.

```json
// pykrete.json (project root)
{
  "typeCheckingMode": "standard",
  "exclude": ["target", ".venv", "generated"],
  "rules": {
    "unionSchemaMismatch": "warning",
    "returnTypeMismatch": "warning"
  }
}
```

```json
// pipelines/new_etl/pykrete.json
{
  "typeCheckingMode": "strict",
  "rules": {
    "unionSchemaMismatch": "error",
    "returnTypeMismatch": "error"
  }
}
```

**What you get.** Files under `pipelines/new_etl/` get strict-mode advisories ([`D0081`](/pykrete/reference/diagnostics/#type-checking-diagnostics), [`D0082`](/pykrete/reference/diagnostics/#type-checking-diagnostics), [`D0083`](/pykrete/reference/diagnostics/#type-checking-diagnostics)) plus the rules at error severity. Files elsewhere get standard mode with the two named rules downgraded to warnings. The language server reads the same files — the editor and CI agree.

**Pitfall.** `exclude` is a list of path substrings, not glob patterns. `"target"` matches `crates/target/`, `target/release/`, and any path containing the string. Be specific enough to avoid accidental matches. See the full reference: [Configuration](/pykrete/reference/configuration/).

## 6. Migrate `DataFrame[X]` to the v2.0 dialect-tagged names

**Goal.** The `DataFrame[X]` alias is deprecated through the v1.x line and removed in v2.0. Replace every site with the dialect-tagged canonical name (`SparkFrame[X]` or `PandasFrame[X]`) before the v2.0 upgrade — or before flipping `"typeCheckingMode": "strict"`, where v1.6+ escalates [`D0090`](/pykrete/reference/diagnostics/#deprecateddataframealias--d0090) to error.

**Step 1 — preview.** `pykrete migrate --check src/` walks every `.pyk` file under `src/`, applies call-graph dialect adjudication to each binding's downstream usage, and prints the file + line of every site that would be rewritten:

```bash
$ pykrete migrate --check src/
src/sales.pyk:5:20: would rewrite to SparkFrame[Sale]
src/sales.pyk:5:40: would rewrite to SparkFrame[Sale]
src/pivot.pyk:11:18: would rewrite to PandasFrame[Order]
src/util.pyk:7:14: ambiguous — needs human review (mixed Spark/pandas usage)
```

Exit code is 1 if any site needs attention (including ambiguous sites), 0 otherwise — drop it into CI to gate merges. Per-site lines go to stdout, so the standard `pykrete migrate --check src/ > report.txt` redirect works. The `--diff src/` variant prints a `patch -p1`-compatible unified diff for review; ambiguous sites in `--diff` show up as an inserted `# pykrete: ambiguous` marker (no tautological rewrite hunks).

**Step 2 — rewrite.** `pykrete migrate src/` does the work in place. Adjudication picks per-site:

- Binding used with Spark-only methods (`withColumn`, `createOrReplaceTempView`, `repartition`, …) → `SparkFrame[X]`.
- Binding used with pandas-only methods (`assign`, `pivot_table`, `.loc`, `.iloc`, `merge`, …) → `PandasFrame[X]`.
- Binding used with *both* dialect-discriminating signals → ambiguous: the rewrite is skipped, the source text stays `DataFrame[X]`, and a `# pykrete: ambiguous` marker is inserted on the line above so you know to adjudicate by hand.
- Binding with no discriminating signal (an unused parameter, a pure return slot, …) → defaults to `SparkFrame[X]`.

```pyk
# Before
def revenue(sales: DataFrame[Sale]) -> DataFrame[Sale]:
    return sales.withColumn("total", sales.qty * sales.unit_price)
```

```pyk
# After (pure-Spark usage → SparkFrame)
def revenue(sales: SparkFrame[Sale]) -> SparkFrame[Sale]:
    return sales.withColumn("total", sales.qty * sales.unit_price)
```

```pyk
# Mixed usage → ambiguous, stays DataFrame with a marker for review
# pykrete: ambiguous
def either(df: DataFrame[Sale]) -> int:
    df.withColumn("a", 1)           # Spark
    return df.assign(b=2)           # pandas
```

**Step 3 — verify.** Re-run `pykrete check src/` (or `pykrete migrate --check src/`). Both should exit 0 once every site is migrated, including under strict mode.

**Pitfall — ambiguous sites are real signal.** A binding used as both a Spark and a pandas dataframe almost always means the code is wrong (the two dialects don't share an API surface; one branch will fail at runtime). The migrator leaves them alone deliberately. Decide which dialect that path takes and pick the right annotation by hand.

## See also

- [Operations](/pykrete/reference/operations/) — every PySpark op pykrete recognizes, and where chains end.
- [Diagnostics](/pykrete/reference/diagnostics/) — every rule, with examples.
- [Schemas](/pykrete/reference/schemas/) — `Pick`, `Omit`, `Merge`, nested types.
- [Configuration](/pykrete/reference/configuration/) — every key in `pykrete.json`.
