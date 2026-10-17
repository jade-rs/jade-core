//! Binary entry point for the stock server: the built-in DOCX tools and
//! nothing else.
//!
//! A consumer that needs domain tools writes their own `main` instead —
//! [`jade_docx_headless_mcp::server::Server::register_tool`] injects them, and
//! [`jade_docx_headless_mcp::serve_stdio`] is this same three lines. See
//! `examples/domain_tools.rs`.

use jade_docx_headless_mcp::{serve_stdio, server::Server};

fn main() {
    let mut server = Server::new();
    serve_stdio(&mut server);
}
