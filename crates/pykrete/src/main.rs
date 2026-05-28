//! Thin CLI shell. All of the analysis lives in the pykrete library; this
//! binary parses arguments and dispatches to `pykrete::check_project` (the
//! analyzer) or `pykrete::transpile` (the `.pyk` → `.py` emitter).

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const TOP_LEVEL_HELP: &str = "\
pykrete — Static schema checking for dataframes (PySpark today).

Usage:
    pykrete <COMMAND> [OPTIONS] [ARGS]

Commands:
    check       Check .pyk files for schema errors
    transpile   Transpile .pyk to .py (for runtime execution)

Options:
    -V, --version    Show version and exit
    -h, --help       Show this help and exit

For help on a subcommand: pykrete <COMMAND> --help
";

const CHECK_HELP: &str = "\
pykrete check — Check .pyk files for schema errors.

Usage:
    pykrete check [OPTIONS] <FILE_OR_DIR> [<FILE_OR_DIR> ...]

Options:
    -v, --verbose    Also print every schema declaration and typed
                     function signature (default: summary line only).
    -h, --help       Show this help and exit.

Example:
    pykrete check examples/orders.pyk
";

const TRANSPILE_HELP: &str = "\
pykrete transpile — Transpile a .pyk file to .py on stdout.

Usage:
    pykrete transpile [OPTIONS] <FILE.pyk>

Options:
    -h, --help    Show this help and exit.

Example:
    pykrete transpile sales.pyk > sales.py
";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    // Top-level flags first — they short-circuit subcommand dispatch.
    match args.get(1).map(String::as_str) {
        None => {
            print!("{TOP_LEVEL_HELP}");
            return ExitCode::SUCCESS;
        }
        Some("-h" | "--help") => {
            print!("{TOP_LEVEL_HELP}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version") => {
            println!("pykrete {VERSION}");
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    match args.get(1).map(String::as_str) {
        Some("check") => run_check(&args[2..]),
        Some("transpile") => run_transpile(&args[2..]),
        Some(cmd) => {
            eprintln!("unknown command '{cmd}'; see `pykrete --help`");
            ExitCode::from(2)
        }
        None => unreachable!("handled above"),
    }
}

/// Parse `check`'s flags and arguments. Returns the verbose flag and the
/// list of path arguments, or an error message if a flag is unrecognized.
/// Caller is expected to have already short-circuited on `-h` / `--help`.
fn parse_check_args(args: &[String]) -> Result<(bool, Vec<String>), String> {
    let mut verbose = false;
    let mut paths: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "-v" | "--verbose" => verbose = true,
            s if s.starts_with('-') => {
                return Err(format!("unknown option '{s}'; see `pykrete check --help`"));
            }
            _ => paths.push(a.clone()),
        }
    }
    Ok((verbose, paths))
}

fn run_check(args: &[String]) -> ExitCode {
    // `--help` / `-h` short-circuit before any other parsing so that
    // `pykrete check --help` works even with no file argument.
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{CHECK_HELP}");
        return ExitCode::SUCCESS;
    }

    let (verbose, paths) = match parse_check_args(args) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    if paths.is_empty() {
        eprintln!("specify a file or directory; see `pykrete check --help`");
        return ExitCode::from(2);
    }

    // Project config — `pykrete.json`, found at or above the working
    // directory. Absent or malformed → defaults.
    let config = load_config();

    // Phase 1: expand directories to .pyk files, then read every file.
    // If any path fails to expand or read, abort early with a usage-
    // style error rather than analyzing a partial project.
    let mut expanded: Vec<PathBuf> = match expand_paths(&paths) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    // Drop files matched by a `pykrete.json` `exclude` entry.
    expanded.retain(|p| !config.is_excluded(&p.to_string_lossy()));
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

    let mut project = pykrete::check_project_with_mode(&sources, config.check_mode());
    // Apply `pykrete.json` `rules` overrides — drop suppressed codes,
    // re-level the rest — before anything is counted or printed.
    for file in &mut project.files {
        config.apply_rules(&mut file.result.diagnostics);
    }

    // Print per-file summary + body to stdout, in input order. Then dump
    // all diagnostics across all files to stderr at the end (TS-style).
    //
    // Default output is just the summary line — the verbose schema/function
    // dump is gated behind `--verbose` so first-run output matches the
    // promise in the quickstart docs.
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
            if verbose && !r.body.is_empty() {
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

/// Expand a list of CLI paths to `.pyk` files. File paths pass through
/// verbatim; directory paths recursively walk to collect every `.pyk`
/// under them. Results are deduplicated by canonical path and sorted
/// for deterministic output.
fn expand_paths(inputs: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    for input in inputs {
        let path = Path::new(input);
        if path.is_dir() {
            walk_pyk(path, &mut out)
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

fn walk_pyk(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_pyk(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("pyk") {
            out.push(path);
        }
    }
    Ok(())
}

fn run_transpile(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{TRANSPILE_HELP}");
        return ExitCode::SUCCESS;
    }

    // Strip unknown flags / collect the single positional arg.
    let mut positional: Vec<&String> = Vec::new();
    for a in args {
        if a.starts_with('-') {
            eprintln!("unknown option '{a}'; see `pykrete transpile --help`");
            return ExitCode::from(2);
        }
        positional.push(a);
    }

    let path = match positional.as_slice() {
        [p] => p.as_str(),
        [] => {
            eprintln!("specify a file; see `pykrete transpile --help`");
            return ExitCode::from(2);
        }
        _ => {
            eprintln!("transpile takes exactly one file; see `pykrete transpile --help`");
            return ExitCode::from(2);
        }
    };

    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error reading {path}: {e}");
            return ExitCode::from(2);
        }
    };

    let output = pykrete::transpile(&source);
    // Write the transpiled output to stdout. Composable with shell
    // redirection — `pykrete transpile foo.pyk > foo.py`.
    if let Err(e) = io::stdout().write_all(output.as_bytes()) {
        eprintln!("error writing output: {e}");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

/// Load `pykrete.json` from the working directory or an ancestor.
/// Absent → defaults; present but malformed → a warning and defaults
/// (a config typo shouldn't block the whole check).
fn load_config() -> pykrete::Config {
    let Some(path) = find_pykrete_json() else {
        return pykrete::Config::default();
    };
    match fs::read_to_string(&path) {
        Ok(content) => match pykrete::Config::parse(&content) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("warning: ignoring {}: {err}", path.display());
                pykrete::Config::default()
            }
        },
        Err(err) => {
            eprintln!("warning: could not read {}: {err}", path.display());
            pykrete::Config::default()
        }
    }
}

/// Walk up from the working directory looking for a `pykrete.json`.
fn find_pykrete_json() -> Option<PathBuf> {
    let mut dir = env::current_dir().ok()?;
    loop {
        let candidate = dir.join("pykrete.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}
