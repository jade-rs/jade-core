use super::paragraph_property_sync::{
    snapshot_paragraph_properties_xml, sync_paragraph_numbering_xml, sync_paragraph_style_xml,
};
use super::raw_xml_undo::RawXmlUndo;
use super::set_numbering::{tracked_change_supplies_numbering, NUMBERING_OFF};
use super::text_edit::append_runs;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{Numbering, Paragraph};
use crate::word_document::{keep_declared_paragraph_properties, WordDocument};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

fn merge_paragraph_with_next(
    ctx: &mut CommandContext<'_>,
    paragraph_index: usize,
) -> CommandResult<()> {
    let before_count = ctx.document.paragraph_count();
    let next_index = paragraph_index.saturating_add(1);
    if next_index >= before_count {
        return Err(CommandError::ParagraphUnavailable);
    }

    let moved_runs = {
        let paragraph = ctx
            .document
            .paragraph_mut(next_index)
            .ok_or(CommandError::ParagraphUnavailable)?;
        std::mem::take(&mut paragraph.runs)
    };

    let _ = ctx.document.remove_paragraph_at(next_index);
    // remove_paragraph_at() can report false for paragraphs without node_id.
    // Rebuild surfaces and validate by paragraph-count delta instead.
    ctx.document.rebuild_paragraph_surfaces();
    let after_count = ctx.document.paragraph_count();
    if before_count != after_count.saturating_add(1) {
        return Err(CommandError::ParagraphUnavailable);
    }

    let paragraph = ctx
        .document
        .paragraph_mut(paragraph_index)
        .ok_or(CommandError::ParagraphUnavailable)?;
    append_runs(&mut paragraph.runs, moved_runs);
    ctx.document.sync_paragraph_text(paragraph_index);

    Ok(())
}

/// Undo state for paragraph breaks outside notes.
///
/// Undo merges the split back, restores the paragraph as it was and rolls
/// the round-trip XML back to exactly its state before the break. A tracked
/// break also marks the new paragraph mark inserted, taking a revision id,
/// so the revision id counter goes back too and a redo reuses the same ids.
/// Snapshotting the whole document for this cost ~130 ms per tracked Enter
/// on yokohama.
///
/// Merging alone is not an undo: the merged paragraph's XML is rebuilt from
/// its runs, which loses whatever else the split moved into the new
/// paragraph. A break at the start of a commented paragraph moves the
/// comment's range markers there, and the merge dropped them.
struct SplitUndo {
    paragraph: Paragraph,
    revision_id: u32,
    raw_xml_undo: RawXmlUndo,
}

impl SplitUndo {
    fn begin(ctx: &mut CommandContext<'_>, paragraph_index: usize) -> CommandResult<Self> {
        let paragraph = ctx
            .document
            .paragraph(paragraph_index)
            .cloned()
            .ok_or(CommandError::ParagraphUnavailable)?;
        let revision_id = ctx.document.revision_id_counter();
        let mut raw_xml_undo = RawXmlUndo::default();
        raw_xml_undo.begin(ctx);
        Ok(Self {
            paragraph,
            revision_id,
            raw_xml_undo,
        })
    }

    /// Stops recording once the breaks are in.
    fn finish(&mut self, ctx: &mut CommandContext<'_>) {
        self.raw_xml_undo.finish(ctx, true);
    }

    /// Merges the `count` paragraphs split off after `start` back into it.
    fn undo(
        mut self,
        ctx: &mut CommandContext<'_>,
        start: usize,
        count: usize,
    ) -> CommandResult<()> {
        self.raw_xml_undo.finish(ctx, true);
        let paragraph = self.paragraph;
        self.raw_xml_undo.undo(ctx, |ctx| {
            for offset in (0..count).rev() {
                merge_paragraph_with_next(ctx, start + offset)?;
            }
            // The merge rebuilds the paragraph from its halves, bound to XML
            // the roll-back replaces with the original element; restore the
            // original paragraph with it.
            *ctx.document
                .paragraph_mut(start)
                .ok_or(CommandError::ParagraphUnavailable)? = paragraph;
            Ok(())
        })?;
        ctx.document.restore_revision_id_counter(self.revision_id);
        ctx.document.rebuild_paragraph_surfaces();
        // The merge rebuilt the section ranges before the original paragraph,
        // with its section break, was put back: Enter then undo at the start
        // of a section left every range one paragraph too long, and the
        // sections laid out on screen were not the ones saved.
        if let Some(document) = ctx.document.document_mut() {
            WordDocument::rebuild_section_ranges(document);
        }
        Ok(())
    }
}

/// Splits the active paragraph at the caret position, creating a new paragraph
/// that inherits the original paragraph properties.
pub struct InsertParagraphBreakCommand {
    range: SelectionRange,
    previous_selection: Option<SelectionRange>,
    applied_start_paragraph: Option<usize>,
    previous_snapshot: Option<crate::word_document::WordDocumentSnapshot>,
    split_undo: Option<SplitUndo>,
}

impl InsertParagraphBreakCommand {
    /// Creates a new paragraph break command for the provided caret range.
    pub fn new(range: SelectionRange) -> Self {
        Self {
            range,
            previous_selection: None,
            applied_start_paragraph: None,
            previous_snapshot: None,
            split_undo: None,
        }
    }
}

/// Splits the active paragraph multiple times, inserting `count` paragraph breaks.
pub struct InsertParagraphBreaksCommand {
    range: SelectionRange,
    count: usize,
    previous_selection: Option<SelectionRange>,
    applied_start_paragraph: Option<usize>,
    applied_count: usize,
    previous_snapshot: Option<crate::word_document::WordDocumentSnapshot>,
    split_undo: Option<SplitUndo>,
}

impl InsertParagraphBreaksCommand {
    /// Creates a new multi-break command for the provided caret range.
    pub fn new(range: SelectionRange, count: usize) -> Self {
        Self {
            range,
            count: count.max(1),
            previous_selection: None,
            applied_start_paragraph: None,
            applied_count: 0,
            previous_snapshot: None,
            split_undo: None,
        }
    }
}

impl Command for InsertParagraphBreakCommand {
    fn name(&self) -> &'static str {
        "insert_paragraph_break"
    }

    /// Executes the paragraph split, cloning paragraph properties and inserting
    /// the trailing content into a new paragraph.
    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.applied_start_paragraph = None;
        self.previous_snapshot = None;
        self.split_undo = None;
        if !self.range.is_collapsed() {
            return Err(CommandError::UnsupportedSelection);
        }
        let recording = ctx.document.revision_session().is_some();

        let position = self.range.focus;
        let paragraph_count = ctx.document.paragraph_count();
        if position.paragraph >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }

        self.previous_selection = *ctx.selection;

        let paragraph = ctx
            .document
            .paragraph(position.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let paragraph_len: usize = paragraph.runs.iter().map(|run| run.text.len()).sum();
        let caret_offset = ctx
            .document
            .rendered_to_model_text_position(position)
            .offset
            .min(paragraph_len);

        let split_at_end = caret_offset >= paragraph_len;

        if ctx.document.is_note_paragraph_surface(position.paragraph) {
            self.previous_snapshot = Some(
                ctx.document
                    .snapshot_including_package()
                    .ok_or(CommandError::DocumentUnavailable)?,
            );
        } else {
            self.split_undo = Some(SplitUndo::begin(ctx, position.paragraph)?);
        }

        if !ctx
            .document
            .split_paragraph_at(position.paragraph, caret_offset)
        {
            if let Some(snapshot) = self.previous_snapshot.take() {
                ctx.document.restore_snapshot(snapshot);
            }
            if let Some(split) = self.split_undo.take() {
                split.undo(ctx, position.paragraph, 0)?;
            }
            return Err(CommandError::ParagraphUnavailable);
        }

        if recording {
            if let Err(error) = ctx
                .document
                .mark_inserted_paragraph_break(position.paragraph)
            {
                if let Some(snapshot) = self.previous_snapshot.take() {
                    ctx.document.restore_snapshot(snapshot);
                }
                if let Some(split) = self.split_undo.take() {
                    split.undo(ctx, position.paragraph, 1)?;
                }
                return Err(CommandError::Custom(error));
            }
        }

        self.applied_start_paragraph = Some(position.paragraph);

        // Word: Enter at the end of a paragraph starts the style's `w:next`
        // paragraph (a heading is followed by Normal). A split inside the
        // text keeps the style on both halves.
        if split_at_end {
            apply_next_style(ctx.document, position.paragraph + 1);
        }
        if let Some(split) = self.split_undo.as_mut() {
            split.finish(ctx);
        }

        let new_focus = TextPosition::new(position.paragraph + 1, 0);
        *ctx.selection = Some(SelectionRange::collapsed(new_focus));

        Ok(())
    }

    /// Restores the document by merging the inserted paragraph back and
    /// rolling its XML back.
    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(snapshot) = self.previous_snapshot.take() {
            self.applied_start_paragraph = None;
            ctx.document.restore_snapshot(snapshot);
            *ctx.selection = self.previous_selection;
            return Ok(());
        }
        let (Some(split), Some(start)) =
            (self.split_undo.take(), self.applied_start_paragraph.take())
        else {
            return Err(CommandError::Custom("missing state for undo".into()));
        };
        split.undo(ctx, start, 1)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

/// Gives the freshly split paragraph its style's `w:next` style: the style
/// id, the next style's paragraph and run properties in place of the ones
/// cloned from the source, and the `w:pStyle` in the package. Undo merges
/// the paragraph back, which drops these with it.
fn apply_next_style(document: &mut WordDocument, paragraph_index: usize) {
    let Some(styles) = document.styles_part.as_ref() else {
        return;
    };
    let current_style = document
        .paragraph(paragraph_index)
        .and_then(|paragraph| paragraph.properties.as_ref())
        .and_then(|props| props.style.as_deref().map(str::to_string));
    let Some(current_style) = current_style else {
        return;
    };
    let Some(next_style) = styles
        .get_style(&current_style)
        .and_then(|style| style.next_style.clone())
        .filter(|next| *next != current_style)
    else {
        return;
    };
    if styles.get_style(&next_style).is_none() {
        return;
    }
    // What the paragraph inherits, docDefaults and table style included, as
    // a reload resolves it: the style's own properties alone left a style
    // without spacing of its own with none, where the reload has the
    // document's default space after.
    let style_paragraph_props =
        document.inherited_paragraph_properties(paragraph_index, Some(&next_style));
    // The numbering a style's w:numPr gives is resolved into its paragraphs
    // at load, apart from its other properties.
    let style_numbering = styles
        .resolve_numbering(&next_style)
        .filter(|numbering| numbering.id.is_some());
    // The split copied the source's w:pPr, and only w:pStyle changes in it:
    // what it declares stays the paragraph's own, as a reload reads it.
    let declared: Vec<String> = document.paragraph_declared_property_tags(paragraph_index);
    let is_declared = |tag: &str| declared.iter().any(|declared| declared == tag);
    let declared_attributes = document.paragraph_declared_property_attributes(paragraph_index);
    let is_declared_attribute = |tag: &str, attribute: &str| {
        declared_attributes
            .iter()
            .any(|(declared, name)| declared == tag && name == attribute)
    };
    // The new paragraph has the source's mark, with any pending property
    // change on it. The reader numbers a paragraph by the list such a change
    // records, so a next style without numbering has to turn it off.
    let tracked_numbering = !is_declared("w:numPr")
        && style_numbering.is_none()
        && tracked_change_supplies_numbering(
            snapshot_paragraph_properties_xml(document, paragraph_index).as_ref(),
        );
    if let Some(paragraph) = document.paragraph_mut(paragraph_index) {
        let before = paragraph.properties.clone().unwrap_or_default();
        let mut props = style_paragraph_props.unwrap_or_default();
        props.style = Some(next_style.as_str().into());
        keep_declared_paragraph_properties(&mut props, &before, is_declared, is_declared_attribute);
        props.numbering = if is_declared("w:numPr") {
            before.numbering
        } else if tracked_numbering {
            Some(Numbering {
                id: Some(NUMBERING_OFF.to_string()),
                level: None,
            })
        } else {
            style_numbering
        };
        paragraph.properties = Some(props);
        // The split is at the end, so these runs are empty: the place the
        // next character goes. It takes the next style's formatting and none
        // of the source run's, in the package as in the model. The copied
        // `w:rPr` stayed, so typed text was saved in the source's formatting
        // (a heading's colour) while it showed in the next style's.
        for run in &mut paragraph.runs {
            run.properties = None;
        }
    }
    document.remove_paragraph_run_properties(paragraph_index, CHARACTER_FORMATTING_TAGS);
    document.apply_run_font_defaults_at(paragraph_index);
    sync_paragraph_style_xml(document, paragraph_index, Some(next_style.as_str()));
    if tracked_numbering {
        sync_paragraph_numbering_xml(document, paragraph_index, Some(NUMBERING_OFF), None);
    }
}

/// The `w:rPr` children of character formatting, which an Enter at the end of
/// a paragraph does not carry into the next style's paragraph.
const CHARACTER_FORMATTING_TAGS: &[&str] = &[
    "w:rStyle",
    "w:b",
    "w:bCs",
    "w:i",
    "w:iCs",
    "w:u",
    "w:strike",
    "w:dstrike",
    "w:caps",
    "w:smallCaps",
    "w:vanish",
    "w:specVanish",
    "w:shadow",
    "w:outline",
    "w:emboss",
    "w:imprint",
    "w:em",
    "w:effect",
    "w:vertAlign",
    "w:position",
    "w:spacing",
    "w:kern",
    "w:w",
    "w:fitText",
    "w:sz",
    "w:szCs",
    "w:rFonts",
    "w:lang",
    "w:color",
    "w:highlight",
    "w:shd",
    "w:bdr",
];

impl Command for InsertParagraphBreaksCommand {
    fn name(&self) -> &'static str {
        "insert_paragraph_breaks"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.applied_start_paragraph = None;
        self.applied_count = 0;
        self.previous_snapshot = None;
        self.split_undo = None;
        if !self.range.is_collapsed() {
            return Err(CommandError::UnsupportedSelection);
        }
        let recording = ctx.document.revision_session().is_some();

        if ctx
            .document
            .is_note_paragraph_surface(self.range.focus.paragraph)
        {
            self.previous_snapshot = Some(
                ctx.document
                    .snapshot_including_package()
                    .ok_or(CommandError::DocumentUnavailable)?,
            );
        } else {
            self.split_undo = Some(SplitUndo::begin(ctx, self.range.focus.paragraph)?);
        }

        self.previous_selection = *ctx.selection;
        self.applied_start_paragraph = Some(self.range.focus.paragraph);

        let mut collapsed = self.range;
        for _ in 0..self.count {
            let position = collapsed.focus;
            let Some(paragraph) = ctx.document.paragraph(position.paragraph) else {
                if let Some(split) = self.split_undo.take() {
                    split.undo(ctx, self.range.focus.paragraph, self.applied_count)?;
                    self.applied_start_paragraph = None;
                    self.applied_count = 0;
                }
                return Err(CommandError::ParagraphUnavailable);
            };
            let paragraph_len: usize = paragraph.runs.iter().map(|run| run.text.len()).sum();
            let caret_offset = ctx
                .document
                .rendered_to_model_text_position(position)
                .offset
                .min(paragraph_len);
            let split_at_end = caret_offset >= paragraph_len;

            if !ctx
                .document
                .split_paragraph_at(position.paragraph, caret_offset)
            {
                if let Some(snapshot) = self.previous_snapshot.take() {
                    ctx.document.restore_snapshot(snapshot);
                }
                if let Some(split) = self.split_undo.take() {
                    split.undo(ctx, self.range.focus.paragraph, self.applied_count)?;
                }
                self.applied_start_paragraph = None;
                self.applied_count = 0;
                return Err(CommandError::ParagraphUnavailable);
            }
            if recording {
                if let Err(error) = ctx
                    .document
                    .mark_inserted_paragraph_break(position.paragraph)
                {
                    if let Some(snapshot) = self.previous_snapshot.take() {
                        ctx.document.restore_snapshot(snapshot);
                    }
                    if let Some(split) = self.split_undo.take() {
                        split.undo(ctx, self.range.focus.paragraph, self.applied_count + 1)?;
                    }
                    self.applied_start_paragraph = None;
                    self.applied_count = 0;
                    return Err(CommandError::Custom(error));
                }
            }
            // Same rule as the single break: Enter at the end of the text
            // starts the style's `w:next` paragraph.
            if split_at_end {
                apply_next_style(ctx.document, position.paragraph + 1);
            }

            let new_focus = TextPosition::new(position.paragraph + 1, 0);
            collapsed = SelectionRange::collapsed(new_focus);
            self.applied_count = self.applied_count.saturating_add(1);
        }

        if let Some(split) = self.split_undo.as_mut() {
            split.finish(ctx);
        }
        *ctx.selection = Some(collapsed);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(snapshot) = self.previous_snapshot.take() {
            self.applied_start_paragraph = None;
            self.applied_count = 0;
            ctx.document.restore_snapshot(snapshot);
            *ctx.selection = self.previous_selection;
            return Ok(());
        }
        let (Some(split), Some(start)) =
            (self.split_undo.take(), self.applied_start_paragraph.take())
        else {
            return Err(CommandError::Custom("missing state for undo".into()));
        };
        split.undo(ctx, start, std::mem::take(&mut self.applied_count))?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::document::{
        Body, BodyElement, Document, MathRun, Paragraph, Run, RunField, RunFieldKind,
    };
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::{SelectionRange, TextPosition};
    use std::path::PathBuf;

    /// Resolves a fixture file relative to the crate.
    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("public")
            .join("fixtures")
            .join(name)
    }

    /// Returns a single-paragraph document containing `text`.
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

    /// Collects the plain text contents of a paragraph for assertions.
    fn paragraph_text(paragraph: &Paragraph) -> String {
        paragraph.runs.iter().map(|run| run.text.as_str()).collect()
    }

    /// A tracked Enter undoes through an XML journal and a paragraph backup,
    /// not a snapshot of the whole document: undo restores the saved bytes and
    /// the revision id counter, a redo reproduces the same ids, and the
    /// paragraph stays bound to its XML afterwards.
    #[test]
    fn tracked_paragraph_breaks_undo_without_a_document_snapshot() {
        let data = std::fs::read(fixture_path("contract.docx")).expect("read fixture");
        for count in [1, 2] {
            let mut doc =
                executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
            doc.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("reviewer".into()),
                date: Some("2026-10-05T00:00:00Z".into()),
                date_utc: None,
            }));
            let k = (0..doc.paragraph_count())
                .find(|&k| {
                    !doc.is_note_paragraph_surface(k)
                        && doc
                            .paragraph(k)
                            .is_some_and(|p| paragraph_text(p).len() >= 6)
                })
                .expect("a body paragraph with text");
            let original = doc.export_docx().expect("export");
            // Undo must give back the paragraph model as well as the XML; a
            // model rebuilt from the split halves would rewrite that XML on
            // the next edit.
            let original_paragraph = format!("{:?}", doc.paragraph(k));
            let paragraphs = doc.paragraph_count();
            let revision_id = doc.revision_id_counter();
            let range = SelectionRange::collapsed(
                doc.model_to_rendered_text_position(TextPosition::new(k, 3)),
            );
            let mut command: Box<dyn Command> = if count == 1 {
                Box::new(InsertParagraphBreakCommand::new(range))
            } else {
                Box::new(InsertParagraphBreaksCommand::new(range, count))
            };
            let mut selection = Some(range);

            // A redo takes the same revision ids. New paragraphs get fresh
            // `w14:paraId`s: node ids keep counting up rather than being reused.
            let without_para_ids = |doc: &WordDocument| {
                let xml = document_xml(doc);
                let mut out = String::with_capacity(xml.len());
                let mut rest = xml.as_str();
                while let Some(at) = rest.find(" w14:paraId=\"") {
                    out.push_str(&rest[..at]);
                    let after = &rest[at + " w14:paraId=\"".len()..];
                    rest = &after[after.find('"').map_or(after.len(), |end| end + 1)..];
                }
                out.push_str(rest);
                out
            };
            let mut applied = None;
            for round in ["apply", "redo"] {
                {
                    let mut ctx = CommandContext::new(&mut doc, &mut selection);
                    command.apply(&mut ctx).expect(round);
                }
                assert_eq!(
                    doc.paragraph_count(),
                    paragraphs + count,
                    "{count}: {round}"
                );
                let xml = without_para_ids(&doc);
                assert!(
                    xml.contains("<w:ins w:id="),
                    "{count}: the break is tracked"
                );
                match &applied {
                    None => applied = Some(xml),
                    Some(first) => assert!(xml == *first, "{count}: a redo matches the apply"),
                }
                {
                    let mut ctx = CommandContext::new(&mut doc, &mut selection);
                    command.undo(&mut ctx).expect("undo");
                }
                assert_eq!(doc.paragraph_count(), paragraphs, "{count}: {round} undone");
                assert_eq!(
                    format!("{:?}", doc.paragraph(k)),
                    original_paragraph,
                    "{count}"
                );
                assert_eq!(doc.revision_id_counter(), revision_id, "{count}");
                assert!(
                    doc.export_docx().expect("export") == original,
                    "{count}: {round} undone exports the original bytes"
                );
            }

            doc.set_revision_session(None);
            let marker = format!("改行を戻した後{count}");
            let mut insert = crate::commands::insert_text::InsertTextCommand::new(
                SelectionRange::collapsed(
                    doc.model_to_rendered_text_position(TextPosition::new(k, 0)),
                ),
                marker.clone(),
                None,
            );
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            insert.apply(&mut ctx).expect("type");
            assert!(
                document_xml(&doc).contains(&marker),
                "{count}: typed text is saved"
            );
        }
    }

    /// A tracked break that cannot be marked (here: no round-trip XML) fails
    /// without leaving its split behind, and without a document snapshot.
    #[test]
    fn failed_tracked_paragraph_break_leaves_the_document_unchanged() {
        for count in [1, 2] {
            let mut doc = sample_document("abcdef");
            doc.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("reviewer".into()),
                date: None,
                date_utc: None,
            }));
            let range = SelectionRange::collapsed(TextPosition::new(0, 3));
            let mut selection = Some(range);
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            let (result, snapshot) = if count == 1 {
                let mut command = InsertParagraphBreakCommand::new(range);
                (command.apply(&mut ctx), command.previous_snapshot.is_some())
            } else {
                let mut command = InsertParagraphBreaksCommand::new(range, count);
                (command.apply(&mut ctx), command.previous_snapshot.is_some())
            };
            assert!(result.is_err(), "{count}: no raw XML to mark");
            assert!(!snapshot, "{count}");
            assert_eq!(doc.paragraph_count(), 1, "{count}");
            assert_eq!(
                paragraph_text(doc.paragraph(0).expect("paragraph")),
                "abcdef"
            );
        }
    }

    #[test]
    fn undo_join_then_undo_split_preserves_exported_table_paragraphs() {
        use crate::commands::delete_text::{DeleteDirection, DeleteTextCommand};
        use crate::commands::insert_text::InsertTextCommand;

        for (fixture, label) in [
            ("table-complex-nested-merges.docx", "DEEP_2_1"),
            ("table-complex-pagination.docx", "TALL_0_30"),
        ] {
            for replace in [false, true] {
                let data = std::fs::read(fixture_path(fixture)).expect("read fixture");
                let mut doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                    .expect("load fixture");
                let texts = |doc: &WordDocument| {
                    (0..doc.paragraph_count())
                        .map(|p| paragraph_text(doc.paragraph(p).unwrap()))
                        .collect::<Vec<_>>()
                };
                let original = texts(&doc);
                let p = original
                    .iter()
                    .position(|text| text.starts_with(label))
                    .unwrap();
                let range = SelectionRange::collapsed(TextPosition::new(p, 5));
                let mut selection = Some(range);
                let mut history = CommandExecutor::new();
                history
                    .execute(
                        Box::new(InsertParagraphBreakCommand::new(range)),
                        &mut CommandContext::new(&mut doc, &mut selection),
                    )
                    .expect("split");
                let tail_id = doc.paragraph(p + 1).unwrap().node_id;
                let join: Box<dyn Command> = if replace {
                    Box::new(InsertTextCommand::new(
                        SelectionRange::new(TextPosition::new(p, 5), TextPosition::new(p + 1, 0)),
                        "replacement".into(),
                        None,
                    ))
                } else {
                    Box::new(DeleteTextCommand::new(
                        SelectionRange::collapsed(TextPosition::new(p + 1, 0)),
                        DeleteDirection::Backward,
                    ))
                };
                history
                    .execute(join, &mut CommandContext::new(&mut doc, &mut selection))
                    .expect("join");
                history
                    .undo(&mut CommandContext::new(&mut doc, &mut selection))
                    .expect("undo join");
                assert_eq!(
                    doc.paragraph(p + 1).unwrap().node_id,
                    tail_id,
                    "undo must bind the restored model paragraph to its restored XML node"
                );
                history
                    .undo(&mut CommandContext::new(&mut doc, &mut selection))
                    .expect("undo split");
                assert_eq!(texts(&doc), original);
                let exported = doc.export_docx().expect("export");
                let reloaded =
                    executor::block_on(WordDocument::load(&exported, &Options::default()))
                        .expect("reload");
                assert_eq!(
                    texts(&reloaded),
                    original,
                    "{fixture}, replacement={replace}"
                );
            }
        }
    }

    #[test]
    fn tracked_paragraph_splits_create_rejectable_marks_and_preserve_old_mark() {
        for multiple in [false, true] {
            let data = std::fs::read(fixture_path("numbering-with-review.docx"))
                .expect("read review fixture");
            let mut word_doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                .expect("load review fixture");
            word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
                author: Some("reviewer".into()),
                date: Some("2026-09-25T00:00:00Z".into()),
                date_utc: None,
            }));
            let original_count = word_doc.paragraph_count();
            let original_xml = document_xml(&word_doc);
            let range = SelectionRange::collapsed(TextPosition::new(1, 2));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();
            let command: Box<dyn Command> = if multiple {
                Box::new(InsertParagraphBreaksCommand::new(range, 2))
            } else {
                Box::new(InsertParagraphBreakCommand::new(range))
            };
            executor
                .execute(
                    command,
                    &mut CommandContext::new(&mut word_doc, &mut selection),
                )
                .expect("tracked paragraph split");
            let inserted = if multiple { 2 } else { 1 };
            assert_eq!(word_doc.paragraph_count(), original_count + inserted);
            let exported = word_doc.export_docx().expect("export");
            let reloaded = executor::block_on(WordDocument::load(&exported, &Options::default()))
                .expect("reload");
            let mark_ids = |index| {
                let raw = reloaded.raw_document.as_ref().expect("raw XML");
                let paragraph_id = reloaded
                    .paragraph(index)
                    .expect("paragraph")
                    .node_id
                    .expect("id");
                let paragraph = raw.element(paragraph_id).expect("XML paragraph");
                paragraph
                    .children
                    .iter()
                    .filter_map(|child| match child {
                        crate::docx_xml::XmlNode::Element(element) if element.name == "w:pPr" => {
                            Some(element)
                        }
                        _ => None,
                    })
                    .flat_map(|p_pr| p_pr.children.iter())
                    .filter_map(|child| match child {
                        crate::docx_xml::XmlNode::Element(element) if element.name == "w:rPr" => {
                            Some(element)
                        }
                        _ => None,
                    })
                    .flat_map(|r_pr| r_pr.children.iter())
                    .filter_map(|child| match child {
                        crate::docx_xml::XmlNode::Element(element) if element.name == "w:ins" => {
                            element
                                .attributes
                                .iter()
                                .find(|attr| attr.name == "w:id")
                                .map(|attr| attr.value.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let new_marks = (1..1 + inserted).flat_map(mark_ids).collect::<Vec<_>>();
            assert_eq!(new_marks.len(), inserted);
            assert!(new_marks.iter().all(|id| id != "2"));
            assert_eq!(mark_ids(1 + inserted), vec!["2"]);
            let xml = document_xml(&reloaded);
            assert!(xml.contains("w:author=\"reviewer\""));
            assert!(xml.contains("w:date=\"2026-09-25T00:00:00Z\""));
            let mut decision_doc =
                executor::block_on(WordDocument::load(&exported, &Options::default()))
                    .expect("load decision copy");
            assert_eq!(
                decision_doc.reject_revisions(None, None, None, Some(&new_marks[0])),
                Ok(1)
            );
            assert_eq!(
                decision_doc.paragraph_count(),
                original_count + inserted - 1
            );
            assert!(document_xml(&decision_doc).contains("w:id=\"2\""));

            let mut accepted_doc =
                executor::block_on(WordDocument::load(&exported, &Options::default()))
                    .expect("load acceptance copy");
            assert_eq!(
                accepted_doc.accept_revisions(None, None, None, Some(&new_marks[0])),
                Ok(1)
            );
            assert_eq!(accepted_doc.paragraph_count(), original_count + inserted);
            assert!(document_xml(&accepted_doc).contains("w:id=\"2\""));

            executor
                .undo(&mut CommandContext::new(&mut word_doc, &mut selection))
                .expect("undo");
            assert_eq!(word_doc.paragraph_count(), original_count);
            assert_eq!(document_xml(&word_doc), original_xml);
            assert_eq!(selection, Some(range));
        }
    }

    #[test]
    fn tracked_split_without_raw_xml_rolls_back_before_entering_history() {
        let mut word_doc = sample_document("HelloWorld");
        word_doc.set_revision_session(Some(crate::word_document::RevisionSession {
            author: Some("reviewer".into()),
            date: None,
            date_utc: None,
        }));
        let range = SelectionRange::collapsed(TextPosition::new(0, 5));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let error = executor
            .execute(
                Box::new(InsertParagraphBreakCommand::new(range)),
                &mut CommandContext::new(&mut word_doc, &mut selection),
            )
            .expect_err("tracked split needs a raw XML surface");
        assert!(error.to_string().contains("raw XML"));
        assert_eq!(word_doc.paragraph_count(), 1);
        assert_eq!(
            paragraph_text(word_doc.paragraph(0).expect("paragraph")),
            "HelloWorld"
        );
        assert_eq!(selection, Some(range));
        assert!(!executor.can_undo());
    }

    fn document_xml(doc: &WordDocument) -> String {
        use std::io::Read;
        let exported = doc.export_docx().expect("export");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document XML")
            .read_to_string(&mut xml)
            .expect("read XML");
        xml
    }

    fn build_docx_with_body(body_xml: &str) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let document_xml = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
            for (path, content) in [
                (
                    "[Content_Types].xml",
                    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
                ),
                ("word/document.xml", document_xml.as_str()),
            ] {
                zip.start_file(path, options).expect("start entry");
                zip.write_all(content.as_bytes()).expect("write entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    /// Enter before a complex field moves the whole field into the new
    /// paragraph with its `fldChar` runs. Its result run used to be rebuilt
    /// from the model instead, which dropped the `end`: the field stayed open
    /// on reload and swallowed the text of the paragraphs after it.
    #[test]
    fn a_break_before_a_complex_field_keeps_the_field_closed() {
        for (code, result) in [
            ("AUTONUMOUT", ""),
            ("AUTONUMOUT", "<w:r><w:t>78</w:t></w:r>"),
            ("MERGEFIELD name", "<w:r><w:t>78</w:t></w:r>"),
        ] {
            let data = build_docx_with_body(&format!(
                r#"<w:p><w:r><w:t>mmmm</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {code} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{result}<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p><w:p><w:r><w:t>次</w:t></w:r></w:p>"#
            ));
            // Before the paragraph's text, inside it, at the field, inside
            // an editable result (the field then spans both paragraphs, as
            // Word writes it) and after the field.
            for offset in [0, "mm".len(), "mmmm".len(), "mmmm7".len(), "mmmm78".len()] {
                let mut doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                    .expect("load");
                eprintln!(
                    "DBG {result:?} {offset} {:?}",
                    doc.paragraph(0)
                        .unwrap()
                        .runs
                        .iter()
                        .map(|r| (r.text.clone(), r.node_id, r.field.is_some()))
                        .collect::<Vec<_>>()
                );
                let range = SelectionRange::collapsed(
                    doc.model_to_rendered_text_position(TextPosition::new(0, offset)),
                );
                let mut selection = Some(range);
                let mut executor = CommandExecutor::new();
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                executor
                    .execute(Box::new(InsertParagraphBreakCommand::new(range)), &mut ctx)
                    .expect("break");

                let xml = document_xml(&doc);
                let kinds: Vec<&str> = xml
                    .match_indices("w:fldCharType=\"")
                    .map(|(at, found)| {
                        let rest = &xml[at + found.len()..];
                        &rest[..rest.find('"').expect("closing quote")]
                    })
                    .collect();
                assert_eq!(
                    kinds,
                    ["begin", "separate", "end"],
                    "{code} {result:?} at {offset}: {xml}"
                );
                let reloaded = executor::block_on(WordDocument::load(
                    &doc.export_docx().expect("export"),
                    &Options::default(),
                ))
                .expect("reload");
                assert_eq!(
                    reloaded.paragraph_count(),
                    3,
                    "{code} {result:?} at {offset}"
                );
                assert_eq!(
                    paragraph_text(reloaded.paragraph(2).expect("last paragraph")),
                    "次",
                    "{code} {result:?} at {offset}: the paragraph after the field keeps its text"
                );
            }
        }
    }

    /// Undoing Enter gives the paragraph back exactly: its markup, with the
    /// comment and bookmark markers in their places and no formatting run
    /// left in an emptied first half, and its model.
    ///
    /// Undo used to merge the halves and rebuild the paragraph from its runs.
    /// Enter at the start of a commented paragraph moves the comment's range
    /// markers into the new paragraph, and the merge dropped them: the
    /// comment lost its anchor, and undoing the comment itself then failed on
    /// markup it no longer matched.
    #[test]
    fn undoing_paragraph_breaks_restores_the_paragraph_exactly() {
        let data = build_docx_with_body(
            r#"<w:p><w:pPr/><w:commentRangeStart w:id="1"/><w:bookmarkStart w:id="0" w:name="_GoBack"/><w:bookmarkEnd w:id="0"/><w:r><w:rPr><w:b/></w:rPr><w:t>秘密保持</w:t></w:r><w:bookmarkStart w:id="2" w:name="term"/><w:r><w:t>契約書</w:t></w:r><w:bookmarkEnd w:id="2"/><w:r><w:t>は営業上</w:t></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r></w:p><w:p><w:r><w:t>次</w:t></w:r></w:p>"#,
        );
        for offset in [
            0,
            "秘密".len(),
            "秘密保持".len(),
            "秘密保持契約書は営業上".len(),
        ] {
            for count in [1, 3] {
                let mut doc = executor::block_on(WordDocument::load(&data, &Options::default()))
                    .expect("load");
                let original = document_xml(&doc);
                let original_paragraph = format!("{:?}", doc.paragraph(0));
                let paragraphs = doc.paragraph_count();
                let range = SelectionRange::collapsed(
                    doc.model_to_rendered_text_position(TextPosition::new(0, offset)),
                );
                let command: Box<dyn Command> = if count == 1 {
                    Box::new(InsertParagraphBreakCommand::new(range))
                } else {
                    Box::new(InsertParagraphBreaksCommand::new(range, count))
                };
                let mut selection = Some(range);
                let mut executor = CommandExecutor::new();
                let mut ctx = CommandContext::new(&mut doc, &mut selection);
                executor.execute(command, &mut ctx).expect("break");
                assert!(executor.undo(&mut ctx).expect("undo"), "{offset}/{count}");

                assert_eq!(doc.paragraph_count(), paragraphs, "{offset}/{count}");
                assert_eq!(
                    format!("{:?}", doc.paragraph(0)),
                    original_paragraph,
                    "{offset}/{count}: the paragraph model comes back"
                );
                assert_eq!(
                    document_xml(&doc),
                    original,
                    "{offset}/{count}: the markup comes back"
                );
            }
        }
    }

    #[test]
    /// Splitting inside a simple paragraph should move trailing text into a new block.
    fn splits_paragraph_at_caret() {
        let mut word_doc = sample_document("HelloWorld");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(
            0,
            "Hello".len(),
        )));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreakCommand::new(SelectionRange::collapsed(
                        TextPosition::new(0, "Hello".len()),
                    ))),
                    &mut ctx,
                )
                .unwrap();
        }

        let body = word_doc.document.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.elements.len(), 2);

        let first = match &body.elements[0] {
            BodyElement::Paragraph(paragraph) => paragraph.as_ref(),
            _ => panic!("expected paragraph"),
        };
        let second = match &body.elements[1] {
            BodyElement::Paragraph(paragraph) => paragraph.as_ref(),
            _ => panic!("expected paragraph"),
        };

        assert_eq!(paragraph_text(first), "Hello");
        assert_eq!(paragraph_text(second), "World");

        let new_selection = selection.unwrap();
        assert_eq!(new_selection.focus.paragraph, 1);
        assert_eq!(new_selection.focus.offset, 0);
    }

    #[test]
    /// Undo must restore the original paragraph and caret position.
    fn undo_restores_original_paragraph() {
        let mut word_doc = sample_document("HelloWorld");
        let mut selection = Some(SelectionRange::collapsed(TextPosition::new(
            0,
            "Hello".len(),
        )));
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreakCommand::new(SelectionRange::collapsed(
                        TextPosition::new(0, "Hello".len()),
                    ))),
                    &mut ctx,
                )
                .unwrap();
        }

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            assert!(executor.undo(&mut ctx).unwrap());
        }

        let body = word_doc.document.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.elements.len(), 1);
        let only = match &body.elements[0] {
            BodyElement::Paragraph(paragraph) => paragraph.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph_text(only), "HelloWorld");
    }

    #[test]
    fn enter_at_the_end_of_a_heading_starts_its_next_style() {
        let data = std::fs::read(fixture_path("toc-direct-outline-levels.docx"))
            .expect("read toc fixture");
        let mut doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");
        let heading = (0..doc.paragraph_count())
            .find(|index| {
                doc.paragraph(*index)
                    .and_then(|paragraph| paragraph.properties.as_ref())
                    .and_then(|props| props.style.as_deref())
                    == Some("Heading1")
            })
            .expect("a Heading1 paragraph");
        let heading_len: usize = doc
            .paragraph(heading)
            .expect("heading")
            .runs
            .iter()
            .map(|run| run.text.len())
            .sum();
        let style_of = |doc: &WordDocument, index: usize| {
            doc.paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.style.as_deref().map(str::to_string))
        };

        // Enter at the end: the new paragraph takes `w:next` (Normal).
        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreakCommand::new(SelectionRange::collapsed(
                        TextPosition::new(heading, heading_len),
                    ))),
                    &mut ctx,
                )
                .expect("split at end");
        }
        assert_eq!(style_of(&doc, heading).as_deref(), Some("Heading1"));
        assert_eq!(style_of(&doc, heading + 1).as_deref(), Some("Normal"));
        let package = {
            use std::io::Read;
            let exported = doc.export_docx().expect("export");
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(exported)).expect("zip");
            let mut xml = String::new();
            archive
                .by_name("word/document.xml")
                .expect("document.xml")
                .read_to_string(&mut xml)
                .expect("read");
            xml
        };
        let heading_at = package
            .find("w:val=\"Heading1\"")
            .expect("heading style in package");
        let after = &package[heading_at..];
        assert!(
            after.contains("<w:pStyle w:val=\"Normal\""),
            "the new paragraph carries w:pStyle Normal: {}",
            &after[..after.len().min(600)]
        );

        // Undo merges the paragraph back.
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.undo(&mut ctx).expect("undo");
        }
        assert_eq!(style_of(&doc, heading).as_deref(), Some("Heading1"));

        // The batched command the editor uses for Enter applies the same rule.
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreaksCommand::new(
                        SelectionRange::collapsed(TextPosition::new(heading, heading_len)),
                        1,
                    )),
                    &mut ctx,
                )
                .expect("batched split at end");
        }
        assert_eq!(style_of(&doc, heading + 1).as_deref(), Some("Normal"));
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor.undo(&mut ctx).expect("undo batched");
        }

        // Enter inside the heading keeps Heading1 on both halves.
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreakCommand::new(SelectionRange::collapsed(
                        TextPosition::new(heading, 3),
                    ))),
                    &mut ctx,
                )
                .expect("split inside");
        }
        assert_eq!(style_of(&doc, heading).as_deref(), Some("Heading1"));
        assert_eq!(style_of(&doc, heading + 1).as_deref(), Some("Heading1"));
    }

    #[test]
    /// Splitting a real document should copy paragraph properties and produce unique paraIds.
    fn splits_loaded_document_and_clones_properties() {
        let data = std::fs::read(fixture_path("hello.docx")).expect("read hello fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let initial_selection = SelectionRange::collapsed(TextPosition::new(0, "Hel".len()));
        let mut selection = Some(initial_selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreakCommand::new(initial_selection)),
                    &mut ctx,
                )
                .unwrap();
        }

        let body = word_doc.document.as_ref().unwrap().body.as_ref().unwrap();
        assert!(body.elements.len() >= 2);

        let first = body.elements[0].as_paragraph().unwrap();
        let second = body.elements[1].as_paragraph().unwrap();

        assert_eq!(
            first
                .properties
                .as_ref()
                .and_then(|props| props.alignment.clone()),
            second
                .properties
                .as_ref()
                .and_then(|props| props.alignment.clone()),
            "paragraph properties should be cloned"
        );

        let raw = word_doc.raw_document.as_ref().expect("raw xml");
        let first_id = first.node_id.expect("first paragraph id");
        let second_id = second.node_id.expect("second paragraph id");
        let first_xml = raw.element(first_id).expect("first xml paragraph");
        let second_xml = raw.element(second_id).expect("second xml paragraph");

        let first_para_id = first_xml
            .attributes
            .iter()
            .find(|attr| attr.name == "w14:paraId" || attr.name == "w:paraId")
            .map(|attr| attr.value.clone());
        let second_para_id = second_xml
            .attributes
            .iter()
            .find(|attr| attr.name == "w14:paraId" || attr.name == "w:paraId")
            .map(|attr| attr.value.clone());

        if let (Some(first_id), Some(second_id)) = (first_para_id, second_para_id) {
            assert_ne!(first_id, second_id, "paraId must differ");
        }

        let new_selection = selection.unwrap();
        assert_eq!(new_selection.focus.paragraph, 1);
        assert_eq!(new_selection.focus.offset, 0);
    }

    #[test]
    fn split_preserves_floating_image_runs() {
        let data = std::fs::read(fixture_path("image-floating.docx"))
            .expect("read image-floating fixture");
        let mut word_doc =
            executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx");

        let selection = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut state_selection = Some(selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut state_selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreakCommand::new(selection)),
                    &mut ctx,
                )
                .expect("command success");
        }

        let second = word_doc.paragraph(1).expect("second paragraph");
        assert!(
            second.runs.iter().any(|run| run.image.is_some()),
            "expected floating image run to remain after split"
        );
    }

    #[test]
    fn split_preserves_math_runs_in_trailing_content() {
        let paragraph = Paragraph {
            runs: vec![
                Run {
                    text: "Hello".to_string(),
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
                },
                Run {
                    text: String::new(),
                    properties: None,
                    image: None,
                    text_box: None,
                    math: Some(MathRun {
                        raw_omml: "<m:oMath/>".to_string(),
                        ..MathRun::default()
                    }),
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
                },
                Run {
                    text: "World".to_string(),
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
                },
            ],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        });

        let selection = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut state_selection = Some(selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut state_selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreakCommand::new(selection)),
                    &mut ctx,
                )
                .expect("command success");
        }

        let second = word_doc.paragraph(1).expect("second paragraph");
        assert!(
            second
                .runs
                .iter()
                .any(|run| run.math.as_ref().is_some_and(|m| !m.is_empty())),
            "expected trailing math run to remain after split"
        );
        assert_eq!(paragraph_text(second), "World");
    }

    #[test]
    fn split_preserves_field_runs_in_trailing_content() {
        let paragraph = Paragraph {
            runs: vec![
                Run {
                    text: "Hello".to_string(),
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
                },
                Run {
                    text: String::new(),
                    properties: None,
                    image: None,
                    text_box: None,
                    math: None,
                    field: Some(RunField {
                        kind: RunFieldKind::PageNumber,
                        cached_text: None,
                        source_field_id: None,
                        resolved_value: None,
                    }),
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
                },
                Run {
                    text: "World".to_string(),
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
                },
            ],
            properties: None,
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        };

        let mut word_doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: vec![BodyElement::from_paragraph(paragraph)],
            }),
            ..Document::default()
        });

        let selection = SelectionRange::collapsed(TextPosition::new(0, "Hello".len()));
        let mut state_selection = Some(selection);
        let mut executor = CommandExecutor::new();

        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut state_selection);
            executor
                .execute(
                    Box::new(InsertParagraphBreakCommand::new(selection)),
                    &mut ctx,
                )
                .expect("command success");
        }

        let second = word_doc.paragraph(1).expect("second paragraph");
        assert!(
            second.runs.iter().any(|run| run.field.is_some()),
            "expected trailing field run to remain after split"
        );
        assert_eq!(paragraph_text(second), "World");
    }
}
