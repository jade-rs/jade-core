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

/// The 段落 dialog's アウトライン レベル and 既定で折りたたみ.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphOutlineState {
    /// 0–8 for レベル 1–9, 9 for 本文.
    pub level: u8,
    /// The level the paragraph's style gives it: Word greys the list out
    /// when that is a heading's (見出し 1–9 keep their level).
    pub style_level: u8,
    /// 既定で折りたたみ (`w15:collapsed`).
    pub collapsed: bool,
}

impl WordDocument {
    /// The formatting of the run holding byte `offset` of paragraph
    /// `paragraph` — the run before it at the paragraph's end — as the
    /// フォント dialog shows it, including inherited styles.
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
        let direct = last
            .or_else(|| paragraph.runs.last())
            .and_then(|run| run.properties.as_ref());
        let style = paragraph
            .properties
            .as_ref()
            .and_then(|props| props.style.as_deref());
        let mark = paragraph
            .properties
            .as_ref()
            .and_then(|p| p.mark_run_properties.as_ref());
        self.resolved_run_format(style, if length == 0 { mark.or(direct) } else { direct })
    }

    /// Effective formats intersecting a half-open UTF-8 byte range. The end
    /// run (or next paragraph at offset zero) is not part of the selection.
    pub fn run_formats_in_range(
        &self,
        start: (usize, usize),
        end: (usize, usize),
    ) -> Vec<HeadlessRunFormat> {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        if start == end {
            return vec![self.run_format_at(start.0, start.1)];
        }
        let mut formats = Vec::new();
        for index in start.0..=end.0.min(self.paragraph_count().saturating_sub(1)) {
            if index == end.0 && end.1 == 0 {
                break;
            }
            let Some(paragraph) = self.paragraph(index) else {
                continue;
            };
            let from = if index == start.0 { start.1 } else { 0 };
            let to = if index == end.0 { end.1 } else { usize::MAX };
            let style = paragraph
                .properties
                .as_ref()
                .and_then(|p| p.style.as_deref());
            let mut offset = 0;
            for run in &paragraph.runs {
                let next = offset + run.text.len();
                if offset < to && next > from && next > offset {
                    formats.push(self.resolved_run_format(style, run.properties.as_ref()));
                }
                offset = next;
            }
            // Crossing into the next paragraph selects this paragraph's mark,
            // even when none of its text is selected. The final paragraph at
            // offset zero remains excluded by the half-open boundary above.
            if index < end.0 {
                let mark = paragraph
                    .properties
                    .as_ref()
                    .and_then(|p| p.mark_run_properties.as_ref());
                formats.push(self.resolved_run_format(style, mark));
            }
        }
        formats
    }

    pub(crate) fn resolved_run_format(
        &self,
        paragraph_style: Option<&str>,
        direct: Option<&jade_docx_reader::RunProperties>,
    ) -> HeadlessRunFormat {
        let resolved = self
            .styles_part
            .as_ref()
            .and_then(|styles| styles.resolve_effective_run_properties(paragraph_style, direct));
        let mut format = resolved
            .as_ref()
            .or(direct)
            .map(run_format_of)
            .unwrap_or_default();
        // noProof is retained as XML rather than a modeled style property.
        // Style resolution must not discard an authored on/off value.
        if let Some(direct) = direct {
            format.no_proof = run_format_of(direct).no_proof.or(format.no_proof);
        }
        format
    }

    /// The level paragraph `paragraph`'s style gives it: 0–8, or 9 for
    /// 本文 (a style that sets none).
    pub fn style_outline_level(&self, paragraph: usize) -> u8 {
        let Some(styles) = self.styles_part.as_ref() else {
            return 9;
        };
        let style = self
            .paragraph(paragraph)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|props| props.style.as_deref().map(str::to_string))
            .or_else(|| styles.default_paragraph_style_id().map(|id| id.to_string()));
        style
            .and_then(|style| styles.resolve_paragraph_properties(&style))
            .and_then(|props| props.outline_level)
            .map_or(9, |level| level.min(9))
    }

    /// Whether paragraph `paragraph` opens collapsed: 既定で折りたたみ, Word
    /// 2013's `w15:collapsed` in its `w:pPr`.
    pub fn paragraph_collapsed_by_default(&self, paragraph: usize) -> bool {
        crate::commands::paragraph_property_sync::snapshot_paragraph_properties_xml(self, paragraph)
            .is_some_and(|properties| {
                properties.children.iter().any(|child| {
                    matches!(child, crate::docx_xml::XmlNode::Element(element)
                    if element.name == "w15:collapsed"
                        && !element.attributes.iter().any(|attribute| {
                            attribute.name == "w15:val"
                                && matches!(attribute.value.as_str(), "0" | "false" | "off")
                        }))
                })
            })
    }

    /// What the 段落 dialog's アウトライン レベル opens on.
    pub fn paragraph_outline(&self, paragraph: usize) -> ParagraphOutlineState {
        let level = self
            .paragraph(paragraph)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|props| props.outline_level)
            .map_or(9, |level| level.min(9));
        ParagraphOutlineState {
            level,
            style_level: self.style_outline_level(paragraph),
            collapsed: self.paragraph_collapsed_by_default(paragraph),
        }
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
