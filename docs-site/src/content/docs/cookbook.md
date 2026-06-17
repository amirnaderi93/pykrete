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

> **v1.7 default-mode flip.** In v1.6 the default mode of `pykrete migrate src/` was the in-place rewrite. v1.7 flips that to `--check` (preview only). The in-place rewrite is now `pykrete migrate --apply src/`. If you have a CI job or shell alias that ran `pykrete migrate src/` and expected an in-place rewrite, switch it to `pykrete migrate --apply src/`. A first-run on v1.7 with no flag emits a one-line stderr warning so the change is hard to miss.

**Step 1 — preview.** `pykrete migrate src/` (v1.7+ default) walks every `.pyk` file under `src/`, applies call-graph dialect adjudication to each binding's downstream usage, and prints the file + line of every site that would be rewritten. (On v1.6 use `pykrete migrate --check src/` for the same behavior.) The walker reads `.pyk` files only — if your project uses `.py` files via the multiplexer integration, copy or rename them to `.pyk` before running `pykrete migrate`:

```bash
$ pykrete migrate src/
src/sales.pyk:5:20: would rewrite to SparkFrame[Sale]
src/sales.pyk:5:40: would rewrite to SparkFrame[Sale]
src/pivot.pyk:11:18: would rewrite to PandasFrame[Order]
src/util.pyk:7:14: ambiguous — needs human review (mixed Spark/pandas usage)
```

Exit code is 1 if any site needs attention (including ambiguous sites), 0 otherwise — drop it into CI to gate merges. Per-site lines go to stdout, so the standard `pykrete migrate src/ > report.txt` redirect works. The `pykrete migrate --diff src/` variant prints a `patch -p1`-compatible unified diff for review; ambiguous sites in `--diff` show up as an inserted `# pykrete: ambiguous` marker (no tautological rewrite hunks). Files that fail to parse are skipped (v1.7+ reports each skipped file with its parse error on stderr so you can see why a file didn't get migrated).

**Step 2 — rewrite.** `pykrete migrate --apply src/` does the work in place. Adjudication picks per-site:

- Binding used with Spark-only methods (`withColumn`, `createOrReplaceTempView`, `repartition`, `selectExpr`, `mapInPandas`, `writeStream`, …) → `SparkFrame[X]`.
- Binding used with pandas-only methods (`assign`, `pivot_table`, `melt`, `.loc`, `.iloc`, `merge`, …) → `PandasFrame[X]`.
- Binding used with *both* dialect-discriminating signals → ambiguous: the rewrite is skipped, the source text stays `DataFrame[X]`, and a `# pykrete: ambiguous` marker is inserted on the line above so you know to adjudicate by hand. (v1.7+ normalizes the marker's line ending to match the rest of the file — CRLF on Windows, LF on Unix — so the file stays mixed-EOL-free.)
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

**Step 3 — verify.** Re-run `pykrete check src/` (or `pykrete migrate src/` to re-run check-mode). Both should exit 0 once every site is migrated, including under strict mode.

**Step 4 — gate CI on the inventory (v1.8+, simplified in v1.10).** `pykrete check --deprecation-report src/` emits a JSON envelope listing every D0090-firing site with its adjudicated dialect and suggested rewrite. v1.10 adds `--fail-on-nonempty` so the CI gate is a single flag rather than a shell pipeline:

```bash
pykrete check --deprecation-report --fail-on-nonempty src/
```

`--fail-on-nonempty` exits non-zero when the envelope's `sites` array is non-empty (it still prints the JSON to stdout, so you can capture it on failure for the build log). It replaces the v1.8–v1.9 `jq | test` boilerplate adopters were writing by hand. Compatible with `--ack` (gates only on the filtered cohort) and with `--snapshot=<path>` (the gate decision is independent of the file write).

The envelope's shape (v1.9+) is `{deprecationReportVersion: "2", sites: [...], summary: {totalSites, byDialect: {spark, pandas, ambiguous}}}`. Per-site fields include `file`, `line`, `column`, `code` (always `D0090`), `ruleName`, `bindingName`, `rawAnnotation`, `adjudicatedDialect`, `suggestedRewrite` (null for ambiguous sites), and `migrationStatus` (`"pending"` or `"acknowledged"`). Mutually exclusive with `--report-aliases` (passing both exits 2). See the [D0090 diagnostics reference](/pykrete/reference/diagnostics/#deprecateddataframealias--d0090) for the full schema.

**Step 5 — site-by-site gating with `--ack` (v1.9+).** A full all-or-nothing CI gate is unrealistic for large codebases. v1.9 adds per-site acknowledgement: drop a `# pykrete: ack-deprecation` comment on the line above an annotation to flip its `migrationStatus` from `pending` to `acknowledged`, then filter the envelope with `--ack=<pending|acknowledged>` to gate one cohort at a time.

```pyk
# pykrete: ack-deprecation
def revenue(sales: DataFrame[Sale]) -> DataFrame[Sale]:
    ...
```

```bash
# Fail CI on any unacked D0090 site:
pykrete check --deprecation-report --ack=pending --fail-on-nonempty src/

# Inverse: catch regressions where a site flipped acked → pending (the
# marker was removed or the annotation moved):
pykrete check --deprecation-report --ack=acknowledged src/ > acked.json
```

The two-step workflow lets a team land migration in waves: acknowledge an alias site as "we know about this; the migration is intentional and tracked", and the CI gate stops blocking on it while still failing on any site that hasn't been adjudicated yet. The envelope deliberately ships **without** `targetVersion` / `removalVersion` / `shipDate` — pykrete tracks per-site migration progress; you pick the v2.0 ship date.

**Step 6 — snapshot the envelope across releases (v1.10+).** Migration is rarely a single-PR landing; it stretches across release windows. v1.10 adds `--snapshot=<path>` so the v2 envelope can be written to disk as a release-pinned artifact — your CI persists it as a build output (or commits it to a tracking branch) and later compares against the prior snapshot to confirm progress.

```bash
# Write the v2 envelope to disk:
pykrete check --deprecation-report --snapshot=migration.json src/

# Pair with --ack to snapshot only the unacked cohort:
pykrete check --deprecation-report --ack=pending --snapshot=pending-migration.json src/
```

`--snapshot` performs an atomic write — tempfile-plus-rename in the destination directory, nanosecond-suffixed temp name to avoid concurrent-writer collision, cleanup-on-error guard across every error path — so a half-written `migration.json` never lands on disk. Exit code stays at 0 on a successful write (gating lives on `--fail-on-nonempty`; you can combine both: `--snapshot=migration.json --fail-on-nonempty`). The persisted file is bit-identical to what `--deprecation-report` would have printed to stdout, so a `diff` between two release snapshots is the same diff a script would compute over the live invocations. Snapshot-vs-snapshot comparison via a built-in `--compare-to <snapshot>` flag is tracked for v1.11+; until then, `diff -u previous.json migration.json | jq ...` is the manual primitive.

**Pitfall — ambiguous sites are real signal.** A binding used as both a Spark and a pandas dataframe almost always means the code is wrong (the two dialects don't share an API surface; one branch will fail at runtime). The migrator leaves them alone deliberately. Decide which dialect that path takes and pick the right annotation by hand.

## See also

- [Operations](/pykrete/reference/operations/) — every PySpark op pykrete recognizes, and where chains end.
- [Diagnostics](/pykrete/reference/diagnostics/) — every rule, with examples.
- [Schemas](/pykrete/reference/schemas/) — `Pick`, `Omit`, `Merge`, nested types.
- [Configuration](/pykrete/reference/configuration/) — every key in `pykrete.json`.
