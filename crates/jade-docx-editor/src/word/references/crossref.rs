//! 相互参照: a REF, PAGEREF or NOTEREF field at the caret pointing at a
//! heading, a bookmark, a footnote or endnote, or a caption (図, 表, 数式
//! and the labels a document adds), as text, page number or number, linked
//! to its target or not (ECMA-376 Part 1 §17.16.5.45, .47, .53).

use crate::word_document::{CrossReferenceKind, CrossReferenceTarget, CrossReferenceTargetType};
use jade_selection::{SelectionRange, TextPosition};

use super::edit_document;
use crate::word::{Position, WordSession};

/// 参照する項目.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReferenceType {
    Heading,
    Bookmark,
    Footnote,
    Endnote,
    /// Captions with this label.
    Caption(String),
}

/// 相互参照の文字列.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceText {
    /// 見出し文字列, ブックマークの文字列, 図表番号全体.
    Text,
    /// ページ番号.
    Page,
    /// 脚注番号, ラベルと番号のみ.
    Number,
}

impl ReferenceType {
    /// The texts Word offers for the type, the first being its default.
    pub fn texts(&self) -> &'static [ReferenceText] {
        match self {
            ReferenceType::Heading | ReferenceType::Bookmark => {
                &[ReferenceText::Text, ReferenceText::Page]
            }
            ReferenceType::Footnote | ReferenceType::Endnote => {
                &[ReferenceText::Number, ReferenceText::Page]
            }
            ReferenceType::Caption(_) => &[
                ReferenceText::Text,
                ReferenceText::Number,
                ReferenceText::Page,
            ],
        }
    }

    /// The i18n key of a text for this type.
    pub fn text_key(&self, text: ReferenceText) -> &'static str {
        match (self, text) {
            (ReferenceType::Heading, ReferenceText::Text) => "xref.text.heading",
            (ReferenceType::Bookmark, ReferenceText::Text) => "xref.text.bookmark",
            (ReferenceType::Caption(_), ReferenceText::Text) => "xref.text.caption",
            (ReferenceType::Caption(_), ReferenceText::Number) => "xref.text.label_number",
            (ReferenceType::Footnote, ReferenceText::Number) => "xref.text.footnote",
            (ReferenceType::Endnote, ReferenceText::Number) => "xref.text.endnote",
            (_, ReferenceText::Page) => "xref.text.page",
            (_, _) => "xref.text.number",
        }
    }
}

/// The 相互参照 dialog's choices; the target is picked by its index in
/// [`WordCrossReferences::reference_targets`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossReferenceDialog {
    pub kind: ReferenceType,
    pub text: ReferenceText,
    /// ハイパーリンクとして挿入する.
    pub hyperlink: bool,
    pub selected: Option<usize>,
    /// The types the list offers: the fixed ones, then a caption label each.
    pub kinds: Vec<ReferenceType>,
}

impl CrossReferenceDialog {
    /// Picks another type: its first text, nothing selected.
    pub fn set_kind(&mut self, kind: ReferenceType) {
        self.text = kind.texts()[0];
        self.kind = kind;
        self.selected = None;
    }
}

/// The caption labels a document numbers (its `SEQ` identifiers), after
/// Word's own three.
pub fn caption_labels_in(targets: &[CrossReferenceTarget], extra: &[String]) -> Vec<String> {
    let mut labels: Vec<String> = ["図", "表", "数式"]
        .iter()
        .map(|label| label.to_string())
        .collect();
    for label in extra {
        if !labels.contains(label) {
            labels.push(label.clone());
        }
    }
    for target in targets {
        if target.target_type != CrossReferenceTargetType::Caption {
            continue;
        }
        let label: String = target
            .label
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches(|ch: char| ch.is_ascii_digit())
            .to_string();
        if !label.is_empty() && !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

/// Cross-references over every session.
pub trait WordCrossReferences: WordSession {
    /// The dialog, opened on 見出し.
    fn cross_reference_dialog(&self, custom_labels: &[String]) -> CrossReferenceDialog {
        let targets = self.word_document().cross_reference_targets();
        let mut kinds = vec![
            ReferenceType::Heading,
            ReferenceType::Bookmark,
            ReferenceType::Footnote,
            ReferenceType::Endnote,
        ];
        kinds.extend(
            caption_labels_in(&targets, custom_labels)
                .into_iter()
                .map(ReferenceType::Caption),
        );
        CrossReferenceDialog {
            kind: ReferenceType::Heading,
            text: ReferenceText::Text,
            hyperlink: true,
            selected: None,
            kinds,
        }
    }

    /// 参照先: the targets of a type, in document order.
    fn reference_targets(&self, kind: &ReferenceType) -> Vec<CrossReferenceTarget> {
        self.word_document()
            .cross_reference_targets()
            .into_iter()
            .filter(|target| match kind {
                ReferenceType::Heading => target.target_type == CrossReferenceTargetType::Heading,
                ReferenceType::Bookmark => target.target_type == CrossReferenceTargetType::Bookmark,
                ReferenceType::Footnote => target.target_type == CrossReferenceTargetType::Footnote,
                ReferenceType::Endnote => target.target_type == CrossReferenceTargetType::Endnote,
                ReferenceType::Caption(label) => {
                    target.target_type == CrossReferenceTargetType::Caption
                        && target
                            .label
                            .trim_start()
                            .strip_prefix(label.as_str())
                            .is_some_and(|rest| {
                                rest.starts_with(|ch: char| {
                                    ch.is_whitespace() || ch.is_ascii_digit()
                                })
                            })
                }
            })
            .collect()
    }

    /// 挿入: the field at the caret (after the selection), one undo step.
    /// Returns where the caret goes: after the field.
    fn insert_cross_reference(
        &mut self,
        target: &CrossReferenceTarget,
        text: ReferenceText,
        hyperlink: bool,
    ) -> Result<Position, String> {
        let at = match self.selection_range() {
            Some(range) => range.normalized().1,
            None => TextPosition::new(0, 0),
        };
        let kind = match text {
            ReferenceText::Text => CrossReferenceKind::Text,
            ReferenceText::Page => CrossReferenceKind::Page,
            ReferenceText::Number => CrossReferenceKind::Number,
        };
        let target = target.clone();
        edit_document(
            self,
            "insert_cross_reference",
            false,
            move |document, selection| {
                let caret = document.insert_target_cross_reference(
                    SelectionRange::collapsed(at),
                    &target,
                    kind,
                    hyperlink,
                )?;
                *selection = Some(caret);
                Ok(())
            },
        )?;
        Ok(self
            .selection_range()
            .map(|range| (range.focus.paragraph, range.focus.offset))
            .unwrap_or((at.paragraph, at.offset)))
    }
}

impl<T: WordSession + ?Sized> WordCrossReferences for T {}
