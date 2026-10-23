//! Character style command implementation.
//!
//! Assigns (or removes) `w:rStyle` on the selected runs. Word's 文字スタイル is
//! how a document names a recurring inline treatment — a defined term, a
//! cross-reference, an emphasis — instead of hard-coding bold or a colour on
//! every occurrence. Nothing could set it before, so a document that used
//! character styles could be read but never edited without losing them.

use super::run_backup::{capture_paragraph_runs, restore_paragraph_runs, ParagraphRunsBackup};
use super::text_edit::mutate_run_properties;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::Paragraph;
use jade_docx_layout::selection::SelectionRange;
use std::rc::Rc;

/// Applies a character style to the selection, or clears it when `style_id` is
/// `None` (Word の「標準の文字」).
pub struct SetCharacterStyleCommand {
    range: SelectionRange,
    style_id: Option<String>,
    previous_runs: Option<Vec<ParagraphRunsBackup>>,
    previous_selection: Option<SelectionRange>,
}

impl SetCharacterStyleCommand {
    pub fn new(range: SelectionRange, style_id: Option<String>) -> Self {
        Self {
            range,
            style_id,
            previous_runs: None,
            previous_selection: None,
        }
    }
}

impl Command for SetCharacterStyleCommand {
    fn name(&self) -> &'static str {
        "set_character_style"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.range.is_collapsed() {
            return Err(CommandError::InvalidSelection);
        }

        let (start, end) = self.range.normalized();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0
            || start.paragraph >= paragraph_count
            || end.paragraph >= paragraph_count
        {
            return Err(CommandError::ParagraphUnavailable);
        }

        // Accept a display name as well as an id, the same way
        // `SetParagraphStyleCommand` does: a caller reading the gallery has the
        // name in hand, and `w:styleId` is often an opaque token (`a3`).
        let resolved = match self.style_id.as_deref() {
            Some(requested) => {
                let resolved = ctx
                    .document
                    .styles_part
                    .as_ref()
                    .and_then(|styles| styles.resolve_character_style_id(requested))
                    .map(|id| id.to_string());
                match resolved {
                    Some(id) => Some(id),
                    // Applying a style the file does not define would write a
                    // dangling `w:rStyle`; Word renders that as plain text and
                    // the mistake is invisible until someone opens it there.
                    None => {
                        return Err(CommandError::Custom(format!(
                            "character style {requested:?} is not defined in this document"
                        )))
                    }
                }
            }
            None => None,
        };

        self.previous_runs = Some(capture_paragraph_runs(ctx, start.paragraph, end.paragraph)?);
        self.previous_selection = *ctx.selection;

        let style: Option<Rc<str>> = resolved.map(|id| Rc::from(id.as_str()));

        for paragraph_index in start.paragraph..=end.paragraph {
            let zero_width = ctx.document.zero_width_runs(paragraph_index);
            let paragraph = ctx
                .document
                .paragraph_mut(paragraph_index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            let paragraph_len = paragraph_len(paragraph);

            let range_start = if paragraph_index == start.paragraph {
                start.offset.min(paragraph_len)
            } else {
                0
            };
            let range_end = if paragraph_index == end.paragraph {
                end.offset.min(paragraph_len)
            } else {
                paragraph_len
            };

            if range_start >= range_end {
                continue;
            }

            // Each piece's character style before, by where it starts.
            let mut offset = 0;
            let previous: Vec<(usize, usize, Option<Rc<str>>)> = paragraph
                .runs
                .iter()
                .map(|run| {
                    let start = offset;
                    offset += run.text.len();
                    (
                        start,
                        offset,
                        run.properties
                            .as_ref()
                            .and_then(|props| props.style.clone()),
                    )
                })
                .collect();

            let style = style.clone();
            let restyled = mutate_run_properties(
                &mut paragraph.runs,
                range_start,
                range_end,
                &zero_width,
                move |props| {
                    props.style = style.clone();
                },
            );
            let mut offset = 0;
            let mut changes = Vec::new();
            for (index, run) in paragraph.runs.iter().enumerate() {
                let start = offset;
                offset += run.text.len();
                if run.text.is_empty() || start < range_start || start >= range_end {
                    continue;
                }
                let before = previous
                    .iter()
                    .find(|(from, to, _)| *from <= start && start < *to)
                    .and_then(|(_, _, style)| style.clone());
                if before
                    != run
                        .properties
                        .as_ref()
                        .and_then(|props| props.style.clone())
                {
                    changes.push((index, before));
                }
            }
            ctx.document
                .restyle_runs_character_style(paragraph_index, &changes);
            ctx.document
                .sync_paragraph_run_properties(paragraph_index, &restyled);
        }

        *ctx.selection = Some(self.range);
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let backups = self
            .previous_runs
            .take()
            .ok_or_else(|| CommandError::Custom("missing backup for undo".into()))?;
        restore_paragraph_runs(ctx, &backups)?;
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

fn paragraph_len(paragraph: &Paragraph) -> usize {
    paragraph.runs.iter().map(|run| run.text.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandContext, CommandExecutor};
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_layout::selection::TextPosition;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    /// A package whose `styles.xml` defines one character style (`強調`).
    fn build_docx_with_character_style() -> Vec<u8> {
        const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>
"#;
        const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
"#;
        const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>
"#;
        const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="character" w:styleId="Kyocho"><w:name w:val="強調"/><w:rPr><w:b/></w:rPr></w:style>
</w:styles>
"#;
        const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>秘密情報とは</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, body) in [
                ("[Content_Types].xml", CONTENT_TYPES),
                ("_rels/.rels", RELS),
                ("word/_rels/document.xml.rels", DOC_RELS),
                ("word/styles.xml", STYLES),
                ("word/document.xml", DOCUMENT),
            ] {
                zip.start_file(path, options).expect("entry");
                zip.write_all(body.as_bytes()).expect("write");
            }
            zip.finish().expect("finish");
        }
        cursor.into_inner()
    }

    fn load() -> WordDocument {
        let data = build_docx_with_character_style();
        executor::block_on(WordDocument::load(&data, &Options::default())).expect("load docx")
    }

    fn document_xml(bytes: &[u8]) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let mut entry = archive.by_name("word/document.xml").expect("document.xml");
        let mut xml = String::new();
        entry.read_to_string(&mut xml).expect("read document.xml");
        xml
    }

    fn styles_of(word_doc: &WordDocument) -> Vec<Option<String>> {
        word_doc
            .paragraph(0)
            .expect("paragraph")
            .runs
            .iter()
            .map(|run| {
                run.properties
                    .as_ref()
                    .and_then(|props| props.style.as_ref())
                    .map(|style| style.to_string())
            })
            .collect()
    }

    #[test]
    fn applies_a_character_style_to_the_selection_by_name() {
        let mut word_doc = load();
        // 「秘密情報」 is 12 bytes; the trailing 「とは」 must stay unstyled so a
        // partial selection is really partial.
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 12));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    // The gallery hands out the display name, not `Kyocho`.
                    Box::new(SetCharacterStyleCommand::new(
                        range,
                        Some("強調".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("apply character style");
        }

        let styles = styles_of(&word_doc);
        assert_eq!(
            styles,
            vec![Some("Kyocho".to_string()), None],
            "only the selected run carries w:rStyle"
        );
    }

    #[test]
    fn clears_a_character_style() {
        let mut word_doc = load();
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 12));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(SetCharacterStyleCommand::new(
                        range,
                        Some("Kyocho".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("apply");
            // Clearing has to write None. A merge that only sets `Some` fields
            // can turn a style on and never off, which is the reason
            // `mutate_run_properties` exists.
            executor
                .execute(
                    Box::new(SetCharacterStyleCommand::new(range, None)),
                    &mut ctx,
                )
                .expect("clear");
        }

        assert!(
            styles_of(&word_doc).iter().all(Option::is_none),
            "clearing removes w:rStyle, got {:?}",
            styles_of(&word_doc)
        );
    }

    #[test]
    fn clears_a_character_style_after_export_and_reopen() {
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 12));
        let mut word_doc = load();
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
            executor
                .execute(
                    Box::new(SetCharacterStyleCommand::new(
                        range,
                        Some("Kyocho".to_string()),
                    )),
                    &mut ctx,
                )
                .expect("apply");
        }
        let applied = word_doc.export_docx().expect("export applied style");
        assert!(document_xml(&applied).contains("w:rStyle"));

        let mut reopened = executor::block_on(WordDocument::load(&applied, &Options::default()))
            .expect("reopen styled document");
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut reopened, &mut selection);
            executor
                .execute(
                    Box::new(SetCharacterStyleCommand::new(range, None)),
                    &mut ctx,
                )
                .expect("clear reopened style");
        }
        let cleared = reopened.export_docx().expect("export cleared style");
        assert!(
            !document_xml(&cleared).contains("w:rStyle"),
            "clearing after reopen must remove both modeled and preserved w:rStyle"
        );
    }

    #[test]
    fn rejects_a_style_the_document_does_not_define() {
        let mut word_doc = load();
        let range = SelectionRange::new(TextPosition::new(0, 0), TextPosition::new(0, 12));
        let mut selection = Some(range);
        let mut executor = CommandExecutor::new();
        let mut ctx = CommandContext::new(&mut word_doc, &mut selection);
        // Writing a dangling `w:rStyle` is worse than refusing: Word renders it
        // as plain text, so the mistake stays invisible until someone opens the
        // file there.
        let error = executor
            .execute(
                Box::new(SetCharacterStyleCommand::new(
                    range,
                    Some("存在しないスタイル".to_string()),
                )),
                &mut ctx,
            )
            .expect_err("undefined style must be refused");
        assert!(
            error.to_string().contains("not defined"),
            "unexpected error: {error}"
        );
    }
}
