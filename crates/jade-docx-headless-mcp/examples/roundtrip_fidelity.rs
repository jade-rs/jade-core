//! Fidelity round-trip demo.
//!
//! Run from the repo root:
//!
//! ```text
//! cargo run -p jade-docx-headless-mcp --example roundtrip_fidelity
//! ```
//!
//! It drives the MCP server's tool path to edit documents that contain Word
//! **tracked changes** and **comments**, exports each one, reopens the exported
//! bytes, and proves the revision marks and comments survived an agent edit +
//! full serialize/reparse cycle.
//!
//! Why this is the differentiator: agent DOCX tooling built on python-docx
//! (which re-serializes the whole document) or on markdown→pandoc round-trips
//! silently drops revision marks, comments, and fields. This engine preserves
//! the raw XML of unedited parts, so an agent can change one paragraph without
//! destroying the rest of the document.

use jade_docx_headless_mcp::file_access::FileAccess;
use jade_docx_headless_mcp::server::Server;
use serde_json::{json, Value};

const REVIEW_DOCX: &str = "public/fixtures/review.docx";
const COMMENT_DOCX: &str = "public/fixtures/comment.docx";

/// Invoke a tool through the real JSON-RPC `tools/call` path and return the
/// decoded JSON payload, panicking if the tool reported an error.
fn call(server: &mut Server, id: u64, name: &str, arguments: Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
    .to_string();
    let response: Value =
        serde_json::from_str(&server.handle_message(&request).expect("a response")).unwrap();
    let result = &response["result"];
    assert_ne!(
        result["isError"],
        json!(true),
        "tool error: {}",
        result["content"][0]["text"]
    );
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn open(server: &mut Server, id: u64, path: &str) -> String {
    call(server, id, "open_docx", json!({ "path": path }))["session_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn apply(server: &mut Server, id: u64, session: &str, command: Value) {
    let applied = call(
        server,
        id,
        "apply_commands",
        json!({ "session_id": session, "commands": [command] }),
    );
    assert_eq!(applied["applied"], json!(1));
}

/// Export a session to bytes and reopen those bytes as a fresh session — a full
/// serialize → reparse cycle, like saving the file and reloading it.
fn export_and_reopen(server: &mut Server, id: u64, session: &str) -> String {
    let exported = call(server, id, "export_docx", json!({ "session_id": session }));
    println!("  exported {} bytes, reopening…", exported["bytes"]);
    let bytes_base64 = exported["bytes_base64"].as_str().unwrap().to_string();
    call(
        server,
        id + 1,
        "open_docx",
        json!({ "bytes_base64": bytes_base64 }),
    )["session_id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Human-readable one-liner per tracked change in a session.
fn revisions(server: &mut Server, id: u64, session: &str) -> Vec<String> {
    let out = call(
        server,
        id,
        "query",
        json!({
            "session_id": session,
            "queries": [{ "type": "revisions", "start_revision": 0, "max_revisions": 50 }],
        }),
    );
    out["results"][0]["revisions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|r| {
            format!(
                "{} {:?} by {}",
                r["kind"].as_str().unwrap_or("?"),
                r["text"].as_str().unwrap_or(""),
                r["author"].as_str().unwrap_or("?"),
            )
        })
        .collect()
}

/// Human-readable one-liner per comment in a session.
fn comments(server: &mut Server, id: u64, session: &str) -> Vec<String> {
    let out = call(
        server,
        id,
        "query",
        json!({
            "session_id": session,
            "queries": [{ "type": "comments", "start_comment": 0, "max_comments": 50 }],
        }),
    );
    out["results"][0]["comments"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| {
            format!(
                "{:?} by {} (resolved={})",
                c["text"].as_str().unwrap_or(""),
                c["author"].as_str().unwrap_or("?"),
                c["resolved"].as_bool().unwrap_or(false),
            )
        })
        .collect()
}

fn main() {
    // The fixtures live in the repository, not necessarily under the
    // directory this example is run from.
    let mut server = Server::new().with_file_access(FileAccess::new([concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../.."
    )]));
    demo_tracked_changes(&mut server);
    println!();
    demo_comments(&mut server);
}

fn demo_tracked_changes(server: &mut Server) {
    println!("# Tracked changes — {REVIEW_DOCX}\n");
    let session = open(server, 1, REVIEW_DOCX);
    let before = revisions(server, 2, &session);
    println!("BEFORE — tracked changes:");
    for rev in &before {
        println!("  - {rev}");
    }

    // Edit a *different* paragraph (1) so paragraph 0's tracked changes are
    // left entirely untouched.
    apply(
        server,
        3,
        &session,
        json!({ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "Added by the agent." }),
    );

    let session2 = export_and_reopen(server, 4, &session);
    let after = revisions(server, 6, &session2);
    println!("AFTER export → reopen — tracked changes still present:");
    for rev in &after {
        println!("  - {rev}");
    }

    let para1 = call(
        server,
        7,
        "query",
        json!({ "session_id": session2, "queries": [{ "type": "paragraph_text", "paragraph": 1 }] }),
    )["results"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    assert_eq!(before, after, "tracked changes must survive the round-trip");
    assert!(
        para1.contains("Added by the agent."),
        "edit must be present"
    );
    println!(
        "✅ {} tracked change(s) preserved; agent edit landed in paragraph 1.",
        after.len()
    );
}

fn demo_comments(server: &mut Server) {
    println!("# Comments — {COMMENT_DOCX}\n");
    let session = open(server, 10, COMMENT_DOCX);
    let before = comments(server, 11, &session);
    println!("BEFORE — comments:");
    for comment in &before {
        println!("  - {comment}");
    }

    // This document has a single paragraph that the comments anchor into; the
    // edit shifts those anchors but the comment definitions live in
    // comments.xml and must come through untouched.
    apply(
        server,
        12,
        &session,
        json!({ "type": "insert_text", "paragraph": 0, "offset": 0, "text": "[edited] " }),
    );

    let session2 = export_and_reopen(server, 13, &session);
    let after = comments(server, 15, &session2);
    println!("AFTER export → reopen — comments still present:");
    for comment in &after {
        println!("  - {comment}");
    }

    let para0 = call(
        server,
        16,
        "query",
        json!({ "session_id": session2, "queries": [{ "type": "paragraph_text", "paragraph": 0 }] }),
    )["results"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    assert_eq!(before, after, "comments must survive the round-trip");
    assert!(para0.contains("[edited]"), "edit must be present");
    println!(
        "✅ {} comment(s) preserved through an edit in the commented paragraph.",
        after.len()
    );
}
