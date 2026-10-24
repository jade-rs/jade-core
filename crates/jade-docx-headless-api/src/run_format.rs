//! Character formatting as Word's フォント dialog (⌘D) edits it: its フォント
//! tab — the Japanese and Latin fonts, style, size, colour, underline,
//! emphasis mark and effects — and its 詳細設定 tab — scale, spacing,
//! position, kerning and the OpenType features — plus the shading, border
//! and Asian-layout properties the ribbon and 拡張書式 set.
//!
//! The same struct is a snapshot (what a run has) and a patch (what a
//! `set_run_format` command changes): a field left `None` is left alone.

use serde::{Deserialize, Serialize};

/// A theme colour reference (`w:color/@w:themeColor`, ECMA-376 Part 1
/// §17.3.2.6) with its tint or shade, as Word writes for a swatch of the
/// テーマの色 palette.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessThemeColor {
    /// ST_ThemeColor: `text1`, `background1`, `text2`, `background2`,
    /// `accent1` … `accent6`, `hyperlink`, `followedHyperlink`.
    pub name: String,
    /// `w:themeTint`, a hex byte (`99` lightens to 60 %).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<String>,
    /// `w:themeShade`, a hex byte (`BF` darkens to 75 %).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade: Option<String>,
}

/// Word's East Asian layout of a run (`w:eastAsianLayout`, ECMA-376 Part 1
/// §17.3.2.10): 組み文字 / 割注 (`combine`) and 縦中横 (`vert`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessEastAsianLayout {
    /// Two lines in one (組み文字, and 割注 with brackets).
    #[serde(default)]
    pub combine: bool,
    /// The brackets around combined lines: `none`, `round`, `square`,
    /// `angle`, `curly`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combine_brackets: Option<String>,
    /// Horizontal in vertical text (縦中横).
    #[serde(default)]
    pub vert: bool,
    /// 縦中横's 行の幅に合わせる: compress to the line's width.
    #[serde(default)]
    pub vert_compress: bool,
}

impl HeadlessEastAsianLayout {
    pub fn is_empty(&self) -> bool {
        !self.combine && !self.vert
    }
}

/// Character formatting: a snapshot of a run, or a patch to apply.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessRunFormat {
    /// 英数字用のフォント: `w:rFonts/@w:ascii` and `@w:hAnsi`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latin_font: Option<String>,
    /// 日本語用のフォント: `w:rFonts/@w:eastAsia`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub east_asia_font: Option<String>,
    /// The complex-script font: `w:rFonts/@w:cs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complex_font: Option<String>,
    /// Points (`w:sz`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    /// Points (`w:szCs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complex_font_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// `w:bCs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complex_bold: Option<bool>,
    /// `w:iCs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complex_italic: Option<bool>,
    /// ST_Underline (`single`, `words`, `double`, `thick`, `dotted`, `dash`,
    /// `dotDash`, `dotDotDash`, `wave`, `wavyDouble`, …); `none` takes the
    /// underline off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    /// The underline's own colour, `RRGGBB` or `auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    /// `w:dstrike`. Turning it on turns single strike off, as Word does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_strike: Option<bool>,
    /// `baseline`, `superscript` or `subscript` (`w:vertAlign`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<String>,
    /// The text colour, `RRGGBB` or `auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The theme colour `color` is a swatch of. In a patch it travels with
    /// `color`: a colour set without one is a plain colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<HeadlessThemeColor>,
    /// すべて大文字 (`w:caps`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<bool>,
    /// 小型英大文字 (`w:smallCaps`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_caps: Option<bool>,
    /// 隠し文字 (`w:vanish`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// 傍点 (ST_Em): `none`, `dot`, `comma`, `circle`, `underDot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emphasis: Option<String>,
    /// 文字の網かけ: the fill, `RRGGBB`; `none` takes it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shading: Option<String>,
    /// 囲み線: a single ½ pt box round the run, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<bool>,
    /// 倍率: the width in percent (`w:w`), 100 normal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    /// 文字間隔: points added after each character (`w:spacing`), negative
    /// to condense; 0 is normal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f32>,
    /// 位置: points the baseline is raised (`w:position`), negative to lower.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<f32>,
    /// カーニングを行う: the smallest size in points kerned (`w:kern`); 0
    /// turns kerning off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kerning: Option<f32>,
    /// 中抜き (`w:outline`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<bool>,
    /// 影付き (`w:shadow`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    /// 浮き出し (`w:emboss`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emboss: Option<bool>,
    /// 浮き彫り (`w:imprint`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engrave: Option<bool>,
    /// 合字 (`w14:ligatures`): `none`, `standard`, `contextual`,
    /// `historical`, `discretional`, `standardContextual`, `all`, ….
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ligatures: Option<String>,
    /// 数字の間隔 (`w14:numSpacing`): `default`, `proportional`, `tabular`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_spacing: Option<String>,
    /// 数字の書式 (`w14:numForm`): `default`, `lining`, `oldStyle`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_form: Option<String>,
    /// スタイルセット (`w14:stylisticSets`): 1–20, 0 for the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stylistic_set: Option<u8>,
    /// コンテキスト代替を使用する (`w14:cntxtAlts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contextual_alternates: Option<bool>,
    /// 光彩 (`w14:glow`): its colour `RRGGBB`, `none` to take it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow_color: Option<String>,
    /// 光彩's size in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow_radius: Option<f32>,
    /// 文字の輪郭 (`w14:textOutline`): the line's colour `RRGGBB`, `none` to
    /// take it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_outline_color: Option<String>,
    /// 文字の輪郭's width in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_outline_width: Option<f32>,
    /// 組み文字, 割注 and 縦中横; an empty layout takes them off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub east_asian_layout: Option<HeadlessEastAsianLayout>,
    /// 校正言語 for Latin text (`w:lang/@w:val`, a BCP 47 tag such as
    /// `en-US`); empty takes it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// 校正言語 for East Asian text (`w:lang/@w:eastAsia`, `ja-JP`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub east_asia_language: Option<String>,
    /// スペル チェックと文章校正を行わない (`w:noProof`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_proof: Option<bool>,
}

impl HeadlessRunFormat {
    /// Whether the patch changes nothing.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}
