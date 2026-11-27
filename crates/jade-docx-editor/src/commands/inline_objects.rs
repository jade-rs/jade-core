//! Inline pictures and text boxes are one character of their line, as Word
//! treats them: Backspace after one or Delete before one removes it, and
//! deleting a range removes those inside it with the text.

use super::delete_object::DeleteObjectCommand;
use super::paragraph_undo::ParagraphRangeUndo;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{FloatingObjectTarget, WordDocument};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// The inline object a collapsed caret deletes: the one just before it for
/// Backspace, just after it for Delete. `None` when the next character that
/// way is not one (or changes are tracked, which Word records instead).
pub fn inline_object_beside_caret(
    document: &WordDocument,
    caret: TextPosition,
    backwards: bool,
) -> Option<(FloatingObjectTarget, TextPosition)> {
    if document.revision_session().is_some() {
        return None;
    }
    const OBJECT_CHAR_LEN: usize = '\u{fffc}'.len_utf8();
    // The object whose character is the next one that way.
    document
        .inline_object_rendered_starts(caret.paragraph)
        .into_iter()
        .find(|(_, _, start)| {
            if backwards {
                start + OBJECT_CHAR_LEN == caret.offset
            } else {
                *start == caret.offset
            }
        })
        .map(|(object, _, start)| (object, TextPosition::new(caret.paragraph, start)))
}

/// The inline equation a collapsed caret deletes, as (run index, the
/// caret where it was): the one just before it for Backspace, just after
/// it for Delete. Word removes the equation, not the text beside it.
pub fn inline_equation_beside_caret(
    document: &WordDocument,
    caret: TextPosition,
    backwards: bool,
) -> Option<(usize, TextPosition)> {
    if document.revision_session().is_some() {
        return None;
    }
    const OBJECT_CHAR_LEN: usize = '\u{fffc}'.len_utf8();
    document
        .inline_equation_rendered_starts(caret.paragraph)
        .into_iter()
        .find(|(_, start)| {
            if backwards {
                start + OBJECT_CHAR_LEN == caret.offset
            } else {
                *start == caret.offset
            }
        })
        .map(|(run_index, start)| (run_index, TextPosition::new(caret.paragraph, start)))
}

/// Removes an inline equation's run, leaving the caret where it was.
pub struct DeleteInlineEquationCommand {
    paragraph: usize,
    run_index: usize,
    caret: TextPosition,
    undo: Option<ParagraphRangeUndo>,
    previous_selection: Option<SelectionRange>,
}

impl DeleteInlineEquationCommand {
    pub fn new(run_index: usize, caret: TextPosition) -> Self {
        Self {
            paragraph: caret.paragraph,
            run_index,
            caret,
            undo: None,
            previous_selection: None,
        }
    }
}

impl Command for DeleteInlineEquationCommand {
    fn name(&self) -> &'static str {
        "delete_inline_equation"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.previous_selection = *ctx.selection;
        let present = ctx
            .document
            .paragraph(self.paragraph)
            .and_then(|paragraph| paragraph.runs.get(self.run_index))
            .is_some_and(|run| run.math.is_some() && run.text.is_empty());
        if !present {
            return Err(CommandError::Custom(
                "the run holds no inline equation".into(),
            ));
        }
        let mut undo = ParagraphRangeUndo::begin(ctx, Some((self.paragraph, self.paragraph)))?;
        ctx.document
            .paragraph_mut(self.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?
            .runs
            .remove(self.run_index);
        ctx.document.sync_paragraph_text(self.paragraph);
        ctx.document.rebuild_paragraph_surfaces();
        *ctx.selection = Some(SelectionRange::collapsed(self.caret));
        undo.finish(ctx);
        self.undo = Some(undo);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.undo
            .take()
            .ok_or_else(|| CommandError::Custom("missing equation backup for undo".into()))?
            .undo(ctx)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

/// An object that is one character of its line: an inline picture or text
/// box, or (in a body paragraph) an inline equation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineObject {
    Drawing(FloatingObjectTarget),
    Equation { paragraph: usize, run_index: usize },
}

impl InlineObject {
    fn key(&self) -> (usize, usize) {
        match self {
            Self::Drawing(target) => (target.paragraph_index, target.run_index),
            Self::Equation {
                paragraph,
                run_index,
            } => (*paragraph, *run_index),
        }
    }

    fn command(self) -> Box<dyn Command> {
        match self {
            Self::Drawing(target) => Box::new(DeleteObjectCommand::new(target)),
            Self::Equation {
                paragraph,
                run_index,
            } => Box::new(DeleteInlineEquationCommand::new(
                run_index,
                TextPosition::new(paragraph, 0),
            )),
        }
    }
}

/// The inline objects whole inside `range` (rendered offsets), pictures,
/// text boxes and equations alike, in document order.
pub fn inline_objects_in_range(
    document: &WordDocument,
    range: SelectionRange,
) -> Vec<InlineObject> {
    const OBJECT_CHAR_LEN: usize = '\u{fffc}'.len_utf8();
    let (start, end) = range.normalized();
    let mut objects: Vec<InlineObject> = document
        .inline_objects_in_rendered_range(range)
        .into_iter()
        .map(InlineObject::Drawing)
        .collect();
    if start != end {
        for paragraph in start.paragraph..=end.paragraph {
            for (run_index, rendered_start) in document.inline_equation_rendered_starts(paragraph) {
                let after_start = paragraph > start.paragraph || rendered_start >= start.offset;
                let before_end =
                    paragraph < end.paragraph || rendered_start + OBJECT_CHAR_LEN <= end.offset;
                if after_start && before_end {
                    objects.push(InlineObject::Equation {
                        paragraph,
                        run_index,
                    });
                }
            }
        }
    }
    objects.sort_by_key(InlineObject::key);
    objects
}

/// Deletes inline objects, then runs `inner` (the text edit of the same
/// range, its end moved back by the object characters it no longer holds),
/// as one undo step.
pub struct DeleteInlineObjectsCommand {
    objects: Vec<Box<dyn Command>>,
    inner: Option<Box<dyn Command>>,
    inner_range: Option<SelectionRange>,
    caret: TextPosition,
    previous_selection: Option<SelectionRange>,
}

impl DeleteInlineObjectsCommand {
    /// `objects` in document order; they are deleted last first, so the run
    /// indices of the others stay right.
    pub fn new(
        mut objects: Vec<InlineObject>,
        inner: Option<(Box<dyn Command>, SelectionRange)>,
        caret: TextPosition,
    ) -> Self {
        objects.sort_by_key(InlineObject::key);
        let (inner, inner_range) = match inner {
            Some((command, range)) => (Some(command), Some(range)),
            None => (None, None),
        };
        Self {
            objects: objects
                .into_iter()
                .rev()
                .map(InlineObject::command)
                .collect(),
            inner,
            inner_range,
            caret,
            previous_selection: None,
        }
    }

    /// The range `range` becomes once the objects among `objects` are gone:
    /// its end moves back by the object characters before it in its own
    /// paragraph. Its start is in front of every object it holds.
    pub fn range_without(objects: &[InlineObject], range: SelectionRange) -> SelectionRange {
        const OBJECT_CHAR_LEN: usize = '\u{fffc}'.len_utf8();
        let (start, end) = range.normalized();
        let in_end = objects
            .iter()
            .filter(|object| object.key().0 == end.paragraph)
            .count();
        SelectionRange::new(
            start,
            TextPosition::new(
                end.paragraph,
                end.offset.saturating_sub(in_end * OBJECT_CHAR_LEN),
            ),
        )
    }

    fn run(&mut self, ctx: &mut CommandContext<'_>, redo: bool) -> CommandResult<()> {
        self.previous_selection = *ctx.selection;
        let mut applied = 0;
        let mut outcome = Ok(());
        for object in &mut self.objects {
            let result = if redo {
                object.redo(ctx)
            } else {
                object.apply(ctx)
            };
            if let Err(error) = result {
                outcome = Err(error);
                break;
            }
            applied += 1;
        }
        if outcome.is_ok() {
            if let (Some(inner), Some(range)) = (self.inner.as_mut(), self.inner_range) {
                *ctx.selection = Some(range);
                outcome = if redo {
                    inner.redo(ctx)
                } else {
                    inner.apply(ctx)
                };
                if outcome.is_ok() {
                    return Ok(());
                }
            } else {
                *ctx.selection = Some(SelectionRange::collapsed(self.caret));
                return Ok(());
            }
        }
        for object in self.objects[..applied].iter_mut().rev() {
            let _ = object.undo(ctx);
        }
        *ctx.selection = self.previous_selection;
        outcome
    }
}

impl Command for DeleteInlineObjectsCommand {
    fn name(&self) -> &'static str {
        "delete_inline_objects"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.run(ctx, false)
    }

    fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.run(ctx, true)
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if let Some(inner) = self.inner.as_mut() {
            inner.undo(ctx)?;
        }
        for object in self.objects.iter_mut().rev() {
            object.undo(ctx)?;
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::headless_api::apply_headless_commands_to_document;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{HeadlessCommand, HeadlessDeleteDirection, HeadlessEditRequest};
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const PICTURE: &str = "\u{fffc}";

    fn document_with(text: &str) -> WordDocument {
        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
            for (name, body) in [
                (
                    "[Content_Types].xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
                ),
                ("word/document.xml", document_xml.as_str()),
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        executor::block_on(WordDocument::load(
            &cursor.into_inner(),
            &Options::default(),
        ))
        .unwrap()
    }

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255])
            .unwrap()
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    fn apply(document: &mut WordDocument, command: HeadlessCommand) {
        apply_headless_commands_to_document(
            document,
            &HeadlessEditRequest {
                commands: vec![command],
            },
        )
        .unwrap();
    }

    fn pictures(document: &WordDocument) -> usize {
        document
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .filter(|run| run.image.is_some())
            .count()
    }

    fn delete(offset: usize, backwards: bool) -> HeadlessCommand {
        HeadlessCommand::DeleteText {
            paragraph: 0,
            offset,
            direction: if backwards {
                HeadlessDeleteDirection::Backward
            } else {
                HeadlessDeleteDirection::Forward
            },
            count: 1,
        }
    }

    #[test]
    fn typing_and_deleting_treat_an_inline_picture_as_one_character() {
        let mut document = document_with("ABCD");
        document
            .insert_inline_image_at(0, 2, &png(), "image/png", 10.0, 10.0, None, None)
            .unwrap();
        assert_eq!(document.rendered_paragraph_text(0), "AB\u{fffc}CD");
        assert_eq!(document.rendered_paragraph_len(0), "AB\u{fffc}CD".len());

        // Typed right after the picture, the text follows it.
        let after = "AB".len() + PICTURE.len();
        apply(
            &mut document,
            HeadlessCommand::InsertText {
                paragraph: 0,
                offset: after,
                text: "x".into(),
            },
        );
        assert_eq!(document.rendered_paragraph_text(0), "AB\u{fffc}xCD");
        // Typed in front of it, the text goes before it.
        apply(
            &mut document,
            HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 2,
                text: "y".into(),
            },
        );
        assert_eq!(document.rendered_paragraph_text(0), "ABy\u{fffc}xCD");

        // Backspace after it, Delete before it: the picture goes.
        let start = "ABy".len();
        apply(&mut document, delete(start + PICTURE.len(), true));
        assert_eq!(pictures(&document), 0);
        assert_eq!(document.rendered_paragraph_text(0), "AByxCD");
    }

    #[test]
    fn a_range_takes_the_pictures_inside_it_with_its_text() {
        let mut document = document_with("ABCD");
        document
            .insert_inline_image_at(0, 2, &png(), "image/png", 10.0, 10.0, None, None)
            .unwrap();
        apply(
            &mut document,
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 1,
                end_paragraph: 0,
                end_offset: "AB\u{fffc}C".len(),
                text: "z".into(),
            },
        );
        assert_eq!(pictures(&document), 0);
        assert_eq!(document.rendered_paragraph_text(0), "AzD");

        // A range of the picture alone.
        let mut document = document_with("ABCD");
        document
            .insert_inline_image_at(0, 2, &png(), "image/png", 10.0, 10.0, None, None)
            .unwrap();
        apply(
            &mut document,
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 2,
                end_paragraph: 0,
                end_offset: "AB\u{fffc}".len(),
                text: String::new(),
            },
        );
        assert_eq!(pictures(&document), 0);
        assert_eq!(document.rendered_paragraph_text(0), "ABCD");
    }

    #[test]
    fn delete_before_a_note_mark_takes_the_note_not_the_picture_behind_it() {
        let mut document = document_with("AB");
        document
            .insert_inline_image_at(0, 2, &png(), "image/png", 10.0, 10.0, None, None)
            .unwrap();
        apply(
            &mut document,
            HeadlessCommand::InsertEndnote {
                paragraph: 0,
                offset: 2,
                text: "note".into(),
            },
        );
        let starts = document.inline_object_rendered_starts(0);
        assert_eq!(starts.len(), 1);
        let picture_start = starts[0].2;
        // Delete at the mark (in front of the picture) is not the picture's.
        if picture_start > 2 {
            assert!(super::inline_object_beside_caret(
                &document,
                jade_docx_layout::selection::TextPosition::new(0, 2),
                false
            )
            .is_none());
        }
        // Backspace right after the picture takes it, and the note stays.
        apply(&mut document, delete(picture_start + PICTURE.len(), true));
        assert_eq!(pictures(&document), 0);
        assert_eq!(document.reference_marker_spans(0).len(), 1);
    }

    fn equations(document: &WordDocument) -> usize {
        document
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .filter(|run| run.math.is_some())
            .count()
    }

    fn with_equation() -> WordDocument {
        let mut document = document_with("ABCD");
        apply(
            &mut document,
            HeadlessCommand::InsertEquation {
                paragraph: 0,
                offset: 2,
                omml: "<m:oMath><m:r><m:t>x</m:t></m:r></m:oMath>".into(),
                placement: jade_docx_headless_api::HeadlessEquationPlacement::Inline,
            },
        );
        assert_eq!(document.rendered_paragraph_text(0), "AB\u{fffc}CD");
        document
    }

    #[test]
    fn backspace_and_delete_beside_an_inline_equation_take_the_equation() {
        // Backspace after it: the equation goes, not the B in front of it.
        let mut document = with_equation();
        let mut executor = crate::commands::CommandExecutor::new();
        crate::headless_api::apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![delete("AB".len() + PICTURE.len(), true)],
            },
        )
        .unwrap();
        assert_eq!(equations(&document), 0);
        assert_eq!(document.rendered_paragraph_text(0), "ABCD");
        let mut selection = None;
        let mut context = crate::commands::CommandContext::new(&mut document, &mut selection);
        assert!(executor.undo(&mut context).unwrap());
        assert_eq!(equations(&document), 1);
        assert_eq!(document.rendered_paragraph_text(0), "AB\u{fffc}CD");

        // Delete before it: the equation goes, not the C after it.
        let mut document = with_equation();
        apply(&mut document, delete("AB".len(), false));
        assert_eq!(equations(&document), 0);
        assert_eq!(document.rendered_paragraph_text(0), "ABCD");

        // Text typed after it follows it; typed in front, precedes it.
        let mut document = with_equation();
        apply(
            &mut document,
            HeadlessCommand::InsertText {
                paragraph: 0,
                offset: "AB".len() + PICTURE.len(),
                text: "x".into(),
            },
        );
        assert_eq!(document.rendered_paragraph_text(0), "AB\u{fffc}xCD");
        apply(
            &mut document,
            HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 2,
                text: "y".into(),
            },
        );
        assert_eq!(document.rendered_paragraph_text(0), "ABy\u{fffc}xCD");

        // A range over it takes it with the text.
        let mut document = with_equation();
        apply(
            &mut document,
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 1,
                end_paragraph: 0,
                end_offset: "AB\u{fffc}C".len(),
                text: "z".into(),
            },
        );
        assert_eq!(equations(&document), 0);
        assert_eq!(document.rendered_paragraph_text(0), "AzD");
        let mut document = with_equation();
        apply(
            &mut document,
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 2,
                end_paragraph: 0,
                end_offset: "AB\u{fffc}".len(),
                text: String::new(),
            },
        );
        assert_eq!(equations(&document), 0);
        assert_eq!(document.rendered_paragraph_text(0), "ABCD");

        // Backspace in front of it still takes the B.
        let mut document = with_equation();
        apply(&mut document, delete("AB".len(), true));
        assert_eq!(document.rendered_paragraph_text(0), "A\u{fffc}CD");
    }
}
