//! Word's other file formats, to and from `.docx`.
//!
//! Each format is a reader into the [`flow`] model and a writer out of it;
//! `.docx` is one more of each ([`docx_read`], [`docx_write`]). Opening a
//! file in another format is reading it and writing the flow document as a
//! `.docx` for the editor; saving to one is reading the editor's `.docx`
//! and writing the format.
//!
//! - [`text`]: plain text in any of Word's encodings, Shift_JIS among them.
//! - [`rtf`]: Rich Text Format.
//! - [`odt`]: OpenDocument Text.
//! - [`html`]: HTML pages.
//! - `pdf` (with the `pdf` feature): a PDF opened as an editable document.

pub mod docx_read;
pub(crate) use docx_read::hyperlink_instruction as docx_read_hyperlink;
pub mod docx_write;
pub mod flow;
pub mod html;
pub mod odt;
#[cfg(feature = "pdf")]
pub mod pdf;
pub mod rtf;
pub mod text;
pub mod xml;

pub use docx_read::read_docx;
pub use docx_write::write_docx;
pub use flow::FlowDocument;
