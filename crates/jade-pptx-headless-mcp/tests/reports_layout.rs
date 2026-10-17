//! The layout tools as a client sees them: what would be drawn, and what a
//! presenter would notice going wrong.

use std::io::Write;

use jade_pptx_headless_mcp::server::Server;
use serde_json::{json, Value};

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const PRESENTATION: &str = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/></p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;

const SLIDE: &str = r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Tiny box"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="1828800" cy="228600"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></p:spPr>
      <p:txBody><a:bodyPr/><a:p><a:r><a:rPr sz="1800"/><a:t>This sentence is far too long for its little box</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:graphicFrame>
      <p:nvGraphicFramePr><p:cNvPr id="3" name="Table 2"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
      <p:xfrm><a:off x="914400" y="2743200"/><a:ext cx="1828800" cy="384048"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
        <a:tbl>
          <a:tblGrid><a:gridCol w="914400"/><a:gridCol w="914400"/></a:tblGrid>
          <a:tr h="384048">
            <a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>A1</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>
            <a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>B1</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>
          </a:tr>
        </a:tbl>
      </a:graphicData></a:graphic>
    </p:graphicFrame>
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

fn open(server: &mut Server, test: &str) -> String {
    // Tests run in parallel; each writes its own file so none reads a
    // half-written one.
    let path = std::env::temp_dir().join(format!(
        "jade-pptx-mcp-layout-{}-{test}.pptx",
        std::process::id()
    ));
    std::fs::write(&path, deck_bytes()).expect("write deck");
    let response = call(
        server,
        "open_deck",
        json!({ "path": path.to_string_lossy() }),
    );
    response["result"]["structuredContent"]["session_id"]
        .as_str()
        .expect("session id")
        .to_string()
}

#[test]
fn the_catalogue_lists_the_layout_tools() {
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
    assert!(names.contains(&"slide_layout"), "{names:?}");
    assert!(names.contains(&"check_layout"), "{names:?}");
}

#[test]
fn slide_layout_reports_placed_shapes_and_table_cells() {
    let mut server = Server::new();
    let session = open(&mut server, "layout");
    let response = call(
        &mut server,
        "slide_layout",
        json!({ "session_id": session, "slide": 1 }),
    );
    let layout = &response["result"]["structuredContent"];
    assert_eq!(layout["width_px"], 1280.0);
    let shapes = layout["shapes"].as_array().expect("shapes");
    assert_eq!(shapes[0]["name"], "Tiny box");
    assert_eq!(shapes[0]["origin"], "slide");
    assert_eq!(shapes[0]["fill"], "#FF0000");
    assert_eq!(shapes[0]["rect"]["x"], 96.0);
    assert!(shapes[0]["text_overflow_px"].as_f64().expect("overflow") > 20.0);
    assert_eq!(shapes[1]["name"], "Table 2!A1");
    assert_eq!(shapes[1]["lines"], json!(["A1"]));
    assert_eq!(shapes[2]["name"], "Table 2!B1");
}

#[test]
fn check_layout_reports_the_overflow_and_nothing_for_a_missing_slide_argument_error() {
    let mut server = Server::new();
    let session = open(&mut server, "check");
    let response = call(
        &mut server,
        "check_layout",
        json!({ "session_id": session }),
    );
    let issues = response["result"]["structuredContent"]["issues"]
        .as_array()
        .expect("issues");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["shape"], "Tiny box");
    assert_eq!(issues[0]["kind"], "text_overflow");
    assert_eq!(issues[0]["slide"], 1);

    let response = call(
        &mut server,
        "check_layout",
        json!({ "session_id": session, "slide": 2 }),
    );
    assert!(response["error"]["message"]
        .as_str()
        .expect("error")
        .contains("out of range"));
}

#[test]
fn read_slide_carries_table_rows_and_placements() {
    let mut server = Server::new();
    let session = open(&mut server, "read");
    let response = call(
        &mut server,
        "read_slide",
        json!({ "session_id": session, "slide": 1 }),
    );
    let shapes = response["result"]["structuredContent"]["shapes"]
        .as_array()
        .expect("shapes");
    assert_eq!(shapes[0]["rect"]["width"], 192.0);
    assert_eq!(shapes[1]["kind"], "table");
    assert_eq!(shapes[1]["rows"], json!([["A1", "B1"]]));
}

#[test]
fn read_media_returns_a_parts_bytes_base64_encoded() {
    let mut server = Server::new();
    // Build a deck with a media part inline: the fixture deck has none.
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in [
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", PRESENTATION),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE),
            ("ppt/media/image1.jpg", "JPEG"),
        ] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &buffer);
    let response = call(&mut server, "open_deck", json!({ "bytes_base64": encoded }));
    let session = response["result"]["structuredContent"]["session_id"]
        .as_str()
        .expect("session")
        .to_string();
    let response = call(
        &mut server,
        "read_media",
        json!({ "session_id": session, "part": "ppt/media/image1.jpg" }),
    );
    let media = &response["result"]["structuredContent"];
    assert_eq!(media["content_type"], "image/jpeg");
    assert_eq!(media["bytes"], 4);
    assert_eq!(
        media["bytes_base64"],
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"JPEG")
    );
    let response = call(
        &mut server,
        "read_media",
        json!({ "session_id": session, "part": "ppt/media/none.png" }),
    );
    assert!(response["error"]["message"]
        .as_str()
        .expect("error")
        .contains("no part"));
}
