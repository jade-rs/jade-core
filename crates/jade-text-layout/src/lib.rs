//! Format-neutral text layout: where a line may break, and a greedy wrapper
//! that respects it.
//!
//! ## What is here and what is not
//!
//! `jade-docx-layout` owns Word's wrap engine, and that engine is not general:
//! it carries drop caps, tab stops, float exclusions, hanging punctuation, and
//! the per-document compatibility toggles (`kinsoku`, `wordWrap`,
//! `overflowPunct`, `autoSpaceDE`) that decide Word's exact line ends. A
//! spreadsheet cell has none of those — a workbook cannot express them — so
//! running a cell through that engine would mean fabricating Word settings and
//! inheriting decisions Excel does not make.
//!
//! What the two genuinely share is the script-level question of where a break
//! is allowed at all. That is [`kinsoku`], and it is the single table both
//! read. [`wrap`] on top of it is the simple greedy fill a cell or a text box
//! wants; Word's paragraphs do not use it.

pub mod kinsoku;
pub mod wrap;

pub use kinsoku::{forbids_line_end, forbids_line_start};
pub use wrap::{wrap_text, WrapOptions};
