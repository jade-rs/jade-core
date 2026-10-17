//! `jade-xlsx-headless-mcp` — a Model Context Protocol server that drives the
//! headless spreadsheet editor over stdio.
//!
//! The counterpart of `jade-docx-headless-mcp`, and the same shape: an agent
//! opens a workbook once and then reads and writes it through a session id,
//! rather than paying to reparse and rezip the file on every call.
//!
//! ## Transport
//!
//! MCP stdio: newline-delimited JSON-RPC 2.0. One JSON object per line on
//! stdin; the response, if the request had an `id`, is one object per line on
//! stdout. Notifications produce nothing.
//!
//! ## Why a session rather than a path per call
//!
//! A workbook is parsed once and held. Reading a cell out of a 5000-row sheet
//! is then a lookup rather than a parse, and a sequence of writes patches one
//! copy in memory instead of the file five times over.
//!
//! ## What it cannot do
//!
//! Recalculate. A write leaves the cached values of dependent formulas stale;
//! the exported file asks the spreadsheet that opens it to recalculate, but a
//! `read_cell` through this server keeps reporting the cached value until jade
//! grows a calculation engine. Formula cells carry both the expression and a
//! `formula_state` that remains `stale` after export and reopen.

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
