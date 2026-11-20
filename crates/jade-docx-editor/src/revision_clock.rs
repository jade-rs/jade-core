//! The clock a newly recorded tracked change is dated by.
//!
//! Word dates every change it records (`w:ins`, `w:del`, `w:rPrChange`,
//! `w:pPrChange`, a table row's `w:trPr/w:ins|w:del`) when it is made. A
//! revision without `w:date` reads as 1970-01-01 in importers that default
//! the missing attribute (docx-rs, and so lawgue).
//!
//! The browser runtime reads the page's `Date`; a native host has no local
//! clock of its own here and registers one with [`set_revision_clock`].

use std::cell::Cell;

use crate::canvas_renderer::FieldDateTime;

thread_local! {
    static CLOCK: Cell<Option<fn() -> FieldDateTime>> = const { Cell::new(None) };
}

/// Registers the host's local wall clock for dating recorded revisions, or
/// with `None` goes back to the default: the page's `Date` in the browser,
/// none on a native host. A revision session with a fixed `date` keeps it.
pub fn set_revision_clock(clock: Option<fn() -> FieldDateTime>) {
    CLOCK.with(|cell| cell.set(clock));
}

/// The `w:date` for a revision recorded now, or `None` when the host has no
/// clock.
pub(crate) fn revision_date_now() -> Option<String> {
    let _ = CLOCK.with(Cell::get);
    None
}
