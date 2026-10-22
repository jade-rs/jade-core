//! Renderer-independent DOCX parsing, editing, querying, and serialization.
//!
//! `jade-docx-document-core` owns the mutable document model used by both native
//! headless integrations and the browser editor. It intentionally has no
//! dependency on `jade-docx-layout`, `wasm-bindgen`, `web-sys`, or a canvas API.
//!
//! Most headless consumers should use the higher-level
//! `jade-docx-headless-editor` facade. Consumers embedding a long-lived parsed
//! document can use [`word_document::WordDocument`] and [`headless_api`]
//! directly to avoid repeated ZIP parsing and serialization.

// Shared implementation files historically import selection primitives via
// `jade_docx_layout::selection`. Alias this crate under that name so production
// builds resolve those imports without linking the layout engine.
extern crate self as jade_docx_layout;

/// Native ChartML parsing, authoring, embedded data, and portable previews.
#[path = "../../jade-docx-editor/src/chart.rs"]
pub(crate) mod chart;

/// Editing commands and undo/redo execution infrastructure.
#[path = "../../jade-docx-editor/src/commands/mod.rs"]
pub mod commands;

/// Patching a drawing object's XML in place.
#[path = "../../jade-docx-editor/src/drawing_xml.rs"]
pub mod drawing_xml;

/// Pixel work on pictures: colour adjustments, soft edges, background
/// removal.
#[path = "../../jade-docx-editor/src/picture_effects.rs"]
pub mod picture_effects;

/// The art borders' names, which the document model tells borders by.
#[path = "../../jade-docx-editor/src/art_border_names.rs"]
pub(crate) mod art_border_names;

/// SmartArt laid out from its data, and its parts written.
#[path = "../../jade-docx-editor/src/smartart.rs"]
pub mod smartart;

/// Parsed WordprocessingML document model.
pub mod document {
    pub use jade_docx_reader::document::*;
}

pub use jade_docx_reader::{
    comments, common, docx_xml, field_datetime, numbering, parser, settings, styles, theme,
};

/// Renderer-independent command and query adapters for headless callers.
#[path = "../../jade-docx-editor/src/headless_api.rs"]
pub mod headless_api;

/// Article numbers, article titles, chunks and cross-references — the second
/// structure Japanese contracts carry on top of the OOXML one.
#[path = "../../jade-docx-editor/src/legal_structure.rs"]
pub(crate) mod legal_structure;

/// Shared Word number formatting used by note editing coordinates.
#[path = "../../jade-docx-editor/src/number_format.rs"]
pub(crate) mod number_format;

/// Helpers that serialize the in-memory document model.
#[path = "../../jade-docx-editor/src/serializer/mod.rs"]
pub mod serializer;

/// DOCX package loader, mutable document aggregate, and exporter.
#[path = "../../jade-docx-editor/src/word_document.rs"]
pub mod word_document;

/// Logical selection primitives shared with the visual layout layer.
pub mod selection {
    pub use jade_selection::{SelectionRange, TextPosition};
}

// Page-setup validation uses the same pure geometry rules as rendering. Share
// their dependency-free source without linking the layout engine into the core.
#[path = "../../jade-docx-layout/src/section_transition.rs"]
pub mod section_transition;

// The document core stays layout-free: fontTable substitutes are parsed into
// `WordDocument::font_table_part` here, and the editor installs them into the
// font resolver at its own boundary.

// The shared document-model unit tests include a small number of layout
// assertions. Keep that dependency test-only; it is absent from normal
// headless builds and published runtime dependency graphs.
#[cfg(test)]
pub mod font_measurer {
    pub use jade_docx_layout_engine::font_measurer::*;
}

#[cfg(test)]
pub mod font_resolver {
    pub use jade_docx_layout_engine::font_resolver::*;
}

#[cfg(test)]
pub mod text_extract {
    pub use jade_docx_layout_engine::text_extract::*;
}

/// Options that affect parsing and editing without involving rendering.
#[derive(Clone, Debug)]
pub struct Options {
    /// Whether footnote parts should be parsed into the document model.
    pub render_footnotes: bool,
    /// Whether endnote parts should be parsed into the document model.
    pub render_endnotes: bool,
    /// Whether HTML `w:altChunk` parts should become sanitized document blocks.
    pub render_alt_chunks: bool,
    record_changes: bool,
    revision_author: Option<String>,
    revision_date: Option<String>,
    revision_date_utc: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            render_footnotes: true,
            render_endnotes: true,
            render_alt_chunks: true,
            record_changes: false,
            revision_author: None,
            revision_date: None,
            revision_date_utc: None,
        }
    }
}

impl Options {
    /// Enables or disables tracked-change recording for subsequent edits.
    pub fn set_record_changes(&mut self, value: bool) {
        self.record_changes = value;
    }

    /// Sets the author stored on newly recorded revisions.
    pub fn set_revision_author(&mut self, value: Option<String>) {
        self.revision_author = value;
    }

    /// Sets the revision date string stored on newly recorded revisions.
    pub fn set_revision_date(&mut self, value: Option<String>) {
        self.revision_date = value;
    }

    /// Sets the UTC revision date string stored on newly recorded revisions.
    pub fn set_revision_date_utc(&mut self, value: Option<String>) {
        self.revision_date_utc = value;
    }

    pub(crate) fn record_changes_enabled(&self) -> bool {
        self.record_changes
    }

    pub(crate) fn revision_author_internal(&self) -> Option<String> {
        self.revision_author.clone()
    }

    pub(crate) fn revision_date_internal(&self) -> Option<String> {
        self.revision_date.clone()
    }

    pub(crate) fn revision_date_utc_internal(&self) -> Option<String> {
        self.revision_date_utc.clone()
    }
}
