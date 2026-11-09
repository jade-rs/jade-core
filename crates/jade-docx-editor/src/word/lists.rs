//! Lists as Word's 箇条書き, 段落番号 and アウトライン libraries and their
//! dialogs make them:
//!
//! - the number formats of 段落番号 — Japanese ones included (一, 二 / ①② /
//!   ア, イ / イ, ロ / １, ２) — and bullets;
//! - 番号の設定 (set the value), 新しいリストを開始 and 前のリストから継続;
//! - the multilevel list library and 新しいアウトラインの定義: per level the
//!   format, the text around the number, the start, 正規形式の番号を使用する
//!   (legal), the heading style the level is linked to, the number's
//!   alignment and indents, and what follows it (tab, space, nothing);
//! - list styles (新しいリスト スタイルの定義), and the list level of a
//!   paragraph (Tab / ⇧Tab at an item's start, リストのレベルの変更).
//!
//! A list's definition lives in `numbering.xml`, shared by every item, so
//! changing one is a package edit read back in place (see `package`); putting
//! paragraphs in a list is the engine's `SetNumbering`, in the same undo
//! step.

use jade_docx_headless_api::HeadlessCommand;

use super::package::{edit_package, HeadlessStep, PartKind};
use super::styles::{PPR_ORDER, STYLE_ORDER};
use super::xml::{Document, Element, Node};
use super::WordSession;

/// What follows the number (番号に続く空白の扱い).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Suffix {
    Tab,
    Space,
    Nothing,
}

impl Suffix {
    pub const ALL: [Suffix; 3] = [Suffix::Tab, Suffix::Space, Suffix::Nothing];

    fn token(self) -> &'static str {
        match self {
            Suffix::Tab => "tab",
            Suffix::Space => "space",
            Suffix::Nothing => "nothing",
        }
    }

    fn from_token(token: Option<&str>) -> Suffix {
        match token {
            Some("space") => Suffix::Space,
            Some("nothing") => Suffix::Nothing,
            _ => Suffix::Tab,
        }
    }
}

/// One level of a list definition.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLevel {
    /// An `ST_NumberFormat` token: `decimal`, `bullet`, `japaneseCounting`,
    /// `decimalEnclosedCircle`, `aiueoFullWidth`, …
    pub format: String,
    /// Language used for language-sensitive numbering, from `w:rPr/w:lang`.
    pub language: Option<String>,
    /// The level text, `%1` standing for level 1's number: `%1.`, `第%1章`,
    /// or a bullet's glyph.
    pub text: String,
    pub start: u32,
    /// `left`, `center` or `right` (番号の配置).
    pub alignment: String,
    /// Where the text starts, points from the margin (インデント位置).
    pub indent: f32,
    /// How far the number hangs left of the text, points; the number sits
    /// at `indent - hanging` (番号の位置).
    pub hanging: f32,
    pub suffix: Suffix,
    /// 正規形式の番号を使用する.
    pub legal: bool,
    /// The paragraph style linked to the level (レベルと対応付ける見出し
    /// スタイル).
    pub style: Option<String>,
    /// The bullet's font, for a bullet level.
    pub font: Option<String>,
    /// The level restarts after this level (`w:lvlRestart`); `None` after
    /// the level above, Word's default.
    pub restart_after: Option<u8>,
    /// The picture bullet the level shows (`w:lvlPicBulletId`).
    pub picture: Option<String>,
}

impl ListLevel {
    pub fn numbered(level: u8, format: &str, text: &str) -> Self {
        ListLevel {
            format: format.to_string(),
            language: None,
            text: text.to_string(),
            start: 1,
            alignment: "left".to_string(),
            indent: 21.0 * (level as f32 + 1.0),
            hanging: 21.0,
            suffix: Suffix::Tab,
            legal: false,
            style: None,
            font: None,
            restart_after: None,
            picture: None,
        }
    }

    pub fn bullet(level: u8, glyph: &str) -> Self {
        ListLevel {
            format: "bullet".to_string(),
            ..ListLevel::numbered(level, "bullet", glyph)
        }
    }

    pub fn is_bullet(&self) -> bool {
        self.format == "bullet"
    }

    fn element(&self, level: u8) -> Element {
        let mut lvl = Element::new("w:lvl").attr("w:ilvl", &level.to_string());
        lvl.children.push(Node::Element(Element::with_val(
            "w:start",
            &self.start.to_string(),
        )));
        let format = if let Some(custom) = self
            .format
            .strip_prefix(crate::numbering::CUSTOM_FORMAT_PREFIX)
        {
            Element::with_val("w:numFmt", "custom").attr("w:format", custom)
        } else {
            Element::with_val("w:numFmt", &self.format)
        };
        lvl.children.push(Node::Element(format));
        if let Some(after) = self.restart_after {
            lvl.children.push(Node::Element(Element::with_val(
                "w:lvlRestart",
                &after.to_string(),
            )));
        }
        if let Some(style) = self.style.as_deref() {
            lvl.children
                .push(Node::Element(Element::with_val("w:pStyle", style)));
        }
        if self.legal {
            lvl.children.push(Node::Element(Element::new("w:isLgl")));
        }
        if self.suffix != Suffix::Tab {
            lvl.children.push(Node::Element(Element::with_val(
                "w:suff",
                self.suffix.token(),
            )));
        }
        lvl.children
            .push(Node::Element(Element::with_val("w:lvlText", &self.text)));
        if let Some(picture) = self.picture.as_deref() {
            lvl.children.push(Node::Element(Element::with_val(
                "w:lvlPicBulletId",
                picture,
            )));
        }
        lvl.children
            .push(Node::Element(Element::with_val("w:lvlJc", &self.alignment)));
        lvl.children.push(Node::Element(
            Element::new("w:pPr").child(
                Element::new("w:ind")
                    .attr("w:left", &twips(self.indent))
                    .attr("w:hanging", &twips(self.hanging)),
            ),
        ));
        let mut rpr = Element::new("w:rPr");
        if let Some(font) = self.font.as_deref() {
            rpr.children.push(Node::Element(
                Element::new("w:rFonts")
                    .attr("w:ascii", font)
                    .attr("w:hAnsi", font)
                    .attr("w:hint", "default"),
            ));
        }
        if let Some(language) = self.language.as_deref() {
            rpr.children
                .push(Node::Element(Element::with_val("w:lang", language)));
        }
        if !rpr.children.is_empty() {
            lvl.children.push(Node::Element(rpr));
        }
        lvl
    }

    fn read(lvl: &Element) -> ListLevel {
        let ind = lvl.find("w:pPr").and_then(|ppr| ppr.find("w:ind"));
        let points = |name: &str| {
            ind.and_then(|ind| ind.get(name))
                .and_then(|value| value.parse::<f32>().ok())
                .map(|value| value / 20.0)
        };
        ListLevel {
            format: read_number_format(lvl),
            language: lvl.find("w:rPr").and_then(|rpr| rpr.child_val("w:lang")),
            text: lvl.child_val("w:lvlText").unwrap_or_default(),
            start: lvl
                .child_val("w:start")
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            alignment: lvl.child_val("w:lvlJc").unwrap_or_else(|| "left".into()),
            indent: points("w:left")
                .or_else(|| points("w:start"))
                .unwrap_or(0.0),
            hanging: points("w:hanging")
                .or_else(|| points("w:firstLine").map(|first| -first))
                .unwrap_or(0.0),
            suffix: Suffix::from_token(lvl.child_val("w:suff").as_deref()),
            legal: lvl.find("w:isLgl").is_some_and(|legal| {
                !matches!(legal.get("w:val").as_deref(), Some("0" | "false" | "off"))
            }),
            style: lvl.child_val("w:pStyle"),
            font: lvl
                .find("w:rPr")
                .and_then(|rpr| rpr.find("w:rFonts"))
                .and_then(|fonts| fonts.get("w:ascii").or_else(|| fonts.get("w:hAnsi"))),
            restart_after: lvl
                .child_val("w:lvlRestart")
                .and_then(|value| value.parse().ok()),
            picture: lvl.child_val("w:lvlPicBulletId"),
        }
    }

    /// The level's number as it shows for `values` (each level's counter),
    /// for the dialogs' previews.
    pub fn preview(&self, values: &[u32], formats: &[String]) -> String {
        let languages = vec![self.language.as_deref(); formats.len()];
        self.preview_with_languages(values, formats, &languages)
    }

    fn preview_with_languages(
        &self,
        values: &[u32],
        formats: &[String],
        languages: &[Option<&str>],
    ) -> String {
        if self.format == "none" {
            return String::new();
        }
        if self.is_bullet() && !self.legal {
            return self.text.clone();
        }
        let mut out = String::new();
        let mut chars = self.text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '%' {
                if let Some(digit) = chars.peek().and_then(|digit| digit.to_digit(10)) {
                    chars.next();
                    let index = digit.saturating_sub(1) as usize;
                    let value = values.get(index).copied().unwrap_or(1);
                    // Word's legal numbering includes the current level,
                    // with an exception for placeholders formatted as none.
                    let original = formats.get(index).map(String::as_str).unwrap_or("decimal");
                    let format = if self.legal && original != "none" {
                        "decimal"
                    } else {
                        original
                    };
                    out.push_str(&crate::numbering::format_number_value(
                        format,
                        value,
                        languages.get(index).copied().flatten(),
                    ));
                    continue;
                }
            }
            out.push(ch);
        }
        out
    }
}

// Match the reader's supported custom choice preference over its decimal fallback.
fn read_number_format(level: &Element) -> String {
    fn visit(element: &Element, format: &mut String) {
        for child in element.elements() {
            match child.name.as_str() {
                "w:numFmt" => {
                    if child.get("w:val").as_deref() == Some("custom") {
                        if let Some(custom) = child.get("w:format") {
                            *format = format!("{}{custom}", crate::numbering::CUSTOM_FORMAT_PREFIX);
                        }
                    } else if !format.starts_with(crate::numbering::CUSTOM_FORMAT_PREFIX) {
                        if let Some(value) = child.get("w:val") {
                            *format = value;
                        }
                    }
                }
                "mc:AlternateContent" | "mc:Choice" | "mc:Fallback" => visit(child, format),
                _ => {}
            }
        }
    }
    let mut format = "decimal".to_string();
    visit(level, &mut format);
    format
}

fn twips(points: f32) -> String {
    ((points * 20.0).round() as i64).to_string()
}

/// A list definition: nine levels.
#[derive(Clone, Debug)]
pub struct ListDefinition {
    pub levels: Vec<ListLevel>,
    /// The list style this definition is (`w:styleLink`), for a list style.
    pub style_link: Option<String>,
    // Original level markup for lossless edits through the definition dialog.
    source_levels: Vec<Option<Element>>,
}

impl PartialEq for ListDefinition {
    fn eq(&self, other: &Self) -> bool {
        self.levels == other.levels && self.style_link == other.style_link
    }
}

impl ListDefinition {
    /// Every level from `level`.
    pub fn from_levels(level: impl Fn(u8) -> ListLevel) -> Self {
        ListDefinition {
            levels: (0..9).map(level).collect(),
            style_link: None,
            source_levels: Vec::new(),
        }
    }

    /// The number of each level, using its configured start and language.
    pub fn preview(&self, level: usize) -> String {
        let formats: Vec<String> = self
            .levels
            .iter()
            .map(|level| level.format.clone())
            .collect();
        let values: Vec<u32> = self.levels.iter().map(|level| level.start).collect();
        let languages: Vec<Option<&str>> = self
            .levels
            .iter()
            .map(|level| level.language.as_deref())
            .collect();
        self.levels
            .get(level)
            .map(|lvl| lvl.preview_with_languages(&values, &formats, &languages))
            .unwrap_or_default()
    }

    fn element(&self, abstract_id: u32, nsid: u32) -> Element {
        let mut element =
            Element::new("w:abstractNum").attr("w:abstractNumId", &abstract_id.to_string());
        element.children.push(Node::Element(Element::with_val(
            "w:nsid",
            &format!("{nsid:08X}"),
        )));
        element.children.push(Node::Element(Element::with_val(
            "w:multiLevelType",
            "hybridMultilevel",
        )));
        if let Some(style) = self.style_link.as_deref() {
            element
                .children
                .push(Node::Element(Element::with_val("w:styleLink", style)));
        }
        for (index, level) in self.levels.iter().enumerate().take(9) {
            let saved = match self.source_levels.get(index).and_then(Option::as_ref) {
                Some(source) => {
                    let mut saved = source.clone();
                    saved.set_attr("w:ilvl", &index.to_string());
                    patch_level(&mut saved, index as u8, |value| *value = level.clone());
                    saved
                }
                None => level.element(index as u8),
            };
            element.children.push(Node::Element(saved));
        }
        element
    }

    fn read(abstract_num: &Element) -> ListDefinition {
        let mut levels: Vec<(u8, ListLevel)> = abstract_num
            .elements()
            .filter(|element| element.name == "w:lvl")
            .filter_map(|lvl| {
                let index: u8 = lvl.get("w:ilvl")?.parse().ok()?;
                Some((index, ListLevel::read(lvl)))
            })
            .collect();
        levels.sort_by_key(|(index, _)| *index);
        let levels = (0..9u8)
            .map(|index| {
                levels
                    .iter()
                    .find(|(level, _)| *level == index)
                    .map(|(_, level)| level.clone())
                    .unwrap_or_else(|| {
                        ListLevel::numbered(index, "decimal", &format!("%{}.", index + 1))
                    })
            })
            .collect();
        ListDefinition {
            levels,
            style_link: abstract_num.child_val("w:styleLink"),
            source_levels: (0..9u8)
                .map(|index| {
                    abstract_num
                        .elements()
                        .find(|element| {
                            element.name == "w:lvl"
                                && element
                                    .get("w:ilvl")
                                    .and_then(|value| value.parse::<u8>().ok())
                                    == Some(index)
                        })
                        .cloned()
                })
                .collect(),
        }
    }
}

/// A number format of the 段落番号 library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberFormat {
    pub format: &'static str,
    /// The level text, `%1` for the number.
    pub text: &'static str,
}

/// Word's 段落番号 library, Japanese formats included.
pub const NUMBER_FORMATS: [NumberFormat; 14] = [
    NumberFormat {
        format: "decimal",
        text: "%1.",
    },
    NumberFormat {
        format: "decimal",
        text: "%1)",
    },
    NumberFormat {
        format: "decimal",
        text: "(%1)",
    },
    NumberFormat {
        format: "decimalEnclosedCircle",
        text: "%1",
    },
    NumberFormat {
        format: "decimalFullWidth",
        text: "%1.",
    },
    NumberFormat {
        format: "japaneseCounting",
        text: "%1、",
    },
    NumberFormat {
        format: "japaneseCounting",
        text: "第%1",
    },
    NumberFormat {
        format: "aiueoFullWidth",
        text: "%1.",
    },
    NumberFormat {
        format: "irohaFullWidth",
        text: "%1.",
    },
    NumberFormat {
        format: "upperRoman",
        text: "%1.",
    },
    NumberFormat {
        format: "lowerRoman",
        text: "%1.",
    },
    NumberFormat {
        format: "upperLetter",
        text: "%1.",
    },
    NumberFormat {
        format: "lowerLetter",
        text: "%1)",
    },
    NumberFormat {
        format: "ideographTraditional",
        text: "%1.",
    },
];

/// Word's 箇条書き library: the glyph and its font (`None` for the text's).
pub const BULLETS: [(&str, Option<&str>); 7] = [
    ("●", None),
    ("■", None),
    ("◆", None),
    ("○", None),
    ("✓", None),
    ("➢", None),
    ("•", None),
];

/// The multilevel library (リスト ライブラリ).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultilevelPreset {
    /// 1. / 1.1. / 1.1.1.
    DecimalOutline,
    /// 1) / a) / i)
    Parenthesised,
    /// ● / ○ / ■
    Bullets,
    /// 1 見出し 1 / 1.1 見出し 2 / … linked to the heading styles.
    NumberedHeadings,
    /// 第1章 / 第1節 / 第1項, linked to the heading styles.
    Chapters,
    /// Article I. / Section 1.1 / (a), legal-style and linked to headings.
    Articles,
}

impl MultilevelPreset {
    pub const ALL: [MultilevelPreset; 6] = [
        MultilevelPreset::DecimalOutline,
        MultilevelPreset::Parenthesised,
        MultilevelPreset::Bullets,
        MultilevelPreset::NumberedHeadings,
        MultilevelPreset::Chapters,
        MultilevelPreset::Articles,
    ];

    /// The definition, `heading` naming each level's heading style id.
    pub fn definition(self, heading: &dyn Fn(u8) -> Option<String>) -> ListDefinition {
        match self {
            MultilevelPreset::DecimalOutline => ListDefinition::from_levels(|level| {
                let text: Vec<String> = (1..=level + 1).map(|n| format!("%{n}")).collect();
                ListLevel {
                    hanging: 21.0 + 7.0 * level as f32,
                    indent: 21.0 + 28.0 * level as f32,
                    ..ListLevel::numbered(level, "decimal", &format!("{}.", text.join(".")))
                }
            }),
            MultilevelPreset::Parenthesised => ListDefinition::from_levels(|level| {
                let (format, text) = match level % 3 {
                    0 => ("decimal", "%N)"),
                    1 => ("lowerLetter", "%N)"),
                    _ => ("lowerRoman", "%N)"),
                };
                let text = match level / 3 {
                    0 => text.to_string(),
                    1 => format!("({})", text.trim_end_matches(')')),
                    _ => format!("{}.", text.trim_end_matches(')')),
                };
                ListLevel::numbered(level, format, &text.replace('N', &(level + 1).to_string()))
            }),
            MultilevelPreset::Bullets => ListDefinition::from_levels(|level| {
                ListLevel::bullet(level, ["●", "○", "■"][level as usize % 3])
            }),
            MultilevelPreset::NumberedHeadings => ListDefinition::from_levels(|level| {
                let text: Vec<String> = (1..=level + 1).map(|n| format!("%{n}")).collect();
                ListLevel {
                    indent: 0.0,
                    hanging: 0.0,
                    suffix: Suffix::Space,
                    style: heading(level),
                    ..ListLevel::numbered(level, "decimal", &text.join("."))
                }
            }),
            MultilevelPreset::Chapters => ListDefinition::from_levels(|level| {
                let text = match level {
                    0 => "第%1章".to_string(),
                    1 => "第%2節".to_string(),
                    2 => "第%3項".to_string(),
                    _ => format!("({})", format_args!("%{}", level + 1)),
                };
                ListLevel {
                    indent: 0.0,
                    hanging: 0.0,
                    suffix: Suffix::Space,
                    style: heading(level),
                    ..ListLevel::numbered(level, "decimal", &text)
                }
            }),
            MultilevelPreset::Articles => ListDefinition::from_levels(|level| {
                let (format, text, legal) = match level {
                    0 => ("upperRoman", "Article %1.".to_string(), false),
                    1 => ("decimalZero", "Section %1.%2".to_string(), true),
                    2 => ("lowerLetter", "(%3)".to_string(), false),
                    3 => ("lowerRoman", "(%4)".to_string(), false),
                    _ => ("decimal", format!("%{}.", level + 1), false),
                };
                ListLevel {
                    indent: 0.0 + 36.0 * level.saturating_sub(1) as f32,
                    hanging: 0.0,
                    suffix: Suffix::Space,
                    legal,
                    style: heading(level),
                    ..ListLevel::numbered(level, format, &text)
                }
            }),
        }
    }
}

/// The list the caret's paragraph is in.
#[derive(Clone, Debug, PartialEq)]
pub struct ListState {
    pub num_id: String,
    pub abstract_id: String,
    /// 0-based.
    pub level: u8,
    pub definition: ListDefinition,
    /// The value the paragraph's level starts at in this list
    /// (`w:startOverride`, else the level's start).
    pub start: u32,
}

/// What a `numbering.xml` defines.
#[derive(Clone, Debug, Default)]
pub struct Numbering {
    root: Option<Element>,
}

impl Numbering {
    pub fn parse(xml: &str) -> Self {
        Numbering {
            root: Document::parse(xml).ok().map(|document| document.root),
        }
    }

    fn num(&self, num_id: &str) -> Option<&Element> {
        self.root.as_ref()?.elements().find(|element| {
            element.name == "w:num" && element.get("w:numId").as_deref() == Some(num_id)
        })
    }

    pub fn abstract_id(&self, num_id: &str) -> Option<String> {
        self.num(num_id)?.child_val("w:abstractNumId")
    }

    fn abstract_num(&self, abstract_id: &str) -> Option<&Element> {
        self.root.as_ref()?.elements().find(|element| {
            element.name == "w:abstractNum"
                && element.get("w:abstractNumId").as_deref() == Some(abstract_id)
        })
    }

    pub fn definition(&self, num_id: &str) -> Option<ListDefinition> {
        let abstract_num = self.abstract_num(&self.abstract_id(num_id)?)?;
        // A definition that is a list style's reference (numStyleLink) takes
        // the style's own definition.
        if let Some(style) = abstract_num.child_val("w:numStyleLink") {
            if let Some(linked) = self.root.as_ref()?.elements().find(|element| {
                element.name == "w:abstractNum"
                    && element.child_val("w:styleLink").as_deref() == Some(style.as_str())
            }) {
                return Some(self.apply_level_overrides(num_id, ListDefinition::read(linked)));
            }
        }
        Some(self.apply_level_overrides(num_id, ListDefinition::read(abstract_num)))
    }

    fn apply_level_overrides(
        &self,
        num_id: &str,
        mut definition: ListDefinition,
    ) -> ListDefinition {
        if let Some(num) = self.num(num_id) {
            for replacement in num
                .elements()
                .filter(|element| element.name == "w:lvlOverride")
            {
                let index = replacement
                    .get("w:ilvl")
                    .and_then(|value| value.parse::<usize>().ok());
                if let (Some(index), Some(level)) = (index, replacement.find("w:lvl")) {
                    if let Some(target) = definition.levels.get_mut(index) {
                        let start = target.start;
                        let restart_after = target.restart_after;
                        *target = ListLevel::read(level);
                        target.start = start;
                        target.restart_after = restart_after;
                        if let Some(source) = definition.source_levels.get_mut(index) {
                            *source = Some(level.clone());
                        }
                    }
                }
            }
        }
        definition
    }

    pub fn start_override(&self, num_id: &str, level: u8) -> Option<u32> {
        self.num(num_id)?
            .elements()
            .filter(|element| element.name == "w:lvlOverride")
            .find(|element| element.get("w:ilvl").as_deref() == Some(&level.to_string()))
            .and_then(|element| element.child_val("w:startOverride"))
            .and_then(|value| value.parse().ok())
    }
}

/// Adds a picture bullet of the image at `target` (a part name, related from
/// `numbering_part` as `relationship`) to `numbering`, returning its
/// `w:numPicBulletId`. Written as Word writes one: a VML shape whose
/// `v:imagedata` names the image.
pub fn add_picture_bullet(numbering: &mut Element, relationship: &str, size_pt: f32) -> String {
    for (prefix, uri) in [
        ("xmlns:v", "urn:schemas-microsoft-com:vml"),
        ("xmlns:o", "urn:schemas-microsoft-com:office:office"),
        (
            "xmlns:r",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
        ),
    ] {
        if numbering.get(prefix).is_none() {
            numbering.set_attr(prefix, uri);
        }
    }
    let id = numbering
        .elements()
        .filter(|element| element.name == "w:numPicBullet")
        .filter_map(|element| element.get("w:numPicBulletId")?.parse::<u32>().ok())
        .max()
        .map_or(0, |max| max + 1)
        .to_string();
    let size = format!("width:{size_pt}pt;height:{size_pt}pt");
    let bullet = Element::new("w:numPicBullet")
        .attr("w:numPicBulletId", &id)
        .child(
            Element::new("w:pict").child(
                Element::new("v:shape")
                    .attr(
                        "id",
                        &format!("_x0000_i10{}", 25 + id.parse::<u32>().unwrap_or(0)),
                    )
                    .attr("type", "#_x0000_t75")
                    .attr("style", &size)
                    .attr("o:bullet", "t")
                    .child(
                        Element::new("v:imagedata")
                            .attr("r:id", relationship)
                            .attr("o:title", ""),
                    ),
            ),
        );
    numbering.insert_ordered(bullet, NUMBERING_ORDER);
    id
}

/// The `w:numId` of each list style (`w:style w:type="numbering"`) of a
/// `styles.xml`, by its id and name.
pub fn list_styles(styles_xml: &str) -> Vec<(String, String, String)> {
    let Ok(document) = Document::parse(styles_xml) else {
        return Vec::new();
    };
    document
        .root
        .elements()
        .filter(|element| {
            element.name == "w:style" && element.get("w:type").as_deref() == Some("numbering")
        })
        .filter_map(|style| {
            let id = style.get("w:styleId")?;
            let name = style.child_val("w:name").unwrap_or_else(|| id.clone());
            let num_id = style.find("w:pPr")?.find("w:numPr")?.child_val("w:numId")?;
            Some((id, name, num_id))
        })
        .collect()
}

/// `CT_Numbering`'s children.
const NUMBERING_ORDER: &[&str] = &[
    "w:numPicBullet",
    "w:abstractNum",
    "w:num",
    "w:numIdMacAtCleanup",
];

/// Adds `definition` and a list instance of it to `numbering`, returning the
/// instance's `w:numId`.
pub fn add_list(numbering: &mut Element, definition: &ListDefinition) -> String {
    let abstract_id = numbering
        .elements()
        .filter(|element| element.name == "w:abstractNum")
        .filter_map(|element| element.get("w:abstractNumId")?.parse::<u32>().ok())
        .max()
        .map_or(0, |max| max + 1);
    let nsids: Vec<String> = numbering
        .elements()
        .filter_map(|element| element.child_val("w:nsid"))
        .collect();
    // A fresh nsid tells Word this is a list of its own, not one to merge
    // with on paste.
    let mut nsid = 0x4A41_4445u32 ^ (abstract_id.wrapping_mul(0x9E37_79B9));
    while nsids.contains(&format!("{nsid:08X}")) {
        nsid = nsid.wrapping_add(0x0101_0101);
    }
    numbering.insert_ordered(definition.element(abstract_id, nsid), NUMBERING_ORDER);
    add_instance(numbering, &abstract_id.to_string(), None)
}

/// Adds a list instance of `abstract_id`, with the level's start overridden
/// when `start` names one; returns its `w:numId`.
pub fn add_instance(
    numbering: &mut Element,
    abstract_id: &str,
    start: Option<(u8, u32)>,
) -> String {
    let num_id = numbering
        .elements()
        .filter(|element| element.name == "w:num")
        .filter_map(|element| element.get("w:numId")?.parse::<u32>().ok())
        .max()
        .map_or(1, |max| max + 1)
        .to_string();
    let mut num = Element::new("w:num")
        .attr("w:numId", &num_id)
        .child(Element::with_val("w:abstractNumId", abstract_id));
    if let Some((level, value)) = start {
        num.children.push(Node::Element(
            Element::new("w:lvlOverride")
                .attr("w:ilvl", &level.to_string())
                .child(Element::with_val("w:startOverride", &value.to_string())),
        ));
    }
    numbering.insert_ordered(num, NUMBERING_ORDER);
    num_id
}

/// Creates a restarted instance of an existing list.
fn restart_instance(
    numbering: &mut Element,
    source_id: &str,
    level: u8,
    value: u32,
) -> Result<String, String> {
    let source = numbering
        .elements()
        .find(|element| {
            element.name == "w:num" && element.get("w:numId").as_deref() == Some(source_id)
        })
        .ok_or_else(|| format!("no list instance {source_id}"))?;
    let abstract_id = source
        .child_val("w:abstractNumId")
        .ok_or("list has no abstract definition")?;
    let overrides: Vec<Element> = source
        .elements()
        .filter(|element| element.name == "w:lvlOverride")
        .cloned()
        .collect();
    let id = add_instance(numbering, &abstract_id, None);
    let target = numbering
        .elements_mut()
        .find(|element| {
            element.name == "w:num" && element.get("w:numId").as_deref() == Some(id.as_str())
        })
        .expect("new list instance");
    target
        .children
        .extend(overrides.into_iter().map(Node::Element));
    let ilvl = level.to_string();
    let index = target.children.iter().position(|node| {
        matches!(node, Node::Element(element) if element.name == "w:lvlOverride" && element.get("w:ilvl").as_deref() == Some(ilvl.as_str()))
    });
    let index = index.unwrap_or_else(|| {
        target.children.push(Node::Element(
            Element::new("w:lvlOverride").attr("w:ilvl", &ilvl),
        ));
        target.children.len() - 1
    });
    if let Node::Element(replacement) = &mut target.children[index] {
        replacement.put(
            Element::with_val("w:startOverride", &value.to_string()),
            &["w:startOverride", "w:lvl"],
        );
    }
    Ok(id)
}

/// Rewrites level `level` of `abstract_id` in `numbering` with `edit`.
pub fn edit_level(
    numbering: &mut Element,
    abstract_id: &str,
    level: u8,
    edit: impl FnOnce(&mut ListLevel),
) -> Result<(), String> {
    let abstract_num = numbering
        .elements_mut()
        .find(|element| {
            element.name == "w:abstractNum"
                && element.get("w:abstractNumId").as_deref() == Some(abstract_id)
        })
        .ok_or_else(|| format!("no list definition {abstract_id}"))?;
    let ilvl = level.to_string();
    let index = abstract_num.children.iter().position(|node| {
        matches!(node, Node::Element(element) if element.name == "w:lvl" && element.get("w:ilvl").as_deref() == Some(ilvl.as_str()))
    });
    match index {
        Some(index) => {
            if let Node::Element(element) = &mut abstract_num.children[index] {
                patch_level(element, level, edit);
            }
        }
        None => {
            let mut current = ListLevel::numbered(level, "decimal", &format!("%{}.", level + 1));
            edit(&mut current);
            abstract_num
                .children
                .push(Node::Element(current.element(level)));
        }
    }
    Ok(())
}

const LEVEL_ORDER: &[&str] = &[
    "w:start",
    "w:numFmt",
    "w:lvlRestart",
    "w:pStyle",
    "w:isLgl",
    "w:suff",
    "w:lvlText",
    "w:lvlPicBulletId",
    "w:legacy",
    "w:lvlJc",
    "w:pPr",
    "w:rPr",
];

/// Patch only fields the caller changed, retaining the rest of Word's level XML.
fn patch_level(lvl: &mut Element, level: u8, edit: impl FnOnce(&mut ListLevel)) {
    let before = ListLevel::read(lvl);
    let mut after = before.clone();
    edit(&mut after);
    if before == after {
        return;
    }
    let generated = after.element(level);
    for (name, changed) in [
        ("w:start", before.start != after.start),
        ("w:lvlRestart", before.restart_after != after.restart_after),
        ("w:pStyle", before.style != after.style),
        ("w:isLgl", before.legal != after.legal),
        ("w:suff", before.suffix != after.suffix),
        ("w:lvlText", before.text != after.text),
        ("w:lvlPicBulletId", before.picture != after.picture),
        ("w:lvlJc", before.alignment != after.alignment),
    ] {
        if changed {
            match generated.find(name) {
                Some(element) => lvl.put(element.clone(), LEVEL_ORDER),
                None => lvl.remove(name),
            }
        }
    }
    if before.format != after.format {
        // A custom Choice may have a decimal Fallback. Update both without
        // discarding other compatibility markup in the level.
        fn replace_formats(element: &mut Element, format: &Element, fallback: bool) -> bool {
            let mut found = false;
            for child in element.elements_mut() {
                match child.name.as_str() {
                    "w:numFmt" => {
                        *child = if fallback && format.get("w:val").as_deref() == Some("custom") {
                            Element::with_val("w:numFmt", "decimal")
                        } else {
                            format.clone()
                        };
                        found = true;
                    }
                    "mc:AlternateContent" | "mc:Choice" | "mc:Fallback" => {
                        let fallback = fallback || child.name == "mc:Fallback";
                        found |= replace_formats(child, format, fallback);
                    }
                    _ => {}
                }
            }
            found
        }
        let format = generated.find("w:numFmt").expect("generated format");
        if !replace_formats(lvl, format, false) {
            lvl.put(format.clone(), LEVEL_ORDER);
        }
    }
    if before.indent != after.indent || before.hanging != after.hanging {
        let ind = lvl.ensure("w:pPr", LEVEL_ORDER).ensure("w:ind", PPR_ORDER);
        if before.indent != after.indent {
            for name in ["w:start", "w:startChars", "w:leftChars"] {
                ind.remove_attr(name);
            }
            ind.set_attr("w:left", &twips(after.indent));
        }
        if before.hanging != after.hanging {
            for name in [
                "w:hanging",
                "w:firstLine",
                "w:hangingChars",
                "w:firstLineChars",
            ] {
                ind.remove_attr(name);
            }
            let (name, value) = if after.hanging < 0.0 {
                ("w:firstLine", -after.hanging)
            } else {
                ("w:hanging", after.hanging)
            };
            ind.set_attr(name, &twips(value));
        }
    }
    if before.font != after.font {
        let rpr = lvl.ensure("w:rPr", LEVEL_ORDER);
        let fonts = rpr.ensure("w:rFonts", super::styles::RPR_ORDER);
        for name in ["w:ascii", "w:hAnsi", "w:asciiTheme", "w:hAnsiTheme"] {
            fonts.remove_attr(name);
        }
        if let Some(font) = after.font.as_deref() {
            fonts.set_attr("w:ascii", font);
            fonts.set_attr("w:hAnsi", font);
        }
        if fonts.attributes.is_empty() && fonts.children.is_empty() {
            rpr.remove("w:rFonts");
        }
    }
    if before.language != after.language {
        let rpr = lvl.ensure("w:rPr", LEVEL_ORDER);
        let lang = rpr.ensure("w:lang", super::styles::RPR_ORDER);
        match after.language.as_deref() {
            Some(language) => lang.set_attr("w:val", language),
            None => lang.remove_attr("w:val"),
        }
        if lang.attributes.is_empty() && lang.children.is_empty() {
            rpr.remove("w:lang");
        }
    }
}

/// Edits the effective level of this list, preserving an existing instance
/// override rather than changing the shared abstract definition underneath it.
fn edit_instance_level(
    numbering: &mut Element,
    num_id: &str,
    abstract_id: &str,
    level: u8,
    edit: impl FnOnce(&mut ListLevel),
) -> Result<(), String> {
    let ilvl = level.to_string();
    if let Some(lvl) = numbering
        .elements_mut()
        .find(|element| {
            element.name == "w:num" && element.get("w:numId").as_deref() == Some(num_id)
        })
        .and_then(|num| {
            num.elements_mut().find(|element| {
                element.name == "w:lvlOverride"
                    && element.get("w:ilvl").as_deref() == Some(ilvl.as_str())
            })
        })
        .and_then(|replacement| replacement.find_mut("w:lvl"))
    {
        patch_level(lvl, level, edit);
        return Ok(());
    }
    // A numStyleLink refers to the style's actual definition, not to the
    // otherwise empty abstract definition carrying the link.
    let linked_style = numbering
        .elements()
        .find(|element| {
            element.name == "w:abstractNum"
                && element.get("w:abstractNumId").as_deref() == Some(abstract_id)
        })
        .and_then(|element| element.child_val("w:numStyleLink"));
    let effective_id = linked_style
        .and_then(|style| {
            numbering
                .elements()
                .find(|element| {
                    element.name == "w:abstractNum"
                        && element.child_val("w:styleLink").as_deref() == Some(style.as_str())
                })
                .and_then(|element| element.get("w:abstractNumId"))
        })
        .unwrap_or_else(|| abstract_id.to_string());
    edit_level(numbering, &effective_id, level, edit)
}

/// Links each level's heading style to the list: `w:numPr` in the style, as
/// Word writes it, so a paragraph in the style is numbered.
pub fn link_styles(styles: &mut Element, num_id: &str, definition: &ListDefinition) {
    for (index, level) in definition.levels.iter().enumerate() {
        let Some(style_id) = level.style.as_deref() else {
            continue;
        };
        let Some(style) = styles.elements_mut().find(|element| {
            element.name == "w:style" && element.get("w:styleId").as_deref() == Some(style_id)
        }) else {
            continue;
        };
        let ppr = style.ensure("w:pPr", STYLE_ORDER);
        let mut num_pr = Element::new("w:numPr");
        if index > 0 {
            num_pr.children.push(Node::Element(Element::with_val(
                "w:ilvl",
                &index.to_string(),
            )));
        }
        num_pr
            .children
            .push(Node::Element(Element::with_val("w:numId", num_id)));
        ppr.put(num_pr, PPR_ORDER);
    }
}

/// The list commands, for every session.
pub trait WordLists: WordSession {
    /// The document's list definitions.
    fn numbering(&self) -> Numbering {
        self.word_document()
            .related_part_xml("numbering")
            .map(|xml| Numbering::parse(&xml))
            .unwrap_or_default()
    }

    /// The list of the paragraph at the caret, if it is in one.
    fn list_state(&self) -> Option<ListState> {
        let paragraph = list_span(self).0;
        let numbering = self
            .word_document()
            .paragraph(paragraph)?
            .properties
            .as_ref()?
            .numbering
            .clone()?;
        let num_id = numbering.id?;
        if num_id == "0" {
            return None;
        }
        let level = numbering.level.unwrap_or(0).min(8) as u8;
        let part = self.numbering();
        let definition = part.definition(&num_id)?;
        let start = part
            .start_override(&num_id, level)
            .unwrap_or(definition.levels[level as usize].start);
        Some(ListState {
            abstract_id: part.abstract_id(&num_id)?,
            num_id,
            level,
            definition,
            start,
        })
    }

    /// The paragraphs, from `from`, that are in list `num_id`, up to the
    /// document's end — "the rest of this list".
    fn rest_of_list(&self, from: usize, num_id: &str) -> Vec<usize> {
        let document = self.word_document();
        (from..document.paragraph_count())
            .filter(|&index| {
                document
                    .paragraph(index)
                    .and_then(|paragraph| paragraph.properties.as_ref())
                    .and_then(|props| props.numbering.as_ref())
                    .and_then(|numbering| numbering.id.as_deref())
                    == Some(num_id)
            })
            .collect()
    }

    /// 段落番号 / 箇条書き from the library: the selection becomes a list in
    /// that format, or — already in a list — the list's level takes it.
    fn apply_list_format(
        &mut self,
        format: &str,
        text: &str,
        font: Option<&str>,
    ) -> Result<(), String> {
        let format = format.to_string();
        let text = text.to_string();
        let font = font.map(str::to_string);
        if let Some(state) = self.list_state() {
            let abstract_id = state.abstract_id.clone();
            let num_id = state.num_id.clone();
            let level = state.level;
            return edit_package(
                self,
                move |package| {
                    package.edit(PartKind::Numbering, |numbering| {
                        edit_instance_level(numbering, &num_id, &abstract_id, level, |lvl| {
                            lvl.format = format;
                            lvl.text = text;
                            lvl.font = font;
                        })
                    })
                },
                Vec::new(),
            );
        }
        let definition = ListDefinition::from_levels(|level| {
            if level == 0 {
                ListLevel {
                    font: font.clone(),
                    ..ListLevel::numbered(0, &format, &text)
                }
            } else if format == "bullet" {
                ListLevel::bullet(level, ["○", "■", "●"][level as usize % 3])
            } else {
                ListLevel::numbered(level, "decimal", &format!("%{}.", level + 1))
            }
        });
        self.apply_list_definition(&definition, Some(0))
    }

    /// 箇条書き → 図: the selection becomes a list whose bullet is the
    /// picture `image` (PNG, JPEG or GIF bytes, `extension` naming which).
    fn apply_picture_bullet(&mut self, image: &[u8], extension: &str) -> Result<(), String> {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        let content_type = match extension.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            _ => return Err(format!("a picture bullet cannot be a .{extension} file")),
        };
        let (start, end) = list_span(self);
        let num_id = next_num_id(&self.numbering());
        let then = vec![HeadlessStep::boxed(vec![HeadlessCommand::SetNumbering {
            start_paragraph: start,
            end_paragraph: end,
            num_id: Some(num_id.clone()),
            level: Some(0),
        }])];
        let image = image.to_vec();
        edit_package(
            self,
            move |package| {
                let numbering_part = package.part(PartKind::Numbering)?;
                let directory = numbering_part
                    .rsplit_once('/')
                    .map(|(directory, _)| directory.to_string())
                    .unwrap_or_default();
                let media = (1..)
                    .map(|n| format!("{directory}/media/bullet{n}.{extension}"))
                    .find(|name| !package.has(name))
                    .expect("a free name");
                package.put_bytes(&media, image);
                package.declare_default(&extension, content_type)?;
                let relationship = package.relate(&numbering_part, "image", &media, false)?;
                package.edit(PartKind::Numbering, |numbering| {
                    let picture = add_picture_bullet(numbering, &relationship, 9.0);
                    let definition = ListDefinition::from_levels(|level| ListLevel {
                        picture: (level == 0).then(|| picture.clone()),
                        font: (level == 0).then(|| "Symbol".to_string()),
                        ..ListLevel::bullet(
                            level,
                            if level == 0 {
                                "\u{F0B7}"
                            } else {
                                ["○", "■", "●"][level as usize % 3]
                            },
                        )
                    });
                    let made = add_list(numbering, &definition);
                    debug_assert_eq!(made, num_id);
                    Ok(())
                })
            },
            then,
        )
    }

    /// Makes the selection a new list of `definition`, each paragraph at
    /// `level` (or the level it is at, when `None`).
    fn apply_list_definition(
        &mut self,
        definition: &ListDefinition,
        level: Option<u8>,
    ) -> Result<(), String> {
        let (start, end) = list_span(self);
        let levels: Vec<u8> = (start..=end)
            .map(|index| {
                level.unwrap_or_else(|| {
                    self.word_document()
                        .paragraph(index)
                        .and_then(|paragraph| paragraph.properties.as_ref())
                        .and_then(|props| props.numbering.as_ref())
                        .and_then(|numbering| numbering.level)
                        .unwrap_or(0)
                        .min(8) as u8
                })
            })
            .collect();
        let num_id = next_num_id(&self.numbering());
        let then = vec![HeadlessStep::boxed(
            runs(start, &levels)
                .into_iter()
                .map(|(from, to, level)| HeadlessCommand::SetNumbering {
                    start_paragraph: from,
                    end_paragraph: to,
                    num_id: Some(num_id.clone()),
                    level: Some(level as u32),
                })
                .collect(),
        )];
        let definition = definition.clone();
        let linked = definition.levels.iter().any(|level| level.style.is_some());
        edit_package(
            self,
            move |package| {
                let mut made = String::new();
                package.edit(PartKind::Numbering, |numbering| {
                    made = add_list(numbering, &definition);
                    Ok(())
                })?;
                debug_assert_eq!(made, num_id);
                if linked {
                    package.edit(PartKind::Styles, |styles| {
                        link_styles(styles, &made, &definition);
                        Ok(())
                    })?;
                }
                Ok(())
            },
            then,
        )
    }

    /// A multilevel list from the library.
    fn apply_multilevel_preset(&mut self, preset: MultilevelPreset) -> Result<(), String> {
        let headings = heading_ids(self);
        let definition =
            preset.definition(&|level| headings.get(level as usize).cloned().flatten());
        let level = (!definition.levels.iter().any(|level| level.style.is_some())).then_some(0);
        // A heading-linked list numbers the heading paragraphs through their
        // styles; the selection is put in at its own level.
        self.apply_list_definition(&definition, level.or(None))
    }

    /// 番号の設定 / 新しいリストを開始: the caret's item, and the rest of its
    /// list, start again at `value`.
    fn set_list_start(&mut self, value: u32) -> Result<(), String> {
        if value == 0 || value > 32_767 {
            return Err("the value must be between 1 and 32767".to_string());
        }
        let state = self.list_state().ok_or("the caret is not in a list")?;
        let from = list_span(self).0;
        let paragraphs = self.rest_of_list(from, &state.num_id);
        let num_id = next_num_id(&self.numbering());
        let levels: Vec<(usize, u8)> = paragraphs
            .iter()
            .map(|&index| (index, paragraph_level(self, index)))
            .collect();
        let then = vec![HeadlessStep::boxed(
            levels
                .iter()
                .map(|&(index, level)| HeadlessCommand::SetNumbering {
                    start_paragraph: index,
                    end_paragraph: index,
                    num_id: Some(num_id.clone()),
                    level: Some(level as u32),
                })
                .collect(),
        )];
        let source_id = state.num_id.clone();
        let level = state.level;
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Numbering, |numbering| {
                    let made = restart_instance(numbering, &source_id, level, value)?;
                    debug_assert_eq!(made, num_id);
                    Ok(())
                })
            },
            then,
        )
    }

    /// 前のリストから継続: the caret's item, and the rest of its list, carry
    /// on the numbering of the list before it with the same definition (or,
    /// failing that, the nearest list before it).
    fn continue_list(&mut self) -> Result<(), String> {
        let state = self.list_state().ok_or("the caret is not in a list")?;
        let from = list_span(self).0;
        let numbering = self.numbering();
        let document = self.word_document();
        let mut previous: Option<String> = None;
        let mut nearest: Option<String> = None;
        for index in (0..from).rev() {
            let Some(id) = document
                .paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.numbering.as_ref())
                .and_then(|numbering| numbering.id.clone())
            else {
                continue;
            };
            if id == state.num_id || id == "0" {
                continue;
            }
            if nearest.is_none() {
                nearest = Some(id.clone());
            }
            if numbering.abstract_id(&id).as_deref() == Some(state.abstract_id.as_str())
                || numbering
                    .definition(&id)
                    .map(|definition| definition.levels[0].format.clone())
                    == Some(state.definition.levels[0].format.clone())
            {
                previous = Some(id);
                break;
            }
        }
        let target = previous
            .or(nearest)
            .ok_or("there is no list before this one")?;
        let commands: Vec<HeadlessCommand> = self
            .rest_of_list(from, &state.num_id)
            .into_iter()
            .map(|index| HeadlessCommand::SetNumbering {
                start_paragraph: index,
                end_paragraph: index,
                num_id: Some(target.clone()),
                level: Some(paragraph_level(self, index) as u32),
            })
            .collect();
        self.apply_commands(commands, true)
    }

    /// リストのインデントの調整 and the level settings of the define dialog:
    /// rewrites the caret's level of its list.
    fn edit_list_level(
        &mut self,
        edit: impl FnOnce(&mut ListLevel) + 'static,
    ) -> Result<(), String> {
        let state = self.list_state().ok_or("the caret is not in a list")?;
        let abstract_id = state.abstract_id.clone();
        let num_id = state.num_id.clone();
        let level = state.level;
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Numbering, |numbering| {
                    edit_instance_level(numbering, &num_id, &abstract_id, level, edit)
                })
            },
            Vec::new(),
        )
    }

    /// Tab / ⇧Tab at an item's start and リストのレベルの変更: the selected
    /// items go one level down (`delta` 1) or up (-1), or to `level`.
    fn change_list_level(&mut self, delta: i32) -> Result<(), String> {
        let (start, end) = list_span(self);
        let mut commands = Vec::new();
        for index in start..=end {
            let Some(numbering) = self
                .word_document()
                .paragraph(index)
                .and_then(|paragraph| paragraph.properties.as_ref())
                .and_then(|props| props.numbering.clone())
            else {
                continue;
            };
            let Some(num_id) = numbering.id.filter(|id| id != "0") else {
                continue;
            };
            let level = (numbering.level.unwrap_or(0) as i32 + delta).clamp(0, 8) as u32;
            commands.push(HeadlessCommand::SetNumbering {
                start_paragraph: index,
                end_paragraph: index,
                num_id: Some(num_id),
                level: Some(level),
            });
        }
        if commands.is_empty() {
            return Err("the selection is not in a list".to_string());
        }
        self.apply_commands(commands, true)
    }

    fn set_list_level(&mut self, level: u8) -> Result<(), String> {
        let state = self.list_state().ok_or("the caret is not in a list")?;
        self.change_list_level(level.min(8) as i32 - state.level as i32)
    }

    /// 新しいリスト スタイルの定義: a list style named `name`, its definition
    /// `definition`, applied to the selection. Returns the style's id.
    fn create_list_style(
        &mut self,
        name: &str,
        definition: &ListDefinition,
    ) -> Result<String, String> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("a list style needs a name".to_string());
        }
        let styles_xml = self
            .word_document()
            .related_part_xml("styles")
            .unwrap_or_default();
        let catalog = super::styles::catalog_of(&styles_xml);
        if catalog
            .iter()
            .any(|style| style.name.eq_ignore_ascii_case(&name))
        {
            return Err(format!("a style named {name} already exists"));
        }
        let mut id: String = name.chars().filter(|ch| ch.is_alphanumeric()).collect();
        if id.is_empty() {
            id = "ListStyle".to_string();
        }
        while catalog
            .iter()
            .any(|style| style.id.eq_ignore_ascii_case(&id))
        {
            id.push('1');
        }
        let num_id = next_num_id(&self.numbering());
        let (start, end) = list_span(self);
        let then = vec![HeadlessStep::boxed(vec![HeadlessCommand::SetNumbering {
            start_paragraph: start,
            end_paragraph: end,
            num_id: Some(num_id.clone()),
            level: Some(0),
        }])];
        let mut definition = definition.clone();
        definition.style_link = Some(id.clone());
        let style_id = id.clone();
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Numbering, |numbering| {
                    let made = add_list(numbering, &definition);
                    debug_assert_eq!(made, num_id);
                    Ok(())
                })?;
                package.edit(PartKind::Styles, |styles| {
                    let mut style = Element::new("w:style")
                        .attr("w:type", "numbering")
                        .attr("w:customStyle", "1")
                        .attr("w:styleId", &style_id);
                    style.put(Element::with_val("w:name", &name), STYLE_ORDER);
                    style.put(
                        Element::new("w:pPr").child(
                            Element::new("w:numPr").child(Element::with_val("w:numId", &num_id)),
                        ),
                        STYLE_ORDER,
                    );
                    styles.children.push(Node::Element(style));
                    Ok(())
                })
            },
            then,
        )?;
        Ok(id)
    }

    /// Applies a list style: the selection joins its list.
    fn apply_list_style(&mut self, num_id: &str) -> Result<(), String> {
        let (start, end) = list_span(self);
        self.apply_command(
            HeadlessCommand::SetNumbering {
                start_paragraph: start,
                end_paragraph: end,
                num_id: Some(num_id.to_string()),
                level: Some(0),
            },
            true,
        )
    }
}

impl<T: WordSession + ?Sized> WordLists for T {}

/// The paragraphs the selection touches, or the caret's.
fn list_span<S: WordSession + ?Sized>(session: &S) -> (usize, usize) {
    match session.selection_range() {
        Some(range) if !range.is_collapsed() => {
            let (start, end) = range.normalized();
            (start.paragraph, end.paragraph)
        }
        Some(range) => (range.focus.paragraph, range.focus.paragraph),
        None => (0, 0),
    }
}

fn paragraph_level<S: WordSession + ?Sized>(session: &S, index: usize) -> u8 {
    session
        .word_document()
        .paragraph(index)
        .and_then(|paragraph| paragraph.properties.as_ref())
        .and_then(|props| props.numbering.as_ref())
        .and_then(|numbering| numbering.level)
        .unwrap_or(0)
        .min(8) as u8
}

/// The `w:numId` the next list added gets (see [`add_instance`]).
fn next_num_id(numbering: &Numbering) -> String {
    numbering
        .root
        .as_ref()
        .map(|root| {
            root.elements()
                .filter(|element| element.name == "w:num")
                .filter_map(|element| element.get("w:numId")?.parse::<u32>().ok())
                .max()
                .map_or(1, |max| max + 1)
        })
        .unwrap_or(1)
        .to_string()
}

/// Consecutive paragraphs at one level, as `(from, to, level)`.
fn runs(start: usize, levels: &[u8]) -> Vec<(usize, usize, u8)> {
    let mut out: Vec<(usize, usize, u8)> = Vec::new();
    for (offset, &level) in levels.iter().enumerate() {
        let index = start + offset;
        match out.last_mut() {
            Some((_, to, current)) if *current == level && *to + 1 == index => *to = index,
            _ => out.push((index, index, level)),
        }
    }
    out
}

/// The ids of the heading 1–9 styles, as the document names them.
fn heading_ids<S: WordSession + ?Sized>(session: &S) -> Vec<Option<String>> {
    let catalog = session
        .word_document()
        .related_part_xml("styles")
        .map(|xml| super::styles::catalog_of(&xml))
        .unwrap_or_default();
    (1..=9)
        .map(|level| {
            let name = format!("heading {level}");
            catalog
                .iter()
                .find(|style| style.name.eq_ignore_ascii_case(&name))
                .map(|style| style.id.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_read_like_word() {
        let chapters = MultilevelPreset::Chapters.definition(&|_| None);
        assert_eq!(chapters.preview(0), "第1章");
        let articles = MultilevelPreset::Articles.definition(&|_| None);
        assert_eq!(articles.preview(0), "Article I.");
        // Legal: both the parent and current placeholder use decimal.
        assert_eq!(articles.preview(1), "Section 1.1");
        let mut legal = ListLevel::numbered(1, "decimalZero", "%1.%2");
        let formats = ["upperRoman".into(), "decimalZero".into()];
        assert_eq!(legal.preview(&[3, 1], &formats), "III.01");
        legal.legal = true;
        assert_eq!(legal.preview(&[3, 1], &formats), "3.1");
        assert_eq!(legal.preview(&[0, 1], &formats), "0.1");
        assert_eq!(
            legal.preview(&[3, 1], &["none".into(), "decimalZero".into()]),
            ".1"
        );
        legal.format = "none".into();
        assert_eq!(legal.preview(&[3, 1], &formats), "");
        let outline = MultilevelPreset::DecimalOutline.definition(&|_| None);
        assert_eq!(outline.preview(2), "1.1.1.");
        let circle = ListLevel::numbered(0, "decimalEnclosedCircle", "%1");
        assert_eq!(circle.preview(&[3], &["decimalEnclosedCircle".into()]), "③");
        let kanji = ListLevel::numbered(0, "japaneseCounting", "%1、");
        assert_eq!(kanji.preview(&[2], &["japaneseCounting".into()]), "二、");
    }

    #[test]
    fn definitions_round_trip_through_numbering_xml() {
        let mut root = Document::parse(r#"<w:numbering xmlns:w="urn:w"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="3"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#).unwrap().root;
        let mut definition =
            MultilevelPreset::Articles.definition(&|level| Some(format!("Heading{}", level + 1)));
        definition.levels[2].suffix = Suffix::Nothing;
        definition.levels[2].alignment = "right".into();
        let num_id = add_list(&mut root, &definition);
        assert_eq!(num_id, "4");
        let restarted = add_instance(&mut root, "1", Some((1, 5)));
        let numbering = Numbering {
            root: Some(root.clone()),
        };
        assert_eq!(numbering.definition("4").unwrap(), definition);
        assert_eq!(numbering.start_override(&restarted, 1), Some(5));
        // Every abstractNum comes before the first num.
        let names: Vec<&str> = root.elements().map(|e| e.name.as_str()).collect();
        let last_abstract = names
            .iter()
            .rposition(|name| *name == "w:abstractNum")
            .unwrap();
        let first_num = names.iter().position(|name| *name == "w:num").unwrap();
        assert!(last_abstract < first_num, "{names:?}");
    }
}

#[cfg(test)]
mod concrete_level_tests {
    use super::Numbering;

    #[test]
    fn list_dialog_reads_concrete_level_without_mutating_base_definition() {
        let numbering = Numbering::parse(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="8"/><w:lvl w:ilvl="0"><w:start w:val="4"/><w:lvlRestart w:val="1"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="Article %1)"/><w:lvlJc w:val="right"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#,
        );
        let replacement = &numbering.definition("2").unwrap().levels[0];
        assert_eq!(replacement.format, "upperRoman");
        assert_eq!(replacement.text, "Article %1)");
        assert_eq!(replacement.start, 1);
        assert_eq!(replacement.restart_after, None);
        assert_eq!(replacement.alignment, "right");
        assert_eq!(replacement.indent, 36.0);
        assert_eq!(replacement.hanging, 18.0);
        assert_eq!(numbering.start_override("2", 0), Some(8));
        let original = &numbering.definition("1").unwrap().levels[0];
        assert_eq!(original.format, "decimal");
        assert_eq!(original.start, 1);
    }
}

#[cfg(test)]
mod numbering_format_round_trip_tests {
    use super::*;

    #[test]
    fn language_controls_preview_and_survives_list_edits() {
        let xml = r#"<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="cardinalText"/><w:lvlText w:val="%1"/><w:rPr><w:rFonts w:ascii="Calibri"/><w:lang w:val="en-GB"/></w:rPr></w:lvl>"#;
        let mut level = ListLevel::read(&Document::parse(xml).unwrap().root);
        assert_eq!(level.preview(&[21], &[level.format.clone()]), "twenty-one");
        level.indent = 48.0;
        let saved = level.element(0);
        assert_eq!(
            saved.find("w:rPr").unwrap().child_val("w:lang").as_deref(),
            Some("en-GB")
        );
        assert_eq!(ListLevel::read(&saved), level);
    }

    #[test]
    fn custom_choice_survives_fallback_and_list_edits() {
        let xml = r#"<w:lvl w:ilvl="0"><w:start w:val="4"/><mc:AlternateContent><mc:Choice Requires="w14"><w:numFmt w:val="custom" w:format="&#48;01, 002, 003, ..."/></mc:Choice><mc:Fallback><w:numFmt w:val="decimal"/></mc:Fallback></mc:AlternateContent><w:lvlText w:val="%1."/></w:lvl>"#;
        let mut level = ListLevel::read(&Document::parse(xml).unwrap().root);
        assert_eq!(level.format, "custom:001, 002, 003, ...");
        assert_eq!(level.preview(&[4], &[level.format.clone()]), "004.");
        level.alignment = "right".into();
        let saved = level.element(0);
        let format = saved.find("w:numFmt").unwrap();
        assert_eq!(format.get("w:val").as_deref(), Some("custom"));
        assert_eq!(
            format.get("w:format").as_deref(),
            Some("001, 002, 003, ...")
        );
        assert_eq!(ListLevel::read(&saved), level);
    }
    #[test]
    fn definition_preview_and_saved_reader_use_each_levels_start_and_language() {
        let mut definition =
            ListDefinition::from_levels(|level| ListLevel::numbered(level, "decimal", "%1"));
        definition.levels[0].format = "cardinalText".into();
        definition.levels[0].language = Some("en-GB".into());
        definition.levels[0].start = 21;
        definition.levels[1].format = "cardinalText".into();
        definition.levels[1].language = Some("ja-JP".into());
        definition.levels[1].start = 4;
        definition.levels[1].text = "%1-%2".into();
        definition.levels[2].format = "custom:001, 002, 003, ...".into();
        definition.levels[2].start = 7;
        definition.levels[2].text = "%1-%2-%3".into();
        assert_eq!(definition.preview(1), "twenty-one-4");
        assert_eq!(definition.preview(2), "twenty-one-4-007");
        let mut root = Element::new("w:numbering").attr(
            "xmlns:w",
            "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
        );
        root.children.push(Node::Element(definition.element(0, 1)));
        let mut num = Element::new("w:num").attr("w:numId", "1");
        num.children
            .push(Node::Element(Element::with_val("w:abstractNumId", "0")));
        root.children.push(Node::Element(num));
        let xml = Document {
            prolog: String::new(),
            root,
        }
        .to_xml();
        let numbering = crate::numbering::NumberingPart::from_xml(&xml).unwrap();
        let mut counters = crate::numbering::NumberingCounter::new();
        for level in 0..3 {
            counters.increment_with(
                Some(&numbering),
                "1",
                "0",
                level,
                Some(definition.levels[level as usize].start),
                false,
            );
            assert_eq!(
                numbering.format_number("1", level, &counters),
                definition.preview(level as usize)
            );
        }
    }
}

#[cfg(test)]
mod instance_edit_tests {
    use super::*;

    #[test]
    fn edits_override_and_preserves_sibling_and_counter_settings() {
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="8"/><w:lvl w:ilvl="0"><w:start w:val="4"/><w:lvlRestart w:val="0"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="Article %1)"/><w:rPr><w:lang w:val="en-GB"/></w:rPr></w:lvl></w:lvlOverride></w:num></w:numbering>"#;
        let mut document = Document::parse(xml).unwrap();
        let original = document.root.find("w:abstractNum").unwrap().clone();
        edit_instance_level(&mut document.root, "2", "0", 0, |level| {
            level.indent = 48.0;
            level.format = "custom:001, 002, 003, ...".into();
        })
        .unwrap();
        assert_eq!(document.root.find("w:abstractNum"), Some(&original));
        let xml = document.to_xml();
        let numbering = Numbering::parse(&xml);
        let definition = numbering.definition("2").unwrap();
        let level = &definition.levels[0];
        assert_eq!(level.indent, 48.0);
        assert_eq!(level.text, "Article %1)");
        assert_eq!(level.start, 3);
        assert_eq!(level.restart_after, None);
        assert_eq!(level.language.as_deref(), Some("en-GB"));
        assert_eq!(numbering.start_override("2", 0), Some(8));
        assert_eq!(numbering.definition("1").unwrap().preview(0), "3.");
        let reader = crate::numbering::NumberingPart::from_xml(&xml).unwrap();
        let mut counters = crate::numbering::NumberingCounter::new();
        counters.increment_with(Some(&reader), "2", "0", 0, Some(8), true);
        assert_eq!(reader.format_number("2", 0, &counters), "Article 008)");
    }

    #[test]
    fn linked_style_is_read_and_edited_with_instance_override_precedence() {
        let xml = r#"<w:numbering xmlns:w="urn:w"><w:abstractNum w:abstractNumId="0"><w:numStyleLink w:val="ListStyle"/></w:abstractNum><w:abstractNum w:abstractNumId="1"><w:styleLink w:val="ListStyle"/><w:lvl w:ilvl="0"><w:start w:val="2"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1)"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="6"/></w:lvlOverride></w:num><w:num w:numId="2"><w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="0"><w:lvl w:ilvl="0"><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="[%1]"/></w:lvl></w:lvlOverride></w:num></w:numbering>"#;
        let mut document = Document::parse(xml).unwrap();
        let numbering = Numbering::parse(xml);
        assert_eq!(numbering.definition("1").unwrap().preview(0), "II)");
        assert_eq!(numbering.definition("2").unwrap().preview(0), "[b]");
        edit_instance_level(&mut document.root, "1", "0", 0, |level| {
            level.text = "Chapter %1".into()
        })
        .unwrap();
        let saved = Numbering::parse(&document.to_xml());
        assert_eq!(saved.definition("1").unwrap().preview(0), "Chapter II");
        assert_eq!(saved.start_override("1", 0), Some(6));
        assert_eq!(saved.definition("2").unwrap().preview(0), "[b]");
        assert!(document
            .root
            .find("w:abstractNum")
            .unwrap()
            .find("w:lvl")
            .is_none());
        edit_instance_level(&mut document.root, "2", "0", 0, |level| {
            level.text = "Item %1".into()
        })
        .unwrap();
        let saved = Numbering::parse(&document.to_xml());
        assert_eq!(saved.definition("2").unwrap().preview(0), "Item b");
        assert_eq!(saved.definition("1").unwrap().preview(0), "Chapter II");
    }
}

#[cfg(test)]
mod preserved_level_tests {
    use super::*;

    const XML: &str = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0" w:tentative="1"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:legacy w:legacy="1" w:legacySpace="120"/><w:pPr><w:tabs><w:tab w:val="num" w:pos="720"/></w:tabs><w:ind w:left="720" w:right="240" w:hanging="360"/><w:spacing w:after="80"/></w:pPr><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Yu Mincho" w:asciiTheme="minorHAnsi"/><w:b/><w:color w:val="CC0000"/><w:sz w:val="28"/><w:lang w:val="en-GB" w:eastAsia="ja-JP" w:bidi="ar-SA"/><w:rtl/></w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#;

    fn level(root: &Element) -> &Element {
        root.find("w:abstractNum").unwrap().find("w:lvl").unwrap()
    }

    #[test]
    fn unchanged_level_retains_its_xml_tree() {
        let mut document = Document::parse(XML).unwrap();
        let before = document.root.clone();
        edit_level(&mut document.root, "0", 0, |_| {}).unwrap();
        assert_eq!(document.root, before);
    }

    #[test]
    fn indent_edit_preserves_number_runs_and_other_paragraph_settings() {
        let mut document = Document::parse(XML).unwrap();
        let before = level(&document.root).clone();
        edit_level(&mut document.root, "0", 0, |value| value.indent = 48.0).unwrap();
        let after = level(&document.root);
        assert_eq!(after.attributes, before.attributes);
        assert_eq!(after.find("w:rPr"), before.find("w:rPr"));
        assert_eq!(after.find("w:legacy"), before.find("w:legacy"));
        let ppr = after.find("w:pPr").unwrap();
        assert_eq!(
            ppr.find("w:tabs"),
            before.find("w:pPr").unwrap().find("w:tabs")
        );
        assert_eq!(
            ppr.find("w:spacing"),
            before.find("w:pPr").unwrap().find("w:spacing")
        );
        let indent = ppr.find("w:ind").unwrap();
        assert_eq!(indent.get("w:left").as_deref(), Some("960"));
        assert_eq!(indent.get("w:right").as_deref(), Some("240"));
        assert_eq!(indent.get("w:hanging").as_deref(), Some("360"));
    }

    #[test]
    fn font_and_language_edit_preserve_other_scripts_and_run_properties() {
        let mut document = Document::parse(XML).unwrap();
        edit_level(&mut document.root, "0", 0, |value| {
            value.font = Some("Arial".into());
            value.language = Some("en-US".into());
        })
        .unwrap();
        let rpr = level(&document.root).find("w:rPr").unwrap();
        assert_eq!(rpr.child_val("w:color").as_deref(), Some("CC0000"));
        assert!(rpr.find("w:b").is_some());
        let fonts = rpr.find("w:rFonts").unwrap();
        assert_eq!(fonts.get("w:ascii").as_deref(), Some("Arial"));
        assert_eq!(fonts.get("w:eastAsia").as_deref(), Some("Yu Mincho"));
        assert_eq!(fonts.get("w:asciiTheme"), None);
        let lang = rpr.find("w:lang").unwrap();
        assert_eq!(lang.get("w:val").as_deref(), Some("en-US"));
        assert_eq!(lang.get("w:eastAsia").as_deref(), Some("ja-JP"));
        assert_eq!(lang.get("w:bidi").as_deref(), Some("ar-SA"));
    }

    #[test]
    fn format_edit_on_instance_preserves_unmodeled_level_settings() {
        let mut document = Document::parse(XML).unwrap();
        let original = level(&document.root).clone();
        let mut replacement = Element::new("w:lvlOverride").attr("w:ilvl", "0");
        replacement.children.push(Node::Element(original.clone()));
        document
            .root
            .find_mut("w:num")
            .unwrap()
            .children
            .push(Node::Element(replacement));
        edit_instance_level(&mut document.root, "1", "0", 0, |value| {
            value.format = "upperRoman".into()
        })
        .unwrap();
        assert_eq!(level(&document.root), &original);
        let saved = document
            .root
            .find("w:num")
            .unwrap()
            .find("w:lvlOverride")
            .unwrap()
            .find("w:lvl")
            .unwrap();
        assert_eq!(saved.find("w:rPr"), original.find("w:rPr"));
        assert_eq!(saved.find("w:pPr"), original.find("w:pPr"));
        assert_eq!(saved.attributes, original.attributes);
        assert_eq!(saved.child_val("w:numFmt").as_deref(), Some("upperRoman"));
    }
    #[test]
    fn format_edit_updates_compatibility_branches_without_losing_other_content() {
        let xml = r#"<w:numbering xmlns:w="urn:w"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><mc:AlternateContent><mc:Choice Requires="w14"><w:numFmt w:val="custom" w:format="001, 002, 003, ..."/><w14:extension w14:val="keep"/></mc:Choice><mc:Fallback><w:numFmt w:val="decimal"/></mc:Fallback></mc:AlternateContent><w:lvlText w:val="%1"/></w:lvl></w:abstractNum></w:numbering>"#;
        let mut document = Document::parse(xml).unwrap();
        edit_level(&mut document.root, "0", 0, |value| {
            value.format = "lowerRoman".into()
        })
        .unwrap();
        assert_eq!(ListLevel::read(level(&document.root)).format, "lowerRoman");
        let ac = level(&document.root).find("mc:AlternateContent").unwrap();
        assert!(ac
            .find("mc:Choice")
            .unwrap()
            .find("w14:extension")
            .is_some());
        assert_eq!(
            ac.find("mc:Fallback")
                .unwrap()
                .child_val("w:numFmt")
                .as_deref(),
            Some("lowerRoman")
        );
        edit_level(&mut document.root, "0", 0, |value| {
            value.format = "custom:001, 002, 003, ...".into()
        })
        .unwrap();
        assert_eq!(
            ListLevel::read(level(&document.root)).format,
            "custom:001, 002, 003, ..."
        );
        let ac = level(&document.root).find("mc:AlternateContent").unwrap();
        assert_eq!(
            ac.find("mc:Fallback")
                .unwrap()
                .child_val("w:numFmt")
                .as_deref(),
            Some("decimal")
        );
    }

    #[test]
    fn changing_indent_clears_conflicting_units_and_supports_first_line_indent() {
        let mut document = Document::parse(XML).unwrap();
        let ind = document
            .root
            .find_mut("w:abstractNum")
            .unwrap()
            .find_mut("w:lvl")
            .unwrap()
            .find_mut("w:pPr")
            .unwrap()
            .find_mut("w:ind")
            .unwrap();
        for name in [
            "w:start",
            "w:startChars",
            "w:leftChars",
            "w:firstLine",
            "w:firstLineChars",
            "w:hangingChars",
        ] {
            ind.set_attr(name, "200");
        }
        edit_level(&mut document.root, "0", 0, |value| {
            value.indent = 60.0;
            value.hanging = -12.0;
        })
        .unwrap();
        let ind = level(&document.root)
            .find("w:pPr")
            .unwrap()
            .find("w:ind")
            .unwrap();
        assert_eq!(ind.get("w:left").as_deref(), Some("1200"));
        assert_eq!(ind.get("w:firstLine").as_deref(), Some("240"));
        for name in [
            "w:start",
            "w:startChars",
            "w:leftChars",
            "w:hanging",
            "w:firstLineChars",
            "w:hangingChars",
        ] {
            assert_eq!(ind.get(name), None, "{name}");
        }
        assert_eq!(ListLevel::read(level(&document.root)).hanging, -12.0);
        assert_eq!(ind.get("w:right").as_deref(), Some("240"));
    }

    #[test]
    fn new_definition_from_existing_list_keeps_unmodeled_level_properties() {
        let mut document = Document::parse(XML).unwrap();
        let original = level(&document.root).clone();
        let mut definition = Numbering::parse(XML).definition("1").unwrap();
        definition.levels[0].indent = 48.0;
        definition.levels[0].text = "Article %1)".into();
        let id = add_list(&mut document.root, &definition);
        let new_id = document
            .root
            .elements()
            .find(|e| e.name == "w:num" && e.get("w:numId").as_deref() == Some(id.as_str()))
            .unwrap()
            .child_val("w:abstractNumId")
            .unwrap();
        let saved = document
            .root
            .elements()
            .find(|e| {
                e.name == "w:abstractNum"
                    && e.get("w:abstractNumId").as_deref() == Some(new_id.as_str())
            })
            .unwrap()
            .find("w:lvl")
            .unwrap();
        assert_eq!(saved.find("w:rPr"), original.find("w:rPr"));
        assert_eq!(saved.find("w:legacy"), original.find("w:legacy"));
        assert_eq!(saved.get("w:tentative"), original.get("w:tentative"));
        assert_eq!(
            saved.find("w:pPr").unwrap().find("w:tabs"),
            original.find("w:pPr").unwrap().find("w:tabs")
        );
        assert_eq!(ListLevel::read(saved).indent, 48.0);
        assert_eq!(ListLevel::read(saved).text, "Article %1)");
        assert_eq!(level(&document.root), &original);
    }

    #[test]
    fn copied_override_retains_its_own_run_properties_and_effective_counter_settings() {
        let mut document = Document::parse(XML).unwrap();
        let mut overridden = level(&document.root).clone();
        overridden
            .find_mut("w:rPr")
            .unwrap()
            .find_mut("w:color")
            .unwrap()
            .set_attr("w:val", "0000FF");
        overridden.put(Element::with_val("w:start", "99"), LEVEL_ORDER);
        overridden.put(Element::with_val("w:lvlRestart", "0"), LEVEL_ORDER);
        document
            .root
            .find_mut("w:num")
            .unwrap()
            .children
            .push(Node::Element(
                Element::new("w:lvlOverride")
                    .attr("w:ilvl", "0")
                    .child(overridden.clone()),
            ));
        let mut definition = Numbering::parse(&document.to_xml())
            .definition("1")
            .unwrap();
        definition.levels[0].format = "upperRoman".into();
        let generated = definition.element(2, 3);
        let saved = generated.find("w:lvl").unwrap();
        assert_eq!(saved.find("w:rPr"), overridden.find("w:rPr"));
        assert_eq!(saved.find("w:pPr"), overridden.find("w:pPr"));
        assert_eq!(saved.child_val("w:start").as_deref(), Some("1"));
        assert!(saved.find("w:lvlRestart").is_none());
        assert_eq!(ListLevel::read(saved).format, "upperRoman");
    }

    #[test]
    fn restarting_a_list_preserves_all_level_overrides_and_the_source_instance() {
        let mut document = Document::parse(XML).unwrap();
        let source_level = level(&document.root).clone();
        let source = document.root.find_mut("w:num").unwrap();
        source.children.push(Node::Element(
            Element::new("w:lvlOverride")
                .attr("w:ilvl", "0")
                .child(Element::with_val("w:startOverride", "8"))
                .child(source_level.clone()),
        ));
        source.children.push(Node::Element(
            Element::new("w:lvlOverride")
                .attr("w:ilvl", "1")
                .child(Element::with_val("w:startOverride", "4")),
        ));
        source.children.push(Node::Element(
            Element::new("w:lvlOverride")
                .attr("w:ilvl", "2")
                .child(ListLevel::numbered(2, "lowerLetter", "(%3)").element(2)),
        ));
        let original = source.clone();
        let id = restart_instance(&mut document.root, "1", 0, 3).unwrap();
        assert_eq!(document.root.find("w:num"), Some(&original));
        let saved = document
            .root
            .elements()
            .find(|element| {
                element.name == "w:num" && element.get("w:numId").as_deref() == Some(id.as_str())
            })
            .unwrap();
        let overrides: Vec<_> = saved
            .elements()
            .filter(|e| e.name == "w:lvlOverride")
            .collect();
        assert_eq!(overrides.len(), 3);
        assert_eq!(
            overrides[0].child_val("w:startOverride").as_deref(),
            Some("3")
        );
        assert_eq!(overrides[0].find("w:lvl"), Some(&source_level));
        let previous: Vec<_> = original
            .elements()
            .filter(|e| e.name == "w:lvlOverride")
            .collect();
        assert_eq!(overrides[1], previous[1]);
        assert_eq!(overrides[2], previous[2]);
        let numbering = Numbering::parse(&document.to_xml());
        assert_eq!(numbering.start_override(&id, 0), Some(3));
        assert_eq!(numbering.start_override(&id, 1), Some(4));
        assert_eq!(
            numbering.definition(&id).unwrap().levels[2].format,
            "lowerLetter"
        );
        let reader = crate::numbering::NumberingPart::from_xml(&document.to_xml()).unwrap();
        let mut counters = crate::numbering::NumberingCounter::new();
        counters.increment_with(Some(&reader), &id, "0", 0, Some(3), true);
        assert_eq!(reader.format_number(&id, 0, &counters), "3.");
        counters.increment_with(Some(&reader), &id, "0", 2, Some(1), false);
        assert_eq!(reader.format_number(&id, 2, &counters), "(a)");
        let next = restart_instance(&mut document.root, &id, 3, 5).unwrap();
        let numbering = Numbering::parse(&document.to_xml());
        assert_eq!(numbering.start_override(&next, 0), Some(3));
        assert_eq!(numbering.start_override(&next, 1), Some(4));
        assert_eq!(numbering.start_override(&next, 3), Some(5));
        assert_eq!(
            numbering.definition(&next).unwrap().levels[2].format,
            "lowerLetter"
        );
    }

    #[test]
    fn invalid_restart_source_does_not_change_numbering() {
        let mut document = Document::parse(XML).unwrap();
        let original = document.root.clone();
        assert!(restart_instance(&mut document.root, "missing", 0, 3).is_err());
        assert_eq!(document.root, original);
        document
            .root
            .find_mut("w:num")
            .unwrap()
            .remove("w:abstractNumId");
        let original = document.root.clone();
        assert!(restart_instance(&mut document.root, "1", 0, 3).is_err());
        assert_eq!(document.root, original);
    }
}

#[cfg(test)]
mod legal_flag_tests {
    use super::*;
    use crate::numbering::NumberingPart;

    fn xml(flag: &str, concrete: bool) -> String {
        let level = format!(
            r#"<w:lvl w:ilvl="0"><w:start w:val="3"/><w:numFmt w:val="upperRoman"/>{flag}<w:lvlText w:val="%1"/><w:rPr><w:color w:val="CC0000"/></w:rPr></w:lvl>"#
        );
        let (base, replacement) = if concrete {
            (
                if flag.is_empty() {
                    level.replacen("<w:lvlText", "<w:isLgl/><w:lvlText", 1)
                } else {
                    level.replace(flag, "<w:isLgl/>")
                },
                format!(r#"<w:lvlOverride w:ilvl="0">{level}</w:lvlOverride>"#),
            )
        } else {
            (level, String::new())
        };
        format!(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0">{base}</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/>{replacement}</w:num></w:numbering>"#
        )
    }

    #[test]
    fn dialog_legal_flags_match_the_document_reader_for_base_and_override_levels() {
        for (flag, expected) in [
            ("", false),
            ("<w:isLgl/>", true),
            (r#"<w:isLgl w:val="on"/>"#, true),
            (r#"<w:isLgl w:val="true"/>"#, true),
            (r#"<w:isLgl w:val="1"/>"#, true),
            (r#"<w:isLgl w:val="off"/>"#, false),
            (r#"<w:isLgl w:val="false"/>"#, false),
            (r#"<w:isLgl w:val="0"/>"#, false),
        ] {
            for concrete in [false, true] {
                let xml = xml(flag, concrete);
                let definition = Numbering::parse(&xml).definition("1").unwrap();
                assert_eq!(
                    definition.levels[0].legal, expected,
                    "{flag}, override={concrete}"
                );
                assert_eq!(definition.preview(0), if expected { "3" } else { "III" });
                let reader = NumberingPart::from_xml(&xml).unwrap();
                assert_eq!(
                    definition.levels[0].legal,
                    reader.get_level_info("1", 0).unwrap().is_legal
                );
            }
        }
    }

    #[test]
    fn an_explicit_off_flag_is_preserved_and_can_be_enabled_then_disabled() {
        for concrete in [false, true] {
            let xml = xml(r#"<w:isLgl w:val="off"/>"#, concrete);
            let mut document = Document::parse(&xml).unwrap();
            let original = document.to_xml();
            edit_instance_level(&mut document.root, "1", "0", 0, |level| level.legal = false)
                .unwrap();
            assert_eq!(
                document.to_xml(),
                original,
                "disabling an off flag is a no-op"
            );
            edit_instance_level(&mut document.root, "1", "0", 0, |level| {
                level.text = "Item %1".into()
            })
            .unwrap();
            assert_eq!(
                Numbering::parse(&document.to_xml())
                    .definition("1")
                    .unwrap()
                    .preview(0),
                "Item III"
            );
            assert!(document.to_xml().contains(r#"w:val="off""#));
            edit_instance_level(&mut document.root, "1", "0", 0, |level| level.legal = true)
                .unwrap();
            assert_eq!(
                Numbering::parse(&document.to_xml())
                    .definition("1")
                    .unwrap()
                    .preview(0),
                "Item 3"
            );
            edit_instance_level(&mut document.root, "1", "0", 0, |level| level.legal = false)
                .unwrap();
            let saved = document.to_xml();
            assert_eq!(
                Numbering::parse(&saved).definition("1").unwrap().preview(0),
                "Item III"
            );
            assert!(
                !NumberingPart::from_xml(&saved)
                    .unwrap()
                    .get_level_info("1", 0)
                    .unwrap()
                    .is_legal
            );
            assert!(saved.contains(r#"<w:color w:val="CC0000"/>"#));
        }
    }
}
