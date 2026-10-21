//! The spreadsheet model.
//!
//! Deliberately close to the file format: a `Cell` keeps the style index the
//! sheet part wrote rather than a resolved style, and a formula cell keeps both
//! the expression and the value Excel cached for it. Layout and rendering
//! resolve what they need through [`StyleTable`], which leaves this model able
//! to describe a workbook faithfully enough to write back later.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::{Result, XlsxError};

/// XLSX worksheet bounds defined by SpreadsheetML.
pub const MAX_COLUMN_COUNT: u32 = 16_384;
pub const MAX_ROW_COUNT: u32 = 1_048_576;

/// A zero-based cell address. Row and column are separate axes here even though
/// the file writes them as `A1`, because every geometric decision downstream is
/// per axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellRef {
    pub column: u32,
    pub row: u32,
}

impl CellRef {
    pub fn new(column: u32, row: u32) -> Self {
        Self { column, row }
    }

    /// Parses an `A1`-style reference, ignoring any `$` absolute markers.
    ///
    /// Column letters are base-26 with no zero: `A` is 0, `Z` is 25, `AA` is 26.
    pub fn parse(value: &str) -> Result<Self> {
        Self::parse_impl(value)
    }

    fn parse_impl(value: &str) -> Result<Self> {
        let bad = || XlsxError::BadCellReference {
            value: value.to_string(),
        };
        let mut column: u32 = 0;
        let mut letters = 0usize;
        let mut digits = String::new();
        for c in value.chars() {
            match c {
                '$' => continue,
                'A'..='Z' | 'a'..='z' => {
                    if !digits.is_empty() {
                        return Err(bad());
                    }
                    let index = u32::from(c.to_ascii_uppercase() as u8 - b'A') + 1;
                    column = column
                        .checked_mul(26)
                        .and_then(|c| c.checked_add(index))
                        .ok_or_else(bad)?;
                    letters += 1;
                }
                '0'..='9' => digits.push(c),
                _ => return Err(bad()),
            }
        }
        if letters == 0 || digits.is_empty() {
            return Err(bad());
        }
        let row: u32 = digits.parse().map_err(|_| bad())?;
        if row == 0 || row > MAX_ROW_COUNT || column > MAX_COLUMN_COUNT {
            return Err(bad());
        }
        Ok(Self {
            column: column - 1,
            row: row - 1,
        })
    }
}

/// An inclusive rectangle of cells, as `mergeCell` and friends write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellRange {
    pub start: CellRef,
    pub end: CellRef,
}

impl CellRange {
    /// Parses `A1:B2`. A single reference (`A1`) is a one-cell range.
    pub fn parse(value: &str) -> Result<Self> {
        match value.split_once(':') {
            Some((start, end)) => Ok(Self {
                start: CellRef::parse_impl(start)?,
                end: CellRef::parse_impl(end)?,
            }),
            None => {
                let only = CellRef::parse_impl(value)?;
                Ok(Self {
                    start: only,
                    end: only,
                })
            }
        }
    }

    pub fn contains(&self, cell: CellRef) -> bool {
        cell.column >= self.start.column
            && cell.column <= self.end.column
            && cell.row >= self.start.row
            && cell.row <= self.end.row
    }
}

/// A cell's value as the file stores it.
///
/// `Number` covers dates too: a date in xlsx is a number plus a number format,
/// and keeping them one variant means the format is the only thing that decides
/// how it reads — which is what Excel does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CellValue {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    /// `#REF!`, `#DIV/0!` and the rest, kept verbatim.
    Error(String),
}

impl CellValue {
    pub fn is_empty(&self) -> bool {
        matches!(self, CellValue::Empty)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub reference: CellRef,
    pub value: CellValue,
    /// Index into [`StyleTable::cell_formats`], as written in the `s` attribute.
    pub style_index: Option<u32>,
    /// The formula expression without its leading `=`. The cached result stays
    /// in `value`, which is what rendering uses until a recalculation engine is
    /// wired in.
    pub formula: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// Zero-based.
    pub index: u32,
    /// Height in points as the file states it. Excel writes this both for a
    /// height the user pinned and for one it measured itself, so it is the
    /// height to draw; `custom_height` says which of the two it is.
    pub height_points: Option<f32>,
    /// Effective pinned height, inherited from the worksheet unless overridden
    /// by `customHeight`; re-measuring wrapped text must not grow a pinned row.
    pub custom_height: bool,
    /// `customFormat="1"` — `style_index` is an intentional row default.
    /// Without this flag Excel ignores the row's `s` attribute for cells that
    /// do not carry their own style.
    #[serde(default)]
    pub custom_format: bool,
    /// Effective visibility, inherited from `sheetFormatPr/@zeroHeight` unless
    /// the row has an explicit `hidden` attribute.
    pub hidden: bool,
    pub style_index: Option<u32>,
    pub cells: Vec<Cell>,
}

/// One `<col>` entry: a run of columns sharing a width.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnSpan {
    /// Zero-based, inclusive.
    pub first: u32,
    /// Zero-based, inclusive.
    pub last: u32,
    /// Width in "characters" — the unit the file uses. Kept unconverted here;
    /// turning it into pixels needs font metrics and belongs to layout.
    /// Raw SpreadsheetML encoded digit units, including padding.
    pub width_chars: Option<f32>,
    /// `customWidth="1"` — the width was set rather than derived from the
    /// sheet default.
    pub custom_width: bool,
    pub hidden: bool,
    pub style_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    #[serde(default)]
    pub auto_filter: Option<crate::auto_filter::AutoFilter>,
    #[serde(default)]
    pub filter_mode: Option<bool>,
    #[serde(default, skip_serializing)]
    pub filter_row_state: Option<crate::filter_state::FilterRowState>,
    pub name: String,
    /// The part name inside the archive, e.g. `xl/worksheets/sheet1.xml`.
    pub part_name: String,
    /// Tab visibility from `workbook.xml`; omitted there means visible.
    #[serde(default)]
    pub visibility: SheetVisibility,
    pub columns: Vec<ColumnSpan>,
    pub rows: Vec<Row>,
    pub merges: Vec<CellRange>,
    /// Cell or range hyperlinks declared by the worksheet. External targets
    /// come from the worksheet's relationship part; workbook-internal links
    /// use `location` instead.
    #[serde(default)]
    pub hyperlinks: Vec<Hyperlink>,
    /// Legacy cell comments (called notes by current Excel).
    #[serde(default)]
    pub comments: Vec<CellComment>,
    /// Standard worksheet input rules and the ranges they apply to.
    #[serde(default)]
    pub data_validations: Vec<DataValidation>,
    #[serde(default)]
    pub conditional_formats: Vec<crate::conditional_format::ConditionalFormatting>,
    /// `sheetFormatPr/@defaultColWidth`, in encoded digit units.
    pub default_column_width_chars: Option<f32>,
    /// `sheetFormatPr/@defaultRowHeight`, in points.
    pub default_row_height_points: Option<f32>,
    /// The worksheet default height is pinned rather than auto-grown.
    #[serde(default)]
    pub default_row_custom_height: bool,
    /// Rows without an explicit visibility override are hidden.
    #[serde(default)]
    pub default_rows_hidden: bool,
    /// `sheetView/@showGridLines`; Excel defaults it to on.
    pub show_grid_lines: bool,
    /// Fixed rows and columns from the sheet's frozen `<pane>`, if any.
    #[serde(default)]
    pub freeze_pane: Option<FreezePane>,
    /// Excel tables (`ListObject`s) on the sheet, from its `<tableParts>`.
    #[serde(default)]
    pub tables: Vec<Table>,
    /// The page header and footer, from `<headerFooter>`.
    #[serde(default)]
    pub header_footer: Option<HeaderFooter>,
    /// Manual and automatic page breaks between rows, from `<rowBreaks>`.
    #[serde(default)]
    pub row_breaks: Vec<PageBreak>,
    /// Page breaks between columns, from `<colBreaks>`.
    #[serde(default)]
    pub column_breaks: Vec<PageBreak>,
    /// How the sheet prints: its paper, orientation, scaling, margins and
    /// print titles.
    #[serde(default)]
    pub page_setup: PageSetup,
    /// Shapes, text boxes, pictures and charts anchored on the sheet, from
    /// its drawing part.
    #[serde(default)]
    pub shapes: Vec<Shape>,
    /// The array formulas (`<f t="array" ref>`): dynamic arrays and the older
    /// Ctrl+Shift+Enter ones. The rest of each range is the formula's
    /// output, which the file stores as plain values.
    #[serde(default)]
    pub array_formulas: Vec<ArrayFormula>,
    /// The furigana Excel saved with text cells (`<rPh>`), as the reading
    /// PHONETIC returns: 山田太郎 with ヤマダ and タロウ reads ヤマダタロウ.
    #[serde(default)]
    pub phonetic_readings: std::collections::BTreeMap<CellRef, String>,
    /// The sheet's protection (`<sheetProtection sheet="1">`), when it is
    /// protected: what it still allows.
    #[serde(default)]
    pub protection: Option<SheetProtection>,
}

/// What a protected sheet still allows, as its `sheetProtection` says. Each
/// flag is `true` when the operation is allowed; Excel's defaults allow
/// none of them. Cells are locked unless their format unlocks them
/// ([`CellFormat::unlocked`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SheetProtection {
    pub format_cells: bool,
    pub format_columns: bool,
    pub format_rows: bool,
    pub insert_columns: bool,
    pub insert_rows: bool,
    pub insert_hyperlinks: bool,
    pub delete_columns: bool,
    pub delete_rows: bool,
    pub sort: bool,
    pub auto_filter: bool,
    /// Notes, shapes and charts (`objects`).
    pub objects: bool,
}

/// An array formula: its anchor holds the formula, and the rest of `range`
/// holds what it returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrayFormula {
    pub anchor: CellRef,
    pub range: CellRange,
}

impl ArrayFormula {
    /// Whether `cell` holds this formula's output rather than a value of
    /// its own.
    pub fn outputs_to(&self, cell: CellRef) -> bool {
        cell != self.anchor
            && (self.range.start.column..=self.range.end.column).contains(&cell.column)
            && (self.range.start.row..=self.range.end.row).contains(&cell.row)
    }
}

/// An Excel table — what the ribbon calls "Format as Table" and the object
/// model a `ListObject` — as `xl/tables/tableN.xml` declares it.
///
/// Excel checks a table against the grid when it opens the file: the column
/// count has to equal the width of `reference`, and each header cell has to
/// hold its column's name. A writer that moves cells without keeping these in
/// step produces a file Excel offers to repair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    #[serde(default)]
    pub filter: Option<crate::auto_filter::AutoFilter>,
    /// The part name inside the archive, e.g. `xl/tables/table1.xml`.
    pub part_name: String,
    /// `name`: the identifier structured references use (`Table1[Amount]`).
    pub name: String,
    /// `displayName`: what the ribbon shows; usually the same as `name`.
    pub display_name: String,
    /// The cells the table covers, header and totals rows included.
    pub reference: CellRange,
    /// `headerRowCount`; Excel writes 0 or 1 (the attribute defaults to 1).
    pub header_row_count: u32,
    /// `totalsRowCount`; Excel writes 0 or 1 (the attribute defaults to 0).
    pub totals_row_count: u32,
    /// Column names, left to right — the same text the header cells hold.
    pub columns: Vec<String>,
    /// `tableStyleInfo/@name`, e.g. `TableStyleMedium2`.
    pub style_name: Option<String>,
    /// `tableStyleInfo/@showFirstColumn`: the first column is emphasised.
    #[serde(default)]
    pub show_first_column: bool,
    /// `tableStyleInfo/@showLastColumn`.
    #[serde(default)]
    pub show_last_column: bool,
    /// `tableStyleInfo/@showRowStripes`: banded rows.
    #[serde(default)]
    pub show_row_stripes: bool,
    /// `tableStyleInfo/@showColumnStripes`: banded columns.
    #[serde(default)]
    pub show_column_stripes: bool,
    /// Whether the table carries an `<autoFilter>` (the header dropdowns).
    pub auto_filter: bool,
}

impl Table {
    /// The row the column names sit on, when the table shows a header.
    pub fn header_row(&self) -> Option<u32> {
        (self.header_row_count > 0).then_some(self.reference.start.row)
    }

    /// The row the totals sit on, when the table shows one.
    pub fn totals_row(&self) -> Option<u32> {
        (self.totals_row_count > 0).then_some(self.reference.end.row)
    }

    /// The name of the column `cell` is in, when the cell is inside the table.
    pub fn column_name_at(&self, cell: CellRef) -> Option<&str> {
        if !self.reference.contains(cell) {
            return None;
        }
        self.columns
            .get((cell.column - self.reference.start.column) as usize)
            .map(String::as_str)
    }
}

/// The page header and footer of a sheet, in Excel's own code language:
/// `&L`, `&C` and `&R` open the left, centre and right sections, `&P` is the
/// page number, `&N` the page count, `&D` the date, `&A` the sheet name, `&F`
/// the file name, and `&&` a literal ampersand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HeaderFooter {
    pub odd_header: Option<String>,
    pub odd_footer: Option<String>,
    /// Used only when `different_odd_even` is set.
    pub even_header: Option<String>,
    pub even_footer: Option<String>,
    /// Used only when `different_first` is set.
    pub first_header: Option<String>,
    pub first_footer: Option<String>,
    pub different_odd_even: bool,
    pub different_first: bool,
    /// `scaleWithDoc`; Excel defaults it to on.
    pub scale_with_doc: bool,
    /// `alignWithMargins`; Excel defaults it to on.
    pub align_with_margins: bool,
}

/// How a sheet prints, from its `<pageSetup>`, `<pageMargins>`,
/// `sheetPr/pageSetUpPr` and the workbook's `_xlnm.Print_Titles` for it —
/// what Excel's Page Setup dialog shows. `None` is an attribute the file
/// leaves out, which Excel reads as its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PageSetup {
    /// `paperSize`: Excel's paper code — 1 Letter, 5 Legal, 8 A3, 9 A4,
    /// 11 A5, 12 B4, 13 B5 and so on. Unset, the printer's default.
    pub paper_size: Option<u32>,
    /// `orientation`; unset (or `default`), portrait.
    pub orientation: Option<PageOrientation>,
    /// `scale`: the percentage to print at, 10 to 400; unset, 100.
    pub scale: Option<u32>,
    /// `pageSetUpPr/@fitToPage`: Fit to N pages wide by M tall, rather than
    /// `scale`.
    pub fit_to_page: bool,
    /// `fitToWidth`: pages across when fitting, 0 for as many as it takes;
    /// unset, 1.
    pub fit_to_width: Option<u32>,
    /// `fitToHeight`: pages down when fitting, 0 for as many as it takes;
    /// unset, 1.
    pub fit_to_height: Option<u32>,
    /// `pageOrder="overThenDown"`: number the pages across before down.
    pub over_then_down: bool,
    /// `<pageMargins>`, in inches.
    pub margins: Option<PageMargins>,
    /// The rows repeated at the top of every page, zero-based and inclusive,
    /// from `_xlnm.Print_Titles` (`Sheet1!$1:$2`).
    pub title_rows: Option<(u32, u32)>,
    /// The columns repeated at the left of every page (`Sheet1!$A:$A`).
    pub title_columns: Option<(u32, u32)>,
}

impl PageSetup {
    /// Pages across and down when fitting to pages, 0 for as many as it
    /// takes; `None` when the sheet prints at a scale instead.
    pub fn fit_to(&self) -> Option<(u32, u32)> {
        self.fit_to_page.then(|| {
            (
                self.fit_to_width.unwrap_or(1),
                self.fit_to_height.unwrap_or(1),
            )
        })
    }
}

/// A page's orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageOrientation {
    Portrait,
    Landscape,
}

/// A sheet's margins, in inches, as `<pageMargins>` states them: the cells'
/// from each edge of the paper, and the header's and footer's from the top
/// and bottom edges.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageMargins {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub header: f64,
    pub footer: f64,
}

impl Default for PageMargins {
    /// Excel's Normal margins.
    fn default() -> Self {
        Self {
            left: 0.7,
            right: 0.7,
            top: 0.75,
            bottom: 0.75,
            header: 0.3,
            footer: 0.3,
        }
    }
}

/// A page break, as `<brk>` states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageBreak {
    /// The zero-based row (or column) the break sits *above* (or to the left
    /// of): Excel writes `id="5"` for a break between rows 5 and 6, and the
    /// break belongs to row 6 — it moves with that row.
    pub before: u32,
    /// `man="1"`: placed by the user rather than by pagination.
    pub manual: bool,
}

/// What a drawing anchor holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    /// A shape whose `cNvSpPr` says `txBox="1"`: the ribbon's Text Box.
    TextBox,
    /// Any other `sp`: a rectangle, an arrow, a callout.
    Shape,
    /// A `pic`.
    Picture,
    /// A `graphicFrame` holding a chart.
    Chart,
    /// A `graphicFrame` holding something else — a SmartArt diagram, say.
    GraphicFrame,
    /// A `grpSp`.
    Group,
    /// A `cxnSp`.
    Connector,
}

/// One object from the sheet's drawing part, by the cells it is anchored to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    /// `cNvPr/@id`, unique within the drawing.
    pub id: u32,
    /// `cNvPr/@name`, e.g. `TextBox 1`.
    pub name: String,
    pub kind: ShapeKind,
    /// The cell the top-left corner sits in.
    pub from: CellRef,
    /// How far into `from` the corner sits, as (x, y) in EMUs — 914,400 to
    /// the inch, 9,525 to a pixel at 96 dpi.
    #[serde(default)]
    pub from_offset: (i64, i64),
    /// The cell the bottom-right corner sits in, for a two-cell anchor.
    pub to: Option<CellRef>,
    /// How far into `to` that corner sits, in EMUs.
    #[serde(default)]
    pub to_offset: (i64, i64),
    /// The object's size in EMUs, for a one-cell anchor (which has no `to`).
    #[serde(default)]
    pub extent: Option<(i64, i64)>,
    /// Paragraphs of the shape's text, joined with `\n`, for a shape that has
    /// a text body. `Some("")` is an empty text box; `None` a shape with no
    /// text body at all.
    pub text: Option<String>,
    #[serde(default)]
    pub picture: Option<Picture>,
    #[serde(default)]
    pub chart: Option<crate::chart::Chart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    pub relationship_id: Option<String>,
    pub media: Option<MediaPart>,
    pub external_target: Option<String>,
    /// DrawingML percentages in 1/1000 percent; negative values extend the image.
    pub crop: [i32; 4],
    #[serde(default)]
    pub fill: [i32; 4],
    #[serde(default)]
    pub unsupported_properties: Vec<String>,
    /// DrawingML angle in 1/60000 degree.
    pub rotation: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub opacity: u32,
}

impl Default for Picture {
    fn default() -> Self {
        Self {
            relationship_id: None,
            media: None,
            external_target: None,
            crop: [0; 4],
            fill: [0; 4],
            unsupported_properties: Vec::new(),
            rotation: 0,
            flip_h: false,
            flip_v: false,
            opacity: 100_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPart {
    pub part_name: String,
    pub cache_key: String,
    pub content_type: String,
    pub bytes: std::rc::Rc<Vec<u8>>,
}

/// EMUs per pixel at 96 dpi: 914,400 per inch over 96.
pub const EMU_PER_PX: f64 = 9525.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SheetVisibility {
    #[default]
    Visible,
    Hidden,
    VeryHidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezePane {
    pub frozen_columns: u32,
    pub frozen_rows: u32,
    /// The first cell in the scrollable bottom-right pane.
    pub top_left_cell: Option<CellRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hyperlink {
    pub reference: CellRange,
    pub target: Option<String>,
    pub location: Option<String>,
    pub display: Option<String>,
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellComment {
    pub reference: CellRef,
    pub author: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataValidation {
    pub ranges: Vec<CellRange>,
    pub validation_type: Option<String>,
    pub operator: Option<String>,
    pub allow_blank: bool,
    pub show_input_message: bool,
    pub show_error_message: bool,
    pub prompt_title: Option<String>,
    pub prompt: Option<String>,
    pub error_title: Option<String>,
    pub error: Option<String>,
    pub formula1: Option<String>,
    pub formula2: Option<String>,
}

impl Sheet {
    /// Saved hidden flags in these active filter bodies are filtered rows.
    /// Criteria are not dynamically reapplied when cell values change.
    pub fn filtered_row_ranges(&self) -> (Vec<(u32, u32)>, bool) {
        if self.filter_mode == Some(false) {
            return (vec![], false);
        }
        let mut ranges = vec![];
        let mut unknown = false;
        let mut add = |filter: &crate::auto_filter::AutoFilter, table: Option<&Table>| {
            let issues = filter
                .issues
                .iter()
                .any(|issue| issue != "missing filter range" || table.is_none());
            if issues
                && (filter.has_criteria()
                    || filter
                        .issues
                        .iter()
                        .any(|issue| issue != "missing filter range"))
            {
                unknown = true;
                return;
            }
            if !filter.has_criteria() {
                return;
            }
            let reference = filter
                .reference
                .or_else(|| table.map(|table| table.reference));
            if let Some(reference) = reference {
                let end = table
                    .and_then(|table| table.totals_row())
                    .map(|total| reference.end.row.min(total.saturating_sub(1)))
                    .unwrap_or(reference.end.row);
                let header_rows = table.map(|table| table.header_row_count).unwrap_or(1);
                if header_rows > 1 {
                    unknown = true;
                    return;
                }
                let first = reference.start.row.saturating_add(header_rows);
                if filter.columns.iter().any(|column| {
                    column.column_id > reference.end.column.saturating_sub(reference.start.column)
                }) {
                    unknown = true;
                } else if first <= end {
                    ranges.push((first, end));
                } else if first > end.saturating_add(1) {
                    unknown = true;
                }
            } else {
                unknown = true;
            }
        };
        if let Some(filter) = &self.auto_filter {
            add(filter, None);
        }
        for table in &self.tables {
            if let Some(filter) = &table.filter {
                add(filter, Some(table));
            }
        }
        unknown |= self.filter_mode == Some(true) && ranges.is_empty();
        (ranges, unknown)
    }

    /// The width entry covering `column`, if the sheet declares one.
    pub fn column_span(&self, column: u32) -> Option<&ColumnSpan> {
        self.columns
            .iter()
            .find(|span| column >= span.first && column <= span.last)
    }

    /// The style Excel applies at `reference`.
    ///
    /// A cell's explicit style wins. Otherwise an intentional row style wins
    /// over the column default. Keeping this resolution here ensures layout,
    /// rendering, and headless reads cannot disagree about formatting.
    pub fn effective_style_index(
        &self,
        reference: CellRef,
        cell_style_index: Option<u32>,
    ) -> Option<u32> {
        let row = self.rows.iter().find(|row| row.index == reference.row);
        self.effective_style_index_in_row(reference, cell_style_index, row)
    }

    /// The same resolution when the caller already has the containing row.
    /// Layout walks rows in order and uses this path to keep style lookup O(1)
    /// per cell on tall sheets.
    pub fn effective_style_index_in_row(
        &self,
        reference: CellRef,
        cell_style_index: Option<u32>,
        row: Option<&Row>,
    ) -> Option<u32> {
        cell_style_index
            .or_else(|| {
                row.filter(|row| row.custom_format)
                    .and_then(|row| row.style_index)
            })
            .or_else(|| {
                self.column_span(reference.column)
                    .and_then(|column| column.style_index)
            })
    }

    /// The merged range `cell` belongs to, if any.
    pub fn merge_at(&self, cell: CellRef) -> Option<&CellRange> {
        self.merges.iter().find(|range| range.contains(cell))
    }

    /// The hyperlink applying to `cell`, including links whose `ref` covers a
    /// range rather than a single cell.
    pub fn hyperlink_at(&self, cell: CellRef) -> Option<&Hyperlink> {
        self.hyperlinks
            .iter()
            .find(|link| link.reference.contains(cell))
    }

    /// The legacy comment attached to `cell`, if any.
    pub fn comment_at(&self, cell: CellRef) -> Option<&CellComment> {
        self.comments
            .iter()
            .find(|comment| comment.reference == cell)
    }

    /// The first standard input rule applying to `cell`, if any.
    pub fn data_validation_at(&self, cell: CellRef) -> Option<&DataValidation> {
        self.data_validations
            .iter()
            .find(|validation| validation.ranges.iter().any(|range| range.contains(cell)))
    }

    /// The table `cell` is inside, if any. Excel does not let tables overlap,
    /// so the first match is the only one.
    pub fn table_at(&self, cell: CellRef) -> Option<&Table> {
        self.tables
            .iter()
            .find(|table| table.reference.contains(cell))
    }

    /// The table called `name` (its `name` or `displayName`, compared the
    /// way Excel compares them, without case).
    pub fn table_named(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|table| {
            table.name.eq_ignore_ascii_case(name) || table.display_name.eq_ignore_ascii_case(name)
        })
    }

    /// The shape called `name`, or whose id `name` spells.
    pub fn shape_named(&self, name: &str) -> Option<&Shape> {
        self.shapes
            .iter()
            .find(|shape| shape.name == name)
            .or_else(|| {
                let id = name.trim().parse::<u32>().ok()?;
                self.shapes.iter().find(|shape| shape.id == id)
            })
    }

    /// One past the highest column index any row writes a cell in.
    pub fn used_column_count(&self) -> u32 {
        self.rows
            .iter()
            .filter_map(|row| row.cells.last().map(|cell| cell.reference.column + 1))
            .max()
            .unwrap_or(0)
    }

    /// One past the highest row index the sheet writes.
    pub fn used_row_count(&self) -> u32 {
        self.rows.last().map(|row| row.index + 1).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Color {
    /// `rgb="FFRRGGBB"`, kept as ARGB in file order.
    Argb([u8; 4]),
    /// `indexed="N"` — a position in the legacy palette.
    Indexed(u32),
    /// `theme="N"` with an optional `tint`. The semantic reference is kept,
    /// while `resolved` carries the base RGB from theme1.xml when available.
    Theme {
        index: u32,
        tint: f32,
        #[serde(default)]
        resolved: Option<[u8; 3]>,
    },
    Auto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Font {
    pub name: Option<String>,
    pub size_points: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Fill {
    /// `none`, `solid`, `gray125`, ...
    pub pattern: Option<String>,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BorderEdge {
    /// `thin`, `medium`, `dashed`, ... as written.
    pub style: String,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Border {
    pub left: Option<BorderEdge>,
    pub right: Option<BorderEdge>,
    pub top: Option<BorderEdge>,
    pub bottom: Option<BorderEdge>,
    /// The shared style/color for one or both diagonal directions.
    #[serde(default)]
    pub diagonal: Option<BorderEdge>,
    /// Bottom-left to top-right.
    #[serde(default)]
    pub diagonal_up: bool,
    /// Top-left to bottom-right.
    #[serde(default)]
    pub diagonal_down: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HorizontalAlignment {
    /// Excel's default: text left, numbers right.
    #[default]
    General,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VerticalAlignment {
    Top,
    Center,
    #[default]
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Alignment {
    pub horizontal: HorizontalAlignment,
    pub vertical: VerticalAlignment,
    pub wrap_text: bool,
    #[serde(default)]
    pub shrink_to_fit: bool,
    /// Excel indentation level. Each increment is three spaces in the Normal
    /// style's font.
    #[serde(default)]
    pub indent: u32,
    /// SpreadsheetML text rotation: 0-90 is above the horizon, 91-180
    /// represents 1-90 degrees below it, and 255 is vertically stacked text.
    #[serde(default)]
    pub text_rotation: u16,
}

/// One `<xf>` in `cellXfs`: indexes into the font/fill/border tables plus the
/// number format id.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CellFormat {
    pub font_index: Option<u32>,
    pub fill_index: Option<u32>,
    pub border_index: Option<u32>,
    pub number_format_id: Option<u32>,
    pub alignment: Alignment,
    /// `<protection locked="0">`: the cell stays editable on a protected
    /// sheet. Cells are locked by default.
    #[serde(default)]
    pub unlocked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StyleTable {
    pub fonts: Vec<Font>,
    pub fills: Vec<Fill>,
    pub borders: Vec<Border>,
    pub cell_formats: Vec<CellFormat>,
    /// Named-style formats referenced by cellStyles/@xfId.
    #[serde(default)]
    pub style_formats: Vec<CellFormat>,
    /// The built-in Normal style (builtinId=0), independent of its localized name.
    #[serde(default)]
    pub normal_style_index: Option<u32>,
    #[serde(default)]
    pub differential_styles: Vec<crate::conditional_format::DifferentialStyle>,
    /// Custom `numFmt` codes by id. Built-in ids (0-49) are not in the file and
    /// are not in this map.
    pub number_formats: HashMap<u32, String>,
    /// The theme's twelve colours (`lt1`, `dk1`, `lt2`, `dk2`, accents,
    /// links) in `theme="N"` order, when the workbook has a theme.
    #[serde(default)]
    pub theme_colors: Vec<Option<[u8; 3]>>,
    /// Custom table styles (`<tableStyles>`); built-in ones are not stored.
    #[serde(default)]
    pub table_styles: Vec<TableStyleDefinition>,
}

/// A custom table style: which differential format each part of a table
/// takes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TableStyleDefinition {
    pub name: String,
    pub elements: Vec<TableStyleElement>,
}

/// `<tableStyleElement type dxfId size>`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TableStyleElement {
    /// `wholeTable`, `headerRow`, `firstRowStripe`...
    pub kind: String,
    pub differential_style_index: Option<u32>,
    /// How many rows or columns a stripe spans.
    pub size: u32,
}

impl StyleTable {
    pub fn normal_font(&self) -> Option<&Font> {
        self.normal_style_index
            .and_then(|index| self.style_formats.get(index as usize))
            .and_then(|format| self.fonts.get(format.font_index.unwrap_or(0) as usize))
            .or_else(|| self.fonts.first())
    }

    pub fn cell_format(&self, index: Option<u32>) -> Option<&CellFormat> {
        self.cell_formats.get(index? as usize)
    }

    pub fn font(&self, format: &CellFormat) -> Option<&Font> {
        self.fonts.get(format.font_index? as usize)
    }

    pub fn fill(&self, format: &CellFormat) -> Option<&Fill> {
        self.fills.get(format.fill_index? as usize)
    }

    pub fn border(&self, format: &CellFormat) -> Option<&Border> {
        self.borders.get(format.border_index? as usize)
    }

    /// The format code for a cell format, looking through the custom map first
    /// and then the built-in ids this crate knows.
    pub fn number_format_code(&self, format: &CellFormat) -> Option<&str> {
        let id = format.number_format_id?;
        if let Some(code) = self.number_formats.get(&id) {
            return Some(code.as_str());
        }
        builtin_number_format(id)
    }
}

/// The subset of the built-in number formats (ECMA-376 §18.8.30) that changes
/// what a cell reads as. Ids outside this list fall back to General.
pub fn builtin_number_format(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "$#,##0;($#,##0)",
        6 => "$#,##0;[Red]($#,##0)",
        7 => "$#,##0.00;($#,##0.00)",
        8 => "$#,##0.00;[Red]($#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        41 => "#,##0;(#,##0);-;@",
        42 => "$#,##0;($#,##0);$-;@",
        43 => "#,##0.00;(#,##0.00);-;@",
        44 => "$#,##0.00;($#,##0.00);$-;@",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// Which epoch a workbook's date serials count from.
///
/// Excel for Mac shipped with 1904 and the setting survives in files today, so
/// reading it wrong is a four-year error rather than a rounding one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DateSystem {
    /// Serial 1 is 1900-01-01, with Lotus's phantom 29 February 1900.
    #[default]
    Nineteen00,
    /// Serial 0 is 1904-01-01.
    Nineteen04,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub styles: StyleTable,
    /// Workbook- and sheet-scoped names from `workbook.xml`.
    #[serde(default)]
    pub defined_names: Vec<DefinedName>,
    /// From `workbookPr/@date1904`.
    pub date_system: DateSystem,
    /// Saved `workbookView/@activeTab`, in workbook order.
    #[serde(default)]
    pub active_sheet_index: Option<usize>,
    /// `calcPr/@fullCalcOnLoad`; cached formula results are unsafe when true.
    #[serde(default)]
    pub full_calculation_on_load: bool,
    #[serde(default)]
    pub calculation_mode: CalculationMode,
    #[serde(default = "default_calculate_on_save")]
    pub calculate_on_save: bool,
}

fn default_calculate_on_save() -> bool {
    true
}

impl Default for Workbook {
    fn default() -> Self {
        Self {
            sheets: Vec::new(),
            styles: StyleTable::default(),
            defined_names: Vec::new(),
            date_system: DateSystem::default(),
            active_sheet_index: None,
            full_calculation_on_load: false,
            calculation_mode: CalculationMode::default(),
            calculate_on_save: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CalculationMode {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "autoNoTable")]
    AutoNoTable,
}

impl CalculationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::AutoNoTable => "autoNoTable",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "manual" => Some(Self::Manual),
            "autoNoTable" => Some(Self::AutoNoTable),
            _ => None,
        }
    }
    pub fn calculates_on_edit(self) -> bool {
        self != Self::Manual
    }
}

/// A `<definedName>` exactly as the workbook declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinedName {
    pub name: String,
    /// The range, constant, or formula behind the name, without a leading `=`.
    pub formula: String,
    /// `localSheetId`, an index into workbook tab order. `None` is global.
    pub local_sheet_index: Option<usize>,
    pub hidden: bool,
}

impl Workbook {
    pub fn sheet_by_name(&self, name: &str) -> Option<&Sheet> {
        self.sheets.iter().find(|sheet| sheet.name == name)
    }
}
