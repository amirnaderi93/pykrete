//! Performance smoke test for `check_project` on **nested** control-flow
//! bodies — the walker's new hot path from v0.1.26.
//!
//! `perf.rs` covers a flat 50-file / 1500-schema project where every
//! function body is one-liner `return df.select(...)`. That misses the
//! whole point of the v0.1.26 walker refactor, which now descends into
//! `if` / `for` / `with` / `try` blocks recursively. This test pins the
//! deeply-nested case so a future refactor that re-introduces quadratic
//! behavior in the walker fails CI before it ships.

use std::time::Instant;

use pykrete::check_project;

/// Build a `(path, source)` pair whose every function body is a 5-level
/// deep nest: `with` ⊃ `try` ⊃ `if` ⊃ `for` ⊃ `while` ⊃ a column
/// reference. Each schema is unique by index so the project-wide schema
/// list grows with the file count.
fn synth_nested_file(file_index: usize, schemas_per_file: usize) -> (String, String) {
    let mut src = String::with_capacity(schemas_per_file * 800);
    for i in 0..schemas_per_file {
        let idx = file_index * 10_000 + i;
        src.push_str(&format!(
            r#"
class Schema{idx}(Schema):
    id_{idx}: int
    amount_{idx}: double
    region_{idx}: string


def deep_{idx}(df: SparkFrame[Schema{idx}], debug: bool, n: int) -> None:
    with open("x") as fh:
        try:
            if debug:
                for i in range(3):
                    while n > 0:
                        df.select(col("id_{idx}"), col("amount_{idx}"), col("region_{idx}")).show()
                        n = n - 1
        except Exception as e:
            df.select(col("region_{idx}")).show()
        finally:
            df.select(col("id_{idx}")).show()


def deep_assign_{idx}(df: SparkFrame[Schema{idx}], debug: bool) -> None:
    if debug:
        out = df.select(col("region_{idx}"), col("amount_{idx}"))
        for _ in range(3):
            with open("y") as gh:
                out.select(col("region_{idx}")).show()
"#,
        ));
    }
    (format!("nested_file_{file_index}.pyk"), src)
}

#[test]
fn nested_control_flow_project_checks_cleanly_and_quickly() {
    // 20 files × 30 schemas × 2 functions = 1200 deeply-nested function
    // bodies. Smaller than the flat perf suite because each body is
    // heavier (5 levels of nesting + several col() refs per level), so
    // the work-per-function ratio is comparable.
    let files: Vec<(String, String)> = (0..20).map(|i| synth_nested_file(i, 30)).collect();
    let started = Instant::now();
    let project = check_project(&files);
    let elapsed = started.elapsed();

    let mut total_diags = 0;
    for file in &project.files {
        total_diags += file.result.diagnostics.len();
    }
    if total_diags != 0 {
        for file in &project.files {
            for d in &file.result.diagnostics {
                eprintln!(
                    "{}:{}:{} {}: {}",
                    file.path, d.line, d.column, d.code, d.message
                );
            }
        }
        panic!("synthetic nested project should be clean; got {total_diags} diagnostics");
    }

    #[cfg(not(debug_assertions))]
    {
        // Same 5 s budget as the flat perf test: this workload is ~13×
        // smaller in function count but ~5× heavier per function, so the
        // total work is comparable. A blow-out here means the walker
        // recursion got accidentally quadratic in nesting depth.
        let budget = std::time::Duration::from_secs(5);
        assert!(
            elapsed < budget,
            "check_project on synthetic nested workload took {elapsed:?}; budget {budget:?}"
        );
    }
    let _ = elapsed;
}
