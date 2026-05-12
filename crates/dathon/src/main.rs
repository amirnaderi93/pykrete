mod dataframe;
mod diagnostics;
mod schema;
mod types;
mod walk;

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::process::ExitCode;

use ruff_source_file::LineIndex;
use ruff_text_size::Ranged;

use crate::dataframe::{DataFrameAnnotation, SlotLabel, typed_slots};
use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::{FieldResolution, Schema, discover_schemas};
use crate::types::COLUMN_TYPE_NAMES;
use crate::walk::{discover_top_level_classes, discover_top_level_functions};

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
    let functions = discover_top_level_functions(module);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Body is rendered into a buffer so the summary can be printed first, with
    // counts (including diagnostic count) computed before any output happens.
    let mut body = String::new();

    for schema in &schemas {
        render_schema(schema, &source, &line_index, &mut body, &mut diagnostics);
    }

    let typed_functions: Vec<_> = functions
        .iter()
        .map(|f| (f, typed_slots(f)))
        .filter(|(_, slots)| !slots.is_empty())
        .collect();

    for (func, slots) in &typed_functions {
        render_function(
            func,
            slots,
            &schemas,
            &source,
            &line_index,
            &mut body,
            &mut diagnostics,
        );
    }

    println!(
        "{}: parsed OK — {} schema(s), {} typed function(s), {} issue(s)",
        path,
        schemas.len(),
        typed_functions.len(),
        diagnostics.len(),
    );
    if !body.is_empty() {
        println!();
        print!("{body}");
    }

    if !diagnostics.is_empty() {
        eprintln!();
        for d in &diagnostics {
            eprintln!("{}", d.format(path));
        }
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

fn render_schema(
    schema: &Schema<'_>,
    source: &str,
    line_index: &LineIndex,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let lc = line_index.line_column(schema.class.def.range.start(), source);
    writeln!(
        out,
        "  {}:{}  schema {}",
        lc.line.get(),
        lc.column.get(),
        schema.name(),
    )
    .unwrap();
    for field in schema.fields() {
        let ann_range = field.annotation.range();
        let raw_text = &source[ann_range];
        match field.resolve() {
            FieldResolution::Resolved(ct) => {
                writeln!(out, "          {}: {}", field.name, ct).unwrap();
            }
            FieldResolution::UnknownType { name } => {
                writeln!(out, "          {}: {}  (unresolved)", field.name, raw_text).unwrap();
                diagnostics.push(Diagnostic::at(
                    Severity::Error,
                    "D0010",
                    format!(
                        "Unknown column type '{name}'. Expected one of: {COLUMN_TYPE_NAMES}.",
                    ),
                    ann_range.start(),
                    source,
                    line_index,
                ));
            }
            FieldResolution::NotABareName => {
                writeln!(out, "          {}: {}  (unresolved)", field.name, raw_text).unwrap();
                diagnostics.push(Diagnostic::at(
                    Severity::Error,
                    "D0011",
                    format!(
                        "Column type '{raw_text}' is not a bare name. \
                         Subscripted/complex column types are not yet \
                         supported in v0.1. Use one of: {COLUMN_TYPE_NAMES}.",
                    ),
                    ann_range.start(),
                    source,
                    line_index,
                ));
            }
        }
    }
}

fn render_function(
    func: &crate::walk::DiscoveredFunction<'_>,
    slots: &[crate::dataframe::TypedSlot<'_>],
    schemas: &[Schema<'_>],
    source: &str,
    line_index: &LineIndex,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let lc = line_index.line_column(func.def.range.start(), source);
    writeln!(out, "  {}:{}  fn {}", lc.line.get(), lc.column.get(), func.name()).unwrap();

    for slot in slots {
        let ann_range = slot.annotation.range();
        let raw_text = &source[ann_range];
        let prefix = match slot.label {
            SlotLabel::Param(name) => format!("          {name}: "),
            SlotLabel::Return => "          -> ".to_string(),
        };

        match slot.kind {
            DataFrameAnnotation::Typed(name) => {
                if schemas.iter().any(|s| s.name() == name) {
                    writeln!(out, "{prefix}DataFrame[{name}]").unwrap();
                } else {
                    writeln!(out, "{prefix}{raw_text}  (unresolved)").unwrap();
                    diagnostics.push(Diagnostic::at(
                        Severity::Error,
                        "D0020",
                        format!(
                            "Unknown schema '{name}' referenced in DataFrame[…]. \
                             Declare it as a class extending Schema.",
                        ),
                        ann_range.start(),
                        source,
                        line_index,
                    ));
                }
            }
            DataFrameAnnotation::Untyped => {
                writeln!(out, "{prefix}DataFrame  (untyped)").unwrap();
            }
            DataFrameAnnotation::NonBareName => {
                writeln!(out, "{prefix}{raw_text}  (unresolved)").unwrap();
                diagnostics.push(Diagnostic::at(
                    Severity::Error,
                    "D0021",
                    format!(
                        "DataFrame schema must be a bare name; got '{raw_text}'. \
                         Subscripted/complex schema expressions are not supported in v0.1.",
                    ),
                    ann_range.start(),
                    source,
                    line_index,
                ));
            }
        }
    }
}
