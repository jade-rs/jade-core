//! スライド ショーの設定, 目的別スライド ショー, リハーサルのタイミング and the
//! slide view's grid and guides, as package edits and as undoable steps of an
//! [`EditSession`].

use crate::{xml, EditSession, Result};
use jade_pptx_reader::{CustomShow, ShowSettings, ViewProps};

pub use xml::presenting::print_what;

/// Rewrites `<p:showPr>` (making `presProps.xml` when the deck has none).
pub fn set_show_settings(bytes: &[u8], settings: &ShowSettings) -> Result<Vec<u8>> {
    xml::presenting::set_show_settings(bytes, settings)
}

/// Rewrites the deck's custom shows; slide indices are 0-based.
pub fn set_custom_shows(bytes: &[u8], shows: &[CustomShow]) -> Result<Vec<u8>> {
    xml::presenting::set_custom_shows(bytes, shows)
}

/// Sets or clears each listed slide's automatic advance, in milliseconds.
pub fn set_slide_advances(bytes: &[u8], advances: &[(usize, Option<u32>)]) -> Result<Vec<u8>> {
    xml::presenting::set_slide_advances(bytes, advances)
}

/// Rewrites the grid, guides and last view in `viewProps.xml`.
pub fn set_view_props(bytes: &[u8], props: &ViewProps) -> Result<Vec<u8>> {
    xml::presenting::set_view_props(bytes, props)
}

/// `<p:prnPr prnWhat>`: `slides`, `handouts1`…`handouts9`, `notes` or
/// `outline`.
pub fn set_print_what(bytes: &[u8], what: &str) -> Result<Vec<u8>> {
    xml::presenting::set_print_what(bytes, what)
}

impl EditSession {
    pub fn set_show_settings(&mut self, settings: &ShowSettings) -> Result<bool> {
        let next = set_show_settings(self.bytes(), settings)?;
        Ok(self.commit(next))
    }

    pub fn set_custom_shows(&mut self, shows: &[CustomShow]) -> Result<bool> {
        let next = set_custom_shows(self.bytes(), shows)?;
        Ok(self.commit(next))
    }

    /// Rehearsed timings land as one step of Undo.
    pub fn set_slide_advances(&mut self, advances: &[(usize, Option<u32>)]) -> Result<bool> {
        let next = set_slide_advances(self.bytes(), advances)?;
        Ok(self.commit(next))
    }

    pub fn set_view_props(&mut self, props: &ViewProps) -> Result<bool> {
        let next = set_view_props(self.bytes(), props)?;
        Ok(self.commit(next))
    }

    pub fn set_print_what(&mut self, what: &str) -> Result<bool> {
        let next = set_print_what(self.bytes(), what)?;
        Ok(self.commit(next))
    }
}

pub use xml::page_masters::{page_header_footer, PageHeaderFooter};

/// Makes the notes master when the deck has none.
pub fn ensure_notes_master(bytes: &[u8]) -> Result<Vec<u8>> {
    xml::page_masters::ensure_notes_master(bytes)
}

/// Makes the handout master when the deck has none.
pub fn ensure_handout_master(bytes: &[u8]) -> Result<Vec<u8>> {
    xml::page_masters::ensure_handout_master(bytes)
}

/// ヘッダーとフッター › ノートと配布資料, applied to both masters.
pub fn set_page_header_footer(bytes: &[u8], settings: &PageHeaderFooter) -> Result<Vec<u8>> {
    xml::page_masters::set_page_header_footer(bytes, settings)
}

/// ノートの向き / 配布資料の向き.
pub fn set_notes_orientation(bytes: &[u8], portrait: bool) -> Result<Vec<u8>> {
    xml::page_masters::set_notes_orientation(bytes, portrait)
}

impl EditSession {
    pub fn ensure_notes_master(&mut self) -> Result<bool> {
        let next = ensure_notes_master(self.bytes())?;
        Ok(self.commit(next))
    }

    pub fn ensure_handout_master(&mut self) -> Result<bool> {
        let next = ensure_handout_master(self.bytes())?;
        Ok(self.commit(next))
    }

    pub fn set_page_header_footer(&mut self, settings: &PageHeaderFooter) -> Result<bool> {
        let next = set_page_header_footer(self.bytes(), settings)?;
        Ok(self.commit(next))
    }

    pub fn set_notes_orientation(&mut self, portrait: bool) -> Result<bool> {
        let next = set_notes_orientation(self.bytes(), portrait)?;
        Ok(self.commit(next))
    }
}

/// ノート の書式: formats characters `start..end` (UTF-16, as the notes are
/// typed) of slide `slide`'s notes.
pub fn format_notes_range(
    bytes: &[u8],
    slide: usize,
    start: usize,
    end: usize,
    format: &crate::TextFormat,
) -> Result<Vec<u8>> {
    format.validate()?;
    xml::edit_notes_body(bytes, slide, |notes, shape_id| {
        xml::format_text_range(
            notes,
            &crate::TextFormatEdit {
                shape_id,
                start,
                end,
                format: format.clone(),
            },
        )
    })
}

/// The paragraphs of slide `slide`'s notes touched by `start..end`: their
/// alignment, bullets and levels.
pub fn format_notes_paragraphs(
    bytes: &[u8],
    slide: usize,
    start: usize,
    end: usize,
    format: &crate::ParagraphFormat,
) -> Result<Vec<u8>> {
    format.validate()?;
    xml::edit_notes_body(bytes, slide, |notes, shape_id| {
        xml::format_paragraphs(
            notes,
            &crate::ParagraphFormatEdit {
                shape_id,
                start,
                end,
                format: format.clone(),
            },
        )
    })
}

impl EditSession {
    pub fn format_notes_range(
        &mut self,
        slide: usize,
        start: usize,
        end: usize,
        format: &crate::TextFormat,
    ) -> Result<bool> {
        let next = format_notes_range(self.bytes(), slide, start, end, format)?;
        Ok(self.commit(next))
    }

    pub fn format_notes_paragraphs(
        &mut self,
        slide: usize,
        start: usize,
        end: usize,
        format: &crate::ParagraphFormat,
    ) -> Result<bool> {
        let next = format_notes_paragraphs(self.bytes(), slide, start, end, format)?;
        Ok(self.commit(next))
    }
}

/// New Slide at `index` (on the layout of the slide before it), as a
/// package edit, for batches.
pub fn insert_slide(bytes: &[u8], index: usize) -> Result<Vec<u8>> {
    xml::edit_slides(bytes, xml::SlideEdit::Insert(index))
}

impl EditSession {
    /// Several package edits — free functions of this crate chained by the
    /// caller — as one step of Undo. The result must still read as a deck.
    pub fn apply_batch(&mut self, edit: impl FnOnce(&[u8]) -> Result<Vec<u8>>) -> Result<bool> {
        let next = edit(self.bytes())?;
        jade_pptx_reader::read_deck(&next)?;
        Ok(self.commit(next))
    }
}

pub use xml::ink::InkPath;

/// Writes a show's kept ink onto slide `slide` as freeform lines.
pub fn insert_ink(bytes: &[u8], slide: usize, paths: &[InkPath]) -> Result<Vec<u8>> {
    if paths.is_empty() {
        return Ok(bytes.to_vec());
    }
    let (mut package, part, xml) = crate::slide_xml(bytes, slide)?;
    let changed = xml::ink::insert_ink(&xml, paths)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}
