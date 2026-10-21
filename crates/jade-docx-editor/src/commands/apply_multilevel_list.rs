//! Applies one of Jade's stable nine-level list-gallery definitions.

use super::paragraph_property_sync::sync_paragraph_numbering_xml;
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::Numbering;
use crate::numbering::{MultilevelListPreset, NumberingLevel};
use crate::word_document::WordDocumentSnapshot;
use jade_docx_layout::selection::SelectionRange;

pub struct ApplyMultilevelListCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    definition: MultilevelListDefinition,
    level: u32,
    previous: Option<WordDocumentSnapshot>,
    previous_selection: Option<SelectionRange>,
}

enum MultilevelListDefinition {
    Preset(MultilevelListPreset),
    Custom(Vec<NumberingLevel>),
}

impl ApplyMultilevelListCommand {
    pub fn new(
        start_paragraph: usize,
        end_paragraph: usize,
        preset: MultilevelListPreset,
        level: u32,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            definition: MultilevelListDefinition::Preset(preset),
            level,
            previous: None,
            previous_selection: None,
        }
    }

    pub fn new_custom(
        start_paragraph: usize,
        end_paragraph: usize,
        levels: Vec<NumberingLevel>,
        level: u32,
    ) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            definition: MultilevelListDefinition::Custom(levels),
            level,
            previous: None,
            previous_selection: None,
        }
    }

    fn normalized_range(&self) -> (usize, usize) {
        if self.start_paragraph <= self.end_paragraph {
            (self.start_paragraph, self.end_paragraph)
        } else {
            (self.end_paragraph, self.start_paragraph)
        }
    }
}

impl Command for ApplyMultilevelListCommand {
    fn name(&self) -> &'static str {
        "apply_multilevel_list"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }
        if self.level > 8 {
            return Err(CommandError::Custom(
                "multilevel list level must be between 0 and 8".to_string(),
            ));
        }
        if let MultilevelListDefinition::Custom(levels) = &self.definition {
            validate_custom_levels(levels, self.level)?;
        }

        let snapshot = ctx
            .document
            .snapshot_including_package()
            .ok_or(CommandError::DocumentUnavailable)?;
        let previous_selection = *ctx.selection;
        let result = (|| {
            let num_id = match &self.definition {
                MultilevelListDefinition::Preset(preset) => {
                    ctx.document.ensure_multilevel_numbering_preset(*preset)
                }
                MultilevelListDefinition::Custom(levels) => ctx
                    .document
                    .create_custom_multilevel_numbering(levels.clone()),
            }
            .map_err(CommandError::Custom)?;
            for paragraph_index in start..=end {
                let paragraph = ctx
                    .document
                    .paragraph_mut(paragraph_index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let properties = paragraph.properties.get_or_insert_with(Default::default);
                properties.numbering = Some(Numbering {
                    id: Some(num_id.clone()),
                    level: Some(self.level),
                });
                sync_paragraph_numbering_xml(
                    ctx.document,
                    paragraph_index,
                    Some(&num_id),
                    Some(self.level),
                );
            }
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.previous = Some(snapshot);
                self.previous_selection = previous_selection;
                Ok(())
            }
            Err(error) => {
                ctx.document.restore_snapshot(snapshot);
                *ctx.selection = previous_selection;
                Err(error)
            }
        }
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let snapshot = self.previous.take().ok_or_else(|| {
            CommandError::Custom("multilevel list snapshot is missing".to_string())
        })?;
        ctx.document.restore_snapshot(snapshot);
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

fn validate_custom_levels(levels: &[NumberingLevel], selected_level: u32) -> CommandResult<()> {
    if levels.is_empty() || levels.len() > 9 {
        return Err(CommandError::Custom(
            "custom multilevel list must contain between 1 and 9 levels".to_string(),
        ));
    }
    if selected_level as usize >= levels.len() {
        return Err(CommandError::Custom(
            "selected list level is not defined".to_string(),
        ));
    }
    for (index, level) in levels.iter().enumerate() {
        if level.ilvl != index as u32 {
            return Err(CommandError::Custom(
                "custom multilevel list levels must be consecutive".to_string(),
            ));
        }
        let num_fmt = level.num_fmt.as_deref().unwrap_or_default();
        if !matches!(
            num_fmt,
            "decimal"
                | "bullet"
                | "lowerLetter"
                | "upperLetter"
                | "lowerRoman"
                | "upperRoman"
                | "chineseCountingThousand"
        ) {
            return Err(CommandError::Custom(format!(
                "unsupported custom numbering format: {num_fmt}"
            )));
        }
        let lvl_text = level.lvl_text.as_deref().unwrap_or_default();
        if lvl_text.is_empty()
            || lvl_text.chars().count() > 256
            || lvl_text.chars().any(char::is_control)
        {
            return Err(CommandError::Custom(
                "custom numbering text must contain 1 to 256 printable characters".to_string(),
            ));
        }
        if level.start == Some(0) || level.start.unwrap_or(1) > 32_767 {
            return Err(CommandError::Custom(
                "custom numbering start must be between 1 and 32767".to_string(),
            ));
        }
        if let Some(indent) = level.indent.as_ref() {
            for value in [indent.left, indent.hanging].into_iter().flatten() {
                if !value.is_finite() || !(0.0..=1_584.0).contains(&value) {
                    return Err(CommandError::Custom(
                        "custom numbering indentation must be between 0 and 31680 twips"
                            .to_string(),
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use proptest::prelude::*;
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    fn fixture(with_existing_numbering: bool) -> Vec<u8> {
        let numbering_override = if with_existing_numbering {
            r#"<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>"#
        } else {
            ""
        };
        let content_types = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>{numbering_override}</Types>"#
        );
        let numbering_relationship = if with_existing_numbering {
            r#"<Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#
        } else {
            ""
        };
        let document_relationships = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{numbering_relationship}</Relationships>"#
        );
        let document = r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>A</w:t></w:r></w:p><w:p><w:r><w:t>B</w:t></w:r></w:p><w:p><w:r><w:t>C</w:t></w:r></w:p><w:p><w:r><w:t>D</w:t></w:r></w:p></w:body></w:document>"#;
        let package_relationships = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
        let existing_numbering = r#"<?xml version="1.0" encoding="UTF-8"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w:abstractNum w:abstractNumId="42" w15:restartNumberingAfterBreak="1"><w:nsid w:val="CAFEF00D"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="77"><w:abstractNumId w:val="42"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="4"/></w:lvlOverride></w:num></w:numbering>"#;

        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, value) in [
                ("[Content_Types].xml", content_types.as_str()),
                ("_rels/.rels", package_relationships),
                ("word/document.xml", document),
                (
                    "word/_rels/document.xml.rels",
                    document_relationships.as_str(),
                ),
            ] {
                zip.start_file(path, options).expect("start package entry");
                zip.write_all(value.as_bytes())
                    .expect("write package entry");
            }
            if with_existing_numbering {
                zip.start_file("word/numbering.xml", options)
                    .expect("start numbering");
                zip.write_all(existing_numbering.as_bytes())
                    .expect("write numbering");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    /// A package whose default paragraph style sets `w:ind w:left="0"`, the
    /// shape of many Japanese templates (本文 style with explicit zero indent).
    fn fixture_with_zero_indent_style() -> Vec<u8> {
        let content_types = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;
        let document_relationships = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
        let document = r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Body"/></w:pPr><w:r><w:t>A</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Body"/><w:ind w:left="720"/></w:pPr><w:r><w:t>B</w:t></w:r></w:p></w:body></w:document>"#;
        let styles = r#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Body"><w:name w:val="Body"/><w:pPr><w:ind w:left="0" w:right="0" w:firstLine="0"/></w:pPr></w:style></w:styles>"#;
        let package_relationships = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (path, value) in [
                ("[Content_Types].xml", content_types),
                ("_rels/.rels", package_relationships),
                ("word/document.xml", document),
                ("word/_rels/document.xml.rels", document_relationships),
                ("word/styles.xml", styles),
            ] {
                zip.start_file(path, options).expect("start package entry");
                zip.write_all(value.as_bytes())
                    .expect("write package entry");
            }
            zip.finish().expect("finish package");
        }
        cursor.into_inner()
    }

    #[test]
    fn a_numbered_paragraph_takes_the_level_indent_over_the_styles_zero_indent() {
        use crate::commands::set_numbering::SetNumberingCommand;

        let mut doc = executor::block_on(WordDocument::load(
            &fixture_with_zero_indent_style(),
            &Options::default(),
        ))
        .expect("load");
        let indent_of = |doc: &WordDocument, index: usize| {
            doc.paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.indent.clone())
        };
        // The style's zero indent is filled in at load.
        assert_eq!(indent_of(&doc, 0).and_then(|indent| indent.left), Some(0.0));

        let mut selection = None;
        let mut executor = CommandExecutor::new();
        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(ApplyMultilevelListCommand::new(
                        0,
                        1,
                        MultilevelListPreset::BulletHierarchy,
                        0,
                    )),
                    &mut ctx,
                )
                .expect("bullets");
        }
        // Word: the level's indent beats the style's, so the model drops the
        // style-filled left/first-line values and lets the level supply them.
        let indent = indent_of(&doc, 0).expect("indent");
        assert_eq!(indent.left, None, "style left is no longer direct");
        assert_eq!(indent.first_line, None);
        assert_eq!(indent.right, Some(0.0), "the style's right indent stays");
        // A paragraph with its own `w:ind` keeps it (direct beats numbering).
        assert_eq!(
            indent_of(&doc, 1).and_then(|indent| indent.left),
            Some(36.0)
        );

        {
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            executor
                .execute(
                    Box::new(SetNumberingCommand::new(0, 0, None, None)),
                    &mut ctx,
                )
                .expect("remove numbering");
        }
        // Without numbering the style's indent applies again.
        assert_eq!(indent_of(&doc, 0).and_then(|indent| indent.left), Some(0.0));

        // Reopening a numbered paragraph without direct `w:ind` lands in the
        // same state as the live edit.
        let reopened = executor::block_on(WordDocument::load(
            &doc.export_docx().expect("export"),
            &Options::default(),
        ))
        .expect("reopen");
        let reopened_indent = indent_of(&reopened, 1).expect("indent");
        assert_eq!(reopened_indent.left, Some(36.0));
        let numbered_again = {
            let mut doc = executor::block_on(WordDocument::load(
                &fixture_with_zero_indent_style(),
                &Options::default(),
            ))
            .expect("load");
            let mut selection = None;
            let mut ctx = CommandContext::new(&mut doc, &mut selection);
            ApplyMultilevelListCommand::new(0, 0, MultilevelListPreset::BulletHierarchy, 0)
                .apply(&mut ctx)
                .expect("bullets");
            let bytes = doc.export_docx().expect("export");
            executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("reopen")
        };
        assert_eq!(
            indent_of(&numbered_again, 0).and_then(|indent| indent.left),
            None,
            "loaded numbered paragraphs skip the style's left indent too"
        );
    }

    fn zip_text(bytes: &[u8], path: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open package");
        let mut entry = archive.by_name(path).expect("open package part");
        let mut value = String::new();
        entry.read_to_string(&mut value).expect("read package part");
        value
    }

    #[test]
    fn note_lists_roundtrip_in_their_own_part_and_undo_without_changing_body_xml() {
        for footnote in [true, false] {
            let mut document =
                executor::block_on(WordDocument::load(&fixture(false), &Options::default()))
                    .expect("load fixture");
            let note_id = if footnote {
                document.insert_footnote_at(0, 1, "hello")
            } else {
                document.insert_endnote_at(0, 1, "hello")
            }
            .expect("insert note");
            let paragraph = if footnote {
                document.footnote_paragraph_index(&note_id)
            } else {
                (0..document.paragraph_count())
                    .find(|&index| document.is_note_paragraph_surface(index))
            }
            .expect("note paragraph");
            let part = if footnote {
                "word/footnotes.xml"
            } else {
                "word/endnotes.xml"
            };
            let before = document.export_docx().expect("export before");
            let mut selection = Some(SelectionRange::collapsed(
                jade_docx_layout::selection::TextPosition::new(paragraph, 0),
            ));
            let mut commands = CommandExecutor::new();
            commands
                .execute(
                    Box::new(ApplyMultilevelListCommand::new(
                        paragraph,
                        paragraph,
                        MultilevelListPreset::BulletHierarchy,
                        0,
                    )),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .expect("apply list");
            let after = document.export_docx().expect("export after");
            assert!(zip_text(&after, part).contains("w:numPr"));
            assert_eq!(
                zip_text(&after, "word/document.xml"),
                zip_text(&before, "word/document.xml")
            );
            let reopened = executor::block_on(WordDocument::load(&after, &Options::default()))
                .expect("reopen");
            assert!(reopened
                .paragraph(paragraph)
                .unwrap()
                .properties
                .as_ref()
                .unwrap()
                .numbering
                .is_some());
            commands
                .undo(&mut CommandContext::new(&mut document, &mut selection))
                .expect("undo list");
            let undone = document.export_docx().expect("export undo");
            assert_eq!(zip_text(&undone, part), zip_text(&before, part));
        }
    }

    fn preset(value: u8) -> MultilevelListPreset {
        match value % 3 {
            0 => MultilevelListPreset::DecimalHierarchy,
            1 => MultilevelListPreset::ChineseOfficial,
            _ => MultilevelListPreset::BulletHierarchy,
        }
    }

    fn custom_num_fmt(value: u8) -> &'static str {
        match value % 7 {
            0 => "decimal",
            1 => "bullet",
            2 => "lowerLetter",
            3 => "upperLetter",
            4 => "lowerRoman",
            5 => "upperRoman",
            _ => "chineseCountingThousand",
        }
    }

    fn custom_levels(
        formats: &[u8],
        marker: &str,
        indent_twips: i32,
        hanging_twips: i32,
        start: u32,
    ) -> Vec<NumberingLevel> {
        formats
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let num_fmt = custom_num_fmt(*value);
                let lvl_text = if num_fmt == "bullet" {
                    format!("•{marker}")
                } else {
                    format!("%{}.{marker}", index + 1)
                };
                NumberingLevel {
                    ilvl: index as u32,
                    start: Some(start),
                    restart: None,
                    num_fmt: Some(num_fmt.to_string()),
                    lvl_text: Some(lvl_text),
                    lvl_jc: Some("left".to_string()),
                    suffix: Some("tab".to_string()),
                    indent: Some(crate::numbering::NumberingIndent {
                        left: Some(indent_twips as f32 / 20.0),
                        hanging: Some(hanging_twips as f32 / 20.0),
                    }),
                    paragraph_style: None,
                    paragraph_props: None,
                    run_props: None,
                    language: None,
                    is_legal: false,
                }
            })
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn multilevel_lists_export_roundtrip_and_restore_the_whole_package(
            preset_index in 0u8..3,
            level in 0u32..9,
            first in 0usize..4,
            second in 0usize..4,
            with_existing_numbering in any::<bool>(),
        ) {
            let input = fixture(with_existing_numbering);
            let mut document = executor::block_on(WordDocument::load(&input, &Options::default()))
                .expect("load fixture");
            let before = document.export_docx().expect("export before");
            let mut selection = Some(SelectionRange::collapsed(Default::default()));
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(ApplyMultilevelListCommand::new(
                        first,
                        second,
                        preset(preset_index),
                        level,
                    )),
                    &mut context,
                ).expect("apply multilevel list");
            }
            let after = document.export_docx().expect("export after");
            let numbering_xml = zip_text(&after, "word/numbering.xml");
            // Word reads `w:abstractNumId` as an integer and refuses a part
            // where it is not.
            for value in numbering_xml.split("w:abstractNumId=\"").skip(1) {
                let id = value.split('"').next().unwrap_or_default();
                prop_assert!(id.parse::<u32>().is_ok(), "abstractNumId {id:?} is not an integer");
            }
            for value in numbering_xml.split("<w:abstractNumId w:val=\"").skip(1) {
                let id = value.split('"').next().unwrap_or_default();
                prop_assert!(id.parse::<u32>().is_ok(), "abstractNumId {id:?} is not an integer");
            }
            if with_existing_numbering {
                prop_assert!(numbering_xml.contains("restartNumberingAfterBreak"));
                prop_assert!(numbering_xml.contains("CAFEF00D"));
                prop_assert!(numbering_xml.contains("startOverride"));
            }
            let relationships = zip_text(&after, "word/_rels/document.xml.rels");
            prop_assert_eq!(relationships.matches("relationships/numbering").count(), 1);
            let content_types = zip_text(&after, "[Content_Types].xml");
            prop_assert_eq!(content_types.matches("/word/numbering.xml").count(), 1);

            let reopened = executor::block_on(WordDocument::load(&after, &Options::default()))
                .expect("reopen exported package");
            let start = first.min(second);
            let end = first.max(second);
            for paragraph_index in start..=end {
                let numbering = reopened
                    .paragraph(paragraph_index)
                    .and_then(|paragraph| paragraph.properties.as_ref())
                    .and_then(|properties| properties.numbering.as_ref())
                    .expect("roundtripped numbering");
                prop_assert_eq!(numbering.level, Some(level));
                let num_id = numbering.id.as_deref().expect("roundtripped num id");
                let definition = reopened
                    .numbering_part
                    .as_ref()
                    .and_then(|part| part.get_level_info(num_id, level))
                    .expect("roundtripped level definition");
                prop_assert!(definition.lvl_text.is_some());
            }

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.undo(&mut context).expect("undo");
            }
            prop_assert_eq!(document.export_docx().expect("export undo"), before);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.redo(&mut context).expect("redo");
            }
            prop_assert_eq!(document.export_docx().expect("export redo"), after);
        }

        #[test]
        fn custom_multilevel_lists_roundtrip_exact_levels_and_restore_the_package(
            formats in proptest::collection::vec(0u8..7, 1..10),
            marker in "[A-Za-z0-9<>&\"]{0,8}",
            indent_twips in 0i32..=31_680,
            hanging_twips in 0i32..=31_680,
            start_value in 1u32..=32_767,
            selected_seed in any::<u8>(),
            first in 0usize..4,
            second in 0usize..4,
            with_existing_numbering in any::<bool>(),
        ) {
            let selected_level = u32::from(selected_seed) % formats.len() as u32;
            let levels = custom_levels(
                &formats,
                &marker,
                indent_twips,
                hanging_twips,
                start_value,
            );
            let input = fixture(with_existing_numbering);
            let mut document = executor::block_on(WordDocument::load(&input, &Options::default()))
                .expect("load fixture");
            let before = document.export_docx().expect("export before");
            let mut selection = Some(SelectionRange::collapsed(Default::default()));
            let mut commands = CommandExecutor::new();
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.execute(
                    Box::new(ApplyMultilevelListCommand::new_custom(
                        first,
                        second,
                        levels.clone(),
                        selected_level,
                    )),
                    &mut context,
                ).expect("apply custom multilevel list");
            }
            let after = document.export_docx().expect("export after");
            let numbering_xml = zip_text(&after, "word/numbering.xml");
            prop_assert_eq!(numbering_xml.matches("<w:abstractNum ").count(), if with_existing_numbering { 2 } else { 1 });
            prop_assert_eq!(numbering_xml.matches("<w:num ").count(), if with_existing_numbering { 2 } else { 1 });
            if marker.contains('<') {
                prop_assert!(numbering_xml.contains("&lt;"));
            }
            if marker.contains('&') {
                prop_assert!(numbering_xml.contains("&amp;"));
            }
            if marker.contains('"') {
                prop_assert!(numbering_xml.contains("&quot;"));
            }
            if with_existing_numbering {
                prop_assert!(numbering_xml.contains("restartNumberingAfterBreak"));
                prop_assert!(numbering_xml.contains("CAFEF00D"));
                prop_assert!(numbering_xml.contains("startOverride"));
            }
            let relationships = zip_text(&after, "word/_rels/document.xml.rels");
            prop_assert_eq!(relationships.matches("relationships/numbering").count(), 1);
            let content_types = zip_text(&after, "[Content_Types].xml");
            prop_assert_eq!(content_types.matches("/word/numbering.xml").count(), 1);

            let reopened = executor::block_on(WordDocument::load(&after, &Options::default()))
                .expect("reopen exported package");
            let first_target = first.min(second);
            let numbering = reopened
                .paragraph(first_target)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|properties| properties.numbering.as_ref())
                .expect("roundtripped numbering");
            prop_assert_eq!(numbering.level, Some(selected_level));
            let num_id = numbering.id.as_deref().expect("roundtripped num id");
            for (index, expected) in levels.iter().enumerate() {
                let actual = reopened
                    .numbering_part
                    .as_ref()
                    .and_then(|part| part.get_level_info(num_id, index as u32))
                    .expect("roundtripped custom level");
                prop_assert_eq!(actual.num_fmt.as_deref(), expected.num_fmt.as_deref());
                prop_assert_eq!(actual.lvl_text.as_deref(), expected.lvl_text.as_deref());
                prop_assert_eq!(actual.start, expected.start);
                let actual_indent = actual.indent.as_ref().expect("roundtripped indent");
                let expected_indent = expected.indent.as_ref().expect("expected indent");
                prop_assert!((actual_indent.left.unwrap_or_default() - expected_indent.left.unwrap_or_default()).abs() < 0.051);
                prop_assert!((actual_indent.hanging.unwrap_or_default() - expected_indent.hanging.unwrap_or_default()).abs() < 0.051);
            }

            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.undo(&mut context).expect("undo");
            }
            prop_assert_eq!(document.export_docx().expect("export undo"), before);
            {
                let mut context = CommandContext::new(&mut document, &mut selection);
                commands.redo(&mut context).expect("redo");
            }
            prop_assert_eq!(document.export_docx().expect("export redo"), after);
        }

        #[test]
        fn invalid_custom_multilevel_definitions_are_atomic(
            invalid_kind in 0u8..9,
            seed in any::<u16>(),
        ) {
            let mut levels = custom_levels(&[0, 1], "valid", 720, 360, 1);
            let mut selected_level = 0;
            match invalid_kind {
                0 => levels.clear(),
                1 => levels = custom_levels(&[0; 10], "valid", 720, 360, 1),
                2 => selected_level = 2,
                3 => levels[0].ilvl = 1,
                4 => levels[0].num_fmt = Some(format!("unsupported{seed}")),
                5 => levels[0].lvl_text = Some(String::new()),
                6 => levels[0].lvl_text = Some("bad\ntext".to_string()),
                7 => levels[0].start = Some(if seed % 2 == 0 { 0 } else { 32_768 }),
                _ => {
                    levels[0].indent = Some(crate::numbering::NumberingIndent {
                        left: Some(if seed % 2 == 0 { -0.05 } else { 1_584.05 }),
                        hanging: Some(18.0),
                    });
                }
            }

            let input = fixture(seed % 2 == 0);
            let mut document = executor::block_on(WordDocument::load(&input, &Options::default()))
                .expect("load fixture");
            let before = document.export_docx().expect("export before");
            let mut selection = Some(SelectionRange::collapsed(Default::default()));
            let selection_before = selection;
            let mut command = ApplyMultilevelListCommand::new_custom(
                0,
                1,
                levels,
                selected_level,
            );
            command
                .apply(&mut CommandContext::new(&mut document, &mut selection))
                .expect_err("invalid custom list must fail");
            prop_assert_eq!(document.export_docx().expect("export after"), before);
            prop_assert_eq!(selection, selection_before);
        }
    }

    #[test]
    fn invalid_level_is_atomic() {
        let input = fixture(false);
        let mut document =
            executor::block_on(WordDocument::load(&input, &Options::default())).expect("load");
        let before = document.export_docx().expect("export before");
        let mut selection = None;
        let mut command =
            ApplyMultilevelListCommand::new(0, 0, MultilevelListPreset::DecimalHierarchy, 9);
        let error = command
            .apply(&mut CommandContext::new(&mut document, &mut selection))
            .expect_err("invalid level must fail");
        assert!(error.to_string().contains("between 0 and 8"));
        assert_eq!(document.export_docx().expect("export after"), before);
    }

    #[test]
    fn repeated_custom_definitions_create_independent_numbering_instances() {
        let input = fixture(false);
        let mut document =
            executor::block_on(WordDocument::load(&input, &Options::default())).expect("load");
        let mut selection = None;
        let mut commands = CommandExecutor::new();
        let levels = custom_levels(&[0, 1, 2], "same", 720, 360, 1);
        for paragraph in [0, 1] {
            commands
                .execute(
                    Box::new(ApplyMultilevelListCommand::new_custom(
                        paragraph,
                        paragraph,
                        levels.clone(),
                        0,
                    )),
                    &mut CommandContext::new(&mut document, &mut selection),
                )
                .expect("apply custom definition");
        }

        let num_ids = [0, 1].map(|paragraph| {
            document
                .paragraph(paragraph)
                .and_then(|value| value.properties.as_ref())
                .and_then(|properties| properties.numbering.as_ref())
                .and_then(|numbering| numbering.id.clone())
                .expect("paragraph numbering id")
        });
        assert_ne!(num_ids[0], num_ids[1]);
        let numbering = document.numbering_part.as_ref().expect("numbering part");
        assert_ne!(
            numbering.abstract_num_id(&num_ids[0]),
            numbering.abstract_num_id(&num_ids[1])
        );
        let exported = document.export_docx().expect("export");
        let numbering_xml = zip_text(&exported, "word/numbering.xml");
        assert_eq!(numbering_xml.matches("<w:abstractNum ").count(), 2);
        assert_eq!(numbering_xml.matches("<w:num ").count(), 2);
    }
}
