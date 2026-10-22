//! What a Word ribbon does to a document — character and paragraph
//! formatting, paragraph styles, lists and the table commands — ported from
//! jade-ide's ribbon (`JadeEditorStyleMenu`, `JadeAdvancedRibbon`), whose
//! runtime sends the same headless commands.
//!
//! Both shells run it: the native app over `jade-docx-native-render`'s
//! `NativeDocument`, the browser build over `jade-web`'s tiny-skia
//! `Document`. Each is a [`WordSession`] — a document, the last paint's table
//! cells, the selection, and a way to apply a command as one undo step — and
//! every session gets [`WordFormatting`].
//!
//! Every command repaints the whole document: a bold run or a heading moves
//! the lines after it, and a table edit moves everything below the table. It
//! is a click, not a keystroke, so the cost of a whole pass is affordable.
//!
//! Table commands address a table by its order in the body and a cell by its
//! row and column, none of which the caret knows; the cell registry of the
//! last paint bridges the two, as the browser runtime does.

use crate::commands::Command;
use crate::headless_api::execute_headless_queries_on_document;
use crate::word_document::WordDocument;
use jade_docx_headless_api::{
    HeadlessBorderEdge, HeadlessCommand, HeadlessFontSizeDirection, HeadlessFontSizeStep,
    HeadlessIndentDirection, HeadlessIndentTarget, HeadlessMultilevelListPreset, HeadlessQuery,
    HeadlessQueryRequest, HeadlessQueryResult, HeadlessRunStyle, HeadlessStyleKindFilter,
    HeadlessTableCellSelection,
};
pub use jade_docx_layout::selection::CellLayout;
use jade_selection::{SelectionRange, TextPosition};

pub mod accessibility;
pub mod asian;
pub mod autocorrect;
pub mod borders;
pub mod building_blocks;
pub mod clipboard;
pub mod compare;
pub mod compat;
pub mod compress;
pub mod connectors;
pub mod context_menu;
pub mod controls;
pub mod count;
pub mod customize;
pub mod design;
pub mod diagrams;
pub mod embedding;
pub mod equation;
pub mod fields;
pub mod find;
pub mod font;
pub mod form_view;
pub mod formats;
pub mod group;
pub mod handles;
pub mod html_clip;
pub mod hyphenation;
pub mod immersive;
pub mod ink_tools;
pub mod insert;
pub mod inspect;
pub mod language;
pub mod links;
pub mod lists;
pub mod mailings;
pub mod markup;
pub mod master;
pub mod new;
pub mod objects;
pub mod outline;
pub mod package;
pub mod page;
pub mod page_setup;
pub mod painter;
pub mod paragraph;
pub mod paragraph_options;
pub mod picture;
pub mod picture_format;
pub mod platform;
pub mod printing;
pub mod proofing;
pub mod protection;
pub mod read_aloud;
pub mod recovery;
pub mod references;
pub mod repair;
pub mod review;
pub mod reviewer;
pub mod ruler;
pub mod screen_reader;
pub mod search;
pub mod selection;
pub mod shape_gallery;
pub mod shapes;
pub mod signatures;
pub mod styles;
pub mod table;
pub mod table_ui;
pub mod templates;
pub mod thesaurus;
pub mod typography;
pub mod view;
pub mod xml;

pub use count::{WordCount, WordCounting};
pub use find::{FindOptions, Found, WordFind};
/// The engine's command and query types, for shells that build commands.
pub use jade_docx_headless_api as headless;
/// The selection type [`WordSession`] speaks, for shells that wrap one.
pub use jade_selection::SelectionRange as SelectionRangeOf;
pub use links::{Hyperlink, LinkDraft, LinkForm, LinkTarget, WordLinks};
pub use painter::{CopiedFormat, WordFormatPainter};
pub use paragraph::{
    IndentUnit, LineSpacing, ParagraphField, ParagraphForm, ParagraphFormat, SpecialIndent,
    WordParagraphs,
};
pub use references::{NoteForm, NoteKind, WordReferences};

/// A paragraph and a byte offset into its text.
pub type Position = (usize, usize);

/// An open Word document a ribbon can drive.
pub trait WordSession {
    /// The document model.
    fn word_document(&self) -> &WordDocument;
    /// The selection, a caret when collapsed.
    fn selection_range(&self) -> Option<SelectionRange>;
    /// Sets the selection without repainting.
    fn set_selection_range(&mut self, range: Option<SelectionRange>);
    /// Applies `command` as one undo step and repaints the whole document.
    /// With `keep_selection` the selection survives the command; otherwise
    /// it is wherever the command left it.
    fn apply_command(
        &mut self,
        command: HeadlessCommand,
        keep_selection: bool,
    ) -> Result<(), String>;
    /// Applies several commands as one action — one undo step — and repaints
    /// the whole document. When one fails, the ones before it are taken back.
    fn apply_commands(
        &mut self,
        commands: Vec<HeadlessCommand>,
        keep_selection: bool,
    ) -> Result<(), String>;
    /// Executes a command the shell built itself — a picture with its bytes,
    /// a header edit — as one undo step, and repaints the whole document.
    fn execute_command(
        &mut self,
        command: Box<dyn Command>,
        keep_selection: bool,
    ) -> Result<(), String>;
    /// The table cell a paragraph was laid out in at the last paint, by its
    /// id in the renderer's cell registry.
    fn cell_id_for_paragraph(&self, paragraph: usize) -> Option<usize>;
    fn cell_layout(&self, cell_id: usize) -> Option<&CellLayout>;
    /// Every cell of the last paint, in document order.
    fn cell_layouts(&self) -> &[CellLayout];
    /// A laid-out table's order in the body, which table commands take.
    fn table_order_for(&self, table_id: usize) -> Option<usize>;
    /// Where the pages show hyperlinks, from the document as it now is and
    /// the last layout: a rectangle a line of each, in the document's
    /// coordinates.
    fn hyperlink_regions(&self) -> Vec<crate::canvas_renderer::HyperlinkRegion>;

    /// Selects whole cells — the rectangle from one cell to another, by
    /// their ids in the last paint's cell registry — and repaints so the
    /// cells show selected, as Word's row, column and table selections do.
    fn select_cells(&mut self, anchor_cell: usize, focus_cell: usize) -> Result<(), String>;

    /// Pages in the laid-out document, when the session lays out pages
    /// (NUMPAGES, SECTIONPAGES).
    fn page_count(&self) -> Option<usize> {
        None
    }

    /// The page (1-based) a paragraph starts on at the last layout, when it
    /// has been laid out: what an index, a table of contents' page numbers,
    /// the navigation pane and the PAGE and PAGEREF fields show.
    fn paragraph_page(&self, _paragraph: usize) -> Option<usize> {
        None
    }

    /// Where a caret at a position would be drawn at the last layout —
    /// its x, its top in document coordinates and its height — without
    /// moving the selection.
    fn position_metrics(&self, _at: Position) -> Option<(f64, f64, f64)> {
        None
    }

    /// Where the last layout drew the text from `start` to `end` (rendered
    /// positions): a rectangle a line, in the document's coordinates.
    fn text_regions(
        &self,
        _start: Position,
        _end: Position,
    ) -> Vec<crate::canvas_renderer::TextRegion> {
        Vec::new()
    }

    /// Where the last paint drew every equation: what the equation editor
    /// places its caret and answers clicks with.
    fn math_zones(&self) -> Vec<crate::canvas_renderer::MathZone> {
        Vec::new()
    }

    /// Where the last layout put the floating object in run `run` of
    /// paragraph `paragraph`: its page (0-based) and its box there, in px.
    fn object_region(&self, _paragraph: usize, _run: usize) -> Option<(usize, f64, f64, f64, f64)> {
        None
    }
}

pub(crate) fn query(document: &WordDocument, query: HeadlessQuery) -> Option<HeadlessQueryResult> {
    execute_headless_queries_on_document(
        document,
        &HeadlessQueryRequest {
            queries: vec![query],
        },
    )
    .ok()?
    .results
    .into_iter()
    .next()
}

/// A character format the ribbon toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterFormat {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Superscript,
    Subscript,
}

/// What the ribbon shows as current: the run at the caret (or at the start of
/// the selection) and its paragraph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormatState {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    /// The paragraph's `w:pStyle`, or `None` for the default style.
    pub style_id: Option<String>,
    /// `left`, `center`, `right` or `both`; `None` is the style's own.
    pub alignment: Option<String>,
    /// The list the paragraph is in, by its first level's format.
    pub list: Option<ListKind>,
    /// The cell the caret is in, when it is in a table.
    pub table: Option<TableCell>,
    /// Everything else about the run: what the フォント dialog and the font
    /// group's third row show (`font::WordCharacters`).
    pub character: jade_docx_headless_api::HeadlessRunFormat,
}

/// What kind of list a paragraph is in, which decides what the ribbon's two
/// list buttons do: the one that matches turns the list off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Decimal,
    Other,
}

/// A paragraph style as the gallery lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParagraphStyle {
    pub id: String,
    /// `w:name`, which Word matches built-in styles by; the id is whatever the
    /// authoring Word's language made it (`a` and `1` in a Japanese document).
    pub name: String,
    pub is_default: bool,
}

/// Where the caret is in a table, in the table commands' terms, and what the
/// table tools can do there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableCell {
    pub table_order_index: usize,
    pub row: usize,
    pub column: usize,
    pub rows: usize,
    pub columns: usize,
    /// Whether the selection spans two or more cells of this table.
    pub can_merge: bool,
    /// Whether the caret's cell is merged, across or down.
    pub can_unmerge: bool,
    /// Whether the caret's row repeats at the top of every page.
    pub repeat_header: bool,
}

/// The table commands the ribbon's 表 tabs offer.
#[derive(Clone, Debug, PartialEq)]
pub enum TableCommand {
    InsertRowAbove,
    InsertRowBelow,
    InsertColumnLeft,
    InsertColumnRight,
    DeleteRow,
    DeleteColumn,
    DeleteTable,
    /// Merges the cells the selection spans.
    MergeCells,
    /// Gives the caret's merged cell back its grid cells (結合したセルを分割).
    UnmergeCells,
    /// Splits the caret's cell into `columns` × `rows` cells (セルの分割).
    SplitCell {
        columns: usize,
        rows: usize,
    },
    /// Splits the table above the caret's row (表の分割).
    SplitTable,
    DistributeRows,
    DistributeColumns,
    /// How the table sits between the margins: `left`, `center`, `right`.
    Align(String),
    /// Where the caret cell's content sits: `top`, `center`, `bottom`.
    CellVerticalAlign(String),
    /// Fills the caret's cell, or clears it with `None`.
    CellShading(Option<String>),
    /// One of the engine's direct-formatting presets: `plain`, `grid`,
    /// `lightBlue`, `blueBands`, `grayBands`.
    StylePreset(String),
    Borders(TableBorders),
    /// Whether the caret's row repeats at the top of every page (タイトル行の
    /// 繰り返し, `w:tblHeader`).
    RepeatHeader(bool),
}

impl TableCommand {
    /// Whether the command adds or removes cells, which renumbers the
    /// paragraphs after it and moves the caret, rather than formatting what
    /// is there.
    pub fn is_structural(&self) -> bool {
        matches!(
            self,
            TableCommand::InsertRowAbove
                | TableCommand::InsertRowBelow
                | TableCommand::InsertColumnLeft
                | TableCommand::InsertColumnRight
                | TableCommand::DeleteRow
                | TableCommand::DeleteColumn
                | TableCommand::DeleteTable
                | TableCommand::MergeCells
                | TableCommand::UnmergeCells
                | TableCommand::SplitCell { .. }
                | TableCommand::SplitTable
        )
    }
}

/// The cell-format popover's two border sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableBorders {
    All,
    None,
}

/// The paragraph borders of the 段落 dialog: Word's sides, as jade-ide names
/// them (`b`, `t`, `tblr`, and none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParagraphBorders {
    Bottom,
    Top,
    Box,
    None,
}

fn line(color: &str, width: f32) -> HeadlessBorderEdge {
    HeadlessBorderEdge {
        style: Some("single".to_string()),
        width: Some(width),
        color: Some(color.to_string()),
    }
}

fn no_line() -> HeadlessBorderEdge {
    HeadlessBorderEdge {
        style: Some("none".to_string()),
        width: None,
        color: None,
    }
}

/// `#RRGGBB` from `RRGGBB` or `#RRGGBB`: what reaches the canvas is a CSS
/// colour, and one given the bare form keeps the colour it already had.
fn css_color(color: &str) -> String {
    let hex = color.trim().trim_start_matches('#');
    format!("#{hex}")
}

/// What the public commands are built from.
trait Helpers: WordSession {
    fn query(&self, request: HeadlessQuery) -> Option<HeadlessQueryResult> {
        query(self.word_document(), request)
    }

    fn text_of(&self, paragraph: usize) -> Option<String> {
        match self.query(HeadlessQuery::ParagraphSlice {
            start_paragraph: paragraph,
            max_paragraphs: 1,
        })? {
            HeadlessQueryResult::ParagraphSlice { paragraphs, .. } => paragraphs
                .into_iter()
                .find(|entry| entry.paragraph == paragraph)
                .map(|entry| entry.text),
            _ => None,
        }
    }

    fn paragraph_total(&self) -> usize {
        match self.query(HeadlessQuery::ParagraphCount) {
            Some(HeadlessQueryResult::ParagraphCount { paragraph_count }) => paragraph_count,
            _ => 0,
        }
    }

    /// Makes the selection a caret at `position`, without repainting.
    fn collapse_to(&mut self, position: Position) {
        let range = SelectionRange::collapsed(TextPosition::new(position.0, position.1));
        self.set_selection_range(Some(range));
    }

    // ---- reading ------------------------------------------------------------

    /// The kind of list paragraph `index` is in, read from its numbering's
    /// first level, as jade-ide's list toggles read it.
    fn list_kind(&self, index: usize) -> Option<ListKind> {
        let num_id = self
            .word_document()
            .paragraph(index)?
            .properties
            .as_ref()?
            .numbering
            .as_ref()?
            .id
            .clone()?;
        let format = self
            .word_document()
            .numbering_part
            .as_ref()
            .and_then(|numbering| numbering.get_level_info(&num_id, 0))
            .and_then(|level| level.num_fmt.clone());
        Some(match format.as_deref() {
            Some("bullet") => ListKind::Bullet,
            Some("decimal") => ListKind::Decimal,
            _ => ListKind::Other,
        })
    }

    /// Where the ribbon reads the formatting: the selection's start, or the
    /// character before a caret — what typing there would continue.
    fn format_anchor(&self) -> Position {
        match self.selection_range() {
            Some(range) if !range.is_collapsed() => {
                let (start, _) = range.normalized();
                (start.paragraph, start.offset)
            }
            Some(range) => (range.focus.paragraph, range.focus.offset.saturating_sub(1)),
            None => (0, 0),
        }
    }

    fn table_cell_of(&self, paragraph: usize) -> Option<TableCell> {
        let cell_id = self.cell_id_for_paragraph(paragraph)?;
        let layout = self.cell_layout(cell_id)?;
        let table_order_index = self.table_order_for(layout.table_id)?;
        let (rows, columns) = self.word_document().table_dimensions(table_order_index)?;
        let snapshot = match self.query(HeadlessQuery::Table { table_order_index }) {
            Some(HeadlessQueryResult::Table { value, .. }) => value,
            _ => None,
        };
        let (can_unmerge, repeat_header) = snapshot
            .map(|table| {
                let merged = table
                    .rows
                    .get(layout.row)
                    .and_then(|row| row.cells.iter().find(|cell| cell.column == layout.column))
                    .is_some_and(|cell| {
                        cell.span.is_some_and(|span| span > 1)
                            || cell.vertical_merge.as_deref() == Some("restart")
                    });
                let header = table.rows.get(layout.row).is_some_and(|row| row.is_header);
                (merged, header)
            })
            .unwrap_or((false, false));
        Some(TableCell {
            table_order_index,
            row: layout.row,
            column: layout.column,
            rows,
            columns,
            can_merge: false,
            can_unmerge,
            repeat_header,
        })
    }

    // ---- what a command acts on ---------------------------------------------

    /// The paragraphs the selection touches, or the caret's twice. Paragraph
    /// formatting applies to whole paragraphs, as in Word.
    fn paragraph_span(&self) -> (usize, usize) {
        match self.selection_range() {
            Some(range) if !range.is_collapsed() => {
                let (start, end) = range.normalized();
                (start.paragraph, end.paragraph)
            }
            Some(range) => (range.focus.paragraph, range.focus.paragraph),
            None => (0, 0),
        }
    }

    /// The range a run command formats: the selection, or — as Word does
    /// with a bare caret — the word around the caret. `None` between words,
    /// where Word would only change what is typed next.
    fn run_span(&self) -> Option<(Position, Position)> {
        let range = self.selection_range()?;
        if !range.is_collapsed() {
            let (start, end) = range.normalized();
            return Some(((start.paragraph, start.offset), (end.paragraph, end.offset)));
        }
        let (paragraph, offset) = (range.focus.paragraph, range.focus.offset);
        let text = self.text_of(paragraph)?;
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let start = text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, ch)| ch.is_alphanumeric())
            .last()
            .map(|(index, _)| index)
            .unwrap_or(offset);
        let end = text[offset..]
            .char_indices()
            .take_while(|(_, ch)| ch.is_alphanumeric())
            .last()
            .map(|(index, ch)| offset + index + ch.len_utf8())
            .unwrap_or(offset);
        (start < end).then_some(((paragraph, start), (paragraph, end)))
    }

    fn run_on_span(
        &mut self,
        command: impl FnOnce(Position, Position) -> HeadlessCommand,
    ) -> Result<(), String> {
        let (start, end) = self
            .run_span()
            .ok_or_else(|| "no text to format".to_string())?;
        self.apply_command(command(start, end), true)
    }

    fn run_on_paragraphs(
        &mut self,
        command: impl FnOnce(usize, usize) -> HeadlessCommand,
    ) -> Result<(), String> {
        let (start, end) = self.paragraph_span();
        self.apply_command(command(start, end), true)
    }

    /// The first paragraph of the cell at `(row, column)` of a table, from
    /// the last paint — a cell spanning `column` counts.
    fn cell_paragraph(
        &self,
        table_order_index: usize,
        (row, column): (usize, usize),
    ) -> Option<usize> {
        self.cell_layouts()
            .iter()
            .filter(|layout| self.table_order_for(layout.table_id) == Some(table_order_index))
            .find(|layout| {
                layout.row == row
                    && (layout.column..layout.column + layout.column_span.max(1)).contains(&column)
            })
            .and_then(|layout| layout.paragraphs.first().copied())
    }

    fn table_first_paragraph(&self, table_order_index: usize) -> Option<usize> {
        self.cell_layouts()
            .iter()
            .filter(|layout| self.table_order_for(layout.table_id) == Some(table_order_index))
            .filter_map(|layout| layout.paragraphs.first().copied())
            .min()
    }

    /// The cells between the selection's two ends, as the merge command
    /// takes them: every cell of the table inside the rectangle they span.
    fn selected_cells(
        &self,
        table_order_index: usize,
    ) -> Result<Vec<HeadlessTableCellSelection>, String> {
        let range = self
            .selection_range()
            .filter(|range| !range.is_collapsed())
            .ok_or_else(|| "select the cells to merge".to_string())?;
        let anchor = self.table_cell_of(range.anchor.paragraph);
        let focus = self.table_cell_of(range.focus.paragraph);
        let (Some(anchor), Some(focus)) = (anchor, focus) else {
            return Err("select the cells to merge".to_string());
        };
        if anchor.table_order_index != table_order_index
            || focus.table_order_index != table_order_index
        {
            return Err("the cells to merge are in different tables".to_string());
        }
        let rows = anchor.row.min(focus.row)..=anchor.row.max(focus.row);
        let columns = anchor.column.min(focus.column)..=anchor.column.max(focus.column);
        let table_id = self
            .cell_id_for_paragraph(range.focus.paragraph)
            .and_then(|cell| self.cell_layout(cell))
            .map(|layout| layout.table_id);
        let mut cells: Vec<HeadlessTableCellSelection> = self
            .cell_layouts()
            .iter()
            .filter(|layout| Some(layout.table_id) == table_id)
            .filter(|layout| rows.contains(&layout.row) && columns.contains(&layout.column))
            .map(|layout| HeadlessTableCellSelection {
                row: layout.row,
                column: layout.column,
                column_span: layout.column_span.max(1),
            })
            .collect();
        cells.sort_by_key(|cell| (cell.row, cell.column));
        cells.dedup();
        if cells.len() < 2 {
            return Err("select two or more cells to merge".to_string());
        }
        Ok(cells)
    }

    fn average_row_height(&self, table_order_index: usize) -> Option<f32> {
        let Some(HeadlessQueryResult::Table {
            value: Some(table), ..
        }) = self.query(HeadlessQuery::Table { table_order_index })
        else {
            return None;
        };
        let heights: Vec<f32> = table.rows.iter().filter_map(|row| row.height).collect();
        (!heights.is_empty()).then(|| heights.iter().sum::<f32>() / heights.len() as f32)
    }
}

impl<T: WordSession + ?Sized> Helpers for T {}

/// The ribbon's commands and what it shows as current, for every session.
pub trait WordFormatting: WordSession {
    /// The run style at a position: what the ribbon shows for the caret.
    fn run_style_at(&self, at: Position) -> Option<HeadlessRunStyle> {
        match self.query(HeadlessQuery::CharacterStyleAt {
            paragraph: at.0,
            offset: at.1,
        })? {
            HeadlessQueryResult::CharacterStyleAt { value, .. } => value.map(|value| value.style),
            _ => None,
        }
    }

    /// The paragraph styles the document defines, in its own order.
    fn paragraph_styles(&self) -> Vec<ParagraphStyle> {
        let Some(HeadlessQueryResult::Styles { styles }) = self.query(HeadlessQuery::Styles {
            kind: Some(HeadlessStyleKindFilter::Paragraph),
        }) else {
            return Vec::new();
        };
        styles
            .into_iter()
            .map(|style| ParagraphStyle {
                name: style.name.unwrap_or_else(|| style.style_id.clone()),
                id: style.style_id,
                is_default: style.is_default,
            })
            .collect()
    }

    /// The formatting at the caret, or at the start of the selection.
    fn format_state(&self) -> FormatState {
        let at = self.format_anchor();
        let run = self.run_style_at(at).unwrap_or_default();
        let paragraph = match self.query(HeadlessQuery::Paragraph { paragraph: at.0 }) {
            Some(HeadlessQueryResult::Paragraph { value, .. }) => value,
            _ => None,
        };
        let baseline = run.baseline.as_deref();
        let list = self.list_kind(at.0);
        FormatState {
            bold: run.bold.unwrap_or(false),
            italic: run.italic.unwrap_or(false),
            underline: run.underline.unwrap_or(false),
            strike: run.strike.unwrap_or(false),
            superscript: baseline == Some("superscript"),
            subscript: baseline == Some("subscript"),
            font_size: run.font_size,
            font_family: run.font_family,
            style_id: paragraph.as_ref().and_then(|p| p.style_id.clone()),
            alignment: paragraph.and_then(|p| p.alignment),
            list,
            table: self.caret_table_cell(),
            character: font::WordCharacters::run_format_at(self, at),
        }
    }

    /// The table cell the caret is in, if it is in one.
    fn caret_table_cell(&self) -> Option<TableCell> {
        let paragraph = self.selection_range()?.focus.paragraph;
        let mut cell = self.table_cell_of(paragraph)?;
        cell.can_merge = self.selected_cells(cell.table_order_index).is_ok();
        Some(cell)
    }

    // ---- characters ---------------------------------------------------------

    /// Turns a format on, or off when the run already has it — the ribbon's
    /// toggle buttons.
    fn toggle_format(&mut self, format: CharacterFormat) -> Result<(), String> {
        let state = self.format_state();
        let enabled = !match format {
            CharacterFormat::Bold => state.bold,
            CharacterFormat::Italic => state.italic,
            CharacterFormat::Underline => state.underline,
            CharacterFormat::Strikethrough => state.strike,
            CharacterFormat::Superscript => state.superscript,
            CharacterFormat::Subscript => state.subscript,
        };
        self.run_on_span(|start, end| {
            let (start_paragraph, start_offset, end_paragraph, end_offset) =
                (start.0, start.1, end.0, end.1);
            match format {
                CharacterFormat::Bold => HeadlessCommand::ApplyBold {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    enabled,
                },
                CharacterFormat::Italic => HeadlessCommand::ApplyItalic {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    enabled,
                },
                CharacterFormat::Underline => HeadlessCommand::ApplyUnderline {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    kind: None,
                    enabled,
                },
                CharacterFormat::Strikethrough => HeadlessCommand::ApplyStrike {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    double: false,
                    enabled,
                },
                CharacterFormat::Superscript => HeadlessCommand::ApplySuperscript {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    enabled,
                },
                CharacterFormat::Subscript => HeadlessCommand::ApplySubscript {
                    start_paragraph,
                    start_offset,
                    end_paragraph,
                    end_offset,
                    enabled,
                },
            }
        })
    }

    /// Strips the run formatting of the selection (書式のクリア).
    fn clear_formatting(&mut self) -> Result<(), String> {
        self.run_on_span(|start, end| HeadlessCommand::ClearFormatting {
            start_paragraph: start.0,
            start_offset: start.1,
            end_paragraph: end.0,
            end_offset: end.1,
        })
    }

    fn set_font_size(&mut self, points: f32) -> Result<(), String> {
        self.run_on_span(|start, end| HeadlessCommand::SetFontSize {
            start_paragraph: start.0,
            start_offset: start.1,
            end_paragraph: end.0,
            end_offset: end.1,
            font_size: points.clamp(1.0, 1638.0),
        })
    }

    /// Steps each run through Word's ribbon size list (A↑ / A↓).
    fn step_font_size(&mut self, grow: bool) -> Result<(), String> {
        self.run_on_span(|start, end| HeadlessCommand::AdjustFontSize {
            start_paragraph: start.0,
            start_offset: start.1,
            end_paragraph: end.0,
            end_offset: end.1,
            direction: if grow {
                HeadlessFontSizeDirection::Grow
            } else {
                HeadlessFontSizeDirection::Shrink
            },
            step: HeadlessFontSizeStep::Ladder,
        })
    }

    fn set_font_family(&mut self, family: &str) -> Result<(), String> {
        let font_family = family.to_string();
        self.run_on_span(|start, end| HeadlessCommand::SetFontFamily {
            start_paragraph: start.0,
            start_offset: start.1,
            end_paragraph: end.0,
            end_offset: end.1,
            font_family,
        })
    }

    /// Sets the text colour, as `RRGGBB` or `#RRGGBB`, or Word's `auto`.
    fn set_font_color(&mut self, color: &str) -> Result<(), String> {
        let color = if color.eq_ignore_ascii_case("auto") {
            "auto".to_string()
        } else {
            css_color(color)
        };
        self.run_on_span(|start, end| HeadlessCommand::SetFontColor {
            start_paragraph: start.0,
            start_offset: start.1,
            end_paragraph: end.0,
            end_offset: end.1,
            color,
        })
    }

    /// Highlights the selection with one of Word's highlight names —
    /// `yellow`, `green`, … — or `none` to take it off.
    fn set_highlight(&mut self, color: &str) -> Result<(), String> {
        let color = color.to_string();
        self.run_on_span(|start, end| HeadlessCommand::SetHighlightColor {
            start_paragraph: start.0,
            start_offset: start.1,
            end_paragraph: end.0,
            end_offset: end.1,
            color,
        })
    }

    // ---- paragraphs ---------------------------------------------------------

    /// Word's vocabulary: `left`, `center`, `right`, `both`.
    fn set_alignment(&mut self, alignment: &str) -> Result<(), String> {
        let alignment = alignment.to_string();
        self.run_on_paragraphs(|start, end| HeadlessCommand::SetParagraphAlignment {
            start_paragraph: start,
            end_paragraph: end,
            alignment,
        })
    }

    /// Steps the start indent one tab stop (or one character, for a
    /// character-unit indent), as the ribbon's インデント buttons do.
    fn step_indent(&mut self, increase: bool) -> Result<(), String> {
        self.run_on_paragraphs(|start, end| HeadlessCommand::AdjustParagraphIndent {
            start_paragraph: start,
            end_paragraph: end,
            direction: if increase {
                HeadlessIndentDirection::Increase
            } else {
                HeadlessIndentDirection::Decrease
            },
            target: HeadlessIndentTarget::Start,
        })
    }

    /// Line height as a multiple of a single line (1.0, 1.15, 1.5, …): Word's
    /// `auto` rule, which the command takes as the multiple itself and
    /// writes in 240ths of a line.
    fn set_line_spacing(&mut self, multiple: f32) -> Result<(), String> {
        self.run_on_paragraphs(|start, end| HeadlessCommand::SetParagraphLineSpacing {
            start_paragraph: start,
            end_paragraph: end,
            line: Some(Some(multiple)),
            line_rule: Some("auto".to_string()),
            before: None,
            after: None,
        })
    }

    /// Space before and after, in points; `None` leaves that side alone.
    fn set_paragraph_spacing(
        &mut self,
        before: Option<f32>,
        after: Option<f32>,
    ) -> Result<(), String> {
        self.run_on_paragraphs(|start, end| HeadlessCommand::SetParagraphLineSpacing {
            start_paragraph: start,
            end_paragraph: end,
            line: None,
            line_rule: None,
            before,
            after,
        })
    }

    fn set_paragraph_shading(&mut self, color: Option<&str>) -> Result<(), String> {
        let color = color.map(css_color);
        self.run_on_paragraphs(|start, end| HeadlessCommand::SetParagraphShading {
            start_paragraph: start,
            end_paragraph: end,
            color,
        })
    }

    /// Puts a border on the paragraphs' sides, or takes them off. Black,
    /// ¾ pt, as jade-ide's runtime draws them.
    fn set_paragraph_borders(&mut self, borders: ParagraphBorders) -> Result<(), String> {
        let edge = || Some(line("#000000", 0.75));
        let (top, bottom, sides) = match borders {
            ParagraphBorders::Bottom => (None, edge(), None),
            ParagraphBorders::Top => (edge(), None, None),
            ParagraphBorders::Box => (edge(), edge(), edge()),
            ParagraphBorders::None => (None, None, None),
        };
        self.run_on_paragraphs(|start, end| HeadlessCommand::SetParagraphBorders {
            start_paragraph: start,
            end_paragraph: end,
            top,
            bottom,
            left: sides.clone(),
            right: sides,
        })
    }

    /// Applies a paragraph style by id — one the document defines, or a
    /// built-in one (`Heading1`, `Title`, …) the engine adds when it is
    /// missing. `None` goes back to the default style.
    fn set_paragraph_style(&mut self, style_id: Option<&str>) -> Result<(), String> {
        let style_id = style_id.map(str::to_string);
        self.run_on_paragraphs(|start, end| HeadlessCommand::SetParagraphStyle {
            start_paragraph: start,
            end_paragraph: end,
            style_id,
        })
    }

    /// The 箇条書き / 段落番号 buttons: a bulleted or numbered list, or — when
    /// the paragraph is already in that kind of list — plain paragraphs again.
    /// The list definition is created in `numbering.xml` when it is missing.
    fn toggle_list(&mut self, bullets: bool) -> Result<(), String> {
        let (start, _) = self.paragraph_span();
        let wanted = if bullets {
            ListKind::Bullet
        } else {
            ListKind::Decimal
        };
        if self.list_kind(start) == Some(wanted) {
            return self.run_on_paragraphs(|start, end| HeadlessCommand::SetNumbering {
                start_paragraph: start,
                end_paragraph: end,
                num_id: None,
                level: None,
            });
        }
        self.run_on_paragraphs(|start, end| HeadlessCommand::ApplyMultilevelList {
            start_paragraph: start,
            end_paragraph: end,
            preset: if bullets {
                HeadlessMultilevelListPreset::BulletHierarchy
            } else {
                HeadlessMultilevelListPreset::DecimalHierarchy
            },
            level: 0,
        })
    }

    // ---- tables -------------------------------------------------------------

    /// Inserts a `rows` × `columns` table where Word puts one for the caret,
    /// and puts the caret in its first cell. Returns where the caret went.
    fn insert_table(&mut self, rows: usize, columns: usize) -> Result<Position, String> {
        let (paragraph, offset) = self
            .selection_range()
            .map(|range| {
                let (start, _) = range.normalized();
                (start.paragraph, start.offset)
            })
            .unwrap_or((0, 0));
        let outer_table = self
            .cell_id_for_paragraph(paragraph)
            .and_then(|id| self.cell_layout(id))
            .map(|cell| cell.table_id);
        // The caret moves into the table, so no highlight is to be painted.
        self.collapse_to((paragraph, offset));
        // With Word's 表 (格子) style, as Word's 挿入 ▸ 表 gives every table:
        // without it Word shows the table with no lines.
        self.execute_command(
            Box::new(
                crate::commands::insert_table::InsertTableCommand::at_caret(
                    paragraph,
                    offset,
                    rows.max(1),
                    columns.max(1),
                    Vec::new(),
                )
                .with_style("TableGrid"),
            ),
            false,
        )?;
        // In a cell the new table is nested in it: its first cell is the
        // first paragraph after the caret's laid out in another table.
        if let Some(outer) = outer_table {
            let inner = (paragraph + 1..paragraph + 256)
                .find(|&candidate| {
                    self.cell_id_for_paragraph(candidate)
                        .and_then(|id| self.cell_layout(id))
                        .is_some_and(|cell| cell.table_id != outer)
                })
                .unwrap_or(paragraph);
            return Ok((inner, 0));
        }
        // Above the paragraph at its start, after it otherwise: the first
        // cell is whichever of the two is now in a table.
        let first_cell = [paragraph, paragraph + 1]
            .into_iter()
            .find(|candidate| self.table_cell_of(*candidate).is_some())
            .unwrap_or(paragraph);
        Ok((first_cell, 0))
    }

    /// Runs a table command on the caret's table. Returns where the caret
    /// should go: a structural edit renumbers the paragraphs after it.
    fn table_command(&mut self, command: TableCommand) -> Result<Position, String> {
        let cell = self
            .caret_table_cell()
            .ok_or_else(|| "the caret is not in a table".to_string())?;
        let caret = self
            .selection_range()
            .map(|range| (range.focus.paragraph, range.focus.offset))
            .unwrap_or((0, 0));
        let table_order_index = cell.table_order_index;
        let (row, column) = (cell.row, cell.column);
        // The cell the caret goes back to after a structural edit, which
        // renumbers the paragraphs after it.
        let mut caret_cell = (row, column);
        let mut caret_table = table_order_index;
        let command_is_structural = command.is_structural();
        let headless = match command {
            TableCommand::InsertRowAbove => {
                caret_cell.0 += 1;
                HeadlessCommand::InsertTableRow {
                    table_order_index,
                    row_index: row,
                    template_row: Some(row),
                }
            }
            TableCommand::InsertRowBelow => HeadlessCommand::InsertTableRow {
                table_order_index,
                row_index: row + 1,
                template_row: Some(row),
            },
            TableCommand::InsertColumnLeft | TableCommand::InsertColumnRight => {
                let column_index = if command == TableCommand::InsertColumnLeft {
                    caret_cell.1 += 1;
                    column
                } else {
                    column + 1
                };
                HeadlessCommand::InsertTableColumn {
                    table_order_index,
                    column_index,
                    width: self
                        .word_document()
                        .column_insert_width(table_order_index, column),
                    template_column: Some(column),
                }
            }
            TableCommand::DeleteRow if cell.rows <= 1 => {
                HeadlessCommand::DeleteTable { table_order_index }
            }
            TableCommand::DeleteRow => {
                caret_cell.0 = row.min(cell.rows - 2);
                HeadlessCommand::DeleteTableRow {
                    table_order_index,
                    row_index: row,
                }
            }
            TableCommand::DeleteColumn if cell.columns <= 1 => {
                HeadlessCommand::DeleteTable { table_order_index }
            }
            TableCommand::DeleteColumn => {
                caret_cell.1 = column.min(cell.columns - 2);
                HeadlessCommand::DeleteTableColumn {
                    table_order_index,
                    column_index: column,
                }
            }
            TableCommand::DeleteTable => HeadlessCommand::DeleteTable { table_order_index },
            TableCommand::MergeCells => {
                let selected_cells = self.selected_cells(table_order_index)?;
                caret_cell = selected_cells
                    .iter()
                    .map(|cell| (cell.row, cell.column))
                    .min()
                    .unwrap_or(caret_cell);
                HeadlessCommand::MergeTableCells {
                    table_order_index,
                    selected_cells,
                }
            }
            TableCommand::UnmergeCells => {
                if !cell.can_unmerge {
                    return Err("the cell is not merged".to_string());
                }
                HeadlessCommand::UnmergeTableCells {
                    table_order_index,
                    target_cells: vec![HeadlessTableCellSelection {
                        row,
                        column,
                        column_span: 1,
                    }],
                }
            }
            TableCommand::SplitCell { columns, rows } => HeadlessCommand::SplitTableCell {
                table_order_index,
                row,
                column,
                columns: columns.max(1),
                rows: rows.max(1),
            },
            TableCommand::SplitTable => {
                // The caret's row heads the second table; at the first row the
                // table stays whole, with an empty paragraph put before it.
                if row > 0 {
                    caret_table += 1;
                    caret_cell.0 = 0;
                }
                HeadlessCommand::SplitTable {
                    table_order_index,
                    row_index: row,
                }
            }
            TableCommand::DistributeRows => HeadlessCommand::DistributeTableRows {
                table_order_index,
                // Word's own: the rows share the height they have, and rows
                // with no height of their own get the default line.
                height: Some(self.average_row_height(table_order_index).unwrap_or(18.0)),
            },
            TableCommand::DistributeColumns => HeadlessCommand::DistributeTableColumns {
                table_order_index,
                total_width: None,
            },
            TableCommand::Align(alignment) => HeadlessCommand::SetTableAlignment {
                table_order_index,
                alignment,
            },
            TableCommand::CellVerticalAlign(alignment) => {
                HeadlessCommand::SetTableCellVerticalAlignment {
                    table_order_index,
                    row,
                    column,
                    alignment,
                }
            }
            TableCommand::CellShading(color) => HeadlessCommand::SetTableCellShading {
                table_order_index,
                row,
                column,
                color: color.as_deref().map(css_color),
            },
            TableCommand::StylePreset(preset) => HeadlessCommand::ApplyTableStylePreset {
                table_order_index,
                preset,
            },
            TableCommand::Borders(borders) => {
                let edge = match borders {
                    TableBorders::All => line("#808080", 0.5),
                    TableBorders::None => no_line(),
                };
                HeadlessCommand::SetTableBorders {
                    table_order_index,
                    top: Some(edge.clone()),
                    bottom: Some(edge.clone()),
                    left: Some(edge.clone()),
                    right: Some(edge.clone()),
                    inside_h: Some(edge.clone()),
                    inside_v: Some(edge),
                }
            }
            TableCommand::RepeatHeader(on) => HeadlessCommand::SetTableRowProperties {
                table_order_index,
                row,
                repeat_header: Some(on),
                cant_split: None,
            },
        };
        let structural = command_is_structural;
        // Where the table starts, for when it is gone: the paragraph after it
        // takes that index.
        let table_start = self.table_first_paragraph(table_order_index);
        if structural {
            // The caret is put back afterwards; a highlight painted now would
            // stay on the pages.
            self.collapse_to(caret);
        }
        self.apply_command(headless, !structural)?;
        if !structural {
            return Ok(caret);
        }
        let paragraph = self
            .cell_paragraph(caret_table, caret_cell)
            .or(table_start)
            .unwrap_or(caret.0)
            .min(self.paragraph_total().saturating_sub(1));
        let position = (paragraph, 0);
        self.collapse_to(position);
        Ok(position)
    }

    /// Tab and ⇧Tab in a table, as in Word: the next or the previous cell,
    /// its text to be selected so typing replaces it. Tab in the last cell
    /// adds a row below and goes into its first cell. Nothing is selected
    /// here: the shell selects what this returns, as it selects anything.
    fn step_cell(&mut self, forwards: bool) -> Result<CellStep, String> {
        let Some(paragraph) = self.selection_range().map(|range| range.focus.paragraph) else {
            return Ok(CellStep::NotInTable);
        };
        let Some(current) = self
            .cell_id_for_paragraph(paragraph)
            .and_then(|id| self.cell_layout(id))
            .map(|cell| (cell.table_id, cell.row, cell.column))
        else {
            return Ok(CellStep::NotInTable);
        };
        // The table's cells in reading order, each with its first and last
        // paragraph: a cell split over two pages is laid out as two pieces.
        let mut cells = std::collections::BTreeMap::<(usize, usize), (usize, usize)>::new();
        for cell in self.cell_layouts() {
            let (Some(&first), Some(&last)) = (cell.paragraphs.first(), cell.paragraphs.last())
            else {
                continue;
            };
            if cell.table_id != current.0 {
                continue;
            }
            cells
                .entry((cell.row, cell.column))
                .and_modify(|(from, to)| {
                    *from = (*from).min(first);
                    *to = (*to).max(last);
                })
                .or_insert((first, last));
        }
        let here = (current.1, current.2);
        let target = if forwards {
            cells
                .range((std::ops::Bound::Excluded(here), std::ops::Bound::Unbounded))
                .next()
        } else {
            cells.range(..here).next_back()
        };
        match target {
            Some((_, &(first, last))) => {
                let end = self.text_of(last).map_or(0, |text| text.len());
                Ok(CellStep::Select((first, 0), (last, end)))
            }
            None if forwards => {
                let caret = self.table_command(TableCommand::InsertRowBelow)?;
                self.collapse_to(caret);
                match self.step_cell(true)? {
                    CellStep::NotInTable => Ok(CellStep::Stay),
                    step => Ok(step),
                }
            }
            None => Ok(CellStep::Stay),
        }
    }
}

impl<T: WordSession + ?Sized> WordFormatting for T {}

/// Where Tab takes the caret: see [`WordFormatting::step_cell`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellStep {
    /// The caret is not in a table, where Tab types a tab.
    NotInTable,
    /// ⇧Tab in the first cell: nowhere to go.
    Stay,
    /// The cell to go to, from its start to its end.
    Select(Position, Position),
}

/// One entry of the style gallery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GalleryStyle {
    /// The id to apply; `None` takes the paragraph style off (標準).
    pub id: Option<String>,
    /// The document's own name for it, or `None` for the four fixed entries,
    /// which the shell labels in its own language.
    pub name: Option<String>,
    /// Which fixed entry this is: 0 標準, 1–3 見出し 1–3.
    pub preset: Option<usize>,
}

/// The fixed head of the gallery and what each entry also answers to —
/// the names and ids Word gives these styles in other languages.
const PRESETS: [(Option<&str>, &[&str]); 4] = [
    (None, &["Normal", "Standard", "標準"]),
    (Some("Heading1"), &["heading 1", "見出し 1", "1"]),
    (Some("Heading2"), &["heading 2", "見出し 2", "2"]),
    (Some("Heading3"), &["heading 3", "見出し 3", "3"]),
];

/// Lower case and no spaces, so `heading 1` and `Heading1` are one style.
fn style_token(value: &str) -> Option<String> {
    let token: String = value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    (!token.is_empty()).then_some(token)
}

/// jade-ide's style gallery: 標準 and 見出し 1–3 always — Word offers them
/// even to a document that only defines Normal — then the document's own
/// paragraph styles, default first, headings in level order, the rest by
/// name, without the ones that are a fixed entry under another name.
pub fn style_gallery(styles: &[ParagraphStyle]) -> Vec<GalleryStyle> {
    let mut gallery: Vec<GalleryStyle> = PRESETS
        .iter()
        .enumerate()
        .map(|(index, (id, _))| GalleryStyle {
            id: id.map(str::to_string),
            name: None,
            preset: Some(index),
        })
        .collect();
    let mut seen: std::collections::HashSet<String> = PRESETS
        .iter()
        .flat_map(|(id, aliases)| id.iter().chain(aliases.iter()))
        .filter_map(|token| style_token(token))
        .collect();
    let heading_level = |style: &ParagraphStyle| {
        style
            .name
            .to_lowercase()
            .strip_prefix("heading ")
            .and_then(|level| level.trim().parse::<u32>().ok())
    };
    let mut styles: Vec<&ParagraphStyle> = styles.iter().collect();
    styles.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| match (heading_level(a), heading_level(b)) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.cmp(&b.name),
            })
    });
    for style in styles {
        let id = style_token(&style.id);
        let name = style_token(&style.name);
        if [&id, &name]
            .into_iter()
            .flatten()
            .any(|token| seen.contains(token))
        {
            continue;
        }
        seen.extend(id.into_iter().chain(name));
        gallery.push(GalleryStyle {
            id: Some(style.id.clone()),
            name: Some(style.name.clone()),
            preset: None,
        });
    }
    gallery
}

impl GalleryStyle {
    /// Whether this entry is the paragraph's style. A fixed entry matches
    /// any of its names; the default style, or none, is 標準.
    pub fn is_active(&self, style_id: Option<&str>, default_id: Option<&str>) -> bool {
        let active = style_id.and_then(style_token);
        match self.preset {
            Some(index) => {
                let (id, aliases) = PRESETS[index];
                if id.is_none() {
                    return active.is_none()
                        || active == default_id.and_then(style_token)
                        || aliases.iter().any(|alias| style_token(alias) == active);
                }
                id.iter()
                    .chain(aliases.iter())
                    .any(|token| style_token(token) == active)
            }
            None => active.is_some() && self.id.as_deref().and_then(style_token) == active,
        }
    }
}
