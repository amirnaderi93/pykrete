mod diagnostics;
mod schema;
mod walk;

use std::env;
use std::fs;
use std::process::ExitCode;

use ruff_source_file::LineIndex;
use ruff_text_size::Ranged;

use crate::diagnostics::{Diagnostic, Severity};
use crate::schema::discover_schemas;
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

    match ruff_python_parser::parse_module(&source) {
        Ok(parsed) => {
            let module = parsed.syntax();
            let classes = discover_top_level_classes(module);
            let schemas = discover_schemas(&classes);
            println!(
                "{}: parsed OK — {} top-level class(es), {} schema(s)",
                path,
                classes.len(),
                schemas.len(),
            );
            for schema in &schemas {
                let lc = line_index.line_column(schema.class.def.range.start(), &source);
                println!(
                    "  {}:{}  schema {}",
                    lc.line.get(),
                    lc.column.get(),
                    schema.name(),
                );
                for field in schema.fields() {
                    let ann_text = &source[field.annotation.range()];
                    println!("          {}: {}", field.name, ann_text);
                }
            }
            ExitCode::SUCCESS
        }
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
            ExitCode::from(1)
        }
    }
}
