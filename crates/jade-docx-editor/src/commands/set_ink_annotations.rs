//! Atomic replacement of the editable ink layer.

use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::{InkAnnotationInfo, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

pub struct SetInkAnnotationsCommand {
    annotations: Vec<InkAnnotationInfo>,
    snapshot: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
    changed: bool,
}

impl SetInkAnnotationsCommand {
    pub fn new(annotations: Vec<InkAnnotationInfo>) -> Self {
        Self {
            annotations,
            snapshot: None,
            previous_selection: None,
            changed: false,
        }
    }
}

impl Command for SetInkAnnotationsCommand {
    fn name(&self) -> &'static str {
        "set_ink_annotations"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".to_string()))?;
        let previous_selection = *ctx.selection;
        if ctx.document.ink_annotations() == self.annotations {
            self.changed = false;
            return Ok(());
        }
        match ctx.document.replace_ink_annotations(&self.annotations) {
            Ok(()) => {
                self.snapshot = Some(snapshot);
                self.previous_selection = previous_selection;
                self.changed = true;
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                self.changed = false;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self.snapshot.take().ok_or_else(|| {
            CommandError::Custom("set_ink_annotations snapshot is missing".to_string())
        })?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }

    fn should_record_history(&self) -> bool {
        self.changed
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        HistoryRenderFootprint::Structural
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::{InkAnnotationInfo, WordDocument};
    use crate::Options;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use futures::executor;
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    const PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    fn build_docx() -> Vec<u8> {
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
        const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>一つ目 &amp; A</w:t></w:r></w:p><w:p><w:r><w:t>二つ目 😀</w:t></w:r></w:p></w:body></w:document>"#;
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", ROOT_RELS),
                ("word/document.xml", DOCUMENT),
                ("word/_rels/document.xml.rels", DOC_RELS),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn build_shared_media_docx() -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
        const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
        const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rIdInk" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/shared.png"/>
  <Relationship Id="rIdForeign" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/shared.png"/>
</Relationships>"#;
        const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
 xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
 xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
 xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"
 xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
 <w:body><w:p>
  <w:r><w:drawing><wp:anchor behindDoc="0"><wp:extent cx="9525" cy="9525"/>
   <wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH>
   <wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>
   <wp:docPr id="1" name="aidocs-ink shared" descr="{&quot;v&quot;:1}"/>
   <a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rIdInk"/></pic:blipFill></pic:pic></a:graphicData></a:graphic>
  </wp:anchor></w:drawing></w:r>
  <w:r><w:drawing><wp:inline><wp:extent cx="9525" cy="9525"/><wp:docPr id="2" name="Picture 2"/>
   <a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rIdForeign"/></pic:blipFill></pic:pic></a:graphicData></a:graphic>
  </wp:inline></w:drawing></w:r>
 </w:p></w:body>
</w:document>"#;
        let png = BASE64
            .decode(PNG_DATA_URI.strip_prefix("data:image/png;base64,").unwrap())
            .expect("decode PNG");
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", ROOT_RELS),
                ("word/document.xml", DOCUMENT),
                ("word/_rels/document.xml.rels", DOC_RELS),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.start_file("word/media/shared.png", options)
                .expect("start media entry");
            zip.write_all(&png).expect("write shared media");
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn zip_paths(bytes: &[u8]) -> Vec<String> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open zip");
        (0..archive.len())
            .map(|index| archive.by_index(index).expect("entry").name().to_string())
            .collect()
    }

    fn part(bytes: &[u8], path: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open zip");
        let mut entry = archive.by_name(path).expect("part");
        let mut value = String::new();
        entry.read_to_string(&mut value).expect("read part");
        value
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn generated_ink_is_atomic_undoable_and_round_trips_without_orphans(
            specs in proptest::collection::vec(
                (
                    0usize..2,
                    1u16..1200,
                    1u16..1200,
                    -2000i16..2000,
                    -2000i16..2000,
                    proptest::collection::vec(prop::sample::select(vec!['a', 'あ', '😀', '<', '&', '"', '\n']), 0..40),
                ),
                1..8,
            ),
        ) {
            let source = build_docx();
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load generated document");
            let baseline = document.export_docx().expect("baseline");
            let mut annotations = specs.into_iter().enumerate().map(
                |(index, (paragraph_index, width, height, x, y, payload))| {
                    let payload = payload.into_iter().collect::<String>();
                    InkAnnotationInfo {
                        id: format!("stroke-{index}"),
                        paragraph_index,
                        data_uri: PNG_DATA_URI.to_string(),
                        width_px: f64::from(width),
                        height_px: f64::from(height),
                        offset_x_px: f64::from(x) / 10.0,
                        offset_y_px: f64::from(y) / 10.0,
                        payload: (!payload.is_empty()).then_some(payload),
                    }
                }
            ).collect::<Vec<_>>();
            annotations.sort_by_key(|annotation| annotation.paragraph_index);
            let mut selection = Some(SelectionRange::collapsed(jade_docx_layout::selection::TextPosition::new(1, 0)));
            let mut command_executor = CommandExecutor::new();
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                command_executor.execute(Box::new(SetInkAnnotationsCommand::new(annotations.clone())), &mut ctx)
                    .expect("set ink");
            }
            prop_assert_eq!(document.ink_annotations(), annotations.clone());

            let applied = document.export_docx().expect("export ink");
            let paths = zip_paths(&applied);
            prop_assert_eq!(paths.iter().filter(|path| path.starts_with("word/media/aidocsink")).count(), annotations.len());
            let xml = part(&applied, "word/document.xml");
            prop_assert_eq!(xml.matches("<wp:anchor").count(), annotations.len());
            // Word stores the same non-visual name on `wp:docPr` and
            // `pic:cNvPr`; GenOffice discovers the former.
            prop_assert_eq!(xml.matches("name=\"aidocs-ink stroke-").count(), annotations.len() * 2);
            prop_assert!(xml.contains("relativeFrom=\"column\""));
            prop_assert!(xml.contains("relativeFrom=\"paragraph\""));

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen ink document");
            let reopened_annotations = reopened.ink_annotations();
            prop_assert_eq!(reopened_annotations.len(), annotations.len());
            for (actual, expected) in reopened_annotations.iter().zip(&annotations) {
                prop_assert_eq!(&actual.id, &expected.id);
                prop_assert_eq!(actual.paragraph_index, expected.paragraph_index);
                prop_assert_eq!(&actual.data_uri, &expected.data_uri);
                prop_assert!((actual.width_px - expected.width_px).abs() < 0.001);
                prop_assert!((actual.height_px - expected.height_px).abs() < 0.001);
                prop_assert!((actual.offset_x_px - expected.offset_x_px).abs() < 0.001);
                prop_assert!((actual.offset_y_px - expected.offset_y_px).abs() < 0.001);
                prop_assert_eq!(&actual.payload, &expected.payload);
            }

            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.undo(&mut ctx).expect("undo ink"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            prop_assert_eq!(selection, Some(SelectionRange::collapsed(jade_docx_layout::selection::TextPosition::new(1, 0))));
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(command_executor.redo(&mut ctx).expect("redo ink"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }
    }

    #[test]
    fn invalid_replacement_keeps_existing_ink_and_package_byte_identical() {
        let source = build_docx();
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let valid = InkAnnotationInfo {
            id: "valid".to_string(),
            paragraph_index: 0,
            data_uri: PNG_DATA_URI.to_string(),
            width_px: 20.0,
            height_px: 10.0,
            offset_x_px: 1.0,
            offset_y_px: 2.0,
            payload: Some(r#"{"v":1,"tool":"pen"}"#.to_string()),
        };
        document
            .replace_ink_annotations(std::slice::from_ref(&valid))
            .expect("seed ink");
        let before = document.export_docx().expect("before invalid");
        let mut invalid = valid.clone();
        invalid.id = "invalid id".to_string();
        invalid.data_uri = "data:image/png;base64,not-base64".to_string();
        let mut selection = Some(SelectionRange::collapsed(
            jade_docx_layout::selection::TextPosition::new(0, 0),
        ));
        let mut command_executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(command_executor
                .execute(
                    Box::new(SetInkAnnotationsCommand::new(vec![valid.clone(), invalid])),
                    &mut ctx,
                )
                .is_err());
        }
        assert_eq!(document.ink_annotations(), vec![valid]);
        assert_eq!(document.export_docx().expect("after invalid"), before);
        assert!(!command_executor.can_undo());
        assert_eq!(
            selection,
            Some(SelectionRange::collapsed(
                jade_docx_layout::selection::TextPosition::new(0, 0)
            ))
        );

        // Replacing the layer with its exact current value is a package no-op
        // and must not consume one undo slot.
        let current = document.ink_annotations();
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            command_executor
                .execute(Box::new(SetInkAnnotationsCommand::new(current)), &mut ctx)
                .expect("no-op replacement");
        }
        assert!(!command_executor.can_undo());
        assert_eq!(document.export_docx().expect("after no-op"), before);
    }

    #[test]
    fn clearing_reopened_ink_removes_only_its_source_media_and_relationship() {
        let source = build_docx();
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let png = BASE64
            .decode(PNG_DATA_URI.strip_prefix("data:image/png;base64,").unwrap())
            .expect("decode png");
        document
            .insert_inline_image_at(0, 0, &png, "image/png", 12.0, 12.0, Some("foreign"), None)
            .expect("insert foreign image");
        document
            .replace_ink_annotations(&[InkAnnotationInfo {
                id: "erase-me".to_string(),
                paragraph_index: 1,
                data_uri: PNG_DATA_URI.to_string(),
                width_px: 24.0,
                height_px: 18.0,
                offset_x_px: 3.0,
                offset_y_px: 4.0,
                payload: Some(r#"{"v":1,"tool":"highlighter"}"#.to_string()),
            }])
            .expect("add ink");
        let authored = document.export_docx().expect("export authored");
        assert!(zip_paths(&authored)
            .iter()
            .any(|path| path == "word/media/aidocsink1.png"));

        let mut reopened = executor::block_on(WordDocument::load(&authored, &Options::default()))
            .expect("reopen authored");
        reopened.replace_ink_annotations(&[]).expect("clear ink");
        let cleared = reopened.export_docx().expect("export cleared");
        let paths = zip_paths(&cleared);
        assert!(!paths.iter().any(|path| path.contains("aidocsink")));
        assert!(paths.iter().any(|path| path.contains("jade-image")));
        assert!(!part(&cleared, "word/document.xml").contains("aidocs-ink"));
        let rels = part(&cleared, "word/_rels/document.xml.rels");
        assert!(!rels.contains("aidocsink"));
        assert!(rels.contains("jade-image"));
        assert_eq!(
            reopened
                .paragraph(0)
                .unwrap()
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "一つ目 & A"
        );
        let before_noop = cleared;
        reopened.replace_ink_annotations(&[]).expect("clear again");
        assert_eq!(reopened.export_docx().expect("export no-op"), before_noop);
    }

    #[test]
    fn clearing_ink_preserves_media_shared_by_a_foreign_relationship() {
        let source = build_shared_media_docx();
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load shared-media document");
        assert_eq!(document.ink_annotations().len(), 1);

        document.replace_ink_annotations(&[]).expect("clear ink");
        let cleared = document.export_docx().expect("export cleared document");
        let paths = zip_paths(&cleared);
        assert!(paths.iter().any(|path| path == "word/media/shared.png"));
        let rels = part(&cleared, "word/_rels/document.xml.rels");
        assert!(!rels.contains("rIdInk"));
        assert!(rels.contains("rIdForeign"));
        let xml = part(&cleared, "word/document.xml");
        assert!(!xml.contains("aidocs-ink"));
        assert!(xml.contains("r:embed=\"rIdForeign\""));
    }
}
