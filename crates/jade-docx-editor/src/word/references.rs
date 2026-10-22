//! The 参考資料 tab over any [`WordSession`]: 脚注の挿入 and 文末脚注の
//!挿入 (⌥⌘F and ⌥⌘D in Word for Mac) at the caret — the note's text typed
//! into a bar and put in with its reference as one undo step — and the table
//! of contents, inserted from the headings and updated after they change.

pub mod bookmarks;
pub mod captions;
pub mod citations;
pub mod crossref;
pub mod index;
pub mod navigation;
pub mod notes;
pub mod toc;

use jade_docx_headless_api::HeadlessCommand;

use crate::commands::document_edit::DocumentEditCommand;
use crate::word_document::WordDocument;
use jade_selection::SelectionRange;

use super::{Position, WordSession};
use crate::text_edit::TextEdit;

/// Runs `edit` on the document as one undo step. It is handed the
/// selection to move; with `keep_selection` the selection stays as it was.
pub(crate) fn edit_document<S: WordSession + ?Sized>(
    session: &mut S,
    name: &'static str,
    keep_selection: bool,
    mut edit: impl FnMut(&mut WordDocument, &mut Option<SelectionRange>) -> Result<(), String> + 'static,
) -> Result<(), String> {
    session.execute_command(
        Box::new(DocumentEditCommand::new(
            name,
            Box::new(move |document, selection| edit(document, selection)),
        )),
        keep_selection,
    )
}

/// Which notes: at the foot of the page, or at the end of the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Footnote,
    Endnote,
}

/// A note being typed: where its reference goes, and its text.
pub struct NoteForm {
    pub kind: NoteKind,
    pub at: Position,
    pub text: TextEdit,
}

impl NoteForm {
    /// The note's reference goes where Word puts it: at the caret, or after
    /// the selection.
    pub fn new<S: WordSession + ?Sized>(session: &S, kind: NoteKind) -> Self {
        Self {
            kind,
            at: session.note_anchor(),
            text: TextEdit::new(String::new()),
        }
    }

    /// OK: the reference and the note, one undo step. Returns where the caret
    /// goes: where the reference went.
    pub fn submit<S: WordSession + ?Sized>(&self, session: &mut S) -> Result<Position, String> {
        session.insert_note(self.kind, self.at, self.text.text.trim())
    }
}

/// 参考資料 over every session.
pub trait WordReferences: WordSession {
    /// Where a note's reference goes: the caret, or the end of the selection.
    fn note_anchor(&self) -> Position {
        match self.selection_range() {
            Some(range) => {
                let (_, end) = range.normalized();
                (end.paragraph, end.offset)
            }
            None => (0, 0),
        }
    }

    /// Puts a note's reference mark at `at` and the note, holding `text`, in
    /// `footnotes.xml` or `endnotes.xml` (ECMA-376 Part 1 §17.11), one undo
    /// step. Returns where the caret goes: where the reference went.
    fn insert_note(
        &mut self,
        kind: NoteKind,
        at: Position,
        text: &str,
    ) -> Result<Position, String> {
        let (paragraph, offset) = at;
        let text = text.to_string();
        let command = match kind {
            NoteKind::Footnote => HeadlessCommand::InsertFootnote {
                paragraph,
                offset,
                text,
            },
            NoteKind::Endnote => HeadlessCommand::InsertEndnote {
                paragraph,
                offset,
                text,
            },
        };
        // The caret goes after the reference mark, as Word leaves it when
        // the note's text is done; the engine leaves its selection in the
        // note.
        self.apply_command(command, false)?;
        let caret = notes::WordNotes::note_marks(self)
            .into_iter()
            .find(|mark| mark.at == at)
            .map_or(at, |mark| mark.after());
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(caret.0, caret.1),
        )));
        Ok(caret)
    }

    /// The notes of a kind the document has, in the order they are first
    /// referred to: each one's text.
    fn note_texts(&self, kind: NoteKind) -> Vec<String> {
        let notes = match kind {
            NoteKind::Footnote => self.word_document().footnote_infos(),
            NoteKind::Endnote => self.word_document().endnote_infos(),
        };
        notes.into_iter().map(|note| note.text).collect()
    }

    /// 目次: a table of contents from headings 1–3, after the caret's
    /// paragraph.
    fn insert_table_of_contents(&mut self) -> Result<(), String> {
        let after_paragraph = self
            .selection_range()
            .map(|range| range.normalized().0.paragraph)
            .unwrap_or(0);
        self.apply_command(
            HeadlessCommand::InsertTableOfContents {
                after_paragraph,
                outline_min: 1,
                outline_max: 3,
            },
            true,
        )
    }

    /// 目次の更新: the document's table of contents made again from its
    /// headings as they now are.
    fn update_table_of_contents(&mut self) -> Result<(), String> {
        self.apply_command(HeadlessCommand::UpdateTableOfContents, true)
    }
}

impl<T: WordSession + ?Sized> WordReferences for T {}
