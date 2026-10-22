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
        lvl.children
            .push(Node::Element(Element::with_val("w:numFmt", &self.format)));
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
        if let Some(font) = self.font.as_deref() {
            lvl.children.push(Node::Element(
                Element::new("w:rPr").child(
                    Element::new("w:rFonts")
                        .attr("w:ascii", font)
                        .attr("w:hAnsi", font)
                        .attr("w:hint", "default"),
                ),
            ));
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
            format: lvl
                .child_val("w:numFmt")
                .unwrap_or_else(|| "decimal".into()),
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
            legal: lvl
                .find("w:isLgl")
                .is_some_and(|legal| !matches!(legal.get("w:val").as_deref(), Some("0" | "false"))),
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
        // This level's own placeholder: the highest one the text names.
        let own = self
            .text
            .match_indices('%')
            .filter_map(|(at, _)| self.text[at + 1..].chars().next()?.to_digit(10))
            .max()
            .map(|digit| digit.saturating_sub(1) as usize);
        if self.is_bullet() {
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
                    // Legal: the levels above in decimal, this one as it
                    // is (Word's `Section 1.01`).
                    let format = if self.legal && Some(index) != own {
                        "decimal"
                    } else {
                        formats.get(index).map(String::as_str).unwrap_or("decimal")
                    };
                    out.push_str(&crate::numbering::format_number_value(format, value, None));
                    continue;
                }
            }
            out.push(ch);
        }
        out
    }
}

fn twips(points: f32) -> String {
    ((points * 20.0).round() as i64).to_string()
}

/// A list definition: nine levels.
#[derive(Clone, Debug, PartialEq)]
pub struct ListDefinition {
    pub levels: Vec<ListLevel>,
    /// The list style this definition is (`w:styleLink`), for a list style.
    pub style_link: Option<String>,
}

impl ListDefinition {
    /// Every level from `level`.
    pub fn from_levels(level: impl Fn(u8) -> ListLevel) -> Self {
        ListDefinition {
            levels: (0..9).map(level).collect(),
            style_link: None,
        }
    }

    /// The number of each level, as a preview with every level at 1.
    pub fn preview(&self, level: usize) -> String {
        let formats: Vec<String> = self
            .levels
            .iter()
            .map(|level| level.format.clone())
            .collect();
        self.levels
            .get(level)
            .map(|lvl| lvl.preview(&[1; 9], &formats))
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
            element
                .children
                .push(Node::Element(level.element(index as u8)));
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
    /// Article I. / Section 1.01 / (a), legal-style and linked to headings.
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
                return Some(ListDefinition::read(linked));
            }
        }
        Some(ListDefinition::read(abstract_num))
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
    let mut current = match index.map(|index| &abstract_num.children[index]) {
        Some(Node::Element(element)) => ListLevel::read(element),
        _ => ListLevel::numbered(level, "decimal", &format!("%{}.", level + 1)),
    };
    edit(&mut current);
    let element = current.element(level);
    match index {
        Some(index) => abstract_num.children[index] = Node::Element(element),
        None => abstract_num.children.push(Node::Element(element)),
    }
    Ok(())
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
            let level = state.level;
            return edit_package(
                self,
                move |package| {
                    package.edit(PartKind::Numbering, |numbering| {
                        edit_level(numbering, &abstract_id, level, |lvl| {
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
        let abstract_id = state.abstract_id.clone();
        let level = state.level;
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Numbering, |numbering| {
                    let made = add_instance(numbering, &abstract_id, Some((level, value)));
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
        let level = state.level;
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Numbering, |numbering| {
                    edit_level(numbering, &abstract_id, level, edit)
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
        // Legal: level 1's roman numeral reads as a decimal.
        assert_eq!(articles.preview(1), "Section 1.01");
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
