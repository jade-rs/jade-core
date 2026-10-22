//! Styles as Word's スタイル gallery, スタイル pane and スタイルの変更 dialog
//! handle them: the catalogue in Word's order (`w:uiPriority`, the quick
//! styles first), applying paragraph, character and linked styles, creating
//! a style (新しいスタイル), modifying one — its name, 基準にするスタイル,
//! 次の段落のスタイル, its format, 自動的に更新する, スタイル ギャラリーに追加
//! — updating one to match the selection, attaching a template's styles
//! (テンプレートとアドイン) and the style's own format, read for the dialog.
//!
//! A style reaches every paragraph that uses it, so a style edit is a
//! package edit read back in place (see `package`): the pages show exactly
//! what a reopened file shows.

use jade_docx_headless_api::{HeadlessCommand, HeadlessQuery, HeadlessQueryResult};

use super::package::{edit_package, HeadlessStep, Package, PartKind};
use super::xml::{Document, Element, Node};
use super::{query, GalleryStyle, WordFormatting, WordSession};

/// `CT_Styles`' children.
pub const STYLES_ORDER: &[&str] = &["w:docDefaults", "w:latentStyles", "w:style"];
/// `CT_Style`'s children.
pub const STYLE_ORDER: &[&str] = &[
    "w:name",
    "w:aliases",
    "w:basedOn",
    "w:next",
    "w:link",
    "w:autoRedefine",
    "w:hidden",
    "w:uiPriority",
    "w:semiHidden",
    "w:unhideWhenUsed",
    "w:qFormat",
    "w:locked",
    "w:personal",
    "w:personalCompose",
    "w:personalReply",
    "w:rsid",
    "w:pPr",
    "w:rPr",
    "w:tblPr",
    "w:trPr",
    "w:tcPr",
    "w:tblStylePr",
];
/// `CT_PPrBase`'s children (and `CT_PPr`'s tail).
pub const PPR_ORDER: &[&str] = &[
    "w:pStyle",
    "w:keepNext",
    "w:keepLines",
    "w:pageBreakBefore",
    "w:framePr",
    "w:widowControl",
    "w:numPr",
    "w:suppressLineNumbers",
    "w:pBdr",
    "w:shd",
    "w:tabs",
    "w:suppressAutoHyphens",
    "w:kinsoku",
    "w:wordWrap",
    "w:overflowPunct",
    "w:topLinePunct",
    "w:autoSpaceDE",
    "w:autoSpaceDN",
    "w:bidi",
    "w:adjustRightInd",
    "w:snapToGrid",
    "w:spacing",
    "w:ind",
    "w:contextualSpacing",
    "w:mirrorIndents",
    "w:suppressOverlap",
    "w:jc",
    "w:textDirection",
    "w:textAlignment",
    "w:textboxTightWrap",
    "w:outlineLvl",
    "w:divId",
    "w:cnfStyle",
    "w:rPr",
    "w:sectPr",
    "w:pPrChange",
];
/// `CT_RPr`'s children.
pub const RPR_ORDER: &[&str] = &[
    "w:rStyle",
    "w:rFonts",
    "w:b",
    "w:bCs",
    "w:i",
    "w:iCs",
    "w:caps",
    "w:smallCaps",
    "w:strike",
    "w:dstrike",
    "w:outline",
    "w:shadow",
    "w:emboss",
    "w:imprint",
    "w:noProof",
    "w:snapToGrid",
    "w:vanish",
    "w:webHidden",
    "w:color",
    "w:spacing",
    "w:w",
    "w:kern",
    "w:position",
    "w:sz",
    "w:szCs",
    "w:highlight",
    "w:u",
    "w:effect",
    "w:bdr",
    "w:shd",
    "w:fitText",
    "w:vertAlign",
    "w:rtl",
    "w:cs",
    "w:em",
    "w:lang",
    "w:eastAsianLayout",
    "w:specVanish",
    "w:oMath",
];

/// What kind of style it is, as the スタイルの種類 list names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StyleKind {
    Paragraph,
    Character,
    /// A paragraph style with a linked character style (段落と文字).
    Linked,
    Table,
    /// A list style (リスト).
    Numbering,
}

impl StyleKind {
    fn token(self) -> &'static str {
        match self {
            StyleKind::Paragraph | StyleKind::Linked => "paragraph",
            StyleKind::Character => "character",
            StyleKind::Table => "table",
            StyleKind::Numbering => "numbering",
        }
    }
}

/// A style's own format — what the dialog's 書式 section and its sub-dialogs
/// edit. `None` is "not set by this style" (it inherits).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct StyleFormat {
    /// The Latin face (`w:ascii`, `w:hAnsi`).
    pub font: Option<String>,
    /// The Japanese face (`w:eastAsia`).
    pub east_asia_font: Option<String>,
    /// Points.
    pub size: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// `RRGGBB`.
    pub color: Option<String>,
    /// `left`, `center`, `right`, `both`, `distribute`.
    pub alignment: Option<String>,
    /// A multiple of single spacing.
    pub line_spacing: Option<f32>,
    /// Points.
    pub space_before: Option<f32>,
    pub space_after: Option<f32>,
    /// Points.
    pub indent_left: Option<f32>,
    /// Points; negative for a hanging indent.
    pub first_line: Option<f32>,
    pub keep_next: Option<bool>,
    /// 0–8; 9 is body text.
    pub outline_level: Option<u8>,
    /// A ¾ pt line under the paragraph (style sets' headings).
    pub bottom_border: Option<bool>,
}

impl StyleFormat {
    pub fn is_empty(&self) -> bool {
        self == &StyleFormat::default()
    }

    /// Everything `other` sets, over this.
    pub fn overlay(&mut self, other: &StyleFormat) {
        macro_rules! take {
            ($($field:ident),*) => {$(
                if other.$field.is_some() {
                    self.$field = other.$field.clone();
                }
            )*};
        }
        take!(
            font,
            east_asia_font,
            size,
            bold,
            italic,
            underline,
            color,
            alignment,
            line_spacing,
            space_before,
            space_after,
            indent_left,
            first_line,
            keep_next,
            outline_level,
            bottom_border
        );
    }
}

fn on(element: Option<&Element>) -> Option<bool> {
    element.map(|element| {
        !matches!(
            element.get("w:val").as_deref(),
            Some("0") | Some("false") | Some("off")
        )
    })
}

fn twips(value: Option<String>) -> Option<f32> {
    value
        .and_then(|value| value.parse::<f32>().ok())
        .map(|value| value / 20.0)
}

/// The format a `w:rPr` and a `w:pPr` set.
pub fn read_format(rpr: Option<&Element>, ppr: Option<&Element>) -> StyleFormat {
    let mut format = StyleFormat::default();
    if let Some(rpr) = rpr {
        if let Some(fonts) = rpr.find("w:rFonts") {
            format.font = fonts.get("w:ascii").or_else(|| fonts.get("w:hAnsi"));
            format.east_asia_font = fonts.get("w:eastAsia");
        }
        format.size = rpr
            .child_val("w:sz")
            .and_then(|value| value.parse::<f32>().ok())
            .map(|half_points| half_points / 2.0);
        format.bold = on(rpr.find("w:b"));
        format.italic = on(rpr.find("w:i"));
        format.underline = rpr
            .child_val("w:u")
            .map(|value| value != "none")
            .or_else(|| rpr.find("w:u").map(|_| true));
        format.color = rpr
            .child_val("w:color")
            .filter(|value| !value.eq_ignore_ascii_case("auto"))
            .map(|value| value.to_ascii_uppercase());
    }
    if let Some(ppr) = ppr {
        format.alignment = ppr.child_val("w:jc");
        if let Some(spacing) = ppr.find("w:spacing") {
            format.space_before = twips(spacing.get("w:before"));
            format.space_after = twips(spacing.get("w:after"));
            if spacing.get("w:lineRule").is_none_or(|rule| rule == "auto") {
                format.line_spacing = spacing
                    .get("w:line")
                    .and_then(|value| value.parse::<f32>().ok())
                    .map(|line| line / 240.0);
            }
        }
        if let Some(ind) = ppr.find("w:ind") {
            format.indent_left = twips(ind.get("w:left").or_else(|| ind.get("w:start")));
            format.first_line = twips(ind.get("w:firstLine"))
                .or_else(|| twips(ind.get("w:hanging")).map(|hanging| -hanging));
        }
        format.keep_next = on(ppr.find("w:keepNext"));
        format.outline_level = ppr
            .child_val("w:outlineLvl")
            .and_then(|value| value.parse().ok());
        format.bottom_border = ppr
            .find("w:pBdr")
            .map(|borders| borders.find("w:bottom").is_some());
    }
    format
}

fn twips_text(points: f32) -> String {
    ((points * 20.0).round() as i64).to_string()
}

/// Writes the run part of `format` into `rpr`; what it leaves `None` stays.
pub fn write_run_format(rpr: &mut Element, format: &StyleFormat) {
    if format.font.is_some() || format.east_asia_font.is_some() {
        let fonts = rpr.ensure("w:rFonts", RPR_ORDER);
        if let Some(font) = format.font.as_deref() {
            fonts.set_attr("w:ascii", font);
            fonts.set_attr("w:hAnsi", font);
            fonts.remove_attr("w:asciiTheme");
            fonts.remove_attr("w:hAnsiTheme");
        }
        if let Some(font) = format.east_asia_font.as_deref() {
            fonts.set_attr("w:eastAsia", font);
            fonts.remove_attr("w:eastAsiaTheme");
        }
    }
    let flag = |rpr: &mut Element, name: &str, complex: &str, value: Option<bool>| {
        let Some(value) = value else { return };
        for tag in [name, complex] {
            let element = if value {
                Element::new(tag)
            } else {
                Element::with_val(tag, "0")
            };
            rpr.put(element, RPR_ORDER);
        }
    };
    flag(rpr, "w:b", "w:bCs", format.bold);
    flag(rpr, "w:i", "w:iCs", format.italic);
    if let Some(underline) = format.underline {
        rpr.put(
            Element::with_val("w:u", if underline { "single" } else { "none" }),
            RPR_ORDER,
        );
    }
    if let Some(color) = format.color.as_deref() {
        rpr.put(
            Element::with_val(
                "w:color",
                &color.trim_start_matches('#').to_ascii_uppercase(),
            ),
            RPR_ORDER,
        );
    }
    if let Some(size) = format.size {
        let half_points = ((size * 2.0).round() as i64).to_string();
        rpr.put(Element::with_val("w:sz", &half_points), RPR_ORDER);
        rpr.put(Element::with_val("w:szCs", &half_points), RPR_ORDER);
    }
}

/// Writes the paragraph part of `format` into `ppr`.
pub fn write_paragraph_format(ppr: &mut Element, format: &StyleFormat) {
    if let Some(keep) = format.keep_next {
        ppr.set_flag("w:keepNext", keep, PPR_ORDER);
    }
    if let Some(border) = format.bottom_border {
        if border {
            ppr.put(
                Element::new("w:pBdr").child(
                    Element::with_val("w:bottom", "single")
                        .attr("w:sz", "6")
                        .attr("w:space", "1")
                        .attr("w:color", "auto"),
                ),
                PPR_ORDER,
            );
        } else {
            ppr.remove("w:pBdr");
        }
    }
    if format.space_before.is_some()
        || format.space_after.is_some()
        || format.line_spacing.is_some()
    {
        let spacing = ppr.ensure("w:spacing", PPR_ORDER);
        if let Some(before) = format.space_before {
            spacing.set_attr("w:before", &twips_text(before));
            spacing.remove_attr("w:beforeLines");
            spacing.remove_attr("w:beforeAutospacing");
        }
        if let Some(after) = format.space_after {
            spacing.set_attr("w:after", &twips_text(after));
            spacing.remove_attr("w:afterLines");
            spacing.remove_attr("w:afterAutospacing");
        }
        if let Some(line) = format.line_spacing {
            spacing.set_attr("w:line", &((line * 240.0).round() as i64).to_string());
            spacing.set_attr("w:lineRule", "auto");
        }
    }
    if format.indent_left.is_some() || format.first_line.is_some() {
        let ind = ppr.ensure("w:ind", PPR_ORDER);
        if let Some(left) = format.indent_left {
            ind.remove_attr("w:start");
            ind.remove_attr("w:leftChars");
            ind.remove_attr("w:startChars");
            ind.set_attr("w:left", &twips_text(left));
        }
        if let Some(first) = format.first_line {
            for name in [
                "w:firstLine",
                "w:hanging",
                "w:firstLineChars",
                "w:hangingChars",
            ] {
                ind.remove_attr(name);
            }
            if first < 0.0 {
                ind.set_attr("w:hanging", &twips_text(-first));
            } else {
                ind.set_attr("w:firstLine", &twips_text(first));
            }
        }
    }
    if let Some(alignment) = format.alignment.as_deref() {
        ppr.put(Element::with_val("w:jc", alignment), PPR_ORDER);
    }
    if let Some(level) = format.outline_level {
        if level >= 9 {
            ppr.remove("w:outlineLvl");
        } else {
            ppr.put(
                Element::with_val("w:outlineLvl", &level.to_string()),
                PPR_ORDER,
            );
        }
    }
}

/// Writes `format` into a style element's `w:pPr` and `w:rPr`.
pub fn write_style_format(style: &mut Element, format: &StyleFormat, paragraph: bool) {
    if paragraph {
        let has_paragraph_part = format.alignment.is_some()
            || format.line_spacing.is_some()
            || format.space_before.is_some()
            || format.space_after.is_some()
            || format.indent_left.is_some()
            || format.first_line.is_some()
            || format.keep_next.is_some()
            || format.outline_level.is_some()
            || format.bottom_border.is_some();
        if has_paragraph_part {
            let ppr = style.ensure("w:pPr", STYLE_ORDER);
            write_paragraph_format(ppr, format);
        }
    }
    let has_run_part = format.font.is_some()
        || format.east_asia_font.is_some()
        || format.size.is_some()
        || format.bold.is_some()
        || format.italic.is_some()
        || format.underline.is_some()
        || format.color.is_some();
    if has_run_part {
        let rpr = style.ensure("w:rPr", STYLE_ORDER);
        write_run_format(rpr, format);
    }
}

/// One style of the document, as the スタイル pane lists it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleInfo {
    pub id: String,
    /// `w:name`, Word's built-in English name for its own styles.
    pub name: String,
    pub kind: StyleKind,
    pub based_on: Option<String>,
    pub next: Option<String>,
    /// The other half of a linked pair.
    pub link: Option<String>,
    /// `w:uiPriority`; 99 when missing, as Word reads it.
    pub priority: i32,
    /// `w:qFormat`: in the style gallery.
    pub quick: bool,
    pub hidden: bool,
    pub semi_hidden: bool,
    /// `w:autoRedefine`: 自動的に更新する.
    pub auto_update: bool,
    pub is_default: bool,
    /// `w:customStyle`: made by a user rather than built in.
    pub custom: bool,
    /// The style's own format.
    pub format: StyleFormat,
}

impl StyleInfo {
    /// Whether the pane shows it: not hidden, not a table style, not one of
    /// Word's implementation styles.
    pub fn listed(&self) -> bool {
        // Word's implementation styles stay out of the pane.
        let implementation = matches!(
            self.name.to_ascii_lowercase().as_str(),
            "default paragraph font" | "no list" | "normal table"
        );
        !implementation
            && !self.hidden
            && !self.semi_hidden
            && matches!(
                self.kind,
                StyleKind::Paragraph | StyleKind::Character | StyleKind::Linked
            )
    }

    /// Whether a paragraph style or the paragraph half of a linked one.
    pub fn is_paragraph(&self) -> bool {
        matches!(self.kind, StyleKind::Paragraph | StyleKind::Linked)
    }
}

/// Every style of a `styles.xml`, in document order.
pub fn catalog_of(styles_xml: &str) -> Vec<StyleInfo> {
    let Ok(document) = Document::parse(styles_xml) else {
        return Vec::new();
    };
    let styles: Vec<&Element> = document
        .root
        .elements()
        .filter(|element| element.name == "w:style")
        .collect();
    let character_ids: Vec<String> = styles
        .iter()
        .filter(|style| style.get("w:type").as_deref() == Some("character"))
        .filter_map(|style| style.get("w:styleId"))
        .collect();
    styles
        .iter()
        .filter_map(|style| {
            let id = style.get("w:styleId")?;
            let link = style.child_val("w:link");
            let kind = match style.get("w:type").as_deref().unwrap_or("paragraph") {
                "character" => StyleKind::Character,
                "table" => StyleKind::Table,
                "numbering" => StyleKind::Numbering,
                _ if link
                    .as_ref()
                    .is_some_and(|link| character_ids.contains(link)) =>
                {
                    StyleKind::Linked
                }
                _ => StyleKind::Paragraph,
            };
            Some(StyleInfo {
                name: style.child_val("w:name").unwrap_or_else(|| id.clone()),
                kind,
                based_on: style.child_val("w:basedOn"),
                next: style.child_val("w:next"),
                link,
                priority: style
                    .child_val("w:uiPriority")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(99),
                quick: on(style.find("w:qFormat")).unwrap_or(false),
                hidden: on(style.find("w:hidden")).unwrap_or(false),
                semi_hidden: on(style.find("w:semiHidden")).unwrap_or(false),
                auto_update: on(style.find("w:autoRedefine")).unwrap_or(false),
                is_default: matches!(style.get("w:default").as_deref(), Some("1" | "true" | "on")),
                custom: matches!(
                    style.get("w:customStyle").as_deref(),
                    Some("1" | "true" | "on")
                ),
                format: read_format(style.find("w:rPr"), style.find("w:pPr")),
                id,
            })
        })
        .collect()
}

/// Word's 標準 and built-in names as Japanese Word shows them.
pub fn display_name(name: &str, japanese: bool) -> String {
    if !japanese {
        let mut chars = name.chars();
        return match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        };
    }
    let lower = name.to_ascii_lowercase();
    if let Some(level) = lower.strip_prefix("heading ") {
        return format!("見出し {level}");
    }
    if let Some(level) = lower.strip_prefix("toc ") {
        return format!("目次 {level}");
    }
    let known = match lower.as_str() {
        "normal" => "標準",
        "title" => "表題",
        "subtitle" => "副題",
        "quote" => "引用文",
        "intense quote" => "引用文 2",
        "emphasis" => "強調斜体",
        "strong" => "強調太字",
        "subtle emphasis" => "斜体",
        "intense emphasis" => "強調斜体 2",
        "subtle reference" => "参照",
        "intense reference" => "参照 2",
        "book title" => "書名",
        "list paragraph" => "リスト段落",
        "no spacing" => "行間詰め",
        "caption" => "図表番号",
        "default paragraph font" => "段落フォント",
        "hyperlink" => "ハイパーリンク",
        "header" => "ヘッダー",
        "footer" => "フッター",
        "footnote text" => "脚注文字列",
        "endnote text" => "文末脚注文字列",
        "table grid" => "表 (格子)",
        "date" => "日付",
        "closing" => "結語",
        "salutation" => "挨拶文",
        "note heading" => "記",
        _ => return name.to_string(),
    };
    known.to_string()
}

/// The style gallery in Word's order: 標準 and 見出し 1–3 always (Word offers
/// them even before a document defines them), then every quick style by
/// `w:uiPriority` and name.
pub fn quick_style_gallery(catalog: &[StyleInfo]) -> Vec<GalleryStyle> {
    let fixed: Vec<GalleryStyle> = super::style_gallery(&[]);
    let mut gallery = fixed;
    let mut quick: Vec<&StyleInfo> = catalog
        .iter()
        .filter(|style| style.quick && style.listed())
        .filter(|style| {
            // The fixed head already stands for these.
            let token: String = style
                .name
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .flat_map(char::to_lowercase)
                .collect();
            !matches!(
                token.as_str(),
                "normal" | "heading1" | "heading2" | "heading3"
            ) && !style.is_default
        })
        .collect();
    quick.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.name.cmp(&b.name))
    });
    for style in quick {
        gallery.push(GalleryStyle {
            id: Some(style.id.clone()),
            name: Some(style.name.clone()),
            preset: None,
        });
    }
    // Styles that are not quick ones stay reachable from the pane.
    gallery
}

/// A new style's settings (新しいスタイル).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewStyle {
    pub name: String,
    pub kind: StyleKind,
    #[serde(default)]
    pub based_on: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub format: StyleFormat,
    /// スタイル ギャラリーに追加.
    #[serde(default)]
    pub quick: bool,
    /// 自動的に更新する.
    #[serde(default)]
    pub auto_update: bool,
}

/// What a スタイルの変更 changes; `None` leaves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleChange {
    pub name: Option<String>,
    /// `Some(None)` takes the base off (スタイルなし).
    pub based_on: Option<Option<String>>,
    pub next: Option<String>,
    pub format: StyleFormat,
    pub quick: Option<bool>,
    pub auto_update: Option<bool>,
}

/// A style id from a name: its letters and digits, made unique.
fn new_style_id(name: &str, taken: &[String]) -> String {
    let base: String = name.chars().filter(|ch| ch.is_alphanumeric()).collect();
    let base = if base.is_empty() {
        "Style".to_string()
    } else {
        base
    };
    if !taken.iter().any(|id| id.eq_ignore_ascii_case(&base)) {
        return base;
    }
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|candidate| !taken.iter().any(|id| id.eq_ignore_ascii_case(candidate)))
        .expect("an unused id")
}

fn style_ids(styles: &Element) -> Vec<String> {
    styles
        .elements()
        .filter(|element| element.name == "w:style")
        .filter_map(|style| style.get("w:styleId"))
        .collect()
}

fn find_style_mut<'a>(styles: &'a mut Element, id: &str) -> Option<&'a mut Element> {
    styles.elements_mut().find(|element| {
        element.name == "w:style" && element.get("w:styleId").as_deref() == Some(id)
    })
}

/// The id of the style named `name` (case and spaces aside).
fn style_id_named(styles: &Element, name: &str) -> Option<String> {
    let token = |value: &str| -> String {
        value
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let wanted = token(name);
    styles
        .elements()
        .filter(|element| element.name == "w:style")
        .find(|style| {
            style
                .child_val("w:name")
                .is_some_and(|value| token(&value) == wanted)
        })
        .and_then(|style| style.get("w:styleId"))
}

fn default_paragraph_style(styles: &Element) -> Option<String> {
    styles
        .elements()
        .filter(|element| element.name == "w:style")
        .find(|style| {
            style.get("w:type").as_deref() == Some("paragraph")
                && matches!(style.get("w:default").as_deref(), Some("1" | "true" | "on"))
        })
        .and_then(|style| style.get("w:styleId"))
}

/// Adds a style to `styles`, returning its id.
pub fn add_style(styles: &mut Element, new: &NewStyle) -> Result<String, String> {
    let name = new.name.trim();
    if name.is_empty() {
        return Err("a style needs a name".to_string());
    }
    if style_id_named(styles, name).is_some() {
        return Err(format!("a style named {name} already exists"));
    }
    let id = new_style_id(name, &style_ids(styles));
    let paragraph = matches!(new.kind, StyleKind::Paragraph | StyleKind::Linked);
    let mut style = Element::new("w:style")
        .attr("w:type", new.kind.token())
        .attr("w:customStyle", "1")
        .attr("w:styleId", &id);
    style.put(Element::with_val("w:name", name), STYLE_ORDER);
    if let Some(base) = new.based_on.as_deref() {
        style.put(Element::with_val("w:basedOn", base), STYLE_ORDER);
    }
    if paragraph {
        let next = new.next.clone().unwrap_or_else(|| id.clone());
        style.put(Element::with_val("w:next", &next), STYLE_ORDER);
    }
    if new.auto_update && paragraph {
        style.put(Element::new("w:autoRedefine"), STYLE_ORDER);
    }
    if new.quick {
        style.put(Element::new("w:qFormat"), STYLE_ORDER);
    }
    write_style_format(&mut style, &new.format, paragraph);
    if new.kind == StyleKind::Linked {
        // The character half: the run format, linked back.
        let char_id = new_style_id(&format!("{id}Char"), &style_ids(styles));
        style.put(Element::with_val("w:link", &char_id), STYLE_ORDER);
        let mut character = Element::new("w:style")
            .attr("w:type", "character")
            .attr("w:customStyle", "1")
            .attr("w:styleId", &char_id);
        character.put(
            Element::with_val("w:name", &format!("{name} (文字)")),
            STYLE_ORDER,
        );
        character.put(Element::with_val("w:link", &id), STYLE_ORDER);
        character.put(Element::new("w:semiHidden"), STYLE_ORDER);
        let run_only = StyleFormat {
            font: new.format.font.clone(),
            east_asia_font: new.format.east_asia_font.clone(),
            size: new.format.size,
            bold: new.format.bold,
            italic: new.format.italic,
            underline: new.format.underline,
            color: new.format.color.clone(),
            ..StyleFormat::default()
        };
        write_style_format(&mut character, &run_only, false);
        styles.children.push(Node::Element(style));
        styles.children.push(Node::Element(character));
    } else {
        styles.children.push(Node::Element(style));
    }
    Ok(id)
}

/// Applies `change` to style `id` in `styles`.
pub fn change_style(styles: &mut Element, id: &str, change: &StyleChange) -> Result<(), String> {
    if let Some(name) = change.name.as_deref() {
        let name = name.trim();
        if name.is_empty() {
            return Err("a style needs a name".to_string());
        }
        if style_id_named(styles, name).is_some_and(|other| other != id) {
            return Err(format!("a style named {name} already exists"));
        }
    }
    if let Some(Some(base)) = change.based_on.as_ref() {
        // A style cannot be based on itself, or on one based on it.
        let mut cursor = Some(base.clone());
        let mut steps = 0;
        while let Some(current) = cursor {
            if current == id {
                return Err("a style cannot be based on itself".to_string());
            }
            steps += 1;
            if steps > 64 {
                break;
            }
            cursor = styles
                .elements()
                .find(|style| {
                    style.name == "w:style"
                        && style.get("w:styleId").as_deref() == Some(current.as_str())
                })
                .and_then(|style| style.child_val("w:basedOn"));
        }
    }
    let link = {
        let style = find_style_mut(styles, id).ok_or_else(|| format!("no style {id}"))?;
        let paragraph = style.get("w:type").as_deref().unwrap_or("paragraph") == "paragraph";
        if let Some(name) = change.name.as_deref() {
            style.put(Element::with_val("w:name", name.trim()), STYLE_ORDER);
        }
        match change.based_on.as_ref() {
            Some(Some(base)) => style.put(Element::with_val("w:basedOn", base), STYLE_ORDER),
            Some(None) => style.remove("w:basedOn"),
            None => {}
        }
        if let (Some(next), true) = (change.next.as_deref(), paragraph) {
            style.put(Element::with_val("w:next", next), STYLE_ORDER);
        }
        if let Some(quick) = change.quick {
            style.set_flag("w:qFormat", quick, STYLE_ORDER);
        }
        if let (Some(auto), true) = (change.auto_update, paragraph) {
            style.set_flag("w:autoRedefine", auto, STYLE_ORDER);
        }
        write_style_format(style, &change.format, paragraph);
        style.child_val("w:link")
    };
    // A linked style's character half keeps the same run format.
    if let Some(link) = link {
        if let Some(character) = find_style_mut(styles, &link) {
            if character.get("w:type").as_deref() == Some("character") {
                write_style_format(character, &change.format, false);
            }
        }
    }
    Ok(())
}

/// Word's built-in definition of a style it adds the first time it is
/// used: its id, type, priority and paragraph format.
fn builtin(name: &str) -> Option<(String, u8, Option<u8>, bool)> {
    let lower = name.to_ascii_lowercase();
    if lower == "normal" {
        return Some(("Normal".into(), 0, None, false));
    }
    if lower == "title" {
        return Some(("Title".into(), 10, None, false));
    }
    let level: u8 = lower.strip_prefix("heading ")?.trim().parse().ok()?;
    (1..=9)
        .contains(&level)
        .then(|| (format!("Heading{level}"), 9, Some(level - 1), true))
}

/// Applies `format` to the built-in style named `name` (Normal, heading N,
/// Title), adding Word's definition first when the document has none.
pub fn edit_builtin_style(
    styles: &mut Element,
    name: &str,
    format: &StyleFormat,
) -> Result<(), String> {
    let id = match style_id_named(styles, name) {
        Some(id) => id,
        None => {
            let (base_id, priority, outline, keep_next) =
                builtin(name).ok_or_else(|| format!("{name} is not a built-in style"))?;
            let ids = style_ids(styles);
            let id = new_style_id(&base_id, &ids);
            let default = default_paragraph_style(styles);
            let mut style = Element::new("w:style")
                .attr("w:type", "paragraph")
                .attr("w:styleId", &id);
            style.put(Element::with_val("w:name", name), STYLE_ORDER);
            if name.eq_ignore_ascii_case("normal") {
                style.set_attr("w:default", "1");
            } else if let Some(default) = default.as_deref() {
                style.put(Element::with_val("w:basedOn", default), STYLE_ORDER);
                style.put(Element::with_val("w:next", default), STYLE_ORDER);
            }
            style.put(
                Element::with_val("w:uiPriority", &priority.to_string()),
                STYLE_ORDER,
            );
            style.put(Element::new("w:qFormat"), STYLE_ORDER);
            write_style_format(
                &mut style,
                &StyleFormat {
                    keep_next: keep_next.then_some(true),
                    outline_level: outline,
                    ..StyleFormat::default()
                },
                true,
            );
            styles.children.push(Node::Element(style));
            id
        }
    };
    change_style(
        styles,
        &id,
        &StyleChange {
            format: format.clone(),
            ..StyleChange::default()
        },
    )
}

/// Copies every style of `template` into `styles`: a style of the same name
/// is replaced by the template's (keeping the document's id, which its
/// paragraphs name), one the document lacks is added, and the template's
/// document defaults replace the document's. Returns how many it copied.
pub fn copy_template_styles(styles: &mut Element, template: &Element) -> usize {
    let mut copied = 0;
    if let Some(defaults) = template.find("w:docDefaults") {
        styles.put(defaults.clone(), STYLES_ORDER);
    }
    // Template ids the document knows by another id, so references in the
    // copied styles (basedOn, next, link) can be renamed to match.
    let mut renamed: Vec<(String, String)> = Vec::new();
    for style in template
        .elements()
        .filter(|element| element.name == "w:style")
    {
        let (Some(template_id), Some(name)) = (style.get("w:styleId"), style.child_val("w:name"))
        else {
            continue;
        };
        if let Some(existing) = style_id_named(styles, &name) {
            if existing != template_id {
                renamed.push((template_id, existing));
            }
        }
    }
    let rename = |value: String| -> String {
        renamed
            .iter()
            .find(|(from, _)| *from == value)
            .map(|(_, to)| to.clone())
            .unwrap_or(value)
    };
    for style in template
        .elements()
        .filter(|element| element.name == "w:style")
    {
        let (Some(template_id), Some(name)) = (style.get("w:styleId"), style.child_val("w:name"))
        else {
            continue;
        };
        let mut copy = style.clone();
        let id = rename(template_id.clone());
        copy.set_attr("w:styleId", &id);
        for reference in ["w:basedOn", "w:next", "w:link"] {
            if let Some(value) = copy.child_val(reference) {
                copy.put(Element::with_val(reference, &rename(value)), STYLE_ORDER);
            }
        }
        let existing = style_id_named(styles, &name).or_else(|| {
            style_ids(styles)
                .into_iter()
                .find(|existing| *existing == id)
        });
        match existing {
            Some(existing) => {
                copy.set_attr("w:styleId", &existing);
                if let Some(slot) = find_style_mut(styles, &existing) {
                    *slot = copy;
                }
            }
            None => styles.children.push(Node::Element(copy)),
        }
        copied += 1;
    }
    copied
}

/// The run and paragraph format at the caret, as a style would take it
/// (選択個所と一致するように更新, and 新しいスタイル's starting format).
pub fn format_at_caret<S: WordSession + ?Sized>(session: &S) -> StyleFormat {
    let state = WordFormatting::format_state(session);
    let at = session
        .selection_range()
        .map(|range| range.normalized().0.paragraph)
        .unwrap_or(0);
    let paragraph = match query(
        session.word_document(),
        HeadlessQuery::Paragraph { paragraph: at },
    ) {
        Some(HeadlessQueryResult::Paragraph {
            value: Some(paragraph),
            ..
        }) => Some(paragraph),
        _ => None,
    };
    let run = WordFormatting::run_style_at(session, (at, 0));
    let spacing = paragraph
        .as_ref()
        .and_then(|paragraph| paragraph.spacing.clone());
    let indent = paragraph
        .as_ref()
        .and_then(|paragraph| paragraph.indent.clone());
    StyleFormat {
        font: state.font_family.clone(),
        east_asia_font: state.font_family.clone(),
        size: state.font_size,
        bold: Some(state.bold),
        italic: Some(state.italic),
        underline: Some(state.underline),
        color: run
            .and_then(|run| run.color)
            .map(|color| color.trim_start_matches('#').to_ascii_uppercase())
            .filter(|color| color.len() == 6 && color != "000000"),
        alignment: state.alignment.clone(),
        line_spacing: spacing
            .as_ref()
            .filter(|spacing| {
                spacing
                    .line_rule
                    .as_deref()
                    .is_none_or(|rule| rule == "auto")
            })
            .and_then(|spacing| spacing.line),
        space_before: spacing.as_ref().and_then(|spacing| spacing.before),
        space_after: spacing.as_ref().and_then(|spacing| spacing.after),
        indent_left: indent.as_ref().and_then(|indent| indent.left),
        first_line: indent.as_ref().and_then(|indent| {
            indent
                .first_line
                .or_else(|| indent.hanging.map(|hanging| -hanging))
        }),
        keep_next: None,
        outline_level: None,
        bottom_border: None,
    }
}

/// The style commands, for every session.
pub trait WordStyles: WordSession {
    /// Every style of the document, in its own order.
    fn style_catalog(&self) -> Vec<StyleInfo> {
        self.word_document()
            .related_part_xml("styles")
            .map(|xml| catalog_of(&xml))
            .unwrap_or_default()
    }

    /// Applies a style from the gallery or the pane the way Word does: a
    /// paragraph style to the paragraphs; a character style to the
    /// selection (or the word at the caret); a linked style to the
    /// selection's characters when it lies within one paragraph, and to the
    /// paragraphs otherwise.
    fn apply_style(&mut self, id: &str) -> Result<(), String> {
        let catalog = self.style_catalog();
        let Some(style) = catalog.iter().find(|style| style.id == id) else {
            // A built-in the document does not define yet: the engine adds
            // Word's definition.
            return WordFormatting::set_paragraph_style(self, Some(id));
        };
        let partial = self.selection_range().is_some_and(|range| {
            !range.is_collapsed() && range.anchor.paragraph == range.focus.paragraph
        });
        match style.kind {
            StyleKind::Character => self.apply_character_style(Some(id)),
            StyleKind::Linked if partial => {
                let link = style.link.clone().unwrap_or_else(|| id.to_string());
                self.apply_character_style(Some(&link))
            }
            StyleKind::Numbering => Err("a list style is applied from the list gallery".into()),
            StyleKind::Table => Err("a table style is applied from the table tools".into()),
            _ => WordFormatting::set_paragraph_style(self, Some(id)),
        }
    }

    /// A character style on the selection, or on the word at the caret;
    /// `None` takes it off (段落フォント).
    fn apply_character_style(&mut self, id: Option<&str>) -> Result<(), String> {
        let (start, end) = character_span(self).ok_or("select the text to format")?;
        self.apply_command(
            HeadlessCommand::SetCharacterStyle {
                start_paragraph: start.0,
                start_offset: start.1,
                end_paragraph: end.0,
                end_offset: end.1,
                style_id: id.map(str::to_string),
            },
            true,
        )
    }

    /// 新しいスタイル: adds the style and applies it to the selection, as
    /// Word's dialog does. Returns the new style's id.
    fn create_style(&mut self, new: &NewStyle) -> Result<String, String> {
        let current = self
            .word_document()
            .related_part_xml("styles")
            .unwrap_or_default();
        let mut probe = Document::parse(&current)
            .map(|document| document.root)
            .unwrap_or_else(|_| Element::new("w:styles"));
        let id = add_style(&mut probe, new)?;
        let then = match new.kind {
            StyleKind::Paragraph | StyleKind::Linked => {
                let (start, end) = paragraph_span(self);
                vec![HeadlessStep::boxed(vec![
                    HeadlessCommand::SetParagraphStyle {
                        start_paragraph: start,
                        end_paragraph: end,
                        style_id: Some(id.clone()),
                    },
                ])]
            }
            StyleKind::Character => match character_span(self) {
                Some((start, end)) => vec![HeadlessStep::boxed(vec![
                    HeadlessCommand::SetCharacterStyle {
                        start_paragraph: start.0,
                        start_offset: start.1,
                        end_paragraph: end.0,
                        end_offset: end.1,
                        style_id: Some(id.clone()),
                    },
                ])],
                None => Vec::new(),
            },
            _ => Vec::new(),
        };
        let new = new.clone();
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Styles, |styles| {
                    add_style(styles, &new).map(|_| ())
                })
            },
            then,
        )?;
        Ok(id)
    }

    /// スタイルの変更.
    fn modify_style(&mut self, id: &str, change: &StyleChange) -> Result<(), String> {
        let id = id.to_string();
        let change = change.clone();
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Styles, |styles| {
                    change_style(styles, &id, &change)
                })
            },
            Vec::new(),
        )
    }

    /// 選択個所と一致するように更新: the style takes the caret's format.
    fn update_style_to_match(&mut self, id: &str) -> Result<(), String> {
        let format = format_at_caret(self);
        let change = StyleChange {
            format,
            ..StyleChange::default()
        };
        self.modify_style(id, &change)
    }

    /// For a paragraph whose style updates itself (自動的に更新する): after
    /// the paragraph was formatted directly, the style takes the format, and
    /// so does every paragraph in it. Returns whether it did.
    fn auto_update_style(&mut self) -> Result<bool, String> {
        let style_id = WordFormatting::format_state(self).style_id;
        let Some(style_id) = style_id else {
            return Ok(false);
        };
        let auto = self
            .style_catalog()
            .iter()
            .any(|style| style.id == style_id && style.auto_update);
        if !auto {
            return Ok(false);
        }
        self.update_style_to_match(&style_id)?;
        Ok(true)
    }

    /// テンプレートとアドイン: attaches `template` (a .dotx or .docx, by its
    /// bytes and file name) and, with `update`, copies its styles into the
    /// document (文書のスタイルを自動的に更新する). Returns how many
    /// styles were copied.
    fn attach_template(
        &mut self,
        template: &[u8],
        path: &str,
        update: bool,
    ) -> Result<usize, String> {
        let source = Package::open(template)?;
        let source_main = source.main().to_string();
        let template_styles = source
            .related(&source_main, "styles")
            .map(|name| source.xml(&name))
            .transpose()?
            .map(|document| document.root);
        let path = path.to_string();
        let mut copied = 0usize;
        let counter = &mut copied;
        edit_package(
            self,
            |package| {
                let settings = package.part(PartKind::Settings)?;
                let id = package.relate(&settings, "attachedTemplate", &file_url(&path), true)?;
                let mut document = package.xml(&settings)?;
                if !document
                    .root
                    .attributes
                    .iter()
                    .any(|(key, _)| key == "xmlns:r")
                {
                    document.root.set_attr(
                        "xmlns:r",
                        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                    );
                }
                document.root.put(
                    Element::new("w:attachedTemplate").attr("r:id", &id),
                    super::paragraph_options::SETTINGS_ORDER,
                );
                package.put_xml(&settings, &document);
                if let (true, Some(template_styles)) = (update, template_styles.as_ref()) {
                    package.edit(PartKind::Styles, |styles| {
                        *counter = copy_template_styles(styles, template_styles);
                        Ok(())
                    })?;
                }
                Ok(())
            },
            Vec::new(),
        )?;
        Ok(copied)
    }
}

impl<T: WordSession + ?Sized> WordStyles for T {}

/// A `file://` URL for a template path, as Word writes `attachedTemplate`.
fn file_url(path: &str) -> String {
    if path.contains("://") {
        return path.to_string();
    }
    let encoded: String = path
        .chars()
        .map(|ch| match ch {
            ' ' => "%20".to_string(),
            ch => ch.to_string(),
        })
        .collect();
    format!("file://{encoded}")
}

/// The paragraphs the selection touches, or the caret's.
fn paragraph_span<S: WordSession + ?Sized>(session: &S) -> (usize, usize) {
    match session.selection_range() {
        Some(range) if !range.is_collapsed() => {
            let (start, end) = range.normalized();
            (start.paragraph, end.paragraph)
        }
        Some(range) => (range.focus.paragraph, range.focus.paragraph),
        None => (0, 0),
    }
}

/// The selection, or the word around a caret, as a character style takes it.
fn character_span<S: WordSession + ?Sized>(
    session: &S,
) -> Option<((usize, usize), (usize, usize))> {
    let range = session.selection_range()?;
    if !range.is_collapsed() {
        let (start, end) = range.normalized();
        return Some(((start.paragraph, start.offset), (end.paragraph, end.offset)));
    }
    let paragraph = range.focus.paragraph;
    let text = match query(
        session.word_document(),
        HeadlessQuery::ParagraphText { paragraph },
    )? {
        HeadlessQueryResult::ParagraphText { text, .. } => text?,
        _ => return None,
    };
    let mut offset = range.focus.offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_alphanumeric())
        .last()
        .map(|(index, _)| index)
        .unwrap_or(offset);
    let end = text[offset..]
        .char_indices()
        .take_while(|(_, ch)| ch.is_alphanumeric())
        .last()
        .map(|(index, ch)| offset + index + ch.len_utf8())
        .unwrap_or(offset);
    (start < end).then_some(((paragraph, start), (paragraph, end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="paragraph" w:styleId="1"><w:name w:val="heading 1"/><w:basedOn w:val="a"/><w:next w:val="a"/><w:link w:val="10"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="character" w:customStyle="1" w:styleId="10"><w:name w:val="見出し 1 (文字)"/><w:link w:val="1"/><w:rPr><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="a"/><w:uiPriority w:val="29"/><w:qFormat/></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:uiPriority w:val="10"/><w:qFormat/></w:style><w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:uiPriority w:val="22"/><w:qFormat/><w:rPr><w:b/></w:rPr></w:style></w:styles>"#;

    #[test]
    fn the_catalog_reads_kinds_links_and_priorities() {
        let catalog = catalog_of(STYLES);
        let heading = catalog.iter().find(|style| style.id == "1").unwrap();
        assert_eq!(heading.kind, StyleKind::Linked);
        assert_eq!(heading.priority, 9);
        assert_eq!(heading.format.size, Some(16.0));
        assert_eq!(heading.format.outline_level, Some(0));
        assert_eq!(
            catalog
                .iter()
                .find(|style| style.id == "Strong")
                .unwrap()
                .kind,
            StyleKind::Character
        );
        assert_eq!(display_name("heading 1", true), "見出し 1");
        assert_eq!(display_name("Normal", true), "標準");
    }

    #[test]
    fn the_gallery_follows_ui_priority() {
        let gallery = quick_style_gallery(&catalog_of(STYLES));
        let names: Vec<Option<&str>> = gallery
            .iter()
            .skip(4)
            .map(|entry| entry.name.as_deref())
            .collect();
        assert_eq!(names, [Some("Title"), Some("Strong"), Some("Quote")]);
    }

    #[test]
    fn styles_are_added_changed_and_based_on() {
        let mut document = Document::parse(STYLES).unwrap();
        let id = add_style(
            &mut document.root,
            &NewStyle {
                name: "My Heading".into(),
                kind: StyleKind::Linked,
                based_on: Some("1".into()),
                next: Some("a".into()),
                format: StyleFormat {
                    color: Some("C00000".into()),
                    space_before: Some(18.0),
                    ..StyleFormat::default()
                },
                quick: true,
                auto_update: true,
            },
        )
        .unwrap();
        assert_eq!(id, "MyHeading");
        assert!(add_style(
            &mut document.root,
            &NewStyle {
                name: "my heading".into(),
                kind: StyleKind::Paragraph,
                based_on: None,
                next: None,
                format: StyleFormat::default(),
                quick: false,
                auto_update: false,
            }
        )
        .is_err());
        change_style(
            &mut document.root,
            &id,
            &StyleChange {
                name: Some("Chapter".into()),
                based_on: Some(Some("Title".into())),
                format: StyleFormat {
                    bold: Some(true),
                    ..StyleFormat::default()
                },
                ..StyleChange::default()
            },
        )
        .unwrap();
        // Title is now this style's base: basing it on this style would
        // make a loop.
        assert!(change_style(
            &mut document.root,
            "Title",
            &StyleChange {
                based_on: Some(Some(id.clone())),
                ..StyleChange::default()
            }
        )
        .is_err());
        assert!(change_style(
            &mut document.root,
            &id,
            &StyleChange {
                based_on: Some(Some(id.clone())),
                ..StyleChange::default()
            }
        )
        .is_err());
        let catalog = catalog_of(&document.to_xml());
        let style = catalog.iter().find(|style| style.id == id).unwrap();
        assert_eq!(style.name, "Chapter");
        assert_eq!(style.kind, StyleKind::Linked);
        assert!(style.auto_update && style.quick);
        assert_eq!(style.format.bold, Some(true));
        assert_eq!(style.format.color.as_deref(), Some("C00000"));
        assert_eq!(style.format.space_before, Some(18.0));
        let character = catalog
            .iter()
            .find(|other| Some(&other.id) == style.link.as_ref())
            .unwrap();
        assert_eq!(character.format.bold, Some(true));
    }

    #[test]
    fn a_template_replaces_same_named_styles_under_the_documents_ids() {
        let mut document = Document::parse(STYLES).unwrap();
        let template = Document::parse(
            r#"<w:styles xmlns:w="urn:w"><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:color w:val="FF0000"/></w:rPr></w:style><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Memo"><w:name w:val="Memo"/><w:basedOn w:val="Heading1"/></w:style></w:styles>"#,
        )
        .unwrap();
        assert_eq!(copy_template_styles(&mut document.root, &template.root), 3);
        let catalog = catalog_of(&document.to_xml());
        let heading = catalog.iter().find(|style| style.id == "1").unwrap();
        assert_eq!(heading.format.color.as_deref(), Some("FF0000"));
        assert_eq!(heading.based_on.as_deref(), Some("a"));
        let memo = catalog.iter().find(|style| style.id == "Memo").unwrap();
        assert_eq!(memo.based_on.as_deref(), Some("1"));
    }
}
