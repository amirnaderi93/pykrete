//! Thin CLI shell. All of the analysis lives in the dathon library; this
//! binary just reads a file, calls [`dathon::check`], and prints the
//! formatted output.

use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 || args[1] != "check" {
        eprintln!("usage: dathon check <file.dpy>");
        return ExitCode::from(2);
    }

    let path = &args[2];
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
