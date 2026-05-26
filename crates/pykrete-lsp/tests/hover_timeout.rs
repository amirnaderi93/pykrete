//! End-to-end check that `textDocument/hover` doesn't hang when the
//! embedded Python engine never answers.
//!
//! Reproduces the v0.1.8 bug: basedpyright sometimes never replies to
//! hover requests (notably when its workspace has "No source files
//! found"), and pykrete-lsp's fanout-and-merge path would wait for it
//! forever — the editor's hover popup never appeared.
//!
//! Strategy: spawn the real `pykrete-lsp` binary, point it at the
//! `silent-child-lsp` test fixture via `initializationOptions.pythonServer`
//! (the fixture answers `initialize` and then ignores everything),
//! send a `textDocument/hover` request, and assert the response arrives
//! within a few seconds carrying pykrete's standalone result.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Spawned pykrete-lsp under test: writer for its stdin plus a channel
/// of every framed JSON-RPC message it emits on stdout.
struct Server {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
}

impl Server {
    fn spawn() -> Server {
        let exe = env!("CARGO_BIN_EXE_pykrete-lsp");
        let mut child = Command::new(exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Stderr is the LSP server's own log channel — drop it so it
            // doesn't pollute test output.
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn pykrete-lsp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || reader_loop(BufReader::new(stdout), tx));
        Server { child, stdin, rx }
    }

    fn send(&mut self, msg: &Value) {
        let body = serde_json::to_vec(msg).expect("serialize");
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len()).expect("write header");
        self.stdin.write_all(&body).expect("write body");
        self.stdin.flush().expect("flush");
    }

    /// Receive messages until one matches `pred`, or the overall budget
    /// runs out. Other messages (log notifications, diagnostics, child
    /// proxy requests) are drained and ignored.
    fn recv_matching<F>(&self, mut pred: F, total: Duration) -> Option<Value>
    where
        F: FnMut(&Value) -> bool,
    {
        let deadline = Instant::now() + total;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match self.rx.recv_timeout(remaining) {
                Ok(msg) => {
                    if pred(&msg) {
                        return Some(msg);
                    }
                }
                Err(_) => return None,
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn reader_loop(mut reader: BufReader<ChildStdout>, tx: mpsc::Sender<Value>) {
    loop {
        match read_framed(&mut reader) {
            Ok(Some(msg)) => {
                if tx.send(msg).is_err() {
                    return;
                }
            }
            _ => return,
        }
    }
}

fn read_framed<R: BufRead>(input: &mut R) -> std::io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = input.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let Some(len) = content_length else {
        return Ok(None);
    };
    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body).ok())
}

#[test]
fn hover_falls_back_to_pykrete_only_when_child_doesnt_reply() {
    // Point pykrete-lsp at the silent-child fixture so its embedded
    // engine handshakes successfully but then never answers hover.
    let silent_child = env!("CARGO_BIN_EXE_silent-child-lsp");
    let mut server = Server::spawn();

    server.send(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": null,
            "capabilities": {},
            "initializationOptions": {
                "pythonServer": { "command": silent_child, "args": [] },
            },
        },
    }));
    let initialize_response = server
        .recv_matching(
            |m| m.get("id").and_then(|v| v.as_i64()) == Some(1),
            // The initialize handshake includes the child's own
            // `initialize` round-trip, which is normally fast.
            Duration::from_secs(5),
        )
        .expect("initialize response");
    assert!(initialize_response.get("result").is_some());

    server.send(&json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {},
    }));

    // Open a `.pyk` document with a `Schema` class — pykrete's own hover
    // can identify it, so we have a non-null fallback to assert against.
    let uri = "file:///t.pyk";
    let source = "class Orders(Schema):\n    place_code: int\n    price: int\n";
    server.send(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "pyk",
                "version": 1,
                "text": source,
            },
        },
    }));

    let started = Instant::now();
    server.send(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": uri },
            // Line 0 char 6 → inside the "Orders" symbol.
            "position": { "line": 0, "character": 6 },
        },
    }));
    // The fanout timeout is 2s; allow generous slack for CI but keep it
    // well below "forever" so a regression of the bug fails loudly.
    let hover_response = server
        .recv_matching(
            |m| m.get("id").and_then(|v| v.as_i64()) == Some(2),
            Duration::from_secs(8),
        )
        .expect("hover response within the timeout window");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(7),
        "hover should answer within ~2s of the deadline, took {elapsed:?}",
    );

    // The response must carry pykrete's own hover content for the Schema
    // class — the child contributed nothing because it stayed silent.
    let text = hover_response
        .pointer("/result/contents/value")
        .and_then(|v| v.as_str())
        .expect("hover markdown content");
    assert!(
        text.contains("Orders"),
        "expected pykrete's hover for Orders, got: {text}",
    );
}
