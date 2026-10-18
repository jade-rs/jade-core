//! The server as a client sees it: raw JSON-RPC in, raw JSON-RPC out.

use base64::Engine;
use jade_xlsx_headless_mcp::server::Server;
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures"
    ))
    .join(name)
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

/// The part of a tool result an agent should read.
fn structured(response: &Value) -> &Value {
    &response["result"]["structuredContent"]
}

fn open(server: &mut Server, name: &str) -> String {
    let response = call(
        server,
        "open_workbook",
        json!({ "path": fixture_path(name).to_string_lossy() }),
    );
    structured(&response)["session_id"]
        .as_str()
        .expect("session")
        .to_string()
}

#[test]
fn open_rejects_a_non_string_source_instead_of_ignoring_it() {
    let bytes = std::fs::read(fixture_path("sample-workbook.xlsx")).expect("fixture");
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut server = Server::new();

    let response = call(
        &mut server,
        "open_workbook",
        json!({ "path": 7, "bytes_base64": encoded }),
    );

    assert_eq!(response["error"]["code"], -32602, "{response}");
    assert!(response["error"]["message"]
        .as_str()
        .expect("message")
        .contains("path"));
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
        "jade-xlsx-headless-mcp"
    );
    // A client only knows what the handshake tells it, so the caveat that
    // matters most has to be in there.
    let instructions = response["result"]["instructions"]
        .as_str()
        .expect("instructions");
    assert!(instructions.contains("recalculate"));
    assert!(instructions.contains("session_id"));
}

#[test]
fn ping_confirms_the_headless_server_is_alive() {
    let mut server = Server::new();
    let response = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }),
    );

    assert_eq!(response["id"], 7);
    assert_eq!(response["result"], json!({}));
}

#[test]
fn every_tool_is_described_well_enough_to_call() {
    let mut server = Server::new();
    let response = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    let tools = response["result"]["tools"].as_array().expect("tools");

    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("name"))
        .collect();
    assert!(names.contains(&"open_workbook"));
    assert!(names.contains(&"read_cell"));
    assert!(names.contains(&"read_cells"));
    assert!(names.contains(&"set_cell"));
    assert!(names.contains(&"set_cells"));
    assert!(names.contains(&"set_workbook_cells"));
    assert!(names.contains(&"export_workbook"));

    for tool in tools {
        let name = tool["name"].as_str().expect("name");
        assert!(
            !tool["description"].as_str().unwrap_or("").is_empty(),
            "{name} has no description"
        );
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert_eq!(
            tool["inputSchema"]["additionalProperties"], false,
            "{name} must advertise strict arguments"
        );
    }

    // The one tool that changes something has to say so.
    let set = tools
        .iter()
        .find(|tool| tool["name"] == "set_cell")
        .expect("set_cell");
    assert_eq!(set["annotations"]["readOnlyHint"], false);
    assert_eq!(set["inputSchema"]["additionalProperties"], false);
    for name in ["set_cells", "set_workbook_cells"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("batch write tool");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(
            tool["inputSchema"]["properties"]["cells"]["items"]["additionalProperties"],
            false
        );
    }
    let incompatible_modes = json!({
        "required": ["as_text", "as_formula"],
        "properties": {
            "as_text": { "const": true },
            "as_formula": { "const": true }
        }
    });
    assert_eq!(set["inputSchema"]["not"], incompatible_modes);
    for name in ["set_cells", "set_workbook_cells"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .expect("batch write tool");
        assert_eq!(
            tool["inputSchema"]["properties"]["cells"]["items"]["not"],
            incompatible_modes
        );
    }
    assert!(set["description"]
        .as_str()
        .expect("description")
        .contains("recalculates"));

    let read = tools
        .iter()
        .find(|tool| tool["name"] == "read_cell")
        .expect("read_cell");
    assert!(read["description"]
        .as_str()
        .expect("description")
        .contains("hyperlink"));
    assert!(read["description"]
        .as_str()
        .expect("description")
        .contains("comment"));
    assert!(read["description"]
        .as_str()
        .expect("description")
        .contains("input validation"));

    let read_cells = tools
        .iter()
        .find(|tool| tool["name"] == "read_cells")
        .expect("read_cells");
    assert_eq!(
        read_cells["inputSchema"]["properties"]["cells"]["maxItems"],
        1_000
    );
    assert_eq!(
        read_cells["inputSchema"]["properties"]["cells"]["items"]["additionalProperties"],
        false
    );

    let summary = tools
        .iter()
        .find(|tool| tool["name"] == "summary")
        .expect("summary");
    assert!(summary["description"]
        .as_str()
        .expect("description")
        .contains("defined names"));
    assert!(summary["description"]
        .as_str()
        .expect("description")
        .contains("frozen panes"));
    assert!(summary["description"]
        .as_str()
        .expect("description")
        .contains("visibility"));

    let open = tools
        .iter()
        .find(|tool| tool["name"] == "open_workbook")
        .expect("open_workbook");
    assert_eq!(
        open["inputSchema"]["oneOf"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(open["inputSchema"]["oneOf"][0]["required"], json!(["path"]));
    assert_eq!(
        open["inputSchema"]["oneOf"][1]["required"],
        json!(["bytes_base64"])
    );
}

#[test]
fn a_workbook_is_opened_once_and_read_through_a_session() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let cell = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B9" }),
    );
    let cell = structured(&cell);
    assert_eq!(cell["number"], 45000.0);
    // Only the display text says it is a date.
    assert_eq!(cell["display"], "2023/03/15");
}

#[test]
fn cells_across_sheets_are_read_in_one_strict_call() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let response = call(
        &mut server,
        "read_cells",
        json!({
            "session_id": session,
            "cells": [
                { "sheet": "売上サマリ", "reference": "D4" },
                { "sheet": "Notes", "reference": "A1" }
            ]
        }),
    );
    let cells = structured(&response)["cells"].as_array().expect("cells");
    assert_eq!(cells.len(), 2, "{response}");
    assert_eq!(cells[0]["reference"], "D4");
    assert_eq!(cells[0]["formula"], "SUM(B4:C4)");
    assert_eq!(cells[1]["reference"], "A1");
    assert_eq!(cells[1]["display"], "備考");

    let unknown = call(
        &mut server,
        "read_cells",
        json!({
            "session_id": session,
            "cells": [{ "sheet": "Notes", "reference": "A1", "extra": true }]
        }),
    );
    assert_eq!(unknown["error"]["code"], -32602, "{unknown}");

    let too_many = (0..1_001)
        .map(|_| json!({ "sheet": "Notes", "reference": "A1" }))
        .collect::<Vec<_>>();
    let oversized = call(
        &mut server,
        "read_cells",
        json!({ "session_id": session, "cells": too_many }),
    );
    assert_eq!(oversized["error"]["code"], -32602, "{oversized}");
}

#[test]
fn a_sheet_can_be_read_whole() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let response = call(
        &mut server,
        "read_sheet",
        json!({ "session_id": session, "sheet": "0" }),
    );
    let sheet = structured(&response);
    assert_eq!(sheet["reference"], "A1:D10");
    let rows = sheet["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 10);
    assert_eq!(rows[3][0], "横浜");
}

#[test]
fn a_far_offset_sheet_reads_only_its_actual_used_rectangle() {
    let mut server = Server::new();
    let session = open(&mut server, "sparse-offset-workbook.xlsx");

    let response = call(
        &mut server,
        "read_sheet",
        json!({ "session_id": session, "sheet": "Sparse" }),
    );
    let sheet = structured(&response);
    assert_eq!(sheet["reference"], "XFD1048576:XFD1048576");
    assert_eq!(sheet["rows"], json!([["far away"]]));
}

#[test]
fn a_write_lands_and_is_visible_to_the_next_read() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let written = call(
        &mut server,
        "set_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B4", "value": 777 }),
    );
    // The tool answers with the cell as it now reads, format included.
    assert_eq!(structured(&written)["display"], "¥777");

    let read = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B4" }),
    );
    assert_eq!(structured(&read)["number"], 777.0);
}

#[test]
fn a_formula_is_an_explicit_write_mode() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let written = call(
        &mut server,
        "set_cell",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "reference": "A20",
            "value": "=SUM(B4:C4)",
            "as_formula": true
        }),
    );
    assert_eq!(structured(&written)["formula"], "SUM(B4:C4)");
    // The session recalculates, so the formula answers with its result.
    assert_eq!(structured(&written)["kind"], "number");
    assert_eq!(structured(&written)["formula_state"], "cached");

    let ambiguous = call(
        &mut server,
        "set_cell",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "reference": "A20",
            "value": "=1+1",
            "as_text": true,
            "as_formula": true
        }),
    );
    assert_eq!(ambiguous["error"]["code"], -32602, "{ambiguous}");

    let non_string = call(
        &mut server,
        "set_cell",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "reference": "A20",
            "value": 12,
            "as_formula": true
        }),
    );
    assert_eq!(non_string["error"]["code"], -32602, "{non_string}");
}

#[test]
fn write_tools_reject_unknown_fields_before_changing_the_workbook() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let misspelled = call(
        &mut server,
        "set_cell",
        json!({
            "session_id": session,
            "sheet": "Notes",
            "reference": "A20",
            "value": "=1+1",
            "as_formlua": true
        }),
    );
    assert_eq!(misspelled["error"]["code"], -32602, "{misspelled}");
    assert!(misspelled["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("as_formlua"));

    let nested = call(
        &mut server,
        "set_cells",
        json!({
            "session_id": session,
            "sheet": "Notes",
            "cells": [{ "reference": "A20", "value": 12, "as_txt": true }]
        }),
    );
    assert_eq!(nested["error"]["code"], -32602, "{nested}");

    let cell = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "Notes", "reference": "A20" }),
    );
    assert_eq!(structured(&cell)["kind"], "empty");
}

#[test]
fn read_tools_reject_unknown_fields_instead_of_changing_the_request() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let misspelled = call(
        &mut server,
        "find",
        json!({
            "session_id": session,
            "needle": "横浜",
            "case_sensitve": true
        }),
    );
    assert_eq!(misspelled["error"]["code"], -32602, "{misspelled}");
    assert!(misspelled["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("case_sensitve"));

    let extra = call(
        &mut server,
        "read_cell",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "reference": "A1",
            "format": "raw"
        }),
    );
    assert_eq!(extra["error"]["code"], -32602, "{extra}");
}

#[test]
fn json_types_decide_what_a_written_value_is() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let set = |server: &mut Server, value: Value, as_text: bool| {
        let response = call(
            server,
            "set_cell",
            json!({
                "session_id": session,
                "sheet": "売上サマリ",
                "reference": "A20",
                "value": value,
                "as_text": as_text
            }),
        );
        structured(&response).clone()
    };

    assert_eq!(set(&mut server, json!(12.5), false)["kind"], "number");
    assert_eq!(set(&mut server, json!(true), false)["kind"], "bool");
    assert_eq!(set(&mut server, json!(null), false)["kind"], "empty");
    // A string that looks like a number is one, unless the caller says not.
    assert_eq!(set(&mut server, json!("007"), false)["kind"], "number");
    let as_text = set(&mut server, json!("007"), true);
    assert_eq!(as_text["kind"], "text");
    assert_eq!(as_text["display"], "007");
}

#[test]
fn set_cell_rejects_structured_json_instead_of_stringifying_it() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    for value in [json!([1, 2]), json!({ "amount": 12 })] {
        let response = call(
            &mut server,
            "set_cell",
            json!({
                "session_id": session,
                "sheet": "売上サマリ",
                "reference": "A20",
                "value": value
            }),
        );
        assert_eq!(response["error"]["code"], -32602);
        assert!(response["error"]["message"]
            .as_str()
            .expect("message")
            .contains("number, string, boolean, or null"));
    }

    let cell = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "A20" }),
    );
    assert_eq!(structured(&cell)["kind"], "empty");
}

#[test]
fn optional_boolean_arguments_reject_the_wrong_json_type() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let set = call(
        &mut server,
        "set_cell",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "reference": "A20",
            "value": "007",
            "as_text": "true"
        }),
    );
    assert_eq!(set["error"]["code"], -32602, "{set}");

    let find = call(
        &mut server,
        "find",
        json!({ "session_id": session, "needle": "横浜", "case_sensitive": 1 }),
    );
    assert_eq!(find["error"]["code"], -32602, "{find}");

    let cell = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "A20" }),
    );
    assert_eq!(structured(&cell)["kind"], "empty");
}

#[test]
fn several_cells_can_be_set_atomically_in_one_call() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let response = call(
        &mut server,
        "set_cells",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "cells": [
                { "reference": "B4", "value": 10 },
                { "reference": "C4", "value": "20" },
                { "reference": "A20", "value": "007", "as_text": true }
            ]
        }),
    );
    let cells = structured(&response)["cells"].as_array().expect("cells");
    assert_eq!(cells[0]["number"], 10.0);
    assert_eq!(cells[1]["number"], 20.0);
    assert_eq!(cells[2]["text"], "007");

    let before = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B5" }),
    );
    let failed = call(
        &mut server,
        "set_cells",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "cells": [
                { "reference": "B5", "value": 999 },
                { "reference": "XFE1", "value": 1 }
            ]
        }),
    );
    assert!(failed.get("error").is_some());
    let after = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B5" }),
    );
    assert_eq!(structured(&after), structured(&before));
}

#[test]
fn several_sheets_can_be_set_atomically_in_one_call() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let response = call(
        &mut server,
        "set_workbook_cells",
        json!({
            "session_id": session,
            "cells": [
                { "sheet": "売上サマリ", "reference": "B4", "value": 10 },
                { "sheet": "Notes", "reference": "A20", "value": "updated", "as_text": true }
            ]
        }),
    );
    let cells = structured(&response)["cells"].as_array().expect("cells");
    assert_eq!(cells[0]["number"], 10.0);
    assert_eq!(cells[1]["text"], "updated");

    let before = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B5" }),
    );
    let failed = call(
        &mut server,
        "set_workbook_cells",
        json!({
            "session_id": session,
            "cells": [
                { "sheet": "売上サマリ", "reference": "B5", "value": 999 },
                { "sheet": "missing", "reference": "A1", "value": 1 }
            ]
        }),
    );
    assert_eq!(failed["error"]["code"], -32602, "{failed}");
    let after = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B5" }),
    );
    assert_eq!(structured(&after), structured(&before));
}

#[test]
fn read_cell_exposes_formula_cache_freshness_after_a_write() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let cached = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "D4" }),
    );
    assert_eq!(structured(&cached)["formula_state"], "cached");

    call(
        &mut server,
        "set_cell",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "reference": "B4",
            "value": 1
        }),
    );
    let current = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "D4" }),
    );
    // The write recalculated its dependents, so the cache stays current and
    // the number moved with the edit.
    assert_eq!(structured(&current)["formula_state"], "cached");
    assert_eq!(
        structured(&current)["number"].as_f64().expect("a number"),
        1.0 + 15_230_000.0
    );
}

#[test]
fn duplicate_batch_references_are_invalid_params_and_do_not_write() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");
    let before = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B4" }),
    );

    let failed = call(
        &mut server,
        "set_cells",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "cells": [
                { "reference": "B4", "value": 1 },
                { "reference": "B4", "value": 2 }
            ]
        }),
    );
    assert_eq!(failed["error"]["code"], -32602, "{failed}");

    let after = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B4" }),
    );
    assert_eq!(structured(&after), structured(&before));
}

#[test]
fn an_exported_workbook_can_be_reopened() {
    let mut server = Server::new();
    let bytes = std::fs::read(fixture_path("sample-workbook.xlsx")).expect("fixture");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);

    let opened = call(
        &mut server,
        "open_workbook",
        json!({ "bytes_base64": encoded }),
    );
    let session = structured(&opened)["session_id"].as_str().expect("session");
    let session = session.to_string();

    call(
        &mut server,
        "set_cell",
        json!({ "session_id": session, "sheet": "0", "reference": "A20", "value": "追記" }),
    );
    call(
        &mut server,
        "set_cell",
        json!({ "session_id": session, "sheet": "0", "reference": "B4", "value": 1 }),
    );
    let exported = call(
        &mut server,
        "export_workbook",
        json!({ "session_id": session }),
    );
    // Opened from bytes with nowhere to write, so it comes back as bytes
    // rather than the server picking a file.
    let returned = structured(&exported)["bytes_base64"]
        .as_str()
        .expect("bytes_base64");

    let reopened = call(
        &mut server,
        "open_workbook",
        json!({ "bytes_base64": returned }),
    );
    let second = structured(&reopened)["session_id"]
        .as_str()
        .expect("session")
        .to_string();
    let cell = call(
        &mut server,
        "read_cell",
        json!({ "session_id": second, "sheet": "0", "reference": "A20" }),
    );
    assert_eq!(structured(&cell)["display"], "追記");
    let formula = call(
        &mut server,
        "read_cell",
        json!({ "session_id": second, "sheet": "0", "reference": "D4" }),
    );
    // The write recalculated the workbook before it was exported.
    assert_eq!(structured(&formula)["formula_state"], "cached");
}

#[test]
fn export_writes_over_the_file_it_was_opened_from() {
    let directory = std::env::temp_dir().join("jade-xlsx-mcp-export");
    std::fs::create_dir_all(&directory).expect("temp dir");
    let path = directory.join("book.xlsx");
    std::fs::copy(fixture_path("sample-workbook.xlsx"), &path).expect("copy");

    let mut server = Server::new();
    let opened = call(
        &mut server,
        "open_workbook",
        json!({ "path": path.to_string_lossy() }),
    );
    let session = structured(&opened)["session_id"]
        .as_str()
        .expect("session")
        .to_string();

    call(
        &mut server,
        "set_cell",
        json!({ "session_id": session, "sheet": "0", "reference": "A20", "value": 5 }),
    );
    let exported = call(
        &mut server,
        "export_workbook",
        json!({ "session_id": session }),
    );
    assert_eq!(
        structured(&exported)["path"].as_str(),
        Some(path.to_string_lossy().as_ref())
    );

    let written = std::fs::read(&path).expect("written");
    let workbook = jade_xlsx_headless_editor::HeadlessWorkbook::open(&written).expect("reopen");
    assert_eq!(workbook.cell("0", "A20").expect("A20").number, Some(5.0));
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn save_as_becomes_the_session_destination_for_later_exports() {
    let bytes = std::fs::read(fixture_path("sample-workbook.xlsx")).expect("fixture");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let directory = std::env::temp_dir().join("jade-xlsx-mcp-save-as");
    std::fs::create_dir_all(&directory).expect("temp dir");
    let path = directory.join("saved.xlsx");

    let mut server = Server::new();
    let opened = call(
        &mut server,
        "open_workbook",
        json!({ "bytes_base64": encoded }),
    );
    let session = structured(&opened)["session_id"]
        .as_str()
        .expect("session")
        .to_string();

    call(
        &mut server,
        "export_workbook",
        json!({ "session_id": session, "path": path.to_string_lossy() }),
    );
    call(
        &mut server,
        "set_cell",
        json!({ "session_id": session, "sheet": "0", "reference": "A20", "value": 9 }),
    );
    let second = call(
        &mut server,
        "export_workbook",
        json!({ "session_id": session }),
    );

    assert_eq!(
        structured(&second)["path"].as_str(),
        Some(path.to_string_lossy().as_ref())
    );
    let saved = std::fs::read(&path).expect("saved again");
    let workbook = jade_xlsx_headless_editor::HeadlessWorkbook::open(&saved).expect("reopen");
    assert_eq!(workbook.cell("0", "A20").expect("A20").number, Some(9.0));
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn export_rejects_a_non_string_path_instead_of_ignoring_it() {
    let bytes = std::fs::read(fixture_path("sample-workbook.xlsx")).expect("fixture");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mut server = Server::new();
    let opened = call(
        &mut server,
        "open_workbook",
        json!({ "bytes_base64": encoded }),
    );
    let session = structured(&opened)["session_id"].as_str().expect("session");

    let response = call(
        &mut server,
        "export_workbook",
        json!({ "session_id": session, "path": 7 }),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");
    assert!(response["error"]["message"]
        .as_str()
        .expect("message")
        .contains("path"));
}

#[test]
fn sessions_are_listed_and_can_be_closed() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let listed = call(&mut server, "list_sessions", json!({}));
    let sessions = structured(&listed)["sessions"]
        .as_array()
        .expect("sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["session_id"], session);

    call(
        &mut server,
        "close_workbook",
        json!({ "session_id": session }),
    );
    let listed = call(&mut server, "list_sessions", json!({}));
    assert!(structured(&listed)["sessions"]
        .as_array()
        .expect("sessions")
        .is_empty());
}

#[test]
fn a_call_against_a_session_that_is_not_there_says_so() {
    let mut server = Server::new();
    let response = call(
        &mut server,
        "read_cell",
        json!({ "session_id": "wb-99", "sheet": "0", "reference": "A1" }),
    );
    let message = response["error"]["message"].as_str().expect("message");
    assert!(message.contains("no session"), "{message}");
    assert!(message.contains("open_workbook first"), "{message}");
}

#[test]
fn bad_sheet_and_cell_references_are_invalid_params() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    for arguments in [
        json!({ "session_id": session, "sheet": "missing", "reference": "A1" }),
        json!({ "session_id": session, "sheet": "0", "reference": "XFE1" }),
    ] {
        let response = call(&mut server, "read_cell", arguments);
        assert_eq!(response["error"]["code"], -32602, "{response}");
    }
}

#[test]
fn find_can_discover_formula_source() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let response = call(
        &mut server,
        "find",
        json!({ "session_id": session, "needle": "SUM(B4:C4)" }),
    );
    let hits = structured(&response).as_array().expect("find hits");
    assert_eq!(hits.len(), 1, "{response}");
    assert_eq!(hits[0]["reference"], "D4");
    assert_eq!(hits[0]["formula"], "SUM(B4:C4)");
}

#[test]
fn xml_forbidden_cell_text_round_trips_through_the_x_escape() {
    // XML 1.0 forbids a NUL, but SpreadsheetML's `_xHHHH_` escape carries it,
    // the same way Excel stores control characters pasted into a cell.
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    call(
        &mut server,
        "set_cell",
        json!({
            "session_id": session,
            "sheet": "売上サマリ",
            "reference": "A20",
            "value": "before\u{0}after",
            "as_text": true
        }),
    );

    let cell = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "A20" }),
    );
    assert_eq!(structured(&cell)["text"], "before\u{0}after");
}

#[test]
fn a_notification_gets_no_reply_and_a_bad_line_does() {
    let mut server = Server::new();
    assert!(server
        .handle_message(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string()
        )
        .is_none());

    let response = server.handle_message("{ not json").expect("a response");
    assert!(response.contains("parse error"));
}

#[test]
fn open_needs_exactly_one_source() {
    let mut server = Server::new();
    let neither = call(&mut server, "open_workbook", json!({}));
    assert!(neither["error"]["message"]
        .as_str()
        .expect("message")
        .contains("exactly one"));

    let both = call(
        &mut server,
        "open_workbook",
        json!({ "path": "a.xlsx", "bytes_base64": "AA==" }),
    );
    assert!(both["error"]["message"]
        .as_str()
        .expect("message")
        .contains("exactly one"));
}

#[test]
fn recalculate_reports_what_it_did() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let summary = call(&mut server, "recalculate", json!({ "session_id": session }));
    let summary = structured(&summary);
    // A freshly opened workbook's caches are current, so nothing needs
    // refreshing and nothing is skipped.
    assert_eq!(summary["refreshed"], 0, "{summary}");
    assert_eq!(summary["skipped"].as_array().expect("skipped").len(), 0);
    assert!(summary["unchanged"].as_u64().expect("unchanged") > 0);
}

#[test]
fn undo_and_redo_step_the_session_through_its_writes() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    call(
        &mut server,
        "set_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B4", "value": 999 }),
    );
    let undone = call(&mut server, "undo", json!({ "session_id": session }));
    assert_eq!(structured(&undone)["undone"], true);
    let read = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "D4" }),
    );
    // The dependent is back to the file's number, not left at the stale one.
    assert_eq!(structured(&read)["display"], "¥28,080,000");

    let redone = call(&mut server, "redo", json!({ "session_id": session }));
    assert_eq!(structured(&redone)["redone"], true);
    let read = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B4" }),
    );
    assert_eq!(structured(&read)["display"], "¥999");

    // Nothing left to redo: the tool says so rather than erroring.
    let again = call(&mut server, "redo", json!({ "session_id": session }));
    assert_eq!(structured(&again)["redone"], false);
}

#[test]
fn fill_range_continues_a_run_and_reports_what_it_wrote() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");
    call(
        &mut server,
        "set_cells",
        json!({ "session_id": session, "sheet": "売上サマリ", "cells": [
            { "reference": "F1", "value": 10 },
            { "reference": "F2", "value": 20 },
            { "reference": "G1", "value": "Item 1" }
        ] }),
    );

    let filled = call(
        &mut server,
        "fill_range",
        json!({ "session_id": session, "sheet": "売上サマリ", "source": "F1:G2", "target": "F1:G4" }),
    );
    let summary = structured(&filled);
    assert_eq!(summary["source"], "F1:G2");
    assert_eq!(summary["target"], "F1:G4");
    assert_eq!(summary["cells"], json!(["F3", "G3", "F4", "G4"]));

    let read = call(
        &mut server,
        "read_range",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "F3:G4" }),
    );
    let rows = &structured(&read)["rows"];
    assert_eq!(rows[0][0]["display"], "30");
    assert_eq!(rows[1][0]["display"], "40");
    // G1:G2 is "Item 1" over a blank: a pattern, repeated.
    assert_eq!(rows[0][1]["display"], "Item 1");
    assert_eq!(rows[1][1]["display"], "");

    // The fill is one undo step.
    let undone = call(&mut server, "undo", json!({ "session_id": session }));
    assert_eq!(structured(&undone)["undone"], true);
    let read = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "F3" }),
    );
    assert_eq!(structured(&read)["kind"], "empty");

    // A target that is not the source grown along one axis is the caller's
    // mistake, not the server's.
    let refused = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
            "name": "fill_range",
            "arguments": { "session_id": session, "sheet": "売上サマリ", "source": "F1:F2", "target": "G1:G4" }
        } }),
    );
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
}

#[test]
fn copy_format_paints_formats_and_leaves_values_alone() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");
    call(
        &mut server,
        "set_cells",
        json!({ "session_id": session, "sheet": "売上サマリ", "cells": [
            { "reference": "F1", "value": 1 },
            { "reference": "F2", "value": 2 },
            { "reference": "F3", "value": "=F1+F2", "as_formula": true }
        ] }),
    );

    // B4 wears the fixture's yen format.
    let painted = call(
        &mut server,
        "copy_format",
        json!({ "session_id": session, "sheet": "売上サマリ", "source": "B4", "target": "F1:F3" }),
    );
    let summary = structured(&painted);
    assert_eq!(summary["target"], "F1:F3");
    assert_eq!(summary["cells"], json!(["F1", "F2", "F3"]));

    let read = call(
        &mut server,
        "read_range",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "F1:F3" }),
    );
    let rows = &structured(&read)["rows"];
    assert_eq!(rows[0][0]["display"], "¥1");
    assert_eq!(rows[1][0]["display"], "¥2");
    // The formula and its value survive the restyle.
    assert_eq!(rows[2][0]["display"], "¥3");
    assert_eq!(rows[2][0]["formula"], "F1+F2");
}

#[test]
fn set_format_changes_the_look_and_only_the_look() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let formatted = call(
        &mut server,
        "set_format",
        json!({ "session_id": session, "sheet": "売上サマリ", "range": "B4:B5", "bold": true, "number_format": "#,##0" }),
    );
    let summary = structured(&formatted);
    assert_eq!(summary["cells"], json!(["B4", "B5"]));
    assert_eq!(summary["styles_added"], 1);

    let read = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B4" }),
    );
    // The yen sign is gone with the new number format; the value is not.
    assert_eq!(structured(&read)["display"], "12,850,000");
    assert_eq!(structured(&read)["number"], 12_850_000.0);

    // The same look again adds nothing.
    let again = call(
        &mut server,
        "set_format",
        json!({ "session_id": session, "sheet": "売上サマリ", "range": "B6", "bold": true, "number_format": "#,##0" }),
    );
    assert_eq!(structured(&again)["styles_added"], 0);

    // A change that names nothing, and an unknown alignment, are refused.
    for arguments in [
        json!({ "session_id": session, "sheet": "売上サマリ", "range": "B4" }),
        json!({ "session_id": session, "sheet": "売上サマリ", "range": "B4", "horizontal": "middle" }),
    ] {
        let refused = send(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
                "name": "set_format", "arguments": arguments
            } }),
        );
        assert_eq!(refused["error"]["code"], -32602, "{refused}");
    }
}

#[test]
fn set_format_draws_borders_and_read_cell_reports_them() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let formatted = call(
        &mut server,
        "set_format",
        json!({
            "session_id": session, "sheet": "売上サマリ", "range": "F2:G3",
            "border_top": { "style": "thick", "color": "C00000" },
            "border_bottom": { "style": "mediumDashed" },
            "border_diagonal": { "style": "thin", "color": "#00b050" },
            "diagonal_down": true
        }),
    );
    assert_eq!(
        structured(&formatted)["cells"],
        json!(["F2", "G2", "F3", "G3"])
    );

    let read = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "G3" }),
    );
    assert_eq!(
        structured(&read)["border"],
        json!({
            "top": { "style": "thick", "color": "C00000" },
            "bottom": { "style": "mediumDashed" },
            "diagonal": { "style": "thin", "color": "00B050" },
            "diagonal_down": true
        })
    );

    // A style OOXML does not have is refused.
    let refused = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
            "name": "set_format", "arguments": {
                "session_id": session, "sheet": "売上サマリ", "range": "B4",
                "border_left": { "style": "wavy" }
            }
        } }),
    );
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
}

#[test]
fn rows_and_columns_move_through_the_protocol() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");

    let inserted = call(
        &mut server,
        "insert_rows",
        json!({ "session_id": session, "sheet": "売上サマリ", "before_row": 5, "count": 2 }),
    );
    assert_eq!(structured(&inserted)["at"], "5");
    assert_eq!(structured(&inserted)["inserted"], true);
    let read = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "B9" }),
    );
    assert_eq!(structured(&read)["formula"], "SUM(B4:B8)");
    assert_eq!(structured(&read)["display"], "¥28,400,000");

    let deleted = call(
        &mut server,
        "delete_columns",
        json!({ "session_id": session, "sheet": "売上サマリ", "first_column": "B" }),
    );
    assert_eq!(structured(&deleted)["axis"], "columns");
    assert_eq!(structured(&deleted)["count"], 1);
    let read = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "売上サマリ", "reference": "C4" }),
    );
    assert_eq!(structured(&read)["formula"], "SUM(B4:B4)");

    // A row of zero, or a column that is not a letter, is refused.
    for (name, arguments) in [
        (
            "insert_rows",
            json!({ "session_id": session, "sheet": "売上サマリ", "before_row": 0 }),
        ),
        (
            "delete_columns",
            json!({ "session_id": session, "sheet": "売上サマリ", "first_column": "7" }),
        ),
        (
            "insert_rows",
            json!({ "session_id": session, "sheet": "売上サマリ", "before_row": 3, "count": 0 }),
        ),
    ] {
        let refused = send(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
                "name": name, "arguments": arguments
            } }),
        );
        assert_eq!(refused["error"]["code"], -32602, "{refused}");
    }
}

#[test]
fn the_catalogue_offers_the_header_footer_and_shape_text_tools() {
    let mut server = Server::new();
    let response = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    let names: Vec<&str> = response["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"set_header_footer"), "{names:?}");
    assert!(names.contains(&"set_shape_text"), "{names:?}");
}

#[test]
fn the_summary_reports_tables_and_shapes_and_the_tools_change_them() {
    let mut server = Server::new();
    let session = open(&mut server, "objects-workbook.xlsx");

    let summary = call(&mut server, "summary", json!({ "session_id": session }));
    let sheet = &structured(&summary)["sheets"][0];
    assert_eq!(sheet["tables"][0]["name"], "Table1");
    assert_eq!(sheet["tables"][0]["columns"][1], "Qty");
    assert_eq!(sheet["shapes"][0]["name"], "TextBox 1");
    assert_eq!(sheet["shapes"][0]["text"], "Draft");
    assert_eq!(sheet["header_footer"]["odd_header"], "&CConfidential");
    assert_eq!(sheet["page_breaks"]["rows"][0], 4);

    let cell = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": "Data", "reference": "B1" }),
    );
    assert_eq!(structured(&cell)["table"]["column"], "Qty");
    assert_eq!(structured(&cell)["table"]["header"], true);

    let response = call(
        &mut server,
        "set_header_footer",
        json!({ "session_id": session, "sheet": "Data", "odd_footer": "&RPage &P", "first_header": null }),
    );
    let view = structured(&response);
    assert_eq!(view["odd_footer"], "&RPage &P", "{response}");
    assert_eq!(view["odd_header"], "&CConfidential");
    assert!(view.get("first_header").is_none(), "{response}");

    let response = call(
        &mut server,
        "set_header_footer",
        json!({ "session_id": session, "sheet": "Data", "odd_footr": "x" }),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");

    let response = call(
        &mut server,
        "set_shape_text",
        json!({ "session_id": session, "sheet": "Data", "shape": "TextBox 1", "text": "Final" }),
    );
    assert_eq!(structured(&response)["text"], "Final", "{response}");
    assert_eq!(structured(&response)["anchor"], "E2:G4");

    let response = call(
        &mut server,
        "set_shape_text",
        json!({ "session_id": session, "sheet": "Data", "shape": "Chart 2", "text": "Final" }),
    );
    assert_eq!(response["error"]["code"], -32602, "{response}");

    // Typing over a header renames the column.
    let response = call(
        &mut server,
        "set_cell",
        json!({ "session_id": session, "sheet": "Data", "reference": "B1", "value": "Item" }),
    );
    assert_eq!(structured(&response)["display"], "Item2", "{response}");
    assert_eq!(structured(&response)["table"]["column"], "Item2");
}

#[test]
fn notes_are_set_and_deleted_over_the_protocol() {
    let mut server = Server::new();
    let session = open(&mut server, "sample-workbook.xlsx");
    let sheet = call(&mut server, "summary", json!({ "session_id": session }));
    let sheet = structured(&sheet)["sheets"][0]["name"]
        .as_str()
        .expect("name")
        .to_string();

    let response = call(
        &mut server,
        "set_note",
        json!({ "session_id": session, "sheet": sheet, "reference": "B4", "text": "Verify" }),
    );
    assert_eq!(structured(&response)["text"], "Verify", "{response}");
    assert_eq!(structured(&response)["author"], "jade");

    let cell = call(
        &mut server,
        "read_cell",
        json!({ "session_id": session, "sheet": sheet, "reference": "B4" }),
    );
    assert_eq!(structured(&cell)["comment"]["text"], "Verify");

    let response = call(
        &mut server,
        "delete_note",
        json!({ "session_id": session, "sheet": sheet, "reference": "B4" }),
    );
    assert_eq!(structured(&response)["deleted"], true, "{response}");
    let response = call(
        &mut server,
        "delete_note",
        json!({ "session_id": session, "sheet": sheet, "reference": "B4" }),
    );
    assert_eq!(structured(&response)["deleted"], false, "{response}");
}
