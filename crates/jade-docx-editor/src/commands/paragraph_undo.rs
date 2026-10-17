//! Undo for edits that rewrite a run of paragraphs in place.
//!
//! Marking a selection deleted under change tracking, clearing text across
//! table cells, accepting a change or anchoring a comment rewrite paragraphs
//! without adding or removing any. They used to keep a snapshot of the whole
//! document for undo, which on a 149-page document copied the body model and
//! `document.xml` (~40 ms) for an edit to two or three paragraphs. These
//! paragraphs as they were, a journal of the round-trip XML and the revision
//! id counter put the document back just as exactly.

use super::raw_xml_undo::RawXmlUndo;
use super::{CommandContext, CommandError, CommandResult};
use crate::document::Paragraph;
use crate::word_document::PackageStateSnapshot;

/// What an in-place edit of a run of paragraphs can change.
#[derive(Debug)]
pub(crate) struct ParagraphRangeUndo {
    start: usize,
    /// `None` for a block equation, which no paragraph edit reaches.
    paragraphs: Vec<Option<Paragraph>>,
    /// Kept for edits that also write the parts outside `document.xml`
    /// (comments, notes); see [`Self::begin_with_package`].
    package: Option<Box<PackageStateSnapshot>>,
    next_revision_id: u32,
    raw_xml: RawXmlUndo,
}

impl ParagraphRangeUndo {
    /// Starts an edit of the paragraphs `start..=end` (none for `None`).
    ///
    /// The paragraphs have to keep their XML in `document.xml`: a note's
    /// lives in its own part, which the journal does not cover.
    ///
    /// A block equation in the range is skipped rather than refused: it has
    /// no paragraph for an edit to change, and its XML is in the journal. Any
    /// other surface without a paragraph is stale, and fails.
    pub(crate) fn begin(
        ctx: &mut CommandContext<'_>,
        range: Option<(usize, usize)>,
    ) -> CommandResult<Self> {
        let (start, paragraphs) = match range {
            Some((start, end)) => (
                start,
                (start..=end)
                    .map(|index| {
                        if ctx.document.is_math_surface(index) {
                            return Ok(None);
                        }
                        ctx.document
                            .paragraph(index)
                            .cloned()
                            .map(Some)
                            .ok_or(CommandError::ParagraphUnavailable)
                    })
                    .collect::<CommandResult<Vec<_>>>()?,
            ),
            None => (0, Vec::new()),
        };
        let mut raw_xml = RawXmlUndo::default();
        raw_xml.begin(ctx);
        Ok(Self {
            start,
            paragraphs,
            package: None,
            next_revision_id: ctx.document.revision_id_counter(),
            raw_xml,
        })
    }

    /// [`Self::begin`] for an edit that also writes the package: comments,
    /// their parts and relationships, note parts. Copying those is cheap next
    /// to the body model and `document.xml`.
    pub(crate) fn begin_with_package(
        ctx: &mut CommandContext<'_>,
        range: Option<(usize, usize)>,
    ) -> CommandResult<Self> {
        let mut undo = Self::begin(ctx, range)?;
        undo.package = Some(Box::new(ctx.document.package_state()));
        Ok(undo)
    }

    /// Ends a successful edit.
    pub(crate) fn finish(&mut self, ctx: &mut CommandContext<'_>) {
        self.raw_xml.finish(ctx, true);
    }

    /// Puts the document back after an edit that failed part-way.
    pub(crate) fn abort(mut self, ctx: &mut CommandContext<'_>) {
        self.raw_xml.abort(ctx);
        self.restore_model(ctx);
    }

    pub(crate) fn undo(mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let raw_xml = std::mem::take(&mut self.raw_xml);
        // The paragraphs go back whole: they match the XML as it was before
        // the edit, which is what the journal rolls the XML back to.
        raw_xml.undo(ctx, |ctx| {
            self.restore_model(ctx);
            Ok(())
        })
    }

    fn restore_model(self, ctx: &mut CommandContext<'_>) {
        for (offset, paragraph) in self.paragraphs.into_iter().enumerate() {
            let Some(paragraph) = paragraph else {
                continue;
            };
            if let Some(target) = ctx.document.paragraph_mut(self.start + offset) {
                *target = paragraph;
            }
        }
        if let Some(package) = self.package {
            ctx.document.restore_package_state(*package);
        }
        ctx.document
            .restore_revision_id_counter(self.next_revision_id);
        ctx.document.rebuild_paragraph_surfaces();
    }
}

/// Whether `index` is a body paragraph with comment anchors, whose edits
/// re-syncing its runs cannot undo exactly (see the delete and insert text
/// commands).
pub(crate) fn commented_body_paragraph(ctx: &CommandContext<'_>, index: usize) -> bool {
    !ctx.document.is_note_paragraph_surface(index)
        && ctx.document.paragraph(index).is_some_and(|paragraph| {
            !paragraph.comment_ranges.is_empty() || !paragraph.comment_markers.is_empty()
        })
}
