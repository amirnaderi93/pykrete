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
    migrate     Rewrite deprecated DataFrame[X] aliases to SparkFrame[X]

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
        --report-aliases   Emit a JSON inventory of every deprecated
                           'DataFrame[X]' annotation site (planning aid
                           for the v2.0 alias removal); suppresses normal
                           diagnostic output and always exits 0.
    -h, --help             Show this help and exit.

Example:
    pykrete check examples/orders.pyk
    pykrete check --format json examples/orders.pyk
    pykrete check --report-aliases src/
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

const MIGRATE_HELP: &str = "\
pykrete migrate — Rewrite deprecated 'DataFrame[X]' annotations to
'SparkFrame[X]' (v2.0 alias removal remediation).

Usage:
    pykrete migrate [OPTIONS] <FILE_OR_DIR> [<FILE_OR_DIR> ...]

Modes (mutually exclusive):
        --check    Exit 1 if any file would change, 0 if none. No writes.
        --diff     Print a unified diff of the proposed changes to stdout.
                   No writes.
    (default)      Rewrite each matching file in place. One line per
                   modified file is printed to stdout ('rewrote: <path>');
                   files with no aliases are left untouched. Exits 0 on
                   success.

Options:
    -h, --help     Show this help and exit.

Example:
    pykrete migrate --check src/
    pykrete migrate --diff sales.pyk
    pykrete migrate src/
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
        Some("migrate") => run_migrate(&args[2..]),
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

/// Parse `check`'s flags and arguments. Returns the parsed configuration
/// or an error message if a flag is unrecognized. Caller is expected to
/// have already short-circuited on `-h` / `--help`.
struct CheckArgs {
    verbose: bool,
    format: OutputFormat,
    report_aliases: bool,
    paths: Vec<String>,
}

fn parse_check_args(args: &[String]) -> Result<CheckArgs, String> {
    let mut verbose = false;
    let mut format = OutputFormat::Text;
    let mut report_aliases = false;
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
            "--report-aliases" => report_aliases = true,
            s if s.starts_with('-') => {
                return Err(format!("unknown option '{s}'; see `pykrete check --help`"));
            }
            _ => paths.push(a.clone()),
        }
        i += 1;
    }
    Ok(CheckArgs {
        verbose,
        format,
        report_aliases,
        paths,
    })
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

    let CheckArgs {
        verbose,
        format,
        report_aliases,
        paths,
    } = match parse_check_args(args) {
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

    // `--report-aliases` is invocation-only: report the alias inventory
    // and skip the normal diagnostic pipeline entirely. Exit code is 0
    // even when records are emitted — this is informational, not a
    // diagnostic. v1.5 PR-D spec §5.1.
    if report_aliases {
        let sites = pykrete::collect_alias_sites(&sources);
        let rendered = pykrete::render_alias_report_json(&sites);
        println!("{rendered}");
        return ExitCode::SUCCESS;
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

/// Mode selected via `--check` / `--diff` (or neither).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrateMode {
    /// Default: apply the rewrite in place. For each file with one or
    /// more `DataFrame[X]` alias sites, splice in the `SparkFrame[X]`
    /// replacements and atomically write the result back (tempfile +
    /// rename). Clean files are left untouched.
    Apply,
    /// `--check`: exit 1 if any file would change, 0 otherwise. No
    /// writes, no diff.
    Check,
    /// `--diff`: print a unified diff of the proposed changes to
    /// stdout. No writes.
    Diff,
}

struct MigrateArgs {
    mode: MigrateMode,
    paths: Vec<String>,
}

fn parse_migrate_args(args: &[String]) -> Result<MigrateArgs, String> {
    let mut mode: Option<MigrateMode> = None;
    let mut paths: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => {
                if mode == Some(MigrateMode::Diff) {
                    return Err("--check and --diff are mutually exclusive; pick one".to_string());
                }
                mode = Some(MigrateMode::Check);
            }
            "--diff" => {
                if mode == Some(MigrateMode::Check) {
                    return Err("--check and --diff are mutually exclusive; pick one".to_string());
                }
                mode = Some(MigrateMode::Diff);
            }
            s if s.starts_with('-') => {
                return Err(format!(
                    "unknown option '{s}'; see `pykrete migrate --help`"
                ));
            }
            _ => paths.push(a.clone()),
        }
    }
    Ok(MigrateArgs {
        mode: mode.unwrap_or(MigrateMode::Apply),
        paths,
    })
}

fn run_migrate(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{MIGRATE_HELP}");
        return ExitCode::SUCCESS;
    }

    let MigrateArgs { mode, paths } = match parse_migrate_args(args) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    if paths.is_empty() {
        eprintln!("specify a file or directory; see `pykrete migrate --help`");
        return ExitCode::from(2);
    }

    let expanded: Vec<PathBuf> = match expand_paths(&paths) {
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

    let sites = pykrete::collect_alias_sites(&sources);

    match mode {
        MigrateMode::Check => {
            if sites.is_empty() {
                ExitCode::SUCCESS
            } else {
                for s in &sites {
                    eprintln!(
                        "{}:{}:{}: would rewrite to {}",
                        s.file, s.line, s.column, s.would_be_replacement
                    );
                }
                ExitCode::from(1)
            }
        }
        MigrateMode::Diff => {
            // Group sites by file (collect_alias_sites preserves input
            // order, which is sorted), emit one unified diff per file.
            // No write side-effects. Per-edit hunks: one `@@ -L,1 +L,1 @@`
            // hunk per alias site, single-line old + single-line new.
            let mut by_file: Vec<(&str, &str, Vec<&pykrete::AliasSite>)> = Vec::new();
            for (path, source) in &sources {
                let file_sites: Vec<&pykrete::AliasSite> =
                    sites.iter().filter(|s| s.file == *path).collect();
                if !file_sites.is_empty() {
                    by_file.push((path.as_str(), source.as_str(), file_sites));
                }
            }
            for (path, source, file_sites) in &by_file {
                let diff = unified_diff(path, source, file_sites);
                print!("{diff}");
            }
            ExitCode::SUCCESS
        }
        MigrateMode::Apply => {
            // sources[i].0 is the string path; expanded[i] is the same
            // path as a PathBuf. Iterate in lockstep so we have a
            // PathBuf for atomic_write and a &str for the sites filter.
            //
            // Round-2 reviewer caught: previous loop would `return` on
            // first error, leaving the user with a half-migrated tree
            // and no summary of what was/wasn't done. Now: collect
            // errors, attempt every file, then summarize.
            let mut rewrote = 0usize;
            let mut failed: Vec<(std::path::PathBuf, io::Error)> = Vec::new();
            let mut skipped = 0usize;
            for (path_buf, (path_str, source)) in expanded.iter().zip(sources.iter()) {
                let file_sites: Vec<&pykrete::AliasSite> =
                    sites.iter().filter(|s| s.file == *path_str).collect();
                if file_sites.is_empty() {
                    skipped += 1;
                    continue;
                }
                let rewritten = apply_alias_rewrites(source, &file_sites);
                match atomic_write(path_buf, &rewritten) {
                    Ok(()) => {
                        println!("rewrote: {}", path_buf.display());
                        rewrote += 1;
                    }
                    Err(e) => {
                        eprintln!("error writing {}: {e}", path_buf.display());
                        failed.push((path_buf.clone(), e));
                    }
                }
            }
            if !failed.is_empty() {
                eprintln!(
                    "migrate: rewrote {rewrote} file(s), {} failed, {skipped} clean (no aliases)",
                    failed.len()
                );
                return ExitCode::from(2);
            }
            ExitCode::SUCCESS
        }
    }
}

/// Write `contents` to `path` atomically: canonicalize through any
/// symlinks first, then write to a tempfile in the canonical target's
/// directory, then `rename` over the original. The canonicalize step
/// is round-2-reviewer-mandated: without it, `fs::rename` on a symlink
/// path would replace the SYMLINK ENTRY with a regular file, silently
/// de-linking and leaving the real source untouched. The rename is
/// atomic on POSIX (and on Windows when both paths are on the same
/// volume — guaranteed here because the tempfile lives next to the
/// resolved target). A crash mid-write or mid-rename always leaves
/// the original intact: the tempfile is cleaned up on EVERY failure
/// path (fs::write error, fs::rename error).
fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let target = fs::canonicalize(path)?;
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(".pykrete-migrate.tmp");
    let tmp = dir.join(tmp_name);
    // Round-2 reviewer: prior code didn't clean up on fs::write failure,
    // leaving stale `.pykrete-migrate.tmp` dotfiles in user source trees
    // after disk-full / EIO events. Wrap both fallible steps with the
    // same cleanup path.
    if let Err(e) = fs::write(&tmp, contents) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, &target) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Apply every `AliasSite`'s `would_be_replacement` to `source` via
/// direct byte-range substitution, returning the rewritten string.
/// Sites are spliced in descending byte-offset order so earlier edits
/// don't shift later positions. The byte range comes from `AliasSite.range`
/// (populated by the walker from `expr.range()`), so this routine never
/// re-tokenizes — token-preserving by construction.
fn apply_alias_rewrites(source: &str, sites: &[&pykrete::AliasSite]) -> String {
    let mut edits: Vec<(usize, usize, &str)> = sites
        .iter()
        .map(|s| {
            (
                usize::from(s.range.start()),
                usize::from(s.range.end()),
                s.would_be_replacement.as_str(),
            )
        })
        .collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));

    let mut out = source.to_string();
    for (start, end, replacement) in edits {
        out.replace_range(start..end, replacement);
    }
    out
}

/// Per-edit unified diff. One `@@ -L,1 +L,1 @@` hunk per `AliasSite`,
/// showing the single original line as `-` and the single rewritten
/// line as `+`. Output is `patch -p1`-compatible. Because every alias
/// rewrite is a single-line edit (`DataFrame[X]` → `SparkFrame[X]`
/// never spans a newline), each hunk needs exactly one old line + one
/// new line and no surrounding context.
fn unified_diff(path: &str, source: &str, sites: &[&pykrete::AliasSite]) -> String {
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let line_at = |offset: usize| -> usize {
        line_starts
            .binary_search(&offset)
            .unwrap_or_else(|idx| idx - 1)
    };
    let line_text = |idx: usize| -> &str {
        let start = line_starts[idx];
        let end = line_starts.get(idx + 1).copied().unwrap_or(source.len());
        &source[start..end]
    };

    // Build (line_idx, edits-on-this-line) groups in source order so the
    // diff reads top-to-bottom. Multiple sites on the same line collapse
    // into one hunk whose `-` line is the original and whose `+` line
    // has every alias replaced.
    let mut by_line: Vec<(usize, Vec<&pykrete::AliasSite>)> = Vec::new();
    let mut sorted: Vec<&pykrete::AliasSite> = sites.to_vec();
    sorted.sort_by_key(|s| usize::from(s.range.start()));
    for s in sorted {
        let line_idx = line_at(usize::from(s.range.start()));
        match by_line.last_mut() {
            Some((last_line, edits)) if *last_line == line_idx => edits.push(s),
            _ => by_line.push((line_idx, vec![s])),
        }
    }

    // Round-2 reviewer: prior `--- a/{path}` with an absolute path
    // produced `--- a//abs/path` (double slash) which breaks the standard
    // `patch -p1` workflow. Strip a single leading slash so the diff
    // headers look like `--- a/abs/path` consistently for both relative
    // and absolute input paths.
    let normalized_path = path.strip_prefix('/').unwrap_or(path);
    let mut out = String::new();
    out.push_str(&format!("--- a/{normalized_path}\n"));
    out.push_str(&format!("+++ b/{normalized_path}\n"));
    for (line_idx, edits) in &by_line {
        let original = line_text(*line_idx);
        let line_start_offset = line_starts[*line_idx];
        // Splice descending so earlier edits' offsets stay valid as we
        // mutate the buffer.
        let mut rewritten = original.to_string();
        let mut sorted_edits: Vec<&&pykrete::AliasSite> = edits.iter().collect();
        sorted_edits.sort_by_key(|s| std::cmp::Reverse(usize::from(s.range.start())));
        for s in sorted_edits {
            let s_start = usize::from(s.range.start()) - line_start_offset;
            let s_end = usize::from(s.range.end()) - line_start_offset;
            rewritten.replace_range(s_start..s_end, &s.would_be_replacement);
        }
        let line_no = line_idx + 1;
        out.push_str(&format!("@@ -{line_no},1 +{line_no},1 @@\n"));
        out.push('-');
        out.push_str(original);
        // Round-2 reviewer: POSIX unified-diff requires the marker
        // `\ No newline at end of file` when a hunk line doesn't end
        // with `\n`. Without it, `patch -p1` rejects the hunk because
        // the in-memory line carries a `\n` the source file doesn't.
        // Apply mode operates on raw bytes and is unaffected; only
        // `--diff` output needs the marker.
        if !original.ends_with('\n') {
            out.push('\n');
            out.push_str("\\ No newline at end of file\n");
        }
        out.push('+');
        out.push_str(&rewritten);
        if !rewritten.ends_with('\n') {
            out.push('\n');
            out.push_str("\\ No newline at end of file\n");
        }
    }
    out
}
