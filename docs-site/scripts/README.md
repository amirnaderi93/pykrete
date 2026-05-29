# docs-site scripts

## `gen-pyspark-symbols.py`

Generates `public/pyspark-symbols.json` by introspecting a real
`pyspark` install. The playground reads the JSON at runtime to drive
Monaco autocomplete and hover for Spark APIs (DataFrame, Column,
GroupedData, Window, `pyspark.sql.functions`).

CI does not run this — the generated JSON is committed. Re-run it
manually when bumping the pinned pyspark version.

```bash
cd docs-site
python3 -m venv .venv-symbols
. .venv-symbols/bin/activate
pip install -r scripts/requirements.txt
python3 scripts/gen-pyspark-symbols.py
deactivate
```

The script writes to `docs-site/public/pyspark-symbols.json`. Commit
the result.
