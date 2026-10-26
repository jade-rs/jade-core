//! Word's Japanese typography: the 段落 dialog's 体裁 tab on paragraphs
//! ([`SetParagraphTypographyCommand`]) and the document's 文字体裁 options in
//! `settings.xml` ([`SetJapaneseTypographyCommand`]). Both change the model
//! the line breaker reads as well as the XML, so the pages re-break at once.

use jade_docx_headless_api::{HeadlessJapaneseTypography, HeadlessParagraphTypography};

use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml, sync_paragraph_value_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{EastAsianLineBreakingSettings, ParagraphProperties};
use crate::docx_xml::{DocxXmlDocument, XmlAttribute, XmlElement, XmlNode};
use jade_docx_layout::selection::SelectionRange;

/// The 体裁 tab's options on a range of paragraphs.
pub struct SetParagraphTypographyCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    typography: HeadlessParagraphTypography,
    previous: Vec<(usize, Option<ParagraphProperties>, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetParagraphTypographyCommand {
    pub fn new(
        start_paragraph: usize,
        end_paragraph: usize,
        typography: HeadlessParagraphTypography,
    ) -> Self {
        Self {
            start_paragraph: start_paragraph.min(end_paragraph),
            end_paragraph: start_paragraph.max(end_paragraph),
            typography,
            previous: Vec::new(),
            previous_selection: None,
        }
    }
}

/// Each option with its element and Word's default: a value equal to the
/// default is written by leaving the element out, as Word does.
fn options(typography: &HeadlessParagraphTypography) -> [(&'static str, Option<bool>, bool); 6] {
    [
        ("w:kinsoku", typography.kinsoku, true),
        ("w:wordWrap", typography.word_wrap, true),
        ("w:overflowPunct", typography.overflow_punctuation, true),
        ("w:topLinePunct", typography.top_line_punctuation, false),
        ("w:autoSpaceDE", typography.auto_space_de, true),
        ("w:autoSpaceDN", typography.auto_space_dn, true),
    ]
}

impl Command for SetParagraphTypographyCommand {
    fn tracks_formatting(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "set_paragraph_typography"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let count = ctx.document.paragraph_count();
        if count == 0 || self.end_paragraph >= count {
            return Err(CommandError::ParagraphUnavailable);
        }
        self.previous.clear();
        self.previous_selection = *ctx.selection;
        for index in self.start_paragraph..=self.end_paragraph {
            let xml = snapshot_paragraph_properties_xml(ctx.document, index);
            let paragraph = ctx
                .document
                .paragraph_mut(index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            self.previous
                .push((index, paragraph.properties.clone(), xml));
            let settings = paragraph
                .properties
                .get_or_insert_with(Default::default)
                .east_asian_settings
                .get_or_insert_with(EastAsianLineBreakingSettings::default);
            settings.use_east_asian_layout = true;
            let typography = &self.typography;
            if let Some(value) = typography.kinsoku {
                settings.kinsoku = Some(value);
            }
            if let Some(value) = typography.word_wrap {
                settings.word_wrap = Some(value);
            }
            if let Some(value) = typography.overflow_punctuation {
                settings.overflow_punctuation = Some(value);
            }
            if let Some(value) = typography.top_line_punctuation {
                settings.top_line_punctuation = Some(value);
            }
            if let Some(value) = typography.auto_space_de {
                settings.auto_space_de = Some(value);
            }
            if let Some(value) = typography.auto_space_dn {
                settings.auto_space_dn = Some(value);
            }
            for (tag, value, default) in options(&self.typography) {
                let Some(value) = value else { continue };
                let written = (value != default).then_some(if value { "1" } else { "0" });
                sync_paragraph_value_xml(ctx.document, index, tag, written);
            }
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (index, properties, xml) in std::mem::take(&mut self.previous) {
            let paragraph = ctx
                .document
                .paragraph_mut(index)
                .ok_or(CommandError::ParagraphUnavailable)?;
            paragraph.properties = properties;
            restore_paragraph_properties_xml(ctx.document, index, xml.as_ref());
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

/// The document's 文字体裁.
pub struct SetJapaneseTypographyCommand {
    typography: HeadlessJapaneseTypography,
    previous: Option<(String, EastAsianLineBreakingSettings)>,
}

impl SetJapaneseTypographyCommand {
    pub fn new(typography: HeadlessJapaneseTypography) -> Self {
        Self {
            typography,
            previous: None,
        }
    }
}

fn element(doc: &mut DocxXmlDocument, name: &str, attributes: &[(&str, &str)]) -> XmlNode {
    XmlNode::Element(
        doc.create_element(
            name.to_string(),
            attributes
                .iter()
                .map(|(name, value)| XmlAttribute {
                    name: name.to_string(),
                    value: value.to_string(),
                })
                .collect(),
            Vec::new(),
        ),
    )
}

/// Whether a `w:lang` names Japanese: `ja-JP`, or the localized name some
/// writers put there (`日本語`) instead of a language tag.
fn is_japanese_lang(lang: &str) -> bool {
    lang.to_ascii_lowercase().starts_with("ja") || !lang.is_ascii()
}

/// `settings.xml` with the typography applied.
fn rewrite_settings(xml: &str, typography: &HeadlessJapaneseTypography) -> Result<String, String> {
    let mut doc =
        DocxXmlDocument::parse(xml).map_err(|error| format!("settings.xml: {error:?}"))?;
    let mut remove: Vec<&str> = Vec::new();
    let mut add: Vec<XmlNode> = Vec::new();
    if let Some(control) = typography.character_spacing_control.as_deref() {
        remove.push("w:characterSpacingControl");
        add.push(element(
            &mut doc,
            "w:characterSpacingControl",
            &[("w:val", control)],
        ));
    }
    if let Some(level) = typography.kinsoku_level.as_deref() {
        remove.extend([
            "w:strictFirstAndLastChars",
            "w:noLineBreaksAfter",
            "w:noLineBreaksBefore",
        ]);
        match level {
            "strict" => add.push(element(&mut doc, "w:strictFirstAndLastChars", &[])),
            "custom" => {
                let after = typography.no_line_break_after.clone().unwrap_or_default();
                let before = typography.no_line_break_before.clone().unwrap_or_default();
                add.push(element(
                    &mut doc,
                    "w:noLineBreaksAfter",
                    &[("w:lang", "ja-JP"), ("w:val", &after)],
                ));
                add.push(element(
                    &mut doc,
                    "w:noLineBreaksBefore",
                    &[("w:lang", "ja-JP"), ("w:val", &before)],
                ));
            }
            _ => {}
        }
    }
    let root_id = doc.root().id;
    let root = doc
        .element_mut(root_id)
        .ok_or_else(|| "settings.xml has no root".to_string())?;
    root.children.retain(|child| {
        !matches!(child, XmlNode::Element(element) if remove.contains(&element.name.as_str())
            && (element.name.as_str() == "w:characterSpacingControl"
                || element.name.as_str() == "w:strictFirstAndLastChars"
                || element.attributes.iter().any(|attribute| attribute.name == "w:lang"
                    && is_japanese_lang(&attribute.value))))
    });
    root.children.extend(add);
    doc.rebuild_index();
    doc.to_string()
        .map_err(|error| format!("settings.xml: {error:?}"))
}

impl Command for SetJapaneseTypographyCommand {
    fn name(&self) -> &'static str {
        "set_japanese_typography"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let previous_xml = ctx
            .document
            .settings_xml_content()
            .ok_or_else(|| CommandError::Custom("settings.xml is not present".into()))?;
        let xml =
            rewrite_settings(&previous_xml, &self.typography).map_err(CommandError::Custom)?;
        let settings = jade_docx_reader::settings::DocumentSettings::from_xml(&xml)
            .map_err(|error| CommandError::Custom(error.to_string()))?;
        let document = ctx
            .document
            .document_mut()
            .ok_or(CommandError::ParagraphUnavailable)?;
        let previous_model = document.east_asian_line_breaking.clone();
        let mut model = settings.to_line_breaking_settings();
        model.compatibility_mode = previous_model.compatibility_mode;
        document.east_asian_line_breaking = model;
        ctx.document.restore_settings_xml(xml);
        self.previous = Some((previous_xml, previous_model));
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        let (xml, model) = self
            .previous
            .take()
            .ok_or_else(|| CommandError::Custom("missing settings backup for undo".into()))?;
        if let Some(document) = ctx.document.document_mut() {
            document.east_asian_line_breaking = model;
        }
        ctx.document.restore_settings_xml(xml);
        Ok(())
    }
}
