//! ブックマーク: the dialog's list — sorted by name or by place, hidden
//! ones shown on request — and its 追加, 削除 and ジャンプ, over the
//! selection Word bookmarks (ECMA-376 Part 1 §17.13.6).

use super::edit_document;
use crate::word::{Position, WordSession};

/// A bookmark: its name and the text it spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarkInfo {
    pub name: String,
    pub start: Position,
    pub end: Position,
}

impl BookmarkInfo {
    /// A hidden bookmark — `_Toc…`, `_Ref…` — Word lists only on request.
    pub fn hidden(&self) -> bool {
        self.name.starts_with('_')
    }
}

/// 並べ替え.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookmarkSort {
    Name,
    Location,
}

/// The ブックマーク dialog's state; the name is typed in the shell's field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarkDialog {
    pub sort: BookmarkSort,
    /// 自動作成されたブックマークを表示する.
    pub show_hidden: bool,
    /// The bookmark picked in the list.
    pub selected: Option<String>,
}

impl Default for BookmarkDialog {
    fn default() -> Self {
        Self {
            sort: BookmarkSort::Name,
            show_hidden: false,
            selected: None,
        }
    }
}

/// Whether Word accepts a bookmark name: a letter first, then letters,
/// digits and underscores, 40 characters at most.
pub fn valid_bookmark_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    name.chars().count() <= 40
        && first.is_alphabetic()
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

/// Bookmarks over every session.
pub trait WordBookmarks: WordSession {
    /// Every bookmark in the body, in document order.
    fn bookmarks(&self) -> Vec<BookmarkInfo> {
        let document = self.word_document();
        let Some(model) = document.document.as_ref() else {
            return Vec::new();
        };
        let mut bookmarks: Vec<BookmarkInfo> = model
            .bookmarks
            .iter()
            .filter_map(|bookmark| {
                let start = document.bookmark_position(&bookmark.name)?;
                let end = document
                    .bookmark_end_position(&bookmark.name)
                    .unwrap_or(start);
                Some(BookmarkInfo {
                    name: bookmark.name.clone(),
                    start: (start.paragraph, start.offset),
                    end: (end.paragraph, end.offset),
                })
            })
            .collect();
        bookmarks.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.name.cmp(&b.name)));
        bookmarks
    }

    /// The dialog's list: hidden ones only when asked, sorted as asked.
    fn bookmark_list(&self, dialog: &BookmarkDialog) -> Vec<BookmarkInfo> {
        let mut list: Vec<BookmarkInfo> = self
            .bookmarks()
            .into_iter()
            .filter(|bookmark| dialog.show_hidden || !bookmark.hidden())
            .collect();
        if dialog.sort == BookmarkSort::Name {
            list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
        list
    }

    /// 追加: a bookmark around the selection, or at the caret. A name
    /// already used moves to here, as in Word. One undo step.
    fn add_bookmark(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim().to_string();
        if !valid_bookmark_name(&name) {
            return Err(
                "ブックマーク名は文字で始め、文字・数字・アンダースコアだけを使ってください"
                    .to_string(),
            );
        }
        let range = self
            .selection_range()
            .ok_or_else(|| "カーソルの位置がありません".to_string())?;
        let exists = self
            .bookmarks()
            .iter()
            .any(|bookmark| bookmark.name == name);
        edit_document(self, "add_bookmark", true, move |document, _| {
            if exists {
                document.delete_bookmark(&name)?;
            }
            document.create_bookmark_at_range(range, &name)
        })
    }

    /// 削除, one undo step.
    fn remove_bookmark(&mut self, name: &str) -> Result<(), String> {
        let name = name.to_string();
        edit_document(self, "delete_bookmark", true, move |document, _| {
            document.delete_bookmark(&name)
        })
    }

    /// ジャンプ: the bookmark's text, to select.
    fn bookmark_range(&self, name: &str) -> Option<(Position, Position)> {
        self.bookmarks()
            .into_iter()
            .find(|bookmark| bookmark.name == name)
            .map(|bookmark| (bookmark.start, bookmark.end))
    }

    /// ブックマーク表示: where the gray brackets go — `[` at each start, `]`
    /// at each end, an I-beam (start and end at one place) for a point.
    fn bookmark_marks(&self) -> Vec<(Position, Position)> {
        self.bookmarks()
            .into_iter()
            .filter(|bookmark| !bookmark.hidden())
            .map(|bookmark| (bookmark.start, bookmark.end))
            .collect()
    }
}

impl<T: WordSession + ?Sized> WordBookmarks for T {}

#[cfg(test)]
mod tests {
    use super::valid_bookmark_name;

    #[test]
    fn names_word_accepts() {
        assert!(valid_bookmark_name("Chapter_1"));
        assert!(valid_bookmark_name("見出し"));
        assert!(!valid_bookmark_name("1st"));
        assert!(!valid_bookmark_name("_hidden"));
        assert!(!valid_bookmark_name("a b"));
        assert!(!valid_bookmark_name(""));
        assert!(!valid_bookmark_name(&"a".repeat(41)));
    }
}
