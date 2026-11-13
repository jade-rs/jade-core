pub use jade_number_format::{canonical_number_format, format_number_value, CUSTOM_FORMAT_PREFIX};
pub(crate) use jade_number_format::{
    is_english_language, to_english_cardinal, to_english_ordinal, to_english_ordinal_text, to_hex,
    to_letter, to_roman,
};

#[cfg(test)]
mod counter_histories;

use crate::document::{FontHint, ParagraphProperties, RunFonts, RunProperties};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;

type ParseResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Parsed representation of `word/numbering.xml`. Stores abstract numbering
/// definitions and concrete numbering instances that paragraphs refer to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingPart {
    pub abstract_nums: HashMap<String, AbstractNum>,
    pub numbering_instances: HashMap<String, NumberingInstance>,
    /// Picture bullets (`w:numPicBullet`, ECMA-376 Part 1 §17.9.21) by
    /// `w:numPicBulletId`, which a level names with `w:lvlPicBulletId`.
    #[serde(default)]
    pub picture_bullets: HashMap<String, PictureBullet>,
}

/// A picture a list level shows instead of its bullet glyph.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PictureBullet {
    /// The image's relationship in `numbering.xml`'s relationships.
    pub relationship_id: Option<String>,
    /// The picture's authored box, in points (`v:shape` style or
    /// `wp:extent`), which gives its aspect.
    pub width_pt: Option<f32>,
    pub height_pt: Option<f32>,
    /// The image itself, filled in by the loader that has the package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_uri: Option<String>,
}

impl Default for NumberingPart {
    fn default() -> Self {
        Self::new()
    }
}

/// Describes a multiline numbering definition that may contain up to nine
/// levels, each with its own formatting and indentation rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbstractNum {
    pub id: String,
    pub levels: Vec<NumberingLevel>,
    /// The built-in gallery this definition was created from in this session,
    /// so applying the same gallery again continues the same list. Word keeps
    /// no such marker in `numbering.xml`, so a reopened document has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<MultilevelListPreset>,
}

/// Stores the configuration for a single numbering level, including the
/// textual template and any optional paragraph/run overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingLevel {
    pub ilvl: u32,
    pub start: Option<u32>,
    pub restart: Option<u32>,
    pub num_fmt: Option<String>,
    pub lvl_text: Option<String>,
    pub lvl_jc: Option<String>,
    pub suffix: Option<String>,
    pub indent: Option<NumberingIndent>,
    pub paragraph_style: Option<String>,
    pub paragraph_props: Option<ParagraphProperties>,
    pub run_props: Option<RunProperties>,
    /// Language of the numbering label from `w:lvl/w:rPr/w:lang/@w:val`.
    /// Language-dependent formats must not guess from the document body.
    pub language: Option<String>,
    /// `w:lvlPicBulletId`: the picture bullet the level shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_bullet: Option<String>,
    /// `w:isLgl` (ECMA-376 Part 1 §17.9.4): the level's text shows every
    /// level it names in decimal — Word's 正規形式の番号を使用する, which
    /// turns `第一章.1` into `1.1`.
    #[serde(default)]
    pub is_legal: bool,
}

/// Indentation offsets used when the numbering definition contributes its own
/// layout (common for bullet lists).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingIndent {
    pub left: Option<f32>,
    pub hanging: Option<f32>,
}

/// Associates a numbering instance with its referenced abstract definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingInstance {
    pub id: String,
    pub abstract_num_id: String,
    pub overrides: HashMap<u32, LevelOverride>,
}

/// Built-in nine-level list galleries shared by the editor integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MultilevelListPreset {
    DecimalHierarchy,
    ChineseOfficial,
    BulletHierarchy,
}

impl MultilevelListPreset {
    pub fn key(self) -> &'static str {
        match self {
            Self::DecimalHierarchy => "decimalHierarchy",
            Self::ChineseOfficial => "chineseOfficial",
            Self::BulletHierarchy => "bulletHierarchy",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LevelOverride {
    pub start: Option<u32>,
    /// A complete replacement for this instance's abstract numbering level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<NumberingLevel>,
}

impl NumberingPart {
    /// Creates an empty numbering part. Useful when documents do not contain
    /// any numbering and we need a neutral default.
    pub fn new() -> Self {
        Self {
            abstract_nums: HashMap::new(),
            numbering_instances: HashMap::new(),
            picture_bullets: HashMap::new(),
        }
    }

    /// Parses an XML string containing numbering definitions and returns the
    /// populated [`NumberingPart`].
    pub fn from_xml(xml: &str) -> ParseResult<Self> {
        let mut part = Self::new();
        part.parse(xml)?;
        Ok(part)
    }

    /// The `w:numPicBullet`s of a numbering part.
    fn parse_picture_bullets(&mut self, xml: &str) {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut current: Option<(String, PictureBullet)> = None;
        let points = |value: &str| -> Option<f32> {
            let value = value.trim();
            let (number, scale) = if let Some(number) = value.strip_suffix("pt") {
                (number, 1.0)
            } else if let Some(number) = value.strip_suffix("in") {
                (number, 72.0)
            } else if let Some(number) = value.strip_suffix("px") {
                (number, 0.75)
            } else {
                (value, 1.0)
            };
            number
                .trim()
                .parse::<f32>()
                .ok()
                .map(|number| number * scale)
        };
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"w:numPicBullet" => {
                        current = attr_value(e, b"w:numPicBulletId")
                            .map(|id| (id.into_owned(), PictureBullet::default()));
                    }
                    b"v:imagedata" => {
                        if let Some((_, bullet)) = current.as_mut() {
                            bullet.relationship_id = attr_value(e, b"r:id").map(Cow::into_owned);
                        }
                    }
                    b"a:blip" => {
                        if let Some((_, bullet)) = current.as_mut() {
                            bullet.relationship_id = attr_value(e, b"r:embed").map(Cow::into_owned);
                        }
                    }
                    b"v:shape" => {
                        if let (Some((_, bullet)), Some(style)) =
                            (current.as_mut(), attr_value(e, b"style"))
                        {
                            for declaration in style.split(';') {
                                if let Some((key, value)) = declaration.split_once(':') {
                                    match key.trim() {
                                        "width" => bullet.width_pt = points(value),
                                        "height" => bullet.height_pt = points(value),
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    b"wp:extent" => {
                        if let Some((_, bullet)) = current.as_mut() {
                            let emu = |key: &[u8]| {
                                attr_value(e, key)
                                    .and_then(|value| value.parse::<f32>().ok())
                                    .map(|value| value / 12_700.0)
                            };
                            bullet.width_pt = emu(b"cx");
                            bullet.height_pt = emu(b"cy");
                        }
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) if e.name().as_ref() == b"w:numPicBullet" => {
                    if let Some((id, bullet)) = current.take() {
                        self.picture_bullets.insert(id, bullet);
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    /// The picture a level shows for its bullet, if it shows one.
    pub fn level_picture_bullet(&self, num_id: &str, level: u32) -> Option<&PictureBullet> {
        let id = self
            .get_level_info(num_id, level)?
            .picture_bullet
            .as_deref()?;
        self.picture_bullets.get(id)
    }

    /// Parses numbering information and mutates the existing structure.
    pub fn parse(&mut self, xml: &str) -> ParseResult<()> {
        self.parse_picture_bullets(xml);
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut current_abstract: Option<AbstractNum> = None;
        let mut current_level: Option<NumberingLevel> = None;
        let mut current_num: Option<NumberingInstance> = None;
        let mut current_override_level: Option<u32> = None;
        let mut in_level_paragraph_props = false;
        let mut in_level_run_props = false;

        loop {
            // The input already lives in memory; borrow events directly from it
            // instead of copying each tag into a temporary buffer.
            match reader.read_event() {
                Ok(Event::Start(ref e)) => {
                    self.handle_start_event(
                        e,
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                        &mut in_level_paragraph_props,
                        &mut in_level_run_props,
                    )?;
                }
                Ok(Event::Empty(ref e)) => {
                    self.handle_start_event(
                        e,
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                        &mut in_level_paragraph_props,
                        &mut in_level_run_props,
                    )?;
                    self.handle_end_event(
                        e.name().as_ref(),
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                        &mut in_level_paragraph_props,
                        &mut in_level_run_props,
                    );
                }
                Ok(Event::End(ref e)) => {
                    self.handle_end_event(
                        e.name().as_ref(),
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                        &mut in_level_paragraph_props,
                        &mut in_level_run_props,
                    );
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(format!("Numbering XML parsing error: {}", e).into());
                }
                _ => {}
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_start_event(
        &mut self,
        e: &BytesStart,
        current_abstract: &mut Option<AbstractNum>,
        current_level: &mut Option<NumberingLevel>,
        current_num: &mut Option<NumberingInstance>,
        current_override_level: &mut Option<u32>,
        in_level_paragraph_props: &mut bool,
        in_level_run_props: &mut bool,
    ) -> ParseResult<()> {
        match e.name().as_ref() {
            b"w:abstractNum" => {
                if let Some(id) = attr_value(e, b"w:abstractNumId") {
                    *current_abstract = Some(AbstractNum {
                        id: id.into_owned(),
                        levels: Vec::new(),
                        preset: None,
                    });
                }
            }
            b"w:lvl" => {
                if let Some(ilvl) = attr_value(e, b"w:ilvl") {
                    let level = NumberingLevel {
                        ilvl: ilvl.parse().unwrap_or(0),
                        start: None,
                        restart: None,
                        num_fmt: None,
                        lvl_text: None,
                        lvl_jc: None,
                        suffix: None,
                        indent: None,
                        paragraph_style: None,
                        paragraph_props: None,
                        run_props: None,
                        language: None,
                        is_legal: false,
                        picture_bullet: None,
                    };
                    *current_level = Some(level);
                }
            }
            b"w:start" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.start = val.parse().ok();
                    }
                }
            }
            b"w:lvlRestart" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.restart = val.parse().ok();
                    }
                }
            }
            b"w:numFmt" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        let format = attr_value(e, b"w:format");
                        if val == "custom" {
                            if let Some(format) = format {
                                level.num_fmt = Some(format!("{CUSTOM_FORMAT_PREFIX}{format}"));
                            }
                        } else if !level
                            .num_fmt
                            .as_deref()
                            .is_some_and(|current| current.starts_with(CUSTOM_FORMAT_PREFIX))
                        {
                            // A custom format's mc:Fallback decimal does not
                            // replace the custom format read before it.
                            level.num_fmt = Some(val.into_owned());
                        }
                    }
                }
            }
            b"w:lvlText" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.lvl_text = Some(val.into_owned());
                    }
                }
            }
            b"w:lvlJc" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.lvl_jc = Some(val.into_owned());
                    }
                }
            }
            b"w:lvlPicBulletId" => {
                if let Some(level) = current_level.as_mut() {
                    level.picture_bullet = attr_value(e, b"w:val").map(Cow::into_owned);
                }
            }
            b"w:isLgl" => {
                if let Some(level) = current_level.as_mut() {
                    level.is_legal = attr_value(e, b"w:val")
                        .is_none_or(|value| !matches!(value.as_ref(), "0" | "false" | "off"));
                }
            }
            b"w:suff" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.suffix = Some(val.into_owned());
                    }
                }
            }
            b"w:pStyle" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.paragraph_style = Some(val.into_owned());
                    }
                }
            }
            b"w:pPr" => {
                if current_level.is_some() {
                    *in_level_paragraph_props = true;
                }
            }
            b"w:tab" if *in_level_paragraph_props => {
                if let (Some(level), Some(tab)) = (
                    current_level.as_mut(),
                    crate::styles::parse_tab_definition(e),
                ) {
                    level
                        .paragraph_props
                        .get_or_insert_with(ParagraphProperties::default)
                        .tabs
                        .push(tab);
                }
            }
            b"w:ind" => {
                if *in_level_paragraph_props {
                    if let Some(level) = current_level.as_mut() {
                        let mut indent = level.indent.clone().unwrap_or(NumberingIndent {
                            left: None,
                            hanging: None,
                        });

                        if let Some(val) = attr_value(e, b"w:left") {
                            if let Ok(parsed) = val.parse::<f32>() {
                                indent.left = Some(parsed / 20.0);
                            }
                        }

                        if let Some(val) = attr_value(e, b"w:hanging") {
                            if let Ok(parsed) = val.parse::<f32>() {
                                indent.hanging = Some(parsed / 20.0);
                            }
                        }

                        level.indent = Some(indent);
                    }
                }
            }
            b"w:rPr" => {
                if current_level.is_some() {
                    *in_level_run_props = true;
                }
            }
            b"w:b" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        props.bold = Some(crate::parser::DocumentParser::parse_on_off_attr(
                            e.attributes(),
                        ));
                    }
                }
            }
            b"w:i" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        props.italic = Some(crate::parser::DocumentParser::parse_on_off_attr(
                            e.attributes(),
                        ));
                    }
                }
            }
            b"w:bCs" | b"w:iCs" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    let enabled = crate::parser::DocumentParser::parse_on_off_attr(e.attributes());
                    let props = ensure_run_props(level);
                    if e.name().as_ref() == b"w:bCs" {
                        props.bold_complex = Some(enabled);
                    } else {
                        props.italic_complex = Some(enabled);
                    }
                }
            }
            b"w:bdr" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    ensure_run_props(level).border = Some(
                        crate::parser::DocumentParser::build_border_edge(e.attributes()),
                    );
                }
            }
            b"w:shd" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    ensure_run_props(level).shading =
                        Some(crate::parser::DocumentParser::parse_shading(e.attributes()));
                }
            }
            b"w:em" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    let value = attr_value(e, b"w:val").map(|value| value.into_owned());
                    ensure_run_props(level).emphasis =
                        crate::parser::normalize_emphasis_mark(value).map(Rc::from);
                }
            }
            b"w:shadow" | b"w:outline" | b"w:emboss" | b"w:imprint" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    let enabled = crate::parser::DocumentParser::parse_on_off_attr(e.attributes());
                    let props = ensure_run_props(level);
                    match e.name().as_ref() {
                        b"w:shadow" => props.shadow = Some(enabled),
                        b"w:outline" => props.outline = Some(enabled),
                        b"w:emboss" => props.emboss = Some(enabled),
                        b"w:imprint" => props.imprint = Some(enabled),
                        _ => unreachable!(),
                    }
                }
            }
            b"w:smallCaps" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    ensure_run_props(level).small_caps = Some(
                        crate::parser::DocumentParser::parse_on_off_attr(e.attributes()),
                    );
                }
            }
            b"w:caps" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    ensure_run_props(level).caps = Some(
                        crate::parser::DocumentParser::parse_on_off_attr(e.attributes()),
                    );
                }
            }
            b"w:strike" | b"w:dstrike" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    let enabled = crate::parser::DocumentParser::parse_on_off_attr(e.attributes());
                    let props = ensure_run_props(level);
                    if e.name().as_ref() == b"w:dstrike" {
                        props.double_strike = Some(enabled);
                    } else {
                        props.strike = Some(enabled);
                    }
                }
            }
            b"w:vertAlign" if *in_level_run_props => {
                if let (Some(level), Some(value)) =
                    (current_level.as_mut(), attr_value(e, b"w:val"))
                {
                    ensure_run_props(level).baseline = Some(Rc::from(value.as_ref()));
                }
            }
            b"w:u" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        let parsed = crate::parser::parse_underline(e.attributes());
                        props.underline = parsed.on_off;
                        props.underline_kind = parsed.kind.map(Rc::from);
                        props.underline_color = parsed.color.map(Rc::from);
                        props.underline_theme_color = parsed.theme.map(Box::new);
                    }
                }
            }
            b"w:color" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        props.color = crate::parser::DocumentParser::run_color(e).map(Rc::from);
                        props.theme_color =
                            crate::parser::DocumentParser::run_theme_color(e.attributes())
                                .map(Box::new);
                    }
                }
            }
            b"w:sz" | b"w:szCs" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        if let Some(val) = attr_value(e, b"w:val") {
                            if let Ok(parsed) = val.parse::<f32>() {
                                let props = ensure_run_props(level);
                                if e.name().as_ref() == b"w:szCs" {
                                    props.font_size_complex = Some(parsed / 2.0);
                                } else {
                                    props.font_size = Some(parsed / 2.0);
                                }
                            }
                        }
                    }
                }
            }
            b"w:w" if *in_level_run_props => {
                if let (Some(level), Some(percent)) = (
                    current_level.as_mut(),
                    crate::parser::parse_text_scale_percent(e.attributes()),
                ) {
                    ensure_run_props(level).text_scale = Some(percent);
                }
            }
            b"w:position" if *in_level_run_props => {
                if let (Some(level), Some(points)) = (
                    current_level.as_mut(),
                    crate::parser::parse_text_position_points(e.attributes()),
                ) {
                    ensure_run_props(level).text_position = Some(points);
                }
            }
            b"w:kern" if *in_level_run_props => {
                if let (Some(level), Some(points)) = (
                    current_level.as_mut(),
                    crate::parser::parse_kerning_points(e),
                ) {
                    ensure_run_props(level).kerning = Some(points);
                }
            }
            b"w:snapToGrid" if *in_level_run_props => {
                if let Some(level) = current_level.as_mut() {
                    ensure_run_props(level).snap_to_grid = Some(
                        attr_value(e, b"w:val")
                            .is_none_or(|value| !matches!(value.as_ref(), "0" | "false" | "off")),
                    );
                }
            }
            b"w:spacing" if *in_level_run_props => {
                if let (Some(level), Some(value)) =
                    (current_level.as_mut(), attr_value(e, b"w:val"))
                {
                    if let Ok(twips) = value.parse::<f32>() {
                        if twips.is_finite() {
                            ensure_run_props(level).char_spacing = Some(twips / 20.0);
                        }
                    }
                }
            }
            b"w:rFonts" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        let fonts = props.fonts.get_or_insert_with(RunFonts::default);
                        for attr in e.attributes().flatten() {
                            let key = attr.key.as_ref();
                            let value = attr
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value));
                            match key {
                                b"w:ascii" => {
                                    fonts.ascii = Some(Rc::from(value.as_ref()));
                                }
                                b"w:hAnsi" => {
                                    fonts.h_ansi = Some(Rc::from(value.as_ref()));
                                }
                                b"w:eastAsia" => {
                                    fonts.east_asia = Some(Rc::from(value.as_ref()));
                                }
                                b"w:cs" => {
                                    fonts.complex = Some(Rc::from(value.as_ref()));
                                }
                                b"w:asciiTheme" => {
                                    fonts.ascii_theme = Some(Rc::from(value.as_ref()));
                                }
                                b"w:hAnsiTheme" => {
                                    fonts.h_ansi_theme = Some(Rc::from(value.as_ref()));
                                }
                                b"w:eastAsiaTheme" => {
                                    fonts.east_asia_theme = Some(Rc::from(value.as_ref()));
                                }
                                b"w:cstheme" => {
                                    fonts.complex_theme = Some(Rc::from(value.as_ref()));
                                }
                                b"w:hint" => {
                                    fonts.hint = FontHint::from_xml_hint(&value);
                                }
                                _ => {}
                            }
                        }
                        fonts.drop_names_shadowed_by_themes();
                        if props.font_family.is_none() {
                            if let Some(family) = fonts.preferred_latin() {
                                props.font_family = Some(Rc::from(family));
                            }
                        }
                    }
                }
            }
            b"w:lang" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        if let Some(language) = attr_value(e, b"w:val") {
                            ensure_run_props(level).language = Some(Rc::from(language.as_ref()));
                            level.language = Some(language.into_owned());
                        }
                        if let Some(language) = attr_value(e, b"w:eastAsia") {
                            ensure_run_props(level).east_asia_language =
                                Some(Rc::from(language.as_ref()));
                        }
                    }
                }
            }
            b"w:num" => {
                if let Some(id) = attr_value(e, b"w:numId") {
                    *current_num = Some(NumberingInstance {
                        id: id.into_owned(),
                        abstract_num_id: String::new(),
                        overrides: HashMap::new(),
                    });
                }
            }
            b"w:abstractNumId" => {
                if let Some(num) = current_num.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        num.abstract_num_id = val.into_owned();
                    }
                }
            }
            b"w:lvlOverride" => {
                if let Some(ilvl) = attr_value(e, b"w:ilvl") {
                    *current_override_level = ilvl.parse::<u32>().ok();
                }
            }
            b"w:startOverride" => {
                if let (Some(level), Some(num)) = (*current_override_level, current_num.as_mut()) {
                    if let Some(val) = attr_value(e, b"w:val") {
                        let override_level = num.overrides.entry(level).or_default();
                        override_level.start = val.parse().ok();
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_end_event(
        &mut self,
        name: &[u8],
        current_abstract: &mut Option<AbstractNum>,
        current_level: &mut Option<NumberingLevel>,
        current_num: &mut Option<NumberingInstance>,
        current_override_level: &mut Option<u32>,
        in_level_paragraph_props: &mut bool,
        in_level_run_props: &mut bool,
    ) {
        match name {
            b"w:lvl" => {
                if let Some(level) = current_level.take() {
                    if let (Some(index), Some(num)) =
                        (*current_override_level, current_num.as_mut())
                    {
                        let mut level = level;
                        level.ilvl = index;
                        num.overrides.entry(index).or_default().level = Some(level);
                    } else if let Some(abs) = current_abstract.as_mut() {
                        abs.levels.push(level);
                    }
                }
            }
            b"w:abstractNum" => {
                if let Some(abs) = current_abstract.take() {
                    if !abs.id.is_empty() {
                        self.abstract_nums.insert(abs.id.clone(), abs);
                    }
                }
            }
            b"w:num" => {
                if let Some(num) = current_num.take() {
                    if !num.id.is_empty() && !num.abstract_num_id.is_empty() {
                        self.numbering_instances.insert(num.id.clone(), num);
                    }
                }
            }
            b"w:lvlOverride" => {
                *current_override_level = None;
            }
            b"w:pPr" => {
                *in_level_paragraph_props = false;
            }
            b"w:rPr" => {
                *in_level_run_props = false;
            }
            _ => {}
        }
    }

    /// Finds an existing numbering instance whose level-0 format matches
    /// `num_fmt` (e.g. `"bullet"` or `"decimal"`).  Returns the instance ID.
    pub fn find_instance_by_format(&self, num_fmt: &str) -> Option<&str> {
        for id in self.numbering_instances.keys() {
            if self
                .get_level_info(id, 0)
                .and_then(|level| level.num_fmt.as_deref())
                == Some(num_fmt)
            {
                return Some(id.as_str());
            }
        }
        None
    }

    /// Returns an existing bullet list numbering instance ID, or creates one
    /// and returns its ID.
    pub fn ensure_bullet_instance(&mut self) -> String {
        if let Some(id) = self.find_instance_by_format("bullet") {
            return id.to_string();
        }
        self.create_builtin_instance("bullet")
    }

    /// Returns an existing numbered list instance ID, or creates one and
    /// returns its ID.
    pub fn ensure_decimal_instance(&mut self) -> String {
        if let Some(id) = self.find_instance_by_format("decimal") {
            return id.to_string();
        }
        self.create_builtin_instance("decimal")
    }

    /// Returns the stable instance for a built-in nine-level list gallery,
    /// creating the definition when this document has not used it before.
    pub fn ensure_multilevel_instance(&mut self, preset: MultilevelListPreset) -> String {
        let existing = self
            .abstract_nums
            .values()
            .find(|definition| definition.preset == Some(preset))
            .map(|definition| definition.id.clone());
        if let Some(abstract_id) = &existing {
            if let Some((id, _)) = self
                .numbering_instances
                .iter()
                .find(|(_, instance)| &instance.abstract_num_id == abstract_id)
            {
                return id.clone();
            }
        }

        // `w:abstractNumId` is an integer (ST_DecimalNumber); Word refuses a
        // numbering part whose ids are not.
        let abstract_id = existing.unwrap_or_else(|| {
            let id = self.next_abstract_num_id();
            self.abstract_nums.insert(
                id.clone(),
                AbstractNum {
                    id: id.clone(),
                    levels: multilevel_preset_levels(preset),
                    preset: Some(preset),
                },
            );
            id
        });
        let instance_id = self.next_instance_id();
        self.numbering_instances.insert(
            instance_id.clone(),
            NumberingInstance {
                id: instance_id.clone(),
                abstract_num_id: abstract_id,
                overrides: HashMap::new(),
            },
        );
        instance_id
    }

    /// Creates a distinct abstract definition and numbering instance for a
    /// custom multilevel list. Custom definitions are intentionally not
    /// deduplicated: Word's "Define New List" command creates a new list whose
    /// later restart/continue behavior must remain independent from an older
    /// definition with the same visible formatting.
    pub fn create_custom_multilevel_instance(&mut self, levels: Vec<NumberingLevel>) -> String {
        let abstract_id = self.next_abstract_num_id();
        let instance_id = self.next_instance_id();
        self.abstract_nums.insert(
            abstract_id.clone(),
            AbstractNum {
                id: abstract_id.clone(),
                levels,
                preset: None,
            },
        );
        self.numbering_instances.insert(
            instance_id.clone(),
            NumberingInstance {
                id: instance_id.clone(),
                abstract_num_id: abstract_id,
                overrides: HashMap::new(),
            },
        );
        instance_id
    }

    /// Creates a minimal abstract numbering + instance for the given format.
    fn create_builtin_instance(&mut self, num_fmt: &str) -> String {
        let abstract_id = self.next_abstract_num_id();
        let instance_id = self.next_instance_id();

        let (lvl_text, run_props) = if num_fmt == "bullet" {
            (
                Some("\u{2022}".to_string()), // bullet character
                Some(RunProperties {
                    fonts: Some(RunFonts {
                        ascii: Some(Rc::from("Symbol")),
                        h_ansi: Some(Rc::from("Symbol")),
                        hint: Some(FontHint::Default),
                        ..RunFonts::default()
                    }),
                    ..RunProperties::default()
                }),
            )
        } else {
            (Some("%1.".to_string()), None)
        };

        let abstract_num = AbstractNum {
            id: abstract_id.clone(),
            levels: vec![NumberingLevel {
                ilvl: 0,
                start: Some(1),
                restart: None,
                num_fmt: Some(num_fmt.to_string()),
                lvl_text,
                lvl_jc: Some("left".to_string()),
                suffix: Some("tab".to_string()),
                indent: Some(NumberingIndent {
                    left: Some(36.0),
                    hanging: Some(18.0),
                }),
                paragraph_style: None,
                paragraph_props: None,
                run_props,
                language: None,
                is_legal: false,
                picture_bullet: None,
            }],
            preset: None,
        };

        let instance = NumberingInstance {
            id: instance_id.clone(),
            abstract_num_id: abstract_id.clone(),
            overrides: HashMap::new(),
        };

        self.abstract_nums.insert(abstract_id, abstract_num);
        self.numbering_instances
            .insert(instance_id.clone(), instance);
        instance_id
    }

    fn next_instance_id(&self) -> String {
        let max_numeric = self
            .numbering_instances
            .keys()
            .filter_map(|id| id.parse::<u32>().ok())
            .max()
            .unwrap_or(999);
        let mut candidate = max_numeric.saturating_add(1).max(1000);
        while self
            .numbering_instances
            .contains_key(&candidate.to_string())
        {
            candidate = candidate.saturating_add(1);
        }
        candidate.to_string()
    }

    fn next_abstract_num_id(&self) -> String {
        let max_numeric = self
            .abstract_nums
            .keys()
            .filter_map(|id| id.parse::<u32>().ok())
            .max()
            .unwrap_or(999);
        let mut candidate = max_numeric.saturating_add(1).max(1000);
        while self.abstract_nums.contains_key(&candidate.to_string()) {
            candidate = candidate.saturating_add(1);
        }
        candidate.to_string()
    }

    /// Retrieves the numbering level configuration for a concrete numbering
    /// instance. Returns `None` when the instance or level is missing.
    pub fn get_level_info(&self, num_id: &str, level: u32) -> Option<&NumberingLevel> {
        let instance = self.numbering_instances.get(num_id)?;
        if let Some(replacement) = instance
            .overrides
            .get(&level)
            .and_then(|value| value.level.as_ref())
        {
            return Some(replacement);
        }
        let abstract_num = self.abstract_nums.get(&instance.abstract_num_id)?;
        abstract_num.levels.iter().find(|l| l.ilvl == level)
    }

    /// All stored levels, including instance-specific replacements.
    pub fn levels_mut(&mut self) -> impl Iterator<Item = &mut NumberingLevel> {
        self.abstract_nums
            .values_mut()
            .flat_map(|definition| definition.levels.iter_mut())
            .chain(self.numbering_instances.values_mut().flat_map(|instance| {
                instance
                    .overrides
                    .values_mut()
                    .filter_map(|value| value.level.as_mut())
            }))
    }

    pub fn has_instance(&self, num_id: &str) -> bool {
        self.numbering_instances.contains_key(num_id)
    }

    pub fn abstract_num_id(&self, num_id: &str) -> Option<&str> {
        self.numbering_instances
            .get(num_id)
            .map(|instance| instance.abstract_num_id.as_str())
    }

    pub fn has_start_override(&self, num_id: &str, level: u32) -> bool {
        self.numbering_instances
            .get(num_id)
            .and_then(|instance| instance.overrides.get(&level))
            .is_some_and(|value| value.start.is_some())
    }

    pub fn start_value(&self, num_id: &str, level: u32) -> Option<u32> {
        let instance = self.numbering_instances.get(num_id)?;
        if let Some(override_level) = instance.overrides.get(&level) {
            if override_level.start.is_some() {
                return override_level.start;
            }
        }
        self.abstract_level_info(num_id, level)
            .and_then(|level| level.start)
    }

    fn abstract_level_info(&self, num_id: &str, level: u32) -> Option<&NumberingLevel> {
        let instance = self.numbering_instances.get(num_id)?;
        self.abstract_nums
            .get(&instance.abstract_num_id)?
            .levels
            .iter()
            .find(|info| info.ilvl == level)
    }

    /// Word ignores start/restart properties inside a concrete level override.
    pub fn restart_value(&self, num_id: &str, level: u32) -> Option<u32> {
        self.abstract_level_info(num_id, level)
            .and_then(|info| info.restart)
            .filter(|restart| *restart <= level && *restart <= 7)
    }

    /// Formats a numbering text according to the level template (e.g. `%1.` or
    /// `%1.%2.`). The method depends on the [`NumberingCounter`] to provide
    /// per-level counter values.
    pub fn format_number(&self, num_id: &str, level: u32, counters: &NumberingCounter) -> String {
        self.format_number_with_offsets(num_id, level, counters, None)
    }

    pub fn format_number_with_offsets(
        &self,
        num_id: &str,
        level: u32,
        counters: &NumberingCounter,
        offsets: Option<&HashMap<String, Vec<u32>>>,
    ) -> String {
        if !self.has_instance(num_id) {
            return String::new();
        }

        // `none` suppresses the numbering label, not merely the numeric part
        // substituted into `lvlText`. Returning before template expansion keeps
        // punctuation such as `%1.` from leaking as a standalone period.
        if self
            .get_level_info(num_id, level)
            .and_then(|info| info.num_fmt.as_deref())
            == Some("none")
        {
            return String::new();
        }

        let default_template = "%1.".to_string();

        let template = self
            .get_level_info(num_id, level)
            .and_then(|info| info.lvl_text.clone())
            .unwrap_or(default_template);

        let legal = self
            .get_level_info(num_id, level)
            .is_some_and(|info| info.is_legal);
        let mut result = String::new();
        let mut chars = template.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '%' {
                let mut digits = String::new();
                while let Some(next) = chars.peek() {
                    if next.is_ascii_digit() {
                        digits.push(*next);
                        chars.next();
                    } else {
                        break;
                    }
                }

                if digits.is_empty() {
                    result.push('%');
                    continue;
                }

                if let Ok(idx) = digits.parse::<u32>() {
                    if idx >= 1 {
                        let placeholder_level = idx - 1;
                        let mut value = counters.get_counter(num_id, placeholder_level);
                        if counters.counter_value(num_id, placeholder_level).is_none()
                            && self.get_level_info(num_id, placeholder_level).is_some()
                        {
                            value = self.start_value(num_id, placeholder_level).unwrap_or(1);
                        }
                        if let Some(offsets) = offsets {
                            if let Some(level_offsets) = offsets.get(num_id) {
                                if let Some(offset) = level_offsets.get(placeholder_level as usize)
                                {
                                    if *offset > 0 && value > 0 {
                                        value = value.saturating_sub(*offset);
                                        if value == 0 {
                                            value = 1;
                                        }
                                    }
                                }
                            }
                        }
                        // MS-OI29500 2.1.278: Word applies legal numbering to
                        // the current level as well, but preserves numFmt=none.
                        let hidden = self
                            .get_level_info(num_id, placeholder_level)
                            .and_then(|info| info.num_fmt.as_deref())
                            == Some("none");
                        let formatted = if legal && !hidden {
                            value.to_string()
                        } else {
                            self.format_counter_value(num_id, placeholder_level, value)
                        };
                        result.push_str(&formatted);
                        continue;
                    }
                }

                // Fallback to literal if parsing failed
                result.push('%');
                result.push_str(&digits);
            } else {
                result.push(ch);
            }
        }

        if let Some(level_info) = self.get_level_info(num_id, level) {
            if let Some(suffix) = level_info.suffix.as_deref() {
                if suffix == "tab" || suffix == "space" {
                    result.push(' ');
                }
            }
        }

        result
    }

    fn format_counter_value(&self, num_id: &str, level: u32, value: u32) -> String {
        let format = self
            .get_level_info(num_id, level)
            .and_then(|info| info.num_fmt.clone())
            .unwrap_or_else(|| "decimal".to_string());

        // Decimal formats represent zero directly. Preserve the existing
        // one-based fallback for alphabetic and other numbering systems.
        let value = if matches!(
            format.as_str(),
            "decimal"
                | "decimalHalfWidth"
                | "decimalFullWidth"
                | "decimalFullWidth2"
                | "decimalZero"
        ) {
            value
        } else {
            value.max(1)
        };
        let language = self
            .get_level_info(num_id, level)
            .and_then(|info| info.language.as_deref());

        format_number_value(&format, value, language)
    }
}

fn multilevel_preset_levels(preset: MultilevelListPreset) -> Vec<NumberingLevel> {
    (0..9)
        .map(|ilvl| {
            let (num_fmt, lvl_text, hanging) = match preset {
                MultilevelListPreset::DecimalHierarchy => (
                    "decimal",
                    format!(
                        "{}.",
                        (0..=ilvl)
                            .map(|index| format!("%{}", index + 1))
                            .collect::<Vec<_>>()
                            .join(".")
                    ),
                    21.6,
                ),
                MultilevelListPreset::ChineseOfficial if ilvl == 0 => {
                    ("chineseCountingThousand", "%1、".to_string(), 21.25)
                }
                MultilevelListPreset::ChineseOfficial if ilvl == 1 => {
                    ("chineseCountingThousand", "(%2)".to_string(), 21.25)
                }
                MultilevelListPreset::ChineseOfficial => {
                    ("decimal", format!("%{}.", ilvl + 1), 18.0)
                }
                MultilevelListPreset::BulletHierarchy => {
                    let glyph = ["•", "○", "■"][ilvl as usize % 3];
                    ("bullet", glyph.to_string(), 18.0)
                }
            };
            let run_props = (num_fmt == "bullet").then(|| RunProperties {
                fonts: Some(RunFonts {
                    ascii: Some(Rc::from("Symbol")),
                    h_ansi: Some(Rc::from("Symbol")),
                    hint: Some(FontHint::Default),
                    ..RunFonts::default()
                }),
                ..RunProperties::default()
            });
            NumberingLevel {
                ilvl,
                start: Some(1),
                restart: None,
                num_fmt: Some(num_fmt.to_string()),
                lvl_text: Some(lvl_text),
                lvl_jc: Some("left".to_string()),
                suffix: Some("tab".to_string()),
                indent: Some(NumberingIndent {
                    left: Some(36.0 * (ilvl + 1) as f32),
                    hanging: Some(hanging),
                }),
                paragraph_style: None,
                paragraph_props: None,
                run_props,
                language: None,
                is_legal: false,
                picture_bullet: None,
            }
        })
        .collect()
}

// Numeric attributes can be parsed while borrowed. Only values retained in the
// numbering model need an owned String; escaped or invalid values retain the
// same unescaping and lossy fallback behavior as before.
fn attr_value<'a>(e: &'a BytesStart<'_>, key: &[u8]) -> Option<Cow<'a, str>> {
    // Avoid the checked iterator's per-lookup allocation on ordinary short
    // attribute lists. Replay malformed or duplicate lists with its original
    // recovery behavior before accepting a later key.
    let mut seen = [b"".as_slice(); 8];
    let mut seen_len = 0;
    let mut attributes = e.attributes();
    attributes.with_checks(false);
    let attr = loop {
        match attributes.next() {
            Some(Ok(attr)) if !seen[..seen_len].contains(&attr.key.into_inner()) => {
                if attr.key.as_ref() == key {
                    break attr;
                }
                if seen_len == seen.len() {
                    break e
                        .attributes()
                        .flatten()
                        .find(|attr| attr.key.as_ref() == key)?;
                }
                seen[seen_len] = attr.key.into_inner();
                seen_len += 1;
            }
            Some(_) => {
                break e
                    .attributes()
                    .flatten()
                    .find(|attr| attr.key.as_ref() == key)?
            }
            None => return None,
        }
    };
    Some(
        attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .unwrap_or_else(|_| Cow::Owned(String::from_utf8_lossy(&attr.value).into_owned())),
    )
}

fn ensure_run_props(level: &mut NumberingLevel) -> &mut RunProperties {
    level.run_props.get_or_insert_with(RunProperties::default)
}

/// Keeps track of numbering counters for every numbering instance and level.
/// The structure mimics Word's behaviour where advancing one level resets the
/// deeper levels automatically.
#[derive(Clone, Debug)]
pub struct NumberingCounter {
    counters: HashMap<String, Vec<Option<u32>>>,
    abstract_counters: HashMap<String, Vec<Option<u32>>>,
}

/// The counters that can affect a marker for one concrete numbering instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberingCounterState {
    concrete: Vec<Option<u32>>,
    abstract_levels: Vec<Option<u32>>,
}

impl NumberingCounter {
    pub fn state_for(&self, num_id: &str, abstract_num_id: &str) -> NumberingCounterState {
        NumberingCounterState {
            concrete: self.counters.get(num_id).cloned().unwrap_or_default(),
            abstract_levels: self
                .abstract_counters
                .get(abstract_num_id)
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// Compares cache inputs without allocating another snapshot.
    pub fn matches_state(
        &self,
        num_id: &str,
        abstract_num_id: &str,
        state: &NumberingCounterState,
    ) -> bool {
        self.counters.get(num_id).map(Vec::as_slice).unwrap_or(&[]) == state.concrete
            && self
                .abstract_counters
                .get(abstract_num_id)
                .map(Vec::as_slice)
                .unwrap_or(&[])
                == state.abstract_levels
    }

    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            abstract_counters: HashMap::new(),
        }
    }

    /// A list that starts at a child level still has an implicit parent item.
    /// Seed absent ancestors without advancing ancestors already in use.
    pub fn initialize_ancestor_levels(
        &mut self,
        numbering: Option<&NumberingPart>,
        num_id: &str,
        level: u32,
    ) {
        if level == 0 {
            return;
        }
        if self.counters.get(num_id).is_some_and(|levels| {
            levels.len() >= level as usize && levels[..level as usize].iter().all(Option::is_some)
        }) {
            return;
        }
        let Some(numbering) = numbering else { return };
        let Some(abstract_id) = numbering.abstract_num_id(num_id) else {
            return;
        };
        let concrete = self.counters.entry(num_id.to_owned()).or_default();
        let shared = self
            .abstract_counters
            .entry(abstract_id.to_owned())
            .or_default();
        for ancestor in 0..level {
            if numbering.get_level_info(num_id, ancestor).is_none() {
                continue;
            }
            let index = ancestor as usize;
            concrete.resize(concrete.len().max(index + 1), None);
            shared.resize(shared.len().max(index + 1), None);
            if concrete[index].is_none() {
                // An overridden level restarts this num alone; seeding it
                // must not move the definition's shared list (a num that
                // restarts chapter 7 but only numbers articles leaves the
                // chapters where they were).
                if numbering.has_start_override(num_id, ancestor) {
                    concrete[index] = Some(numbering.start_value(num_id, ancestor).unwrap_or(1));
                } else {
                    concrete[index] =
                        Some(shared[index].unwrap_or_else(|| {
                            numbering.start_value(num_id, ancestor).unwrap_or(1)
                        }));
                    shared[index] = concrete[index];
                }
            }
        }
    }

    pub fn increment(
        &mut self,
        num_id: &str,
        abstract_num_id: &str,
        level: u32,
        start: Option<u32>,
        has_start_override: bool,
    ) -> u32 {
        self.increment_with(
            None,
            num_id,
            abstract_num_id,
            level,
            start,
            has_start_override,
        )
    }

    /// [`Self::increment`] reading each level's `w:lvlRestart` and start
    /// overrides from `numbering`: a deeper level restarts only after the
    /// levels its `w:lvlRestart` names (`0` never restarts it), and the
    /// ancestors a num shares with its definition show the list's values.
    pub fn increment_with(
        &mut self,
        numbering: Option<&NumberingPart>,
        num_id: &str,
        abstract_num_id: &str,
        level: u32,
        start: Option<u32>,
        has_start_override: bool,
    ) -> u32 {
        // `w:lvlRestart` is one-based: the level restarts after any level up
        // to the one it names.
        let restarts = |deeper: usize| {
            numbering
                .and_then(|numbering| numbering.restart_value(num_id, deeper as u32))
                .is_none_or(|restart| level < restart)
        };
        let counters = self.counters.entry(num_id.to_string()).or_default();

        // None distinguishes an unused/reset level from a valid zero start.
        counters.resize(counters.len().max(level as usize + 1), None);
        for (deeper, value) in counters.iter_mut().enumerate().skip(level as usize + 1) {
            if restarts(deeper) {
                *value = None;
            }
        }

        let counter = &mut counters[level as usize];
        // Word keeps one list per definition: a num without a start override
        // continues it after whichever num advanced it last. A level the
        // definition has tracked, even one a parent reset, is the list's
        // state; only an untracked one keeps the num's own.
        let abstract_levels = self
            .abstract_counters
            .entry(abstract_num_id.to_string())
            .or_default();
        let shared = abstract_levels.get(level as usize).copied();
        let previous = match shared {
            Some(shared) if !has_start_override => shared,
            _ => *counter,
        };
        let value = previous.map_or_else(|| start.unwrap_or(1), |previous| previous + 1);
        *counter = Some(value);

        while abstract_levels.len() <= level as usize {
            abstract_levels.push(None);
        }
        for (deeper, value) in abstract_levels
            .iter_mut()
            .enumerate()
            .skip((level + 1) as usize)
        {
            if restarts(deeper) {
                *value = None;
            }
        }
        abstract_levels[level as usize] = Some(value);
        if let Some(numbering) = numbering {
            let concrete = counters;
            for (ancestor, shared) in abstract_levels.iter().enumerate().take(level as usize) {
                if shared.is_some()
                    && ancestor < concrete.len()
                    && !numbering.has_start_override(num_id, ancestor as u32)
                {
                    concrete[ancestor] = *shared;
                }
            }
        }

        value
    }

    pub fn get_counter(&self, num_id: &str, level: u32) -> u32 {
        self.counter_value(num_id, level).unwrap_or(0)
    }

    fn counter_value(&self, num_id: &str, level: u32) -> Option<u32> {
        self.counters
            .get(num_id)
            .and_then(|counters| counters.get(level as usize))
            .copied()
            .flatten()
    }
}

impl Default for NumberingCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod decimal_full_width_tests {
    use super::{NumberingCounter, NumberingPart};

    #[test]
    fn formats_decimal_full_width_numbers() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="301">
                <w:lvl w:ilvl="0">
                  <w:start w:val="12"/>
                  <w:numFmt w:val="decimalFullWidth"/>
                  <w:lvlText w:val="%1"/>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="301">
                <w:abstractNumId w:val="301"/>
              </w:num>
            </w:numbering>
        "#;

        let numbering = NumberingPart::from_xml(xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();
        let num_id = "301";
        let level = 0;
        counter.increment(
            num_id,
            numbering.abstract_num_id(num_id).unwrap_or_default(),
            level,
            numbering.start_value(num_id, level),
            numbering.has_start_override(num_id, level),
        );

        assert_eq!(numbering.format_number(num_id, level, &counter), "１２");
    }
}

#[cfg(test)]
mod additional_word_number_format_tests {
    use super::{NumberingCounter, NumberingPart};

    #[test]
    fn the_formats_that_used_to_fall_back_to_decimal_are_formatted() {
        for (format, start, expected) in [
            ("koreanDigital2", 12, "一二"),
            ("vietnameseCounting", 21, "hai mươi mốt"),
            ("hindiVowels", 2, "आ"),
            ("hindiConsonants", 3, "ग"),
            ("hindiCounting", 3, "तीन"),
            ("thaiLetters", 3, "ค"),
            ("thaiCounting", 11, "สิบเอ็ด"),
            ("bahtText", 2, "สองบาทถ้วน"),
            ("dollarText", 5, "five and 00/100"),
        ] {
            assert_eq!(format_at_start(format, start), expected, "{format}");
        }
    }

    #[test]
    fn a_custom_format_wins_over_its_decimal_fallback() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
                         xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"
                         xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml">
              <w:abstractNum w:abstractNumId="1">
                <w:lvl w:ilvl="0">
                  <w:start w:val="7"/>
                  <mc:AlternateContent>
                    <mc:Choice Requires="w14"><w:numFmt w:val="custom" w:format="001, 002, 003, ..."/></mc:Choice>
                    <mc:Fallback><w:numFmt w:val="decimal"/></mc:Fallback>
                  </mc:AlternateContent>
                  <w:lvlText w:val="%1."/>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>
            </w:numbering>
        "#;
        let numbering = NumberingPart::from_xml(xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();
        counter.increment(
            "1",
            numbering.abstract_num_id("1").unwrap_or_default(),
            0,
            numbering.start_value("1", 0),
            numbering.has_start_override("1", 0),
        );
        assert_eq!(numbering.format_number("1", 0, &counter), "007.");
    }

    fn format_at_start(num_fmt: &str, start: u32) -> String {
        format_at_start_with_template(num_fmt, start, "%1")
    }

    fn format_at_start_with_template(num_fmt: &str, start: u32, template: &str) -> String {
        // Exercise the same parse/counter/template path used by real DOCX
        // numbering. Testing only the eventual kana helper would miss token
        // spelling or level-template integration regressions.
        let xml = format!(
            r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="401">
                <w:lvl w:ilvl="0">
                  <w:start w:val="{start}"/>
                  <w:numFmt w:val="{num_fmt}"/>
                  <w:lvlText w:val="{template}"/>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="401"><w:abstractNumId w:val="401"/></w:num>
            </w:numbering>
            "#
        );
        let numbering = NumberingPart::from_xml(&xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();
        counter.increment("401", "401", 0, numbering.start_value("401", 0), false);
        numbering.format_number("401", 0, &counter)
    }

    #[test]
    fn letters_roll_over_by_repeating_like_word() {
        assert_eq!(format_at_start("upperLetter", 26), "Z");
        assert_eq!(format_at_start("upperLetter", 27), "AA");
        assert_eq!(format_at_start("upperLetter", 28), "BB");
        assert_eq!(format_at_start("lowerLetter", 53), "aaa");
        assert_eq!(format_at_start("russianLower", 29), "аа");
        assert_eq!(format_at_start("russianUpper", 3), "В");
    }

    #[test]
    fn chicago_doubles_its_four_symbols() {
        let labels: Vec<_> = (1..=6).map(|n| format_at_start("chicago", n)).collect();
        assert_eq!(labels, ["*", "†", "‡", "§", "**", "††"]);
    }

    #[test]
    fn stems_branches_and_the_sexagenary_cycle() {
        assert_eq!(format_at_start("ideographTraditional", 1), "甲");
        assert_eq!(format_at_start("ideographTraditional", 10), "癸");
        assert_eq!(format_at_start("ideographTraditional", 11), "甲");
        assert_eq!(format_at_start("ideographZodiac", 12), "亥");
        assert_eq!(format_at_start("ideographZodiacTraditional", 1), "甲子");
        assert_eq!(format_at_start("ideographZodiacTraditional", 11), "甲戌");
        assert_eq!(format_at_start("ideographZodiacTraditional", 60), "癸亥");
        assert_eq!(format_at_start("ideographZodiacTraditional", 61), "甲子");
    }

    #[test]
    fn chinese_counting_keeps_one_before_hundreds_and_marks_gaps_with_zero() {
        for (value, expected) in [
            (1, "一"),
            (10, "十"),
            (11, "十一"),
            (20, "二十"),
            (100, "一百"),
            (101, "一百〇一"),
            (110, "一百一十"),
            (1_001, "一千〇一"),
            (10_000, "一万"),
            (10_001, "一万〇一"),
            (100_000, "十万"),
            (100_000_001, "一亿〇一"),
        ] {
            assert_eq!(
                format_at_start("chineseCounting", value),
                expected,
                "{value}"
            );
        }
        assert_eq!(format_at_start("chineseLegalSimplified", 10), "壹拾");
        assert_eq!(format_at_start("chineseLegalSimplified", 102), "壹佰零贰");
        assert_eq!(format_at_start("ideographLegalTraditional", 30_000), "參萬");
        assert_eq!(format_at_start("taiwaneseDigital", 105), "一〇五");
    }

    #[test]
    fn korean_formats() {
        assert_eq!(format_at_start("ganada", 1), "가");
        assert_eq!(format_at_start("ganada", 14), "하");
        assert_eq!(format_at_start("chosung", 2), "ㄴ");
        assert_eq!(format_at_start("koreanDigital", 10), "일영");
        assert_eq!(format_at_start("koreanCounting", 1), "일");
        assert_eq!(format_at_start("koreanCounting", 11), "십일");
        assert_eq!(format_at_start("koreanCounting", 215), "이백십오");
        assert_eq!(format_at_start("koreanCounting", 10_000), "만");
        assert_eq!(format_at_start("koreanLegal", 1), "하나");
        assert_eq!(format_at_start("koreanLegal", 21), "스물하나");
        assert_eq!(format_at_start("koreanLegal", 100), "100");
    }

    #[test]
    fn hebrew_and_arabic_formats() {
        assert_eq!(format_at_start("hebrew1", 1), "א");
        assert_eq!(format_at_start("hebrew1", 15), "טו");
        assert_eq!(format_at_start("hebrew1", 16), "טז");
        assert_eq!(format_at_start("hebrew1", 115), "קטו");
        assert_eq!(format_at_start("hebrew1", 742), "תשמב");
        assert_eq!(format_at_start("hebrew2", 22), "ת");
        assert_eq!(format_at_start("arabicAlpha", 3), "ت");
        assert_eq!(format_at_start("arabicAbjad", 3), "ج");
    }

    #[test]
    fn shared_formatter_accepts_lowercased_section_tokens() {
        use super::format_number_value;
        assert_eq!(format_number_value("upperroman", 4, None), "IV");
        assert_eq!(format_number_value("decimalfullwidth", 12, None), "１２");
        assert_eq!(format_number_value("japanesecounting", 21, None), "二十一");
        assert_eq!(format_number_value("decimalenclosedcircle", 3, None), "③");
        // Zero has no letter or Roman form; it stays visible as a number.
        assert_eq!(format_number_value("lowerroman", 0, None), "0");
        assert_eq!(format_number_value("upperletter", 0, None), "0");
        assert_eq!(format_number_value("decimalZero", 0, None), "00");
    }

    #[test]
    fn decimal_zero_pads_only_single_digit_values() {
        assert_eq!(format_at_start("decimalZero", 1), "01");
        assert_eq!(format_at_start("decimalZero", 9), "09");
        assert_eq!(format_at_start("decimalZero", 10), "10");
        assert_eq!(format_at_start("decimalZero", 100), "100");
    }

    #[test]
    fn aiueo_matches_word_katakana_width_variants() {
        assert_eq!(format_at_start("aiueo", 1), "ｱ");
        assert_eq!(format_at_start("aiueo", 6), "ｶ");
        assert_eq!(format_at_start("aiueo", 46), "ﾝ");
        assert_eq!(format_at_start("aiueoFullWidth", 1), "ア");
        assert_eq!(format_at_start("aiueoFullWidth", 6), "カ");
        assert_eq!(format_at_start("aiueoFullWidth", 46), "ン");
    }

    #[test]
    fn iroha_matches_word_katakana_width_variants() {
        assert_eq!(format_at_start("iroha", 1), "ｲ");
        assert_eq!(format_at_start("iroha", 2), "ﾛ");
        // Unicode has no half-width variants for the historic iroha kana.
        assert_eq!(format_at_start("iroha", 25), "ヰ");
        assert_eq!(format_at_start("iroha", 43), "ヱ");
        assert_eq!(format_at_start("iroha", 47), "ｽ");
        assert_eq!(format_at_start("irohaFullWidth", 1), "イ");
        assert_eq!(format_at_start("irohaFullWidth", 47), "ス");
    }

    #[test]
    fn japanese_digital_formats_use_positional_kanji_digits() {
        for num_fmt in ["japaneseDigitalTenThousand", "ideographDigital"] {
            assert_eq!(format_at_start(num_fmt, 1), "一");
            assert_eq!(format_at_start(num_fmt, 9), "九");
            assert_eq!(format_at_start(num_fmt, 10), "一〇");
            assert_eq!(format_at_start(num_fmt, 101), "一〇一");
            assert_eq!(format_at_start(num_fmt, 10_000), "一〇〇〇〇");
            assert_eq!(format_at_start(num_fmt, u32::MAX), "四二九四九六七二九五");
        }
    }

    #[test]
    fn japanese_counting_uses_multiplicative_kanji_units() {
        assert_eq!(format_at_start("japaneseCounting", 1), "一");
        assert_eq!(format_at_start("japaneseCounting", 10), "十");
        assert_eq!(format_at_start("japaneseCounting", 11), "十一");
        assert_eq!(format_at_start("japaneseCounting", 105), "百五");
        assert_eq!(format_at_start("japaneseCounting", 10_000), "一万");
        assert_eq!(
            format_at_start("japaneseCounting", 123_456_789),
            "一億二千三百四十五万六千七百八十九"
        );
        assert_eq!(
            format_at_start("japaneseCounting", u32::MAX),
            "四十二億九千四百九十六万七千二百九十五"
        );
    }

    #[test]
    fn japanese_legal_uses_financial_digits_and_units() {
        assert_eq!(format_at_start("japaneseLegal", 1), "壱");
        assert_eq!(format_at_start("japaneseLegal", 2), "弐");
        assert_eq!(format_at_start("japaneseLegal", 3), "参");
        assert_eq!(format_at_start("japaneseLegal", 5), "伍");
        assert_eq!(format_at_start("japaneseLegal", 10), "拾");
        assert_eq!(format_at_start("japaneseLegal", 11), "拾壱");
        assert_eq!(format_at_start("japaneseLegal", 20), "弐拾");
        assert_eq!(format_at_start("japaneseLegal", 10_000), "壱萬");
    }

    #[test]
    fn hex_and_number_in_dash_match_word_examples() {
        assert_eq!(format_at_start("hex", 9), "9");
        assert_eq!(format_at_start("hex", 10), "A");
        assert_eq!(format_at_start("hex", 255), "FF");
        assert_eq!(format_at_start("numberInDash", 12), "- 12 -");
    }

    #[test]
    fn none_suppresses_the_entire_numbering_template() {
        // A formatter that only replaces %1 with an empty string would leak the
        // punctuation from `%1.`. Word's `none` means no numbering label at all.
        assert_eq!(format_at_start_with_template("none", 1, "%1."), "");
    }

    #[test]
    fn enclosed_decimal_formats_use_unicode_number_glyphs_with_documented_limits() {
        assert_eq!(format_at_start("decimalEnclosedCircle", 1), "①");
        assert_eq!(format_at_start("decimalEnclosedCircle", 20), "⑳");
        assert_eq!(format_at_start("decimalEnclosedCircle", 21), "21");

        assert_eq!(format_at_start("decimalEnclosedFullstop", 1), "⒈");
        assert_eq!(format_at_start("decimalEnclosedFullstop", 20), "⒛");
        assert_eq!(format_at_start("decimalEnclosedFullstop", 21), "21");

        assert_eq!(format_at_start("decimalEnclosedParen", 1), "⑴");
        assert_eq!(format_at_start("decimalEnclosedParen", 20), "⒇");
        assert_eq!(format_at_start("decimalEnclosedParen", 21), "21");

        // The Chinese circle variant is specified only through ten.
        assert_eq!(format_at_start("decimalEnclosedCircleChinese", 10), "⑩");
        assert_eq!(format_at_start("decimalEnclosedCircleChinese", 11), "11");
    }

    #[test]
    fn alternate_width_formats_match_their_ooxml_digit_width() {
        // Exercise the public XML/counter/template path so the exact OOXML
        // token spellings remain covered, even though half-width decimal is
        // visually identical to the ordinary decimal formatter.
        assert_eq!(format_at_start("decimalHalfWidth", 12), "12");
        assert_eq!(format_at_start("decimalFullWidth2", 12), "１２");
    }

    #[test]
    fn ideograph_enclosed_circle_uses_circled_kanji_through_ten() {
        assert_eq!(format_at_start("ideographEnclosedCircle", 1), "㊀");
        assert_eq!(format_at_start("ideographEnclosedCircle", 10), "㊉");

        // ST_NumberFormat permits non-enclosed equivalents once the sequence
        // reaches 11. Continue in the matching ideographic counting system
        // instead of falling all the way back to Arabic decimal digits.
        assert_eq!(format_at_start("ideographEnclosedCircle", 11), "十一");
    }

    #[test]
    fn native_decimal_digit_formats_replace_every_place_value() {
        // Use multiple places, including zero, so an implementation cannot
        // accidentally pass by mapping only the first few sequence values.
        assert_eq!(format_at_start("hindiNumbers", 1), "१");
        assert_eq!(format_at_start("hindiNumbers", 2048), "२०४८");
        assert_eq!(format_at_start("thaiNumbers", 1), "๑");
        assert_eq!(format_at_start("thaiNumbers", 2048), "๒๐๔๘");
    }

    fn format_at_start_with_language(num_fmt: &str, start: u32, language: &str) -> String {
        let xml = format!(
            r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="402">
                <w:lvl w:ilvl="0">
                  <w:start w:val="{start}"/>
                  <w:numFmt w:val="{num_fmt}"/>
                  <w:lvlText w:val="%1"/>
                  <w:rPr><w:lang w:val="{language}"/></w:rPr>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="402"><w:abstractNumId w:val="402"/></w:num>
            </w:numbering>
            "#
        );
        let numbering = NumberingPart::from_xml(&xml).expect("parse localized numbering xml");
        let mut counter = NumberingCounter::new();
        counter.increment("402", "402", 0, numbering.start_value("402", 0), false);
        numbering.format_number("402", 0, &counter)
    }

    #[test]
    fn english_ordinal_uses_language_aware_suffixes() {
        for (value, expected) in [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (21, "21st"),
            (112, "112th"),
        ] {
            assert_eq!(
                format_at_start_with_language("ordinal", value, "en-US"),
                expected
            );
        }

        // The OOXML format follows the run language. Until another locale is
        // implemented, retain the existing decimal fallback instead of
        // emitting an incorrect English suffix in a Japanese numbering level.
        assert_eq!(format_at_start_with_language("ordinal", 1, "ja-JP"), "1");
    }

    #[test]
    fn english_cardinal_and_ordinal_text_follow_word_wording() {
        for (value, expected) in [
            (1, "one"),
            (21, "twenty-one"),
            (105, "one hundred five"),
            (790, "seven hundred ninety"),
            (1_001, "one thousand one"),
            (
                1_234_567,
                "one million two hundred thirty-four thousand five hundred sixty-seven",
            ),
        ] {
            assert_eq!(
                format_at_start_with_language("cardinalText", value, "en-US"),
                expected
            );
        }

        for (value, expected) in [
            (1, "first"),
            (2, "second"),
            (3, "third"),
            (12, "twelfth"),
            (20, "twentieth"),
            (21, "twenty-first"),
            (105, "one hundred fifth"),
            (790, "seven hundred ninetieth"),
            (1_000, "one thousandth"),
            (1_001, "one thousand first"),
        ] {
            assert_eq!(
                format_at_start_with_language("ordinalText", value, "en-GB"),
                expected
            );
        }

        assert_eq!(
            format_at_start_with_language("cardinalText", 21, "ja-JP"),
            "21"
        );
        assert_eq!(
            format_at_start_with_language("ordinalText", 21, "ja-JP"),
            "21"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{NumberingCounter, NumberingPart};

    #[test]
    fn attribute_fast_path_preserves_checked_lookup_recovery_and_normalization() {
        use quick_xml::events::BytesStart;
        for source in [
            r#"lvl a="1" w:val="&#x32;""#,
            r#"lvl a="1" a="2" w:val="&#x32;""#,
            r#"lvl w:val="1" w:val="2""#,
            r#"lvl a="1" invalid w:val="2""#,
            r#"lvl a="1" b="2" c="3" d="4" e="5" f="6" g="7" h="8" i="9" w:val="2""#,
            r#"lvl w:val="a&#x9;b&amp;c""#,
            r#"lvl w:val="&invalid;""#,
        ] {
            let event = BytesStart::from_content(source, 3);
            for key in [b"w:val".as_slice(), b"a", b"missing"] {
                let checked = event
                    .attributes()
                    .flatten()
                    .find(|attr| attr.key.as_ref() == key)
                    .map(|attr| {
                        attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .unwrap_or_else(|_| {
                                String::from_utf8_lossy(&attr.value).into_owned().into()
                            })
                            .into_owned()
                    });
                assert_eq!(
                    super::attr_value(&event, key).map(|value| value.into_owned()),
                    checked,
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn original_toc_child_first_list_retains_the_implicit_parent_number() {
        let bytes = include_bytes!("../../../public/fixtures/toc.docx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/numbering.xml").unwrap(),
            &mut xml,
        )
        .unwrap();
        let numbering = NumberingPart::from_xml(&xml).unwrap();
        let mut counter = NumberingCounter::new();
        // Original Word PDF: a,b,c,d,e children precede 2.Test, then
        // a,b children, 3.Test, a child, and 4.tueaeuea.
        for (level, expected) in [
            (1, "a."),
            (1, "b."),
            (1, "c."),
            (1, "d."),
            (1, "e."),
            (0, "2."),
            (1, "a."),
            (1, "b."),
            (0, "3."),
            (1, "a."),
            (0, "4."),
        ] {
            counter.initialize_ancestor_levels(Some(&numbering), "1", level);
            counter.increment(
                "1",
                numbering.abstract_num_id("1").unwrap(),
                level,
                numbering.start_value("1", level),
                numbering.has_start_override("1", level),
            );
            assert_eq!(numbering.format_number("1", level, &counter), expected);
        }
    }

    #[test]
    fn implicit_ancestors_preserve_zero_starts_overrides_and_existing_values() {
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:abstractNum w:abstractNumId="0">
            <w:lvl w:ilvl="0"><w:start w:val="0"/></w:lvl>
            <w:lvl w:ilvl="1"><w:start w:val="7"/></w:lvl>
            <w:lvl w:ilvl="2"><w:start w:val="1"/></w:lvl>
          </w:abstractNum>
          <w:num w:numId="a"><w:abstractNumId w:val="0"/></w:num>
          <w:num w:numId="b"><w:abstractNumId w:val="0"/>
            <w:lvlOverride w:ilvl="0"><w:startOverride w:val="9"/></w:lvlOverride>
          </w:num></w:numbering>"#;
        let numbering = NumberingPart::from_xml(xml).unwrap();
        let mut counter = NumberingCounter::new();
        counter.initialize_ancestor_levels(Some(&numbering), "a", 2);
        assert_eq!(counter.counter_value("a", 0), Some(0));
        assert_eq!(counter.counter_value("a", 1), Some(7));
        assert_eq!(counter.increment("a", "0", 0, Some(0), false), 1);
        counter.initialize_ancestor_levels(Some(&numbering), "a", 2);
        assert_eq!(counter.counter_value("a", 0), Some(1));
        assert_eq!(counter.counter_value("a", 1), Some(7));
        let state = counter.state_for("a", "0");
        counter.initialize_ancestor_levels(Some(&numbering), "a", 2);
        assert!(counter.matches_state("a", "0", &state));
        counter.initialize_ancestor_levels(Some(&numbering), "b", 2);
        assert_eq!(counter.counter_value("b", 0), Some(9));
        assert_eq!(counter.counter_value("b", 1), Some(7));
        assert_eq!(counter.increment("b", "0", 0, Some(9), true), 10);
    }

    #[test]
    fn numbering_levels_preserve_character_tracking_in_twips() {
        for (tracking, expected) in [
            ("-20", Some(-1.0)),
            ("10", Some(0.5)),
            ("0", Some(0.0)),
            ("invalid", None),
        ] {
            let xml = format!(
                r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0">
                <w:pPr><w:spacing w:val="60" w:before="120"/></w:pPr>
                <w:rPr><w:spacing w:val="{tracking}"/></w:rPr>
              </w:lvl><w:lvl w:ilvl="1"><w:rPr><w:spacing w:val="-10"/></w:rPr></w:lvl></w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
            );
            let part = NumberingPart::from_xml(&xml).unwrap();
            let level = part.get_level_info("1", 0).unwrap();
            assert_eq!(
                level
                    .run_props
                    .as_ref()
                    .and_then(|props| props.char_spacing),
                expected
            );
            let second_level = part.get_level_info("1", 1).unwrap();
            assert_eq!(
                second_level
                    .run_props
                    .as_ref()
                    .and_then(|props| props.char_spacing),
                Some(-0.5)
            );
        }
    }

    #[test]
    fn numbering_border_preserves_edges_and_scope() {
        for expanded in [false, true] {
            for token in ["single", "double", "dashed", "nil", "none"] {
                let attrs = format!(r#"w:val="{token}" w:sz="12" w:space="3" w:color="ABCDEF""#);
                let property = if expanded {
                    format!("<w:bdr {attrs}></w:bdr>")
                } else {
                    format!("<w:bdr {attrs}/>")
                };
                let part = NumberingPart::from_xml(&format!(r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#)).unwrap();
                for level in [0, 2] {
                    let props = part
                        .get_level_info("1", level)
                        .unwrap()
                        .run_props
                        .as_ref()
                        .unwrap();
                    let edge = props.border.as_ref().expect("numbering border");
                    assert_eq!(edge.val.as_deref(), Some(token));
                    assert_eq!(edge.color.as_deref(), Some("#ABCDEF"));
                    assert_eq!(edge.size_eighths, Some(12));
                    assert_eq!(edge.space, Some(3));
                }
                assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
            }
        }
    }

    #[test]
    fn numbering_shading_preserves_colors_and_scope() {
        for expanded in [false, true] {
            for token in ["clear", "solid", "pct25", "nil"] {
                let attrs = format!(r#"w:val="{token}" w:color="123456" w:fill="ABCDEF""#);
                let property = if expanded {
                    format!("<w:shd {attrs}></w:shd>")
                } else {
                    format!("<w:shd {attrs}/>")
                };
                let part = NumberingPart::from_xml(&format!(r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#)).unwrap();
                for level in [0, 2] {
                    let props = part
                        .get_level_info("1", level)
                        .unwrap()
                        .run_props
                        .as_ref()
                        .unwrap();
                    let shading = props.shading.as_ref().expect("numbering shading");
                    assert_eq!(shading.val.as_deref(), Some(token));
                    assert_eq!(shading.color.as_deref(), Some("#123456"));
                    assert_eq!(shading.fill.as_deref(), Some("#ABCDEF"));
                }
                assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
            }
        }
    }

    #[test]
    fn numbering_emphasis_marks_preserve_tokens_and_scope() {
        for expanded in [false, true] {
            for (value, expected) in [
                (Some("dot"), Some("dot")),
                (Some("comma"), Some("comma")),
                (Some("circle"), Some("circle")),
                (Some("underDot"), Some("underdot")),
                (Some("none"), Some("none")),
                (Some(""), None),
                (None, None),
            ] {
                let attrs = value
                    .map(|value| format!(r#" w:val="{value}""#))
                    .unwrap_or_default();
                let property = if expanded {
                    format!("<w:em{attrs}></w:em>")
                } else {
                    format!("<w:em{attrs}/>")
                };
                let part = NumberingPart::from_xml(&format!(r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#)).unwrap();
                for level in [0, 2] {
                    assert_eq!(
                        part.get_level_info("1", level)
                            .unwrap()
                            .run_props
                            .as_ref()
                            .and_then(|props| props.emphasis.as_deref()),
                        expected,
                        "{property}, level={level}"
                    );
                }
                assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
            }
        }
    }

    #[test]
    fn numbering_character_effects_preserve_explicit_on_off() {
        for expanded in [false, true] {
            for (token, expected) in [
                (None, true),
                (Some("1"), true),
                (Some("true"), true),
                (Some("on"), true),
                (Some("0"), false),
                (Some("false"), false),
                (Some("off"), false),
            ] {
                let attrs = token
                    .map(|value| format!(r#" w:val="{value}""#))
                    .unwrap_or_default();
                for tag in ["shadow", "outline", "emboss", "imprint"] {
                    let property = if expanded {
                        format!("<w:{tag}{attrs}></w:{tag}>")
                    } else {
                        format!("<w:{tag}{attrs}/>")
                    };
                    let xml = format!(
                        r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#
                    );
                    let part = NumberingPart::from_xml(&xml).unwrap();
                    for level in [0, 2] {
                        let props = part
                            .get_level_info("1", level)
                            .unwrap()
                            .run_props
                            .as_ref()
                            .expect("numbering character effect must be read");
                        for (name, value) in [
                            ("shadow", props.shadow),
                            ("outline", props.outline),
                            ("emboss", props.emboss),
                            ("imprint", props.imprint),
                        ] {
                            assert_eq!(
                                value,
                                if name == tag { Some(expected) } else { None },
                                "{property}: level={level}, effect={name}"
                            );
                        }
                    }
                    assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
                }
            }
        }
    }

    #[test]
    fn numbering_languages_preserve_independent_slots() {
        for expanded in [false, true] {
            for (attrs, language, east_asia) in [
                (
                    r#"w:val="fr-FR" w:eastAsia="ja-JP""#,
                    Some("fr-FR"),
                    Some("ja-JP"),
                ),
                (r#"w:eastAsia="zh-CN""#, None, Some("zh-CN")),
                (r#"w:val="de-DE""#, Some("de-DE"), None),
            ] {
                let property = if expanded {
                    format!("<w:lang {attrs}></w:lang>")
                } else {
                    format!("<w:lang {attrs}/>")
                };
                let xml = format!(
                    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#
                );
                let part = NumberingPart::from_xml(&xml).unwrap();
                for level in [0, 2] {
                    let level = part.get_level_info("1", level).unwrap();
                    assert_eq!(level.language.as_deref(), language);
                    let props = level.run_props.as_ref().unwrap();
                    assert_eq!(props.language.as_deref(), language);
                    assert_eq!(props.east_asia_language.as_deref(), east_asia);
                }
                let guard = part.get_level_info("1", 1).unwrap();
                assert!(guard.language.is_none());
                assert!(guard.run_props.is_none());
            }
        }
    }

    #[test]
    fn numbering_complex_script_preserves_independent_properties() {
        for expanded in [false, true] {
            for (token, expected) in [
                (None, true),
                (Some("1"), true),
                (Some("true"), true),
                (Some("on"), true),
                (Some("0"), false),
                (Some("false"), false),
                (Some("off"), false),
            ] {
                let attrs = token
                    .map(|value| format!(r#" w:val="{value}""#))
                    .unwrap_or_default();
                let property = if expanded {
                    format!("<w:bCs{attrs}></w:bCs><w:iCs{attrs}></w:iCs><w:szCs w:val=\"37\"></w:szCs><w:b w:val=\"0\"/><w:i w:val=\"0\"/><w:sz w:val=\"22\"/>")
                } else {
                    format!("<w:bCs{attrs}/><w:iCs{attrs}/><w:szCs w:val=\"37\"/><w:b w:val=\"0\"/><w:i w:val=\"0\"/><w:sz w:val=\"22\"/>")
                };
                let xml = format!(
                    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#
                );
                let part = NumberingPart::from_xml(&xml).unwrap();
                for level in [0, 2] {
                    let props = part
                        .get_level_info("1", level)
                        .unwrap()
                        .run_props
                        .as_ref()
                        .unwrap();
                    assert_eq!(props.bold_complex, Some(expected));
                    assert_eq!(props.italic_complex, Some(expected));
                    assert_eq!(props.font_size_complex, Some(18.5));
                    assert_eq!(props.bold, Some(false));
                    assert_eq!(props.italic, Some(false));
                    assert_eq!(props.font_size, Some(11.0));
                }
                assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
            }
        }
    }

    #[test]
    fn numbering_small_caps_preserves_explicit_on_off() {
        for expanded in [false, true] {
            for (token, expected) in [
                (None, true),
                (Some("1"), true),
                (Some("true"), true),
                (Some("on"), true),
                (Some("0"), false),
                (Some("false"), false),
                (Some("off"), false),
            ] {
                let attrs = token
                    .map(|value| format!(r#" w:val="{value}""#))
                    .unwrap_or_default();
                let property = if expanded {
                    format!("<w:smallCaps{attrs}></w:smallCaps>")
                } else {
                    format!("<w:smallCaps{attrs}/>")
                };
                let xml = format!(
                    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#
                );
                let part = NumberingPart::from_xml(&xml).unwrap();
                for level in [0, 2] {
                    assert_eq!(
                        part.get_level_info("1", level)
                            .unwrap()
                            .run_props
                            .as_ref()
                            .and_then(|props| props.small_caps),
                        Some(expected)
                    );
                }
                assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
            }
        }
    }

    #[test]
    fn numbering_caps_preserves_explicit_on_off() {
        for expanded in [false, true] {
            for (token, expected) in [
                (None, true),
                (Some("1"), true),
                (Some("true"), true),
                (Some("on"), true),
                (Some("0"), false),
                (Some("false"), false),
                (Some("off"), false),
            ] {
                let attrs = token
                    .map(|value| format!(r#" w:val="{value}""#))
                    .unwrap_or_default();
                let property = if expanded {
                    format!("<w:caps{attrs}></w:caps>")
                } else {
                    format!("<w:caps{attrs}/>")
                };
                let xml = format!(
                    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#
                );
                let part = NumberingPart::from_xml(&xml).unwrap();
                for level in [0, 2] {
                    assert_eq!(
                        part.get_level_info("1", level)
                            .unwrap()
                            .run_props
                            .as_ref()
                            .and_then(|props| props.caps),
                        Some(expected)
                    );
                }
                assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
            }
        }
    }

    #[test]
    fn numbering_strike_and_baseline_preserve_explicit_values() {
        for expanded in [false, true] {
            for (token, expected) in [
                (None, true),
                (Some("1"), true),
                (Some("true"), true),
                (Some("on"), true),
                (Some("0"), false),
                (Some("false"), false),
                (Some("off"), false),
            ] {
                for (tag, double) in [("strike", false), ("dstrike", true)] {
                    let attrs = token
                        .map(|v| format!(r#" w:val="{v}""#))
                        .unwrap_or_default();
                    let property = if expanded {
                        format!("<w:{tag}{attrs}></w:{tag}>")
                    } else {
                        format!("<w:{tag}{attrs}/>")
                    };
                    let xml = format!(
                        r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{property}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
                    );
                    let part = NumberingPart::from_xml(&xml).unwrap();
                    let props = part
                        .get_level_info("1", 0)
                        .unwrap()
                        .run_props
                        .clone()
                        .unwrap_or_default();
                    assert_eq!(
                        if double {
                            props.double_strike
                        } else {
                            props.strike
                        },
                        Some(expected),
                        "tag={tag}, token={token:?}, expanded={expanded}"
                    );
                    assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
                }
            }
            for alignment in ["superscript", "subscript", "baseline"] {
                let property = if expanded {
                    format!(r#"<w:vertAlign w:val="{alignment}"></w:vertAlign>"#)
                } else {
                    format!(r#"<w:vertAlign w:val="{alignment}"/>"#)
                };
                let xml = format!(
                    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{property}</w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="1"><w:lvl w:ilvl="1"><w:rPr>{property}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#
                );
                let part = NumberingPart::from_xml(&xml).unwrap();
                for level in [0, 1] {
                    assert_eq!(
                        part.get_level_info("1", level)
                            .unwrap()
                            .run_props
                            .as_ref()
                            .and_then(|p| p.baseline.as_deref()),
                        Some(alignment)
                    );
                }
            }
        }
    }

    #[test]
    fn numbering_character_metrics_preserve_overrides_and_resets() {
        for expanded in [false, true] {
            for (scale, position, kern, expected) in [
                ("50", "-6", "16", (Some(50.0), Some(-3.0), Some(8.0))),
                ("100", "0", "0", (Some(100.0), Some(0.0), Some(0.0))),
                ("NaN", "inf", "-1", (None, None, None)),
            ] {
                let properties: String = [("w", scale), ("position", position), ("kern", kern)]
                    .into_iter()
                    .map(|(tag, value)| {
                        if expanded {
                            format!(r#"<w:{tag} w:val="{value}"></w:{tag}>"#)
                        } else {
                            format!(r#"<w:{tag} w:val="{value}"/>"#)
                        }
                    })
                    .collect();
                let xml = format!(
                    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{properties}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{properties}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="2"><w:lvl w:ilvl="2"><w:rPr>{properties}</w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#
                );
                let part = NumberingPart::from_xml(&xml).unwrap();
                for level in [0, 2] {
                    let props = part
                        .get_level_info("1", level)
                        .unwrap()
                        .run_props
                        .clone()
                        .unwrap_or_default();
                    assert_eq!(
                        (props.text_scale, props.text_position, props.kerning),
                        expected,
                        "expanded={expanded}, level={level}, scale={scale}"
                    );
                }
                assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
            }
        }
    }

    #[test]
    fn numbering_emphasis_preserves_explicit_on_off_in_both_element_forms() {
        for (token, expected) in [
            (None, true),
            (Some("0"), false),
            (Some("false"), false),
            (Some("off"), false),
            (Some("1"), true),
            (Some("true"), true),
            (Some("on"), true),
        ] {
            for expanded in [false, true] {
                let attrs = token
                    .map(|token| format!(r#" w:val="{token}""#))
                    .unwrap_or_default();
                let properties: String = ["b", "i"]
                    .into_iter()
                    .map(|tag| {
                        if expanded {
                            format!("<w:{tag}{attrs}></w:{tag}>")
                        } else {
                            format!("<w:{tag}{attrs}/>")
                        }
                    })
                    .collect();
                let xml = format!(
                    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{properties}</w:rPr></w:lvl><w:lvl w:ilvl="1"><w:pPr>{properties}</w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
                );
                let part = NumberingPart::from_xml(&xml).unwrap();
                let props = part
                    .get_level_info("1", 0)
                    .unwrap()
                    .run_props
                    .as_ref()
                    .unwrap();
                assert_eq!(
                    props.bold,
                    Some(expected),
                    "token={token:?}, expanded={expanded}"
                );
                assert_eq!(
                    props.italic,
                    Some(expected),
                    "token={token:?}, expanded={expanded}"
                );
                assert!(part.get_level_info("1", 1).unwrap().run_props.is_none());
            }
        }
    }

    #[test]
    fn run_grid_switch_is_preserved_in_numbering_levels() {
        for (value, expected) in [
            ("0", false),
            ("false", false),
            ("off", false),
            ("1", true),
            ("true", true),
            ("on", true),
        ] {
            for expanded in [false, true] {
                let element = if expanded {
                    format!(r#"<w:snapToGrid w:val="{value}"></w:snapToGrid>"#)
                } else {
                    format!(r#"<w:snapToGrid w:val="{value}"/>"#)
                };
                let xml = format!(
                    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:rPr>{element}</w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
                );
                let part = NumberingPart::from_xml(&xml).unwrap();
                assert_eq!(
                    part.get_level_info("1", 0)
                        .unwrap()
                        .run_props
                        .as_ref()
                        .unwrap()
                        .snap_to_grid,
                    Some(expected)
                );
            }
        }
    }

    #[test]
    fn zero_start_override_formats_zero_then_one() {
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0">
            <w:start w:val="7"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/>
          </w:lvl></w:abstractNum>
          <w:num w:numId="2"><w:abstractNumId w:val="1"/>
            <w:lvlOverride w:ilvl="0"><w:startOverride w:val="0"/></w:lvlOverride>
          </w:num></w:numbering>"#;
        let numbering = NumberingPart::from_xml(xml).unwrap();
        let mut counters = NumberingCounter::new();
        for expected in ["0.", "1."] {
            counters.increment(
                "2",
                "1",
                0,
                numbering.start_value("2", 0),
                numbering.has_start_override("2", 0),
            );
            assert_eq!(numbering.format_number("2", 0, &counters), expected);
        }
    }

    #[test]
    fn zero_start_advances_and_resets_child_levels() {
        let mut counter = NumberingCounter::new();
        assert_eq!(counter.increment("a", "definition", 0, Some(0), false), 0);
        assert_eq!(counter.increment("a", "definition", 1, Some(0), false), 0);
        assert_eq!(counter.increment("a", "definition", 1, Some(0), false), 1);
        assert_eq!(counter.increment("a", "definition", 0, Some(0), false), 1);
        assert_eq!(counter.increment("a", "definition", 1, Some(0), false), 0);
        assert_eq!(counter.increment("a", "definition", 1, Some(0), false), 1);
    }

    #[test]
    fn zero_start_continues_shared_definition_unless_overridden() {
        let mut counter = NumberingCounter::new();
        assert_eq!(counter.increment("a", "definition", 0, Some(0), false), 0);
        assert_eq!(counter.increment("b", "definition", 0, Some(0), false), 1);
        assert_eq!(counter.increment("c", "definition", 0, Some(0), true), 0);
        assert_eq!(counter.increment("c", "definition", 0, Some(0), true), 1);
    }

    /// Nums sharing a definition without overrides are one list in Word:
    /// scroll-offset-repro numbers `第４３条` through a heading style's num
    /// and the next article, under the paragraph's own num, `第４４条`.
    #[test]
    fn interleaved_nums_of_one_definition_continue_one_list() {
        let mut counter = NumberingCounter::new();
        assert_eq!(counter.increment("21", "16", 1, Some(1), false), 1);
        assert_eq!(counter.increment("21", "16", 1, Some(1), false), 2);
        assert_eq!(counter.increment("22", "16", 1, Some(1), false), 3);
        assert_eq!(counter.increment("21", "16", 1, Some(1), false), 4);
        assert_eq!(counter.get_counter("21", 1), 4);
        // A parent advanced through another num restarts the shared child.
        assert_eq!(counter.increment("22", "16", 0, Some(1), false), 1);
        assert_eq!(counter.increment("21", "16", 1, Some(1), false), 1);
    }

    /// scroll-offset-repro: num 23 restarts chapters at 7 and articles at 1
    /// but numbers only articles; the chapters of num 22 stay at 1, 2.
    #[test]
    fn an_unused_overridden_ancestor_leaves_the_shared_list() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="16">
                <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="第%1章"/></w:lvl>
                <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="第%2条"/></w:lvl>
              </w:abstractNum>
              <w:num w:numId="21"><w:abstractNumId w:val="16"/></w:num>
              <w:num w:numId="22"><w:abstractNumId w:val="16"/></w:num>
              <w:num w:numId="23"><w:abstractNumId w:val="16"/>
                <w:lvlOverride w:ilvl="0"><w:startOverride w:val="7"/></w:lvlOverride>
                <w:lvlOverride w:ilvl="1"><w:startOverride w:val="1"/></w:lvlOverride>
              </w:num>
            </w:numbering>
        "#;
        let numbering = NumberingPart::from_xml(xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();
        let mut number = |num_id: &str, level: u32| {
            counter.initialize_ancestor_levels(Some(&numbering), num_id, level);
            counter.increment(
                num_id,
                numbering.abstract_num_id(num_id).unwrap_or_default(),
                level,
                numbering.start_value(num_id, level),
                numbering.has_start_override(num_id, level),
            );
            numbering.format_number(num_id, level, &counter)
        };
        assert_eq!(number("22", 0), "第1章");
        assert_eq!(number("23", 1), "第1条");
        assert_eq!(number("21", 1), "第2条");
        assert_eq!(number("22", 0), "第2章");
        assert_eq!(number("21", 1), "第1条");
        assert_eq!(number("22", 1), "第2条");
        assert_eq!(number("21", 1), "第3条");
    }

    /// scroll-offset-repro numbers articles on across chapters: its article
    /// level has `w:lvlRestart w:val="0"`, and a chapter through the heading
    /// style's num does not restart them.
    #[test]
    fn level_restart_zero_continues_articles_across_chapters() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="16">
                <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="第%1章"/></w:lvl>
                <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlRestart w:val="0"/><w:lvlText w:val="第%2条"/></w:lvl>
                <w:lvl w:ilvl="2"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlRestart w:val="1"/><w:lvlText w:val="%3"/></w:lvl>
              </w:abstractNum>
              <w:num w:numId="21"><w:abstractNumId w:val="16"/></w:num>
              <w:num w:numId="22"><w:abstractNumId w:val="16"/></w:num>
            </w:numbering>
        "#;
        let numbering = NumberingPart::from_xml(xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();
        let mut number = |num_id: &str, level: u32| {
            counter.initialize_ancestor_levels(Some(&numbering), num_id, level);
            counter.increment_with(
                Some(&numbering),
                num_id,
                numbering.abstract_num_id(num_id).unwrap_or_default(),
                level,
                numbering.start_value(num_id, level),
                numbering.has_start_override(num_id, level),
            );
            numbering.format_number(num_id, level, &counter)
        };
        assert_eq!(number("22", 0), "第1章");
        assert_eq!(number("21", 1), "第1条");
        assert_eq!(number("21", 2), "1");
        assert_eq!(number("21", 2), "2");
        assert_eq!(number("21", 1), "第2条");
        // `w:lvlRestart="1"` restarts level 2 after a chapter, not an article.
        assert_eq!(number("21", 2), "3");
        assert_eq!(number("22", 0), "第2章");
        assert_eq!(number("21", 1), "第3条");
        assert_eq!(number("21", 2), "1");
    }

    #[test]
    fn parses_escaped_numeric_attributes_and_preserves_invalid_values() {
        let xml = r#"<w:numbering xmlns:w="urn:w">
          <w:abstractNum w:abstractNumId="&#49;">
            <w:lvl w:ilvl="&#48;">
              <w:start w:val="&#x32;"/><w:lvlRestart w:val="invalid"/>
              <w:lvlText w:val="&unknown;"/>
              <w:pPr><w:ind w:left="&#55;20" w:hanging="invalid"/></w:pPr>
              <w:rPr><w:sz w:val="&#50;2"/><w:spacing w:val="NaN"/>
                <w:lang w:val="ja&#45;JP"/>
                <w:rFonts w:ascii="A&amp;B" w:eastAsia="&unknown;"/>
              </w:rPr>
            </w:lvl>
          </w:abstractNum>
          <w:num w:numId="&#49;"><w:abstractNumId w:val="&#49;"/>
            <w:lvlOverride w:ilvl="&#48;"><w:startOverride w:val="&#48;"/></w:lvlOverride>
          </w:num>
        </w:numbering>"#;
        let part = NumberingPart::from_xml(xml).expect("parse escaped attributes");
        let level = part.get_level_info("1", 0).unwrap();
        assert_eq!(level.start, Some(2));
        assert_eq!(level.restart, None);
        assert_eq!(level.lvl_text.as_deref(), Some("&unknown;"));
        assert_eq!(level.language.as_deref(), Some("ja-JP"));
        let indent = level.indent.as_ref().unwrap();
        assert_eq!(indent.left, Some(36.0));
        assert_eq!(indent.hanging, None);
        let props = level.run_props.as_ref().unwrap();
        assert_eq!(props.font_size, Some(11.0));
        assert_eq!(props.char_spacing, None);
        let fonts = props.fonts.as_ref().unwrap();
        assert_eq!(fonts.ascii.as_deref(), Some("A&B"));
        assert_eq!(fonts.east_asia.as_deref(), Some("&unknown;"));
        assert_eq!(part.numbering_instances["1"].overrides[&0].start, Some(0));
    }

    #[test]
    fn unescapes_numbering_text_and_font_attributes() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="1">
                <w:lvl w:ilvl="0">
                  <w:numFmt w:val="decimal"/>
                  <w:lvlText w:val="%1. &amp; &quot;A&quot; &lt;B&gt;"/>
                  <w:rPr><w:rFonts w:ascii="A&amp;B"/></w:rPr>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>
            </w:numbering>
        "#;

        let numbering = NumberingPart::from_xml(xml).expect("parse numbering XML");
        let level = numbering.get_level_info("1", 0).expect("numbering level");
        assert_eq!(level.lvl_text.as_deref(), Some("%1. & \"A\" <B>"));
        assert_eq!(
            level
                .run_props
                .as_ref()
                .and_then(|props| props.fonts.as_ref())
                .and_then(|fonts| fonts.ascii.as_deref()),
            Some("A&B")
        );
    }

    #[test]
    fn continues_numbering_across_num_id_without_start_override() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="16">
                <w:lvl w:ilvl="1">
                  <w:start w:val="41"/>
                  <w:numFmt w:val="decimal"/>
                  <w:lvlText w:val="第%2条"/>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="23">
                <w:abstractNumId w:val="16"/>
                <w:lvlOverride w:ilvl="1">
                  <w:startOverride w:val="1"/>
                </w:lvlOverride>
              </w:num>
              <w:num w:numId="21">
                <w:abstractNumId w:val="16"/>
              </w:num>
            </w:numbering>
        "#;

        let numbering = NumberingPart::from_xml(xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();

        let first_num_id = "23";
        let first_level = 1;
        counter.increment(
            first_num_id,
            numbering.abstract_num_id(first_num_id).unwrap_or_default(),
            first_level,
            numbering.start_value(first_num_id, first_level),
            numbering.has_start_override(first_num_id, first_level),
        );
        assert_eq!(
            numbering.format_number(first_num_id, first_level, &counter),
            "第1条"
        );

        let second_num_id = "21";
        let second_level = 1;
        counter.increment(
            second_num_id,
            numbering.abstract_num_id(second_num_id).unwrap_or_default(),
            second_level,
            numbering.start_value(second_num_id, second_level),
            numbering.has_start_override(second_num_id, second_level),
        );
        assert_eq!(
            numbering.format_number(second_num_id, second_level, &counter),
            "第2条"
        );
    }
}

#[cfg(test)]
mod legal_numbering_tests {
    use super::{NumberingCounter, NumberingPart};

    /// Word's 正規形式の番号を使用する (`w:isLgl`): a 第一章 heading's
    /// second level reads `2.1`, not `二.1`; the level without it keeps the
    /// first level's own format. Word also renders the legal level's own
    /// placeholder in decimal (MS-OI29500 2.1.278).
    #[test]
    fn a_legal_level_names_every_level_in_decimal() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="7">
                <w:lvl w:ilvl="0">
                  <w:start w:val="1"/>
                  <w:numFmt w:val="japaneseCounting"/>
                  <w:lvlText w:val="第%1章"/>
                </w:lvl>
                <w:lvl w:ilvl="1">
                  <w:start w:val="1"/>
                  <w:isLgl/>
                  <w:numFmt w:val="decimal"/>
                  <w:lvlText w:val="%1.%2"/>
                </w:lvl>
                <w:lvl w:ilvl="2">
                  <w:start w:val="1"/>
                  <w:numFmt w:val="decimal"/>
                  <w:lvlText w:val="%1-%3"/>
                </w:lvl>
              </w:abstractNum>
              <w:abstractNum w:abstractNumId="8">
                <w:lvl w:ilvl="0">
                  <w:start w:val="1"/>
                  <w:numFmt w:val="upperRoman"/>
                  <w:lvlText w:val="Article %1."/>
                </w:lvl>
                <w:lvl w:ilvl="1">
                  <w:start w:val="1"/>
                  <w:numFmt w:val="decimalZero"/>
                  <w:isLgl/>
                  <w:lvlText w:val="Section %1.%2"/>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="7"><w:abstractNumId w:val="7"/></w:num>
              <w:num w:numId="8"><w:abstractNumId w:val="8"/></w:num>
            </w:numbering>
        "#;
        let numbering = NumberingPart::from_xml(xml).expect("parse numbering xml");
        assert!(numbering.get_level_info("7", 1).unwrap().is_legal);
        assert!(!numbering.get_level_info("7", 0).unwrap().is_legal);
        let mut counter = NumberingCounter::new();
        for level in [0, 0, 1, 2] {
            counter.increment_with(
                Some(&numbering),
                "7",
                "7",
                level,
                numbering.start_value("7", level),
                numbering.has_start_override("7", level),
            );
        }
        assert_eq!(numbering.format_number("7", 0, &counter), "第二章");
        assert_eq!(numbering.format_number("7", 1, &counter), "2.1");
        assert_eq!(numbering.format_number("7", 2, &counter), "二-1");
        let mut counter = NumberingCounter::new();
        for level in [0, 0, 0, 1] {
            counter.increment_with(
                Some(&numbering),
                "8",
                "8",
                level,
                numbering.start_value("8", level),
                numbering.has_start_override("8", level),
            );
        }
        assert_eq!(numbering.format_number("8", 0, &counter), "Article III.");
        assert_eq!(numbering.format_number("8", 1, &counter), "Section 3.1");
    }
}

#[cfg(test)]
mod marker_run_property_tests {
    use super::NumberingPart;

    #[test]
    fn marker_underlines_preserve_kind_color_and_explicit_disable() {
        for (underline, enabled, kind) in [
            (r#"w:val="double""#, Some(true), Some("double")),
            (r#"w:val="none""#, Some(false), None),
            (r#"w:val="0""#, Some(false), None),
            (r#"w:val="single""#, Some(true), None),
            ("", None, None),
        ] {
            let xml = format!(
                r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:rPr><w:color w:val="123456"/><w:u {underline} w:color="654321"/></w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
            );
            let numbering = NumberingPart::from_xml(&xml).unwrap();
            let props = numbering
                .get_level_info("1", 0)
                .unwrap()
                .run_props
                .as_ref()
                .unwrap();
            assert_eq!(props.underline, enabled);
            assert_eq!(props.underline_kind.as_deref(), kind);
            assert_eq!(props.underline_color.as_deref(), Some("654321"));
            assert_eq!(props.color.as_deref(), Some("#123456"));
        }
    }
}

#[cfg(test)]
mod level_replacement_tests {
    use super::{NumberingCounter, NumberingPart};

    #[test]
    fn concrete_level_replaces_format_layout_and_start_without_changing_other_instances() {
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:abstractNum w:abstractNumId="4"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:rPr><w:b/></w:rPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2."/></w:lvl></w:abstractNum>
          <w:num w:numId="5"><w:abstractNumId w:val="4"/></w:num>
          <w:num w:numId="6"><w:abstractNumId w:val="4"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="4"/><w:lvl w:ilvl="0"><w:start w:val="4"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="Article %1)"/><w:lvlJc w:val="right"/><w:suff w:val="space"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:i/><w:color w:val="123456"/></w:rPr></w:lvl></w:lvlOverride></w:num>
          <w:num w:numId="7"><w:abstractNumId w:val="4"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="8"/><w:lvl w:ilvl="0"><w:start w:val="4"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1)"/></w:lvl></w:lvlOverride></w:num>
          <w:num w:numId="8"><w:abstractNumId w:val="4"/><w:lvlOverride w:ilvl="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/></w:lvl></w:lvlOverride></w:num>
        </w:numbering>"#;
        let part = NumberingPart::from_xml(xml).unwrap();
        let level = part.get_level_info("6", 0).unwrap();
        assert_eq!(level.lvl_jc.as_deref(), Some("right"));
        assert_eq!(level.suffix.as_deref(), Some("space"));
        assert_eq!(level.indent.as_ref().unwrap().left, Some(36.0));
        assert_eq!(level.indent.as_ref().unwrap().hanging, Some(18.0));
        let props = level.run_props.as_ref().unwrap();
        assert_eq!(props.italic, Some(true));
        assert_eq!(
            props.bold, None,
            "replacement does not merge the abstract run properties"
        );
        assert_eq!(props.color.as_deref(), Some("#123456"));
        assert_eq!(part.start_value("6", 0), Some(4));
        assert_eq!(part.start_value("7", 0), Some(8));
        assert_eq!(
            part.get_level_info("6", 1).unwrap().num_fmt.as_deref(),
            Some("lowerLetter")
        );
        assert_eq!(
            part.get_level_info("5", 0).unwrap().num_fmt.as_deref(),
            Some("decimal")
        );
        assert_eq!(part.find_instance_by_format("bullet"), Some("8"));
        let mut counters = NumberingCounter::new();
        for (id, expected) in [
            ("5", "1."),
            ("6", "Article IV) "),
            ("7", "8)"),
            ("6", "Article V) "),
        ] {
            counters.increment_with(
                Some(&part),
                id,
                "4",
                0,
                part.start_value(id, 0),
                part.has_start_override(id, 0),
            );
            assert_eq!(part.format_number(id, 0, &counters), expected);
        }
    }
}

#[cfg(test)]
mod word_restart_compatibility_tests {
    use super::{NumberingCounter, NumberingPart};

    fn labels(xml: &str, paragraphs: &[(u32, u32)]) -> Vec<String> {
        let numbering = NumberingPart::from_xml(xml).unwrap();
        let mut counters = NumberingCounter::new();
        paragraphs
            .iter()
            .map(|(id, level)| {
                let id = id.to_string();
                counters.increment_with(
                    Some(&numbering),
                    &id,
                    numbering.abstract_num_id(&id).unwrap(),
                    *level,
                    numbering.start_value(&id, *level),
                    numbering.has_start_override(&id, *level),
                );
                numbering.format_number(&id, *level, &counters)
            })
            .collect()
    }

    #[test]
    fn restart_controls_which_ancestors_reset_a_level() {
        for (restart, expected) in [
            ("", ["3.", "1.", "3.", "1.", "3."]),
            (
                r#"<w:lvlRestart w:val="0"/>"#,
                ["3.", "1.", "4.", "1.", "5."],
            ),
            (
                r#"<w:lvlRestart w:val="1"></w:lvlRestart>"#,
                ["3.", "1.", "4.", "1.", "3."],
            ),
            (
                r#"<w:lvlRestart w:val="2"/>"#,
                ["3.", "1.", "3.", "1.", "3."],
            ),
            // Invalid forward references and values outside Word's range are ignored.
            (
                r#"<w:lvlRestart w:val="3"/>"#,
                ["3.", "1.", "3.", "1.", "3."],
            ),
            (
                r#"<w:lvlRestart w:val="99"/>"#,
                ["3.", "1.", "3.", "1.", "3."],
            ),
        ] {
            let xml = format!(
                r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:lvlText w:val="%1."/></w:lvl><w:lvl w:ilvl="1"><w:lvlText w:val="%2."/></w:lvl><w:lvl w:ilvl="2"><w:start w:val="3"/><w:lvlText w:val="%3."/>{restart}</w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
            );
            assert_eq!(
                labels(&xml, &[(1, 2), (1, 1), (1, 2), (1, 0), (1, 2)]),
                expected,
                "{restart}"
            );
        }
    }

    #[test]
    fn word_ignores_start_and_restart_inside_concrete_level() {
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:lvlText w:val="%1."/></w:lvl><w:lvl w:ilvl="1"><w:start w:val="3"/><w:lvlRestart w:val="0"/><w:lvlText w:val="%2."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="1"><w:lvl w:ilvl="1"><w:start w:val="9"/><w:lvlRestart w:val="1"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%2)"/></w:lvl></w:lvlOverride></w:num></w:numbering>"#;
        let part = NumberingPart::from_xml(xml).unwrap();
        assert_eq!(part.format_number("1", 1, &NumberingCounter::new()), "III)");
        assert_eq!(
            labels(xml, &[(1, 1), (1, 0), (1, 1)]),
            ["III)", "1.", "IV)"]
        );
    }

    #[test]
    fn zero_start_advances_and_shared_num_ids_follow_latest_counter() {
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="0"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
        assert_eq!(
            labels(xml, &[(1, 0), (1, 0), (2, 0), (1, 0)]),
            ["0.", "1.", "2.", "3."]
        );
    }
}

#[cfg(test)]
mod legal_word_compatibility_tests {
    use super::{NumberingCounter, NumberingPart};
    fn labels(xml: &str, paragraphs: &[(u32, u32)]) -> Vec<String> {
        let numbering = NumberingPart::from_xml(xml).unwrap();
        let mut counter = NumberingCounter::new();
        paragraphs
            .iter()
            .map(|(id, level)| {
                let id = id.to_string();
                counter.increment_with(
                    Some(&numbering),
                    &id,
                    numbering.abstract_num_id(&id).unwrap(),
                    *level,
                    numbering.start_value(&id, *level),
                    numbering.has_start_override(&id, *level),
                );
                numbering.format_number(&id, *level, &counter)
            })
            .collect()
    }

    #[test]
    fn legal_numbering_uses_decimal_for_ancestors_and_current_level() {
        for (flag, expected) in [
            ("", "III.01"),
            ("<w:isLgl/>", "3.1"),
            (r#"<w:isLgl w:val="true"></w:isLgl>"#, "3.1"),
            (r#"<w:isLgl w:val="1"/>"#, "3.1"),
            (r#"<w:isLgl w:val="on"/>"#, "3.1"),
            (r#"<w:isLgl w:val="0"/>"#, "III.01"),
            (r#"<w:isLgl w:val="false"></w:isLgl>"#, "III.01"),
            (r#"<w:isLgl w:val="off"/>"#, "III.01"),
        ] {
            let xml = format!(
                r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="decimalZero"/><w:lvlText w:val="%1.%2"/>{flag}</w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
            );
            assert_eq!(labels(&xml, &[(1, 0), (1, 1)]), ["III", expected], "{flag}");
            // Implicit ancestors use their initial value; legal decimal keeps zero.
            assert_eq!(labels(&xml, &[(1, 1)]), [expected], "{flag}");
        }
    }

    #[test]
    fn legal_numbering_honors_concrete_replacement_and_none() {
        for (abstract_flag, replacement_flag, expected) in [
            ("", "<w:isLgl/>", "3.1"),
            ("<w:isLgl/>", "", "III.a"),
            ("<w:isLgl/>", r#"<w:isLgl w:val="0"/>"#, "III.a"),
        ] {
            let xml = format!(
                r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="decimal"/><w:lvlText w:val="%2"/>{abstract_flag}</w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="1"><w:lvl w:ilvl="1"><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1.%2"/>{replacement_flag}</w:lvl></w:lvlOverride></w:num></w:numbering>"#
            );
            assert_eq!(labels(&xml, &[(1, 0), (1, 1)]), ["III", expected]);
        }
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="none"/><w:lvlText w:val="%1"/><w:isLgl/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="decimalZero"/><w:lvlText w:val="%1.%2"/><w:isLgl/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
        assert_eq!(labels(xml, &[(1, 0), (1, 1)]), ["", ".1"]);
        let xml = xml.replace(
            r#"<w:numFmt w:val="none"/>"#,
            r#"<w:start w:val="0"/><w:numFmt w:val="upperRoman"/>"#,
        );
        assert_eq!(labels(&xml, &[(1, 0), (1, 1)]), ["0", "0.1"]);
    }
}
