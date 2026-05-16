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

pub mod child;
pub mod multiplex;
pub mod project;
pub mod virtualdoc;

use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use lsp_server::{
    Connection, ErrorCode, Message, Notification, Request, RequestId, Response, ResponseError,
};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, CodeActionResponse,
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, Location, MarkupContent, MarkupKind, NumberOrString, OneOf, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit,
};

pub fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    // dathon's own capabilities. The embedded Python engine's are folded
    // in below, after it handshakes.
    let dathon_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        completion_provider: Some(CompletionOptions {
            // Editors invoke completion automatically after the user types
            // one of these characters. `"` for `col("…")`, `.` for `df.…`,
            // `[` for `DataFrame[…]`.
            trigger_characters: Some(vec!["\"".to_string(), ".".to_string(), "[".to_string()]),
            ..Default::default()
        }),
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
        ..Default::default()
    })?;

    // Manual `initialize` handshake: we need the editor's params to
    // spawn the embedded engine, and the engine's capabilities to fold
    // into what we advertise back. So receive `initialize`, bring the
    // engine up, then answer with the merged capability set.
    let (init_id, init_params) = connection.initialize_start()?;

    // Spawn + handshake the embedded Python LSP. `explicit` is the
    // `dathon.pythonServer.path` initialization option, if the client
    // set one. When no engine is found the multiplexer's `child` is
    // `None` and dathon-lsp runs dathon-only.
    let explicit = init_params
        .get("initializationOptions")
        .and_then(|o| o.get("pythonServerPath"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let multiplexer = multiplex::Multiplexer::start(explicit.as_deref(), &init_params);

    // Advertise dathon's capabilities plus the embedded engine's, for
    // the methods dathon-lsp knows how to proxy.
    let capabilities =
        multiplex::merge_capabilities(dathon_capabilities, multiplexer.child_capabilities());
    let initialize_result = serde_json::json!({
        "capabilities": capabilities,
        "serverInfo": { "name": "dathon-lsp", "version": env!("CARGO_PKG_VERSION") },
    });
    connection.initialize_finish(init_id, initialize_result)?;

    main_loop(connection, multiplexer)?;
    io_threads.join()?;
    Ok(())
}

fn main_loop(
    connection: Connection,
    mut multiplexer: multiplex::Multiplexer,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    // In-memory shadow of every open document. `didOpen` adds; `didChange`
    // overwrites (FULL sync); `didClose` removes.
    let mut docs: HashMap<Url, String> = HashMap::new();

    // The embedded Python engine's outbound message channel. When no
    // engine is embedded we use a never-ready receiver so `select!`
    // simply only ever fires on the editor side. When the engine later
    // exits, `child_rx` is swapped back to `never()` so the closed
    // channel doesn't spin the select loop.
    let mut child_rx = multiplexer
        .child
        .as_ref()
        .map(|c| c.receiver.clone())
        .unwrap_or_else(crossbeam_channel::never);

    loop {
        crossbeam_channel::select! {
            recv(connection.receiver) -> msg => {
                let Ok(msg) = msg else {
                    break; // editor disconnected
                };
                match msg {
                    Message::Request(req) => {
                        if connection.handle_shutdown(&req)? {
                            multiplexer.shutdown();
                            return Ok(());
                        }
                        handle_request(&connection, &docs, &mut multiplexer, req)?;
                    }
                    Message::Notification(notif) => {
                        handle_notification(&connection, &mut docs, &mut multiplexer, notif)?;
                    }
                    Message::Response(resp) => {
                        // A response from the editor to a request we
                        // proxied on the embedded engine's behalf —
                        // route it back to the engine.
                        handle_editor_response(&mut multiplexer, resp);
                    }
                }
            }
            recv(child_rx) -> msg => {
                let Ok(msg) = msg else {
                    // The embedded engine exited. Answer any in-flight
                    // fanned-out requests with dathon's own result so
                    // they don't hang, drop the child, and swap the
                    // channel to `never()` so this branch stops firing.
                    for pending in multiplexer.drain_pending() {
                        connection.sender.send(Message::Response(Response {
                            id: pending.editor_id,
                            result: Some(pending.dathon_result),
                            error: None,
                        }))?;
                    }
                    multiplexer.child = None;
                    child_rx = crossbeam_channel::never();
                    continue;
                };
                handle_child_message(&connection, &mut multiplexer, msg)?;
            }
        }
    }
    multiplexer.shutdown();
    Ok(())
}

/// Handle one message emitted by the embedded Python engine.
///
/// - `publishDiagnostics` notifications are remapped out of
///   virtual-document coordinates and merged with dathon's diagnostics.
/// - Responses to requests we fanned out (`dathon-req-*`) are merged
///   with dathon's own result and forwarded to the editor.
/// - Requests the engine sends us (`workspace/configuration`,
///   `client/registerCapability`, …) are proxied to the real editor;
///   the editor's reply is routed back from `handle_editor_response`.
/// - Notifications the engine emits (`window/logMessage`,
///   `window/showMessage`, `$/progress`, …) are relayed to the editor
///   verbatim.
fn handle_child_message(
    connection: &Connection,
    multiplexer: &mut multiplex::Multiplexer,
    msg: serde_json::Value,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let method = msg.get("method").and_then(|m| m.as_str());
    let id = msg.get("id");

    match (method, id) {
        (Some("textDocument/publishDiagnostics"), _) => {
            if let Some(params) = msg.get("params")
                && let Some((uri, child_diags)) = multiplex::child_diagnostics_to_editor(params)
            {
                let merged = multiplexer.diagnostics.set_child(uri.clone(), child_diags);
                send_diagnostics(connection, &uri, merged)?;
            }
        }
        (Some(method), Some(child_id)) => {
            // A request from the engine — proxy it to the real editor
            // so the engine sees the editor's actual answers (Python
            // interpreter / analysis config, progress tokens, …). The
            // editor's reply comes back via `handle_editor_response`.
            let editor_id = multiplexer.register_child_request(child_id.clone());
            connection.sender.send(Message::Request(Request {
                id: RequestId::from(editor_id),
                method: method.to_string(),
                params: msg
                    .get("params")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            }))?;
        }
        (None, Some(id)) => {
            // A response to a request we fanned out. Match it to the
            // parked editor request, merge dathon's result with the
            // child's, and send the combined reply to the editor.
            if let Some(child_id) = id.as_str()
                && let Some(pending) = multiplexer.take_pending(child_id)
            {
                // An `error` response (or no `result`) becomes `Null` —
                // the merge then just yields dathon's own answer.
                let child_result = msg
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let merged = multiplex::merge_child_response(
                    &pending.method,
                    pending.dathon_result,
                    child_result,
                );
                connection.sender.send(Message::Response(Response {
                    id: pending.editor_id,
                    result: Some(merged),
                    error: None,
                }))?;
            }
        }
        (Some(method), None) => {
            // A notification from the engine (window/logMessage,
            // window/showMessage, $/progress, …). These carry no
            // document coordinates, so relay them to the editor
            // verbatim — log messages land in its LSP output channel.
            connection.sender.send(Message::Notification(Notification {
                method: method.to_string(),
                params: msg
                    .get("params")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            }))?;
        }
        _ => {
            // A response with a non-string id, or some other shape we
            // don't route. Nothing actionable.
        }
    }
    Ok(())
}

/// Route the editor's response to a proxied child request back to the
/// embedded engine.
///
/// Every `Message::Response` from the editor is an answer to a request
/// dathon-lsp forwarded on the engine's behalf — dathon-lsp issues the
/// editor no requests of its own. A response whose id isn't in the
/// proxy table is ignored.
fn handle_editor_response(multiplexer: &mut multiplex::Multiplexer, resp: Response) {
    let Some(editor_id) = request_id_as_str(&resp.id) else {
        return;
    };
    let Some(child_id) = multiplexer.take_child_request(&editor_id) else {
        return;
    };
    let error = resp.error.and_then(|e| serde_json::to_value(e).ok());
    multiplexer.respond_to_child(child_id, resp.result, error);
}

/// The string form of a `RequestId`. dathon-lsp issues proxied-request
/// ids as `dathon-c2e-N` strings, so a non-string id is one we never
/// issued.
fn request_id_as_str(id: &RequestId) -> Option<String> {
    match serde_json::to_value(id).ok()? {
        serde_json::Value::String(s) => Some(s),
        _ => None,
    }
}

fn handle_request(
    connection: &Connection,
    docs: &HashMap<Url, String>,
    multiplexer: &mut multiplex::Multiplexer,
    req: Request,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        // hover / definition / completion are fanned out: dathon's own
        // answer is computed here, then merged with the embedded Python
        // engine's. When no engine is embedded the reply goes out
        // immediately; otherwise it's deferred until the child answers
        // (see `fanout_request` / `handle_child_message`).
        "textDocument/hover" => {
            let params: HoverParams = serde_json::from_value(req.params.clone())?;
            let dathon_result = serde_json::to_value(handle_hover(docs, params))?;
            fanout_request(connection, multiplexer, req, dathon_result)?;
        }
        "textDocument/definition" => {
            let params: GotoDefinitionParams = serde_json::from_value(req.params.clone())?;
            let dathon_result = serde_json::to_value(handle_definition(docs, params))?;
            fanout_request(connection, multiplexer, req, dathon_result)?;
        }
        "textDocument/completion" => {
            let params: CompletionParams = serde_json::from_value(req.params.clone())?;
            let dathon_result = serde_json::to_value(handle_completion(docs, params))?;
            fanout_request(connection, multiplexer, req, dathon_result)?;
        }
        // signatureHelp / references are pure passthrough — dathon has
        // no schema-specific answer, so dathon's result is `null` and
        // the merge just returns the (remapped) child result. These are
        // only advertised to the editor when the embedded engine
        // provides them (see `merge_capabilities`).
        "textDocument/signatureHelp" | "textDocument/references" => {
            fanout_request(connection, multiplexer, req, serde_json::Value::Null)?;
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
        "textDocument/codeAction" => {
            let params: CodeActionParams = serde_json::from_value(req.params)?;
            let response_value = handle_code_action(params);
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

/// Fan a request out to the embedded Python engine.
///
/// `dathon_result` is dathon's own answer, already computed. When an
/// engine is embedded the request is forwarded (its cursor position
/// shifted into virtual-document coordinates) and the editor reply is
/// deferred — `handle_child_message` merges the two answers and
/// replies. When there's no engine, dathon's result is sent right away.
fn fanout_request(
    connection: &Connection,
    multiplexer: &mut multiplex::Multiplexer,
    req: Request,
    dathon_result: serde_json::Value,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let child_params = multiplex::request_params_to_child(req.params);
    if let Some(dathon_only) =
        multiplexer.forward_request(req.id.clone(), &req.method, child_params, dathon_result)
    {
        connection.sender.send(Message::Response(Response {
            id: req.id,
            result: Some(dathon_only),
            error: None,
        }))?;
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
    let _ = docs.get(uri)?; // ensure the URI is open
    // LSP positions are 0-indexed; dathon's hover entry point is 1-indexed.
    let line = (pos.line as usize).checked_add(1)?;
    let column = (pos.character as usize).checked_add(1)?;
    let info = match project::build_project_snapshot(docs) {
        Some(snapshot) => {
            let focus_path = uri.to_file_path().ok()?;
            let focus_path_str = focus_path.to_string_lossy().to_string();
            dathon::hover_in_project(&snapshot, &focus_path_str, line, column)?
        }
        None => {
            // Single-file fallback for untitled / non-file URIs.
            let text = docs.get(uri)?;
            dathon::hover(text, line, column)?
        }
    };
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
    let _ = docs.get(&uri)?;
    let line = (pos.line as usize).checked_add(1)?;
    let column = (pos.character as usize).checked_add(1)?;
    let span = match project::build_project_snapshot(docs) {
        Some(snapshot) => {
            let focus_path = uri.to_file_path().ok()?;
            let focus_path_str = focus_path.to_string_lossy().to_string();
            dathon::definition_in_project(&snapshot, &focus_path_str, line, column)?
        }
        None => {
            let text = docs.get(&uri)?;
            dathon::definition(text, line, column)?
        }
    };
    let location = Location {
        uri,
        range: span_to_range(span),
    };
    Some(GotoDefinitionResponse::Scalar(location))
}

/// Handle a `textDocument/completion` request by routing through
/// `dathon::completions`. Three completion surfaces are recognized: the
/// `X` in `DataFrame[X]`, the literal inside `col("…")`, and the attr
/// after `df.`. Returns an empty list (encoded as `CompletionList`) when
/// no completions apply at the cursor.
pub fn handle_completion(
    docs: &HashMap<Url, String>,
    params: CompletionParams,
) -> Option<CompletionResponse> {
    let uri = &params.text_document_position.text_document.uri;
    let pos = params.text_document_position.position;
    let _ = docs.get(uri)?;
    let line = (pos.line as usize).checked_add(1)?;
    let column = (pos.character as usize).checked_add(1)?;
    let raw_items = match project::build_project_snapshot(docs) {
        Some(snapshot) => {
            let focus_path = uri.to_file_path().ok()?;
            let focus_path_str = focus_path.to_string_lossy().to_string();
            dathon::completions_in_project(&snapshot, &focus_path_str, line, column)
        }
        None => {
            let text = docs.get(uri)?;
            dathon::completions(text, line, column)
        }
    };
    let items: Vec<CompletionItem> = raw_items.into_iter().map(to_lsp_completion_item).collect();
    Some(CompletionResponse::Array(items))
}

fn to_lsp_completion_item(item: dathon::CompletionItem) -> CompletionItem {
    CompletionItem {
        label: item.label,
        detail: item.detail,
        kind: Some(match item.kind {
            dathon::CompletionItemKind::Class => CompletionItemKind::CLASS,
            dathon::CompletionItemKind::Field => CompletionItemKind::FIELD,
        }),
        ..Default::default()
    }
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
    multiplexer: &mut multiplex::Multiplexer,
    notif: Notification,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match notif.method.as_str() {
        "textDocument/didOpen" => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(notif.params)?;
            let uri = params.text_document.uri.clone();
            let text = params.text_document.text;
            // Forward to the embedded Python engine as a virtual document
            // before we stash the real text — `forward_did_open` does the
            // preamble injection internally.
            multiplexer.forward_did_open(
                &uri,
                &params.text_document.language_id,
                params.text_document.version as i64,
                &text,
            );
            docs.insert(uri, text);
            publish_project_diagnostics(connection, docs, multiplexer)?;
        }
        "textDocument/didChange" => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(notif.params)?;
            let uri = params.text_document.uri.clone();
            let version = params.text_document.version as i64;
            // FULL sync — content_changes is a single entry whose `text`
            // field is the entire new buffer.
            if let Some(change) = params.content_changes.into_iter().next() {
                let text = change.text;
                multiplexer.forward_did_change(&uri, version, &text);
                docs.insert(uri, text);
                publish_project_diagnostics(connection, docs, multiplexer)?;
            }
        }
        "textDocument/didClose" => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(notif.params)?;
            let uri = params.text_document.uri;
            docs.remove(&uri);
            multiplexer.forward_did_close(&uri);
            // Drop the merged-diagnostic state for the closed file and
            // clear any underlines the editor was still showing, then
            // re-publish project diagnostics for the rest — closing a
            // file can change what other open files see (e.g. removing
            // an import target).
            multiplexer.diagnostics.clear(&uri);
            publish_empty_diagnostics(connection, &uri)?;
            publish_project_diagnostics(connection, docs, multiplexer)?;
        }
        // didSave, willSave, etc. are ignored — diagnostics already update
        // on every didChange. `initialized` arrives once and is a no-op.
        _ => {}
    }
    Ok(())
}

/// Run the analyzer over the entire project — every `.dpy` file under
/// the project root, with open documents' in-memory contents
/// overriding disk — and push diagnostics for each currently-open
/// document.
///
/// Falls back to single-file analysis for the open documents when no
/// project root can be derived (e.g. `untitled://` buffers or open
/// files outside any filesystem-rooted project).
fn publish_project_diagnostics(
    connection: &Connection,
    docs: &HashMap<Url, String>,
    multiplexer: &mut multiplex::Multiplexer,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let Some(snapshot) = project::build_project_snapshot(docs) else {
        // Fall back: no usable project root. Check each open doc on
        // its own so we still publish *something* — the user will
        // see the same single-file behaviour we had before iteration 33.
        for (uri, text) in docs {
            publish_single_file_diagnostics(connection, multiplexer, uri, text)?;
        }
        return Ok(());
    };

    let result = dathon::check_project(&snapshot);
    // Map each file in the project back to a Url for diagnostic
    // delivery. Only publish for URIs that are actually open in the
    // editor — closed files' diagnostics would be invisible.
    let open_paths: HashMap<PathBuf, Url> = docs
        .keys()
        .filter_map(|uri| uri.to_file_path().ok().map(|p| (p, uri.clone())))
        .collect();
    for file in &result.files {
        let path = PathBuf::from(&file.path);
        let Some(uri) = open_paths.get(&path) else {
            continue;
        };
        let diagnostics: Vec<Diagnostic> = file
            .result
            .diagnostics
            .iter()
            .map(to_lsp_diagnostic)
            .collect();
        // Route dathon's diagnostics through the merge store so they're
        // published alongside whatever the embedded Python engine last
        // reported for the same file.
        let merged = multiplexer.diagnostics.set_dathon(uri.clone(), diagnostics);
        send_diagnostics(connection, uri, merged)?;
    }
    Ok(())
}

/// Single-file analysis fallback used when no project snapshot is
/// available (untitled buffers, opens outside a filesystem-rooted
/// project). Same behaviour the LSP had before iteration 33.
fn publish_single_file_diagnostics(
    connection: &Connection,
    multiplexer: &mut multiplex::Multiplexer,
    uri: &Url,
    text: &str,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let path = uri_to_path(uri);
    let result = dathon::check(&path, text);
    let diagnostics: Vec<Diagnostic> = result.diagnostics.iter().map(to_lsp_diagnostic).collect();
    let merged = multiplexer.diagnostics.set_dathon(uri.clone(), diagnostics);
    send_diagnostics(connection, uri, merged)
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
    // Round-trip the suggestion in the diagnostic's `data` field so
    // `textDocument/codeAction` can read it back when the editor sends
    // the diagnostic with the code-action request.
    let data = d
        .suggestion
        .as_ref()
        .map(|s| serde_json::json!({ "suggestion": s }));
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
        data,
    }
}

/// Handle a `textDocument/codeAction` request. For each `D0030` diagnostic
/// the editor included in the params that carries a `suggestion` payload,
/// emit a `QuickFix` action whose edit replaces the diagnostic's range
/// with the suggested `"name"` literal.
///
/// Editors render this as a lightbulb on the underlined token; selecting
/// the action swaps in the closest matching column name.
pub fn handle_code_action(params: CodeActionParams) -> CodeActionResponse {
    let uri = params.text_document.uri;
    let mut actions: Vec<CodeActionOrCommand> = Vec::new();
    for diag in params.context.diagnostics {
        // Only D0030 carries a suggestion today.
        let Some(NumberOrString::String(code)) = &diag.code else {
            continue;
        };
        if code != "D0030" {
            continue;
        }
        let Some(suggestion) = diag
            .data
            .as_ref()
            .and_then(|d| d.get("suggestion"))
            .and_then(|s| s.as_str())
        else {
            continue;
        };
        let edit = TextEdit {
            range: diag.range,
            new_text: format!("\"{suggestion}\""),
        };
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        changes.insert(uri.clone(), vec![edit]);
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: format!("Replace with '{suggestion}'"),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diag.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            is_preferred: Some(true),
            ..Default::default()
        }));
    }
    actions
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
            suggestion: None,
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

    // -----------------------------------------------------------------------
    // completion handler
    // -----------------------------------------------------------------------

    fn completion_params_at(uri: &Url, line: u32, character: u32) -> CompletionParams {
        CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: None,
        }
    }

    #[test]
    fn handle_completion_returns_none_when_uri_not_open() {
        let docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///nonexistent.dpy").unwrap();
        let result = handle_completion(&docs, completion_params_at(&uri, 0, 0));
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // diagnostic ↔ suggestion round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn diagnostic_suggestion_is_serialized_into_lsp_data_field() {
        let mut d = dathon_diag(
            (2, 5),
            (2, 15),
            "D0030",
            "Column 'prce' does not exist. Did you mean 'price'?",
        );
        d.suggestion = Some("price".to_string());
        let lsp = to_lsp_diagnostic(&d);
        let data = lsp.data.expect("expected data");
        assert_eq!(data["suggestion"], serde_json::json!("price"));
    }

    // -----------------------------------------------------------------------
    // codeAction handler
    // -----------------------------------------------------------------------

    use lsp_types::{CodeActionContext, CodeActionResponse as CARes};

    fn lsp_diag_with_suggestion(
        start: (u32, u32),
        end: (u32, u32),
        suggestion: &str,
    ) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position {
                    line: start.0,
                    character: start.1,
                },
                end: Position {
                    line: end.0,
                    character: end.1,
                },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("D0030".to_string())),
            code_description: None,
            source: Some("dathon".to_string()),
            message: format!("Column does not exist. Did you mean '{suggestion}'?"),
            related_information: None,
            tags: None,
            data: Some(serde_json::json!({ "suggestion": suggestion })),
        }
    }

    fn code_action_params(uri: &Url, diagnostics: Vec<Diagnostic>) -> CodeActionParams {
        CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
            },
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        }
    }

    #[test]
    fn code_action_offers_quickfix_replacement_for_d0030_with_suggestion() {
        let uri = Url::parse("file:///t.dpy").unwrap();
        let diag = lsp_diag_with_suggestion((1, 18), (1, 25), "price");
        let params = code_action_params(&uri, vec![diag.clone()]);
        let result: CARes = handle_code_action(params);
        assert_eq!(result.len(), 1);
        let CodeActionOrCommand::CodeAction(action) = &result[0] else {
            panic!("expected CodeAction, got {:?}", result[0]);
        };
        assert_eq!(action.title, "Replace with 'price'");
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        let edit = action.edit.as_ref().expect("expected edit");
        let changes = edit.changes.as_ref().expect("expected changes");
        let edits = changes.get(&uri).expect("expected edits for uri");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "\"price\"");
        assert_eq!(edits[0].range, diag.range);
    }

    #[test]
    fn code_action_skips_d0030_diagnostic_without_suggestion_data() {
        let uri = Url::parse("file:///t.dpy").unwrap();
        let mut diag = lsp_diag_with_suggestion((1, 18), (1, 25), "price");
        diag.data = None;
        let params = code_action_params(&uri, vec![diag]);
        let result = handle_code_action(params);
        assert!(result.is_empty());
    }

    #[test]
    fn code_action_skips_non_d0030_diagnostics() {
        let uri = Url::parse("file:///t.dpy").unwrap();
        let mut diag = lsp_diag_with_suggestion((1, 18), (1, 25), "price");
        diag.code = Some(NumberOrString::String("D0020".to_string()));
        let params = code_action_params(&uri, vec![diag]);
        let result = handle_code_action(params);
        assert!(result.is_empty());
    }

    #[test]
    fn handle_completion_returns_schema_names_inside_dataframe_subscript() {
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.dpy").unwrap();
        // Line 0: `class Orders(Schema):` (Orders at col 6)
        // Line 1: field
        // Line 2: blank
        // Line 3: `class Returns(Schema):`
        // Line 4: field
        // Line 5: blank
        // Line 6: `def f(raw: DataFrame[Orders]) -> ...` — cursor inside `Orders` slot.
        let src = "class Orders(Schema):\n    x: int\n\nclass Returns(Schema):\n    y: int\n\ndef f(raw: DataFrame[Orders]) -> DataFrame[Orders]:\n    return raw\n";
        docs.insert(uri.clone(), src.to_string());

        // Cursor inside the `Orders` token of `DataFrame[Orders]` on line 6.
        // `def f(raw: DataFrame[` is 21 characters, so character 22 lands
        // on the first letter of `Orders`.
        let result = handle_completion(&docs, completion_params_at(&uri, 6, 22)).expect("response");
        match result {
            CompletionResponse::Array(items) => {
                let mut names: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
                names.sort();
                assert_eq!(names, vec!["Orders", "Returns"]);
                for item in &items {
                    assert_eq!(item.kind, Some(CompletionItemKind::CLASS));
                }
            }
            other => panic!("expected Array, got {other:?}"),
        }
    }
}
