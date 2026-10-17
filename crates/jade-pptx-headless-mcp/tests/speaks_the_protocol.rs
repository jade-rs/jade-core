//! The server as a client sees it: raw JSON-RPC in, raw JSON-RPC out.

use std::io::Write;

use base64::Engine;
use jade_pptx_headless_mcp::server::Server;
use serde_json::{json, Value};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:p><a:r><a:t>Quarterly Review</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#;

fn deck_bytes() -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    buffer
}

fn send(server: &mut Server, request: Value) -> Value {
    let line = server
        .handle_message(&request.to_string())
        .expect("a response");
    serde_json::from_str(&line).expect("json")
}

fn call(server: &mut Server, name: &str, arguments: Value) -> Value {
    send(
        server,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    )
}

fn structured(response: &Value) -> &Value {
    &response["result"]["structuredContent"]
}

fn open(server: &mut Server) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(deck_bytes());
    let response = call(server, "open_deck", json!({ "bytes_base64": encoded }));
    structured(&response)["session_id"]
        .as_str()
        .expect("session")
        .to_string()
}

#[test]
fn the_handshake_says_what_the_server_is() {
    let mut server = Server::new();
    let response = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
    );
    assert_eq!(
        response["result"]["serverInfo"]["name"],
        "jade-pptx-headless-mcp"
    );
    let instructions = response["result"]["instructions"]
        .as_str()
        .expect("instructions");
    assert!(instructions.contains("session_id"));
    assert!(instructions.contains("read"));
}

#[test]
fn the_catalogue_lists_every_tool() {
    let mut server = Server::new();
    let response = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    let names: Vec<&str> = response["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("name"))
        .collect();
    for expected in [
        "open_deck",
        "summary",
        "read_slide",
        "deck_text",
        "find",
        "close_deck",
        "list_sessions",
    ] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }
}

#[test]
fn opening_a_deck_returns_a_session_and_its_summary() {
    let mut server = Server::new();
    let encoded = base64::engine::general_purpose::STANDARD.encode(deck_bytes());
    let response = call(&mut server, "open_deck", json!({ "bytes_base64": encoded }));
    let body = structured(&response);
    assert!(body["session_id"].as_str().is_some(), "{response}");
    assert_eq!(body["summary"]["slide_count"], 1);
    assert_eq!(body["summary"]["slides"][0]["title"], "Quarterly Review");
}

#[test]
fn a_slide_reads_back_through_the_session() {
    let mut server = Server::new();
    let session = open(&mut server);
    let response = call(
        &mut server,
        "read_slide",
        json!({ "session_id": session, "slide": 1 }),
    );
    let body = structured(&response);
    assert_eq!(body["shapes"][0]["name"], "Title 1");
    assert_eq!(body["shapes"][0]["text"], "Quarterly Review");
}

#[test]
fn deck_text_and_find_walk_the_whole_deck() {
    let mut server = Server::new();
    let session = open(&mut server);

    let text = call(&mut server, "deck_text", json!({ "session_id": session }));
    assert_eq!(structured(&text)["text"], "## Slide 1\nQuarterly Review\n");

    let found = call(
        &mut server,
        "find",
        json!({ "session_id": session, "needle": "quarterly" }),
    );
    let matches = structured(&found)["matches"].as_array().expect("matches");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["slide"], 1);
}

#[test]
fn a_closed_session_stops_answering() {
    let mut server = Server::new();
    let session = open(&mut server);
    let closed = call(&mut server, "close_deck", json!({ "session_id": session }));
    assert_eq!(structured(&closed)["closed"], session);

    let after = call(&mut server, "summary", json!({ "session_id": session }));
    assert_eq!(after["error"]["code"], -32602, "{after}");
}

#[test]
fn an_out_of_range_slide_is_an_invalid_param() {
    let mut server = Server::new();
    let session = open(&mut server);
    let response = call(
        &mut server,
        "read_slide",
        json!({ "session_id": session, "slide": 9 }),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");
}
