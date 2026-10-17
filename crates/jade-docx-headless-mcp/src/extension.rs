//! Tools injected into the server from above.
//!
//! The base server here knows about DOCX: paragraphs, runs, tables, tracked
//! changes, comments. It deliberately does not know about *your* documents —
//! that a `第５条` is an article, that a clause is missing its 見出し, that this
//! draft violates your house style. That knowledge belongs to whoever owns the
//! documents, changes with their workflow, and has no business being versioned
//! in this repository.
//!
//! So a consumer builds their own binary: depend on this crate, register the
//! tools their domain needs, and serve. The injected tools sit beside the
//! built-in ones — same `tools/list`, same schema quality, same session, same
//! undo history — and the client cannot tell which is which.
//!
//! ```no_run
//! use jade_docx_headless_mcp::extension::{ToolArg, ToolSpec};
//! use jade_docx_headless_mcp::{serve_stdio, server::Server};
//! use serde_json::json;
//!
//! // `with_server_info` makes the client show your name, not this crate's.
//! let mut server = Server::new().with_server_info("count-mcp", "Counting MCP");
//! server
//!     .register_tool(
//!         ToolSpec::new("count_articles", "Count articles", "How many 第N条 this draft has.")
//!             .read_only(),
//!         |ctx, _args| {
//!             let paragraphs = ctx.document().paragraph_count();
//!             Ok(json!({ "paragraphs": paragraphs }))
//!         },
//!     )
//!     .expect("registration");
//! serve_stdio(&mut server);
//! ```

use std::collections::HashMap;
use std::fmt;

use jade_docx_document_core::commands::CommandExecutor;
use jade_docx_document_core::headless_api::{
    apply_headless_commands_with_executor_to_document, execute_headless_queries_on_document,
};
use jade_docx_document_core::word_document::WordDocument;

use crate::server::Session;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessEditRequest, HeadlessQuery, HeadlessQueryRequest,
};
use jade_selection::SelectionRange;
use serde_json::{json, Value};

/// What a registered tool receives: the session its `session_id` names, opened
/// and resolved by the server before the handler runs.
///
/// The document is handed over mutably and the history comes with it, so an
/// injected tool can do anything a built-in one can — and its edits step back
/// under `undo` exactly the same way, provided they go through [`Self::apply`].
pub struct ToolContext<'a> {
    pub(crate) document: &'a mut WordDocument,
    pub(crate) selection: &'a mut Option<SelectionRange>,
    pub(crate) executor: &'a mut CommandExecutor,
}

impl ToolContext<'_> {
    /// The open document, for reading.
    pub fn document(&self) -> &WordDocument {
        self.document
    }

    /// The open document, for edits this crate has no command for.
    ///
    /// Edits made straight through the model are invisible to `undo`. Prefer
    /// [`Self::apply`] whenever a [`HeadlessCommand`] can express the change.
    pub fn document_mut(&mut self) -> &mut WordDocument {
        self.document
    }

    /// Runs queries against the session document — the same `HeadlessQuery`
    /// values the built-in `query` tool takes.
    pub fn query(&self, queries: Vec<HeadlessQuery>) -> Result<Value, String> {
        let response =
            execute_headless_queries_on_document(self.document, &HeadlessQueryRequest { queries })
                .map_err(|err| format!("query failed: {err}"))?;
        serde_json::to_value(response).map_err(|err| err.to_string())
    }

    /// Applies commands through the session's history, so an injected tool's
    /// edits are undoable and land on the same stack as everything else.
    ///
    /// One command is one undo step, matching `apply_commands`.
    pub fn apply(&mut self, commands: Vec<HeadlessCommand>) -> Result<(), String> {
        *self.selection = apply_headless_commands_with_executor_to_document(
            self.document,
            self.executor,
            self.selection.take(),
            &HeadlessEditRequest { commands },
        )
        .map_err(|err| format!("apply failed: {err}"))?;
        Ok(())
    }

    /// Current selection, as the last command left it.
    pub fn selection(&self) -> Option<SelectionRange> {
        *self.selection
    }

    pub fn can_undo(&self) -> bool {
        self.executor.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.executor.can_redo()
    }
}

/// One argument of an injected tool, or one field of its result.
///
/// The description is not optional: a client picks arguments by reading them,
/// and the server refuses to publish a tool whose arguments say nothing. It is
/// the same bar the built-in catalogue holds itself to.
#[derive(Debug, Clone)]
pub struct ToolArg {
    name: String,
    description: String,
    schema: Value,
    required: bool,
}

impl ToolArg {
    /// An argument with a hand-written JSON Schema, for shapes the helpers
    /// below do not cover.
    pub fn new(name: impl Into<String>, description: impl Into<String>, schema: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            schema,
            required: true,
        }
    }

    pub fn text(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, description, json!({ "type": "string" }))
    }

    /// A non-negative integer: an index, an offset or a count.
    pub fn index(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(
            name,
            description,
            json!({ "type": "integer", "minimum": 0 }),
        )
    }

    pub fn number(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, description, json!({ "type": "number" }))
    }

    pub fn boolean(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, description, json!({ "type": "boolean" }))
    }

    /// A string restricted to a fixed set of values.
    pub fn choice(
        name: impl Into<String>,
        description: impl Into<String>,
        values: &[&str],
    ) -> Self {
        Self::new(
            name,
            description,
            json!({ "type": "string", "enum": values }),
        )
    }

    pub fn array_of(name: impl Into<String>, description: impl Into<String>, items: Value) -> Self {
        Self::new(
            name,
            description,
            json!({ "type": "array", "items": items }),
        )
    }

    pub fn object(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(name, description, json!({ "type": "object" }))
    }

    /// Marks the argument optional; arguments are required by default.
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn property_schema(&self) -> Value {
        let mut schema = self.schema.clone();
        schema["description"] = json!(self.description);
        schema
    }

    pub(crate) fn is_required(&self) -> bool {
        self.required
    }
}

/// Behavioural hints published as the tool's MCP `annotations`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ToolHints {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

/// Everything a client is told about an injected tool.
///
/// `session_id` is not declared here: every registered tool is session-scoped,
/// and the server adds the argument, resolves it, and reports an unknown id the
/// same way it does for its own tools.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) arguments: Vec<ToolArg>,
    pub(crate) output: Vec<ToolArg>,
    pub(crate) hints: ToolHints,
    /// Whether the server adds and resolves `session_id` for this tool.
    /// False for the tools that open a session rather than receive one.
    pub(crate) session_scoped: bool,
}

impl ToolSpec {
    /// A tool named `name`, shown as `title`, doing what `description` says.
    ///
    /// The description is what the model reads to decide whether to call this
    /// instead of something else — write it for that reader.
    pub fn new(
        name: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            description: description.into(),
            arguments: Vec::new(),
            output: Vec::new(),
            hints: ToolHints::default(),
            session_scoped: true,
        }
    }

    /// Adds an argument. Order is the order clients display them in.
    pub fn argument(mut self, argument: ToolArg) -> Self {
        self.arguments.push(argument);
        self
    }

    /// Declares a field of the object the tool returns. Publishing the result
    /// shape lets a client parse it instead of guessing at prose.
    pub fn result(mut self, field: ToolArg) -> Self {
        self.output.push(field);
        self
    }

    /// The tool only reads the document.
    pub fn read_only(mut self) -> Self {
        self.hints.read_only = true;
        self
    }

    /// The tool can remove or overwrite content.
    pub fn destructive(mut self) -> Self {
        self.hints.destructive = true;
        self
    }

    /// Calling it twice with the same arguments leaves the same state.
    pub fn idempotent(mut self) -> Self {
        self.hints.idempotent = true;
        self
    }

    /// The tool touches something outside this process — the filesystem, a
    /// network service.
    pub fn open_world(mut self) -> Self {
        self.hints.open_world = true;
        self
    }
}

/// The function a registered tool runs: the resolved session, the arguments as
/// the client sent them, and a JSON object back (or an error message, which the
/// client sees as a failed tool call rather than a protocol error).
pub type ToolHandler =
    dyn Fn(&mut ToolContext<'_>, &Value) -> Result<Value, String> + Send + Sync + 'static;

/// The function a registered *session-opening* tool runs.
///
/// Some tools have no session to be given: they are the ones that produce one.
/// A consumer that fetches a document from somewhere this crate knows nothing
/// about — an internal service, a signed URL — needs to hand the loaded
/// document back and get a `session_id` the rest of the tools accept.
pub type SessionToolHandler =
    dyn Fn(&mut Sessions<'_>, &Value) -> Result<Value, String> + Send + Sync + 'static;

/// The server's open documents, handed to a session-opening tool.
///
/// This is deliberately small: opening, reaching an already-open session, and
/// closing. Everything else a tool needs is on [`ToolContext`].
pub struct Sessions<'a> {
    pub(crate) sessions: &'a mut HashMap<String, Session>,
    pub(crate) next_id: &'a mut u64,
}

impl Sessions<'_> {
    /// Registers a loaded document as a new session and returns its id.
    ///
    /// The id is minted by the server, in the same sequence `open_docx` uses,
    /// so a client cannot tell which tool opened a document.
    pub fn open(&mut self, document: WordDocument) -> String {
        *self.next_id += 1;
        let id = format!("doc-{}", self.next_id);
        self.sessions.insert(
            id.clone(),
            Session {
                doc: document,
                executor: CommandExecutor::new(),
                selection: None,
            },
        );
        id
    }

    /// Borrows an open session, so a session-opening tool can read or edit the
    /// document it just produced without a second round trip.
    pub fn get(&mut self, session_id: &str) -> Option<ToolContext<'_>> {
        let session = self.sessions.get_mut(session_id)?;
        Some(ToolContext {
            document: &mut session.doc,
            selection: &mut session.selection,
            executor: &mut session.executor,
        })
    }

    /// Drops a session. `false` when no session had that id.
    pub fn close(&mut self, session_id: &str) -> bool {
        self.sessions.remove(session_id).is_some()
    }

    /// Ids of every open session, sorted.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.sessions.keys().cloned().collect();
        ids.sort();
        ids
    }
}

/// Why a tool could not be registered.
///
/// Registration fails loudly instead of publishing a tool no client can use:
/// these are all mistakes in the consumer's own code, caught at start-up rather
/// than in the middle of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// A tool with this name already exists — built-in or previously injected.
    DuplicateName(String),
    /// A name, title or description was blank.
    Missing { tool: String, field: &'static str },
    /// An argument or result field had no description.
    UndocumentedField { tool: String, field: String },
    /// The tool declared `session_id` itself; the server owns that argument.
    ReservedArgument { tool: String },
}

impl fmt::Display for RegisterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegisterError::DuplicateName(name) => {
                write!(formatter, "a tool named `{name}` is already registered")
            }
            RegisterError::Missing { tool, field } => {
                write!(formatter, "tool `{tool}` has an empty {field}")
            }
            RegisterError::UndocumentedField { tool, field } => write!(
                formatter,
                "tool `{tool}`: `{field}` has no description, so a client cannot tell what to pass",
            ),
            RegisterError::ReservedArgument { tool } => write!(
                formatter,
                "tool `{tool}` must not declare `session_id`: the server adds and resolves it",
            ),
        }
    }
}

impl std::error::Error for RegisterError {}

/// A registered tool: its published spec and the function behind it.
pub(crate) struct RegisteredTool {
    pub spec: ToolSpec,
    pub handler: RegisteredHandler,
}

/// Which shape a registered tool's function takes.
pub(crate) enum RegisteredHandler {
    /// Runs against the session named by `session_id`, resolved by the server.
    Session(std::sync::Arc<ToolHandler>),
    /// Runs against the session store, because it is the tool that opens one.
    Store(std::sync::Arc<SessionToolHandler>),
}

impl ToolSpec {
    /// Checks everything that would otherwise surface as a tool the model
    /// cannot use.
    pub(crate) fn validate(&self) -> Result<(), RegisterError> {
        for (value, field) in [
            (&self.name, "name"),
            (&self.title, "title"),
            (&self.description, "description"),
        ] {
            if value.trim().is_empty() {
                return Err(RegisterError::Missing {
                    tool: self.name.clone(),
                    field,
                });
            }
        }
        for argument in self.arguments.iter().chain(&self.output) {
            if argument.name.trim().is_empty() || argument.description.trim().is_empty() {
                return Err(RegisterError::UndocumentedField {
                    tool: self.name.clone(),
                    field: argument.name.clone(),
                });
            }
        }
        if self.session_scoped
            && self
                .arguments
                .iter()
                .any(|argument| argument.name == "session_id")
        {
            return Err(RegisterError::ReservedArgument {
                tool: self.name.clone(),
            });
        }
        Ok(())
    }
}
