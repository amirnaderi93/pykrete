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
        --deprecation-report
                           Emit a JSON envelope listing every D0090-firing
                           site (file, line, column, raw annotation,
                           adjudicated dialect, suggested rewrite) for CI
                           gating and migration dashboards; suppresses
                           normal diagnostic output and always exits 0.
                           Mutually exclusive with --report-aliases.
    -h, --help             Show this help and exit.

Example:
    pykrete check examples/orders.pyk
    pykrete check --format json examples/orders.pyk
    pykrete check --report-aliases src/
    pykrete check --deprecation-report src/
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

Walks `.pyk` files only. If your project uses `.py` files via the
multiplexer integration, copy or rename them to `.pyk` before running
`pykrete migrate`.

Usage:
    pykrete migrate [OPTIONS] <FILE_OR_DIR> [<FILE_OR_DIR> ...]

Modes (mutually exclusive):
        --apply    Rewrite each matching file in place. One line per
                   modified file is printed to stdout ('rewrote: <path>');
                   files with no aliases are left untouched. Exits 0 on
                   success.
        --diff     Print a unified diff of the proposed changes to stdout.
                   No writes.
    (default)      Check mode. Exit 1 if any file would change, 0 if none.
                   No writes. (Use --apply to rewrite.) The `--check` flag
                   is accepted as an explicit alias for the default.

Migration from v1.6 → v1.7: `pykrete migrate <path>` no longer rewrites
files by default. Pass `--apply` to restore v1.6's rewrite-on-default
behavior, or `--check` / `--diff` to inspect proposed changes without
writes. This is TS-style `--noEmit`-by-default safety.

When a binding's downstream usage is ambiguous between Spark and pandas,
the migrator leaves the `DataFrame[X]` text intact and (under `--apply`)
injects a `# pykrete: ambiguous` comment on the line above so the human
review the next step needs is visible in the diff.

Options:
    -h, --help     Show this help and exit.

Example:
    pykrete migrate src/                # dry-run check (default)
    pykrete migrate --apply src/        # rewrite in place
    pykrete migrate --diff sales.pyk    # preview as unified diff
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
    deprecation_report: bool,
    paths: Vec<String>,
}

fn parse_check_args(args: &[String]) -> Result<CheckArgs, String> {
    let mut verbose = false;
    let mut format = OutputFormat::Text;
    let mut report_aliases = false;
    let mut deprecation_report = false;
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
            "--deprecation-report" => deprecation_report = true,
            s if s.starts_with('-') => {
                return Err(format!("unknown option '{s}'; see `pykrete check --help`"));
            }
            _ => paths.push(a.clone()),
        }
        i += 1;
    }
    if report_aliases && deprecation_report {
        return Err(
            "--report-aliases and --deprecation-report are mutually exclusive; pass only one"
                .to_string(),
        );
    }
    Ok(CheckArgs {
        verbose,
        format,
        report_aliases,
        deprecation_report,
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
        deprecation_report,
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
        let mut sites = pykrete::collect_alias_sites(&sources);
        pykrete::adjudicate_alias_sites(&sources, &mut sites);
        let rendered = pykrete::render_alias_report_json(&sites);
        println!("{rendered}");
        return ExitCode::SUCCESS;
    }

    // `--deprecation-report` is the v1.8 PR-V1 sibling: same collection
    // + adjudication plumbing, different envelope. Every alias site is a
    // D0090-firing site, so no filter — the envelope mirrors the alias
    // inventory but adds the diagnostic code, rule name, binding name,
    // and a precomputed by-dialect summary so CI gates and migration
    // dashboards can consume the report without re-aggregating. Exit 0
    // matches `--report-aliases`: this is an inventory, not a gate.
    if deprecation_report {
        let mut sites = pykrete::collect_alias_sites(&sources);
        pykrete::adjudicate_alias_sites(&sources, &mut sites);
        let rendered = pykrete::render_deprecation_report_json(&sites);
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

// v1.7 backlog: optional `--include-py` flag for multiplexer cohort.
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

/// Mode selected via `--apply` / `--check` / `--diff` (or neither).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrateMode {
    /// `--apply`: rewrite each matching file in place. For each file
    /// with one or more `DataFrame[X]` alias sites, splice in the
    /// `SparkFrame[X]` replacements and atomically write the result
    /// back (tempfile + rename). Clean files are left untouched.
    Apply,
    /// Default mode (v1.7+): exit 1 if any file would change, 0
    /// otherwise. No writes, no diff. `--check` is accepted as an
    /// explicit alias for clarity.
    Check,
    /// `--diff`: print a unified diff of the proposed changes to
    /// stdout. No writes.
    Diff,
}

struct MigrateArgs {
    mode: MigrateMode,
    /// True when none of `--apply` / `--check` / `--diff` was passed —
    /// the user is on the v1.7 default (`Check`). The CLI prints a
    /// one-line stderr warning in this case so v1.6 users who expected
    /// rewrite-on-default see the migration path.
    mode_was_implicit: bool,
    paths: Vec<String>,
}

fn parse_migrate_args(args: &[String]) -> Result<MigrateArgs, String> {
    let mut mode: Option<MigrateMode> = None;
    let mut paths: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--apply" => {
                if let Some(existing) = mode
                    && existing != MigrateMode::Apply
                {
                    return Err(mutex_error(existing, MigrateMode::Apply));
                }
                mode = Some(MigrateMode::Apply);
            }
            "--check" => {
                if let Some(existing) = mode
                    && existing != MigrateMode::Check
                {
                    return Err(mutex_error(existing, MigrateMode::Check));
                }
                mode = Some(MigrateMode::Check);
            }
            "--diff" => {
                if let Some(existing) = mode
                    && existing != MigrateMode::Diff
                {
                    return Err(mutex_error(existing, MigrateMode::Diff));
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
    let mode_was_implicit = mode.is_none();
    Ok(MigrateArgs {
        mode: mode.unwrap_or(MigrateMode::Check),
        mode_was_implicit,
        paths,
    })
}

fn mode_flag_name(mode: MigrateMode) -> &'static str {
    match mode {
        MigrateMode::Apply => "--apply",
        MigrateMode::Check => "--check",
        MigrateMode::Diff => "--diff",
    }
}

fn mutex_error(a: MigrateMode, b: MigrateMode) -> String {
    format!(
        "{} and {} are mutually exclusive; pick one",
        mode_flag_name(a),
        mode_flag_name(b)
    )
}

fn run_migrate(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{MIGRATE_HELP}");
        return ExitCode::SUCCESS;
    }

    let MigrateArgs {
        mode,
        mode_was_implicit,
        paths,
    } = match parse_migrate_args(args) {
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

    if mode_was_implicit {
        eprintln!(
            "pykrete: migrate default is now --check; pass --apply to rewrite in place (v1.7+)"
        );
    }

    let expanded: Vec<PathBuf> = match expand_paths(&paths) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let mut expanded_kept: Vec<PathBuf> = Vec::with_capacity(expanded.len());
    let mut sources: Vec<(String, String)> = Vec::with_capacity(expanded.len());
    for path in &expanded {
        match fs::read_to_string(path) {
            Ok(source) => {
                // Parse each source once up-front so we can surface
                // unparseable files to the user. The downstream walkers
                // (`collect_alias_sites`, `adjudicate_alias_sites`) silently
                // skip on parse failure — that's fine for the report path
                // (D0001 is the diagnostic channel for parse errors), but
                // the migrate CLI runs without `check` semantics so the
                // user never learns a file was skipped. Print once per
                // skipped file on stderr and drop it from both slices so
                // the lockstep with `expanded_kept` stays consistent.
                let path_str = path.to_string_lossy().into_owned();
                if ruff_python_parser::parse_module(&source).is_err() {
                    eprintln!("skipped (parse error): {path_str}");
                    continue;
                }
                expanded_kept.push(path.clone());
                sources.push((path_str, source));
            }
            Err(e) => {
                eprintln!("error reading {}: {e}", path.display());
                return ExitCode::from(2);
            }
        }
    }
    let expanded = expanded_kept;

    let mut sites = pykrete::collect_alias_sites(&sources);
    // v1.6 PR-M3: walk each binding's downstream usage and re-tag each
    // site as Spark / Pandas / ambiguous before any output. Ambiguous
    // sites keep their raw `DataFrame[X]` text — the rewriter is a
    // no-op there and the marker is injected separately below.
    pykrete::adjudicate_alias_sites(&sources, &mut sites);

    match mode {
        MigrateMode::Check => {
            if sites.is_empty() {
                ExitCode::SUCCESS
            } else {
                // Round-2 reviewer (B2): per-site lines go to STDOUT, not
                // stderr. Cookbook example treats them as data the user
                // pipes into `tee` / grep / CI gating. Stderr is reserved
                // for actual runtime errors. Matches ruff/pyright/clippy
                // convention.
                // Round-2 reviewer (B2): ambiguous sites get a distinct
                // message — the old "would rewrite to DataFrame[Sale]"
                // was tautological (same string is already in source) and
                // gave the user no signal that the site needed human
                // adjudication, not auto-rewrite.
                for s in &sites {
                    if s.verdict == Some(pykrete::AdjudicatedDialect::Ambiguous) {
                        println!(
                            "{}:{}:{}: ambiguous — needs human review (mixed Spark/pandas usage)",
                            s.file, s.line, s.column
                        );
                    } else {
                        println!(
                            "{}:{}:{}: would rewrite to {}",
                            s.file, s.line, s.column, s.would_be_replacement
                        );
                    }
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
                let rewritten =
                    apply_alias_rewrites_with_ambiguous(source, &file_sites, &sites, path_str);
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

/// Apply `apply_alias_rewrites` and additionally inject a `# pykrete:
/// ambiguous` comment on the line above every ambiguous site. The
/// ambiguous lookup goes through the full `all_sites` slice (not just
/// `file_sites`) because the helper API takes both — caller already
/// scoped to this file with `path_str`, the helper re-filters internally.
/// Ambiguous sites' `would_be_replacement` equals the raw source text,
/// so `apply_alias_rewrites` is a no-op on them and the marker is the
/// only edit.
fn apply_alias_rewrites_with_ambiguous(
    source: &str,
    file_sites: &[&pykrete::AliasSite],
    all_sites: &[pykrete::AliasSite],
    path: &str,
) -> String {
    let rewritten = apply_alias_rewrites(source, file_sites);
    let ambiguous_offsets = pykrete::ambiguous_site_offsets(all_sites, path);
    inject_ambiguous_markers(&rewritten, source, &ambiguous_offsets)
}

/// For each ambiguous offset (positions in the ORIGINAL source), insert
/// `# pykrete: ambiguous` on the line above. Inserts in descending line
/// order so earlier inserts don't shift later line offsets. One marker
/// per distinct line, and idempotent across re-runs: if the line above
/// the target already trims to `# pykrete: ambiguous`, skip — so running
/// `pykrete migrate` twice on a file with unresolved ambiguous sites
/// does not double-stack markers.
///
/// EOL: matches the source file's predominant line ending. A `\r\n`-ended
/// source (Windows-checked-out git tree) gets the marker emitted with
/// `\r\n`; an `\n`-ended source gets `\n`. v1.6 hard-wired `\n`, which
/// created mixed-EOL files on CRLF sources — git surfaced those as a
/// whole-file change on the next commit.
fn inject_ambiguous_markers(
    rewritten: &str,
    original_source: &str,
    offsets: &[ruff_text_size::TextSize],
) -> String {
    if offsets.is_empty() {
        return rewritten.to_string();
    }
    let eol = detect_eol(original_source);
    // Determine the line number for each offset (1-based), then for each
    // distinct line, find the byte position in `rewritten` at the start
    // of that line and insert the marker comment with the matching
    // indent. Lines stay stable: byte rewrites earlier in M3 only
    // replace `DataFrame` (9 bytes) with `SparkFrame`/`PandasFrame`
    // (10/11 bytes) — never insert or delete newlines.
    let mut line_indices: Vec<usize> = offsets
        .iter()
        .map(|off| line_index_of(original_source, usize::from(*off)))
        .collect();
    line_indices.sort();
    line_indices.dedup();
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(rewritten.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let mut out = rewritten.to_string();
    for line_idx in line_indices.into_iter().rev() {
        // Idempotency: if the preceding line already trims to the marker
        // comment, skip. Round-2 reviewer (important): the docstring
        // promised this; the impl did not. Re-runs on an unresolved file
        // used to stack duplicate markers.
        if line_idx > 0 {
            let prev_start = line_starts.get(line_idx - 1).copied().unwrap_or(0);
            let prev_end = line_starts.get(line_idx).copied().unwrap_or(out.len());
            let prev_line = &out[prev_start..prev_end];
            if prev_line.trim() == "# pykrete: ambiguous" {
                continue;
            }
        }
        let start = line_starts.get(line_idx).copied().unwrap_or(out.len());
        let end = line_starts.get(line_idx + 1).copied().unwrap_or(out.len());
        let line_text = &out[start..end];
        let indent: String = line_text
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let marker = format!("{indent}# pykrete: ambiguous{eol}");
        out.insert_str(start, &marker);
    }
    out
}

/// Sniff the source's predominant line ending. Looks at the first
/// `\n` only: if preceded by `\r`, the source is CRLF; otherwise LF.
/// Empty / no-newline files default to LF. Mixed-EOL sources (rare)
/// inherit whichever EOL the first newline lands on — per spec, don't
/// over-engineer.
fn detect_eol(source: &str) -> &'static str {
    match source.find('\n') {
        Some(i) if i > 0 && source.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

fn line_index_of(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
}

/// Apply every `AliasSite`'s `would_be_replacement` to `source` via
/// direct byte-range substitution, returning the rewritten string.
/// Sites are spliced in descending byte-offset order so earlier edits
/// don't shift later positions. The byte range comes from `AliasSite.range`
/// (populated by the walker from `expr.range()`), so this routine never
/// re-tokenizes — token-preserving by construction. Ambiguous sites,
/// where `would_be_replacement` equals the raw source text, are no-ops
/// here; the `# pykrete: ambiguous` marker is injected separately.
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

/// Per-edit unified diff. One `@@ -L,1 +L,1 @@` hunk per non-ambiguous
/// site (`DataFrame[X]` → `SparkFrame[X]` / `PandasFrame[X]`), plus a
/// `@@ -L,0 +L,1 @@` insertion hunk above each ambiguous site for the
/// `# pykrete: ambiguous` marker. Round-2 reviewer (B1): pre-fix, the
/// diff for ambiguous sites was a tautological `-line` / `+line` no-op
/// AND the marker was missing — so `pykrete migrate --diff | patch -p1`
/// did NOT round-trip to the same result as `pykrete migrate --apply`.
/// Now: ambiguous sites are skipped from the rewrite hunks and get
/// their marker emitted as a pure-insertion hunk. The diff is
/// `patch -p1`-compatible AND equivalent to apply mode end-to-end.
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

    // Round-2 reviewer (B1): split sites into ambiguous (marker-only)
    // and rewritable (real rewrite). The rewrite hunk path only sees
    // sites that actually change the source; ambiguous sites get a
    // pure-insertion marker hunk emitted alongside.
    let ambiguous: Vec<&pykrete::AliasSite> = sites
        .iter()
        .copied()
        .filter(|s| s.verdict == Some(pykrete::AdjudicatedDialect::Ambiguous))
        .collect();
    let rewritable: Vec<&pykrete::AliasSite> = sites
        .iter()
        .copied()
        .filter(|s| s.verdict != Some(pykrete::AdjudicatedDialect::Ambiguous))
        .collect();

    // Build (line_idx, edits-on-this-line) groups in source order so the
    // diff reads top-to-bottom. Multiple sites on the same line collapse
    // into one hunk whose `-` line is the original and whose `+` line
    // has every alias replaced.
    let mut by_line: Vec<(usize, Vec<&pykrete::AliasSite>)> = Vec::new();
    let mut sorted: Vec<&pykrete::AliasSite> = rewritable.clone();
    sorted.sort_by_key(|s| usize::from(s.range.start()));
    for s in sorted {
        let line_idx = line_at(usize::from(s.range.start()));
        match by_line.last_mut() {
            Some((last_line, edits)) if *last_line == line_idx => edits.push(s),
            _ => by_line.push((line_idx, vec![s])),
        }
    }

    // Build the set of (line_idx, indent) for ambiguous marker insertions.
    // One marker per distinct ambiguous line (matches `inject_ambiguous_markers`).
    let mut ambiguous_lines: Vec<(usize, String)> = Vec::new();
    let mut seen_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut sorted_amb = ambiguous;
    sorted_amb.sort_by_key(|s| usize::from(s.range.start()));
    for s in sorted_amb {
        let line_idx = line_at(usize::from(s.range.start()));
        if !seen_lines.insert(line_idx) {
            continue;
        }
        let line = line_text(line_idx);
        let indent: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        ambiguous_lines.push((line_idx, indent));
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

    // Round-2 reviewer (B1): walk both rewrite and marker hunks in
    // source order so the diff reads top-to-bottom. patch -p1 tracks
    // line-number offsets across hunks automatically, so hunk headers
    // stay at their original-source line numbers. For a tie (marker
    // + rewrite on same line) emit the marker first so the inserted
    // comment appears above the rewritten line.
    let mut rewrite_iter = by_line.iter().peekable();
    let mut marker_iter = ambiguous_lines.iter().peekable();
    loop {
        let next_rewrite_line = rewrite_iter.peek().map(|(l, _)| *l);
        let next_marker_line = marker_iter.peek().map(|(l, _)| *l);
        let emit_marker = match (next_marker_line, next_rewrite_line) {
            (Some(m), Some(r)) => m <= r,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if emit_marker {
            let (line_idx, indent) = marker_iter.next().expect("peek matched");
            let line_no = line_idx + 1;
            // GNU patch interprets `@@ -L,0 +L,1 @@` as "insert AFTER
            // original line L", so to land the marker BEFORE the
            // ambiguous-site line we use `L-1` on the original-count
            // side. For the very first line, `L-1 == 0` means "insert
            // at the start of the file" — valid.
            let before = line_no.saturating_sub(1);
            out.push_str(&format!("@@ -{before},0 +{line_no},1 @@\n"));
            out.push('+');
            out.push_str(indent);
            out.push_str("# pykrete: ambiguous\n");
            continue;
        }
        let Some((line_idx, edits)) = rewrite_iter.next() else {
            break;
        };
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
        // with `\n`. Apply mode operates on raw bytes and is unaffected.
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
