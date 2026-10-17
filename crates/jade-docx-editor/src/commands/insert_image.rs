use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{FloatingObjectKind, FloatingObjectTarget, WordDocumentSnapshot};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// Inserts a new inline bitmap at an exact collapsed body caret. The command
/// owns the image bytes so redo can recreate the media part after undo.
pub struct InsertImageCommand {
    range: SelectionRange,
    bytes: Vec<u8>,
    mime_type: String,
    width_px: f64,
    height_px: f64,
    title: Option<String>,
    description: Option<String>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    inserted_target: Option<FloatingObjectTarget>,
}

impl InsertImageCommand {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        range: SelectionRange,
        bytes: Vec<u8>,
        mime_type: String,
        width_px: f64,
        height_px: f64,
        title: Option<String>,
        description: Option<String>,
    ) -> Self {
        Self {
            range,
            bytes,
            mime_type,
            width_px,
            height_px,
            title,
            description,
            snapshot: None,
            previous_selection: None,
            inserted_target: None,
        }
    }

    pub fn inserted_target(&self) -> Option<FloatingObjectTarget> {
        self.inserted_target
    }
}

impl Command for InsertImageCommand {
    fn name(&self) -> &'static str {
        "insert_image"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if !self.range.is_collapsed() {
            return Err(CommandError::Custom(
                "image insertion requires a collapsed selection".into(),
            ));
        }
        let rendered_position = self.range.focus;
        if !ctx
            .document
            .is_body_paragraph_surface(rendered_position.paragraph)
        {
            return Err(CommandError::Custom(format!(
                "images can only be inserted in body paragraphs; caret is in {}",
                ctx.document
                    .paragraph_story_description(rendered_position.paragraph)
            )));
        }
        let paragraph = ctx
            .document
            .paragraph(rendered_position.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?;
        let model_len = paragraph.runs.iter().map(|run| run.text.len()).sum();
        let rendered_len = ctx
            .document
            .model_to_rendered_text_position(TextPosition::new(
                rendered_position.paragraph,
                model_len,
            ))
            .offset;
        if rendered_position.offset > rendered_len {
            return Err(CommandError::Custom(format!(
                "offset {} is past the end of paragraph {}, which is {rendered_len} bytes long",
                rendered_position.offset, rendered_position.paragraph
            )));
        }

        let model_position = ctx
            .document
            .rendered_to_model_text_position(rendered_position);
        let text = ctx
            .document
            .paragraph(model_position.paragraph)
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            })
            .ok_or(CommandError::ParagraphUnavailable)?;
        if !text.is_char_boundary(model_position.offset) {
            return Err(CommandError::Custom(format!(
                "offset {} is inside a character in paragraph {}",
                rendered_position.offset, rendered_position.paragraph
            )));
        }

        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        let run_index = match ctx.document.insert_inline_image_at(
            model_position.paragraph,
            model_position.offset,
            &self.bytes,
            &self.mime_type,
            self.width_px,
            self.height_px,
            self.title.as_deref(),
            self.description.as_deref(),
        ) {
            Ok(run_index) => run_index,
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                return Err(CommandError::Custom(error));
            }
        };

        self.snapshot = Some(snapshot);
        self.previous_selection = previous_selection;
        self.inserted_target = Some(FloatingObjectTarget {
            paragraph_index: model_position.paragraph,
            run_index,
            kind: FloatingObjectKind::Image,
        });
        *ctx.selection = Some(SelectionRange::collapsed(rendered_position));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        self.inserted_target = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::{WordDocument, MAX_INLINE_IMAGE_BYTES};
    use crate::Options;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use futures::executor;
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    const PNG_1X1_BASE64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    fn image_bytes(mime: &str) -> Vec<u8> {
        match mime {
            "image/png" => BASE64.decode(PNG_1X1_BASE64).expect("decode png"),
            "image/jpeg" => vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x01, 0xff, 0xd9],
            "image/gif" => b"GIF89a\x01\x00\x01\x00".to_vec(),
            _ => unreachable!("test image MIME"),
        }
    }

    fn build_docx(text: &str) -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
        const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
        const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;
        let escaped = text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t xml:space="preserve">{escaped}</w:t></w:r></w:p></w:body></w:document>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", ROOT_RELS),
                ("word/document.xml", document_xml.as_str()),
                ("word/_rels/document.xml.rels", DOC_RELS),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn part(bytes: &[u8], path: &str) -> Option<Vec<u8>> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open zip");
        let mut entry = archive.by_name(path).ok()?;
        let mut value = Vec::new();
        entry.read_to_end(&mut value).expect("read zip entry");
        Some(value)
    }

    fn apply_image(
        document: &mut WordDocument,
        selection: &mut Option<SelectionRange>,
        executor: &mut CommandExecutor,
        mime: &str,
        width: f64,
        height: f64,
    ) -> Result<(), CommandError> {
        let range = selection.expect("selection");
        let mut ctx = CommandContext::new(document, selection);
        executor.execute(
            Box::new(InsertImageCommand::new(
                range,
                image_bytes(mime),
                mime.to_string(),
                width,
                height,
                Some("ロゴ & mark".to_string()),
                Some("説明 <alt>".to_string()),
            )),
            &mut ctx,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn generated_inline_images_round_trip_at_every_utf8_boundary(
            chars in proptest::collection::vec(
                prop::sample::select(vec!['a', 'Z', '0', ' ', 'あ', '界', '😀', 'é']),
                0..24,
            ),
            boundary_pick in any::<usize>(),
            mime in prop::sample::select(vec!["image/png", "image/jpeg", "image/gif"]),
            width in 1u16..1600,
            height in 1u16..1600,
        ) {
            let text = chars.into_iter().collect::<String>();
            let mut boundaries = text.char_indices().map(|(index, _)| index).collect::<Vec<_>>();
            boundaries.push(text.len());
            let offset = boundaries[boundary_pick % boundaries.len()];
            let source = build_docx(&text);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load generated document");
            let baseline = document.export_docx().expect("export baseline");
            let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, offset)));
            let mut command_executor = CommandExecutor::new();

            apply_image(
                &mut document,
                &mut selection,
                &mut command_executor,
                mime,
                f64::from(width),
                f64::from(height),
            )
            .expect("insert generated image");

            let paragraph = document.paragraph(0).expect("paragraph");
            prop_assert_eq!(paragraph.runs.iter().map(|run| run.text.as_str()).collect::<String>(), text.clone());
            let image = paragraph.runs.iter().find_map(|run| run.image.as_ref()).expect("image run");
            prop_assert_eq!(image.width_px, Some(f64::from(width)));
            prop_assert_eq!(image.height_px, Some(f64::from(height)));
            prop_assert_eq!(image.title.as_deref(), Some("ロゴ & mark"));
            prop_assert_eq!(image.description.as_deref(), Some("説明 <alt>"));

            let applied = document.export_docx().expect("export image document");
            let document_xml = String::from_utf8(part(&applied, "word/document.xml").expect("document xml"))
                .expect("utf8 document");
            let rels = String::from_utf8(part(&applied, "word/_rels/document.xml.rels").expect("rels"))
                .expect("utf8 rels");
            let content_types = String::from_utf8(part(&applied, "[Content_Types].xml").expect("content types"))
                .expect("utf8 content types");
            prop_assert!(document_xml.contains("<wp:inline"));
            prop_assert!(document_xml.contains("r:embed=\"rId"));
            prop_assert!(document_xml.contains("title=\"ロゴ &amp; mark\""));
            prop_assert!(document_xml.contains("descr=\"説明 &lt;alt>\""));
            prop_assert!(rels.contains("relationships/image"));
            prop_assert!(content_types.contains(mime));
            let extension = if mime == "image/png" { "png" } else if mime == "image/jpeg" { "jpeg" } else { "gif" };
            let media_path = format!("word/media/jade-image1.{extension}");
            prop_assert_eq!(part(&applied, &media_path), Some(image_bytes(mime)));

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.undo(&mut ctx).expect("undo image"));
            }
            prop_assert_eq!(document.export_docx().expect("export undo"), baseline);
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.redo(&mut ctx).expect("redo image"));
            }
            prop_assert_eq!(document.export_docx().expect("export redo"), applied.clone());

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen image document");
            let reopened_image = reopened.paragraph(0).expect("reopened paragraph")
                .runs.iter().find_map(|run| run.image.as_ref()).expect("reopened image");
            prop_assert_eq!(reopened_image.width_px, Some(f64::from(width)));
            prop_assert_eq!(reopened_image.height_px, Some(f64::from(height)));
            let expected_data_uri_prefix = format!("data:{mime};base64,");
            prop_assert!(reopened_image
                .data_uri
                .as_deref()
                .is_some_and(|uri| uri.starts_with(&expected_data_uri_prefix)));
        }
    }

    #[test]
    fn invalid_image_commands_leave_package_and_history_unchanged() {
        let source = build_docx("あA");
        let cases = [
            (
                SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1)),
                image_bytes("image/png"),
                "image/png",
                10.0,
                10.0,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 1)),
                image_bytes("image/png"),
                "image/png",
                10.0,
                10.0,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 99)),
                image_bytes("image/png"),
                "image/png",
                10.0,
                10.0,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 3)),
                image_bytes("image/png"),
                "image/jpeg",
                10.0,
                10.0,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 3)),
                image_bytes("image/png"),
                "image/png",
                f64::NAN,
                10.0,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 3)),
                image_bytes("image/png"),
                "image/png",
                f64::INFINITY,
                10.0,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 3)),
                image_bytes("image/png"),
                "image/png",
                10.0,
                0.0,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 3)),
                image_bytes("image/png"),
                "image/bmp",
                10.0,
                10.0,
            ),
            (
                SelectionRange::collapsed(TextPosition::new(0, 3)),
                Vec::new(),
                "image/png",
                10.0,
                10.0,
            ),
        ];
        for (range, bytes, mime, width, height) in cases {
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let baseline = document.export_docx().expect("export baseline");
            let mut selection = Some(range);
            let mut command_executor = CommandExecutor::new();
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            let result = command_executor.execute(
                Box::new(InsertImageCommand::new(
                    range,
                    bytes,
                    mime.to_string(),
                    width,
                    height,
                    None,
                    None,
                )),
                &mut ctx,
            );
            assert!(result.is_err());
            assert_eq!(
                document.export_docx().expect("export rejected document"),
                baseline
            );
            assert!(!command_executor.can_undo());
            assert_eq!(selection, Some(range));
        }

        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let baseline = document.export_docx().expect("export baseline");
        let range = SelectionRange::collapsed(TextPosition::new(0, 3));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        assert!(executor
            .execute(
                Box::new(InsertImageCommand::new(
                    range,
                    vec![0; MAX_INLINE_IMAGE_BYTES + 1],
                    "image/png".to_string(),
                    10.0,
                    10.0,
                    None,
                    None,
                )),
                &mut ctx,
            )
            .is_err());
        assert_eq!(
            document.export_docx().expect("export oversized rejection"),
            baseline
        );
        assert!(!executor.can_undo());
    }
}
