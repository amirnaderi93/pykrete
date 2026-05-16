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
//!
//! Foundation scope (this iteration): lifecycle + text sync + diagnostic
//! merge. Request fan-out (hover / completion / definition) is the next
//! iteration — until then those stay dathon-only.

use std::collections::HashMap;

use lsp_server::RequestId;
use lsp_types::{Diagnostic, Url};
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
}

/// The embedded Python language server plus the merged-diagnostic state.
///
/// `child` is `None` when no Python engine was found — dathon-lsp then
/// runs dathon-only, exactly the pre-multiplexer behavior.
pub struct Multiplexer {
    pub child: Option<ChildLsp>,
    pub diagnostics: DiagnosticStore,
    /// Requests forwarded to the child, keyed by the synthetic id we
    /// gave the child (`dathon-req-N`). Drained as the child answers.
    pending: HashMap<String, PendingRequest>,
    /// Monotonic counter for synthetic child request ids.
    next_request_seq: u64,
}

impl Multiplexer {
    /// Spawn and initialize the embedded Python engine, handing it the
    /// same `InitializeParams` the editor sent dathon-lsp. `explicit`
    /// is the `dathon.pythonServer.path` setting, if configured.
    ///
    /// Never fails — if no engine is found or it doesn't handshake,
    /// the returned `Multiplexer` simply has `child: None`.
    pub fn start(explicit: Option<&str>, init_params: &Value) -> Multiplexer {
        let child = crate::child::discover(explicit)
            .and_then(|(program, args)| ChildLsp::spawn(&program, &args))
            .and_then(|mut child| Self::handshake(&mut child, init_params).then_some(child));
        Multiplexer {
            child,
            diagnostics: DiagnosticStore::default(),
            pending: HashMap::new(),
            next_request_seq: 0,
        }
    }

    /// Run the LSP `initialize` / `initialized` handshake with the
    /// child. Returns whether it succeeded.
    fn handshake(child: &mut ChildLsp, init_params: &Value) -> bool {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": "dathon-child-init",
            "method": "initialize",
            "params": init_params,
        });
        if child.send(&initialize).is_err() {
            return false;
        }
        // Wait for the child's initialize response. The child shouldn't
        // emit anything else before it, but tolerate interleaved
        // notifications by looping until we see our id back.
        loop {
            match child.receiver.recv() {
                Ok(msg) => {
                    if msg.get("id").and_then(|v| v.as_str()) == Some("dathon-child-init") {
                        break;
                    }
                }
                Err(_) => return false, // child died during handshake
            }
        }
        let initialized = json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {},
        });
        child.send(&initialized).is_ok()
    }

    /// Whether a Python engine is embedded. `false` means dathon-only.
    pub fn has_child(&self) -> bool {
        self.child.is_some()
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

    /// Send a JSON-RPC response back to the child for a request it made
    /// of us (`workspace/configuration`, `client/registerCapability`, …).
    /// The foundation answers these minimally so the child doesn't stall;
    /// real editor proxying is a later iteration.
    pub fn reply_to_child(&mut self, id: Value, result: Value) {
        self.send_to_child(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }));
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
/// Foundation scope: only `textDocument/hover` is fanned out — dathon's
/// schema-aware hover is stacked above the Python engine's. Any other
/// method just returns dathon's result unchanged (it isn't fanned out
/// yet, so the child result is empty anyway).
pub fn merge_child_response(method: &str, dathon_result: Value, child_result: Value) -> Value {
    match method {
        "textDocument/hover" => merge_hover(dathon_result, child_result),
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
/// coordinates in place. Returns `false` (leaving the range partially
/// mutated) if either endpoint falls inside the preamble.
fn remap_range_to_editor(range: &mut Value) -> bool {
    let remap_endpoint = |range: &mut Value, key: &str| -> Option<()> {
        let line = range.pointer_mut(&format!("/{key}/line"))?;
        let mapped = virtualdoc::to_editor_line(line.as_u64()? as u32)?;
        *line = json!(mapped);
        Some(())
    };
    remap_endpoint(range, "start").is_some() && remap_endpoint(range, "end").is_some()
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
        // PREAMBLE_LINE_COUNT + 3 — that's real line 3.
        let vline = virtualdoc::PREAMBLE_LINE_COUNT + 3;
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
        // Real line 7 lands at virtual line 7 + PREAMBLE_LINE_COUNT;
        // the character is untouched.
        assert_eq!(
            shifted["position"]["line"],
            json!(virtualdoc::PREAMBLE_LINE_COUNT + 7),
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
        // PREAMBLE_LINE_COUNT + 2 — that's real line 2.
        let vline = virtualdoc::PREAMBLE_LINE_COUNT + 2;
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

    #[test]
    fn forward_request_with_no_child_replies_dathon_only() {
        let mut mux = Multiplexer {
            child: None,
            diagnostics: DiagnosticStore::default(),
            pending: HashMap::new(),
            next_request_seq: 0,
        };
        let dathon_only = mux.forward_request(
            RequestId::from(1),
            "textDocument/hover",
            json!({}),
            hover("dathon"),
        );
        // No child → the caller gets dathon's result straight back, and
        // nothing is parked in the pending table.
        assert_eq!(dathon_only, Some(hover("dathon")));
        assert!(mux.drain_pending().is_empty());
    }

    #[test]
    fn take_pending_returns_none_for_an_unknown_id() {
        let mut mux = Multiplexer {
            child: None,
            diagnostics: DiagnosticStore::default(),
            pending: HashMap::new(),
            next_request_seq: 0,
        };
        assert!(mux.take_pending("dathon-req-999").is_none());
    }
}
