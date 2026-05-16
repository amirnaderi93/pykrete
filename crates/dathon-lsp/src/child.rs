//! Embedded Python language server — child-process management.
//!
//! The multiplexer spawns a Python LSP (basedpyright today) as a child
//! process and speaks LSP JSON-RPC to it over its stdin/stdout pipes.
//! This module owns:
//!
//! - [`encode_message`] / [`read_message`] — the `Content-Length`-framed
//!   JSON-RPC wire format, as pure functions over `Write` / `BufRead`
//!   so they're unit-testable without a real subprocess.
//! - [`ChildLsp`] — the spawned process: a writer for its stdin and a
//!   channel of messages parsed off its stdout by a reader thread.
//!
//! Discovery (where the Python LSP binary lives) is in [`discover`];
//! when nothing is found the multiplexer runs dathon-only.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde_json::Value;

/// Frame a JSON-RPC message with its `Content-Length` header and write
/// it to `out` — the LSP base protocol's wire format.
pub fn encode_message<W: Write>(out: &mut W, msg: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(&body)?;
    out.flush()
}

/// Read one `Content-Length`-framed JSON-RPC message from `input`.
///
/// Returns `Ok(None)` at clean end-of-stream (the child closed its
/// stdout), `Err` on a malformed frame or I/O failure.
pub fn read_message<R: BufRead>(input: &mut R) -> std::io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = input.read_line(&mut line)?;
        if n == 0 {
            // EOF. Clean only if it happened before any header bytes.
            return if content_length.is_none() {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "stream ended mid-header",
                ))
            };
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line terminates the header block
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
        // Other headers (Content-Type, …) are ignored per the spec.
    }
    let len = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message had no Content-Length header",
        )
    })?;
    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    let value = serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(value))
}

/// A spawned child language server.
///
/// `send` writes a JSON-RPC message to the child; `receiver` yields
/// every message the child emits on its stdout (parsed by a background
/// reader thread). When the child exits, the reader thread ends and the
/// channel closes.
pub struct ChildLsp {
    process: Child,
    stdin: ChildStdin,
    /// Messages parsed off the child's stdout. Closes when the child's
    /// stdout reaches EOF (i.e. the child exited).
    pub receiver: Receiver<Value>,
}

impl ChildLsp {
    /// Spawn `program` with `args` as a child LSP speaking JSON-RPC over
    /// stdio. Returns `None` if the process can't be started.
    pub fn spawn(program: &str, args: &[String]) -> Option<ChildLsp> {
        let mut process = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The child's stderr is its own diagnostic log; let it
            // inherit so it lands in the editor's LSP output channel
            // rather than being silently dropped.
            .stderr(Stdio::inherit())
            .spawn()
            .ok()?;
        let stdin = process.stdin.take()?;
        let stdout = process.stdout.take()?;
        let (tx, rx) = unbounded();
        thread::Builder::new()
            .name("dathon-child-lsp-reader".to_string())
            .spawn(move || reader_loop(BufReader::new(stdout), tx))
            .ok()?;
        Some(ChildLsp {
            process,
            stdin,
            receiver: rx,
        })
    }

    /// Write a JSON-RPC message to the child.
    pub fn send(&mut self, msg: &Value) -> std::io::Result<()> {
        encode_message(&mut self.stdin, msg)
    }

    /// Ask the child to terminate. Best-effort: kills the process if it
    /// doesn't exit on its own. Called when dathon-lsp itself shuts down.
    pub fn shutdown(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// Read framed messages off the child's stdout until the pipe closes,
/// forwarding each onto `tx`. A malformed frame ends the loop — a child
/// LSP that corrupts its own output stream can't be trusted to recover.
fn reader_loop<R: Read>(mut reader: BufReader<R>, tx: Sender<Value>) {
    loop {
        match read_message(&mut reader) {
            Ok(Some(msg)) => {
                if tx.send(msg).is_err() {
                    return; // multiplexer dropped the receiver
                }
            }
            Ok(None) => return, // clean EOF — child exited
            Err(_) => return,   // malformed frame / I/O error
        }
    }
}

/// Locate a Python language server to embed.
///
/// Resolution order:
/// 1. An `explicit` launch spec the editor supplied — the basedpyright
///    the VS Code extension bundles, or a `dathon.pythonServer.path`
///    override. Used verbatim.
/// 2. `basedpyright-langserver` on `PATH`.
/// 3. `pyright-langserver` on `PATH`.
///
/// Returns the `(program, args)` to spawn, or `None` — in which case
/// the multiplexer runs dathon-only (no Python features), exactly the
/// pre-multiplexer behavior.
pub fn discover(explicit: Option<(String, Vec<String>)>) -> Option<(String, Vec<String>)> {
    if let Some(spec) = explicit {
        return Some(spec);
    }
    let stdio = vec!["--stdio".to_string()];
    for candidate in ["basedpyright-langserver", "pyright-langserver"] {
        if is_on_path(candidate) {
            return Some((candidate.to_string(), stdio));
        }
    }
    None
}

/// Whether `program` is an executable findable on `PATH`. Uses a probe
/// spawn rather than re-implementing `PATH` resolution.
fn is_on_path(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut c| {
            let _ = c.wait();
            true
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn encode_then_read_round_trips_a_message() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "uri": "file:///t.dpy" },
        });
        let mut buf: Vec<u8> = Vec::new();
        encode_message(&mut buf, &msg).unwrap();

        // The frame must carry a Content-Length header.
        let as_text = String::from_utf8_lossy(&buf);
        assert!(as_text.starts_with("Content-Length: "));
        assert!(as_text.contains("\r\n\r\n"));

        let mut cursor = Cursor::new(buf);
        let decoded = read_message(&mut cursor).unwrap().expect("a message");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn read_message_returns_none_at_clean_eof() {
        let mut empty = Cursor::new(Vec::<u8>::new());
        assert!(read_message(&mut empty).unwrap().is_none());
    }

    #[test]
    fn read_message_handles_two_messages_back_to_back() {
        let a = serde_json::json!({ "id": 1, "method": "a" });
        let b = serde_json::json!({ "id": 2, "method": "b" });
        let mut buf: Vec<u8> = Vec::new();
        encode_message(&mut buf, &a).unwrap();
        encode_message(&mut buf, &b).unwrap();

        let mut cursor = Cursor::new(buf);
        assert_eq!(read_message(&mut cursor).unwrap().unwrap(), a);
        assert_eq!(read_message(&mut cursor).unwrap().unwrap(), b);
        assert!(read_message(&mut cursor).unwrap().is_none());
    }

    #[test]
    fn read_message_errors_on_a_frame_with_no_content_length() {
        // Header block (blank line) with no Content-Length, then EOF.
        let mut cursor = Cursor::new(b"X-Other: 1\r\n\r\n".to_vec());
        assert!(read_message(&mut cursor).is_err());
    }

    #[test]
    fn discover_uses_an_explicit_spec_verbatim() {
        // The bundled-engine spec — `node <langserver.js> --stdio` —
        // must come back exactly as given, multi-arg and all.
        let spec = (
            "node".to_string(),
            vec![
                "/ext/node_modules/basedpyright/langserver.index.js".to_string(),
                "--stdio".to_string(),
            ],
        );
        assert_eq!(discover(Some(spec.clone())), Some(spec));
    }

    #[test]
    fn discover_without_an_explicit_spec_searches_path() {
        // No spec → PATH discovery. The result depends on the host, so
        // we only assert the call doesn't panic.
        let _ = discover(None);
    }
}
