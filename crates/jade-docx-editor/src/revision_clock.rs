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

use crate::field_datetime::{to_w3cdtf, FieldDateTime};

thread_local! {
    static CLOCK: Cell<Option<fn() -> FieldDateTime>> = const { Cell::new(None) };
}

/// Registers the host's local wall clock for dating recorded revisions, or
/// with `None` takes it away. A revision session with a fixed `date` keeps
/// that date either way.
pub fn set_revision_clock(clock: Option<fn() -> FieldDateTime>) {
    CLOCK.with(|cell| cell.set(clock));
}

/// The `w:date` for a revision recorded now, or `None` when the host has
/// registered no clock.
///
/// The form is the comment one (`to_word_comment_date`, the same
/// `to_w3cdtf`): the local wall clock with a `Z` suffix, which lawgue's
/// importer reads back as local time. Word writes a revision's date to the
/// minute, seconds as `00`, which also keeps one minute's typing in one
/// revision.
pub(crate) fn revision_date_now() -> Option<String> {
    let now = CLOCK.with(Cell::get)?();
    Some(to_w3cdtf(&FieldDateTime { second: 0, ..now }))
}
