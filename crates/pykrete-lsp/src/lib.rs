//! pykrete-lsp — Language Server Protocol server for pykrete.
//!
//! Editors launch the `pykrete-lsp` binary and speak JSON-RPC to it over
//! stdio. On every text-document change, the server runs pykrete's checker
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
use std::time::Instant;

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
    PrepareRenameResponse, PublishDiagnosticsParams, Range, ReferenceParams, RenameOptions,
    RenameParams, ServerCapabilities, SymbolKind, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit,
};

pub fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    // pykrete's own capabilities. The embedded Python engine's are folded
    // in below, after it handshakes.
    let pykrete_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
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
    // launch spec the editor supplied (the bundled engine, or a
    // `pykrete.pythonServer.path` override). When it's `None` the
    // multiplexer falls back to `PATH` discovery, and if that finds
    // nothing the `child` is `None` and pykrete-lsp runs pykrete-only.
    let explicit = explicit_python_server(&init_params);
    // The `pykrete.typeCheckingMode` setting drives both pykrete's own
    // checker and the embedded engine; defaults to `standard`.
    let mode = init_params
        .get("initializationOptions")
        .and_then(|options| options.get("typeCheckingMode"))
        .and_then(|value| value.as_str())
        .map(pykrete::CheckMode::parse)
        .unwrap_or_default();
    let multiplexer = multiplex::Multiplexer::start(explicit, mode, &init_params);

    // Advertise pykrete's capabilities plus the embedded engine's, for
    // the methods pykrete-lsp knows how to proxy.
    let capabilities =
        multiplex::merge_capabilities(pykrete_capabilities, multiplexer.child_capabilities());
    let initialize_result = serde_json::json!({
        "capabilities": capabilities,
        "serverInfo": { "name": "pykrete-lsp", "version": env!("CARGO_PKG_VERSION") },
    });
    connection.initialize_finish(init_id, initialize_result)?;

    main_loop(connection, multiplexer)?;
    io_threads.join()?;
    Ok(())
}

/// Read the embedded Python engine's launch spec out of the editor's
/// `initializationOptions`. Two forms are accepted:
///
/// - `pythonServer: { command, args }` — a full launch spec, used by
///   the VS Code extension to point at the basedpyright it bundles
///   (`node <langserver.js> --stdio`). `args` defaults to `["--stdio"]`.
/// - `pythonServerPath: "<path>"` — just a path to a langserver binary,
///   the `pykrete.pythonServer.path` user override.
///
/// `None` (neither set, or set empty) means fall back to `PATH`
/// discovery.
fn explicit_python_server(init_params: &serde_json::Value) -> Option<(String, Vec<String>)> {
    let options = init_params.get("initializationOptions")?;

    if let Some(server) = options.get("pythonServer") {
        let command = server.get("command")?.as_str()?.trim();
        if command.is_empty() {
            return None;
        }
        let args = match server.get("args").and_then(|a| a.as_array()) {
            Some(raw) => raw
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            None => vec!["--stdio".to_string()],
        };
        return Some((command.to_string(), args));
    }

    let path = options.get("pythonServerPath")?.as_str()?.trim();
    (!path.is_empty()).then(|| (path.to_string(), vec!["--stdio".to_string()]))
}

fn main_loop(
    connection: Connection,
    mut multiplexer: multiplex::Multiplexer,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    // In-memory shadow of every open document. `didOpen` adds; `didChange`
    // overwrites (FULL sync); `didClose` removes.
    let mut docs: HashMap<Url, String> = HashMap::new();
    // Cross-file snapshot cache; survives every request in this session.
    // Open docs are read live; on-disk files are tiered by recency. The
    // `pykrete/refreshSnapshot` custom command and didOpen/didClose for
    // paths outside the tracked union drop it.
    let mut snapshot_cache = project::SnapshotCache::new();

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
        // If there's a fanned-out request in flight, cap the receive wait
        // at its deadline — if the child doesn't reply in time we reap it
        // and reply to the editor with pykrete's standalone result. With
        // nothing pending the receive can block indefinitely.
        let pending_deadline = multiplexer.next_pending_deadline();
        let select_result = match pending_deadline {
            Some(deadline) => crossbeam_channel::select! {
                recv(connection.receiver) -> msg => SelectOutcome::Editor(msg),
                recv(child_rx) -> msg => SelectOutcome::Child(msg),
                default(deadline.saturating_duration_since(Instant::now())) => SelectOutcome::Timeout,
            },
            None => crossbeam_channel::select! {
                recv(connection.receiver) -> msg => SelectOutcome::Editor(msg),
                recv(child_rx) -> msg => SelectOutcome::Child(msg),
            },
        };
        match select_result {
            SelectOutcome::Editor(msg) => {
                let Ok(msg) = msg else {
                    break; // editor disconnected
                };
                match msg {
                    Message::Request(req) => {
                        if connection.handle_shutdown(&req)? {
                            multiplexer.shutdown();
                            return Ok(());
                        }
                        handle_request(
                            &connection,
                            &docs,
                            &mut multiplexer,
                            &mut snapshot_cache,
                            req,
                        )?;
                    }
                    Message::Notification(notif) => {
                        handle_notification(
                            &connection,
                            &mut docs,
                            &mut multiplexer,
                            &mut snapshot_cache,
                            notif,
                        )?;
                    }
                    Message::Response(resp) => {
                        // A response from the editor to a request we
                        // proxied on the embedded engine's behalf —
                        // route it back to the engine.
                        handle_editor_response(&mut multiplexer, resp);
                    }
                }
            }
            SelectOutcome::Child(msg) => {
                let Ok(msg) = msg else {
                    // The embedded engine exited. Answer any in-flight
                    // fanned-out requests with pykrete's own result so
                    // they don't hang, drop the child, and swap the
                    // channel to `never()` so this branch stops firing.
                    for pending in multiplexer.drain_pending() {
                        connection.sender.send(Message::Response(Response {
                            id: pending.editor_id,
                            result: Some(pending.pykrete_result),
                            error: None,
                        }))?;
                    }
                    multiplexer.child = None;
                    child_rx = crossbeam_channel::never();
                    continue;
                };
                handle_child_message(&connection, &docs, &mut multiplexer, msg)?;
            }
            SelectOutcome::Timeout => {}
        }
        // Reap any fanned-out requests whose deadline expired — most
        // iterations this is a no-op (linear over a typically-empty map).
        // When the child stays silent past its deadline (basedpyright
        // sometimes never answers hover requests, see FANOUT_TIMEOUT) the
        // editor gets pykrete's standalone result instead of hanging.
        let now = Instant::now();
        for pending in multiplexer.reap_expired_pending(now) {
            // Re-use the merge path with `Null` as the child's result so
            // each method's response shape stays correct (hover stays a
            // hover, completion stays a completion list, …).
            let merged = if pending.method == "textDocument/semanticTokens/full" {
                multiplex::remap_semantic_tokens(pending.pykrete_result, pending.future_line)
            } else {
                multiplex::merge_child_response(
                    &pending.method,
                    pending.pykrete_result,
                    serde_json::Value::Null,
                )
            };
            connection.sender.send(Message::Response(Response {
                id: pending.editor_id,
                result: Some(merged),
                error: None,
            }))?;
        }
    }
    multiplexer.shutdown();
    Ok(())
}

/// Which arm of the main loop's `select!` fired.
enum SelectOutcome {
    Editor(Result<Message, crossbeam_channel::RecvError>),
    Child(Result<serde_json::Value, crossbeam_channel::RecvError>),
    Timeout,
}

/// Handle one message emitted by the embedded Python engine.
///
/// - `publishDiagnostics` notifications are remapped out of
///   virtual-document coordinates and merged with pykrete's diagnostics;
///   the engine's unresolved-import complaints about cross-`.pyk`
///   imports are dropped (pykrete resolves those itself).
/// - Responses to requests we fanned out (`pykrete-req-*`) are merged
///   with pykrete's own result and forwarded to the editor.
/// - Requests the engine sends us (`workspace/configuration`,
///   `client/registerCapability`, …) are proxied to the real editor;
///   the editor's reply is routed back from `handle_editor_response`.
/// - Notifications the engine emits (`window/logMessage`,
///   `window/showMessage`, `$/progress`, …) are relayed to the editor
///   verbatim.
fn handle_child_message(
    connection: &Connection,
    docs: &HashMap<Url, String>,
    multiplexer: &mut multiplex::Multiplexer,
    msg: serde_json::Value,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let method = msg.get("method").and_then(|m| m.as_str());
    let id = msg.get("id");

    match (method, id) {
        (Some("textDocument/publishDiagnostics"), _) => {
            if let Some(params) = msg.get("params")
                && let Some((uri, mut child_diags)) = multiplex::child_diagnostics_to_editor(params)
            {
                // Drop the engine's unresolved-import complaints about
                // cross-`.pyk` imports — pykrete resolves those itself.
                if let Some(source) = docs.get(&uri) {
                    child_diags.retain(|d| !multiplex::is_unresolved_pykrete_import(d, source));
                }
                let merged = multiplexer.diagnostics.set_child(uri.clone(), child_diags);
                send_diagnostics(connection, &uri, merged)?;
            }
        }
        (Some(method), Some(child_id)) => {
            // A request from the engine — proxy it to the real editor
            // so the engine sees the editor's actual answers (Python
            // interpreter / analysis config, progress tokens, …). The
            // editor's reply comes back via `handle_editor_response`.
            //
            // For `workspace/configuration` the requested sections are
            // kept so the reply can be patched (`patch_engine_config`).
            let config_sections: Vec<String> = if method == "workspace/configuration" {
                msg.pointer("/params/items")
                    .and_then(|items| items.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| {
                                item.get("section")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or_default()
                                    .to_string()
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let editor_id = multiplexer.register_child_request(child_id.clone(), config_sections);
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
            // parked editor request, merge pykrete's result with the
            // child's, and send the combined reply to the editor.
            if let Some(child_id) = id.as_str()
                && let Some(pending) = multiplexer.take_pending(child_id)
            {
                // An `error` response (or no `result`) becomes `Null` —
                // the merge then just yields pykrete's own answer.
                let child_result = msg
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                // Semantic tokens need the document's `__future__`-import
                // line to place the hoisted import's tokens; the rest of
                // the merge doesn't, so it's branched here.
                let merged = if pending.method == "textDocument/semanticTokens/full" {
                    multiplex::remap_semantic_tokens(child_result, pending.future_line)
                } else {
                    multiplex::merge_child_response(
                        &pending.method,
                        pending.pykrete_result,
                        child_result,
                    )
                };
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
/// pykrete-lsp forwarded on the engine's behalf — pykrete-lsp issues the
/// editor no requests of its own. A response whose id isn't in the
/// proxy table is ignored.
fn handle_editor_response(multiplexer: &mut multiplex::Multiplexer, resp: Response) {
    let Some(editor_id) = request_id_as_str(&resp.id) else {
        return;
    };
    let Some(proxied) = multiplexer.take_child_request(&editor_id) else {
        return;
    };
    let error = resp.error.and_then(|e| serde_json::to_value(e).ok());
    // A `workspace/configuration` reply is patched so the engine's
    // type-checking level matches `pykrete.typeCheckingMode`.
    let result = match resp.result {
        Some(result) if !proxied.config_sections.is_empty() => {
            Some(multiplex::patch_engine_config(
                result,
                &proxied.config_sections,
                multiplexer.type_checking_mode,
            ))
        }
        other => other,
    };
    multiplexer.respond_to_child(proxied.child_id, result, error);
}

/// The string form of a `RequestId`. pykrete-lsp issues proxied-request
/// ids as `pykrete-c2e-N` strings, so a non-string id is one we never
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
    snapshot_cache: &mut project::SnapshotCache,
    req: Request,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        // hover / definition / completion are fanned out: pykrete's own
        // answer is computed here, then merged with the embedded Python
        // engine's. When no engine is embedded the reply goes out
        // immediately; otherwise it's deferred until the child answers
        // (see `fanout_request` / `handle_child_message`).
        "textDocument/hover" => {
            let params: HoverParams = serde_json::from_value(req.params.clone())?;
            let pykrete_result = serde_json::to_value(handle_hover(docs, snapshot_cache, params))?;
            fanout_request(connection, multiplexer, docs, req, pykrete_result)?;
        }
        "textDocument/definition" => {
            let params: GotoDefinitionParams = serde_json::from_value(req.params.clone())?;
            let pykrete_result =
                serde_json::to_value(handle_definition(docs, snapshot_cache, params))?;
            fanout_request(connection, multiplexer, docs, req, pykrete_result)?;
        }
        "textDocument/completion" => {
            let params: CompletionParams = serde_json::from_value(req.params.clone())?;
            let pykrete_result =
                serde_json::to_value(handle_completion(docs, snapshot_cache, params))?;
            fanout_request(connection, multiplexer, docs, req, pykrete_result)?;
        }
        // Custom command — drop every tier of the snapshot cache. Useful
        // for users hitting "Restart LSP server" replacement workflows
        // without actually restarting; also the escape hatch for the 30s
        // cold-walk staleness window when no file watcher is wired.
        "pykrete/refreshSnapshot" => {
            snapshot_cache.invalidate();
            connection.sender.send(Message::Response(Response {
                id: req.id,
                result: Some(serde_json::Value::Null),
                error: None,
            }))?;
        }
        "textDocument/references" => {
            let params: ReferenceParams = serde_json::from_value(req.params.clone())?;
            let refs = handle_references(docs, params);
            // An empty list would merge as "pykrete found nothing"; send
            // `null` instead so `merge_locations` falls back cleanly to
            // the child engine's references.
            let pykrete_result = if refs.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::to_value(refs)?
            };
            fanout_request(connection, multiplexer, docs, req, pykrete_result)?;
        }
        "textDocument/rename" => {
            let params: RenameParams = serde_json::from_value(req.params.clone())?;
            let pykrete_result = match handle_rename(docs, params) {
                Some(edit) => serde_json::to_value(edit)?,
                None => serde_json::Value::Null,
            };
            fanout_request(connection, multiplexer, docs, req, pykrete_result)?;
        }
        "textDocument/prepareRename" => {
            let params: TextDocumentPositionParams = serde_json::from_value(req.params.clone())?;
            let pykrete_result = match handle_prepare_rename(docs, params) {
                Some(resp) => serde_json::to_value(resp)?,
                None => serde_json::Value::Null,
            };
            fanout_request(connection, multiplexer, docs, req, pykrete_result)?;
        }
        // These are pure passthrough — pykrete has no schema-specific
        // answer, so pykrete's result is `null` and the merge just
        // returns the (remapped) child result. They're only advertised
        // to the editor when the embedded engine provides them (see
        // `merge_capabilities`).
        "textDocument/signatureHelp"
        | "textDocument/documentHighlight"
        | "textDocument/semanticTokens/full" => {
            fanout_request(connection, multiplexer, docs, req, serde_json::Value::Null)?;
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
                    message: format!("pykrete-lsp doesn't yet handle '{}'", req.method),
                    data: None,
                }),
            }))?;
        }
    }
    Ok(())
}

/// Fan a request out to the embedded Python engine.
///
/// `pykrete_result` is pykrete's own answer, already computed. When an
/// engine is embedded the request is forwarded (its cursor position
/// shifted into virtual-document coordinates) and the editor reply is
/// deferred — `handle_child_message` merges the two answers and
/// replies. When there's no engine, pykrete's result is sent right away.
fn fanout_request(
    connection: &Connection,
    multiplexer: &mut multiplex::Multiplexer,
    docs: &HashMap<Url, String>,
    req: Request,
    pykrete_result: serde_json::Value,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    // The document's `from __future__` line, if any — semantic tokens
    // need it to place the hoisted import's tokens back (see
    // `remap_semantic_tokens`).
    let future_line = req
        .params
        .pointer("/textDocument/uri")
        .and_then(|uri| uri.as_str())
        .and_then(|uri| uri.parse::<Url>().ok())
        .and_then(|uri| docs.get(&uri))
        .and_then(|source| virtualdoc::first_future_import_line(source));
    let child_params = multiplex::request_params_to_child(req.params);
    if let Some(pykrete_only) = multiplexer.forward_request(
        req.id.clone(),
        &req.method,
        child_params,
        pykrete_result,
        future_line,
    ) {
        connection.sender.send(Message::Response(Response {
            id: req.id,
            result: Some(pykrete_only),
            error: None,
        }))?;
    }
    Ok(())
}

/// Handle a `textDocument/hover` request by routing through
/// `pykrete::hover`. Returns `None` if no symbol is at the cursor —
/// LSP's contract is that the response body in that case is `null`,
/// which our `serde_json::to_value(None::<Hover>)` produces.
pub fn handle_hover(
    docs: &HashMap<Url, String>,
    snapshot_cache: &mut project::SnapshotCache,
    params: HoverParams,
) -> Option<Hover> {
    let uri = &params.text_document_position_params.text_document.uri;
    let pos = params.text_document_position_params.position;
    let _ = docs.get(uri)?; // ensure the URI is open
    // LSP positions are 0-indexed; pykrete's hover entry point is 1-indexed.
    let line = (pos.line as usize).checked_add(1)?;
    let column = (pos.character as usize).checked_add(1)?;
    let info = match snapshot_cache.snapshot(docs) {
        Some(snapshot) => {
            let focus_path = uri.to_file_path().ok()?;
            let focus_path_str = focus_path.to_string_lossy().to_string();
            pykrete::hover_in_project(&snapshot, &focus_path_str, line, column)?
        }
        None => {
            // Single-file fallback for untitled / non-file URIs.
            let text = docs.get(uri)?;
            pykrete::hover(text, line, column)?
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
/// `pykrete::document_symbols`. Returns the document outline as a nested
/// list of `DocumentSymbol`s — VS Code renders this in the breadcrumb
/// bar and the file outline panel.
pub fn handle_document_symbol(
    docs: &HashMap<Url, String>,
    params: DocumentSymbolParams,
) -> Option<DocumentSymbolResponse> {
    let text = docs.get(&params.text_document.uri)?;
    let symbols = pykrete::document_symbols(text);
    let converted: Vec<DocumentSymbol> = symbols.iter().map(to_lsp_symbol).collect();
    Some(DocumentSymbolResponse::Nested(converted))
}

/// Handle a `textDocument/definition` request by routing through
/// `pykrete::definition`. Returns the source range of the declaration
/// that the cursor points at (single-file only in v0.1).
pub fn handle_definition(
    docs: &HashMap<Url, String>,
    snapshot_cache: &mut project::SnapshotCache,
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
    // `(target_uri, span)` — the definition may live in a different
    // file than the cursor (a column declared in an imported schema).
    let (target_uri, span) = match snapshot_cache.snapshot(docs) {
        Some(snapshot) => {
            let focus_path = uri.to_file_path().ok()?;
            let focus_path_str = focus_path.to_string_lossy().to_string();
            let (target_path, span) =
                pykrete::definition_in_project(&snapshot, &focus_path_str, line, column)?;
            (
                Url::from_file_path(&target_path).unwrap_or_else(|()| uri.clone()),
                span,
            )
        }
        None => {
            let text = docs.get(&uri)?;
            (uri.clone(), pykrete::definition(text, line, column)?)
        }
    };
    let location = Location {
        uri: target_uri,
        range: span_to_range(span),
    };
    Some(GotoDefinitionResponse::Scalar(location))
}

/// Handle a `textDocument/references` request by routing through
/// `pykrete::references` — every use of the column under the cursor.
/// Single-file in v0.1; all results carry the request's document URI.
pub fn handle_references(docs: &HashMap<Url, String>, params: ReferenceParams) -> Vec<Location> {
    let uri = params.text_document_position.text_document.uri.clone();
    let pos = params.text_document_position.position;
    let Some(text) = docs.get(&uri) else {
        return Vec::new();
    };
    let (Some(line), Some(column)) = (
        (pos.line as usize).checked_add(1),
        (pos.character as usize).checked_add(1),
    ) else {
        return Vec::new();
    };
    pykrete::references(text, line, column)
        .into_iter()
        .map(|span| Location {
            uri: uri.clone(),
            range: span_to_range(span),
        })
        .collect()
}

/// Handle a `textDocument/rename` request via `pykrete::rename` — rename
/// the column under the cursor across the file. `None` when the cursor
/// isn't on a column (the embedded engine then gets a turn).
pub fn handle_rename(docs: &HashMap<Url, String>, params: RenameParams) -> Option<WorkspaceEdit> {
    let uri = params.text_document_position.text_document.uri.clone();
    let pos = params.text_document_position.position;
    let text = docs.get(&uri)?;
    let line = (pos.line as usize).checked_add(1)?;
    let column = (pos.character as usize).checked_add(1)?;
    let spans = pykrete::rename(text, line, column);
    if spans.is_empty() {
        return None;
    }
    let edits: Vec<TextEdit> = spans
        .into_iter()
        .map(|span| TextEdit {
            range: span_to_range(span),
            new_text: params.new_name.clone(),
        })
        .collect();
    let mut changes = HashMap::new();
    changes.insert(uri, edits);
    Some(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    })
}

/// Handle a `textDocument/prepareRename` request via
/// `pykrete::prepare_rename` — confirm the cursor is on a column and
/// return that token's range. `None` when it isn't.
pub fn handle_prepare_rename(
    docs: &HashMap<Url, String>,
    params: TextDocumentPositionParams,
) -> Option<PrepareRenameResponse> {
    let text = docs.get(&params.text_document.uri)?;
    let line = (params.position.line as usize).checked_add(1)?;
    let column = (params.position.character as usize).checked_add(1)?;
    let span = pykrete::prepare_rename(text, line, column)?;
    Some(PrepareRenameResponse::Range(span_to_range(span)))
}

/// Handle a `textDocument/completion` request by routing through
/// `pykrete::completions`. Three completion surfaces are recognized: the
/// `X` in `DataFrame[X]`, the literal inside `col("…")`, and the attr
/// after `df.`. Returns an empty list (encoded as `CompletionList`) when
/// no completions apply at the cursor.
pub fn handle_completion(
    docs: &HashMap<Url, String>,
    snapshot_cache: &mut project::SnapshotCache,
    params: CompletionParams,
) -> Option<CompletionResponse> {
    let uri = &params.text_document_position.text_document.uri;
    let pos = params.text_document_position.position;
    let _ = docs.get(uri)?;
    let line = (pos.line as usize).checked_add(1)?;
    let column = (pos.character as usize).checked_add(1)?;
    let raw_items = match snapshot_cache.snapshot(docs) {
        Some(snapshot) => {
            let focus_path = uri.to_file_path().ok()?;
            let focus_path_str = focus_path.to_string_lossy().to_string();
            pykrete::completions_in_project(&snapshot, &focus_path_str, line, column)
        }
        None => {
            let text = docs.get(uri)?;
            pykrete::completions(text, line, column)
        }
    };
    let items: Vec<CompletionItem> = raw_items.into_iter().map(to_lsp_completion_item).collect();
    Some(CompletionResponse::Array(items))
}

fn to_lsp_completion_item(item: pykrete::CompletionItem) -> CompletionItem {
    CompletionItem {
        label: item.label,
        detail: item.detail,
        kind: Some(match item.kind {
            pykrete::CompletionItemKind::Class => CompletionItemKind::CLASS,
            pykrete::CompletionItemKind::Field => CompletionItemKind::FIELD,
        }),
        ..Default::default()
    }
}

#[allow(deprecated)]
fn to_lsp_symbol(s: &pykrete::symbols::DocumentSymbol) -> DocumentSymbol {
    DocumentSymbol {
        name: s.name.clone(),
        detail: s.detail.clone(),
        kind: match s.kind {
            pykrete::SymbolKind::Class => SymbolKind::CLASS,
            pykrete::SymbolKind::Field => SymbolKind::FIELD,
            pykrete::SymbolKind::Function => SymbolKind::FUNCTION,
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

fn span_to_range(span: pykrete::Span) -> Range {
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
    snapshot_cache: &mut project::SnapshotCache,
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
            // didOpen for a `.pyk` outside whatever the cache was last
            // tracking means the project key (anchor or root) may have
            // shifted. Drop the cache so the next snapshot rewalks.
            if uri_is_outside_cache(snapshot_cache, &uri) {
                snapshot_cache.invalidate();
            }
            docs.insert(uri, text);
            publish_project_diagnostics(connection, docs, multiplexer, snapshot_cache)?;
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
                publish_project_diagnostics(connection, docs, multiplexer, snapshot_cache)?;
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
            publish_project_diagnostics(connection, docs, multiplexer, snapshot_cache)?;
        }
        "workspace/didChangeWatchedFiles" => {
            // The client is telling us a watched file changed outside the
            // editor. Drop the cache and let the next snapshot rewalk.
            snapshot_cache.invalidate();
        }
        // didSave, willSave, etc. are ignored — diagnostics already update
        // on every didChange. `initialized` arrives once and is a no-op.
        _ => {}
    }
    Ok(())
}

/// `true` when `uri` resolves to a filesystem path that the snapshot
/// cache hasn't seen yet — opening such a file may shift the project
/// key (a different `pyproject.toml` ancestor), so the safe move is to
/// drop the cache.
fn uri_is_outside_cache(cache: &project::SnapshotCache, uri: &Url) -> bool {
    let Ok(path) = uri.to_file_path() else {
        return false;
    };
    !cache.tracks_path(&path)
}

/// Run the analyzer over the entire project — every `.pyk` file under
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
    snapshot_cache: &mut project::SnapshotCache,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let Some(snapshot) = snapshot_cache.snapshot(docs) else {
        // Fall back: no usable project root. Check each open doc on
        // its own so we still publish *something* — the user will
        // see the same single-file behaviour we had before iteration 33.
        for (uri, text) in docs {
            publish_single_file_diagnostics(connection, multiplexer, uri, text)?;
        }
        return Ok(());
    };

    // Project config — `pykrete.json` at or above the project root. Its
    // `typeCheckingMode` overrides the editor's setting; `rules`
    // re-levels or drops codes; `exclude` silences whole files.
    let config = snapshot_cache.config(docs);
    let mode = config
        .check_mode_override()
        .unwrap_or(multiplexer.type_checking_mode);
    let mut result = pykrete::check_project_with_mode(&snapshot, mode);
    for file in &mut result.files {
        config.apply_rules(&mut file.result.diagnostics);
    }
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
        // A `pykrete.json` `exclude`d file: pykrete stays silent on it
        // (an empty list still clears any stale underlines).
        let diagnostics: Vec<Diagnostic> = if config.is_excluded(&file.path) {
            Vec::new()
        } else {
            file.result
                .diagnostics
                .iter()
                .map(to_lsp_diagnostic)
                .collect()
        };
        // Route pykrete's diagnostics through the merge store so they're
        // published alongside whatever the embedded Python engine last
        // reported for the same file.
        let merged = multiplexer
            .diagnostics
            .set_pykrete(uri.clone(), diagnostics);
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
    let result = pykrete::check_with_mode(&path, text, multiplexer.type_checking_mode);
    let diagnostics: Vec<Diagnostic> = result.diagnostics.iter().map(to_lsp_diagnostic).collect();
    let merged = multiplexer
        .diagnostics
        .set_pykrete(uri.clone(), diagnostics);
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
/// passing into pykrete's checker (used purely as the path embedded in
/// diagnostic messages). Non-file URIs fall back to the URI's stringified
/// form.
fn uri_to_path(uri: &Url) -> String {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| uri.to_string())
}

/// Translate pykrete's diagnostic format into LSP's. pykrete's positions are
/// 1-indexed; LSP is 0-indexed. The diagnostic carries a start and end
/// position so editors can underline the entire offending token.
pub fn to_lsp_diagnostic(d: &pykrete::diagnostics::Diagnostic) -> Diagnostic {
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
            pykrete::diagnostics::Severity::Error => DiagnosticSeverity::ERROR,
            pykrete::diagnostics::Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(
            pykrete::diagnostics::rule_name(d.code).to_string(),
        )),
        code_description: None,
        source: Some("pykrete".to_string()),
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
        // Only the unknown-column rule (D0030) carries a suggestion today.
        let Some(NumberOrString::String(code)) = &diag.code else {
            continue;
        };
        if code != pykrete::diagnostics::rule_name("D0030") {
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
#[allow(non_snake_case)] // Test names embed PySpark/pykrete type names (DataFrame, …).
mod tests {
    use super::*;
    use pykrete::diagnostics::{Diagnostic as PykreteDiagnostic, Severity};

    /// Helper: build a pykrete Diagnostic at a known line/column. We use
    /// the struct's public fields directly rather than the `Diagnostic::at`
    /// constructor (which would require pulling in `ruff_source_file` just
    /// for the LineIndex). The conversion code under test only reads
    /// these fields anyway.
    fn pykrete_diag(
        start: (usize, usize),
        end: (usize, usize),
        code: &'static str,
        msg: &str,
    ) -> PykreteDiagnostic {
        PykreteDiagnostic {
            severity: Severity::Error,
            code,
            message: msg.to_string(),
            line: start.0,
            column: start.1,
            end_line: end.0,
            end_column: end.1,
            suggestion: None,
            min_mode: pykrete::CheckMode::Basic,
        }
    }

    #[test]
    fn pykrete_line_column_become_zero_indexed_lsp_position() {
        // pykrete: line=3, column=5 (1-indexed) → LSP: line=2, character=4
        let d = pykrete_diag((3, 5), (3, 10), "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.range.start.line, 2);
        assert_eq!(lsp.range.start.character, 4);
    }

    #[test]
    fn lsp_diagnostic_range_spans_to_end_position() {
        // A diagnostic that covers `"BadColumn"` (10 chars including quotes)
        // should produce a range whose end is offset by the token length.
        let d = pykrete_diag((2, 5), (2, 15), "D0030", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.range.start.line, 1);
        assert_eq!(lsp.range.start.character, 4);
        assert_eq!(lsp.range.end.line, 1);
        assert_eq!(lsp.range.end.character, 14);
    }

    #[test]
    fn pykrete_error_severity_maps_to_lsp_error() {
        let d = pykrete_diag((1, 1), (1, 1), "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn pykrete_code_becomes_lsp_string_code() {
        // The editor shows the readable rule name (`unknownColumn`) — the
        // same identifier `pykrete.json`'s `rules` block and the CLI use.
        let d = pykrete_diag(
            (1, 1),
            (1, 1),
            "D0030",
            "Column 'X' does not exist on schema 'Orders'.",
        );
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(
            lsp.code,
            Some(NumberOrString::String("unknownColumn".to_string())),
        );
    }

    #[test]
    fn lsp_diagnostic_source_is_pykrete() {
        let d = pykrete_diag((1, 1), (1, 1), "D0001", "test");
        let lsp = to_lsp_diagnostic(&d);
        assert_eq!(lsp.source.as_deref(), Some("pykrete"));
    }

    #[test]
    fn pykrete_message_is_preserved_verbatim() {
        let msg = "Column 'priec' does not exist on schema 'Orders'.";
        let d = pykrete_diag((1, 1), (1, 1), "D0030", msg);
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
        let uri = Url::parse("file:///nonexistent.pyk").unwrap();
        let result = handle_hover(
            &docs,
            &mut project::SnapshotCache::new(),
            hover_params_at(&uri, 0, 0),
        );
        assert!(result.is_none());
    }

    #[test]
    fn handle_hover_returns_markdown_hover_for_a_schema_class_name() {
        // didOpen sets the doc text; hover on the Schema class name
        // returns a Hover with markdown content rendering the schema as
        // a Python class block. Field names of varying length so the
        // colon-alignment is visible.
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.pyk").unwrap();
        let src = "class Orders(Schema):\n    place_code: int\n    price: int\n";
        docs.insert(uri.clone(), src.to_string());

        // LSP position is 0-indexed: line 0, character 6 lands inside
        // "Orders" on the first line.
        let result = handle_hover(
            &docs,
            &mut project::SnapshotCache::new(),
            hover_params_at(&uri, 0, 6),
        )
        .expect("hover");
        match result.contents {
            HoverContents::Markup(m) => {
                let md = &m.value;
                assert!(
                    md.contains("**schema** `Orders`"),
                    "unexpected heading, got {md:?}",
                );
                assert!(
                    md.contains("```python"),
                    "expected fenced python block, got {md:?}"
                );
                assert!(
                    md.contains("class Orders(Schema):"),
                    "missing class line, got {md:?}"
                );
                // place_code is the longer field name; price gets padded
                // so its type column lines up with Int.
                assert!(
                    md.contains("    place_code: Int"),
                    "place_code row wrong, got {md:?}"
                );
                assert!(
                    md.contains("    price:      Int"),
                    "price row wrong (alignment?), got {md:?}"
                );
            }
            _ => panic!("expected MarkupContent"),
        }
    }

    #[test]
    fn handle_hover_translates_lsp_0_indexed_to_pykrete_1_indexed() {
        // The fixture starts with a blank line so the schema is on
        // line 1 (0-indexed). If position translation is wrong, hover
        // would miss the symbol and return None.
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.pyk").unwrap();
        let src = "\nclass Orders(Schema):\n    x: int\n";
        docs.insert(uri.clone(), src.to_string());

        // Line 1 (0-indexed), character 6 → "Orders".
        let result = handle_hover(
            &docs,
            &mut project::SnapshotCache::new(),
            hover_params_at(&uri, 1, 6),
        );
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
        let uri = Url::parse("file:///nonexistent.pyk").unwrap();
        let result = handle_document_symbol(&docs, doc_sym_params(&uri));
        assert!(result.is_none());
    }

    #[test]
    fn handle_document_symbol_returns_nested_outline_with_schema_class_and_function() {
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.pyk").unwrap();
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
        let uri = Url::parse("file:///nonexistent.pyk").unwrap();
        let result = handle_definition(
            &docs,
            &mut project::SnapshotCache::new(),
            def_params_at(&uri, 0, 0),
        );
        assert!(result.is_none());
    }

    #[test]
    fn handle_definition_resolves_DataFrame_inner_schema_to_class_decl() {
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.pyk").unwrap();
        // Line 0: class header
        // Line 1: field
        // Line 2: blank
        // Line 3: def header — Orders appears at column 21 (after "def f(raw: DataFrame[")
        let src = "class Orders(Schema):\n    x: int\n\ndef f(raw: DataFrame[Orders]) -> DataFrame[Orders]:\n    return raw\n";
        docs.insert(uri.clone(), src.to_string());

        // Click on `Orders` inside `DataFrame[Orders]` on line 3.
        let result = handle_definition(
            &docs,
            &mut project::SnapshotCache::new(),
            def_params_at(&uri, 3, 22),
        )
        .expect("response");
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
        let uri = Url::parse("file:///nonexistent.pyk").unwrap();
        let result = handle_completion(
            &docs,
            &mut project::SnapshotCache::new(),
            completion_params_at(&uri, 0, 0),
        );
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // diagnostic ↔ suggestion round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn diagnostic_suggestion_is_serialized_into_lsp_data_field() {
        let mut d = pykrete_diag(
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
            code: Some(NumberOrString::String("unknownColumn".to_string())),
            code_description: None,
            source: Some("pykrete".to_string()),
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
        let uri = Url::parse("file:///t.pyk").unwrap();
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
        let uri = Url::parse("file:///t.pyk").unwrap();
        let mut diag = lsp_diag_with_suggestion((1, 18), (1, 25), "price");
        diag.data = None;
        let params = code_action_params(&uri, vec![diag]);
        let result = handle_code_action(params);
        assert!(result.is_empty());
    }

    #[test]
    fn code_action_skips_non_d0030_diagnostics() {
        let uri = Url::parse("file:///t.pyk").unwrap();
        let mut diag = lsp_diag_with_suggestion((1, 18), (1, 25), "price");
        diag.code = Some(NumberOrString::String("unknownSchema".to_string()));
        let params = code_action_params(&uri, vec![diag]);
        let result = handle_code_action(params);
        assert!(result.is_empty());
    }

    #[test]
    fn handle_completion_returns_schema_names_inside_dataframe_subscript() {
        let mut docs: HashMap<Url, String> = HashMap::new();
        let uri = Url::parse("file:///t.pyk").unwrap();
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
        let result = handle_completion(
            &docs,
            &mut project::SnapshotCache::new(),
            completion_params_at(&uri, 6, 22),
        )
        .expect("response");
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

    // -----------------------------------------------------------------------
    // embedded-engine launch spec
    // -----------------------------------------------------------------------

    #[test]
    fn explicit_python_server_reads_a_full_launch_spec() {
        // The VS Code extension points pykrete-lsp at the basedpyright
        // it bundles via a `node <langserver.js> --stdio` spec.
        let init = serde_json::json!({
            "initializationOptions": {
                "pythonServer": {
                    "command": "node",
                    "args": ["/ext/basedpyright/langserver.index.js", "--stdio"],
                },
            },
        });
        assert_eq!(
            explicit_python_server(&init),
            Some((
                "node".to_string(),
                vec![
                    "/ext/basedpyright/langserver.index.js".to_string(),
                    "--stdio".to_string(),
                ],
            )),
        );
    }

    #[test]
    fn explicit_python_server_defaults_args_to_stdio() {
        let init = serde_json::json!({
            "initializationOptions": { "pythonServer": { "command": "pyright-langserver" } },
        });
        assert_eq!(
            explicit_python_server(&init),
            Some((
                "pyright-langserver".to_string(),
                vec!["--stdio".to_string()]
            )),
        );
    }

    #[test]
    fn explicit_python_server_reads_the_path_override_form() {
        // `pykrete.pythonServer.path` — a plain path to a langserver.
        let init = serde_json::json!({
            "initializationOptions": { "pythonServerPath": "/usr/local/bin/basedpyright-langserver" },
        });
        assert_eq!(
            explicit_python_server(&init),
            Some((
                "/usr/local/bin/basedpyright-langserver".to_string(),
                vec!["--stdio".to_string()],
            )),
        );
    }

    #[test]
    fn explicit_python_server_is_none_when_unset_or_empty() {
        // No options at all.
        assert_eq!(explicit_python_server(&serde_json::json!({})), None);
        // Present but empty — treated as "not set", falls back to PATH.
        let empty_path = serde_json::json!({
            "initializationOptions": { "pythonServerPath": "  " },
        });
        assert_eq!(explicit_python_server(&empty_path), None);
        let empty_command = serde_json::json!({
            "initializationOptions": { "pythonServer": { "command": "" } },
        });
        assert_eq!(explicit_python_server(&empty_command), None);
    }
}
