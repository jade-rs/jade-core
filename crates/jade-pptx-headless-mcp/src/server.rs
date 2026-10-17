//! Protocol handling and the tools.
//!
//! Kept separate from the binary so it can be tested by feeding raw JSON-RPC
//! strings, which is the only way to be sure a client sees what it should.

use std::collections::BTreeMap;

use base64::Engine;
use jade_pptx_reader::Deck;
use serde_json::{json, Value};

const INVALID_PARAMS: i64 = -32602;
const TOOL_FAILED: i64 = -32000;

const PROTOCOL_VERSION: &str = "2024-11-05";

const INSTRUCTIONS: &str = "\
Open a presentation once with open_deck and pass the session_id it returns to \
every other tool; close_deck frees it. Slides are numbered from 1, in \
presentation order. summary, read_slide, deck_text and find report what the \
deck says — text, titles, tables, speaker notes, shape structure, placements \
in px. slide_layout and check_layout report what it would draw: every shape's \
resolved geometry and text lines, and the text that overflows its box or \
shapes off the slide. read_media returns a picture's bytes. Nothing writes the \
deck back yet.";

/// An open deck and where it came from.
struct Session {
    deck: Deck,
    path: Option<String>,
    /// The package as opened, so media parts can be read back.
    bytes: Vec<u8>,
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
                "serverInfo": { "name": "jade-pptx-headless-mcp", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_catalogue() })),
            "tools/call" => self.call(&params),
            _ if id.is_none() => return None,
            other => Err((-32601, format!("unknown method: {other}"))),
        };

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
            "open_deck" => self.open(&arguments),
            "summary" => self.summary(&arguments),
            "read_slide" => self.read_slide(&arguments),
            "deck_text" => self.deck_text(&arguments),
            "find" => self.find(&arguments),
            "slide_layout" => self.slide_layout(&arguments),
            "check_layout" => self.check_layout(&arguments),
            "read_media" => self.read_media(&arguments),
            "close_deck" => self.close(&arguments),
            "list_sessions" => Ok(self.list_sessions()),
            other => Err((INVALID_PARAMS, format!("unknown tool: {other}"))),
        }?;

        Ok(json!({
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default() }],
            "structuredContent": result,
        }))
    }

    fn open(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
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

        let deck = jade_pptx_reader::read_deck(&bytes)
            .map_err(|error| (TOOL_FAILED, error.to_string()))?;
        let summary = jade_pptx_headless_cli::summarize(&deck);
        self.next_id += 1;
        let session_id = format!("deck-{}", self.next_id);
        self.sessions
            .insert(session_id.clone(), Session { deck, path, bytes });
        Ok(json!({ "session_id": session_id, "summary": summary }))
    }

    fn summary(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        let session = self.session(arguments)?;
        to_value(&jade_pptx_headless_cli::summarize(&session.deck))
    }

    fn read_slide(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        let slide = slide_number(arguments, "read_slide")?;
        let session = self.session(arguments)?;
        let view = jade_pptx_headless_cli::view(&session.deck, slide)
            .map_err(|error| (INVALID_PARAMS, error.to_string()))?;
        to_value(&view)
    }

    fn deck_text(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        let session = self.session(arguments)?;
        Ok(json!({ "text": jade_pptx_headless_cli::text(&session.deck) }))
    }

    fn find(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        let needle = text(arguments, "needle")?;
        let case_sensitive = optional_bool(arguments, "case_sensitive")?;
        let session = self.session(arguments)?;
        let matches = jade_pptx_headless_cli::find(&session.deck, &needle, case_sensitive);
        Ok(json!({ "matches": matches }))
    }

    fn slide_layout(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        let slide = slide_number(arguments, "slide_layout")?;
        let session = self.session(arguments)?;
        let layout = jade_pptx_headless_cli::layout_view(&session.deck, slide)
            .map_err(|error| (INVALID_PARAMS, error.to_string()))?;
        to_value(&layout)
    }

    fn check_layout(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        let slide = match arguments.get("slide") {
            None | Some(Value::Null) => None,
            Some(_) => Some(slide_number(arguments, "check_layout")?),
        };
        let session = self.session(arguments)?;
        let issues = jade_pptx_headless_cli::check(&session.deck, slide)
            .map_err(|error| (INVALID_PARAMS, error.to_string()))?;
        Ok(json!({ "issues": issues }))
    }

    fn read_media(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        let part = text(arguments, "part")?;
        let session = self.session(arguments)?;
        let (data, content_type) = jade_pptx_headless_cli::media(&session.bytes, &part)
            .map_err(|error| (INVALID_PARAMS, error.to_string()))?;
        Ok(json!({
            "part": part,
            "content_type": content_type,
            "bytes": data.len(),
            "bytes_base64": base64::engine::general_purpose::STANDARD.encode(&data),
        }))
    }

    fn close(&mut self, arguments: &Value) -> Result<Value, (i64, String)> {
        let id = text(arguments, "session_id")?;
        match self.sessions.remove(&id) {
            Some(_) => Ok(json!({ "closed": id })),
            None => Err((INVALID_PARAMS, format!("no session {id}"))),
        }
    }

    fn list_sessions(&self) -> Value {
        json!({
            "sessions": self
                .sessions
                .iter()
                .map(|(id, session)| json!({
                    "session_id": id,
                    "path": session.path,
                    "slides": session.deck.slides.len(),
                }))
                .collect::<Vec<_>>()
        })
    }

    fn session(&mut self, arguments: &Value) -> Result<&mut Session, (i64, String)> {
        let id = text(arguments, "session_id")?;
        self.sessions
            .get_mut(&id)
            .ok_or_else(|| (INVALID_PARAMS, format!("no session {id}; open_deck first")))
    }
}

fn slide_number(arguments: &Value, tool: &str) -> Result<usize, (i64, String)> {
    arguments
        .get("slide")
        .and_then(Value::as_u64)
        .filter(|slide| *slide >= 1)
        .map(|slide| slide as usize)
        .ok_or_else(|| (INVALID_PARAMS, format!("{tool} needs a 1-based `slide`")))
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
        "description": "The id open_deck returned."
    });
    let slide =
        json!({ "type": "integer", "description": "1-based slide number, in presentation order." });

    vec![
        json!({
            "name": "open_deck",
            "title": "Open a presentation",
            "description": "Load a .pptx into an in-memory session and return the session_id every other tool takes, plus a summary. Pass exactly one of path or bytes_base64.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the .pptx." },
                    "bytes_base64": { "type": "string", "description": "The file's bytes, base64 encoded." }
                }
            }
        }),
        json!({
            "name": "summary",
            "title": "Summarize the deck",
            "description": "Slide count, slide size in px, and each slide's title, shape count and whether it has speaker notes.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": session },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "read_slide",
            "title": "Read one slide",
            "description": "The slide's shapes in z-order with their ids, names, placeholder roles, geometry, placement in px, text, table cells (rows) and hyperlinks, plus the speaker notes. Groups nest their children.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": session, "slide": slide },
                "required": ["session_id", "slide"]
            }
        }),
        json!({
            "name": "deck_text",
            "title": "Read the whole deck as text",
            "description": "Every slide's text in order, headed by its slide number. Table rows are tab-separated; speaker notes follow each slide quoted with '> '.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": session },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "find",
            "title": "Find text",
            "description": "Shapes, table cells (named Table!B2) and speaker notes whose text contains the needle, with their slide numbers. Case-insensitive unless case_sensitive is true.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": session,
                    "needle": { "type": "string" },
                    "case_sensitive": { "type": "boolean" }
                },
                "required": ["session_id", "needle"]
            }
        }),
        json!({
            "name": "slide_layout",
            "title": "Lay out one slide",
            "description": "What the renderer would draw for the slide: every shape in paint order (master and layout shapes included, marked by origin) with its resolved rect in px, rotation, geometry, fill, outline, text lines as wrapped, and how many px its text overflows the shape.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": session, "slide": slide },
                "required": ["session_id", "slide"]
            }
        }),
        json!({
            "name": "check_layout",
            "title": "Check a deck for layout problems",
            "description": "Text that overflows its shape and shapes lying wholly outside the slide, for one slide or (without slide) the whole deck. Empty issues means nothing was found.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": session, "slide": slide },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "read_media",
            "title": "Read a picture's bytes",
            "description": "The bytes of a media part, base64 encoded, with its MIME type. Pass the image_part a read_slide picture reports.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": session,
                    "part": { "type": "string", "description": "Package part name, e.g. ppt/media/image1.png." }
                },
                "required": ["session_id", "part"]
            }
        }),
        json!({
            "name": "close_deck",
            "title": "Close a session",
            "description": "Frees the in-memory deck.",
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": session },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "list_sessions",
            "title": "List open sessions",
            "description": "Every open session with its path and slide count.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}
