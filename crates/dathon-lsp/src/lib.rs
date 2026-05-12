//! dathon-lsp — Language Server Protocol server for dathon.
//!
//! Editors launch the `dathon-lsp` binary and speak JSON-RPC to it over
//! stdio. On every text-document change, the server runs dathon's checker
//! and pushes diagnostics back. v0.1 LSP is single-file analysis with FULL
//! text sync; workspace-folder / multi-file analysis lands in a follow-up.
//!
//! The library entry point is [`run`]; `main.rs` is a thin shell.
//!
//! Implementation conventions:
//!
//! - Sync, single-threaded server loop via `lsp-server`. No async runtime;
//!   each notification is handled in order and the checker runs inline.
//! - `FULL` text-document sync — the client sends the entire new buffer
//!   on every `didChange`. Incremental sync is a follow-up.
//! - Diagnostics are zero-width at their start position; editors typically
//!   extend the underline to the word at that position.

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, Message, Notification};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, NumberOrString, Position, PublishDiagnosticsParams, Range,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};

pub fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    })?;
    connection.initialize(server_capabilities)?;

    main_loop(connection)?;
    io_threads.join()?;
    Ok(())
}

fn main_loop(connection: Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    // In-memory shadow of every open document. `didOpen` adds; `didChange`
    // overwrites (FULL sync); `didClose` removes.
    let mut docs: HashMap<Url, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                // No other request handlers in the skeleton iteration.
                // Hover / definition / symbols land in subsequent iterations.
            }
            Message::Notification(notif) => {
                handle_notification(&connection, &mut docs, notif)?;
            }
            Message::Response(_) => {
                // We don't currently send requests TO the client, so we
                // don't expect responses back.
            }
        }
    }
    Ok(())
}

fn handle_notification(
    connection: &Connection,
    docs: &mut HashMap<Url, String>,
    notif: Notification,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match notif.method.as_str() {
        "textDocument/didOpen" => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(notif.params)?;
            let uri = params.text_document.uri.clone();
            let text = params.text_document.text;
            docs.insert(uri.clone(), text.clone());
            publish_diagnostics(connection, &uri, &text)?;
        }
        "textDocument/didChange" => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(notif.params)?;
            let uri = params.text_document.uri.clone();
            // FULL sync — content_changes is a single entry whose `text`
            // field is the entire new buffer.
            if let Some(change) = params.content_changes.into_iter().next() {
                let text = change.text;
                docs.insert(uri.clone(), text.clone());
                publish_diagnostics(connection, &uri, &text)?;
            }
        }
        "textDocument/didClose" => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(notif.params)?;
            let uri = params.text_document.uri;
            docs.remove(&uri);
            // Clear any existing diagnostics for the closed file so the
            // editor doesn't show stale errors.
            publish_empty_diagnostics(connection, &uri)?;
        }
        // didSave, willSave, etc. are ignored — diagnostics already update
        // on every didChange. `initialized` arrives once and is a no-op.
        _ => {}
    }
    Ok(())
}

/// Run the checker and push diagnostics for one document.
fn publish_diagnostics(
    connection: &Connection,
    uri: &Url,
    text: &str,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let path = uri_to_path(uri);
    let result = dathon::check(&path, text);
    let diagnostics: Vec<Diagnostic> = result.diagnostics.iter().map(to_lsp_diagnostic).collect();
    send_diagnostics(connection, uri, diagnostics)
}

/// Push an empty diagnostic list — used when a document closes, to clear
/// any underlines the editor was still showing.
fn publish_empty_diagnostics(
    connection: &Connection,
    uri: &Url,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    send_diagnostics(connection, uri, Vec::new())
}

fn send_diagnostics(
    connection: &Connection,
    uri: &Url,
    diagnostics: Vec<Diagnostic>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics,
        version: None,
    };
    connection.sender.send(Message::Notification(Notification {
        method: "textDocument/publishDiagnostics".to_string(),
        params: serde_json::to_value(params)?,
    }))?;
    Ok(())
}

/// Best-effort conversion of a file:// URL to a filesystem path for
/// passing into dathon's checker (used purely as the path embedded in
/// diagnostic messages). Non-file URIs fall back to the URI's stringified
/// form.
fn uri_to_path(uri: &Url) -> String {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| uri.to_string())
}

/// Translate dathon's diagnostic format into LSP's. dathon's positions are
/// 1-indexed; LSP is 0-indexed. The diagnostic carries only a start point;
/// we emit a zero-width range and let the editor extend it to the word at
/// the position.
pub fn to_lsp_diagnostic(d: &dathon::diagnostics::Diagnostic) -> Diagnostic {
    let line = d.line.saturating_sub(1) as u32;
    let character = d.column.saturating_sub(1) as u32;
    let position = Position { line, character };
    let range = Range {
        start: position,
        end: position,
    };
    Diagnostic {
        range,
        severity: Some(match d.severity {
            dathon::diagnostics::Severity::Error => DiagnosticSeverity::ERROR,
            dathon::diagnostics::Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(d.code.to_string())),
        code_description: None,
        source: Some("dathon".to_string()),
        message: d.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dathon::diagnostics::{Diagnostic as DathonDiagnostic, Severity};

    /// Helper: build a dathon Diagnostic at a known line/column. We use
    /// the struct's public fields directly rather than the `Diagnostic::at`
    /// constructor (which would require pulling in `ruff_source_file` just
    /// for the LineIndex). The conversion code under test only reads
    /// these fields anyway.
    fn dathon_diag_at(
        line: usize,
        column: usize,
        code: &'static str,
        msg: &str,
    ) -> DathonDiagnostic {
        DathonDiagnostic {
            severity: Severity::Error,
            code,
            message: msg.to_string(),
            line,
            column,
        }
    }

    #[test]
    fn dathon_line_column_become_zero_indexed_lsp_position() {
        // dathon: line=3, column=5 (1-indexed) → LSP: line=2, character=4
        let d = dathon_diag_at(3, 5, "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.range.start.line, 2);
        assert_eq!(lsp.range.start.character, 4);
    }

    #[test]
    fn lsp_diagnostic_range_is_zero_width_at_the_start_position() {
        // dathon only carries a start position. The editor extends the
        // underline to the word boundary on its end, so a zero-width range
        // is the right thing to send.
        let d = dathon_diag_at(1, 1, "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.range.start, lsp.range.end);
    }

    #[test]
    fn dathon_error_severity_maps_to_lsp_error() {
        let d = dathon_diag_at(1, 1, "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn dathon_code_becomes_lsp_string_code() {
        // The code shows up alongside the diagnostic in the editor's UI;
        // it must be the exact dathon code string (D0030, etc.) so users
        // can grep the docs for it.
        let d = dathon_diag_at(
            1,
            1,
            "D0030",
            "Column 'X' does not exist on schema 'Orders'.",
        );
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.code, Some(NumberOrString::String("D0030".to_string())));
    }

    #[test]
    fn lsp_diagnostic_source_is_dathon() {
        let d = dathon_diag_at(1, 1, "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.source.as_deref(), Some("dathon"));
    }

    #[test]
    fn dathon_message_is_preserved_verbatim() {
        let msg = "Column 'priec' does not exist on schema 'Orders'.";
        let d = dathon_diag_at(1, 1, "D0030", msg);
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.message, msg);
    }
}
