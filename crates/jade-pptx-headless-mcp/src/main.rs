//! Binary entry point: the built-in presentation tools over stdio.

use jade_pptx_headless_mcp::{serve_stdio, server::Server};

fn main() {
    let mut server = Server::new();
    serve_stdio(&mut server);
}
