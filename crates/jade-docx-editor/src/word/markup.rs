//! How a document's tracked changes are shown, and deciding only the ones
//! on show: Word's 変更内容の表示 (シンプルな変更履歴/コメント, すべての変更
//! 履歴/コメント, 変更履歴/コメントなし, 初版), its 変更履歴とコメントの表示
//! menu — コメント, 挿入と削除, 書式設定, 校閲者 one by one — with each
//! reviewer in a colour of their own, and 表示されたすべての変更を反映 /
//! 元に戻す.

use crate::revision_view::{colors_by_author, RevisionView};

use super::review::{Revision, WordReview};
use super::WordSession;

/// 変更内容の表示.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkupMode {
    /// シンプルな変更履歴/コメント: the text as it will read, a red bar
    /// beside each changed line.
    Simple,
    /// すべての変更履歴/コメント: every change marked up in place.
    #[default]
    All,
    /// 変更履歴/コメントなし: the text as it will read, nothing marked.
    NoMarkup,
    /// 初版: the text before the changes.
    Original,
}

impl MarkupMode {
    pub const ALL: [MarkupMode; 4] = [
        MarkupMode::Simple,
        MarkupMode::All,
        MarkupMode::NoMarkup,
        MarkupMode::Original,
    ];

    /// The i18n key of its name in the 変更内容の表示 list.
    pub fn label_key(self) -> &'static str {
        match self {
            MarkupMode::Simple => "review.markup.simple",
            MarkupMode::All => "review.markup.all",
            MarkupMode::NoMarkup => "review.markup.none",
            MarkupMode::Original => "review.markup.original",
        }
    }
}

/// Word's colour for the changed-line bar in シンプルな変更履歴.
pub const SIMPLE_CHANGE_BAR: &str = "#C00000";
/// And beside inline markup, where Word draws it in the text colour.
pub const ALL_CHANGE_BAR: &str = "#595959";

/// The 変更内容の表示 mode and the 変更履歴とコメントの表示 menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupOptions {
    pub mode: MarkupMode,
    /// コメント: the comments pane and the commented text's highlight.
    pub comments: bool,
    /// 挿入と削除.
    pub insertions_deletions: bool,
    /// 書式設定.
    pub formatting: bool,
    /// 校閲者 whose changes are not shown.
    pub hidden_reviewers: Vec<String>,
}

impl Default for MarkupOptions {
    fn default() -> Self {
        Self {
            mode: MarkupMode::All,
            comments: true,
            insertions_deletions: true,
            formatting: true,
            hidden_reviewers: Vec::new(),
        }
    }
}

impl MarkupOptions {
    /// Whether `reviewer`'s changes are on show.
    pub fn shows_reviewer(&self, reviewer: &str) -> bool {
        !self.hidden_reviewers.iter().any(|name| name == reviewer)
    }

    /// すべての校閲者 / one reviewer, ticked or unticked.
    pub fn set_reviewer_shown(&mut self, reviewer: &str, shown: bool) {
        self.hidden_reviewers.retain(|name| name != reviewer);
        if !shown {
            self.hidden_reviewers.push(reviewer.to_string());
        }
    }

    /// Whether `revision` is one the view shows: its kind is on and its
    /// reviewer is ticked.
    pub fn shows(&self, revision: &Revision) -> bool {
        let kind_shown = if revision.is_formatting() {
            self.formatting
        } else {
            self.insertions_deletions
        };
        kind_shown
            && revision
                .author
                .as_deref()
                .is_none_or(|author| self.shows_reviewer(author))
    }

    /// What the renderer is told for `reviewers`, in their order of
    /// appearance: markup on, the original instead of the final text, and
    /// the view.
    pub fn display(&self, reviewers: &[String]) -> (bool, bool, RevisionView) {
        let markup = self.mode == MarkupMode::All;
        let original = self.mode == MarkupMode::Original;
        let change_bar = match self.mode {
            MarkupMode::Simple => Some(SIMPLE_CHANGE_BAR),
            MarkupMode::All => Some(ALL_CHANGE_BAR),
            MarkupMode::NoMarkup | MarkupMode::Original => None,
        };
        let view = RevisionView {
            change_bar: change_bar.map(Into::into),
            author_colors: colors_by_author(reviewers.iter().map(String::as_str)),
            hidden_authors: self.hidden_reviewers.clone(),
            hide_insertions_deletions: !self.insertions_deletions,
            hide_formatting: !self.formatting,
        };
        (markup, original, view)
    }
}

/// A document whose pages can show its changes another way.
pub trait WordMarkupDisplay {
    /// Markup on, the original instead of the final text, and the view;
    /// returns whether the pages changed.
    fn set_revision_display(
        &mut self,
        markup: bool,
        original: bool,
        view: RevisionView,
    ) -> Result<bool, String>;
}

/// Markup views and the decisions on what is on show, for every session
/// that can show them.
pub trait WordMarkup: WordSession + WordMarkupDisplay {
    /// Every reviewer — of a change or a comment — in the order they first
    /// appear, as the 校閲者 list names them and colours are handed out.
    fn reviewers(&self) -> Vec<String> {
        let mut reviewers: Vec<String> = Vec::new();
        let mut add = |name: Option<String>| {
            if let Some(name) = name.filter(|name| !name.is_empty()) {
                if !reviewers.contains(&name) {
                    reviewers.push(name);
                }
            }
        };
        for revision in self.revisions() {
            add(revision.author);
        }
        fn walk(comments: Vec<super::review::Comment>, add: &mut dyn FnMut(Option<String>)) {
            for comment in comments {
                add(comment.author);
                walk(comment.replies, add);
            }
        }
        walk(self.comments(), &mut add);
        reviewers
    }

    /// Shows the document's changes as `options` say; returns whether the
    /// pages changed.
    fn show_markup(&mut self, options: &MarkupOptions) -> Result<bool, String> {
        let (markup, original, view) = options.display(&self.reviewers());
        self.set_revision_display(markup, original, view)
    }

    /// The changes on show: every one whose kind and reviewer the options
    /// show.
    fn visible_revisions(&self, options: &MarkupOptions) -> Vec<Revision> {
        self.revisions()
            .into_iter()
            .filter(|revision| options.shows(revision))
            .collect()
    }

    /// 表示されたすべての変更を反映 (or 元に戻す): only the changes on show
    /// are decided, as one undo step; the others stay.
    fn decide_visible_revisions(
        &mut self,
        options: &MarkupOptions,
        accept: bool,
    ) -> Result<(), String> {
        let visible = self.visible_revisions(options);
        self.decide_revisions(&visible, accept)
    }
}

impl<T: WordSession + WordMarkupDisplay + ?Sized> WordMarkup for T {}

/// What a formatting change changed, as the 変更履歴ウィンドウ and Word's
/// balloons name it: i18n keys of each property that differs.
pub fn formatting_changed(
    before: &crate::document::RunProperties,
    after: &crate::document::RunProperties,
) -> Vec<&'static str> {
    let on = |value: Option<bool>| value.unwrap_or(false);
    let mut changed = Vec::new();
    if on(before.bold) != on(after.bold) {
        changed.push(if on(after.bold) {
            "review.format.bold"
        } else {
            "review.format.not_bold"
        });
    }
    if on(before.italic) != on(after.italic) {
        changed.push(if on(after.italic) {
            "review.format.italic"
        } else {
            "review.format.not_italic"
        });
    }
    if on(before.underline) != on(after.underline) || before.underline_kind != after.underline_kind
    {
        changed.push("review.format.underline");
    }
    if on(before.strike) != on(after.strike) || on(before.double_strike) != on(after.double_strike)
    {
        changed.push("review.format.strike");
    }
    if before.font_size != after.font_size {
        changed.push("review.format.size");
    }
    if before.fonts != after.fonts || before.font_family != after.font_family {
        changed.push("review.format.font");
    }
    if before.color != after.color || before.theme_color != after.theme_color {
        changed.push("review.format.color");
    }
    if before.highlight != after.highlight || before.shading != after.shading {
        changed.push("review.format.highlight");
    }
    if before.baseline != after.baseline || before.text_position != after.text_position {
        changed.push("review.format.position");
    }
    if before.style != after.style {
        changed.push("review.format.style");
    }
    if changed.is_empty() && before != after {
        changed.push("review.format.other");
    }
    changed
}

/// What the 変更履歴ウィンドウ says a formatting change changed: the
/// properties of the first run in it that carries the change, or the
/// paragraph's when it is the paragraph's own.
pub fn describe_formatting<S: WordSession + ?Sized>(
    session: &S,
    revision: &Revision,
) -> Vec<&'static str> {
    if !revision.is_formatting() {
        return Vec::new();
    }
    let Some(paragraph) = session.word_document().paragraph(revision.paragraph) else {
        return Vec::new();
    };
    let mut offset = 0;
    for run in &paragraph.runs {
        let (start, end) = (offset, offset + run.text.len());
        offset = end;
        if end <= revision.start || start >= revision.end.max(revision.start + 1) {
            continue;
        }
        if let Some(properties) = run.properties.as_ref() {
            if let Some(change) = properties.change.as_deref() {
                let mut after = properties.clone();
                after.change = None;
                return formatting_changed(&change.previous, &after);
            }
        }
    }
    vec!["review.format.paragraph"]
}

/// The i18n key of a change's kind, as the 変更履歴ウィンドウ names it.
pub fn kind_key(revision: &Revision) -> &'static str {
    match revision.kind.as_str() {
        "inserted" => "review.kind.inserted",
        "deleted" => "review.kind.deleted",
        "movedFrom" => "review.kind.moved_from",
        "movedTo" => "review.kind.moved_to",
        "formatted" => "review.kind.formatted",
        "insertedRow" => "review.kind.inserted_row",
        "deletedRow" => "review.kind.deleted_row",
        kind if kind.to_ascii_lowercase().starts_with("ins") => "review.kind.inserted",
        _ => "review.kind.deleted",
    }
}
