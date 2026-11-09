use super::delete_text::{DeleteDirection, DeleteTextCommand};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::objects::ObjectClip;
use crate::word_document::{
    FloatingObjectKind, FloatingObjectTarget, WordDocument, WordDocumentSnapshot,
};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Pastes a detached drawing and its package dependencies as one undo step.
pub struct PasteObjectCommand {
    range: SelectionRange,
    clip: ObjectClip,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    inserted_target: Option<FloatingObjectTarget>,
}

impl PasteObjectCommand {
    pub fn new(range: SelectionRange, clip: ObjectClip) -> Self {
        Self {
            range,
            clip,
            snapshot: None,
            previous_selection: None,
            inserted_target: None,
        }
    }

    pub fn inserted_target(&self) -> Option<FloatingObjectTarget> {
        self.inserted_target
    }
}

impl Command for PasteObjectCommand {
    fn name(&self) -> &'static str {
        "paste_object"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, end) = self.range.normalized();
        validate_position(ctx.document, start)?;
        validate_position(ctx.document, end)?;
        if (start.paragraph..=end.paragraph)
            .any(|paragraph| !ctx.document.is_body_paragraph_surface(paragraph))
        {
            return Err(CommandError::Custom(
                "drawing replacement requires body paragraphs".into(),
            ));
        }
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let result = (|| {
            let rendered_position = if self.range.is_collapsed() {
                start
            } else {
                // Reuse text deletion for paragraph joins, revisions, note
                // references and other anchors. The outer package snapshot
                // makes deletion plus drawing import one atomic history step.
                DeleteTextCommand::new(self.range, DeleteDirection::Backward).apply(ctx)?;
                ctx.selection
                    .as_ref()
                    .ok_or_else(|| CommandError::Custom("deletion did not leave a caret".into()))?
                    .focus
            };
            let model_position = validate_position(ctx.document, rendered_position)?;
            let run_index = ctx
                .document
                .insert_object_clip_at(model_position.paragraph, model_position.offset, &self.clip)
                .map_err(CommandError::Custom)?;
            Ok((rendered_position, model_position, run_index))
        })();
        let (rendered_position, model_position, run_index) = match result {
            Ok(inserted) => inserted,
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                return Err(error);
            }
        };
        self.snapshot = Some(snapshot);
        self.previous_selection = previous_selection;
        self.inserted_target = Some(FloatingObjectTarget {
            paragraph_index: model_position.paragraph,
            run_index,
            kind: if ctx
                .document
                .paragraph(model_position.paragraph)
                .and_then(|p| p.runs.get(run_index))
                .is_some_and(|r| r.text_box.is_some())
            {
                FloatingObjectKind::TextBox
            } else {
                FloatingObjectKind::Image
            },
        });
        *ctx.selection = Some(SelectionRange::collapsed(rendered_position));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("missing drawing backup for undo".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        self.inserted_target = None;
        Ok(())
    }
}

/// Check both ends before deletion: clamping an invalid range must not erase
/// text and then paste at an unintended position.
fn validate_position(
    document: &WordDocument,
    position: TextPosition,
) -> CommandResult<TextPosition> {
    if !document.is_body_paragraph_surface(position.paragraph) {
        return Err(CommandError::Custom(
            "drawings can only be inserted in body paragraphs".into(),
        ));
    }
    let paragraph = document
        .paragraph(position.paragraph)
        .ok_or(CommandError::ParagraphUnavailable)?;
    let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
    let rendered_len = document
        .model_to_rendered_text_position(TextPosition::new(position.paragraph, text.len()))
        .offset;
    if position.offset > rendered_len {
        return Err(CommandError::Custom(
            "drawing paste position is past the paragraph end".into(),
        ));
    }
    let model = document.rendered_to_model_text_position(position);
    if !text.is_char_boundary(model.offset) {
        return Err(CommandError::Custom(
            "drawing paste position is inside a character".into(),
        ));
    }
    Ok(model)
}
