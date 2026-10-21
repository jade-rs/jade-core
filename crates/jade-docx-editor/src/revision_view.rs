//! How tracked changes are shown, beyond markup on or off: Word's 変更内容の
//! 表示 (シンプルな変更履歴, すべての変更履歴, 変更履歴なし, 初版) and its
//! 変更履歴とコメントの表示 menu — which kinds of change are marked up, whose
//! changes are, and the colour each reviewer's changes are drawn in.
//!
//! A [`RevisionView`] rides on [`crate::Options`]; the renderer reads it for
//! the pass it draws. It changes only what is drawn: the document, its
//! revisions and the selection's offsets are the same whatever it says.

use jade_docx_reader::document::{RevisionKind, RevisionMark};
use std::rc::Rc;

/// What Word's markup views and filters say about drawing tracked changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RevisionView {
    /// 変更された行: a bar in the left margin beside each line a change is
    /// on, in this colour. Word draws it red in シンプルな変更履歴 and grey
    /// beside inline markup.
    pub change_bar: Option<Rc<str>>,
    /// 作成者別: each reviewer's changes in their own colour, by author name.
    /// An author missing from the list keeps the default markup colours.
    pub author_colors: Vec<(String, Rc<str>)>,
    /// 校閲者 unchecked: these reviewers' changes are shown as they would be
    /// once accepted — insertions as plain text, deletions gone — with no
    /// markup, while everyone else's stay marked up.
    pub hidden_authors: Vec<String>,
    /// 挿入と削除 unchecked: no insertion or deletion is marked up, for
    /// anyone; the text reads as accepted.
    pub hide_insertions_deletions: bool,
    /// 書式設定 unchecked: formatting changes get no change bar.
    pub hide_formatting: bool,
}

impl RevisionView {
    /// Whether the view changes nothing about how markup is drawn.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// Whether `mark` is marked up when markup is shown at all: its kind and
    /// its author are among those the view shows.
    pub fn marks_up(&self, mark: &RevisionMark) -> bool {
        !self.hide_insertions_deletions && !self.hides_author(mark.author.as_deref())
    }

    /// Whether a change by `author` is left out of the markup.
    pub fn hides_author(&self, author: Option<&str>) -> bool {
        author.is_some_and(|author| self.hidden_authors.iter().any(|name| name == author))
    }

    /// The colour `mark`'s author's changes are drawn in, when the view
    /// colours by author.
    pub fn color_for(&self, mark: &RevisionMark) -> Option<&Rc<str>> {
        let author = mark.author.as_deref()?;
        self.author_colors
            .iter()
            .find(|(name, _)| name == author)
            .map(|(_, color)| color)
    }

    /// Whether, with markup on, `mark`'s text is left out of the layout: a
    /// deletion or move away that the view shows as accepted.
    pub fn drops(&self, mark: &RevisionMark) -> bool {
        !self.marks_up(mark) && matches!(mark.kind, RevisionKind::Deleted | RevisionKind::MovedFrom)
    }
}

/// Word's colours for reviewers 作成者別, in the order it hands them out:
/// the first reviewer's changes red, the next blue, and so on, the list
/// starting again after the eighth.
pub const AUTHOR_COLORS: [&str; 8] = [
    "#B5082E", // dark red
    "#2E74B5", // blue
    "#00843D", // green
    "#7030A0", // violet
    "#C55A11", // orange
    "#008080", // teal
    "#BF8F00", // dark yellow
    "#C00080", // pink
];

/// Each author in `authors` (in order of first appearance, repeats
/// ignored) with its colour from [`AUTHOR_COLORS`].
pub fn colors_by_author<'a>(authors: impl IntoIterator<Item = &'a str>) -> Vec<(String, Rc<str>)> {
    let mut colors: Vec<(String, Rc<str>)> = Vec::new();
    for author in authors {
        if colors.iter().any(|(name, _)| name == author) {
            continue;
        }
        let color = AUTHOR_COLORS[colors.len() % AUTHOR_COLORS.len()];
        colors.push((author.to_string(), Rc::from(color)));
    }
    colors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(kind: RevisionKind, author: &str) -> RevisionMark {
        RevisionMark {
            kind,
            id: Some("1".into()),
            author: Some(author.into()),
            date: None,
            date_utc: None,
            insertion: None,
        }
    }

    #[test]
    fn a_hidden_reviewer_reads_as_accepted() {
        let view = RevisionView {
            hidden_authors: vec!["Ann".into()],
            ..RevisionView::default()
        };
        assert!(!view.marks_up(&mark(RevisionKind::Inserted, "Ann")));
        assert!(view.drops(&mark(RevisionKind::Deleted, "Ann")));
        assert!(!view.drops(&mark(RevisionKind::Inserted, "Ann")));
        assert!(view.marks_up(&mark(RevisionKind::Deleted, "Bob")));
        assert!(!view.drops(&mark(RevisionKind::Deleted, "Bob")));
        assert!(!view.is_plain());
        assert!(RevisionView::default().is_plain());
    }

    #[test]
    fn reviewers_take_colours_in_order_of_appearance() {
        let colors = colors_by_author(["Ann", "Bob", "Ann", "Cy"]);
        let names: Vec<_> = colors.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["Ann", "Bob", "Cy"]);
        assert_eq!(&*colors[0].1, AUTHOR_COLORS[0]);
        assert_eq!(&*colors[1].1, AUTHOR_COLORS[1]);
        let view = RevisionView {
            author_colors: colors,
            ..RevisionView::default()
        };
        assert_eq!(
            view.color_for(&mark(RevisionKind::Inserted, "Bob"))
                .map(|color| &**color),
            Some(AUTHOR_COLORS[1])
        );
        assert!(view
            .color_for(&mark(RevisionKind::Inserted, "Dee"))
            .is_none());
    }
}
