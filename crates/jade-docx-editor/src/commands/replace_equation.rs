//! Replacing an equation's OMML in place, as the equation editor does on
//! every keystroke inside a math zone, with atomic history.

use super::{Command, CommandContext, CommandError, CommandResult, HistoryRenderFootprint};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

pub struct ReplaceEquationCommand {
    paragraph: usize,
    run_index: Option<usize>,
    omml: String,
    previous_document: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

impl ReplaceEquationCommand {
    /// `paragraph` is the rendered paragraph the equation is in (a display
    /// equation's own); `run_index` the math run of an inline one.
    pub fn new(paragraph: usize, run_index: Option<usize>, omml: String) -> Self {
        Self {
            paragraph,
            run_index,
            omml,
            previous_document: None,
            previous_selection: None,
        }
    }
}

impl Command for ReplaceEquationCommand {
    fn name(&self) -> &'static str {
        "replace_equation"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_selection = *ctx.selection;
        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or_else(|| CommandError::Custom("document is unavailable".into()))?;
        match ctx
            .document
            .replace_equation_at(self.paragraph, self.run_index, &self.omml)
        {
            Ok(()) => {
                self.previous_document = Some(snapshot);
                self.previous_selection = previous_selection;
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(CommandError::Custom(error))
            }
        }
    }

    fn history_render_footprint(&self) -> HistoryRenderFootprint {
        HistoryRenderFootprint::Structural
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self
            .previous_document
            .take()
            .ok_or_else(|| CommandError::Custom("equation history is missing".into()))?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    const M: &str = r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;

    fn build_docx(body: &str) -> Vec<u8> {
        let document_xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" {M}><w:body>{body}</w:body></w:document>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, content) in [
                (
                    "[Content_Types].xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
                ),
                ("word/document.xml", document_xml.as_str()),
            ] {
                zip.start_file(path, options).expect("start zip entry");
                zip.write_all(content.as_bytes()).expect("write zip entry");
            }
            zip.finish().expect("finish zip");
        }
        cursor.into_inner()
    }

    fn part(bytes: &[u8], path: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open zip");
        let mut entry = archive.by_name(path).expect("part");
        let mut value = String::new();
        entry.read_to_string(&mut value).expect("read part");
        value
    }

    fn replace(
        document: &mut WordDocument,
        executor: &mut CommandExecutor,
        paragraph: usize,
        run_index: Option<usize>,
        omml: &str,
    ) -> CommandResult<()> {
        let mut selection = None;
        let mut context = CommandContext::new(document, &mut selection);
        executor.execute(
            Box::new(ReplaceEquationCommand::new(
                paragraph,
                run_index,
                omml.to_string(),
            )),
            &mut context,
        )
    }

    #[test]
    fn an_inline_equation_is_replaced_in_place_and_undone() {
        let body = r#"<w:p><w:r><w:t xml:space="preserve">a </w:t></w:r><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath><w:r><w:t xml:space="preserve"> b </w:t></w:r><m:oMath><m:r><m:t>y</m:t></m:r></m:oMath></w:p>"#;
        let source = build_docx(body);
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        let run_index = document
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .enumerate()
            .filter(|(_, run)| run.math.is_some())
            .nth(1)
            .map(|(index, _)| index)
            .unwrap();
        let mut executor = CommandExecutor::new();
        let omml = format!(
            r#"<m:oMath {M}><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>z</m:t></m:r></m:den></m:f></m:oMath>"#
        );
        replace(&mut document, &mut executor, 0, Some(run_index), &omml).unwrap();
        let saved = document.export_docx().unwrap();
        let xml = part(&saved, "word/document.xml");
        assert!(
            xml.contains("<m:t>x</m:t>"),
            "the first equation stays: {xml}"
        );
        assert!(xml.contains("<m:den><m:r><m:t>z</m:t>"), "{xml}");
        assert!(!xml.contains("<m:t>y</m:t>"), "{xml}");
        assert!(xml.contains(" b "));
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        let math = reopened.paragraph(0).unwrap().runs[run_index]
            .math
            .clone()
            .unwrap();
        assert!(math.raw_omml.contains("m:f"));
        let mut selection = None;
        let mut context = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut context).unwrap();
        let xml = part(&document.export_docx().unwrap(), "word/document.xml");
        assert!(xml.contains("<m:t>y</m:t>"));
    }

    #[test]
    fn a_display_equation_is_replaced_in_its_wrapper_paragraph() {
        let body = r#"<w:p><w:r><w:t>before</w:t></w:r></w:p><w:p><m:oMathPara><m:oMath><m:r><m:t>x=1</m:t></m:r></m:oMath></m:oMathPara></w:p><w:p><w:r><w:t>after</w:t></w:r></w:p>"#;
        let source = build_docx(body);
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        assert!(document.is_math_surface(1));
        let mut executor = CommandExecutor::new();
        let omml = format!(
            r#"<m:oMathPara {M}><m:oMath><m:rad><m:radPr><m:degHide m:val="1"/></m:radPr><m:deg/><m:e><m:r><m:t>2</m:t></m:r></m:e></m:rad></m:oMath></m:oMathPara>"#
        );
        replace(&mut document, &mut executor, 1, None, &omml).unwrap();
        let xml = part(&document.export_docx().unwrap(), "word/document.xml");
        assert!(xml.contains("<m:rad>"), "{xml}");
        assert!(!xml.contains("x=1"));
        assert_eq!(xml.matches("<m:oMathPara").count(), 1);
        assert!(xml.contains("before") && xml.contains("after"));
        assert!(replace(&mut document, &mut executor, 0, None, &omml).is_err());
    }
}
