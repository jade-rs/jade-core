//! Word's paragraph options that have no command of their own: the tab stops
//! of タブとリーダー (and the ruler), the border box and shading of 線種と
//! ページ罫線と網かけの設定, and three 段落 switches — 同じスタイルの場合は段落間
//! にスペースを追加しない (`w:contextualSpacing`), 行番号を表示しない
//! (`w:suppressLineNumbers`) and 段落の自動ハイフネーションを行わない
//! (`w:suppressAutoHyphens`).
//!
//! Each is written as Word writes direct formatting: a switch the paragraph's
//! style already has is not repeated, and one the style turns on is turned off
//! with `w:val="0"`; tab stops are what the paragraph adds to its style's, and
//! a style tab stop it no longer has is a `w:tab w:val="clear"`.

use super::paragraph_property_sync::{
    restore_paragraph_properties_xml, snapshot_paragraph_properties_xml,
    sync_paragraph_borders_xml, sync_paragraph_flag_xml, sync_paragraph_shading_xml,
    sync_paragraph_tabs_xml,
};
use super::{Command, CommandContext, CommandError, CommandResult};
use crate::document::{BorderEdge, ParagraphBorders, ParagraphProperties, Shading, TabStop};
use crate::docx_xml::XmlElement;
use jade_docx_layout::selection::SelectionRange;

/// What a [`SetParagraphOptionsCommand`] changes; `None` leaves a property as
/// it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphOptions {
    /// The effective tab stops, positions in points from the start edge.
    pub tabs: Option<Vec<TabStop>>,
    pub contextual_spacing: Option<bool>,
    pub suppress_line_numbers: Option<bool>,
    pub suppress_auto_hyphens: Option<bool>,
    /// The whole border box; an empty one takes the borders off.
    pub borders: Option<ParagraphBorders>,
    pub shading: Option<Option<Shading>>,
}

impl ParagraphOptions {
    fn is_empty(&self) -> bool {
        self == &ParagraphOptions::default()
    }
}

/// Word's `ST_TabJc` values a tab stop can be set to (`clear` and the
/// deprecated `num` are not offered).
const TAB_ALIGNMENTS: [&str; 7] = ["left", "center", "right", "decimal", "bar", "start", "end"];
/// Word's `ST_TabTlc` leaders.
const TAB_LEADERS: [&str; 6] = ["none", "dot", "hyphen", "underscore", "heavy", "middleDot"];
/// `ST_Border` line styles Word's dialog offers.
pub const BORDER_STYLES: [&str; 24] = [
    "single",
    "dotted",
    "dashSmallGap",
    "dashed",
    "dotDash",
    "dotDotDash",
    "double",
    "triple",
    "thinThickSmallGap",
    "thickThinSmallGap",
    "thinThickThinSmallGap",
    "thinThickMediumGap",
    "thickThinMediumGap",
    "thinThickThinMediumGap",
    "thinThickLargeGap",
    "thickThinLargeGap",
    "thinThickThinLargeGap",
    "wave",
    "doubleWave",
    "dashDotStroked",
    "threeDEmboss",
    "threeDEngrave",
    "outset",
    "inset",
];
/// `ST_Shd` patterns Word's 網かけ offers.
pub const SHADING_PATTERNS: [&str; 25] = [
    "clear", "solid", "pct5", "pct10", "pct12", "pct15", "pct20", "pct25", "pct30", "pct35",
    "pct37", "pct40", "pct45", "pct50", "pct55", "pct60", "pct62", "pct65", "pct70", "pct75",
    "pct80", "pct85", "pct87", "pct90", "pct95",
];
const SHADING_LINES: [&str; 12] = [
    "horzStripe",
    "vertStripe",
    "reverseDiagStripe",
    "diagStripe",
    "horzCross",
    "diagCross",
    "thinHorzStripe",
    "thinVertStripe",
    "thinReverseDiagStripe",
    "thinDiagStripe",
    "thinHorzCross",
    "thinDiagCross",
];

/// A canonical token from `allowed`, matched without regard to case.
fn token(value: &str, allowed: &[&str], what: &str) -> CommandResult<String> {
    let value = value.trim();
    allowed
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(value))
        .map(|candidate| candidate.to_string())
        .ok_or_else(|| CommandError::Custom(format!("unknown {what}: {value}")))
}

/// `#RRGGBB` upper case, from `RRGGBB` or `#rrggbb`.
pub(crate) fn hex_color(value: &str, what: &str) -> CommandResult<String> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CommandError::Custom(format!(
            "{what} must be #RRGGBB, not {value}"
        )));
    }
    Ok(format!("#{}", hex.to_ascii_uppercase()))
}

/// A tab stop from the dialog's terms, checked.
pub fn tab_stop(
    position: f32,
    alignment: Option<&str>,
    leader: Option<&str>,
) -> CommandResult<TabStop> {
    if !position.is_finite() || !(-1584.0..=1584.0).contains(&position) {
        return Err(CommandError::Custom(
            "tab stop position must be within 22 inches of the margin".into(),
        ));
    }
    let alignment = token(
        alignment.unwrap_or("left"),
        &TAB_ALIGNMENTS,
        "tab alignment",
    )?;
    let leader = match leader.map(str::trim) {
        None | Some("") => None,
        Some(leader) => {
            Some(token(leader, &TAB_LEADERS, "tab leader")?).filter(|leader| leader != "none")
        }
    };
    Ok(TabStop {
        kind: Some(alignment),
        // Twips are what the file keeps; a position in points is rounded to
        // them so a reload reads back the same stop.
        position: Some((position * 20.0).round() / 20.0),
        leader,
    })
}

/// A border line from the dialog's terms, checked.
pub fn border_line(
    style: &str,
    width: f32,
    color: Option<&str>,
    space: Option<f32>,
) -> CommandResult<BorderEdge> {
    let style = token(style, &BORDER_STYLES, "border style")?;
    if !width.is_finite() || !(0.25..=6.0).contains(&width) {
        return Err(CommandError::Custom(
            "border width must be between 0.25 and 6 points".into(),
        ));
    }
    let space = space.unwrap_or(1.0);
    if !space.is_finite() || !(0.0..=31.0).contains(&space) {
        return Err(CommandError::Custom(
            "border distance must be between 0 and 31 points".into(),
        ));
    }
    let color = match color.map(str::trim) {
        None | Some("") => None,
        Some(color) if color.eq_ignore_ascii_case("auto") => None,
        Some(color) => Some(hex_color(color, "border color")?),
    };
    Ok(BorderEdge {
        val: Some(style),
        size_eighths: Some((width * 8.0).round().clamp(2.0, 96.0) as u32),
        space: Some(space.round() as u32),
        color,
    })
}

/// Shading from the dialog's terms, checked.
pub fn shading(
    pattern: Option<&str>,
    color: Option<&str>,
    fill: Option<&str>,
) -> CommandResult<Shading> {
    let patterns: Vec<&str> = SHADING_PATTERNS
        .iter()
        .chain(SHADING_LINES.iter())
        .copied()
        .collect();
    let pattern = token(pattern.unwrap_or("clear"), &patterns, "shading pattern")?;
    let colour = |value: Option<&str>, what: &str| -> CommandResult<Option<String>> {
        match value.map(str::trim) {
            None | Some("") => Ok(None),
            Some(value) if value.eq_ignore_ascii_case("auto") => Ok(None),
            Some(value) => hex_color(value, what).map(Some),
        }
    };
    Ok(Shading {
        val: Some(pattern),
        color: Some(colour(color, "shading color")?.unwrap_or_else(|| "auto".to_string())),
        fill: Some(colour(fill, "shading fill")?.unwrap_or_else(|| "auto".to_string())),
    })
}

/// The effective tab stops, positions rounded to twips and sorted, with no
/// two at one position.
fn normalized_tabs(tabs: &[TabStop]) -> Vec<TabStop> {
    let mut tabs: Vec<TabStop> = tabs
        .iter()
        .filter(|tab| tab.kind.as_deref() != Some("clear"))
        .filter(|tab| tab.position.is_some())
        .cloned()
        .collect();
    tabs.sort_by(|a, b| {
        a.position
            .unwrap_or(0.0)
            .partial_cmp(&b.position.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out: Vec<TabStop> = Vec::with_capacity(tabs.len());
    for tab in tabs {
        match out.last_mut() {
            Some(last) if same_position(last, &tab) => *last = tab,
            _ => out.push(tab),
        }
    }
    out
}

fn same_position(a: &TabStop, b: &TabStop) -> bool {
    (a.position.unwrap_or(0.0) - b.position.unwrap_or(0.0)).abs() < 0.025
}

/// The style's tab stops a paragraph sees: what it defines, less what it
/// clears.
pub(crate) fn effective_tabs(tabs: &[TabStop]) -> Vec<TabStop> {
    let cleared: Vec<&TabStop> = tabs
        .iter()
        .filter(|tab| tab.kind.as_deref() == Some("clear"))
        .collect();
    normalized_tabs(tabs)
        .into_iter()
        .filter(|tab| !cleared.iter().any(|clear| same_position(clear, tab)))
        .collect()
}

/// What the paragraph's own `w:tabs` holds for `wanted` over the style's
/// `style`: the stops the style lacks (or has otherwise), and a clear for
/// each style stop that is not wanted.
pub(crate) fn direct_tabs(style: &[TabStop], wanted: &[TabStop]) -> Vec<TabStop> {
    let style = effective_tabs(style);
    let wanted = normalized_tabs(wanted);
    let mut direct: Vec<TabStop> = style
        .iter()
        .filter(|tab| !wanted.iter().any(|wanted| wanted == *tab))
        .filter(|tab| !wanted.iter().any(|wanted| same_position(wanted, tab)))
        .map(|tab| TabStop {
            kind: Some("clear".to_string()),
            position: tab.position,
            leader: None,
        })
        .collect();
    direct.extend(
        wanted
            .iter()
            .filter(|tab| !style.iter().any(|style| style == *tab))
            .cloned(),
    );
    direct.sort_by(|a, b| {
        a.position
            .unwrap_or(0.0)
            .partial_cmp(&b.position.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    direct
}

/// Sets [`ParagraphOptions`] on a contiguous paragraph range.
pub struct SetParagraphOptionsCommand {
    start_paragraph: usize,
    end_paragraph: usize,
    options: ParagraphOptions,
    previous: Vec<(usize, Option<ParagraphProperties>, Option<XmlElement>)>,
    previous_selection: Option<SelectionRange>,
}

impl SetParagraphOptionsCommand {
    pub fn new(start_paragraph: usize, end_paragraph: usize, options: ParagraphOptions) -> Self {
        Self {
            start_paragraph,
            end_paragraph,
            options,
            previous: Vec::new(),
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

impl Command for SetParagraphOptionsCommand {
    fn name(&self) -> &'static str {
        "set_paragraph_options"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        if self.options.is_empty() {
            return Err(CommandError::Custom(
                "set_paragraph_options needs at least one option".into(),
            ));
        }
        let (start, end) = self.normalized_range();
        let paragraph_count = ctx.document.paragraph_count();
        if paragraph_count == 0 || start >= paragraph_count || end >= paragraph_count {
            return Err(CommandError::ParagraphUnavailable);
        }
        self.previous.clear();
        self.previous_selection = *ctx.selection;
        for index in start..=end {
            let previous_xml = snapshot_paragraph_properties_xml(ctx.document, index);
            let previous = ctx
                .document
                .paragraph(index)
                .map(|paragraph| paragraph.properties.clone())
                .ok_or(CommandError::ParagraphUnavailable)?;
            let style_id = previous
                .as_ref()
                .and_then(|props| props.style.as_deref())
                .map(str::to_string);
            let style = ctx
                .document
                .inherited_paragraph_properties(index, style_id.as_deref())
                .unwrap_or_default();
            self.previous.push((index, previous, previous_xml));

            let options = self.options.clone();
            let direct = options
                .tabs
                .as_ref()
                .map(|wanted| direct_tabs(&style.tabs, wanted));
            {
                let paragraph = ctx
                    .document
                    .paragraph_mut(index)
                    .ok_or(CommandError::ParagraphUnavailable)?;
                let props = paragraph.properties.get_or_insert_with(Default::default);
                if let Some(direct) = direct.as_ref() {
                    // A load reads the style's stops and then the
                    // paragraph's own, clears included.
                    let mut tabs = style.tabs.clone();
                    tabs.extend(direct.iter().cloned());
                    props.tabs = if direct.is_empty() {
                        style.tabs.clone()
                    } else {
                        tabs
                    };
                }
                if let Some(value) = options.contextual_spacing {
                    props.contextual_spacing = Some(value);
                }
                if let Some(value) = options.suppress_line_numbers {
                    props.suppress_line_numbers = Some(value);
                }
                if let Some(value) = options.suppress_auto_hyphens {
                    props.suppress_auto_hyphens = Some(value);
                }
                if let Some(borders) = options.borders.as_ref() {
                    props.borders = (!borders.is_empty()).then(|| borders.clone());
                }
                if let Some(shading) = options.shading.as_ref() {
                    props.shading = shading.clone();
                }
            }
            if let Some(direct) = direct.as_ref() {
                sync_paragraph_tabs_xml(ctx.document, index, direct);
            }
            for (tag, value, inherited) in [
                (
                    "w:contextualSpacing",
                    options.contextual_spacing,
                    style.contextual_spacing,
                ),
                (
                    "w:suppressLineNumbers",
                    options.suppress_line_numbers,
                    style.suppress_line_numbers,
                ),
                (
                    "w:suppressAutoHyphens",
                    options.suppress_auto_hyphens,
                    style.suppress_auto_hyphens,
                ),
            ] {
                if let Some(value) = value {
                    let declared = (value != inherited.unwrap_or(false)).then_some(value);
                    sync_paragraph_flag_xml(ctx.document, index, tag, declared);
                }
            }
            if let Some(borders) = options.borders.as_ref() {
                sync_paragraph_borders_xml(
                    ctx.document,
                    index,
                    (!borders.is_empty()).then_some(borders),
                );
            }
            if let Some(shading) = options.shading.as_ref() {
                sync_paragraph_shading_xml(ctx.document, index, shading.as_ref());
            }
        }
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for (index, previous, previous_xml) in &self.previous {
            ctx.document
                .paragraph_mut(*index)
                .ok_or(CommandError::ParagraphUnavailable)?
                .properties = previous.clone();
            restore_paragraph_properties_xml(ctx.document, *index, previous_xml.as_ref());
        }
        *ctx.selection = self.previous_selection;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless_api::apply_headless_commands_to_document;
    use crate::word_document::WordDocument;
    use crate::Options;
    use futures::executor;
    use jade_docx_headless_api::{
        HeadlessCommand, HeadlessEditRequest, HeadlessParagraphBorderBox,
        HeadlessParagraphBorderLine, HeadlessParagraphShading, HeadlessTabStop,
    };
    use std::io::{Read, Write};
    use zip::write::SimpleFileOptions;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    /// A package whose `List` style has a left tab at 1 inch and contextual
    /// spacing, and two paragraphs in that style.
    fn styled_docx() -> Vec<u8> {
        let document = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body><w:p><w:pPr><w:pStyle w:val="List"/></w:pPr><w:r><w:t>one	two</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="List"/></w:pPr><w:r><w:t>three</w:t></w:r></w:p></w:body></w:document>"#
        );
        let styles = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="List"><w:name w:val="List"/><w:basedOn w:val="Normal"/><w:pPr><w:tabs><w:tab w:val="left" w:pos="1440"/></w:tabs><w:contextualSpacing/></w:pPr></w:style></w:styles>"#
        );
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = SimpleFileOptions::default();
            for (path, value) in [
                (
                    "[Content_Types].xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
                ),
                (
                    "word/_rels/document.xml.rels",
                    r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
                ),
                ("word/document.xml", document.as_str()),
                ("word/styles.xml", styles.as_str()),
            ] {
                zip.start_file(path, options).unwrap();
                zip.write_all(value.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn document_xml(bytes: &[u8]) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut value = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut value)
            .unwrap();
        value
    }

    fn props(document: &WordDocument, index: usize) -> ParagraphProperties {
        document
            .paragraph(index)
            .and_then(|paragraph| paragraph.properties.clone())
            .unwrap_or_default()
    }

    fn effective(document: &WordDocument, index: usize) -> Vec<(f32, String, Option<String>)> {
        effective_tabs(&props(document, index).tabs)
            .into_iter()
            .map(|tab| (tab.position.unwrap(), tab.kind.unwrap(), tab.leader))
            .collect()
    }

    #[test]
    fn tabs_and_switches_round_trip_over_the_style() {
        let source = styled_docx();
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        assert_eq!(effective(&document, 0), vec![(72.0, "left".into(), None)]);
        let baseline = document.export_docx().unwrap();
        let line = |style: &str| HeadlessParagraphBorderLine {
            style: style.to_string(),
            width: 1.5,
            color: Some("#C00000".into()),
            space: Some(4.0),
        };
        apply_headless_commands_to_document(
            &mut document,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetParagraphOptions {
                    start_paragraph: 0,
                    end_paragraph: 0,
                    tabs: Some(vec![
                        HeadlessTabStop {
                            position: 108.0,
                            alignment: Some("right".into()),
                            leader: Some("dot".into()),
                        },
                        HeadlessTabStop {
                            position: 36.0,
                            alignment: Some("center".into()),
                            leader: None,
                        },
                    ]),
                    contextual_spacing: Some(false),
                    suppress_line_numbers: Some(true),
                    suppress_auto_hyphens: Some(true),
                    borders: Some(HeadlessParagraphBorderBox {
                        top: Some(line("double")),
                        bottom: Some(line("dashSmallGap")),
                        ..Default::default()
                    }),
                    shading: Some(Some(HeadlessParagraphShading {
                        pattern: Some("pct20".into()),
                        color: None,
                        fill: Some("FFF2CC".into()),
                    })),
                }],
            },
        )
        .unwrap();
        let expected = vec![
            (36.0, "center".to_string(), None),
            (108.0, "right".to_string(), Some("dot".to_string())),
        ];
        assert_eq!(effective(&document, 0), expected);
        let applied = document.export_docx().unwrap();
        let xml = document_xml(&applied);
        assert!(
            xml.contains(r#"<w:tab w:val="clear" w:pos="1440"/>"#),
            "{xml}"
        );
        assert!(xml.contains(r#"<w:contextualSpacing w:val="0"/>"#), "{xml}");
        assert!(xml.contains("<w:suppressLineNumbers/>"), "{xml}");
        assert!(xml.contains("<w:suppressAutoHyphens/>"), "{xml}");
        assert!(xml.contains(r#"w:val="dashSmallGap""#), "{xml}");
        assert!(xml.contains(r#"w:val="pct20""#), "{xml}");

        let reopened =
            executor::block_on(WordDocument::load(&applied, &Options::default())).unwrap();
        assert_eq!(effective(&reopened, 0), expected);
        let first = props(&reopened, 0);
        assert_eq!(first.contextual_spacing, Some(false));
        assert_eq!(first.suppress_line_numbers, Some(true));
        assert_eq!(first.suppress_auto_hyphens, Some(true));
        let borders = first.borders.unwrap();
        assert_eq!(borders.top.unwrap().val.as_deref(), Some("double"));
        assert_eq!(borders.bottom.as_ref().unwrap().size_eighths, Some(12));
        assert_eq!(borders.bottom.unwrap().space, Some(4));
        assert_eq!(first.shading.unwrap().val.as_deref(), Some("pct20"));
        // The second paragraph keeps the style's.
        assert_eq!(effective(&reopened, 1), vec![(72.0, "left".into(), None)]);
        assert_eq!(props(&reopened, 1).contextual_spacing, Some(true));

        // Back to the style's own stops: nothing of the paragraph's is left.
        apply_headless_commands_to_document(
            &mut document,
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::SetParagraphOptions {
                    start_paragraph: 0,
                    end_paragraph: 0,
                    tabs: Some(vec![HeadlessTabStop {
                        position: 72.0,
                        alignment: None,
                        leader: None,
                    }]),
                    contextual_spacing: Some(true),
                    suppress_line_numbers: None,
                    suppress_auto_hyphens: None,
                    borders: Some(HeadlessParagraphBorderBox::default()),
                    shading: Some(None),
                }],
            },
        )
        .unwrap();
        let xml = document_xml(&document.export_docx().unwrap());
        assert!(!xml.contains("<w:tabs>"), "{xml}");
        assert!(!xml.contains("contextualSpacing"), "{xml}");
        assert!(!xml.contains("<w:pBdr>"), "{xml}");
        assert!(!xml.contains("<w:shd"), "{xml}");
        assert_ne!(document.export_docx().unwrap(), baseline);
    }

    #[test]
    fn undo_puts_the_paragraph_back_exactly() {
        let source = styled_docx();
        let mut document =
            executor::block_on(WordDocument::load(&source, &Options::default())).unwrap();
        let baseline = document.export_docx().unwrap();
        let mut selection = None;
        let mut commands = crate::commands::CommandExecutor::new();
        let options = ParagraphOptions {
            tabs: Some(vec![tab_stop(200.0, Some("bar"), None).unwrap()]),
            suppress_line_numbers: Some(true),
            ..Default::default()
        };
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            commands
                .execute(
                    Box::new(SetParagraphOptionsCommand::new(0, 1, options)),
                    &mut ctx,
                )
                .unwrap();
        }
        let applied = document.export_docx().unwrap();
        assert_ne!(applied, baseline);
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(commands.undo(&mut ctx).unwrap());
        }
        assert_eq!(document.export_docx().unwrap(), baseline);
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(commands.redo(&mut ctx).unwrap());
        }
        assert_eq!(document.export_docx().unwrap(), applied);
    }

    fn tab(kind: &str, position: f32, leader: Option<&str>) -> TabStop {
        TabStop {
            kind: Some(kind.to_string()),
            position: Some(position),
            leader: leader.map(str::to_string),
        }
    }

    #[test]
    fn direct_tabs_add_what_the_style_lacks_and_clear_what_it_has() {
        let style = vec![tab("left", 36.0, None), tab("right", 144.0, Some("dot"))];
        let wanted = vec![tab("right", 144.0, Some("dot")), tab("center", 72.0, None)];
        let direct = direct_tabs(&style, &wanted);
        assert_eq!(
            direct,
            vec![tab("clear", 36.0, None), tab("center", 72.0, None)]
        );
        // What a load makes of the style's stops and these is what was wanted.
        let mut loaded = style.clone();
        loaded.extend(direct);
        assert_eq!(effective_tabs(&loaded), normalized_tabs(&wanted));
    }

    #[test]
    fn a_style_stop_changed_in_place_is_written_over() {
        let style = vec![tab("left", 36.0, None)];
        let wanted = vec![tab("decimal", 36.0, Some("hyphen"))];
        assert_eq!(direct_tabs(&style, &wanted), wanted);
    }

    #[test]
    fn dialog_values_are_checked() {
        assert!(tab_stop(36.0, Some("Decimal"), Some("dot")).is_ok());
        assert!(tab_stop(36.0, Some("sideways"), None).is_err());
        assert!(tab_stop(f32::NAN, None, None).is_err());
        assert_eq!(
            border_line("dashsmallgap", 1.5, Some("ff0000"), Some(4.0))
                .unwrap()
                .val
                .as_deref(),
            Some("dashSmallGap")
        );
        assert!(border_line("single", 40.0, None, None).is_err());
        assert_eq!(
            shading(Some("pct25"), None, Some("#ffff00")).unwrap(),
            Shading {
                val: Some("pct25".into()),
                color: Some("auto".into()),
                fill: Some("#FFFF00".into()),
            }
        );
    }
}
