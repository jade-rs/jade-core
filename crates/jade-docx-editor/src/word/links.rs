//! Hyperlinks over any [`WordSession`], as Word has them: ⌘-click follows
//! one — a web or mail address to the system, a bookmark (`#name`) to its
//! place in the document — and ⌘K inserts one on the selection, or edits the
//! one the caret is in, from its display text and address.
//!
//! What the pages show as a link comes from the renderer's link regions
//! (`w:hyperlink`, and the `HYPERLINK` field's result), so a click is tested
//! against what was drawn. Writing one goes through the editor's
//! `InsertHyperlinkCommand`, which adds the relationship for an external
//! address (ECMA-376 Part 1 §17.16.22) and is one undo step.

use crate::canvas_renderer::HyperlinkRegion;
use crate::commands::hyperlink::{InsertHyperlinkCommand, RemoveHyperlinkCommand};
use jade_selection::{SelectionRange, TextPosition};

use super::{Helpers, Position, WordSession};
use crate::text_edit::TextEdit;

/// A hyperlink in the text: the run of a paragraph it covers and where it
/// goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub paragraph: usize,
    /// The byte range it covers in the paragraph's text.
    pub start: usize,
    pub end: usize,
    /// A web or mail address, or `#name` for a bookmark in the document.
    pub address: String,
    pub tooltip: Option<String>,
}

impl Hyperlink {
    fn from_region(region: &HyperlinkRegion) -> Option<Self> {
        let address = match (&region.target, &region.anchor) {
            (Some(target), _) if !target.is_empty() => target.clone(),
            (_, Some(anchor)) if !anchor.is_empty() => format!("#{anchor}"),
            _ => return None,
        };
        Some(Self {
            paragraph: region.paragraph,
            start: region.start,
            end: region.end,
            address,
            tooltip: region.tooltip.clone(),
        })
    }

    /// Whether it goes to a bookmark in the document rather than out of it.
    pub fn is_internal(&self) -> bool {
        self.address.starts_with('#')
    }
}

/// Where following a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// Out of the document: the shell hands it to the system (the browser
    /// opens it in a new tab).
    Address(String),
    /// A bookmark in the document: the caret goes there.
    Position(Position),
}

/// What the ハイパーリンクの挿入 dialog (⌘K) opens with: the text to show
/// and the address, and the text they apply to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkDraft {
    pub display: String,
    pub address: String,
    /// The text the link goes on — the selection, the caret, or the link
    /// being edited.
    pub range: (Position, Position),
    /// Whether the dialog edits a link already there.
    pub editing: bool,
}

/// The ハイパーリンクの挿入 dialog's two fields — 表示する文字列 and アドレス
/// — and the text they go on. Typing goes into the address first, as in
/// Word; Tab moves between the two.
pub struct LinkForm {
    pub draft: LinkDraft,
    pub display: TextEdit,
    pub address: TextEdit,
    /// Whether typing goes into the address rather than the display text.
    pub on_address: bool,
}

impl LinkForm {
    pub fn new(draft: LinkDraft) -> Self {
        Self {
            display: TextEdit::new(draft.display.clone()),
            address: TextEdit::new(draft.address.clone()),
            on_address: true,
            draft,
        }
    }

    /// The field typing goes into.
    pub fn field(&self) -> &TextEdit {
        if self.on_address {
            &self.address
        } else {
            &self.display
        }
    }

    pub fn field_mut(&mut self) -> &mut TextEdit {
        if self.on_address {
            &mut self.address
        } else {
            &mut self.display
        }
    }

    /// Tab: the other field.
    pub fn switch_field(&mut self) {
        self.on_address = !self.on_address;
    }

    /// OK: the link on its text, one undo step. Returns where the caret
    /// goes.
    pub fn submit<S: WordSession + ?Sized>(&self, session: &mut S) -> Result<Position, String> {
        session.apply_link(&self.draft, &self.display.text, &self.address.text)
    }
}

/// An address as typed into the dialog, made one a link can hold: Word
/// adds `http://` to `www.example.com` and `mailto:` to a mail address.
pub fn normalize_address(address: &str) -> String {
    let address = address.trim();
    let lower = address.to_ascii_lowercase();
    if address.is_empty()
        || address.starts_with('#')
        || lower.contains("://")
        || lower.starts_with("mailto:")
    {
        return address.to_string();
    }
    if address.contains('@') && !address.contains('/') {
        return format!("mailto:{address}");
    }
    format!("http://{address}")
}

/// Hyperlinks over every session.
pub trait WordLinks: WordSession {
    /// The link drawn under a point of the pages, in the document's
    /// coordinates (the pages stacked with their gaps), as a click lands.
    fn hyperlink_at_point(&self, x: f64, document_y: f64) -> Option<Hyperlink> {
        let regions = self.hyperlink_regions();
        let region = regions.iter().find(|region| {
            x >= region.x
                && x <= region.x + region.width
                && document_y >= region.y
                && document_y <= region.y + region.height
        })?;
        let hit = Hyperlink::from_region(region)?;
        Some(whole_link(&regions, hit))
    }

    /// The link at a text position: the one whose text the position is in,
    /// or at the end of.
    fn hyperlink_at(&self, (paragraph, offset): Position) -> Option<Hyperlink> {
        let regions = self.hyperlink_regions();
        let links: Vec<Hyperlink> = regions
            .iter()
            .filter(|region| region.paragraph == paragraph)
            .filter_map(Hyperlink::from_region)
            .map(|link| whole_link(&regions, link))
            .collect();
        links
            .iter()
            .find(|link| (link.start..link.end).contains(&offset))
            .or_else(|| {
                links
                    .iter()
                    .find(|link| link.start < offset && offset == link.end)
            })
            .cloned()
    }

    /// Where following `link` goes; `None` for a bookmark the document does
    /// not have.
    fn link_target(&self, link: &Hyperlink) -> Option<LinkTarget> {
        match link.address.strip_prefix('#') {
            Some(name) => self
                .word_document()
                .bookmark_position(name)
                .map(|position| LinkTarget::Position((position.paragraph, position.offset))),
            None => Some(LinkTarget::Address(link.address.clone())),
        }
    }

    /// What ⌘K opens with: the link the caret or the selection is in, to
    /// edit; otherwise the selection's text (one paragraph of it) to make a
    /// link of, or nothing typed yet at a bare caret.
    fn link_draft(&self) -> LinkDraft {
        let range = self
            .selection_range()
            .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::new(0, 0)));
        let (start, end) = range.normalized();
        let (start, end) = ((start.paragraph, start.offset), (end.paragraph, end.offset));
        let inside = self
            .hyperlink_at(start)
            .filter(|link| link.paragraph == end.0 && end.1 <= link.end && start.1 >= link.start);
        if let Some(link) = inside {
            let display = self
                .text_of(link.paragraph)
                .and_then(|text| text.get(link.start..link.end).map(str::to_string))
                .unwrap_or_default();
            return LinkDraft {
                display,
                address: link.address,
                range: ((link.paragraph, link.start), (link.paragraph, link.end)),
                editing: true,
            };
        }
        let display = if start.0 == end.0 {
            self.text_of(start.0)
                .and_then(|text| text.get(start.1..end.1).map(str::to_string))
                .unwrap_or_default()
        } else {
            String::new()
        };
        LinkDraft {
            display,
            address: String::new(),
            range: (start, end),
            editing: false,
        }
    }

    /// Puts the link the dialog describes on its text: `display` replaces
    /// the text when it was changed (or there was none), `address` is where
    /// it goes. An empty address takes an edited link off. Returns where the
    /// caret goes: after the link.
    fn apply_link(
        &mut self,
        draft: &LinkDraft,
        display: &str,
        address: &str,
    ) -> Result<Position, String> {
        let (from, to) = draft.range;
        let range = SelectionRange::new(
            TextPosition::new(from.0, from.1),
            TextPosition::new(to.0, to.1),
        );
        let address = normalize_address(address);
        if address.is_empty() {
            if !draft.editing {
                return Err("type an address for the link".to_string());
            }
            self.execute_command(Box::new(RemoveHyperlinkCommand::new(range)), false)?;
            self.collapse_to(to);
            return Ok(to);
        }
        let current = (from.0 == to.0)
            .then(|| self.text_of(from.0))
            .flatten()
            .and_then(|text| text.get(from.1..to.1).map(str::to_string));
        let display = display.trim();
        // The text stays as it is — formatting and all — unless the dialog
        // changed it; at a bare caret the address is the text, as in Word.
        let replacement = if from == to {
            Some(if display.is_empty() {
                address.clone()
            } else {
                display.to_string()
            })
        } else if !display.is_empty() && current.as_deref() != Some(display) {
            Some(display.to_string())
        } else {
            None
        };
        let caret = match &replacement {
            Some(text) => (from.0, from.1 + text.len()),
            None => to,
        };
        self.execute_command(
            Box::new(InsertHyperlinkCommand::new(
                range,
                replacement,
                address,
                None,
            )),
            false,
        )?;
        self.collapse_to(caret);
        Ok(caret)
    }
}

impl<T: WordSession + ?Sized> WordLinks for T {}

/// A link drawn over several lines is a region a line: the whole of it is
/// the run of regions in its paragraph that go to the same place and touch.
fn whole_link(regions: &[HyperlinkRegion], mut link: Hyperlink) -> Hyperlink {
    let mut pieces: Vec<(usize, usize)> = regions
        .iter()
        .filter(|region| region.paragraph == link.paragraph)
        .filter_map(|region| {
            Hyperlink::from_region(region)
                .filter(|other| other.address == link.address)
                .map(|other| (other.start, other.end))
        })
        .collect();
    pieces.sort_unstable();
    let mut grown = true;
    while grown {
        grown = false;
        for &(start, end) in &pieces {
            if start <= link.end && end >= link.start && (start < link.start || end > link.end) {
                link.start = link.start.min(start);
                link.end = link.end.max(end);
                grown = true;
            }
        }
    }
    link
}
