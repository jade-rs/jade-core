//! Bookmark management and bookmark-backed cross-reference commands.

use super::{Command, CommandContext, CommandError, CommandResult};
use crate::word_document::{CrossReferenceKind, CrossReferenceTarget, WordDocumentSnapshot};
use jade_docx_layout::selection::SelectionRange;

struct SnapshotState {
    document: WordDocumentSnapshot,
    selection: Option<SelectionRange>,
}

fn snapshot(ctx: &mut CommandContext<'_>) -> CommandResult<SnapshotState> {
    Ok(SnapshotState {
        document: ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?,
        selection: *ctx.selection,
    })
}

fn restore(ctx: &mut CommandContext<'_>, state: SnapshotState) {
    ctx.document.restore_snapshot(state.document);
    *ctx.selection = state.selection;
}

pub struct CreateBookmarkCommand {
    paragraph: usize,
    name: String,
    previous: Option<SnapshotState>,
}

impl CreateBookmarkCommand {
    pub fn new(paragraph: usize, name: String) -> Self {
        Self {
            paragraph,
            name,
            previous: None,
        }
    }
}

impl Command for CreateBookmarkCommand {
    fn name(&self) -> &'static str {
        "create_bookmark"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = snapshot(ctx)?;
        match ctx
            .document
            .create_bookmark_at_paragraph(self.paragraph, &self.name)
        {
            Ok(()) => {
                self.previous = Some(previous);
                Ok(())
            }
            Err(error) => {
                restore(ctx, previous);
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("bookmark snapshot is missing".into()))?;
        restore(ctx, previous);
        Ok(())
    }
}

pub struct DeleteBookmarkCommand {
    name: String,
    previous: Option<SnapshotState>,
}

impl DeleteBookmarkCommand {
    pub fn new(name: String) -> Self {
        Self {
            name,
            previous: None,
        }
    }
}

impl Command for DeleteBookmarkCommand {
    fn name(&self) -> &'static str {
        "delete_bookmark"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = snapshot(ctx)?;
        match ctx.document.delete_bookmark(&self.name) {
            Ok(()) => {
                self.previous = Some(previous);
                Ok(())
            }
            Err(error) => {
                restore(ctx, previous);
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("bookmark snapshot is missing".into()))?;
        restore(ctx, previous);
        Ok(())
    }
}

pub struct InsertCrossReferenceCommand {
    range: SelectionRange,
    bookmark_name: String,
    target: Option<CrossReferenceTarget>,
    kind: CrossReferenceKind,
    previous: Option<SnapshotState>,
}

impl InsertCrossReferenceCommand {
    pub fn new(range: SelectionRange, bookmark_name: String, kind: CrossReferenceKind) -> Self {
        Self {
            range,
            bookmark_name,
            target: None,
            kind,
            previous: None,
        }
    }
}

impl InsertCrossReferenceCommand {
    pub fn for_target(
        range: SelectionRange,
        target: CrossReferenceTarget,
        kind: CrossReferenceKind,
    ) -> Self {
        Self {
            range,
            bookmark_name: String::new(),
            target: Some(target),
            kind,
            previous: None,
        }
    }
}

impl Command for InsertCrossReferenceCommand {
    fn name(&self) -> &'static str {
        "insert_cross_reference"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = snapshot(ctx)?;
        let result = match self.target.as_ref() {
            Some(target) => ctx
                .document
                .insert_target_cross_reference_at_caret(self.range, target, self.kind),
            None => ctx.document.insert_cross_reference_at_caret(
                self.range,
                &self.bookmark_name,
                self.kind,
            ),
        };
        match result {
            Ok(selection) => {
                self.previous = Some(previous);
                *ctx.selection = Some(selection);
                Ok(())
            }
            Err(error) => {
                restore(ctx, previous);
                Err(CommandError::Custom(error))
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("cross-reference snapshot is missing".into()))?;
        restore(ctx, previous);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{insert_text::InsertTextCommand, CommandContext, CommandExecutor};
    use crate::document::RunFieldKind;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::TextPosition;
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    fn escape_text(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn build_docx(paragraphs: &[String]) -> Vec<u8> {
        let body = paragraphs
            .iter()
            .map(|paragraph| {
                let runs = paragraph
                    .chars()
                    .map(|character| {
                        format!(
                            "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
                            escape_text(&character.to_string())
                        )
                    })
                    .collect::<String>();
                format!("<w:p>{runs}</w:p>")
            })
            .collect::<String>();
        build_docx_body(&body)
    }

    fn build_docx_body(body: &str) -> Vec<u8> {
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>"
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, value) in [
                (
                    "[Content_Types].xml",
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>",
                ),
                (
                    "_rels/.rels",
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>",
                ),
                ("word/document.xml", document.as_str()),
            ] {
                zip.start_file(path, options).expect("start entry");
                zip.write_all(value.as_bytes()).expect("write entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    fn document_xml(bytes: &[u8]) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        let mut part = archive.by_name("word/document.xml").expect("document part");
        let mut value = String::new();
        part.read_to_string(&mut value).expect("read document");
        value
    }

    #[test]
    fn note_targets_insert_scoped_noteref_fields_with_exact_history() {
        for is_footnote in [true, false] {
            let source = build_docx(&["Hello".into(), "See: ".into()]);
            let mut document =
                executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
            let note_id = if is_footnote {
                document.insert_footnote_at(0, 5, "注本文 😀").unwrap()
            } else {
                document.insert_endnote_at(0, 5, "注本文 😀").unwrap()
            };
            let target_type = if is_footnote { "footnote" } else { "endnote" };
            let target = document
                .cross_reference_targets()
                .into_iter()
                .find(|target| serde_json::to_value(target).unwrap()["targetType"] == target_type)
                .expect("note reference target");
            assert!(target.label.contains("注本文 😀"));
            let destination = (0..document.paragraph_count())
                .find(|index| {
                    document
                        .paragraph(*index)
                        .unwrap()
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>()
                        == "See: "
                })
                .unwrap();
            let range = SelectionRange::collapsed(TextPosition::new(destination, 5));
            let mut selection = Some(range);
            let mut history = CommandExecutor::new();
            let before = document.export_docx().unwrap();
            history
                .execute(
                    Box::new(InsertCrossReferenceCommand::for_target(
                        range,
                        target.clone(),
                        CrossReferenceKind::Number,
                    )),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .unwrap();
            let applied = document.export_docx().unwrap();
            let xml = document_xml(&applied);
            assert!(xml.contains(" NOTEREF _Ref"), "{xml}");
            assert!(xml.contains("\\h"));
            let bookmark = &document.document.as_ref().unwrap().bookmarks[0];
            let anchor = &document.paragraph(0).unwrap().runs[bookmark.start_run..bookmark.end_run];
            assert_eq!(
                anchor.len(),
                1,
                "bookmark must enclose only the reference run"
            );
            assert_eq!(
                if is_footnote {
                    &anchor[0].footnote_reference_ids
                } else {
                    &anchor[0].endnote_reference_ids
                },
                &[note_id]
            );
            history
                .undo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            assert_eq!(
                document_xml(&document.export_docx().unwrap()),
                document_xml(&before)
            );
            assert_eq!(selection, Some(range));
            history
                .redo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            assert_eq!(document_xml(&document.export_docx().unwrap()), xml);
            let mut reopened =
                executor::block_on(WordDocument::load(&applied, &Options::default())).unwrap();
            reopened
                .set_note_numbering(is_footnote, "upperRoman", 8, "continuous", Some(0))
                .unwrap();
            let saved = reopened.export_docx().unwrap();
            assert!(
                document_xml(&saved).contains("<w:t>VIII</w:t>"),
                "{}",
                document_xml(&saved)
            );
            let reopened =
                executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
            assert!(reopened
                .paragraph(destination)
                .unwrap()
                .runs
                .iter()
                .any(|run| run.text == "VIII"));
        }
    }

    #[test]
    fn page_restarted_noteref_preserves_cache_until_layout_resolves_it() {
        let source = build_docx(&["Hello".into(), "See".into()]);
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        document.insert_footnote_at(0, 5, "note").unwrap();
        let target = document
            .cross_reference_targets()
            .into_iter()
            .find(|target| {
                target.target_type == crate::word_document::CrossReferenceTargetType::Footnote
            })
            .unwrap();
        let destination = (0..document.paragraph_count())
            .find(|index| {
                document
                    .paragraph(*index)
                    .unwrap()
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    == "See"
            })
            .unwrap();
        document
            .insert_target_cross_reference_at_caret(
                SelectionRange::collapsed(TextPosition::new(destination, 0)),
                &target,
                CrossReferenceKind::Number,
            )
            .unwrap();
        let run = document
            .paragraph_mut(destination)
            .unwrap()
            .runs
            .iter_mut()
            .find(|run| {
                matches!(
                    run.field.as_ref().map(|field| &field.kind),
                    Some(RunFieldKind::NoteRef { .. })
                )
            })
            .unwrap();
        run.text = "cached".into();
        let field = run.field.as_mut().unwrap();
        field.cached_text = Some("cached".into());
        field.resolved_value = Some("cached".into());
        document
            .set_note_numbering(true, "decimal", 1, "eachPage", Some(0))
            .unwrap();
        let run = document
            .paragraph(destination)
            .unwrap()
            .runs
            .iter()
            .find(|run| run.field.is_some())
            .unwrap();
        assert_eq!(run.text, "cached");
    }

    #[test]
    fn note_page_reference_in_a_table_maps_the_physical_anchor_surface() {
        let source = build_docx_body("<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>See</w:t></w:r></w:p>");
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        document.insert_footnote_at(1, 1, "table note").unwrap();
        let target = document
            .cross_reference_targets()
            .into_iter()
            .find(|target| {
                target.target_type == crate::word_document::CrossReferenceTargetType::Footnote
            })
            .unwrap();
        let destination = (0..document.paragraph_count())
            .find(|index| {
                document
                    .paragraph(*index)
                    .unwrap()
                    .runs
                    .iter()
                    .any(|run| run.text == "See")
            })
            .unwrap();
        document
            .insert_target_cross_reference_at_caret(
                SelectionRange::collapsed(TextPosition::new(destination, 0)),
                &target,
                CrossReferenceKind::Page,
            )
            .unwrap();
        let name = &document.document.as_ref().unwrap().bookmarks[0].name;
        assert_eq!(document.bookmark_render_map().get(name), Some(&1));
        let saved = document.export_docx().unwrap();
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert_eq!(reopened.bookmark_render_map().get(name), Some(&1));
    }

    #[test]
    fn heading_and_caption_targets_insert_atomic_hidden_bookmark_references() {
        let source = build_docx_body(
            "<w:p><w:pPr><w:outlineLvl w:val=\"0\"/></w:pPr><w:r><w:t>見出し 😀</w:t></w:r></w:p>\
             <w:p><w:r><w:t xml:space=\"preserve\">図 </w:t></w:r><w:fldSimple w:instr=\" SEQ 図 \"><w:r><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t xml:space=\"preserve\"> 構成</w:t></w:r></w:p>\
             <w:p><w:r><w:t xml:space=\"preserve\">参照: </w:t></w:r></w:p>",
        );
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        let targets = document.cross_reference_targets();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].label, "見出し 😀");
        assert_eq!(targets[1].label, "図 1 構成");
        let range = SelectionRange::collapsed(TextPosition::new(2, "参照: ".len()));
        let mut selection = Some(range);
        let mut history = CommandExecutor::new();
        let before = document_xml(&document.export_docx().unwrap());
        for target in &targets {
            history
                .execute(
                    Box::new(InsertCrossReferenceCommand::for_target(
                        range,
                        target.clone(),
                        CrossReferenceKind::Text,
                    )),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .unwrap();
            let exported = document.export_docx().unwrap();
            let applied = document_xml(&exported);
            assert!(applied.contains(" REF _Ref"));
            assert!(applied.contains("w:bookmarkStart"));
            let reopened =
                executor::block_on(WordDocument::load(&exported, &Options::default())).unwrap();
            assert!(
                reopened
                    .paragraph(2)
                    .unwrap()
                    .runs
                    .iter()
                    .any(|run| run.text == target.label),
                "target={target:?}, runs={:?}",
                reopened.paragraph(2).unwrap().runs
            );
            history
                .undo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            assert_eq!(document_xml(&document.export_docx().unwrap()), before);
            assert_eq!(selection, Some(range));
            history
                .redo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            assert_eq!(document_xml(&document.export_docx().unwrap()), applied);
            history
                .undo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
        }
        let mut stale = targets[0].clone();
        stale.id = "missing".into();
        assert!(history
            .execute(
                Box::new(InsertCrossReferenceCommand::for_target(
                    range,
                    stale,
                    CrossReferenceKind::Page
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .is_err());
        assert_eq!(document_xml(&document.export_docx().unwrap()), before);
        assert!(!history.can_undo());
    }

    #[test]
    fn adjacent_cross_references_keep_independent_fields_after_export_and_reload() {
        for kinds in [
            [CrossReferenceKind::Text, CrossReferenceKind::Page],
            [CrossReferenceKind::Text, CrossReferenceKind::Text],
            [CrossReferenceKind::Page, CrossReferenceKind::Page],
        ] {
            let source = build_docx_body(
                "<w:p><w:bookmarkStart w:id=\"1\" w:name=\"Target\"/><w:r><w:t>Hello</w:t><w:br w:type=\"page\"/><w:tab/><w:cr/><w:noBreakHyphen/><w:softHyphen/></w:r><w:bookmarkEnd w:id=\"1\"/></w:p><w:p><w:r><w:t>World</w:t></w:r></w:p>",
            );
            let mut document =
                executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
            let mut range = SelectionRange::collapsed(TextPosition::new(1, 0));
            for kind in kinds {
                range = document
                    .insert_cross_reference_at_caret(range, "Target", kind)
                    .unwrap();
            }
            let exported = document.export_docx().unwrap();
            let xml = document_xml(&exported);
            assert_eq!(xml.matches("<w:fldSimple ").count(), 2, "{kinds:?}: {xml}");
            let mut reopened =
                executor::block_on(WordDocument::load(&exported, &Options::default())).unwrap();
            let expected = kinds
                .iter()
                .map(|kind| match kind {
                    CrossReferenceKind::Text => "Hello\u{000c}\t\n\u{2011}\u{00ad}",
                    CrossReferenceKind::Page => "1",
                    CrossReferenceKind::Number => unreachable!(),
                })
                .collect::<String>()
                + "World";
            assert_eq!(
                reopened
                    .paragraph(1)
                    .unwrap()
                    .runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
                expected
            );
            let fields: Vec<_> = reopened
                .paragraph(1)
                .unwrap()
                .runs
                .iter()
                .filter_map(|run| run.field.as_ref().map(|field| &field.kind))
                .collect();
            assert_eq!(fields.len(), 2, "{kinds:?}: {fields:?}");
            for (field, kind) in fields.iter().zip(kinds) {
                assert!(matches!(
                    (field, kind),
                    (RunFieldKind::Ref { .. }, CrossReferenceKind::Text)
                        | (RunFieldKind::PageRef { .. }, CrossReferenceKind::Page)
                ));
            }
            reopened
                .insert_cross_reference_at_caret(range, "Target", kinds[1])
                .unwrap();
            assert_eq!(
                document_xml(&reopened.export_docx().unwrap())
                    .matches("<w:fldSimple ")
                    .count(),
                3
            );
        }
    }

    #[test]
    fn caption_number_reference_tracks_only_the_sequence_and_round_trips_history() {
        for field in [
            r#"<w:fldSimple w:instr=" SEQ 図 \* ROMAN "><w:r><w:t>stale</w:t></w:r></w:fldSimple>"#,
            r#"<w:fldSimple w:instr=" SEQ 図 \* ROMAN "><w:r><w:t>sta</w:t></w:r><w:r><w:t>le</w:t></w:r></w:fldSimple>"#,
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> SEQ 図 \* ROMAN </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>stale</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#,
        ] {
            let source = build_docx_body(&format!(
                r#"<w:p><w:r><w:t>図 😀 </w:t></w:r>{field}<w:r><w:t> 説明</w:t></w:r></w:p><w:p/>"#,
            ));
            let mut document =
                executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
            let target = document
                .cross_reference_targets()
                .into_iter()
                .next()
                .unwrap();
            let range = SelectionRange::collapsed(TextPosition::new(1, 0));
            let mut selection = Some(range);
            let mut history = CommandExecutor::new();
            let before = document_xml(&document.export_docx().unwrap());
            for _ in 0..2 {
                history
                    .execute(
                        Box::new(InsertCrossReferenceCommand::for_target(
                            range,
                            target.clone(),
                            CrossReferenceKind::Number,
                        )),
                        &mut CommandContext::new(&mut document, &mut selection),
                    )
                    .unwrap();
            }
            assert_eq!(
                document.document.as_ref().unwrap().bookmarks.len(),
                1,
                "reuse number anchor"
            );
            let applied = document.export_docx().unwrap();
            let xml = document_xml(&applied);
            assert!(xml.contains(" REF _Ref"), "{xml}");
            let reopened =
                executor::block_on(WordDocument::load(&applied, &Options::default())).unwrap();
            let bookmark = &reopened.document.as_ref().unwrap().bookmarks[0];
            assert_eq!(
                reopened
                    .document
                    .as_ref()
                    .unwrap()
                    .bookmark_display_text(bookmark)
                    .as_deref(),
                Some("I")
            );
            assert_eq!(
                reopened
                    .paragraph(1)
                    .unwrap()
                    .runs
                    .iter()
                    .map(|run| run
                        .field
                        .as_ref()
                        .and_then(|field| field.resolved_value.as_deref())
                        .unwrap_or(&run.text))
                    .collect::<String>(),
                "II"
            );
            history
                .undo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            history
                .undo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            assert_eq!(document_xml(&document.export_docx().unwrap()), before);
            assert_eq!(selection, Some(range));
            history
                .redo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            history
                .redo(&mut CommandContext::new(&mut document, &mut selection))
                .unwrap();
            assert_eq!(document_xml(&document.export_docx().unwrap()), xml);
        }
    }

    #[test]
    fn number_and_full_caption_anchors_remain_distinct_after_renumbering() {
        let mut body = "<w:p/>".to_string();
        for _ in 0..9 {
            body.push_str(r#"<w:p><w:r><w:t>Figure </w:t></w:r><w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>stale</w:t></w:r></w:fldSimple><w:r><w:t> description</w:t></w:r></w:p>"#);
        }
        body.push_str("<w:p/>");
        let mut document = executor::block_on(WordDocument::load(
            &build_docx_body(&body),
            &Options::default(),
        ))
        .unwrap();
        let target = document
            .cross_reference_targets()
            .into_iter()
            .last()
            .unwrap();
        let range = SelectionRange::collapsed(TextPosition::new(10, 0));
        let mut selection = Some(range);
        let mut history = CommandExecutor::new();
        for kind in [CrossReferenceKind::Text, CrossReferenceKind::Number] {
            history
                .execute(
                    Box::new(InsertCrossReferenceCommand::for_target(
                        range,
                        target.clone(),
                        kind,
                    )),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .unwrap();
        }
        assert_eq!(document.document.as_ref().unwrap().bookmarks.len(), 2);
        history
            .execute(
                Box::new(crate::commands::insert_caption::InsertCaptionCommand::new(
                    SelectionRange::collapsed(TextPosition::new(0, 0)),
                    "Figure".into(),
                    "earlier".into(),
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap();
        let saved = document.export_docx().unwrap();
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        let values = reopened
            .paragraph(11)
            .unwrap()
            .runs
            .iter()
            .filter_map(|run| run.field.as_ref()?.resolved_value.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(values, ["10", "Figure10description"]);
        let bookmarks = &reopened.document.as_ref().unwrap().bookmarks;
        assert_eq!(bookmarks[0].start_paragraph, 10);
        assert_eq!(bookmarks[1].start_paragraph, 10);
    }

    #[test]
    fn newly_authored_caption_number_reference_needs_no_reload() {
        let source = build_docx_body("<w:p><w:r><w:t>before</w:t></w:r></w:p><w:p/>");
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        crate::headless_api::apply_headless_commands_to_document(
            &mut document,
            &jade_docx_headless_api::HeadlessEditRequest {
                commands: vec![jade_docx_headless_api::HeadlessCommand::InsertCaption {
                    after_paragraph: 0,
                    label: "Figure".into(),
                    description: "new".into(),
                }],
            },
        )
        .unwrap();
        let target = document
            .cross_reference_targets()
            .into_iter()
            .find(|target| {
                target.target_type == crate::word_document::CrossReferenceTargetType::Caption
            })
            .unwrap();
        let range = SelectionRange::collapsed(TextPosition::new(2, 0));
        let mut selection = Some(range);
        let mut history = CommandExecutor::new();
        history
            .execute(
                Box::new(InsertCrossReferenceCommand::for_target(
                    range,
                    target,
                    CrossReferenceKind::Number,
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap();
        assert_eq!(
            document
                .paragraph(2)
                .unwrap()
                .runs
                .iter()
                .filter_map(|run| run.field.as_ref())
                .next()
                .unwrap()
                .resolved_value
                .as_deref(),
            Some("1")
        );
        history
            .undo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap();
        history
            .redo(&mut CommandContext::new(&mut document, &mut selection))
            .unwrap();
        history
            .execute(
                Box::new(crate::commands::insert_caption::InsertCaptionCommand::new(
                    SelectionRange::collapsed(TextPosition::new(0, 0)),
                    "Figure".into(),
                    "earlier".into(),
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap();
        assert_eq!(
            document.document.as_ref().unwrap().bookmarks[0].start_paragraph,
            2
        );
        assert_eq!(
            document
                .paragraph(3)
                .unwrap()
                .runs
                .iter()
                .find_map(|run| run.field.as_ref())
                .unwrap()
                .resolved_value
                .as_deref(),
            Some("2")
        );
    }

    #[test]
    fn invalid_number_targets_and_selections_leave_package_and_history_unchanged() {
        let source = build_docx_body(
            r#"<w:p><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:r><w:t>Heading</w:t></w:r></w:p><w:p><w:r><w:t>Figure</w:t></w:r><w:fldSimple w:instr=" SEQ Figure "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p><w:p><w:r><w:t>text</w:t></w:r></w:p>"#,
        );
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        document
            .create_bookmark_at_paragraph(0, "HeadingBookmark")
            .unwrap();
        let targets = document.cross_reference_targets();
        let before = document_xml(&document.export_docx().unwrap());
        let range = SelectionRange::collapsed(TextPosition::new(2, 0));
        let mut selection = Some(range);
        let mut history = CommandExecutor::new();
        for target in &targets {
            let selected =
                if target.target_type == crate::word_document::CrossReferenceTargetType::Caption {
                    SelectionRange {
                        anchor: TextPosition::new(2, 0),
                        focus: TextPosition::new(2, 4),
                    }
                } else {
                    range
                };
            assert!(history
                .execute(
                    Box::new(InsertCrossReferenceCommand::for_target(
                        selected,
                        target.clone(),
                        CrossReferenceKind::Number
                    )),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .is_err());
            assert_eq!(document_xml(&document.export_docx().unwrap()), before);
            assert_eq!(selection, Some(range));
            assert!(!history.can_undo());
        }
        assert!(history
            .execute(
                Box::new(InsertCrossReferenceCommand::new(
                    range,
                    "HeadingBookmark".into(),
                    CrossReferenceKind::Number
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .is_err());
        assert_eq!(document_xml(&document.export_docx().unwrap()), before);
        assert!(!history.can_undo());
    }

    #[test]
    fn heading_reference_after_table_uses_body_element_bookmark_coordinates() {
        let source = build_docx_body(
            "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell one</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>cell two</w:t></w:r></w:p></w:tc></w:tr></w:tbl>\
             <w:p><w:pPr><w:outlineLvl w:val=\"0\"/></w:pPr><w:r><w:t>Heading after table</w:t></w:r></w:p>\
             <w:p/>",
        );
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        let target = document
            .cross_reference_targets()
            .into_iter()
            .next()
            .unwrap();
        let range = SelectionRange::collapsed(TextPosition::new(document.paragraph_count() - 1, 0));
        let mut selection = Some(range);
        CommandExecutor::new()
            .execute(
                Box::new(InsertCrossReferenceCommand::for_target(
                    range,
                    target.clone(),
                    CrossReferenceKind::Text,
                )),
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap();
        let bookmark = document
            .document
            .as_ref()
            .unwrap()
            .bookmarks
            .first()
            .unwrap();
        assert_eq!(bookmark.start_paragraph, 1);
        assert_eq!(document.bookmark_render_map().get(&bookmark.name), Some(&2));
        let exported = document.export_docx().unwrap();
        let reopened =
            executor::block_on(WordDocument::load(&exported, &Options::default())).unwrap();
        assert!(reopened
            .paragraph(reopened.paragraph_count() - 1)
            .unwrap()
            .runs
            .iter()
            .any(|run| run.text == target.label));
    }

    fn boundaries(value: &str) -> Vec<usize> {
        let mut result = value
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        result.push(value.len());
        result
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

        #[test]
        fn unicode_bookmarks_and_cross_references_round_trip_at_every_run_boundary(
            target_chars in proptest::collection::vec(
                prop::sample::select(vec!['A', 'z', 'あ', '界', '😀', 'é', ' ']),
                1..24,
            ),
            prefix_chars in proptest::collection::vec(
                prop::sample::select(vec!['p', '前', '🙂']),
                0..12,
            ),
            suffix_chars in proptest::collection::vec(
                prop::sample::select(vec!['s', '後', 'é']),
                0..12,
            ),
            boundary_pick in any::<usize>(),
            page_reference in any::<bool>(),
            name_suffix in 0u16..10_000,
        ) {
            let target = target_chars.into_iter().collect::<String>();
            let prefix = prefix_chars.into_iter().collect::<String>();
            let suffix = suffix_chars.into_iter().collect::<String>();
            let reference_paragraph = format!("{prefix}{suffix}");
            let source = build_docx(&[target.clone(), reference_paragraph]);
            let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
                .expect("load document");
            let baseline = document.export_docx().expect("baseline");
            let name = format!("章_{name_suffix}");
            let caret_boundaries = boundaries(&format!("{prefix}{suffix}"));
            let caret = caret_boundaries[boundary_pick % caret_boundaries.len()];
            let range = SelectionRange::collapsed(TextPosition::new(1, caret));
            let mut selection = Some(range);
            let mut executor = CommandExecutor::new();

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(CreateBookmarkCommand::new(0, name.clone())),
                    &mut context,
                ).expect("create bookmark");
            }
            let bookmarked = document.export_docx().expect("bookmark export");
            let bookmark_xml = document_xml(&bookmarked);
            prop_assert_eq!(bookmark_xml.matches("<w:bookmarkStart").count(), 1);
            prop_assert_eq!(bookmark_xml.matches("<w:bookmarkEnd").count(), 1);
            let expected_name_attribute = format!("w:name=\"{}\"", name);
            prop_assert!(bookmark_xml.contains(&expected_name_attribute));

            let kind = if page_reference {
                CrossReferenceKind::Page
            } else {
                CrossReferenceKind::Text
            };
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                executor.execute(
                    Box::new(InsertCrossReferenceCommand::new(range, name.clone(), kind)),
                    &mut context,
                ).expect("insert cross reference");
            }
            let applied = document.export_docx().expect("cross reference export");
            let xml = document_xml(&applied);
            let instruction = if page_reference { "PAGEREF" } else { "REF" };
            prop_assert!(xml.contains("<w:fldSimple"));
            let expected_instruction = format!("{} {}", instruction, name);
            prop_assert!(xml.contains(&expected_instruction));
            prop_assert!(xml.contains("\\h"));
            let expected_display = if page_reference { "1" } else { target.as_str() };
            prop_assert!(xml.contains(&escape_text(expected_display)));

            let reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
                .expect("reopen document");
            prop_assert!(reopened.document.as_ref().expect("model").bookmarks.iter()
                .any(|bookmark| bookmark.name == name));
            let field_run = reopened.paragraph(1).expect("reference paragraph").runs.iter()
                .find(|run| run.field.is_some()).expect("field run");
            prop_assert_eq!(field_run.text.as_str(), expected_display);
            match (&field_run.field.as_ref().expect("field").kind, page_reference) {
                (RunFieldKind::PageRef { name: actual, hyperlink: true }, true) => {
                    prop_assert_eq!(actual, &name);
                }
                (RunFieldKind::Ref { name: actual, hyperlink: true, .. }, false) => {
                    prop_assert_eq!(actual, &name);
                }
                (actual, expected_page) => prop_assert!(false, "unexpected field {actual:?}, page={expected_page}"),
            }

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut context).expect("undo reference"));
                prop_assert!(executor.undo(&mut context).expect("undo bookmark"));
            }
            prop_assert_eq!(document.export_docx().expect("undo export"), baseline);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut context).expect("redo bookmark"));
                prop_assert!(executor.redo(&mut context).expect("redo reference"));
            }
            prop_assert_eq!(document.export_docx().expect("redo export"), applied);
        }
    }

    #[test]
    fn bookmark_deletion_preserves_text_and_invalid_operations_are_atomic() {
        let source = build_docx(&["Target text".to_string(), "after".to_string()]);
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let range = SelectionRange::collapsed(TextPosition::new(0, 0));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut context = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(CreateBookmarkCommand::new(0, "Target_1".to_string())),
                    &mut context,
                )
                .expect("create bookmark");
            executor
                .execute(
                    Box::new(DeleteBookmarkCommand::new("Target_1".to_string())),
                    &mut context,
                )
                .expect("delete bookmark");
        }
        let deleted = document.export_docx().expect("deleted export");
        assert!(!document_xml(&deleted).contains("bookmarkStart"));
        assert_eq!(
            document
                .paragraph(0)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "Target text"
        );

        for name in [
            "",
            "1startsWithDigit",
            "has space",
            "punct!",
            &"a".repeat(41),
        ] {
            let before = document.export_docx().expect("before invalid");
            let undo_before = executor.can_undo();
            let mut context = CommandContext::new(&mut document, &mut selection);
            assert!(executor
                .execute(
                    Box::new(CreateBookmarkCommand::new(0, name.to_string())),
                    &mut context,
                )
                .is_err());
            assert_eq!(document.export_docx().expect("after invalid"), before);
            assert_eq!(executor.can_undo(), undo_before);
        }

        let before = document.export_docx().expect("before invalid reference");
        let invalid_range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 1));
        let mut context = CommandContext::new(&mut document, &mut selection);
        assert!(executor
            .execute(
                Box::new(InsertCrossReferenceCommand::new(
                    invalid_range,
                    "Missing".to_string(),
                    CrossReferenceKind::Text,
                )),
                &mut context,
            )
            .is_err());
        assert_eq!(document.export_docx().expect("unchanged"), before);
    }

    #[test]
    fn unrelated_edit_preserves_existing_complex_cross_reference_field() {
        let source = build_docx_body(
            "<w:p><w:bookmarkStart w:id=\"7\" w:name=\"Target_1\"/><w:r><w:t>Target text</w:t></w:r><w:bookmarkEnd w:id=\"7\"/></w:p>\
             <w:p><w:r><w:t xml:space=\"preserve\">before </w:t></w:r>\
             <w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>\
             <w:r><w:instrText xml:space=\"preserve\"> REF Target_1 \\h </w:instrText></w:r>\
             <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>\
             <w:r><w:t>Target text</w:t></w:r>\
             <w:r><w:fldChar w:fldCharType=\"end\"/></w:r>\
             <w:r><w:t xml:space=\"preserve\"> after</w:t></w:r></w:p>",
        );
        let mut document = executor::block_on(WordDocument::load(&source, &Options::default()))
            .expect("load document");
        let range = SelectionRange::collapsed(TextPosition::new(1, 0));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();

        {
            let mut context = CommandContext::new(&mut document, &mut selection);
            executor
                .execute(
                    Box::new(InsertTextCommand::new(range, "edited ".to_string(), None)),
                    &mut context,
                )
                .expect("insert adjacent text");
        }

        let exported = document.export_docx().expect("export edited document");
        let xml = document_xml(&exported);
        assert_eq!(xml.matches("w:fldCharType=\"begin\"").count(), 1);
        assert_eq!(xml.matches("w:fldCharType=\"separate\"").count(), 1);
        assert_eq!(xml.matches("w:fldCharType=\"end\"").count(), 1);
        assert!(xml.contains("REF Target_1 \\h"));

        let reopened = executor::block_on(WordDocument::load(&exported, &Options::default()))
            .expect("reopen edited document");
        let paragraph = reopened.paragraph(1).expect("reference paragraph");
        assert_eq!(
            paragraph
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "edited before Target text after"
        );
        let field = paragraph
            .runs
            .iter()
            .find_map(|run| run.field.as_ref())
            .expect("preserved REF field");
        assert!(matches!(
            &field.kind,
            RunFieldKind::Ref {
                name,
                hyperlink: true,
                ..
            } if name == "Target_1"
        ));
    }
}
