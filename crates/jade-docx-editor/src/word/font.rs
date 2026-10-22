//! Word's character formatting beyond the ribbon's toggles, over any
//! [`WordSession`]: the フォント dialog (⌘D) with its フォント and 詳細設定
//! tabs, the ribbon's underline styles, theme colours and custom colours,
//! 文字の網かけ and 囲み線, 文字種の変換 (Change Case, 全角 / 半角) and the
//! 拡張書式 commands that are run formatting (縦中横, 組み文字, 割注, 文字の
//! 拡大/縮小).
//!
//! Every change is one `set_run_format` command — one undo step — carrying
//! only what the dialog changed, so a property the user left alone keeps
//! whatever each run had.

use crate::commands::set_run_format::run_format_of;
use jade_docx_headless_api::HeadlessCommand;
pub use jade_docx_headless_api::{
    HeadlessChangeCaseMode, HeadlessEastAsianLayout, HeadlessRunFormat, HeadlessThemeColor,
};

use super::{Helpers, Position, WordSession};

/// The dialog's two tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontTab {
    /// フォント: fonts, style, size, colour, underline, emphasis, effects.
    Font,
    /// 詳細設定: scale, spacing, position, kerning, OpenType features.
    Advanced,
}

/// Word's underline styles (下線のスタイル), as ST_Underline values, in the
/// order Word for Mac lists them.
pub const UNDERLINE_STYLES: [&str; 18] = [
    "none",
    "single",
    "words",
    "double",
    "thick",
    "dotted",
    "dottedHeavy",
    "dash",
    "dashedHeavy",
    "dashLong",
    "dashLongHeavy",
    "dotDash",
    "dashDotHeavy",
    "dotDotDash",
    "dashDotDotHeavy",
    "wave",
    "wavyHeavy",
    "wavyDouble",
];

/// 蛍光ペンの色: every `ST_HighlightColor` value (ECMA-376 Part 1
/// §17.18.40) but `none`, as `w:highlight` names it, with the colour Word
/// draws for it. Word's own palette leaves out white; it is offered so a
/// highlight that hides text on a coloured page can be set too.
pub const HIGHLIGHT_COLORS: [(&str, u32); 16] = [
    ("yellow", 0xffff00),
    ("green", 0x00ff00),
    ("cyan", 0x00ffff),
    ("magenta", 0xff00ff),
    ("blue", 0x0000ff),
    ("red", 0xff0000),
    ("darkBlue", 0x000080),
    ("darkCyan", 0x008080),
    ("darkGreen", 0x008000),
    ("darkMagenta", 0x800080),
    ("darkRed", 0x800000),
    ("darkYellow", 0x808000),
    ("darkGray", 0x808080),
    ("lightGray", 0xc0c0c0),
    ("black", 0x000000),
    ("white", 0xffffff),
];

/// 傍点: Word's emphasis marks (ST_Em).
pub const EMPHASIS_MARKS: [&str; 5] = ["none", "dot", "comma", "circle", "underDot"];

/// 倍率: the scales Word's 詳細設定 tab and 文字の拡大/縮小 offer, percent.
pub const SCALES: [f32; 8] = [200.0, 150.0, 100.0, 90.0, 80.0, 66.0, 50.0, 33.0];

/// 合字: the ligature settings the 詳細設定 tab offers.
pub const LIGATURES: [&str; 5] = [
    "none",
    "standard",
    "standardContextual",
    "historicalDiscretional",
    "all",
];

/// 数字の間隔 and 数字の書式.
pub const NUMBER_SPACINGS: [&str; 3] = ["default", "proportional", "tabular"];
pub const NUMBER_FORMS: [&str; 3] = ["default", "lining", "oldStyle"];

/// The sizes the dialog's サイズ list offers, as the ribbon's.
pub const FONT_SIZES: [f32; 21] = [
    5.0, 5.5, 6.5, 7.5, 8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0,
    28.0, 36.0, 48.0, 72.0,
];

/// 標準の色: the row of standard colours under the theme colours.
pub const STANDARD_COLORS: [&str; 10] = [
    "C00000", "FF0000", "FFC000", "FFFF00", "92D050", "00B050", "00B0F0", "0070C0", "002060",
    "7030A0",
];

/// The Japanese families every Mac has, for when the shell cannot list
/// installed fonts (and to put them first when it can).
pub const JAPANESE_FONTS: [&str; 16] = [
    "游明朝",
    "游ゴシック",
    "Yu Mincho",
    "Yu Gothic",
    "YuMincho",
    "YuGothic",
    "ＭＳ 明朝",
    "ＭＳ ゴシック",
    "MS Mincho",
    "MS Gothic",
    "Hiragino Mincho ProN",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Hiragino Maru Gothic ProN",
    "Osaka",
    "Noto Sans JP",
];

/// Latin families every Mac has.
pub const LATIN_FONTS: [&str; 10] = [
    "Century",
    "Times New Roman",
    "Arial",
    "Helvetica",
    "Helvetica Neue",
    "Georgia",
    "Verdana",
    "Courier New",
    "Menlo",
    "Calibri",
];

/// Whether `family` is a font for Japanese text: what the 日本語用のフォント
/// list holds.
pub fn is_japanese_font(family: &str) -> bool {
    crate::commands::set_font_family::font_family_targets_east_asia(family)
        || family.starts_with("Hiragino")
        || family.starts_with("ヒラギノ")
        || family.starts_with("Osaka")
        || family.starts_with("Toppan")
        || family.starts_with("Klee")
        || family.starts_with("Tsukushi")
        || family.starts_with("BIZ UD")
        || family.starts_with("Noto Sans JP")
        || family.starts_with("Noto Serif JP")
}

/// The two font lists, from the families the system has (`installed`): the
/// Japanese ones for 日本語用のフォント, every one for 英数字用のフォント —
/// Word lists Japanese fonts in its Latin list too. The common Mac families
/// come first when installed, or always when nothing is.
pub fn font_lists(installed: &[String]) -> (Vec<String>, Vec<String>) {
    let mut names: Vec<String> = installed
        .iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .collect();
    names.sort();
    names.dedup();
    let first = |preferred: &[&str], names: &[String]| -> Vec<String> {
        let mut list: Vec<String> = preferred
            .iter()
            .filter(|name| names.is_empty() || names.iter().any(|known| known == *name))
            .map(|name| name.to_string())
            .collect();
        for name in names {
            if !list.contains(name) {
                list.push(name.clone());
            }
        }
        list
    };
    let japanese: Vec<String> = names
        .iter()
        .filter(|name| is_japanese_font(name))
        .cloned()
        .collect();
    (
        first(&JAPANESE_FONTS, &japanese),
        first(&LATIN_FONTS, &names),
    )
}

/// One swatch of the テーマの色 palette: what it looks like and what it
/// writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeSwatch {
    /// `RRGGBB`, the colour drawn and written as `w:val`.
    pub color: String,
    pub theme: HeadlessThemeColor,
}

/// The palette's ten columns: Word's theme slots as `w:themeColor` names
/// them, with the slot of the theme part each reads.
pub const THEME_COLUMNS: [(&str, &str); 10] = [
    ("background1", "lt1"),
    ("text1", "dk1"),
    ("background2", "lt2"),
    ("text2", "dk2"),
    ("accent1", "accent1"),
    ("accent2", "accent2"),
    ("accent3", "accent3"),
    ("accent4", "accent4"),
    ("accent5", "accent5"),
    ("accent6", "accent6"),
];

/// Office's default theme colours, for a document without a theme part.
const OFFICE_THEME: [&str; 10] = [
    "FFFFFF", "000000", "E7E6E6", "44546A", "4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5",
    "70AD47",
];

fn parse_hex(color: &str) -> Option<(f32, f32, f32)> {
    let hex = color.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some((
        channel(0)? as f32 / 255.0,
        channel(2)? as f32 / 255.0,
        channel(4)? as f32 / 255.0,
    ))
}

fn to_hex((r, g, b): (f32, f32, f32)) -> String {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b))
}

fn rgb_to_hsl((r, g, b): (f32, f32, f32)) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let delta = max - min;
    let saturation = if lightness > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let hue = if max == r {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } / 6.0;
    (hue, saturation, lightness)
}

fn hsl_to_rgb((hue, saturation, lightness): (f32, f32, f32)) -> (f32, f32, f32) {
    if saturation <= 0.0 {
        return (lightness, lightness, lightness);
    }
    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;
    let channel = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (
        channel(hue + 1.0 / 3.0),
        channel(hue),
        channel(hue - 1.0 / 3.0),
    )
}

/// `color` with Word's `w:themeTint` (`tint`, 0–255) applied: its
/// luminance moved toward white, `L·t + (1 − t)`.
pub fn tinted(color: &str, tint: u8) -> String {
    let Some(rgb) = parse_hex(color) else {
        return color.to_string();
    };
    let (h, s, l) = rgb_to_hsl(rgb);
    let t = tint as f32 / 255.0;
    to_hex(hsl_to_rgb((h, s, l * t + (1.0 - t))))
}

/// `color` with Word's `w:themeShade` (`shade`, 0–255) applied: its
/// luminance scaled toward black.
pub fn shaded(color: &str, shade: u8) -> String {
    let Some(rgb) = parse_hex(color) else {
        return color.to_string();
    };
    let (h, s, l) = rgb_to_hsl(rgb);
    to_hex(hsl_to_rgb((h, s, l * shade as f32 / 255.0)))
}

/// The variations under each base colour, top to bottom, as Word for Mac
/// shows them: lighter ones as tints, darker ones as shades. White's
/// column only darkens and black's only lightens.
fn variations(column: usize, base: &str) -> [(Option<u8>, Option<u8>); 5] {
    let lightness = parse_hex(base).map_or(0.5, |rgb| rgb_to_hsl(rgb).2);
    if column == 0 || lightness > 0.9 {
        // 5 %, 15 %, 25 %, 35 %, 50 % darker.
        [
            (None, Some(0xF2)),
            (None, Some(0xD9)),
            (None, Some(0xBF)),
            (None, Some(0xA6)),
            (None, Some(0x80)),
        ]
    } else if column == 1 || lightness < 0.1 {
        // 50 %, 35 %, 25 %, 15 %, 5 % lighter.
        [
            (Some(0x80), None),
            (Some(0xA6), None),
            (Some(0xBF), None),
            (Some(0xD9), None),
            (Some(0xF2), None),
        ]
    } else {
        // 80 %, 60 %, 40 % lighter; 25 %, 50 % darker.
        [
            (Some(0x33), None),
            (Some(0x66), None),
            (Some(0x99), None),
            (None, Some(0xBF)),
            (None, Some(0x80)),
        ]
    }
}

/// The テーマの色 palette: six rows of ten — the theme's colours, then their
/// lighter and darker variations. `theme_colors` maps the theme part's slots
/// (`dk2`, `accent1`, …) to `RRGGBB`; missing ones take Office's.
pub fn theme_palette(theme_colors: &[(String, String)]) -> Vec<Vec<ThemeSwatch>> {
    let base: Vec<String> = THEME_COLUMNS
        .iter()
        .enumerate()
        .map(|(index, (_, slot))| {
            theme_colors
                .iter()
                .find(|(name, value)| name == slot && parse_hex(value).is_some())
                .map(|(_, value)| value.trim_start_matches('#').to_ascii_uppercase())
                .unwrap_or_else(|| OFFICE_THEME[index].to_string())
        })
        .collect();
    let mut rows = vec![THEME_COLUMNS
        .iter()
        .zip(&base)
        .map(|((name, _), color)| ThemeSwatch {
            color: color.clone(),
            theme: HeadlessThemeColor {
                name: name.to_string(),
                tint: None,
                shade: None,
            },
        })
        .collect::<Vec<_>>()];
    for row in 0..5 {
        rows.push(
            THEME_COLUMNS
                .iter()
                .zip(&base)
                .enumerate()
                .map(|(column, ((name, _), color))| {
                    let (tint, shade) = variations(column, color)[row];
                    let drawn = match (tint, shade) {
                        (Some(tint), _) => tinted(color, tint),
                        (_, Some(shade)) => shaded(color, shade),
                        _ => color.clone(),
                    };
                    ThemeSwatch {
                        color: drawn,
                        theme: HeadlessThemeColor {
                            name: name.to_string(),
                            tint: tint.map(|value| format!("{value:02X}")),
                            shade: shade.map(|value| format!("{value:02X}")),
                        },
                    }
                })
                .collect(),
        );
    }
    rows
}

/// The fields of [`HeadlessRunFormat`] that differ between two snapshots:
/// what the dialog changed.
pub fn format_change(before: &HeadlessRunFormat, after: &HeadlessRunFormat) -> HeadlessRunFormat {
    let mut change = HeadlessRunFormat::default();
    macro_rules! diff {
        ($($field:ident),* $(,)?) => {
            $(
                if after.$field != before.$field && after.$field.is_some() {
                    change.$field = after.$field.clone();
                }
            )*
        };
    }
    diff!(
        latin_font,
        east_asia_font,
        complex_font,
        font_size,
        complex_font_size,
        bold,
        italic,
        complex_bold,
        complex_italic,
        underline,
        underline_color,
        strike,
        double_strike,
        vertical_align,
        caps,
        small_caps,
        hidden,
        emphasis,
        shading,
        border,
        scale,
        spacing,
        position,
        kerning,
        outline,
        shadow,
        emboss,
        engrave,
        ligatures,
        number_spacing,
        number_form,
        stylistic_set,
        contextual_alternates,
        glow_color,
        glow_radius,
        text_outline_color,
        text_outline_width,
        east_asian_layout,
    );
    // A colour travels with its theme reference.
    if after.color != before.color || after.theme_color != before.theme_color {
        change.color = after.color.clone();
        change.theme_color = after.theme_color.clone();
    }
    change
}

/// Fills in what a run without direct formatting shows: Word's dialog
/// shows every control set, not blank.
fn with_defaults(mut format: HeadlessRunFormat) -> HeadlessRunFormat {
    format.bold.get_or_insert(false);
    format.italic.get_or_insert(false);
    format.underline.get_or_insert_with(|| "none".to_string());
    format.strike.get_or_insert(false);
    format.double_strike.get_or_insert(false);
    format
        .vertical_align
        .get_or_insert_with(|| "baseline".to_string());
    format.color.get_or_insert_with(|| "auto".to_string());
    format.caps.get_or_insert(false);
    format.small_caps.get_or_insert(false);
    format.hidden.get_or_insert(false);
    format.emphasis.get_or_insert_with(|| "none".to_string());
    format.shading.get_or_insert_with(|| "none".to_string());
    format.border.get_or_insert(false);
    format.scale.get_or_insert(100.0);
    format.spacing.get_or_insert(0.0);
    format.position.get_or_insert(0.0);
    format.kerning.get_or_insert(0.0);
    format.outline.get_or_insert(false);
    format.shadow.get_or_insert(false);
    format.emboss.get_or_insert(false);
    format.engrave.get_or_insert(false);
    format.ligatures.get_or_insert_with(|| "none".to_string());
    format
        .number_spacing
        .get_or_insert_with(|| "default".to_string());
    format
        .number_form
        .get_or_insert_with(|| "default".to_string());
    format.stylistic_set.get_or_insert(0);
    format.contextual_alternates.get_or_insert(false);
    format
}

/// The フォント dialog while it is open: what the selection had and what
/// the controls now say.
#[derive(Clone, Debug, PartialEq)]
pub struct FontDialog {
    pub tab: FontTab,
    /// The selection's formatting when the dialog opened.
    pub original: HeadlessRunFormat,
    /// The controls.
    pub format: HeadlessRunFormat,
    /// Text typed to narrow the font lists.
    pub filter: String,
}

impl FontDialog {
    pub fn open(format: HeadlessRunFormat) -> Self {
        let format = with_defaults(format);
        Self {
            tab: FontTab::Font,
            original: format.clone(),
            format,
            filter: String::new(),
        }
    }

    /// What OK applies: only what the controls changed.
    pub fn change(&self) -> HeadlessRunFormat {
        format_change(&self.original, &self.format)
    }

    /// スタイル: Word's four: 標準, 斜体, 太字, 太字 斜体.
    pub fn set_style(&mut self, bold: bool, italic: bool) {
        self.format.bold = Some(bold);
        self.format.italic = Some(italic);
    }

    /// The 文字飾り check boxes, with the pairs Word keeps exclusive.
    pub fn set_effect(&mut self, effect: Effect, on: bool) {
        let format = &mut self.format;
        match effect {
            Effect::Strike => {
                format.strike = Some(on);
                if on {
                    format.double_strike = Some(false);
                }
            }
            Effect::DoubleStrike => {
                format.double_strike = Some(on);
                if on {
                    format.strike = Some(false);
                }
            }
            Effect::Superscript => {
                format.vertical_align = Some(if on { "superscript" } else { "baseline" }.into())
            }
            Effect::Subscript => {
                format.vertical_align = Some(if on { "subscript" } else { "baseline" }.into())
            }
            Effect::SmallCaps => {
                format.small_caps = Some(on);
                if on {
                    format.caps = Some(false);
                }
            }
            Effect::AllCaps => {
                format.caps = Some(on);
                if on {
                    format.small_caps = Some(false);
                }
            }
            Effect::Hidden => format.hidden = Some(on),
            Effect::Outline => {
                format.outline = Some(on);
                if on {
                    format.emboss = Some(false);
                    format.engrave = Some(false);
                }
            }
            Effect::Shadow => {
                format.shadow = Some(on);
                if on {
                    format.emboss = Some(false);
                    format.engrave = Some(false);
                }
            }
            Effect::Emboss => {
                format.emboss = Some(on);
                if on {
                    format.engrave = Some(false);
                    format.outline = Some(false);
                    format.shadow = Some(false);
                }
            }
            Effect::Engrave => {
                format.engrave = Some(on);
                if on {
                    format.emboss = Some(false);
                    format.outline = Some(false);
                    format.shadow = Some(false);
                }
            }
        }
    }

    /// Whether a 文字飾り box is checked.
    pub fn effect(&self, effect: Effect) -> bool {
        let format = &self.format;
        match effect {
            Effect::Strike => format.strike == Some(true),
            Effect::DoubleStrike => format.double_strike == Some(true),
            Effect::Superscript => format.vertical_align.as_deref() == Some("superscript"),
            Effect::Subscript => format.vertical_align.as_deref() == Some("subscript"),
            Effect::SmallCaps => format.small_caps == Some(true),
            Effect::AllCaps => format.caps == Some(true),
            Effect::Hidden => format.hidden == Some(true),
            Effect::Outline => format.outline == Some(true),
            Effect::Shadow => format.shadow == Some(true),
            Effect::Emboss => format.emboss == Some(true),
            Effect::Engrave => format.engrave == Some(true),
        }
    }

    /// 文字間隔: Word's 標準 / 広く / 狭く with the 間隔 in points.
    pub fn spacing(&self) -> f32 {
        self.format.spacing.unwrap_or(0.0)
    }

    pub fn step_spacing(&mut self, points: f32) {
        let next = ((self.spacing() + points) * 10.0).round() / 10.0;
        self.format.spacing = Some(next.clamp(-63.0, 1584.0));
    }

    /// 位置: 標準 / 上げる / 下げる with the 間隔 in points.
    pub fn position(&self) -> f32 {
        self.format.position.unwrap_or(0.0)
    }

    pub fn step_position(&mut self, points: f32) {
        let next = ((self.position() + points) * 2.0).round() / 2.0;
        self.format.position = Some(next.clamp(-1584.0, 1584.0));
    }

    /// カーニングを行う, and from what size.
    pub fn kerning(&self) -> Option<f32> {
        self.format.kerning.filter(|points| *points > 0.0)
    }

    pub fn set_kerning(&mut self, on: bool) {
        self.format.kerning = Some(if on {
            self.format.font_size.unwrap_or(10.5).max(1.0)
        } else {
            0.0
        });
    }

    pub fn step_kerning(&mut self, points: f32) {
        if let Some(kerning) = self.kerning() {
            self.format.kerning = Some((kerning + points).clamp(1.0, 1638.0));
        }
    }

    pub fn step_size(&mut self, grow: bool) {
        let size = self.format.font_size.unwrap_or(10.5);
        let next = if grow {
            FONT_SIZES.iter().copied().find(|step| *step > size + 0.01)
        } else {
            FONT_SIZES
                .iter()
                .rev()
                .copied()
                .find(|step| *step < size - 0.01)
        };
        self.format.font_size = Some(next.unwrap_or(size));
    }

    /// What the preview box says: the fonts' names, as Word's does.
    pub fn preview_text(&self) -> String {
        let japanese = self.format.east_asia_font.as_deref().unwrap_or("游明朝");
        let latin = self.format.latin_font.as_deref().unwrap_or("Century");
        format!("あア亜Ay123 {japanese} / {latin}")
    }
}

/// 文字飾り and 文字の効果.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Strike,
    DoubleStrike,
    Superscript,
    Subscript,
    SmallCaps,
    AllCaps,
    Hidden,
    Outline,
    Shadow,
    Emboss,
    Engrave,
}

impl Effect {
    /// The フォント tab's 文字飾り, in Word for Mac's order.
    pub const DECORATIONS: [Effect; 7] = [
        Effect::Strike,
        Effect::DoubleStrike,
        Effect::Superscript,
        Effect::Subscript,
        Effect::SmallCaps,
        Effect::AllCaps,
        Effect::Hidden,
    ];
    /// The 文字の効果 Word keeps from its older dialogs.
    pub const TEXT_EFFECTS: [Effect; 4] = [
        Effect::Outline,
        Effect::Shadow,
        Effect::Emboss,
        Effect::Engrave,
    ];
}

/// 拡張書式's run commands.
#[derive(Clone, Debug, PartialEq)]
pub enum AsianLayout {
    /// 縦中横: horizontal in vertical text, fitted to the line or not.
    Vertical { compress: bool },
    /// 組み文字: two lines in one character's height.
    Combine,
    /// 割注: two lines in one, in brackets (`round`, `square`, `angle`,
    /// `curly`, or `none`).
    WarichuBrackets(String),
    /// 解除: the run back to plain.
    Remove,
}

impl AsianLayout {
    fn layout(&self) -> HeadlessEastAsianLayout {
        match self {
            AsianLayout::Vertical { compress } => HeadlessEastAsianLayout {
                vert: true,
                vert_compress: *compress,
                ..Default::default()
            },
            AsianLayout::Combine => HeadlessEastAsianLayout {
                combine: true,
                ..Default::default()
            },
            AsianLayout::WarichuBrackets(brackets) => HeadlessEastAsianLayout {
                combine: true,
                combine_brackets: Some(brackets.clone()),
                ..Default::default()
            },
            AsianLayout::Remove => HeadlessEastAsianLayout::default(),
        }
    }
}

/// The commands, for every session.
pub trait WordCharacters: WordSession {
    /// The formatting of the run at the caret, or at the start of the
    /// selection: what the dialog opens on.
    fn run_format_at(&self, (paragraph, offset): Position) -> HeadlessRunFormat {
        let Some(paragraph) = self.word_document().paragraph(paragraph) else {
            return HeadlessRunFormat::default();
        };
        let length: usize = paragraph.runs.iter().map(|run| run.text.len()).sum();
        let target = offset.min(length.saturating_sub(1));
        let mut start = 0;
        let mut last = None;
        for run in &paragraph.runs {
            let end = start + run.text.len();
            if !run.text.is_empty() {
                last = Some(run);
                if target < end {
                    break;
                }
            }
            start = end;
        }
        last.and_then(|run| run.properties.as_ref())
            .map(run_format_of)
            .unwrap_or_default()
    }

    /// What the フォント dialog opens on.
    fn character_format(&self) -> HeadlessRunFormat {
        self.run_format_at(self.format_anchor())
    }

    /// Opens the フォント dialog on the selection.
    fn font_dialog(&self) -> FontDialog {
        FontDialog::open(self.character_format())
    }

    /// Applies a patch to the selection — or the word at a bare caret — as
    /// one undo step. An empty patch changes nothing.
    fn apply_run_format(&mut self, format: HeadlessRunFormat) -> Result<(), String> {
        if format.is_empty() {
            return Ok(());
        }
        self.run_on_span(|start, end| HeadlessCommand::SetRunFormat {
            start_paragraph: start.0,
            start_offset: start.1,
            end_paragraph: end.0,
            end_offset: end.1,
            format: Box::new(format),
        })
    }

    /// The dialog's OK.
    fn apply_font_dialog(&mut self, dialog: &FontDialog) -> Result<(), String> {
        self.apply_run_format(dialog.change())
    }

    /// The ribbon's underline list: a style, or `none`.
    fn set_underline_style(&mut self, kind: &str) -> Result<(), String> {
        self.apply_run_format(HeadlessRunFormat {
            underline: Some(kind.to_string()),
            ..Default::default()
        })
    }

    /// 下線の色: `RRGGBB` or `auto`. An underline is put on when there is
    /// none, as Word does.
    fn set_underline_color(&mut self, color: &str) -> Result<(), String> {
        let underlined = self
            .character_format()
            .underline
            .is_some_and(|kind| kind != "none");
        self.apply_run_format(HeadlessRunFormat {
            underline: (!underlined).then(|| "single".to_string()),
            underline_color: Some(color.trim_start_matches('#').to_string()),
            ..Default::default()
        })
    }

    /// A テーマの色 swatch: the colour it draws, with its theme reference.
    fn set_theme_font_color(&mut self, swatch: &ThemeSwatch) -> Result<(), String> {
        self.apply_run_format(HeadlessRunFormat {
            color: Some(swatch.color.clone()),
            theme_color: Some(swatch.theme.clone()),
            ..Default::default()
        })
    }

    /// 文字の網かけ: Word's button lays a 15 % grey; again takes it off.
    fn toggle_character_shading(&mut self) -> Result<(), String> {
        let shaded = self
            .character_format()
            .shading
            .is_some_and(|fill| fill != "none");
        self.apply_run_format(HeadlessRunFormat {
            shading: Some(if shaded { "none" } else { "D9D9D9" }.to_string()),
            ..Default::default()
        })
    }

    /// 囲み線: a box round the text, or none.
    fn toggle_character_border(&mut self) -> Result<(), String> {
        let boxed = self.character_format().border == Some(true);
        self.apply_run_format(HeadlessRunFormat {
            border: Some(!boxed),
            ..Default::default()
        })
    }

    /// 文字の拡大/縮小: the width in percent.
    fn set_character_scale(&mut self, percent: f32) -> Result<(), String> {
        self.apply_run_format(HeadlessRunFormat {
            scale: Some(percent),
            ..Default::default()
        })
    }

    /// 縦中横, 組み文字, 割注, or none of them.
    fn set_asian_layout(&mut self, layout: AsianLayout) -> Result<(), String> {
        self.apply_run_format(HeadlessRunFormat {
            east_asian_layout: Some(layout.layout()),
            ..Default::default()
        })
    }

    /// 文字種の変換: changes the selection's text, keeping its runs'
    /// formatting. Needs a selection, as Word's does.
    fn change_case(&mut self, mode: HeadlessChangeCaseMode) -> Result<(), String> {
        let range = self
            .selection_range()
            .filter(|range| !range.is_collapsed())
            .ok_or_else(|| "select the text to change".to_string())?;
        let (start, end) = range.normalized();
        self.apply_command(
            HeadlessCommand::ChangeCase {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                mode,
            },
            false,
        )
    }
}

impl<T: WordSession + ?Sized> WordCharacters for T {}

#[cfg(test)]
mod tests {
    #[test]
    fn the_highlight_palette_offers_every_st_highlight_color() {
        let names: Vec<_> = super::HIGHLIGHT_COLORS
            .iter()
            .map(|(name, _)| *name)
            .collect();
        for value in [
            "black",
            "blue",
            "cyan",
            "green",
            "magenta",
            "red",
            "yellow",
            "white",
            "darkBlue",
            "darkCyan",
            "darkGreen",
            "darkMagenta",
            "darkRed",
            "darkYellow",
            "darkGray",
            "lightGray",
        ] {
            assert!(names.contains(&value), "{value} cannot be chosen");
        }
    }

    use super::*;

    #[test]
    fn the_theme_palette_tints_and_shades_as_word_writes_them() {
        let palette = theme_palette(&[("accent1".to_string(), "4472C4".to_string())]);
        assert_eq!(palette.len(), 6);
        assert!(palette.iter().all(|row| row.len() == 10));
        let accent = &palette[0][4];
        assert_eq!(accent.color, "4472C4");
        assert_eq!(accent.theme.name, "accent1");
        // 80 % lighter is tint 33 and draws Word's D9E2F3 (give or take a
        // rounding step); 50 % darker is shade 80.
        let lighter = &palette[1][4];
        assert_eq!(lighter.theme.tint.as_deref(), Some("33"));
        let (r, g, b) = parse_hex(&lighter.color).unwrap();
        let (wr, wg, wb) = parse_hex("D9E2F3").unwrap();
        assert!((r - wr).abs() < 0.02 && (g - wg).abs() < 0.02 && (b - wb).abs() < 0.02);
        assert_eq!(palette[5][4].theme.shade.as_deref(), Some("80"));
        // White only darkens, black only lightens.
        assert_eq!(palette[1][0].theme.shade.as_deref(), Some("F2"));
        assert_eq!(palette[1][1].theme.tint.as_deref(), Some("80"));
        assert_eq!(palette[1][1].color, "7F7F7F", "Word writes 7F7F7F");
    }

    #[test]
    fn the_dialog_applies_only_what_changed() {
        let mut dialog = FontDialog::open(HeadlessRunFormat {
            bold: Some(true),
            font_size: Some(10.5),
            ..Default::default()
        });
        assert!(dialog.change().is_empty());
        dialog.set_effect(Effect::DoubleStrike, true);
        dialog.format.emphasis = Some("dot".into());
        dialog.step_spacing(1.5);
        let change = dialog.change();
        assert_eq!(change.double_strike, Some(true));
        assert_eq!(change.emphasis.as_deref(), Some("dot"));
        assert_eq!(change.spacing, Some(1.5));
        // Untouched: the bold and the size stay each run's own.
        assert_eq!(change.bold, None);
        assert_eq!(change.font_size, None);
        // Exclusive pairs, as Word keeps them.
        dialog.set_effect(Effect::Strike, true);
        assert!(!dialog.effect(Effect::DoubleStrike));
        dialog.set_effect(Effect::Emboss, true);
        dialog.set_effect(Effect::Engrave, true);
        assert!(!dialog.effect(Effect::Emboss));
    }

    #[test]
    fn the_font_lists_put_japanese_fonts_in_their_own_list() {
        let installed = [
            "Arial",
            "Hiragino Mincho ProN",
            "ヒラギノ角ゴシック",
            "Times New Roman",
            ".SF NS",
        ]
        .map(String::from);
        let (japanese, latin) = font_lists(&installed);
        assert!(japanese.contains(&"Hiragino Mincho ProN".to_string()));
        assert!(!japanese.contains(&"Arial".to_string()));
        assert!(latin.contains(&"Arial".to_string()));
        assert!(!latin.iter().any(|name| name.starts_with('.')));
        assert_eq!(latin[0], "Times New Roman", "Word's common faces first");
        // Without a list from the system, Word's usual families.
        let (japanese, _) = font_lists(&[]);
        assert_eq!(japanese[0], "游明朝");
    }
}
