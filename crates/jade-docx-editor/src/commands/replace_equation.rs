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
    fn a_text_box_inline_equation_saves_reopens_undoes_and_redoes_in_its_story() {
        let body = r#"<w:p><m:oMath><m:r><m:t>body</m:t></m:r></m:oMath><w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="keep-shape" style="width:180pt;height:90pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>neighbor</w:t></w:r><m:oMath><m:r><m:t>other</m:t></m:r></m:oMath></w:p><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t xml:space="preserve">inside </w:t></w:r><m:oMath><m:r><m:t>first</m:t></m:r></m:oMath><w:r><w:t xml:space="preserve"> between </w:t></w:r><m:oMath><m:r><m:t>target</m:t></m:r></m:oMath><w:r><w:t xml:space="preserve"> after</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p><w:p><w:r><w:t>outside</w:t></w:r></w:p>"#;
        let mut document =
            executor::block_on(WordDocument::load(&build_docx(body), &Options::default())).unwrap();
        let (paragraph, run_index) = (0..document.paragraph_count())
            .find_map(|index| {
                document
                    .paragraph(index)?
                    .runs
                    .iter()
                    .enumerate()
                    .find(|(_, run)| {
                        run.math
                            .as_ref()
                            .is_some_and(|math| math.raw_omml.contains("target"))
                    })
                    .map(|(run, _)| (index, run))
            })
            .expect("the box equation must be modeled");
        let original = part(&document.export_docx().unwrap(), "word/document.xml");
        let omml = format!(
            r#"<m:oMath {M}><m:f><m:num><m:r><m:t>updated</m:t></m:r></m:num><m:den><m:r><m:t>denominator</m:t></m:r></m:den></m:f></m:oMath>"#
        );
        let mut history = CommandExecutor::new();
        replace(
            &mut document,
            &mut history,
            paragraph,
            Some(run_index),
            &omml,
        )
        .unwrap();
        let saved = document.export_docx().unwrap();
        let changed = part(&saved, "word/document.xml");
        assert!(!changed.contains("<m:t>target</m:t>"), "{changed}");
        for preserved in [
            "<m:t>body</m:t>",
            "<m:t>other</m:t>",
            "<m:t>first</m:t>",
            " between ",
            " after",
            "keep-shape",
            "outside",
            "w:val=\"right\"",
        ] {
            assert!(
                changed.contains(preserved),
                "missing {preserved}: {changed}"
            );
        }
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        let math = reopened.paragraph(paragraph).unwrap().runs[run_index]
            .math
            .as_ref()
            .unwrap();
        assert!(math.raw_omml.contains("updated") && math.raw_omml.contains("denominator"));
        let mut selection = None;
        let mut context = CommandContext::new(&mut document, &mut selection);
        history.undo(&mut context).unwrap();
        assert_eq!(
            part(
                &context.document.export_docx().unwrap(),
                "word/document.xml"
            ),
            original
        );
        history.redo(&mut context).unwrap();
        assert_eq!(
            part(
                &context.document.export_docx().unwrap(),
                "word/document.xml"
            ),
            changed
        );
    }

    #[test]
    fn a_text_box_display_equation_in_a_table_keeps_its_story_on_save_and_undo() {
        let body = r#"<w:p><m:oMathPara><m:oMath><m:r><m:t>body-before</m:t></m:r></m:oMath></m:oMathPara></w:p><w:p><w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml" id="keep-box" style="width:180pt;height:120pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>inside</w:t></w:r></w:p><w:p><m:oMathPara><m:oMath><m:r><m:t>box-first</m:t></m:r></m:oMath></m:oMathPara></w:p><w:tbl><w:tr><w:tc><w:p><w:pPr><w:jc w:val="right"/></w:pPr><m:oMathPara><m:oMath><m:r><m:t>box-target</m:t></m:r></m:oMath></m:oMathPara></w:p></w:tc></w:tr></w:tbl></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p><w:p><m:oMathPara><m:oMath><m:r><m:t>body-after</m:t></m:r></m:oMath></m:oMathPara></w:p>"#;
        let mut document =
            executor::block_on(WordDocument::load(&build_docx(body), &Options::default())).unwrap();
        let target = (0..document.paragraph_count())
            .filter(|index| document.is_math_surface(*index))
            .nth(2)
            .expect("the box table equation must have a surface");
        let original = part(&document.export_docx().unwrap(), "word/document.xml");
        let omml = format!(
            r#"<m:oMathPara {M}><m:oMath><m:f><m:num><m:r><m:t>updated</m:t></m:r></m:num><m:den><m:r><m:t>denominator</m:t></m:r></m:den></m:f></m:oMath></m:oMathPara>"#
        );
        let mut history = CommandExecutor::new();
        replace(&mut document, &mut history, target, None, &omml).unwrap();
        assert!(document
            .math_paragraph(target)
            .unwrap()
            .raw_omml
            .contains("updated"));
        let saved = document.export_docx().unwrap();
        let changed = part(&saved, "word/document.xml");
        assert!(!changed.contains("box-target"), "{changed}");
        for preserved in [
            "body-before",
            "body-after",
            "box-first",
            "inside",
            "keep-box",
            "w:val=\"right\"",
        ] {
            assert!(
                changed.contains(preserved),
                "missing {preserved}: {changed}"
            );
        }
        assert_eq!(changed.matches("<m:oMathPara").count(), 4);
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert!(reopened
            .math_paragraph(target)
            .unwrap()
            .raw_omml
            .contains("updated"));
        let mut selection = None;
        let mut context = CommandContext::new(&mut document, &mut selection);
        history.undo(&mut context).unwrap();
        assert_eq!(
            part(
                &context.document.export_docx().unwrap(),
                "word/document.xml"
            ),
            original
        );
        history.redo(&mut context).unwrap();
        assert_eq!(
            part(
                &context.document.export_docx().unwrap(),
                "word/document.xml"
            ),
            changed
        );
    }

    #[test]
    fn a_text_box_display_equation_updates_drawingml_and_vml_fallback_together() {
        let content = r#"<w:p><m:oMathPara><m:oMath><m:r><m:t>first</m:t></m:r></m:oMath></m:oMathPara></w:p><w:p><m:oMathPara><m:oMath><m:r><m:t>target</m:t></m:r></m:oMath></m:oMathPara></w:p>"#;
        let body = format!(
            r#"<w:p><m:oMathPara><m:oMath><m:r><m:t>body-before</m:t></m:r></m:oMath></m:oMathPara></w:p>
            <w:p><w:r><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml">
            <mc:Choice Requires="wps"><w:drawing><wp:inline><wp:extent cx="2286000" cy="1143000"/><wp:docPr id="1" name="keep-drawing"/>
            <a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></wps:spPr><wps:txbx><w:txbxContent>{content}</w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></mc:Choice>
            <mc:Fallback><w:pict><v:shape id="keep-fallback" style="width:180pt;height:90pt"><v:textbox><w:txbxContent>{content}</w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback>
            </mc:AlternateContent></w:r></w:p>
            <w:p><m:oMathPara><m:oMath><m:r><m:t>body-after</m:t></m:r></m:oMath></m:oMathPara></w:p>"#
        );
        let mut document =
            executor::block_on(WordDocument::load(&build_docx(&body), &Options::default()))
                .unwrap();
        let target = (0..document.paragraph_count())
            .filter(|index| document.is_math_surface(*index))
            .nth(2)
            .unwrap();
        assert!(document
            .math_paragraph(target)
            .unwrap()
            .raw_omml
            .contains("target"));
        let mut history = CommandExecutor::new();
        let omml = format!(
            r#"<m:oMathPara {M}><m:oMath><m:r><m:t>updated</m:t></m:r></m:oMath></m:oMathPara>"#
        );
        replace(&mut document, &mut history, target, None, &omml).unwrap();
        let saved = document.export_docx().unwrap();
        let xml = part(&saved, "word/document.xml");
        assert_eq!(xml.matches("<m:t>updated</m:t>").count(), 2, "{xml}");
        assert_eq!(xml.matches("<m:t>first</m:t>").count(), 2, "{xml}");
        assert!(!xml.contains("<m:t>target</m:t>"), "{xml}");
        for preserved in ["body-before", "body-after", "keep-drawing", "keep-fallback"] {
            assert!(xml.contains(preserved), "{xml}");
        }
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert!(reopened
            .math_paragraph(target)
            .unwrap()
            .raw_omml
            .contains("updated"));
    }

    #[test]
    fn a_text_box_inline_equation_updates_drawingml_and_vml_fallback_together() {
        let content = r#"<w:p><m:oMath><m:r><m:t>first</m:t></m:r></m:oMath><w:r><w:t> between </w:t></w:r><m:oMath><m:r><m:t>target</m:t></m:r></m:oMath></w:p>"#;
        let body = format!(
            r#"<w:p><m:oMath><m:r><m:t>body-before</m:t></m:r></m:oMath></w:p>
            <w:p><w:r><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml">
            <mc:Choice Requires="wps"><w:drawing><wp:inline><wp:extent cx="2286000" cy="1143000"/><wp:docPr id="1" name="keep-drawing"/>
            <a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></wps:spPr><wps:txbx><w:txbxContent>{content}</w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></mc:Choice>
            <mc:Fallback><w:pict><v:shape id="keep-fallback" style="width:180pt;height:90pt"><v:textbox><w:txbxContent>{content}</w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback>
            </mc:AlternateContent></w:r></w:p>
            <w:p><m:oMath><m:r><m:t>body-after</m:t></m:r></m:oMath></w:p>"#
        );
        let mut document =
            executor::block_on(WordDocument::load(&build_docx(&body), &Options::default()))
                .unwrap();
        let (target, run_index) = (0..document.paragraph_count())
            .find_map(|index| {
                document
                    .paragraph(index)?
                    .runs
                    .iter()
                    .enumerate()
                    .find(|(_, run)| {
                        run.math
                            .as_ref()
                            .is_some_and(|math| math.raw_omml.contains("target"))
                    })
                    .map(|(run, _)| (index, run))
            })
            .unwrap();
        let original = part(&document.export_docx().unwrap(), "word/document.xml");
        let mut history = CommandExecutor::new();
        let omml = format!(r#"<m:oMath {M}><m:r><m:t>updated</m:t></m:r></m:oMath>"#);
        replace(&mut document, &mut history, target, Some(run_index), &omml).unwrap();
        let saved = document.export_docx().unwrap();
        let xml = part(&saved, "word/document.xml");
        assert_eq!(xml.matches("<m:t>updated</m:t>").count(), 2, "{xml}");
        assert_eq!(xml.matches("<m:t>first</m:t>").count(), 2, "{xml}");
        assert!(!xml.contains("<m:t>target</m:t>"), "{xml}");
        for preserved in ["body-before", "body-after", "keep-drawing", "keep-fallback"] {
            assert!(xml.contains(preserved), "{xml}");
        }
        let reopened = executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert!(reopened.paragraph(target).unwrap().runs[run_index]
            .math
            .as_ref()
            .unwrap()
            .raw_omml
            .contains("updated"));
        let mut selection = None;
        let mut context = CommandContext::new(&mut document, &mut selection);
        history.undo(&mut context).unwrap();
        assert_eq!(
            part(
                &context.document.export_docx().unwrap(),
                "word/document.xml"
            ),
            original
        );
        history.redo(&mut context).unwrap();
        assert_eq!(
            part(
                &context.document.export_docx().unwrap(),
                "word/document.xml"
            ),
            xml
        );
    }

    fn build_docx_note(body: &str, kind: &str, notes: String) -> Vec<u8> {
        let part_path = format!("word/{kind}s.xml");
        let source = build_docx(body);
        let mut input = zip::ZipArchive::new(std::io::Cursor::new(source)).unwrap();
        let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for index in 0..input.len() {
            let mut entry = input.by_index(index).unwrap();
            let name = entry.name().unwrap().into_owned();
            let mut content = String::new();
            entry.read_to_string(&mut content).unwrap();
            if name == "[Content_Types].xml" {
                content = content.replace("</Types>", &format!(r#"<Override PartName="/{part_path}" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}s+xml"/></Types>"#));
            }
            output
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            output.write_all(content.as_bytes()).unwrap();
        }
        for (name, content) in [
            (part_path.clone(), notes),
            (
                "word/_rels/document.xml.rels".to_string(),
                format!(
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rNote" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}s" Target="{kind}s.xml"/></Relationships>"#
                ),
            ),
        ] {
            output
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            output.write_all(content.as_bytes()).unwrap();
        }
        output.finish().unwrap().into_inner()
    }

    #[test]
    fn inline_equations_in_footnotes_and_endnotes_keep_their_story_and_history() {
        for kind in ["footnote", "endnote"] {
            let body = format!(
                r#"<w:p><m:oMath><m:r><m:t>body</m:t></m:r></m:oMath><w:r><w:{kind}Reference w:id="7"/></w:r></w:p>"#
            );
            let notes = format!(
                r#"<w:{kind}s xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" {M}><w:{kind} w:id="7"><w:p><w:r><w:t>neighbor</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>prefix</w:t></w:r><m:oMath><m:r><m:t>first</m:t></m:r></m:oMath><m:oMath><m:r><m:t>target</m:t></m:r></m:oMath><w:r><w:t>suffix</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:{kind}><w:{kind} w:id="8"><w:p><w:r><w:t>other-note</w:t></w:r></w:p></w:{kind}></w:{kind}s>"#
            );
            let part_path = format!("word/{kind}s.xml");
            let source = build_docx_note(&body, kind, notes);
            let mut document =
                executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
            let (target, run_index) = (0..document.paragraph_count())
                .find_map(|index| {
                    document
                        .paragraph(index)?
                        .runs
                        .iter()
                        .enumerate()
                        .find(|(_, run)| {
                            run.math
                                .as_ref()
                                .is_some_and(|math| math.raw_omml.contains("target"))
                        })
                        .map(|(run, _)| (index, run))
                })
                .expect("note equation is modeled");
            let original = document.export_docx().unwrap();
            let mut history = CommandExecutor::new();
            let omml = format!(r#"<m:oMath {M}><m:r><m:t>updated</m:t></m:r></m:oMath>"#);
            replace(&mut document, &mut history, target, Some(run_index), &omml).unwrap();
            let saved = document.export_docx().unwrap();
            assert_eq!(
                part(&saved, "word/document.xml"),
                part(&original, "word/document.xml")
            );
            let changed = part(&saved, &part_path);
            assert!(changed.contains("<m:t>updated</m:t>"), "{kind}: {changed}");
            assert!(!changed.contains("<m:t>target</m:t>"), "{kind}: {changed}");
            for preserved in [
                "first",
                "prefix",
                "suffix",
                "neighbor",
                "other-note",
                r#"w:val="right""#,
            ] {
                assert!(changed.contains(preserved), "{kind}: {changed}");
            }
            let reopened =
                executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
            assert!(reopened.paragraph(target).unwrap().runs[run_index]
                .math
                .as_ref()
                .unwrap()
                .raw_omml
                .contains("updated"));
            let mut selection = None;
            let mut context = CommandContext::new(&mut document, &mut selection);
            history.undo(&mut context).unwrap();
            assert_eq!(
                part(&context.document.export_docx().unwrap(), &part_path),
                part(&original, &part_path)
            );
            history.redo(&mut context).unwrap();
            assert_eq!(
                part(&context.document.export_docx().unwrap(), &part_path),
                changed
            );
        }
    }

    #[test]
    fn display_equations_in_note_tables_keep_their_location_and_history() {
        for kind in ["footnote", "endnote"] {
            let body = format!(
                r#"<w:p><m:oMathPara><m:oMath><m:r><m:t>body-display</m:t></m:r></m:oMath></m:oMathPara></w:p><w:p><m:oMath><m:r><m:t>body</m:t></m:r></m:oMath><w:r><w:{kind}Reference w:id="7"/></w:r></w:p>"#
            );
            let notes = format!(
                r#"<w:{kind}s xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" {M}><w:{kind} w:id="7"><w:p><w:r><w:t>neighbor</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>prefix</w:t></w:r><m:oMathPara><m:oMath><m:r><m:t>first</m:t></m:r></m:oMath></m:oMathPara></w:p><w:p><w:pPr><w:jc w:val="right"/></w:pPr><m:oMathPara><m:oMath><m:r><m:t>target</m:t></m:r></m:oMath></m:oMathPara><w:r><w:t>suffix</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:{kind}><w:{kind} w:id="8"><w:p><w:r><w:t>other-note</w:t></w:r></w:p><w:p><m:oMathPara><m:oMath><m:r><m:t>other-note-equation</m:t></m:r></m:oMath></m:oMathPara></w:p></w:{kind}></w:{kind}s>"#
            );
            let part_path = format!("word/{kind}s.xml");
            let source = build_docx_note(&body, kind, notes);
            let mut document =
                executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
            let target = (0..document.paragraph_count())
                .find(|&index| {
                    document
                        .math_paragraph(index)
                        .is_some_and(|math| math.raw_omml.contains("target"))
                })
                .expect("note display equation is addressable");
            let original = document.export_docx().unwrap();
            let mut history = CommandExecutor::new();
            let omml = format!(
                r#"<m:oMathPara {M}><m:oMath><m:r><m:t>updated</m:t></m:r></m:oMath></m:oMathPara>"#
            );
            replace(&mut document, &mut history, target, None, &omml).unwrap();
            let saved = document.export_docx().unwrap();
            assert_eq!(
                part(&saved, "word/document.xml"),
                part(&original, "word/document.xml")
            );
            let changed = part(&saved, &part_path);
            assert!(changed.contains("<m:t>updated</m:t>"), "{kind}: {changed}");
            assert!(!changed.contains("<m:t>target</m:t>"), "{kind}: {changed}");
            for preserved in [
                "first",
                "prefix",
                "suffix",
                "neighbor",
                "other-note",
                "other-note-equation",
                r#"w:val="right""#,
            ] {
                assert!(changed.contains(preserved), "{kind}: {changed}");
            }
            let reopened =
                executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
            assert!(reopened
                .math_paragraph(target)
                .unwrap()
                .raw_omml
                .contains("updated"));
            let mut selection = None;
            let mut context = CommandContext::new(&mut document, &mut selection);
            history.undo(&mut context).unwrap();
            assert_eq!(
                part(&context.document.export_docx().unwrap(), &part_path),
                part(&original, &part_path)
            );
            history.redo(&mut context).unwrap();
            assert_eq!(
                part(&context.document.export_docx().unwrap(), &part_path),
                changed
            );
        }
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
