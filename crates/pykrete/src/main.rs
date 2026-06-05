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
    -v, --verbose          Also print every schema declaration and typed
                           function signature (default: summary line only).
        --format <FORMAT>  Output format: 'text' (default, human-readable)
                           or 'json' (machine-readable on stdout).
    -h, --help             Show this help and exit.

Example:
    pykrete check examples/orders.pyk
    pykrete check --format json examples/orders.pyk
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

/// Output format selected via `--format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

/// Parse `check`'s flags and arguments. Returns the verbose flag, the
/// output format, and the list of path arguments, or an error message if
/// a flag is unrecognized. Caller is expected to have already short-
/// circuited on `-h` / `--help`.
fn parse_check_args(args: &[String]) -> Result<(bool, OutputFormat, Vec<String>), String> {
    let mut verbose = false;
    let mut format = OutputFormat::Text;
    let mut paths: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-v" | "--verbose" => verbose = true,
            "--format" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--format requires a value (text|json)".to_string())?;
                format = parse_format(value)?;
            }
            s if s.starts_with("--format=") => {
                let value = &s["--format=".len()..];
                format = parse_format(value)?;
            }
            s if s.starts_with('-') => {
                return Err(format!("unknown option '{s}'; see `pykrete check --help`"));
            }
            _ => paths.push(a.clone()),
        }
        i += 1;
    }
    Ok((verbose, format, paths))
}

fn parse_format(value: &str) -> Result<OutputFormat, String> {
    match value {
        "text" => Ok(OutputFormat::Text),
        "json" => Ok(OutputFormat::Json),
        other => Err(format!(
            "unknown --format value '{other}'; expected 'text' or 'json'"
        )),
    }
}

fn run_check(args: &[String]) -> ExitCode {
    // `--help` / `-h` short-circuit before any other parsing so that
    // `pykrete check --help` works even with no file argument.
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{CHECK_HELP}");
        return ExitCode::SUCCESS;
    }

    let (verbose, format, paths) = match parse_check_args(args) {
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

    // Project config — `pykrete.json`, found at or above the first
    // input file's directory, falling back to the working directory if
    // no input resolved to a real file path. Absent or malformed →
    // defaults. Anchoring on the file (not just CWD) means
    // `pykrete check /abs/path/to/project/foo.pyk` from any CWD still
    // picks up `/abs/path/to/project/pykrete.json`.
    let config = load_config(expanded.first().map(PathBuf::as_path));
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

    match format {
        OutputFormat::Text => render_text(&project, verbose),
        OutputFormat::Json => render_json(&project),
    }
}

/// Default text renderer. Per-file summary + body to stdout in input order,
/// all diagnostics dumped to stderr at the end (TS-style).
///
/// Default output is just the summary line — the verbose schema/function
/// dump is gated behind `--verbose` so first-run output matches the
/// promise in the quickstart docs.
fn render_text(project: &pykrete::ProjectCheckResult, verbose: bool) -> ExitCode {
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

/// Machine-readable renderer. Emits one JSON object on stdout with the
/// shape documented in the v1.0 stability contract:
///
/// ```json
/// {
///   "schemaVersion": "1",
///   "version": "X.Y.Z",
///   "diagnostics": [
///     { "file", "line", "column", "endLine", "endColumn",
///       "code", "ruleName", "severity", "source", "message",
///       "suggestion", "relatedInformation" }
///   ],
///   "summary": { "filesChecked", "errorCount", "warningCount" }
/// }
/// ```
///
/// `schemaVersion` is the version of this JSON shape, distinct from
/// `version` (the pykrete release that produced the output). Consumers
/// pin to `schemaVersion`. Bump policy:
/// - Adding a new top-level or per-diagnostic field: non-breaking, keep
///   `schemaVersion` at `"1"`. Consumers must accept unknown fields.
/// - Adding a new severity or D-code: non-breaking, keep at `"1"`.
///   Consumers must handle unknown severities/codes gracefully.
/// - Renaming a field, changing its type, or changing its meaning:
///   breaking — bump `schemaVersion` to `"2"` alongside the pykrete
///   SemVer-major bump.
///
/// Positions are 1-indexed (matching the `text` format and most editor
/// gutter labels). pykrete-lsp re-indexes to 0-indexed on the wire per
/// the LSP spec; tools consuming this JSON directly should not.
fn render_json(project: &pykrete::ProjectCheckResult) -> ExitCode {
    let mut diagnostics_json: Vec<serde_json::Value> = Vec::new();
    let mut error_count: usize = 0;
    let mut warning_count: usize = 0;
    for file in &project.files {
        for d in &file.result.diagnostics {
            let severity = match d.severity {
                pykrete::diagnostics::Severity::Error => "error",
                pykrete::diagnostics::Severity::Warning => "warning",
            };
            match d.severity {
                pykrete::diagnostics::Severity::Error => error_count += 1,
                pykrete::diagnostics::Severity::Warning => warning_count += 1,
            }
            diagnostics_json.push(serde_json::json!({
                "file": file.path,
                "line": d.line,
                "column": d.column,
                "endLine": d.end_line,
                "endColumn": d.end_column,
                "code": d.code,
                "ruleName": pykrete::diagnostics::rule_name(d.code),
                "severity": severity,
                "source": "pykrete",
                "message": d.message,
                "suggestion": d.suggestion,
                "relatedInformation": [],
            }));
        }
    }

    let payload = serde_json::json!({
        "schemaVersion": "1",
        "version": VERSION,
        "diagnostics": diagnostics_json,
        "summary": {
            "filesChecked": project.files.len(),
            "errorCount": error_count,
            "warningCount": warning_count,
        },
    });

    // Pretty-print so the output is greppable by humans too; tools that
    // care about size pipe through `jq -c`.
    let rendered = serde_json::to_string_pretty(&payload)
        .expect("project JSON is composed of types that always serialize");
    println!("{rendered}");

    if error_count + warning_count > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
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

/// Load `pykrete.json` walking up from `anchor`'s parent directory (or
/// the working directory if `anchor` is `None`). Absent → defaults;
/// present but malformed → a warning and defaults (a config typo
/// shouldn't block the whole check).
fn load_config(anchor: Option<&Path>) -> pykrete::Config {
    let Some(path) = find_pykrete_json(anchor) else {
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

/// Walk up from `anchor`'s parent directory looking for a
/// `pykrete.json`. If `anchor` is `None` or its parent can't be
/// resolved to an absolute path, fall back to the working directory.
/// Anchoring on the input file (not just CWD) lets
/// `pykrete check /abs/path/to/project/foo.pyk` from any CWD pick up
/// `/abs/path/to/project/pykrete.json`.
fn find_pykrete_json(anchor: Option<&Path>) -> Option<PathBuf> {
    // canonicalize() resolves symlinks before the walk — a .pyk reached
    // through a symlink discovers the real project's pykrete.json.
    let start = anchor
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let start = match start {
        Some(s) => s,
        None => env::current_dir().ok()?,
    };
    let mut dir = start;
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
