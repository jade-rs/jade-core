//! Undo support for edits that reach beyond `word/document.xml`.
//!
//! Accepting a tracked change, adding a comment or resolving one rewrites the
//! body, the raw XML, the comment map and several sibling parts at once.
//! Reversing each of those mutations individually would mean a second
//! implementation of every write path, kept in step by hand. So the command
//! keeps the state before the mutation and puts it back on undo.
//!
//! What it keeps depends on what the mutation can change ([`EditScope`]). A
//! whole-document snapshot — the body model and `document.xml` copied whole —
//! took ~40 ms per accepted change on a 149-page document, twice over, and
//! another round of copying to put back. Most review actions change a known
//! run of paragraphs, so they keep those paragraphs, the package parts (small
//! next to the body) and a journal of the round-trip XML instead.

use super::paragraph_undo::ParagraphRangeUndo;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{WordDocument, WordDocumentSnapshot};
use jade_docx_layout::selection::{SelectionRange, TextPosition};
use std::collections::HashSet;

/// The mutation a [`SnapshotEditCommand`] wraps.
type DocumentMutation = Box<dyn FnMut(&mut WordDocument) -> Result<(), String>>;

/// What a [`SnapshotEditCommand`]'s mutation can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditScope {
    /// Anything. Undo restores a snapshot of the whole document.
    Document,
    /// The package parts outside `document.xml`, the round-trip XML, the
    /// revision id counter and the body paragraphs in this inclusive range
    /// (none for `None`) — without adding or removing paragraphs.
    Paragraphs(Option<(usize, usize)>),
}

/// A document mutation made undoable by keeping the state it can change. The
/// mutation is re-run on redo against the restored document, so it must be a
/// function of document state alone (all the review operations are), and so
/// must its scope.
pub struct SnapshotEditCommand {
    name: &'static str,
    scope: EditScope,
    mutate: DocumentMutation,
    undo: Option<EditUndo>,
    previous_selection: Option<SelectionRange>,
}

enum EditUndo {
    Snapshot(Box<WordDocumentSnapshot>),
    Paragraphs(ParagraphRangeUndo),
}

impl SnapshotEditCommand {
    pub fn new(
        name: &'static str,
        mutate: impl FnMut(&mut WordDocument) -> Result<(), String> + 'static,
    ) -> Self {
        Self::with_scope(name, EditScope::Document, mutate)
    }

    /// A command whose mutation changes no more than `scope`.
    pub fn with_scope(
        name: &'static str,
        scope: EditScope,
        mutate: impl FnMut(&mut WordDocument) -> Result<(), String> + 'static,
    ) -> Self {
        Self {
            name,
            scope,
            mutate: Box::new(mutate),
            undo: None,
            previous_selection: None,
        }
    }
}

impl Command for SnapshotEditCommand {
    fn name(&self) -> &'static str {
        self.name
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        // The journal can only roll back XML that exists; a document whose
        // round-trip XML is not prepared yet keeps the snapshot instead, and
        // so does a range past the last paragraph, so the mutation reports
        // that itself.
        let paragraph_count = ctx.document.paragraph_count();
        let scope = match self.scope {
            EditScope::Paragraphs(range)
                if ctx.document.raw_document.is_some()
                    && range.is_none_or(|(_, end)| end < paragraph_count) =>
            {
                EditScope::Paragraphs(range)
            }
            _ => EditScope::Document,
        };
        // Keep the state first and the undo data only once the mutation
        // succeeded: a failed command must leave neither the document changed
        // nor a history entry behind.
        let undo = match scope {
            EditScope::Paragraphs(range) => {
                let mut checkpoint = ParagraphRangeUndo::begin_with_package(ctx, range)?;
                if let Err(error) = (self.mutate)(ctx.document) {
                    checkpoint.abort(ctx);
                    return Err(CommandError::Custom(error));
                }
                checkpoint.finish(ctx);
                debug_assert_eq!(
                    ctx.document.paragraph_count(),
                    paragraph_count,
                    "{} changed the paragraph count outside its edit scope",
                    self.name
                );
                EditUndo::Paragraphs(checkpoint)
            }
            EditScope::Document => {
                let snapshot = ctx
                    .document
                    .snapshot_including_package()
                    .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
                if let Err(error) = (self.mutate)(ctx.document) {
                    ctx.document.restore_snapshot(snapshot);
                    return Err(CommandError::Custom(error));
                }
                EditUndo::Snapshot(Box::new(snapshot))
            }
        };
        self.undo = Some(undo);
        self.previous_selection = previous_selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        match self
            .undo
            .take()
            .ok_or_else(|| CommandError::Custom(format!("{} snapshot is missing", self.name)))?
        {
            EditUndo::Snapshot(snapshot) => ctx.document.restore_snapshot(*snapshot),
            EditUndo::Paragraphs(checkpoint) => checkpoint.undo(ctx)?,
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

/// Applies a pre-parsed package edit only if its source has not changed.
/// The normal command executor owns undo/redo; browser imports do not reset it.
///
/// `caret` is where the edit wants the caret afterwards — for a paste, right
/// after the pasted content, as in Word. Without one (or when it does not
/// address the replacement) the caret goes to the document start, because a
/// replacement can have fewer paragraphs and shorter runs.
pub fn package_edit_command(
    expected: Vec<u8>,
    replacement: WordDocumentSnapshot,
    caret: Option<TextPosition>,
) -> Box<dyn Command> {
    Box::new(PackageEditCommand {
        expected,
        replacement,
        caret,
        previous: None,
        selection: None,
    })
}

struct PackageEditCommand {
    expected: Vec<u8>,
    replacement: WordDocumentSnapshot,
    caret: Option<TextPosition>,
    previous: Option<WordDocumentSnapshot>,
    selection: Option<SelectionRange>,
}
impl Command for PackageEditCommand {
    fn name(&self) -> &'static str {
        "edit_document_package"
    }
    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let bytes = ctx
            .document
            .export_docx()
            .map_err(|e| CommandError::Custom(e.to_string()))?;
        if bytes != self.expected {
            return Err(CommandError::Custom(
                "The document changed while preparing the edit; try again".into(),
            ));
        }
        self.previous = ctx.document.snapshot_for_replacement();
        self.selection = *ctx.selection;
        ctx.document.restore_snapshot(self.replacement.clone());
        let caret = self
            .caret
            .and_then(|caret| clamp_caret(ctx.document, caret))
            .unwrap_or_default();
        *ctx.selection = Some(SelectionRange::collapsed(caret));
        Ok(())
    }
    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("replacement snapshot missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.selection;
        Ok(())
    }
}

/// `caret` inside the document: a paragraph that exists, with the offset
/// clamped to that paragraph's end.
fn clamp_caret(document: &WordDocument, caret: TextPosition) -> Option<TextPosition> {
    let paragraph = document.paragraph(caret.paragraph)?;
    let model_end = paragraph
        .runs
        .iter()
        .map(|run| run.text.len())
        .sum::<usize>();
    let end = document
        .model_to_rendered_text_position(TextPosition::new(caret.paragraph, model_end))
        .offset;
    Some(TextPosition::new(caret.paragraph, caret.offset.min(end)))
}

/// Wraps a command whose own undo cannot put the whole package back, and keeps
/// the comments consistent with what it did.
///
/// Deleting a table takes the comments anchored inside it with it. The command
/// that does the deleting restores `word/document.xml` on undo, but
/// `comments.xml` is a different part: without this wrapper the export ends up
/// declaring a comment nothing points at (a comment Word can neither show nor
/// dismiss), and undo brings the anchor back only halfway.
///
/// The comments removed are exactly those the inner command unanchored —
/// computed as the difference between the ids the body referenced before and
/// after — so a comment living somewhere this editor does not scan (a header,
/// say) is never mistaken for an orphan.
pub struct PackageSnapshotCommand {
    inner: Box<dyn Command>,
    /// Whether the inner command's own undo restores everything it changed,
    /// so only the comment clean-up needs keeping; see
    /// [`Self::wrapping_exact_undo`].
    inner_undo_is_exact: bool,
    undo: Option<PackageUndo>,
    previous_selection: Option<SelectionRange>,
}

enum PackageUndo {
    Snapshot(Box<WordDocumentSnapshot>),
    /// What deleting the orphaned comments changed, if there were any; the
    /// inner command undoes the rest.
    Comments(Option<ParagraphRangeUndo>),
}

impl PackageSnapshotCommand {
    pub fn wrapping(inner: Box<dyn Command>) -> Self {
        Self {
            inner,
            inner_undo_is_exact: false,
            undo: None,
            previous_selection: None,
        }
    }

    /// Wraps a command whose undo puts the body and its XML back exactly — a
    /// text edit across paragraphs. Only the comments it leaves unanchored,
    /// usually none, are kept for undo, instead of the whole document: on a
    /// 149-page document that snapshot was most of the time a delete across
    /// paragraphs took.
    pub fn wrapping_exact_undo(inner: Box<dyn Command>) -> Self {
        Self {
            inner_undo_is_exact: true,
            ..Self::wrapping(inner)
        }
    }

    fn run(&mut self, ctx: &mut CommandContext<'_>, redo: bool) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let snapshot = if self.inner_undo_is_exact {
            None
        } else {
            Some(
                ctx.document
                    .snapshot_including_package()
                    .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?,
            )
        };
        let before = ctx.document.referenced_comment_ids();

        if redo {
            self.inner.redo(ctx)?;
        } else {
            self.inner.apply(ctx)?;
        }

        let after = ctx.document.referenced_comment_ids();
        let mut orphaned: Vec<&String> = before.difference(&after).collect();
        orphaned.sort();
        let undo = match snapshot {
            Some(snapshot) => {
                delete_orphaned_comments(ctx.document, &orphaned).map_err(CommandError::Custom)?;
                PackageUndo::Snapshot(Box::new(snapshot))
            }
            None if orphaned.is_empty() => PackageUndo::Comments(None),
            None => {
                // Nothing in the body references these comments any more, so
                // deleting them changes the package and what is left of their
                // markers in the XML. Their replies go with them, and only a
                // reply the edit left anchored changes a paragraph.
                let doomed: HashSet<String> = orphaned
                    .iter()
                    .flat_map(|id| ctx.document.comment_and_replies(id))
                    .collect();
                let span = ctx.document.comment_anchor_paragraph_span(&doomed);
                let mut comments = match ParagraphRangeUndo::begin_with_package(ctx, span) {
                    Ok(comments) => comments,
                    Err(error) => {
                        let _ = self.inner.undo(ctx);
                        return Err(error);
                    }
                };
                if let Err(error) = delete_orphaned_comments(ctx.document, &orphaned) {
                    comments.abort(ctx);
                    let _ = self.inner.undo(ctx);
                    return Err(CommandError::Custom(error));
                }
                comments.finish(ctx);
                PackageUndo::Comments(Some(comments))
            }
        };

        self.undo = Some(undo);
        self.previous_selection = previous_selection;
        Ok(())
    }
}

/// Deletes the comments an edit unanchored. Deleting one takes its replies
/// with it, so a reply orphaned together with its parent can be gone by the
/// time its turn comes.
fn delete_orphaned_comments(
    document: &mut WordDocument,
    orphaned: &[&String],
) -> Result<(), String> {
    let mut taken = HashSet::new();
    for &id in orphaned {
        if taken.contains(id) {
            continue;
        }
        taken.extend(document.comment_and_replies(id));
        document.delete_comment(id)?;
    }
    Ok(())
}

impl Command for PackageSnapshotCommand {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.run(ctx, false)
    }

    fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.run(ctx, true)
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        match self
            .undo
            .take()
            .ok_or_else(|| CommandError::Custom("package snapshot is missing".into()))?
        {
            PackageUndo::Snapshot(snapshot) => ctx.document.restore_snapshot(*snapshot),
            PackageUndo::Comments(comments) => {
                if let Some(comments) = comments {
                    comments.undo(ctx)?;
                }
                self.inner.undo(ctx)?;
            }
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::headless_api::apply_headless_commands_with_executor_to_document;
    use crate::word_document::RevisionSession;
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
    use std::path::PathBuf;

    fn load(name: &str) -> WordDocument {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name);
        let data = std::fs::read(path).expect("read fixture");
        executor::block_on(WordDocument::load(&data, &Options::default())).expect("load")
    }

    fn session() -> Option<RevisionSession> {
        Some(RevisionSession {
            author: Some("reviewer".into()),
            date: Some("2026-10-05T00:00:00Z".into()),
            date_utc: None,
        })
    }

    fn text(doc: &WordDocument, index: usize) -> String {
        doc.paragraph(index).map_or(String::new(), |paragraph| {
            paragraph.runs.iter().map(|run| run.text.as_str()).collect()
        })
    }

    /// The byte offset of the `chars`-th character of a paragraph.
    fn at(doc: &WordDocument, paragraph: usize, chars: usize) -> (usize, usize) {
        let text = text(doc, paragraph);
        let offset = text
            .char_indices()
            .nth(chars)
            .map_or(text.len(), |(offset, _)| offset);
        (paragraph, offset)
    }

    /// Three neighbouring body paragraphs with text of their own.
    fn text_paragraphs(doc: &WordDocument) -> usize {
        (0..doc.paragraph_count().saturating_sub(3))
            .find(|&k| {
                (k..k + 3).all(|index| {
                    text(doc, index).chars().count() >= 8
                        && doc.paragraphs_share_a_container(k, index)
                        && !doc.is_note_paragraph_surface(index)
                })
            })
            .expect("three neighbouring body paragraphs with text")
    }

    /// Everything undo has to put back: the bytes the document saves to and
    /// the model the editor lays out and edits from.
    fn state(doc: &WordDocument) -> (Vec<u8>, String) {
        let mut comments: Vec<String> = doc
            .comments
            .iter()
            .map(|(id, comment)| format!("{id}: {comment:?}"))
            .collect();
        comments.sort();
        (
            doc.export_docx().expect("export"),
            format!(
                "{:?}\n{comments:?}\n{}",
                doc.document,
                doc.revision_id_counter()
            ),
        )
    }

    fn without_node_ids(model: &str) -> String {
        let mut out = String::with_capacity(model.len());
        let mut rest = model;
        while let Some(start) = rest.find("XmlNodeId(") {
            out.push_str(&rest[..start]);
            out.push_str("XmlNodeId(_");
            rest = &rest[start + "XmlNodeId(".len()..];
            rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        }
        out.push_str(rest);
        out
    }

    fn run(doc: &mut WordDocument, executor: &mut CommandExecutor, command: HeadlessCommand) {
        apply_headless_commands_with_executor_to_document(
            doc,
            executor,
            None,
            &HeadlessEditRequest {
                commands: vec![command],
            },
        )
        .expect("apply");
    }

    /// Applies `command`, then undoes and redoes it twice: every undo has to
    /// give back the state before it and every redo the state after it.
    fn assert_undo_round_trips(doc: &mut WordDocument, command: HeadlessCommand) {
        let mut executor = CommandExecutor::new();
        let before = state(doc);
        run(doc, &mut executor, command.clone());
        let after = state(doc);
        assert!(after.0 != before.0, "{command:?} changes the document");
        for round in 0..2 {
            let mut selection = None;
            let mut ctx = CommandContext::new(doc, &mut selection);
            assert!(executor.undo(&mut ctx).expect("undo"));
            let undone = state(ctx.document);
            assert!(
                undone.0 == before.0,
                "{command:?}: undo {round} saves the original bytes"
            );
            assert_eq!(
                undone.1, before.1,
                "{command:?}: undo {round} restores the model"
            );
            assert!(executor.redo(&mut ctx).expect("redo"));
            let redone = state(ctx.document);
            assert!(
                redone.0 == after.0,
                "{command:?}: redo {round} saves the same bytes"
            );
            // Elements a redo creates take fresh node ids.
            assert_eq!(
                without_node_ids(&redone.1),
                without_node_ids(&after.1),
                "{command:?}: redo {round} rebuilds the model"
            );
        }
    }

    fn decide(accept: bool, paragraph: usize) -> HeadlessCommand {
        let (start_paragraph, end_paragraph) = (Some(paragraph), Some(paragraph));
        if accept {
            HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph,
                end_paragraph,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }
        } else {
            HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph,
                end_paragraph,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }
        }
    }

    fn add_comment(start: (usize, usize), end: (usize, usize)) -> HeadlessCommand {
        HeadlessCommand::AddComment {
            start_paragraph: start.0,
            start_offset: Some(start.1),
            end_paragraph: end.0,
            end_offset: Some(end.1),
            text: "check this".into(),
            author: Some("reviewer".into()),
            initials: None,
            date: Some("2026-10-05T00:00:00Z".into()),
        }
    }

    /// A tracked insertion and deletion in paragraph `k`.
    fn track_changes(doc: &mut WordDocument, k: usize) {
        let mut executor = CommandExecutor::new();
        doc.set_revision_session(session());
        let offset = at(doc, k, 1).1;
        run(
            doc,
            &mut executor,
            HeadlessCommand::InsertText {
                paragraph: k,
                offset,
                text: "new ".into(),
            },
        );
        let (start, end) = (at(doc, k, 6).1, at(doc, k, 7).1);
        run(
            doc,
            &mut executor,
            HeadlessCommand::ReplaceRange {
                start_paragraph: k,
                start_offset: start,
                end_paragraph: k,
                end_offset: end,
                text: String::new(),
            },
        );
        doc.set_revision_session(None);
    }

    #[test]
    fn deciding_revisions_undoes_exactly() {
        for accept in [true, false] {
            let mut doc = load("contract.docx");
            let k = text_paragraphs(&doc);
            track_changes(&mut doc, k);
            assert_undo_round_trips(&mut doc, decide(accept, k));
        }
    }

    #[test]
    fn comment_edits_undo_exactly() {
        let mut doc = load("contract.docx");
        let k = text_paragraphs(&doc);
        let comment = add_comment(at(&doc, k, 1), at(&doc, k, 3));
        assert_undo_round_trips(&mut doc, comment);
        let comment = add_comment(at(&doc, k, 1), at(&doc, k + 2, 2));
        assert_undo_round_trips(&mut doc, comment);

        let mut executor = CommandExecutor::new();
        let comment = add_comment(at(&doc, k + 1, 0), at(&doc, k + 1, 2));
        run(&mut doc, &mut executor, comment);
        let comment_id = doc.comments.keys().next().expect("comment").clone();
        assert_undo_round_trips(
            &mut doc,
            HeadlessCommand::ResolveComment {
                comment_id: comment_id.clone(),
                resolved: true,
            },
        );
        assert_undo_round_trips(
            &mut doc,
            HeadlessCommand::UpdateComment {
                comment_id: comment_id.clone(),
                text: "checked".into(),
            },
        );
        assert_undo_round_trips(
            &mut doc,
            HeadlessCommand::ReplyToComment {
                comment_id,
                text: "agreed".into(),
                author: Some("author".into()),
                initials: None,
                date: Some("2026-10-05T00:00:00Z".into()),
            },
        );
    }

    /// The review commands keep the paragraphs they touch rather than a copy
    /// of the whole document, which on a long document was most of their time.
    #[test]
    fn review_edits_keep_their_paragraphs_instead_of_a_snapshot() {
        let mut doc = load("contract.docx");
        let k = text_paragraphs(&doc);
        track_changes(&mut doc, k);
        for accept in [true, false] {
            let scope = doc.revision_decision_scope(accept, None, Some(k), Some(k), None, None);
            assert_eq!(scope, EditScope::Paragraphs(Some((k, k))));
            let mut command = SnapshotEditCommand::with_scope("decide", scope, move |doc| {
                doc.decide_revisions_unguarded(accept, None, Some(k), Some(k), None, None)
                    .map(|_| ())
            });
            let mut selection = None;
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            command.apply(&mut ctx).expect("decide");
            assert!(matches!(command.undo, Some(EditUndo::Paragraphs(_))));
            command.undo(&mut ctx).expect("undo");
        }
    }

    /// Rejecting an inserted paragraph mark joins two paragraphs, which the
    /// paragraph backups cannot undo; that decision keeps a snapshot.
    #[test]
    fn decisions_that_join_paragraphs_keep_a_snapshot() {
        let mut doc = load("contract.docx");
        let k = text_paragraphs(&doc);
        let mut executor = CommandExecutor::new();
        doc.set_revision_session(session());
        let offset = at(&doc, k, 2).1;
        run(
            &mut doc,
            &mut executor,
            HeadlessCommand::InsertParagraphBreak {
                paragraph: k,
                offset,
                count: 1,
            },
        );
        doc.set_revision_session(None);
        assert_eq!(
            doc.revision_decision_scope(false, None, Some(k), Some(k + 1), None, None),
            EditScope::Document
        );
        assert_undo_round_trips(&mut doc, {
            let mut command = decide(false, k);
            if let HeadlessCommand::RejectRevisions { end_paragraph, .. } = &mut command {
                *end_paragraph = Some(k + 1);
            }
            command
        });
    }

    /// A mutation that fails part-way leaves nothing changed behind it.
    #[test]
    fn a_failed_review_edit_leaves_the_document_unchanged() {
        let mut doc = load("contract.docx");
        let k = text_paragraphs(&doc);
        let ((_, start), (_, end)) = (at(&doc, k, 1), at(&doc, k, 3));
        let before = state(&doc);
        let mut command = SnapshotEditCommand::with_scope(
            "add_comment",
            EditScope::Paragraphs(Some((k, k))),
            move |doc| {
                doc.add_comment(crate::word_document::AddCommentRequest {
                    start_paragraph: k,
                    start_offset: start,
                    end_paragraph: k,
                    end_offset: end,
                    text: "check this".into(),
                    author: None,
                    initials: None,
                    date: None,
                })?;
                Err("failed after anchoring".into())
            },
        );
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        command.apply(&mut ctx).expect_err("the mutation fails");
        assert!(command.undo.is_none());
        let after = state(&doc);
        assert!(
            after.0 == before.0,
            "the failed edit saves the original bytes"
        );
        assert_eq!(
            after.1, before.1,
            "the failed edit leaves the model as it was"
        );
    }

    /// A delete across paragraphs keeps no snapshot; when it takes a whole
    /// comment range with it, the comment goes too and undo brings it back.
    #[test]
    fn deleting_across_paragraphs_undoes_the_comments_it_orphans() {
        let mut doc = load("contract.docx");
        let k = text_paragraphs(&doc);
        let delete = HeadlessCommand::ReplaceRange {
            start_paragraph: k,
            start_offset: at(&doc, k, 1).1,
            end_paragraph: k + 2,
            end_offset: at(&doc, k + 2, 1).1,
            text: String::new(),
        };
        assert_undo_round_trips(&mut doc, delete.clone());

        let mut doc = load("contract.docx");
        let mut executor = CommandExecutor::new();
        let comment = add_comment(at(&doc, k + 1, 0), at(&doc, k + 1, 2));
        run(&mut doc, &mut executor, comment);
        let comments = doc.comments.len();
        let before = state(&doc);
        run(&mut doc, &mut executor, delete.clone());
        assert_eq!(
            doc.comments.len(),
            comments - 1,
            "the orphaned comment goes"
        );
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        assert!(executor.undo(&mut ctx).expect("undo"));
        let undone = state(&doc);
        assert!(undone.0 == before.0, "undo saves the original bytes");
        assert_eq!(undone.1, before.1, "undo brings the comment back");
        assert_undo_round_trips(&mut doc, delete);
    }

    /// Deleting the first comment of a thread takes its reply with it (Word's
    /// "Delete thread"), and a single undo brings both back.
    #[test]
    fn deleting_a_thread_root_takes_its_replies_in_one_undo_step() {
        let mut doc = load("comment.docx");
        assert_eq!(
            doc.comments
                .get("1")
                .and_then(|comment| comment.thread_parent_id.as_deref()),
            Some("0")
        );
        assert_undo_round_trips(
            &mut doc,
            HeadlessCommand::DeleteComment {
                comment_id: "0".into(),
            },
        );
        // The round trip ends redone.
        assert!(
            doc.comments.is_empty(),
            "the reply goes with its parent: {:?}",
            doc.comments.keys().collect::<Vec<_>>()
        );
        assert!(doc.referenced_comment_ids().is_empty());
    }

    /// Deleting the text a thread is anchored to unanchors the comment and its
    /// reply together. Deleting the comment already takes the reply, which
    /// must not then fail the edit as a comment that cannot be found.
    #[test]
    fn deleting_text_that_orphans_a_thread_takes_it_whole() {
        let mut doc = load("contract.docx");
        let k = text_paragraphs(&doc);
        let mut executor = CommandExecutor::new();
        let comments = doc.comments.len();
        let existing: HashSet<String> = doc.comments.keys().cloned().collect();
        let comment = add_comment(at(&doc, k + 1, 0), at(&doc, k + 1, 2));
        run(&mut doc, &mut executor, comment);
        let root = doc
            .comments
            .keys()
            .find(|id| !existing.contains(*id))
            .expect("the new comment")
            .clone();
        run(
            &mut doc,
            &mut executor,
            HeadlessCommand::ReplyToComment {
                comment_id: root,
                text: "agreed".into(),
                author: Some("author".into()),
                initials: None,
                date: Some("2026-10-05T00:00:00Z".into()),
            },
        );
        assert_eq!(doc.comments.len(), comments + 2);
        let delete = HeadlessCommand::ReplaceRange {
            start_paragraph: k,
            start_offset: at(&doc, k, 1).1,
            end_paragraph: k + 2,
            end_offset: at(&doc, k + 2, 1).1,
            text: String::new(),
        };
        assert_undo_round_trips(&mut doc, delete);
        assert_eq!(doc.comments.len(), comments, "the thread goes whole");
    }

    /// A reply anchored away from its parent outlives an edit that unanchors
    /// only the parent. It goes with the parent all the same, and undo has to
    /// restore its anchors in a paragraph the edit itself never touched.
    #[test]
    fn a_reply_anchored_elsewhere_goes_with_its_orphaned_parent_and_comes_back() {
        use std::io::Read;

        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/comment.docx");
        let bytes = std::fs::read(path).expect("read fixture");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open fixture");
        let mut files = std::collections::HashMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).expect("package entry");
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents).expect("read part");
            files.insert(entry.name().unwrap().into_owned(), contents);
        }
        let document = String::from_utf8(files["word/document.xml"].clone()).expect("UTF-8");
        let body_start = document.find("<w:body>").expect("body") + "<w:body>".len();
        let body_end = document.find("<w:sectPr").expect("sectPr");
        let plain = |text: &str| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>");
        let anchored = |id: u32, text: &str| {
            format!(
                r#"<w:p><w:commentRangeStart w:id="{id}"/><w:r><w:t>{text}</w:t></w:r><w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r></w:p>"#
            )
        };
        let document = format!(
            "{}{}{}{}{}{}{}",
            &document[..body_start],
            plain("First paragraph"),
            anchored(0, "Parent anchor"),
            plain("Third paragraph"),
            plain("Fourth paragraph"),
            anchored(1, "Reply anchor"),
            &document[body_end..]
        );
        let input = jade_docx_writer::write_docx(
            &files,
            &std::collections::HashMap::from([(
                "word/document.xml".to_string(),
                document.into_bytes(),
            )]),
        )
        .expect("build package");
        let mut doc =
            executor::block_on(WordDocument::load(&input, &Options::default())).expect("load");
        assert_eq!(
            doc.comments
                .get("1")
                .and_then(|comment| comment.thread_parent_id.as_deref()),
            Some("0")
        );

        // Paragraphs 0..=2 join into one; the reply's paragraph moves up to 2.
        let delete = HeadlessCommand::ReplaceRange {
            start_paragraph: 0,
            start_offset: at(&doc, 0, 1).1,
            end_paragraph: 2,
            end_offset: at(&doc, 2, 1).1,
            text: String::new(),
        };
        assert_undo_round_trips(&mut doc, delete);
        assert!(
            doc.comments.is_empty(),
            "the reply goes with its parent: {:?}",
            doc.comments.keys().collect::<Vec<_>>()
        );
        assert!(doc.referenced_comment_ids().is_empty());
        assert_eq!(text(&doc, 2), "Reply anchor");
    }
}
