//! Inline pictures and text boxes are one character of their line, as Word
//! treats them: Backspace after one or Delete before one removes it, and
//! deleting a range removes those inside it with the text.

use super::delete_object::DeleteObjectCommand;
use super::{Command, CommandContext, CommandResult};
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

/// Deletes inline objects, then runs `inner` (the text edit of the same
/// range, its end moved back by the object characters it no longer holds),
/// as one undo step.
pub struct DeleteInlineObjectsCommand {
    objects: Vec<DeleteObjectCommand>,
    inner: Option<Box<dyn Command>>,
    inner_range: Option<SelectionRange>,
    caret: TextPosition,
    previous_selection: Option<SelectionRange>,
}

impl DeleteInlineObjectsCommand {
    /// `objects` in document order; they are deleted last first, so the run
    /// indices of the others stay right.
    pub fn new(
        objects: Vec<FloatingObjectTarget>,
        inner: Option<(Box<dyn Command>, SelectionRange)>,
        caret: TextPosition,
    ) -> Self {
        let (inner, inner_range) = match inner {
            Some((command, range)) => (Some(command), Some(range)),
            None => (None, None),
        };
        Self {
            objects: objects
                .into_iter()
                .rev()
                .map(DeleteObjectCommand::new)
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
    pub fn range_without(
        objects: &[FloatingObjectTarget],
        range: SelectionRange,
    ) -> SelectionRange {
        const OBJECT_CHAR_LEN: usize = '\u{fffc}'.len_utf8();
        let (start, end) = range.normalized();
        let in_end = objects
            .iter()
            .filter(|object| object.paragraph_index == end.paragraph)
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
        assert_eq!(document.paragraph_text_with_objects(0), "AB\u{fffc}CD");
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
        assert_eq!(document.paragraph_text_with_objects(0), "AB\u{fffc}xCD");
        // Typed in front of it, the text goes before it.
        apply(
            &mut document,
            HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 2,
                text: "y".into(),
            },
        );
        assert_eq!(document.paragraph_text_with_objects(0), "ABy\u{fffc}xCD");

        // Backspace after it, Delete before it: the picture goes.
        let start = "ABy".len();
        apply(&mut document, delete(start + PICTURE.len(), true));
        assert_eq!(pictures(&document), 0);
        assert_eq!(document.paragraph_text_with_objects(0), "AByxCD");
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
        assert_eq!(document.paragraph_text_with_objects(0), "AzD");

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
        assert_eq!(document.paragraph_text_with_objects(0), "ABCD");
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
}
