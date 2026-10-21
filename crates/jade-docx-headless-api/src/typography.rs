//! Word's Japanese typography: the 段落 dialog's 体裁 tab, per paragraph,
//! and the document's 文字体裁 options (禁則処理のレベル, the custom 行頭 /
//! 行末禁則文字, カーニング of punctuation and kana), kept in `settings.xml`.
//!
//! As with [`crate::HeadlessRunFormat`], a field left `None` is left alone.

use serde::{Deserialize, Serialize};

/// The 体裁 options of a paragraph (`w:pPr`, ECMA-376 Part 1 §17.3.1).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessParagraphTypography {
    /// 禁則処理を行う (`w:kinsoku`, on by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinsoku: Option<bool>,
    /// 英単語の途中で改行しない: `w:wordWrap` on (the default); off lets a
    /// Latin word break anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<bool>,
    /// 句読点のぶら下げを行う (`w:overflowPunct`, on by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow_punctuation: Option<bool>,
    /// 行頭の記号を 1/2 の幅にする (`w:topLinePunct`, off by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_line_punctuation: Option<bool>,
    /// 日本語と英字の間隔を自動調整する (`w:autoSpaceDE`, on by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_space_de: Option<bool>,
    /// 日本語と数字の間隔を自動調整する (`w:autoSpaceDN`, on by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_space_dn: Option<bool>,
}

/// The document's 文字体裁 (`w:settings`, ECMA-376 Part 1 §17.15.1).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessJapaneseTypography {
    /// 禁則処理のレベル: `standard` (標準), `strict` (高レベル,
    /// `w:strictFirstAndLastChars`) or `custom` (ユーザー設定, the two
    /// character sets below as `w:noLineBreaksBefore/After w:lang="ja-JP"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinsoku_level: Option<String>,
    /// 行頭禁則文字, for `custom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_line_break_before: Option<String>,
    /// 行末禁則文字, for `custom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_line_break_after: Option<String>,
    /// 文字間隔の調整 (`w:characterSpacingControl`): `doNotCompress`,
    /// `compressPunctuation` or `compressPunctuationAndJapaneseKana`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_spacing_control: Option<String>,
}
