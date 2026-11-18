use super::Run;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Distinguish literal U+00AD in `w:t` from `w:softHyphen` without replacing
/// user text. Windows Word preserves both forms but paints them differently.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTextControls {
    pub literal_soft_hyphens: Vec<usize>,
}

impl Run {
    /// Append authored XML character data. The parser appends explicit
    /// `w:softHyphen` controls separately, so they acquire no literal marker.
    pub(crate) fn append_source_text(&mut self, text: &str) {
        let start = self.text.len();
        for (offset, _) in text.match_indices('\u{ad}') {
            self.text_controls
                .get_or_insert_with(Default::default)
                .literal_soft_hyphens
                .push(start + offset);
        }
        self.text.push_str(text);
    }

    pub fn is_literal_soft_hyphen(&self, offset: usize) -> bool {
        self.text_controls.as_ref().is_some_and(|controls| {
            controls.literal_soft_hyphens.contains(&offset)
                && self.text.get(offset..offset.saturating_add(2)) == Some("\u{ad}")
        })
    }

    /// Case transforms keep controls in order but can change preceding UTF-8
    /// lengths (for example dotless `ı` becomes `I`). Keep literal identity by
    /// control ordinal, rather than borrowing now-stale source byte offsets.
    pub fn replace_text_preserving_control_order(&mut self, replacement: String) {
        if self.text_controls.is_some() {
            let kinds: Vec<_> = self
                .text
                .match_indices('\u{ad}')
                .map(|(offset, _)| self.is_literal_soft_hyphen(offset))
                .collect();
            let offsets: Vec<_> = replacement
                .match_indices('\u{ad}')
                .map(|(offset, _)| offset)
                .collect();
            assert_eq!(
                kinds.len(),
                offsets.len(),
                "A control-preserving text transform changed its controls"
            );
            let literals: Vec<_> = kinds
                .into_iter()
                .zip(offsets)
                .filter_map(|(literal, offset)| literal.then_some(offset))
                .collect();
            self.text_controls = (!literals.is_empty()).then(|| {
                Box::new(RunTextControls {
                    literal_soft_hyphens: literals,
                })
            });
        }
        self.text = replacement;
    }

    /// Replace editable text while retaining the identities of untouched
    /// source controls. Newly inserted U+00AD keeps the editor's existing
    /// optional-hyphen semantics; authored literal text is annotated on load.
    pub fn replace_text_range(&mut self, range: Range<usize>, replacement: &str) {
        assert!(range.start <= range.end && range.end <= self.text.len());
        assert!(self.text.is_char_boundary(range.start) && self.text.is_char_boundary(range.end));
        if let Some(controls) = self.text_controls.as_mut() {
            controls.literal_soft_hyphens.retain_mut(|offset| {
                if *offset < range.start {
                    true
                } else if *offset >= range.end {
                    *offset = range.start + replacement.len() + (*offset - range.end);
                    true
                } else {
                    false
                }
            });
            if controls.literal_soft_hyphens.is_empty() {
                self.text_controls = None;
            }
        }
        self.text.replace_range(range, replacement);
    }

    /// Copy a text slice, rebasing its source controls. Callers retain or clear
    /// XML node identities according to whether the source node also moves.
    pub fn with_text_range(&self, range: Range<usize>) -> Self {
        let text = self.text[range.clone()].to_string();
        let mut run = self.clone();
        run.text = text;
        run.text_controls = self.text_controls.as_ref().and_then(|controls| {
            let offsets: Vec<_> = controls
                .literal_soft_hyphens
                .iter()
                .copied()
                .filter(|offset| range.contains(offset))
                .map(|offset| offset - range.start)
                .collect();
            (!offsets.is_empty()).then(|| {
                Box::new(RunTextControls {
                    literal_soft_hyphens: offsets,
                })
            })
        });
        run
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BodyElement, DocumentParser};

    fn run() -> Run {
        let xml = "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>日\u{ad}a</w:t><w:softHyphen/><w:t>b\u{ad}c</w:t></w:r></w:p></w:body></w:document>";
        let document = DocumentParser::new().parse_document_xml(xml).unwrap();
        let BodyElement::Paragraph(paragraph) = &document.body.unwrap().elements[0] else {
            panic!("paragraph")
        };
        paragraph.runs[0].clone()
    }

    #[test]
    fn literal_and_xml_optional_controls_keep_identical_user_text_but_distinct_origins() {
        let run = run();
        assert_eq!(run.text, "日\u{ad}a\u{ad}b\u{ad}c");
        assert_eq!(
            run.text_controls.as_ref().unwrap().literal_soft_hyphens,
            [3, 9]
        );
        assert!(run.is_literal_soft_hyphen(3));
        assert!(!run.is_literal_soft_hyphen(6));
        assert!(run.is_literal_soft_hyphen(9));
        let restored: Run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(restored.text_controls, run.text_controls);
        assert_eq!(restored.text, run.text);
    }

    #[test]
    fn unicode_insert_delete_and_replace_rebase_only_untouched_literals() {
        let mut run = run();
        run.replace_text_range(0..0, "🦀");
        assert_eq!(
            run.text_controls.as_ref().unwrap().literal_soft_hyphens,
            [7, 13]
        );
        run.replace_text_range(7..9, "");
        assert_eq!(
            run.text_controls.as_ref().unwrap().literal_soft_hyphens,
            [11]
        );
        assert!(run.is_literal_soft_hyphen(11));
        run.replace_text_range(11..13, "\u{ad}");
        assert!(run.text_controls.is_none());
        assert_eq!(run.text, "🦀日a\u{ad}b\u{ad}c");
    }

    #[test]
    fn case_transforms_rebase_literal_identity_after_utf8_length_changes() {
        let mut source = run();
        source.replace_text_range(0..3, "ı");
        let uppercase = source.text.to_uppercase();
        source.replace_text_preserving_control_order(uppercase);
        assert_eq!(source.text, "I\u{ad}A\u{ad}B\u{ad}C");
        assert_eq!(
            source.text_controls.as_ref().unwrap().literal_soft_hyphens,
            [1, 7]
        );
        assert!(!source.is_literal_soft_hyphen(4));
    }

    #[test]
    fn run_slices_keep_source_control_identity_at_rebased_utf8_offsets() {
        let source = run();
        let left = source.with_text_range(0..6);
        let right = source.with_text_range(6..source.text.len());
        assert_eq!(left.text, "日\u{ad}a");
        assert_eq!(right.text, "\u{ad}b\u{ad}c");
        assert!(left.is_literal_soft_hyphen(3));
        assert!(!right.is_literal_soft_hyphen(0));
        assert!(right.is_literal_soft_hyphen(3));
        assert_eq!(left.text + &right.text, source.text);
    }
}
