//! Command pattern infrastructure for document editing.
//!
//! The editing pipeline is designed around small command objects that mutate an
//! `EditableDocument` and update the caret/selection state. Capturing mutation
//! in discrete commands keeps the door open for undo/redo stacks, collaborative
//! editing, and plugin extensibility without tightly coupling the renderer to
//! editing semantics.

use std::fmt;

use crate::word_document::WordDocument;
use jade_docx_layout::selection::SelectionRange;

pub mod adjust_font_size;
pub mod adjust_paragraph_indent;
pub mod apply_bold;
pub mod apply_italic;
pub mod apply_multilevel_list;
pub mod apply_strike;
pub mod apply_subscript;
pub mod apply_superscript;
pub mod apply_underline;
pub mod bookmark;
pub mod change_case;
pub mod clear_formatting;
pub mod composite;
pub mod delete_object;
pub mod delete_table;
pub mod delete_text;
pub mod document_edit;
pub mod edit_drawing;
mod edit_offset;
pub mod format_image;
pub mod format_text_box;
pub mod hyperlink;
pub mod insert_blank_page;
pub mod insert_caption;
pub mod insert_chart;
pub mod insert_citation;
pub mod insert_cover_page;
pub mod insert_endnote;
pub mod insert_equation;
pub mod insert_field;
pub mod insert_footnote;
pub mod insert_image;
pub mod insert_index;
pub mod insert_paragraph_break;
pub mod insert_section_break;
pub mod insert_shape;
pub mod insert_table;
pub mod insert_text;
pub mod insert_text_box;
pub mod insert_toc;
pub mod merge_table_cells;
pub(crate) mod paragraph_property_sync;
pub(crate) mod paragraph_undo;
pub mod range_notes;
pub(crate) mod raw_xml_undo;
pub mod replace_image;
pub mod reset_paragraph_formatting;
pub mod resize_table_column;
pub mod resize_table_row;
pub(crate) mod run_backup;
pub mod set_bold;
pub mod set_character_style;
pub mod set_document_protection;
pub mod set_document_setting;
pub mod set_document_theme;
pub mod set_drop_cap;
pub mod set_endnote_position;
pub mod set_font_color;
pub mod set_font_family;
pub mod set_font_size;
pub mod set_header_footer_variants;
pub mod set_highlight_color;
pub mod set_ink_annotations;
pub mod set_note_numbering;
pub mod set_numbering;
pub mod set_page_borders;
pub mod set_page_number_format;
pub mod set_page_setup;
pub mod set_paragraph_alignment;
pub mod set_paragraph_direction;
pub mod set_paragraph_indent;
pub mod set_paragraph_line_spacing;
pub mod set_paragraph_metrics;
pub mod set_paragraph_options;
pub mod set_paragraph_pagination;
pub mod set_paragraph_style;
pub mod set_paragraph_visuals;
pub mod set_ruby;
pub mod set_run_format;
pub mod set_typography;
pub mod set_watermark;
pub mod snapshot_edit;
pub mod split_table;
pub mod table_structure;
pub(crate) mod text_edit;
pub mod toggle_paragraph_space_before;
pub mod transform_object;
pub mod unmerge_table_cells;
pub mod update_chart;

/// Result type returned by command execution.
pub type CommandResult<T> = Result<T, CommandError>;

/// Paragraph-level footprint of an undo/redo, used to decide whether a history
/// action can take the fast incremental render path (dirty-from-paragraph seek +
/// band redraw) that ordinary typing uses, or whether it must fall back to a
/// conservative full render.
///
/// A history action is only eligible for the incremental path when it is
/// provably equivalent to editing the text of a *single existing paragraph*
/// without changing the paragraph count or reflowing structure. Anything else
/// (paragraph split/merge, table structure, multi-paragraph range restore,
/// document-wide settings) must render fully — correctness over speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryRenderFootprint {
    /// The action restored/re-applied text within a single paragraph at
    /// `index` and did not change the paragraph count. Safe for the same
    /// incremental dirty/seek/band-redraw path that typing uses.
    SingleParagraph { index: usize },
    /// The action is structural or spans multiple paragraphs. The caller must
    /// use the full render path.
    Structural,
}

/// Generic error type produced by editing commands.
#[derive(Debug)]
pub enum CommandError {
    DocumentUnavailable,
    ParagraphUnavailable,
    InvalidSelection,
    UnsupportedSelection,
    Custom(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::DocumentUnavailable => {
                write!(f, "document is not editable (missing body)")
            }
            CommandError::ParagraphUnavailable => {
                write!(f, "target paragraph is missing or not a text paragraph")
            }
            CommandError::InvalidSelection => {
                write!(f, "selection range is outside of the paragraph bounds")
            }
            CommandError::UnsupportedSelection => {
                write!(
                    f,
                    "operation is not yet implemented for multi-paragraph selections"
                )
            }
            CommandError::Custom(message) => write!(f, "command specific error: {message}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Runtime context received by every command execution.
///
/// The context intentionally exposes only the mutable pieces a command needs:
/// the parsed `WordDocument` and the logical text selection. Keeping the
/// surface area small prevents accidental cross-cutting dependencies and makes
/// it easier to replay commands in a different environment (e.g. collaborative
/// editing workers).
pub struct CommandContext<'a> {
    pub document: &'a mut WordDocument,
    pub selection: &'a mut Option<SelectionRange>,
}

impl<'a> CommandContext<'a> {
    pub fn new(document: &'a mut WordDocument, selection: &'a mut Option<SelectionRange>) -> Self {
        Self {
            document,
            selection,
        }
    }
}

/// Core trait implemented by every editing command.
pub trait Command {
    /// Human readable identifier (useful for debugging and telemetry).
    fn name(&self) -> &'static str;

    /// Applies the command, mutating the document and updating the selection.
    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()>;

    /// Reverts the command. Commands are responsible for restoring both the
    /// document and the selection to their previous state.
    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()>;

    /// Optional hook used when re-applying a previously undone command. In most
    /// cases it is identical to [`Self::apply`] so the default implementation simply
    /// delegates to it.
    fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.apply(ctx)
    }

    /// Whether the command should be recorded in undo/redo history after it
    /// successfully applies. Commands that detect they had no observable effect
    /// should return false so navigation keys like Backspace at document start
    /// do not pollute history or clear redo.
    fn should_record_history(&self) -> bool {
        true
    }

    /// Declares the paragraph-level footprint of undoing/redoing this command so
    /// the renderer can pick the incremental fast path for single-paragraph text
    /// edits and fall back to a full render for structural ones.
    ///
    /// The default is conservative (`Structural`): a command opts into the fast
    /// path only when it can prove that undo *and* redo touch the text of one
    /// existing paragraph without changing the paragraph count. This is queried
    /// *before* the action runs, so implementations must derive the footprint
    /// from their own recorded range/backup data, not from the post-mutation
    /// document.
    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        HistoryRenderFootprint::Structural
    }
}

/// Maximum number of undoable commands retained. Each command holds backup
/// state (cloned runs/paragraphs, and for some structural edits whole-table or
/// whole-document snapshots), so an unbounded stack grows memory without limit
/// over a long editing session. Oldest entries past this cap are dropped.
const MAX_UNDO_HISTORY: usize = 200;

/// Lightweight executor that owns the undo/redo stacks and orchestrates command
/// lifecycle.
#[derive(Default)]
pub struct CommandExecutor {
    undo_stack: Vec<Box<dyn Command>>, // newest command at the end
    redo_stack: Vec<Box<dyn Command>>,
    /// Commands applied since [`CommandExecutor::begin_group`], in application
    /// order. While this is `Some`, `execute` collects here instead of pushing
    /// onto the undo stack.
    group: Option<Vec<Box<dyn Command>>>,
    /// Nesting depth of `begin_group`; only the outermost pair records.
    group_depth: usize,
    /// Counts history entries ever recorded. Unlike the stack depth it moves
    /// even once the history is full and the oldest entry falls off.
    recorded: u64,
}

impl CommandExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes a command and clears the redo stack.
    #[cfg_attr(feature = "otel", tracing::instrument(skip_all, fields(command_name = command.name())))]
    pub fn execute(
        &mut self,
        mut command: Box<dyn Command>,
        ctx: &mut CommandContext<'_>,
    ) -> CommandResult<()> {
        command.apply(ctx)?;
        if command.should_record_history() {
            if let Some(group) = self.group.as_mut() {
                group.push(command);
            } else {
                self.record(command);
            }
        }
        Ok(())
    }

    fn record(&mut self, command: Box<dyn Command>) {
        self.recorded += 1;
        self.undo_stack.push(command);
        if self.undo_stack.len() > MAX_UNDO_HISTORY {
            let overflow = self.undo_stack.len() - MAX_UNDO_HISTORY;
            self.undo_stack.drain(0..overflow);
        }
        self.redo_stack.clear();
    }

    /// Starts collecting the commands executed from here on into one undo
    /// step. Word's table ribbon acts on the whole selected cell range, which
    /// jade applies as one command per row/cell; without a group each of those
    /// would need its own Ctrl+Z. Calls nest, and only the outermost pair
    /// records.
    pub fn begin_group(&mut self) {
        self.group_depth += 1;
        if self.group.is_none() {
            self.group = Some(Vec::new());
        }
    }

    /// Closes the group opened by [`CommandExecutor::begin_group`] and records
    /// it as a single history entry (a lone command is recorded as itself, and
    /// a group that applied nothing records nothing).
    pub fn end_group(&mut self) {
        self.group_depth = self.group_depth.saturating_sub(1);
        if self.group_depth > 0 {
            return;
        }
        let Some(children) = self.group.take() else {
            return;
        };
        match children.len() {
            0 => {}
            1 => {
                let command = children.into_iter().next().expect("one child");
                self.record(command);
            }
            _ => self.record(Box::new(composite::CompositeCommand::from_applied(
                children,
            ))),
        }
    }

    /// Rolls the open group back and closes it. Used when a command in the
    /// middle of a batch fails: the ones that already ran must come back off
    /// the document, since the caller reports the whole batch as rejected.
    pub fn abort_group(&mut self, ctx: &mut CommandContext<'_>) {
        self.group_depth = 0;
        let Some(mut children) = self.group.take() else {
            return;
        };
        while let Some(mut command) = children.pop() {
            let _ = command.undo(ctx);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Current number of undoable steps. Lets callers detect no-op commands:
    /// a command that succeeds without changing the document records no
    /// history, so the depth stays put.
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Changes whenever an edit is recorded in the history.
    pub fn history_revision(&self) -> u64 {
        self.recorded
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn next_undo_name(&self) -> Option<&'static str> {
        self.undo_stack.last().map(|command| command.name())
    }

    pub fn next_redo_name(&self) -> Option<&'static str> {
        self.redo_stack.last().map(|command| command.name())
    }

    pub fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<bool> {
        Ok(self.undo_with_footprint(ctx)?.is_some())
    }

    pub fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<bool> {
        Ok(self.redo_with_footprint(ctx)?.is_some())
    }

    /// Reverts the newest history entry and forgets it, leaving the redo
    /// history alone. For edits that were only ever provisional, like the
    /// live preview of a table border drag.
    pub fn retract_last(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<bool> {
        let Some(mut command) = self.undo_stack.pop() else {
            return Ok(false);
        };
        command.undo(ctx)?;
        Ok(true)
    }

    /// Like [`undo`], but also returns the paragraph-level render footprint of
    /// the reverted command so the caller can select the incremental fast path.
    /// The footprint is captured *before* `undo` runs, because undoing consumes
    /// the command's recorded backup state.
    pub fn undo_with_footprint(
        &mut self,
        ctx: &mut CommandContext<'_>,
    ) -> CommandResult<Option<HistoryRenderFootprint>> {
        let mut command = match self.undo_stack.pop() {
            Some(cmd) => cmd,
            None => return Ok(None),
        };
        let footprint = command.history_render_footprint();
        command.undo(ctx)?;
        self.redo_stack.push(command);
        Ok(Some(footprint))
    }

    /// Like [`redo`], but also returns the paragraph-level render footprint of
    /// the re-applied command.
    pub fn redo_with_footprint(
        &mut self,
        ctx: &mut CommandContext<'_>,
    ) -> CommandResult<Option<HistoryRenderFootprint>> {
        let mut command = match self.redo_stack.pop() {
            Some(cmd) => cmd,
            None => return Ok(None),
        };
        let footprint = command.history_render_footprint();
        command.redo(ctx)?;
        if command.should_record_history() {
            self.undo_stack.push(command);
        }
        Ok(Some(footprint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::insert_text::InsertTextCommand;
    use crate::document::{Body, BodyElement, Document, Paragraph, Run};
    use jade_docx_layout::selection::{SelectionRange, TextPosition};

    fn sample_document(text: &str) -> WordDocument {
        let paragraph = Paragraph {
            runs: vec![Run {
                text: text.to_string(),
                properties: None,
                image: None,
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            }],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        })
    }

    #[test]
    fn execute_populates_undo_stack_and_clears_redo() {
        let mut document = sample_document("Hello");
        let mut selection = Some(SelectionRange::collapsed(TextPosition {
            paragraph: 0,
            offset: 5,
        }));
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let mut executor = CommandExecutor::new();

        executor
            .execute(
                Box::new(InsertTextCommand::new(
                    SelectionRange::collapsed(TextPosition {
                        paragraph: 0,
                        offset: 5,
                    }),
                    " world".to_string(),
                    None,
                )),
                &mut ctx,
            )
            .expect("execute insert text");

        assert!(executor.can_undo());
        assert!(!executor.can_redo());

        // Pushing another command should clear redo stack.
        executor.redo_stack.push(Box::new(InsertTextCommand::new(
            SelectionRange::collapsed(TextPosition {
                paragraph: 0,
                offset: 11,
            }),
            "!".to_string(),
            None,
        )));

        executor
            .execute(
                Box::new(InsertTextCommand::new(
                    SelectionRange::collapsed(TextPosition {
                        paragraph: 0,
                        offset: 6,
                    }),
                    "beautiful ".to_string(),
                    None,
                )),
                &mut ctx,
            )
            .expect("execute second insert");

        assert!(executor.can_undo());
        assert!(
            !executor.can_redo(),
            "redo stack should be cleared after execute"
        );
    }

    #[test]
    fn undo_and_redo_restore_document_state() {
        let mut document = sample_document("Hello");
        let mut selection = Some(SelectionRange::collapsed(TextPosition {
            paragraph: 0,
            offset: 5,
        }));
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let mut executor = CommandExecutor::new();

        executor
            .execute(
                Box::new(InsertTextCommand::new(
                    SelectionRange::collapsed(TextPosition {
                        paragraph: 0,
                        offset: 5,
                    }),
                    " world".to_string(),
                    None,
                )),
                &mut ctx,
            )
            .expect("execute insert");

        assert!(executor.can_undo());
        assert!(!executor.can_redo());

        assert!(executor.undo(&mut ctx).expect("undo should succeed"));
        assert!(!executor.can_undo());
        assert!(executor.can_redo());

        let text_after_undo = ctx
            .document
            .paragraph(0)
            .expect("paragraph 0 exists")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(text_after_undo, "Hello");

        assert!(executor.redo(&mut ctx).expect("redo should succeed"));
        assert!(executor.can_undo());

        let text_after_redo = ctx
            .document
            .paragraph(0)
            .expect("paragraph 0 exists")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(text_after_redo, "Hello world");
    }

    #[test]
    fn retract_last_reverts_the_newest_entry_without_offering_a_redo() {
        let mut document = sample_document("Hello");
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let mut executor = CommandExecutor::new();
        let insert = |offset: usize, text: &str| {
            Box::new(InsertTextCommand::new(
                SelectionRange::collapsed(TextPosition {
                    paragraph: 0,
                    offset,
                }),
                text.to_string(),
                None,
            ))
        };

        executor
            .execute(insert(5, " world"), &mut ctx)
            .expect("execute first insert");
        executor.undo(&mut ctx).expect("undo first insert");
        let revision = executor.history_revision();
        executor
            .execute(insert(5, "!"), &mut ctx)
            .expect("execute provisional insert");
        assert_ne!(executor.history_revision(), revision);

        assert!(executor.retract_last(&mut ctx).expect("retract"));
        let text = ctx
            .document
            .paragraph(0)
            .expect("paragraph 0 exists")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(text, "Hello");
        assert!(!executor.can_undo());
        // The provisional edit is gone for good; executing it had already
        // replaced the earlier redo entry, as any new edit does.
        assert!(!executor.can_redo());
        assert!(!executor.retract_last(&mut ctx).expect("nothing to retract"));
    }

    /// A command that does not override `history_render_footprint` must default
    /// to `Structural`, keeping unknown/structural edits on the full-render path.
    struct NoopCommand;
    impl Command for NoopCommand {
        fn name(&self) -> &'static str {
            "noop"
        }
        fn apply(&mut self, _ctx: &mut CommandContext<'_>) -> CommandResult<()> {
            Ok(())
        }
        fn undo(&mut self, _ctx: &mut CommandContext<'_>) -> CommandResult<()> {
            Ok(())
        }
    }

    #[test]
    fn default_footprint_is_structural() {
        assert_eq!(
            NoopCommand.history_render_footprint(),
            HistoryRenderFootprint::Structural,
        );
    }

    #[test]
    fn executor_returns_none_footprint_when_stacks_empty() {
        let mut document = sample_document("Hello");
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        let mut executor = CommandExecutor::new();
        assert_eq!(executor.undo_with_footprint(&mut ctx).unwrap(), None);
        assert_eq!(executor.redo_with_footprint(&mut ctx).unwrap(), None);
    }

    fn paragraph_text(document: &WordDocument) -> String {
        document
            .paragraph(0)
            .expect("paragraph 0 exists")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect()
    }

    /// A ribbon action that spans a cell range runs as several commands but is
    /// one action to the user: one Ctrl+Z has to take all of it back.
    #[test]
    fn grouped_commands_undo_and_redo_as_one_step() {
        let mut document = sample_document("Hello");
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            executor.begin_group();
            for text in [" one", " two", " three"] {
                let range = SelectionRange::collapsed(TextPosition::new(
                    0,
                    paragraph_text(ctx.document).len(),
                ));
                executor
                    .execute(
                        Box::new(InsertTextCommand::new(range, text.to_string(), None)),
                        &mut ctx,
                    )
                    .expect("insert should succeed");
            }
            executor.end_group();
        }
        assert_eq!(paragraph_text(&document), "Hello one two three");
        assert_eq!(executor.undo_depth(), 1, "the group is one history entry");
        assert_eq!(executor.next_undo_name(), Some("insert_text"));

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        assert!(executor.undo(&mut ctx).expect("undo should succeed"));
        assert_eq!(paragraph_text(ctx.document), "Hello");
        assert!(!executor.can_undo());

        assert!(executor.redo(&mut ctx).expect("redo should succeed"));
        assert_eq!(paragraph_text(ctx.document), "Hello one two three");
    }

    /// A group of one is indistinguishable from an ungrouped command, and an
    /// empty group must not leave a history entry behind.
    #[test]
    fn groups_of_zero_or_one_command_record_plainly() {
        let mut document = sample_document("Hello");
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);

        executor.begin_group();
        executor.end_group();
        assert_eq!(executor.undo_depth(), 0);

        executor.begin_group();
        executor
            .execute(
                Box::new(InsertTextCommand::new(
                    SelectionRange::collapsed(TextPosition::new(0, 5)),
                    "!".to_string(),
                    None,
                )),
                &mut ctx,
            )
            .expect("insert should succeed");
        executor.end_group();
        assert_eq!(executor.undo_depth(), 1);
        assert_eq!(executor.next_undo_name(), Some("insert_text"));
        assert!(executor.undo(&mut ctx).expect("undo should succeed"));
        assert_eq!(paragraph_text(ctx.document), "Hello");
    }

    /// A command failing part-way through a batch must not leave the earlier
    /// ones applied: the caller reports the whole batch as rejected.
    #[test]
    fn aborting_a_group_rolls_the_applied_commands_back() {
        let mut document = sample_document("Hello");
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);

        executor.begin_group();
        executor
            .execute(
                Box::new(InsertTextCommand::new(
                    SelectionRange::collapsed(TextPosition::new(0, 5)),
                    " world".to_string(),
                    None,
                )),
                &mut ctx,
            )
            .expect("insert should succeed");
        assert_eq!(paragraph_text(ctx.document), "Hello world");

        executor.abort_group(&mut ctx);
        assert_eq!(paragraph_text(ctx.document), "Hello");
        assert_eq!(executor.undo_depth(), 0);
        assert!(!executor.can_redo());
    }
}
