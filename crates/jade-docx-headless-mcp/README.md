# jade-docx-headless-mcp

A [Model Context Protocol](https://modelcontextprotocol.io) (MCP) server that lets
an MCP client — an AI agent such as Claude, or any MCP host — **edit `.docx`
files through the headless DOCX editor**.

It speaks JSON-RPC 2.0 over stdio (newline-delimited) and wraps the native
`jade-docx-headless-editor` / jade core. Documents are held **in memory by
session**, so an agent opens a file once and then issues many edit/query calls
without re-parsing and re-zipping on every operation — the fast path for
iterative editing.

## Build & run

```bash
cargo build -p jade-docx-headless-mcp --release
# binary: target/release/jade-docx-headless-mcp
```

Register it with an MCP host. For Claude Code:

```bash
claude mcp add jade-headless -- /absolute/path/to/target/release/jade-docx-headless-mcp
```

Or in a `mcpServers` config block:

```json
{
  "mcpServers": {
    "jade-headless": {
      "command": "/absolute/path/to/target/release/jade-docx-headless-mcp"
    }
  }
}
```

## File access

`open_docx` and `export_docx` take paths from tool arguments, which a model
writes while reading untrusted documents. A planted instruction must not be
able to read or overwrite arbitrary files, so the server confines them:

- Paths must resolve — after symlinks — inside an allowed directory. Relative
  paths are resolved against the first one.
- Only `.docx` files are read or written.
- `export_docx` with `path` only creates a new file (`O_CREAT | O_EXCL`): it
  never overwrites and never writes through a symlink. Omit `path` to get the
  bytes back instead.

The allowed directories come from `JADE_MCP_ALLOWED_ROOTS` (separated like
`PATH`). Unset, the server's working directory is allowed — MCP hosts such as
Claude Code start stdio servers in the opened project.

```json
{
  "mcpServers": {
    "jade-headless": {
      "command": "/absolute/path/to/target/release/jade-docx-headless-mcp",
      "env": { "JADE_MCP_ALLOWED_ROOTS": "/Users/me/contracts" }
    }
  }
}
```

An embedding binary sets its own with
`Server::new().with_file_access(FileAccess::new([dir]))`.

Hosts should also keep `export_docx` (and any embedder tool that sends a file
off the machine) behind a user confirmation: the tools publish
`destructiveHint` / `openWorldHint`, so do not add them to an auto-approve list.

## Tools

| Tool | Purpose |
| --- | --- |
| `open_docx` | Load a `.docx` (`path` or `bytes_base64`) into a session → returns `session_id` + stats. |
| `apply_commands` | Apply a batch of `HeadlessCommand`s to a session. Set `dry_run: true` to validate and inspect simulated query/diff results without mutating the session. Layout lives here too: page margins and size, paragraph indent / line spacing / alignment / style, and table geometry (`set_table_width`, `set_table_column_widths`, `distribute_table_rows`, `distribute_table_columns`, `split_table`, `split_table_cell`, row and column insert/delete, merge/unmerge), appearance (`set_table_alignment`, `set_table_cell_shading`, `set_table_cell_vertical_alignment`, `set_table_borders`, `set_table_cell_borders`, `set_table_cell_margins`) and page-break behaviour (`set_table_row_properties`). |
| `replace_text` | Find-and-replace, literal or `regex` with `$1` captures (handles matches spanning runs; `case_insensitive` for drifting terminology; `dry_run` to preview; scope/cap via `start_paragraph`/`end_paragraph`/`max_replacements`). Returns the matches it touched. |
| `start_change_tracking` / `stop_change_tracking` | Toggle Word change tracking so the agent's edits become tracked changes. |
| `accept_revisions` / `reject_revisions` | Accept/reject tracked changes — all of them, scoped by range/kind, or a single `revision_id` from the `revisions` query. |
| `add_comment` | Attach a Word comment to a paragraph range; offsets are optional (omit them to comment on whole paragraphs). |
| `comment_on_text` | Attach a comment to a phrase, located by search — no offsets to compute (`occurrence` picks a later match). |
| `get_paragraph_by_para_id` | Resolve a paragraph by its stable `w14:paraId` (`getElementById`-style) → current index + snapshot. |
| `comment_by_para_id` | Attach a comment to a range within the paragraph identified by its `w14:paraId` (robust to index shifts). |
| `reply_to_comment` | Answer a comment in its own Word thread (inherits the parent's anchor; replying to a reply joins the same thread). |
| `resolve_comment` | Mark a comment resolved, or reopen it — written to `commentsExtended.xml`, where Word reads it. |
| `update_comment` / `delete_comment` | Rewrite a comment's body, or remove it together with its anchor, thread entry and replies. |
| `undo` / `redo` | Step the session's command history backward / forward; optional `count` steps at once (returns `steps` actually taken). Comment and revision operations are undoable like any other edit. |
| `query` | Inspect a session with `HeadlessQuery`s (no mutation): text, outline, tables, revisions, comments, `styles` / `page_setup` / `header_footer_text` / `document_properties`, plus `chunks` and `references` for article-level structure. |
| `export_docx` | Serialize a session back to `.docx` (to a `path`, or as `bytes_base64`). |
| `close_docx` | Drop a session. |
| `list_sessions` | List open session ids. |

`apply_commands.commands` and `query.queries` take the **same JSON shapes** as
the `HeadlessCommand` / `HeadlessQuery` enums in `jade-docx-headless-api`: each object
is tagged by a snake_case `type` field.

`apply_commands` also accepts optional `queries` and `include_diff`. With
`dry_run: true`, commands are applied to a cloned document, queries run against
that simulated state, and the live session + undo history remain unchanged.

Coordinate interpretation is explicit per batch:

- `coordinates: "sequential"` is the default and preserves the original
  constructive behavior. Each command addresses the document left by the
  command before it, so callers may intentionally create a paragraph and then
  edit that new paragraph later in the same batch.
- `coordinates: "snapshot"` means every position came from the document as it
  stood at batch start. Jade rebases non-overlapping `insert_text`, `delete_text`,
  `replace_range`, and `insert_paragraph_break` coordinates, plus text-range
  formatting (`apply_*`, `set_bold`, font/color/highlight setters, and
  `set_character_style`), through earlier edits while preserving command and undo
  order. Formatting ranges may overlap each other. Ambiguous overlaps between
  text mutations and formatting, two insertions at the same point, a command
  aimed inside a paragraph another command removes, and unsupported paragraph,
  table, search-based, or review command types are rejected before the session
  changes.

## What the client is told (discovery)

A host only ever sees what `initialize` and `tools/list` publish, so the whole
interface is published there — nothing has to be learned from this README:

- **`initialize`** returns server `instructions`: the session lifecycle, the
  index/offset/unit conventions below, and the error convention.
- **Each tool** carries a `title`, a description, and `annotations`
  (`readOnlyHint` / `destructiveHint` / `idempotentHint` / `openWorldHint`), so a
  host can tell `query` (read-only) from `close_docx` (destructive) without
  reading prose.
- **`inputSchema`** describes every argument with a type and a description, and
  is closed (`additionalProperties: false`). `apply_commands.commands` and
  `query.queries` are **not** opaque objects: their `items` is a `oneOf` over
  every `HeadlessCommand` / `HeadlessQuery` variant, each with a `const` `type`
  discriminator, its own required-field list and per-field descriptions.
- **`outputSchema`** describes the result of every tool, and successful calls
  return that object as `structuredContent` next to the usual text block.

Conventions that hold everywhere (also carried in the server instructions):

- `paragraph` is a zero-based index into the flattened paragraph order
  (paragraphs inside tables included); `offset` is a zero-based UTF-8 **byte**
  offset within a paragraph, and `end_offset` is exclusive. An offset past the
  end of its paragraph, or one that falls inside a character, is rejected rather
  than moved to the nearest legal position — take offsets from a `search_text`
  or `paragraph_slice` result instead of counting characters.
- `table_order_index` counts top-level tables in document order.
- Measurements are in points; colors are `#RRGGBB`.
- Tool failures are results with `isError: true` and a text message; JSON-RPC
  errors are reserved for protocol problems.

`catalogue_describes_every_headless_command_and_query` derives the variant list
from the `jade-docx-headless-api` source, so a command added to the API without a
published schema fails the tests; `every_tool_publishes_a_documented_interface`
asserts every tool and every schema property carries a description.

### Example flow

```jsonc
// 1. open
{"name":"open_docx","arguments":{"path":"/tmp/contract.docx"}}
// → {"session_id":"doc-1","stats":{...}}

// 2. edit (insert text, bump a font size, add a tracked comment …)
{"name":"apply_commands","arguments":{
  "session_id":"doc-1",
  "commands":[
    {"type":"insert_text","paragraph":0,"offset":0,"text":"DRAFT — "},
    {"type":"set_font_size","start_paragraph":0,"start_offset":0,"end_paragraph":0,"end_offset":8,"font_size":18},
    {"type":"add_comment","start_paragraph":0,"start_offset":0,"end_paragraph":0,"end_offset":8,"text":"please confirm","author":"agent"}
  ]
}}

// 2a. dry-run a template edit before mutating the live session
{"name":"apply_commands","arguments":{
  "session_id":"doc-1",
  "dry_run":true,
  "include_diff":true,
  "commands":[
    {"type":"replace_placeholder","placeholder":"${customer_name}","text":"Acme Inc."},
    {"type":"clone_table_row","table_order_index":0,"source_row":1,"target_row":2},
    {"type":"fill_table_row","table_order_index":0,"row":2,"values":["SKU-42","2","1200"]}
  ],
  "queries":[{"type":"table","table_order_index":0}]
}}

// 3. verify
{"name":"query","arguments":{
  "session_id":"doc-1",
  "queries":[{"type":"paragraph_text","paragraph":0}]
}}

// 4. save
{"name":"export_docx","arguments":{"session_id":"doc-1","path":"/tmp/contract.edited.docx"}}
```

### Review workflow — the agent's edits become reviewable tracked changes

Turn on change tracking before editing, and every edit the agent makes is
recorded as a Word tracked change attributed to it — so a human reviewer opens
the file and sees *exactly* what the AI changed, then accepts or rejects it.
The agent can also act on existing revisions and leave comments.

```jsonc
// the agent's edits are now recorded as revisions by "AI Assistant"
{"name":"start_change_tracking","arguments":{"session_id":"doc-1","author":"AI Assistant"}}
{"name":"apply_commands","arguments":{"session_id":"doc-1",
  "commands":[{"type":"insert_text","paragraph":0,"offset":0,"text":"Confidential — "}]}}

// review what changed, then accept or reject — all of them, or one by id
{"name":"query","arguments":{"session_id":"doc-1","queries":[{"type":"revisions","start_revision":0,"max_revisions":50}]}}
{"name":"accept_revisions","arguments":{"session_id":"doc-1","revision_id":"7"}}
{"name":"accept_revisions","arguments":{"session_id":"doc-1"}}

// or leave a comment instead of editing
{"name":"comment_on_text","arguments":{"session_id":"doc-1",
  "query":"the Supplier","text":"Confirm the counterparty name","author":"AI Assistant"}}

// read the counterparty's threads, answer one, and close it
{"name":"query","arguments":{"session_id":"doc-1",
  "queries":[{"type":"comment_threads","start_thread":0,"max_threads":50}]}}
{"name":"reply_to_comment","arguments":{"session_id":"doc-1","comment_id":"3",
  "text":"Agreed — changed to 30 days.","author":"AI Assistant"}}
{"name":"resolve_comment","arguments":{"session_id":"doc-1","comment_id":"3","resolved":true}}

// what is still open after the round trip
{"name":"query","arguments":{"session_id":"doc-1",
  "queries":[{"type":"comments","resolved":false,"max_comments":50}]}}

// changed your mind: every one of those steps is one undo step
{"name":"undo","arguments":{"session_id":"doc-1","count":3}}
```

### Reviewing a contract — reading the parts a redline depends on

Terms drift in casing, styles have to be named before they can be applied, and
the formal attributes of a document live outside the body text:

```jsonc
// find a defined term however the draft capitalises it, then replace it
{"name":"query","arguments":{"session_id":"doc-1",
  "queries":[{"type":"search_text","query":"force majeure","case_insensitive":true}]}}
{"name":"replace_text","arguments":{"session_id":"doc-1",
  "find":"supplier","replace":"Vendor","case_insensitive":true}}

// the style ids set_paragraph_style will accept
{"name":"query","arguments":{"session_id":"doc-1","queries":[{"type":"styles","kind":"paragraph"}]}}

// page geometry, header/footer text and package metadata
{"name":"query","arguments":{"session_id":"doc-1",
  "queries":[{"type":"page_setup"},{"type":"header_footer_text"},{"type":"document_properties"}]}}
```

`header_footer_text` returns each line in the same tab-joined form
`set_header_paragraph_text` / `set_footer_paragraph_text` take, so a header can
be read, edited and written straight back.

### Reading a contract the way it is written — articles, not paragraphs

A contract is drafted, cited and negotiated one **article** at a time. `chunks`
cuts the document on that grain and hands back each article with its number and
its title; `references` resolves the citations between them.

```jsonc
// page through the document by article — number and title only, no body text
{"name":"query","arguments":{"session_id":"doc-1",
  "queries":[{"type":"chunks","include_text":false,"max_chunks":50}]}}
// → [{"index":1,"kind":"article","number":"第１条","title":"秘密保持",
//     "number_key":"1","start_paragraph":8,"end_paragraph":12,"char_count":321}, …]

// then read only the article that matters
{"name":"query","arguments":{"session_id":"doc-1",
  "queries":[{"type":"chunks","start_chunk":1,"max_chunks":1}]}}

// which citations does this draft make, and which of them point at nothing?
{"name":"query","arguments":{"session_id":"doc-1","queries":[{"type":"references"}]}}
{"name":"query","arguments":{"session_id":"doc-1",
  "queries":[{"type":"references","unresolved_only":true}]}}
// → {"paragraph":31,"text":"第２条第２項","start_offset":18,"end_offset":36,
//    "target_paragraph":13,"target_number":"第２条","target_title":"秘密情報等の取扱い",
//    "sub_references":["第２項"],"resolved":true}
```

**Article titles** are recovered from both forms Japanese drafts use, and the
first wins when both are present:

```text
（秘密保持）        ← title_source: "preceding_line"
第１条　ああああ     ← …so this line is body text, not the title

第１条　秘密保持     ← title_source: "inline"
```

The parenthesised line opens the article's chunk rather than closing the one
above it, and `document_outline` reports the same `title` / `title_source` on
every heading.

`references` covers `第５条`, `第５条第２項`, `第２条の２`, `前条` / `次条` /
`本条`, and ranges (`第３条から第７条まで`, which also reports its far end). The
byte offsets are the citation's own, so a dangling one can be commented on or
rewritten in place.

### Replacing across a draft

```jsonc
// what would this touch? (changes nothing)
{"name":"replace_text","arguments":{"session_id":"doc-1",
  "find":"甲","replace":"委託者","dry_run":true}}
// → {"replaced":24,"dry_run":true,"matches":[{"paragraph":8,"start_offset":0,
//    "end_offset":3,"text":"甲","replacement":"委託者"}, …]}

// regular expressions, with capture references
{"name":"replace_text","arguments":{"session_id":"doc-1",
  "find":"第(\\d+)条","replace":"第$1項","regex":true}}

// scoped to one article — take the range from `chunks`
{"name":"replace_text","arguments":{"session_id":"doc-1",
  "find":"秘密情報","replace":"秘密情報等","start_paragraph":13,"end_paragraph":22}}
```

`regex` runs in this server rather than in the shared query API on purpose: the
query API is compiled into the WebAssembly editor core, where a regex engine
would cost every browser user bundle size. This binary is native and pays
nothing for it.

## Domain tools: build your own binary

> Optional, and additive. The stock binary is unchanged: register it exactly as
> above, get the same tools, the same schemas and the same responses. Nothing
> below is required to keep using this server the way you already do —
> `the_stock_server_publishes_exactly_the_built_in_catalogue` and
> `driving_the_server_by_hand_still_works` hold that line.

This crate knows DOCX. It does not know *your* documents — your house style,
your clause library, the checks your reviewers actually run. That knowledge
belongs in your repository, on your release cadence, not versioned here.

So build your own binary: depend on this crate, register the tools your domain
needs, serve. Injected tools sit beside the built-in ones — same `tools/list`,
same schema quality, same session, same undo history — and a client cannot tell
which is which.

```toml
# your-mcp/Cargo.toml
[dependencies]
jade-docx-headless-mcp = { git = "https://github.com/jade-rs/jade-core" }
serde_json = "1"
```

```rust
use jade_docx_headless_mcp::extension::{ToolArg, ToolSpec};
use jade_docx_headless_mcp::jade_docx_headless_api::HeadlessQuery;
use jade_docx_headless_mcp::{serve_stdio, server::Server};
use serde_json::json;

fn main() {
    let mut server = Server::new();

    server
        .register_tool(
            ToolSpec::new(
                "house_style_check",
                "Check house style",
                "Flag clauses that break our drafting rules.",
            )
            .read_only()
            .argument(ToolArg::text("ruleset", "Which rule set to apply.").optional())
            .result(ToolArg::index("findings", "How many problems were found.")),
            |ctx, args| {
                // Read through the same query API the built-in `query` exposes…
                let outline = ctx.query(vec![HeadlessQuery::DocumentOutline {
                    start_heading: 0,
                    max_headings: 500,
                    max_level: None,
                }])?;
                // …and edit through `ctx.apply`, which puts every command on the
                // session's undo stack.
                let _ = args;
                Ok(json!({ "findings": 0, "outline": outline }))
            },
        )
        .expect("house_style_check registers");

    serve_stdio(&mut server);
}
```

### Tools that open a session

A document has to enter the server somehow. `register_session_tool` registers a
tool that is handed the **session store** instead of a session, so a consumer
that fetches a document from somewhere this crate knows nothing about — an
internal service, a signed URL — can hand the loaded document back and return a
`session_id` the built-in tools accept.

```rust
server.register_session_tool(
    ToolSpec::new("open_from_service", "Open from the service", "…")
        .argument(ToolArg::text("url", "Where to fetch it from."))
        .result(ToolArg::text("session_id", "The new session.")),
    |sessions, args| {
        let document = fetch_and_parse(args)?;   // your code
        let session_id = sessions.open(document); // same ids `open_docx` mints
        Ok(json!({ "session_id": session_id }))
    },
)?;
```

`session_id` is **not** added to this tool's schema — it is what the tool
produces. `Sessions` also exposes `get` (borrow the session you just opened),
`close` and `ids`.

Injected tools are **appended** to the catalogue — they never rewrite or shadow
a built-in one (registering a name that already exists is an error), so a client
that knew the old server sees exactly what it saw before, plus your tools.

`Server::new().with_server_info("your-mcp", "Your MCP")` renames the server in
`initialize`. A binary that publishes its own catalogue is a different server to
the client, and the name shows up in their UI and their logs — the version stays
this crate's, since that is the engine actually answering.

What the server does for you:

- **`session_id` is handled.** Do not declare it — the server adds the argument,
  publishes it, resolves it, and reports an unknown id with the same message the
  built-in tools use. Your handler is given the open session.
- **Reads and edits.** [`ToolContext`] hands over the document mutably, plus
  `query(...)` and `apply(...)` for the shared `HeadlessQuery` / `HeadlessCommand`
  vocabulary. Edits made through `apply` are undoable: `undo` steps back over
  your tool exactly as it does over `replace_text`.
- **The same published interface.** Your spec becomes a `title`, a description,
  a closed `inputSchema` with a description on every property, an `outputSchema`,
  and MCP `annotations` from `read_only()` / `destructive()` / `idempotent()` /
  `open_world()`.
- **Registration fails loudly.** A duplicate name (including shadowing a
  built-in), a blank title, an argument with no description, or a tool trying to
  own `session_id` is rejected at start-up rather than published as something no
  model can use.

A runnable example ships with the crate — two Japanese-contract tools, one
read-only and one that edits through the history:

```bash
cargo run -p jade-docx-headless-mcp --example domain_tools
```

```jsonc
{"name":"defined_terms","arguments":{"session_id":"doc-1"}}
// → {"count":7,"terms":[{"term":"秘密情報","paragraph":9,"context":"…"}, …]}

{"name":"rename_party","arguments":{"session_id":"doc-1","label":"甲","name":"委託者"}}
// → {"replaced":13,"by_paragraph":{"4":2,"9":3,…},"can_undo":true}
```

See [`examples/domain_tools.rs`](examples/domain_tools.rs) for the whole file.

## Fidelity demo — why an agent should edit through this, not python-docx

Agent DOCX tooling built on `python-docx` (which re-serializes the entire
document on save) or on `markdown → pandoc` round-trips **silently drops**
revision marks, comments, and fields. This engine preserves the raw XML of
unedited parts, so an agent can change one paragraph without destroying the
rest.

A runnable demonstration ships with the crate:

```bash
cargo run -p jade-docx-headless-mcp --example roundtrip_fidelity
```

It opens documents with Word **tracked changes** and **comments**, edits them
through the tool path, exports to real `.docx` bytes, reopens those bytes, and
shows the revision marks and comments survived byte-for-byte:

```text
# Tracked changes — public/fixtures/review.docx
BEFORE — tracked changes:
  - inserted "Hello " by 植木 智之
  - deleted "World" by 植木 智之
AFTER export → reopen — tracked changes still present:
  - inserted "Hello " by 植木 智之
  - deleted "World" by 植木 智之
✅ 2 tracked change(s) preserved; agent edit landed in paragraph 1.

# Comments — public/fixtures/comment.docx
BEFORE — comments:
  - "Hello Comment" by 植木 智之 (resolved=false)
  - "Reply" by 植木 智之 (resolved=false)
AFTER export → reopen — comments still present:
  - "Hello Comment" by 植木 智之 (resolved=false)
  - "Reply" by 植木 智之 (resolved=false)
✅ 2 comment(s) preserved through an edit in the commented paragraph.
```

**Fields** (a TOC driven by `TOC` / `PAGEREF` field codes) are preserved too: an
edit to an unrelated body paragraph leaves the `w:instrText` / `w:fldChar`
markers in the exported `document.xml` byte-identical.

These guarantees are locked down by the `export_reopen_preserves_tracked_changes`,
`export_reopen_preserves_comments`, and `export_preserves_field_codes` unit tests.

## Why this is the fast path

The stateless `jade-docx-headless-editor` convenience functions load → apply → export
on every call, paying the parse/zip cost each time. This server keeps the parsed
`WordDocument` resident between calls, so only the edit itself runs per
`apply_commands`. That matters when an agent makes many small, dependent edits.

## Notes / limitations (v1)

- Sessions live for the lifetime of the server process; there is no persistence
  across restarts.
- `undo` / `redo` step at the editor's own history granularity (one command per
  step). Pass `count` to take several steps at once; a multi-command
  `apply_commands` batch therefore needs that many steps to fully revert.
  Correctness is property-tested: undoing an arbitrary edit history (mixing
  single edits and multi-command `replace_text` batches) returns to the original
  and redoing returns to the final state, and an interleaved edit/undo/redo
  stream stays in step with a past/present/future model.
- Lossless round-trip relies on the editor's raw-XML preservation: unedited
  parts are preserved as-is.
- `add_comment` anchors to plain run text; anchoring a comment over a range that
  already contains tracked-change runs can fail (the tool reports the error
  rather than producing a broken anchor). Commenting on normal text works.
- `replace_text` is literal and case-sensitive, matches non-overlapping (like
  `str::replace`), and applies edits highest-offset-first so length changes never
  mis-place a later replacement. A match split across multiple `w:r` runs is
  handled by the underlying run-splitting edit; this is covered by a property
  test against a plain-string model here, and by an `apply_text_edit` cross-run
  property test in jade (which proves bytes outside an edit keep their
  original run formatting). Undo granularity is one step per replaced match.
