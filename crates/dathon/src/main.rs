mod diagnostics;
mod schema;
mod types;
mod walk;

use std::env;
use std::fs;
use std::process::ExitCode;

use ruff_source_file::LineIndex;
use ruff_text_size::Ranged;

use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::{FieldResolution, discover_schemas};
use crate::types::COLUMN_TYPE_NAMES;
use crate::walk::discover_top_level_classes;

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

    let line_index = LineIndex::from_source_text(&source);

    let parsed = match ruff_python_parser::parse_module(&source) {
        Ok(p) => p,
        Err(err) => {
            let d = Diagnostic::at(
                Severity::Error,
                "D0001",
                err.error.to_string(),
                err.location.start(),
                &source,
                &line_index,
            );
            eprintln!("{}", d.format(path));
            return ExitCode::from(1);
        }
    };

    let module = parsed.syntax();
    let classes = discover_top_level_classes(module);
    let schemas = discover_schemas(&classes);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    for schema in &schemas {
        let lc = line_index.line_column(schema.class.def.range.start(), &source);
        println!(
            "  {}:{}  schema {}",
            lc.line.get(),
            lc.column.get(),
            schema.name(),
        );
        for field in schema.fields() {
            let ann_range = field.annotation.range();
            let raw_text = &source[ann_range];
            match field.resolve() {
                FieldResolution::Resolved(ct) => {
                    println!("          {}: {}", field.name, ct);
                }
                FieldResolution::UnknownType { name } => {
                    println!("          {}: {}  (unresolved)", field.name, raw_text);
                    diagnostics.push(Diagnostic::at(
                        Severity::Error,
                        "D0010",
                        format!(
                            "Unknown column type '{name}'. Expected one of: {COLUMN_TYPE_NAMES}.",
                        ),
                        ann_range.start(),
                        &source,
                        &line_index,
                    ));
                }
                FieldResolution::NotABareName => {
                    println!("          {}: {}  (unresolved)", field.name, raw_text);
                    diagnostics.push(Diagnostic::at(
                        Severity::Error,
                        "D0011",
                        format!(
                            "Column type '{raw_text}' is not a bare name. \
                             Subscripted/complex column types are not yet \
                             supported in v0.1. Use one of: {COLUMN_TYPE_NAMES}.",
                        ),
                        ann_range.start(),
                        &source,
                        &line_index,
                    ));
                }
            }
        }
    }

    // Summary header — printed AFTER schema bodies so it can include diagnostic count.
    // Top-of-output would be nicer; we'll fix output ordering when the pipeline grows.
    println!(
        "\n{}: parsed OK — {} top-level class(es), {} schema(s), {} issue(s)",
        path,
        classes.len(),
        schemas.len(),
        diagnostics.len(),
    );

    if !diagnostics.is_empty() {
        eprintln!();
        for d in &diagnostics {
            eprintln!("{}", d.format(path));
        }
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
