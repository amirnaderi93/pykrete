//! Thin CLI shell. All of the analysis lives in the dathon library; this
//! binary parses arguments and dispatches to `dathon::check` (the
//! analyzer) or `dathon::transpile` (the `.dpy` → `.py` emitter).

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("check") if args.len() == 3 => run_check(&args[2]),
        Some("transpile") if args.len() == 3 => run_transpile(&args[2]),
        _ => {
            eprintln!("usage: dathon <check|transpile> <file.dpy>");
            ExitCode::from(2)
        }
    }
}

fn run_check(path: &str) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::from(2);
        }
    };

    let result = dathon::check(path, &source);

    if result.parse_error {
        for d in &result.diagnostics {
            eprintln!("{}", d.format(path));
        }
        return ExitCode::from(1);
    }

    println!(
        "{}: parsed OK — {} schema(s), {} typed function(s), {} issue(s)",
        path,
        result.schema_count,
        result.typed_function_count,
        result.diagnostics.len(),
    );
    if !result.body.is_empty() {
        println!();
        print!("{}", result.body);
    }

    if !result.diagnostics.is_empty() {
        eprintln!();
        for d in &result.diagnostics {
            eprintln!("{}", d.format(path));
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
