//! Test fixture: a deliberately broken Python-LSP impersonator.
//!
//! Speaks just enough JSON-RPC to clear pykrete-lsp's `initialize`
//! handshake, then drops every subsequent request on the floor. Used by
//! the hover-timeout integration test to reproduce basedpyright's
//! never-answers-hover behavior (seen when its workspace has "No source
//! files found", a common state for `.pyk`-only workspaces).
//!
//! Not shipped: only built for `cargo test`.

use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

fn main() {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    loop {
        let Ok(Some(msg)) = read_framed(&mut reader) else {
            return;
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        // Only `initialize` gets a reply — everything else (notifications
        // included) is ignored. That includes `shutdown`, so the parent
        // ends up killing us; fine for a test fixture.
        if method == "initialize"
            && let Some(id) = id
        {
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "capabilities": { "hoverProvider": true } },
            });
            let _ = write_framed(&mut writer, &response);
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
    std::io::Read::read_exact(input, &mut body)?;
    Ok(serde_json::from_slice(&body).ok())
}

fn write_framed<W: Write>(out: &mut W, msg: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(&body)?;
    out.flush()
}
