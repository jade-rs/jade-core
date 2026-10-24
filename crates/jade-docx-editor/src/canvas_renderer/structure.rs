//! The structure a tagged PDF is given (ISO 32000 §14.7): what each piece
//! of a page is, in reading order — a paragraph of the body, a table's cell,
//! a figure — recorded as the renderer draws it. Each one gets a node id the
//! painter is told before its drawing ([`Painter::set_structure_node`]),
//! and the entries say what each node is and what it is inside, for the PDF
//! writer to build the structure tree from.
//!
//! Headers, footers and notes are drawn with no node: they are the page's
//! furniture (artifacts), not its content.

use super::*;

/// What a node of the structure is.
#[derive(Clone, Debug, PartialEq)]
pub enum StructureKind {
    /// A paragraph of the body: its index among the paragraphs, and among
    /// the body's elements (what the document's outline counts by).
    Paragraph {
        paragraph_index: usize,
        body_element_index: usize,
    },
    /// A table cell: the table (numbered as tables are drawn), its row and
    /// its grid column.
    Cell {
        table: usize,
        row: usize,
        column: usize,
    },
    /// A picture, with its alternative text.
    Figure { alt: Option<String> },
}

/// A node of the structure: its id, the node it is inside (`0` for the
/// document), and what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct StructureEntry {
    pub id: i32,
    pub parent: i32,
    pub kind: StructureKind,
}

/// The structure being recorded.
#[derive(Default, Debug)]
pub(crate) struct StructureState {
    pub(crate) enabled: bool,
    entries: Vec<StructureEntry>,
    /// The nodes open, innermost last.
    open: Vec<i32>,
    /// While a header, footer or note is drawn: nothing is tagged.
    suppressed: usize,
    /// The tables being drawn, innermost last, and how many have begun.
    tables: Vec<usize>,
    tables_begun: usize,
    /// The row of the cell about to be drawn.
    pub(crate) row: usize,
}

impl<P: Painter> CanvasRenderer<P>
where
    P::Error: PainterError,
{
    /// Records the structure of the next render, for a tagged PDF.
    pub fn set_structure_tagging(&mut self, enabled: bool) {
        let mut state = self.structure.borrow_mut();
        *state = StructureState {
            enabled,
            ..StructureState::default()
        };
    }

    /// The structure the last render recorded, in reading order.
    pub fn take_structure_entries(&mut self) -> Vec<StructureEntry> {
        std::mem::take(&mut self.structure.borrow_mut().entries)
    }

    /// Opens a node of `kind` inside the one open, and tells the painter.
    /// `None` when nothing is being tagged.
    pub(super) fn begin_structure(&self, kind: StructureKind) -> Option<i32> {
        let id = {
            let mut state = self.structure.borrow_mut();
            if !state.enabled || state.suppressed > 0 {
                return None;
            }
            let id = state.entries.len() as i32 + 1;
            let parent = state.open.last().copied().unwrap_or(0);
            state.entries.push(StructureEntry { id, parent, kind });
            state.open.push(id);
            id
        };
        self.host.painter().set_structure_node(id);
        Some(id)
    }

    /// Closes the node `begin_structure` opened.
    pub(super) fn end_structure(&self, opened: Option<i32>) {
        if opened.is_none() {
            return;
        }
        let node = {
            let mut state = self.structure.borrow_mut();
            state.open.pop();
            state.open.last().copied().unwrap_or(0)
        };
        self.host.painter().set_structure_node(node);
    }

    /// While a header, footer or note is drawn: no node, and none opened.
    pub(super) fn suppress_structure(&self, suppress: bool) {
        let node = {
            let mut state = self.structure.borrow_mut();
            if !state.enabled {
                return;
            }
            if suppress {
                state.suppressed += 1;
                0
            } else {
                state.suppressed = state.suppressed.saturating_sub(1);
                if state.suppressed > 0 {
                    0
                } else {
                    state.open.last().copied().unwrap_or(0)
                }
            }
        };
        self.host.painter().set_structure_node(node);
    }

    /// After the painter has moved to another page: the node open goes on
    /// there.
    pub(super) fn resume_structure_on_new_page(&self) {
        let node = {
            let state = self.structure.borrow();
            if !state.enabled {
                return;
            }
            if state.suppressed > 0 {
                0
            } else {
                state.open.last().copied().unwrap_or(0)
            }
        };
        self.host.painter().set_structure_node(node);
    }

    pub(super) fn begin_structure_table(&self) {
        let mut state = self.structure.borrow_mut();
        if state.enabled {
            let table = state.tables_begun;
            state.tables_begun += 1;
            state.tables.push(table);
        }
    }

    pub(super) fn end_structure_table(&self) {
        let mut state = self.structure.borrow_mut();
        if state.enabled {
            state.tables.pop();
        }
    }

    /// Opens the node of a cell of the table being drawn, in the row last
    /// set ([`StructureState::row`]).
    pub(super) fn begin_structure_cell(&self, column: usize) -> Option<i32> {
        let (table, row) = {
            let state = self.structure.borrow();
            (state.tables.last().copied()?, state.row)
        };
        self.begin_structure(StructureKind::Cell { table, row, column })
    }

    pub(super) fn set_structure_row(&self, row: usize) {
        self.structure.borrow_mut().row = row;
    }
}
