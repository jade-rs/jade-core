//! A section's page layout as Word's ページ設定 dialog edits it: the paper,
//! the margins and the binding, the columns, the document grid
//! (文字数と行数), the text direction, the vertical alignment, the line
//! numbers, the header and footer layout and the page numbering — the read
//! side ([`HeadlessSectionLayout`]) and the write side
//! ([`HeadlessSectionLayoutPatch`]) of one section's `w:sectPr`, plus the
//! document-wide `settings.xml` switches the same dialog shows
//! (`w:mirrorMargins`, `w:gutterAtTop`, `w:bookFoldPrinting`,
//! `w:printTwoOnOne`, `w:evenAndOddHeaders`).
//!
//! Lengths are in points.

use serde::{Deserialize, Serialize};

/// Which sections an edit reaches, as the dialog's 設定対象 names them.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum HeadlessSectionScope {
    /// The section holding the caret (このセクション); the final section for
    /// a caller without a selection.
    #[default]
    Caret,
    /// One section by its index.
    Index { index: usize },
    /// Sections `first..=last` (選択しているセクション).
    Range { first: usize, last: usize },
    /// The caret's section and every one after it (これ以降).
    CaretToEnd,
    /// Every section (文書全体).
    All,
}

/// `w:cols/w:col`: one column of an unequal set.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessSectionColumn {
    pub width: f32,
    /// The space after the column; the last column's is ignored.
    pub space: f32,
}

/// `w:cols`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessSectionColumns {
    pub count: u32,
    /// The space between equal columns.
    pub space: f32,
    pub equal_width: bool,
    /// 境界線を引く (`w:sep`).
    pub separator: bool,
    /// Each column's width and the space after it, when they are not equal.
    #[serde(default)]
    pub columns: Vec<HeadlessSectionColumn>,
}

/// `w:lnNumType`, as Word's 行番号 dialog states it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessLineNumbering {
    /// 行番号を追加する. Off removes `w:lnNumType`.
    pub enabled: bool,
    /// 開始番号: the number the first line shows (`w:start` + 1).
    pub start_at: u32,
    /// 行番号の間隔 (`w:countBy`).
    pub count_by: u32,
    /// 文字列との間隔; `None` is 自動.
    #[serde(default)]
    pub distance: Option<f32>,
    /// 番号の付け方: `newPage`, `newSection` or `continuous`.
    pub restart: String,
}

/// `w:docGrid`, the 文字数と行数 tab.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessDocGrid {
    /// `default` (標準の文字数を使う), `lines` (行数だけを指定する),
    /// `linesAndChars` (文字数と行数を指定する) or `snapToChars`
    /// (原稿用紙の設定にする).
    pub grid_type: String,
    /// 行送り (`w:linePitch`).
    #[serde(default)]
    pub line_pitch: Option<f32>,
    /// 字送り: the width each character is given on the grid — the default
    /// font size plus `w:charSpace`.
    #[serde(default)]
    pub char_pitch: Option<f32>,
}

/// `w:pgNumType`, Word's ページ番号の書式.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HeadlessPageNumbering {
    /// `decimal`, `lowerRoman`, `upperRoman`, `lowerLetter`, `upperLetter`,
    /// `decimalFullWidth`, `japaneseCounting`, …
    pub format: String,
    /// 開始番号; `None` is 前のセクションから継続.
    #[serde(default)]
    pub start: Option<u32>,
    /// 章番号を含める: the heading level (1–9) whose number leads.
    #[serde(default)]
    pub chapter_style: Option<u8>,
    /// 区切り文字: `hyphen`, `period`, `colon`, `emDash` or `enDash`.
    #[serde(default)]
    pub chapter_separator: Option<String>,
}

/// One section's layout and the document switches beside it.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessSectionLayout {
    pub section: usize,
    pub section_count: usize,
    pub width: f32,
    pub height: f32,
    pub landscape: bool,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    pub header_distance: f32,
    pub footer_distance: f32,
    pub gutter: f32,
    /// とじしろの位置: 上 (`w:gutterAtTop`, document-wide).
    pub gutter_at_top: bool,
    /// The gutter is on the right (`w:rtlGutter`).
    pub rtl_gutter: bool,
    /// 見開きページ (`w:mirrorMargins`).
    pub mirror_margins: bool,
    /// 本 (縦方向に谷折り) (`w:bookFoldPrinting`).
    pub book_fold: bool,
    /// 袋とじ / 2 ページ / 枚 (`w:printTwoOnOne`).
    pub print_two_on_one: bool,
    /// セクションの開始位置: `nextPage`, `continuous`, `evenPage`,
    /// `oddPage` or `nextColumn`.
    pub start: String,
    /// 垂直方向の配置: `top`, `center`, `both` or `bottom`.
    pub vertical_alignment: String,
    /// 先頭ページのみ別指定 (`w:titlePg`).
    pub different_first_page: bool,
    /// 奇数/偶数ページ別指定 (`w:evenAndOddHeaders`, document-wide).
    pub different_odd_even: bool,
    pub columns: HeadlessSectionColumns,
    pub line_numbering: HeadlessLineNumbering,
    pub doc_grid: HeadlessDocGrid,
    /// The document's default font size, which the grid's 字送り is
    /// measured from.
    pub default_font_size: f32,
    /// 文字方向: `lrTb` (横書き) or `tbRl` (縦書き).
    pub text_direction: String,
    /// The section reads right to left (`w:bidi`).
    pub bidi: bool,
    pub page_numbering: HeadlessPageNumbering,
}

/// What an edit changes; every `None` leaves that value as it is, so one
/// patch can be applied to sections that differ elsewhere.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HeadlessSectionLayoutPatch {
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub height: Option<f32>,
    #[serde(default)]
    pub landscape: Option<bool>,
    #[serde(default)]
    pub margin_top: Option<f32>,
    #[serde(default)]
    pub margin_bottom: Option<f32>,
    #[serde(default)]
    pub margin_left: Option<f32>,
    #[serde(default)]
    pub margin_right: Option<f32>,
    #[serde(default)]
    pub header_distance: Option<f32>,
    #[serde(default)]
    pub footer_distance: Option<f32>,
    #[serde(default)]
    pub gutter: Option<f32>,
    #[serde(default)]
    pub gutter_at_top: Option<bool>,
    #[serde(default)]
    pub rtl_gutter: Option<bool>,
    #[serde(default)]
    pub mirror_margins: Option<bool>,
    #[serde(default)]
    pub book_fold: Option<bool>,
    #[serde(default)]
    pub print_two_on_one: Option<bool>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub vertical_alignment: Option<String>,
    #[serde(default)]
    pub different_first_page: Option<bool>,
    #[serde(default)]
    pub different_odd_even: Option<bool>,
    #[serde(default)]
    pub columns: Option<HeadlessSectionColumns>,
    #[serde(default)]
    pub line_numbering: Option<HeadlessLineNumbering>,
    #[serde(default)]
    pub doc_grid: Option<HeadlessDocGrid>,
    #[serde(default)]
    pub text_direction: Option<String>,
    #[serde(default)]
    pub bidi: Option<bool>,
    #[serde(default)]
    pub page_numbering: Option<HeadlessPageNumbering>,
}

impl HeadlessSectionLayoutPatch {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}
