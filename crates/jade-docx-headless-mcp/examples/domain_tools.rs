//! A consumer's MCP binary: the stock DOCX tools plus tools this repository
//! has no business knowing about.
//!
//! This is the shape to copy. Depend on `jade-docx-headless-mcp`, write the tools
//! your documents actually need, register them, serve. Nothing here touches the
//! server's internals — everything goes through [`ToolContext`], so the domain
//! code lives entirely in the consumer's own repository and moves at its speed.
//!
//! The two tools below are deliberately domain-specific — Japanese contract
//! drafting conventions, not DOCX:
//!
//! - `defined_terms` reads the definitions a draft introduces
//!   (`以下「秘密情報」という`), which is what a reviewer needs before they can
//!   judge whether the rest of the document uses them consistently.
//! - `rename_party` swaps the party labels (`甲` → `委託者`) across the draft,
//!   through the session's history so `undo` steps back over it.
//!
//! Run it as an MCP server exactly like the stock binary:
//!
//! ```bash
//! cargo run -p jade-docx-headless-mcp --example domain_tools
//! ```

use std::collections::BTreeMap;

use jade_docx_headless_mcp::extension::{ToolArg, ToolContext, ToolSpec};
use jade_docx_headless_mcp::jade_docx_headless_api::{HeadlessCommand, HeadlessQuery};
use jade_docx_headless_mcp::{serve_stdio, server::Server};
use serde_json::{json, Value};

fn main() {
    let mut server = Server::new();

    server
        .register_tool(defined_terms_spec(), defined_terms)
        .expect("defined_terms registers");
    server
        .register_tool(rename_party_spec(), rename_party)
        .expect("rename_party registers");

    serve_stdio(&mut server);
}

// --- defined_terms ---------------------------------------------------------

fn defined_terms_spec() -> ToolSpec {
    ToolSpec::new(
        "defined_terms",
        "List defined terms",
        "List the terms this draft defines — the 「…」 phrases introduced by \
         とは / という / をいう — with the paragraph each definition sits in. Read \
         this before judging whether the rest of the document uses them \
         consistently.",
    )
    .read_only()
    .argument(
        ToolArg::index(
            "start_paragraph",
            "First paragraph to scan; omit to scan the whole document.",
        )
        .optional(),
    )
    .result(ToolArg::array_of(
        "terms",
        "The defined terms, in document order.",
        json!({
            "type": "object",
            "properties": {
                "term": { "type": "string", "description": "The term as the draft writes it." },
                "paragraph": { "type": "integer", "description": "Where it is defined." },
                "context": { "type": "string", "description": "The sentence introducing it." }
            },
            "required": ["term", "paragraph", "context"]
        }),
    ))
    .result(ToolArg::index("count", "How many definitions were found."))
}

fn defined_terms(ctx: &mut ToolContext<'_>, args: &Value) -> Result<Value, String> {
    let start = args
        .get("start_paragraph")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    // Read the body through the same query API the built-in `query` tool
    // exposes: an injected tool never needs a private door.
    let response = ctx.query(vec![HeadlessQuery::ParagraphSlice {
        start_paragraph: start,
        max_paragraphs: usize::MAX,
    }])?;

    let mut terms = Vec::new();
    for paragraph in response["results"][0]["paragraphs"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let index = paragraph["paragraph"].as_u64().unwrap_or_default() as usize;
        let text = paragraph["text"].as_str().unwrap_or_default();
        for term in definitions_in(text) {
            terms.push(json!({
                "term": term,
                "paragraph": index,
                "context": text,
            }));
        }
    }

    Ok(json!({ "count": terms.len(), "terms": terms }))
}

/// Terms a sentence defines: `「秘密情報」とは`, `以下「本件業務」という`.
///
/// Only 「」 quotes followed by a defining verb count, so an ordinary quotation
/// is not mistaken for a definition.
fn definitions_in(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('「') {
        let after_open = &rest[open + '「'.len_utf8()..];
        let Some(close) = after_open.find('」') else {
            break;
        };
        let term = &after_open[..close];
        let tail = &after_open[close + '」'.len_utf8()..];
        let defines = ["とは", "という", "をいう", "と総称"]
            .iter()
            .any(|marker| tail.starts_with(marker));
        if defines && !term.is_empty() {
            terms.push(term.to_string());
        }
        rest = tail;
    }
    terms
}

// --- rename_party ----------------------------------------------------------

fn rename_party_spec() -> ToolSpec {
    ToolSpec::new(
        "rename_party",
        "Rename a party label",
        "Replace a party label (甲, 乙, 丙) with a name throughout the draft — \
         甲 → 委託者 and so on. Applies through the session's history, so `undo` \
         steps back over it.",
    )
    .destructive()
    .argument(ToolArg::choice(
        "label",
        "The party label to replace.",
        &["甲", "乙", "丙", "丁"],
    ))
    .argument(ToolArg::text("name", "What to call that party instead."))
    .result(ToolArg::index(
        "replaced",
        "How many occurrences were replaced.",
    ))
    .result(ToolArg::object(
        "by_paragraph",
        "Replacement count per paragraph index.",
    ))
}

fn rename_party(ctx: &mut ToolContext<'_>, args: &Value) -> Result<Value, String> {
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .ok_or("rename_party requires `label`")?
        .to_string();
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or("rename_party requires `name`")?
        .to_string();

    let response = ctx.query(vec![HeadlessQuery::SearchText {
        query: label.clone(),
        start_paragraph: 0,
        max_results: usize::MAX,
        context_before_chars: 0,
        context_after_chars: 0,
        case_insensitive: false,
    }])?;

    let mut found: Vec<(usize, usize, usize)> = response["results"][0]["matches"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|found| {
            Some((
                found["paragraph"].as_u64()? as usize,
                found["start_offset"].as_u64()? as usize,
                found["end_offset"].as_u64()? as usize,
            ))
        })
        .collect();

    let mut by_paragraph: BTreeMap<usize, usize> = BTreeMap::new();
    for (paragraph, _, _) in &found {
        *by_paragraph.entry(*paragraph).or_default() += 1;
    }

    // Highest offset first, so a length-changing replacement never invalidates
    // the offsets of a match that has not been applied yet.
    found.sort_by(|left, right| right.cmp(left));
    let commands = found
        .iter()
        .map(|(paragraph, start, end)| HeadlessCommand::ReplaceRange {
            start_paragraph: *paragraph,
            start_offset: *start,
            end_paragraph: *paragraph,
            end_offset: *end,
            text: name.clone(),
        })
        .collect::<Vec<_>>();

    let replaced = commands.len();
    if replaced > 0 {
        ctx.apply(commands)?;
    }

    Ok(json!({
        "replaced": replaced,
        "by_paragraph": by_paragraph
            .into_iter()
            .map(|(paragraph, count)| (paragraph.to_string(), json!(count)))
            .collect::<serde_json::Map<_, _>>(),
        "can_undo": ctx.can_undo(),
    }))
}
