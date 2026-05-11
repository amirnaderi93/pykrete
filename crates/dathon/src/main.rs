mod diagnostics;
mod walk;

use std::env;
use std::fs;
use std::process::ExitCode;

use ruff_source_file::LineIndex;

use crate::diagnostics::{Diagnostic, Severity};
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
            println!(
                "{}: parsed OK — {} top-level statement(s), {} class(es) found",
                path,
                module.body.len(),
                classes.len(),
            );
            for class in &classes {
                let lc = line_index.line_column(class.def.range.start(), &source);
                let bases = class.base_names();
                let suffix = if bases.is_empty() {
                    String::new()
                } else {
                    format!("({})", bases.join(", "))
                };
                println!(
                    "  {}:{}  class {}{}",
                    lc.line.get(),
                    lc.column.get(),
                    class.name(),
                    suffix,
                );
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
