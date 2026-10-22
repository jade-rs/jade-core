//! What Word's フォント, 段落 (改ページと改行, 体裁) and 文字体裁 dialogs open on:
//! the state of a run, a paragraph and the document, in the same shapes the
//! headless commands that change them take.

use super::WordDocument;
use crate::commands::set_run_format::run_format_of;
use jade_docx_headless_api::{
    HeadlessJapaneseTypography, HeadlessParagraphTypography, HeadlessRunFormat,
};
use serde::Serialize;

/// The 段落 dialog's 改ページと改行 tab, and the other paragraph options the
/// dialog shows, as the paragraph sets them directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphPaginationState {
    pub widow_control: bool,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub page_break_before: bool,
    pub suppress_line_numbers: bool,
    pub suppress_auto_hyphens: bool,
    pub contextual_spacing: bool,
    /// 1 ページの行数を指定時に文字を行グリッド線に合わせる; Word's default is on.
    pub snap_to_grid: bool,
}

impl WordDocument {
    /// The formatting of the run holding byte `offset` of paragraph
    /// `paragraph` — the run before it at the paragraph's end — as the
    /// フォント dialog shows it. Direct formatting only, as Word's dialog
    /// reads it.
    pub fn run_format_at(&self, paragraph: usize, offset: usize) -> HeadlessRunFormat {
        let Some(paragraph) = self.paragraph(paragraph) else {
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

    /// The paragraph's 改ページと改行 options, off where it sets none.
    pub fn paragraph_pagination(&self, paragraph: usize) -> ParagraphPaginationState {
        let Some(props) = self
            .paragraph(paragraph)
            .and_then(|paragraph| paragraph.properties.as_ref())
        else {
            return ParagraphPaginationState {
                snap_to_grid: true,
                ..Default::default()
            };
        };
        ParagraphPaginationState {
            widow_control: props.widow_control.unwrap_or(false),
            keep_next: props.keep_next.unwrap_or(false),
            keep_lines: props.keep_lines.unwrap_or(false),
            page_break_before: props.page_break_before.unwrap_or(false),
            suppress_line_numbers: props.suppress_line_numbers.unwrap_or(false),
            suppress_auto_hyphens: props.suppress_auto_hyphens.unwrap_or(false),
            contextual_spacing: props.contextual_spacing.unwrap_or(false),
            snap_to_grid: props.snap_to_grid.unwrap_or(true),
        }
    }

    /// The paragraph's 体裁, with the document's and then Word's defaults
    /// where it sets none.
    pub fn paragraph_typography(&self, paragraph: usize) -> HeadlessParagraphTypography {
        let own = self
            .paragraph(paragraph)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.east_asian_settings.clone())
            .unwrap_or_default();
        let document = self
            .document
            .as_ref()
            .map(|document| document.east_asian_line_breaking.clone())
            .unwrap_or_default();
        let pick = |own: Option<bool>, default: Option<bool>, word: bool| {
            Some(own.or(default).unwrap_or(word))
        };
        HeadlessParagraphTypography {
            kinsoku: pick(own.kinsoku, document.kinsoku, true),
            word_wrap: pick(own.word_wrap, document.word_wrap, true),
            overflow_punctuation: pick(
                own.overflow_punctuation,
                document.overflow_punctuation,
                true,
            ),
            top_line_punctuation: pick(
                own.top_line_punctuation,
                document.top_line_punctuation,
                false,
            ),
            auto_space_de: pick(own.auto_space_de, document.auto_space_de, true),
            auto_space_dn: pick(own.auto_space_dn, document.auto_space_dn, true),
        }
    }

    /// The document's 文字体裁: the kinsoku level (`standard`, `strict` or
    /// `custom` with its own sets) and the compression of punctuation.
    pub fn japanese_typography(&self) -> HeadlessJapaneseTypography {
        let settings = self
            .document
            .as_ref()
            .map(|document| document.east_asian_line_breaking.clone())
            .unwrap_or_default();
        let custom = settings.no_line_break_before_replaces_default
            || settings.no_line_break_after_replaces_default;
        let level = if custom {
            "custom"
        } else if settings.strict_first_and_last_chars {
            "strict"
        } else {
            "standard"
        };
        HeadlessJapaneseTypography {
            kinsoku_level: Some(level.to_string()),
            no_line_break_before: custom.then(|| settings.no_line_break_before.concat()),
            no_line_break_after: custom.then(|| settings.no_line_break_after.concat()),
            character_spacing_control: Some(
                settings
                    .character_spacing_control
                    .unwrap_or_else(|| "doNotCompress".to_string()),
            ),
        }
    }
}
