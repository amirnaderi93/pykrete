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

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response, ResponseError};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, Location, MarkupContent, MarkupKind, NumberOrString, OneOf, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url,
};

pub fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
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
                handle_request(&connection, &docs, req)?;
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

fn handle_request(
    connection: &Connection,
    docs: &HashMap<Url, String>,
    req: Request,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        "textDocument/hover" => {
            let params: HoverParams = serde_json::from_value(req.params)?;
            let response_value = handle_hover(docs, params);
            connection.sender.send(Message::Response(Response {
                id: req.id,
                result: Some(serde_json::to_value(response_value)?),
                error: None,
            }))?;
        }
        "textDocument/documentSymbol" => {
            let params: DocumentSymbolParams = serde_json::from_value(req.params)?;
            let response_value = handle_document_symbol(docs, params);
            connection.sender.send(Message::Response(Response {
                id: req.id,
                result: Some(serde_json::to_value(response_value)?),
                error: None,
            }))?;
        }
        "textDocument/definition" => {
            let params: GotoDefinitionParams = serde_json::from_value(req.params)?;
            let response_value = handle_definition(docs, params);
            connection.sender.send(Message::Response(Response {
                id: req.id,
                result: Some(serde_json::to_value(response_value)?),
                error: None,
            }))?;
        }
        // Unknown methods get a MethodNotFound error so the client can
        // distinguish "the server is broken" from "the server doesn't
        // support this feature yet".
        _ => {
            connection.sender.send(Message::Response(Response {
                id: req.id,
                result: None,
                error: Some(ResponseError {
                    code: ErrorCode::MethodNotFound as i32,
                    message: format!("dathon-lsp doesn't yet handle '{}'", req.method),
                    data: None,
                }),
            }))?;
        }
    }
    Ok(())
}

/// Handle a `textDocument/hover` request by routing through
/// `dathon::hover`. Returns `None` if no symbol is at the cursor —
/// LSP's contract is that the response body in that case is `null`,
/// which our `serde_json::to_value(None::<Hover>)` produces.
pub fn handle_hover(docs: &HashMap<Url, String>, params: HoverParams) -> Option<Hover> {
    let uri = &params.text_document_position_params.text_document.uri;
    let pos = params.text_document_position_params.position;
    let text = docs.get(uri)?;
    // LSP positions are 0-indexed; dathon's hover entry point is 1-indexed.
    let line = (pos.line as usize).checked_add(1)?;
    let column = (pos.character as usize).checked_add(1)?;
    let info = dathon::hover(text, line, column)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: info.markdown,
        }),
        range: None,
    })
}

/// Handle a `textDocument/documentSymbol` request by routing through
/// `dathon::document_symbols`. Returns the document outline as a nested
/// list of `DocumentSymbol`s — VS Code renders this in the breadcrumb
/// bar and the file outline panel.
pub fn handle_document_symbol(
    docs: &HashMap<Url, String>,
    params: DocumentSymbolParams,
) -> Option<DocumentSymbolResponse> {
    let text = docs.get(&params.text_document.uri)?;
    let symbols = dathon::document_symbols(text);
    let converted: Vec<DocumentSymbol> = symbols.iter().map(to_lsp_symbol).collect();
    Some(DocumentSymbolResponse::Nested(converted))
}

/// Handle a `textDocument/definition` request by routing through
/// `dathon::definition`. Returns the source range of the declaration
/// that the cursor points at (single-file only in v0.1).
pub fn handle_definition(
    docs: &HashMap<Url, String>,
    params: GotoDefinitionParams,
) -> Option<GotoDefinitionResponse> {
    let uri = params
        .text_document_position_params
        .text_document
        .uri
        .clone();
    let pos = params.text_document_position_params.position;
    let text = docs.get(&uri)?;
    let line = (pos.line as usize).checked_add(1)?;
    let column = (pos.character as usize).checked_add(1)?;
    let span = dathon::definition(text, line, column)?;
    let location = Location {
        uri,
        range: span_to_range(span),
    };
    Some(GotoDefinitionResponse::Scalar(location))
}

#[allow(deprecated)]
fn to_lsp_symbol(s: &dathon::symbols::DocumentSymbol) -> DocumentSymbol {
    DocumentSymbol {
        name: s.name.clone(),
        detail: s.detail.clone(),
        kind: match s.kind {
            dathon::SymbolKind::Class => SymbolKind::CLASS,
            dathon::SymbolKind::Field => SymbolKind::FIELD,
            dathon::SymbolKind::Function => SymbolKind::FUNCTION,
        },
        tags: None,
        // `deprecated` is deprecated in the LSP spec itself but lsp_types
        // still exposes it as a non-optional-ish field; `None` is the
        // forward-compatible value.
        deprecated: None,
        range: span_to_range(s.range),
        selection_range: span_to_range(s.selection_range),
        children: if s.children.is_empty() {
            None
        } else {
            Some(s.children.iter().map(to_lsp_symbol).collect())
        },
    }
}

fn span_to_range(span: dathon::Span) -> Range {
    Range {
        start: Position {
            line: span.start_line.saturating_sub(1) as u32,
            character: span.start_column.saturating_sub(1) as u32,
        },
        end: Position {
            line: span.end_line.saturating_sub(1) as u32,
            character: span.end_column.saturating_sub(1) as u32,
        },
    }
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
/// 1-indexed; LSP is 0-indexed. The diagnostic carries a start and end
/// position so editors can underline the entire offending token.
pub fn to_lsp_diagnostic(d: &dathon::diagnostics::Diagnostic) -> Diagnostic {
    let start = Position {
        line: d.line.saturating_sub(1) as u32,
        character: d.column.saturating_sub(1) as u32,
    };
    let end = Position {
        line: d.end_line.saturating_sub(1) as u32,
        character: d.end_column.saturating_sub(1) as u32,
    };
    let range = Range { start, end };
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
    fn dathon_diag(
        start: (usize, usize),
        end: (usize, usize),
        code: &'static str,
        msg: &str,
    ) -> DathonDiagnostic {
        DathonDiagnostic {
            severity: Severity::Error,
            code,
            message: msg.to_string(),
            line: start.0,
            column: start.1,
            end_line: end.0,
            end_column: end.1,
        }
    }

    #[test]
    fn dathon_line_column_become_zero_indexed_lsp_position() {
        // dathon: line=3, column=5 (1-indexed) → LSP: line=2, character=4
        let d = dathon_diag((3, 5), (3, 10), "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.range.start.line, 2);
        assert_eq!(lsp.range.start.character, 4);
    }

    #[test]
    fn lsp_diagnostic_range_spans_to_end_position() {
        // A diagnostic that covers `"BadColumn"` (10 chars including quotes)
        // should produce a range whose end is offset by the token length.
        let d = dathon_diag((2, 5), (2, 15), "D0030", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.range.start.line, 1);
        assert_eq!(lsp.range.start.character, 4);
        assert_eq!(lsp.range.end.line, 1);
        assert_eq!(lsp.range.end.character, 14);
    }

    #[test]
    fn dathon_error_severity_maps_to_lsp_error() {
        let d = dathon_diag((1, 1), (1, 1), "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn dathon_code_becomes_lsp_string_code() {
        // The code shows up alongside the diagnostic in the editor's UI;
        // it must be the exact dathon code string (D0030, etc.) so users
        // can grep the docs for it.
        let d = dathon_diag(
            (1, 1),
            (1, 1),
            "D0030",
            "Column 'X' does not exist on schema 'Orders'.",
        );
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.code, Some(NumberOrString::String("D0030".to_string())));
    }

    #[test]
    fn lsp_diagnostic_source_is_dathon() {
        let d = dathon_diag((1, 1), (1, 1), "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.source.as_deref(), Some("dathon"));
    }

    #[test]
    fn dathon_message_is_preserved_verbatim() {
        let msg = "Column 'priec' does not exist on schema 'Orders'.";
        let d = dathon_diag((1, 1), (1, 1), "D0030", msg);
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.message, msg);
    }

    // -----------------------------------------------------------------------
    // hover handler
    // -----------------------------------------------------------------------

    use lsp_types::{
        HoverContents, PartialResultParams, TextDocumentIdentifier, TextDocumentPositionParams,
        WorkDoneProgressParams,
    };

    fn hover_params_at(uri: &Url, line: u32, character: u32) -> HoverParams {
        HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        }
    }

    #[test]
    fn handle_hover_returns_none_when_uri_not_open() {
        // The server only knows about documents the client has told it
        // about via didOpen. Asking for hover on an unknown URI must
        // return None (which serializes to null per the LSP contract).
        let docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///nonexistent.dpy").unwrap();
        let result = handle_hover(&docs, hover_params_at(&uri, 0, 0));
        assert!(result.is_none());
    }

    #[test]
    fn handle_hover_returns_markdown_hover_for_a_schema_class_name() {
        // didOpen sets the doc text; hover on the Schema class name
        // returns a Hover with markdown content listing the fields.
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.dpy").unwrap();
        let src = "class Orders(Schema):\n    place_code: int\n    price: int\n";
        docs.insert(uri.clone(), src.to_string());

        // LSP position is 0-indexed: line 0, character 6 lands inside
        // "Orders" on the first line.
        let result = handle_hover(&docs, hover_params_at(&uri, 0, 6)).expect("hover");
        match result.contents {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("Orders"));
                assert!(m.value.contains("place_code"));
            }
            _ => panic!("expected MarkupContent"),
        }
    }

    #[test]
    fn handle_hover_translates_lsp_0_indexed_to_dathon_1_indexed() {
        // The fixture starts with a blank line so the schema is on
        // line 1 (0-indexed). If position translation is wrong, hover
        // would miss the symbol and return None.
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.dpy").unwrap();
        let src = "\nclass Orders(Schema):\n    x: int\n";
        docs.insert(uri.clone(), src.to_string());

        // Line 1 (0-indexed), character 6 → "Orders".
        let result = handle_hover(&docs, hover_params_at(&uri, 1, 6));
        assert!(result.is_some());
    }

    // -----------------------------------------------------------------------
    // documentSymbol handler
    // -----------------------------------------------------------------------

    use lsp_types::{DocumentSymbolParams as DocSymParams, GotoDefinitionParams as DefParams};

    fn doc_sym_params(uri: &Url) -> DocSymParams {
        DocSymParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        }
    }

    fn def_params_at(uri: &Url, line: u32, character: u32) -> DefParams {
        DefParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        }
    }

    #[test]
    fn handle_document_symbol_returns_none_when_uri_not_open() {
        let docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///nonexistent.dpy").unwrap();
        let result = handle_document_symbol(&docs, doc_sym_params(&uri));
        assert!(result.is_none());
    }

    #[test]
    fn handle_document_symbol_returns_nested_outline_with_schema_class_and_function() {
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.dpy").unwrap();
        let src = "class Orders(Schema):\n    x: int\n\ndef f(raw: DataFrame[Orders]) -> DataFrame[Orders]:\n    return raw\n";
        docs.insert(uri.clone(), src.to_string());

        let result = handle_document_symbol(&docs, doc_sym_params(&uri)).expect("response");
        match result {
            DocumentSymbolResponse::Nested(syms) => {
                assert_eq!(syms.len(), 2);
                assert_eq!(syms[0].name, "Orders");
                assert_eq!(syms[0].kind, SymbolKind::CLASS);
                let children = syms[0].children.as_ref().expect("schema children");
                assert_eq!(children.len(), 1);
                assert_eq!(children[0].name, "x");
                assert_eq!(children[0].kind, SymbolKind::FIELD);
                assert_eq!(syms[1].name, "f");
                assert_eq!(syms[1].kind, SymbolKind::FUNCTION);
            }
            other => panic!("expected Nested, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // definition handler
    // -----------------------------------------------------------------------

    #[test]
    fn handle_definition_returns_none_when_uri_not_open() {
        let docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///nonexistent.dpy").unwrap();
        let result = handle_definition(&docs, def_params_at(&uri, 0, 0));
        assert!(result.is_none());
    }

    #[test]
    fn handle_definition_resolves_DataFrame_inner_schema_to_class_decl() {
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.dpy").unwrap();
        // Line 0: class header
        // Line 1: field
        // Line 2: blank
        // Line 3: def header — Orders appears at column 21 (after "def f(raw: DataFrame[")
        let src = "class Orders(Schema):\n    x: int\n\ndef f(raw: DataFrame[Orders]) -> DataFrame[Orders]:\n    return raw\n";
        docs.insert(uri.clone(), src.to_string());

        // Click on `Orders` inside `DataFrame[Orders]` on line 3.
        let result = handle_definition(&docs, def_params_at(&uri, 3, 22)).expect("response");
        match result {
            GotoDefinitionResponse::Scalar(loc) => {
                assert_eq!(loc.uri, uri);
                // Should jump to the class declaration name on line 0 at
                // character 6 ("class " is 6 chars).
                assert_eq!(loc.range.start.line, 0);
                assert_eq!(loc.range.start.character, 6);
            }
            other => panic!("expected Scalar location, got {other:?}"),
        }
    }
}
