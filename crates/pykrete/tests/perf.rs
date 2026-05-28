//! Performance smoke test for `check_project`.
//!
//! Generates a synthetic 50-file project (~1500 schemas / 4500 typed
//! functions) and asserts the checker finishes inside a generous wall-clock
//! budget. The intent is not to pin down a precise number — the threshold
//! is loose enough to absorb noisy CI runners — but to catch order-of-
//! magnitude regressions in the hot path that show up when someone adds an
//! accidentally-quadratic walk.
//!
//! Run with `cargo test --release -p pykrete --test perf` for a meaningful
//! wall-clock; the debug build is roughly 30× slower and is skipped via the
//! `#[cfg(not(debug_assertions))]` gate on the timed assertion so the
//! `cargo test --workspace` debug runs don't fail on slow CI hardware.

use std::time::Instant;

use pykrete::check_project;

/// Build a `(path, source)` pair carrying `schemas_per_file` schemas and
/// matching typed functions. Each schema is unique by index so the
/// project-wide schema list grows with the file count.
fn synth_file(file_index: usize, schemas_per_file: usize) -> (String, String) {
    let mut src = String::with_capacity(schemas_per_file * 400);
    for i in 0..schemas_per_file {
        let idx = file_index * 10_000 + i;
        src.push_str(&format!(
            r#"
class Schema{idx}(Schema):
    id_{idx}: int
    name_{idx}: string
    amount_{idx}: double
    region_{idx}: string
    flag_{idx}: bool


def transform_{idx}(df: DataFrame[Schema{idx}]) -> DataFrame[Schema{idx}]:
    return df.select(col("id_{idx}"), col("name_{idx}"), col("amount_{idx}"), col("region_{idx}"), col("flag_{idx}"))


def filter_{idx}(df: DataFrame[Schema{idx}]) -> DataFrame[Schema{idx}]:
    return df.where(col("amount_{idx}") > 100).select(col("id_{idx}"), col("name_{idx}"), col("amount_{idx}"), col("region_{idx}"), col("flag_{idx}"))


def agg_{idx}(df: DataFrame[Schema{idx}]) -> DataFrame[Pick[Schema{idx}, "region_{idx}", "amount_{idx}"]]:
    return df.groupBy(col("region_{idx}")).agg(F.sum(col("amount_{idx}")).alias("amount_{idx}"))
"#,
        ));
    }
    (format!("file_{file_index}.pyk"), src)
}

#[test]
fn synthetic_50_file_project_checks_cleanly_and_quickly() {
    let files: Vec<(String, String)> = (0..50).map(|i| synth_file(i, 30)).collect();
    let started = Instant::now();
    let project = check_project(&files);
    let elapsed = started.elapsed();

    let mut total_schemas = 0;
    let mut total_funcs = 0;
    let mut total_diags = 0;
    for file in &project.files {
        total_schemas += file.result.schema_count;
        total_funcs += file.result.typed_function_count;
        total_diags += file.result.diagnostics.len();
    }
    assert_eq!(total_schemas, 50 * 30, "schema count drifted");
    assert_eq!(total_funcs, 50 * 90, "typed-function count drifted");
    if total_diags != 0 {
        for file in &project.files {
            for d in &file.result.diagnostics {
                eprintln!(
                    "{}:{}:{} {}: {}",
                    file.path, d.line, d.column, d.code, d.message
                );
            }
        }
        panic!("synthetic project should be clean; got {total_diags} diagnostics");
    }

    // Only enforce a wall-clock budget in release builds. Debug builds are
    // ~30× slower and run on every `cargo test --workspace`; pinning them
    // would just produce flaky failures on slow CI hardware. The release
    // budget — 5 s for 1500 schemas / 4500 typed functions — catches an
    // order-of-magnitude regression while staying comfortably above the
    // real ~0.2 s baseline.
    #[cfg(not(debug_assertions))]
    {
        let budget = std::time::Duration::from_secs(5);
        assert!(
            elapsed < budget,
            "check_project on synthetic 50-file workload took {elapsed:?}; budget {budget:?}"
        );
    }
    let _ = elapsed; // silence unused-binding warning in debug builds.
}
