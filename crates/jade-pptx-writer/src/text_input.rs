use crate::{
    format_text_range, replace_text_range, shape_text, EditSession, Result, TextFormat,
    TextFormatEdit, WriteError,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TypingKind {
    Insert,
    Backspace,
    Delete,
}

/// Browser UTF-16 range plus a stale-input precondition. A history hint joins
/// only adjacent compatible edits; omitted history always creates an atomic edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextInputEdit {
    pub shape_id: u32,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub expected_text: String,
    #[serde(default)]
    pub soft_break: bool,
    pub history: Option<TypingKind>,
    #[serde(default)]
    pub format: TextFormat,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInputResult {
    pub changed: bool,
    pub coalesced: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotesInputEdit {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub expected_text: String,
    #[serde(default)]
    pub soft_break: bool,
    pub history: Option<TypingKind>,
}

pub(crate) struct TypingGroup {
    slide: usize,
    shape_id: Option<u32>,
    kind: TypingKind,
    caret: usize,
    format: TextFormat,
}
impl EditSession {
    pub fn end_text_history(&mut self) {
        self.typing = None;
    }
    pub fn apply_text_input(
        &mut self,
        slide: usize,
        edit: &TextInputEdit,
    ) -> Result<TextInputResult> {
        edit.format.validate()?;
        let before = shape_text(self.bytes(), slide, edit.shape_id)?;
        if before.text != edit.expected_text {
            return Err(WriteError::StaleTextInput);
        }
        let inserted = edit.text.encode_utf16().count();
        let caret = edit
            .start
            .checked_add(inserted)
            .ok_or(WriteError::InvalidTextRange)?;
        let kind = edit.history.filter(|kind| {
            !edit.soft_break
                && !edit.text.contains('\n')
                && match kind {
                    TypingKind::Insert => inserted > 0,
                    TypingKind::Backspace | TypingKind::Delete => {
                        inserted == 0 && edit.start < edit.end
                    }
                }
        });
        let next = replace_text_range(
            self.bytes(),
            slide,
            edit.shape_id,
            edit.start,
            edit.end,
            &edit.text,
            edit.soft_break,
        )?;
        let next = if inserted > 0 && !edit.format.is_empty() {
            format_text_range(
                &next,
                slide,
                &TextFormatEdit {
                    shape_id: edit.shape_id,
                    start: edit.start,
                    end: caret,
                    format: edit.format.clone(),
                },
            )?
        } else {
            next
        };
        let group = kind.map(|kind| TypingGroup {
            slide,
            shape_id: Some(edit.shape_id),
            kind,
            caret,
            format: edit.format.clone(),
        });
        Ok(self.commit_text_input(next, group, edit.start, edit.end))
    }
    pub fn apply_notes_input(
        &mut self,
        slide: usize,
        edit: &NotesInputEdit,
    ) -> Result<TextInputResult> {
        crate::validate_text(&edit.text, true)?;
        let before = crate::notes_text(self.bytes(), slide)?;
        if before.text != edit.expected_text {
            return Err(WriteError::StaleTextInput);
        }
        let inserted = edit.text.encode_utf16().count();
        let caret = edit
            .start
            .checked_add(inserted)
            .ok_or(WriteError::InvalidTextRange)?;
        let kind = edit.history.filter(|kind| {
            !edit.soft_break
                && !edit.text.contains('\n')
                && match kind {
                    TypingKind::Insert => inserted > 0,
                    _ => inserted == 0 && edit.start < edit.end,
                }
        });
        let next = crate::xml::replace_notes_range(
            self.bytes(),
            slide,
            edit.start,
            edit.end,
            &edit.text,
            edit.soft_break,
        )?;
        let group = kind.map(|kind| TypingGroup {
            slide,
            shape_id: None,
            kind,
            caret,
            format: TextFormat::default(),
        });
        Ok(self.commit_text_input(next, group, edit.start, edit.end))
    }
    fn commit_text_input(
        &mut self,
        next: Vec<u8>,
        group: Option<TypingGroup>,
        start: usize,
        end: usize,
    ) -> TextInputResult {
        let coalesced = self
            .typing
            .as_ref()
            .zip(group.as_ref())
            .is_some_and(|(old, new)| {
                old.slide == new.slide
                    && old.shape_id == new.shape_id
                    && old.kind == new.kind
                    && old.format == new.format
                    && self.redo.is_empty()
                    && match old.kind {
                        TypingKind::Insert => start == end && start == old.caret,
                        TypingKind::Backspace => end == old.caret,
                        TypingKind::Delete => start == old.caret,
                    }
            });
        if next == *self.current {
            if group.is_none() {
                self.end_text_history();
            }
            return TextInputResult {
                changed: false,
                coalesced: false,
            };
        }
        if coalesced {
            self.replace_current(next);
        } else {
            self.commit(next);
        }
        self.typing = group;
        TextInputResult {
            changed: true,
            coalesced,
        }
    }
}
