//! The LSP multiplexer.
//!
//! dathon-lsp embeds a Python language server (basedpyright) as a child
//! process and presents a single merged LSP to the editor. This module
//! owns:
//!
//! - [`DiagnosticStore`] — per-URI merge of dathon's schema diagnostics
//!   with the child engine's Python diagnostics. `publishDiagnostics`
//!   replaces the whole set for a URI, so whenever either source
//!   changes we re-emit the union.
//! - [`Multiplexer`] — the optional child process plus the helpers that
//!   forward text-sync notifications to it (transformed into virtual
//!   documents) and translate the child's messages back.
//! - Request fan-out — `hover` / `completion` / `definition` requests
//!   are answered by both engines. The pending-request table correlates
//!   each forwarded request with the child's eventual reply, and
//!   [`merge_child_response`] combines the two answers.

use std::collections::HashMap;

use lsp_server::RequestId;
use lsp_types::{Diagnostic, NumberOrString, Url};
use serde_json::{Value, json};

use crate::child::ChildLsp;
use crate::virtualdoc;

/// Per-URI merge of the two diagnostic sources. dathon's set and the
/// child's set are stored separately; [`merged`] returns their union,
/// which is what actually gets published to the editor.
#[derive(Default)]
pub struct DiagnosticStore {
    entries: HashMap<Url, Sources>,
}

#[derive(Default)]
struct Sources {
    dathon: Vec<Diagnostic>,
    child: Vec<Diagnostic>,
}

impl DiagnosticStore {
    /// Replace dathon's diagnostics for `uri`. Returns the merged set
    /// the caller should publish.
    pub fn set_dathon(&mut self, uri: Url, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
        let entry = self.entries.entry(uri).or_default();
        entry.dathon = diagnostics;
        Self::merge(entry)
    }

    /// Replace the child engine's diagnostics for `uri`. Returns the
    /// merged set the caller should publish.
    pub fn set_child(&mut self, uri: Url, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
        let entry = self.entries.entry(uri).or_default();
        entry.child = diagnostics;
        Self::merge(entry)
    }

    /// Drop all diagnostics for `uri` — used on `didClose`.
    pub fn clear(&mut self, uri: &Url) {
        self.entries.remove(uri);
    }

    fn merge(entry: &Sources) -> Vec<Diagnostic> {
        let mut out = Vec::with_capacity(entry.dathon.len() + entry.child.len());
        out.extend(entry.dathon.iter().cloned());
        out.extend(entry.child.iter().cloned());
        out
    }
}

/// Translate a `publishDiagnostics` notification the child emitted (in
/// virtual-document coordinates) into editor coordinates.
///
/// Returns `(uri, diagnostics)`. Diagnostics whose range lies entirely
/// inside the injected preamble are dropped — they have no real-file
/// counterpart (e.g. an unused-import warning the child raises against
/// the preamble itself). Diagnostics that straddle the preamble
/// boundary are clamped to the first real line.
pub fn child_diagnostics_to_editor(params: &Value) -> Option<(Url, Vec<Diagnostic>)> {
    let uri: Url = params.get("uri")?.as_str()?.parse().ok()?;
    let raw = params.get("diagnostics")?.as_array()?;
    let mut out = Vec::with_capacity(raw.len());
    for d in raw {
        let Ok(mut diag) = serde_json::from_value::<Diagnostic>(d.clone()) else {
            continue;
        };
        // The diagnostic's end line is the deepest point it touches; if
        // even that is inside the preamble the whole diagnostic is
        // preamble-only and gets dropped.
        if virtualdoc::to_editor_line(diag.range.end.line).is_none() {
            continue;
        }
        diag.range.start.line = virtualdoc::to_editor_line(diag.range.start.line).unwrap_or(0);
        diag.range.end.line = virtualdoc::to_editor_line(diag.range.end.line).unwrap_or(0);
        // Tag the source so a user can tell a Python-engine diagnostic
        // apart from a dathon one in the editor UI.
        if diag.source.is_none() {
            diag.source = Some("python".to_string());
        }
        out.push(diag);
    }
    Some((uri, out))
}

/// Whether `diagnostic` is the embedded engine reporting an import it
/// couldn't resolve, where the import targets another dathon module —
/// i.e. a relative import (`from .schemas import …`). The engine
/// resolves imports as `.py`, so it never finds the sibling `.dpy`;
/// dathon resolves those itself, so the engine's complaint there is
/// noise and gets dropped. Absolute imports (`pyspark`, `os`) keep
/// their unresolved-import diagnostic — those are real.
///
/// `source` is the document text in editor coordinates; the
/// diagnostic's start line indexes into it to read the import
/// statement.
pub fn is_unresolved_dathon_import(diagnostic: &Diagnostic, source: &str) -> bool {
    let is_missing_import = matches!(
        diagnostic.code.as_ref(),
        Some(NumberOrString::String(code))
            if code == "reportMissingImports" || code == "reportMissingModuleSource"
    );
    if !is_missing_import {
        return false;
    }
    source
        .lines()
        .nth(diagnostic.range.start.line as usize)
        .is_some_and(|line| line.trim_start().starts_with("from ."))
}

/// An editor request that was fanned out to the child and is still
/// awaiting the child's reply. dathon's own answer is computed up front
/// and parked here; when the child responds the two are merged.
pub struct PendingRequest {
    /// The id the editor used — the merged response must carry it back.
    pub editor_id: RequestId,
    /// The LSP method, so the merge knows how to combine the two results.
    pub method: String,
    /// dathon's own result for this request, already computed. `Null`
    /// when dathon had nothing at the cursor.
    pub dathon_result: Value,
    /// The editor line of the document's `from __future__` import, if it
    /// has one. The import is hoisted into the virtual document's
    /// preamble region, so its semantic tokens come back on virtual
    /// line 0; this maps them back. `None` for documents without one.
    pub future_line: Option<u32>,
}

/// A request the child sent us that was proxied to the editor, awaiting
/// the editor's reply.
pub struct ProxiedRequest {
    /// The child's original request id — the reply must carry it back.
    pub child_id: Value,
    /// For a `workspace/configuration` request, the `section` of each
    /// requested item, in order; empty for other methods. Used to patch
    /// the per-slot response (see [`patch_engine_config`]).
    pub config_sections: Vec<String>,
}

/// The embedded Python language server plus the merged-diagnostic state.
///
/// `child` is `None` when no Python engine was found — dathon-lsp then
/// runs dathon-only, exactly the pre-multiplexer behavior.
pub struct Multiplexer {
    pub child: Option<ChildLsp>,
    pub diagnostics: DiagnosticStore,
    /// The `dathon.typeCheckingMode` setting — drives both dathon's own
    /// checker and the embedded engine's `typeCheckingMode`.
    pub type_checking_mode: dathon::CheckMode,
    /// The child engine's advertised `ServerCapabilities`, captured from
    /// its `initialize` response. `Null` when no engine is embedded.
    child_capabilities: Value,
    /// Requests forwarded to the child, keyed by the synthetic id we
    /// gave the child (`dathon-req-N`). Drained as the child answers.
    pending: HashMap<String, PendingRequest>,
    /// Monotonic counter for synthetic child request ids.
    next_request_seq: u64,
    /// Requests the child sent us and we proxied to the editor, keyed
    /// by the editor-facing id (`dathon-c2e-N`). Drained as the editor
    /// answers.
    child_requests: HashMap<String, ProxiedRequest>,
    /// Monotonic counter for editor-facing proxied-request ids.
    next_child_request_seq: u64,
}

impl Multiplexer {
    /// Spawn and initialize the embedded Python engine, handing it the
    /// same `InitializeParams` the editor sent dathon-lsp. `explicit`
    /// is the launch spec the editor supplied (the bundled engine, or a
    /// user override), or `None` to fall back to `PATH` discovery.
    /// `mode` is the `dathon.typeCheckingMode` setting.
    ///
    /// Never fails — if no engine is found or it doesn't handshake,
    /// the returned `Multiplexer` simply has `child: None`.
    pub fn start(
        explicit: Option<(String, Vec<String>)>,
        mode: dathon::CheckMode,
        init_params: &Value,
    ) -> Multiplexer {
        let mut child_capabilities = Value::Null;
        let child = crate::child::discover(explicit)
            .and_then(|(program, args)| ChildLsp::spawn(&program, &args))
            .and_then(|mut child| {
                Self::handshake(&mut child, init_params).map(|capabilities| {
                    child_capabilities = capabilities;
                    child
                })
            });
        Multiplexer {
            child,
            diagnostics: DiagnosticStore::default(),
            type_checking_mode: mode,
            child_capabilities,
            pending: HashMap::new(),
            next_request_seq: 0,
            child_requests: HashMap::new(),
            next_child_request_seq: 0,
        }
    }

    /// Run the LSP `initialize` / `initialized` handshake with the
    /// child. Returns the child's advertised `ServerCapabilities` on
    /// success, or `None` if the child failed to handshake.
    fn handshake(child: &mut ChildLsp, init_params: &Value) -> Option<Value> {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": "dathon-child-init",
            "method": "initialize",
            "params": child_init_params(init_params),
        });
        if child.send(&initialize).is_err() {
            return None;
        }
        // Wait for the child's initialize response. The child shouldn't
        // emit anything else before it, but tolerate interleaved
        // notifications by looping until we see our id back.
        let capabilities = loop {
            match child.receiver.recv() {
                Ok(msg) => {
                    if msg.get("id").and_then(|v| v.as_str()) == Some("dathon-child-init") {
                        break msg
                            .pointer("/result/capabilities")
                            .cloned()
                            .unwrap_or(Value::Null);
                    }
                }
                Err(_) => return None, // child died during handshake
            }
        };
        let initialized = json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {},
        });
        child.send(&initialized).ok()?;
        Some(capabilities)
    }

    /// Whether a Python engine is embedded. `false` means dathon-only.
    pub fn has_child(&self) -> bool {
        self.child.is_some()
    }

    /// The embedded engine's advertised `ServerCapabilities` — `Null`
    /// when no engine is embedded.
    pub fn child_capabilities(&self) -> &Value {
        &self.child_capabilities
    }

    /// Forward a `textDocument/didOpen` to the child, rewriting the
    /// document text into its virtual form (dathon preamble + source).
    pub fn forward_did_open(&mut self, uri: &Url, language_id: &str, version: i64, text: &str) {
        self.send_to_child(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri.as_str(),
                    "languageId": language_id,
                    "version": version,
                    "text": virtualdoc::to_virtual(text),
                }
            }
        }));
    }

    /// Forward a `textDocument/didChange` (FULL sync) to the child with
    /// the new text rewritten into virtual form.
    pub fn forward_did_change(&mut self, uri: &Url, version: i64, text: &str) {
        self.send_to_child(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri.as_str(), "version": version },
                "contentChanges": [ { "text": virtualdoc::to_virtual(text) } ],
            }
        }));
    }

    /// Forward a `textDocument/didClose` to the child.
    pub fn forward_did_close(&mut self, uri: &Url) {
        self.send_to_child(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri.as_str() } }
        }));
    }

    /// Register a request the child sent us, to be proxied to the
    /// editor. `config_sections` is the requested `section` list for a
    /// `workspace/configuration` request (empty otherwise) — kept so
    /// the response can be patched per slot. Returns the editor-facing
    /// request id to forward it under.
    pub fn register_child_request(
        &mut self,
        child_id: Value,
        config_sections: Vec<String>,
    ) -> String {
        self.next_child_request_seq += 1;
        let editor_id = format!("dathon-c2e-{}", self.next_child_request_seq);
        self.child_requests.insert(
            editor_id.clone(),
            ProxiedRequest {
                child_id,
                config_sections,
            },
        );
        editor_id
    }

    /// Look up (and remove) the proxied request an editor response
    /// belongs to. `None` for an id dathon-lsp never issued.
    pub fn take_child_request(&mut self, editor_id: &str) -> Option<ProxiedRequest> {
        self.child_requests.remove(editor_id)
    }

    /// Send a JSON-RPC response back to the child for a request it made
    /// of us — carrying either the editor's `result` or its `error`.
    pub fn respond_to_child(
        &mut self,
        child_id: Value,
        result: Option<Value>,
        error: Option<Value>,
    ) {
        let mut response = json!({ "jsonrpc": "2.0", "id": child_id });
        match error {
            Some(error) => response["error"] = error,
            None => response["result"] = result.unwrap_or(Value::Null),
        }
        self.send_to_child(&response);
    }

    /// Fan an editor request out to the child.
    ///
    /// `dathon_result` is dathon's own answer, computed by the caller
    /// up front. Returns `Some(dathon_result)` when there's no child to
    /// fan out to (or the forward failed) — the caller should reply with
    /// it immediately. Returns `None` when the request was forwarded;
    /// the caller defers, and the merged reply goes out from
    /// [`take_pending`] once the child answers.
    pub fn forward_request(
        &mut self,
        editor_id: RequestId,
        method: &str,
        params: Value,
        dathon_result: Value,
        future_line: Option<u32>,
    ) -> Option<Value> {
        if self.child.is_none() {
            return Some(dathon_result);
        }
        self.next_request_seq += 1;
        let child_id = format!("dathon-req-{}", self.next_request_seq);
        self.send_to_child(&json!({
            "jsonrpc": "2.0",
            "id": child_id,
            "method": method,
            "params": params,
        }));
        if self.child.is_none() {
            // The send failed and dropped the child — reply dathon-only.
            return Some(dathon_result);
        }
        self.pending.insert(
            child_id,
            PendingRequest {
                editor_id,
                method: method.to_string(),
                dathon_result,
                future_line,
            },
        );
        None
    }

    /// Look up and remove the pending request the child's response with
    /// id `child_id` belongs to. `None` for an unrecognized id (e.g. the
    /// child's reply to the `initialize` handshake).
    pub fn take_pending(&mut self, child_id: &str) -> Option<PendingRequest> {
        self.pending.remove(child_id)
    }

    /// Drain every still-pending request — called when the child exits
    /// so the caller can answer in-flight editor requests dathon-only
    /// instead of leaving them to hang.
    pub fn drain_pending(&mut self) -> Vec<PendingRequest> {
        self.pending.drain().map(|(_, v)| v).collect()
    }

    /// Shut the child down — sent when dathon-lsp itself exits.
    pub fn shutdown(&mut self) {
        if let Some(child) = self.child.as_mut() {
            child.shutdown();
        }
    }

    fn send_to_child(&mut self, msg: &Value) {
        if let Some(child) = self.child.as_mut() {
            // A send failure means the child died; drop it and fall
            // back to dathon-only for the rest of the session.
            if child.send(msg).is_err() {
                self.child = None;
            }
        }
    }
}

/// The `InitializeParams` to hand the child, with the editor's
/// pull-diagnostics client capabilities stripped out.
///
/// dathon-lsp implements **push** diagnostics (`publishDiagnostics`) —
/// it merges and remaps the child's pushed set. If the child sees the
/// editor's pull capability it instead registers a `textDocument/
/// diagnostic` provider; that registration would be proxied to the
/// editor, which would then pull from dathon-lsp — a method dathon-lsp
/// doesn't serve. Removing the capability keeps the child on the push
/// path dathon-lsp actually handles.
fn child_init_params(init_params: &Value) -> Value {
    let mut params = init_params.clone();
    if let Some(text_document) = params
        .pointer_mut("/capabilities/textDocument")
        .and_then(Value::as_object_mut)
    {
        text_document.remove("diagnostic");
    }
    if let Some(workspace) = params
        .pointer_mut("/capabilities/workspace")
        .and_then(Value::as_object_mut)
    {
        workspace.remove("diagnostics");
    }
    params
}

/// Build the capability set dathon-lsp advertises to the editor: its
/// own, plus the embedded engine's for the methods dathon-lsp knows how
/// to proxy. A child capability is taken only when dathon doesn't
/// already provide it (dathon's own handler wins) and the method is on
/// the proxy allowlist.
///
/// Every entry needs the child's result for that method to survive the
/// virtual↔editor coordinate transform — see [`merge_child_response`].
/// `semanticTokensProvider` is special-cased: its `legend` is forwarded
/// (the editor needs it to decode token types) but only `full` requests
/// are advertised, since `range` / `full/delta` aren't remapped.
pub fn merge_capabilities(mut dathon: Value, child: &Value) -> Value {
    const PROXIED: [&str; 4] = [
        "signatureHelpProvider",
        "referencesProvider",
        "documentHighlightProvider",
        "renameProvider",
    ];
    for key in PROXIED {
        if dathon.get(key).is_none()
            && let Some(value) = child.get(key)
            && !value.is_null()
        {
            dathon[key] = value.clone();
        }
    }
    if dathon.get("semanticTokensProvider").is_none()
        && let Some(legend) = child.pointer("/semanticTokensProvider/legend")
    {
        dathon["semanticTokensProvider"] = json!({ "legend": legend, "full": true });
    }
    dathon
}

/// Patch the editor's `workspace/configuration` response before it
/// reaches the embedded engine: force `typeCheckingMode` on the
/// Python-analysis sections to `mode`. dathon's own
/// `dathon.typeCheckingMode` setting is authoritative for `.dpy` files,
/// so it drives the embedded engine too — instead of basedpyright's
/// louder out-of-the-box default.
///
/// `result` is an array parallel to `sections`, one settings value per
/// requested config item.
pub fn patch_engine_config(
    mut result: Value,
    sections: &[String],
    mode: dathon::CheckMode,
) -> Value {
    let Some(slots) = result.as_array_mut() else {
        return result;
    };
    for (slot, section) in slots.iter_mut().zip(sections) {
        match section.as_str() {
            // The analysis subtree was requested directly.
            "python.analysis" | "basedpyright.analysis" => set_type_checking_mode(slot, mode),
            // The whole `python` / `basedpyright` tree — analysis nests
            // under `.analysis`.
            "python" | "basedpyright" => {
                if !slot.is_object() {
                    *slot = json!({});
                }
                set_type_checking_mode(&mut slot["analysis"], mode);
            }
            _ => {}
        }
    }
    result
}

/// Force `settings.typeCheckingMode` to `mode`.
fn set_type_checking_mode(settings: &mut Value, mode: dathon::CheckMode) {
    if !settings.is_object() {
        *settings = json!({});
    }
    let obj = settings.as_object_mut().expect("set to an object above");
    obj.insert("typeCheckingMode".to_string(), json!(mode.as_str()));
}

/// Rewrite an editor request's params for the child: the cursor
/// `position` is shifted down by the preamble so it lands on the
/// matching line of the virtual document. Params without a `position`
/// (the request carries the cursor elsewhere) pass through untouched.
pub fn request_params_to_child(mut params: Value) -> Value {
    if let Some(line) = params.pointer_mut("/position/line")
        && let Some(v) = line.as_u64()
    {
        *line = json!(virtualdoc::to_child_line(v as u32));
    }
    params
}

/// Merge dathon's own answer for a fanned-out request with the child
/// engine's answer.
///
/// `hover` stacks the two; `completion` concatenates the item lists;
/// `definition` / `references` union the locations. The remaining
/// methods are pure passthroughs — the child is the only source — but
/// most still need their result remapped out of virtual coordinates:
/// `documentHighlight`, `rename`, `prepareRename`, and `semanticTokens`
/// all carry document positions; `signatureHelp` does not. Any other
/// method just returns dathon's result.
///
/// `semanticTokens/full` is handled directly by [`remap_semantic_tokens`]
/// at the call site — it needs the document's `__future__`-import line,
/// which the rest of the merge doesn't.
pub fn merge_child_response(method: &str, dathon_result: Value, child_result: Value) -> Value {
    match method {
        "textDocument/hover" => merge_hover(dathon_result, child_result),
        "textDocument/completion" => merge_completion(dathon_result, child_result),
        "textDocument/definition" | "textDocument/references" => {
            merge_locations(dathon_result, child_result)
        }
        "textDocument/signatureHelp" => child_result,
        "textDocument/documentHighlight" => remap_highlights(child_result),
        // `rename` / `prepareRename`: dathon answers for a column (its
        // edit is already in editor coordinates — no remap); otherwise
        // the child answers and its result is remapped.
        "textDocument/rename" => {
            if dathon_result.is_null() {
                remap_workspace_edit(child_result)
            } else {
                dathon_result
            }
        }
        "textDocument/prepareRename" => {
            if dathon_result.is_null() {
                remap_prepare_rename(child_result)
            } else {
                dathon_result
            }
        }
        "textDocument/semanticTokens/full" => remap_semantic_tokens(child_result, None),
        _ => dathon_result,
    }
}

/// Merge two `Hover` results. The child's hover is first remapped out
/// of virtual-document coordinates. When both sources have content they
/// are stacked, dathon first, separated by a horizontal rule.
fn merge_hover(dathon: Value, child: Value) -> Value {
    let child = remap_hover_to_editor(child);
    match (hover_text(&dathon), hover_text(&child)) {
        (None, None) => Value::Null,
        (Some(_), None) => dathon,
        (None, Some(_)) => child,
        (Some(d), Some(c)) => {
            // dathon's hover carries no range; fall back to the child's
            // so the editor still highlights the hovered token.
            let range = dathon
                .get("range")
                .filter(|r| !r.is_null())
                .or_else(|| child.get("range").filter(|r| !r.is_null()))
                .cloned();
            let mut out = json!({
                "contents": { "kind": "markdown", "value": format!("{d}\n\n---\n\n{c}") },
            });
            if let Some(range) = range {
                out["range"] = range;
            }
            out
        }
    }
}

/// Extract a hover's text content as a string, or `None` if the value
/// is `null` / carries no `contents`.
fn hover_text(hover: &Value) -> Option<String> {
    let contents = hover.get("contents")?;
    let text = hover_contents_to_string(contents);
    if text.is_empty() { None } else { Some(text) }
}

/// Flatten any of the LSP `Hover.contents` shapes — a plain string, a
/// `MarkedString`, a `MarkupContent`, or an array of those — into one
/// markdown string.
fn hover_contents_to_string(contents: &Value) -> String {
    match contents {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(hover_contents_to_string)
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(obj) => match obj.get("value").and_then(|v| v.as_str()) {
            // `{language, value}` is a MarkedString — render as a fence.
            Some(value) => match obj.get("language").and_then(|l| l.as_str()) {
                Some(lang) => format!("```{lang}\n{value}\n```"),
                None => value.to_string(),
            },
            None => String::new(),
        },
        _ => String::new(),
    }
}

/// Remap a child `Hover`'s `range` out of virtual coordinates. A range
/// that lands inside the preamble has no editor counterpart, so it's
/// dropped (the hover content is still useful without a highlight).
fn remap_hover_to_editor(mut hover: Value) -> Value {
    if let Some(range) = hover.get_mut("range")
        && !remap_range_to_editor(range)
        && let Some(obj) = hover.as_object_mut()
    {
        obj.remove("range");
    }
    hover
}

/// Shift a `Range`'s `start.line` and `end.line` from virtual to editor
/// coordinates. Atomic: if either endpoint falls inside the preamble
/// the range is left untouched and `false` is returned.
fn remap_range_to_editor(range: &mut Value) -> bool {
    let endpoint = |key: &str| -> Option<u32> {
        let line = range.pointer(&format!("/{key}/line"))?.as_u64()? as u32;
        virtualdoc::to_editor_line(line)
    };
    let (Some(start), Some(end)) = (endpoint("start"), endpoint("end")) else {
        return false;
    };
    range["start"]["line"] = json!(start);
    range["end"]["line"] = json!(end);
    true
}

// ---------------------------------------------------------------------------
// completion merge
// ---------------------------------------------------------------------------

/// Merge two completion results. Items from both sources are
/// concatenated, dathon's first; each child item's edit ranges are
/// remapped out of virtual coordinates. The result is a
/// `CompletionList` when the child reports its list incomplete (so the
/// editor keeps re-querying), otherwise a plain item array.
///
/// The child's `itemDefaults` are dropped — applying them to the merged
/// list would also (wrongly) apply them to dathon's items. Child items
/// that relied on a default edit range fall back to label insertion,
/// which is correct for the common `.`-member-access case.
fn merge_completion(dathon: Value, child: Value) -> Value {
    let mut items = completion_items(&dathon);
    for mut item in completion_items(&child) {
        remap_completion_item(&mut item);
        items.push(item);
    }
    let child_incomplete = child
        .get("isIncomplete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if child_incomplete {
        json!({ "isIncomplete": true, "items": items })
    } else {
        Value::Array(items)
    }
}

/// Pull the item array out of either completion shape — a bare
/// `CompletionItem[]` or a `CompletionList { items }`.
fn completion_items(result: &Value) -> Vec<Value> {
    match result {
        Value::Array(items) => items.clone(),
        Value::Object(obj) => obj
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Remap a child completion item's edit ranges out of virtual
/// coordinates — its `textEdit` and any `additionalTextEdits`.
fn remap_completion_item(item: &mut Value) {
    if let Some(text_edit) = item.get_mut("textEdit") {
        remap_text_edit(text_edit);
    }
    if let Some(extra) = item
        .get_mut("additionalTextEdits")
        .and_then(|e| e.as_array_mut())
    {
        for edit in extra {
            remap_text_edit(edit);
        }
    }
}

/// Remap whatever range shape a text edit carries: a `TextEdit`'s
/// `range`, or an `InsertReplaceEdit`'s `insert` + `replace`.
fn remap_text_edit(edit: &mut Value) {
    for key in ["range", "insert", "replace"] {
        if let Some(range) = edit.get_mut(key) {
            remap_range_to_editor(range);
        }
    }
}

// ---------------------------------------------------------------------------
// definition merge
// ---------------------------------------------------------------------------

/// Merge two location-list results (`definition` / `references`) into
/// one `Location[]`. dathon's schema-aware locations come first
/// (already in editor coordinates); the child's are normalized from
/// whatever shape it used (`Location` / `LocationLink`, scalar or
/// array), remapped out of virtual coordinates, and de-duplicated
/// against what's already there. For `references` dathon contributes
/// nothing, so this is effectively the remapped child list.
///
/// A child location inside the preamble is dropped — it points at
/// injected code the user can't see.
fn merge_locations(dathon: Value, child: Value) -> Value {
    let mut out = locations(&dathon);
    for loc in locations(&child) {
        let Some(loc) = remap_location_to_editor(loc) else {
            continue; // landed inside the preamble
        };
        if !out.iter().any(|existing| same_location(existing, &loc)) {
            out.push(loc);
        }
    }
    if out.is_empty() {
        Value::Null
    } else {
        Value::Array(out)
    }
}

/// Normalize any definition-response shape — a scalar or array of
/// `Location` / `LocationLink` — into a flat list of plain `Location`s.
fn locations(result: &Value) -> Vec<Value> {
    match result {
        Value::Array(items) => items.iter().filter_map(to_location).collect(),
        Value::Object(_) => to_location(result).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Coerce one `Location` or `LocationLink` into a plain `Location`.
fn to_location(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    if let Some(uri) = obj.get("uri") {
        // Already a Location.
        Some(json!({ "uri": uri, "range": obj.get("range")? }))
    } else if let Some(target_uri) = obj.get("targetUri") {
        // LocationLink — collapse to its target selection range.
        let range = obj
            .get("targetSelectionRange")
            .or_else(|| obj.get("targetRange"))?;
        Some(json!({ "uri": target_uri, "range": range }))
    } else {
        None
    }
}

/// Remap a `Location`'s range out of virtual coordinates; `None` when
/// it falls inside the preamble.
///
/// Only `.dpy` files are virtual documents carrying the injected
/// preamble. A location in a real file — stdlib, site-packages, a plain
/// `.py` the engine resolved an import to — is already in editor
/// coordinates and passes through untouched; remapping it would shift
/// its line numbers by the preamble height (often underflowing the
/// location into nothing).
fn remap_location_to_editor(mut loc: Value) -> Option<Value> {
    let in_virtual_doc = loc
        .get("uri")
        .and_then(Value::as_str)
        .is_some_and(|uri| uri.ends_with(".dpy"));
    if !in_virtual_doc {
        return Some(loc);
    }
    let range = loc.get_mut("range")?;
    remap_range_to_editor(range).then_some(loc)
}

/// Whether two locations point at the same place — same URI and same
/// start position. Used to drop the duplicate when dathon and the child
/// both resolve, say, a `Schema` class name to its declaration.
fn same_location(a: &Value, b: &Value) -> bool {
    a.get("uri") == b.get("uri") && a.pointer("/range/start") == b.pointer("/range/start")
}

// ---------------------------------------------------------------------------
// documentHighlight remap
// ---------------------------------------------------------------------------

/// Remap a `DocumentHighlight[]` result out of virtual coordinates,
/// dropping any highlight whose range lands inside the preamble.
fn remap_highlights(child: Value) -> Value {
    let Some(items) = child.as_array() else {
        return Value::Null;
    };
    let out: Vec<Value> = items
        .iter()
        .filter_map(|highlight| {
            let mut highlight = highlight.clone();
            let range = highlight.get_mut("range")?;
            remap_range_to_editor(range).then_some(highlight)
        })
        .collect();
    if out.is_empty() {
        Value::Null
    } else {
        Value::Array(out)
    }
}

// ---------------------------------------------------------------------------
// rename remap
// ---------------------------------------------------------------------------

/// Remap a `rename` result — a `WorkspaceEdit` — out of virtual
/// coordinates. Edits are carried in `changes` (a URI→`TextEdit[]` map)
/// and/or `documentChanges` (an array of `TextDocumentEdit`s); every
/// edit range is remapped and edits inside the preamble are dropped.
fn remap_workspace_edit(mut edit: Value) -> Value {
    if !edit.is_object() {
        return Value::Null;
    }
    if let Some(changes) = edit.get_mut("changes").and_then(|c| c.as_object_mut()) {
        for edits in changes.values_mut() {
            remap_text_edit_array(edits);
        }
    }
    if let Some(doc_changes) = edit
        .get_mut("documentChanges")
        .and_then(|d| d.as_array_mut())
    {
        for change in doc_changes {
            // TextDocumentEdit carries `edits`; CreateFile / RenameFile /
            // DeleteFile carry no ranges and are left untouched.
            if let Some(edits) = change.get_mut("edits") {
                remap_text_edit_array(edits);
            }
        }
    }
    edit
}

/// Remap a `TextEdit[]` in place, dropping any edit whose range falls
/// inside the preamble.
fn remap_text_edit_array(edits: &mut Value) {
    if let Some(array) = edits.as_array_mut() {
        array.retain_mut(|edit| {
            edit.get_mut("range")
                .map(remap_range_to_editor)
                .unwrap_or(false)
        });
    }
}

/// Remap a `prepareRename` result out of virtual coordinates. The
/// result is either a bare `Range`, a `{ range, placeholder }` object,
/// or `{ defaultBehavior }` / `null` (no range — returned as-is).
fn remap_prepare_rename(mut result: Value) -> Value {
    if result.get("start").is_some() {
        // Bare `Range`.
        remap_range_to_editor(&mut result);
    } else if let Some(range) = result.get_mut("range") {
        remap_range_to_editor(range);
    }
    result
}

// ---------------------------------------------------------------------------
// semanticTokens remap
// ---------------------------------------------------------------------------

/// Remap a `semanticTokens/full` result out of virtual coordinates.
///
/// The `data` array is a flat stream of 5-tuples `[deltaLine,
/// deltaStartChar, length, tokenType, tokenModifiers]`, each token's
/// line delta-encoded against the previous one. The stream is decoded
/// to absolute positions, tokens inside the preamble are dropped, the
/// rest are shifted up by the preamble height, and the stream is
/// re-encoded.
///
/// The one exception is the hoisted `from __future__` import: it lives
/// on virtual line 0 (the preamble region), so its tokens are mapped to
/// `future_line` — the editor line the import really sits on — instead
/// of being dropped. Without this the `__future__` line would lose its
/// semantic-token coloring.
pub fn remap_semantic_tokens(result: Value, future_line: Option<u32>) -> Value {
    let Some(data) = result.get("data").and_then(|d| d.as_array()) else {
        return Value::Null;
    };
    let preamble = i64::from(virtualdoc::PREFIX_LINE_COUNT);

    // Decode the delta stream to absolute (line, char, length, type, mods).
    let mut tokens: Vec<[i64; 5]> = Vec::with_capacity(data.len() / 5);
    let (mut abs_line, mut abs_char) = (0i64, 0i64);
    for tuple in data.chunks_exact(5) {
        let field = |i: usize| tuple[i].as_i64().unwrap_or(0);
        let (delta_line, delta_start) = (field(0), field(1));
        abs_line += delta_line;
        abs_char = if delta_line == 0 {
            abs_char + delta_start
        } else {
            delta_start
        };
        tokens.push([abs_line, abs_char, field(2), field(3), field(4)]);
    }

    // Map each token's virtual line to an editor line, then re-encode.
    let mut out: Vec<i64> = Vec::new();
    let (mut prev_line, mut prev_char) = (0i64, 0i64);
    for [line, char, length, token_type, modifiers] in tokens {
        let editor_line = if line >= preamble {
            line - preamble
        } else if line == 0 {
            // The hoisted `from __future__` import.
            match future_line {
                Some(future) => i64::from(future),
                None => continue,
            }
        } else {
            continue; // elsewhere in the preamble — no editor counterpart
        };
        let delta_line = editor_line - prev_line;
        let delta_start = if delta_line == 0 {
            char - prev_char
        } else {
            char
        };
        out.extend([delta_line, delta_start, length, token_type, modifiers]);
        (prev_line, prev_char) = (editor_line, char);
    }

    let mut remapped = result;
    remapped["data"] = json!(out);
    remapped
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{DiagnosticSeverity, Position, Range};

    fn diag(line: u32, msg: &str) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position { line, character: 0 },
                end: Position { line, character: 4 },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            message: msg.to_string(),
            ..Default::default()
        }
    }

    fn uri() -> Url {
        Url::parse("file:///t.dpy").unwrap()
    }

    #[test]
    fn store_merges_dathon_and_child_diagnostics() {
        let mut store = DiagnosticStore::default();
        let after_dathon = store.set_dathon(uri(), vec![diag(1, "D0030")]);
        assert_eq!(after_dathon.len(), 1);

        let after_child = store.set_child(uri(), vec![diag(2, "py-error")]);
        // Both sources now present — merged set has both.
        assert_eq!(after_child.len(), 2);
        assert!(after_child.iter().any(|d| d.message == "D0030"));
        assert!(after_child.iter().any(|d| d.message == "py-error"));
    }

    #[test]
    fn store_replaces_per_source_not_appends() {
        let mut store = DiagnosticStore::default();
        store.set_dathon(uri(), vec![diag(1, "first")]);
        let merged = store.set_dathon(uri(), vec![diag(1, "second")]);
        // The second set_dathon replaced the first, didn't append.
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].message, "second");
    }

    #[test]
    fn store_clear_drops_everything_for_a_uri() {
        let mut store = DiagnosticStore::default();
        store.set_dathon(uri(), vec![diag(1, "x")]);
        store.set_child(uri(), vec![diag(2, "y")]);
        store.clear(&uri());
        // After clear, a fresh dathon set starts from empty.
        let merged = store.set_dathon(uri(), vec![]);
        assert!(merged.is_empty());
    }

    #[test]
    fn child_diagnostics_are_remapped_out_of_virtual_coordinates() {
        // The child reports a diagnostic at virtual line
        // PREFIX_LINE_COUNT + 3 — that's real line 3.
        let vline = virtualdoc::PREFIX_LINE_COUNT + 3;
        let params = json!({
            "uri": "file:///t.dpy",
            "diagnostics": [{
                "range": {
                    "start": { "line": vline, "character": 0 },
                    "end": { "line": vline, "character": 5 },
                },
                "message": "undefined name",
            }],
        });
        let (_, diags) = child_diagnostics_to_editor(&params).expect("parsed");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].range.start.line, 3);
        assert_eq!(diags[0].range.end.line, 3);
        assert_eq!(diags[0].source.as_deref(), Some("python"));
    }

    #[test]
    fn child_diagnostics_inside_the_preamble_are_dropped() {
        // A diagnostic entirely on preamble line 1 has no real-file
        // counterpart — it must not reach the editor.
        let params = json!({
            "uri": "file:///t.dpy",
            "diagnostics": [{
                "range": {
                    "start": { "line": 1, "character": 0 },
                    "end": { "line": 1, "character": 5 },
                },
                "message": "unused import in the injected preamble",
            }],
        });
        let (_, diags) = child_diagnostics_to_editor(&params).expect("parsed");
        assert!(diags.is_empty());
    }

    fn missing_import_diag(line: u32, code: &str) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position { line, character: 0 },
                end: Position {
                    line,
                    character: 10,
                },
            },
            code: Some(lsp_types::NumberOrString::String(code.to_string())),
            ..Default::default()
        }
    }

    #[test]
    fn unresolved_relative_import_is_recognized_as_a_dathon_import() {
        let source =
            "from pyspark.sql import functions as F\nfrom .schemas import RawEvents\n";
        // Line 1 is the relative import — the engine can't resolve the
        // sibling `.dpy`, so its complaint is dathon's to own.
        let relative = missing_import_diag(1, "reportMissingImports");
        assert!(is_unresolved_dathon_import(&relative, source));
    }

    #[test]
    fn unresolved_absolute_import_is_kept() {
        // Line 0 is an absolute import — a genuinely missing package is
        // a real diagnostic and must reach the editor.
        let source =
            "from pyspark.sql import functions as F\nfrom .schemas import RawEvents\n";
        let absolute = missing_import_diag(0, "reportMissingImports");
        assert!(!is_unresolved_dathon_import(&absolute, source));
    }

    #[test]
    fn non_import_diagnostic_is_never_treated_as_a_dathon_import() {
        let source = "from .schemas import RawEvents\n";
        // Same line, but a different rule — not an import complaint.
        let other = missing_import_diag(0, "reportUnusedVariable");
        assert!(!is_unresolved_dathon_import(&other, source));
    }

    // -----------------------------------------------------------------------
    // request fan-out: param/position remapping
    // -----------------------------------------------------------------------

    #[test]
    fn request_params_shift_the_cursor_into_virtual_coordinates() {
        let params = json!({
            "textDocument": { "uri": "file:///t.dpy" },
            "position": { "line": 7, "character": 4 },
        });
        let shifted = request_params_to_child(params);
        // Real line 7 lands at virtual line 7 + PREFIX_LINE_COUNT;
        // the character is untouched.
        assert_eq!(
            shifted["position"]["line"],
            json!(virtualdoc::PREFIX_LINE_COUNT + 7),
        );
        assert_eq!(shifted["position"]["character"], json!(4));
    }

    #[test]
    fn request_params_without_a_position_pass_through_untouched() {
        let params = json!({ "textDocument": { "uri": "file:///t.dpy" } });
        assert_eq!(request_params_to_child(params.clone()), params);
    }

    // -----------------------------------------------------------------------
    // request fan-out: hover merge
    // -----------------------------------------------------------------------

    fn hover(markdown: &str) -> Value {
        json!({ "contents": { "kind": "markdown", "value": markdown } })
    }

    #[test]
    fn merge_hover_stacks_both_sources_with_a_rule() {
        let merged = merge_child_response(
            "textDocument/hover",
            hover("**dathon:** schema info"),
            hover("**python:** type info"),
        );
        let text = merged["contents"]["value"].as_str().expect("markdown");
        // dathon's section comes first, then a horizontal rule, then
        // the Python engine's.
        assert!(text.starts_with("**dathon:** schema info"));
        assert!(text.contains("\n\n---\n\n"));
        assert!(text.ends_with("**python:** type info"));
    }

    #[test]
    fn merge_hover_falls_back_to_the_only_present_source() {
        // dathon empty → the child's hover is returned as-is.
        let only_child =
            merge_child_response("textDocument/hover", Value::Null, hover("python only"));
        assert_eq!(only_child["contents"]["value"], json!("python only"));

        // child empty → dathon's hover is returned as-is.
        let only_dathon =
            merge_child_response("textDocument/hover", hover("dathon only"), Value::Null);
        assert_eq!(only_dathon["contents"]["value"], json!("dathon only"));
    }

    #[test]
    fn merge_hover_is_null_when_neither_source_has_content() {
        let merged = merge_child_response("textDocument/hover", Value::Null, Value::Null);
        assert!(merged.is_null());
    }

    #[test]
    fn merge_hover_remaps_the_child_range_out_of_virtual_coordinates() {
        // The child reports its hover range at virtual line
        // PREFIX_LINE_COUNT + 2 — that's real line 2.
        let vline = virtualdoc::PREFIX_LINE_COUNT + 2;
        let child = json!({
            "contents": { "kind": "markdown", "value": "python" },
            "range": {
                "start": { "line": vline, "character": 0 },
                "end": { "line": vline, "character": 6 },
            },
        });
        let merged = merge_child_response("textDocument/hover", Value::Null, child);
        assert_eq!(merged["range"]["start"]["line"], json!(2));
        assert_eq!(merged["range"]["end"]["line"], json!(2));
    }

    #[test]
    fn merge_hover_drops_a_child_range_that_lands_in_the_preamble() {
        // A range on a preamble line has no editor counterpart — the
        // hover content survives but the range is dropped.
        let child = json!({
            "contents": { "kind": "markdown", "value": "python" },
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 6 },
            },
        });
        let merged = merge_child_response("textDocument/hover", Value::Null, child);
        assert_eq!(merged["contents"]["value"], json!("python"));
        assert!(merged.get("range").is_none());
    }

    #[test]
    fn hover_contents_flattens_a_marked_string_into_a_fence() {
        // The child may answer with the legacy `{language, value}`
        // MarkedString shape — it must render as a fenced code block.
        let child = json!({ "contents": { "language": "python", "value": "def f(): ..." } });
        let merged = merge_child_response("textDocument/hover", hover("dathon"), child);
        let text = merged["contents"]["value"].as_str().expect("markdown");
        assert!(text.contains("```python\ndef f(): ...\n```"));
    }

    // -----------------------------------------------------------------------
    // request fan-out: pending table
    // -----------------------------------------------------------------------

    /// A childless multiplexer for exercising the bookkeeping tables
    /// (pending requests, proxied child requests) without a subprocess.
    fn test_multiplexer() -> Multiplexer {
        Multiplexer {
            child: None,
            diagnostics: DiagnosticStore::default(),
            type_checking_mode: dathon::CheckMode::Standard,
            child_capabilities: Value::Null,
            pending: HashMap::new(),
            next_request_seq: 0,
            child_requests: HashMap::new(),
            next_child_request_seq: 0,
        }
    }

    #[test]
    fn forward_request_with_no_child_replies_dathon_only() {
        let mut mux = test_multiplexer();
        let dathon_only = mux.forward_request(
            RequestId::from(1),
            "textDocument/hover",
            json!({}),
            hover("dathon"),
            None,
        );
        // No child → the caller gets dathon's result straight back, and
        // nothing is parked in the pending table.
        assert_eq!(dathon_only, Some(hover("dathon")));
        assert!(mux.drain_pending().is_empty());
    }

    #[test]
    fn take_pending_returns_none_for_an_unknown_id() {
        let mut mux = test_multiplexer();
        assert!(mux.take_pending("dathon-req-999").is_none());
    }

    // -----------------------------------------------------------------------
    // child→editor request proxying
    // -----------------------------------------------------------------------

    #[test]
    fn register_then_take_child_request_round_trips_the_child_id() {
        let mut mux = test_multiplexer();
        // The child's id can be a number or a string — it's echoed back
        // verbatim, so it must survive the round-trip unchanged.
        let editor_id = mux.register_child_request(json!(42), Vec::new());
        assert_eq!(
            mux.take_child_request(&editor_id).map(|p| p.child_id),
            Some(json!(42)),
        );
        // A second take of the same id finds nothing — it was consumed.
        assert!(mux.take_child_request(&editor_id).is_none());
    }

    #[test]
    fn register_child_request_issues_distinct_editor_ids() {
        let mut mux = test_multiplexer();
        let first = mux.register_child_request(json!("a"), Vec::new());
        let second = mux.register_child_request(json!("b"), Vec::new());
        assert_ne!(first, second);
        assert_eq!(
            mux.take_child_request(&first).map(|p| p.child_id),
            Some(json!("a")),
        );
        assert_eq!(
            mux.take_child_request(&second).map(|p| p.child_id),
            Some(json!("b")),
        );
    }

    #[test]
    fn register_child_request_keeps_the_config_sections() {
        let mut mux = test_multiplexer();
        let sections = vec!["python".to_string(), "python.analysis".to_string()];
        let editor_id = mux.register_child_request(json!(1), sections.clone());
        let proxied = mux.take_child_request(&editor_id).expect("registered");
        assert_eq!(proxied.config_sections, sections);
    }

    #[test]
    fn take_child_request_returns_none_for_an_unissued_id() {
        let mut mux = test_multiplexer();
        assert!(mux.take_child_request("dathon-c2e-999").is_none());
    }

    // -----------------------------------------------------------------------
    // workspace/configuration patching
    // -----------------------------------------------------------------------

    #[test]
    fn patch_engine_config_sets_type_checking_on_analysis_sections() {
        // The editor has no Python config — both analysis slots come
        // back null and must carry dathon's mode.
        let sections = vec!["python".to_string(), "python.analysis".to_string()];
        let patched =
            patch_engine_config(json!([null, null]), &sections, dathon::CheckMode::Standard);
        assert_eq!(
            patched[0]["analysis"]["typeCheckingMode"],
            json!("standard")
        );
        assert_eq!(patched[1]["typeCheckingMode"], json!("standard"));
    }

    #[test]
    fn patch_engine_config_makes_dathons_mode_authoritative() {
        // `dathon.typeCheckingMode` drives the engine for `.dpy` files —
        // it overrides whatever the editor's Python config returned.
        let sections = vec!["python.analysis".to_string()];
        let patched = patch_engine_config(
            json!([{ "typeCheckingMode": "recommended" }]),
            &sections,
            dathon::CheckMode::Strict,
        );
        assert_eq!(patched[0]["typeCheckingMode"], json!("strict"));
    }

    #[test]
    fn patch_engine_config_ignores_non_analysis_sections() {
        // A non-analysis section (e.g. an editor section) is untouched.
        let patched = patch_engine_config(
            json!(["keep-me"]),
            &["editor".to_string()],
            dathon::CheckMode::Off,
        );
        assert_eq!(patched[0], json!("keep-me"));
    }

    // -----------------------------------------------------------------------
    // request fan-out: completion merge
    // -----------------------------------------------------------------------

    fn item(label: &str) -> Value {
        json!({ "label": label })
    }

    #[test]
    fn merge_completion_concatenates_both_sources_dathon_first() {
        let merged = merge_child_response(
            "textDocument/completion",
            json!([item("price"), item("place_code")]),
            json!([item("explode"), item("split")]),
        );
        let labels: Vec<&str> = merged
            .as_array()
            .expect("array")
            .iter()
            .map(|i| i["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, vec!["price", "place_code", "explode", "split"]);
    }

    #[test]
    fn merge_completion_remaps_a_child_item_text_edit_range() {
        // The child's textEdit range is in virtual coordinates — at
        // virtual line PREFIX_LINE_COUNT + 4, i.e. real line 4.
        let vline = virtualdoc::PREFIX_LINE_COUNT + 4;
        let child = json!([{
            "label": "explode",
            "textEdit": {
                "range": {
                    "start": { "line": vline, "character": 8 },
                    "end": { "line": vline, "character": 12 },
                },
                "newText": "explode",
            },
        }]);
        let merged = merge_child_response("textDocument/completion", json!([]), child);
        let range = &merged[0]["textEdit"]["range"];
        assert_eq!(range["start"]["line"], json!(4));
        assert_eq!(range["end"]["line"], json!(4));
        // The character is untouched.
        assert_eq!(range["start"]["character"], json!(8));
    }

    #[test]
    fn merge_completion_is_a_completion_list_when_the_child_is_incomplete() {
        let merged = merge_child_response(
            "textDocument/completion",
            json!([item("price")]),
            json!({ "isIncomplete": true, "items": [item("explode")] }),
        );
        // An incomplete child list must surface as a CompletionList so
        // the editor keeps re-querying as the user types.
        assert_eq!(merged["isIncomplete"], json!(true));
        assert_eq!(merged["items"].as_array().expect("items").len(), 2);
    }

    // -----------------------------------------------------------------------
    // request fan-out: definition merge
    // -----------------------------------------------------------------------

    fn location(uri: &str, line: u32) -> Value {
        json!({
            "uri": uri,
            "range": {
                "start": { "line": line, "character": 0 },
                "end": { "line": line, "character": 4 },
            },
        })
    }

    #[test]
    fn merge_definition_unions_locations_from_both_sources() {
        // dathon's location is already in editor coordinates; the
        // child's is in another `.dpy` file at a distinct virtual line.
        let child = location("file:///other.dpy", virtualdoc::PREFIX_LINE_COUNT + 9);
        let merged = merge_child_response(
            "textDocument/definition",
            location("file:///t.dpy", 0),
            child,
        );
        let locs = merged.as_array().expect("array");
        assert_eq!(locs.len(), 2);
        assert_eq!(locs[0]["range"]["start"]["line"], json!(0));
        assert_eq!(locs[1]["range"]["start"]["line"], json!(9));
    }

    #[test]
    fn merge_definition_keeps_real_file_locations_unremapped() {
        // The child resolved a `pyspark` symbol to a real `.py` file —
        // that file has no injected preamble, so its line numbers must
        // pass through untouched (not shifted down by the prefix).
        let child = location("file:///site-packages/pyspark/sql/functions.py", 42);
        let merged = merge_child_response("textDocument/definition", Value::Null, child);
        assert_eq!(merged[0]["range"]["start"]["line"], json!(42));
    }

    #[test]
    fn merge_definition_coerces_a_location_link_to_a_location() {
        // The child answers with a LocationLink (the editor advertised
        // linkSupport) — it must collapse to a plain Location.
        let vline = virtualdoc::PREFIX_LINE_COUNT + 2;
        let child = json!([{
            "targetUri": "file:///t.dpy",
            "targetRange": {
                "start": { "line": vline, "character": 0 },
                "end": { "line": vline, "character": 20 },
            },
            "targetSelectionRange": {
                "start": { "line": vline, "character": 6 },
                "end": { "line": vline, "character": 12 },
            },
        }]);
        let merged = merge_child_response("textDocument/definition", Value::Null, child);
        let loc = &merged[0];
        assert_eq!(loc["uri"], json!("file:///t.dpy"));
        // Collapsed to the target *selection* range, remapped to real line 2.
        assert_eq!(loc["range"]["start"]["line"], json!(2));
        assert_eq!(loc["range"]["start"]["character"], json!(6));
    }

    #[test]
    fn merge_definition_drops_a_child_location_inside_the_preamble() {
        // The child resolves a name to the injected preamble — that
        // location has no editor counterpart and must be dropped.
        let merged = merge_child_response(
            "textDocument/definition",
            Value::Null,
            location("file:///t.dpy", 1),
        );
        assert!(merged.is_null());
    }

    #[test]
    fn merge_definition_dedups_a_location_both_sources_resolve() {
        // dathon and the child both resolve a Schema name to its class
        // declaration on real line 0 — the merged list has it once.
        let dathon = location("file:///t.dpy", 0);
        let child = location("file:///t.dpy", virtualdoc::PREFIX_LINE_COUNT);
        let merged = merge_child_response("textDocument/definition", dathon, child);
        assert_eq!(merged.as_array().expect("array").len(), 1);
    }

    #[test]
    fn merge_references_remaps_the_child_locations() {
        // `references` is a pure passthrough — dathon contributes
        // nothing — but the child's locations still need remapping.
        let child = location("file:///t.dpy", virtualdoc::PREFIX_LINE_COUNT + 7);
        let merged = merge_child_response("textDocument/references", Value::Null, child);
        assert_eq!(merged[0]["range"]["start"]["line"], json!(7));
    }

    #[test]
    fn merge_signature_help_passes_the_child_result_through() {
        // A SignatureHelp carries no document coordinates — it's
        // returned exactly as the child sent it.
        let child = json!({
            "signatures": [{ "label": "explode(col: Column) -> Column" }],
            "activeSignature": 0,
            "activeParameter": 0,
        });
        let merged = merge_child_response("textDocument/signatureHelp", Value::Null, child.clone());
        assert_eq!(merged, child);
    }

    // -----------------------------------------------------------------------
    // capability negotiation
    // -----------------------------------------------------------------------

    #[test]
    fn merge_capabilities_adopts_an_allowlisted_child_capability() {
        let dathon = json!({ "hoverProvider": true });
        let child = json!({
            "signatureHelpProvider": { "triggerCharacters": ["(", ","] },
            "referencesProvider": true,
        });
        let merged = merge_capabilities(dathon, &child);
        // Both allowlisted capabilities are now advertised, with the
        // child's own value (so trigger characters are preserved).
        assert_eq!(
            merged["signatureHelpProvider"]["triggerCharacters"],
            json!(["(", ","]),
        );
        assert_eq!(merged["referencesProvider"], json!(true));
        // dathon's own capability is untouched.
        assert_eq!(merged["hoverProvider"], json!(true));
    }

    #[test]
    fn merge_capabilities_ignores_capabilities_off_the_allowlist() {
        // The child supports type-definition and code lenses, but
        // dathon-lsp doesn't remap those results yet — they must not be
        // advertised to the editor.
        let merged = merge_capabilities(
            json!({ "hoverProvider": true }),
            &json!({ "typeDefinitionProvider": true, "codeLensProvider": {} }),
        );
        assert!(merged.get("typeDefinitionProvider").is_none());
        assert!(merged.get("codeLensProvider").is_none());
    }

    #[test]
    fn merge_capabilities_keeps_dathons_value_when_both_provide_a_method() {
        // dathon already answers references itself (hypothetically) —
        // its own capability wins over the child's.
        let merged = merge_capabilities(
            json!({ "referencesProvider": false }),
            &json!({ "referencesProvider": true }),
        );
        assert_eq!(merged["referencesProvider"], json!(false));
    }

    #[test]
    fn merge_capabilities_with_no_child_leaves_dathons_unchanged() {
        // No embedded engine → child capabilities are `Null`.
        let dathon = json!({ "hoverProvider": true, "definitionProvider": true });
        let merged = merge_capabilities(dathon.clone(), &Value::Null);
        assert_eq!(merged, dathon);
    }

    #[test]
    fn child_init_params_strips_pull_diagnostic_capabilities() {
        let params = json!({
            "capabilities": {
                "textDocument": { "diagnostic": { "dynamicRegistration": true }, "hover": {} },
                "workspace": { "diagnostics": { "refreshSupport": true }, "configuration": true },
            },
        });
        let sanitized = child_init_params(&params);
        // The pull-diagnostics capabilities are gone — so the child
        // falls back to pushing `publishDiagnostics`.
        assert!(
            sanitized
                .pointer("/capabilities/textDocument/diagnostic")
                .is_none()
        );
        assert!(
            sanitized
                .pointer("/capabilities/workspace/diagnostics")
                .is_none()
        );
        // Every other capability is forwarded untouched.
        assert!(
            sanitized
                .pointer("/capabilities/textDocument/hover")
                .is_some()
        );
        assert_eq!(
            sanitized.pointer("/capabilities/workspace/configuration"),
            Some(&json!(true)),
        );
    }

    #[test]
    fn merge_capabilities_adopts_document_highlight_and_rename() {
        let merged = merge_capabilities(
            json!({ "hoverProvider": true }),
            &json!({
                "documentHighlightProvider": true,
                "renameProvider": { "prepareProvider": true },
            }),
        );
        assert_eq!(merged["documentHighlightProvider"], json!(true));
        assert_eq!(merged["renameProvider"]["prepareProvider"], json!(true));
    }

    #[test]
    fn merge_capabilities_restricts_semantic_tokens_to_full_with_the_legend() {
        // The child supports range and delta requests too; dathon-lsp
        // only remaps `full`, so it advertises just that — but it must
        // forward the legend so the editor can decode token types.
        let legend = json!({ "tokenTypes": ["class", "function"], "tokenModifiers": [] });
        let merged = merge_capabilities(
            json!({ "hoverProvider": true }),
            &json!({
                "semanticTokensProvider": {
                    "legend": legend,
                    "full": { "delta": true },
                    "range": true,
                },
            }),
        );
        let st = &merged["semanticTokensProvider"];
        assert_eq!(st["legend"], legend);
        assert_eq!(st["full"], json!(true));
        assert!(st.get("range").is_none());
    }

    // -----------------------------------------------------------------------
    // wider passthrough: documentHighlight / rename / semanticTokens
    // -----------------------------------------------------------------------

    fn ranged(line: u32) -> Value {
        json!({
            "start": { "line": line, "character": 0 },
            "end": { "line": line, "character": 4 },
        })
    }

    #[test]
    fn document_highlight_remaps_and_drops_preamble_hits() {
        let child = json!([
            { "range": ranged(1), "kind": 1 },                                  // preamble
            { "range": ranged(virtualdoc::PREFIX_LINE_COUNT + 6), "kind": 2 }, // real line 6
        ]);
        let merged = merge_child_response("textDocument/documentHighlight", Value::Null, child);
        let out = merged.as_array().expect("array");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["range"]["start"]["line"], json!(6));
        assert_eq!(out[0]["kind"], json!(2));
    }

    #[test]
    fn rename_remaps_workspace_edit_changes_and_drops_preamble_edits() {
        let uri = "file:///t.dpy";
        let child = json!({
            "changes": {
                uri: [
                    { "range": ranged(1), "newText": "x" },                                  // preamble
                    { "range": ranged(virtualdoc::PREFIX_LINE_COUNT + 4), "newText": "x" }, // real line 4
                ],
            },
        });
        let merged = merge_child_response("textDocument/rename", Value::Null, child);
        let edits = merged["changes"][uri].as_array().expect("edits");
        // The preamble edit is dropped; the real edit is remapped.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["range"]["start"]["line"], json!(4));
    }

    #[test]
    fn rename_remaps_document_changes_form() {
        let child = json!({
            "documentChanges": [{
                "textDocument": { "uri": "file:///t.dpy", "version": 1 },
                "edits": [
                    { "range": ranged(virtualdoc::PREFIX_LINE_COUNT + 9), "newText": "y" },
                ],
            }],
        });
        let merged = merge_child_response("textDocument/rename", Value::Null, child);
        let edits = merged["documentChanges"][0]["edits"]
            .as_array()
            .expect("edits");
        assert_eq!(edits[0]["range"]["start"]["line"], json!(9));
    }

    #[test]
    fn prepare_rename_remaps_both_result_shapes() {
        // Bare `Range`.
        let bare = merge_child_response(
            "textDocument/prepareRename",
            Value::Null,
            ranged(virtualdoc::PREFIX_LINE_COUNT + 3),
        );
        assert_eq!(bare["start"]["line"], json!(3));

        // `{ range, placeholder }`.
        let with_placeholder = merge_child_response(
            "textDocument/prepareRename",
            Value::Null,
            json!({ "range": ranged(virtualdoc::PREFIX_LINE_COUNT + 3), "placeholder": "old" }),
        );
        assert_eq!(with_placeholder["range"]["start"]["line"], json!(3));
        assert_eq!(with_placeholder["placeholder"], json!("old"));
    }

    #[test]
    fn semantic_tokens_drop_the_preamble_and_shift_real_tokens() {
        let preamble = i64::from(virtualdoc::PREFIX_LINE_COUNT);
        // Token A on preamble line 1; token B on real line 2 (virtual
        // line preamble+2), delta-encoded against A.
        let child = json!({
            "data": [
                1, 0, 4, 0, 0,                  // A: deltaLine 1 → virtual line 1
                preamble + 1, 3, 5, 1, 0,       // B: → virtual line preamble+2
            ],
        });
        let merged = merge_child_response("textDocument/semanticTokens/full", Value::Null, child);
        // A is dropped; B becomes the first token at editor line 2.
        assert_eq!(merged["data"], json!([2, 3, 5, 1, 0]));
    }

    #[test]
    fn semantic_tokens_preserve_same_line_delta_encoding() {
        let preamble = i64::from(virtualdoc::PREFIX_LINE_COUNT);
        // Two real tokens on the same line (virtual line preamble+0):
        // the second is delta-encoded against the first.
        let child = json!({
            "data": [
                preamble, 0, 3, 0, 0,   // first real token, char 0
                0, 5, 2, 0, 0,          // same line, char 5
            ],
        });
        let merged = merge_child_response("textDocument/semanticTokens/full", Value::Null, child);
        // First token shifts to editor line 0; the same-line delta is intact.
        assert_eq!(merged["data"], json!([0, 0, 3, 0, 0, 0, 5, 2, 0, 0]));
    }

    #[test]
    fn semantic_tokens_map_the_hoisted_future_line_to_its_editor_line() {
        // A token on virtual line 0 is the hoisted `from __future__`
        // import; with the import on editor line 3 its tokens land there
        // rather than being dropped.
        let preamble = i64::from(virtualdoc::PREFIX_LINE_COUNT);
        let child = json!({
            "data": [
                0, 5, 10, 0, 0,             // virtual line 0 — the hoisted import
                preamble + 8, 0, 4, 1, 0,   // virtual line preamble+8 — real line 8
            ],
        });
        let remapped = remap_semantic_tokens(child, Some(3));
        // The `__future__` token → editor line 3; the body token → line 8.
        assert_eq!(remapped["data"], json!([3, 5, 10, 0, 0, 5, 0, 4, 1, 0]));
    }

    #[test]
    fn semantic_tokens_drop_the_hoisted_line_when_there_is_no_future_import() {
        // A virtual-line-0 token with no `from __future__` line to map
        // to is dropped (it can only be preamble noise).
        let child = json!({ "data": [0, 0, 4, 0, 0] });
        let remapped = remap_semantic_tokens(child, None);
        assert_eq!(remapped["data"], json!([]));
    }
}
