//! A paragraph's own formatting carried to another paragraph, as Word's
//! paste carries it with the paragraph mark: indents, spacing, numbering,
//! tabs, borders, shading and the rest of `w:pPr`, exactly — including the
//! absence of what the source did not have.

use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::ParagraphProperties;
use crate::docx_xml::{XmlElement, XmlNode};
use crate::word_document::WordDocument;
use jade_docx_layout::selection::SelectionRange;

/// A paragraph's own formatting as a copy takes it: its properties and its
/// `w:pPr`, without the section break it may end, its revision marks or
/// its identity.
#[derive(Clone, Debug)]
pub struct ParagraphFormat {
    properties: Option<ParagraphProperties>,
    xml: Option<XmlElement>,
}

impl PartialEq for ParagraphFormat {
    fn eq(&self, other: &Self) -> bool {
        format!("{self:?}") == format!("{other:?}")
    }
}

/// `w:pPr` children a copied format leaves behind: the section the
/// paragraph ends and the record of a tracked change.
const NOT_COPIED: [&str; 2] = ["w:sectPr", "w:pPrChange"];
/// Children of the mark's `w:rPr` that record tracked changes.
const MARK_REVISIONS: [&str; 4] = ["w:ins", "w:del", "w:moveFrom", "w:moveTo"];

impl WordDocument {
    /// Paragraph `index`'s own formatting, to give another paragraph with
    /// [`SetParagraphFormatCommand`].
    pub fn paragraph_format(&self, index: usize) -> Option<ParagraphFormat> {
        let mut properties = self.paragraph(index)?.properties.clone();
        if let Some(properties) = properties.as_mut() {
            properties.section = None;
            properties.para_id = None;
            properties.mark_revision = None;
        }
        let xml = snapshot_paragraph_properties_xml(self, index).map(|mut xml| {
            xml.children.retain(
                |child| !matches!(child, XmlNode::Element(element) if NOT_COPIED.contains(&element.name.as_str())),
            );
            for child in &mut xml.children {
                if let XmlNode::Element(element) = child {
                    if element.name == "w:rPr" {
                        element.children.retain(|mark| {
                            !matches!(mark, XmlNode::Element(element) if MARK_REVISIONS.contains(&element.name.as_str()))
                        });
                    }
                }
            }
            xml
        });
        Some(ParagraphFormat { properties, xml })
    }
}

/// Gives a paragraph a copied [`ParagraphFormat`] in place of its own,
/// keeping the section break it ends, its revision marks and its identity.
pub struct SetParagraphFormatCommand {
    paragraph: usize,
    format: ParagraphFormat,
    previous: Option<(Option<ParagraphProperties>, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetParagraphFormatCommand {
    pub fn new(paragraph: usize, format: ParagraphFormat) -> Self {
        Self {
            paragraph,
            format,
            previous: None,
            previous_selection: None,
        }
    }
}

impl Command for SetParagraphFormatCommand {
    fn name(&self) -> &'static str {
        "set_paragraph_format"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let index = self.paragraph;
        if ctx.document.is_math_surface(index) {
            return Err(CommandError::ParagraphUnavailable);
        }
        let previous = ctx
            .document
            .paragraph(index)
            .map(|paragraph| paragraph.properties.clone())
            .ok_or(CommandError::ParagraphUnavailable)?;
        let previous_xml = snapshot_paragraph_properties_xml(ctx.document, index);

        let mut properties = self.format.properties.clone();
        if let Some(own) = previous.as_ref() {
            let kept = properties.get_or_insert_with(Default::default);
            kept.section = own.section.clone();
            kept.para_id = own.para_id.clone();
            kept.mark_revision = own.mark_revision.clone();
        }
        // The section break the paragraph ends stays with it.
        let section = previous_xml.as_ref().and_then(|xml| {
            xml.children
                .iter()
                .find(|child| matches!(child, XmlNode::Element(element) if element.name == "w:sectPr"))
                .cloned()
        });
        let xml = match (self.format.xml.clone(), section) {
            (Some(mut xml), Some(section)) => {
                xml.children.push(section);
                Some(xml)
            }
            (None, Some(section)) => previous_xml.clone().map(|mut own| {
                own.children = vec![section];
                own
            }),
            (xml, None) => xml,
        };

        ctx.document
            .paragraph_mut(index)
            .ok_or(CommandError::ParagraphUnavailable)?
            .properties = properties;
        restore_paragraph_properties_xml(ctx.document, index, xml.as_ref());
        self.previous = Some((previous, previous_xml));
        self.previous_selection = *ctx.selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (properties, xml) = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("paragraph format snapshot is missing".into()))?;
        ctx.document
            .paragraph_mut(self.paragraph)
            .ok_or(CommandError::ParagraphUnavailable)?
            .properties = properties;
        restore_paragraph_properties_xml(ctx.document, self.paragraph, xml.as_ref());
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::set_paragraph_indent::SetParagraphIndentCommand;
    use crate::commands::CommandExecutor;
    use crate::Options;
    use futures::executor;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn docx(body: &str) -> Vec<u8> {
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr/></w:body></w:document>"
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
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
                zip.start_file(path, options).unwrap();
                zip.write_all(value.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn open(body: &str) -> WordDocument {
        executor::block_on(WordDocument::load(&docx(body), &Options::default())).unwrap()
    }

    fn saved(document: &WordDocument) -> String {
        let bytes = document.export_docx().unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut xml)
            .unwrap();
        xml
    }

    #[test]
    fn a_copied_format_replaces_the_paragraphs_own_and_undoes() {
        let mut document = open(
            "<w:p><w:pPr><w:spacing w:after=\"240\"/><w:jc w:val=\"center\"/></w:pPr><w:r><w:t>source</w:t></w:r></w:p>\
             <w:p><w:pPr><w:ind w:left=\"720\"/></w:pPr><w:r><w:t>target</w:t></w:r></w:p>",
        );
        let format = document.paragraph_format(0).unwrap();
        let before = saved(&document);
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetParagraphFormatCommand::new(1, format)),
                &mut ctx,
            )
            .unwrap();
        let properties = document.paragraph(1).unwrap().properties.clone().unwrap();
        assert!(
            properties.indent.is_none(),
            "the target's own indent is gone"
        );
        assert_eq!(properties.alignment.as_deref(), Some("center"));
        let xml = saved(&document);
        let target = &xml[xml.find("source").unwrap()..];
        assert!(target.contains("w:jc"), "{xml}");
        assert!(!target.contains("w:ind"), "{xml}");

        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor.undo(&mut ctx).unwrap();
        assert_eq!(saved(&document), before);
    }

    #[test]
    fn the_section_break_a_paragraph_ends_stays_with_it() {
        let mut document = open(
            "<w:p><w:pPr><w:ind w:left=\"720\"/><w:sectPr><w:type w:val=\"continuous\"/></w:sectPr></w:pPr><w:r><w:t>ends a section</w:t></w:r></w:p>\
             <w:p><w:pPr><w:jc w:val=\"right\"/></w:pPr><w:r><w:t>plain</w:t></w:r></w:p>",
        );
        // A copy of the first leaves its section break behind.
        let format = document.paragraph_format(0).unwrap();
        let mut executor = CommandExecutor::new();
        let mut selection = None;
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetParagraphFormatCommand::new(1, format)),
                &mut ctx,
            )
            .unwrap();
        let xml = saved(&document);
        assert_eq!(
            xml.matches("w:type w:val=\"continuous\"").count(),
            1,
            "{xml}"
        );
        // Given another's format, the first keeps the break it ends.
        let plain = document.paragraph_format(1).unwrap();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(
                Box::new(SetParagraphIndentCommand::new(
                    1,
                    1,
                    Some(0.0),
                    None,
                    None,
                    None,
                )),
                &mut ctx,
            )
            .unwrap();
        let mut ctx = CommandContext::new(&mut document, &mut selection);
        executor
            .execute(Box::new(SetParagraphFormatCommand::new(0, plain)), &mut ctx)
            .unwrap();
        let xml = saved(&document);
        let first = &xml[..xml.find("ends a section").unwrap()];
        assert!(first.contains("continuous"), "{xml}");
        assert!(document
            .paragraph(0)
            .unwrap()
            .properties
            .as_ref()
            .unwrap()
            .section
            .is_some());
    }
}
