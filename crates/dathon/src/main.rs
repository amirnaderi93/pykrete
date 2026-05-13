//! Thin CLI shell. All of the analysis lives in the dathon library; this
//! binary parses arguments and dispatches to `dathon::check_project` (the
//! analyzer) or `dathon::transpile` (the `.dpy` → `.py` emitter).

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("check") if args.len() >= 3 => run_check(&args[2..]),
        Some("transpile") if args.len() == 3 => run_transpile(&args[2]),
        _ => {
            eprintln!("usage:");
            eprintln!("  dathon check <file-or-dir> [<file-or-dir> ...]");
            eprintln!("  dathon transpile <file.dpy>");
            ExitCode::from(2)
        }
    }
}

fn run_check(paths: &[String]) -> ExitCode {
    // Phase 1: expand directories to .dpy files, then read every file.
    // If any path fails to expand or read, abort early with a usage-
    // style error rather than analyzing a partial project.
    let expanded: Vec<PathBuf> = match expand_paths(paths) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut sources: Vec<(String, String)> = Vec::with_capacity(expanded.len());
    for path in &expanded {
        match fs::read_to_string(path) {
            Ok(source) => sources.push((path.to_string_lossy().into_owned(), source)),
            Err(e) => {
                eprintln!("error reading {}: {e}", path.display());
                return ExitCode::from(2);
            }
        }
    }

    let project = dathon::check_project(&sources);

    // Print per-file summary + body to stdout, in input order. Then dump
    // all diagnostics across all files to stderr at the end (TS-style).
    let mut had_diagnostics = false;
    for (i, file) in project.files.iter().enumerate() {
        let r = &file.result;
        if i > 0 {
            println!();
        }
        if r.parse_error {
            // Parse errors are surfaced together with the rest at the end;
            // here we just note the file couldn't be analyzed.
            println!("{}: parse error (see diagnostics below)", file.path);
        } else {
            println!(
                "{}: parsed OK — {} schema(s), {} typed function(s), {} issue(s)",
                file.path,
                r.schema_count,
                r.typed_function_count,
                r.diagnostics.len(),
            );
            if !r.body.is_empty() {
                println!();
                print!("{}", r.body);
            }
        }
        if !r.diagnostics.is_empty() {
            had_diagnostics = true;
        }
    }

    if had_diagnostics {
        eprintln!();
        for file in &project.files {
            for d in &file.result.diagnostics {
                eprintln!("{}", d.format(&file.path));
            }
        }
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

/// Expand a list of CLI paths to `.dpy` files. File paths pass through
/// verbatim; directory paths recursively walk to collect every `.dpy`
/// under them. Results are deduplicated by canonical path and sorted
/// for deterministic output.
fn expand_paths(inputs: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    for input in inputs {
        let path = Path::new(input);
        if path.is_dir() {
            walk_dpy(path, &mut out)
                .map_err(|e| format!("error walking {}: {e}", path.display()))?;
        } else if path.is_file() {
            out.push(path.to_path_buf());
        } else {
            return Err(format!("{}: not a file or directory", path.display()));
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn walk_dpy(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_dpy(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("dpy") {
            out.push(path);
        }
    }
    Ok(())
}

fn run_transpile(path: &str) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::from(2);
        }
    };

    let output = dathon::transpile(&source);
    // Write the transpiled output to stdout. Composable with shell
    // redirection — `dathon transpile foo.dpy > foo.py`.
    if let Err(e) = io::stdout().write_all(output.as_bytes()) {
        eprintln!("error writing output: {e}");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
