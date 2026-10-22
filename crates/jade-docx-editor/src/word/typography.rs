//! Word's Japanese typography over any [`WordSession`]: the 段落 dialog's
//! 体裁 tab (禁則処理, 英単語の途中で改行, 句読点のぶら下げ, 行頭の記号を 1/2
//! の幅にする, 日本語と英字・数字の間隔の自動調整) for the selected
//! paragraphs, and the document's 文字体裁 (禁則処理のレベル with its 行頭 /
//! 行末禁則文字, 文字間隔の調整). OK applies both as one undo step.

use jade_docx_headless_api::HeadlessCommand;
pub use jade_docx_headless_api::{HeadlessJapaneseTypography, HeadlessParagraphTypography};

use super::{Helpers, WordSession};

/// The 行頭禁則文字 Word lists for 標準 — the set layout applies.
pub const STANDARD_NO_LINE_START: &str = jade_docx_layout::word_kinsoku::STANDARD_NO_LINE_START;
/// The 行頭禁則文字 Word lists for 高レベル: 標準's and the small kana and ー.
pub const STRICT_NO_LINE_START: &str = jade_docx_layout::word_kinsoku::STRICT_NO_LINE_START;
/// The 行末禁則文字 Word lists for both levels.
pub const NO_LINE_END: &str = jade_docx_layout::word_kinsoku::NO_LINE_END;

/// 文字間隔の調整 (`w:characterSpacingControl`), in Word's order.
pub const SPACING_CONTROLS: [&str; 3] = [
    "doNotCompress",
    "compressPunctuation",
    "compressPunctuationAndJapaneseKana",
];

/// 禁則処理のレベル.
pub const KINSOKU_LEVELS: [&str; 3] = ["standard", "strict", "custom"];

/// The typography dialog while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct TypographyDialog {
    pub original_paragraph: HeadlessParagraphTypography,
    pub paragraph: HeadlessParagraphTypography,
    pub original_document: HeadlessJapaneseTypography,
    pub document: HeadlessJapaneseTypography,
}

fn changed<T: PartialEq + Clone>(before: &Option<T>, after: &Option<T>) -> Option<T> {
    (before != after).then(|| after.clone()).flatten()
}

impl TypographyDialog {
    pub fn open(
        paragraph: HeadlessParagraphTypography,
        document: HeadlessJapaneseTypography,
    ) -> Self {
        Self {
            original_paragraph: paragraph.clone(),
            paragraph,
            original_document: document.clone(),
            document,
        }
    }

    /// The paragraph options OK changes, if any.
    pub fn paragraph_change(&self) -> Option<HeadlessParagraphTypography> {
        let (before, after) = (&self.original_paragraph, &self.paragraph);
        let change = HeadlessParagraphTypography {
            kinsoku: changed(&before.kinsoku, &after.kinsoku),
            word_wrap: changed(&before.word_wrap, &after.word_wrap),
            overflow_punctuation: changed(
                &before.overflow_punctuation,
                &after.overflow_punctuation,
            ),
            top_line_punctuation: changed(
                &before.top_line_punctuation,
                &after.top_line_punctuation,
            ),
            auto_space_de: changed(&before.auto_space_de, &after.auto_space_de),
            auto_space_dn: changed(&before.auto_space_dn, &after.auto_space_dn),
        };
        (change != HeadlessParagraphTypography::default()).then_some(change)
    }

    /// The document options OK changes, if any. A custom set is written
    /// whole, so a change to either list rewrites both.
    pub fn document_change(&self) -> Option<HeadlessJapaneseTypography> {
        let (before, after) = (&self.original_document, &self.document);
        let sets_changed = before.kinsoku_level != after.kinsoku_level
            || (after.kinsoku_level.as_deref() == Some("custom")
                && (before.no_line_break_before != after.no_line_break_before
                    || before.no_line_break_after != after.no_line_break_after));
        let change = HeadlessJapaneseTypography {
            kinsoku_level: sets_changed.then(|| after.kinsoku_level.clone()).flatten(),
            no_line_break_before: sets_changed
                .then(|| after.no_line_break_before.clone())
                .flatten(),
            no_line_break_after: sets_changed
                .then(|| after.no_line_break_after.clone())
                .flatten(),
            character_spacing_control: changed(
                &before.character_spacing_control,
                &after.character_spacing_control,
            ),
        };
        (change != HeadlessJapaneseTypography::default()).then_some(change)
    }

    /// 禁則処理のレベル: choosing ユーザー設定 starts from the level's lists.
    pub fn set_level(&mut self, level: &str) {
        let document = &mut self.document;
        if level == "custom" && document.kinsoku_level.as_deref() != Some("custom") {
            let start = if document.kinsoku_level.as_deref() == Some("strict") {
                STRICT_NO_LINE_START
            } else {
                STANDARD_NO_LINE_START
            };
            document.no_line_break_before = Some(start.to_string());
            document.no_line_break_after = Some(NO_LINE_END.to_string());
        }
        document.kinsoku_level = Some(level.to_string());
    }

    /// The 行頭 / 行末禁則文字 the dialog shows for the level.
    pub fn line_start_set(&self) -> String {
        match self.document.kinsoku_level.as_deref() {
            Some("custom") => self
                .document
                .no_line_break_before
                .clone()
                .unwrap_or_default(),
            Some("strict") => STRICT_NO_LINE_START.to_string(),
            _ => STANDARD_NO_LINE_START.to_string(),
        }
    }

    pub fn line_end_set(&self) -> String {
        match self.document.kinsoku_level.as_deref() {
            Some("custom") => self
                .document
                .no_line_break_after
                .clone()
                .unwrap_or_default(),
            _ => NO_LINE_END.to_string(),
        }
    }
}

/// Reading and applying it, for every session.
pub trait WordTypography: WordSession {
    /// The selection's first paragraph's 体裁, with Word's defaults where it
    /// sets none.
    fn paragraph_typography(&self) -> HeadlessParagraphTypography {
        let (paragraph, _) = self.paragraph_span();
        let settings = self
            .word_document()
            .paragraph(paragraph)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.east_asian_settings.clone())
            .unwrap_or_default();
        let document = self
            .word_document()
            .document
            .as_ref()
            .map(|document| document.east_asian_line_breaking.clone())
            .unwrap_or_default();
        let pick = |own: Option<bool>, default: Option<bool>, word: bool| {
            Some(own.or(default).unwrap_or(word))
        };
        HeadlessParagraphTypography {
            kinsoku: pick(settings.kinsoku, document.kinsoku, true),
            word_wrap: pick(settings.word_wrap, document.word_wrap, true),
            overflow_punctuation: pick(
                settings.overflow_punctuation,
                document.overflow_punctuation,
                true,
            ),
            top_line_punctuation: pick(
                settings.top_line_punctuation,
                document.top_line_punctuation,
                false,
            ),
            auto_space_de: pick(settings.auto_space_de, document.auto_space_de, true),
            auto_space_dn: pick(settings.auto_space_dn, document.auto_space_dn, true),
        }
    }

    /// The document's 文字体裁.
    fn japanese_typography(&self) -> HeadlessJapaneseTypography {
        let settings = self
            .word_document()
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

    fn typography_dialog(&self) -> TypographyDialog {
        TypographyDialog::open(self.paragraph_typography(), self.japanese_typography())
    }

    /// OK: what changed, as one undo step.
    fn apply_typography(&mut self, dialog: &TypographyDialog) -> Result<(), String> {
        let (start, end) = self.paragraph_span();
        let mut commands = Vec::new();
        if let Some(typography) = dialog.paragraph_change() {
            commands.push(HeadlessCommand::SetParagraphTypography {
                start_paragraph: start,
                end_paragraph: end,
                typography,
            });
        }
        if let Some(typography) = dialog.document_change() {
            commands.push(HeadlessCommand::SetJapaneseTypography { typography });
        }
        if commands.is_empty() {
            return Ok(());
        }
        self.apply_commands(commands, true)
    }
}

impl<T: WordSession + ?Sized> WordTypography for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_changed_is_applied() {
        let paragraph = HeadlessParagraphTypography {
            kinsoku: Some(true),
            auto_space_de: Some(true),
            ..Default::default()
        };
        let document = HeadlessJapaneseTypography {
            kinsoku_level: Some("standard".into()),
            character_spacing_control: Some("doNotCompress".into()),
            ..Default::default()
        };
        let mut dialog = TypographyDialog::open(paragraph, document);
        assert_eq!(dialog.paragraph_change(), None);
        assert_eq!(dialog.document_change(), None);
        dialog.paragraph.auto_space_de = Some(false);
        dialog.set_level("custom");
        let change = dialog.paragraph_change().unwrap();
        assert_eq!(change.auto_space_de, Some(false));
        assert_eq!(change.kinsoku, None);
        let change = dialog.document_change().unwrap();
        assert_eq!(change.kinsoku_level.as_deref(), Some("custom"));
        assert_eq!(
            change.no_line_break_before.as_deref(),
            Some(STANDARD_NO_LINE_START)
        );
        assert_eq!(change.character_spacing_control, None);
    }
}
