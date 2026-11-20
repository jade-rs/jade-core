//! The clock a newly recorded tracked change is dated by.
//!
//! Word dates every change it records (`w:ins`, `w:del`, `w:rPrChange`,
//! `w:pPrChange`, a table row's `w:trPr/w:ins|w:del`) when it is made. A
//! revision without `w:date` reads as 1970-01-01 in importers that default
//! the missing attribute (docx-rs, and so lawgue).
//!
//! The engine has no clock of its own: the host registers its local wall
//! clock. The browser view registers the page's `Date`; a native host (the
//! headless MCP server) registers its own.

use std::cell::Cell;

use crate::field_datetime::{datetime_from_unix, to_w3cdtf, unix_seconds, FieldDateTime};

/// What the host's clock reads now: the local wall clock and how far it is
/// ahead of UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostTime {
    pub local: FieldDateTime,
    pub utc_offset_seconds: i64,
}

thread_local! {
    static CLOCK: Cell<Option<fn() -> HostTime>> = const { Cell::new(None) };
}

/// Registers the host's local wall clock for dating recorded revisions, or
/// with `None` takes it away. A revision session with a fixed `date` keeps
/// that date either way.
pub fn set_revision_clock(clock: Option<fn() -> HostTime>) {
    CLOCK.with(|cell| cell.set(clock));
}

/// `(w:date, w16du:dateUtc)` for a revision recorded now, or `None` when the
/// host has registered no clock.
///
/// `w:date` takes the comment form (`to_word_comment_date`, the same
/// `to_w3cdtf`): the local wall clock with a `Z` suffix, which lawgue's
/// importer reads back as local time. `w16du:dateUtc` is the true UTC
/// instant, which Word 365 writes beside it and readers that localize the
/// time prefer. Word writes revision dates to the minute, seconds as `00`,
/// which also keeps one minute's typing in one revision.
pub(crate) fn revision_date_now() -> Option<(String, String)> {
    let now = CLOCK.with(Cell::get)?();
    let local = FieldDateTime {
        second: 0,
        ..now.local
    };
    let utc = datetime_from_unix(unix_seconds(&local) - now.utc_offset_seconds, 0);
    Some((to_w3cdtf(&local), to_w3cdtf(&utc)))
}
