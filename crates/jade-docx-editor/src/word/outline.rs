//! アウトライン, as far as both shells share it: each body paragraph's
//! outline level (見出し 1–9, or 本文), how far the view sets each one in,
//! which paragraphs レベルの表示 hides, and the level レベル上げ gives 本文.

use jade_docx_reader::docx_xml::XmlNodeId;
use std::collections::{BTreeSet, HashMap};

use super::{references::navigation::WordNavigation, WordSession};
use crate::word_document::WordDocument;

/// View preferences shared by navigation and the outline ribbon. Paragraphs
/// use native surface indices; browser entry points translate body indices.
#[derive(Clone, Debug)]
pub struct OutlineDisplay {
    pub collapsed: BTreeSet<usize>,
    /// Native node identities at the last resolved surface positions. This is
    /// view state only; it must never add identifiers to the exported DOCX.
    #[doc(hidden)]
    pub collapsed_nodes: HashMap<usize, XmlNodeId>,
    pub shown: u8,
    pub active: bool,
}

impl Default for OutlineDisplay {
    fn default() -> Self {
        Self {
            collapsed: BTreeSet::new(),
            collapsed_nodes: HashMap::new(),
            shown: BODY_LEVEL,
            active: false,
        }
    }
}

impl OutlineDisplay {
    /// Merge both sources of hidden body elements. Removed headings cease to
    /// be collapsed, while leaving Outline keeps navigation's folds intact.
    pub fn hidden_blocks(&mut self, session: &(impl WordSession + ?Sized)) -> Vec<usize> {
        let headings: BTreeSet<_> = session
            .outline()
            .into_iter()
            .map(|heading| heading.paragraph)
            .collect();
        let document = session.word_document();
        // An explicit fold/unfold replaces the index set. Otherwise resolve
        // the same nodes after an insertion, move, or history restoration.
        if self.collapsed != self.collapsed_nodes.keys().copied().collect() {
            self.collapsed_nodes = self
                .collapsed
                .iter()
                .filter_map(|&paragraph| Some((paragraph, document.paragraph(paragraph)?.node_id?)))
                .collect();
        }
        let nodes: std::collections::HashSet<_> = self.collapsed_nodes.values().copied().collect();
        self.collapsed_nodes = headings
            .into_iter()
            .filter_map(|paragraph| {
                let node = document.paragraph(paragraph)?.node_id?;
                nodes.contains(&node).then_some((paragraph, node))
            })
            .collect();
        self.collapsed = self.collapsed_nodes.keys().copied().collect();
        let mut hidden = session.hidden_by_collapse(&self.collapsed);
        if self.active {
            hidden.extend(hidden_by_level(session.word_document(), self.shown));
        }
        hidden.sort_unstable();
        hidden.dedup();
        hidden
    }
}

/// How far each outline level is set in, in CSS pixels.
pub const LEVEL_STEP: f64 = 24.0;
/// The room left of the text for the symbols (⊕, ⊖, ○).
pub const SYMBOL_ROOM: f64 = 40.0;
/// What 本文 counts as when levels are compared.
pub const BODY_LEVEL: u8 = 10;

/// A body paragraph's outline level: 1–9 for a heading, [`BODY_LEVEL`] for
/// 本文; `None` for a paragraph outside the body (a cell's, a note's).
pub fn level_of(document: &WordDocument, paragraph: usize) -> Option<u8> {
    if !document.is_body_paragraph_surface(paragraph) {
        return None;
    }
    let paragraph = document.paragraph(paragraph)?;
    Some(WordDocument::paragraph_heading_level(paragraph).unwrap_or(BODY_LEVEL))
}

/// Every body paragraph with its level, in order.
pub fn levels(document: &WordDocument) -> Vec<(usize, u8)> {
    (0..document.paragraph_count())
        .filter_map(|index| Some((index, level_of(document, index)?)))
        .collect()
}

/// How far each paragraph is set in: a heading by its level, body text
/// one step past the heading it is under.
pub fn indents(document: &WordDocument) -> HashMap<usize, f64> {
    indents_of(&levels(document))
}

/// [`indents`] for paragraphs and levels already read.
pub fn indents_of(levels: &[(usize, u8)]) -> HashMap<usize, f64> {
    let mut parent = 0u8;
    levels
        .iter()
        .map(|&(paragraph, level)| {
            let steps = if level == BODY_LEVEL {
                parent
            } else {
                parent = level;
                level - 1
            };
            (paragraph, SYMBOL_ROOM + f64::from(steps) * LEVEL_STEP)
        })
        .collect()
}

/// The top-level body elements レベルの表示 hides when it shows levels
/// down to `shown` (1–9; 0 or [`BODY_LEVEL`] show everything).
pub fn hidden_by_level(document: &WordDocument, shown: u8) -> Vec<usize> {
    if shown == 0 || shown >= BODY_LEVEL {
        return Vec::new();
    }
    levels(document)
        .into_iter()
        .filter(|(_, level)| *level > shown)
        .filter_map(|(paragraph, _)| document.body_element_of(paragraph))
        .collect()
}

/// The level レベル上げ gives 本文 at `paragraph`: the level of the heading
/// it is under, 見出し 1 under none.
pub fn promoted_body_level(levels: &[(usize, u8)], paragraph: usize) -> u8 {
    levels
        .iter()
        .take_while(|(index, _)| *index < paragraph)
        .filter(|(_, level)| *level < BODY_LEVEL)
        .last()
        .map_or(1, |(_, level)| *level)
}

/// The symbol アウトライン shows before a paragraph: ⊕ before a heading with
/// something under it (or collapsed), ⊖ before one without, ○ before body
/// text. `next` is the level of the paragraph after it.
pub fn symbol(level: u8, next: Option<u8>, collapsed: bool) -> &'static str {
    if level == BODY_LEVEL {
        "○"
    } else if collapsed || next.is_some_and(|next| next > level) {
        "⊕"
    } else {
        "⊖"
    }
}

/// Symbols are a view snapshot, refreshed with outline indents after structural
/// edits. Ordinary character input must not rescan paragraph heading styles.
pub fn symbols(
    document: &WordDocument,
    collapsed: &BTreeSet<usize>,
) -> HashMap<usize, &'static str> {
    let levels = levels(document);
    levels
        .iter()
        .enumerate()
        .map(|(at, &(paragraph, level))| {
            (
                paragraph,
                symbol(
                    level,
                    levels.get(at + 1).map(|&(_, next)| next),
                    collapsed.contains(&paragraph),
                ),
            )
        })
        .collect()
}
