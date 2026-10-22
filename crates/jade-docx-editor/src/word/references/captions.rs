//! 図表番号の挿入: a caption under (or over) the selected item — 図, 表,
//! 数式 or a label of the user's — numbered by a `SEQ` field in a format,
//! after the chapter's number when asked (`STYLEREF n \s`, `SEQ \s n`).

use crate::document::SeqNumberFormat;
use crate::word_document::CaptionOptions;

use super::edit_document;
use super::navigation::WordNavigation;
use crate::word::{Position, WordSession};

/// 番号付け's 書式.
pub const CAPTION_FORMATS: [SeqNumberFormat; 5] = [
    SeqNumberFormat::Arabic,
    SeqNumberFormat::LowerAlphabetic,
    SeqNumberFormat::UpperAlphabetic,
    SeqNumberFormat::LowerRoman,
    SeqNumberFormat::UpperRoman,
];

/// The format's sample, as the list shows it.
pub fn format_sample(format: SeqNumberFormat) -> &'static str {
    match format {
        SeqNumberFormat::Arabic => "1, 2, 3, …",
        SeqNumberFormat::LowerAlphabetic => "a, b, c, …",
        SeqNumberFormat::UpperAlphabetic => "A, B, C, …",
        SeqNumberFormat::LowerRoman => "i, ii, iii, …",
        SeqNumberFormat::UpperRoman => "I, II, III, …",
        _ => "1, 2, 3, …",
    }
}

/// 区切り文字 between the chapter's number and the caption's.
pub const CHAPTER_SEPARATORS: [&str; 5] = ["-", ".", ":", "—", "–"];

/// The 図表番号 dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionDialog {
    /// ラベル: Word's three, then the document's and the user's own.
    pub labels: Vec<String>,
    pub label: String,
    /// 位置: 選択した項目の上.
    pub above: bool,
    /// ラベルを表示しない.
    pub exclude_label: bool,
    pub format: SeqNumberFormat,
    /// 章番号を含める.
    pub chapter: bool,
    /// 章タイトルのスタイル: the heading level that starts a chapter.
    pub chapter_level: u8,
    pub separator: &'static str,
}

fn number_text(format: SeqNumberFormat, value: usize) -> String {
    let token = match format {
        SeqNumberFormat::LowerAlphabetic => "lowerLetter",
        SeqNumberFormat::UpperAlphabetic => "upperLetter",
        SeqNumberFormat::LowerRoman => "lowerRoman",
        SeqNumberFormat::UpperRoman => "upperRoman",
        _ => "decimal",
    };
    crate::numbering::format_number_value(token, value.max(1) as u32, None)
}

/// Captions over every session.
pub trait WordCaptions: WordSession {
    /// The dialog: labels from Word, the document's captions and `custom`.
    fn caption_dialog(&self, custom: &[String]) -> CaptionDialog {
        let mut labels: Vec<String> = ["図", "表", "数式"].iter().map(|l| l.to_string()).collect();
        for label in self
            .caption_labels_used()
            .into_iter()
            .chain(custom.iter().cloned())
        {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        CaptionDialog {
            label: labels[0].clone(),
            labels,
            above: false,
            exclude_label: false,
            format: SeqNumberFormat::Arabic,
            chapter: false,
            chapter_level: 1,
            separator: "-",
        }
    }

    /// The labels the document's `SEQ` fields number.
    fn caption_labels_used(&self) -> Vec<String> {
        let document = self.word_document();
        let mut labels = Vec::new();
        for paragraph in 0..document.paragraph_count() {
            let Some(model) = document.paragraph(paragraph) else {
                continue;
            };
            for run in &model.runs {
                if let Some(crate::document::RunFieldKind::Seq { identifier, .. }) =
                    run.field.as_ref().map(|field| &field.kind)
                {
                    if !labels.contains(identifier) {
                        labels.push(identifier.clone());
                    }
                }
            }
        }
        labels
    }

    /// The chapter's number at a paragraph: how many headings of the level
    /// came before it (its own included).
    fn chapter_number(&self, paragraph: usize, level: u8) -> usize {
        self.outline()
            .into_iter()
            .filter(|heading| heading.level == level && heading.paragraph <= paragraph)
            .count()
            .max(1)
    }

    /// The caption's number, as the dialog's 図表番号 box shows it: the
    /// next number for the label, within the chapter when asked.
    fn caption_preview(&self, dialog: &CaptionDialog, anchor: usize) -> String {
        let document = self.word_document();
        let chapter_start = dialog.chapter.then(|| {
            self.outline()
                .into_iter()
                .filter(|heading| {
                    heading.level <= dialog.chapter_level && heading.paragraph <= anchor
                })
                .map(|heading| heading.paragraph)
                .next_back()
                .unwrap_or(0)
        });
        let mut count = 0;
        for paragraph in
            chapter_start.unwrap_or(0)..=anchor.min(document.paragraph_count().saturating_sub(1))
        {
            let Some(model) = document.paragraph(paragraph) else {
                continue;
            };
            count += model
                .runs
                .iter()
                .filter(|run| {
                    matches!(
                        run.field.as_ref().map(|field| &field.kind),
                        Some(crate::document::RunFieldKind::Seq { identifier, .. })
                            if *identifier == dialog.label
                    )
                })
                .count();
        }
        let number = number_text(dialog.format, count + 1);
        let number = if dialog.chapter {
            format!(
                "{}{}{number}",
                self.chapter_number(anchor, dialog.chapter_level),
                dialog.separator
            )
        } else {
            number
        };
        if dialog.exclude_label {
            number
        } else {
            format!("{} {number}", dialog.label)
        }
    }

    /// OK: the caption by the paragraph `anchor` (the selected item's),
    /// with `text` after its number, one undo step. Returns where the
    /// caret goes: the caption's end.
    fn insert_caption(
        &mut self,
        dialog: &CaptionDialog,
        anchor: usize,
        text: &str,
    ) -> Result<Position, String> {
        let label = dialog.label.trim().to_string();
        if label.is_empty() {
            return Err("ラベルを入力してください".to_string());
        }
        let options = CaptionOptions {
            label,
            description: text.trim().to_string(),
            above: dialog.above,
            exclude_label: dialog.exclude_label,
            number_format: dialog.format,
            chapter: dialog.chapter.then(|| {
                (
                    dialog.chapter_level,
                    dialog.separator.to_string(),
                    self.chapter_number(anchor, dialog.chapter_level)
                        .to_string(),
                )
            }),
        };
        edit_document(self, "insert_caption", false, move |document, selection| {
            *selection = Some(document.insert_caption_with(anchor, &options)?);
            Ok(())
        })?;
        Ok(self
            .selection_range()
            .map(|range| (range.focus.paragraph, range.focus.offset))
            .unwrap_or((anchor, 0)))
    }
}

impl<T: WordSession + ?Sized> WordCaptions for T {}
