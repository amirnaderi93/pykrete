//! Thin CLI shell. All of the analysis lives in the dathon library; this
//! binary parses arguments and dispatches to `dathon::check_project` (the
//! analyzer) or `dathon::transpile` (the `.dpy` → `.py` emitter).

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("check") if args.len() >= 3 => run_check(&args[2..]),
        Some("transpile") if args.len() == 3 => run_transpile(&args[2]),
        _ => {
            eprintln!("usage:");
            eprintln!("  dathon check <file.dpy> [<file.dpy> ...]");
            eprintln!("  dathon transpile <file.dpy>");
            ExitCode::from(2)
        }
    }
}

fn run_check(paths: &[String]) -> ExitCode {
    // Phase 1: read every file. If any fails to read, abort early with a
    // usage-style error rather than trying to analyze a partial project.
    let mut sources: Vec<(String, String)> = Vec::with_capacity(paths.len());
    for path in paths {
        match fs::read_to_string(path) {
            Ok(source) => sources.push((path.clone(), source)),
            Err(e) => {
                eprintln!("error reading {path}: {e}");
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
