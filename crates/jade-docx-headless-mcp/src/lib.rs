//! `jade-docx-headless-mcp` — a Model Context Protocol (MCP) server that drives the
//! headless DOCX editor over stdio.
//!
//! The server keeps loaded documents in memory keyed by a session id, so an MCP
//! client (e.g. an AI agent) can `open_docx` once and then issue many
//! `apply_commands` / `query` / `undo` / `redo` calls without re-parsing and
//! re-zipping the file on every edit. That in-memory session model is what makes
//! iterative editing fast compared with the stateless `jade-docx-headless-editor`
//! convenience functions that load + apply + export per call.
//!
//! ## Transport
//!
//! MCP stdio transport: newline-delimited JSON-RPC 2.0. One JSON object per line
//! on stdin; the response (if any) is written as one JSON object per line on
//! stdout. Notifications (requests without an `id`) produce no response.
//!
//! ## Discovery
//!
//! A client only knows what the handshake tells it, so the whole interface is
//! published: `initialize` returns server `instructions` (session lifecycle,
//! index/offset/unit conventions, error convention), and `tools/list` gives
//! every tool a `title`, behavioural `annotations`, a fully described
//! `inputSchema` and an `outputSchema` — with `apply_commands.commands` and
//! `query.queries` expanded into a `oneOf` over every `HeadlessCommand` /
//! `HeadlessQuery` variant rather than an opaque object. Successful calls
//! return that result as `structuredContent` beside the text block.
//!
//! ## Domain tools
//!
//! The built-in tools know DOCX and stop there. Anything specific to a
//! consumer's documents — their clause library, their review checks, their
//! house style — belongs in the consumer's own binary: depend on this crate,
//! [`server::Server::register_tool`] the tools they need, and [`serve_stdio`].
//! Injected tools are published, resolved and undone exactly like built-in
//! ones. See [`extension`] and `examples/domain_tools.rs`.
//!
//! The protocol/dispatch logic lives in [`server::Server`] so it can be unit
//! tested by feeding raw JSON-RPC strings; the binary (`src/main.rs`) only wires
//! it to stdin/stdout. See `examples/roundtrip_fidelity.rs` for a runnable
//! demonstration that edits a document with tracked changes and proves they
//! survive the export → reopen cycle.

/// The command and query vocabulary injected tools work in, re-exported so a
/// consumer needs one dependency rather than three.
pub use jade_docx_document_core;
pub use jade_docx_headless_api;

pub mod extension;
pub mod file_access;
pub mod server;

/// Serves `server` over MCP's stdio transport until stdin closes.
///
/// This is the whole of a consumer's `main`: build a [`server::Server`],
/// register the tools your documents need, hand it here.
/// Dates the revisions this thread records by the machine's local wall clock,
/// as Word does, when a caller starts change tracking without a `date`.
/// [`serve_stdio`] calls it; an embedder serving some other way calls it on
/// the thread that handles requests.
pub fn use_local_revision_clock() {
    jade_docx_document_core::revision_clock::set_revision_clock(Some(local_now));
}

fn local_now() -> jade_docx_document_core::revision_clock::HostTime {
    use chrono::{Datelike, Offset, Timelike};
    let now = chrono::Local::now();
    jade_docx_document_core::revision_clock::HostTime {
        local: jade_docx_document_core::field_datetime::FieldDateTime {
            year: now.year(),
            month: now.month(),
            day: now.day(),
            hour: now.hour(),
            minute: now.minute(),
            second: now.second(),
            weekday: now.weekday().num_days_from_sunday(),
        },
        utc_offset_seconds: i64::from(now.offset().fix().local_minus_utc()),
    }
}

pub fn serve_stdio(server: &mut server::Server) {
    use std::io::{self, BufRead, Write};

    use_local_revision_clock();

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
