//! Protocol handling and the tools.
//!
//! Kept separate from the binary so it can be tested by feeding raw JSON-RPC
//! strings, which is the only way to be sure a client sees what it should.

use std::collections::BTreeMap;

use base64::Engine;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{HeadlessError, HeadlessWorkbook};
use serde_json::{json, Value};

/// JSON-RPC error codes this server uses. `-32602` is the protocol's own
/// "invalid params"; `-32000` is the implementation-defined range, used for a
/// request that was well-formed and could not be carried out.
const INVALID_PARAMS: i64 = -32602;
const TOOL_FAILED: i64 = -32000;
const READ_CELLS_LIMIT: usize = 1_000;

const PROTOCOL_VERSION: &str = "2024-11-05";

const INSTRUCTIONS: &str = "\
Open a workbook once with open_workbook and pass the session_id it returns to \
every other tool; close_workbook frees it. Sheets are named or numbered from 0. \
Cells use A1 references and ranges use A1:D10. \
Values come back twice: `number`/`text` is what the file stores and `display` is \
what a spreadsheet shows, which is the only one that reads a date as a date. \
Writes do not recalculate: a cell whose formula depended on what you changed \
keeps its cached value here, and the exported file asks the spreadsheet opening \
it to recalculate. Formula cells carry `formula_state`: `cached`, `stale`, or \
`uncalculated`.";

/// An open workbook and where it came from.
struct Session {
    workbook: HeadlessWorkbook,
    /// The path it was opened from, when it was opened from one — so
    /// `export_workbook` can default to saving over it.
    path: Option<String>,
}

#[derive(Default)]
pub struct Server {
    sessions: BTreeMap<String, Session>,
    next_id: u64,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one JSON-RPC message, returning the response line when the
    /// message was a request rather than a notification.
    pub fn handle_message(&mut self, line: &str) -> Option<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => {
                return Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("parse error: {error}"),
                ))
            }
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(json!({}));

        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "jade-xlsx-headless-mcp", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_catalogue() })),
            "tools/call" => self.call(&params),
            // A notification, and anything else a client sends before it has
            // read the catalogue.
            _ if id.is_none() => return None,
            other => Err((-32601, format!("unknown method: {other}"))),
        };

        // Notifications get nothing back, however they turned out.
        let id = id?;
        Some(match result {
            Ok(result) => success_response(id, result),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    fn call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| (INVALID_PARAMS, "tools/call needs a name".to_string()))?;
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let result = match name {
            "open_workbook" => self.open(&arguments),
            "summary" => self.summary(&arguments),
            "read_cell" => self.read_cell(&arguments),
            "read_cells" => self.read_cells(&arguments),
            "read_range" => self.read_range(&arguments),
            "read_sheet" => self.read_sheet(&arguments),
            "find" => self.find(&arguments),
            "set_cell" => self.set_cell(&arguments),
            "set_cells" => self.set_cells(&arguments),
            "set_workbook_cells" => self.set_workbook_cells(&arguments),
            "recalculate" => self.recalculate(&arguments),
            "fill_range" => self.fill_range(&arguments),
            "copy_format" => self.copy_format(&arguments),
            "set_format" => self.set_format(&arguments),
            "insert_rows" => self.restructure(&arguments, "insert_rows"),
            "delete_rows" => self.restructure(&arguments, "delete_rows"),
            "insert_columns" => self.restructure(&arguments, "insert_columns"),
            "delete_columns" => self.restructure(&arguments, "delete_columns"),
            "set_header_footer" => self.set_header_footer(&arguments),
            "set_shape_text" => self.set_shape_text(&arguments),
            "set_note" => self.set_note(&arguments),
            "delete_note" => self.delete_note(&arguments),
            "undo" => self.undo(&arguments),
            "redo" => self.redo(&arguments),
            "export_workbook" => self.export(&arguments),
            "close_workbook" => self.close(&arguments),
            "list_sessions" => self.list_sessions(&arguments),
            other => Err((INVALID_PARAMS, format!("unknown tool: {other}"))),
        }?;

        // MCP wants human-readable content; the structured copy is what an
        // agent should actually read.
        Ok(json!({
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default() }],
            "structuredContent": result,
        }))
    }

    fn open(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["path", "bytes_base64"])?;
        let path = optional_text(arguments, "path")?;
        let encoded = optional_text(arguments, "bytes_base64")?;
        let bytes = match (path.as_deref(), encoded.as_deref()) {
            (Some(path), None) => {
                std::fs::read(path).map_err(|error| (INVALID_PARAMS, format!("{path}: {error}")))?
            }
            (None, Some(encoded)) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| (INVALID_PARAMS, format!("bytes_base64: {error}")))?,
            _ => {
                return Err((
                    INVALID_PARAMS,
                    "pass exactly one of path or bytes_base64".to_string(),
                ))
            }
        };

        let workbook =
            HeadlessWorkbook::open(&bytes).map_err(|error| (TOOL_FAILED, error.to_string()))?;
        let summary = workbook.summary();
        self.next_id += 1;
        let session_id = format!("wb-{}", self.next_id);
        self.sessions
            .insert(session_id.clone(), Session { workbook, path });
        Ok(json!({ "session_id": session_id, "summary": summary }))
    }

    fn summary(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id"])?;
        let session = self.session(arguments)?;
        to_value(&session.workbook.summary())
    }

    fn read_cell(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "sheet", "reference"])?;
        let sheet = text(arguments, "sheet")?;
        let reference = text(arguments, "reference")?;
        let session = self.session(arguments)?;
        let cell = session
            .workbook
            .cell(&sheet, &reference)
            .map_err(headless_error)?;
        to_value(&cell)
    }

    fn read_cells(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "cells"])?;
        let cells = arguments
            .get("cells")
            .and_then(Value::as_array)
            .ok_or_else(|| (INVALID_PARAMS, "read_cells needs a cells array".to_string()))?;
        if cells.len() > READ_CELLS_LIMIT {
            return Err((
                INVALID_PARAMS,
                format!("read_cells accepts at most {READ_CELLS_LIMIT} cells"),
            ));
        }
        let mut references = Vec::with_capacity(cells.len());
        for cell in cells {
            reject_unknown_fields(cell, &["sheet", "reference"])?;
            references.push((text(cell, "sheet")?, text(cell, "reference")?));
        }

        let session = self.session(arguments)?;
        let cells = references
            .iter()
            .map(|(sheet, reference)| {
                session
                    .workbook
                    .cell(sheet, reference)
                    .map_err(headless_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "cells": cells }))
    }

    fn read_range(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "sheet", "reference"])?;
        let sheet = text(arguments, "sheet")?;
        let reference = text(arguments, "reference")?;
        let session = self.session(arguments)?;
        let range = session
            .workbook
            .range(&sheet, &reference)
            .map_err(headless_error)?;
        to_value(&range)
    }

    fn read_sheet(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "sheet"])?;
        let sheet = text(arguments, "sheet")?;
        let session = self.session(arguments)?;
        let text = session
            .workbook
            .sheet_text(&sheet)
            .map_err(headless_error)?;
        to_value(&text)
    }

    fn find(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "needle", "case_sensitive"])?;
        let needle = text(arguments, "needle")?;
        let case_sensitive = optional_bool(arguments, "case_sensitive")?;
        let session = self.session(arguments)?;
        to_value(&session.workbook.find(&needle, case_sensitive))
    }

    fn set_cell(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(
            arguments,
            &[
                "session_id",
                "sheet",
                "reference",
                "value",
                "as_text",
                "as_formula",
            ],
        )?;
        let sheet = text(arguments, "sheet")?;
        let reference = text(arguments, "reference")?;
        let as_text = optional_bool(arguments, "as_text")?;
        let as_formula = optional_bool(arguments, "as_formula")?;
        let value = arguments
            .get("value")
            .ok_or_else(|| (INVALID_PARAMS, "set_cell needs a value".to_string()))?;

        // JSON already carries the type, so a number stays a number without
        // being guessed at. A string is read the way a spreadsheet reads what
        // you type, unless the caller says to keep it as text.
        let input = cell_input(value, as_text, as_formula)?;

        let session = self.session(arguments)?;
        session
            .workbook
            .set(&sheet, &reference, input)
            .map_err(headless_error)?;
        let cell = session
            .workbook
            .cell(&sheet, &reference)
            .map_err(headless_error)?;
        to_value(&cell)
    }

    fn set_cells(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "sheet", "cells"])?;
        let sheet = text(arguments, "sheet")?;
        let cells = arguments
            .get("cells")
            .and_then(Value::as_array)
            .ok_or_else(|| (INVALID_PARAMS, "set_cells needs a cells array".to_string()))?;
        let mut values = Vec::with_capacity(cells.len());
        for cell in cells {
            reject_unknown_fields(cell, &["reference", "value", "as_text", "as_formula"])?;
            let reference = text(cell, "reference")?;
            let value = cell
                .get("value")
                .ok_or_else(|| (INVALID_PARAMS, "each cell needs a value".to_string()))?;
            let as_text = optional_bool(cell, "as_text")?;
            let as_formula = optional_bool(cell, "as_formula")?;
            values.push((reference, cell_input(value, as_text, as_formula)?));
        }

        let session = self.session(arguments)?;
        session
            .workbook
            .set_cells(&sheet, &values)
            .map_err(headless_error)?;
        let cells = values
            .iter()
            .map(|(reference, _)| {
                session
                    .workbook
                    .cell(&sheet, reference)
                    .map_err(headless_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "cells": cells }))
    }

    fn set_workbook_cells(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "cells"])?;
        let cells = arguments
            .get("cells")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                (
                    INVALID_PARAMS,
                    "set_workbook_cells needs a cells array".to_string(),
                )
            })?;
        let mut values = Vec::with_capacity(cells.len());
        for cell in cells {
            reject_unknown_fields(
                cell,
                &["sheet", "reference", "value", "as_text", "as_formula"],
            )?;
            let sheet = text(cell, "sheet")?;
            let reference = text(cell, "reference")?;
            let value = cell
                .get("value")
                .ok_or_else(|| (INVALID_PARAMS, "each cell needs a value".to_string()))?;
            let as_text = optional_bool(cell, "as_text")?;
            let as_formula = optional_bool(cell, "as_formula")?;
            values.push((sheet, reference, cell_input(value, as_text, as_formula)?));
        }

        let session = self.session(arguments)?;
        session
            .workbook
            .set_workbook_cells(&values)
            .map_err(headless_error)?;
        let cells = values
            .iter()
            .map(|(sheet, reference, _)| {
                session
                    .workbook
                    .cell(sheet, reference)
                    .map_err(headless_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "cells": cells }))
    }

    fn fill_range(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "sheet", "source", "target"])?;
        let sheet = text(arguments, "sheet")?;
        let source = text(arguments, "source")?;
        let target = text(arguments, "target")?;
        let session = self.session(arguments)?;
        let summary = session
            .workbook
            .fill(&sheet, &source, &target)
            .map_err(headless_error)?;
        to_value(&summary)
    }

    fn copy_format(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "sheet", "source", "target"])?;
        let sheet = text(arguments, "sheet")?;
        let source = text(arguments, "source")?;
        let target = text(arguments, "target")?;
        let session = self.session(arguments)?;
        let summary = session
            .workbook
            .copy_format(&sheet, &source, &target)
            .map_err(headless_error)?;
        to_value(&summary)
    }

    fn set_format(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(
            arguments,
            &[
                "session_id",
                "sheet",
                "range",
                "bold",
                "italic",
                "underline",
                "strike",
                "font_color",
                "font_size",
                "font_name",
                "fill_color",
                "number_format",
                "horizontal",
                "vertical",
                "wrap_text",
            ],
        )?;
        let sheet = text(arguments, "sheet")?;
        let range = text(arguments, "range")?;
        let mut change = arguments.clone();
        if let Some(object) = change.as_object_mut() {
            object.remove("session_id");
            object.remove("sheet");
            object.remove("range");
        }
        let change: jade_xlsx_headless_editor::FormatChange = serde_json::from_value(change)
            .map_err(|error| (INVALID_PARAMS, format!("set_format: {error}")))?;
        let session = self.session(arguments)?;
        let summary = session
            .workbook
            .set_format(&sheet, &range, &change)
            .map_err(headless_error)?;
        to_value(&summary)
    }

    fn restructure(&mut self, arguments: &Value, tool: &str) -> Result<Value, (i64, String)> {
        let rows = tool.ends_with("rows");
        let inserted = tool.starts_with("insert");
        let position_key = match (rows, inserted) {
            (true, true) => "before_row",
            (true, false) => "first_row",
            (false, true) => "before_column",
            (false, false) => "first_column",
        };
        reject_unknown_fields(arguments, &["session_id", "sheet", position_key, "count"])?;
        let sheet = text(arguments, "sheet")?;
        let count = match arguments.get("count") {
            None => 1,
            Some(value) => value
                .as_u64()
                .and_then(|count| u32::try_from(count).ok())
                .ok_or_else(|| (INVALID_PARAMS, "count must be a whole number".to_string()))?,
        };
        let session = self.session(arguments)?;
        let summary = if rows {
            let row = arguments
                .get(position_key)
                .and_then(Value::as_u64)
                .and_then(|row| u32::try_from(row).ok())
                .ok_or_else(|| {
                    (
                        INVALID_PARAMS,
                        format!("{tool} needs {position_key}, a row number"),
                    )
                })?;
            if inserted {
                session.workbook.insert_rows(&sheet, row, count)
            } else {
                session.workbook.delete_rows(&sheet, row, count)
            }
        } else {
            let column = text(arguments, position_key)?;
            if inserted {
                session.workbook.insert_columns(&sheet, &column, count)
            } else {
                session.workbook.delete_columns(&sheet, &column, count)
            }
        }
        .map_err(headless_error)?;
        to_value(&summary)
    }

    fn set_header_footer(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(
            arguments,
            &[
                "session_id",
                "sheet",
                "odd_header",
                "odd_footer",
                "even_header",
                "even_footer",
                "first_header",
                "first_footer",
                "different_odd_even",
                "different_first",
                "scale_with_doc",
                "align_with_margins",
            ],
        )?;
        let sheet = text(arguments, "sheet")?;
        let mut change = arguments.clone();
        if let Some(object) = change.as_object_mut() {
            object.remove("session_id");
            object.remove("sheet");
        }
        let change: jade_xlsx_headless_api::HeaderFooterChange = serde_json::from_value(change)
            .map_err(|error| (INVALID_PARAMS, format!("set_header_footer: {error}")))?;
        let session = self.session(arguments)?;
        let view = session
            .workbook
            .set_header_footer(&sheet, &change)
            .map_err(headless_error)?;
        to_value(&view)
    }

    fn set_shape_text(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "sheet", "shape", "text"])?;
        let sheet = text(arguments, "sheet")?;
        let shape = text(arguments, "shape")?;
        let new_text = text(arguments, "text")?;
        let session = self.session(arguments)?;
        let view = session
            .workbook
            .set_shape_text(&sheet, &shape, &new_text)
            .map_err(headless_error)?;
        to_value(&view)
    }

    fn set_note(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(
            arguments,
            &["session_id", "sheet", "reference", "text", "author"],
        )?;
        let sheet = text(arguments, "sheet")?;
        let reference = text(arguments, "reference")?;
        let note = text(arguments, "text")?;
        let author = optional_text(arguments, "author")?.unwrap_or_else(|| "jade".to_string());
        let session = self.session(arguments)?;
        let view = session
            .workbook
            .set_note(&sheet, &reference, &note, &author)
            .map_err(headless_error)?;
        to_value(&view)
    }

    fn delete_note(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "sheet", "reference"])?;
        let sheet = text(arguments, "sheet")?;
        let reference = text(arguments, "reference")?;
        let session = self.session(arguments)?;
        let deleted = session
            .workbook
            .delete_note(&sheet, &reference)
            .map_err(headless_error)?;
        Ok(json!({ "deleted": deleted }))
    }

    fn undo(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id"])?;
        let session = self.session(arguments)?;
        let undone = session.workbook.undo().map_err(headless_error)?;
        Ok(
            json!({ "undone": undone, "can_undo": session.workbook.can_undo(), "can_redo": session.workbook.can_redo() }),
        )
    }

    fn redo(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id"])?;
        let session = self.session(arguments)?;
        let redone = session.workbook.redo().map_err(headless_error)?;
        Ok(
            json!({ "redone": redone, "can_undo": session.workbook.can_undo(), "can_redo": session.workbook.can_redo() }),
        )
    }

    fn recalculate(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id"])?;
        let session = self.session(arguments)?;
        let summary = session.workbook.recalculate().map_err(headless_error)?;
        to_value(&summary)
    }

    fn export(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id", "path"])?;
        let requested = optional_text(arguments, "path")?;
        let session = self.session(arguments)?;
        let bytes = session.workbook.export_bytes().map_err(headless_error)?;
        let path = requested.clone().or_else(|| session.path.clone());

        match path {
            Some(path) => {
                std::fs::write(&path, &bytes)
                    .map_err(|error| (TOOL_FAILED, format!("{path}: {error}")))?;
                if requested.is_some() {
                    session.path = Some(path.clone());
                }
                Ok(json!({ "path": path, "bytes": bytes.len() }))
            }
            // Opened from bytes and given nowhere to write: hand it back the
            // way it came in rather than picking a file for the caller.
            None => Ok(json!({
                "bytes_base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
                "bytes": bytes.len(),
            })),
        }
    }

    fn close(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &["session_id"])?;
        let id = text(arguments, "session_id")?;
        match self.sessions.remove(&id) {
            Some(_) => Ok(json!({ "closed": id })),
            None => Err((INVALID_PARAMS, format!("no session {id}"))),
        }
    }

    fn list_sessions(&self, arguments: &Value) -> Result<Value, (i64, String)> {
        reject_unknown_fields(arguments, &[])?;
        Ok(json!({
            "sessions": self
                .sessions
                .iter()
                .map(|(id, session)| json!({
                    "session_id": id,
                    "path": session.path,
                    "sheets": session
                        .workbook
                        .summary()
                        .sheets
                        .iter()
                        .map(|sheet| sheet.name.clone())
                        .collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>()
        }))
    }

    fn session(&mut self, arguments: &Value) -> Result<&mut Session, (i64, String)> {
        let id = text(arguments, "session_id")?;
        self.sessions.get_mut(&id).ok_or_else(|| {
            (
                INVALID_PARAMS,
                format!("no session {id}; open_workbook first"),
            )
        })
    }
}

fn text(arguments: &Value, key: &str) -> Result<String, (i64, String)> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| (INVALID_PARAMS, format!("missing `{key}`")))
}

fn optional_bool(arguments: &Value, key: &str) -> Result<bool, (i64, String)> {
    match arguments.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err((INVALID_PARAMS, format!("`{key}` must be a boolean"))),
    }
}

fn optional_text(arguments: &Value, key: &str) -> Result<Option<String>, (i64, String)> {
    match arguments.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err((INVALID_PARAMS, format!("`{key}` must be a string"))),
    }
}

fn cell_input(value: &Value, as_text: bool, as_formula: bool) -> Result<CellInput, (i64, String)> {
    if as_text && as_formula {
        return Err((
            INVALID_PARAMS,
            "`as_text` and `as_formula` cannot be used together".to_string(),
        ));
    }
    if as_formula {
        return match value {
            Value::String(text) => Ok(CellInput::Formula(text.clone())),
            _ => Err((
                INVALID_PARAMS,
                "a formula value must be a string".to_string(),
            )),
        };
    }
    match value {
        Value::Number(number) => Ok(CellInput::Number(number.as_f64().unwrap_or_default())),
        Value::Bool(flag) => Ok(CellInput::Bool(*flag)),
        Value::Null => Ok(CellInput::Blank),
        Value::String(text) if as_text => Ok(CellInput::Text(text.clone())),
        Value::String(text) => Ok(CellInput::parse(text)),
        _ => Err((
            INVALID_PARAMS,
            "cell value must be a number, string, boolean, or null".to_string(),
        )),
    }
}

fn reject_unknown_fields(arguments: &Value, allowed: &[&str]) -> Result<(), (i64, String)> {
    let object = arguments
        .as_object()
        .ok_or_else(|| (INVALID_PARAMS, "arguments must be an object".to_string()))?;
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err((INVALID_PARAMS, format!("unknown argument `{field}`")));
    }
    Ok(())
}

fn headless_error(error: HeadlessError) -> (i64, String) {
    let code = if error.is_invalid_input() {
        INVALID_PARAMS
    } else {
        TOOL_FAILED
    };
    (code, error.to_string())
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, (i64, String)> {
    serde_json::to_value(value).map_err(|error| (TOOL_FAILED, error.to_string()))
}

fn success_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// Every tool, described well enough that a client needs no other
/// documentation — which is all a client gets.
fn tool_catalogue() -> Vec<Value> {
    let session = json!({
        "type": "string",
        "description": "The id open_workbook returned."
    });
    let sheet = json!({
        "type": "string",
        "description": "A sheet name, or its index from 0 when no sheet has that name."
    });
    let incompatible_write_modes = json!({
        "required": ["as_text", "as_formula"],
        "properties": {
            "as_text": { "const": true },
            "as_formula": { "const": true }
        }
    });

    vec![
        json!({
            "name": "open_workbook",
            "title": "Open a workbook",
            "description": "Load an .xlsx into an in-memory session and return the session_id every other tool takes, plus a summary. Pass exactly one of path or bytes_base64.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the .xlsx." },
                    "bytes_base64": { "type": "string", "description": "The file's bytes, base64 encoded." }
                },
                "oneOf": [
                    { "required": ["path"] },
                    { "required": ["bytes_base64"] }
                ]
            }
        }),
        json!({
            "name": "summary",
            "title": "Describe the workbook",
            "description": "Sheets and their visibility, used and merged ranges, frozen panes, Excel tables (name, range, header and totals rows, column names), the page header/footer, manual page breaks, shapes and text boxes with their anchors and text, global and sheet-scoped defined names, and which epoch date serials count from.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "session_id": session },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "read_cell",
            "title": "Read one cell",
            "description": "What the cell stores (number or text), what it shows after its number format, the formula behind it, that format's code, any external or workbook-internal hyperlink, legacy comment/note, input validation rule, and the Excel table it belongs to (with its column, and whether the cell is on the header or totals row).",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "reference": { "type": "string", "description": "An A1-style reference, e.g. B9." }
                },
                "required": ["session_id", "sheet", "reference"]
            }
        }),
        json!({
            "name": "read_cells",
            "title": "Read several cells",
            "description": "Read up to 1000 cells across one or several sheets in one call, preserving request order. Each result includes stored/display values, formula, format, hyperlink, comment, and validation just like read_cell.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "cells": {
                        "type": "array",
                        "maxItems": 1000,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "sheet": sheet,
                                "reference": { "type": "string", "description": "An A1-style reference, e.g. B9." }
                            },
                            "required": ["sheet", "reference"]
                        }
                    }
                },
                "required": ["session_id", "cells"]
            }
        }),
        json!({
            "name": "read_range",
            "title": "Read a range",
            "description": "A rectangle of cells in row-major order. Cells the file does not write come back empty rather than missing, so the grid can be indexed. Capped at a million cells.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "reference": { "type": "string", "description": "An A1:D10-style range." }
                },
                "required": ["session_id", "sheet", "reference"]
            }
        }),
        json!({
            "name": "read_sheet",
            "title": "Read a whole sheet as text",
            "description": "The actual used rectangle as its A1 reference plus rows of display text — the cheapest way to see what a sheet says. Leading empty rows and columns before that rectangle are not materialised.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "session_id": session, "sheet": sheet },
                "required": ["session_id", "sheet"]
            }
        }),
        json!({
            "name": "find",
            "title": "Search the workbook",
            "description": "Every cell whose displayed text or formula source contains the needle. Searching what is shown rather than what is stored makes a date findable by the date it shows; searching formulas makes calculations findable by function or reference.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "needle": { "type": "string" },
                    "case_sensitive": { "type": "boolean", "default": false }
                },
                "required": ["session_id", "needle"]
            }
        }),
        json!({
            "name": "set_cell",
            "title": "Put a value in a cell",
            "description": "Writes into the session. The cell keeps its format. A value replaces any formula; as_formula writes a formula. The session then recalculates: dependents and written formulas answer with fresh results, except formulas the engine cannot evaluate, which keep their cached values and read as formula_state=stale. Writing into an Excel table's header cell renames that column, as in Excel: the name is made unique (Amount2) and non-empty (Column1), and the cell holds the resulting text. Returns the cell as it now reads.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "reference": { "type": "string" },
                    "value": {
                        "description": "A number, a boolean, null to clear the cell, or a string. A string is read the way a spreadsheet reads what you type unless as_text is set.",
                        "type": ["number", "string", "boolean", "null"]
                    },
                    "as_text": { "type": "boolean", "default": false, "description": "Keep a string as text even if it looks like a number." },
                    "as_formula": { "type": "boolean", "default": false, "description": "Write a string as a formula. A leading = is optional. Cannot be combined with as_text." }
                },
                "required": ["session_id", "sheet", "reference", "value"],
                "not": incompatible_write_modes.clone()
            }
        }),
        json!({
            "name": "set_cells",
            "title": "Put values in several cells",
            "description": "Atomically writes several values or explicit formulas in one sheet. If any reference or value is invalid, none of the cells change. Formats are kept, and the session recalculates dependents the way set_cell does.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "cells": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "reference": { "type": "string" },
                                "value": { "type": ["number", "string", "boolean", "null"] },
                                "as_text": { "type": "boolean", "default": false },
                                "as_formula": { "type": "boolean", "default": false }
                            },
                            "required": ["reference", "value"],
                            "not": incompatible_write_modes.clone()
                        }
                    }
                },
                "required": ["session_id", "sheet", "cells"]
            }
        }),
        json!({
            "name": "set_workbook_cells",
            "title": "Put values in several sheets",
            "description": "Atomically writes values or explicit formulas across several sheets. Every sheet and reference is validated first; if any cell is invalid, no sheet changes. The session then recalculates dependents the way set_cell does.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "cells": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "sheet": sheet,
                                "reference": { "type": "string" },
                                "value": { "type": ["number", "string", "boolean", "null"] },
                                "as_text": { "type": "boolean", "default": false },
                                "as_formula": { "type": "boolean", "default": false }
                            },
                            "required": ["sheet", "reference", "value"],
                            "not": incompatible_write_modes
                        }
                    }
                },
                "required": ["session_id", "cells"]
            }
        }),
        json!({
            "name": "recalculate",
            "title": "Recalculate every formula",
            "description": "Evaluates all formulas and refreshes their cached values in the session — useful for a workbook opened stale. Reports how many were refreshed, how many were already current, and which were skipped (with the engine's reason); skipped formulas keep their cached values. Writes recalculate automatically, so this is rarely needed after set_cell.",
            "annotations": { "readOnlyHint": false },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "session_id": session },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "fill_range",
            "title": "Continue a run into neighbouring cells",
            "description": "What dragging the fill handle does: continues the run in `source` into the rest of `target`, which must be the source grown by rows or by columns (not both). Two or more numbers continue as a linear series (their difference, or the least-squares trend of more); a lone date steps a day at a time; text ending in a number counts on (`Item 1` → `Item 2`); a lone number, formulas, booleans and mixed runs repeat as a pattern, with formula references moved the way a paste moves them. One atomic write, recalculated and undoable like set_cells. Answers the cells written.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "source": { "type": "string", "description": "The run to continue, as F1:F2 (a single cell as F1:F1)." },
                    "target": { "type": "string", "description": "The rectangle to cover, source included, as F1:F6." }
                },
                "required": ["session_id", "sheet", "source", "target"]
            }
        }),
        json!({
            "name": "copy_format",
            "title": "Copy formats onto other cells",
            "description": "Excel's format painter: gives the cells of `target` the number formats, fonts, fills, borders and alignment of `source`, tiled when the target is larger; a single target cell takes the source's whole shape from there. Values, formulas and caches stay as they are — use set_cells to change those. A source cell with no format clears the target's. One atomic write, undoable. Answers the cells written.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "source": { "type": "string", "description": "The cells whose formats to copy, as B4:B6 (a single cell as B4 or B4:B4)." },
                    "target": { "type": "string", "description": "The cells to format, as F1:F6 — or one cell, F1, to stamp the source's shape from it." }
                },
                "required": ["session_id", "sheet", "source", "target"]
            }
        }),
        json!({
            "name": "set_format",
            "title": "Change how cells look",
            "description": "Formats the cells of `range`: bold, italic, underline, strike, font colour/size/name, a solid fill colour, a number format code, horizontal/vertical alignment, wrapping. Only what is given changes; everything else each cell has — its border, its number format when only bold is asked for — is kept. Values and formulas stay. New looks are added to the file's style table and reused when they exist already. One atomic write, undoable. Answers the cells written and how many styles were added.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "range": { "type": "string", "description": "Cells as B4:D6 (a single cell as B4), complete rows as 3:5, complete columns as B:D, or the sheet as A1:XFD1048576. Axis defaults also apply to future cells; the returned cell list includes only patched stored/intersection cells." },
                    "bold": { "type": "boolean" },
                    "italic": { "type": "boolean" },
                    "underline": { "type": "boolean" },
                    "strike": { "type": "boolean" },
                    "font_color": { "type": "string", "description": "RRGGBB." },
                    "font_size": { "type": "number", "description": "Points." },
                    "font_name": { "type": "string" },
                    "fill_color": { "type": "string", "description": "RRGGBB for a solid fill; the empty string for no fill." },
                    "number_format": { "type": "string", "description": "A format code such as 0.00, #,##0, 0.0%, yyyy/mm/dd; General or the empty string for General." },
                    "horizontal": { "type": "string", "enum": ["general", "left", "center", "right"] },
                    "vertical": { "type": "string", "enum": ["top", "center", "bottom"] },
                    "wrap_text": { "type": "boolean" }
                },
                "required": ["session_id", "sheet", "range"]
            }
        }),
        json!({
            "name": "insert_rows",
            "title": "Insert rows",
            "description": "Inserts empty rows before `before_row` (1-based, as Excel numbers them). Everything below moves down — cells, merges, validations, conditional formats, protected ranges, page breaks, comments, shapes and charts, Excel tables — and every formula in the workbook that pointed below follows, as do defined names and chart series; a range or table that spanned the insertion grows. One atomic write, recalculated and undoable.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "before_row": { "type": "integer", "minimum": 1 },
                    "count": { "type": "integer", "minimum": 1, "default": 1 }
                },
                "required": ["session_id", "sheet", "before_row"]
            }
        }),
        json!({
            "name": "delete_rows",
            "title": "Delete rows",
            "description": "Deletes rows from `first_row` (1-based) on. Everything below moves up; ranges, tables and shapes that covered the rows shrink, references into them become #REF!, merges, comments and shapes wholly inside them go, and a table whose every row went is removed. Deleting a table's header row makes the next row the header (rewritten with the column names); deleting its totals row turns totals off. One atomic write, recalculated and undoable.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "first_row": { "type": "integer", "minimum": 1 },
                    "count": { "type": "integer", "minimum": 1, "default": 1 }
                },
                "required": ["session_id", "sheet", "first_row"]
            }
        }),
        json!({
            "name": "insert_columns",
            "title": "Insert columns",
            "description": "Inserts empty columns before `before_column` (a letter such as C). Everything to the right moves — cells, comments, shapes, tables — and every formula, defined name and chart series that pointed there follows. A column inserted inside an Excel table becomes a table column named Column1, Column2, ... with that name in its header cell. One atomic write, recalculated and undoable.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "before_column": { "type": "string", "description": "A column letter, such as C." },
                    "count": { "type": "integer", "minimum": 1, "default": 1 }
                },
                "required": ["session_id", "sheet", "before_column"]
            }
        }),
        json!({
            "name": "delete_columns",
            "title": "Delete columns",
            "description": "Deletes columns from `first_column` (a letter such as C) on. Everything to the right moves left; ranges, tables and shapes that covered the columns shrink, references into them become #REF!, comments and shapes wholly inside them go, a table column inside them goes, and a table whose every column went is removed. One atomic write, recalculated and undoable.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "first_column": { "type": "string", "description": "A column letter, such as C." },
                    "count": { "type": "integer", "minimum": 1, "default": 1 }
                },
                "required": ["session_id", "sheet", "first_column"]
            }
        }),
        json!({
            "name": "set_header_footer",
            "title": "Set the page header and footer",
            "description": "Sets a sheet's printed header and footer. Each section given is set — null clears it — and each left out is kept. Text is in Excel's code language: &L, &C and &R start the left, centre and right parts; &P is the page number, &N the page count, &D the date, &T the time, &A the sheet name, &F the file name, and && a literal ampersand. Even and first-page sections apply only when different_odd_even / different_first are on. One undo step.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "odd_header": { "type": ["string", "null"], "description": "The header of every page (of odd pages when different_odd_even is on)." },
                    "odd_footer": { "type": ["string", "null"] },
                    "even_header": { "type": ["string", "null"] },
                    "even_footer": { "type": ["string", "null"] },
                    "first_header": { "type": ["string", "null"] },
                    "first_footer": { "type": ["string", "null"] },
                    "different_odd_even": { "type": "boolean" },
                    "different_first": { "type": "boolean" },
                    "scale_with_doc": { "type": "boolean" },
                    "align_with_margins": { "type": "boolean" }
                },
                "required": ["session_id", "sheet"]
            }
        }),
        json!({
            "name": "set_shape_text",
            "title": "Put text in a text box or shape",
            "description": "Replaces the text of a text box or shape on the sheet — named as summary lists it (TextBox 1) or by its id — one paragraph per line, in the font the shape's text had. A picture or a chart has no text to replace. Everything else about the shape (position, size, fill, line) stays. One undo step. Answers the shape as it now reads.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "shape": { "type": "string", "description": "The shape's name (TextBox 1) or its numeric id." },
                    "text": { "type": "string", "description": "The new text; newlines separate paragraphs." }
                },
                "required": ["session_id", "sheet", "shape", "text"]
            }
        }),
        json!({
            "name": "set_note",
            "title": "Put a note on a cell",
            "description": "Attaches a note (what older Excel calls a comment: the yellow box shown when the cell is pointed at) to a cell, replacing any it has. The comments part and the VML that places the box are created when the sheet has none. One undo step. Answers the note as it now reads.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "reference": { "type": "string", "description": "An A1-style reference." },
                    "text": { "type": "string" },
                    "author": { "type": "string", "description": "The note's author; `jade` when not given." }
                },
                "required": ["session_id", "sheet", "reference", "text"]
            }
        }),
        json!({
            "name": "delete_note",
            "title": "Remove a cell's note",
            "description": "Removes the note on a cell. Answers deleted=false, changing nothing, when the cell has none. One undo step.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": session,
                    "sheet": sheet,
                    "reference": { "type": "string", "description": "An A1-style reference." }
                },
                "required": ["session_id", "sheet", "reference"]
            }
        }),
        json!({
            "name": "undo",
            "title": "Take back the last write",
            "description": "Restores the session to the bytes before the last set_cell / set_cells / set_workbook_cells (the recalculation that followed it included). Up to 100 writes can be undone. Answers undone=false, changing nothing, when there is nothing to undo.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "session_id": session },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "redo",
            "title": "Reapply the last undone write",
            "description": "Reapplies the write most recently taken back by undo. A new write after an undo discards what could be redone. Answers redone=false when there is nothing to redo.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "session_id": session },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "export_workbook",
            "title": "Write the workbook out",
            "description": "Saves the session. Writes to path, or back over the file it was opened from; a workbook opened from bytes with no path comes back as bytes_base64.",
            "annotations": { "readOnlyHint": false },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "session_id": session, "path": { "type": "string" } },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "close_workbook",
            "title": "Close a session",
            "description": "Frees the workbook. Unsaved changes are lost.",
            "annotations": { "readOnlyHint": false, "destructiveHint": true },
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "session_id": session },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "list_sessions",
            "title": "List open workbooks",
            "description": "Every session this server holds, with where it came from.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": { "type": "object", "additionalProperties": false, "properties": {} }
        }),
    ]
}
