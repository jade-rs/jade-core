//! `jade-pptx-headless-mcp` — a Model Context Protocol server that reads
//! presentations over stdio.
//!
//! The counterpart of `jade-docx-headless-mcp` and `jade-xlsx-headless-mcp`,
//! and the same shape: an agent opens a deck once and queries it through a
//! session id rather than paying to reparse the file on every call. Unlike
//! its siblings it is read-only for now — pptx write-back does not exist yet.
//!
//! ## Transport
//!
//! MCP stdio: newline-delimited JSON-RPC 2.0. One JSON object per line on
//! stdin; the response, if the request had an `id`, is one object per line on
//! stdout. Notifications produce nothing.

pub mod server;

/// Serves `server` over MCP's stdio transport until stdin closes.
pub fn serve_stdio(server: &mut server::Server) {
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_message(&line) {
            if out.write_all(response.as_bytes()).is_err()
                || out.write_all(b"\n").is_err()
                || out.flush().is_err()
            {
                break;
            }
        }
    }
}
