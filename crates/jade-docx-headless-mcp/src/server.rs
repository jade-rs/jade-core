//! JSON-RPC dispatch and tool implementations for the headless DOCX MCP server.
//!
//! [`Server`] owns the in-memory session store and turns a single line of
//! JSON-RPC into an optional response line. Keeping all logic here (instead of
//! in `main`) lets the dispatch be exercised by feeding raw JSON strings in
//! tests, with no real stdio.

use std::collections::HashMap;
use std::sync::Arc;

use crate::extension::{
    RegisterError, RegisteredHandler, RegisteredTool, Sessions, ToolArg, ToolContext, ToolSpec,
};
use crate::file_access::FileAccess;
use base64::Engine as _;
use futures::executor::block_on;
use jade_docx_document_core::commands::{CommandContext, CommandExecutor};
use jade_docx_document_core::headless_api::{
    apply_headless_commands_with_executor_to_document,
    apply_snapshot_headless_commands_with_executor_to_document,
    execute_headless_queries_on_document,
};
use jade_docx_document_core::word_document::WordDocument;
use jade_docx_document_core::Options;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessEditRequest, HeadlessQuery, HeadlessQueryRequest,
};
use jade_selection::SelectionRange;
use serde_json::{json, Value};

/// MCP protocol version echoed back when the client does not request one.
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
/// Identity a server reports unless the binary that owns it says otherwise.
const SERVER_NAME: &str = "jade-docx-headless-mcp";
/// Human-readable server name for client UIs.
const SERVER_TITLE: &str = "jade headless DOCX editor";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

const BASE64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// One located replacement: where it is and what the text becomes. Regex
/// replacements differ per match (the captures are expanded into each), so the
/// replacement travels with the match rather than being applied globally.
struct ReplaceTarget {
    paragraph: usize,
    start_offset: usize,
    end_offset: usize,
    matched: String,
    replacement: String,
}

/// How many matches a `replace_text` result describes individually. A
/// document-wide replacement can have thousands; the count is always exact,
/// the listing is capped so the response stays readable.
const MAX_REPORTED_MATCHES: usize = 50;

/// The matches a caller can act on: byte offsets they can feed straight back
/// into `add_comment` or `replace_range`, in document order.
fn report_matches(targets: &[ReplaceTarget]) -> Value {
    let mut ordered: Vec<&ReplaceTarget> = targets.iter().collect();
    ordered.sort_by_key(|target| (target.paragraph, target.start_offset));
    Value::Array(
        ordered
            .into_iter()
            .take(MAX_REPORTED_MATCHES)
            .map(|target| {
                json!({
                    "paragraph": target.paragraph,
                    "start_offset": target.start_offset,
                    "end_offset": target.end_offset,
                    "text": target.matched,
                    "replacement": target.replacement,
                })
            })
            .collect::<Vec<_>>(),
    )
}

/// One open document: the parsed model plus the command history (executor) and
/// current selection, which together let `undo` / `redo` work across MCP calls.
pub(crate) struct Session {
    pub(crate) doc: WordDocument,
    pub(crate) executor: CommandExecutor,
    pub(crate) selection: Option<SelectionRange>,
}

/// Stateful MCP server: a map of session id → open session, a monotonic
/// counter used to mint deterministic session ids, and any tools injected by
/// the binary that owns this server.
/// A hook that can refuse a tool call before it runs.
///
/// Receives the tool name and its arguments, and returns `Err(message)` to
/// refuse. The message is surfaced to the client as a tool error, so write it
/// for whoever has to act on it rather than as a diagnostic.
///
/// This crate knows about DOCX and nothing else. Whether a caller is *allowed*
/// to edit is the consumer's business — entitlement, licensing, tenancy all
/// live above. So the gate is a hole to plug logic into, not a policy baked in
/// here.
pub type ToolGate = dyn Fn(&str, &Value) -> Result<(), String> + Send + Sync + 'static;

pub struct Server {
    sessions: HashMap<String, Session>,
    next_id: u64,
    /// Set by the consumer to refuse tool calls. `None` means every call runs,
    /// which is what an embedder that has no such concept wants.
    gate: Option<Arc<ToolGate>>,
    /// Tools registered from above, in registration order so `tools/list`
    /// reads the way the consumer wrote them.
    extensions: Vec<RegisteredTool>,
    /// What this server calls itself in `initialize`. A binary that injects its
    /// own tools is a different server to the client — it publishes a different
    /// catalogue — so it should be able to say so rather than reporting the
    /// name of the crate it was built on.
    server_name: String,
    server_title: String,
    /// Where `open_docx` may read and `export_docx` may write. See
    /// [`FileAccess`] for why this is narrower than the process's own reach.
    file_access: FileAccess,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// Creates an empty server with no open document sessions.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            next_id: 0,
            gate: None,
            extensions: Vec::new(),
            server_name: SERVER_NAME.to_string(),
            server_title: SERVER_TITLE.to_string(),
            file_access: FileAccess::from_env(),
        }
    }

    /// Confines `open_docx` / `export_docx` paths to `access`.
    ///
    /// [`Self::new`] reads the roots from `JADE_MCP_ALLOWED_ROOTS` (or uses the
    /// working directory). An embedder that knows where its documents live —
    /// a download folder it manages, a per-user workspace — says so here.
    #[must_use]
    pub fn with_file_access(mut self, access: FileAccess) -> Self {
        self.file_access = access;
        self
    }

    /// Renames the server for `initialize`.
    ///
    /// A binary that registers its own tools publishes a catalogue this crate
    /// does not know about, and clients show the server name in their UI and
    /// their logs. Reporting `jade-docx-headless-mcp` there tells the user the wrong
    /// thing about which server answered.
    ///
    /// The version stays this crate's — it describes the engine the client is
    /// actually talking to.
    #[must_use]
    pub fn with_server_info(mut self, name: impl Into<String>, title: impl Into<String>) -> Self {
        self.server_name = name.into();
        self.server_title = title.into();
        self
    }

    /// Install a hook that can refuse tool calls.
    ///
    /// Called for every tool before it runs, built in and injected alike.
    /// Returning `Err(message)` turns the call into a tool error carrying that
    /// message; returning `Ok(())` lets it through.
    ///
    /// Not setting one leaves the server as it was, so embedders that have no
    /// notion of entitlement are unaffected.
    ///
    /// ```no_run
    /// # use jade_docx_headless_mcp::server::Server;
    /// let mut server = Server::new();
    /// server.set_tool_gate(|_tool, _args| {
    ///     Err("Lawgue の利用権を確認できません。サインインし直してください。".to_string())
    /// });
    /// ```
    pub fn set_tool_gate(
        &mut self,
        gate: impl Fn(&str, &Value) -> Result<(), String> + Send + Sync + 'static,
    ) {
        self.gate = Some(Arc::new(gate));
    }

    /// Registers a tool implemented by the caller.
    ///
    /// The tool joins the built-in catalogue: it is published by `tools/list`
    /// with the same schema, annotations and result shape, it is called with
    /// the session its `session_id` names already resolved, and its edits share
    /// the session's undo history. Nothing about it reads as second class to a
    /// client.
    ///
    /// Registration fails rather than publishing a tool a model cannot use —
    /// see [`RegisterError`].
    pub fn register_tool(
        &mut self,
        spec: ToolSpec,
        handler: impl Fn(&mut ToolContext<'_>, &Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Result<(), RegisterError> {
        spec.validate()?;
        if TOOLS.iter().any(|tool| tool.name == spec.name)
            || self
                .extensions
                .iter()
                .any(|extension| extension.spec.name == spec.name)
        {
            return Err(RegisterError::DuplicateName(spec.name));
        }
        self.extensions.push(RegisteredTool {
            spec,
            handler: RegisteredHandler::Session(Arc::new(handler)),
        });
        Ok(())
    }

    /// Registers a tool that *opens* a session instead of receiving one.
    ///
    /// The session-scoped [`Self::register_tool`] covers almost everything, but
    /// a document has to enter the server somehow. A consumer that loads one
    /// from a place this crate knows nothing about — an internal service, a
    /// signed URL — registers here and gets the session store, so the
    /// `session_id` it returns is the same kind `open_docx` mints and every
    /// other tool accepts.
    ///
    /// No `session_id` argument is added: this tool is what produces it. That
    /// also means the tool may declare its own, should it want to reopen into
    /// an existing session.
    ///
    /// # Errors
    /// Same as [`Self::register_tool`].
    pub fn register_session_tool(
        &mut self,
        spec: ToolSpec,
        handler: impl Fn(&mut Sessions<'_>, &Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Result<(), RegisterError> {
        let mut spec = spec;
        spec.session_scoped = false;
        spec.validate()?;
        if TOOLS.iter().any(|tool| tool.name == spec.name)
            || self
                .extensions
                .iter()
                .any(|extension| extension.spec.name == spec.name)
        {
            return Err(RegisterError::DuplicateName(spec.name));
        }
        self.extensions.push(RegisteredTool {
            spec,
            handler: RegisteredHandler::Store(Arc::new(handler)),
        });
        Ok(())
    }

    /// [`Self::register_tool`] for a `main` that builds the server in one
    /// expression.
    pub fn with_tool(
        mut self,
        spec: ToolSpec,
        handler: impl Fn(&mut ToolContext<'_>, &Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Result<Self, RegisterError> {
        self.register_tool(spec, handler)?;
        Ok(self)
    }

    /// Handle one JSON-RPC message. Returns `Some(response_line)` for requests
    /// (anything with an `id`) and `None` for notifications and blank input.
    pub fn handle_message(&mut self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        let message: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                return Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("parse error: {err}"),
                ));
            }
        };

        let method = match message.get("method").and_then(Value::as_str) {
            Some(method) => method.to_string(),
            None => {
                let id = message.get("id").cloned().unwrap_or(Value::Null);
                return Some(error_response(
                    id,
                    -32600,
                    "invalid request: missing method",
                ));
            }
        };

        // Notifications never get a response. `notifications/initialized` is the
        // standard post-handshake notification; we also ignore any other
        // `notifications/*` defensively.
        if method == "initialized" || method.starts_with("notifications/") {
            return None;
        }

        let has_id = message.get("id").is_some();
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        let outcome: Result<Value, (i64, String)> = match method.as_str() {
            "initialize" => Ok(self.handle_initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.tool_catalogue() })),
            "tools/call" => self.handle_tools_call(&params),
            other => Err((-32601, format!("method not found: {other}"))),
        };

        // A request without an `id` is a notification: do work but stay silent.
        if !has_id {
            return None;
        }

        Some(match outcome {
            Ok(result) => success_response(id, result),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    fn handle_initialize(&self, params: &Value) -> Value {
        let protocol = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL_VERSION)
            .to_string();
        json!({
            "protocolVersion": protocol,
            // The tool catalogue is fixed for the process lifetime, so we never
            // send `notifications/tools/list_changed`.
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": self.server_name,
                "title": self.server_title,
                "version": SERVER_VERSION,
            },
            // Server-level usage notes; clients surface these to the model
            // before it picks a tool.
            "instructions": INSTRUCTIONS,
        })
    }

    fn handle_tools_call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "invalid params: missing tool name".to_string()))?
            .to_string();
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        // Every tool — built in and injected — is dispatched from here, so one
        // check covers the lot. Refusal is a tool error rather than a JSON-RPC
        // error: the caller is usually an agent, and it can only relay the
        // reason to the user if it comes back as content.
        if let Some(gate) = self.gate.clone() {
            if let Err(message) = gate(&name, &args) {
                return Ok(tool_result(Err(message)));
            }
        }

        let result = match name.as_str() {
            "open_docx" => self.tool_open(&args),
            "apply_commands" => self.tool_apply(&args),
            "replace_text" => self.tool_replace_text(&args),
            "undo" => self.tool_undo(&args),
            "redo" => self.tool_redo(&args),
            // Review-workflow convenience tools — thin wrappers over the
            // matching HeadlessCommand (also reachable via apply_commands).
            "start_change_tracking" => self.apply_named_command("start_change_tracking", &args),
            "stop_change_tracking" => self.apply_named_command("stop_change_tracking", &args),
            "accept_revisions" => self.apply_named_command("accept_revisions", &args),
            "reject_revisions" => self.apply_named_command("reject_revisions", &args),
            "add_comment" => self.apply_named_command("add_comment", &args),
            "comment_on_text" => self.apply_named_command("add_comment_by_text", &args),
            "comment_by_para_id" => self.apply_named_command("add_comment_by_para_id", &args),
            "reply_to_comment" => self.apply_named_command("reply_to_comment", &args),
            "resolve_comment" => self.apply_named_command("resolve_comment", &args),
            "update_comment" => self.apply_named_command("update_comment", &args),
            "delete_comment" => self.apply_named_command("delete_comment", &args),
            "get_paragraph_by_para_id" => self.tool_get_paragraph_by_para_id(&args),
            "query" => self.tool_query(&args),
            "export_docx" => self.tool_export(&args),
            "close_docx" => self.tool_close(&args),
            "list_sessions" => Ok(self.tool_list_sessions()),
            other => match self.call_extension(other, &args) {
                Some(result) => result,
                None => return Err((-32602, format!("unknown tool: {other}"))),
            },
        };

        Ok(tool_result(result))
    }

    /// Runs a registered tool, or `None` when no tool by that name exists.
    ///
    /// The session is resolved here so an injected tool never re-implements
    /// session lookup, and an unknown id fails with the same message the
    /// built-in tools produce.
    fn call_extension(&mut self, name: &str, args: &Value) -> Option<Result<Value, String>> {
        // The handler is cloned out first: it lives in `self`, and calling it
        // needs `self.sessions` borrowed mutably at the same time.
        let handler = match &self
            .extensions
            .iter()
            .find(|extension| extension.spec.name == name)?
            .handler
        {
            RegisteredHandler::Session(handler) => RegisteredHandler::Session(Arc::clone(handler)),
            RegisteredHandler::Store(handler) => RegisteredHandler::Store(Arc::clone(handler)),
        };
        Some((|| match handler {
            RegisteredHandler::Session(handler) => {
                let session = self.session_mut(args)?;
                let mut context = ToolContext {
                    document: &mut session.doc,
                    selection: &mut session.selection,
                    executor: &mut session.executor,
                };
                handler(&mut context, args)
            }
            RegisteredHandler::Store(handler) => {
                let mut sessions = Sessions {
                    sessions: &mut self.sessions,
                    next_id: &mut self.next_id,
                };
                handler(&mut sessions, args)
            }
        })())
    }

    // --- tools -------------------------------------------------------------

    fn tool_open(&mut self, args: &Value) -> Result<Value, String> {
        let bytes = load_input_bytes(&self.file_access, args)?;
        let doc = block_on(WordDocument::load(&bytes, &Options::default()))
            .map_err(|err| format!("failed to load docx: {err}"))?;
        let stats = document_stats(&doc)?;
        let session_id = self.alloc_session_id();
        self.sessions.insert(
            session_id.clone(),
            Session {
                doc,
                executor: CommandExecutor::new(),
                selection: None,
            },
        );
        Ok(json!({ "session_id": session_id, "stats": stats }))
    }

    fn tool_apply(&mut self, args: &Value) -> Result<Value, String> {
        let commands_value = args.get("commands").cloned().unwrap_or_else(|| json!([]));
        let commands: Vec<HeadlessCommand> = serde_json::from_value(commands_value)
            .map_err(|err| format!("invalid commands: {err}"))?;
        let applied = commands.len();
        let queries = parse_queries_arg(args)?;
        let include_diff = args
            .get("include_diff")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let dry_run = args
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let snapshot_coordinates = match args.get("coordinates") {
            None => false,
            Some(Value::String(value)) if value == "sequential" => false,
            Some(Value::String(value)) if value == "snapshot" => true,
            Some(_) => {
                return Err("coordinates must be either `sequential` or `snapshot`".to_string());
            }
        };
        let request = HeadlessEditRequest { commands };

        if dry_run {
            let session = self.session_ref(args)?;
            let mut doc = session.doc.clone();
            let mut executor = CommandExecutor::new();
            let before_stats = include_diff
                .then(|| document_stats_payload(&doc))
                .transpose()?;
            let before_paragraphs = include_diff.then(|| paragraph_texts(&doc));
            let mut selection = session.selection;

            selection = if snapshot_coordinates {
                apply_snapshot_headless_commands_with_executor_to_document(
                    &mut doc,
                    &mut executor,
                    selection,
                    &request,
                )
            } else {
                apply_headless_commands_with_executor_to_document(
                    &mut doc,
                    &mut executor,
                    selection,
                    &request,
                )
            }
            .map_err(|err| format!("apply failed: {err}"))?;

            let mut response = json!({
                "ok": true,
                "dry_run": true,
                "applied": applied,
                // A command that succeeds without changing the document (for
                // example re-applying the value a property already has)
                // records no history; this counts the steps that did.
                "recorded_undo_steps": executor.undo_depth(),
                "can_undo": session.executor.can_undo(),
                "can_redo": session.executor.can_redo(),
            });
            if !queries.is_empty() {
                response["query"] = serde_json::to_value(
                    execute_headless_queries_on_document(&doc, &HeadlessQueryRequest { queries })
                        .map_err(|err| format!("query failed: {err}"))?,
                )
                .map_err(|err| err.to_string())?;
            }
            if include_diff {
                let after_paragraphs = paragraph_texts(&doc);
                response["diff"] = json!({
                    "before": before_stats.unwrap_or_else(|| json!({})),
                    "after": document_stats_payload(&doc)?,
                    "changed_paragraphs": changed_paragraph_indices(
                        before_paragraphs.as_deref().unwrap_or(&[]),
                        &after_paragraphs,
                    ),
                });
            }
            let _ = selection;
            return Ok(response);
        }

        // Validate a real multi-command batch (and its post-edit queries) on
        // a throw-away document before touching the live session. Individual
        // commands are internally atomic, but without this preflight a later
        // coordinate/schema error could leave earlier commands and undo steps
        // applied even though apply_commands returned an error.
        if request.commands.len() > 1 || !queries.is_empty() {
            let session = self.session_ref(args)?;
            let mut preview_doc = session.doc.clone();
            let mut preview_executor = CommandExecutor::new();
            let preview_selection = if snapshot_coordinates {
                apply_snapshot_headless_commands_with_executor_to_document(
                    &mut preview_doc,
                    &mut preview_executor,
                    session.selection,
                    &request,
                )
            } else {
                apply_headless_commands_with_executor_to_document(
                    &mut preview_doc,
                    &mut preview_executor,
                    session.selection,
                    &request,
                )
            }
            .map_err(|err| format!("apply failed: {err}"))?;
            if !queries.is_empty() {
                execute_headless_queries_on_document(
                    &preview_doc,
                    &HeadlessQueryRequest {
                        queries: queries.clone(),
                    },
                )
                .map_err(|err| format!("query failed: {err}"))?;
            }
            let _ = preview_selection;
        }

        let before_stats = include_diff
            .then(|| {
                let session = self.session_ref(args)?;
                document_stats_payload(&session.doc)
            })
            .transpose()?;
        let before_paragraphs = include_diff
            .then(|| {
                let session = self.session_ref(args)?;
                Ok::<_, String>(paragraph_texts(&session.doc))
            })
            .transpose()?;

        let session = self.session_mut(args)?;
        let undo_depth_before = session.executor.undo_depth();
        // Route through the executor so each command lands on the undo stack;
        // the returned selection is threaded into the next call.
        session.selection = if snapshot_coordinates {
            apply_snapshot_headless_commands_with_executor_to_document(
                &mut session.doc,
                &mut session.executor,
                session.selection.take(),
                &request,
            )
        } else {
            apply_headless_commands_with_executor_to_document(
                &mut session.doc,
                &mut session.executor,
                session.selection.take(),
                &request,
            )
        }
        .map_err(|err| format!("apply failed: {err}"))?;
        let mut response = json!({
            "ok": true,
            "applied": applied,
            // A command that succeeds without changing the document (for
            // example re-applying the value a property already has) records
            // no history; this counts the steps that did.
            "recorded_undo_steps":
                session.executor.undo_depth().saturating_sub(undo_depth_before),
            "can_undo": session.executor.can_undo(),
            "can_redo": session.executor.can_redo(),
        });
        if !queries.is_empty() {
            response["query"] = serde_json::to_value(
                execute_headless_queries_on_document(
                    &session.doc,
                    &HeadlessQueryRequest { queries },
                )
                .map_err(|err| format!("query failed: {err}"))?,
            )
            .map_err(|err| err.to_string())?;
        }
        if include_diff {
            let after_paragraphs = paragraph_texts(&session.doc);
            response["diff"] = json!({
                "before": before_stats.unwrap_or_else(|| json!({})),
                "after": document_stats_payload(&session.doc)?,
                "changed_paragraphs": changed_paragraph_indices(
                    before_paragraphs.as_deref().unwrap_or(&[]),
                    &after_paragraphs,
                ),
            });
        }
        Ok(response)
    }

    /// Find-and-replace all (non-overlapping) literal occurrences of `find`
    /// with `replace` — optionally ignoring ASCII case. Matches are located on the current document, then the
    /// resulting `ReplaceRange` edits are applied **highest-offset first** so a
    /// length-changing replacement never invalidates a not-yet-applied match's
    /// offsets. Matches that span multiple runs are handled by the underlying
    /// run-splitting edit (covered by property tests).
    fn tool_replace_text(&mut self, args: &Value) -> Result<Value, String> {
        let find = args
            .get("find")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("replace_text requires a non-empty `find`")?
            .to_string();
        let replace = args
            .get("replace")
            .and_then(Value::as_str)
            .ok_or("replace_text requires `replace` (use \"\" to delete the matches)")?
            .to_string();
        let start_paragraph = args
            .get("start_paragraph")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(0);
        let end_paragraph = args
            .get("end_paragraph")
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let max = args
            .get("max_replacements")
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let regex = args.get("regex").and_then(Value::as_bool).unwrap_or(false);
        let dry_run = args
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // 1) Locate the matches and the text each one becomes.
        let mut targets = if regex {
            self.regex_targets(
                args,
                &find,
                &replace,
                case_insensitive,
                start_paragraph,
                end_paragraph,
            )?
        } else {
            self.literal_targets(
                args,
                &find,
                &replace,
                case_insensitive,
                start_paragraph,
                end_paragraph,
            )?
        };
        if let Some(max) = max {
            targets.truncate(max); // first N in document order
        }
        self.reject_targets_over_foreign_revisions(args, &targets)?;
        let replaced = targets.len();
        let matches = report_matches(&targets);

        if replaced == 0 {
            return Ok(json!({
                "replaced": 0,
                "dry_run": dry_run,
                "matches": [],
                "can_undo": false,
                "can_redo": false,
            }));
        }
        if dry_run {
            // Nothing is applied: the caller is asking what this replacement
            // would do before letting it touch the document.
            let session = self.session_ref(args)?;
            return Ok(json!({
                "replaced": replaced,
                "dry_run": true,
                "matches": matches,
                "can_undo": session.executor.can_undo(),
                "can_redo": session.executor.can_redo(),
            }));
        }

        // 2) Apply highest-offset first (descending) so earlier, not-yet-applied
        //    match offsets stay valid however the replacement changes length.
        targets.sort_by(|left, right| {
            (right.paragraph, right.start_offset).cmp(&(left.paragraph, left.start_offset))
        });
        let commands: Vec<HeadlessCommand> = targets
            .iter()
            .map(|target| {
                serde_json::from_value(json!({
                    "type": "replace_range",
                    "start_paragraph": target.paragraph,
                    "start_offset": target.start_offset,
                    "end_paragraph": target.paragraph,
                    "end_offset": target.end_offset,
                    "text": target.replacement,
                }))
                .map_err(|err| err.to_string())
            })
            .collect::<Result<_, String>>()?;

        let session = self.session_mut(args)?;
        session.selection = apply_headless_commands_with_executor_to_document(
            &mut session.doc,
            &mut session.executor,
            session.selection.take(),
            &HeadlessEditRequest { commands },
        )
        .map_err(|err| format!("replace failed: {err}"))?;
        Ok(json!({
            "replaced": replaced,
            "dry_run": false,
            "matches": matches,
            "can_undo": session.executor.can_undo(),
            "can_redo": session.executor.can_redo(),
        }))
    }

    /// Refuses a replacement that would rewrite someone else's tracked change.
    ///
    /// A match can land inside a revision the agent does not own: text another
    /// reviewer has marked deleted, or text they have proposed and not had
    /// accepted yet. Replacing it destroys their change — the deletion
    /// disappears from the file, or the new wording is silently published under
    /// their name. Neither is something a caller can detect afterwards, so the
    /// call fails and says which revision is in the way.
    ///
    /// A revision by the session's own change-tracking author is fair game:
    /// that is the agent editing its own pending redline.
    fn reject_targets_over_foreign_revisions(
        &self,
        args: &Value,
        targets: &[ReplaceTarget],
    ) -> Result<(), String> {
        if targets.is_empty() {
            return Ok(());
        }
        let session = self.session_ref(args)?;
        let own_author = session
            .doc
            .revision_session()
            .and_then(|revision| revision.author.clone());

        let queries: Vec<HeadlessQuery> = serde_json::from_value(json!([{
            "type": "revisions",
            "start_revision": 0,
            "max_revisions": 100_000,
        }]))
        .map_err(|err| err.to_string())?;
        let response =
            execute_headless_queries_on_document(&session.doc, &HeadlessQueryRequest { queries })
                .map_err(|err| format!("revision lookup failed: {err}"))?;
        let response = serde_json::to_value(response).map_err(|err| err.to_string())?;
        let Some(revisions) = response["results"][0]["revisions"].as_array() else {
            return Ok(());
        };

        for target in targets {
            for revision in revisions {
                if revision["paragraph"].as_u64().map(|p| p as usize) != Some(target.paragraph) {
                    continue;
                }
                let (start, end) = (
                    revision["start_offset"].as_u64().unwrap_or(0) as usize,
                    revision["end_offset"].as_u64().unwrap_or(0) as usize,
                );
                // Half-open ranges: touching at an edge is not an overlap.
                if target.end_offset <= start || target.start_offset >= end {
                    continue;
                }
                let author = revision["author"].as_str();
                if author.is_some() && author == own_author.as_deref() {
                    continue;
                }
                let kind = revision["kind"].as_str().unwrap_or("tracked");
                let who = author.unwrap_or("an unknown author");
                return Err(format!(
                    "replace_text refused: the match in paragraph {} at {}..{} overlaps a tracked {} by {}. \
                     Accept or reject that change first (accept_revisions / reject_revisions), or scope \
                     the replacement past it with start_paragraph / end_paragraph.",
                    target.paragraph, target.start_offset, target.end_offset, kind, who,
                ));
            }
        }
        Ok(())
    }

    /// Literal matches, located by the shared `search_text` query so literal
    /// replacement and literal search agree on what a match is.
    fn literal_targets(
        &self,
        args: &Value,
        find: &str,
        replace: &str,
        case_insensitive: bool,
        start_paragraph: usize,
        end_paragraph: Option<usize>,
    ) -> Result<Vec<ReplaceTarget>, String> {
        let search: Vec<HeadlessQuery> = serde_json::from_value(json!([{
            "type": "search_text",
            "query": find,
            "start_paragraph": start_paragraph,
            "max_results": 1_000_000,
            "context_before_chars": 0,
            "context_after_chars": 0,
            "case_insensitive": case_insensitive,
        }]))
        .map_err(|err| err.to_string())?;
        let response = {
            let doc = &self.session_ref(args)?.doc;
            execute_headless_queries_on_document(doc, &HeadlessQueryRequest { queries: search })
                .map_err(|err| format!("search failed: {err}"))?
        };
        let response = serde_json::to_value(response).map_err(|err| err.to_string())?;

        Ok(response["results"][0]["matches"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|found| {
                Some(ReplaceTarget {
                    paragraph: found["paragraph"].as_u64()? as usize,
                    start_offset: found["start_offset"].as_u64()? as usize,
                    end_offset: found["end_offset"].as_u64()? as usize,
                    matched: found["matched_text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    replacement: replace.to_string(),
                })
            })
            .filter(|target| end_paragraph.is_none_or(|last| target.paragraph <= last))
            .collect())
    }

    /// Regex matches, with `$1` / `${name}` expanded from the captures.
    ///
    /// The pattern runs here rather than through the shared query API on
    /// purpose: the query API is compiled into the WebAssembly editor core,
    /// where a regex engine would be paid for in bundle size by every browser
    /// user. This server is a native binary and pays nothing.
    fn regex_targets(
        &self,
        args: &Value,
        find: &str,
        replace: &str,
        case_insensitive: bool,
        start_paragraph: usize,
        end_paragraph: Option<usize>,
    ) -> Result<Vec<ReplaceTarget>, String> {
        let pattern = regex::RegexBuilder::new(find)
            .case_insensitive(case_insensitive)
            .build()
            .map_err(|err| format!("invalid regex: {err}"))?;

        let doc = &self.session_ref(args)?.doc;
        let last = end_paragraph.unwrap_or(usize::MAX);
        let queries: Vec<HeadlessQuery> = serde_json::from_value(json!([{
            "type": "paragraph_slice",
            "start_paragraph": start_paragraph,
            "max_paragraphs": usize::MAX,
        }]))
        .map_err(|err| err.to_string())?;
        let response = execute_headless_queries_on_document(doc, &HeadlessQueryRequest { queries })
            .map_err(|err| format!("read failed: {err}"))?;
        let response = serde_json::to_value(response).map_err(|err| err.to_string())?;

        let mut targets = Vec::new();
        for paragraph in response["results"][0]["paragraphs"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let Some(index) = paragraph["paragraph"].as_u64().map(|n| n as usize) else {
                continue;
            };
            if index < start_paragraph || index > last {
                continue;
            }
            let text = paragraph["text"].as_str().unwrap_or_default();
            for found in pattern.captures_iter(text) {
                let whole = found.get(0).expect("capture 0 always exists");
                let mut replacement = String::new();
                found.expand(replace, &mut replacement);
                targets.push(ReplaceTarget {
                    paragraph: index,
                    start_offset: whole.start(),
                    end_offset: whole.end(),
                    matched: whole.as_str().to_string(),
                    replacement,
                });
            }
        }
        Ok(targets)
    }

    /// Resolve a paragraph by its stable `w14:paraId` (getElementById-style),
    /// returning its current positional index and snapshot, or nulls if no
    /// paragraph carries the id.
    fn tool_get_paragraph_by_para_id(&self, args: &Value) -> Result<Value, String> {
        let para_id = args
            .get("para_id")
            .and_then(Value::as_str)
            .ok_or("get_paragraph_by_para_id requires `para_id`")?;
        let queries: Vec<HeadlessQuery> = serde_json::from_value(json!([{
            "type": "paragraph_by_para_id",
            "para_id": para_id,
        }]))
        .map_err(|err| err.to_string())?;
        let doc = &self.session_ref(args)?.doc;
        let response = execute_headless_queries_on_document(doc, &HeadlessQueryRequest { queries })
            .map_err(|err| format!("query failed: {err}"))?;
        let response = serde_json::to_value(response).map_err(|err| err.to_string())?;
        Ok(response["results"][0].clone())
    }

    /// Apply a single named [`HeadlessCommand`] whose body is the tool's
    /// arguments (minus `session_id`) tagged with `type`. Used by the
    /// review-workflow convenience tools so their argument names line up 1:1
    /// with the command's fields.
    fn apply_named_command(&mut self, type_name: &str, args: &Value) -> Result<Value, String> {
        let mut command = args.clone();
        match command.as_object_mut() {
            Some(object) => {
                object.remove("session_id");
                object.insert("type".to_string(), json!(type_name));
            }
            None => return Err("arguments must be an object".to_string()),
        }
        let parsed: HeadlessCommand = serde_json::from_value(command)
            .map_err(|err| format!("invalid {type_name} arguments: {err}"))?;

        let session = self.session_mut(args)?;
        let request = HeadlessEditRequest {
            commands: vec![parsed],
        };
        session.selection = apply_headless_commands_with_executor_to_document(
            &mut session.doc,
            &mut session.executor,
            session.selection.take(),
            &request,
        )
        .map_err(|err| format!("{type_name} failed: {err}"))?;
        Ok(json!({
            "ok": true,
            "can_undo": session.executor.can_undo(),
            "can_redo": session.executor.can_redo(),
        }))
    }

    fn tool_undo(&mut self, args: &Value) -> Result<Value, String> {
        let count = undo_redo_count(args)?;
        let session = self.session_mut(args)?;
        let mut steps = 0usize;
        for _ in 0..count {
            let stepped = {
                let mut ctx = CommandContext::new(&mut session.doc, &mut session.selection);
                session
                    .executor
                    .undo(&mut ctx)
                    .map_err(|err| format!("undo failed: {err}"))?
            };
            if !stepped {
                break; // nothing left on the undo stack
            }
            steps += 1;
        }
        Ok(json!({
            "undone": steps > 0,
            "steps": steps,
            "can_undo": session.executor.can_undo(),
            "can_redo": session.executor.can_redo(),
        }))
    }

    fn tool_redo(&mut self, args: &Value) -> Result<Value, String> {
        let count = undo_redo_count(args)?;
        let session = self.session_mut(args)?;
        let mut steps = 0usize;
        for _ in 0..count {
            let stepped = {
                let mut ctx = CommandContext::new(&mut session.doc, &mut session.selection);
                session
                    .executor
                    .redo(&mut ctx)
                    .map_err(|err| format!("redo failed: {err}"))?
            };
            if !stepped {
                break; // nothing left on the redo stack
            }
            steps += 1;
        }
        Ok(json!({
            "redone": steps > 0,
            "steps": steps,
            "can_undo": session.executor.can_undo(),
            "can_redo": session.executor.can_redo(),
        }))
    }

    fn tool_query(&self, args: &Value) -> Result<Value, String> {
        let queries_value = args.get("queries").cloned().unwrap_or_else(|| json!([]));
        let queries: Vec<HeadlessQuery> = serde_json::from_value(queries_value)
            .map_err(|err| format!("invalid queries: {err}"))?;
        let doc = &self.session_ref(args)?.doc;
        let request = HeadlessQueryRequest { queries };
        let response = execute_headless_queries_on_document(doc, &request)
            .map_err(|err| format!("query failed: {err}"))?;
        serde_json::to_value(response).map_err(|err| err.to_string())
    }

    fn tool_export(&self, args: &Value) -> Result<Value, String> {
        let path = args.get("path").and_then(Value::as_str).map(str::to_string);
        let doc = &self.session_ref(args)?.doc;
        let bytes = doc
            .export_docx()
            .map_err(|err| format!("export failed: {err}"))?;
        let size = bytes.len();
        match path {
            Some(path) => {
                let (written, mut file) = self.file_access.create_new(&path)?;
                if let Err(err) = std::io::Write::write_all(&mut file, &bytes) {
                    // Do not leave a truncated .docx behind under the new name.
                    drop(file);
                    let _ = std::fs::remove_file(&written);
                    return Err(format!("failed to write {path}: {err}"));
                }
                Ok(json!({ "path": written.to_string_lossy(), "bytes": size }))
            }
            None => Ok(json!({ "bytes_base64": BASE64.encode(&bytes), "bytes": size })),
        }
    }

    fn tool_close(&mut self, args: &Value) -> Result<Value, String> {
        let id = require_session_id(args)?;
        let closed = self.sessions.remove(&id).is_some();
        Ok(json!({ "session_id": id, "closed": closed }))
    }

    /// Everything this server publishes: the built-in tools followed by the
    /// injected ones, in registration order.
    fn tool_catalogue(&self) -> Value {
        let mut tools = TOOLS.iter().map(tool_definition).collect::<Vec<_>>();
        tools.extend(self.extensions.iter().map(extension_definition));
        Value::Array(tools)
    }

    fn tool_list_sessions(&self) -> Value {
        let mut ids: Vec<&str> = self.sessions.keys().map(String::as_str).collect();
        ids.sort_unstable();
        json!({ "session_ids": ids })
    }

    // --- helpers -----------------------------------------------------------

    fn alloc_session_id(&mut self) -> String {
        self.next_id += 1;
        format!("doc-{}", self.next_id)
    }

    fn session_mut(&mut self, args: &Value) -> Result<&mut Session, String> {
        let id = require_session_id(args)?;
        self.sessions
            .get_mut(&id)
            .ok_or_else(|| format!("unknown session_id: {id}"))
    }

    fn session_ref(&self, args: &Value) -> Result<&Session, String> {
        let id = require_session_id(args)?;
        self.sessions
            .get(&id)
            .ok_or_else(|| format!("unknown session_id: {id}"))
    }
}

/// Optional `count` argument for `undo` / `redo` (how many history steps to
/// take). Defaults to 1; must be a non-negative integer.
fn undo_redo_count(args: &Value) -> Result<usize, String> {
    match args.get("count") {
        None | Some(Value::Null) => Ok(1),
        Some(value) => value
            .as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| "count must be a non-negative integer".to_string()),
    }
}

fn require_session_id(args: &Value) -> Result<String, String> {
    args.get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "missing required argument: session_id".to_string())
}

fn load_input_bytes(access: &FileAccess, args: &Value) -> Result<Vec<u8>, String> {
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        let resolved = access.resolve_existing(path)?;
        return std::fs::read(resolved).map_err(|err| format!("failed to read {path}: {err}"));
    }
    if let Some(encoded) = args.get("bytes_base64").and_then(Value::as_str) {
        return BASE64
            .decode(encoded)
            .map_err(|err| format!("invalid bytes_base64: {err}"));
    }
    Err("open_docx requires either `path` or `bytes_base64`".to_string())
}

fn parse_queries_arg(args: &Value) -> Result<Vec<HeadlessQuery>, String> {
    let queries_value = args.get("queries").cloned().unwrap_or_else(|| json!([]));
    serde_json::from_value(queries_value).map_err(|err| format!("invalid queries: {err}"))
}

/// Run the built-in `document_stats` query so `open_docx` can hand the client a
/// quick overview (paragraph/table/word counts) without a second round trip.
fn document_stats(doc: &WordDocument) -> Result<Value, String> {
    let queries: Vec<HeadlessQuery> = serde_json::from_value(json!([{ "type": "document_stats" }]))
        .map_err(|err| err.to_string())?;
    let response = execute_headless_queries_on_document(doc, &HeadlessQueryRequest { queries })
        .map_err(|err| format!("stats query failed: {err}"))?;
    serde_json::to_value(response.results.into_iter().next()).map_err(|err| err.to_string())
}

fn document_stats_payload(doc: &WordDocument) -> Result<Value, String> {
    let stats_result = document_stats(doc)?;
    Ok(stats_result
        .get("stats")
        .cloned()
        .unwrap_or_else(|| stats_result.clone()))
}

fn paragraph_texts(doc: &WordDocument) -> Vec<String> {
    (0..doc.paragraph_count())
        .map(|index| {
            doc.paragraph(index)
                .map(|paragraph| {
                    paragraph
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>()
                })
                .unwrap_or_default()
        })
        .collect()
}

fn changed_paragraph_indices(before: &[String], after: &[String]) -> Vec<usize> {
    let max = before.len().max(after.len());
    (0..max)
        .filter(|index| before.get(*index) != after.get(*index))
        .collect()
}

/// Wrap a tool outcome into the MCP `tools/call` result shape. Tool execution
/// failures are reported as a result with `isError: true` (per the MCP
/// convention), not as a JSON-RPC protocol error.
///
/// Successes carry the payload twice: as `structuredContent` (matching the
/// tool's advertised `outputSchema`) for clients that parse results, and as the
/// pretty-printed text block older clients still expect.
fn tool_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": pretty(&value) }],
            "structuredContent": value,
            "isError": false,
        }),
        Err(message) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        }),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn success_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

// --- tool catalogue --------------------------------------------------------
//
// Everything a client can learn about this server arrives through `initialize`
// (server instructions) and `tools/list` (per-tool JSON Schema), so this
// catalogue *is* the published interface: every tool, every argument, every
// result field, and every `HeadlessCommand` / `HeadlessQuery` variant that
// `apply_commands` / `query` accept. The `catalogue_covers_every_*` tests keep
// it in step with the enums in `jade-docx-headless-api`.

/// Usage notes returned from `initialize`. Clients show these to the model
/// before any tool call, so they carry the conventions that would otherwise
/// have to be repeated in every tool description.
const INSTRUCTIONS: &str = "\
jade-docx-headless-mcp edits real .docx files in memory. Parts the edit does not \
touch keep their original XML, so tracked changes, comments and field codes \
survive an open → edit → export round trip (they do not with python-docx or \
markdown/pandoc pipelines).\n\
\n\
Workflow: `open_docx` returns a `session_id`; `query` and `apply_commands` \
then work on that resident document as many times as needed; `export_docx` \
serialises it back to .docx and `close_docx` frees it. Sessions live only for \
the lifetime of this process.\n\
\n\
Conventions shared by every tool:\n\
- `paragraph` is a zero-based index into the document's flattened paragraph \
order; paragraphs inside tables are part of that order.\n\
- `offset` is a zero-based UTF-8 *byte* offset inside a paragraph's text, and \
range ends (`end_offset`) are exclusive. One character is often several bytes, \
so take offsets from a `search_text` or `paragraph_slice` result rather than \
counting: an offset past the end of the paragraph, or inside a character, is \
rejected rather than moved to the nearest legal position.\n\
- `table_order_index` is a zero-based index over the body's top-level tables \
in document order.\n\
- Measurements (font size, widths, margins, indents, spacing) are in points; \
colors are `#RRGGBB`.\n\
- Positional indices shift as the document is edited, including between \
commands in one `apply_commands` batch. The default `coordinates: sequential` \
keeps the existing constructive semantics: every command addresses the state \
left by the previous command. Use `coordinates: snapshot` when all coordinates \
came from one query of the batch's starting document; supported text edits \
and text-range formatting are rebased safely, while ambiguous overlaps and \
unsupported commands are rejected before anything changes. Every real batch \
is preflighted on a clone, so an error in a later command or verification query \
leaves the live document and undo history unchanged. For anchors that \
must \
survive edits, address paragraphs by their stable `w14:paraId` via \
`get_paragraph_by_para_id` / `comment_by_para_id`.\n\
- Every mutating call pushes one undo step per command — comment and revision \
operations included; `undo` / `redo` walk that history, and `apply_commands` \
with `dry_run: true` runs a batch against a throw-away clone instead.\n\
- Review is a conversation: answer a point with `reply_to_comment` so it stays \
in one Word thread, and close it with `resolve_comment` rather than deleting \
it. `comment_threads` reads the exchange; `comments` with `resolved: false` is \
the backlog.\n\
- Tool failures come back as a normal result with `isError: true` and a text \
message. JSON-RPC errors are reserved for protocol-level problems.\n\
- Turn on `start_change_tracking` before editing to make the agent's own edits \
reviewable Word tracked changes.\n";

/// JSON Schema shape of one argument. A closed set, so every catalogue entry
/// describes its type the same way.
#[derive(Clone, Copy)]
enum ArgType {
    /// Non-negative integer: index, offset or count.
    Index,
    /// Integer that may be negative.
    Int,
    /// Measurement in points.
    Number,
    Text,
    Flag,
    /// Text, or `null` to clear the underlying property.
    NullableText,
    /// Number, or `null` to clear the underlying property.
    NullableNumber,
    /// Text restricted to a fixed set of values.
    Choice(&'static [&'static str]),
    /// Text restricted to a fixed set of values, or `null` to clear it.
    NullableChoice(&'static [&'static str]),
    NumberList,
    TextList,
    /// Styled paragraphs accepted by `insert_cover_page`.
    CoverParagraphList,
    /// One-to-nine custom numbering levels accepted by
    /// `apply_custom_multilevel_list`.
    CustomNumberingLevelList,
    /// One Word bibliography source accepted by `add_citation_source`.
    CitationSource,
    /// Word editing-restriction metadata, or null to remove it.
    DocumentProtection,
    /// Editable package-wide DrawingML theme fonts and colors.
    DocumentTheme,
    /// `[{"row": …, "column": …, "column_span": …}, …]`
    CellList,
    /// `[{"column": …, "kind": "text"|"number"|"date", "descending": …}, …]`
    SortKeyList,
    /// Free-form JSON object.
    Object,
    /// One table border edge: `{ style, width, color }`, or null.
    BorderEdge,
    /// `HeadlessCommand` objects — a `oneOf` over [`COMMANDS`].
    CommandList,
    /// `HeadlessQuery` objects — a `oneOf` over [`QUERIES`].
    QueryList,
    /// Array of `HeadlessQueryResult` objects.
    ResultList,
    /// Array of replacement matches, as `replace_text` reports them.
    MatchList,
}

impl ArgType {
    fn schema(self) -> Value {
        match self {
            ArgType::Index => json!({ "type": "integer", "minimum": 0 }),
            ArgType::Int => json!({ "type": "integer" }),
            ArgType::Number => json!({ "type": "number" }),
            ArgType::Text => json!({ "type": "string" }),
            ArgType::Flag => json!({ "type": "boolean" }),
            ArgType::NullableText => json!({ "type": ["string", "null"] }),
            ArgType::NullableNumber => json!({ "type": ["number", "null"] }),
            ArgType::Choice(values) => json!({ "type": "string", "enum": values }),
            ArgType::NullableChoice(values) => json!({
                "anyOf": [
                    { "type": "string", "enum": values },
                    { "type": "null" }
                ]
            }),
            ArgType::NumberList => json!({ "type": "array", "items": { "type": "number" } }),
            ArgType::TextList => json!({ "type": "array", "items": { "type": "string" } }),
            ArgType::CoverParagraphList => json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Paragraph text." },
                        "bold": { "type": "boolean", "description": "Whether the paragraph text is bold." },
                        "italic": { "type": "boolean", "description": "Whether the paragraph text is italic." },
                        "color": { "type": ["string", "null"], "description": "Text colour as #RRGGBB." },
                        "font_size": { "type": ["number", "null"], "description": "Font size in points." },
                        "alignment": { "type": ["string", "null"], "enum": ["left", "center", "right", null], "description": "Paragraph alignment." },
                        "space_before": { "type": ["number", "null"], "description": "Spacing before in points." },
                        "space_after": { "type": ["number", "null"], "description": "Spacing after in points." },
                        "shading_color": { "type": ["string", "null"], "description": "Paragraph shading as #RRGGBB." },
                        "border_top": { "type": "boolean", "description": "Draw a top border." },
                        "border_bottom": { "type": "boolean", "description": "Draw a bottom border." },
                        "border_left": { "type": "boolean", "description": "Draw a left border." },
                        "border_right": { "type": "boolean", "description": "Draw a right border." },
                        "indent_left": { "type": ["number", "null"], "description": "Left indent in points." },
                        "indent_right": { "type": ["number", "null"], "description": "Right indent in points." }
                    },
                    "additionalProperties": false
                }
            }),
            ArgType::CustomNumberingLevelList => json!({
                "type": "array",
                "minItems": 1,
                "maxItems": 9,
                "items": {
                    "type": "object",
                    "properties": {
                        "numFmt": { "type": "string", "minLength": 1, "description": "Word numbering format, such as decimal, upperRoman, or bullet." },
                        "lvlText": { "type": "string", "minLength": 1, "description": "Level marker template, using %1 through %9 placeholders or a bullet glyph." },
                        "indentLeft": { "type": "integer", "description": "Left indent in OOXML twips." },
                        "hanging": { "type": ["integer", "null"], "description": "Optional hanging indent in OOXML twips." },
                        "start": { "type": ["integer", "null"], "minimum": 1, "description": "Optional first number for this level." }
                    },
                    "required": ["numFmt", "lvlText", "indentLeft"],
                    "additionalProperties": false
                }
            }),
            ArgType::CitationSource => json!({
                "type": "object",
                "properties": {
                    "tag": { "type": "string", "description": "Stable unique Word citation tag." },
                    "type": { "type": "string", "enum": ["Book", "JournalArticle", "InternetSite", "Report", "Misc"], "description": "Word bibliography source type." },
                    "author": { "type": "string", "description": "Author name, preferably Last, First." },
                    "title": { "type": "string", "description": "Source title; must not be empty." },
                    "year": { "type": "string", "description": "Publication year as Word stores it." },
                    "publisher": { "type": ["string", "null"], "description": "Publisher, journal, or report institution when applicable." },
                    "url": { "type": ["string", "null"], "description": "Source URL for web sources." }
                },
                "required": ["tag", "type", "title"],
                "additionalProperties": false
            }),
            ArgType::DocumentProtection => json!({
                "type": ["object", "null"],
                "properties": {
                    "edit": { "type": "string", "enum": ["readOnly"], "description": "Supported Word editing mode." },
                    "enforced": { "type": "boolean", "description": "Must be true when protection is present." },
                    "hash": { "type": ["string", "null"], "description": "Optional base64 SHA-512 password verifier." },
                    "salt": { "type": ["string", "null"], "description": "Optional base64 16-byte salt; required with hash." },
                    "spinCount": { "type": ["integer", "null"], "minimum": 1, "maximum": 10000000, "description": "SHA-512 iteration count; defaults to 100000." },
                    "algorithmSid": { "type": ["integer", "null"], "enum": [14, null], "description": "Word algorithm id 14 (SHA-512)." }
                },
                "required": ["edit", "enforced"],
                "additionalProperties": false
            }),
            ArgType::DocumentTheme => json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "minLength": 1, "maxLength": 128, "description": "Theme, color-scheme, and font-scheme name." },
                    "majorFont": { "type": "string", "minLength": 1, "maxLength": 128, "description": "Latin major/headings font family." },
                    "minorFont": { "type": "string", "minLength": 1, "maxLength": 128, "description": "Latin minor/body font family." },
                    "eastAsiaFont": { "type": "string", "minLength": 1, "maxLength": 128, "description": "East Asian font family for both theme families." },
                    "dark2": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$", "description": "Secondary dark color as RRGGBB." },
                    "light2": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$", "description": "Secondary light color as RRGGBB." },
                    "accent1": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$", "description": "First accent color as RRGGBB." },
                    "accent2": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$", "description": "Second accent color as RRGGBB." },
                    "accent3": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$", "description": "Third accent color as RRGGBB." },
                    "accent4": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$", "description": "Fourth accent color as RRGGBB." },
                    "accent5": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$", "description": "Fifth accent color as RRGGBB." },
                    "accent6": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$", "description": "Sixth accent color as RRGGBB." }
                },
                "required": ["name", "majorFont", "minorFont", "eastAsiaFont", "dark2", "light2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6"],
                "additionalProperties": false
            }),
            ArgType::CellList => json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "row": { "type": "integer", "minimum": 0, "description": "Zero-based row index." },
                        "column": { "type": "integer", "minimum": 0, "description": "Zero-based logical-grid column index." },
                        "column_span": { "type": "integer", "minimum": 1, "description": "How many grid columns the cell occupies (1 when it is not merged)." }
                    },
                    "required": ["row", "column", "column_span"],
                    "additionalProperties": false
                }
            }),
            ArgType::SortKeyList => json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "column": { "type": "integer", "minimum": 0, "description": "Zero-based logical-grid column the key reads." },
                        "kind": { "type": "string", "enum": ["text", "number", "date"], "description": "How the column compares (Word's 種類). Defaults to text." },
                        "descending": { "type": "boolean", "description": "Sort this key from largest to smallest. Defaults to false." }
                    },
                    "required": ["column"],
                    "additionalProperties": false
                }
            }),
            ArgType::Object => json!({ "type": "object" }),
            ArgType::BorderEdge => json!({
                "type": ["object", "null"],
                "properties": {
                    "style": {
                        "type": "string",
                        "enum": ["single", "double", "dashed", "dotted", "thick", "wave", "dashSmallGap", "dotDash", "dotDotDash", "triple", "none", "nil"],
                        "description": "Line style Word draws; `none` removes the line. Defaults to single."
                    },
                    "width": { "type": "number", "minimum": 0, "description": "Line thickness in points." },
                    "color": { "type": "string", "description": "Line colour as #RRGGBB, or auto." }
                },
                "additionalProperties": false
            }),
            ArgType::CommandList => json!({
                "type": "array",
                "items": { "oneOf": variant_schemas(COMMANDS) }
            }),
            ArgType::QueryList => json!({
                "type": "array",
                "items": { "oneOf": variant_schemas(QUERIES) }
            }),
            ArgType::MatchList => json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "description": "One match: where it is and what it becomes.",
                    "properties": {
                        "paragraph": { "type": "integer", "minimum": 0, "description": "Zero-based paragraph index." },
                        "start_offset": { "type": "integer", "minimum": 0, "description": "UTF-8 byte offset where the match starts." },
                        "end_offset": { "type": "integer", "minimum": 0, "description": "Exclusive UTF-8 byte offset where it ends." },
                        "text": { "type": "string", "description": "The matched text as the document has it." },
                        "replacement": { "type": "string", "description": "What it was (or would be) replaced with." }
                    },
                    "required": ["paragraph", "start_offset", "end_offset", "text", "replacement"]
                }
            }),
            ArgType::ResultList => json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "description": "A HeadlessQueryResult, tagged with the same snake_case `type` as the query that produced it.",
                    "properties": {
                        "type": {
                            "type": "string",
                            "description": "The query variant this result answers, e.g. paragraph_text."
                        }
                    },
                    "required": ["type"]
                }
            }),
        }
    }
}

/// One named argument or result field: its shape, whether it is always present,
/// and the description clients show to the model.
struct Arg {
    name: &'static str,
    ty: ArgType,
    required: bool,
    description: &'static str,
}

const fn req(name: &'static str, ty: ArgType, description: &'static str) -> Arg {
    Arg {
        name,
        ty,
        required: true,
        description,
    }
}

const fn opt(name: &'static str, ty: ArgType, description: &'static str) -> Arg {
    Arg {
        name,
        ty,
        required: false,
        description,
    }
}

/// The session handle every tool but `open_docx` / `list_sessions` takes.
const SESSION: Arg = req(
    "session_id",
    ArgType::Text,
    "Session id returned by `open_docx`.",
);
/// Result field: whether the session's undo stack is non-empty afterwards.
const CAN_UNDO: Arg = req(
    "can_undo",
    ArgType::Flag,
    "Whether the session has anything left to undo.",
);
/// Result field: whether the session's redo stack is non-empty afterwards.
const CAN_REDO: Arg = req(
    "can_redo",
    ArgType::Flag,
    "Whether the session has anything left to redo.",
);

/// Arguments shared by commands that act on a text span.
const TEXT_RANGE: &[Arg] = &[
    req(
        "start_paragraph",
        ArgType::Index,
        "Zero-based index of the paragraph the range starts in.",
    ),
    req(
        "start_offset",
        ArgType::Index,
        "UTF-8 byte offset of the range start inside that paragraph's text.",
    ),
    req(
        "end_paragraph",
        ArgType::Index,
        "Zero-based index of the paragraph the range ends in.",
    ),
    req(
        "end_offset",
        ArgType::Index,
        "Exclusive UTF-8 byte offset of the range end inside that paragraph's text.",
    ),
];

/// Arguments shared by commands that act on whole paragraphs.
const PARAGRAPH_RANGE: &[Arg] = &[
    req(
        "start_paragraph",
        ArgType::Index,
        "Zero-based index of the first paragraph to change (inclusive).",
    ),
    req(
        "end_paragraph",
        ArgType::Index,
        "Zero-based index of the last paragraph to change (inclusive).",
    ),
];

/// A `type`-tagged variant of `HeadlessCommand` / `HeadlessQuery`.
struct Variant {
    name: &'static str,
    description: &'static str,
    /// Range arguments prepended to `args`; empty when the variant has none.
    scope: &'static [Arg],
    args: &'static [Arg],
}

/// A variant that carries only its own arguments.
const fn variant(name: &'static str, description: &'static str, args: &'static [Arg]) -> Variant {
    Variant {
        name,
        description,
        scope: &[],
        args,
    }
}

/// A variant scoped to a text span ([`TEXT_RANGE`] + `args`).
const fn span(name: &'static str, description: &'static str, args: &'static [Arg]) -> Variant {
    Variant {
        name,
        description,
        scope: TEXT_RANGE,
        args,
    }
}

/// A variant scoped to a paragraph range ([`PARAGRAPH_RANGE`] + `args`).
const fn paragraphs(
    name: &'static str,
    description: &'static str,
    args: &'static [Arg],
) -> Variant {
    Variant {
        name,
        description,
        scope: PARAGRAPH_RANGE,
        args,
    }
}

/// Every `HeadlessCommand` variant `apply_commands` accepts.
const COMMANDS: &[Variant] = &[
    // -- text -------------------------------------------------------------
    variant(
        "insert_text",
        "Insert text at a byte offset inside a paragraph.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req(
                "offset",
                ArgType::Index,
                "UTF-8 byte offset in the paragraph's text; 0 inserts before the first character.",
            ),
            req("text", ArgType::Text, "Text to insert."),
        ],
    ),
    span(
        "replace_range",
        "Replace everything between two positions with new text — the general text-edit primitive. Ranges may span paragraphs.",
        &[req(
            "text",
            ArgType::Text,
            "Replacement text; pass an empty string to delete the range.",
        )],
    ),
    variant(
        "replace_placeholder",
        "Replace every occurrence of a literal placeholder token throughout the body. The workhorse for filling templates.",
        &[
            req(
                "placeholder",
                ArgType::Text,
                "Literal token to look for, e.g. ${customer_name}.",
            ),
            req("text", ArgType::Text, "Text put in its place."),
            opt(
                "max_replacements",
                ArgType::Index,
                "Stop after this many replacements; omit to replace every occurrence.",
            ),
        ],
    ),
    variant(
        "delete_text",
        "Delete characters next to a caret position, as Backspace or Delete would. A run of characters inside one paragraph is one change: one tracked deletion to accept or reject, and one undo step. A count that runs past the paragraph's edge keeps going into the neighbouring one, joining them, exactly as holding Backspace does.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req(
                "offset",
                ArgType::Index,
                "UTF-8 byte offset of the caret in the paragraph's text.",
            ),
            opt(
                "direction",
                ArgType::Choice(&["backward", "forward"]),
                "backward removes the character before the caret (Backspace), forward the one after it (Delete). Default backward.",
            ),
            opt(
                "count",
                ArgType::Index,
                "How many characters to delete (default 1).",
            ),
        ],
    ),
    variant(
        "insert_paragraph_break",
        "Split a paragraph at an offset, as pressing Enter would.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req("offset", ArgType::Index, "UTF-8 byte offset to split at."),
            opt(
                "count",
                ArgType::Index,
                "How many breaks to insert (default 1).",
            ),
        ],
    ),
    variant(
        "insert_section_break",
        "Split a top-level body paragraph at an offset and begin the following Word section with the requested break type, preserving the active section's page setup and header/footer references.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req("offset", ArgType::Index, "UTF-8 byte offset to split at."),
            req(
                "break_type",
                ArgType::Choice(&["next_page", "continuous", "even_page", "odd_page"]),
                "How the following section starts.",
            ),
        ],
    ),
    variant(
        "insert_endnote",
        "Insert a Word endnote reference at an exact UTF-8 text boundary and create its package entry.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req(
                "offset",
                ArgType::Index,
                "UTF-8 byte offset where the reference is anchored.",
            ),
            opt("text", ArgType::Text, "Plain-text endnote body."),
        ],
    ),
    variant(
        "update_endnote",
        "Replace the plain-text body of an existing substantive endnote while preserving its id and references.",
        &[
            req("endnote_id", ArgType::Text, "Existing Word endnote id."),
            req("text", ArgType::Text, "Replacement plain-text body."),
        ],
    ),
    variant(
        "delete_endnote",
        "Delete one substantive endnote and every body reference to its id.",
        &[req("endnote_id", ArgType::Text, "Existing Word endnote id.")],
    ),
    variant(
        "insert_footnote",
        "Insert a Word footnote reference at an exact UTF-8 text boundary and create its package entry (word/footnotes.xml). The footnote body then appears as editable paragraphs after the referencing paragraph.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req(
                "offset",
                ArgType::Index,
                "UTF-8 byte offset where the reference is anchored.",
            ),
            opt("text", ArgType::Text, "Plain-text footnote body."),
        ],
    ),
    variant(
        "update_footnote",
        "Replace the plain-text body of an existing substantive footnote while preserving its id and references.",
        &[
            req("footnote_id", ArgType::Text, "Existing Word footnote id."),
            req("text", ArgType::Text, "Replacement plain-text body."),
        ],
    ),
    variant(
        "delete_footnote",
        "Delete one substantive footnote and every body reference to its id.",
        &[req("footnote_id", ArgType::Text, "Existing Word footnote id.")],
    ),
    variant(
        "convert_footnote_to_endnote",
        "Convert a footnote to an endnote, preserving its formatted body, relationships, and every reference. The destination id may differ to avoid collisions.",
        &[req("footnote_id", ArgType::Text, "Existing Word footnote id.")],
    ),
    variant(
        "convert_endnote_to_footnote",
        "Convert an endnote to a footnote, preserving its formatted body, relationships, and every reference. The destination id may differ to avoid collisions.",
        &[req("endnote_id", ArgType::Text, "Existing Word endnote id.")],
    ),
    variant(
        "add_citation_source",
        "Add a source to Word's bibliography custom XML catalog without inserting visible text.",
        &[req(
            "source",
            ArgType::CitationSource,
            "Word-compatible source metadata; tag must be unique.",
        )],
    ),
    span(
        "insert_citation",
        "Replace an exact text selection with the selected source's GenOffice-compatible citation text.",
        &[req("tag", ArgType::Text, "Existing bibliography source tag.")],
    ),
    variant(
        "insert_bibliography",
        "Insert a heading and one APA-like paragraph per bibliography source after a body paragraph.",
        &[
            req(
                "after_paragraph",
                ArgType::Index,
                "Zero-based body paragraph after which to insert the bibliography.",
            ),
            opt(
                "heading",
                ArgType::Text,
                "Visible heading; defaults to Bibliography.",
            ),
        ],
    ),
    variant(
        "set_document_protection",
        "Set Word's enforced read-only editing restriction, or pass null to remove it. This is document protection metadata, not encryption.",
        &[req(
            "protection",
            ArgType::DocumentProtection,
            "Read-only protection credentials, or null to remove documentProtection.",
        )],
    ),
    // -- character formatting ---------------------------------------------
    span(
        "apply_bold",
        "Set bold on or off for the runs in the range.",
        &[opt(
            "enabled",
            ArgType::Flag,
            "true enables bold (default), false removes it.",
        )],
    ),
    span(
        "apply_italic",
        "Set italic on or off for the runs in the range.",
        &[opt(
            "enabled",
            ArgType::Flag,
            "true enables italic (default), false removes it.",
        )],
    ),
    span(
        "apply_underline",
        "Set a Word underline style on or off for the runs in the range.",
        &[
            opt(
                "kind",
                ArgType::Text,
                "OOXML underline value such as single, words, double, thick, dotted, dash, or wave; defaults to single.",
            ),
            opt(
                "enabled",
                ArgType::Flag,
                "true enables underline (default), false removes it.",
            ),
        ],
    ),
    span(
        "apply_strike",
        "Set single or double strikethrough on or off for the runs in the range.",
        &[
            opt(
                "double",
                ArgType::Flag,
                "true uses Word's double strikethrough; defaults to false.",
            ),
            opt(
                "enabled",
                ArgType::Flag,
                "true enables strikethrough (default), false removes both variants.",
            ),
        ],
    ),
    span(
        "apply_superscript",
        "Set or remove superscript for the runs in the range.",
        &[opt(
            "enabled",
            ArgType::Flag,
            "true enables superscript (default), false removes it.",
        )],
    ),
    span(
        "apply_subscript",
        "Set or remove subscript for the runs in the range.",
        &[opt(
            "enabled",
            ArgType::Flag,
            "true enables subscript (default), false removes it.",
        )],
    ),
    span(
        "set_bold",
        "Set bold explicitly on or off for the runs in the range (unlike apply_bold, which only turns it on).",
        &[req(
            "value",
            ArgType::Flag,
            "true makes the range bold, false clears bold.",
        )],
    ),
    span(
        "set_font_size",
        "Set the font size of the runs in the range.",
        &[req(
            "font_size",
            ArgType::Number,
            "Font size in points, e.g. 10.5.",
        )],
    ),
    span(
        "adjust_font_size",
        "Grow or shrink the font one step, each run relative to its own effective size. The default ladder step is Word's ribbon grow/shrink (a 12pt/20pt mixed range becomes 14pt/22pt); step=point is Word's Ctrl+] / Ctrl+[ (13pt/21pt), clamped to 1..=1638pt.",
        &[
            req(
                "direction",
                ArgType::Choice(&["grow", "shrink"]),
                "grow steps up, shrink steps down.",
            ),
            opt(
                "step",
                ArgType::Choice(&["ladder", "point"]),
                "ladder (default) walks Word's ribbon size list; point moves exactly one point.",
            ),
        ],
    ),
    span(
        "set_font_family",
        "Set the font of the runs in the range.",
        &[req(
            "font_family",
            ArgType::Text,
            "Font name as Word stores it, e.g. Yu Gothic or Arial.",
        )],
    ),
    span(
        "clear_formatting",
        "Remove direct character formatting and character style from the range while preserving its text, highlighting (as Word's Ctrl+Space does) and semantic metadata.",
        &[],
    ),
    span(
        "change_case",
        "Change the letter case of the text in the range without changing its coordinates or formatting.",
        &[req(
            "mode",
            ArgType::Choice(&["upper", "lower", "title", "sentence"]),
            "Case transformation to apply.",
        )],
    ),
    span(
        "set_font_color",
        "Set the text color of the runs in the range.",
        &[req("color", ArgType::Text, "Color as #RRGGBB.")],
    ),
    span(
        "set_highlight_color",
        "Set the highlight (marker) color behind the runs in the range.",
        &[req("color", ArgType::Text, "Highlight color as #RRGGBB.")],
    ),
    // -- paragraph formatting ---------------------------------------------
    paragraphs(
        "set_paragraph_alignment",
        "Set horizontal alignment on a paragraph range.",
        &[req(
            "alignment",
            ArgType::Choice(&["left", "center", "right", "justify", "start", "end", "both"]),
            "left/start, center, right/end, or justify/both.",
        )],
    ),
    paragraphs(
        "set_paragraph_direction",
        "Set the paragraph base direction independently of its alignment.",
        &[req(
            "bidi",
            ArgType::Flag,
            "true selects right-to-left; false explicitly selects left-to-right.",
        )],
    ),
    span(
        "set_character_style",
        "Apply a character style (Word の文字スタイル) to a text span, or clear it. Read style_catalogue first: applying a style the document does not define is refused rather than written as a dangling w:rStyle.",
        &[opt(
            "style_id",
            ArgType::NullableText,
            "Character style id or display name from the document's styles part; null removes the style.",
        )],
    ),
    paragraphs(
        "set_paragraph_style",
        "Apply a paragraph style (Heading1, Normal, …) to a paragraph range.",
        &[opt(
            "style_id",
            ArgType::NullableText,
            "Style id from the document's styles part; null clears the explicit style.",
        )],
    ),
    paragraphs(
        "set_paragraph_shading",
        "Replace the direct solid background fill on a paragraph range, or clear it.",
        &[opt(
            "color",
            ArgType::NullableText,
            "Solid fill as #RRGGBB; null removes direct paragraph shading.",
        )],
    ),
    paragraphs(
        "set_paragraph_borders",
        "Replace the four direct border sides on a paragraph range. Omitted sides are removed, so omitting every side clears the border box.",
        &[
            opt(
                "top",
                ArgType::BorderEdge,
                "Top border edge; omit it to remove the top border.",
            ),
            opt(
                "bottom",
                ArgType::BorderEdge,
                "Bottom border edge; omit it to remove the bottom border.",
            ),
            opt(
                "left",
                ArgType::BorderEdge,
                "Left border edge; omit it to remove the left border.",
            ),
            opt(
                "right",
                ArgType::BorderEdge,
                "Right border edge; omit it to remove the right border.",
            ),
        ],
    ),
    paragraphs(
        "set_drop_cap",
        "Apply a Word drop cap to the first letter of each paragraph in the range, or remove it.",
        &[
            opt(
                "kind",
                ArgType::NullableChoice(&["drop", "margin"]),
                "drop places the initial inside the text block; margin places it in the margin. Omit or pass null to remove the drop cap.",
            ),
            opt(
                "lines",
                ArgType::Index,
                "Number of text lines the initial spans (default 3).",
            ),
        ],
    ),
    paragraphs(
        "set_paragraph_indent",
        "Set indents on a paragraph range. Every measurement is optional; the ones you omit stay as they are.",
        &[
            opt("left", ArgType::NullableNumber, "Left indent in points."),
            opt("right", ArgType::NullableNumber, "Right indent in points."),
            opt(
                "first_line",
                ArgType::NullableNumber,
                "First-line indent in points.",
            ),
            opt(
                "hanging",
                ArgType::NullableNumber,
                "Hanging indent in points.",
            ),
            opt(
                "left_chars",
                ArgType::Int,
                "Character-based left indent (Word's w:leftChars, 100 = one character). Setting it clears the point-based left indent.",
            ),
            opt(
                "right_chars",
                ArgType::Int,
                "Character-based right indent (w:rightChars, 100 = one character).",
            ),
            opt(
                "clear_special",
                ArgType::Flag,
                "Word's 特殊インデント「なし」: remove any first-line or hanging indent (character twins included) while keeping the left/right edges.",
            ),
        ],
    ),
    paragraphs(
        "adjust_paragraph_indent",
        "Ribbon-style increase/decrease indent. Each paragraph steps relative to its own state: list paragraphs promote/demote their list level (w:ilvl, clamped to 0..=8), character-unit indents move by one character (100 leftChars), and point indents snap on the document's default tab grid. With target=hanging it is Word's Ctrl+T / Ctrl+Shift+T instead: the left edge steps on the same grid but the first line stays where it is, so the difference becomes the hanging (or first-line) indent; list levels are left alone.",
        &[
            req(
                "direction",
                ArgType::Choice(&["increase", "decrease"]),
                "increase steps toward the text, decrease steps back toward the margin (never below zero).",
            ),
            opt(
                "target",
                ArgType::Choice(&["start", "hanging"]),
                "start (default) moves the whole paragraph like Ctrl+M; hanging keeps the first line in place like Ctrl+T.",
            ),
        ],
    ),
    paragraphs(
        "toggle_paragraph_space_before",
        "Word's Ctrl+0: add 12pt of space before every paragraph in the range, or remove it (set 0) when every paragraph already carries exactly 12pt. One undo step.",
        &[],
    ),
    paragraphs(
        "reset_paragraph_formatting",
        "Word's Ctrl+Q: strip direct paragraph formatting (alignment, indents, spacing, shading, borders, tab stops, frames) from the range while keeping each paragraph's style, numbering, and run formatting.",
        &[],
    ),
    paragraphs(
        "set_paragraph_metrics",
        "Set the paragraph dialog's left/right indents, special indent and before/after spacing as one atomic edit and one undo step. Like Word's dialog, omitted fields stay as they are (character-unit indents and line-unit spacing included).",
        &[
            opt("left", ArgType::Number, "Left indent in points."),
            opt("right", ArgType::Number, "Right indent in points."),
            opt("before", ArgType::Number, "Space above the paragraph, in points."),
            opt("after", ArgType::Number, "Space below the paragraph, in points."),
            opt(
                "first_line",
                ArgType::Number,
                "Special indent 字下げ (first-line indent) in points.",
            ),
            opt(
                "hanging",
                ArgType::Number,
                "Special indent ぶら下げ (hanging indent) in points.",
            ),
            opt(
                "clear_special",
                ArgType::Flag,
                "Special indent なし: remove any first-line or hanging indent.",
            ),
        ],
    ),
    paragraphs(
        "set_paragraph_line_spacing",
        "Set the spacing of a paragraph range: the line height and the space above and below it. Each field is optional, so the space after a clause can be set without touching its line height. Read the current values back with the `paragraph` query's `spacing`.",
        &[
            opt(
                "line",
                ArgType::NullableNumber,
                "Line height: a multiple of single spacing under the default auto rule (1.0, 1.5, 2.0), or points under exact / atLeast. Leave the field out to keep the current line height; pass null to clear it.",
            ),
            opt(
                "line_rule",
                ArgType::NullableText,
                "How `line` is read: auto (default), exact, or atLeast.",
            ),
            opt(
                "before",
                ArgType::NullableNumber,
                "Space above the paragraph, in points.",
            ),
            opt(
                "after",
                ArgType::NullableNumber,
                "Space below the paragraph, in points.",
            ),
        ],
    ),
    paragraphs(
        "set_numbering",
        "Attach a paragraph range to a numbering definition (bullets or numbered list), or detach it.",
        &[
            opt(
                "num_id",
                ArgType::NullableText,
                "Numbering id from the document's numbering part; null removes the numbering.",
            ),
            opt(
                "level",
                ArgType::Index,
                "Zero-based list level within that numbering definition.",
            ),
        ],
    ),
    paragraphs(
        "apply_multilevel_list",
        "Apply a persistent nine-level list gallery to a paragraph range, creating the required Word numbering definitions when needed.",
        &[
            req(
                "preset",
                ArgType::Choice(&[
                    "decimalHierarchy",
                    "chineseOfficial",
                    "bulletHierarchy",
                ]),
                "List gallery: decimal hierarchy, Chinese official headings, or rotating bullet hierarchy.",
            ),
            req(
                "level",
                ArgType::Index,
                "Zero-based list level from 0 through 8.",
            ),
        ],
    ),
    paragraphs(
        "apply_custom_multilevel_list",
        "Create and apply an independent one-to-nine-level Word list definition to a paragraph range.",
        &[
            req(
                "levels",
                ArgType::CustomNumberingLevelList,
                "Ordered level definitions from level 0 through level 8.",
            ),
            req(
                "level",
                ArgType::Index,
                "Zero-based level to apply; it must exist in levels.",
            ),
        ],
    ),
    // -- tracked changes ---------------------------------------------------
    variant(
        "start_change_tracking",
        "Turn Word change tracking on, so later edits in this session are recorded as tracked changes a reviewer can accept or reject.",
        &[
            opt(
                "author",
                ArgType::NullableText,
                "Author recorded on the revisions.",
            ),
            opt("initials", ArgType::NullableText, "Author initials."),
            opt(
                "date",
                ArgType::NullableText,
                "ISO-8601 timestamp recorded on the revisions.",
            ),
            opt(
                "date_utc",
                ArgType::NullableText,
                "ISO-8601 UTC timestamp (Word's w16du:dateUtc).",
            ),
        ],
    ),
    variant(
        "stop_change_tracking",
        "Turn Word change tracking off; later edits apply directly again.",
        &[],
    ),
    variant(
        "accept_revisions",
        "Accept tracked changes: insertions become normal text, deletions are dropped. Without a filter, accepts them all.",
        &[
            opt(
                "kind",
                ArgType::Choice(&["inserted", "deleted"]),
                "Only accept revisions of this kind.",
            ),
            opt(
                "start_paragraph",
                ArgType::Index,
                "First paragraph to act on (inclusive).",
            ),
            opt(
                "end_paragraph",
                ArgType::Index,
                "Last paragraph to act on (inclusive).",
            ),
            opt(
                "revision_id",
                ArgType::Text,
                "Act on only the revision with this id, as reported by the `revisions` query — how a reviewer takes one specific change.",
            ),
        ],
    ),
    variant(
        "reject_revisions",
        "Reject tracked changes: insertions are dropped, deleted text is restored. Without a filter, rejects them all.",
        &[
            opt(
                "kind",
                ArgType::Choice(&["inserted", "deleted"]),
                "Only reject revisions of this kind.",
            ),
            opt(
                "start_paragraph",
                ArgType::Index,
                "First paragraph to act on (inclusive).",
            ),
            opt(
                "end_paragraph",
                ArgType::Index,
                "Last paragraph to act on (inclusive).",
            ),
            opt(
                "revision_id",
                ArgType::Text,
                "Act on only the revision with this id, as reported by the `revisions` query — how a reviewer takes one specific change.",
            ),
        ],
    ),
    // -- comments -----------------------------------------------------------
    variant(
        "add_comment",
        "Attach a Word comment to a paragraph range. Omit the offsets to comment on whole paragraphs; to comment on a phrase, prefer add_comment_by_text.",
        &[
            req(
                "start_paragraph",
                ArgType::Index,
                "Zero-based index of the paragraph the anchor starts in.",
            ),
            opt(
                "start_offset",
                ArgType::Index,
                "UTF-8 byte offset where the anchor starts; omit to start at the beginning of that paragraph.",
            ),
            req(
                "end_paragraph",
                ArgType::Index,
                "Zero-based index of the paragraph the anchor ends in.",
            ),
            opt(
                "end_offset",
                ArgType::Index,
                "Exclusive UTF-8 byte offset where the anchor ends; omit to end at the end of that paragraph.",
            ),
            req("text", ArgType::Text, "Comment body."),
            opt("author", ArgType::NullableText, "Comment author."),
            opt("initials", ArgType::NullableText, "Author initials."),
            opt("date", ArgType::NullableText, "ISO-8601 timestamp."),
        ],
    ),
    variant(
        "add_comment_by_text",
        "Attach a Word comment to a phrase, located by searching the document — no byte offsets to compute. Exposed as the `comment_on_text` tool.",
        &[
            req(
                "query",
                ArgType::Text,
                "Exact text the comment is anchored to.",
            ),
            opt(
                "occurrence",
                ArgType::Index,
                "Which match to use, zero-based in document order (default 0).",
            ),
            req("text", ArgType::Text, "Comment body."),
            opt("author", ArgType::NullableText, "Comment author."),
            opt("initials", ArgType::NullableText, "Author initials."),
            opt("date", ArgType::NullableText, "ISO-8601 timestamp."),
        ],
    ),
    variant(
        "add_comment_by_para_id",
        "Attach a Word comment to a range inside the paragraph with this stable `w14:paraId`, so the anchor survives index shifts.",
        &[
            req(
                "para_id",
                ArgType::Text,
                "The paragraph's w14:paraId, as returned by the paragraph queries.",
            ),
            req(
                "start_offset",
                ArgType::Index,
                "UTF-8 byte offset where the comment anchor starts.",
            ),
            req(
                "end_offset",
                ArgType::Index,
                "Exclusive UTF-8 byte offset where the anchor ends.",
            ),
            req("text", ArgType::Text, "Comment body."),
            opt("author", ArgType::NullableText, "Comment author."),
            opt("initials", ArgType::NullableText, "Author initials."),
            opt("date", ArgType::NullableText, "ISO-8601 timestamp."),
        ],
    ),
    variant(
        "reply_to_comment",
        "Reply to an existing comment, keeping the exchange in one Word thread. The reply inherits the anchor of the comment it answers; replying to a reply joins the same thread.",
        &[
            req(
                "comment_id",
                ArgType::Text,
                "Comment id being replied to, from the `comments` or `comment_threads` query.",
            ),
            req("text", ArgType::Text, "Reply body."),
            opt("author", ArgType::NullableText, "Reply author."),
            opt("initials", ArgType::NullableText, "Author initials."),
            opt("date", ArgType::NullableText, "ISO-8601 timestamp."),
        ],
    ),
    variant(
        "update_comment",
        "Rewrite the body of an existing comment.",
        &[
            req(
                "comment_id",
                ArgType::Text,
                "Comment id from the `comments` query.",
            ),
            req("text", ArgType::Text, "New comment body."),
        ],
    ),
    variant(
        "delete_comment",
        "Delete a comment and its anchor.",
        &[req(
            "comment_id",
            ArgType::Text,
            "Comment id from the `comments` query.",
        )],
    ),
    variant(
        "resolve_comment",
        "Mark a comment resolved or reopen it.",
        &[
            req(
                "comment_id",
                ArgType::Text,
                "Comment id from the `comments` query.",
            ),
            req(
                "resolved",
                ArgType::Flag,
                "true resolves the comment, false reopens it.",
            ),
        ],
    ),
    // -- headers and footers ------------------------------------------------
    variant(
        "set_header_paragraph_text",
        "Replace the text of one paragraph in the page header.",
        &[
            req(
                "paragraph",
                ArgType::Index,
                "Zero-based paragraph index within the header.",
            ),
            req("text", ArgType::Text, "New header paragraph text."),
        ],
    ),
    variant(
        "set_footer_paragraph_text",
        "Replace the text of one paragraph in the page footer.",
        &[
            req(
                "paragraph",
                ArgType::Index,
                "Zero-based paragraph index within the footer.",
            ),
            req("text", ArgType::Text, "New footer paragraph text."),
        ],
    ),
    variant(
        "set_header_footer_variants",
        "Enable or disable first-page and odd/even header and footer variants, materializing blank editable stories when needed.",
        &[
            req(
                "different_first_page",
                ArgType::Flag,
                "Use distinct header/footer stories on the first page.",
            ),
            req(
                "different_odd_even_pages",
                ArgType::Flag,
                "Use distinct header/footer stories on odd and even pages.",
            ),
            opt(
                "section",
                ArgType::Index,
                "Zero-based section whose first-page switch to set; omit for the final section. Odd/even is document-wide either way.",
            ),
        ],
    ),
    // -- insert ribbon ------------------------------------------------------
    variant(
        "insert_caption",
        "Insert a centered Word caption with a live SEQ number after a body paragraph.",
        &[
            req(
                "after_paragraph",
                ArgType::Index,
                "Zero-based body paragraph after which to insert the caption.",
            ),
            req("label", ArgType::Text, "Caption label, such as Figure or Table."),
            opt(
                "description",
                ArgType::Text,
                "Caption text following the generated number.",
            ),
        ],
    ),
    span(
        "insert_field",
        "Replace a text selection with an inline Word field and its cached display text.",
        &[
            req(
                "kind",
                ArgType::Choice(&["date", "time", "page", "numPages", "fileName"]),
                "Word field to insert.",
            ),
            req(
                "cached_text",
                ArgType::Text,
                "Display text shown until Word updates the field.",
            ),
        ],
    ),
    variant(
        "insert_equation",
        "Insert native Office Math Markup Language at a collapsed current-state UTF-8 text boundary; block equations split the paragraph around the equation.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req(
                "offset",
                ArgType::Index,
                "UTF-8 byte offset at which to insert the equation.",
            ),
            req(
                "omml",
                ArgType::Text,
                "Inline m:oMath or block m:oMathPara XML, matching placement.",
            ),
            req(
                "placement",
                ArgType::Choice(&["inline", "block"]),
                "Insert inside the paragraph or as a separate display equation.",
            ),
        ],
    ),
    span(
        "insert_blank_page",
        "Replace the current text selection with a blank Word page bounded by page breaks.",
        &[],
    ),
    variant(
        "insert_cover_page",
        "Insert styled cover-page paragraphs at the start of the document and push existing content to page 2.",
        &[req(
            "paragraphs",
            ArgType::CoverParagraphList,
            "Styled paragraphs, from top to bottom, for the cover page.",
        )],
    ),
    variant(
        "insert_text_among_anchors",
        "Insert text where text boxes or note references sit at one offset, after the first `after_anchors` of them (insert_text types before them all).",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req("offset", ArgType::Index, "UTF-8 byte offset where the text boxes or note references sit."),
            req("after_anchors", ArgType::Index, "How many of the text boxes and note references at the offset come before the text."),
            req("text", ArgType::Text, "Text to insert."),
        ],
    ),
    variant(
        "delete_text_box",
        "Delete a text box (its drawing and everything in it), named by the paragraph that anchors it and its place among that paragraph's text boxes.",
        &[
            req("paragraph", ArgType::Index, "Zero-based index of the paragraph anchoring the text box."),
            req("text_box", ArgType::Index, "Zero-based index among that paragraph's text boxes, in run order."),
        ],
    ),
    variant(
        "insert_shape",
        "Insert an editable floating Word preset shape or stroke-only connector at a UTF-8 text boundary.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req(
                "offset",
                ArgType::Index,
                "UTF-8 byte offset where the shape is anchored.",
            ),
            req(
                "shape",
                ArgType::Choice(&[
                    "rectangle",
                    "line",
                    "lineArrow",
                    "lineArrowDouble",
                    "lineBent",
                    "lineCurved",
                    "roundedRectangle",
                    "ellipse",
                    "diamond",
                    "triangle",
                    "rightTriangle",
                    "parallelogram",
                    "trapezoid",
                    "pentagon",
                    "hexagon",
                    "octagon",
                    "plus",
                    "mathPlus",
                    "leftBrace",
                    "rightBrace",
                    "blockArc",
                    "pie",
                    "chord",
                    "teardrop",
                    "frame",
                    "halfFrame",
                    "corner",
                    "diagStripe",
                    "donut",
                    "noSmoking",
                    "foldedCorner",
                    "bevel",
                    "cube",
                    "can",
                    "lightningBolt",
                    "heart",
                    "sun",
                    "moon",
                    "cloud",
                    "arc",
                    "plaque",
                    "smileyFace",
                    "leftBracket",
                    "rightBracket",
                    "fourPointStar",
                    "fivePointStar",
                    "sixPointStar",
                    "sevenPointStar",
                    "eightPointStar",
                    "tenPointStar",
                    "twelvePointStar",
                    "sixteenPointStar",
                    "twentyFourPointStar",
                    "thirtyTwoPointStar",
                    "irregularSealOne",
                    "irregularSealTwo",
                    "ribbon",
                    "ribbon2",
                    "wave",
                    "doubleWave",
                    "flowChartProcess",
                    "flowChartAlternateProcess",
                    "flowChartDecision",
                    "flowChartPredefinedProcess",
                    "flowChartInternalStorage",
                    "flowChartDocument",
                    "flowChartMultidocument",
                    "flowChartTerminator",
                    "flowChartPreparation",
                    "flowChartManualInput",
                    "flowChartManualOperation",
                    "flowChartConnector",
                    "flowChartOffpageConnector",
                    "flowChartMagneticDisk",
                    "flowChartMagneticDrum",
                    "flowChartDisplay",
                    "flowChartDelay",
                    "flowChartOr",
                    "flowChartCollate",
                    "flowChartSort",
                    "flowChartExtract",
                    "flowChartMerge",
                    "flowChartSummingJunction",
                    "flowChartPunchedTape",
                    "wedgeRectCallout",
                    "wedgeRoundRectCallout",
                    "wedgeEllipseCallout",
                    "cloudCallout",
                    "rightArrow",
                    "leftArrow",
                    "upArrow",
                    "downArrow",
                    "leftRightArrow",
                    "upDownArrow",
                    "notchedRightArrow",
                    "chevron",
                    "homePlate",
                    "quadArrow",
                    "bentArrow",
                    "uturnArrow",
                    "curvedRightArrow",
                    "stripedRightArrow",
                    "snipOneRectangle",
                    "snipTwoSameRectangle",
                    "snipTwoDiagonalRectangle",
                    "snipRoundRectangle",
                    "roundOneRectangle",
                    "roundTwoSameRectangle",
                    "roundTwoDiagonalRectangle",
                ]),
                "DrawingML preset geometry to preserve through DOCX round-trips.",
            ),
            req("width_px", ArgType::Number, "Shape width in CSS pixels."),
            req("height_px", ArgType::Number, "Shape height in CSS pixels."),
            opt(
                "fill_color",
                ArgType::NullableText,
                "Fill colour as #RRGGBB, or null for no fill.",
            ),
            opt(
                "border_color",
                ArgType::NullableText,
                "Outline colour as #RRGGBB, or null for no outline.",
            ),
            req(
                "border_width_px",
                ArgType::Number,
                "Outline width in CSS pixels.",
            ),
            opt("text", ArgType::Text, "Editable text inside the shape."),
        ],
    ),
    variant(
        "mark_index_entry",
        "Insert an invisible Word XE index-entry marker at a UTF-8 text boundary.",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req(
                "offset",
                ArgType::Index,
                "UTF-8 byte offset where the marker is anchored.",
            ),
            req("term", ArgType::Text, "Index term stored in the XE field."),
        ],
    ),
    variant(
        "insert_index",
        "Insert a generated Word INDEX field after a body paragraph, collecting current XE markers.",
        &[req(
            "after_paragraph",
            ArgType::Index,
            "Zero-based body paragraph after which to insert the index.",
        )],
    ),
    // -- tables -------------------------------------------------------------
    variant(
        "insert_table",
        "Insert an empty table after a paragraph, or at a caret position the way Word places it.",
        &[
            req(
                "after_paragraph",
                ArgType::Index,
                "Zero-based index of the paragraph the table is placed after (the caret paragraph when caret_offset is given).",
            ),
            req("rows", ArgType::Index, "Number of rows."),
            req("columns", ArgType::Index, "Number of columns."),
            opt(
                "column_widths",
                ArgType::NumberList,
                "Column widths in points; omit or pass an empty array for automatic widths.",
            ),
            opt(
                "caret_offset",
                ArgType::Index,
                "UTF-8 byte offset of a caret in after_paragraph. Word placement: above the paragraph at 0 or in an empty paragraph, between the halves of a paragraph split in the middle, after it at the end (an empty paragraph keeps the paragraph mark after the table at the end of the body or a section). Omit to place the table after the paragraph.",
            ),
        ],
    ),
    variant(
        "set_table_width",
        "Set the overall width of a table.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "width",
                ArgType::Text,
                "Width spec: `auto`, points such as `420pt`, or a percentage such as `100%`.",
            ),
        ],
    ),
    variant(
        "set_table_fit",
        "Choose Word's AutoFit mode for a table: fit to contents, fit to the page window, or preserve fixed column geometry.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "mode",
                ArgType::Choice(&["contents", "window", "fixed"]),
                "AutoFit mode to apply.",
            ),
        ],
    ),
    variant(
        "set_table_text_wrapping",
        "Place a table inline with the document flow, or use Word's floating-table Around wrapping.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "wrapping",
                ArgType::Choice(&["none", "around"]),
                "Text wrapping mode to apply.",
            ),
        ],
    ),
    variant(
        "set_table_column_widths",
        "Set every column width of a table at once and use fixed layout. One width per column: a different count is refused, because the grid has to keep describing the cells — use insert_table_column / delete_table_column to change how many columns there are. Values below 9pt are clamped to 9pt.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "widths",
                ArgType::NumberList,
                "One width in points per column, in column order.",
            ),
        ],
    ),
    variant(
        "split_table",
        "Word's 表の分割 (Ctrl+Shift+Enter): the rows from `row_index` on become a second table with an empty paragraph between the two. `row_index: 0` puts the empty paragraph in front of the table instead. Nested tables cannot be split.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "row_index",
                ArgType::Index,
                "First row of the second table.",
            ),
        ],
    ),
    variant(
        "split_table_cell",
        "Word's セルの分割: the cell at (`row`, `column`) becomes `columns` × `rows` cells. Extra columns are new grid columns that the other rows' cells widen over; extra rows are new rows whose other cells are vertically merged with the row above. A vertically merged cell has to be unmerged first.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row of the cell."),
            req(
                "column",
                ArgType::Index,
                "Zero-based grid column the cell covers.",
            ),
            req("columns", ArgType::Index, "Number of cells across (1 keeps the width)."),
            req("rows", ArgType::Index, "Number of cells down (1 keeps the height)."),
        ],
    ),
    variant(
        "distribute_table_rows",
        "Word's 行の高さを揃える: give every row of a table the same height, with the `atLeast` rule so taller content still grows the row. Omit `height` to use the average of the rows' explicit heights (refused when a row has none).",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            opt(
                "height",
                ArgType::Number,
                "Row height in points to apply to every row.",
            ),
        ],
    ),
    variant(
        "distribute_table_columns",
        "Word's 列の幅を揃える: give every grid column of a table the same width and use fixed layout. Omit `total_width` to share out the grid's explicit widths (refused when a column has none).",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            opt(
                "total_width",
                ArgType::Number,
                "Total table width in points to share between the columns.",
            ),
        ],
    ),
    variant(
        "set_table_alignment",
        "Set how the table sits between the margins — a 別表 is usually centred while the body text around it is not. This is the table's own alignment, not the alignment of the paragraphs inside its cells.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "alignment",
                ArgType::Choice(&["left", "center", "right"]),
                "Where the table sits between the page margins.",
            ),
        ],
    ),
    variant(
        "set_table_cell_shading",
        "Fill a cell's background, the way a header row is shaded. Pass null to clear the fill.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            req(
                "column",
                ArgType::Index,
                "Zero-based logical-grid column; a merged cell answers to any column it spans.",
            ),
            opt(
                "color",
                ArgType::NullableText,
                "Fill colour as #RRGGBB, or null to clear it.",
            ),
        ],
    ),
    variant(
        "apply_table_style_preset",
        "Apply a deterministic direct-formatting gallery preset to a table. Every cell fill and border is persisted, so the result does not depend on package-local named styles.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "preset",
                ArgType::Choice(&["plain", "grid", "lightBlue", "blueBands", "grayBands"]),
                "Portable visual preset to apply.",
            ),
        ],
    ),
    variant(
        "apply_table_named_style",
        "Apply a table style defined in the DOCX package, or remove the named style while preserving direct formatting.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            opt(
                "style_id",
                ArgType::NullableText,
                "Package-local table style id; null removes the named style.",
            ),
        ],
    ),
    variant(
        "set_table_look",
        "Toggle the conditional regions (header row, total row, first/last column, row and column bands) stored in Word's table-look properties.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            opt(
                "first_row",
                ArgType::Flag,
                "Whether the named table style's first-row formatting is active.",
            ),
            opt(
                "banded_rows",
                ArgType::Flag,
                "Whether the named table style's alternating row bands are active.",
            ),
            opt(
                "last_row",
                ArgType::Flag,
                "Whether the named table style's last-row (total row) formatting is active.",
            ),
            opt(
                "first_column",
                ArgType::Flag,
                "Whether the named table style's first-column formatting is active.",
            ),
            opt(
                "last_column",
                ArgType::Flag,
                "Whether the named table style's last-column formatting is active.",
            ),
            opt(
                "banded_columns",
                ArgType::Flag,
                "Whether the named table style's alternating column bands are active.",
            ),
        ],
    ),
    variant(
        "set_table_borders",
        "Set the table's borders: the four outer edges plus the interior grid lines. Only the edges you name are touched, so switching the inside lines off leaves the frame alone. A cell's own borders win over these.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            opt("top", ArgType::BorderEdge, "Top outer edge."),
            opt("bottom", ArgType::BorderEdge, "Bottom outer edge."),
            opt("left", ArgType::BorderEdge, "Left outer edge."),
            opt("right", ArgType::BorderEdge, "Right outer edge."),
            opt(
                "inside_h",
                ArgType::BorderEdge,
                "Horizontal lines between rows.",
            ),
            opt(
                "inside_v",
                ArgType::BorderEdge,
                "Vertical lines between columns.",
            ),
        ],
    ),
    variant(
        "set_table_cell_borders",
        "Set one cell's own borders, which win over the table's defaults — the way a total row gets a double rule above it. A cell has only its four edges.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            req(
                "column",
                ArgType::Index,
                "Zero-based logical-grid column; a merged cell answers to any column it spans.",
            ),
            opt("top", ArgType::BorderEdge, "Top edge."),
            opt("bottom", ArgType::BorderEdge, "Bottom edge."),
            opt("left", ArgType::BorderEdge, "Left edge."),
            opt("right", ArgType::BorderEdge, "Right edge."),
        ],
    ),
    variant(
        "set_table_row_properties",
        "Row properties for a table that runs over a page break: repeat this row as the header at the top of every page, and keep the row from splitting across pages. A 別表 that spans pages needs both.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            opt(
                "repeat_header",
                ArgType::Flag,
                "Repeat this row at the top of every page the table continues onto (w:tblHeader).",
            ),
            opt(
                "cant_split",
                ArgType::Flag,
                "Keep the row's lines together rather than splitting it across a page boundary (w:cantSplit).",
            ),
        ],
    ),
    variant(
        "reset_table_cell_margins",
        "Remove all direct cell padding and inherit the row/table defaults again.",
        &[
            req("table_order_index", ArgType::Index, "Zero-based table index."),
            req("row", ArgType::Index, "Zero-based row index."),
            req("column", ArgType::Index, "Zero-based logical-grid column."),
        ],
    ),
    variant(
        "set_table_row_height_properties",
        "Set authored row height: auto removes the constraint; atLeast/exact require height in points (excluding padding).",
        &[
            req("table_order_index", ArgType::Index, "Zero-based table index."),
            req("row", ArgType::Index, "Zero-based row index."),
            req("mode", ArgType::Text, "auto, atLeast, or exact."),
            opt("height", ArgType::NullableNumber, "Positive height in points; omit for auto."),
        ],
    ),
    variant(
        "set_table_cell_margins",
        "Set the inner padding of a cell, or the table's default for every cell that has none of its own when row and column are omitted. Each edge is optional and in points — widening the left padding leaves the others alone.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            opt(
                "row",
                ArgType::Index,
                "Zero-based row index; omit together with `column` to set the table default.",
            ),
            opt(
                "column",
                ArgType::Index,
                "Zero-based logical-grid column; omit together with `row` to set the table default.",
            ),
            opt("top", ArgType::NullableNumber, "Top padding in points."),
            opt(
                "bottom",
                ArgType::NullableNumber,
                "Bottom padding in points.",
            ),
            opt("left", ArgType::NullableNumber, "Left padding in points."),
            opt("right", ArgType::NullableNumber, "Right padding in points."),
        ],
    ),
    variant(
        "set_table_cell_vertical_alignment",
        "Set where a cell's content sits in its box. Header cells in a contract's tables are usually centred vertically.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            req(
                "column",
                ArgType::Index,
                "Zero-based logical-grid column; a merged cell answers to any column it spans.",
            ),
            req(
                "alignment",
                ArgType::Choice(&["top", "center", "bottom"]),
                "Vertical alignment of the cell's content.",
            ),
        ],
    ),
    variant(
        "set_table_cell_text",
        "Replace the text of a single table cell.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            req("column", ArgType::Index, "Zero-based column index."),
            req("text", ArgType::Text, "New cell text."),
        ],
    ),
    variant(
        "fill_table_row",
        "Replace the text of a whole row, cell by cell.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            req(
                "values",
                ArgType::TextList,
                "One string per cell, left to right.",
            ),
        ],
    ),
    variant(
        "insert_table_row",
        "Insert an empty row into a table.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "row_index",
                ArgType::Index,
                "Position the new row takes; equal to the row count to append.",
            ),
            opt(
                "template_row",
                ArgType::Index,
                "Row whose formatting (height, shading, cell alignment) the new row copies, as Word does for 行の挿入; omit for a plain row.",
            ),
        ],
    ),
    variant(
        "clone_table_row",
        "Duplicate a row, keeping its formatting — the template idiom for growing an itemised table.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("source_row", ArgType::Index, "Row to copy."),
            req(
                "target_row",
                ArgType::Index,
                "Position the copy is inserted at.",
            ),
            opt(
                "clear_text",
                ArgType::Flag,
                "true (default) empties the copied cells, keeping only their formatting; false copies the text too.",
            ),
        ],
    ),
    variant(
        "delete_table_row",
        "Delete a row. Deleting the last row removes the table.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row_index", ArgType::Index, "Zero-based row index."),
        ],
    ),
    variant(
        "insert_table_column",
        "Insert an empty column into a table.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "column_index",
                ArgType::Index,
                "Position the new column takes; equal to the column count to append.",
            ),
            opt(
                "width",
                ArgType::NullableNumber,
                "Width of the new column in points; omit for automatic.",
            ),
            opt(
                "template_column",
                ArgType::Index,
                "Column whose cell formatting the new cells copy, as Word does for 列の挿入; omit for plain cells.",
            ),
        ],
    ),
    variant(
        "delete_table_column",
        "Delete a column. Deleting the last column removes the table.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("column_index", ArgType::Index, "Zero-based column index."),
        ],
    ),
    variant(
        "delete_table_cell",
        "Delete one logical table cell and shift the remaining cells left or upward while keeping a valid Word table grid.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            req(
                "column",
                ArgType::Index,
                "Zero-based logical-grid column of the cell to delete.",
            ),
            req(
                "direction",
                ArgType::Choice(&["shift_left", "shift_up"]),
                "Direction in which the remaining cells move.",
            ),
        ],
    ),
    variant(
        "insert_table_cell",
        "Insert one empty cell before a table cell, as Word's セルの挿入 does: shift_right moves the rest of the row right (the row grows by one grid column), shift_down moves the cells below down and appends a row for the last one.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            req(
                "column",
                ArgType::Index,
                "Zero-based logical-grid column of the cell the new cell goes before.",
            ),
            req(
                "direction",
                ArgType::Choice(&["shift_right", "shift_down"]),
                "Direction in which the existing cells move.",
            ),
        ],
    ),
    variant(
        "delete_table",
        "Remove an entire top-level table from the body.",
        &[req(
            "table_order_index",
            ArgType::Index,
            "Zero-based index of the table in document order.",
        )],
    ),
    variant(
        "resize_table_column_between",
        "Drag the border between two neighbouring columns: the pair's total width stays the same.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "left_column_index",
                ArgType::Index,
                "Zero-based index of the column on the left of the border.",
            ),
            req(
                "new_left_width",
                ArgType::Number,
                "New width of the left column, in points.",
            ),
            req(
                "new_right_width",
                ArgType::Number,
                "New width of the column on its right, in points.",
            ),
        ],
    ),
    variant(
        "resize_table_column_right_edge",
        "Drag the right edge of the last column, changing the table's total width.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("column_index", ArgType::Index, "Zero-based column index."),
            req("new_width", ArgType::Number, "New column width in points."),
        ],
    ),
    variant(
        "resize_table_row_between",
        "Drag the border between two neighbouring rows: the pair's total height stays the same.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "row_index",
                ArgType::Index,
                "Zero-based index of the row above the border.",
            ),
            req(
                "new_top_height",
                ArgType::Number,
                "New height of that row, in points.",
            ),
            req(
                "new_bottom_height",
                ArgType::Number,
                "New height of the row below it, in points.",
            ),
        ],
    ),
    variant(
        "resize_table_row_bottom_edge",
        "Drag the bottom edge of a row, changing the table's total height.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row_index", ArgType::Index, "Zero-based row index."),
            req("new_height", ArgType::Number, "New row height in points."),
        ],
    ),
    variant(
        "merge_table_cells",
        "Merge a rectangular block of cells into one.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "selected_cells",
                ArgType::CellList,
                "The cells to merge, given as logical-grid positions.",
            ),
        ],
    ),
    variant(
        "unmerge_table_cells",
        "Split merged cells back apart, so each returns to its own grid column.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req(
                "target_cells",
                ArgType::CellList,
                "Logical-grid positions currently held by the merged master cells.",
            ),
        ],
    ),
    variant(
        "set_table_cell_spacing",
        "Set the space between cells (Word's Table Properties ▸ Options ▸ cell spacing), or remove it.",
        &[
            req("table_order_index", ArgType::Index, "Zero-based index of the table in document order."),
            opt("spacing", ArgType::NullableNumber, "Spacing in points as stored in w:tblCellSpacing; null removes it."),
        ],
    ),
    variant(
        "set_table_cell_text_direction",
        "Set the text direction of one cell (Word's Text Direction), e.g. tbRl for vertical Japanese.",
        &[
            req("table_order_index", ArgType::Index, "Zero-based index of the table in document order."),
            req("row", ArgType::Index, "Zero-based row index."),
            req("column", ArgType::Index, "Zero-based logical-grid column index."),
            opt(
                "direction",
                ArgType::NullableChoice(&["lrTb", "tbRl", "btLr", "lrTbV", "tbRlV", "btLrV"]),
                "The w:textDirection value; null restores horizontal text.",
            ),
        ],
    ),
    variant(
        "set_table_indent",
        "Set the table's indent from the left margin (w:tblInd).",
        &[
            req("table_order_index", ArgType::Index, "Zero-based index of the table in document order."),
            req("indent", ArgType::Number, "Indent in points."),
        ],
    ),
    variant(
        "set_table_position",
        "Float the table and position it (Word's Table Positioning): alignment or offset on each axis, anchors, distance from surrounding text, overlap.",
        &[
            req("table_order_index", ArgType::Index, "Zero-based index of the table in document order."),
            opt("horizontal_align", ArgType::NullableChoice(&["left", "center", "right", "inside", "outside"]), "Horizontal alignment; wins over the offset."),
            opt("horizontal_offset", ArgType::NullableNumber, "Horizontal offset from the anchor in points."),
            req("horizontal_anchor", ArgType::Choice(&["margin", "page", "text"]), "What the horizontal position is measured from."),
            opt("vertical_align", ArgType::NullableChoice(&["top", "center", "bottom", "inside", "outside"]), "Vertical alignment; wins over the offset."),
            opt("vertical_offset", ArgType::NullableNumber, "Vertical offset from the anchor in points."),
            req("vertical_anchor", ArgType::Choice(&["margin", "page", "text"]), "What the vertical position is measured from; text moves the table with its paragraph."),
            opt("distance_top", ArgType::Number, "Distance from the text above, in points."),
            opt("distance_bottom", ArgType::Number, "Distance from the text below, in points."),
            opt("distance_left", ArgType::Number, "Distance from the text on the left, in points."),
            opt("distance_right", ArgType::Number, "Distance from the text on the right, in points."),
            opt("allow_overlap", ArgType::Flag, "Whether the table may overlap other floating tables. Defaults to true."),
        ],
    ),
    variant(
        "convert_text_to_table",
        "Turn paragraphs into a table (Word's Convert Text to Table), splitting each at a separator.",
        &[
            req("start_paragraph", ArgType::Index, "First paragraph to convert."),
            req("end_paragraph", ArgType::Index, "Last paragraph to convert (inclusive)."),
            req("separator", ArgType::Text, "tab, comma, paragraph (one paragraph per cell), or one other character."),
            opt("columns", ArgType::Index, "Number of columns; defaults to the widest row."),
        ],
    ),
    variant(
        "convert_table_to_text",
        "Turn a table into paragraphs (Word's Convert to Text), one per row with the cells joined by a separator.",
        &[
            req("table_order_index", ArgType::Index, "Zero-based index of the table in document order."),
            req("separator", ArgType::Text, "tab, comma, paragraph (one paragraph per cell), or one other character."),
        ],
    ),
    variant(
        "sort_table",
        "Sort a table's rows by one or more columns (Word's Sort), keeping header rows in place.",
        &[
            req("table_order_index", ArgType::Index, "Zero-based index of the table in document order."),
            req("keys", ArgType::SortKeyList, "Sort keys, most significant first."),
            opt("header_rows", ArgType::Index, "How many leading rows are headers and stay put."),
        ],
    ),
    variant(
        "insert_table_formula",
        "Insert a formula field into a table cell (Word's Formula, e.g. =SUM(ABOVE)), with its result computed.",
        &[
            req("paragraph", ArgType::Index, "Cell paragraph to insert into."),
            req("offset", ArgType::Index, "UTF-8 byte offset in that paragraph."),
            req("formula", ArgType::Text, "The expression, with or without the leading =."),
            opt("number_format", ArgType::NullableText, "Word's numeric picture, e.g. #,##0 or 0.00."),
        ],
    ),
    variant(
        "update_table_formulas",
        "Recompute every formula field in the document's tables (Word's Update Field).",
        &[],
    ),
    // -- table of contents --------------------------------------------------
    variant(
        "insert_table_of_contents",
        "Insert a generated table of contents after a paragraph (Word's References ▸ Table of Contents).",
        &[
            req(
                "after_paragraph",
                ArgType::Index,
                "Zero-based index of the paragraph the TOC is placed after.",
            ),
            opt(
                "outline_min",
                ArgType::Index,
                "Shallowest heading outline level to include (default 1).",
            ),
            opt(
                "outline_max",
                ArgType::Index,
                "Deepest heading outline level to include (default 3).",
            ),
        ],
    ),
    variant(
        "update_table_of_contents",
        "Regenerate the first existing table of contents from the current headings (Word's Update Table).",
        &[],
    ),
    // -- page setup ---------------------------------------------------------
    variant(
        "set_document_setting",
        "Toggle a boolean element directly under <w:settings> in word/settings.xml.",
        &[
            req(
                "name",
                ArgType::Text,
                "Fully qualified element name, e.g. w:evenAndOddHeaders, w:autoHyphenation, w:mirrorMargins, w:trackChanges.",
            ),
            req(
                "enabled",
                ArgType::Flag,
                "true inserts the element if missing, false removes it.",
            ),
        ],
    ),
    variant(
        "set_page_orientation",
        "Set the default section's page orientation.",
        &[req(
            "landscape",
            ArgType::Flag,
            "true for landscape, false for portrait.",
        )],
    ),
    variant(
        "set_page_size",
        "Set the default section's page size, given in portrait terms; the current orientation is kept.",
        &[
            req(
                "portrait_width",
                ArgType::Number,
                "Portrait width in points (595.3 for A4).",
            ),
            req(
                "portrait_height",
                ArgType::Number,
                "Portrait height in points (841.9 for A4); must be at least the width.",
            ),
        ],
    ),
    variant(
        "set_page_margins",
        "Set the default section's page margins. Every field is optional and what is left out keeps its current value, so the binding gutter can be widened without restating the four edges. All values are in points.",
        &[
            opt("top", ArgType::NullableNumber, "Top margin in points."),
            opt(
                "bottom",
                ArgType::NullableNumber,
                "Bottom margin in points.",
            ),
            opt("left", ArgType::NullableNumber, "Left margin in points."),
            opt("right", ArgType::NullableNumber, "Right margin in points."),
            opt(
                "header",
                ArgType::NullableNumber,
                "Distance from the top page edge to the header story.",
            ),
            opt(
                "footer",
                ArgType::NullableNumber,
                "Distance from the bottom page edge to the footer story.",
            ),
            opt(
                "gutter",
                ArgType::NullableNumber,
                "Binding gutter: extra space reserved on the bound edge, for a document that will be stapled or bound.",
            ),
        ],
    ),
    variant(
        "set_page_columns",
        "Set equal-width columns on the final/default section.",
        &[
            req(
                "count",
                ArgType::Index,
                "Column count from 1 through 12.",
            ),
            req(
                "space",
                ArgType::Number,
                "Non-negative gutter width in points.",
            ),
            req(
                "separator",
                ArgType::Flag,
                "Draw a vertical rule in each gutter; ignored for one column.",
            ),
        ],
    ),
    variant(
        "set_page_borders",
        "Set or clear the default section's page frame.",
        &[
            req(
                "clear",
                ArgType::Flag,
                "true removes the frame; false applies the supplied edges.",
            ),
            opt("top", ArgType::BorderEdge, "Top page-frame edge."),
            opt("bottom", ArgType::BorderEdge, "Bottom page-frame edge."),
            opt("left", ArgType::BorderEdge, "Left page-frame edge."),
            opt("right", ArgType::BorderEdge, "Right page-frame edge."),
            opt(
                "offset_from",
                ArgType::Choice(&["page", "text"]),
                "Measure the frame from the paper edge or the body text box.",
            ),
        ],
    ),
    variant(
        "set_watermark",
        "Set or clear the Word-compatible text watermark in all referenced header variants.",
        &[
            req(
                "clear",
                ArgType::Flag,
                "true removes the watermark; false sets it from text.",
            ),
            opt(
                "text",
                ArgType::NullableText,
                "Watermark text; required by the command when clear is false.",
            ),
        ],
    ),
    variant(
        "set_document_theme",
        "Author the package-wide Word document theme while preserving unmodeled theme XML.",
        &[req(
            "theme",
            ArgType::DocumentTheme,
            "Complete editable theme font/color subset.",
        )],
    ),
    variant(
        "set_page_background_color",
        "Set the page background color.",
        &[opt(
            "color",
            ArgType::NullableText,
            "Background color as #RRGGBB; null clears it.",
        )],
    ),
    // -- heading-relative insertion ----------------------------------------
    variant(
        "insert_after_heading",
        "Insert a new paragraph just after the section heading whose text matches — an index-free way to place content.",
        &[
            req(
                "heading",
                ArgType::Text,
                "Heading text to match, including its numbering label if it has one.",
            ),
            req("text", ArgType::Text, "Text of the new paragraph."),
            opt(
                "level",
                ArgType::Index,
                "Only match headings at this outline level.",
            ),
            opt(
                "occurrence",
                ArgType::Index,
                "Which match to use when the heading text repeats (zero-based, default 0).",
            ),
        ],
    ),
    variant(
        "insert_before_heading",
        "Insert a new paragraph just before the section heading whose text matches.",
        &[
            req(
                "heading",
                ArgType::Text,
                "Heading text to match, including its numbering label if it has one.",
            ),
            req("text", ArgType::Text, "Text of the new paragraph."),
            opt(
                "level",
                ArgType::Index,
                "Only match headings at this outline level.",
            ),
            opt(
                "occurrence",
                ArgType::Index,
                "Which match to use when the heading text repeats (zero-based, default 0).",
            ),
        ],
    ),
];

/// Every `HeadlessQuery` variant the `query` tool accepts. Each result is
/// tagged with the same `type` as the query that produced it.
const QUERIES: &[Variant] = &[
    variant(
        "document_stats",
        "Paragraph, table, comment and revision counts — the cheapest overview of a document.",
        &[],
    ),
    variant(
        "document_outline",
        "The heading outline: level, text, numbering label and paragraph index per heading. Use it to navigate before editing.",
        &[
            opt(
                "start_heading",
                ArgType::Index,
                "Zero-based heading to start the page at (default 0).",
            ),
            opt("max_headings", ArgType::Index, "Page size (default 100)."),
            opt(
                "max_level",
                ArgType::Index,
                "Skip headings deeper than this outline level.",
            ),
        ],
    ),
    variant("paragraph_count", "Number of paragraphs in the body.", &[]),
    variant(
        "paragraph_text",
        "Plain text of one paragraph, plus its numbering label if it has one.",
        &[req(
            "paragraph",
            ArgType::Index,
            "Zero-based paragraph index.",
        )],
    ),
    variant(
        "paragraph",
        "Full snapshot of one paragraph: text, alignment, indents and per-run styling with run byte offsets.",
        &[req(
            "paragraph",
            ArgType::Index,
            "Zero-based paragraph index.",
        )],
    ),
    variant(
        "paragraph_by_para_id",
        "Resolve a paragraph by its stable `w14:paraId`, returning its current index and snapshot.",
        &[req("para_id", ArgType::Text, "The w14:paraId to look up.")],
    ),
    variant(
        "paragraph_slice",
        "A page of paragraph texts — the way to read a document without one call per paragraph.",
        &[
            opt(
                "start_paragraph",
                ArgType::Index,
                "Zero-based paragraph to start the page at (default 0).",
            ),
            opt("max_paragraphs", ArgType::Index, "Page size (default 20)."),
        ],
    ),
    variant(
        "search_text",
        "Find literal occurrences of a string, returning paragraph index and byte offsets you can feed straight into replace_range or add_comment.",
        &[
            req(
                "query",
                ArgType::Text,
                "Literal text to find; case-sensitive unless case_insensitive is set.",
            ),
            opt(
                "start_paragraph",
                ArgType::Index,
                "First paragraph to search (default 0).",
            ),
            opt(
                "max_results",
                ArgType::Index,
                "Cap on matches returned (default 20).",
            ),
            opt(
                "context_before_chars",
                ArgType::Index,
                "Characters of context returned before each match (default 24).",
            ),
            opt(
                "context_after_chars",
                ArgType::Index,
                "Characters of context returned after each match (default 24).",
            ),
            opt(
                "case_insensitive",
                ArgType::Flag,
                "Ignore ASCII case, so a defined term is found however the draft capitalises it. Offsets and matched_text still describe the document's own text.",
            ),
        ],
    ),
    variant(
        "character_style_at",
        "The effective run styling at one position (bold, italic, font, size, color …).",
        &[
            req("paragraph", ArgType::Index, "Zero-based paragraph index."),
            req(
                "offset",
                ArgType::Index,
                "UTF-8 byte offset inside the paragraph's text.",
            ),
        ],
    ),
    variant(
        "table_count",
        "Number of top-level tables in the body.",
        &[],
    ),
    variant(
        "table",
        "Snapshot of a whole table: grid, column widths and cell texts.",
        &[req(
            "table_order_index",
            ArgType::Index,
            "Zero-based index of the table in document order.",
        )],
    ),
    variant(
        "table_row",
        "Snapshot of one table row.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
        ],
    ),
    variant(
        "table_column",
        "Snapshot of one table column.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("column", ArgType::Index, "Zero-based column index."),
        ],
    ),
    variant(
        "table_cell",
        "Snapshot of one table cell, including its merge spans.",
        &[
            req(
                "table_order_index",
                ArgType::Index,
                "Zero-based index of the table in document order.",
            ),
            req("row", ArgType::Index, "Zero-based row index."),
            req("column", ArgType::Index, "Zero-based column index."),
        ],
    ),
    variant(
        "revisions",
        "Tracked changes in the document: kind, text, author and date. Read this before accept_revisions / reject_revisions.",
        &[
            opt(
                "start_revision",
                ArgType::Index,
                "Zero-based revision to start the page at (default 0).",
            ),
            opt("max_revisions", ArgType::Index, "Page size (default 100)."),
            opt(
                "kind",
                ArgType::Choice(&["inserted", "deleted"]),
                "Only return revisions of this kind.",
            ),
            opt(
                "author",
                ArgType::Text,
                "Only return revisions by this author.",
            ),
            opt(
                "date_from",
                ArgType::Text,
                "Only return revisions at or after this ISO-8601 timestamp.",
            ),
            opt(
                "date_to",
                ArgType::Text,
                "Only return revisions at or before this ISO-8601 timestamp.",
            ),
        ],
    ),
    variant(
        "comments",
        "Comments in the document, flat: id, author, text, resolved flag and anchor. Filter to the open review backlog with resolved=false.",
        &[
            opt(
                "start_comment",
                ArgType::Index,
                "Zero-based comment to start the page at (default 0).",
            ),
            opt("max_comments", ArgType::Index, "Page size (default 50)."),
            opt(
                "author",
                ArgType::Text,
                "Only comments by this author (exact match).",
            ),
            opt(
                "resolved",
                ArgType::Flag,
                "Only comments in this resolved state; false is what still needs an answer. The returned total counts the filtered set.",
            ),
        ],
    ),
    variant(
        "comment_threads",
        "Comments grouped into reply threads.",
        &[
            opt(
                "start_thread",
                ArgType::Index,
                "Zero-based thread to start the page at (default 0).",
            ),
            opt("max_threads", ArgType::Index, "Page size (default 50)."),
        ],
    ),
    variant(
        "styles",
        "The document's style catalogue (id, name, based_on, default flag). Read it before set_paragraph_style, which needs a style_id that exists.",
        &[opt(
            "kind",
            ArgType::Choice(&["paragraph", "character"]),
            "Only styles of this kind; omit for both.",
        )],
    ),
    variant(
        "page_setup",
        "Page geometry of the default section — size, orientation, margins, header/footer distance and background. The read side of the set_page_* commands.",
        &[],
    ),
    variant(
        "header_footer_text",
        "Text of every header and footer story present (default / first page / even pages). Each line comes back in the same tab-joined form set_header_paragraph_text and set_footer_paragraph_text take.",
        &[],
    ),
    variant(
        "document_properties",
        "Package metadata: title, creator, lastModifiedBy, created/modified timestamps and any custom properties.",
        &[],
    ),
    variant(
        "citation_sources",
        "Word bibliography sources stored in the document package. Read this catalog before insert_citation so the command can use a stable source tag.",
        &[],
    ),
    variant(
        "document_protection",
        "Word editing-restriction metadata from settings.xml, including the verifier fields needed by a trusted client to check an unlock password.",
        &[],
    ),
    variant(
        "chunks",
        "Split the document into article-sized chunks and read them. A contract is written and cited one article at a time, so this is the unit to read it in: each chunk carries its number (第５条), its title (秘密保持), its paragraph range and its text. Page with start_chunk / max_chunks.",
        &[
            opt(
                "start_chunk",
                ArgType::Index,
                "Zero-based chunk to start the page at (default 0).",
            ),
            opt("max_chunks", ArgType::Index, "Page size (default 20)."),
            opt(
                "disable_table_splitting",
                ArgType::Flag,
                "Keep a top-level table inside the article it appears in instead of giving it a chunk of its own.",
            ),
            opt(
                "enable_branch_splitting",
                ArgType::Flag,
                "Give 第N条のM its own chunk instead of keeping it with 第N条.",
            ),
            opt(
                "include_text",
                ArgType::Flag,
                "Return the chunk texts (default true). Set false to page through a long document by number and title alone, which is far cheaper while looking for the right article.",
            ),
        ],
    ),
    variant(
        "references",
        "Cross-references in the body (第５条第２項, 前条, 第３条から第７条まで), resolved to the articles they name. Each carries the byte offsets of the citation and the target's paragraph, paraId, number and title. Use unresolved_only=true to find the citations that renumbering broke.",
        &[
            opt(
                "start_reference",
                ArgType::Index,
                "Zero-based reference to start the page at (default 0).",
            ),
            opt("max_references", ArgType::Index, "Page size (default 100)."),
            opt(
                "unresolved_only",
                ArgType::Flag,
                "Only references that point at no article in this document — the dangling citations.",
            ),
            opt(
                "start_paragraph",
                ArgType::Index,
                "Only references written at or after this paragraph.",
            ),
            opt(
                "end_paragraph",
                ArgType::Index,
                "Only references written at or before this paragraph.",
            ),
        ],
    ),
];

/// Behavioural hints published as the tool's MCP `annotations`, so a client can
/// tell a read-only inspection from a call that mutates a session or writes a
/// file.
struct Hints {
    read_only: bool,
    destructive: bool,
    idempotent: bool,
    /// Touches state outside this process (the filesystem).
    open_world: bool,
}

const fn hints(read_only: bool, destructive: bool, idempotent: bool, open_world: bool) -> Hints {
    Hints {
        read_only,
        destructive,
        idempotent,
        open_world,
    }
}

/// One entry of the `tools/list` catalogue.
struct Tool {
    name: &'static str,
    /// Human-readable name for UIs.
    title: &'static str,
    description: &'static str,
    args: &'static [Arg],
    /// Fields of the JSON object the tool returns; published as `outputSchema`
    /// and returned as `structuredContent`.
    output: &'static [Arg],
    /// "At least one of these groups is required", rendered as JSON Schema
    /// `anyOf`; empty for tools with no such constraint.
    any_of: &'static [&'static [&'static str]],
    hints: Hints,
}

/// The published tool catalogue.
const TOOLS: &[Tool] = &[
    Tool {
        name: "open_docx",
        title: "Open a .docx",
        description: "Load a .docx into an in-memory editing session and return the `session_id` every other tool takes, plus document stats. Pass either a filesystem `path` or base64 `bytes_base64` — exactly one of them. `path` must name a .docx inside the directories this server is allowed to use (its working directory unless `JADE_MCP_ALLOWED_ROOTS` says otherwise); symlinks are resolved before that check.",
        args: &[
            opt(
                "path",
                ArgType::Text,
                "Path to the .docx file to open: absolute, or relative to the first allowed directory. Must be a .docx inside an allowed directory; `..` is refused.",
            ),
            opt(
                "bytes_base64",
                ArgType::Text,
                "Base64-encoded .docx bytes, as an alternative to `path`.",
            ),
        ],
        output: &[
            req(
                "session_id",
                ArgType::Text,
                "Handle for this open document; pass it to every other tool.",
            ),
            req(
                "stats",
                ArgType::Object,
                "The `document_stats` query result for the freshly opened document.",
            ),
        ],
        any_of: &[&["path"], &["bytes_base64"]],
        hints: hints(false, false, false, true),
    },
    Tool {
        name: "apply_commands",
        title: "Apply edit commands",
        description: "Apply a batch of editing commands to a session. This is the main editing entry point: every `type` in the `commands` schema is a supported operation, from insert_text and replace_placeholder through table, comment, revision and page-setup commands. Each command becomes one undo step. Real batches and their verification queries are preflighted on a clone, so a later error leaves the live document and undo history unchanged. `coordinates: sequential` (the default) reads each position against the evolving document. `coordinates: snapshot` reads every position against the batch's starting document, safely rebases supported text edits and text-range formatting, and rejects ambiguous overlaps or unsupported commands before mutation. Set `dry_run: true` to try the batch against a throw-away clone and inspect the simulated result without touching the session.",
        args: &[
            SESSION,
            req(
                "commands",
                ArgType::CommandList,
                "Commands to apply, in order. Each is an object tagged by a snake_case `type`, e.g. {\"type\":\"insert_text\",\"paragraph\":0,\"offset\":0,\"text\":\"Hi\"}. Set `coordinates` to declare whether positions refer to the evolving document or the batch-start snapshot.",
            ),
            opt(
                "coordinates",
                ArgType::Choice(&["sequential", "snapshot"]),
                "How positional fields are interpreted. `sequential` (default) addresses the document left by earlier commands. `snapshot` addresses the document at batch start and supports insert_text, delete_text, replace_range, insert_paragraph_break, insert_section_break, insert_endnote, insert_footnote, add_citation_source, insert_citation, and text-range formatting; ambiguous overlaps are rejected.",
            ),
            opt(
                "dry_run",
                ArgType::Flag,
                "When true, apply to a clone and return the simulated result; the live session and its undo history stay unchanged.",
            ),
            opt(
                "include_diff",
                ArgType::Flag,
                "When true, also return before/after document stats and the indices of the paragraphs whose text changed.",
            ),
            opt(
                "queries",
                ArgType::QueryList,
                "Queries run right after the batch, so an edit and its verification take one round trip. With dry_run they see the simulated document.",
            ),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the batch applied."),
            req("applied", ArgType::Index, "How many commands were applied."),
            opt(
                "recorded_undo_steps",
                ArgType::Index,
                "How many of the applied commands actually changed the document and became undo steps; a no-op (re-applying a value a property already has) succeeds without one.",
            ),
            opt(
                "dry_run",
                ArgType::Flag,
                "Present and true when the batch ran against a clone.",
            ),
            opt(
                "query",
                ArgType::Object,
                "Results of the `queries` argument, shaped like the `query` tool's output.",
            ),
            opt(
                "diff",
                ArgType::Object,
                "Present with include_diff: `before` / `after` document stats and `changed_paragraphs`.",
            ),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, true, false, false),
    },
    Tool {
        name: "replace_text",
        title: "Find and replace",
        description: "Replace every non-overlapping occurrence of `find` with `replace` — literally, or as a regular expression with $1 capture references. Matches split across runs by formatting are handled, and edits are applied highest-offset-first so length changes never mis-place a later replacement. Scope it with start_paragraph / end_paragraph and cap it with max_replacements. Undo takes one step per replaced match. A match that overlaps another author's tracked change is refused rather than rewritten — accept or reject that change first. With change tracking on, each replacement is recorded as a deletion of the old text plus an insertion of the new one, the way Word redlines it.",
        args: &[
            SESSION,
            req(
                "find",
                ArgType::Text,
                "Literal text to search for; must not be empty.",
            ),
            req(
                "replace",
                ArgType::Text,
                "Replacement text; pass an empty string to delete the matches.",
            ),
            opt(
                "start_paragraph",
                ArgType::Index,
                "First paragraph to search (default 0).",
            ),
            opt(
                "end_paragraph",
                ArgType::Index,
                "Last paragraph to search (inclusive); omit to search to the end.",
            ),
            opt(
                "max_replacements",
                ArgType::Index,
                "Replace at most this many matches, in document order.",
            ),
            opt(
                "case_insensitive",
                ArgType::Flag,
                "Match without regard to case, so a term is replaced however the draft capitalises it.",
            ),
            opt(
                "regex",
                ArgType::Flag,
                "Treat `find` as a Rust regular expression and expand $1 / ${name} capture references in `replace`. Matches never span paragraphs.",
            ),
            opt(
                "dry_run",
                ArgType::Flag,
                "Report the matches this would replace and change nothing — the way to check the blast radius of a document-wide replacement first.",
            ),
        ],
        output: &[
            req(
                "replaced",
                ArgType::Index,
                "How many occurrences were replaced, or would be with dry_run.",
            ),
            req(
                "dry_run",
                ArgType::Flag,
                "Whether this call only previewed the replacement.",
            ),
            req(
                "matches",
                ArgType::MatchList,
                "The matches, in document order, capped at 50 entries however many were replaced.",
            ),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, true, false, false),
    },
    Tool {
        name: "start_change_tracking",
        title: "Start change tracking",
        description: "Turn Word change tracking on for a session, so every edit the agent makes afterwards is recorded as a tracked insertion or deletion a human reviewer can accept or reject. Optionally attribute the revisions to an author.",
        args: &[
            SESSION,
            opt(
                "author",
                ArgType::Text,
                "Author name recorded on the revisions, e.g. AI Assistant.",
            ),
            opt("initials", ArgType::Text, "Author initials."),
            opt(
                "date",
                ArgType::Text,
                "ISO-8601 timestamp recorded on the revisions.",
            ),
            opt(
                "date_utc",
                ArgType::Text,
                "ISO-8601 UTC timestamp (Word's w16du:dateUtc).",
            ),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, true, false),
    },
    Tool {
        name: "stop_change_tracking",
        title: "Stop change tracking",
        description: "Turn Word change tracking off for a session; later edits are applied directly again.",
        args: &[SESSION],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, true, false),
    },
    Tool {
        name: "accept_revisions",
        title: "Accept tracked changes",
        description: "Accept tracked changes in a session: insertions become normal text and deleted text is dropped. Without arguments it accepts all of them; scope it by paragraph range, revision kind, or a single revision_id taken from the `revisions` query.",
        args: &[
            SESSION,
            opt(
                "kind",
                ArgType::Choice(&["inserted", "deleted"]),
                "Only accept revisions of this kind.",
            ),
            opt(
                "start_paragraph",
                ArgType::Index,
                "First paragraph to act on (inclusive).",
            ),
            opt(
                "end_paragraph",
                ArgType::Index,
                "Last paragraph to act on (inclusive).",
            ),
            opt(
                "revision_id",
                ArgType::Text,
                "Accept only the revision with this id, as reported by the `revisions` query — how a reviewer acts on one specific change.",
            ),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, true, true, false),
    },
    Tool {
        name: "reject_revisions",
        title: "Reject tracked changes",
        description: "Reject tracked changes in a session: insertions are dropped and deleted text is restored. Without arguments it rejects all of them; scope it by paragraph range, revision kind, or a single revision_id taken from the `revisions` query.",
        args: &[
            SESSION,
            opt(
                "kind",
                ArgType::Choice(&["inserted", "deleted"]),
                "Only reject revisions of this kind.",
            ),
            opt(
                "start_paragraph",
                ArgType::Index,
                "First paragraph to act on (inclusive).",
            ),
            opt(
                "end_paragraph",
                ArgType::Index,
                "Last paragraph to act on (inclusive).",
            ),
            opt(
                "revision_id",
                ArgType::Text,
                "Reject only the revision with this id, as reported by the `revisions` query — how a reviewer acts on one specific change.",
            ),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, true, true, false),
    },
    Tool {
        name: "add_comment",
        title: "Add a comment",
        description: "Attach a Word comment to a paragraph range — the non-destructive way to raise a point instead of editing. Omit the offsets to comment on whole paragraphs; give them (straight from a `search_text` result) to anchor on part of one. The range may span paragraphs, and may cover text that already carries tracked changes. To comment on a phrase, `comment_on_text` is usually easier.",
        args: &[
            SESSION,
            req(
                "start_paragraph",
                ArgType::Index,
                "Zero-based index of the paragraph the anchor starts in.",
            ),
            opt(
                "start_offset",
                ArgType::Index,
                "UTF-8 byte offset where the anchor starts; omit to start at the beginning of that paragraph.",
            ),
            req(
                "end_paragraph",
                ArgType::Index,
                "Zero-based index of the paragraph the anchor ends in.",
            ),
            opt(
                "end_offset",
                ArgType::Index,
                "Exclusive UTF-8 byte offset where the anchor ends; omit to end at the end of that paragraph.",
            ),
            req("text", ArgType::Text, "Comment body."),
            opt("author", ArgType::Text, "Comment author."),
            opt("initials", ArgType::Text, "Author initials."),
            opt("date", ArgType::Text, "ISO-8601 timestamp."),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, false, false),
    },
    Tool {
        name: "comment_on_text",
        title: "Comment on a phrase",
        description: "Attach a Word comment to a phrase, found by searching the document — usually what you want, since there are no byte offsets to compute. Set `occurrence` to pick a later match; it defaults to the first. A phrase that is not found is reported as an error naming it.",
        args: &[
            SESSION,
            req(
                "query",
                ArgType::Text,
                "The exact text to anchor the comment to.",
            ),
            opt(
                "occurrence",
                ArgType::Index,
                "Which match to use, zero-based in document order (default 0).",
            ),
            req("text", ArgType::Text, "Comment body."),
            opt("author", ArgType::Text, "Comment author."),
            opt("initials", ArgType::Text, "Author initials."),
            opt("date", ArgType::Text, "ISO-8601 timestamp."),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, false, false),
    },
    Tool {
        name: "get_paragraph_by_para_id",
        title: "Find a paragraph by paraId",
        description: "Look a paragraph up by its stable `w14:paraId`, getElementById-style, instead of by a positional index that edits keep shifting. Returns its current index and full snapshot, or nulls when no paragraph carries that id.",
        args: &[
            SESSION,
            req("para_id", ArgType::Text, "The w14:paraId to resolve."),
        ],
        output: &[
            req("type", ArgType::Text, "Always paragraph_by_para_id."),
            req("para_id", ArgType::Text, "The id that was looked up."),
            req(
                "paragraph",
                ArgType::Index,
                "Current zero-based index of that paragraph, or null when the id is unknown.",
            ),
            req(
                "value",
                ArgType::Object,
                "Paragraph snapshot (text, alignment, indent, runs), or null when the id is unknown.",
            ),
        ],
        any_of: &[],
        hints: hints(true, false, true, false),
    },
    Tool {
        name: "comment_by_para_id",
        title: "Comment by paraId",
        description: "Attach a Word comment to a range inside the paragraph identified by its stable `w14:paraId`, so the anchor survives the index shifts other edits cause.",
        args: &[
            SESSION,
            req(
                "para_id",
                ArgType::Text,
                "The w14:paraId of the paragraph to comment on.",
            ),
            req(
                "start_offset",
                ArgType::Index,
                "UTF-8 byte offset where the anchor starts.",
            ),
            req(
                "end_offset",
                ArgType::Index,
                "Exclusive UTF-8 byte offset where the anchor ends.",
            ),
            req("text", ArgType::Text, "Comment body."),
            opt("author", ArgType::Text, "Comment author."),
            opt("initials", ArgType::Text, "Author initials."),
            opt("date", ArgType::Text, "ISO-8601 timestamp."),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, false, false),
    },
    Tool {
        name: "reply_to_comment",
        title: "Reply to a comment",
        description: "Reply to a comment, keeping the point and its answer in one Word thread rather than as two unrelated notes on the same words. The reply inherits the anchor of the comment it answers, and replying to a reply joins that same thread — which is how Word stores a conversation. Read the thread first with the `comment_threads` query.",
        args: &[
            SESSION,
            req(
                "comment_id",
                ArgType::Text,
                "Comment id being replied to, from the `comments` or `comment_threads` query.",
            ),
            req("text", ArgType::Text, "Reply body."),
            opt("author", ArgType::Text, "Reply author."),
            opt("initials", ArgType::Text, "Author initials."),
            opt("date", ArgType::Text, "ISO-8601 timestamp."),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, false, false),
    },
    Tool {
        name: "resolve_comment",
        title: "Resolve a comment",
        description: "Mark a comment resolved — Word's Resolve — or reopen it. The state is written where Word reads it (`commentsExtended.xml`), so a thread the agent has answered comes back greyed out in Word rather than still open. Resolving a thread's root is what closes the exchange; the `comments` query with `resolved: false` is the remaining backlog.",
        args: &[
            SESSION,
            req(
                "comment_id",
                ArgType::Text,
                "Comment id from the `comments` query.",
            ),
            req(
                "resolved",
                ArgType::Flag,
                "true resolves the comment, false reopens it.",
            ),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, true, false),
    },
    Tool {
        name: "update_comment",
        title: "Edit a comment",
        description: "Rewrite the body of an existing comment, leaving its anchor, author and thread position alone.",
        args: &[
            SESSION,
            req(
                "comment_id",
                ArgType::Text,
                "Comment id from the `comments` query.",
            ),
            req("text", ArgType::Text, "New comment body."),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, true, false),
    },
    Tool {
        name: "delete_comment",
        title: "Delete a comment",
        description: "Delete a comment, its anchor in the body and its thread entry. Its replies go with it, as Word's Delete thread does; deleting a reply removes only that reply. Resolving is usually the right move on a review comment; deleting is for one the agent itself added.",
        args: &[
            SESSION,
            req(
                "comment_id",
                ArgType::Text,
                "Comment id from the `comments` query.",
            ),
        ],
        output: &[
            req("ok", ArgType::Flag, "Always true when the command applied."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, true, true, false),
    },
    Tool {
        name: "undo",
        title: "Undo",
        description: "Undo the most recent command(s) in a session. History granularity is one step per command, so a batch of N commands needs count: N to be fully reverted.",
        args: &[
            SESSION,
            opt(
                "count",
                ArgType::Index,
                "How many history steps to undo (default 1). Stops early when the stack runs out.",
            ),
        ],
        output: &[
            req(
                "undone",
                ArgType::Flag,
                "Whether at least one step was undone.",
            ),
            req("steps", ArgType::Index, "How many steps were undone."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, false, false),
    },
    Tool {
        name: "redo",
        title: "Redo",
        description: "Replay the most recently undone command(s) in a session. History granularity is one step per command.",
        args: &[
            SESSION,
            opt(
                "count",
                ArgType::Index,
                "How many history steps to redo (default 1). Stops early when the stack runs out.",
            ),
        ],
        output: &[
            req(
                "redone",
                ArgType::Flag,
                "Whether at least one step was redone.",
            ),
            req("steps", ArgType::Index, "How many steps were redone."),
            CAN_UNDO,
            CAN_REDO,
        ],
        any_of: &[],
        hints: hints(false, false, false, false),
    },
    Tool {
        name: "query",
        title: "Query a document",
        description: "Inspect a session without changing it. Every `type` in the `queries` schema is a supported read: document_stats and document_outline to get oriented, paragraph_slice and paragraph_text to read, search_text to locate byte offsets for an edit, table_* for tables, revisions and comments for review state. Results come back in the order the queries were given.",
        args: &[
            SESSION,
            req(
                "queries",
                ArgType::QueryList,
                "Queries to run, in order. Each is an object tagged by a snake_case `type`, e.g. {\"type\":\"paragraph_text\",\"paragraph\":0}.",
            ),
        ],
        output: &[req(
            "results",
            ArgType::ResultList,
            "One result per query, in the same order.",
        )],
        any_of: &[],
        hints: hints(true, false, true, false),
    },
    Tool {
        name: "export_docx",
        title: "Export to .docx",
        description: "Serialize the edited session back to .docx bytes, preserving the untouched parts of the original file. With `path` a new .docx is created inside an allowed directory and its path returned; without it the bytes come back base64-encoded. The session stays open.",
        args: &[
            SESSION,
            opt(
                "path",
                ArgType::Text,
                "Output path for a new .docx: absolute, or relative to the first allowed directory. Must not exist yet — the export never overwrites and never writes through a symlink. Omit to get the bytes back as base64 instead.",
            ),
        ],
        output: &[
            req(
                "bytes",
                ArgType::Index,
                "Size of the exported file in bytes.",
            ),
            opt(
                "path",
                ArgType::Text,
                "Where the file was written (symlink-free); present only when `path` was given.",
            ),
            opt(
                "bytes_base64",
                ArgType::Text,
                "The exported .docx, base64-encoded; present only when `path` was omitted.",
            ),
        ],
        any_of: &[],
        hints: hints(false, true, true, true),
    },
    Tool {
        name: "close_docx",
        title: "Close a session",
        description: "Drop an in-memory session and free its document. Unexported edits are lost, so export first.",
        args: &[SESSION],
        output: &[
            req("session_id", ArgType::Text, "The session that was closed."),
            req(
                "closed",
                ArgType::Flag,
                "false when no session with that id was open.",
            ),
        ],
        any_of: &[],
        hints: hints(false, true, true, false),
    },
    Tool {
        name: "list_sessions",
        title: "List open sessions",
        description: "List the ids of every editing session currently open in this server process.",
        args: &[],
        output: &[req(
            "session_ids",
            ArgType::TextList,
            "Ids of the open sessions, sorted.",
        )],
        any_of: &[],
        hints: hints(true, false, true, false),
    },
];

/// Render `args` as a JSON Schema object. `additional` controls whether unknown
/// keys are allowed: inputs are closed, so a client can catch a mistyped
/// argument name instead of watching it be ignored; outputs are open, so adding
/// a result field is not a breaking change.
fn args_schema(args: &[Arg], additional: bool) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for arg in args {
        let mut schema = arg.ty.schema();
        schema["description"] = json!(arg.description);
        properties.insert(arg.name.to_string(), schema);
        if arg.required {
            required.push(json!(arg.name));
        }
    }
    json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": required,
        "additionalProperties": additional,
    })
}

/// JSON Schema for one `type`-tagged command/query variant.
fn variant_schema(variant: &Variant) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = vec![json!("type")];
    properties.insert(
        "type".to_string(),
        json!({
            "type": "string",
            "const": variant.name,
            "description": "Discriminator selecting this variant.",
        }),
    );
    for arg in variant.scope.iter().chain(variant.args) {
        let mut schema = arg.ty.schema();
        schema["description"] = json!(arg.description);
        properties.insert(arg.name.to_string(), schema);
        if arg.required {
            required.push(json!(arg.name));
        }
    }
    json!({
        "type": "object",
        "title": variant.name,
        "description": variant.description,
        "properties": Value::Object(properties),
        "required": required,
        "additionalProperties": false,
    })
}

fn variant_schemas(variants: &[Variant]) -> Vec<Value> {
    variants.iter().map(variant_schema).collect()
}

fn tool_definition(tool: &Tool) -> Value {
    let mut input = args_schema(tool.args, false);
    if !tool.any_of.is_empty() {
        input["anyOf"] = Value::Array(
            tool.any_of
                .iter()
                .map(|group| json!({ "required": group }))
                .collect(),
        );
    }
    json!({
        "name": tool.name,
        "title": tool.title,
        "description": tool.description,
        "inputSchema": input,
        "outputSchema": args_schema(tool.output, true),
        "annotations": {
            "title": tool.title,
            "readOnlyHint": tool.hints.read_only,
            "destructiveHint": tool.hints.destructive,
            "idempotentHint": tool.hints.idempotent,
            "openWorldHint": tool.hints.open_world,
        },
    })
}

/// JSON Schema object for an injected tool's arguments or result, built the
/// same way the built-in catalogue builds its own.
fn extension_schema(fields: &[ToolArg], leading: &[(&str, Value)], additional: bool) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, schema) in leading {
        properties.insert((*name).to_string(), schema.clone());
        required.push(json!(name));
    }
    for field in fields {
        properties.insert(field.name().to_string(), field.property_schema());
        if field.is_required() {
            required.push(json!(field.name()));
        }
    }
    json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": required,
        "additionalProperties": additional,
    })
}

/// Publishes an injected tool exactly like a built-in one, with the
/// server-owned `session_id` argument added in front.
fn extension_definition(tool: &RegisteredTool) -> Value {
    let spec = &tool.spec;
    let session = json!({
        "type": "string",
        "description": "Session id returned by `open_docx`.",
    });
    // A tool that opens a session has no session to be given, so the server
    // does not add the argument it would otherwise own.
    let leading: &[(&str, Value)] = if spec.session_scoped {
        &[("session_id", session)]
    } else {
        &[]
    };
    json!({
        "name": spec.name,
        "title": spec.title,
        "description": spec.description,
        "inputSchema": extension_schema(&spec.arguments, leading, false),
        "outputSchema": extension_schema(&spec.output, &[], true),
        "annotations": {
            "title": spec.title,
            "readOnlyHint": spec.hints.read_only,
            "destructiveHint": spec.hints.destructive,
            "idempotentHint": spec.hints.idempotent,
            "openWorldHint": spec.hints.open_world,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const HELLO_DOCX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hello.docx");
    /// Fixture carrying Word tracked changes (1 insertion, 1 deletion).
    const REVIEW_DOCX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/review.docx"
    );
    /// Fixture carrying a comment and a threaded reply.
    const COMMENT_DOCX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/comment.docx"
    );
    /// Fixture with a real Japanese contract: articles, titles and citations.
    const CONTRACT_DOCX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/contract.docx"
    );
    /// Fixture with a table of contents driven by `TOC` / `PAGEREF` fields.
    const TOC_DOCX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/toc.docx"
    );
    /// Fixture with one 2x2 table: paragraphs 0-3 are its cells, 4 is the body
    /// paragraph after it.
    const TABLE_DOCX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/table2x2.docx"
    );

    /// A server whose file tools may use the repository, where the fixtures
    /// live. `Server::new` would confine them to the working directory, which
    /// under `cargo test` is this crate and holds no fixtures.
    fn test_server() -> Server {
        Server::new().with_file_access(FileAccess::new([concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../.."
        )]))
    }

    /// Extract `word/document.xml` from raw .docx (zip) bytes.
    fn document_xml(docx_bytes: &[u8]) -> String {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(docx_bytes)).unwrap();
        let mut file = archive.by_name("word/document.xml").unwrap();
        let mut xml = String::new();
        file.read_to_string(&mut xml).unwrap();
        xml
    }

    /// Rebuild .docx bytes with `word/document.xml` rewritten by `edit`. Lets a
    /// test derive a focused fixture from an existing one instead of checking a
    /// near-duplicate binary into `public/fixtures/`.
    fn with_document_xml(docx_bytes: &[u8], edit: impl Fn(String) -> String) -> Vec<u8> {
        use std::io::{Read, Write};
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(docx_bytes)).unwrap();
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).unwrap();
                let name = entry.name().to_string();
                let mut content = Vec::new();
                entry.read_to_end(&mut content).unwrap();
                if name == "word/document.xml" {
                    content = edit(String::from_utf8(content).unwrap()).into_bytes();
                }
                writer.start_file(name, options).unwrap();
                writer.write_all(&content).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer
    }

    /// `(instrText, fldChar, PAGEREF)` marker counts — a fingerprint of the
    /// document's Word fields. Naive re-serializers flatten these to zero.
    fn field_marker_counts(xml: &str) -> (usize, usize, usize) {
        (
            xml.matches("w:instrText").count(),
            xml.matches("w:fldChar").count(),
            xml.matches("PAGEREF").count(),
        )
    }

    /// `(author, text, resolved)` of every comment in a session.
    fn comments(server: &mut Server, session: &str) -> Vec<(String, String, bool)> {
        let response = server
            .handle_message(&call(
                98,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "comments", "start_comment": 0, "max_comments": 50 }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["comments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|comment| {
                (
                    comment["author"].as_str().unwrap_or_default().to_string(),
                    comment["text"].as_str().unwrap_or_default().to_string(),
                    comment["resolved"].as_bool().unwrap_or(false),
                )
            })
            .collect()
    }

    /// `(kind, text)` of every tracked change in a session, for equality checks.
    fn revisions(server: &mut Server, session: &str) -> Vec<(String, String)> {
        let response = server
            .handle_message(&call(
                99,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "revisions", "start_revision": 0, "max_revisions": 50 }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["revisions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|rev| {
                (
                    rev["kind"].as_str().unwrap_or_default().to_string(),
                    rev["text"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    fn call(id: u64, name: &str, arguments: Value) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        })
        .to_string()
    }

    /// Decode the JSON payload carried in a `tools/call` text result, asserting
    /// the tool did not report an error.
    fn tool_json(response: &str) -> Value {
        let value: Value = serde_json::from_str(response).unwrap();
        assert_ne!(
            value["result"]["isError"],
            json!(true),
            "tool returned an error: {value}"
        );
        let text = value["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn tool_gate_defaults_to_open() {
        // An embedder that never sets a gate must see the server it always saw.
        let mut server = test_server();
        let response = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();

        let value: Value = serde_json::from_str(&response).unwrap();
        assert_ne!(value["result"]["isError"], json!(true));
    }

    #[test]
    fn tool_gate_refuses_with_its_own_message() {
        let mut server = test_server();
        server.set_tool_gate(|_tool, _args| Err("利用権を確認できません".to_string()));

        let response = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();

        // A refusal is a tool error, not a JSON-RPC error: the caller is usually
        // an agent, and it can only relay the reason if it arrives as content.
        let value: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["result"]["isError"], json!(true));
        assert_eq!(
            value["result"]["content"][0]["text"],
            json!("利用権を確認できません")
        );
        assert!(value.get("error").is_none());
    }

    #[test]
    fn tool_gate_sees_the_tool_name_and_arguments() {
        use std::sync::Mutex;

        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);

        let mut server = test_server();
        server.set_tool_gate(move |tool, args| {
            recorder
                .lock()
                .unwrap()
                .push((tool.to_string(), args.clone()));
            Ok(())
        });

        server.handle_message(&call(1, "list_sessions", json!({})));

        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "list_sessions");
        assert_eq!(calls[0].1, json!({}));
    }

    #[test]
    fn tool_gate_covers_unknown_tools_too() {
        // The gate runs before dispatch, so a refusal wins over "unknown tool".
        // That keeps a locked server from leaking which tools it has.
        let mut server = test_server();
        server.set_tool_gate(|_tool, _args| Err("locked".to_string()));

        let response = server
            .handle_message(&call(1, "no_such_tool", json!({})))
            .unwrap();

        let value: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["result"]["isError"], json!(true));
        assert_eq!(value["result"]["content"][0]["text"], json!("locked"));
    }

    /// Open a fixture and return its session id.
    fn open_session(server: &mut Server, path: &str) -> String {
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": path })))
            .unwrap();
        tool_json(&open)["session_id"].as_str().unwrap().to_string()
    }

    /// Current text of a paragraph in a session.
    fn paragraph_text(server: &mut Server, session: &str, paragraph: usize) -> String {
        let response = server
            .handle_message(&call(
                97,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "paragraph_text", "paragraph": paragraph }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// Run an arbitrary single command batch (helper for test setup).
    fn apply(server: &mut Server, session: &str, commands: Value) {
        server
            .handle_message(&call(
                96,
                "apply_commands",
                json!({ "session_id": session, "commands": commands }),
            ))
            .unwrap();
    }

    #[test]
    fn initialize_advertises_tool_capability() {
        let mut server = test_server();
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" },
        })
        .to_string();
        let response: Value =
            serde_json::from_str(&server.handle_message(&request).unwrap()).unwrap();
        assert_eq!(response["result"]["serverInfo"]["name"], json!(SERVER_NAME));
        assert_eq!(response["result"]["protocolVersion"], json!("2024-11-05"));
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_exposes_the_core_tools() {
        let mut server = test_server();
        let request = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).to_string();
        let response: Value =
            serde_json::from_str(&server.handle_message(&request).unwrap()).unwrap();
        let names: Vec<&str> = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        for expected in [
            "open_docx",
            "apply_commands",
            "replace_text",
            "start_change_tracking",
            "stop_change_tracking",
            "accept_revisions",
            "reject_revisions",
            "add_comment",
            "comment_on_text",
            "get_paragraph_by_para_id",
            "comment_by_para_id",
            "undo",
            "redo",
            "query",
            "export_docx",
            "close_docx",
            "list_sessions",
        ] {
            assert!(names.contains(&expected), "missing tool: {expected}");
        }
    }

    /// Fetch the `tools/list` payload as the client sees it.
    fn listed_tools(server: &mut Server) -> Vec<Value> {
        let request = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).to_string();
        let response: Value =
            serde_json::from_str(&server.handle_message(&request).unwrap()).unwrap();
        response["result"]["tools"].as_array().unwrap().clone()
    }

    /// `title`s of the variants a `oneOf` schema accepts.
    fn one_of_names(schema: &Value) -> Vec<String> {
        schema["oneOf"]
            .as_array()
            .expect("oneOf schema")
            .iter()
            .map(|variant| variant["title"].as_str().unwrap().to_string())
            .collect()
    }

    /// Variant names of an enum declared in `jade-docx-headless-api`, in the
    /// snake_case form serde puts on the wire. Deriving them from the source
    /// (rather than a second hand-written list) is what makes the catalogue
    /// tests real drift guards: a command added to the API but not described
    /// here fails the build's test run.
    fn api_variant_names(enum_name: &str) -> Vec<String> {
        const API_SOURCE: &str = include_str!("../../jade-docx-headless-api/src/lib.rs");
        let body = API_SOURCE
            .split_once(&format!("pub enum {enum_name} {{\n"))
            .unwrap_or_else(|| panic!("{enum_name} declaration"))
            .1;
        let mut names = Vec::new();
        for line in body.lines() {
            if line == "}" {
                break; // end of the enum body
            }
            // Variants sit at four-space indentation and start uppercase;
            // fields, attributes and doc comments never match both.
            let Some(rest) = line.strip_prefix("    ") else {
                continue;
            };
            if !rest.starts_with(|c: char| c.is_ascii_uppercase()) {
                continue;
            }
            let ident: String = rest
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            let mut snake = String::new();
            for (index, ch) in ident.char_indices() {
                if ch.is_ascii_uppercase() && index > 0 {
                    snake.push('_');
                }
                snake.push(ch.to_ascii_lowercase());
            }
            names.push(snake);
        }
        names
    }

    /// Every property of every schema in `value` must carry a description —
    /// that text is all a client has to choose an argument by.
    fn assert_every_property_is_described(value: &Value, path: &str) {
        match value {
            Value::Object(map) => {
                if let Some(Value::Object(properties)) = map.get("properties") {
                    for (name, property) in properties {
                        assert!(
                            property
                                .get("description")
                                .and_then(Value::as_str)
                                .is_some_and(|text| !text.trim().is_empty()),
                            "{path}.{name} has no description",
                        );
                    }
                }
                for (key, child) in map {
                    assert_every_property_is_described(child, &format!("{path}.{key}"));
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    assert_every_property_is_described(item, &format!("{path}[{index}]"));
                }
            }
            _ => {}
        }
    }

    #[test]
    fn apply_commands_schema_exposes_dry_run_and_the_command_catalogue() {
        let mut server = test_server();
        let tools = listed_tools(&mut server);
        let apply = tools
            .iter()
            .find(|tool| tool["name"] == json!("apply_commands"))
            .expect("apply_commands tool");

        let properties = &apply["inputSchema"]["properties"];
        assert_eq!(
            properties["coordinates"]["enum"],
            json!(["sequential", "snapshot"]),
        );
        assert!(properties.get("dry_run").is_some());
        assert!(properties.get("include_diff").is_some());
        assert!(properties.get("queries").is_some());

        // The command shapes must be in the schema, not just named in prose.
        let commands = one_of_names(&properties["commands"]["items"]);
        for command_name in [
            "insert_text",
            "replace_placeholder",
            "clone_table_row",
            "fill_table_row",
            "insert_after_heading",
            "insert_before_heading",
            "set_paragraph_shading",
            "set_paragraph_borders",
            "apply_multilevel_list",
            "apply_custom_multilevel_list",
        ] {
            assert!(
                commands.contains(&command_name.to_string()),
                "commands schema should describe {command_name}",
            );
        }
        let insert_text = properties["commands"]["items"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|variant| variant["title"] == json!("insert_text"))
            .expect("insert_text variant");
        assert_eq!(
            insert_text["properties"]["type"]["const"],
            json!("insert_text")
        );
        assert_eq!(
            insert_text["required"],
            json!(["type", "paragraph", "offset", "text"]),
        );
    }

    /// Headings the navigation pane finds through the resolved `w:outlineLvl`
    /// (a localized heading style id, a direct `w:outlineLvl`) are reachable
    /// through the heading-relative commands, not only `Heading1`-style ids.
    #[test]
    fn heading_relative_inserts_find_outline_level_headings() {
        let fixture = |name: &str| {
            format!(
                "{}/../../public/fixtures/{name}",
                env!("CARGO_MANIFEST_DIR")
            )
        };
        // (fixture, command, heading, occurrence, inserted paragraph, heading paragraph)
        let cases = [
            (
                "numbering-restart.docx",
                "insert_after_heading",
                "Article1",
                0,
                2,
                1,
            ),
            (
                "toc-direct-outline-levels.docx",
                "insert_before_heading",
                "Direct outline paragraph",
                0,
                6,
                7,
            ),
            ("toc.docx", "insert_after_heading", "Test", 1, 16, 15),
        ];
        let mut server = test_server();
        for (name, command, heading, occurrence, inserted_at, heading_at) in cases {
            let session = open_session(&mut server, &fixture(name));
            try_tool(
                &mut server,
                "apply_commands",
                json!({
                    "session_id": session,
                    "commands": [{
                        "type": command,
                        "heading": heading,
                        "occurrence": occurrence,
                        "text": "Inserted",
                    }],
                }),
            )
            .unwrap_or_else(|error| panic!("{name}: {command} {heading:?}: {error}"));
            assert_eq!(
                paragraph_text(&mut server, &session, inserted_at),
                "Inserted",
                "{name}",
            );
            assert_eq!(
                paragraph_text(&mut server, &session, heading_at),
                heading,
                "{name}",
            );
        }
    }

    #[test]
    fn snapshot_coordinates_rebase_front_to_back_text_edits() {
        let mut server = test_server();

        let sequential = open_session(&mut server, HELLO_DOCX);
        try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": sequential,
                "coordinates": "sequential",
                "commands": [
                    { "type": "insert_text", "paragraph": 0, "offset": 0, "text": "AAA" },
                    { "type": "insert_text", "paragraph": 0, "offset": 5, "text": "BBB" },
                ],
            }),
        )
        .expect("sequential batch");
        assert_eq!(
            paragraph_text(&mut server, &sequential, 0),
            "AAAHeBBBllo\u{c}",
            "the default mode must keep the existing evolving-document semantics",
        );

        let snapshot = open_session(&mut server, HELLO_DOCX);
        try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": snapshot,
                "coordinates": "snapshot",
                "commands": [
                    { "type": "insert_text", "paragraph": 0, "offset": 0, "text": "AAA" },
                    { "type": "insert_text", "paragraph": 0, "offset": 5, "text": "BBB" },
                ],
            }),
        )
        .expect("snapshot batch");
        assert_eq!(
            paragraph_text(&mut server, &snapshot, 0),
            "AAAHelloBBB\u{c}",
        );
        let undone = try_tool(&mut server, "undo", json!({ "session_id": snapshot }))
            .expect("undo snapshot batch");
        assert_eq!(undone["steps"], 1);
        assert_eq!(
            paragraph_text(&mut server, &snapshot, 0),
            "AAAHello\u{c}",
            "undo order must still follow the caller's command order",
        );
        try_tool(&mut server, "undo", json!({ "session_id": snapshot }))
            .expect("undo first snapshot command");
        assert_eq!(paragraph_text(&mut server, &snapshot, 0), "Hello\u{c}");
    }

    #[test]
    fn snapshot_coordinates_rebase_text_range_formatting_and_allow_style_overlap() {
        let mut server = test_server();
        let session = open_session(&mut server, HELLO_DOCX);

        try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": session,
                "coordinates": "snapshot",
                "commands": [
                    { "type": "insert_text", "paragraph": 0, "offset": 0, "text": "A" },
                    {
                        "type": "apply_bold",
                        "start_paragraph": 0,
                        "start_offset": 1,
                        "end_paragraph": 0,
                        "end_offset": 4
                    },
                    {
                        "type": "apply_italic",
                        "start_paragraph": 0,
                        "start_offset": 2,
                        "end_paragraph": 0,
                        "end_offset": 5
                    }
                ],
            }),
        )
        .expect("snapshot formatting batch");

        let paragraph = paragraph_snapshot(&mut server, &session, 0);
        assert_eq!(paragraph["text"], "AHello\u{c}");
        let runs = paragraph["runs"].as_array().expect("runs array");
        for offset in 0..7 {
            let run = runs
                .iter()
                .find(|run| {
                    run["start_offset"]
                        .as_u64()
                        .is_some_and(|start| start <= offset)
                        && run["end_offset"].as_u64().is_some_and(|end| offset < end)
                })
                .expect("each byte belongs to a run");
            assert_eq!(
                run["style"]["bold"].as_bool() == Some(true),
                (2..5).contains(&offset),
                "bold at byte {offset}",
            );
            assert_eq!(
                run["style"]["italic"].as_bool() == Some(true),
                (3..6).contains(&offset),
                "italic at byte {offset}",
            );
        }
    }

    #[test]
    fn snapshot_coordinates_reject_a_command_targeting_a_removed_paragraph() {
        let mut server = test_server();
        let session = open_session(&mut server, CONTRACT_DOCX);
        let before = paragraph_text(&mut server, &session, 1);

        let error = try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": session,
                "coordinates": "snapshot",
                "commands": [
                    {
                        "type": "replace_range",
                        "start_paragraph": 0,
                        "start_offset": 0,
                        "end_paragraph": 2,
                        "end_offset": 0,
                        "text": ""
                    },
                    { "type": "insert_text", "paragraph": 1, "offset": 0, "text": "must fail" },
                ],
            }),
        )
        .expect_err("the second command targets a paragraph removed by the first");

        assert!(error.contains("index 1"), "{error}");
        assert!(error.contains("overlap"), "{error}");
        assert_eq!(
            paragraph_text(&mut server, &session, 1),
            before,
            "snapshot conflicts must be rejected before any command mutates the session",
        );
    }

    #[test]
    fn snapshot_coordinates_reject_ambiguous_and_unsupported_batches_atomically() {
        let mut server = test_server();

        let same_point = open_session(&mut server, HELLO_DOCX);
        let before = paragraph_text(&mut server, &same_point, 0);
        let error = try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": same_point,
                "coordinates": "snapshot",
                "commands": [
                    { "type": "insert_text", "paragraph": 0, "offset": 1, "text": "A" },
                    { "type": "insert_text", "paragraph": 0, "offset": 1, "text": "B" },
                ],
            }),
        )
        .expect_err("two insertions at one snapshot point have no unambiguous order");
        assert!(error.contains("overlap"), "{error}");
        assert_eq!(paragraph_text(&mut server, &same_point, 0), before);

        let unsupported = open_session(&mut server, HELLO_DOCX);
        let before = paragraph_text(&mut server, &unsupported, 0);
        let error = try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": unsupported,
                "coordinates": "snapshot",
                "commands": [
                    { "type": "insert_text", "paragraph": 0, "offset": 0, "text": "A" },
                    {
                        "type": "set_paragraph_style",
                        "start_paragraph": 0,
                        "end_paragraph": 0,
                        "style_id": null
                    },
                ],
            }),
        )
        .expect_err("partially rebasing a mixed batch would be unsafe");
        assert!(error.contains("index 1"), "{error}");
        assert!(error.contains("set_paragraph_style"), "{error}");
        assert_eq!(paragraph_text(&mut server, &unsupported, 0), before);
    }

    #[test]
    fn snapshot_coordinates_validate_the_mode_and_work_in_dry_run() {
        let mut server = test_server();
        let session = open_session(&mut server, HELLO_DOCX);
        let before = paragraph_text(&mut server, &session, 0);

        let error = try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": session,
                "coordinates": "stale",
                "commands": [
                    { "type": "insert_text", "paragraph": 0, "offset": 0, "text": "A" }
                ],
            }),
        )
        .expect_err("unknown coordinate modes must not be ignored");
        assert!(error.contains("sequential"), "{error}");
        assert!(error.contains("snapshot"), "{error}");

        let result = try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": session,
                "coordinates": "snapshot",
                "dry_run": true,
                "commands": [
                    { "type": "insert_text", "paragraph": 0, "offset": 0, "text": "AAA" },
                    { "type": "insert_text", "paragraph": 0, "offset": 5, "text": "BBB" },
                ],
                "queries": [{ "type": "paragraph_text", "paragraph": 0 }],
            }),
        )
        .expect("snapshot dry run");
        assert_eq!(result["query"]["results"][0]["text"], "AAAHelloBBB\u{c}");
        assert_eq!(paragraph_text(&mut server, &session, 0), before);
    }

    #[test]
    fn catalogue_describes_every_headless_command_and_query() {
        let commands = api_variant_names("HeadlessCommand");
        let queries = api_variant_names("HeadlessQuery");
        assert!(
            commands.len() > 40 && queries.len() > 10,
            "source scan found too few variants ({} commands, {} queries) — the parser drifted",
            commands.len(),
            queries.len(),
        );

        for (declared, described, label) in
            [(commands, COMMANDS, "command"), (queries, QUERIES, "query")]
        {
            let described: Vec<&str> = described.iter().map(|variant| variant.name).collect();
            for name in &declared {
                assert!(
                    described.contains(&name.as_str()),
                    "{label} {name} exists in jade-docx-headless-api but is missing from the MCP schema",
                );
            }
            for name in &described {
                assert!(
                    declared.iter().any(|variant| variant == name),
                    "{label} {name} is advertised but no longer exists in jade-docx-headless-api",
                );
            }
        }
    }

    /// A value of the shape the schema promises for one argument.
    fn sample_value(ty: ArgType) -> Value {
        match ty {
            ArgType::Index | ArgType::Int => json!(0),
            ArgType::Number | ArgType::NullableNumber => json!(1.0),
            ArgType::Text | ArgType::NullableText => json!("x"),
            ArgType::Flag => json!(true),
            ArgType::Choice(values) | ArgType::NullableChoice(values) => json!(values[0]),
            ArgType::NumberList => json!([1.0]),
            ArgType::TextList => json!(["x"]),
            ArgType::CoverParagraphList => json!([{}]),
            ArgType::CustomNumberingLevelList => json!([{
                "numFmt": "decimal",
                "lvlText": "%1.",
                "indentLeft": 720,
                "hanging": 360,
                "start": 1
            }]),
            ArgType::CitationSource => json!({
                "tag": "Smith2024",
                "type": "Book",
                "author": "Smith, Jane",
                "title": "Example",
                "year": "2024"
            }),
            ArgType::DocumentProtection => json!({
                "edit": "readOnly",
                "enforced": true
            }),
            ArgType::DocumentTheme => json!({
                "name": "Office",
                "majorFont": "Calibri Light",
                "minorFont": "Calibri",
                "eastAsiaFont": "等线",
                "dark2": "44546A",
                "light2": "E7E6E6",
                "accent1": "4472C4",
                "accent2": "ED7D31",
                "accent3": "A5A5A5",
                "accent4": "FFC000",
                "accent5": "5B9BD5",
                "accent6": "70AD47"
            }),
            ArgType::CellList => json!([{ "row": 0, "column": 0, "column_span": 1 }]),
            ArgType::SortKeyList => json!([{ "column": 0, "kind": "number", "descending": true }]),
            ArgType::BorderEdge => json!({ "style": "single", "width": 0.5, "color": "#000000" }),
            ArgType::Object
            | ArgType::CommandList
            | ArgType::QueryList
            | ArgType::ResultList
            | ArgType::MatchList => {
                unreachable!("command and query variants never take these shapes")
            }
        }
    }

    /// Build a payload for `variant` straight from its advertised schema.
    fn sample_payload(variant: &Variant, include_optional: bool) -> Value {
        let mut payload = serde_json::Map::new();
        payload.insert("type".to_string(), json!(variant.name));
        for arg in variant.scope.iter().chain(variant.args) {
            if arg.required || include_optional {
                payload.insert(arg.name.to_string(), sample_value(arg.ty));
            }
        }
        Value::Object(payload)
    }

    /// The advertised schema has to be the truth, not a parallel description of
    /// it: a payload built only from the `required` fields must deserialize,
    /// and a payload using every advertised field must round-trip to exactly
    /// the field set the API serializes. That catches a required/optional
    /// mismatch, a misspelled field and a wrong type in one pass.
    fn assert_schema_matches_api<T>(variants: &[Variant], label: &str)
    where
        T: serde::de::DeserializeOwned + serde::Serialize,
    {
        for variant in variants {
            let name = variant.name;
            let minimal = sample_payload(variant, false);
            serde_json::from_value::<T>(minimal.clone()).unwrap_or_else(|err| {
                panic!("{label} {name} advertises too few required fields: {err} ({minimal})")
            });

            let full = sample_payload(variant, true);
            let parsed: T = serde_json::from_value(full.clone()).unwrap_or_else(|err| {
                panic!("{label} {name} advertises a field the API rejects: {err} ({full})")
            });
            let mut advertised: Vec<&str> = full
                .as_object()
                .unwrap()
                .keys()
                .map(|k| k.as_str())
                .collect();
            let serialized = serde_json::to_value(&parsed).unwrap();
            let mut actual: Vec<&str> = serialized
                .as_object()
                .unwrap_or_else(|| panic!("{label} {name} did not serialize as an object"))
                .keys()
                .map(|k| k.as_str())
                .collect();
            advertised.sort_unstable();
            actual.sort_unstable();
            assert_eq!(
                advertised, actual,
                "{label} {name}: advertised fields differ from the API's own fields",
            );
        }
    }

    #[test]
    fn advertised_command_schemas_match_the_api() {
        assert_schema_matches_api::<HeadlessCommand>(COMMANDS, "command");
    }

    #[test]
    fn document_theme_command_is_advertised_and_applies_through_mcp() {
        let mut server = test_server();
        let session = open_session(&mut server, HELLO_DOCX);
        let applied = try_tool(
            &mut server,
            "apply_commands",
            json!({
                "session_id": session,
                "commands": [{
                    "type": "set_document_theme",
                    "theme": {
                        "name": "MCP & テーマ",
                        "majorFont": "Aptos Display",
                        "minorFont": "Aptos & Body",
                        "eastAsiaFont": "游ゴシック",
                        "dark2": "102A43",
                        "light2": "F0F4F8",
                        "accent1": "#486581",
                        "accent2": "829AB1",
                        "accent3": "9FB3C8",
                        "accent4": "BCCCDC",
                        "accent5": "D9E2EC",
                        "accent6": "243B53"
                    }
                }]
            }),
        )
        .expect("apply MCP document theme");
        assert_eq!(applied["applied"], json!(1));
        assert_eq!(applied["can_undo"], json!(true));

        let exported = try_tool(&mut server, "export_docx", json!({ "session_id": session }))
            .expect("export themed package");
        let bytes = BASE64
            .decode(exported["bytes_base64"].as_str().expect("base64 DOCX"))
            .expect("decode DOCX");
        let theme = part_xml(&bytes, "word/theme/theme1.xml").expect("theme part");
        assert!(theme.contains("MCP &amp; テーマ"));
        assert!(theme.contains("Aptos &amp; Body"));
        assert!(theme.contains("486581"));
    }

    #[test]
    fn advertised_query_schemas_match_the_api() {
        assert_schema_matches_api::<HeadlessQuery>(QUERIES, "query");
    }

    #[test]
    fn every_tool_publishes_a_documented_interface() {
        // An injected tool is held to the same bar as a built-in one, so the
        // server is checked with one registered.
        let mut server = test_server();
        server
            .register_tool(sample_spec(), |_ctx, _args| Ok(json!({ "ok": true })))
            .expect("registration");
        for tool in listed_tools(&mut server) {
            let name = tool["name"].as_str().expect("tool name");
            for field in ["title", "description"] {
                assert!(
                    tool[field]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty()),
                    "{name} has no {field}",
                );
            }
            assert_eq!(tool["inputSchema"]["type"], json!("object"), "{name}");
            assert_eq!(
                tool["inputSchema"]["additionalProperties"],
                json!(false),
                "{name} should reject unknown arguments",
            );
            assert_eq!(tool["outputSchema"]["type"], json!("object"), "{name}");
            for hint in [
                "readOnlyHint",
                "destructiveHint",
                "idempotentHint",
                "openWorldHint",
            ] {
                assert!(tool["annotations"][hint].is_boolean(), "{name}.{hint}");
            }
            assert_every_property_is_described(&tool["inputSchema"], &format!("{name}.input"));
            assert_every_property_is_described(&tool["outputSchema"], &format!("{name}.output"));
        }
    }

    /// 拡張機構が入っても、従来のクライアントから見える形が変わらないこと。
    ///
    /// 何も登録しないサーバーは拡張機構が入る前と同じ物でなければならない。
    /// stock binary をそのまま使っている既存の利用者がいる以上、ここが崩れたら
    /// それは breaking change になる。
    #[test]
    fn the_stock_server_publishes_exactly_the_built_in_catalogue() {
        let mut server = test_server();
        let listed = listed_tools(&mut server);

        let expected: Vec<&str> = TOOLS.iter().map(|tool| tool.name).collect();
        let actual: Vec<&str> = listed
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(actual, expected, "built-in tools, in their original order");
        assert_eq!(
            listed,
            TOOLS.iter().map(tool_definition).collect::<Vec<_>>(),
            "a server with nothing injected publishes the built-in definitions verbatim",
        );
    }

    /// 注入しても、既存ツールの公開内容は 1 バイトも変わらないこと。
    #[test]
    fn injecting_a_tool_leaves_the_built_in_catalogue_untouched() {
        let mut stock = test_server();
        let before = listed_tools(&mut stock);

        let mut extended = test_server();
        extended
            .register_tool(sample_spec(), |_ctx, _args| Ok(json!({})))
            .expect("registration");
        let after = listed_tools(&mut extended);

        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(
            after[..before.len()],
            before[..],
            "injected tools are appended; they never rewrite what a client already knew",
        );
        assert_eq!(after[before.len()]["name"], json!("count_paragraphs"));
    }

    /// 従来どおりの使い方 — 自前のループで `handle_message` を回す — が
    /// そのまま動くこと。`serve_stdio` はその上に足した便利関数でしかない。
    #[test]
    fn driving_the_server_by_hand_still_works() {
        let mut server = test_server();

        let initialize: Value = serde_json::from_str(
            &server
                .handle_message(
                    &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} })
                        .to_string(),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            initialize["result"]["serverInfo"]["name"],
            json!(SERVER_NAME)
        );

        let session = open_session(&mut server, HELLO_DOCX);
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 0, "offset": 0, "text": "OLD-" }]),
        );
        assert!(paragraph_text(&mut server, &session, 0).starts_with("OLD-"));

        // Unknown tools keep failing as a JSON-RPC error, not as a tool result:
        // the extension lookup falls through to the same message as before.
        let unknown: Value = serde_json::from_str(
            &server
                .handle_message(&call(9, "does_not_exist", json!({})))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], json!(-32602));
        assert_eq!(
            unknown["error"]["message"],
            json!("unknown tool: does_not_exist"),
        );
    }

    /// Commands in one `apply_commands` batch run against the document as the
    /// previous command left it: their coordinates are deliberately not rebased.
    ///
    /// This is pinned because the failure is silent. A caller who takes offsets
    /// from one `search_text` sweep and emits them front-to-back — the natural
    /// thing to do — corrupts the paragraph instead of getting an error, and the
    /// corruption looks like plausible text. Changing this behaviour would also
    /// silently reinterpret every batch callers have already written, so it has
    /// to be a deliberate, visible decision rather than a refactor.
    #[test]
    fn positional_commands_in_a_batch_are_not_rebased() {
        let mut server = test_server();
        let session = open_session(&mut server, HELLO_DOCX);
        // The fixture paragraph ends with a page-break character.
        assert_eq!(paragraph_text(&mut server, &session, 0), "Hello\u{c}");

        // Front-to-back: offset 5 is read against "AAAHello", landing mid-word.
        apply(
            &mut server,
            &session,
            json!([
                { "type": "insert_text", "paragraph": 0, "offset": 0, "text": "AAA" },
                { "type": "insert_text", "paragraph": 0, "offset": 5, "text": "BBB" },
            ]),
        );
        assert_eq!(paragraph_text(&mut server, &session, 0), "AAAHeBBBllo\u{c}");

        // Back-to-front — the ordering the tool description prescribes — keeps
        // every coordinate valid, because an edit only moves what follows it.
        let session = open_session(&mut server, HELLO_DOCX);
        apply(
            &mut server,
            &session,
            json!([
                { "type": "insert_text", "paragraph": 0, "offset": 5, "text": "BBB" },
                { "type": "insert_text", "paragraph": 0, "offset": 0, "text": "AAA" },
            ]),
        );
        assert_eq!(paragraph_text(&mut server, &session, 0), "AAAHelloBBB\u{c}");
    }

    /// A tool that opens a session can be injected too — a document has to
    /// enter the server somehow, and the consumer may fetch it from somewhere
    /// this crate knows nothing about.
    #[test]
    fn an_injected_tool_can_open_a_session() {
        let mut server = test_server();
        server
            .register_session_tool(
                ToolSpec::new(
                    "open_fixture",
                    "Open a fixture",
                    "Load a .docx the consumer resolved itself.",
                )
                .argument(ToolArg::text("path", "Where the consumer found it."))
                .result(ToolArg::text("session_id", "The new session.")),
                |sessions, args| {
                    let path = args
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or("open_fixture requires `path`")?;
                    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
                    let doc = block_on(WordDocument::load(&bytes, &Options::default()))
                        .map_err(|err| err.to_string())?;
                    let session_id = sessions.open(doc);
                    // The store hands the session straight back, so the tool can
                    // report something about the document it just opened.
                    let paragraphs = sessions
                        .get(&session_id)
                        .map_or(0, |ctx| ctx.document().paragraph_count());
                    Ok(json!({ "session_id": session_id, "paragraphs": paragraphs }))
                },
            )
            .expect("registration");

        // `session_id` is not an argument: this tool is what produces one.
        let tools = listed_tools(&mut server);
        let injected = tools
            .iter()
            .find(|tool| tool["name"] == json!("open_fixture"))
            .expect("listed");
        assert_eq!(injected["inputSchema"]["required"], json!(["path"]));
        assert!(injected["inputSchema"]["properties"]["session_id"].is_null());

        let opened = server
            .handle_message(&call(9, "open_fixture", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let opened = tool_json(&opened);
        let session = opened["session_id"].as_str().expect("session id");
        assert!(opened["paragraphs"].as_u64().unwrap_or(0) > 0);

        // The id is the same kind `open_docx` mints, and the built-in tools
        // accept it.
        assert!(session.starts_with("doc-"));
        let text = paragraph_text(&mut server, session, 0);
        assert!(!text.is_empty(), "built-in query works on the new session");
        let listed = server
            .handle_message(&call(10, "list_sessions", json!({})))
            .unwrap();
        assert!(tool_json(&listed)["session_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == session));

        // …and the built-in close drops it.
        server
            .handle_message(&call(11, "close_docx", json!({ "session_id": session })))
            .unwrap();
        let listed = server
            .handle_message(&call(12, "list_sessions", json!({})))
            .unwrap();
        assert!(tool_json(&listed)["session_ids"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    /// セッションを開くツールは `session_id` を自分で宣言してもよい
    /// （既存セッションへ開き直す用途）。セッションスコープのツールでは
    /// 引き続き予約語として弾く。
    #[test]
    fn only_session_scoped_tools_reserve_the_session_id_argument() {
        let mut server = test_server();
        assert!(matches!(
            server.register_tool(
                ToolSpec::new("owns_session", "T", "…")
                    .argument(ToolArg::text("session_id", "mine")),
                |_, _| Ok(json!({}))
            ),
            Err(RegisterError::ReservedArgument { .. }),
        ));
        server
            .register_session_tool(
                ToolSpec::new("reopen", "Reopen", "Reopen into an existing session.")
                    .argument(ToolArg::text("session_id", "Session to replace.")),
                |_, _| Ok(json!({})),
            )
            .expect("session-opening tools may declare it themselves");
    }

    /// A domain tool of the kind a consumer would inject.
    fn sample_spec() -> ToolSpec {
        ToolSpec::new(
            "count_paragraphs",
            "Count paragraphs",
            "How many paragraphs the open document has.",
        )
        .read_only()
        .argument(ToolArg::text("note", "Free-form note echoed back.").optional())
        .result(ToolArg::index("paragraphs", "The paragraph count."))
    }

    /// 注入したツールが、組み込みツールと見分けのつかない形で公開されること。
    #[test]
    fn an_injected_tool_is_published_like_a_built_in_one() {
        let mut server = test_server();
        server
            .register_tool(sample_spec(), |ctx, args| {
                Ok(json!({
                    "paragraphs": ctx.document().paragraph_count(),
                    "note": args.get("note").cloned().unwrap_or(Value::Null),
                }))
            })
            .expect("registration");

        let tools = listed_tools(&mut server);
        let injected = tools
            .iter()
            .find(|tool| tool["name"] == json!("count_paragraphs"))
            .expect("the injected tool is listed");

        assert_eq!(injected["title"], json!("Count paragraphs"));
        assert_eq!(injected["annotations"]["readOnlyHint"], json!(true));
        // The server owns `session_id`: the tool did not declare it, and a
        // client still sees it as required.
        let input = &injected["inputSchema"];
        assert_eq!(input["required"], json!(["session_id"]));
        assert!(input["properties"]["session_id"]["description"].is_string());
        assert!(input["properties"]["note"]["description"].is_string());
        assert_eq!(input["additionalProperties"], json!(false));
        assert_eq!(
            injected["outputSchema"]["properties"]["paragraphs"]["type"],
            json!("integer"),
        );

        let session = open_session(&mut server, HELLO_DOCX);
        let response = server
            .handle_message(&call(
                9,
                "count_paragraphs",
                json!({ "session_id": session, "note": "hi" }),
            ))
            .unwrap();
        let result = tool_json(&response);
        assert!(result["paragraphs"].as_u64().unwrap_or(0) > 0);
        assert_eq!(result["note"], json!("hi"));

        // …and it is delivered as structuredContent like everything else.
        let value: Value = serde_json::from_str(&response).unwrap();
        assert!(value["result"]["structuredContent"]["paragraphs"].is_number());
    }

    /// 注入したツールの編集が undo で戻せること（履歴を共有していること）。
    #[test]
    fn an_injected_tools_edits_join_the_session_undo_history() {
        let mut server = test_server();
        server
            .register_tool(
                ToolSpec::new("prefix_first", "Prefix", "Insert text at the very start.")
                    .argument(ToolArg::text("text", "What to insert."))
                    .result(ToolArg::boolean("ok", "Always true.")),
                |ctx, args| {
                    let text = args
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or("prefix_first requires `text`")?;
                    ctx.apply(vec![serde_json::from_value(json!({
                        "type": "insert_text",
                        "paragraph": 0,
                        "offset": 0,
                        "text": text,
                    }))
                    .map_err(|err: serde_json::Error| err.to_string())?])?;
                    Ok(json!({ "ok": true, "can_undo": ctx.can_undo() }))
                },
            )
            .expect("registration");

        let session = open_session(&mut server, HELLO_DOCX);
        let before = paragraph_text(&mut server, &session, 0);

        let response = server
            .handle_message(&call(
                9,
                "prefix_first",
                json!({ "session_id": session, "text": "EXT-" }),
            ))
            .unwrap();
        assert_eq!(tool_json(&response)["can_undo"], json!(true));
        assert!(paragraph_text(&mut server, &session, 0).starts_with("EXT-"));

        // The built-in undo steps back over the injected tool's edit.
        let undo = server
            .handle_message(&call(10, "undo", json!({ "session_id": session })))
            .unwrap();
        assert_eq!(tool_json(&undo)["undone"], json!(true));
        assert_eq!(paragraph_text(&mut server, &session, 0), before);
    }

    /// セッションの解決はサーバー側の仕事で、注入側は書かなくてよいこと。
    #[test]
    fn the_server_resolves_the_session_for_injected_tools() {
        let mut server = test_server();
        server
            .register_tool(sample_spec(), |ctx, _args| {
                Ok(json!({ "paragraphs": ctx.document().paragraph_count() }))
            })
            .expect("registration");

        let response = server
            .handle_message(&call(
                9,
                "count_paragraphs",
                json!({ "session_id": "nope" }),
            ))
            .unwrap();
        let value: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["result"]["isError"], json!(true));
        assert!(value["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown session_id"));
    }

    /// 使えないツールを公開してしまう前に、登録で落とすこと。
    #[test]
    fn registration_rejects_tools_a_client_could_not_use() {
        let mut server = test_server();
        server
            .register_tool(sample_spec(), |_, _| Ok(json!({})))
            .unwrap();

        assert_eq!(
            server.register_tool(sample_spec(), |_, _| Ok(json!({}))),
            Err(RegisterError::DuplicateName("count_paragraphs".to_string())),
            "the same name twice",
        );
        assert_eq!(
            server.register_tool(
                ToolSpec::new("query", "Shadow", "Shadows a built-in."),
                |_, _| Ok(json!({}))
            ),
            Err(RegisterError::DuplicateName("query".to_string())),
            "shadowing a built-in",
        );
        assert!(matches!(
            server.register_tool(ToolSpec::new("blank", "", "…"), |_, _| Ok(json!({}))),
            Err(RegisterError::Missing { field: "title", .. }),
        ));
        assert!(matches!(
            server.register_tool(
                ToolSpec::new("undocumented", "T", "…").argument(ToolArg::text("who", "")),
                |_, _| Ok(json!({}))
            ),
            Err(RegisterError::UndocumentedField { .. }),
        ));
        assert!(matches!(
            server.register_tool(
                ToolSpec::new("owns_session", "T", "…")
                    .argument(ToolArg::text("session_id", "mine")),
                |_, _| Ok(json!({}))
            ),
            Err(RegisterError::ReservedArgument { .. }),
        ));

        // A rejected registration leaves the catalogue untouched.
        let names: Vec<String> = listed_tools(&mut server)
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names
                .iter()
                .filter(|name| *name == "count_paragraphs")
                .count(),
            1,
        );
        assert!(!names.iter().any(|name| name == "blank"));
    }

    /// A consumer can publish its own identity; the stock default is unchanged.
    ///
    /// A binary that registers its own tools is a different server as far as the
    /// client is concerned, and the name it shows in the UI and writes to logs
    /// should say so.
    #[test]
    fn a_consumer_can_rename_the_server_it_publishes() {
        let initialize = |server: &mut Server| -> Value {
            let response = server
                .handle_message(
                    &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }).to_string(),
                )
                .unwrap();
            serde_json::from_str::<Value>(&response).unwrap()["result"].clone()
        };

        // Unchanged for the stock server.
        let stock = initialize(&mut test_server());
        assert_eq!(stock["serverInfo"]["name"], json!(SERVER_NAME));
        assert_eq!(stock["serverInfo"]["title"], json!(SERVER_TITLE));

        let mut renamed = test_server().with_server_info("elsa-headless-mcp", "Elsa DOCX MCP");
        let renamed = initialize(&mut renamed);
        assert_eq!(renamed["serverInfo"]["name"], json!("elsa-headless-mcp"));
        assert_eq!(renamed["serverInfo"]["title"], json!("Elsa DOCX MCP"));
        // The version stays this crate's: it names the engine answering.
        assert_eq!(
            renamed["serverInfo"]["version"],
            stock["serverInfo"]["version"],
        );
        // Nothing but the name moves.
        assert_eq!(renamed["instructions"], stock["instructions"]);
        assert_eq!(renamed["capabilities"], stock["capabilities"]);
    }

    #[test]
    fn initialize_returns_server_instructions() {
        let mut server = test_server();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }).to_string();
        let response: Value =
            serde_json::from_str(&server.handle_message(&request).unwrap()).unwrap();
        let instructions = response["result"]["instructions"]
            .as_str()
            .expect("instructions");
        for expected in ["open_docx", "byte", "session"] {
            assert!(
                instructions.contains(expected),
                "instructions should explain {expected}: {instructions}",
            );
        }
    }

    #[test]
    fn successful_tool_calls_carry_structured_content() {
        let mut server = test_server();
        let response = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let value: Value = serde_json::from_str(&response).unwrap();
        let structured = &value["result"]["structuredContent"];
        assert!(
            structured["session_id"].is_string(),
            "structuredContent should match the advertised outputSchema: {value}",
        );

        // Errors stay text-only: there is no result object to structure.
        let failure = server
            .handle_message(&call(2, "export_docx", json!({ "session_id": "nope" })))
            .unwrap();
        let failure: Value = serde_json::from_str(&failure).unwrap();
        assert_eq!(failure["result"]["isError"], json!(true));
        assert!(failure["result"]["structuredContent"].is_null());
    }

    #[test]
    fn initialized_notification_produces_no_response() {
        let mut server = test_server();
        let notification =
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
        assert!(server.handle_message(&notification).is_none());
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let mut server = test_server();
        let request = json!({ "jsonrpc": "2.0", "id": 9, "method": "does/not/exist" }).to_string();
        let response: Value =
            serde_json::from_str(&server.handle_message(&request).unwrap()).unwrap();
        assert_eq!(response["error"]["code"], json!(-32601));
    }

    #[test]
    fn apply_to_unknown_session_reports_tool_error() {
        let mut server = test_server();
        let response = server
            .handle_message(&call(
                1,
                "apply_commands",
                json!({
                    "session_id": "nope",
                    "commands": [{ "type": "insert_text", "paragraph": 0, "offset": 0, "text": "x" }],
                }),
            ))
            .unwrap();
        let value: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["result"]["isError"], json!(true));
        assert!(value["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown session_id"));
    }

    #[test]
    fn open_apply_query_export_roundtrip() {
        let mut server = test_server();

        // open
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let session_id = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        // apply: insert text at the very start of the first paragraph
        let apply = server
            .handle_message(&call(
                2,
                "apply_commands",
                json!({
                    "session_id": session_id,
                    "commands": [{ "type": "insert_text", "paragraph": 0, "offset": 0, "text": "MCP-" }],
                }),
            ))
            .unwrap();
        assert_eq!(tool_json(&apply)["applied"], json!(1));

        // query: the first paragraph now starts with the inserted text
        let query = server
            .handle_message(&call(
                3,
                "query",
                json!({
                    "session_id": session_id,
                    "queries": [{ "type": "paragraph_text", "paragraph": 0 }],
                }),
            ))
            .unwrap();
        let text = tool_json(&query)["results"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            text.contains("MCP-"),
            "expected inserted text, got {text:?}"
        );

        // export: bytes round-trip back to a real zip (docx) container
        let export = server
            .handle_message(&call(4, "export_docx", json!({ "session_id": session_id })))
            .unwrap();
        let encoded = tool_json(&export)["bytes_base64"]
            .as_str()
            .unwrap()
            .to_string();
        let bytes = BASE64.decode(encoded).unwrap();
        assert!(bytes.starts_with(b"PK"), "exported docx should be a zip");

        // close
        let close = server
            .handle_message(&call(5, "close_docx", json!({ "session_id": session_id })))
            .unwrap();
        assert_eq!(tool_json(&close)["closed"], json!(true));
    }

    #[test]
    fn dry_run_apply_commands_returns_simulated_query_and_does_not_mutate_session() {
        let mut server = test_server();
        let session_id = open_session(&mut server, HELLO_DOCX);

        let dry_run = server
            .handle_message(&call(
                2,
                "apply_commands",
                json!({
                    "session_id": session_id,
                    "dry_run": true,
                    "include_diff": true,
                    "commands": [{ "type": "insert_text", "paragraph": 0, "offset": 0, "text": "DRY-" }],
                    "queries": [{ "type": "paragraph_text", "paragraph": 0 }]
                }),
            ))
            .unwrap();
        let dry_json = tool_json(&dry_run);
        assert_eq!(dry_json["ok"], json!(true));
        assert_eq!(dry_json["dry_run"], json!(true));
        assert_eq!(dry_json["applied"], json!(1));
        assert!(
            dry_json["query"]["results"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .starts_with("DRY-Hello"),
            "dry-run query should see the simulated edit: {dry_json:?}",
        );
        assert!(
            dry_json["diff"]["changed_paragraphs"]
                .as_array()
                .unwrap()
                .contains(&json!(0)),
            "dry-run diff should report the simulated paragraph change",
        );

        let after = paragraph_text(&mut server, &session_id, 0);
        assert!(
            after.starts_with("Hello"),
            "dry-run must not mutate the live session: {after:?}",
        );

        let undo = server
            .handle_message(&call(
                3,
                "undo",
                json!({ "session_id": session_id, "count": 1 }),
            ))
            .unwrap();
        assert_eq!(tool_json(&undo)["steps"], json!(0));
    }

    #[test]
    fn undo_and_redo_revert_and_replay_the_last_edit() {
        let mut server = test_server();

        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let session_id = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        let paragraph_text = |server: &mut Server, id: u64| -> String {
            let query = server
                .handle_message(&call(
                    id,
                    "query",
                    json!({
                        "session_id": session_id,
                        "queries": [{ "type": "paragraph_text", "paragraph": 0 }],
                    }),
                ))
                .unwrap();
            tool_json(&query)["results"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        };

        let original = paragraph_text(&mut server, 2);

        // apply an insert — history now has one step
        let apply = server
            .handle_message(&call(
                3,
                "apply_commands",
                json!({
                    "session_id": session_id,
                    "commands": [{ "type": "insert_text", "paragraph": 0, "offset": 0, "text": "ZZ-" }],
                }),
            ))
            .unwrap();
        let apply_json = tool_json(&apply);
        assert_eq!(apply_json["can_undo"], json!(true));
        assert_eq!(apply_json["can_redo"], json!(false));
        assert!(paragraph_text(&mut server, 4).starts_with("ZZ-"));

        // undo restores the original text
        let undo = server
            .handle_message(&call(5, "undo", json!({ "session_id": session_id })))
            .unwrap();
        let undo_json = tool_json(&undo);
        assert_eq!(undo_json["undone"], json!(true));
        assert_eq!(undo_json["can_redo"], json!(true));
        assert_eq!(paragraph_text(&mut server, 6), original);

        // redo replays the insert
        let redo = server
            .handle_message(&call(7, "redo", json!({ "session_id": session_id })))
            .unwrap();
        assert_eq!(tool_json(&redo)["redone"], json!(true));
        assert!(paragraph_text(&mut server, 8).starts_with("ZZ-"));

        // undo again, then a fresh undo with nothing left reports `undone: false`
        assert_eq!(
            tool_json(
                &server
                    .handle_message(&call(9, "undo", json!({ "session_id": session_id })))
                    .unwrap()
            )["undone"],
            json!(true)
        );
        let empty = server
            .handle_message(&call(10, "undo", json!({ "session_id": session_id })))
            .unwrap();
        let empty_json = tool_json(&empty);
        assert_eq!(empty_json["undone"], json!(false));
        assert_eq!(empty_json["can_undo"], json!(false));
    }

    /// The fidelity guarantee that makes this worth using from an agent: editing
    /// a paragraph and round-tripping through real .docx bytes must NOT drop the
    /// document's tracked changes (what naive re-serializers lose).
    #[test]
    fn export_reopen_preserves_tracked_changes() {
        let mut server = test_server();

        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": REVIEW_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        let before = revisions(&mut server, &session);
        assert!(
            !before.is_empty(),
            "fixture should contain tracked changes to preserve"
        );

        // Edit a different paragraph (1) so paragraph 0's tracked changes are
        // untouched, then serialize to bytes and reparse them fresh.
        server
            .handle_message(&call(
                2,
                "apply_commands",
                json!({
                    "session_id": session,
                    "commands": [{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "X" }],
                }),
            ))
            .unwrap();
        let export = server
            .handle_message(&call(3, "export_docx", json!({ "session_id": session })))
            .unwrap();
        let bytes_base64 = tool_json(&export)["bytes_base64"]
            .as_str()
            .unwrap()
            .to_string();
        let reopen = server
            .handle_message(&call(
                4,
                "open_docx",
                json!({ "bytes_base64": bytes_base64 }),
            ))
            .unwrap();
        let session2 = tool_json(&reopen)["session_id"]
            .as_str()
            .unwrap()
            .to_string();

        let after = revisions(&mut server, &session2);
        assert_eq!(
            before, after,
            "tracked changes must survive the agent edit + export/reopen"
        );

        // …and the edit itself made it through.
        let query = server
            .handle_message(&call(
                5,
                "query",
                json!({
                    "session_id": session2,
                    "queries": [{ "type": "paragraph_text", "paragraph": 1 }],
                }),
            ))
            .unwrap();
        assert!(tool_json(&query)["results"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains('X'));
    }

    /// Comments (stored in `comments.xml`) must also survive an agent edit +
    /// export/reopen, even when the edit is in the commented paragraph itself.
    #[test]
    fn export_reopen_preserves_comments() {
        let mut server = test_server();

        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": COMMENT_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        let before = comments(&mut server, &session);
        assert!(
            !before.is_empty(),
            "fixture should contain comments to preserve"
        );

        // The single paragraph is the one the comments anchor into; editing it
        // shifts the anchors but must not drop the comment definitions.
        server
            .handle_message(&call(
                2,
                "apply_commands",
                json!({
                    "session_id": session,
                    "commands": [{ "type": "insert_text", "paragraph": 0, "offset": 0, "text": "X" }],
                }),
            ))
            .unwrap();
        let export = server
            .handle_message(&call(3, "export_docx", json!({ "session_id": session })))
            .unwrap();
        let bytes_base64 = tool_json(&export)["bytes_base64"]
            .as_str()
            .unwrap()
            .to_string();
        let reopen = server
            .handle_message(&call(
                4,
                "open_docx",
                json!({ "bytes_base64": bytes_base64 }),
            ))
            .unwrap();
        let session2 = tool_json(&reopen)["session_id"]
            .as_str()
            .unwrap()
            .to_string();

        let after = comments(&mut server, &session2);
        assert_eq!(
            before, after,
            "comments must survive the agent edit + export/reopen"
        );
    }

    /// Word fields (TOC / PAGEREF — `w:instrText` + `w:fldChar`) must survive an
    /// agent edit + export. There is no field query, so we fingerprint the
    /// exported `document.xml` directly.
    #[test]
    fn export_preserves_field_codes() {
        let original = std::fs::read(TOC_DOCX).unwrap();
        let before = field_marker_counts(&document_xml(&original));
        assert!(
            before.0 > 0 && before.1 > 0 && before.2 > 0,
            "fixture must contain field codes (instrText/fldChar/PAGEREF), got {before:?}"
        );

        let mut server = test_server();
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": TOC_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        // Edit a body paragraph far from the field-bearing TOC at the top.
        server
            .handle_message(&call(
                2,
                "apply_commands",
                json!({
                    "session_id": session,
                    "commands": [{ "type": "insert_text", "paragraph": 33, "offset": 0, "text": "Z" }],
                }),
            ))
            .unwrap();
        let export = server
            .handle_message(&call(3, "export_docx", json!({ "session_id": session })))
            .unwrap();
        let exported = BASE64
            .decode(tool_json(&export)["bytes_base64"].as_str().unwrap())
            .unwrap();
        let after = field_marker_counts(&document_xml(&exported));

        assert_eq!(
            before, after,
            "field codes (instrText/fldChar/PAGEREF) must survive the export"
        );
    }

    /// Text written with XML entity references (`&amp;` `&quot;` `&apos;`
    /// `&lt;` `&gt;`) must survive `export_docx`. Regression guard for #16,
    /// where the raw-XML preservation tree dropped every entity because
    /// quick-xml reports them as a standalone `GeneralRef` event rather than as
    /// part of the surrounding `Text` event.
    #[test]
    fn export_preserves_xml_entity_references_in_text() {
        // `A & B "X" <Y> 'Z'`, written entirely with entity references.
        const ESCAPED: &str = "A &amp; B &quot;X&quot; &lt;Y&gt; &apos;Z&apos;";
        const DECODED: &str = "A & B \"X\" <Y> 'Z'";

        let source = std::fs::read(HELLO_DOCX).unwrap();
        let entity_docx = with_document_xml(&source, |xml| {
            let replaced = xml.replace(
                "<w:t>Hello</w:t>",
                &format!("<w:t xml:space=\"preserve\">{ESCAPED}</w:t>"),
            );
            assert_ne!(replaced, xml, "fixture text to rewrite must be present");
            replaced
        });

        let mut server = test_server();
        let open = server
            .handle_message(&call(
                1,
                "open_docx",
                json!({ "bytes_base64": BASE64.encode(&entity_docx) }),
            ))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        // `hello.docx`'s first paragraph ends with a page break, which
        // `paragraph_text` reports as U+000C; only the run text matters here.
        let body_text = |server: &mut Server, session: &str| {
            paragraph_text(server, session, 0)
                .trim_end_matches('\u{c}')
                .to_string()
        };

        // Parsing already resolves the entities correctly.
        assert_eq!(body_text(&mut server, &session), DECODED);

        // Exporting without touching the paragraph must not lose them either.
        let export = server
            .handle_message(&call(2, "export_docx", json!({ "session_id": session })))
            .unwrap();
        let bytes_base64 = tool_json(&export)["bytes_base64"]
            .as_str()
            .unwrap()
            .to_string();
        let exported = BASE64.decode(&bytes_base64).unwrap();

        // The exported XML must still carry every character (escaped or not).
        let exported_xml = document_xml(&exported);
        assert!(
            exported_xml.contains("A &amp; B \"X\" &lt;Y&gt; 'Z'")
                || exported_xml.contains(ESCAPED),
            "exported document.xml lost entity characters: {exported_xml}"
        );

        // …and re-parsing the exported bytes yields the original text.
        let reopen = server
            .handle_message(&call(
                3,
                "open_docx",
                json!({ "bytes_base64": bytes_base64 }),
            ))
            .unwrap();
        let session2 = tool_json(&reopen)["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(body_text(&mut server, &session2), DECODED);
    }

    /// `undo` / `redo` honour a `count`, stopping early when the stack drains,
    /// and report how many steps were actually taken.
    #[test]
    fn undo_and_redo_honour_count_argument() {
        let mut server = test_server();
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        // Three separate edits → three history entries.
        for (i, text) in ["A", "B", "C"].iter().enumerate() {
            server
                .handle_message(&call(
                    10 + i as u64,
                    "apply_commands",
                    json!({
                        "session_id": session,
                        "commands": [{ "type": "insert_text", "paragraph": 0, "offset": 0, "text": text }],
                    }),
                ))
                .unwrap();
        }

        // Undo 2 of the 3.
        let undo = tool_json(
            &server
                .handle_message(&call(
                    20,
                    "undo",
                    json!({ "session_id": session, "count": 2 }),
                ))
                .unwrap(),
        );
        assert_eq!(undo["steps"], json!(2));
        assert_eq!(undo["can_undo"], json!(true));

        // Ask for more than remain → only the final 1 step is taken.
        let undo_rest = tool_json(
            &server
                .handle_message(&call(
                    21,
                    "undo",
                    json!({ "session_id": session, "count": 9 }),
                ))
                .unwrap(),
        );
        assert_eq!(undo_rest["steps"], json!(1));
        assert_eq!(undo_rest["undone"], json!(true));
        assert_eq!(undo_rest["can_undo"], json!(false));

        // Redo all three back.
        let redo = tool_json(
            &server
                .handle_message(&call(
                    22,
                    "redo",
                    json!({ "session_id": session, "count": 3 }),
                ))
                .unwrap(),
        );
        assert_eq!(redo["steps"], json!(3));
        assert_eq!(redo["can_redo"], json!(false));
    }

    /// The review-loop differentiator: with change tracking on, the agent's own
    /// edits are recorded as Word tracked changes attributed to it — so a human
    /// can see exactly what the AI changed and accept/reject it.
    #[test]
    fn start_change_tracking_records_agent_edits_as_revisions() {
        let mut server = test_server();
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        server
            .handle_message(&call(
                2,
                "start_change_tracking",
                json!({ "session_id": session, "author": "AI" }),
            ))
            .unwrap();
        server
            .handle_message(&call(
                3,
                "apply_commands",
                json!({
                    "session_id": session,
                    "commands": [{ "type": "insert_text", "paragraph": 0, "offset": 0, "text": "NEW " }],
                }),
            ))
            .unwrap();

        let revs = revisions(&mut server, &session);
        assert_eq!(
            revs,
            vec![("inserted".to_string(), "NEW ".to_string())],
            "the agent's insert should be a tracked insertion"
        );
    }

    /// `accept_revisions` applies the existing tracked changes (keep insertions,
    /// drop deletions) and clears the revision list.
    #[test]
    fn accept_revisions_resolves_existing_changes() {
        let mut server = test_server();
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": REVIEW_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();
        assert!(!revisions(&mut server, &session).is_empty());

        server
            .handle_message(&call(
                2,
                "accept_revisions",
                json!({ "session_id": session }),
            ))
            .unwrap();

        assert!(
            revisions(&mut server, &session).is_empty(),
            "accepting should clear the tracked changes"
        );
        // The insertion "Hello " is kept and the deletion "World" is applied.
        let para0 = server
            .handle_message(&call(
                3,
                "query",
                json!({ "session_id": session, "queries": [{ "type": "paragraph_text", "paragraph": 0 }] }),
            ))
            .unwrap();
        assert_eq!(
            tool_json(&para0)["results"][0]["text"].as_str().unwrap(),
            "Hello "
        );
    }

    /// オフセットを省略すると段落全体にコメントが付くこと。
    ///
    /// 「この段落にコメント」を表現するのにバイトオフセットを数えさせない。
    #[test]
    fn add_comment_without_offsets_comments_on_the_whole_paragraph() {
        let mut server = test_server();
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        server
            .handle_message(&call(
                2,
                "add_comment",
                json!({
                    "session_id": session,
                    "start_paragraph": 0,
                    "end_paragraph": 0,
                    "text": "whole paragraph",
                }),
            ))
            .unwrap();

        let after = comments(&mut server, &session);
        assert_eq!(after.len(), 1, "a comment should be attached");
        assert_eq!(after[0].1, "whole paragraph");
    }

    /// `comment_on_text` は文字列を探してその場にコメントを付ける。
    ///
    /// search_text -> オフセット -> add_comment を呼び出し側でつながせない。
    #[test]
    fn comment_on_text_anchors_to_the_matched_phrase() {
        let mut server = test_server();
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        // 対象文書の 1 段落目の実テキストから、確実に含まれる部分文字列を取る。
        let para0 = server
            .handle_message(&call(
                2,
                "query",
                json!({ "session_id": session, "queries": [{ "type": "paragraph_text", "paragraph": 0 }] }),
            ))
            .unwrap();
        let text = tool_json(&para0)["results"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        let needle: String = text.chars().take(3).collect();
        assert!(!needle.is_empty(), "fixture paragraph must have text");

        server
            .handle_message(&call(
                3,
                "comment_on_text",
                json!({
                    "session_id": session,
                    "query": needle,
                    "text": "check this phrase",
                    "author": "AI",
                }),
            ))
            .unwrap();

        let after = comments(&mut server, &session);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].1, "check this phrase");
        assert_eq!(after[0].0, "AI");
    }

    /// 一致しない文字列は、何を探して見つからなかったかが分かる形で失敗すること。
    #[test]
    fn comment_on_text_reports_the_missing_phrase() {
        let mut server = test_server();
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();

        let response = server
            .handle_message(&call(
                2,
                "comment_on_text",
                json!({
                    "session_id": session,
                    "query": "この文字列は存在しない",
                    "text": "note",
                }),
            ))
            .unwrap();

        let rendered = serde_json::to_string(&response).unwrap();
        assert!(
            rendered.contains("text not found") && rendered.contains("この文字列は存在しない"),
            "探した文字列を名指しすること: {rendered}"
        );
    }

    /// `add_comment` attaches a new Word comment that the comments query reports.
    #[test]
    fn add_comment_creates_a_comment() {
        let mut server = test_server();
        let open = server
            .handle_message(&call(1, "open_docx", json!({ "path": HELLO_DOCX })))
            .unwrap();
        let session = tool_json(&open)["session_id"].as_str().unwrap().to_string();
        assert!(comments(&mut server, &session).is_empty());

        server
            .handle_message(&call(
                2,
                "add_comment",
                json!({
                    "session_id": session,
                    "start_paragraph": 0,
                    "start_offset": 0,
                    "end_paragraph": 0,
                    "end_offset": 1,
                    "text": "please review",
                    "author": "AI",
                }),
            ))
            .unwrap();

        let after = comments(&mut server, &session);
        assert_eq!(after.len(), 1, "a comment should have been added");
        assert_eq!(after[0].0, "AI");
        assert_eq!(after[0].1, "please review");
    }

    fn replace_text(server: &mut Server, session: &str, args: Value) -> Value {
        let mut full = args;
        full["session_id"] = json!(session);
        tool_json(
            &server
                .handle_message(&call(50, "replace_text", full))
                .unwrap(),
        )
    }

    #[test]
    fn replace_text_replaces_all_occurrences() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        // Build controlled content in the (empty) second paragraph.
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "aXaXa" }]),
        );

        let result = replace_text(
            &mut server,
            &session,
            json!({ "find": "X", "replace": "Y" }),
        );
        assert_eq!(result["replaced"], json!(2));
        assert_eq!(paragraph_text(&mut server, &session, 1), "aYaYa");
    }

    /// 表記ゆれのある語を一括で置換できること。
    ///
    /// 契約書では同じ定義語が `Supplier` と `supplier` のように揺れる。
    /// 置換が case-sensitive のままだと、片方だけ直った差分が残る。
    #[test]
    fn replace_text_can_ignore_ascii_case() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "Supplier and supplier" }]),
        );

        let sensitive = replace_text(
            &mut server,
            &session,
            json!({ "find": "supplier", "replace": "Vendor" }),
        );
        assert_eq!(sensitive["replaced"], json!(1), "only the exact spelling");

        let insensitive = replace_text(
            &mut server,
            &session,
            json!({ "find": "supplier", "replace": "Vendor", "case_insensitive": true }),
        );
        assert_eq!(insensitive["replaced"], json!(1));
        assert_eq!(
            paragraph_text(&mut server, &session, 1),
            "Vendor and Vendor",
        );
    }

    /// 正規表現とキャプチャで置換できること。
    ///
    /// 「甲」「乙」の置換や条番号の付け替えなど、契約書の一括修正は
    /// リテラル一致だけでは書けない。
    #[test]
    fn replace_text_supports_regex_with_capture_references() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "第3条および第12条" }]),
        );

        let result = replace_text(
            &mut server,
            &session,
            json!({ "find": r"第(\d+)条", "replace": "第$1項", "regex": true }),
        );
        assert_eq!(result["replaced"], json!(2));
        assert_eq!(
            paragraph_text(&mut server, &session, 1),
            "第3項および第12項"
        );
    }

    /// 置換前に影響範囲を確認できること（dry run は文書を変えない）。
    #[test]
    fn replace_text_dry_run_reports_matches_without_changing_the_document() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "甲は乙に、甲の費用で" }]),
        );

        let preview = replace_text(
            &mut server,
            &session,
            json!({ "find": "甲", "replace": "委託者", "dry_run": true }),
        );
        assert_eq!(preview["replaced"], json!(2));
        assert_eq!(preview["dry_run"], json!(true));
        let matches = preview["matches"].as_array().expect("matches");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0]["paragraph"], json!(1));
        assert_eq!(matches[0]["text"], json!("甲"));
        assert_eq!(matches[0]["replacement"], json!("委託者"));
        assert!(
            matches[0]["start_offset"].as_u64().unwrap()
                < matches[1]["start_offset"].as_u64().unwrap(),
            "matches come back in document order",
        );
        assert_eq!(
            paragraph_text(&mut server, &session, 1),
            "甲は乙に、甲の費用で",
            "a dry run must not touch the document",
        );

        // The same call without dry_run applies exactly what it previewed.
        let applied = replace_text(
            &mut server,
            &session,
            json!({ "find": "甲", "replace": "委託者" }),
        );
        assert_eq!(applied["replaced"], json!(2));
        assert_eq!(
            paragraph_text(&mut server, &session, 1),
            "委託者は乙に、委託者の費用で",
        );
    }

    /// 契約書を条単位で読めること。見出しは前行の括弧書きから拾う。
    #[test]
    fn chunks_query_reads_a_contract_article_by_article() {
        let mut server = test_server();
        let session = open_session(&mut server, CONTRACT_DOCX);

        let response = server
            .handle_message(&call(
                2,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "chunks", "max_chunks": 5, "include_text": false }],
                }),
            ))
            .unwrap();
        let result = &tool_json(&response)["results"][0];
        let chunks = result["chunks"].as_array().expect("chunks");

        assert!(
            result["total_chunks"].as_u64().unwrap_or(0) > 1,
            "a contract splits into more than one chunk: {result}",
        );
        let articles: Vec<&Value> = chunks
            .iter()
            .filter(|chunk| chunk["kind"] == json!("article"))
            .collect();
        assert!(
            !articles.is_empty(),
            "the fixture has articles: {chunks:#?}"
        );
        for article in &articles {
            assert!(article["number"].is_string(), "{article}");
            assert!(
                article["start_paragraph"].as_u64() <= article["end_paragraph"].as_u64(),
                "{article}",
            );
        }
        assert!(
            articles.iter().any(|article| article["title"].is_string()),
            "article titles should be recovered: {articles:#?}",
        );
        assert_eq!(
            chunks[0]["text"],
            json!(""),
            "include_text=false drops text"
        );
        assert!(chunks[0]["char_count"].as_u64().is_some());
    }

    /// 参照が解決でき、壊れた参照だけを取り出せること。
    #[test]
    fn references_query_resolves_citations_and_finds_the_dangling_ones() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        apply(
            &mut server,
            &session,
            json!([
                { "type": "insert_text", "paragraph": 1, "offset": 0, "text": "第1条　秘密保持" },
                { "type": "insert_paragraph_break", "paragraph": 1, "offset": "第1条　秘密保持".len() },
                { "type": "insert_text", "paragraph": 2, "offset": 0, "text": "第2条　前条および第9条に定めるとおり" },
            ]),
        );

        let response = server
            .handle_message(&call(
                3,
                "query",
                json!({
                    "session_id": session,
                    "queries": [
                        { "type": "references" },
                        { "type": "references", "unresolved_only": true },
                    ],
                }),
            ))
            .unwrap();
        let results = &tool_json(&response)["results"];

        let all = results[0]["references"].as_array().expect("references");
        let texts: Vec<&str> = all
            .iter()
            .map(|reference| reference["text"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(texts, vec!["前条", "第9条"], "{all:#?}");
        assert_eq!(all[0]["target_paragraph"], json!(1), "前条 is 第1条");
        assert_eq!(all[0]["target_title"], json!("秘密保持"));
        assert_eq!(all[0]["resolved"], json!(true));

        let dangling = results[1]["references"].as_array().expect("references");
        assert_eq!(dangling.len(), 1);
        assert_eq!(dangling[0]["text"], json!("第9条"));
        assert_eq!(dangling[0]["resolved"], json!(false));
        assert_eq!(results[1]["total_references"], json!(1));
    }

    /// レビューで使う読み取り系が、実ファイル相手に答えを返すこと。
    #[test]
    fn review_queries_report_styles_page_setup_and_properties() {
        let mut server = test_server();
        let session = open_session(&mut server, HELLO_DOCX);
        let response = server
            .handle_message(&call(
                2,
                "query",
                json!({
                    "session_id": session,
                    "queries": [
                        { "type": "styles" },
                        { "type": "page_setup" },
                        { "type": "document_properties" },
                        { "type": "header_footer_text" },
                    ],
                }),
            ))
            .unwrap();
        let results = &tool_json(&response)["results"];

        let styles = results[0]["styles"].as_array().expect("styles array");
        assert!(!styles.is_empty(), "the fixture defines styles: {styles:?}");
        assert!(
            styles
                .iter()
                .all(|style| style["style_id"].is_string() && style["kind"].is_string()),
            "every style must be addressable by id: {styles:?}",
        );

        let page = &results[1]["page_setup"];
        assert!(
            page["width"].as_f64().is_some_and(|width| width > 0.0),
            "page width should be reported: {page}",
        );

        // Reported as an object even when the package carries no metadata.
        assert!(results[2]["properties"].is_object());
        assert!(results[3]["stories"].is_array());
    }

    /// 変更履歴を 1 件だけ承認できること — `revisions` が返す id をそのまま渡す。
    #[test]
    fn accept_revisions_can_target_one_revision_by_id() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);

        let response = server
            .handle_message(&call(
                2,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "revisions", "start_revision": 0, "max_revisions": 50 }],
                }),
            ))
            .unwrap();
        let listed = tool_json(&response)["results"][0]["revisions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(listed.len() >= 2, "fixture should carry several revisions");
        let target = listed[0]["id"].as_str().expect("revision id").to_string();

        server
            .handle_message(&call(
                3,
                "accept_revisions",
                json!({ "session_id": session, "revision_id": target }),
            ))
            .unwrap();

        let remaining = revisions(&mut server, &session);
        assert_eq!(
            remaining.len(),
            listed.len() - 1,
            "exactly one revision should have been resolved",
        );
    }

    /// 未解決コメントだけを数えられること（レビューの残件把握）。
    #[test]
    fn comments_query_filters_to_the_open_review_backlog() {
        let mut server = test_server();
        let session = open_session(&mut server, COMMENT_DOCX);

        let response = server
            .handle_message(&call(
                2,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "comments", "resolved": false, "max_comments": 50 }],
                }),
            ))
            .unwrap();
        let result = &tool_json(&response)["results"][0];
        let unresolved = result["comments"].as_array().cloned().unwrap_or_default();
        assert!(!unresolved.is_empty(), "fixture has open comments");
        assert!(unresolved
            .iter()
            .all(|comment| comment["resolved"] != json!(true)));
        assert_eq!(
            result["total_comments"],
            json!(unresolved.len()),
            "the total counts the filtered set",
        );

        let none = server
            .handle_message(&call(
                3,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "comments", "author": "nobody", "max_comments": 50 }],
                }),
            ))
            .unwrap();
        assert_eq!(tool_json(&none)["results"][0]["total_comments"], json!(0));
    }

    #[test]
    fn replace_text_spans_runs_split_by_formatting() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "abcdef" }]),
        );
        // Bolding bytes 1..4 splits the paragraph into runs "a" | "bcd" | "ef".
        apply(
            &mut server,
            &session,
            json!([{ "type": "set_bold", "start_paragraph": 1, "start_offset": 1, "end_paragraph": 1, "end_offset": 4, "value": true }]),
        );

        // "bcde" straddles the bold run boundary at byte 4.
        let result = replace_text(
            &mut server,
            &session,
            json!({ "find": "bcde", "replace": "Z" }),
        );
        assert_eq!(result["replaced"], json!(1));
        assert_eq!(paragraph_text(&mut server, &session, 1), "aZf");
    }

    #[test]
    fn replace_text_persists_and_undoes_across_calls() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "foofoo" }]),
        );

        // call 1: replace
        let result = replace_text(
            &mut server,
            &session,
            json!({ "find": "foo", "replace": "bar" }),
        );
        assert_eq!(result["replaced"], json!(2));

        // call 2: a *separate* request sees the persisted state
        assert_eq!(paragraph_text(&mut server, &session, 1), "barbar");

        // call 3: undo both replacements (2 history steps) restores the text
        let undo = tool_json(
            &server
                .handle_message(&call(
                    60,
                    "undo",
                    json!({ "session_id": session, "count": 2 }),
                ))
                .unwrap(),
        );
        assert_eq!(undo["steps"], json!(2));
        assert_eq!(paragraph_text(&mut server, &session, 1), "foofoo");
    }

    #[test]
    fn replace_text_honours_max_and_paragraph_scope() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "aaaa" }]),
        );

        // max_replacements caps to the first N in document order.
        let capped = replace_text(
            &mut server,
            &session,
            json!({ "find": "a", "replace": "b", "max_replacements": 2 }),
        );
        assert_eq!(capped["replaced"], json!(2));
        assert_eq!(paragraph_text(&mut server, &session, 1), "bbaa");

        // a start_paragraph beyond the document matches nothing.
        let scoped = replace_text(
            &mut server,
            &session,
            json!({ "find": "a", "replace": "z", "start_paragraph": 5 }),
        );
        assert_eq!(scoped["replaced"], json!(0));
        assert_eq!(paragraph_text(&mut server, &session, 1), "bbaa");
    }

    #[test]
    fn replace_text_preserves_unrelated_revisions_through_roundtrip() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        let revisions_before = revisions(&mut server, &session);
        assert!(!revisions_before.is_empty());

        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "foofoo" }]),
        );
        replace_text(
            &mut server,
            &session,
            json!({ "find": "foo", "replace": "bar" }),
        );

        // The tracked changes in paragraph 0 are untouched by the replace.
        assert_eq!(revisions(&mut server, &session), revisions_before);

        // …and they survive an export → reopen, with the replacement intact.
        let export = server
            .handle_message(&call(70, "export_docx", json!({ "session_id": session })))
            .unwrap();
        let bytes_base64 = tool_json(&export)["bytes_base64"]
            .as_str()
            .unwrap()
            .to_string();
        let reopen = server
            .handle_message(&call(
                71,
                "open_docx",
                json!({ "bytes_base64": bytes_base64 }),
            ))
            .unwrap();
        let session2 = tool_json(&reopen)["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(revisions(&mut server, &session2), revisions_before);
        assert_eq!(paragraph_text(&mut server, &session2, 1), "barbar");
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

        /// `replace_text` over arbitrary content must equal a plain-string model
        /// `content.replace(find, replace)` — proving find/replace-all (multiple
        /// matches, length changes, descending-order application) never corrupts
        /// or mis-edits text. A small alphabet makes matches frequent.
        #[test]
        fn replace_text_matches_string_model(
            content in "[a-c ]{0,24}",
            find in "[a-c]{1,2}",
            replace in "[a-c]{0,2}",
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, REVIEW_DOCX);
            apply(
                &mut server,
                &session,
                json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": content }]),
            );

            replace_text(
                &mut server,
                &session,
                json!({ "find": find, "replace": replace, "start_paragraph": 1 }),
            );

            let model = content.replace(find.as_str(), replace.as_str());
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), model);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

        /// Applying arbitrary edits — single-command inserts *and* multi-command
        /// `replace_text` batches — then undoing the whole history must return to
        /// the original document, and redoing the whole history must return to
        /// the final state. Exercises full undo/redo over mixed history depths.
        #[test]
        fn undo_all_then_redo_all_round_trips(ops in prop::collection::vec(0u8..3, 1..16)) {
            let mut server = test_server();
            let session = open_session(&mut server, REVIEW_DOCX);
            // Paragraph 1 starts empty; every edit below targets it.
            for (i, op) in ops.iter().enumerate() {
                if *op == 2 {
                    replace_text(
                        &mut server,
                        &session,
                        json!({ "find": "a", "replace": "Xy", "start_paragraph": 1 }),
                    );
                } else {
                    let ch = char::from(b'a' + (i % 5) as u8);
                    apply(
                        &mut server,
                        &session,
                        json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": ch.to_string() }]),
                    );
                }
            }
            let final_text = paragraph_text(&mut server, &session, 1);

            // Undo the entire history → back to the empty original.
            server
                .handle_message(&call(80, "undo", json!({ "session_id": session, "count": 100_000 })))
                .unwrap();
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), "");

            // Redo the entire history → back to the final state.
            server
                .handle_message(&call(81, "redo", json!({ "session_id": session, "count": 100_000 })))
                .unwrap();
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), final_text);
        }

        /// An interleaved stream of edit / undo / redo must keep the document
        /// exactly in step with a textbook undo model (past / present / future),
        /// including the `steps` and `can_undo`/`can_redo` the tools report.
        #[test]
        fn interleaved_undo_redo_tracks_a_model(
            ops in prop::collection::vec((0u8..3, 0u8..5), 1..40)
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, REVIEW_DOCX);

            let mut past: Vec<String> = Vec::new();
            let mut present = String::new();
            let mut future: Vec<String> = Vec::new();

            for (kind, seed) in ops {
                match kind {
                    0 => {
                        // edit: prepend one character → exactly one history entry
                        let ch = char::from(b'a' + seed);
                        let response = tool_json(
                            &server
                                .handle_message(&call(
                                    82,
                                    "apply_commands",
                                    json!({
                                        "session_id": session,
                                        "commands": [{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": ch.to_string() }],
                                    }),
                                ))
                                .unwrap(),
                        );
                        past.push(present.clone());
                        present = format!("{ch}{present}");
                        future.clear();
                        prop_assert_eq!(response["can_undo"].as_bool(), Some(true));
                        prop_assert_eq!(response["can_redo"].as_bool(), Some(false));
                    }
                    1 => {
                        let response = tool_json(
                            &server
                                .handle_message(&call(83, "undo", json!({ "session_id": session })))
                                .unwrap(),
                        );
                        if let Some(previous) = past.pop() {
                            future.push(present.clone());
                            present = previous;
                            prop_assert_eq!(response["steps"].as_u64(), Some(1));
                        } else {
                            prop_assert_eq!(response["steps"].as_u64(), Some(0));
                        }
                        prop_assert_eq!(response["can_undo"].as_bool(), Some(!past.is_empty()));
                        prop_assert_eq!(response["can_redo"].as_bool(), Some(!future.is_empty()));
                    }
                    _ => {
                        let response = tool_json(
                            &server
                                .handle_message(&call(84, "redo", json!({ "session_id": session })))
                                .unwrap(),
                        );
                        if let Some(next) = future.pop() {
                            past.push(present.clone());
                            present = next;
                            prop_assert_eq!(response["steps"].as_u64(), Some(1));
                        } else {
                            prop_assert_eq!(response["steps"].as_u64(), Some(0));
                        }
                        prop_assert_eq!(response["can_undo"].as_bool(), Some(!past.is_empty()));
                        prop_assert_eq!(response["can_redo"].as_bool(), Some(!future.is_empty()));
                    }
                }
                prop_assert_eq!(paragraph_text(&mut server, &session, 1), present.clone());
            }
        }
    }

    // review.docx carries real `w14:paraId`s: paragraph 0 = 7D50D6B3, 1 = 12D9FE30.
    #[test]
    fn get_paragraph_by_para_id_resolves_real_fixture_ids() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);

        let p0 = tool_json(
            &server
                .handle_message(&call(
                    40,
                    "get_paragraph_by_para_id",
                    json!({ "session_id": session, "para_id": "7D50D6B3" }),
                ))
                .unwrap(),
        );
        assert_eq!(p0["paragraph"], json!(0));
        assert_eq!(p0["value"]["text"], json!("Hello World"));

        let p1 = tool_json(
            &server
                .handle_message(&call(
                    41,
                    "get_paragraph_by_para_id",
                    json!({ "session_id": session, "para_id": "12D9FE30" }),
                ))
                .unwrap(),
        );
        assert_eq!(p1["paragraph"], json!(1));

        let absent = tool_json(
            &server
                .handle_message(&call(
                    42,
                    "get_paragraph_by_para_id",
                    json!({ "session_id": session, "para_id": "NO-SUCH-ID" }),
                ))
                .unwrap(),
        );
        assert_eq!(absent["paragraph"], json!(null));
        assert_eq!(absent["value"], json!(null));
    }

    #[test]
    fn comment_by_para_id_attaches_to_the_identified_paragraph() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        // Give paragraph 1 (paraId 12D9FE30) some plain text to comment on.
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "please review this" }]),
        );
        assert!(comments(&mut server, &session).is_empty());

        let result = tool_json(
            &server
                .handle_message(&call(
                    43,
                    "comment_by_para_id",
                    json!({
                        "session_id": session,
                        "para_id": "12D9FE30",
                        "start_offset": 0,
                        "end_offset": 6,
                        "text": "looks good",
                        "author": "AI",
                    }),
                ))
                .unwrap(),
        );
        assert_eq!(result["ok"], json!(true));

        let after = comments(&mut server, &session);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].0, "AI");
        assert_eq!(after[0].1, "looks good");
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// Commenting by `paraId` is equivalent to commenting by the resolved
        /// positional index (same comment metadata) and leaves the paragraph's
        /// text unchanged — over arbitrary content and ranges, on a real docx.
        #[test]
        fn comment_by_para_id_equals_comment_by_index(
            content in "[a-z]{1,16}",
            start_seed in any::<u16>(),
            span_seed in any::<u16>(),
        ) {
            let len = content.len();
            let start = usize::from(start_seed) % len;
            let end = start + 1 + (usize::from(span_seed) % (len - start));

            // A: address paragraph 1 by its paraId.
            let mut server_a = test_server();
            let session_a = open_session(&mut server_a, REVIEW_DOCX);
            apply(&mut server_a, &session_a, json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": content }]));
            tool_json(&server_a.handle_message(&call(1, "comment_by_para_id", json!({
                "session_id": session_a, "para_id": "12D9FE30",
                "start_offset": start, "end_offset": end, "text": "note", "author": "a",
            }))).unwrap());

            // B: address paragraph 1 by its index.
            let mut server_b = test_server();
            let session_b = open_session(&mut server_b, REVIEW_DOCX);
            apply(&mut server_b, &session_b, json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": content }]));
            tool_json(&server_b.handle_message(&call(1, "add_comment", json!({
                "session_id": session_b, "start_paragraph": 1, "end_paragraph": 1,
                "start_offset": start, "end_offset": end, "text": "note", "author": "a",
            }))).unwrap());

            prop_assert_eq!(comments(&mut server_a, &session_a), comments(&mut server_b, &session_b));
            prop_assert_eq!(paragraph_text(&mut server_a, &session_a, 1), content);
        }
    }

    // ---------------------------------------------------------------------
    // Review workflow: tracked changes, comment threads, undo/redo
    //
    // A reviewing agent does not type text; it accepts a change, answers a
    // point, resolves a thread — and hands the file back to Word. Two things
    // have to hold after any sequence of those: the .docx is still one Word can
    // open and show every comment in, and undo really walks it back. The tests
    // below drive the tools the way an agent would and assert both.
    // ---------------------------------------------------------------------

    /// Read one part out of exported .docx bytes.
    fn part_xml(docx_bytes: &[u8], name: &str) -> Option<String> {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(docx_bytes)).unwrap();
        let mut file = archive.by_name(name).ok()?;
        let mut xml = String::new();
        file.read_to_string(&mut xml).unwrap();
        Some(xml)
    }

    /// The `w:id` attribute of every `<element …>` occurrence, in order.
    fn ids_of(xml: &str, element: &str) -> Vec<String> {
        let needle = format!("<{element}");
        let mut ids = Vec::new();
        for (index, _) in xml.match_indices(&needle) {
            let rest = &xml[index + needle.len()..];
            // Only a tag boundary counts, so `w:comment` does not match
            // `w:commentRangeStart`.
            if !rest.starts_with(' ') && !rest.starts_with('>') && !rest.starts_with('/') {
                continue;
            }
            let Some(end) = rest.find('>') else { continue };
            let tag = &rest[..end];
            if let Some(start) = tag.find("w:id=\"") {
                let value = &tag[start + 6..];
                if let Some(quote) = value.find('"') {
                    ids.push(value[..quote].to_string());
                }
            }
        }
        ids
    }

    /// Values of an attribute whose local name is `local`, over the whole part.
    fn attribute_values(xml: &str, local: &str) -> Vec<String> {
        let needle = format!(":{local}=\"");
        xml.match_indices(&needle)
            .filter_map(|(index, _)| {
                let rest = &xml[index + needle.len()..];
                rest.find('"').map(|quote| rest[..quote].to_string())
            })
            .collect()
    }

    /// Everything that has to be true of an exported .docx for Word to open it
    /// and show every comment and tracked change it contains.
    ///
    /// This is the "did the edit break the file" oracle for the property tests
    /// below: it is checked after every single operation, not only at the end,
    /// so the failing operation is named rather than inferred.
    fn assert_docx_is_consistent(docx_bytes: &[u8], label: &str) {
        let document = part_xml(docx_bytes, "word/document.xml")
            .unwrap_or_else(|| panic!("{label}: exported package has no word/document.xml"));
        let comments = part_xml(docx_bytes, "word/comments.xml");

        let mut range_starts = ids_of(&document, "w:commentRangeStart");
        let mut range_ends = ids_of(&document, "w:commentRangeEnd");
        range_starts.sort();
        range_ends.sort();
        assert_eq!(
            range_starts, range_ends,
            "{label}: commentRangeStart and commentRangeEnd do not pair up",
        );

        let mut references = ids_of(&document, "w:commentReference");
        references.sort();
        let mut declared = comments
            .as_deref()
            .map(|xml| ids_of(xml, "w:comment"))
            .unwrap_or_default();
        declared.sort();

        // Word draws a comment where its reference run sits. A comment with a
        // range but no reference is a comment the reader will not show, and a
        // reference to a comment that is not declared makes Word repair the
        // file on open.
        assert_eq!(
            references, declared,
            "{label}: comment references in the body do not match the comments part",
        );
        for id in &range_starts {
            assert!(
                declared.contains(id),
                "{label}: commentRange {id} has no comment in comments.xml",
            );
        }

        if let Some(extended) = part_xml(docx_bytes, "word/commentsExtended.xml") {
            let comments = comments
                .as_deref()
                .expect("a commentsExtended part without a comments part");
            let comment_para_ids: Vec<String> = attribute_values(comments, "paraId");
            for para_id in attribute_values(&extended, "paraId") {
                assert!(
                    comment_para_ids
                        .iter()
                        .any(|id| id.eq_ignore_ascii_case(&para_id)),
                    "{label}: commentsExtended references paraId {para_id}, which no comment carries",
                );
            }
            for parent in attribute_values(&extended, "paraIdParent") {
                assert!(
                    comment_para_ids
                        .iter()
                        .any(|id| id.eq_ignore_ascii_case(&parent)),
                    "{label}: a reply points at parent paraId {parent}, which no comment carries",
                );
            }
            let content_types = part_xml(docx_bytes, "[Content_Types].xml").unwrap();
            assert!(
                content_types.contains("commentsExtended"),
                "{label}: commentsExtended.xml has no content-type override",
            );
            let rels = part_xml(docx_bytes, "word/_rels/document.xml.rels").unwrap();
            assert!(
                rels.contains("commentsExtended.xml"),
                "{label}: commentsExtended.xml is not related to the document",
            );
        }

        // commentsIds addresses a comment by a paragraph's paraId, and
        // commentsExtensible by the durableId commentsIds gave it. An entry
        // outliving its comment describes one that is gone.
        if let Some(ids) = part_xml(docx_bytes, "word/commentsIds.xml") {
            let comment_para_ids: Vec<String> = comments
                .as_deref()
                .map(|xml| attribute_values(xml, "paraId"))
                .unwrap_or_default();
            for para_id in attribute_values(&ids, "paraId") {
                assert!(
                    comment_para_ids
                        .iter()
                        .any(|id| id.eq_ignore_ascii_case(&para_id)),
                    "{label}: commentsIds references paraId {para_id}, which no comment carries",
                );
            }
            if let Some(extensible) = part_xml(docx_bytes, "word/commentsExtensible.xml") {
                let durable_ids = attribute_values(&ids, "durableId");
                for durable_id in attribute_values(&extensible, "durableId") {
                    assert!(
                        durable_ids
                            .iter()
                            .any(|id| id.eq_ignore_ascii_case(&durable_id)),
                        "{label}: commentsExtensible references durableId {durable_id}, which commentsIds does not give out",
                    );
                }
            }
        }

        // The last word on validity: the package parses again.
        let mut server = test_server();
        let response = server
            .handle_message(&call(
                1,
                "open_docx",
                json!({ "bytes_base64": BASE64.encode(docx_bytes) }),
            ))
            .unwrap();
        let parsed: Value = serde_json::from_str(&response).unwrap();
        assert_ne!(
            parsed["result"]["isError"],
            json!(true),
            "{label}: the exported package no longer opens: {parsed}",
        );
    }

    /// Export a session to bytes.
    fn export_bytes(server: &mut Server, session: &str) -> Vec<u8> {
        let export = tool_json(
            &server
                .handle_message(&call(95, "export_docx", json!({ "session_id": session })))
                .unwrap(),
        );
        BASE64
            .decode(export["bytes_base64"].as_str().unwrap())
            .unwrap()
    }

    /// Comment threads flattened to `(text, resolved, reply_to)`, so a thread
    /// shape can be compared across undo and across export → reopen.
    fn comment_threads(server: &mut Server, session: &str) -> Vec<(String, bool, String)> {
        let response = server
            .handle_message(&call(
                94,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "comments", "start_comment": 0, "max_comments": 50 }],
                }),
            ))
            .unwrap();
        let mut threads: Vec<(String, bool, String)> = tool_json(&response)["results"][0]
            ["comments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|comment| {
                (
                    comment["text"].as_str().unwrap_or_default().to_string(),
                    comment["resolved"].as_bool().unwrap_or(false),
                    comment["reply_to_comment_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                )
            })
            .collect();
        threads.sort();
        threads
    }

    /// Paragraph text, comment threads and tracked changes — everything a
    /// reviewer would notice, as one comparable value.
    type ReviewState = (
        Vec<String>,
        Vec<(String, bool, String)>,
        Vec<(String, String)>,
    );

    fn review_state(server: &mut Server, session: &str) -> ReviewState {
        let paragraphs = (0..2)
            .map(|index| paragraph_text(server, session, index))
            .collect();
        (
            paragraphs,
            comment_threads(server, session),
            revisions(server, session),
        )
    }

    /// Call a tool, returning `Err` with the message when it reports one — the
    /// property tests need to tell "this operation does not apply right now"
    /// from "this operation is broken".
    fn try_tool(server: &mut Server, name: &str, arguments: Value) -> Result<Value, String> {
        let response = server.handle_message(&call(93, name, arguments)).unwrap();
        let parsed: Value = serde_json::from_str(&response).unwrap();
        if parsed["result"]["isError"] == json!(true) {
            return Err(parsed["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string());
        }
        Ok(parsed["result"]["structuredContent"].clone())
    }

    /// A scratch directory for the file-access tests, removed on drop.
    struct ScratchDir(std::path::PathBuf);

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "jade-mcp-server-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `open_docx` cannot be pointed at a file outside the allowed directories,
    /// which is what a prompt-injected "open ~/other-matter/contract.docx"
    /// would try.
    #[test]
    fn open_docx_refuses_paths_outside_the_allowed_directories() {
        let workspace = ScratchDir::new("open-workspace");
        let mut server = Server::new().with_file_access(FileAccess::new([&workspace.0]));

        let error = try_tool(&mut server, "open_docx", json!({ "path": HELLO_DOCX })).unwrap_err();
        assert!(error.contains("outside the allowed directories"), "{error}");

        std::fs::copy(HELLO_DOCX, workspace.0.join("hello.docx")).unwrap();
        let opened = try_tool(&mut server, "open_docx", json!({ "path": "hello.docx" })).unwrap();
        assert!(opened["session_id"].is_string());
    }

    /// `export_docx` creates a new file inside the allowed directories and
    /// nothing else: no overwrite, no escape.
    #[test]
    fn export_docx_only_creates_new_files_inside_the_allowed_directories() {
        let workspace = ScratchDir::new("export-workspace");
        let outside = ScratchDir::new("export-outside");
        std::fs::copy(HELLO_DOCX, workspace.0.join("hello.docx")).unwrap();
        let mut server = Server::new().with_file_access(FileAccess::new([&workspace.0]));
        let session = open_session(&mut server, "hello.docx");

        let exported = try_tool(
            &mut server,
            "export_docx",
            json!({ "session_id": session, "path": "edited.docx" }),
        )
        .unwrap();
        let written = std::path::PathBuf::from(exported["path"].as_str().unwrap());
        assert_eq!(written.file_name().unwrap(), "edited.docx");
        assert_eq!(
            std::fs::read(&written).unwrap().len() as u64,
            exported["bytes"].as_u64().unwrap()
        );

        for (path, expected) in [
            ("edited.docx", "never overwrites"),
            ("hello.docx", "never overwrites"),
            ("notes.txt", "only .docx"),
        ] {
            let error = try_tool(
                &mut server,
                "export_docx",
                json!({ "session_id": session, "path": path }),
            )
            .unwrap_err();
            assert!(error.contains(expected), "{path}: {error}");
        }
        let escape = outside.0.join("leak.docx");
        let error = try_tool(
            &mut server,
            "export_docx",
            json!({ "session_id": session, "path": escape.to_string_lossy() }),
        )
        .unwrap_err();
        assert!(error.contains("outside the allowed directories"), "{error}");
        assert!(!escape.exists());
    }

    /// Ids of the comments currently in a session, in document order.
    fn comment_ids(server: &mut Server, session: &str) -> Vec<String> {
        let response = server
            .handle_message(&call(
                92,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "comments", "start_comment": 0, "max_comments": 50 }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["comments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|comment| comment["id"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// `(kind, text)` of the tracked changes in one paragraph.
    fn revisions_in(server: &mut Server, session: &str, paragraph: u64) -> Vec<(String, String)> {
        let response = server
            .handle_message(&call(
                78,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "revisions", "start_revision": 0, "max_revisions": 50 }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["revisions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|revision| revision["paragraph"].as_u64() == Some(paragraph))
            .map(|revision| {
                (
                    revision["kind"].as_str().unwrap_or_default().to_string(),
                    revision["text"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    /// Ids of the tracked changes in one paragraph, in document order.
    fn revision_ids_in(server: &mut Server, session: &str, paragraph: u64) -> Vec<String> {
        let response = server
            .handle_message(&call(
                90,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "revisions", "start_revision": 0, "max_revisions": 50 }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["revisions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|revision| revision["paragraph"].as_u64() == Some(paragraph))
            .filter_map(|revision| revision["id"].as_str().map(str::to_string))
            .collect()
    }

    /// Ids of the tracked changes currently in a session.
    fn revision_ids(server: &mut Server, session: &str) -> Vec<String> {
        let response = server
            .handle_message(&call(
                91,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "revisions", "start_revision": 0, "max_revisions": 50 }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["revisions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|revision| revision["id"].as_str().map(str::to_string))
            .collect()
    }

    /// One step of a review session. `seed` picks between the operands the
    /// document currently offers, so a generated sequence stays meaningful as
    /// the document changes under it.
    ///
    /// Returns whether the step applied; a step with nothing to act on (resolve
    /// with no comments) is skipped rather than counted as history.
    fn run_review_op(server: &mut Server, session: &str, op: u8, seed: usize) -> bool {
        let pick = |items: Vec<String>| -> Option<String> {
            if items.is_empty() {
                None
            } else {
                Some(items[seed % items.len()].clone())
            }
        };
        match op {
            0 => {
                // Plain edit, so review operations are exercised next to
                // ordinary text history rather than in isolation.
                try_tool(
                    server,
                    "apply_commands",
                    json!({
                        "session_id": session,
                        "commands": [{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "x" }],
                    }),
                )
                .is_ok()
            }
            1 => try_tool(
                server,
                "add_comment",
                json!({
                    "session_id": session,
                    "start_paragraph": 0,
                    "end_paragraph": 0,
                    "text": format!("note {seed}"),
                    "author": "AI",
                }),
            )
            .is_ok(),
            2 => match pick(comment_ids(server, session)) {
                Some(id) => try_tool(
                    server,
                    "reply_to_comment",
                    json!({
                        "session_id": session,
                        "comment_id": id,
                        "text": format!("reply {seed}"),
                        "author": "法務",
                    }),
                )
                .is_ok(),
                None => false,
            },
            3 => match pick(comment_ids(server, session)) {
                Some(id) => try_tool(
                    server,
                    "resolve_comment",
                    json!({ "session_id": session, "comment_id": id, "resolved": seed.is_multiple_of(2) }),
                )
                .is_ok(),
                None => false,
            },
            4 => match pick(comment_ids(server, session)) {
                Some(id) => try_tool(
                    server,
                    "delete_comment",
                    json!({ "session_id": session, "comment_id": id }),
                )
                .is_ok(),
                None => false,
            },
            5 => match pick(revision_ids(server, session)) {
                Some(id) => try_tool(
                    server,
                    "accept_revisions",
                    json!({ "session_id": session, "revision_id": id }),
                )
                .is_ok(),
                None => false,
            },
            _ => match pick(revision_ids(server, session)) {
                Some(id) => try_tool(
                    server,
                    "reject_revisions",
                    json!({ "session_id": session, "revision_id": id }),
                )
                .is_ok(),
                None => false,
            },
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// Whatever a reviewing agent does — accept one change, answer a point,
        /// resolve it, delete a note — the file it hands back has to stay a
        /// .docx Word can open, with every comment still anchored and every
        /// thread still pointing at a comment that exists.
        ///
        /// The package is checked after *every* operation, so a sequence that
        /// only breaks on the third step fails naming that step.
        #[test]
        fn review_operations_never_break_the_package(
            ops in prop::collection::vec((0u8..7, 0usize..4), 1..8),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, COMMENT_DOCX);

            for (step, (op, seed)) in ops.iter().enumerate() {
                if !run_review_op(&mut server, &session, *op, *seed) {
                    continue;
                }
                let bytes = export_bytes(&mut server, &session);
                assert_docx_is_consistent(&bytes, &format!("step {step} (op {op})"));
            }
        }

        /// Undo has to cover review operations, not only typing: after any
        /// sequence, undoing every step returns the document — text, comments,
        /// threads, resolved flags and tracked changes — to exactly what was
        /// opened, and redoing every step returns it to where the sequence
        /// ended.
        ///
        /// Before revision and comment commands joined the undo history this
        /// failed on the first accept: the step was not undoable at all, and
        /// `undo` silently reverted an unrelated earlier edit instead.
        #[test]
        fn undoing_a_review_session_returns_the_original_document(
            ops in prop::collection::vec((0u8..7, 0usize..4), 1..8),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, COMMENT_DOCX);
            let original = review_state(&mut server, &session);

            let mut applied = 0usize;
            for (op, seed) in &ops {
                if run_review_op(&mut server, &session, *op, *seed) {
                    applied += 1;
                }
            }
            let final_state = review_state(&mut server, &session);

            let undone = tool_json(
                &server
                    .handle_message(&call(90, "undo", json!({ "session_id": session, "count": 1000 })))
                    .unwrap(),
            );
            prop_assert_eq!(undone["steps"].as_u64(), Some(applied as u64));
            prop_assert_eq!(review_state(&mut server, &session), original);

            let redone = tool_json(
                &server
                    .handle_message(&call(89, "redo", json!({ "session_id": session, "count": 1000 })))
                    .unwrap(),
            );
            prop_assert_eq!(redone["steps"].as_u64(), Some(applied as u64));
            prop_assert_eq!(review_state(&mut server, &session), final_state);
        }

        /// A review session survives the hand-off: export, reopen, and the
        /// comments, their threading and their resolved state read back the
        /// same. This is what makes "resolved" mean anything — the flag has to
        /// be written where Word keeps it, not only held in memory.
        #[test]
        fn review_state_survives_export_and_reopen(
            ops in prop::collection::vec((0u8..7, 0usize..4), 1..8),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, COMMENT_DOCX);
            for (op, seed) in &ops {
                run_review_op(&mut server, &session, *op, *seed);
            }

            let before = comment_threads(&mut server, &session);
            let revisions_before = revisions(&mut server, &session);
            let bytes = export_bytes(&mut server, &session);

            let reopened = tool_json(
                &server
                    .handle_message(&call(
                        88,
                        "open_docx",
                        json!({ "bytes_base64": BASE64.encode(&bytes) }),
                    ))
                    .unwrap(),
            );
            let session2 = reopened["session_id"].as_str().unwrap().to_string();
            prop_assert_eq!(comment_threads(&mut server, &session2), before);
            prop_assert_eq!(revisions(&mut server, &session2), revisions_before);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

        /// Deciding tracked changes one at a time must not depend on the order
        /// they are decided in: the same per-revision verdicts, applied in any
        /// sequence, have to leave the same text.
        ///
        /// This is the shape of a real review — 80 changes come back and the
        /// reviewer works through them in whatever order the document is read
        /// in — and it is exactly where an implementation that shifts offsets
        /// while deciding goes wrong.
        #[test]
        fn deciding_revisions_one_by_one_is_order_independent(
            verdicts in prop::collection::vec(any::<bool>(), 4),
            order in prop::collection::vec(0usize..4, 4),
        ) {
            // Build a document with four tracked insertions, one per position.
            let build = |server: &mut Server| -> String {
                let session = open_session(server, REVIEW_DOCX);
                apply(
                    server,
                    &session,
                    json!([
                        { "type": "insert_text", "paragraph": 1, "offset": 0, "text": "ABCD" },
                        { "type": "start_change_tracking", "author": "AI" },
                    ]),
                );
                for offset in [4usize, 3, 2, 1] {
                    apply(
                        server,
                        &session,
                        json!([{ "type": "insert_text", "paragraph": 1, "offset": offset, "text": "-" }]),
                    );
                }
                apply(server, &session, json!([{ "type": "stop_change_tracking" }]));
                session
            };

            // Reference: decide in document order. review.docx carries two
            // tracked changes of its own in paragraph 0; only the four built
            // above are decided here.
            let mut reference_server = test_server();
            let reference = build(&mut reference_server);
            let reference_ids = revision_ids_in(&mut reference_server, &reference, 1);
            prop_assert_eq!(reference_ids.len(), 4);
            for (index, verdict) in verdicts.iter().enumerate() {
                let tool = if *verdict { "accept_revisions" } else { "reject_revisions" };
                try_tool(
                    &mut reference_server,
                    tool,
                    json!({ "session_id": reference, "revision_id": reference_ids[index] }),
                )
                .unwrap();
            }
            let expected = paragraph_text(&mut reference_server, &reference, 1);

            // Candidate: decide the same verdicts in a shuffled order.
            let mut server = test_server();
            let session = build(&mut server);
            let ids = revision_ids_in(&mut server, &session, 1);
            prop_assert_eq!(&ids, &reference_ids);
            let mut remaining: Vec<usize> = (0..4).collect();
            for pick in &order {
                if remaining.is_empty() {
                    break;
                }
                let index = remaining.remove(pick % remaining.len());
                let tool = if verdicts[index] { "accept_revisions" } else { "reject_revisions" };
                try_tool(
                    &mut server,
                    tool,
                    json!({ "session_id": session, "revision_id": ids[index] }),
                )
                .unwrap();
            }
            for index in remaining {
                let tool = if verdicts[index] { "accept_revisions" } else { "reject_revisions" };
                try_tool(
                    &mut server,
                    tool,
                    json!({ "session_id": session, "revision_id": ids[index] }),
                )
                .unwrap();
            }

            prop_assert_eq!(paragraph_text(&mut server, &session, 1), expected);
            prop_assert!(revision_ids_in(&mut server, &session, 1).is_empty());
            assert_docx_is_consistent(&export_bytes(&mut server, &session), "after deciding every revision");
        }
    }

    #[test]
    fn a_reply_joins_the_thread_word_reads() {
        let mut server = test_server();
        let session = open_session(&mut server, COMMENT_DOCX);

        // Reply to the existing reply: Word threads are one root with a flat
        // list of answers, so this has to join the root's thread.
        try_tool(
            &mut server,
            "reply_to_comment",
            json!({ "session_id": session, "comment_id": "1", "text": "承知しました", "author": "法務" }),
        )
        .expect("reply");
        try_tool(
            &mut server,
            "resolve_comment",
            json!({ "session_id": session, "comment_id": "0", "resolved": true }),
        )
        .expect("resolve");

        let bytes = export_bytes(&mut server, &session);
        assert_docx_is_consistent(&bytes, "reply and resolve");
        let extended = part_xml(&bytes, "word/commentsExtended.xml").expect("commentsExtended");
        // The root comment's paraId, as Word wrote it in the fixture.
        assert!(
            extended.contains(r#"w15:paraId="4C97C83A" w15:done="1""#),
            "the resolved flag must be written where Word reads it: {extended}",
        );
        assert_eq!(
            extended.matches(r#"w15:paraIdParent="4C97C83A""#).count(),
            2,
            "both the original reply and the new one belong to the same thread: {extended}",
        );

        let reopened = tool_json(
            &server
                .handle_message(&call(
                    2,
                    "open_docx",
                    json!({ "bytes_base64": BASE64.encode(&bytes) }),
                ))
                .unwrap(),
        );
        let session2 = reopened["session_id"].as_str().unwrap().to_string();
        let threads = tool_json(
            &server
                .handle_message(&call(
                    3,
                    "query",
                    json!({
                        "session_id": session2,
                        "queries": [{ "type": "comment_threads", "start_thread": 0, "max_threads": 10 }],
                    }),
                ))
                .unwrap(),
        );
        let root = &threads["results"][0]["threads"][0];
        assert_eq!(root["comment"]["resolved"], json!(true));
        assert_eq!(root["replies"].as_array().unwrap().len(), 2);
    }

    /// Two comments in one paragraph: the second must not silently delete the
    /// first one's anchor. Word draws a comment at its `commentReference`, so a
    /// comment that loses it disappears from the review even though the text is
    /// still bracketed by its range markers.
    #[test]
    fn a_second_comment_keeps_the_first_ones_anchor() {
        let mut server = test_server();
        let session = open_session(&mut server, REVIEW_DOCX);
        apply(
            &mut server,
            &session,
            json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": "alpha beta" }]),
        );
        for (query, text) in [("alpha", "first"), ("beta", "second")] {
            try_tool(
                &mut server,
                "comment_on_text",
                json!({ "session_id": session, "query": query, "text": text, "author": "AI" }),
            )
            .expect("comment");
        }

        let bytes = export_bytes(&mut server, &session);
        assert_docx_is_consistent(&bytes, "two comments in one paragraph");
        let document = part_xml(&bytes, "word/document.xml").unwrap();
        assert_eq!(
            ids_of(&document, "w:commentReference").len(),
            2,
            "both comments must keep a reference run: {document}",
        );
    }

    // ---------------------------------------------------------------------
    // Search and replace: offsets, literals, and what a replacement is allowed
    // to overwrite.
    //
    // Every offset the API speaks is a UTF-8 byte offset, and a contract is
    // full of text where a character is three of them. The properties below
    // are written so that a byte/char confusion, a metacharacter leaking into
    // a literal search, or a replacement eating an anchor cannot pass.
    // ---------------------------------------------------------------------

    /// Run `search_text` over a session and return the raw match objects.
    fn search(server: &mut Server, session: &str, query: Value) -> Vec<Value> {
        let mut request = query;
        request["type"] = json!("search_text");
        let response = server
            .handle_message(&call(
                87,
                "query",
                json!({ "session_id": session, "queries": [request] }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["matches"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// A session on `review.docx` whose paragraph 1 holds `content`.
    ///
    /// review.docx carries two tracked changes of its own in paragraph 0, which
    /// is what makes it the right fixture here: it proves a replacement stays
    /// inside its scope instead of wandering into someone else's redline.
    fn session_with_paragraph(server: &mut Server, content: &str) -> String {
        let session = open_session(server, REVIEW_DOCX);
        if !content.is_empty() {
            apply(
                server,
                &session,
                json!([{ "type": "insert_text", "paragraph": 1, "offset": 0, "text": content }]),
            );
        }
        session
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

        /// Every match has to describe itself consistently: the byte offsets
        /// must slice exactly the reported text out of the paragraph, and the
        /// character offsets must be that same span counted in characters.
        ///
        /// Japanese is in the alphabet on purpose — a contract is mostly
        /// multi-byte text, and a byte/char mix-up survives every ASCII test.
        #[test]
        fn search_offsets_slice_the_text_they_claim(
            content in "[a-zA-Z甲乙丙、。 ]{0,40}",
            query in "[a-zA-Z甲乙丙、。]{1,3}",
            case_insensitive in any::<bool>(),
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            let matches = search(&mut server, &session, json!({
                "query": query, "start_paragraph": 1, "max_results": 1000,
                "case_insensitive": case_insensitive,
            }));

            let text = paragraph_text(&mut server, &session, 1);
            for found in &matches {
                let start = found["start_offset"].as_u64().unwrap() as usize;
                let end = found["end_offset"].as_u64().unwrap() as usize;
                let matched = found["matched_text"].as_str().unwrap();
                prop_assert_eq!(
                    text.get(start..end),
                    Some(matched),
                    "byte offsets {}..{} do not slice {:?} out of {:?}", start, end, matched, text,
                );
                prop_assert_eq!(matched.len(), query.len());
                if case_insensitive {
                    prop_assert!(matched.eq_ignore_ascii_case(&query));
                } else {
                    prop_assert_eq!(matched, query.as_str());
                }
                let start_char = found["start_char_offset"].as_u64().unwrap() as usize;
                let end_char = found["end_char_offset"].as_u64().unwrap() as usize;
                prop_assert_eq!(text[..start].chars().count(), start_char);
                prop_assert_eq!(text[..end].chars().count(), end_char);
            }
        }

        /// The set of matches is exactly what a plain string scan finds —
        /// non-overlapping, left to right. `aa` in `aaaa` is two matches, not
        /// three.
        #[test]
        fn search_finds_every_non_overlapping_occurrence(
            content in "[ab甲]{0,32}",
            query in "[ab甲]{1,3}",
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            let matches = search(&mut server, &session, json!({
                "query": query, "start_paragraph": 1, "max_results": 1000,
            }));

            let expected: Vec<usize> = content.match_indices(query.as_str()).map(|(at, _)| at).collect();
            let actual: Vec<usize> = matches
                .iter()
                .map(|found| found["start_offset"].as_u64().unwrap() as usize)
                .collect();
            prop_assert_eq!(actual, expected);
        }

        /// `truncated` is the caller's signal to keep paging, so it must mean
        /// "there are more", not "you asked for exactly this many".
        #[test]
        fn search_truncation_means_there_are_more_matches(
            content in "[ab]{0,24}",
            limit in 1usize..8,
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            let response = server
                .handle_message(&call(86, "query", json!({
                    "session_id": session,
                    "queries": [{
                        "type": "search_text", "query": "a", "start_paragraph": 1,
                        "max_results": limit,
                    }],
                })))
                .unwrap();
            let result = tool_json(&response)["results"][0].clone();
            let returned = result["matches"].as_array().unwrap().len();
            let total = content.matches('a').count();

            prop_assert_eq!(returned, total.min(limit));
            prop_assert_eq!(result["truncated"].as_bool(), Some(total > limit));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

        /// A literal replacement is literal: regex metacharacters in `find` are
        /// characters, and `$1` in `replace` is two characters. An agent
        /// replacing "(a)" or writing "$1,000" must not have the engine read
        /// its own syntax into the request.
        #[test]
        fn literal_replacement_is_literal_on_both_sides(
            content in r"[a-c.*()\[\]$1 ]{0,24}",
            find in r"[a-c.*()\[\]$1]{1,3}",
            replace in r"[a-c$1 ]{0,4}",
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            replace_text(&mut server, &session, json!({
                "find": find, "replace": replace, "start_paragraph": 1,
            }));
            prop_assert_eq!(
                paragraph_text(&mut server, &session, 1),
                content.replace(find.as_str(), replace.as_str()),
            );
        }

        /// The same, over Japanese text: replacement must land on character
        /// boundaries however many bytes each character takes.
        #[test]
        fn replacement_matches_the_string_model_in_japanese(
            content in "[甲乙丙、。]{0,20}",
            find in "[甲乙丙]{1,2}",
            replace in "[甲乙丁 ]{0,3}",
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            replace_text(&mut server, &session, json!({
                "find": find, "replace": replace, "start_paragraph": 1,
            }));
            prop_assert_eq!(
                paragraph_text(&mut server, &session, 1),
                content.replace(find.as_str(), replace.as_str()),
            );
        }

        /// A regex replacement has to agree with the regex crate's own
        /// `replace_all`, capture references included. The engine is the same
        /// one; what is being tested is that the offsets it reports survive the
        /// trip through the document model.
        #[test]
        fn regex_replacement_matches_the_regex_crate_model(
            content in "[abc ]{0,24}",
            pattern in "(a+)|(b[abc])|(c )",
            template in prop::sample::select(vec!["<$0>", "$1!", "-", ""]),
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            replace_text(&mut server, &session, json!({
                "find": pattern, "replace": template, "regex": true, "start_paragraph": 1,
            }));

            let expected = regex::Regex::new(&pattern)
                .unwrap()
                .replace_all(&content, template)
                .to_string();
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), expected);
        }

        /// Formatting splits a paragraph into several runs, and a match that
        /// straddles a run boundary is the case where offset arithmetic goes
        /// wrong. Bolding an arbitrary sub-range first puts the boundary
        /// somewhere new on every case.
        #[test]
        fn replacement_works_across_runs_split_by_formatting(
            content in "[abc]{4,16}",
            bold_start in 0usize..8,
            bold_len in 1usize..6,
            find in "[abc]{2,3}",
        ) {
            let start = bold_start.min(content.len().saturating_sub(1));
            let end = (start + bold_len).min(content.len());
            prop_assume!(start < end);

            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            apply(&mut server, &session, json!([{
                "type": "set_bold", "start_paragraph": 1, "start_offset": start,
                "end_paragraph": 1, "end_offset": end, "value": true,
            }]));

            replace_text(&mut server, &session, json!({
                "find": find, "replace": "Z", "start_paragraph": 1,
            }));
            prop_assert_eq!(
                paragraph_text(&mut server, &session, 1),
                content.replace(find.as_str(), "Z"),
            );
        }

        /// A dry run has to be a faithful preview: the same matches, and not a
        /// byte of the document changed. Its whole purpose is letting a caller
        /// check the blast radius of a document-wide replacement first.
        #[test]
        fn dry_run_previews_exactly_what_the_real_run_does(
            content in "[a-c ]{0,24}",
            find in "[a-c]{1,2}",
            replace in "[a-c]{0,2}",
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            let before = paragraph_text(&mut server, &session, 1);

            let preview = replace_text(&mut server, &session, json!({
                "find": find, "replace": replace, "start_paragraph": 1, "dry_run": true,
            }));
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), before);

            let real = replace_text(&mut server, &session, json!({
                "find": find, "replace": replace, "start_paragraph": 1,
            }));
            prop_assert_eq!(preview["replaced"].clone(), real["replaced"].clone());
            prop_assert_eq!(preview["matches"].clone(), real["matches"].clone());
        }

        /// `max_replacements` takes the first N in document order and leaves the
        /// rest alone, so a capped run is a prefix of the uncapped one.
        #[test]
        fn max_replacements_takes_the_first_matches_in_order(
            content in "[ab]{0,16}",
            max in 1usize..5,
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            replace_text(&mut server, &session, json!({
                "find": "a", "replace": "Z", "start_paragraph": 1, "max_replacements": max,
            }));

            let mut expected = String::new();
            let mut used = 0usize;
            for ch in content.chars() {
                if ch == 'a' && used < max {
                    expected.push('Z');
                    used += 1;
                } else {
                    expected.push(ch);
                }
            }
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), expected);
        }

        /// Undo covers a replacement match for match: N replacements are N
        /// history steps, and undoing them all restores the original text.
        #[test]
        fn undoing_a_replacement_restores_the_original_text(
            content in "[ab ]{0,20}",
            replace in "[xy]{0,3}",
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            let before = paragraph_text(&mut server, &session, 1);

            let result = replace_text(&mut server, &session, json!({
                "find": "a", "replace": replace, "start_paragraph": 1,
            }));
            let replaced = result["replaced"].as_u64().unwrap();

            // Undo exactly as many steps as matches replaced — one per match is
            // the contract — and the paragraph must be back to what it was.
            let undone = tool_json(
                &server
                    .handle_message(&call(85, "undo", json!({ "session_id": session, "count": replaced })))
                    .unwrap(),
            );
            prop_assert_eq!(undone["steps"].as_u64(), Some(replaced));
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), before);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// A replacement anywhere in a commented paragraph must leave every
        /// comment anchored and the package intact.
        ///
        /// Word draws a comment at its `commentReference` run, which holds no
        /// text — so any code that prunes "empty" runs while editing deletes
        /// the anchor and produces a comment Word will not show. The comment
        /// here is on `beta`; the replacement is aimed at each of the three
        /// words in turn, so the case where the edit is nowhere near the
        /// comment is covered too.
        #[test]
        fn replacing_text_never_unanchors_a_comment(
            target in prop::sample::select(vec!["alpha", "beta", "gamma"]),
            replacement in "[a-z]{0,8}",
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, "alpha beta gamma");
            try_tool(&mut server, "comment_on_text", json!({
                "session_id": session, "query": "beta", "text": "確認", "author": "法務",
            })).expect("comment");

            replace_text(&mut server, &session, json!({
                "find": target, "replace": replacement, "start_paragraph": 1,
            }));

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, &format!("after replacing {target:?}"));
            prop_assert_eq!(comments(&mut server, &session).len(), 1);
        }

        /// With change tracking on, a replacement is a redline: the old text
        /// stays as a tracked deletion and the new text arrives as a tracked
        /// insertion. Accepting everything gives the replaced document,
        /// rejecting everything gives back the original — which is the whole
        /// contract of handing an agent's edit to a human reviewer.
        #[test]
        fn a_tracked_replacement_can_be_accepted_or_rejected(
            replacement in "[a-z]{1,6}",
            accept in any::<bool>(),
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, "alpha beta gamma");
            try_tool(&mut server, "start_change_tracking", json!({
                "session_id": session, "author": "AI",
            })).expect("start tracking");

            replace_text(&mut server, &session, json!({
                "find": "beta", "replace": replacement, "start_paragraph": 1,
            }));

            // The removed text is still in the file, marked deleted.
            let kinds = revisions(&mut server, &session);
            prop_assert!(
                kinds.contains(&("deleted".to_string(), "beta".to_string())),
                "the replaced text must survive as a tracked deletion: {kinds:?}",
            );
            prop_assert!(
                kinds.contains(&("inserted".to_string(), replacement.clone())),
                "the new text must arrive as a tracked insertion: {kinds:?}",
            );

            let tool = if accept { "accept_revisions" } else { "reject_revisions" };
            try_tool(&mut server, tool, json!({
                "session_id": session, "start_paragraph": 1, "end_paragraph": 1,
            })).expect("decide");

            let expected = if accept {
                format!("alpha {replacement} gamma")
            } else {
                "alpha beta gamma".to_string()
            };
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), expected);
            assert_docx_is_consistent(&export_bytes(&mut server, &session), "after deciding the redline");
        }

        /// A match that overlaps another author's tracked change is refused,
        /// and refusing means refusing: not one paragraph, comment or revision
        /// moves. review.docx paragraph 0 is "Hello World" with "World" marked
        /// deleted by its author, so every one of these replacements lands on
        /// someone else's redline.
        #[test]
        fn replacing_another_authors_tracked_change_is_refused(
            find in prop::sample::select(vec!["Hello", "World", "o W", "lo Wo"]),
            replacement in "[a-z]{0,4}",
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, REVIEW_DOCX);
            let before = review_state(&mut server, &session);

            let error = try_tool(&mut server, "replace_text", json!({
                "session_id": session, "find": find, "replace": replacement,
                "end_paragraph": 0,
            }))
            .expect_err("a replacement over another author's change must be refused");
            prop_assert!(
                error.contains("植木 智之"),
                "the refusal has to name whose change is in the way: {error}",
            );
            prop_assert_eq!(review_state(&mut server, &session), before);
            // Refusing must not leave half a replacement on the history either.
            let undone = tool_json(
                &server
                    .handle_message(&call(81, "undo", json!({ "session_id": session })))
                    .unwrap(),
            );
            prop_assert_eq!(undone["steps"].as_u64(), Some(0));
        }
    }

    // ---------------------------------------------------------------------
    // Resolving comments
    //
    // Resolution and threading live in commentsExtended.xml, keyed by the
    // paraId of a comment's last paragraph. Anything that rewrites a comment
    // has to keep that key, or the thread quietly detaches.
    // ---------------------------------------------------------------------

    /// `(id, resolved, reply_to)` for every comment, sorted by id.
    fn resolution_state(server: &mut Server, session: &str) -> Vec<(String, bool, String)> {
        let response = server
            .handle_message(&call(
                84,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "comments", "start_comment": 0, "max_comments": 50 }],
                }),
            ))
            .unwrap();
        let mut state: Vec<(String, bool, String)> = tool_json(&response)["results"][0]["comments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|comment| {
                (
                    comment["id"].as_str().unwrap_or_default().to_string(),
                    comment["resolved"].as_bool().unwrap_or(false),
                    comment["reply_to_comment_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                )
            })
            .collect();
        state.sort();
        state
    }

    /// Reopen exported bytes and return the new session id.
    fn reopen(server: &mut Server, bytes: &[u8]) -> String {
        let reopened = tool_json(
            &server
                .handle_message(&call(
                    83,
                    "open_docx",
                    json!({ "bytes_base64": BASE64.encode(bytes) }),
                ))
                .unwrap(),
        );
        reopened["session_id"].as_str().unwrap().to_string()
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// Whatever is resolved, reopened, edited and resolved again, the
        /// resolved flags and thread parents that come back out of the file are
        /// the ones the session had. comment.docx is the interesting fixture:
        /// its reply is a two-paragraph comment, and Word keys commentsExtended
        /// on the *last* paragraph of a comment.
        #[test]
        fn resolution_and_threading_survive_the_round_trip(
            ops in prop::collection::vec((0u8..4, 0usize..3), 1..8),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, COMMENT_DOCX);

            for (op, seed) in &ops {
                let ids: Vec<String> = resolution_state(&mut server, &session)
                    .into_iter()
                    .map(|(id, _, _)| id)
                    .collect();
                if ids.is_empty() {
                    break;
                }
                let id = ids[seed % ids.len()].clone();
                let _ = match op {
                    0 => try_tool(&mut server, "resolve_comment", json!({
                        "session_id": session, "comment_id": id, "resolved": true,
                    })),
                    1 => try_tool(&mut server, "resolve_comment", json!({
                        "session_id": session, "comment_id": id, "resolved": false,
                    })),
                    2 => try_tool(&mut server, "update_comment", json!({
                        "session_id": session, "comment_id": id, "text": format!("書き換え {seed}"),
                    })),
                    _ => try_tool(&mut server, "reply_to_comment", json!({
                        "session_id": session, "comment_id": id, "text": format!("返信 {seed}"),
                        "author": "法務",
                    })),
                };
            }

            let before = resolution_state(&mut server, &session);
            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, "after a resolve/reply session");

            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(resolution_state(&mut server, &reopened), before);
        }

        /// Resolving is a state, not an event: setting it twice is the same as
        /// setting it once, and setting it back leaves the review exactly where
        /// it started.
        #[test]
        fn resolving_is_idempotent_and_reversible(
            comment in prop::sample::select(vec!["0", "1"]),
            extra_round in any::<bool>(),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, COMMENT_DOCX);
            let original = resolution_state(&mut server, &session);

            for _ in 0..(if extra_round { 2 } else { 1 }) {
                try_tool(&mut server, "resolve_comment", json!({
                    "session_id": session, "comment_id": comment, "resolved": true,
                })).expect("resolve");
            }
            let resolved_once = resolution_state(&mut server, &session);
            prop_assert!(
                resolved_once.iter().any(|(id, resolved, _)| id == comment && *resolved),
                "the comment must read back resolved: {resolved_once:?}",
            );

            try_tool(&mut server, "resolve_comment", json!({
                "session_id": session, "comment_id": comment, "resolved": false,
            })).expect("reopen the comment");
            prop_assert_eq!(resolution_state(&mut server, &session), original);
        }
    }

    /// The arguments a caller gets wrong, and what it is told about them.
    ///
    /// An agent only ever sees the message, so each of these is the difference
    /// between it retrying usefully and it retrying the same call. None of them
    /// may half-apply: a rejected call leaves no history behind.
    #[test]
    fn bad_arguments_are_reported_without_touching_the_document() {
        let mut server = test_server();
        let session = session_with_paragraph(&mut server, "alpha beta");
        let before = review_state(&mut server, &session);

        let cases: Vec<(&str, Value, &str)> = vec![
            (
                "replace_text",
                json!({ "session_id": session, "find": "", "replace": "x" }),
                "non-empty `find`",
            ),
            (
                "replace_text",
                json!({ "session_id": session, "find": "a" }),
                "requires `replace`",
            ),
            (
                "replace_text",
                json!({ "session_id": session, "find": "a(", "replace": "x", "regex": true }),
                "invalid regex",
            ),
            (
                "comment_on_text",
                json!({ "session_id": session, "query": "alpha", "occurrence": 9, "text": "x" }),
                "text not found: \"alpha\" (occurrence 9)",
            ),
            (
                "comment_on_text",
                json!({ "session_id": session, "query": "zzz", "text": "x" }),
                "text not found",
            ),
            (
                "resolve_comment",
                json!({ "session_id": session, "comment_id": "99", "resolved": true }),
                "comment 99 was not found",
            ),
            (
                "reply_to_comment",
                json!({ "session_id": session, "comment_id": "99", "text": "x" }),
                "comment 99 was not found",
            ),
            (
                "delete_comment",
                json!({ "session_id": session, "comment_id": "99" }),
                "comment 99 was not found",
            ),
            (
                "resolve_comment",
                json!({ "session_id": "no-such-session", "comment_id": "0", "resolved": true }),
                "unknown session_id",
            ),
        ];

        for (tool, arguments, expected) in cases {
            let error = try_tool(&mut server, tool, arguments.clone())
                .expect_err(&format!("{tool} {arguments} should be rejected"));
            assert!(
                error.contains(expected),
                "{tool}: expected the message to mention {expected:?}, got {error:?}",
            );
        }

        assert_eq!(review_state(&mut server, &session), before);
        let undone = tool_json(
            &server
                .handle_message(&call(82, "undo", json!({ "session_id": session })))
                .unwrap(),
        );
        // Only the setup insert is on the history; none of the failures are.
        assert_eq!(undone["steps"].as_u64(), Some(1));
        assert_eq!(paragraph_text(&mut server, &session, 1), "");
    }

    /// Scopes and caps that select nothing are a no-op, not an error: an agent
    /// sweeping a document paragraph by paragraph would otherwise have to guess
    /// where the text runs out.
    #[test]
    fn empty_scopes_replace_nothing_and_say_so() {
        let mut server = test_server();
        let session = session_with_paragraph(&mut server, "alpha beta");

        for arguments in [
            json!({ "session_id": session, "find": "alpha", "replace": "x", "start_paragraph": 5, "end_paragraph": 1 }),
            json!({ "session_id": session, "find": "alpha", "replace": "x", "max_replacements": 0 }),
            json!({ "session_id": session, "find": "zzz", "replace": "x" }),
        ] {
            let result = try_tool(&mut server, "replace_text", arguments.clone())
                .unwrap_or_else(|err| panic!("{arguments} should be a no-op, got {err}"));
            assert_eq!(result["replaced"], json!(0));
            assert_eq!(result["matches"], json!([]));
        }
        assert_eq!(paragraph_text(&mut server, &session, 1), "alpha beta");

        // A search that starts past the last paragraph is empty, not an error.
        let matches = search(
            &mut server,
            &session,
            json!({ "query": "a", "start_paragraph": 99 }),
        );
        assert!(matches.is_empty());
    }

    /// A document that has never been reviewed has no `commentsExtended.xml`,
    /// no relationship to it and no content-type override — and a comment jade
    /// wrote itself has no `w14:paraId` for it to key on. Resolving has to
    /// build all of that, or the flag has nowhere Word would read it from.
    #[test]
    fn resolving_a_comment_builds_the_part_word_reads() {
        let mut server = test_server();
        let session = open_session(&mut server, HELLO_DOCX);
        try_tool(
            &mut server,
            "comment_on_text",
            json!({ "session_id": session, "query": "Hello", "text": "確認", "author": "法務" }),
        )
        .expect("comment");
        let id = comment_ids(&mut server, &session)[0].clone();

        let bytes = export_bytes(&mut server, &session);
        assert!(
            part_xml(&bytes, "word/commentsExtended.xml").is_none(),
            "a comment on its own should not invent a threading part",
        );

        try_tool(
            &mut server,
            "resolve_comment",
            json!({ "session_id": session, "comment_id": id, "resolved": true }),
        )
        .expect("resolve");

        let bytes = export_bytes(&mut server, &session);
        assert_docx_is_consistent(&bytes, "after resolving on a fresh document");
        let extended = part_xml(&bytes, "word/commentsExtended.xml")
            .expect("resolving must create commentsExtended.xml");
        assert!(
            extended.contains(r#"w15:done="1""#),
            "the resolved flag must be in commentsExtended: {extended}",
        );
        // The paraId it keys on has to exist on the comment itself.
        let comments = part_xml(&bytes, "word/comments.xml").unwrap();
        let para_id = attribute_values(&extended, "paraId")
            .into_iter()
            .next()
            .expect("commentsExtended entry");
        assert!(
            comments.contains(&format!(r#"w14:paraId="{para_id}""#)),
            "the comment must carry the paraId commentsExtended addresses it by: {comments}",
        );

        let reopened = reopen(&mut server, &bytes);
        assert_eq!(
            resolution_state(&mut server, &reopened),
            vec![(id, true, String::new())],
        );
    }

    /// Rewriting a comment's body must not detach it from its thread. The
    /// comment being edited here is a reply whose body is two paragraphs, and
    /// commentsExtended addresses it by the paraId of the last one — an editor
    /// that rebuilds the body from scratch loses exactly that.
    #[test]
    fn editing_a_reply_keeps_its_thread_and_resolution() {
        let mut server = test_server();
        let session = open_session(&mut server, COMMENT_DOCX);

        try_tool(
            &mut server,
            "update_comment",
            json!({ "session_id": session, "comment_id": "1", "text": "書き換えた返信" }),
        )
        .expect("update");
        try_tool(
            &mut server,
            "resolve_comment",
            json!({ "session_id": session, "comment_id": "1", "resolved": true }),
        )
        .expect("resolve");

        let bytes = export_bytes(&mut server, &session);
        assert_docx_is_consistent(&bytes, "after editing a reply");
        let extended = part_xml(&bytes, "word/commentsExtended.xml").unwrap();
        assert!(
            extended.contains(r#"w15:paraId="12D526B1" w15:paraIdParent="4C97C83A" w15:done="1""#),
            "the edited reply must stay in its thread and read resolved: {extended}",
        );

        let reopened = reopen(&mut server, &bytes);
        assert_eq!(
            resolution_state(&mut server, &reopened),
            vec![
                ("0".to_string(), false, String::new()),
                ("1".to_string(), true, "0".to_string()),
            ],
        );
    }

    // ---------------------------------------------------------------------
    // Structure: anchoring over tracked changes, ranges that span paragraphs,
    // and edits that merge or split them.
    //
    // A document under review is a document whose paragraphs are already full
    // of `w:ins` / `w:del` wrappers, and the clause an agent wants to comment
    // on rarely stops at a paragraph boundary. These are the paths where the
    // model's idea of a run and the XML's idea of it drift apart.
    // ---------------------------------------------------------------------

    /// Anchors of every comment, as `(paragraph, start, end)` sorted — a shape
    /// that can be compared across an export → reopen.
    fn comment_anchors(server: &mut Server, session: &str) -> Vec<(u64, u64, u64)> {
        let response = server
            .handle_message(&call(
                80,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "comments", "start_comment": 0, "max_comments": 50 }],
                }),
            ))
            .unwrap();
        let mut anchors: Vec<(u64, u64, u64)> = tool_json(&response)["results"][0]["comments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .flat_map(|comment| {
                comment["anchors"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|anchor| {
                        (
                            anchor["paragraph"].as_u64().unwrap_or_default(),
                            anchor["start"].as_u64().unwrap_or_default(),
                            anchor["end"].as_u64().unwrap_or_default(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        anchors.sort();
        anchors
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// A comment must attach anywhere in a paragraph that already carries
        /// tracked changes. review.docx paragraph 0 is "Hello World" with
        /// "Hello " inserted and "World" deleted by a reviewer, so every range
        /// below lands on, inside or across someone's redline — which is the
        /// normal state of a contract coming back from the other side.
        #[test]
        fn a_comment_anchors_over_tracked_changes(
            start in 0usize..11,
            len in 1usize..11,
        ) {
            let end = (start + len).min(11);
            prop_assume!(start < end);

            let mut server = test_server();
            let session = open_session(&mut server, REVIEW_DOCX);
            try_tool(&mut server, "add_comment", json!({
                "session_id": session,
                "start_paragraph": 0, "start_offset": start,
                "end_paragraph": 0, "end_offset": end,
                "text": "確認", "author": "法務",
            }))
            .unwrap_or_else(|err| panic!("commenting {start}..{end} over a redline failed: {err}"));

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, &format!("comment over a redline at {start}..{end}"));
            // The revisions themselves must come through untouched.
            prop_assert_eq!(revisions(&mut server, &session).len(), 2);

            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(comments(&mut server, &reopened).len(), 1);
            prop_assert_eq!(revisions(&mut server, &reopened).len(), 2);
        }

        /// A comment on a whole clause spans paragraphs. The markers then live
        /// in different paragraphs, and the package still has to pair them up
        /// and survive a reopen with the same anchors.
        #[test]
        fn a_comment_can_span_paragraphs(
            start_offset in 0usize..6,
            end_offset in 0usize..6,
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, CONTRACT_DOCX);
            let first = paragraph_text(&mut server, &session, 0);
            let second = paragraph_text(&mut server, &session, 1);
            // Byte offsets have to land on character boundaries.
            let start = first.char_indices().nth(start_offset).map(|(at, _)| at).unwrap_or(0);
            let end = second.char_indices().nth(end_offset + 1).map(|(at, _)| at).unwrap_or(second.len());

            try_tool(&mut server, "add_comment", json!({
                "session_id": session,
                "start_paragraph": 0, "start_offset": start,
                "end_paragraph": 1, "end_offset": end,
                "text": "この条文全体について", "author": "法務",
            }))
            .unwrap_or_else(|err| panic!("commenting across paragraphs failed: {err}"));

            let anchors = comment_anchors(&mut server, &session);
            prop_assert_eq!(anchors.len(), 2, "a two-paragraph comment anchors in both");

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, "comment spanning paragraphs");
            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(comment_anchors(&mut server, &reopened), anchors);
        }

        /// Replacing a range that spans paragraphs merges them, and the result
        /// has to be exactly the two paragraphs joined with the replacement in
        /// the middle — no text left behind from either side.
        ///
        /// The head of the first paragraph used to survive when the range began
        /// at offset 0, which silently doubled the text a redline was replacing.
        #[test]
        fn replacing_across_paragraphs_joins_them_exactly(
            start in 0usize..6,
            end in 0usize..6,
            text in "[A-Z]{0,4}",
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, HELLO_DOCX);
            apply(&mut server, &session, json!([
                { "type": "insert_text", "paragraph": 1, "offset": 0, "text": "second line" },
            ]));
            let first = paragraph_text(&mut server, &session, 0);
            let second = paragraph_text(&mut server, &session, 1);

            apply(&mut server, &session, json!([{
                "type": "replace_range",
                "start_paragraph": 0, "start_offset": start,
                "end_paragraph": 1, "end_offset": end,
                "text": text,
            }]));

            let expected = format!("{}{}{}", &first[..start], text, &second[end..]);
            prop_assert_eq!(paragraph_text(&mut server, &session, 0), expected);
            assert_docx_is_consistent(&export_bytes(&mut server, &session), "after joining paragraphs");
        }

        /// Splitting a commented paragraph in two leaves the comment spanning
        /// both halves. Wherever the break falls, the package still pairs its
        /// markers up and Word can still show the comment.
        #[test]
        fn splitting_a_commented_paragraph_keeps_the_comment_whole(
            offset in 0usize..16,
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, "alpha beta gamma");
            try_tool(&mut server, "comment_on_text", json!({
                "session_id": session, "query": "beta", "text": "確認", "author": "法務",
            })).expect("comment");

            apply(&mut server, &session, json!([{
                "type": "insert_paragraph_break", "paragraph": 1, "offset": offset,
            }]));

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, &format!("after splitting at {offset}"));
            prop_assert_eq!(
                format!(
                    "{}{}",
                    paragraph_text(&mut server, &session, 1),
                    paragraph_text(&mut server, &session, 2),
                ),
                "alpha beta gamma",
            );
            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(comments(&mut server, &reopened).len(), 1);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

        /// An offset is either a real position in the paragraph or an error.
        ///
        /// Offsets come from a read the document has since moved past, and in
        /// Japanese one character is three bytes, so "off by one" lands inside
        /// a character. Editing at the nearest legal position instead would put
        /// the agent's text somewhere it never asked for, and nothing in the
        /// result would say so.
        #[test]
        fn an_impossible_offset_is_rejected_rather_than_moved(
            content in "[甲乙丙a-c]{1,10}",
            offset in 0usize..24,
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);

            let result = try_tool(&mut server, "apply_commands", json!({
                "session_id": session,
                "commands": [{ "type": "insert_text", "paragraph": 1, "offset": offset, "text": "X" }],
            }));

            let legal = offset <= content.len() && content.is_char_boundary(offset);
            prop_assert_eq!(result.is_ok(), legal, "offset {} in {:?}: {:?}", offset, content, result);
            let expected = if legal {
                format!("{}X{}", &content[..offset], &content[offset..])
            } else {
                content.clone()
            };
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), expected);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

        /// Exporting is a fixed point: whatever a session has been through,
        /// writing it out, reading it back and writing it again produces the
        /// same bytes.
        ///
        /// A document goes round the review loop many times, and each pass runs
        /// through this code. Drift that is invisible in one pass — a marker
        /// that moves, an id that is renumbered — compounds into a file that no
        /// longer matches what the other side signed off.
        #[test]
        fn exporting_twice_produces_the_same_file(
            ops in prop::collection::vec((0u8..5, 0usize..4), 0..6),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, COMMENT_DOCX);
            for (op, seed) in &ops {
                run_review_op(&mut server, &session, *op, *seed);
            }

            let first = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&first, "first export");
            let reopened = reopen(&mut server, &first);
            let second = export_bytes(&mut server, &reopened);

            prop_assert_eq!(
                part_xml(&second, "word/document.xml"),
                part_xml(&first, "word/document.xml"),
                "document.xml drifted on the second pass",
            );
            prop_assert_eq!(
                part_xml(&second, "word/comments.xml"),
                part_xml(&first, "word/comments.xml"),
                "comments.xml drifted on the second pass",
            );
            prop_assert_eq!(
                part_xml(&second, "word/commentsExtended.xml"),
                part_xml(&first, "word/commentsExtended.xml"),
                "commentsExtended.xml drifted on the second pass",
            );
        }
    }

    // ---------------------------------------------------------------------
    // Deleting a run of characters, and editing inside table cells
    //
    // A reviewer deletes a word, not thirteen characters; and a contract's
    // 別表 lives in a table, where the paragraphs are cells. Both are places
    // where the headless API's idea of "one change" has to match Word's.
    // ---------------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

        /// Deleting a run of characters is one change, not one per character.
        ///
        /// With tracking on it has to arrive as a single `w:del` carrying the
        /// whole deleted text — a reviewer accepts "the deleted phrase", and a
        /// panel listing a deleted word as thirteen separate entries is
        /// unusable. Undo has to match: one command, one step.
        #[test]
        fn deleting_a_run_of_characters_is_one_tracked_change(
            content in "[a-z甲乙丙]{1,10}",
            count in 1usize..8,
            forward in any::<bool>(),
        ) {
            let characters: Vec<char> = content.chars().collect();
            let offset = if forward { 0 } else { content.len() };
            // Deleting more characters than the paragraph holds runs into the
            // neighbouring one; that case has its own test below.
            let count = count.min(characters.len());
            let deleted = count;

            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            try_tool(&mut server, "start_change_tracking", json!({
                "session_id": session, "author": "AI",
            })).expect("start tracking");

            apply(&mut server, &session, json!([{
                "type": "delete_text", "paragraph": 1, "offset": offset,
                "direction": if forward { "forward" } else { "backward" },
                "count": count,
            }]));

            let expected: String = if forward {
                characters[deleted..].iter().collect()
            } else {
                characters[..characters.len() - deleted].iter().collect()
            };
            let removed: String = if forward {
                characters[..deleted].iter().collect()
            } else {
                characters[characters.len() - deleted..].iter().collect()
            };

            // Deleted text stays in the paragraph until the change is decided.
            // (review.docx carries two revisions of its own in paragraph 0.)
            let tracked = revisions_in(&mut server, &session, 1);
            prop_assert_eq!(
                &tracked,
                &vec![("deleted".to_string(), removed)],
                "one delete must record one revision",
            );

            // Rejecting it puts the document back; accepting it takes the text.
            let mut rejected = test_server();
            let session_b = session_with_paragraph(&mut rejected, &content);
            try_tool(&mut rejected, "start_change_tracking", json!({
                "session_id": session_b, "author": "AI",
            })).expect("start tracking");
            apply(&mut rejected, &session_b, json!([{
                "type": "delete_text", "paragraph": 1, "offset": offset,
                "direction": if forward { "forward" } else { "backward" },
                "count": count,
            }]));
            try_tool(&mut rejected, "reject_revisions", json!({
                "session_id": session_b, "start_paragraph": 1, "end_paragraph": 1,
            }))
            .expect("reject");
            prop_assert_eq!(paragraph_text(&mut rejected, &session_b, 1), content.clone());

            try_tool(&mut server, "accept_revisions", json!({
                "session_id": session, "start_paragraph": 1, "end_paragraph": 1,
            }))
            .expect("accept");
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), expected);
        }

        /// The same delete, untracked, is also one undo step — and undoing it
        /// restores the paragraph exactly.
        #[test]
        fn deleting_a_run_of_characters_is_one_undo_step(
            content in "[a-z甲乙丙]{1,10}",
            count in 1usize..8,
            forward in any::<bool>(),
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, &content);
            let offset = if forward { 0 } else { content.len() };
            let count = count.min(content.chars().count());

            apply(&mut server, &session, json!([{
                "type": "delete_text", "paragraph": 1, "offset": offset,
                "direction": if forward { "forward" } else { "backward" },
                "count": count,
            }]));

            let undone = tool_json(
                &server
                    .handle_message(&call(79, "undo", json!({ "session_id": session, "count": 1 })))
                    .unwrap(),
            );
            prop_assert_eq!(undone["steps"].as_u64(), Some(1));
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), content);
        }

        /// A delete that runs past the start of its paragraph keeps eating into
        /// the one before it, joining the two — that is what Backspace does at
        /// the start of a line, and the count is characters, not "characters in
        /// this paragraph".
        #[test]
        fn deleting_past_the_paragraph_edge_joins_the_previous_one(
            head in "[a-z]{1,6}",
            tail in "[a-z]{1,6}",
            count in 1usize..10,
        ) {
            // review.docx paragraph 1 is empty, so splitting it gives two
            // paragraphs whose text is exactly what this test puts in them.
            let mut server = test_server();
            let session = open_session(&mut server, REVIEW_DOCX);
            apply(&mut server, &session, json!([
                { "type": "insert_text", "paragraph": 1, "offset": 0, "text": tail },
                { "type": "insert_paragraph_break", "paragraph": 1, "offset": 0 },
            ]));
            apply(&mut server, &session, json!([
                { "type": "insert_text", "paragraph": 1, "offset": 0, "text": head },
            ]));
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), head.clone());
            prop_assert_eq!(paragraph_text(&mut server, &session, 2), tail.clone());

            // One step joins the paragraphs and the rest eat into `head`;
            // going further would join paragraph 1 with the fixture's own
            // paragraph 0, which is not what this test is about.
            let count = count.min(head.len() + 1);
            apply(&mut server, &session, json!([{
                "type": "delete_text", "paragraph": 2, "offset": 0,
                "direction": "backward", "count": count,
            }]));

            // The first backspace joins the paragraphs; the rest eat `head`.
            let eaten = count - 1;
            let expected = format!("{}{}", &head[..head.len() - eaten], tail);
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), expected);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

        /// A contract's 別表 is a table, so review has to work inside one:
        /// comment on a cell, redline a cell, decide the redline, hand the file
        /// back. Paragraph indices run through the cells, and the anchors have
        /// to come back out of the file pointing at the same ones.
        #[test]
        fn reviewing_inside_a_table_cell_round_trips(
            cell in prop::sample::select(vec![0usize, 1, 2, 3]),
            replacement in "[a-z]{1,5}",
            accept in any::<bool>(),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            // Give every cell known text; cells are paragraphs 0..=3.
            for index in 0..4usize {
                let end = paragraph_text(&mut server, &session, index).len();
                apply(&mut server, &session, json!([{
                    "type": "replace_range",
                    "start_paragraph": index, "start_offset": 0,
                    "end_paragraph": index, "end_offset": end,
                    "text": format!("cell{index}"),
                }]));
            }

            try_tool(&mut server, "add_comment", json!({
                "session_id": session,
                "start_paragraph": cell, "start_offset": 0,
                "end_paragraph": cell, "end_offset": 5,
                "text": "セルへの指摘", "author": "法務",
            }))
            .unwrap_or_else(|err| panic!("commenting in cell {cell} failed: {err}"));

            try_tool(&mut server, "start_change_tracking", json!({
                "session_id": session, "author": "AI",
            })).expect("start tracking");
            replace_text(&mut server, &session, json!({
                "find": format!("cell{cell}"), "replace": replacement,
            }));

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, &format!("table cell {cell} under review"));

            // The reviewer opens the file and decides the change there.
            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(comments(&mut server, &reopened).len(), 1);
            let tool = if accept { "accept_revisions" } else { "reject_revisions" };
            try_tool(&mut server, tool, json!({ "session_id": reopened })).expect("decide");

            let expected = if accept { replacement } else { format!("cell{cell}") };
            prop_assert_eq!(paragraph_text(&mut server, &reopened, cell), expected);
            assert_docx_is_consistent(&export_bytes(&mut server, &reopened), "table cell decided");
        }

        /// Deciding every tracked change is a fixed point: nothing is left to
        /// decide, the text does not move again, and deciding a second time
        /// changes nothing. A reviewer who clicks "accept all" twice must not
        /// get a different document than one who clicks it once.
        #[test]
        fn deciding_every_change_twice_changes_nothing(
            accept in any::<bool>(),
            edits in prop::collection::vec("[a-z]{1,4}", 1..4),
        ) {
            let mut server = test_server();
            let session = session_with_paragraph(&mut server, "alpha beta gamma");
            try_tool(&mut server, "start_change_tracking", json!({
                "session_id": session, "author": "AI",
            })).expect("start tracking");
            for (index, edit) in edits.iter().enumerate() {
                apply(&mut server, &session, json!([{
                    "type": "insert_text", "paragraph": 1, "offset": index, "text": edit,
                }]));
            }

            let tool = if accept { "accept_revisions" } else { "reject_revisions" };
            try_tool(&mut server, tool, json!({ "session_id": session })).expect("decide");
            let settled = paragraph_text(&mut server, &session, 1);
            prop_assert!(revisions(&mut server, &session).is_empty());

            try_tool(&mut server, tool, json!({ "session_id": session })).expect("decide again");
            prop_assert_eq!(paragraph_text(&mut server, &session, 1), settled.clone());

            // …and it survives the hand-off unchanged.
            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, "after deciding everything");
            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(paragraph_text(&mut server, &reopened, 1), settled);
            prop_assert!(revisions(&mut server, &reopened).is_empty());
        }
    }

    // ---------------------------------------------------------------------
    // Page margins, line spacing, and table geometry and appearance
    //
    // The layout knobs a drafter reaches for. Each one has to be readable back
    // — an agent that cannot see the current spacing cannot decide whether to
    // change it — survive the hand-off, and be undoable.
    // ---------------------------------------------------------------------

    /// The page setup as the query reports it.
    fn page_setup(server: &mut Server, session: &str) -> Value {
        let response = server
            .handle_message(&call(
                77,
                "query",
                json!({ "session_id": session, "queries": [{ "type": "page_setup" }] }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["page_setup"].clone()
    }

    /// One paragraph's snapshot.
    fn paragraph_snapshot(server: &mut Server, session: &str, paragraph: usize) -> Value {
        let response = server
            .handle_message(&call(
                76,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "paragraph", "paragraph": paragraph }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["value"].clone()
    }

    /// One table's snapshot.
    fn table_snapshot(server: &mut Server, session: &str) -> Value {
        let response = server
            .handle_message(&call(
                75,
                "query",
                json!({
                    "session_id": session,
                    "queries": [{ "type": "table", "table_order_index": 0 }],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"][0]["value"].clone()
    }

    fn table_structure_snapshot(table: &Value) -> Value {
        json!({
            "columns": table["columns"].as_array().cloned().unwrap_or_default()
                .iter().map(|column| column["width"].clone()).collect::<Vec<_>>(),
            "rows": table["rows"].as_array().cloned().unwrap_or_default()
                .iter().map(|row| json!({
                    "grid_before": row["grid_before"].clone(),
                    "grid_after": row["grid_after"].clone(),
                    "cells": row["cells"].as_array().cloned().unwrap_or_default()
                        .iter().map(|cell| json!({
                            "text": cell["text"].clone(),
                            "span": cell["span"].clone(),
                            "vertical_merge": cell["vertical_merge"].clone(),
                        })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
        })
    }

    fn row_grid_offset(row: &Value, field: &str) -> usize {
        row[field].as_u64().unwrap_or(0) as usize
    }

    /// Columns the rows actually occupy: the widest row, counting spans and
    /// the leading/trailing grid columns that a row deliberately omits.
    fn occupied_columns(table: &Value) -> usize {
        table["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|row| {
                row_grid_offset(row, "grid_before")
                    + row["cells"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .map(|cell| cell["span"].as_u64().unwrap_or(1).max(1) as usize)
                        .sum::<usize>()
                    + row_grid_offset(row, "grid_after")
            })
            .max()
            .unwrap_or(0)
    }

    fn assert_table_vertical_merges_are_valid(table: &Value) {
        let Some(rows) = table["rows"].as_array() else {
            return;
        };
        let ranges: Vec<Vec<(usize, usize, String)>> = rows
            .iter()
            .map(|row| {
                let mut column = row_grid_offset(row, "grid_before");
                row["cells"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|cell| {
                        let span = cell["span"].as_u64().unwrap_or(1).max(1) as usize;
                        let start = column;
                        column += span;
                        cell["vertical_merge"]
                            .as_str()
                            .map(|marker| (start, column, marker.to_string()))
                    })
                    .collect()
            })
            .collect();

        for (row_index, row) in ranges.iter().enumerate() {
            for (start, end, marker) in row {
                match marker.as_str() {
                    "restart" => assert!(
                        ranges.get(row_index + 1).is_some_and(|next| {
                            next.iter().any(|(next_start, next_end, next_marker)| {
                                next_start == start
                                    && next_end == end
                                    && next_marker == "continue"
                            })
                        }),
                        "orphan vertical-merge restart at row {row_index}, columns {start}..{end}: {table}",
                    ),
                    "continue" => assert!(
                        row_index.checked_sub(1).is_some_and(|previous| {
                            ranges[previous].iter().any(
                                |(previous_start, previous_end, previous_marker)| {
                                    previous_start == start
                                        && previous_end == end
                                        && matches!(
                                            previous_marker.as_str(),
                                            "restart" | "continue"
                                        )
                                },
                            )
                        }),
                        "orphan vertical-merge continuation at row {row_index}, columns {start}..{end}: {table}",
                    ),
                    other => panic!("unknown vertical-merge marker {other}: {table}"),
                }
            }
        }
    }

    fn xml_grid_offset(row: &str, name: &str) -> usize {
        let mut result = 0usize;
        for_each_direct_element(row, "w:trPr", |_tag, properties| {
            for_each_element(properties, name, |tag, _| {
                result = attribute_values_exact(tag, "w:val")
                    .first()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
            });
        });
        result
    }

    /// The first table in these fixtures is flat. Verify the OOXML preferred
    /// width of every cell against the sum of the logical grid columns it
    /// covers, including `gridSpan` cells produced by a merge.
    fn assert_first_table_cell_widths_match_grid(docx_bytes: &[u8], label: &str) {
        let document = part_xml(docx_bytes, "word/document.xml")
            .unwrap_or_else(|| panic!("{label}: no word/document.xml"));
        let mut first_table = None;
        for_each_element(&document, "w:tbl", |_tag, body| {
            if first_table.is_none() {
                first_table = Some(body.to_string());
            }
        });
        let table = first_table.unwrap_or_else(|| panic!("{label}: no table"));

        let mut grid = Vec::new();
        for_each_element(&table, "w:gridCol", |tag, _| {
            let width = attribute_values_exact(tag, "w:w")
                .first()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or_else(|| panic!("{label}: grid column has no numeric width: {tag}"));
            grid.push(width);
        });

        for_each_element(&table, "w:tr", |_tag, row| {
            let mut column = xml_grid_offset(row, "w:gridBefore");
            let grid_after = xml_grid_offset(row, "w:gridAfter");
            for_each_element(row, "w:tc", |_tag, cell| {
                let mut span = 1usize;
                for_each_element(cell, "w:gridSpan", |tag, _| {
                    span = attribute_values_exact(tag, "w:val")
                        .first()
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(1)
                        .max(1);
                });
                let mut actual = None;
                for_each_element(cell, "w:tcW", |tag, _| {
                    actual = attribute_values_exact(tag, "w:w")
                        .first()
                        .and_then(|value| value.parse::<u32>().ok());
                });
                let expected: u32 = grid
                    .get(column..column + span)
                    .unwrap_or_else(|| {
                        panic!(
                            "{label}: cell at column {column} spans {span} beyond {} grid columns",
                            grid.len()
                        )
                    })
                    .iter()
                    .sum();
                assert_eq!(
                    actual,
                    Some(expected),
                    "{label}: cell at column {column} spanning {span} columns disagrees with tblGrid",
                );
                column += span;
            });
            assert_eq!(
                column + grid_after,
                grid.len(),
                "{label}: row covers {} columns but tblGrid has {}",
                column + grid_after,
                grid.len()
            );
        });
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// A valid first command followed by any deterministic coordinate or
        /// table error must be indistinguishable from never attempting the
        /// batch, including its undo history.
        #[test]
        fn failing_sequential_batches_are_atomic(
            seed_text in "[A-Z]{1,8}",
            batch_text in "[a-z]{1,8}",
            invalid_kind in 0u8..5,
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, HELLO_DOCX);
            apply(&mut server, &session, json!([{
                "type": "insert_text",
                "paragraph": 0,
                "offset": 0,
                "text": seed_text,
            }]));
            let before = document_state(&mut server, &session);

            let invalid = match invalid_kind {
                0 => json!({
                    "type": "insert_text",
                    "paragraph": 999,
                    "offset": 0,
                    "text": "invalid",
                }),
                1 => json!({
                    "type": "replace_range",
                    "start_paragraph": 0,
                    "start_offset": 999,
                    "end_paragraph": 0,
                    "end_offset": 999,
                    "text": "invalid",
                }),
                2 => json!({
                    "type": "insert_table_row",
                    "table_order_index": 999,
                    "row_index": 0,
                }),
                3 => json!({
                    "type": "insert_table_column",
                    "table_order_index": 999,
                    "column_index": 0,
                }),
                _ => Value::Null,
            };
            let arguments = if invalid_kind == 4 {
                json!({
                    "session_id": session,
                    "coordinates": "sequential",
                    "commands": [{
                        "type": "insert_text",
                        "paragraph": 0,
                        "offset": 0,
                        "text": batch_text,
                    }],
                    "queries": [{ "type": "search_text", "query": "" }],
                })
            } else {
                json!({
                    "session_id": session,
                    "coordinates": "sequential",
                    "commands": [
                        {
                            "type": "insert_text",
                            "paragraph": 0,
                            "offset": 0,
                            "text": batch_text,
                        },
                        invalid,
                    ],
                })
            };
            let error = try_tool(&mut server, "apply_commands", arguments)
                .expect_err("the later operation must reject the batch");
            if invalid_kind == 4 {
                prop_assert!(error.contains("query failed"), "{error}");
            } else {
                prop_assert!(error.contains("index 1"), "{error}");
            }
            prop_assert_eq!(document_state(&mut server, &session), before);

            let undone = try_tool(&mut server, "undo", json!({
                "session_id": session,
                "count": 100,
            })).expect("undo pre-existing edit");
            prop_assert_eq!(undone["steps"].as_u64(), Some(1));
            prop_assert_eq!(paragraph_text(&mut server, &session, 0), "Hello\u{c}");
        }

        /// Margins go in, come back out of the query, survive the hand-off, and
        /// undo puts the page back the way it was.
        #[test]
        fn page_margins_round_trip_and_undo(
            top in 0u32..200,
            bottom in 0u32..200,
            left in 0u32..200,
            right in 0u32..200,
        ) {
            // Whole points: the model keeps these as `f32`, and Word stores
            // them as twips, so this is not the place to test float precision.
            let (top, bottom, left, right) =
                (f64::from(top), f64::from(bottom), f64::from(left), f64::from(right));
            let mut server = test_server();
            let session = open_session(&mut server, HELLO_DOCX);
            let before = page_setup(&mut server, &session);

            apply(&mut server, &session, json!([{
                "type": "set_page_margins",
                "top": top, "bottom": bottom, "left": left, "right": right,
            }]));

            let after = page_setup(&mut server, &session);
            prop_assert_eq!(after["margin_top"].as_f64(), Some(top));
            prop_assert_eq!(after["margin_bottom"].as_f64(), Some(bottom));
            prop_assert_eq!(after["margin_left"].as_f64(), Some(left));
            prop_assert_eq!(after["margin_right"].as_f64(), Some(right));
            // Nothing else about the page moved.
            prop_assert_eq!(after["width"].clone(), before["width"].clone());
            prop_assert_eq!(after["height"].clone(), before["height"].clone());
            prop_assert_eq!(after["orientation"].clone(), before["orientation"].clone());

            let bytes = export_bytes(&mut server, &session);
            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(page_setup(&mut server, &reopened), after);

            tool_json(
                &server
                    .handle_message(&call(74, "undo", json!({ "session_id": session })))
                    .unwrap(),
            );
            prop_assert_eq!(page_setup(&mut server, &session), before);
        }

        /// Line spacing and indents are readable after being set — an agent
        /// that cannot see the current spacing cannot decide whether to change
        /// it — and they survive export and reopen.
        #[test]
        fn line_spacing_and_indent_are_readable_and_survive_the_round_trip(
            line in prop::sample::select(vec![0.5f64, 1.0, 1.5, 2.0, 2.5, 3.0]),
            left in -40i32..80,
            first_line in 0u32..40,
        ) {
            let (left, first_line) = (f64::from(left), f64::from(first_line));
            let mut server = test_server();
            let session = open_session(&mut server, CONTRACT_DOCX);
            let before = paragraph_snapshot(&mut server, &session, 0);

            apply(&mut server, &session, json!([
                { "type": "set_paragraph_line_spacing", "start_paragraph": 0, "end_paragraph": 0, "line": line },
                { "type": "set_paragraph_indent", "start_paragraph": 0, "end_paragraph": 0, "left": left, "first_line": first_line },
            ]));

            let after = paragraph_snapshot(&mut server, &session, 0);
            prop_assert_eq!(after["spacing"]["line"].as_f64(), Some(line));
            prop_assert_eq!(after["indent"]["left"].as_f64(), Some(left));
            prop_assert_eq!(after["indent"]["first_line"].as_f64(), Some(first_line));
            prop_assert_eq!(after["text"].clone(), before["text"].clone());

            let bytes = export_bytes(&mut server, &session);
            let reopened = reopen(&mut server, &bytes);
            let reread = paragraph_snapshot(&mut server, &reopened, 0);
            prop_assert_eq!(reread["spacing"]["line"].as_f64(), Some(line));
            prop_assert_eq!(reread["indent"]["left"].as_f64(), Some(left));
            prop_assert_eq!(reread["indent"]["first_line"].as_f64(), Some(first_line));

            // Two commands, two undo steps, and the paragraph is back.
            tool_json(
                &server
                    .handle_message(&call(73, "undo", json!({ "session_id": session, "count": 2 })))
                    .unwrap(),
            );
            prop_assert_eq!(paragraph_snapshot(&mut server, &session, 0), before);
        }

        /// The column widths a caller gives have to describe the columns the
        /// table has. A different count used to rewrite `w:tblGrid` to that
        /// arity and leave the rows alone, which is a table Word has to repair.
        #[test]
        fn column_widths_must_match_the_column_count(
            count in 0usize..5,
            width in 0u32..200,
        ) {
            let width = f64::from(width);
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);
            let columns = occupied_columns(&before);
            prop_assert_eq!(columns, 2, "the fixture is a 2x2 table");

            let widths: Vec<f64> = (0..count).map(|index| width + index as f64).collect();
            let result = try_tool(&mut server, "apply_commands", json!({
                "session_id": session,
                "commands": [{ "type": "set_table_column_widths", "table_order_index": 0, "widths": widths }],
            }));

            let after = table_snapshot(&mut server, &session);
            if count == columns {
                result.expect("a width per column must be accepted");
                let expected: Vec<f64> = widths.iter().map(|width| width.max(9.0)).collect();
                let applied: Vec<f64> = after["columns"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|column| column["width"].as_f64().unwrap_or(f64::NAN))
                    .collect();
                prop_assert_eq!(applied, expected);
            } else {
                prop_assert!(result.is_err(), "{count} widths for {columns} columns must be refused");
                prop_assert_eq!(after, before, "a refused call must not touch the table");
            }
        }

        /// Insert accepts the exact row/column count as append, but anything
        /// larger is a coordinate error and must leave the table untouched.
        #[test]
        fn out_of_range_table_insert_is_an_unchanged_error(
            row_insert in any::<bool>(),
            excess in 1usize..8,
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);
            let index = if row_insert {
                before["rows"].as_array().map_or(0, Vec::len) + excess
            } else {
                before["columns"].as_array().map_or(0, Vec::len) + excess
            };
            let command = if row_insert {
                json!({
                    "type": "insert_table_row",
                    "table_order_index": 0,
                    "row_index": index,
                })
            } else {
                json!({
                    "type": "insert_table_column",
                    "table_order_index": 0,
                    "column_index": index,
                })
            };

            let result = try_tool(&mut server, "apply_commands", json!({
                "session_id": session,
                "commands": [command],
            }));
            prop_assert!(result.is_err());
            prop_assert_eq!(table_snapshot(&mut server, &session), before);
            assert_docx_is_structurally_valid(
                &export_bytes(&mut server, &session),
                "after refused table insertion",
            );
        }

        /// Merge, resize the same logical columns while the cells are merged,
        /// unmerge, export/reopen, then undo and redo the whole sequence. The
        /// grid widths and every cell's tcW must agree at every shape.
        #[test]
        fn merge_resize_unmerge_keeps_logical_widths_through_round_trips(
            merge_case in 0u8..5,
            initial_left in 0u32..240,
            initial_right in 0u32..240,
            resized_left in 0u32..240,
            resized_right in 0u32..240,
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);

            let (selected_cells, target) = match merge_case {
                0 => (
                    json!([
                        { "row": 0, "column": 0, "column_span": 1 },
                        { "row": 0, "column": 1, "column_span": 1 }
                    ]),
                    json!({ "row": 0, "column": 0, "column_span": 2 }),
                ),
                1 => (
                    json!([
                        { "row": 1, "column": 0, "column_span": 1 },
                        { "row": 1, "column": 1, "column_span": 1 }
                    ]),
                    json!({ "row": 1, "column": 0, "column_span": 2 }),
                ),
                2 => (
                    json!([
                        { "row": 0, "column": 0, "column_span": 1 },
                        { "row": 1, "column": 0, "column_span": 1 }
                    ]),
                    json!({ "row": 0, "column": 0, "column_span": 1 }),
                ),
                3 => (
                    json!([
                        { "row": 0, "column": 1, "column_span": 1 },
                        { "row": 1, "column": 1, "column_span": 1 }
                    ]),
                    json!({ "row": 0, "column": 1, "column_span": 1 }),
                ),
                _ => (
                    json!([
                        { "row": 0, "column": 0, "column_span": 1 },
                        { "row": 0, "column": 1, "column_span": 1 },
                        { "row": 1, "column": 0, "column_span": 1 },
                        { "row": 1, "column": 1, "column_span": 1 }
                    ]),
                    json!({ "row": 0, "column": 0, "column_span": 2 }),
                ),
            };

            apply(&mut server, &session, json!([{
                "type": "set_table_column_widths",
                "table_order_index": 0,
                "widths": [initial_left, initial_right]
            }]));
            apply(&mut server, &session, json!([{
                "type": "merge_table_cells",
                "table_order_index": 0,
                "selected_cells": selected_cells
            }]));
            let merged_bytes = export_bytes(&mut server, &session);
            assert_first_table_cell_widths_match_grid(&merged_bytes, "after merge");

            apply(&mut server, &session, json!([{
                "type": "resize_table_column_between",
                "table_order_index": 0,
                "left_column_index": 0,
                "new_left_width": resized_left,
                "new_right_width": resized_right
            }]));
            let resized_bytes = export_bytes(&mut server, &session);
            assert_first_table_cell_widths_match_grid(&resized_bytes, "after merged resize");

            apply(&mut server, &session, json!([{
                "type": "unmerge_table_cells",
                "table_order_index": 0,
                "target_cells": [target]
            }]));
            let after = table_snapshot(&mut server, &session);
            let expected_widths = [resized_left.max(9) as f64, resized_right.max(9) as f64];
            let actual_widths: Vec<f64> = after["columns"]
                .as_array()
                .expect("columns")
                .iter()
                .map(|column| column["width"].as_f64().expect("column width"))
                .collect();
            prop_assert_eq!(actual_widths, expected_widths);
            prop_assert_eq!(occupied_columns(&after), 2);
            for row in after["rows"].as_array().expect("rows") {
                prop_assert_eq!(row["cells"].as_array().expect("cells").len(), 2);
                for cell in row["cells"].as_array().expect("cells") {
                    prop_assert!(cell["span"].is_null() || cell["span"].as_u64() == Some(1));
                    prop_assert!(cell["vertical_merge"].is_null());
                }
            }

            let final_bytes = export_bytes(&mut server, &session);
            assert_docx_is_structurally_valid(&final_bytes, "after merge-resize-unmerge");
            assert_first_table_cell_widths_match_grid(&final_bytes, "after unmerge");
            let reopened = reopen(&mut server, &final_bytes);
            prop_assert_eq!(table_snapshot(&mut server, &reopened), after.clone());

            let undone = try_tool(
                &mut server,
                "undo",
                json!({ "session_id": session, "count": 4 }),
            ).expect("four table edits undo");
            prop_assert_eq!(undone["steps"].as_u64(), Some(4));
            prop_assert_eq!(table_snapshot(&mut server, &session), before);

            let redone = try_tool(
                &mut server,
                "redo",
                json!({ "session_id": session, "count": 4 }),
            ).expect("four table edits redo");
            prop_assert_eq!(redone["steps"].as_u64(), Some(4));
            prop_assert_eq!(table_snapshot(&mut server, &session), after);
        }

        /// Invalid logical column coordinates must be rejected even after a
        /// merge has reduced the physical cell count. They must never grow
        /// tblGrid or leave a partial width edit behind.
        #[test]
        fn out_of_range_resize_after_merge_is_an_unchanged_error(
            resize_kind in any::<bool>(),
            invalid_column in 2usize..8,
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            apply(&mut server, &session, json!([{
                "type": "merge_table_cells",
                "table_order_index": 0,
                "selected_cells": [
                    { "row": 0, "column": 0, "column_span": 1 },
                    { "row": 0, "column": 1, "column_span": 1 }
                ]
            }]));
            let before = table_snapshot(&mut server, &session);
            let command = if resize_kind {
                json!({
                    "type": "resize_table_column_between",
                    "table_order_index": 0,
                    "left_column_index": invalid_column,
                    "new_left_width": 30,
                    "new_right_width": 40
                })
            } else {
                json!({
                    "type": "resize_table_column_right_edge",
                    "table_order_index": 0,
                    "column_index": invalid_column,
                    "new_width": 30
                })
            };

            let result = try_tool(&mut server, "apply_commands", json!({
                "session_id": session,
                "commands": [command]
            }));
            prop_assert!(result.is_err());
            prop_assert_eq!(table_snapshot(&mut server, &session), before);
            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_structurally_valid(&bytes, "after refused merged resize");
            assert_first_table_cell_widths_match_grid(&bytes, "after refused merged resize");
        }

        /// However rows and columns are added and removed, the grid keeps
        /// describing the cells: one width per column the rows occupy, and no
        /// column without a width — a grid column with no width renders as
        /// nothing.
        #[test]
        fn table_structure_edits_keep_the_grid_describing_the_cells(
            ops in prop::collection::vec((0u8..4, 0usize..3), 1..6),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);

            for (op, at) in &ops {
                let command = match op {
                    0 => json!({ "type": "insert_table_row", "table_order_index": 0, "row_index": at }),
                    1 => json!({ "type": "insert_table_column", "table_order_index": 0, "column_index": at }),
                    2 => json!({ "type": "delete_table_row", "table_order_index": 0, "row_index": at }),
                    _ => json!({ "type": "delete_table_column", "table_order_index": 0, "column_index": at }),
                };
                // Out-of-range indices are refused; that is not what is under
                // test here, so only the accepted ones matter.
                let _ = try_tool(&mut server, "apply_commands", json!({
                    "session_id": session, "commands": [command],
                }));

                let table = table_snapshot(&mut server, &session);
                if table.is_null() {
                    break; // the last row or column was deleted with the table
                }
                assert_table_vertical_merges_are_valid(&table);
                let columns = table["columns"].as_array().cloned().unwrap_or_default();
                prop_assert_eq!(
                    columns.len(),
                    occupied_columns(&table),
                    "the grid must have one column per column the rows occupy: {}", table,
                );
                for column in &columns {
                    prop_assert!(
                        column["width"].as_f64().is_some(),
                        "every grid column needs a width: {}", table,
                    );
                }
            }

            assert_docx_is_consistent(&export_bytes(&mut server, &session), "after table structure edits");
        }

        /// A vertical merge followed by an insertion/deletion at any valid row
        /// boundary must remain a valid chain through export/reopen and
        /// undo/redo. This concentrates the random search on row boundaries
        /// around a merge rather than relying only on the whole-surface fuzzer.
        #[test]
        fn vertical_merge_row_edits_round_trip_and_undo(
            rows in 2usize..7,
            merge_start in 0usize..6,
            merge_len in 2usize..7,
            insert in any::<bool>(),
            at in 0usize..7,
        ) {
            prop_assume!(merge_start + merge_len <= rows);
            prop_assume!(if insert { at <= rows } else { at < rows });

            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            for row_count in 2..rows {
                apply(&mut server, &session, json!([{
                    "type": "insert_table_row",
                    "table_order_index": 0,
                    "row_index": row_count,
                }]));
            }
            let selected_cells = (merge_start..merge_start + merge_len)
                .map(|row| json!({ "row": row, "column": 0, "column_span": 1 }))
                .collect::<Vec<_>>();
            apply(&mut server, &session, json!([{
                "type": "merge_table_cells",
                "table_order_index": 0,
                "selected_cells": selected_cells,
            }]));
            let merged = table_snapshot(&mut server, &session);
            assert_table_vertical_merges_are_valid(&merged);

            let edit = if insert {
                json!({
                    "type": "insert_table_row",
                    "table_order_index": 0,
                    "row_index": at,
                })
            } else {
                json!({
                    "type": "delete_table_row",
                    "table_order_index": 0,
                    "row_index": at,
                })
            };
            apply(&mut server, &session, json!([edit]));
            let after = table_snapshot(&mut server, &session);
            assert_table_vertical_merges_are_valid(&after);
            prop_assert_eq!(occupied_columns(&after), 2);

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_structurally_valid(&bytes, "after vertical merge row edit");
            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(
                table_structure_snapshot(&table_snapshot(&mut server, &reopened)),
                table_structure_snapshot(&after),
            );

            let undone = try_tool(&mut server, "undo", json!({
                "session_id": session,
                "count": 1,
            })).expect("row edit undo");
            prop_assert_eq!(undone["steps"].as_u64(), Some(1));
            prop_assert_eq!(
                table_structure_snapshot(&table_snapshot(&mut server, &session)),
                table_structure_snapshot(&merged),
            );

            let redone = try_tool(&mut server, "redo", json!({
                "session_id": session,
                "count": 1,
            })).expect("row edit redo");
            prop_assert_eq!(redone["steps"].as_u64(), Some(1));
            let redone_table = table_snapshot(&mut server, &session);
            assert_table_vertical_merges_are_valid(&redone_table);
            prop_assert_eq!(
                table_structure_snapshot(&redone_table),
                table_structure_snapshot(&after),
            );
        }

        /// Column insertion/deletion is exercised against horizontal,
        /// vertical and rectangular merges, then the surviving merge is
        /// unmerged. Every intermediate shape must round-trip and undo/redo.
        #[test]
        fn merged_table_column_edits_and_unmerge_round_trip(
            merge_case in 0u8..4,
            insert in any::<bool>(),
            at in 0usize..3,
            width in 0u32..160,
        ) {
            prop_assume!(if insert { at <= 2 } else { at < 2 });
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let selected_cells = match merge_case {
                0 => json!([
                    { "row": 0, "column": 0, "column_span": 1 },
                    { "row": 0, "column": 1, "column_span": 1 }
                ]),
                1 => json!([
                    { "row": 0, "column": 0, "column_span": 1 },
                    { "row": 1, "column": 0, "column_span": 1 }
                ]),
                2 => json!([
                    { "row": 0, "column": 1, "column_span": 1 },
                    { "row": 1, "column": 1, "column_span": 1 }
                ]),
                _ => json!([
                    { "row": 0, "column": 0, "column_span": 1 },
                    { "row": 0, "column": 1, "column_span": 1 },
                    { "row": 1, "column": 0, "column_span": 1 },
                    { "row": 1, "column": 1, "column_span": 1 }
                ]),
            };
            apply(&mut server, &session, json!([{
                "type": "merge_table_cells",
                "table_order_index": 0,
                "selected_cells": selected_cells,
            }]));
            let merged = table_snapshot(&mut server, &session);

            let structure_edit = if insert {
                json!({
                    "type": "insert_table_column",
                    "table_order_index": 0,
                    "column_index": at,
                    "width": width,
                })
            } else {
                json!({
                    "type": "delete_table_column",
                    "table_order_index": 0,
                    "column_index": at,
                })
            };
            apply(&mut server, &session, json!([structure_edit]));
            let after = table_snapshot(&mut server, &session);
            let expected_columns = if insert { 3 } else { 1 };
            prop_assert_eq!(after["columns"].as_array().map_or(0, Vec::len), expected_columns);
            prop_assert_eq!(occupied_columns(&after), expected_columns);
            assert_table_vertical_merges_are_valid(&after);
            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_structurally_valid(&bytes, "after merged column edit");
            assert_first_table_cell_widths_match_grid(&bytes, "after merged column edit");
            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(
                table_structure_snapshot(&table_snapshot(&mut server, &reopened)),
                table_structure_snapshot(&after),
            );

            try_tool(&mut server, "undo", json!({ "session_id": session }))
                .expect("undo column edit");
            prop_assert_eq!(
                table_structure_snapshot(&table_snapshot(&mut server, &session)),
                table_structure_snapshot(&merged),
            );
            try_tool(&mut server, "redo", json!({ "session_id": session }))
                .expect("redo column edit");
            prop_assert_eq!(
                table_structure_snapshot(&table_snapshot(&mut server, &session)),
                table_structure_snapshot(&after),
            );

            let target = after["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|row| row["cells"].as_array().into_iter().flatten())
                .find(|cell| {
                    cell["span"].as_u64().unwrap_or(1) > 1
                        || cell["vertical_merge"] == json!("restart")
                })
                .map(|cell| json!({
                    "row": cell["row"].clone(),
                    "column": cell["column"].clone(),
                    "column_span": cell["span"].as_u64().unwrap_or(1),
                }));
            if let Some(target) = target {
                apply(&mut server, &session, json!([{
                    "type": "unmerge_table_cells",
                    "table_order_index": 0,
                    "target_cells": [target],
                }]));
                let unmerged = table_snapshot(&mut server, &session);
                prop_assert_eq!(occupied_columns(&unmerged), expected_columns);
                assert_table_vertical_merges_are_valid(&unmerged);
                let bytes = export_bytes(&mut server, &session);
                assert_docx_is_structurally_valid(&bytes, "after structural edit and unmerge");
                assert_first_table_cell_widths_match_grid(
                    &bytes,
                    "after structural edit and unmerge",
                );

                try_tool(&mut server, "undo", json!({ "session_id": session }))
                    .expect("undo unmerge");
                prop_assert_eq!(
                    table_structure_snapshot(&table_snapshot(&mut server, &session)),
                    table_structure_snapshot(&after),
                );
                try_tool(&mut server, "redo", json!({ "session_id": session }))
                    .expect("redo unmerge");
                prop_assert_eq!(
                    table_structure_snapshot(&table_snapshot(&mut server, &session)),
                    table_structure_snapshot(&unmerged),
                );
            }
        }

        /// Table alignment, cell shading and cell vertical alignment: set,
        /// read back, hand off, reopen, undo.
        #[test]
        fn table_appearance_round_trips_and_undoes(
            alignment in prop::sample::select(vec!["left", "center", "right"]),
            vertical in prop::sample::select(vec!["top", "center", "bottom"]),
            red in 0u8..=255,
            row in 0usize..2,
            column in 0usize..2,
        ) {
            let color = format!("#{red:02X}80FF");
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);

            apply(&mut server, &session, json!([
                { "type": "set_table_alignment", "table_order_index": 0, "alignment": alignment },
                { "type": "set_table_cell_shading", "table_order_index": 0, "row": row, "column": column, "color": color },
                { "type": "set_table_cell_vertical_alignment", "table_order_index": 0, "row": row, "column": column, "alignment": vertical },
            ]));

            let after = table_snapshot(&mut server, &session);
            prop_assert_eq!(after["alignment"].as_str(), Some(alignment));
            prop_assert_eq!(
                after["rows"][row]["cells"][column]["style"]["background-color"].as_str(),
                Some(color.as_str()),
            );

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, "after styling a table");
            let reopened = reopen(&mut server, &bytes);
            let reread = table_snapshot(&mut server, &reopened);
            prop_assert_eq!(reread["alignment"].as_str(), Some(alignment));
            prop_assert_eq!(
                reread["rows"][row]["cells"][column]["style"]["background-color"].as_str(),
                Some(color.as_str()),
            );

            // Three commands, three undo steps.
            tool_json(
                &server
                    .handle_message(&call(72, "undo", json!({ "session_id": session, "count": 3 })))
                    .unwrap(),
            );
            prop_assert_eq!(table_snapshot(&mut server, &session), before);
        }
    }

    /// The colours and keywords these commands refuse, and the fact that a
    /// refusal leaves the table exactly as it was.
    #[test]
    fn table_appearance_rejects_values_word_cannot_store() {
        let mut server = test_server();
        let session = open_session(&mut server, TABLE_DOCX);
        let before = table_snapshot(&mut server, &session);

        for (arguments, expected) in [
            (
                json!({ "type": "set_table_alignment", "table_order_index": 0, "alignment": "justify" }),
                "left, center or right",
            ),
            (
                json!({ "type": "set_table_cell_shading", "table_order_index": 0, "row": 0, "column": 0, "color": "red" }),
                "#RRGGBB",
            ),
            (
                json!({ "type": "set_table_cell_vertical_alignment", "table_order_index": 0, "row": 0, "column": 0, "alignment": "middle" }),
                "top, center or bottom",
            ),
            (
                json!({ "type": "set_table_cell_shading", "table_order_index": 0, "row": 9, "column": 0, "color": "#FFFFFF" }),
                "row 9 is out of range",
            ),
        ] {
            let error = try_tool(
                &mut server,
                "apply_commands",
                json!({ "session_id": session, "commands": [arguments.clone()] }),
            )
            .expect_err(&format!("{arguments} should be rejected"));
            assert!(
                error.contains(expected),
                "expected {expected:?} in {error:?}",
            );
        }

        assert_eq!(table_snapshot(&mut server, &session), before);
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

        /// The rest of the page margins Word puts in the same dialog: the
        /// header and footer distances and the binding gutter. Setting one must
        /// leave the four edges alone — that is the point of the partial form —
        /// and all of it has to survive the hand-off and undo.
        #[test]
        fn header_footer_and_gutter_margins_round_trip_and_undo(
            header in 0u32..80,
            footer in 0u32..80,
            gutter in 0u32..60,
        ) {
            let (header, footer, gutter) =
                (f64::from(header), f64::from(footer), f64::from(gutter));
            let mut server = test_server();
            let session = open_session(&mut server, HELLO_DOCX);
            let before = page_setup(&mut server, &session);

            apply(&mut server, &session, json!([{
                "type": "set_page_margins",
                "header": header, "footer": footer, "gutter": gutter,
            }]));

            let after = page_setup(&mut server, &session);
            prop_assert_eq!(after["header_distance"].as_f64(), Some(header));
            prop_assert_eq!(after["footer_distance"].as_f64(), Some(footer));
            // The four edges were not named, so they must not have moved.
            for edge in ["margin_top", "margin_bottom", "margin_left", "margin_right"] {
                prop_assert_eq!(after[edge].clone(), before[edge].clone(), "{} moved", edge);
            }

            let bytes = export_bytes(&mut server, &session);
            let reopened = reopen(&mut server, &bytes);
            let reread = page_setup(&mut server, &reopened);
            prop_assert_eq!(reread["header_distance"].as_f64(), Some(header));
            prop_assert_eq!(reread["footer_distance"].as_f64(), Some(footer));

            tool_json(
                &server
                    .handle_message(&call(71, "undo", json!({ "session_id": session })))
                    .unwrap(),
            );
            prop_assert_eq!(page_setup(&mut server, &session), before);
        }

        /// Space before and after a paragraph, set without disturbing its line
        /// height — a contract sets the gap between clauses far more often than
        /// it changes leading.
        #[test]
        fn paragraph_space_before_and_after_round_trips(
            before_pt in 0u32..40,
            after_pt in 0u32..40,
            line in prop::sample::select(vec![1.0f64, 1.5, 2.0]),
        ) {
            let (before_pt, after_pt) = (f64::from(before_pt), f64::from(after_pt));
            let mut server = test_server();
            let session = open_session(&mut server, CONTRACT_DOCX);

            // Line height first, then the gaps: the second command must not
            // clear what the first one set.
            apply(&mut server, &session, json!([
                { "type": "set_paragraph_line_spacing", "start_paragraph": 0, "end_paragraph": 0, "line": line },
                { "type": "set_paragraph_line_spacing", "start_paragraph": 0, "end_paragraph": 0, "before": before_pt, "after": after_pt },
            ]));

            let spacing = paragraph_snapshot(&mut server, &session, 0)["spacing"].clone();
            prop_assert_eq!(spacing["before"].as_f64(), Some(before_pt));
            prop_assert_eq!(spacing["after"].as_f64(), Some(after_pt));
            prop_assert_eq!(spacing["line"].as_f64(), Some(line));

            let bytes = export_bytes(&mut server, &session);
            let reopened = reopen(&mut server, &bytes);
            let reread = paragraph_snapshot(&mut server, &reopened, 0)["spacing"].clone();
            prop_assert_eq!(reread["before"].as_f64(), Some(before_pt));
            prop_assert_eq!(reread["after"].as_f64(), Some(after_pt));
            prop_assert_eq!(reread["line"].as_f64(), Some(line));
        }

        /// Cell padding, per cell and as the table default. Setting one edge
        /// leaves the others where they were, and the value comes back out of
        /// the file.
        #[test]
        fn cell_margins_round_trip_per_cell_and_as_the_table_default(
            top in 0u32..20,
            left in 0u32..20,
            whole_table in any::<bool>(),
        ) {
            let (top, left) = (f64::from(top), f64::from(left));
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);

            let mut command = json!({
                "type": "set_table_cell_margins", "table_order_index": 0, "top": top,
            });
            if !whole_table {
                command["row"] = json!(0);
                command["column"] = json!(0);
            }
            apply(&mut server, &session, json!([command.clone()]));
            // A second call naming only `left` must not clear `top`.
            command["top"] = Value::Null;
            command["left"] = json!(left);
            apply(&mut server, &session, json!([command]));

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, "after setting cell margins");
            let xml = part_xml(&bytes, "word/document.xml").unwrap();
            let element = if whole_table { "w:tblCellMar" } else { "w:tcMar" };
            let at = xml.find(element).unwrap_or_else(|| panic!("no {element} in {xml}"));
            let written = &xml[at..(at + 200).min(xml.len())];
            prop_assert!(
                written.contains(&format!("w:w=\"{}\"", (top * 20.0) as i64)),
                "the first edge must survive the second call: {written}",
            );
            prop_assert!(
                written.contains(&format!("w:w=\"{}\"", (left * 20.0) as i64)),
                "the second edge must be written: {written}",
            );

            tool_json(
                &server
                    .handle_message(&call(70, "undo", json!({ "session_id": session, "count": 2 })))
                    .unwrap(),
            );
            prop_assert_eq!(table_snapshot(&mut server, &session), before);
        }
    }

    /// What these three refuse, and that refusing changes nothing.
    #[test]
    fn margin_and_spacing_commands_reject_impossible_values() {
        let mut server = test_server();
        let session = open_session(&mut server, TABLE_DOCX);
        let before = table_snapshot(&mut server, &session);

        for (arguments, expected) in [
            (json!({ "type": "set_page_margins" }), "at least one edge"),
            (
                json!({ "type": "set_paragraph_line_spacing", "start_paragraph": 0, "end_paragraph": 0, "before": -5.0 }),
                "must be a non-negative number",
            ),
            (
                json!({ "type": "set_paragraph_line_spacing", "start_paragraph": 0, "end_paragraph": 0, "line": 1.5, "line_rule": "double" }),
                "auto, exact or atLeast",
            ),
            (
                json!({ "type": "set_table_cell_margins", "table_order_index": 0, "row": 0, "top": 4.0 }),
                "both row and column, or neither",
            ),
            (
                json!({ "type": "set_table_cell_margins", "table_order_index": 0 }),
                "at least one edge",
            ),
            (
                json!({ "type": "set_table_cell_margins", "table_order_index": 0, "row": 0, "column": 0, "left": -1.0 }),
                "finite non-negative",
            ),
        ] {
            let error = try_tool(
                &mut server,
                "apply_commands",
                json!({ "session_id": session, "commands": [arguments.clone()] }),
            )
            .expect_err(&format!("{arguments} should be rejected"));
            assert!(
                error.contains(expected),
                "expected {expected:?} in {error:?}",
            );
        }

        assert_eq!(table_snapshot(&mut server, &session), before);
    }

    // ---------------------------------------------------------------------
    // Table borders and row properties
    //
    // A 別表 is ruled: the frame, the lines between rows, a heavier rule under
    // the header, and — when it runs over a page — a header row that repeats.
    // ---------------------------------------------------------------------

    /// A cell's border style keys, as `(style, width, color)` per edge.
    fn cell_borders(table: &Value, row: usize, column: usize) -> Vec<(String, String, String)> {
        let style = &table["rows"][row]["cells"][column]["style"];
        ["top", "bottom", "left", "right"]
            .into_iter()
            .map(|edge| {
                (
                    style[format!("border-{edge}-style")]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    style[format!("border-{edge}-width")]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    style[format!("border-{edge}-color")]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                )
            })
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

        /// Table borders: what is written comes back out of the file, and the
        /// edges that were not named keep what they had.
        #[test]
        fn table_borders_round_trip_and_leave_unnamed_edges_alone(
            style in prop::sample::select(vec!["single", "double", "dashed", "none"]),
            width in prop::sample::select(vec![0.5f64, 1.0, 2.0]),
            red in 0u8..=255,
        ) {
            let color = format!("#{red:02X}3366");
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);

            apply(&mut server, &session, json!([{
                "type": "set_table_borders", "table_order_index": 0,
                "top": { "style": style, "width": width, "color": color },
                "inside_h": { "style": "dotted", "width": 0.5 },
            }]));

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, "after setting table borders");
            let xml = part_xml(&bytes, "word/document.xml").unwrap();
            let at = xml.find("<w:tblBorders>").expect("tblBorders");
            let written = &xml[at..xml[at..].find("</w:tblBorders>").map(|end| at + end).unwrap_or(xml.len())];
            prop_assert!(
                written.contains(&format!("w:val=\"{style}\"")),
                "the style must be written: {written}",
            );
            prop_assert!(
                written.contains(&format!("w:sz=\"{}\"", (width * 8.0) as i64)),
                "the width must be written in eighths: {written}",
            );
            prop_assert!(
                written.contains(&format!("w:color=\"{}\"", color.trim_start_matches('#'))),
                "the colour must be written: {written}",
            );
            prop_assert!(written.contains("<w:insideH"), "the inside line must be written: {written}");
            // The fixture's own left/right edges were not named, so they stay.
            prop_assert!(written.contains("<w:left"), "an unnamed edge was dropped: {written}");

            // Reopening reads the same table back, and undo puts it back.
            let reopened = reopen(&mut server, &bytes);
            prop_assert!(!table_snapshot(&mut server, &reopened).is_null());
            tool_json(
                &server
                    .handle_message(&call(69, "undo", json!({ "session_id": session })))
                    .unwrap(),
            );
            prop_assert_eq!(table_snapshot(&mut server, &session), before);
        }

        /// A cell's own borders win over the table's, and naming one edge does
        /// not wipe the other three — a total row gets its double rule without
        /// losing the grid around it.
        #[test]
        fn cell_borders_replace_only_the_named_edge(
            style in prop::sample::select(vec!["double", "thick", "dashed"]),
            width in prop::sample::select(vec![0.5f64, 1.5, 3.0]),
            row in 0usize..2,
            column in 0usize..2,
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);
            let untouched: Vec<(String, String, String)> = cell_borders(&before, row, column)
                .into_iter()
                .skip(1) // the top edge is the one being set
                .collect();

            apply(&mut server, &session, json!([{
                "type": "set_table_cell_borders", "table_order_index": 0,
                "row": row, "column": column,
                "top": { "style": style, "width": width },
            }]));

            let after = table_snapshot(&mut server, &session);
            let edges = cell_borders(&after, row, column);
            prop_assert_eq!(&edges[0].0, &style.to_string());
            prop_assert_eq!(&edges[0].1, &format!("{width:.4}pt"));
            prop_assert_eq!(
                edges.into_iter().skip(1).collect::<Vec<_>>(),
                untouched,
                "the other three edges must not move",
            );

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, "after setting cell borders");
            let reopened = reopen(&mut server, &bytes);
            let reread = table_snapshot(&mut server, &reopened);
            prop_assert_eq!(cell_borders(&reread, row, column)[0].0.clone(), style.to_string());

            tool_json(
                &server
                    .handle_message(&call(68, "undo", json!({ "session_id": session })))
                    .unwrap(),
            );
            prop_assert_eq!(table_snapshot(&mut server, &session), before);
        }

        /// Header repetition and no-split, set and cleared, through the file
        /// and back.
        #[test]
        fn row_properties_round_trip(
            row in 0usize..2,
            repeat_header in any::<bool>(),
            cant_split in any::<bool>(),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);

            apply(&mut server, &session, json!([{
                "type": "set_table_row_properties", "table_order_index": 0,
                "row": row, "repeat_header": repeat_header, "cant_split": cant_split,
            }]));

            prop_assert_eq!(
                table_snapshot(&mut server, &session)["rows"][row]["is_header"].as_bool(),
                Some(repeat_header),
            );

            let bytes = export_bytes(&mut server, &session);
            assert_docx_is_consistent(&bytes, "after setting row properties");
            let reopened = reopen(&mut server, &bytes);
            prop_assert_eq!(
                table_snapshot(&mut server, &reopened)["rows"][row]["is_header"].as_bool(),
                Some(repeat_header),
            );

            tool_json(
                &server
                    .handle_message(&call(67, "undo", json!({ "session_id": session })))
                    .unwrap(),
            );
            prop_assert_eq!(table_snapshot(&mut server, &session), before);
        }
    }

    #[test]
    fn table_height_and_margin_reset_commands_preserve_history_and_export() {
        let mut server = test_server();
        let session = open_session(&mut server, TABLE_DOCX);
        let before = table_snapshot(&mut server, &session);
        let original_xml =
            part_xml(&export_bytes(&mut server, &session), "word/document.xml").unwrap();
        apply(
            &mut server,
            &session,
            json!([
                { "type": "set_table_row_height_properties", "table_order_index": 0, "row": 0, "mode": "exact", "height": 24.25 },
                { "type": "reset_table_cell_margins", "table_order_index": 0, "row": 0, "column": 0 }
            ]),
        );
        let bytes = export_bytes(&mut server, &session);
        assert_docx_is_consistent(&bytes, "after height and margin reset");
        let xml = part_xml(&bytes, "word/document.xml").unwrap();
        assert!(xml.contains("w:val=\"485\""));
        assert!(xml.contains("w:hRule=\"exact\""));
        assert_eq!(
            xml.matches("<w:tcMar>").count() + 1,
            original_xml.matches("<w:tcMar>").count()
        );
        let reopened = reopen(&mut server, &bytes);
        assert_eq!(
            table_snapshot(&mut server, &reopened),
            table_snapshot(&mut server, &session)
        );
        tool_json(
            &server
                .handle_message(&call(67, "undo", json!({ "session_id": session })))
                .unwrap(),
        );
        // MCP batches keep one history entry per command. The UI separately
        // groups the selected cells/rows belonging to one user operation.
        let restored_xml =
            part_xml(&export_bytes(&mut server, &session), "word/document.xml").unwrap();
        assert_eq!(
            restored_xml.matches("<w:tcMar>").count(),
            original_xml.matches("<w:tcMar>").count()
        );
        tool_json(
            &server
                .handle_message(&call(68, "undo", json!({ "session_id": session })))
                .unwrap(),
        );
        assert_eq!(table_snapshot(&mut server, &session), before);
    }

    /// What the border commands refuse, and that refusing changes nothing.
    #[test]
    fn border_commands_reject_values_word_cannot_store() {
        let mut server = test_server();
        let session = open_session(&mut server, TABLE_DOCX);
        let before = table_snapshot(&mut server, &session);

        for (arguments, expected) in [
            (
                json!({ "type": "set_table_borders", "table_order_index": 0, "top": { "style": "squiggly" } }),
                "is not one of",
            ),
            (
                json!({ "type": "set_table_borders", "table_order_index": 0, "top": { "width": -1.0 } }),
                "finite non-negative",
            ),
            (
                json!({ "type": "set_table_borders", "table_order_index": 0, "top": { "color": "reddish" } }),
                "#RRGGBB",
            ),
            (
                json!({ "type": "set_table_borders", "table_order_index": 0 }),
                "at least one border edge",
            ),
            (
                json!({ "type": "set_table_cell_borders", "table_order_index": 0, "row": 9, "column": 0, "top": { "style": "single" } }),
                "row 9 is out of range",
            ),
            (
                json!({ "type": "set_table_row_properties", "table_order_index": 0, "row": 0 }),
                "needs repeat_header or cant_split",
            ),
            (
                json!({ "type": "set_table_row_properties", "table_order_index": 0, "row": 9, "repeat_header": true }),
                "row 9 is out of range",
            ),
        ] {
            let error = try_tool(
                &mut server,
                "apply_commands",
                json!({ "session_id": session, "commands": [arguments.clone()] }),
            )
            .expect_err(&format!("{arguments} should be rejected"));
            assert!(
                error.contains(expected),
                "expected {expected:?} in {error:?}",
            );
        }

        assert_eq!(table_snapshot(&mut server, &session), before);
    }

    /// `none` is how a line is removed, and it has to reach the file that way
    /// rather than by leaving the edge out.
    #[test]
    fn a_border_style_of_none_removes_the_line() {
        let mut server = test_server();
        let session = open_session(&mut server, TABLE_DOCX);
        apply(
            &mut server,
            &session,
            json!([{
                "type": "set_table_borders", "table_order_index": 0,
                "inside_h": { "style": "none" }, "inside_v": { "style": "none" },
            }]),
        );

        let bytes = export_bytes(&mut server, &session);
        let xml = part_xml(&bytes, "word/document.xml").unwrap();
        let at = xml.find("<w:tblBorders>").expect("tblBorders");
        let written = &xml[at..at + xml[at..].find("</w:tblBorders>").unwrap()];
        assert!(
            written.contains(r#"<w:insideH w:val="none""#),
            "the inside lines must be switched off explicitly: {written}",
        );
        assert_docx_is_consistent(&bytes, "after removing the inside lines");
    }

    // ---------------------------------------------------------------------
    // Does any sequence of edits still produce a .docx Word can open?
    //
    // Every test above checks one command against what it promises. This one
    // checks the whole surface against what OOXML requires: an agent driving
    // the editor for a while must not be able to reach a document that is
    // structurally wrong, however unlikely the sequence of calls.
    // ---------------------------------------------------------------------

    /// OOXML content-model ordering: inside `parent`, the `properties` element
    /// must come before any of the `content` elements. `w:tr` is the one that
    /// is not simply "first child" — a row exception (`w:tblPrEx`) legitimately
    /// precedes `w:trPr`.
    const PROPERTIES_BEFORE_CONTENT: &[(&str, &str, &[&str])] = &[
        (
            "w:p",
            "w:pPr",
            &[
                "w:r",
                "w:hyperlink",
                "w:ins",
                "w:del",
                "w:commentRangeStart",
            ],
        ),
        (
            "w:r",
            "w:rPr",
            &["w:t", "w:delText", "w:br", "w:tab", "w:drawing"],
        ),
        ("w:tbl", "w:tblPr", &["w:tblGrid", "w:tr"]),
        ("w:tr", "w:trPr", &["w:tc"]),
        ("w:tc", "w:tcPr", &["w:p", "w:tbl"]),
    ];

    /// Walk `xml` and hand every `<name …>` opening tag to `visit`, along with
    /// the tag text and the body up to its matching close. Enough for the
    /// shallow structural questions below, and free of an XML dependency in the
    /// test build.
    fn for_each_element(xml: &str, name: &str, mut visit: impl FnMut(&str, &str)) {
        let open = format!("<{name}");
        let close = format!("</{name}>");
        let mut cursor = 0usize;
        while let Some(found) = xml[cursor..].find(&open) {
            let start = cursor + found;
            let after = &xml[start + open.len()..];
            if !after.starts_with(' ') && !after.starts_with('>') && !after.starts_with('/') {
                cursor = start + open.len();
                continue;
            }
            let Some(tag_end) = xml[start..].find('>').map(|at| start + at + 1) else {
                break;
            };
            if xml[start..tag_end].ends_with("/>") {
                visit(&xml[start..tag_end], "");
                cursor = tag_end;
                continue;
            }
            let body_end = matching_close_start(xml, tag_end, name).unwrap_or_else(|| {
                xml[tag_end..]
                    .find(&close)
                    .map(|at| tag_end + at)
                    .unwrap_or(xml.len())
            });
            visit(&xml[start..tag_end], &xml[tag_end..body_end]);
            cursor = tag_end;
        }
    }

    /// Find the close matching an opening element whose body starts at
    /// `body_start`. Unlike a plain string search, this skips closes belonging
    /// to nested elements with the same name (notably nested `w:tbl`s).
    fn matching_close_start(xml: &str, body_start: usize, name: &str) -> Option<usize> {
        let mut depth = 1usize;
        let mut index = body_start;
        while let Some(relative) = xml[index..].find('<') {
            let start = index + relative;
            let tag_end = xml[start..].find('>').map(|at| start + at + 1)?;
            let raw = xml[start + 1..tag_end - 1].trim();
            let closing = raw.starts_with('/');
            let self_closing = raw.ends_with('/');
            let element_name: String = raw
                .trim_start_matches('/')
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
                .collect();
            if element_name == name {
                if closing {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(start);
                    }
                } else if !self_closing {
                    depth += 1;
                }
            }
            index = tag_end;
        }
        None
    }

    /// Visit direct child elements named `name`. This keeps table structural
    /// checks scoped to one table/row/cell instead of counting a nested table's
    /// grid and rows as part of its parent.
    fn for_each_direct_element(xml: &str, name: &str, mut visit: impl FnMut(&str, &str)) {
        let mut depth = 0usize;
        let mut index = 0usize;
        let mut target: Option<(usize, usize)> = None;
        while let Some(relative) = xml[index..].find('<') {
            let start = index + relative;
            let Some(tag_end) = xml[start..].find('>').map(|at| start + at + 1) else {
                break;
            };
            let raw = xml[start + 1..tag_end - 1].trim();
            if raw.starts_with('?') || raw.starts_with('!') {
                index = tag_end;
                continue;
            }
            let closing = raw.starts_with('/');
            let self_closing = raw.ends_with('/');
            let element_name: String = raw
                .trim_start_matches('/')
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
                .collect();

            if closing {
                depth = depth.saturating_sub(1);
                if depth == 0 && element_name == name {
                    if let Some((open_start, body_start)) = target.take() {
                        visit(&xml[open_start..body_start], &xml[body_start..start]);
                    }
                }
            } else {
                if depth == 0 && element_name == name {
                    if self_closing {
                        visit(&xml[start..tag_end], "");
                    } else {
                        target = Some((start, tag_end));
                    }
                }
                if !self_closing {
                    depth += 1;
                }
            }
            index = tag_end;
        }
    }

    /// Names of the direct child elements of `body`, in order. Depth-aware, so
    /// a nested table's rows do not look like children of the outer one.
    fn direct_children(body: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut depth = 0usize;
        let bytes = body.as_bytes();
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] != b'<' {
                index += 1;
                continue;
            }
            let rest = &body[index + 1..];
            if rest.starts_with('?') || rest.starts_with('!') {
                index += 1;
                continue;
            }
            let Some(tag_len) = rest.find('>') else { break };
            let tag = &rest[..tag_len];
            let closing = tag.starts_with('/');
            let self_closing = tag.ends_with('/');
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
                .collect();
            if closing {
                depth = depth.saturating_sub(1);
            } else {
                if depth == 0 {
                    names.push(name);
                }
                if !self_closing {
                    depth += 1;
                }
            }
            index += 1 + tag_len;
        }
        names
    }

    /// Values of an attribute whose full name is `name`.
    fn attribute_values_exact(xml: &str, name: &str) -> Vec<String> {
        let needle = format!("{name}=\"");
        xml.match_indices(&needle)
            .filter_map(|(index, _)| {
                let rest = &xml[index + needle.len()..];
                rest.find('"').map(|quote| rest[..quote].to_string())
            })
            .collect()
    }

    /// Everything OOXML requires of the package that the editor could get
    /// wrong, checked on the exported bytes.
    ///
    /// This is deliberately about structure rather than about any one command's
    /// promise: it is the net under the fuzz below, and it fails naming the
    /// step that tore it.
    fn assert_docx_is_structurally_valid(docx_bytes: &[u8], label: &str) {
        let document = part_xml(docx_bytes, "word/document.xml")
            .unwrap_or_else(|| panic!("{label}: no word/document.xml"));

        // 1. Properties elements precede the content they describe. Word
        //    refuses a `w:pPr` that comes after a run.
        for (parent, properties, content) in PROPERTIES_BEFORE_CONTENT {
            for_each_element(&document, parent, |_tag, body| {
                let children = direct_children(body);
                let Some(properties_at) = children.iter().position(|name| name == properties)
                else {
                    return;
                };
                for (index, name) in children.iter().enumerate() {
                    if content.contains(&name.as_str()) && index < properties_at {
                        panic!(
                            "{label}: {properties} must precede {name} inside {parent}: {children:?}",
                        );
                    }
                }
            });
        }

        // 2. Every table cell holds at least one paragraph.
        for_each_element(&document, "w:tc", |_tag, body| {
            assert!(
                direct_children(body).iter().any(|name| name == "w:p"),
                "{label}: a table cell has no paragraph, which Word cannot open: {body}",
            );
        });

        // 3. The grid describes the cells: one `w:gridCol` per column the rows
        //    occupy, and every one carries a width.
        for_each_element(&document, "w:tbl", |_tag, table| {
            let mut columns = 0usize;
            for_each_direct_element(table, "w:tblGrid", |_tag, grid| {
                for_each_direct_element(grid, "w:gridCol", |tag, _| {
                    columns += 1;
                    assert!(
                        tag.contains("w:w=\""),
                        "{label}: a grid column has no width: {tag}",
                    );
                });
            });
            let mut widest = 0usize;
            let mut vertical_ranges: Vec<Vec<(usize, usize, String)>> = Vec::new();
            for_each_direct_element(table, "w:tr", |_tag, row| {
                let mut occupied = xml_grid_offset(row, "w:gridBefore");
                let mut row_vertical_ranges = Vec::new();
                for_each_direct_element(row, "w:tc", |_tag, cell| {
                    let mut span = 1usize;
                    let mut vertical_merge = None;
                    for_each_direct_element(cell, "w:tcPr", |_tag, properties| {
                        for_each_element(properties, "w:gridSpan", |tag, _| {
                            span = attribute_values_exact(tag, "w:val")
                                .first()
                                .and_then(|value| value.parse::<usize>().ok())
                                .unwrap_or(1);
                        });
                        for_each_element(properties, "w:vMerge", |tag, _| {
                            vertical_merge = Some(
                                attribute_values_exact(tag, "w:val")
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| "continue".to_string()),
                            );
                        });
                    });
                    let start = occupied;
                    occupied += span.max(1);
                    if let Some(marker) = vertical_merge {
                        row_vertical_ranges.push((start, occupied, marker));
                    }
                });
                occupied += xml_grid_offset(row, "w:gridAfter");
                widest = widest.max(occupied);
                vertical_ranges.push(row_vertical_ranges);
            });
            assert_eq!(
                columns, widest,
                "{label}: the grid has {columns} columns but the widest row occupies {widest}",
            );
            for (row_index, row) in vertical_ranges.iter().enumerate() {
                for (start, end, marker) in row {
                    match marker.as_str() {
                        "restart" => assert!(
                            vertical_ranges.get(row_index + 1).is_some_and(|next| {
                                next.iter().any(|(next_start, next_end, next_marker)| {
                                    next_start == start
                                        && next_end == end
                                        && next_marker == "continue"
                                })
                            }),
                            "{label}: orphan vMerge restart at row {row_index}, columns {start}..{end}",
                        ),
                        "continue" => assert!(
                            row_index.checked_sub(1).is_some_and(|previous| {
                                vertical_ranges[previous].iter().any(
                                    |(previous_start, previous_end, previous_marker)| {
                                        previous_start == start
                                            && previous_end == end
                                            && matches!(
                                                previous_marker.as_str(),
                                                "restart" | "continue"
                                            )
                                    },
                                )
                            }),
                            "{label}: orphan vMerge continuation at row {row_index}, columns {start}..{end}",
                        ),
                        other => panic!("{label}: unknown vMerge marker {other}"),
                    }
                }
            }
        });

        // 4. Tracked changes: deleted text lives in `w:delText` inside `w:del`,
        //    and nothing else does.
        for_each_element(&document, "w:del", |_tag, body| {
            assert!(
                !body.contains("<w:t>") && !body.contains("<w:t "),
                "{label}: a tracked deletion contains live text: {body}",
            );
        });
        let del_text_total = document.matches("<w:delText").count();
        let mut del_text_inside = 0usize;
        for_each_element(&document, "w:del", |_tag, body| {
            del_text_inside += body.matches("<w:delText").count();
        });
        assert_eq!(
            del_text_total, del_text_inside,
            "{label}: delText outside a w:del element",
        );

        // 5. Bookmarks pair up (a TOC insert writes them).
        assert_eq!(
            document.matches("<w:bookmarkStart").count(),
            document.matches("<w:bookmarkEnd").count(),
            "{label}: bookmarkStart and bookmarkEnd do not pair up",
        );

        // 6. Every part in the package has a content type, and every
        //    relationship points at a part that exists.
        let content_types = part_xml(docx_bytes, "[Content_Types].xml").unwrap();
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(docx_bytes)).expect("read the package");
        let names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect();
        for name in &names {
            if name.ends_with('/') || name == "[Content_Types].xml" {
                continue;
            }
            let extension = name.rsplit('.').next().unwrap_or_default();
            let covered = content_types.contains(&format!("PartName=\"/{name}\""))
                || content_types.contains(&format!("Extension=\"{extension}\""));
            assert!(covered, "{label}: {name} has no content type");
        }
        if let Some(rels) = part_xml(docx_bytes, "word/_rels/document.xml.rels") {
            for target in attribute_values_exact(&rels, "Target") {
                if target.starts_with("http") || target.contains("://") {
                    continue;
                }
                // Targets are relative to `word/`, and may climb out of it
                // (`../customXml/item1.xml`).
                let mut segments: Vec<&str> = vec!["word"];
                for segment in target.split('/') {
                    match segment {
                        "." | "" => {}
                        ".." => {
                            segments.pop();
                        }
                        other => segments.push(other),
                    }
                }
                let resolved = segments.join("/");
                assert!(
                    names.iter().any(|name| name == &resolved),
                    "{label}: relationship target {target} is not in the package",
                );
            }
        }

        // 7. …and the whole thing still opens, with its comments intact.
        assert_docx_is_consistent(docx_bytes, label);
    }

    /// One step of an arbitrary editing session, chosen from the whole command
    /// surface and aimed at whatever the document currently holds. Returns
    /// whether it applied and recorded an undo step — a command that succeeds
    /// as a no-op (re-applying a value a property already has) records none.
    fn run_arbitrary_edit(server: &mut Server, session: &str, op: u8, seed: usize) -> bool {
        let paragraphs = tool_json(
            &server
                .handle_message(&call(
                    66,
                    "query",
                    json!({ "session_id": session, "queries": [{ "type": "document_stats" }] }),
                ))
                .unwrap(),
        )["results"][0]["stats"]["paragraph_count"]
            .as_u64()
            .unwrap_or(0) as usize;
        if paragraphs == 0 {
            return false;
        }
        let paragraph = seed % paragraphs;
        let text = paragraph_text(server, session, paragraph);
        let offset = if text.is_empty() {
            0
        } else {
            text.char_indices()
                .map(|(at, _)| at)
                .nth(seed % text.chars().count())
                .unwrap_or(0)
        };

        let command = match op {
            0 => {
                json!({ "type": "insert_text", "paragraph": paragraph, "offset": offset, "text": "甲" })
            }
            1 => {
                if text.is_empty() {
                    return false;
                }
                json!({ "type": "delete_text", "paragraph": paragraph, "offset": offset, "direction": "forward", "count": 1 + seed % 3 })
            }
            2 => {
                json!({ "type": "replace_range", "start_paragraph": paragraph, "start_offset": offset, "end_paragraph": paragraph, "end_offset": text.len(), "text": "X" })
            }
            3 => {
                json!({ "type": "insert_paragraph_break", "paragraph": paragraph, "offset": offset })
            }
            4 => {
                json!({ "type": "set_bold", "start_paragraph": paragraph, "start_offset": 0, "end_paragraph": paragraph, "end_offset": text.len(), "value": seed.is_multiple_of(2) })
            }
            5 => {
                json!({ "type": "set_paragraph_line_spacing", "start_paragraph": paragraph, "end_paragraph": paragraph, "line": 1.5, "before": 6.0 })
            }
            6 => {
                json!({ "type": "insert_table", "after_paragraph": paragraph, "rows": 2, "columns": 2 })
            }
            7 => {
                json!({ "type": "insert_table_row", "table_order_index": 0, "row_index": seed % 3 })
            }
            8 => {
                json!({ "type": "insert_table_column", "table_order_index": 0, "column_index": seed % 3 })
            }
            9 => {
                json!({ "type": "delete_table_row", "table_order_index": 0, "row_index": seed % 3 })
            }
            10 => {
                json!({ "type": "delete_table_column", "table_order_index": 0, "column_index": seed % 3 })
            }
            11 => {
                json!({ "type": "merge_table_cells", "table_order_index": 0, "selected_cells": [{ "row": 0, "column": 0, "column_span": 1 }, { "row": 0, "column": 1, "column_span": 1 }] })
            }
            12 => {
                json!({ "type": "set_table_cell_text", "table_order_index": 0, "row": seed % 2, "column": seed % 2, "text": "セル" })
            }
            13 => {
                json!({ "type": "set_table_borders", "table_order_index": 0, "inside_h": { "style": "dashed", "width": 0.5 } })
            }
            14 => {
                json!({ "type": "set_table_cell_shading", "table_order_index": 0, "row": 0, "column": 0, "color": "#EEEEEE" })
            }
            15 => {
                json!({ "type": "set_table_row_properties", "table_order_index": 0, "row": 0, "repeat_header": true })
            }
            16 => {
                json!({ "type": "add_comment", "start_paragraph": paragraph, "end_paragraph": paragraph, "text": "確認", "author": "法務" })
            }
            17 => json!({ "type": "start_change_tracking", "author": "AI" }),
            18 => json!({ "type": "accept_revisions" }),
            19 => json!({ "type": "reject_revisions" }),
            20 => json!({ "type": "set_page_margins", "top": 40.0, "gutter": 20.0 }),
            21 => {
                json!({ "type": "insert_table_of_contents", "after_paragraph": paragraph, "outline_min": 1, "outline_max": 3 })
            }
            22 => {
                json!({ "type": "set_paragraph_style", "start_paragraph": paragraph, "end_paragraph": paragraph, "style_id": "Heading1" })
            }
            23 => json!({ "type": "delete_table", "table_order_index": 0 }),
            24 => {
                json!({ "type": "set_numbering", "start_paragraph": paragraph, "end_paragraph": paragraph, "numbering_id": 1, "level": 0 })
            }
            25 => {
                let table = table_snapshot(server, session);
                let Some(columns) = table["columns"].as_array() else {
                    return false;
                };
                if columns.is_empty() {
                    return false;
                }
                let widths: Vec<f64> = (0..columns.len())
                    .map(|column| 9.0 + ((seed + column * 17) % 80) as f64)
                    .collect();
                json!({ "type": "set_table_column_widths", "table_order_index": 0, "widths": widths })
            }
            26 => {
                let table = table_snapshot(server, session);
                let columns = table["columns"].as_array().map_or(0, Vec::len);
                if columns < 2 {
                    return false;
                }
                let left = seed % (columns - 1);
                json!({
                    "type": "resize_table_column_between",
                    "table_order_index": 0,
                    "left_column_index": left,
                    "new_left_width": 9 + seed % 80,
                    "new_right_width": 9 + (seed * 7) % 80
                })
            }
            27 => {
                let table = table_snapshot(server, session);
                let columns = table["columns"].as_array().map_or(0, Vec::len);
                if columns == 0 {
                    return false;
                }
                json!({
                    "type": "resize_table_column_right_edge",
                    "table_order_index": 0,
                    "column_index": seed % columns,
                    "new_width": 9 + seed % 80
                })
            }
            28 => {
                json!({
                    "type": "unmerge_table_cells",
                    "table_order_index": 0,
                    "target_cells": [{ "row": seed % 3, "column": seed % 3, "column_span": 1 }]
                })
            }
            _ => {
                let table = table_snapshot(server, session);
                let rows = table["rows"].as_array().map_or(0, Vec::len);
                let columns = table["columns"].as_array().map_or(0, Vec::len);
                if rows < 2 || columns == 0 {
                    return false;
                }
                json!({
                    "type": "merge_table_cells",
                    "table_order_index": 0,
                    "selected_cells": [
                        { "row": 0, "column": 0, "column_span": 1 },
                        { "row": 1, "column": 0, "column_span": 1 }
                    ]
                })
            }
        };
        match try_tool(
            server,
            "apply_commands",
            json!({ "session_id": session, "commands": [command] }),
        ) {
            // Count only edits that actually became an undo step; a no-op
            // success (e.g. re-applying identical line spacing) records none.
            Ok(response) => response["recorded_undo_steps"].as_u64().unwrap_or(1) > 0,
            Err(_) => false,
        }
    }

    /// Everything a reader would notice about a document: the text of every
    /// paragraph, its comments, its tracked changes and the shape of its
    /// tables.
    fn document_state(server: &mut Server, session: &str) -> Value {
        let response = server
            .handle_message(&call(
                64,
                "query",
                json!({
                    "session_id": session,
                    "queries": [
                        { "type": "paragraph_slice", "start_paragraph": 0, "max_paragraphs": 500 },
                        { "type": "comments", "start_comment": 0, "max_comments": 100 },
                        { "type": "revisions", "start_revision": 0, "max_revisions": 100 },
                        { "type": "document_stats" },
                        { "type": "table", "table_order_index": 0 },
                    ],
                }),
            ))
            .unwrap();
        tool_json(&response)["results"].clone()
    }

    #[test]
    fn undo_merge_survives_a_later_table_xml_rewrite() {
        let mut server = test_server();
        let session = open_session(&mut server, CONTRACT_DOCX);
        let original = document_state(&mut server, &session);

        for op in [6, 11, 7] {
            assert!(
                run_arbitrary_edit(&mut server, &session, op, 0),
                "operation {op} must apply"
            );
        }

        let undone = try_tool(
            &mut server,
            "undo",
            json!({ "session_id": session, "count": 3 }),
        )
        .expect("all three table commands undo");
        assert_eq!(undone["steps"], 3);
        assert_eq!(document_state(&mut server, &session), original);
    }

    /// Enter at the very start of a commented paragraph, then undo.
    ///
    /// The undo used to rebuild the merged paragraph from its runs alone,
    /// dropping the comment's range markers that the split had moved to the
    /// new paragraph and keeping the empty run the split left behind. The
    /// comment lost its anchor, and add_comment's own undo then failed on the
    /// markup it no longer recognised.
    #[test]
    fn undoing_a_break_at_the_start_of_a_commented_paragraph_keeps_the_anchor() {
        let mut server = test_server();
        let session = open_session(&mut server, CONTRACT_DOCX);
        let original = document_state(&mut server, &session);

        assert!(
            run_arbitrary_edit(&mut server, &session, 16, 0),
            "add_comment applies"
        );
        let commented = export_bytes(&mut server, &session);
        assert!(
            run_arbitrary_edit(&mut server, &session, 3, 0),
            "insert_paragraph_break applies"
        );

        let undone = try_tool(
            &mut server,
            "undo",
            json!({ "session_id": session, "count": 1 }),
        )
        .expect("the paragraph break undoes");
        assert_eq!(undone["steps"], 1);
        assert_eq!(
            part_xml(&export_bytes(&mut server, &session), "word/document.xml"),
            part_xml(&commented, "word/document.xml"),
            "undoing the break must restore the commented paragraph's markup exactly",
        );

        let undone = try_tool(
            &mut server,
            "undo",
            json!({ "session_id": session, "count": 1000 }),
        )
        .expect("the comment undoes");
        assert_eq!(undone["steps"], 1);
        assert_eq!(document_state(&mut server, &session), original);
    }

    #[test]
    fn undo_unmerge_survives_a_later_table_xml_rewrite() {
        let mut server = test_server();
        let session = open_session(&mut server, TABLE_DOCX);
        let original = document_state(&mut server, &session);

        for command in [
            json!({
                "type": "merge_table_cells",
                "table_order_index": 0,
                "selected_cells": [
                    { "row": 0, "column": 0, "column_span": 1 },
                    { "row": 0, "column": 1, "column_span": 1 }
                ]
            }),
            json!({
                "type": "unmerge_table_cells",
                "table_order_index": 0,
                "target_cells": [{ "row": 0, "column": 0, "column_span": 2 }]
            }),
            json!({
                "type": "insert_table_row",
                "table_order_index": 0,
                "row_index": 0
            }),
        ] {
            try_tool(
                &mut server,
                "apply_commands",
                json!({ "session_id": session, "commands": [command] }),
            )
            .expect("table command applies");
        }

        let undone = try_tool(
            &mut server,
            "undo",
            json!({ "session_id": session, "count": 3 }),
        )
        .expect("all three table commands undo");
        assert_eq!(undone["steps"], 3);
        assert_eq!(document_state(&mut server, &session), original);
    }

    #[test]
    fn structural_validation_scopes_nested_table_grids_to_each_table() {
        let mut server = test_server();
        let session = open_session(&mut server, TABLE_DOCX);

        assert!(run_arbitrary_edit(&mut server, &session, 6, 0));
        let bytes = export_bytes(&mut server, &session);
        let document = part_xml(&bytes, "word/document.xml").expect("document XML");
        assert_eq!(document.matches("<w:tbl>").count(), 2);
        assert_docx_is_structurally_valid(&bytes, "nested table regression");
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

        /// Drive the editor with an arbitrary run of commands from the whole
        /// surface and check the package after every single one. Whatever an
        /// agent does — and it will do things no test author would think of —
        /// the file it hands back has to be one Word opens.
        #[test]
        fn any_sequence_of_edits_leaves_an_openable_document(
            ops in prop::collection::vec((0u8..30, 0usize..5), 1..12),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);

            for (step, (op, seed)) in ops.iter().enumerate() {
                if !run_arbitrary_edit(&mut server, &session, *op, *seed) {
                    continue;
                }
                assert_table_vertical_merges_are_valid(&table_snapshot(&mut server, &session));
                let bytes = export_bytes(&mut server, &session);
                assert_docx_is_structurally_valid(&bytes, &format!("step {step} (op {op})"));
            }
        }

        /// The same run on a contract, and then all the way back: undoing every
        /// step has to restore the document the session opened — every
        /// paragraph's text, every comment, every tracked change, the tables.
        ///
        /// This compares the state a reader sees rather than the bytes: a text
        /// edit rewrites the run it touches (materialising the fonts the style
        /// used to supply), so its undo is faithful to the document rather than
        /// to the original markup. The structural commands, which undo by
        /// restoring a package snapshot, are held to the stricter bar below.
        #[test]
        fn undoing_an_arbitrary_session_restores_the_document(
            ops in prop::collection::vec((0u8..30, 0usize..5), 1..10),
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, CONTRACT_DOCX);
            let original = document_state(&mut server, &session);

            let mut applied = 0usize;
            for (op, seed) in &ops {
                if run_arbitrary_edit(&mut server, &session, *op, *seed) {
                    applied += 1;
                }
            }

            let undone = tool_json(
                &server
                    .handle_message(&call(65, "undo", json!({ "session_id": session, "count": 1000 })))
                    .unwrap(),
            );
            prop_assert_eq!(
                undone["steps"].as_u64(),
                Some(applied as u64),
                "every applied command must be one undo step",
            );
            prop_assert_eq!(document_state(&mut server, &session), original);
            assert_docx_is_structurally_valid(
                &export_bytes(&mut server, &session),
                "after undoing everything",
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

        /// The commands that undo by restoring a package snapshot are held to
        /// the strict bar: after undo the exported markup is byte for byte what
        /// it was, comments part included.
        ///
        /// Deleting a table used to bring only half of a comment's anchor back,
        /// which is the kind of drift this catches.
        #[test]
        fn undoing_a_structural_command_restores_the_markup_exactly(
            op in prop::sample::select(vec![9u8, 10, 23]),
            seed in 0usize..3,
        ) {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            // A comment inside the table, which is what makes the restore hard.
            try_tool(&mut server, "add_comment", json!({
                "session_id": session, "start_paragraph": 0, "end_paragraph": 0,
                "text": "確認", "author": "法務",
            }))
            .expect("comment");
            let original = export_bytes(&mut server, &session);

            prop_assume!(run_arbitrary_edit(&mut server, &session, op, seed));
            assert_docx_is_structurally_valid(
                &export_bytes(&mut server, &session),
                "after a structural command",
            );

            tool_json(
                &server
                    .handle_message(&call(63, "undo", json!({ "session_id": session })))
                    .unwrap(),
            );
            let restored = export_bytes(&mut server, &session);
            for part in ["word/document.xml", "word/comments.xml"] {
                prop_assert_eq!(
                    part_xml(&restored, part),
                    part_xml(&original, part),
                    "undo must restore {} exactly", part,
                );
            }
        }
    }

    /// Backspace at the start of a cell's first paragraph, and Delete at the
    /// end of its last, do nothing — as they do in Word.
    ///
    /// They used to merge the paragraph with its neighbour across the cell
    /// boundary, leaving a `w:tc` with no `w:p` at all: a table Word refuses to
    /// open. table2x2's cells are paragraphs 0..=3.
    #[test]
    fn deleting_at_a_cell_boundary_does_not_empty_the_cell() {
        for (direction, paragraph) in [("forward", 0usize), ("backward", 1)] {
            let mut server = test_server();
            let session = open_session(&mut server, TABLE_DOCX);
            let before = table_snapshot(&mut server, &session);
            let text = paragraph_text(&mut server, &session, paragraph);
            let offset = if direction == "forward" {
                text.len()
            } else {
                0
            };

            try_tool(
                &mut server,
                "apply_commands",
                json!({
                    "session_id": session,
                    "commands": [{
                        "type": "delete_text", "paragraph": paragraph,
                        "offset": offset, "direction": direction, "count": 3,
                    }],
                }),
            )
            .unwrap_or_else(|err| panic!("{direction} delete at a cell edge failed: {err}"));

            assert_eq!(
                table_snapshot(&mut server, &session),
                before,
                "{direction} delete at a cell edge must leave the table alone",
            );
            assert_docx_is_structurally_valid(
                &export_bytes(&mut server, &session),
                &format!("after a {direction} delete at a cell edge"),
            );
        }
    }
}
