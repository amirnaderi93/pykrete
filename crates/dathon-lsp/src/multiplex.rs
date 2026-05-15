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

/// The embedded Python language server plus the merged-diagnostic state.
///
/// `child` is `None` when no Python engine was found — dathon-lsp then
/// runs dathon-only, exactly the pre-multiplexer behavior.
pub struct Multiplexer {
    pub child: Option<ChildLsp>,
    pub diagnostics: DiagnosticStore,
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
}
