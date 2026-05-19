//! Thin entry point. All the work lives in the library.

use std::process::ExitCode;

fn main() -> ExitCode {
    match pykrete_lsp::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pykrete-lsp error: {e}");
            ExitCode::from(1)
        }
    }
}
