//! 脚注 and 文末脚注 beyond putting one in: the notes walked one by one (次
//! の脚注 / 前の脚注), 注の表示 between a reference and its note, a note
//! converted to the other kind or all of them at once, and the 脚注と文末
//! 脚注 dialog — where the notes go, how they are numbered, from what
//! number, and when the numbering starts again (ECMA-376 Part 1 §17.11).

use crate::document::EndnotePosition;
use crate::word_document::{ParagraphStory, WordDocument};
use jade_selection::TextPosition;

use super::{edit_document, NoteKind};
use crate::word::{Position, WordSession};

/// A note's reference mark in the body, where the pages draw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteMark {
    pub kind: NoteKind,
    /// The note's `w:id`.
    pub id: String,
    /// The paragraph and the rendered offset the mark starts at.
    pub at: Position,
    /// How many bytes its number takes.
    pub len: usize,
}

impl NoteMark {
    /// Just after the mark, where the caret goes to stand beside it.
    pub fn after(&self) -> Position {
        (self.at.0, self.at.1 + self.len)
    }
}

/// 場所 for footnotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootnotePlace {
    /// ページの最後.
    PageBottom,
    /// テキストの直後.
    BeneathText,
}

/// 場所 for endnotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndnotePlace {
    /// セクションの最後.
    SectionEnd,
    /// 文書の最後.
    DocumentEnd,
}

/// 番号の付け方.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteRestart {
    /// 連続.
    Continuous,
    /// セクションごとに振り直し.
    EachSection,
    /// ページごとに振り直し (footnotes only).
    EachPage,
}

impl NoteRestart {
    fn token(self) -> &'static str {
        match self {
            NoteRestart::Continuous => "continuous",
            NoteRestart::EachSection => "eachSect",
            NoteRestart::EachPage => "eachPage",
        }
    }

    fn from_token(token: &str) -> Self {
        match token.to_ascii_lowercase().as_str() {
            "eachsect" => NoteRestart::EachSection,
            "eachpage" => NoteRestart::EachPage,
            _ => NoteRestart::Continuous,
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            NoteRestart::Continuous => "notes.restart.continuous",
            NoteRestart::EachSection => "notes.restart.section",
            NoteRestart::EachPage => "notes.restart.page",
        }
    }
}

/// 変更の対象.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteScope {
    /// 文書全体.
    WholeDocument,
    /// このセクション.
    ThisSection,
}

/// 番号書式: `w:numFmt` values Word for Mac lists for notes.
pub const NOTE_FORMATS: [&str; 11] = [
    "decimal",
    "upperLetter",
    "lowerLetter",
    "upperRoman",
    "lowerRoman",
    "chicago",
    "decimalEnclosedCircle",
    "decimalFullWidth",
    "japaneseCounting",
    "aiueoFullWidth",
    "irohaFullWidth",
];

/// What a format looks like: its first three numbers, as the list shows them.
pub fn format_sample(format: &str) -> String {
    let numbers: Vec<String> = (1..=3)
        .map(|value| crate::numbering::format_number_value(format, value, None))
        .collect();
    format!("{}, …", numbers.join(", "))
}

/// The 脚注と文末脚注 dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteDialog {
    /// Which notes the settings are for: 脚注 or 文末脚注.
    pub kind: NoteKind,
    pub footnote_place: FootnotePlace,
    pub endnote_place: EndnotePlace,
    /// A `w:numFmt` token from [`NOTE_FORMATS`].
    pub format: String,
    /// 開始番号.
    pub start: u32,
    pub restart: NoteRestart,
    pub scope: NoteScope,
    /// The caret's section, and how many the document has.
    pub section: usize,
    pub sections: usize,
    /// 境界線 and 継続境界線: whether the kind's separator and continuation
    /// separator draw their rules.
    pub separator: bool,
    pub continuation_separator: bool,
}

impl NoteDialog {
    /// The restarts the kind offers: endnotes cannot start again each page.
    pub fn restarts(&self) -> &'static [NoteRestart] {
        match self.kind {
            NoteKind::Footnote => &[
                NoteRestart::Continuous,
                NoteRestart::EachSection,
                NoteRestart::EachPage,
            ],
            NoteKind::Endnote => &[NoteRestart::Continuous, NoteRestart::EachSection],
        }
    }

    /// The sections the settings land on.
    fn targets(&self) -> Vec<usize> {
        match self.scope {
            NoteScope::WholeDocument => (0..self.sections.max(1)).collect(),
            NoteScope::ThisSection => vec![self.section],
        }
    }
}

/// すべての脚注を文末脚注に変更, and the rest of 変換.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteConversion {
    FootnotesToEndnotes,
    EndnotesToFootnotes,
    Swap,
}

fn is_footnote(kind: NoteKind) -> bool {
    kind == NoteKind::Footnote
}

/// The notes' settings and moves, over every session.
pub trait WordNotes: WordSession {
    /// Every reference mark in the body, in document order.
    fn note_marks(&self) -> Vec<NoteMark> {
        let document = self.word_document();
        let mut marks = Vec::new();
        for paragraph in 0..document.paragraph_count() {
            let mut rendered = 0;
            for span in document.reference_marker_spans(paragraph) {
                let at = span.model_offset + rendered;
                rendered += span.rendered_len;
                marks.push(NoteMark {
                    kind: if span.is_footnote {
                        NoteKind::Footnote
                    } else {
                        NoteKind::Endnote
                    },
                    id: span.note_id,
                    at: (paragraph, at),
                    len: span.rendered_len,
                });
            }
        }
        marks
    }

    /// 次の脚注 / 前の脚注: the next (or previous) mark of a kind after (or
    /// before) `from`. From inside a note, the walk starts at its mark.
    fn step_note(&self, kind: NoteKind, from: Position, forwards: bool) -> Option<NoteMark> {
        let from = match self.note_story_at(from.0) {
            Some((note_kind, id)) => self
                .note_marks()
                .into_iter()
                .find(|mark| mark.kind == note_kind && mark.id == id)
                .map(|mark| if forwards { mark.after() } else { mark.at })
                .unwrap_or(from),
            None => from,
        };
        let mut marks = self
            .note_marks()
            .into_iter()
            .filter(|mark| mark.kind == kind);
        if forwards {
            marks.find(|mark| mark.at >= from)
        } else {
            marks.rfind(|mark| mark.after() <= from)
        }
    }

    /// How many bytes of the drawn text the character before `at` takes: a
    /// whole note reference mark, or the text's own character. The pages'
    /// offsets count the marks; the paragraph's text does not hold them.
    fn drawn_char_before(&self, (paragraph, offset): Position) -> Option<usize> {
        if offset == 0 {
            return None;
        }
        if let Some(mark) = self
            .note_marks()
            .into_iter()
            .find(|mark| mark.after() == (paragraph, offset))
        {
            return Some(mark.len);
        }
        let document = self.word_document();
        let model = document.rendered_to_model_text_position(TextPosition::new(paragraph, offset));
        let text: String = document
            .paragraph(paragraph)?
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        text.get(..model.offset.min(text.len()))?
            .chars()
            .next_back()
            .map(char::len_utf8)
    }

    /// The note a paragraph surface belongs to, when it is one's text.
    fn note_story_at(&self, paragraph: usize) -> Option<(NoteKind, String)> {
        match self.word_document().paragraph_story(paragraph)? {
            ParagraphStory::Footnote { note_id, .. } => Some((NoteKind::Footnote, note_id)),
            ParagraphStory::Endnote { note_id, .. } => Some((NoteKind::Endnote, note_id)),
            _ => None,
        }
    }

    /// Where a note's text starts: its first paragraph, after its number.
    fn note_text_position(&self, kind: NoteKind, id: &str) -> Option<Position> {
        let document = self.word_document();
        let story = match kind {
            NoteKind::Footnote => ParagraphStory::Footnote {
                note_id: id.to_string(),
                paragraph: 0,
            },
            NoteKind::Endnote => ParagraphStory::Endnote {
                note_id: id.to_string(),
                paragraph: 0,
            },
        };
        let surface = document.paragraph_story_surface(&story)?;
        let at = document.model_to_rendered_text_position(TextPosition::new(surface, 0));
        Some((at.paragraph, at.offset))
    }

    /// The note at a position: the one whose text it is in, or whose mark
    /// it stands beside.
    fn note_at(&self, at: Position) -> Option<(NoteKind, String)> {
        if let Some(note) = self.note_story_at(at.0) {
            return Some(note);
        }
        self.note_marks()
            .into_iter()
            .find(|mark| mark.at.0 == at.0 && mark.at.1 <= at.1 && at.1 <= mark.after().1)
            .map(|mark| (mark.kind, mark.id))
    }

    /// 注の表示: from a note's text to its mark; from a mark to its note;
    /// elsewhere, to the next note's text (the first one when none follows).
    fn show_note(&self, at: Position) -> Option<Position> {
        if let Some((kind, id)) = self.note_story_at(at.0) {
            return self
                .note_marks()
                .into_iter()
                .find(|mark| mark.kind == kind && mark.id == id)
                .map(|mark| mark.after());
        }
        let marks = self.note_marks();
        let mark = marks
            .iter()
            .find(|mark| mark.at.0 == at.0 && mark.at.1 <= at.1 && at.1 <= mark.after().1)
            .or_else(|| marks.iter().find(|mark| mark.at >= at))
            .or_else(|| marks.first())?;
        self.note_text_position(mark.kind, &mark.id)
    }

    /// 脚注に変換 / 文末脚注に変換 on one note, one undo step.
    fn convert_note(&mut self, kind: NoteKind, id: &str) -> Result<(), String> {
        let id = id.to_string();
        edit_document(self, "convert_note", true, move |document, _| match kind {
            NoteKind::Footnote => document.convert_footnote_to_endnote(&id).map(|_| ()),
            NoteKind::Endnote => document.convert_endnote_to_footnote(&id).map(|_| ()),
        })
    }

    /// 変換: every footnote to an endnote, every endnote to a footnote, or
    /// the two swapped — one undo step.
    fn convert_all_notes(&mut self, conversion: NoteConversion) -> Result<(), String> {
        let footnotes: Vec<String> = self
            .note_marks()
            .into_iter()
            .filter(|mark| mark.kind == NoteKind::Footnote)
            .map(|mark| mark.id)
            .collect();
        let endnotes: Vec<String> = self
            .note_marks()
            .into_iter()
            .filter(|mark| mark.kind == NoteKind::Endnote)
            .map(|mark| mark.id)
            .collect();
        let (footnotes, endnotes) = match conversion {
            NoteConversion::FootnotesToEndnotes => (unique(footnotes), Vec::new()),
            NoteConversion::EndnotesToFootnotes => (Vec::new(), unique(endnotes)),
            NoteConversion::Swap => (unique(footnotes), unique(endnotes)),
        };
        if footnotes.is_empty() && endnotes.is_empty() {
            return Err("no notes to convert".to_string());
        }
        edit_document(self, "convert_all_notes", true, move |document, _| {
            // The endnotes go first, so a swap does not take the footnotes
            // that just became endnotes back again.
            for id in &endnotes {
                document.convert_endnote_to_footnote(id)?;
            }
            for id in &footnotes {
                document.convert_footnote_to_endnote(id)?;
            }
            Ok(())
        })
    }

    /// 脚注と文末脚注, opened on a kind with the caret's section's settings.
    fn note_dialog(&self, kind: NoteKind) -> NoteDialog {
        let document = self.word_document();
        let section = self
            .selection_range()
            .and_then(|range| document.section_index_for_paragraph(range.focus.paragraph))
            .unwrap_or(0);
        let sections = document
            .document
            .as_ref()
            .map_or(1, |document| document.sections.len().max(1));
        let mut dialog = NoteDialog {
            kind,
            footnote_place: FootnotePlace::PageBottom,
            endnote_place: EndnotePlace::DocumentEnd,
            format: String::new(),
            start: 1,
            restart: NoteRestart::Continuous,
            scope: NoteScope::WholeDocument,
            section,
            sections,
            separator: true,
            continuation_separator: true,
        };
        load_numbering(document, &mut dialog);
        dialog
    }

    /// The dialog switched to the other kind, its numbering read again.
    fn switch_note_dialog(&self, dialog: &mut NoteDialog, kind: NoteKind) {
        dialog.kind = kind;
        load_numbering(self.word_document(), dialog);
    }

    /// 適用: the dialog's place and numbering on its sections, one undo step.
    fn apply_note_dialog(&mut self, dialog: &NoteDialog) -> Result<(), String> {
        if dialog.start == 0 {
            return Err("開始番号は 1 以上にしてください".to_string());
        }
        let dialog = dialog.clone();
        edit_document(self, "note_options", true, move |document, _| {
            let footnote = is_footnote(dialog.kind);
            let restart = match (dialog.kind, dialog.restart) {
                (NoteKind::Endnote, NoteRestart::EachPage) => NoteRestart::Continuous,
                (_, restart) => restart,
            };
            for section in dialog.targets() {
                document.set_note_numbering(
                    footnote,
                    &dialog.format,
                    dialog.start,
                    restart.token(),
                    Some(section),
                )?;
                match dialog.kind {
                    NoteKind::Endnote => {
                        document.set_endnote_position(
                            match dialog.endnote_place {
                                EndnotePlace::SectionEnd => EndnotePosition::SectionEnd,
                                EndnotePlace::DocumentEnd => EndnotePosition::DocumentEnd,
                            },
                            Some(section),
                        )?;
                    }
                    NoteKind::Footnote => {
                        document.set_footnote_beneath_text(
                            dialog.footnote_place == FootnotePlace::BeneathText,
                            Some(section),
                        )?;
                    }
                }
            }
            for (continuation, shown) in [
                (false, dialog.separator),
                (true, dialog.continuation_separator),
            ] {
                // A document without the note has Word's default rule, and
                // nothing to empty.
                if document.note_separator_shown(footnote, continuation) != shown {
                    document.set_note_separator_shown(footnote, continuation, shown)?;
                }
            }
            Ok(())
        })
    }
}

impl<T: WordSession + ?Sized> WordNotes for T {}

fn unique(ids: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    for id in ids {
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
    seen
}

/// The dialog's numbering and place, read from its section.
fn load_numbering(document: &WordDocument, dialog: &mut NoteDialog) {
    let footnote = is_footnote(dialog.kind);
    let numbering = document
        .note_numbering(footnote, Some(dialog.section))
        .unwrap_or_default();
    let format = numbering
        .number_format
        .unwrap_or_else(|| if footnote { "decimal" } else { "lowerRoman" }.to_string());
    // Stored lowercased; the list holds Word's camel-case tokens.
    dialog.format = NOTE_FORMATS
        .iter()
        .find(|token| token.eq_ignore_ascii_case(&format))
        .map_or(format, |token| token.to_string());
    dialog.start = numbering.number_start.unwrap_or(1).max(1);
    dialog.restart = NoteRestart::from_token(numbering.number_restart.as_deref().unwrap_or(""));
    if let Ok(position) = document.endnote_position(Some(dialog.section)) {
        dialog.endnote_place = match position {
            EndnotePosition::SectionEnd => EndnotePlace::SectionEnd,
            EndnotePosition::DocumentEnd => EndnotePlace::DocumentEnd,
        };
    }
    dialog.separator = document.note_separator_shown(footnote, false);
    dialog.continuation_separator = document.note_separator_shown(footnote, true);
    dialog.footnote_place = if document.footnote_beneath_text(Some(dialog.section)) {
        FootnotePlace::BeneathText
    } else {
        FootnotePlace::PageBottom
    };
}
