//! The 校閲 tab over any [`WordSession`]: comments — written, answered,
//! resolved, deleted, listed for a pane — and tracked changes — turned on
//! and off, walked one by one, accepted or rejected one at a time or all at
//! once.

use crate::headless_api::revision_targets_for;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessCommentThreadSnapshot, HeadlessQuery, HeadlessQueryResult,
    HeadlessRevisionSnapshot, HeadlessRevisionTarget,
};

use super::reviewer::Reviewer;
use super::{query, Position, WordSession};

/// One comment and its replies, as the comments pane lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub id: String,
    pub author: Option<String>,
    pub date: Option<String>,
    pub text: String,
    pub resolved: bool,
    /// The text the comment is on, from its start to its end; `None` for a
    /// comment whose anchor the document no longer has.
    pub anchor: Option<(Position, Position)>,
    pub replies: Vec<Comment>,
}

impl Comment {
    fn from_thread(thread: HeadlessCommentThreadSnapshot) -> Self {
        let comment = thread.comment;
        let anchor = match (comment.anchors.first(), comment.anchors.last()) {
            (Some(first), Some(last)) => {
                Some(((first.paragraph, first.start), (last.paragraph, last.end)))
            }
            _ => None,
        };
        Self {
            id: comment.id,
            author: comment.author,
            date: comment.date,
            text: comment.text,
            resolved: comment.resolved.unwrap_or(false),
            anchor,
            replies: thread
                .replies
                .into_iter()
                .map(Comment::from_thread)
                .collect(),
        }
    }
}

/// One tracked change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    pub id: Option<String>,
    /// `insertion`, `deletion`, … as the engine names them.
    pub kind: String,
    pub author: Option<String>,
    pub date: Option<String>,
    pub paragraph: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
    /// The change is the paragraph's mark: a paragraph inserted or joined.
    pub paragraph_mark: bool,
    /// The revisions the engine lists it as, which Word shows and decides
    /// as one change.
    parts: Vec<HeadlessRevisionSnapshot>,
}

impl Revision {
    pub fn start(&self) -> Position {
        (self.paragraph, self.start)
    }

    pub fn end(&self) -> Position {
        (self.paragraph, self.end.max(self.start))
    }

    /// Whether the change was an insertion, as against a deletion.
    pub fn is_insertion(&self) -> bool {
        self.kind.to_ascii_lowercase().starts_with("ins")
    }

    /// Whether the change is to formatting (`w:rPrChange`, `w:pPrChange`)
    /// rather than to the text.
    pub fn is_formatting(&self) -> bool {
        self.kind.to_ascii_lowercase().starts_with("format")
    }

    /// Whether `next` carries this change on: the same author's same kind
    /// of change, starting where this one ends.
    fn continued_by(&self, next: &Revision) -> bool {
        next.paragraph == self.paragraph
            && next.kind == self.kind
            && next.author == self.author
            && !next.paragraph_mark
            && !self.paragraph_mark
            && next.start == self.end
    }
}

/// So many is every one a document has.
const ALL: usize = 100_000;

/// Comments and tracked changes for every session.
pub trait WordReview: WordSession {
    // ---- comments -----------------------------------------------------------

    /// The document's comments, each with its replies, in document order.
    fn comments(&self) -> Vec<Comment> {
        match query(
            self.word_document(),
            HeadlessQuery::CommentThreads {
                start_thread: 0,
                max_threads: ALL,
            },
        ) {
            Some(HeadlessQueryResult::CommentThreads { threads, .. }) => {
                threads.into_iter().map(Comment::from_thread).collect()
            }
            _ => Vec::new(),
        }
    }

    /// The ids of every comment and reply.
    fn comment_ids(&self) -> Vec<String> {
        fn walk(comments: &[Comment], ids: &mut Vec<String>) {
            for comment in comments {
                ids.push(comment.id.clone());
                walk(&comment.replies, ids);
            }
        }
        let mut ids = Vec::new();
        walk(&self.comments(), &mut ids);
        ids
    }

    /// 新しいコメント: a comment on the selection, or on the caret's
    /// paragraph. Returns the new comment's id.
    fn add_comment(&mut self, text: &str, author: &Reviewer) -> Result<Option<String>, String> {
        let before = self.comment_ids();
        let range = self.selection_range().ok_or("no caret")?;
        let (start, end) = range.normalized();
        let (start_offset, end_offset) = if range.is_collapsed() {
            (None, None)
        } else {
            (Some(start.offset), Some(end.offset))
        };
        self.apply_command(
            HeadlessCommand::AddComment {
                start_paragraph: start.paragraph,
                start_offset,
                end_paragraph: end.paragraph,
                end_offset,
                text: text.to_string(),
                author: Some(author.name.clone()),
                initials: author.initials(),
                date: None,
            },
            true,
        )?;
        Ok(self
            .comment_ids()
            .into_iter()
            .find(|id| !before.contains(id)))
    }

    /// 返信: a reply under comment `id`.
    fn reply_to_comment(&mut self, id: &str, text: &str, author: &Reviewer) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::ReplyToComment {
                comment_id: id.to_string(),
                text: text.to_string(),
                author: Some(author.name.clone()),
                initials: author.initials(),
                date: None,
            },
            true,
        )
    }

    /// コメント › 前へ / 次へ: the first comment whose text starts after
    /// `from` (or the last before it), wrapping round the document. A
    /// comment whose text is gone has no place and is passed over.
    fn step_comment(&self, from: Position, forwards: bool) -> Option<Comment> {
        let mut anchored: Vec<Comment> = self
            .comments()
            .into_iter()
            .filter(|comment| comment.anchor.is_some())
            .collect();
        anchored.sort_by_key(|comment| comment.anchor.map(|(start, _)| start));
        let start = |comment: &Comment| comment.anchor.map(|(start, _)| start).unwrap_or_default();
        if forwards {
            anchored
                .iter()
                .find(|comment| start(comment) > from)
                .or_else(|| anchored.first())
                .cloned()
        } else {
            anchored
                .iter()
                .rev()
                .find(|comment| start(comment) < from)
                .or_else(|| anchored.last())
                .cloned()
        }
    }

    /// Rewrites comment `id`'s text, as editing it in the pane does.
    fn edit_comment(&mut self, id: &str, text: &str) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::UpdateComment {
                comment_id: id.to_string(),
                text: text.to_string(),
            },
            true,
        )
    }

    /// 解決 (or 再び開く with `false`).
    fn resolve_comment(&mut self, id: &str, resolved: bool) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::ResolveComment {
                comment_id: id.to_string(),
                resolved,
            },
            true,
        )
    }

    /// 削除: the comment and its replies.
    fn delete_comment(&mut self, id: &str) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::DeleteComment {
                comment_id: id.to_string(),
            },
            true,
        )
    }

    // ---- tracked changes ----------------------------------------------------

    /// Whether edits are being recorded as revisions (変更履歴の記録).
    fn is_tracking_changes(&self) -> bool {
        self.word_document().revision_session().is_some()
    }

    fn set_tracking_changes(&mut self, on: bool, author: &Reviewer) -> Result<(), String> {
        // 変更履歴のロック: tracking stays on until the lock is lifted.
        if !on {
            use super::protection::WordProtection;
            if self.tracking_locked() {
                return Err("tracking is locked".to_string());
            }
        }
        let command = if on {
            HeadlessCommand::StartChangeTracking {
                author: Some(author.name.clone()),
                initials: author.initials(),
                date: None,
                date_utc: None,
            }
        } else {
            HeadlessCommand::StopChangeTracking
        };
        self.apply_command(command, true)
    }

    /// Every tracked change, in document order, the revisions a run of
    /// typing left joined into one.
    fn revisions(&self) -> Vec<Revision> {
        let Some(HeadlessQueryResult::Revisions { revisions, .. }) = query(
            self.word_document(),
            HeadlessQuery::Revisions {
                start_revision: 0,
                max_revisions: ALL,
                kind: None,
                author: None,
                date_from: None,
                date_to: None,
            },
        ) else {
            return Vec::new();
        };
        let mut revisions: Vec<Revision> = revisions
            .into_iter()
            .map(|revision| Revision {
                id: revision.id.clone(),
                kind: revision.kind.clone(),
                author: revision.author.clone(),
                date: revision.date.clone(),
                paragraph: revision.paragraph,
                start: revision.start_offset,
                end: revision.end_offset,
                text: revision.text.clone(),
                paragraph_mark: revision.paragraph_mark,
                parts: vec![revision],
            })
            .collect();
        revisions.sort_by_key(|revision| (revision.paragraph, revision.start, revision.end));
        // A word typed with changes tracked is a revision a keystroke; Word
        // shows them, and decides them, as one change.
        let mut changes: Vec<Revision> = Vec::with_capacity(revisions.len());
        for revision in revisions {
            match changes.last_mut() {
                Some(last) if last.continued_by(&revision) => {
                    last.end = revision.end;
                    last.text.push_str(&revision.text);
                    last.parts.extend(revision.parts);
                }
                _ => changes.push(revision),
            }
        }
        changes
    }

    /// The changes the selection touches, or the one the caret is in.
    fn revisions_at_selection(&self) -> Vec<Revision> {
        let Some(range) = self.selection_range() else {
            return Vec::new();
        };
        let (start, end) = range.normalized();
        let (start, end) = ((start.paragraph, start.offset), (end.paragraph, end.offset));
        self.revisions()
            .into_iter()
            .filter(|revision| {
                if start == end {
                    revision.start() <= start && start <= revision.end()
                } else {
                    revision.start() < end && start < revision.end()
                        || (revision.paragraph_mark
                            && (start.0..=end.0).contains(&revision.paragraph))
                }
            })
            .collect()
    }

    /// 次へ / 前へ: the first change after `from` (or the last before it),
    /// wrapping round the document.
    fn step_revision(&self, from: Position, forwards: bool) -> Option<Revision> {
        let revisions = self.revisions();
        if forwards {
            revisions
                .iter()
                .find(|revision| revision.start() > from)
                .or_else(|| revisions.first())
                .cloned()
        } else {
            revisions
                .iter()
                .rev()
                .find(|revision| revision.start() < from)
                .or_else(|| revisions.last())
                .cloned()
        }
    }

    /// The engine's targets for a change: its listed revisions, each of
    /// which the engine decides with every keystroke typed into it.
    fn targets_of(&self, change: &Revision) -> Vec<HeadlessRevisionTarget> {
        revision_targets_for(&change.parts.iter().collect::<Vec<_>>())
    }

    /// 承諾 or 元に戻す for these changes, as one undo step.
    fn decide_revisions(&mut self, revisions: &[Revision], accept: bool) -> Result<(), String> {
        if revisions.is_empty() {
            return Ok(());
        }
        // A move is decided whole, as Word does: its other half with it —
        // the same text moved by the same reviewer at the same time.
        let mut revisions = revisions.to_vec();
        let partner = |kind: &str| match kind {
            "movedFrom" => Some("movedTo"),
            "movedTo" => Some("movedFrom"),
            _ => None,
        };
        if revisions
            .iter()
            .any(|revision| partner(&revision.kind).is_some())
        {
            for other in self.revisions() {
                let pairs = revisions.iter().any(|revision| {
                    partner(&revision.kind) == Some(other.kind.as_str())
                        && revision.author == other.author
                        && revision.date == other.date
                        && revision.text.trim() == other.text.trim()
                });
                if pairs && !revisions.contains(&other) {
                    revisions.push(other);
                }
            }
        }
        let revision_targets = Some(
            revisions
                .iter()
                .flat_map(|revision| self.targets_of(revision))
                .collect(),
        );
        let command = if accept {
            HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets,
            }
        } else {
            HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets,
            }
        };
        self.apply_command(command, false)
    }

    /// Word's 承諾して次へ進む (or 元に戻して次へ進む): the changes at the
    /// selection decided, and the next one after them returned for the shell
    /// to select. `Ok(None)` when no change is left.
    fn decide_and_step(&mut self, accept: bool) -> Result<Option<Revision>, String> {
        let here = self.revisions_at_selection();
        let from = here
            .first()
            .map(Revision::start)
            .or_else(|| {
                self.selection_range()
                    .map(|range| (range.focus.paragraph, range.focus.offset))
            })
            .unwrap_or((0, 0));
        if here.is_empty() {
            return Ok(self.step_revision(from, true));
        }
        self.decide_revisions(&here, accept)?;
        // The decided text is gone or plain now; the next change is the first
        // one at or after where they started.
        let revisions = self.revisions();
        Ok(revisions
            .iter()
            .find(|revision| revision.start() >= from)
            .or_else(|| revisions.first())
            .cloned())
    }

    /// すべての変更を反映 (or 元に戻す).
    fn decide_all_revisions(&mut self, accept: bool) -> Result<(), String> {
        let command = if accept {
            HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }
        } else {
            HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }
        };
        self.apply_command(command, false)
    }
}

impl<T: WordSession + ?Sized> WordReview for T {}
