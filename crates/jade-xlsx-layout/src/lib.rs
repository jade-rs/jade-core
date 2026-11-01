//! Grid geometry for a worksheet: where every column, row and cell sits, and
//! what text each cell shows.
//!
//! Kept separate from rendering for the same reason `jade-docx-layout` is: the
//! geometry is what a test can assert exactly, and it must be identical
//! whichever backend paints it.
//!
//! Wrapping goes through `jade-text-layout`, which owns the script-level rules
//! about where a line may break — the same table Word's engine reads. What it
//! does *not* reuse is Word's wrap engine itself: that carries drop caps, tab
//! stops, float exclusions and per-document compatibility toggles a workbook
//! cannot express, so a cell run through it would inherit decisions Excel does
//! not make.

pub mod chart;
pub mod color;
pub mod conditional_format;
pub mod filter_button;
pub mod measure;
pub mod navigation;
pub mod number_format;
pub mod outline_gutter;
pub mod pivot_fit;
pub mod pivot_style;
pub mod resize;
pub mod selection;
pub mod shape_style;
pub mod sparkline;
pub mod table_style;
pub mod units;
mod visual_format;

use jade_text_layout::{wrap_text, WrapOptions};
use jade_xlsx_reader::{
    Alignment, Border, CellRange, CellRef, CellValue, Color, Font, HorizontalAlignment, Row,
    ShapeKind, Sheet, StyleTable, VerticalAlignment, EMU_PER_PX,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

pub use measure::{HeuristicMeasure, TextMeasure};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// Whether the two rectangles share any area. Edge contact does not count,
    /// so a cell that ends exactly where the viewport begins is not drawn.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && other.width > 0.0
            && other.height > 0.0
            && self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

/// The columns and rows a viewport touches, as inclusive index ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleRange {
    pub first_column: u32,
    pub last_column: u32,
    pub first_row: u32,
    pub last_row: u32,
}

/// One column or row: where it starts and how tall or wide it is, in px.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisEntry {
    /// Zero-based column or row index.
    pub index: u32,
    pub offset: f64,
    pub size: f64,
    pub hidden: bool,
}

/// The font a cell draws in, after the style tables are resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedFont {
    pub family: String,
    pub size_points: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// `superscript` or `subscript`.
    #[serde(default)]
    pub vert_align: Option<String>,
}

impl Default for ResolvedFont {
    fn default() -> Self {
        Self {
            family: "Calibri".to_string(),
            size_points: 11.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            vert_align: None,
        }
    }
}

/// A SpreadsheetML cell fill after its style reference is resolved.
/// Keeping this in layout makes browser and native painters consume the same
/// pattern and colours.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedFill {
    /// The pattern, or `gradient` for a gradient fill.
    pub pattern: String,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<jade_xlsx_reader::GradientFill>,
}

/// A cell placed on the grid, with everything a painter needs and nothing it
/// would have to look up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedCell {
    pub reference: CellRef,
    /// The rectangle to paint, already covering a merged range if there is one.
    pub rect: Rect,
    /// Text may extend into empty neighbours without changing selection or fill geometry.
    #[serde(default)]
    pub text_clip: Option<Rect>,
    /// The merged range this cell anchors, if any.
    pub merge: Option<CellRange>,
    /// The text to draw, after number formatting.
    pub text: String,
    /// Lines prepared for drawing, including wrapping and width-dependent
    /// numeric representation. `text` retains the complete formatted value.
    pub lines: Vec<String>,
    pub font: ResolvedFont,
    pub text_color: Option<Color>,
    pub fill: Option<ResolvedFill>,
    #[serde(default)]
    pub data_bar: Option<PlacedDataBar>,
    /// An icon-set icon drawn at the cell's left edge.
    #[serde(default)]
    pub icon: Option<PlacedIcon>,
    pub border: Border,
    /// Alignment with `General` already resolved against the value's type.
    pub alignment: Alignment,
    /// Horizontal inset resolved from the alignment indent, in px. Renderers
    /// consume this instead of independently measuring Excel's three spaces.
    #[serde(default)]
    pub text_inset_px: f64,
    /// Rich text: the runs of `text`, each with its own font. Empty for a
    /// cell drawn in one font.
    #[serde(default)]
    pub runs: Vec<PlacedRun>,
    /// Excel's error checking flags the cell — a number stored as text, or
    /// a formula that gives an error — with a green corner triangle.
    #[serde(default)]
    pub error_flag: bool,
}

/// A run of a rich text cell: a byte range of the cell's text and its font.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedRun {
    pub start: usize,
    pub end: usize,
    pub font: ResolvedFont,
    /// The run's own colour; `None` takes the cell's.
    pub color: Option<Color>,
}

/// A conditional-format icon, placed: which icon of which set, and its box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedIcon {
    /// The set's name as the file spells it: `3Arrows`, `5Quarters`...
    pub icon_set: String,
    /// The icon's position in the set, 0 being the one for the lowest values.
    pub index: u32,
    pub rect: Rect,
    /// False when the rule hides the cell's value and shows only the icon.
    pub show_value: bool,
}

/// A conditional data bar with geometry already resolved by shared layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedDataBar {
    pub rect: Rect,
    pub color: [u8; 4],
    pub show_value: bool,
    #[serde(default = "default_bar_gradient")]
    pub gradient: bool,
    #[serde(default)]
    pub reverse: bool,
    #[serde(default)]
    pub border: Option<[u8; 4]>,
    #[serde(default)]
    pub axis: Option<PlacedDataBarAxis>,
}
fn default_bar_gradient() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedDataBarAxis {
    pub rect: Rect,
    pub color: [u8; 4],
}

impl PlacedCell {
    pub fn text_rect(&self) -> Rect {
        self.text_clip.unwrap_or(self.rect)
    }
}

/// A drawing object placed over the grid: a text box, a shape, or the frame
/// a picture or chart occupies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedShape {
    pub id: u32,
    pub name: String,
    pub kind: ShapeKind,
    /// Where it sits, in sheet px, from its anchor cells and their offsets.
    pub rect: Rect,
    #[serde(default)]
    pub picture: Option<PlacedPicture>,
    #[serde(default)]
    pub chart: Option<chart::PlacedChart>,
    /// The shape's text broken into lines that fit its width, for a shape
    /// that has a text body.
    pub lines: Vec<String>,
    pub font: ResolvedFont,
    /// A shape's geometry, fill, outline and text placement; `None` for a
    /// picture, chart or group.
    #[serde(default)]
    pub style: Option<shape_style::PlacedShapeStyle>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedPicture {
    pub media: Option<jade_xlsx_reader::model::MediaPart>,
    pub source: [f64; 4],
    /// Image rectangle in the shape's local coordinates, after clipping any
    /// negative crop outsets to the available source pixels.
    pub image_rect: Rect,
    pub rotation_radians: f64,
    pub flip_h: bool,
    pub flip_v: bool,
    pub opacity: f64,
    pub supported: bool,
}

impl PlacedShape {
    pub fn paint_rect(&self) -> Rect {
        let Some(picture) = self.picture.as_ref() else {
            return self.rect;
        };
        let (sin, cos) = picture.rotation_radians.sin_cos();
        let width = self.rect.width * cos.abs() + self.rect.height * sin.abs();
        let height = self.rect.width * sin.abs() + self.rect.height * cos.abs();
        Rect {
            x: self.rect.x + (self.rect.width - width) / 2.0,
            y: self.rect.y + (self.rect.height - height) / 2.0,
            width,
            height,
        }
    }

    fn contains_point(&self, x: f64, y: f64) -> bool {
        if let Some(picture) = self.picture.as_ref() {
            let x = x - self.rect.x - self.rect.width / 2.0;
            let y = y - self.rect.y - self.rect.height / 2.0;
            let (sin, cos) = picture.rotation_radians.sin_cos();
            let local_x = x * cos + y * sin + self.rect.width / 2.0;
            let local_y = -x * sin + y * cos + self.rect.height / 2.0;
            0.0 <= local_x
                && local_x < self.rect.width
                && 0.0 <= local_y
                && local_y < self.rect.height
        } else {
            contains(&self.rect, x, y)
        }
    }
}

/// A cell that carries a note (a legacy comment), placed on the grid.
///
/// Excel marks such a cell with a small red triangle in its top-right
/// corner, and marks it whether or not the cell holds anything — so this is
/// its own list rather than a flag on [`PlacedCell`], which exists only for
/// cells the file writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedNote {
    pub reference: CellRef,
    /// The cell's rectangle, merges resolved, in sheet px.
    pub rect: Rect,
    /// The note's text, so a shell can show it without going back to the
    /// model.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlankCellStyle {
    pub font: ResolvedFont,
    pub text_color: Option<Color>,
    pub fill: Option<ResolvedFill>,
    pub border: Border,
    pub alignment: Alignment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlankColumnStyle {
    pub first: u32,
    pub last: u32,
    pub style: BlankCellStyle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetLayout {
    pub columns: Vec<AxisEntry>,
    pub rows: Vec<AxisEntry>,
    pub cells: Vec<PlacedCell>,
    #[serde(default)]
    pub filter_buttons: Vec<filter_button::FilterButton>,
    /// Rules already evaluated for an unstored blank value. Windowed painting
    /// materializes their blank cells only inside the requested viewport.
    #[serde(default)]
    pub blank_conditional_rules: Vec<conditional_format::BlankConditionalRule>,
    #[serde(default)]
    pub blank_row_styles: BTreeMap<u32, BlankCellStyle>,
    #[serde(default)]
    pub blank_column_styles: Vec<BlankColumnStyle>,
    /// Drawing objects, in the drawing's own order (back to front).
    #[serde(default)]
    pub shapes: Vec<PlacedShape>,
    /// Cells carrying a note, so a painter can mark them.
    #[serde(default)]
    pub notes: Vec<PlacedNote>,
    /// Tables with a table style, which paints their unstored cells too.
    #[serde(default)]
    pub tables: Vec<table_style::StyledTable>,
    /// Pivot style geometry, including unstored cells inside saved reports.
    #[serde(default)]
    pub pivots: Vec<pivot_style::StyledPivot>,
    /// Sparklines, each in its cell.
    #[serde(default)]
    pub sparklines: Vec<sparkline::PlacedSparkline>,
    pub show_grid_lines: bool,
    pub content_width: f64,
    /// Character-unit conversion used to create these axes.
    #[serde(default = "default_digit_width")]
    pub max_digit_width_px: f64,
    pub content_height: f64,
    #[serde(default)]
    pub frozen_columns: u32,
    #[serde(default)]
    pub frozen_rows: u32,
    /// Gap between a cell's edge and its text, in px. Wrapping measured
    /// against it, so the renderer has to place text against the same value —
    /// hence it lives on the result rather than in both option structs.
    pub cell_padding_px: f64,
    /// Rows and columns grouped, for the outline gutter.
    #[serde(default)]
    pub outline: jade_xlsx_reader::Outline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportRegions {
    pub main: Rect,
    pub frozen_rows: Rect,
    pub frozen_columns: Rect,
    pub corner: Rect,
    pub frozen_width: f64,
    pub frozen_height: f64,
}

fn default_digit_width() -> f64 {
    units::DEFAULT_MAX_DIGIT_WIDTH_PX
}

impl SheetLayout {
    pub fn column_at(&self, x: f64) -> Option<u32> {
        axis_at(&self.columns, x)
    }
    pub fn row_at(&self, y: f64) -> Option<u32> {
        axis_at(&self.rows, y)
    }
    pub fn conditional_blank_cells_in(&self, viewport: Rect) -> Vec<PlacedCell> {
        self.blank_cells_in(viewport)
    }

    /// Resolves default axis styles and conditional rules only for visible
    /// unstored cells. Row defaults override column defaults as in the reader.
    pub fn blank_cells_in(&self, viewport: Rect) -> Vec<PlacedCell> {
        let Some(range) = self.visible_range(viewport) else {
            return Vec::new();
        };
        if self.blank_conditional_rules.is_empty()
            && self.blank_row_styles.is_empty()
            && self.blank_column_styles.is_empty()
            && self.tables.is_empty()
            && self.pivots.is_empty()
        {
            return Vec::new();
        }
        let occupied: HashSet<CellRef> = self
            .cells
            .iter()
            .filter(|cell| cell.rect.intersects(&viewport))
            .map(|cell| cell.reference)
            .collect();
        let merges: Vec<_> = self.cells.iter().filter_map(|cell| cell.merge).collect();
        let mut cells = Vec::new();
        for row in range.first_row..=range.last_row {
            for column in range.first_column..=range.last_column {
                let reference = CellRef::new(column, row);
                if occupied.contains(&reference)
                    || merges.iter().any(|range| range.contains(reference))
                {
                    continue;
                }
                let axis_style = self.blank_row_styles.get(&row).or_else(|| {
                    self.blank_column_styles
                        .iter()
                        .find(|span| column >= span.first && column <= span.last)
                        .map(|span| &span.style)
                });
                let style =
                    conditional_format::blank_style(&self.blank_conditional_rules, reference);
                let table = self
                    .tables
                    .iter()
                    .find_map(|table| table.style_at(reference))
                    .or_else(|| self.pivots.iter().find_map(|p| p.style_at(reference)));
                let fill = conditional_format::conditional_fill(&style).unwrap_or_else(|| {
                    axis_style
                        .and_then(|style| style.fill.clone())
                        .or_else(|| table.as_ref().and_then(|table| table.fill.clone()))
                });
                let mut border = axis_style
                    .map(|style| style.border.clone())
                    .unwrap_or_default();
                if let Some(table) = &table {
                    overlay_table_border(&mut border, table);
                }
                conditional_format::apply_border(&style, &mut border);
                if axis_style.is_none()
                    && table.is_none()
                    && style == jade_xlsx_reader::conditional_format::DifferentialStyle::default()
                {
                    continue;
                }
                let column = &self.columns[column as usize];
                let row = &self.rows[row as usize];
                let rect = Rect {
                    x: column.offset,
                    y: row.offset,
                    width: column.size,
                    height: row.size,
                };
                if rect.width <= 0.0 || rect.height <= 0.0 {
                    continue;
                }
                let mut font = axis_style
                    .map(|style| style.font.clone())
                    .unwrap_or_default();
                if let Some(table) = &table {
                    font.bold |= table.bold;
                    font.italic |= table.italic;
                    font.underline |= table.underline;
                    font.strike |= table.strike;
                }
                conditional_format::apply_font(&style, &mut font);
                cells.push(PlacedCell {
                    reference,
                    rect,
                    text_clip: None,
                    merge: None,
                    text: String::new(),
                    lines: vec![String::new()],
                    font,
                    text_color: style
                        .text_color
                        .or_else(|| axis_style.and_then(|style| style.text_color))
                        .or_else(|| table.as_ref().and_then(|t| t.text_color)),
                    fill,
                    border,
                    alignment: axis_style.map(|style| style.alignment).unwrap_or_default(),
                    data_bar: None,
                    icon: None,
                    text_inset_px: 0.0,
                    runs: Vec::new(),
                    error_flag: false,
                });
            }
        }
        cells
    }
    pub fn column(&self, index: u32) -> Option<&AxisEntry> {
        self.columns.get(index as usize)
    }

    pub fn row(&self, index: u32) -> Option<&AxisEntry> {
        self.rows.get(index as usize)
    }

    pub fn cell(&self, reference: CellRef) -> Option<&PlacedCell> {
        self.cells.iter().find(|cell| cell.reference == reference)
    }

    /// Includes viewport-only conditional formatting for an unstored cell.
    pub fn styled_cell(&self, reference: CellRef) -> Option<Cow<'_, PlacedCell>> {
        if let Some(cell) = self.cell(reference) {
            return Some(Cow::Borrowed(cell));
        }
        let rect = self.cell_rect(reference)?;
        self.blank_cells_in(rect)
            .into_iter()
            .find(|cell| cell.reference == reference)
            .map(Cow::Owned)
    }

    /// Geometry for stored and unstored cells, including blank merges.
    pub fn cell_rect(&self, reference: CellRef) -> Option<Rect> {
        if let Some(cell) = self.cell(reference) {
            return Some(cell.rect);
        }
        let column = self.column(reference.column)?;
        let row = self.row(reference.row)?;
        Some(Rect {
            x: column.offset,
            y: row.offset,
            width: column.size,
            height: row.size,
        })
    }

    /// The cell's body-surface rectangle; frozen axes retain their position.
    pub fn cell_rect_in_viewport(
        &self,
        reference: CellRef,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Option<Rect> {
        let rect = self.cell_rect(reference)?;
        Some(Rect {
            x: rect.x
                - if reference.column < self.frozen_columns {
                    0.0
                } else {
                    scroll_x.max(0.0)
                },
            y: rect.y
                - if reference.row < self.frozen_rows {
                    0.0
                } else {
                    scroll_y.max(0.0)
                },
            ..rect
        })
    }

    /// Minimum pixel-aligned scroll to show a cell in the scrollable pane. A merge
    /// wider than the pane keeps its leading edge visible without oscillation.
    pub fn scroll_to_reveal(&self, reference: CellRef, viewport: Rect) -> (f64, f64) {
        let Some(rect) = self.cell_rect(reference) else {
            return (viewport.x, viewport.y);
        };
        let reveal = |start: f64,
                      size: f64,
                      frozen: f64,
                      window: f64,
                      scroll: f64,
                      content: f64,
                      fixed: bool| {
            let max_scroll = (content - window).max(0.0);
            let mut scroll = scroll.clamp(0.0, max_scroll);
            if !fixed && window > frozen {
                if start < frozen + scroll || (size > window - frozen && start >= window + scroll) {
                    scroll = (start - frozen).floor();
                } else if size <= window - frozen && start + size > window + scroll {
                    scroll = (start + size - window).ceil();
                }
            }
            scroll.clamp(0.0, max_scroll)
        };
        (
            reveal(
                rect.x,
                rect.width,
                self.frozen_width(),
                viewport.width,
                viewport.x,
                self.content_width,
                reference.column < self.frozen_columns,
            ),
            reveal(
                rect.y,
                rect.height,
                self.frozen_height(),
                viewport.height,
                viewport.y,
                self.content_height,
                reference.row < self.frozen_rows,
            ),
        )
    }

    pub fn frozen_width(&self) -> f64 {
        self.columns
            .iter()
            .take(self.frozen_columns as usize)
            .map(|entry| entry.size)
            .sum()
    }

    pub fn frozen_height(&self) -> f64 {
        self.rows
            .iter()
            .take(self.frozen_rows as usize)
            .map(|entry| entry.size)
            .sum()
    }

    /// The four non-overlapping sheet-coordinate regions visible in a grid
    /// viewport. Scroll offsets apply only beyond the fixed row/column bands.
    pub fn viewport_regions(&self, viewport: Rect) -> ViewportRegions {
        let frozen_width = self.frozen_width().min(viewport.width.max(0.0));
        let frozen_height = self.frozen_height().min(viewport.height.max(0.0));
        let main_width = (viewport.width - frozen_width).max(0.0);
        let main_height = (viewport.height - frozen_height).max(0.0);
        let sheet_frozen_width = self.frozen_width();
        let sheet_frozen_height = self.frozen_height();
        ViewportRegions {
            main: Rect {
                x: sheet_frozen_width + viewport.x.max(0.0),
                y: sheet_frozen_height + viewport.y.max(0.0),
                width: main_width,
                height: main_height,
            },
            frozen_rows: Rect {
                x: sheet_frozen_width + viewport.x.max(0.0),
                y: 0.0,
                width: main_width,
                height: frozen_height,
            },
            frozen_columns: Rect {
                x: 0.0,
                y: sheet_frozen_height + viewport.y.max(0.0),
                width: frozen_width,
                height: main_height,
            },
            corner: Rect {
                x: 0.0,
                y: 0.0,
                width: frozen_width,
                height: frozen_height,
            },
            frozen_width,
            frozen_height,
        }
    }

    /// Converts a point in the body surface into the sheet coordinate shown
    /// there, respecting fixed rows and columns.
    pub fn sheet_point_in_viewport(
        &self,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> (f64, f64) {
        let sheet_x = if x < self.frozen_width() {
            x
        } else {
            x + scroll_x.max(0.0)
        };
        let sheet_y = if y < self.frozen_height() {
            y
        } else {
            y + scroll_y.max(0.0)
        };
        (sheet_x, sheet_y)
    }

    /// The columns and rows `viewport` touches, or `None` when it misses the
    /// grid entirely.
    ///
    /// Both axes are sorted by offset, so this is two binary searches rather
    /// than a scan — the difference between drawing a screenful and walking a
    /// hundred thousand rows to find where to start.
    pub fn visible_range(&self, viewport: Rect) -> Option<VisibleRange> {
        let (first_column, last_column) = axis_range(&self.columns, viewport.x, viewport.right())?;
        let (first_row, last_row) = axis_range(&self.rows, viewport.y, viewport.bottom())?;
        Some(VisibleRange {
            first_column,
            last_column,
            first_row,
            last_row,
        })
    }

    /// The cell at a point in sheet coordinates.
    ///
    /// A point inside a merged range gives the range's anchor, which is the
    /// cell Excel selects when you click anywhere in it.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<CellRef> {
        if let Some(merged) = self
            .cells
            .iter()
            .find(|cell| cell.merge.is_some() && contains(&cell.rect, x, y))
        {
            return Some(merged.reference);
        }
        let column = axis_at(&self.columns, x)?;
        let row = axis_at(&self.rows, y)?;
        Some(CellRef::new(column, row))
    }

    /// The size of the row and column header bands for this sheet.
    pub fn header_metrics(&self) -> HeaderMetrics {
        HeaderMetrics::for_rows(self.rows.len() as u32).with_outline(
            self.outline_levels(jade_xlsx_reader::Axis::Rows),
            self.outline_levels(jade_xlsx_reader::Axis::Columns),
        )
    }

    /// The cells that overlap `viewport`.
    ///
    /// Filtered by rectangle rather than by the index range, because a merged
    /// cell anchored above or to the left of the viewport still shows inside
    /// it, and an index range would drop exactly those.
    pub fn cells_in(&self, viewport: Rect) -> impl Iterator<Item = &PlacedCell> {
        self.cells
            .iter()
            .filter(move |cell| cell.rect.intersects(&viewport))
    }

    /// Includes overflow from cells whose own box is outside the window.
    pub fn text_cells_in(&self, viewport: Rect) -> impl Iterator<Item = &PlacedCell> {
        self.cells.iter().filter(move |cell| {
            cell.rect.width > 0.0
                && cell.rect.height > 0.0
                && cell.text_rect().intersects(&viewport)
        })
    }

    /// The drawing objects that overlap `viewport`.
    pub fn shapes_in(&self, viewport: Rect) -> impl Iterator<Item = &PlacedShape> {
        self.shapes
            .iter()
            .filter(move |shape| shape.paint_rect().intersects(&viewport))
    }

    /// The noted cells that overlap `viewport`.
    pub fn notes_in(&self, viewport: Rect) -> impl Iterator<Item = &PlacedNote> {
        self.notes
            .iter()
            .filter(move |note| note.rect.intersects(&viewport))
    }

    /// The drawing object at a point in sheet coordinates.
    ///
    /// The drawing is painted back to front, so a click lands on the last
    /// object covering the point — the one on top — which is the one Excel
    /// selects. A drawing object sits over the grid, so a hit here means the
    /// click was not a cell click at all.
    pub fn shape_at(&self, x: f64, y: f64) -> Option<&PlacedShape> {
        self.shapes
            .iter()
            .rev()
            .find(|shape| shape.contains_point(x, y))
    }
}

/// The band a spreadsheet puts its column letters and row numbers in.
///
/// The row band widens with the number of digits it has to show, which is what
/// stops a sheet used down to row 10000 from clipping its own numbers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeaderMetrics {
    /// Width of the row-number band, in px, the outline gutter included.
    pub row_width: f64,
    /// Height of the column-letter band, in px, the outline gutter included.
    pub column_height: f64,
    /// Width of the rows' outline gutter at the band's left, in px; 0 when
    /// no rows are grouped.
    #[serde(default)]
    pub outline_width: f64,
    /// Height of the columns' outline gutter at the band's top, in px.
    #[serde(default)]
    pub outline_height: f64,
}

impl HeaderMetrics {
    /// Sized for a sheet with `row_count` rows.
    pub fn for_rows(row_count: u32) -> Self {
        let digits = row_count.max(1).to_string().len().max(2) as f64;
        Self {
            // Two digits fit in Excel's own band; each further digit widens it.
            row_width: 24.0 + (digits - 2.0) * 8.0,
            column_height: units::points_to_px(units::DEFAULT_ROW_HEIGHT_POINTS),
            outline_width: 0.0,
            outline_height: 0.0,
        }
    }

    /// With the outline gutters rows grouped `row_levels` deep and columns
    /// `column_levels` deep grow.
    pub fn with_outline(mut self, row_levels: u8, column_levels: u8) -> Self {
        self.outline_width = outline_gutter::gutter_size(row_levels);
        self.outline_height = outline_gutter::gutter_size(column_levels);
        self.row_width += self.outline_width;
        self.column_height += self.outline_height;
        self
    }
}

impl Default for HeaderMetrics {
    fn default() -> Self {
        Self::for_rows(1)
    }
}

/// A spreadsheet column's name: `A`, `Z`, `AA`, `AB`.
///
/// Base 26 with no zero, which is why it is not just a radix conversion: after
/// `Z` comes `AA`, not `BA`.
pub fn column_label(index: u32) -> String {
    let mut label = Vec::new();
    let mut remaining = index as u64 + 1;
    while remaining > 0 {
        let digit = ((remaining - 1) % 26) as u8;
        label.push(b'A' + digit);
        remaining = (remaining - 1) / 26;
    }
    label.reverse();
    String::from_utf8(label).expect("ASCII")
}

fn contains(rect: &Rect, x: f64, y: f64) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}

/// The index of the entry covering `position`, or `None` past the end.
fn axis_at(entries: &[AxisEntry], position: f64) -> Option<u32> {
    if position < 0.0 {
        return None;
    }
    let index = entries.partition_point(|entry| entry.offset + entry.size <= position);
    (index < entries.len()).then_some(index as u32)
}

/// The inclusive index range of `entries` overlapping `[start, end)`.
fn axis_range(entries: &[AxisEntry], start: f64, end: f64) -> Option<(u32, u32)> {
    if entries.is_empty() || end <= start {
        return None;
    }
    // Entries are laid end to end, so `offset + size` is non-decreasing and
    // `offset` is too — both are partitioned by the comparisons below.
    let first = entries.partition_point(|entry| entry.offset + entry.size <= start);
    let last = entries.partition_point(|entry| entry.offset < end);
    if first >= entries.len() || last == 0 || first >= last {
        return None;
    }
    Some((first as u32, (last - 1) as u32))
}

#[derive(Clone)]
pub struct LayoutOptions {
    /// Width of a digit in the workbook's default font, in px. The stored
    /// column width unit is defined against it.
    pub max_digit_width_px: f64,
    /// Font used where the style tables do not say otherwise.
    pub default_font: ResolvedFont,
    /// Columns to lay out even if the sheet writes no cells in them, so an
    /// empty grid still fills the viewport.
    pub min_column_count: u32,
    /// Rows to lay out even if the sheet writes none.
    pub min_row_count: u32,
    /// Gap between a cell's edge and its text, in px. Excel leaves about three
    /// pixels at 100%.
    pub cell_padding_px: f64,
    /// Which epoch the workbook's date serials count from.
    pub date_system: jade_xlsx_reader::DateSystem,
    /// The IANA time zone "today" is in for date rules (`TODAY()`, "dates
    /// occurring"); UTC when unset.
    pub time_zone: Option<String>,
    /// Measures text for wrapping. The default is a deterministic stand-in;
    /// a browser passes one backed by the canvas, and the native shell one
    /// backed by Skia, so line ends match what will actually be drawn.
    pub measure: Rc<dyn TextMeasure>,
}

/// Normal is a named style; font-table order does not select its font.
pub fn resolve_normal_font(styles: &StyleTable, fallback: &ResolvedFont) -> ResolvedFont {
    resolve_font(styles.normal_font(), fallback)
}

impl LayoutOptions {
    /// Preserve the known Calibri 11 baseline when that face is unavailable.
    /// Hosts can explicitly measure it after registering/loading the real font.
    pub fn with_normal_font_metrics(self, styles: &StyleTable) -> Self {
        let normal = resolve_normal_font(styles, &self.default_font);
        if normal.family.eq_ignore_ascii_case("Calibri")
            && normal.size_points == 11.0
            && !normal.bold
            && !normal.italic
        {
            self
        } else {
            self.with_measured_normal_font(styles)
        }
    }

    /// The backend measures digits individually, since their advances can differ.
    pub fn with_measured_normal_font(mut self, styles: &StyleTable) -> Self {
        let font = resolve_normal_font(styles, &self.default_font);
        let width = ('0'..='9')
            .map(|digit| self.measure.measure(&digit.to_string(), &font))
            .filter(|width| width.is_finite() && *width > 0.0)
            .fold(0.0, f64::max);
        if width > 0.0 {
            self.max_digit_width_px = width;
        }
        self
    }
}

impl std::fmt::Debug for LayoutOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LayoutOptions")
            .field("max_digit_width_px", &self.max_digit_width_px)
            .field("default_font", &self.default_font)
            .field("min_column_count", &self.min_column_count)
            .field("min_row_count", &self.min_row_count)
            .field("cell_padding_px", &self.cell_padding_px)
            .field("date_system", &self.date_system)
            .finish_non_exhaustive()
    }
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            max_digit_width_px: units::DEFAULT_MAX_DIGIT_WIDTH_PX,
            default_font: ResolvedFont::default(),
            min_column_count: 0,
            min_row_count: 0,
            cell_padding_px: 3.0,
            date_system: jade_xlsx_reader::DateSystem::default(),
            time_zone: None,
            measure: Rc::new(HeuristicMeasure),
        }
    }
}

/// Lays out one worksheet.
///
/// Two passes, because the two axes depend on each other: a wrapped cell's
/// height is only known once its text has been broken against its column's
/// width. Columns are laid out first, cells are wrapped against them, and the
/// rows that Excel is allowed to auto-fit grow to hold the result.
pub fn layout_sheet(sheet: &Sheet, styles: &StyleTable, options: &LayoutOptions) -> SheetLayout {
    layout_sheet_with_workbook(sheet, styles, options, None)
}

/// Includes cross-sheet chart references from the current workbook snapshot.
pub fn layout_workbook_sheet(
    workbook: &jade_xlsx_reader::Workbook,
    index: usize,
    options: &LayoutOptions,
) -> Option<SheetLayout> {
    Some(layout_sheet_with_workbook(
        workbook.sheets.get(index)?,
        &workbook.styles,
        options,
        Some(workbook),
    ))
}

fn layout_sheet_with_workbook(
    sheet: &Sheet,
    styles: &StyleTable,
    options: &LayoutOptions,
    workbook: Option<&jade_xlsx_reader::Workbook>,
) -> SheetLayout {
    layout_sheet_on_axes(sheet, styles, options, workbook, None).0
}

/// What a layout's axes would have been, when it was placed on another
/// layout's instead.
type ComputedAxes = (Vec<AxisEntry>, Vec<AxisEntry>);

/// The layout, its cells placed on `kept`'s columns and rows when given —
/// with the axes this sheet would have had returned beside it, for the
/// caller to check against them.
fn layout_sheet_on_axes(
    sheet: &Sheet,
    styles: &StyleTable,
    options: &LayoutOptions,
    workbook: Option<&jade_xlsx_reader::Workbook>,
    kept: Option<&SheetLayout>,
) -> (SheetLayout, Option<ComputedAxes>) {
    let filter_definitions = filter_button::definitions(sheet);
    let column_count = sheet
        .used_column_count()
        .max(options.min_column_count)
        .max(merge_extent(sheet, Axis::Column))
        .max(shape_extent(sheet, Axis::Column))
        .max(
            sheet
                .sparkline_groups
                .iter()
                .flat_map(|group| &group.sparklines)
                .map(|sparkline| sparkline.cell.column + 1)
                .max()
                .unwrap_or(0),
        )
        .max(
            filter_definitions
                .values()
                .map(|button| button.reference.column + 1)
                .max()
                .unwrap_or(0),
        );
    let row_count = sheet
        .used_row_count()
        .max(options.min_row_count)
        .max(merge_extent(sheet, Axis::Row))
        .max(shape_extent(sheet, Axis::Row))
        .max(
            sheet
                .sparkline_groups
                .iter()
                .flat_map(|group| &group.sparklines)
                .map(|sparkline| sparkline.cell.row + 1)
                .max()
                .unwrap_or(0),
        )
        .max(
            filter_definitions
                .values()
                .map(|button| button.reference.row + 1)
                .max()
                .unwrap_or(0),
        );

    let styled_tables = table_style::styled_tables(sheet, styles);
    let styled_pivots = workbook
        .map(|b| pivot_style::styled_pivots(b, &sheet.part_name))
        .unwrap_or_default();
    let column_count = column_count.max(
        styled_pivots
            .iter()
            .map(|p| p.range.end.column + 1)
            .max()
            .unwrap_or(0),
    );
    let row_count = row_count.max(
        styled_pivots
            .iter()
            .map(|p| p.range.end.row + 1)
            .max()
            .unwrap_or(0),
    );
    let formula_matches =
        formula_rule_matches(sheet, styles, workbook, column_count, row_count, options);
    let conditional =
        conditional_format::ConditionalStyles::with_formula_matches(sheet, styles, formula_matches);
    let mut columns = column_axis(sheet, column_count, options);
    let normal_style_font = resolve_normal_font(styles, &options.default_font);

    let mut prepared =
        Vec::with_capacity(sheet.rows.iter().map(|row| row.cells.len()).sum::<usize>());
    let mut auto_heights = vec![0.0f32; row_count as usize];
    let merges = MergeRows::new(&sheet.merges);
    for row in &sheet.rows {
        let mut blockers: Vec<(u32, u32)> = row
            .cells
            .iter()
            .filter(|cell| !cell.value.is_empty() || cell.formula.is_some())
            .filter(|cell| {
                columns
                    .get(cell.reference.column as usize)
                    .is_some_and(|column| column.size > 0.0)
            })
            .map(|cell| (cell.reference.column, cell.reference.column))
            .chain(
                merges
                    .in_row(row.index)
                    .map(|range| (range.start.column, range.end.column)),
            )
            .collect();
        blockers.extend(
            filter_definitions
                .range((row.index, 0)..=(row.index, u32::MAX))
                .filter(|((_, column), _)| {
                    columns
                        .get(*column as usize)
                        .is_some_and(|column| !column.hidden && column.size > 0.0)
                })
                .map(|((_, column), _)| (*column, *column)),
        );
        blockers.sort_unstable();
        let mut occupied: Vec<(u32, u32)> = Vec::new();
        for (start, end) in blockers {
            if let Some(last) = occupied.last_mut().filter(|last| start <= last.1) {
                last.1 = last.1.max(end);
            } else {
                occupied.push((start, end));
            }
        }
        for cell in &row.cells {
            // A cell inside a merged range that does not anchor it is covered
            // by the anchor and must not paint its own box or text.
            let merge = merges.at(cell.reference).copied();
            if merge.is_some_and(|range| range.start != cell.reference) {
                continue;
            }
            let format = styles.cell_format(sheet.effective_style_index_in_row(
                cell.reference,
                cell.style_index,
                Some(row),
            ));
            let mut font = resolve_font(
                format.and_then(|format| styles.font(format)),
                &options.default_font,
            );
            let table_style = styled_tables
                .iter()
                .find_map(|table| table.style_at(cell.reference))
                .or_else(|| {
                    styled_pivots
                        .iter()
                        .find_map(|p| p.style_at(cell.reference))
                });
            if format.is_none_or(|f| f.apply_font != Some(true)) {
                if let Some(table) = &table_style {
                    font.bold |= table.bold;
                    font.italic |= table.italic;
                    font.underline |= table.underline;
                    font.strike |= table.strike;
                }
            }
            let (conditional_style, visuals) =
                conditional.for_cell_with_visual(cell.reference, &cell.value);
            let data_bar = visuals.bar;
            let icon = visuals.icon;
            conditional_format::apply_font(&conditional_style, &mut font);
            let number_format_code = conditional_style
                .number_format_code
                .as_deref()
                .or_else(|| format.and_then(|format| styles.number_format_code(format)));
            let mut alignment = format.map(|format| format.alignment).unwrap_or_default();
            // Show Formulas: a formula shows as typed, as text does, left.
            let shown_formula = sheet
                .show_formulas
                .then_some(cell.formula.as_deref())
                .flatten()
                .map(formula_as_typed);
            alignment.horizontal = if shown_formula.is_some() {
                resolve_horizontal(alignment.horizontal, &CellValue::Text(String::new()))
            } else {
                resolve_horizontal(alignment.horizontal, &cell.value)
            };
            // Justified and distributed text wraps in Excel whether or not
            // Wrap Text is ticked; layout and painting see it as wrapped.
            if matches!(
                alignment.horizontal,
                HorizontalAlignment::Justify | HorizontalAlignment::Distributed
            ) || matches!(
                alignment.vertical,
                VerticalAlignment::Justify | VerticalAlignment::Distributed
            ) {
                alignment.wrap_text = true;
            }
            let text_inset_px = match alignment.horizontal {
                HorizontalAlignment::Left
                | HorizontalAlignment::Right
                | HorizontalAlignment::Distributed => {
                    options.measure.measure("   ", &normal_style_font) * f64::from(alignment.indent)
                }
                _ => 0.0,
            } + match (&icon, alignment.horizontal) {
                // Left-aligned text starts after the icon, which sits there.
                (Some(_), HorizontalAlignment::Left) => icon_size_px(&font, f64::INFINITY) + 1.0,
                _ => 0.0,
            };
            let formatted = match &shown_formula {
                Some(formula) => number_format::format_value(
                    &CellValue::Text(formula.clone()),
                    Some("@"),
                    options.date_system,
                ),
                None => number_format::format_value(
                    &cell.value,
                    number_format_code,
                    options.date_system,
                ),
            };
            let fill_at = formatted.fill;
            let text = formatted.text;
            // Without wrap text Excel keeps the cell on one line and shows
            // its line breaks as nothing.
            let one_line = if alignment.wrap_text || !text.contains(['\n', '\r']) {
                None
            } else {
                Some(text.replace(['\n', '\r'], ""))
            };
            let shown = one_line.as_deref().unwrap_or(&text);

            // Center Across Selection: the text centres over the empty cells
            // to its right that are aligned the same way. With none, it is
            // centred text, spilling as that does.
            let continuous_span = match alignment.horizontal {
                HorizontalAlignment::CenterContinuous if merge.is_none() && !text.is_empty() => {
                    center_continuous_span(sheet, styles, row, cell.reference, &columns)
                }
                _ => None,
            };
            if alignment.horizontal == HorizontalAlignment::CenterContinuous
                && continuous_span.is_none()
            {
                alignment.horizontal = HorizontalAlignment::Center;
            }
            let filter_reserve =
                filter_button::text_reserve(&filter_definitions, cell.reference, merge, &columns);
            let text_width = span_width(&columns, cell.reference, merge.as_ref()) - filter_reserve;
            let overflow = if filter_reserve == 0.0
                && (matches!(cell.value, CellValue::Text(_)) || shown_formula.is_some())
                && merge.is_none()
                && !alignment.wrap_text
                && !alignment.shrink_to_fit
                && alignment.text_rotation == 0
                && !text.is_empty()
            {
                text_overflow(
                    &columns,
                    &occupied,
                    cell.reference.column,
                    alignment.horizontal,
                    options.measure.measure(shown, &font),
                    options.cell_padding_px + text_inset_px,
                )
            } else {
                None
            };
            let overflow = continuous_span.or(overflow);

            if alignment.shrink_to_fit && !alignment.wrap_text && !text.is_empty() {
                let available =
                    (text_width - options.cell_padding_px * 2.0 - text_inset_px).max(1.0);
                let measured = options.measure.measure(shown, &font);
                if measured > available {
                    let scale = (available / measured).clamp(0.0, 1.0) as f32;
                    font.size_points = (font.size_points * scale).max(1.0);
                }
            }

            let stacks_text = alignment.text_rotation == 255;
            let drawing_text = match cell.value {
                CellValue::Number(value)
                    if !alignment.wrap_text && alignment.text_rotation == 0 =>
                {
                    let width =
                        (text_width - options.cell_padding_px * 2.0 - text_inset_px).max(0.0);
                    number_format::fit_number(
                        value,
                        number_format_code,
                        &text,
                        width,
                        |candidate| options.measure.measure(candidate, &font),
                    )
                }
                _ => shown.to_string(),
            };
            // `*c` repeats c across whatever width the text leaves.
            let drawing_text = match fill_at {
                Some((at, character))
                    if drawing_text == text
                        && !alignment.wrap_text
                        && alignment.text_rotation == 0 =>
                {
                    let width =
                        (text_width - options.cell_padding_px * 2.0 - text_inset_px).max(0.0);
                    let unit = options.measure.measure(&character.to_string(), &font);
                    let count = if unit > 0.0 {
                        ((width - options.measure.measure(&text, &font)) / unit)
                            .floor()
                            .clamp(0.0, 1000.0) as usize
                    } else {
                        0
                    };
                    let mut filled = text.clone();
                    filled.insert_str(at, &character.to_string().repeat(count));
                    filled
                }
                _ => drawing_text,
            };
            // Fill repeats the text as many whole times as the cell holds.
            let drawing_text = match alignment.horizontal {
                HorizontalAlignment::Fill
                    if !drawing_text.is_empty() && alignment.text_rotation == 0 =>
                {
                    let width =
                        (text_width - options.cell_padding_px * 2.0 - text_inset_px).max(0.0);
                    let unit = options.measure.measure(&drawing_text, &font);
                    let count = if unit > 0.0 {
                        (width / unit).floor().clamp(1.0, 1000.0) as usize
                    } else {
                        1
                    };
                    drawing_text.repeat(count)
                }
                _ => drawing_text,
            };
            let lines = if stacks_text && !text.is_empty() {
                text.graphemes(true).map(str::to_string).collect()
            } else if alignment.wrap_text && !text.is_empty() {
                let width = text_width - options.cell_padding_px * 2.0 - text_inset_px;
                let measure = Rc::clone(&options.measure);
                let font_for_measure = font.clone();
                wrap_text(
                    &text,
                    width.max(1.0),
                    &mut |grapheme| measure.measure(grapheme, &font_for_measure),
                    WrapOptions::default(),
                )
            } else {
                vec![drawing_text]
            };

            // Excel does not auto-fit a row to a merged cell, and neither does
            // this: the height would have to be shared between the merged rows
            // and Excel leaves that to the user.
            if (alignment.wrap_text || stacks_text) && merge.is_none() && lines.len() > 1 {
                let needed = units::line_height_points(font.size_points) * lines.len() as f32;
                let slot = &mut auto_heights[cell.reference.row as usize];
                *slot = slot.max(needed);
            }

            let runs = rich_runs(
                sheet.rich_text.get(&cell.reference),
                &cell.value,
                &text,
                &font,
            );
            prepared.push(PreparedCell {
                reference: cell.reference,
                merge,
                text,
                lines,
                runs,
                font,
                // A colour in the number format wins over the font's own,
                // which is how `[Red]-#,##0` turns a negative red without the
                // cell being styled red.
                text_color: conditional_style
                    .text_color
                    .or_else(|| formatted.color.map(format_color_to_color))
                    .or_else(|| cell_text_color(styles, format, table_style.as_ref())),
                fill: conditional_format::conditional_fill(&conditional_style).unwrap_or_else(
                    || {
                        format
                            .and_then(|format| styles.fill(format))
                            .and_then(resolve_fill)
                            .or_else(|| table_style.as_ref().and_then(|table| table.fill.clone()))
                    },
                ),
                border: {
                    let mut border = cell_border(styles, format, table_style.as_ref());
                    conditional_format::apply_border(&conditional_style, &mut border);
                    border
                },
                alignment,
                text_inset_px,
                data_bar,
                icon,
                overflow,
                error_flag: error_check_flags(&cell.value, cell.formula.is_some()),
            });
        }
    }

    // A blank merge need not have a <c> (or even a <row>) in SpreadsheetML.
    // Give it the same surface and hit target as a merge containing a value.
    let anchors: HashSet<CellRef> = prepared
        .iter()
        .filter(|cell| cell.merge.is_some())
        .map(|cell| cell.reference)
        .collect();
    for merge in &sheet.merges {
        if anchors.contains(&merge.start) {
            continue;
        }
        let format = styles.cell_format(sheet.effective_style_index(merge.start, None));
        let conditional_style = conditional.for_cell(merge.start, &CellValue::Empty);
        let table_style = styled_tables
            .iter()
            .find_map(|table| table.style_at(merge.start))
            .or_else(|| styled_pivots.iter().find_map(|p| p.style_at(merge.start)));
        let mut alignment = format.map(|format| format.alignment).unwrap_or_default();
        alignment.horizontal = resolve_horizontal(alignment.horizontal, &CellValue::Empty);
        prepared.push(PreparedCell {
            reference: merge.start,
            merge: Some(*merge),
            text: String::new(),
            lines: vec![String::new()],
            runs: Vec::new(),
            font: {
                let mut font = resolve_font(
                    format.and_then(|format| styles.font(format)),
                    &options.default_font,
                );
                if format.is_none_or(|f| f.apply_font != Some(true)) {
                    if let Some(table) = &table_style {
                        font.bold |= table.bold;
                        font.italic |= table.italic;
                        font.underline |= table.underline;
                        font.strike |= table.strike;
                    }
                }
                conditional_format::apply_font(&conditional_style, &mut font);
                font
            },
            text_color: conditional_style
                .text_color
                .or_else(|| cell_text_color(styles, format, table_style.as_ref())),
            fill: conditional_format::conditional_fill(&conditional_style).unwrap_or_else(|| {
                format
                    .and_then(|format| styles.fill(format))
                    .and_then(resolve_fill)
                    .or_else(|| table_style.as_ref().and_then(|table| table.fill.clone()))
            }),
            border: {
                let mut border = cell_border(styles, format, table_style.as_ref());
                conditional_format::apply_border(&conditional_style, &mut border);
                border
            },
            alignment,
            text_inset_px: 0.0,
            data_bar: None,
            icon: None,
            overflow: None,
            error_flag: false,
        });
    }

    let mut rows = row_axis(sheet, row_count, &auto_heights);
    let computed = kept.map(|kept| {
        let computed = (columns.clone(), rows.clone());
        columns = kept.columns.clone();
        rows = kept.rows.clone();
        computed
    });

    // One-cell and absolute anchors can extend beyond the used cells. Include
    // their transformed bounds in both the scroll surface and the grid axes.
    let shapes = place_shapes(sheet, &columns, &rows, options, workbook, styles);
    let right = shapes
        .iter()
        .map(|shape| shape.paint_rect())
        .map(|rect| rect.x + rect.width)
        .fold(0.0, f64::max);
    let bottom = shapes
        .iter()
        .map(|shape| shape.paint_rect())
        .map(|rect| rect.y + rect.height)
        .fold(0.0, f64::max);
    extend_columns(sheet, column_count, right, options, &mut columns);
    extend_rows(sheet, row_count, bottom, &auto_heights, &mut rows);

    let content_width = columns
        .last()
        .map(|entry| entry.offset + entry.size)
        .unwrap_or(0.0);
    let content_height = rows
        .last()
        .map(|entry| entry.offset + entry.size)
        .unwrap_or(0.0);

    let filter_buttons = filter_button::place(filter_definitions, &columns, &rows);
    let filter_button_rects: BTreeMap<_, _> = filter_buttons
        .iter()
        .map(|button| ((button.reference.row, button.reference.column), button.rect))
        .collect();
    // Reserved up front: a cell is several hundred bytes, and a vector grown
    // by doubling moves every one of them each time.
    let mut cells = Vec::with_capacity(prepared.len());
    cells.extend(prepared.into_iter().filter_map(|cell| {
        let rect = merged_rect(cell.reference, cell.merge.as_ref(), &columns, &rows)?;
        Some(PlacedCell {
            reference: cell.reference,
            rect,
            text_clip: {
                let clip = cell.overflow.map(|(left, right)| Rect {
                    x: left,
                    width: right - left,
                    ..rect
                });
                let end = cell
                    .merge
                    .map(|range| range.end.column)
                    .unwrap_or(cell.reference.column);
                let button = filter_button_rects
                    .range((cell.reference.row, cell.reference.column)..=(cell.reference.row, end))
                    .next();
                button
                    .map(|(_, button)| Rect {
                        width: (button.x - rect.x).max(0.0),
                        ..rect
                    })
                    .or(clip)
            },
            merge: cell.merge,
            text: cell.text,
            lines: cell.lines,
            font: cell.font.clone(),
            text_color: cell.text_color,
            fill: cell.fill,
            border: cell.border,
            data_bar: cell.data_bar.map(|bar| {
                let inset = 2.0_f64.min(rect.width / 2.0).min(rect.height / 2.0);
                let available = (rect.width - 2.0 * inset).max(0.0);
                let left = rect.x + inset;
                let right = left + available;
                let origin = (left + available * bar.origin).round().clamp(left, right);
                let capacity = if bar.reverse {
                    origin - left
                } else {
                    right - origin
                };
                let width = (rect.width * bar.fraction).round().min(capacity).max(0.0);
                PlacedDataBar {
                    rect: Rect {
                        x: if bar.reverse { origin - width } else { origin },
                        y: rect.y + inset,
                        width,
                        height: rect.height - 2.0 * inset,
                    },
                    color: bar.color,
                    show_value: bar.show_value,
                    gradient: bar.gradient,
                    reverse: bar.reverse,
                    border: bar.border,
                    axis: bar.axis.map(|(axis, color)| PlacedDataBarAxis {
                        rect: Rect {
                            x: (left + available * axis).round().clamp(left, right) - 0.5,
                            y: rect.y + inset,
                            width: 1.0,
                            height: rect.height - 2.0 * inset,
                        },
                        color,
                    }),
                }
            }),
            icon: cell.icon.map(|icon| {
                let size = icon_size_px(&cell.font, rect.height - 2.0);
                let y = match cell.alignment.vertical {
                    VerticalAlignment::Top | VerticalAlignment::Justify => rect.y + 1.0,
                    VerticalAlignment::Center | VerticalAlignment::Distributed => {
                        rect.y + (rect.height - size) / 2.0
                    }
                    VerticalAlignment::Bottom => rect.y + rect.height - size - 2.0,
                };
                PlacedIcon {
                    icon_set: icon.icon_set,
                    index: icon.index,
                    rect: Rect {
                        x: rect.x + 2.0,
                        y: y.max(rect.y).round(),
                        width: size,
                        height: size,
                    },
                    show_value: icon.show_value,
                }
            }),
            alignment: cell.alignment,
            text_inset_px: cell.text_inset_px,
            runs: cell.runs,
            error_flag: cell.error_flag,
        })
    }));

    let frozen_columns = sheet
        .freeze_pane
        .as_ref()
        .map(|pane| pane.frozen_columns)
        .unwrap_or_default()
        .min(columns.len() as u32);
    let frozen_rows = sheet
        .freeze_pane
        .as_ref()
        .map(|pane| pane.frozen_rows)
        .unwrap_or_default()
        .min(rows.len() as u32);

    let notes = place_notes(sheet, &columns, &rows);

    let resolve_blank_style = |index| {
        let format = styles.cell_format(Some(index));
        BlankCellStyle {
            font: resolve_font(
                format.and_then(|format| styles.font(format)),
                &options.default_font,
            ),
            text_color: format
                .and_then(|format| styles.font(format))
                .and_then(|font| font.color),
            fill: format
                .and_then(|format| styles.fill(format))
                .and_then(resolve_fill),
            border: format
                .and_then(|format| styles.border(format))
                .cloned()
                .unwrap_or_default(),
            alignment: format.map(|format| format.alignment).unwrap_or_default(),
        }
    };
    let blank_row_styles = sheet
        .rows
        .iter()
        .filter(|row| row.custom_format)
        .filter_map(|row| {
            row.style_index
                .map(|index| (row.index, resolve_blank_style(index)))
        })
        .collect();
    let blank_column_styles = sheet
        .columns
        .iter()
        .filter_map(|span| {
            span.style_index.map(|index| BlankColumnStyle {
                first: span.first,
                last: span.last,
                style: resolve_blank_style(index),
            })
        })
        .collect();
    let sparklines = sparkline::place(sheet, workbook, &columns, &rows);
    let layout = SheetLayout {
        columns,
        rows,
        cells,
        filter_buttons,
        blank_conditional_rules: conditional.blank_rules(),
        blank_row_styles,
        blank_column_styles,
        shapes,
        notes,
        tables: styled_tables,
        pivots: styled_pivots,
        sparklines,
        show_grid_lines: sheet.show_grid_lines,
        content_width,
        max_digit_width_px: options.max_digit_width_px,
        content_height,
        frozen_columns,
        frozen_rows,
        cell_padding_px: options.cell_padding_px,
        outline: sheet.outline.clone(),
    };
    (layout, computed)
}

/// Lays out again only `rows` of `layout`, after values changed in them and
/// nothing else on the sheet did — what typing into a sheet does, a cell and
/// the formulas reading it. The rows' cells are placed afresh, from a sheet
/// holding only those rows, on the layout's own columns and rows, and take
/// the place of theirs; the rest stays. `false`, leaving `layout` alone,
/// when that would not give what a full layout gives — a row's height
/// changing, or a sheet with what reads values across rows (conditional
/// formats, sparklines, tables, pivots, drawings) or a merge through those
/// rows — and the caller lays the sheet out in full.
pub fn relayout_rows(
    layout: &mut SheetLayout,
    workbook: &jade_xlsx_reader::Workbook,
    index: usize,
    options: &LayoutOptions,
    rows: &std::collections::BTreeSet<u32>,
) -> bool {
    let Some(sheet) = workbook.sheets.get(index) else {
        return false;
    };
    if rows.is_empty() {
        return true;
    }
    if !sheet.conditional_formats.is_empty()
        || !sheet.sparkline_groups.is_empty()
        || !sheet.tables.is_empty()
        || !sheet.shapes.is_empty()
        || !workbook.pivots.is_empty()
        || rows.iter().any(|row| *row as usize >= layout.rows.len())
        || sheet
            .merges
            .iter()
            .any(|merge| rows.range(merge.start.row..=merge.end.row).next().is_some())
    {
        return false;
    }
    let in_rows = |at: &CellRef| rows.contains(&at.row);
    let reduced = Sheet {
        rows: sheet
            .rows
            .iter()
            .filter(|row| rows.contains(&row.index))
            .cloned()
            .collect(),
        rich_text: sheet
            .rich_text
            .iter()
            .filter(|(at, _)| in_rows(at))
            .map(|(at, runs)| (*at, runs.clone()))
            .collect(),
        phonetic_readings: sheet
            .phonetic_readings
            .iter()
            .filter(|(at, _)| in_rows(at))
            .map(|(at, reading)| (*at, reading.clone()))
            .collect(),
        auto_filter: sheet.auto_filter.clone(),
        filter_mode: sheet.filter_mode,
        filter_row_state: sheet.filter_row_state.clone(),
        name: sheet.name.clone(),
        part_name: sheet.part_name.clone(),
        visibility: sheet.visibility,
        columns: sheet.columns.clone(),
        merges: sheet.merges.clone(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: sheet.data_validations.clone(),
        conditional_formats: Vec::new(),
        default_column_width_chars: sheet.default_column_width_chars,
        default_row_height_points: sheet.default_row_height_points,
        default_row_custom_height: sheet.default_row_custom_height,
        default_rows_hidden: sheet.default_rows_hidden,
        show_grid_lines: sheet.show_grid_lines,
        show_formulas: sheet.show_formulas,
        freeze_pane: sheet.freeze_pane.clone(),
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: sheet.page_setup.clone(),
        shapes: Vec::new(),
        array_formulas: sheet.array_formulas.clone(),
        tab_color: None,
        sparkline_groups: Vec::new(),
        protection: sheet.protection,
        outline: sheet.outline.clone(),
    };
    let options = LayoutOptions {
        min_row_count: layout.rows.len() as u32,
        min_column_count: layout.columns.len() as u32,
        ..options.clone()
    };
    let (fresh, computed) = layout_sheet_on_axes(
        &reduced,
        &workbook.styles,
        &options,
        Some(workbook),
        Some(layout),
    );
    let Some((columns, row_axis)) = computed else {
        return false;
    };
    if columns != layout.columns
        || rows.iter().any(|&row| {
            let (now, was) = (&row_axis[row as usize], &layout.rows[row as usize]);
            now.size != was.size || now.hidden != was.hidden
        })
    {
        return false;
    }
    // The rows' new cells go where theirs were: the cells of a layout run in
    // row order, then (after the first step back) its blank merged ranges.
    let mut fresh: Vec<PlacedCell> = fresh
        .cells
        .into_iter()
        .filter(|cell| rows.contains(&cell.reference.row))
        .collect();
    fresh.sort_by_key(|cell| (cell.reference.row, cell.reference.column));
    if splice_rows(&mut layout.cells, rows, &mut fresh) {
        return true;
    }
    let kept: Vec<PlacedCell> = std::mem::take(&mut layout.cells)
        .into_iter()
        .filter(|cell| !(rows.contains(&cell.reference.row) && cell.merge.is_none()))
        .collect();
    let mut cells = Vec::with_capacity(kept.len() + fresh.len());
    let mut fresh = fresh.into_iter().peekable();
    let mut in_row_order = true;
    let mut previous_row = 0;
    for cell in kept {
        if in_row_order && cell.reference.row < previous_row {
            in_row_order = false;
            cells.extend(fresh.by_ref());
        }
        if in_row_order {
            while fresh
                .peek()
                .is_some_and(|next| next.reference.row < cell.reference.row)
            {
                cells.extend(fresh.next());
            }
            previous_row = cell.reference.row;
        }
        cells.push(cell);
    }
    cells.extend(fresh);
    layout.cells = cells;
    true
}

/// [`relayout_rows`]' cells put in place without rebuilding the rest: each
/// row's cells are one run of the row-ordered part of `cells`, so its new
/// ones replace that run where it is, and the cells after it move only when
/// the row's count changed. `false`, with nothing changed, when a row's old
/// cells are not where that finds them.
fn splice_rows(
    cells: &mut Vec<PlacedCell>,
    rows: &std::collections::BTreeSet<u32>,
    fresh: &mut Vec<PlacedCell>,
) -> bool {
    let ordered = cells
        .windows(2)
        .position(|pair| pair[1].reference.row < pair[0].reference.row)
        .map_or(cells.len(), |at| at + 1);
    // The blank merged ranges after it hold no cell of these rows that would
    // be replaced.
    if cells[ordered..]
        .iter()
        .any(|cell| rows.contains(&cell.reference.row) && cell.merge.is_none())
    {
        return false;
    }
    let mut runs = Vec::with_capacity(rows.len());
    for &row in rows {
        let start = cells[..ordered].partition_point(|cell| cell.reference.row < row);
        let end = cells[..ordered].partition_point(|cell| cell.reference.row <= row);
        if cells[start..end].iter().any(|cell| cell.merge.is_some()) {
            return false;
        }
        runs.push((row, start..end));
    }
    // From the last row back, so the runs before stay where they are.
    for (row, run) in runs.into_iter().rev() {
        let from = fresh.partition_point(|cell| cell.reference.row < row);
        let new: Vec<PlacedCell> = fresh.drain(from..).collect();
        cells.splice(run, new);
    }
    true
}

/// A stored formula as Excel shows it with Show Formulas on: with its `=`,
/// and without the file's `_xlfn.` and `_xlws.` prefixes.
fn formula_as_typed(formula: &str) -> String {
    format!(
        "={}",
        formula.replace("_xlfn._xlws.", "").replace("_xlfn.", "")
    )
}

fn resolve_font(font: Option<&Font>, fallback: &ResolvedFont) -> ResolvedFont {
    font.map(|font| ResolvedFont {
        family: font.name.clone().unwrap_or_else(|| fallback.family.clone()),
        size_points: font.size_points.unwrap_or(fallback.size_points),
        bold: font.bold,
        italic: font.italic,
        underline: font.underline,
        strike: font.strike,
        vert_align: font.vert_align.clone(),
    })
    .unwrap_or_else(|| fallback.clone())
}

fn resolve_fill(fill: &jade_xlsx_reader::Fill) -> Option<ResolvedFill> {
    if let Some(gradient) = fill
        .gradient
        .as_ref()
        .filter(|gradient| !gradient.stops.is_empty())
    {
        return Some(ResolvedFill {
            pattern: "gradient".into(),
            foreground: gradient.stops.first().map(|(_, color)| *color),
            background: gradient.stops.last().map(|(_, color)| *color),
            gradient: Some(gradient.clone()),
        });
    }
    let pattern = fill.pattern.as_deref()?;
    if pattern == "none" {
        return None;
    }
    Some(ResolvedFill {
        pattern: pattern.to_string(),
        foreground: fill.foreground,
        background: fill.background,
        gradient: None,
    })
}

/// The runs a string cell is drawn in: its rich text runs, when they still
/// spell its text, or one run for a cell whose own font is raised or
/// lowered.
fn rich_runs(
    runs: Option<&Vec<jade_xlsx_reader::rich_text::TextRun>>,
    value: &CellValue,
    text: &str,
    font: &ResolvedFont,
) -> Vec<PlacedRun> {
    let CellValue::Text(value) = value else {
        return Vec::new();
    };
    if let Some(runs) = runs {
        let joined: String = runs.iter().map(|run| run.text.as_str()).collect();
        if joined == *value && joined == text {
            let mut start = 0;
            return runs
                .iter()
                .filter(|run| !run.text.is_empty())
                .map(|run| {
                    let end = start + run.text.len();
                    let own = &run.font;
                    let placed = PlacedRun {
                        start,
                        end,
                        font: ResolvedFont {
                            family: own.name.clone().unwrap_or_else(|| font.family.clone()),
                            size_points: own.size_points.unwrap_or(font.size_points),
                            bold: own.bold.unwrap_or(font.bold),
                            italic: own.italic.unwrap_or(font.italic),
                            underline: own.underline.unwrap_or(font.underline),
                            strike: own.strike.unwrap_or(font.strike),
                            vert_align: own.vert_align.clone().or_else(|| {
                                own.bold
                                    .is_none()
                                    .then(|| font.vert_align.clone())
                                    .flatten()
                            }),
                        },
                        color: own.color,
                    };
                    start = end;
                    placed
                })
                .collect();
        }
    }
    if font.vert_align.is_some() && !text.is_empty() {
        return vec![PlacedRun {
            start: 0,
            end: text.len(),
            font: font.clone(),
            color: None,
        }];
    }
    Vec::new()
}

/// A cell's text colour: its own font's, unless that is just the workbook's
/// default and a table style colours the text.
fn cell_text_color(
    styles: &StyleTable,
    format: Option<&jade_xlsx_reader::CellFormat>,
    table: Option<&table_style::TableCellStyle>,
) -> Option<Color> {
    let own = format
        .and_then(|format| styles.font(format))
        .and_then(|font| font.color);
    if format.is_some_and(|f| f.apply_font == Some(true)) {
        return own;
    }
    match table.and_then(|table| table.text_color) {
        Some(table_color) => {
            let normal = styles.normal_font().and_then(|font| font.color);
            if own.is_none() || own == normal || own == Some(Color::Auto) {
                Some(table_color)
            } else {
                own
            }
        }
        None => own,
    }
}

/// A cell's own borders over its table style's.
fn cell_border(
    styles: &StyleTable,
    format: Option<&jade_xlsx_reader::CellFormat>,
    table: Option<&table_style::TableCellStyle>,
) -> Border {
    let own = format
        .and_then(|format| styles.border(format))
        .cloned()
        .unwrap_or_default();
    let Some(table) = table else {
        return own;
    };
    let mut border = Border {
        left: table.left.clone(),
        right: table.right.clone(),
        top: table.top.clone(),
        bottom: table.bottom.clone(),
        ..Border::default()
    };
    border.diagonal = own.diagonal.clone();
    border.diagonal_up = own.diagonal_up;
    border.diagonal_down = own.diagonal_down;
    for (side, edge) in [
        (&mut border.left, own.left),
        (&mut border.right, own.right),
        (&mut border.top, own.top),
        (&mut border.bottom, own.bottom),
    ] {
        if edge.is_some() {
            *side = edge;
        }
    }
    border
}

/// A blank cell's table-style borders where nothing else draws one.
fn overlay_table_border(border: &mut Border, table: &table_style::TableCellStyle) {
    for (side, edge) in [
        (&mut border.left, &table.left),
        (&mut border.right, &table.right),
        (&mut border.top, &table.top),
        (&mut border.bottom, &table.bottom),
    ] {
        if side.is_none() {
            *side = edge.clone();
        }
    }
}

/// A cell after its style and text are resolved, but before the row axis
/// exists to place it on.
struct PreparedCell {
    reference: CellRef,
    merge: Option<CellRange>,
    text: String,
    lines: Vec<String>,
    runs: Vec<PlacedRun>,
    font: ResolvedFont,
    text_color: Option<Color>,
    fill: Option<ResolvedFill>,
    border: Border,
    alignment: Alignment,
    text_inset_px: f64,
    data_bar: Option<visual_format::ResolvedDataBar>,
    icon: Option<visual_format::ResolvedIcon>,
    overflow: Option<(f64, f64)>,
    error_flag: bool,
}

/// Whether Excel's background error checking flags a cell, by its default
/// rules: a formula that gives an error, or a number stored as text (text
/// that reads as a number).
fn error_check_flags(value: &CellValue, formula: bool) -> bool {
    match value {
        CellValue::Error(_) => formula,
        CellValue::Text(text) => {
            let text = text.trim();
            text.chars().any(|c| c.is_ascii_digit()) && text.replace(',', "").parse::<f64>().is_ok()
        }
        _ => false,
    }
}

/// An icon set's icons are 16 px for 11 pt text and grow with the font, but
/// never past the row.
fn icon_size_px(font: &ResolvedFont, room: f64) -> f64 {
    let size = (f64::from(font.size_points) * 96.0 / 72.0 * 16.0 / (11.0 * 96.0 / 72.0)).round();
    size.max(8.0).min(room.max(4.0)).floor()
}

/// Evaluates the sheet's formula-decided conditional rules over the laid-out
/// extent, when it has any.
fn formula_rule_matches(
    sheet: &Sheet,
    styles: &StyleTable,
    workbook: Option<&jade_xlsx_reader::Workbook>,
    columns: u32,
    rows: u32,
    options: &LayoutOptions,
) -> Option<Vec<conditional_format::FormulaRuleMatches>> {
    let formula_rules = sheet
        .conditional_formats
        .iter()
        .any(|format| format.rules.iter().any(conditional_format::is_formula_rule));
    if !formula_rules {
        return None;
    }
    let time_zone = options.time_zone.as_deref().unwrap_or("UTC");
    let found = workbook.and_then(|workbook| {
        let index = workbook
            .sheets
            .iter()
            .position(|candidate| std::ptr::eq(candidate, sheet))?;
        Some(jade_xlsx_calc::conditional::conditional_formula_matches(
            workbook, index, columns, rows, time_zone,
        ))
    });
    let found = match found {
        Some(found) => found,
        None => {
            // On its own the sheet is a workbook of one.
            let alone = jade_xlsx_reader::Workbook {
                sheets: vec![sheet.clone()],
                styles: styles.clone(),
                date_system: options.date_system,
                ..Default::default()
            };
            jade_xlsx_calc::conditional::conditional_formula_matches(
                &alone, 0, columns, rows, time_zone,
            )
        }
    };
    found.ok()
}

fn text_overflow(
    columns: &[AxisEntry],
    blockers: &[(u32, u32)],
    column: u32,
    alignment: HorizontalAlignment,
    width: f64,
    inset: f64,
) -> Option<(f64, f64)> {
    let own = columns.get(column as usize)?;
    if own.size <= 0.0 || !width.is_finite() {
        return None;
    }
    let left = own.offset;
    let right = left + own.size;
    let (text_left, text_right) = match alignment {
        HorizontalAlignment::Left => (left + inset, left + inset + width),
        HorizontalAlignment::Right => (right - inset - width, right - inset),
        HorizontalAlignment::Center => (
            left + (own.size - width) / 2.0,
            left + (own.size + width) / 2.0,
        ),
        _ => return None,
    };
    let before = blockers.partition_point(|&(start, _)| start < column);
    let after = blockers.partition_point(|&(start, _)| start <= column);
    let lower = blockers[..before]
        .last()
        .map(|&(_, end)| end)
        .and_then(|index| columns.get(index as usize))
        .map(|entry| entry.offset + entry.size)
        .unwrap_or(0.0);
    let upper = blockers
        .get(after)
        .and_then(|&(start, _)| columns.get(start as usize))
        .map(|entry| entry.offset)
        .unwrap_or_else(|| {
            columns
                .last()
                .map(|entry| entry.offset + entry.size)
                .unwrap_or(right)
        });
    let clip_left = left.min(text_left.max(lower));
    let clip_right = right.max(text_right.min(upper));
    (clip_left < left || clip_right > right).then_some((clip_left, clip_right))
}

/// The width a cell's text has, across its merged range when it has one.
/// A sheet's merged ranges by the rows they cover, so layout asks which
/// merges a row or cell is in without scanning them all for every cell — a
/// form laid out in merged cells (Excel 方眼紙) has thousands. A merge
/// taller than [`MergeRows::TALL`] rows is kept aside and checked for every
/// row instead.
struct MergeRows<'a> {
    by_row: HashMap<u32, Vec<&'a CellRange>>,
    tall: Vec<&'a CellRange>,
}

impl<'a> MergeRows<'a> {
    const TALL: u32 = 4096;

    fn new(merges: &'a [CellRange]) -> Self {
        let mut by_row: HashMap<u32, Vec<&'a CellRange>> = HashMap::new();
        let mut tall = Vec::new();
        for range in merges {
            if range.end.row.saturating_sub(range.start.row) >= Self::TALL {
                tall.push(range);
                continue;
            }
            for row in range.start.row..=range.end.row {
                by_row.entry(row).or_default().push(range);
            }
        }
        Self { by_row, tall }
    }

    /// The merges that cover `row`, in the sheet's order.
    fn in_row(&self, row: u32) -> impl Iterator<Item = &'a CellRange> + '_ {
        self.by_row.get(&row).into_iter().flatten().copied().chain(
            self.tall
                .iter()
                .copied()
                .filter(move |range| range.start.row <= row && row <= range.end.row),
        )
    }

    /// The merge `cell` is in, as [`Sheet::merge_at`] finds it.
    fn at(&self, cell: CellRef) -> Option<&'a CellRange> {
        self.in_row(cell.row).find(|range| range.contains(cell))
    }
}

/// The span Center Across Selection centres a cell's text over: the cell
/// and the empty, visible cells right of it that are aligned
/// `centerContinuous` too. `None` when that is the cell alone.
fn center_continuous_span(
    sheet: &Sheet,
    styles: &StyleTable,
    row: &Row,
    reference: CellRef,
    columns: &[AxisEntry],
) -> Option<(f64, f64)> {
    let own = columns.get(reference.column as usize)?;
    let mut right = own.offset + own.size;
    let mut column = reference.column + 1;
    while let Some(entry) = columns.get(column as usize) {
        let at = CellRef::new(column, reference.row);
        let found = row
            .cells
            .binary_search_by_key(&column, |cell| cell.reference.column)
            .ok()
            .map(|index| &row.cells[index]);
        if found.is_some_and(|cell| !cell.value.is_empty() || cell.formula.is_some())
            || sheet.merge_at(at).is_some()
        {
            break;
        }
        let style = sheet.effective_style_index_in_row(
            at,
            found.and_then(|cell| cell.style_index),
            Some(row),
        );
        let continues = styles.cell_format(style).is_some_and(|format| {
            format.alignment.horizontal == HorizontalAlignment::CenterContinuous
        });
        if !continues {
            break;
        }
        right = entry.offset + entry.size;
        column += 1;
    }
    (right > own.offset + own.size).then_some((own.offset, right))
}

fn span_width(columns: &[AxisEntry], reference: CellRef, merge: Option<&CellRange>) -> f64 {
    let last = merge
        .map(|range| range.end.column)
        .unwrap_or(reference.column);
    (reference.column..=last)
        .filter_map(|index| columns.get(index as usize))
        .map(|entry| entry.size)
        .sum()
}

/// The colours a number format can name, as the model's colour type.
fn format_color_to_color(color: number_format::FormatColor) -> Color {
    use number_format::FormatColor;
    let rgb = match color {
        FormatColor::Black => [0xFF, 0x00, 0x00, 0x00],
        FormatColor::Blue => [0xFF, 0x00, 0x00, 0xFF],
        FormatColor::Cyan => [0xFF, 0x00, 0xFF, 0xFF],
        FormatColor::Green => [0xFF, 0x00, 0xFF, 0x00],
        FormatColor::Magenta => [0xFF, 0xFF, 0x00, 0xFF],
        FormatColor::Red => [0xFF, 0xFF, 0x00, 0x00],
        FormatColor::White => [0xFF, 0xFF, 0xFF, 0xFF],
        FormatColor::Yellow => [0xFF, 0xFF, 0xFF, 0x00],
        FormatColor::Indexed(index) => return Color::Indexed(index),
    };
    Color::Argb(rgb)
}

/// Excel's General alignment: numbers right, text left, and the two value kinds
/// that are neither centred.
fn resolve_horizontal(alignment: HorizontalAlignment, value: &CellValue) -> HorizontalAlignment {
    if alignment != HorizontalAlignment::General {
        return alignment;
    }
    match value {
        CellValue::Number(_) => HorizontalAlignment::Right,
        CellValue::Bool(_) | CellValue::Error(_) => HorizontalAlignment::Center,
        _ => HorizontalAlignment::Left,
    }
}

/// The rectangle a cell occupies, grown over its merged range when it anchors
/// one.
fn cell_rect(
    reference: CellRef,
    sheet: &Sheet,
    columns: &[AxisEntry],
    rows: &[AxisEntry],
) -> Option<Rect> {
    merged_rect(reference, sheet.merge_at(reference), columns, rows)
}

/// [`cell_rect`] with the merged range already found.
fn merged_rect(
    reference: CellRef,
    merge: Option<&CellRange>,
    columns: &[AxisEntry],
    rows: &[AxisEntry],
) -> Option<Rect> {
    let first_column = columns.get(reference.column as usize)?;
    let first_row = rows.get(reference.row as usize)?;
    let (last_column_index, last_row_index) = match merge {
        Some(range) => (range.end.column, range.end.row),
        None => (reference.column, reference.row),
    };
    let last_column = columns
        .get(last_column_index as usize)
        .unwrap_or(first_column);
    let last_row = rows.get(last_row_index as usize).unwrap_or(first_row);
    Some(Rect {
        x: first_column.offset,
        y: first_row.offset,
        width: last_column.offset + last_column.size - first_column.offset,
        height: last_row.offset + last_row.size - first_row.offset,
    })
}

#[derive(Clone, Copy)]
enum Axis {
    Column,
    Row,
}

/// Where the sheet's drawing objects land, from their anchor cells.
///
/// The anchor offsets are EMUs into the anchor cell; the cell's own px
/// position comes from the axes, so a shape follows a column that was
/// widened the way it does in Excel. A one-cell anchor sizes the object by
/// its extent instead of a second cell.
fn place_shapes(
    sheet: &Sheet,
    columns: &[AxisEntry],
    rows: &[AxisEntry],
    options: &LayoutOptions,
    workbook: Option<&jade_xlsx_reader::Workbook>,
    styles: &StyleTable,
) -> Vec<PlacedShape> {
    let corner = |cell: CellRef, offset: (i64, i64)| -> Option<(f64, f64)> {
        let column = columns.get(cell.column as usize)?;
        let row = rows.get(cell.row as usize)?;
        Some((
            column.offset + offset.0 as f64 / EMU_PER_PX,
            row.offset + offset.1 as f64 / EMU_PER_PX,
        ))
    };
    sheet
        .shapes
        .iter()
        .filter_map(|shape| {
            let (x, y) = corner(shape.from, shape.from_offset)?;
            let (right, bottom) = match (shape.to, shape.extent) {
                (Some(to), _) => corner(to, shape.to_offset)?,
                (None, Some((cx, cy))) => (x + cx as f64 / EMU_PER_PX, y + cy as f64 / EMU_PER_PX),
                (None, None) => return None,
            };
            let rect = Rect {
                x,
                y,
                width: (right - x).max(0.0),
                height: (bottom - y).max(0.0),
            };
            let mut font = options.default_font.clone();
            let style = shape.style.as_ref().map(|style| {
                shape_style::place(
                    style,
                    rect.width,
                    rect.height,
                    styles,
                    &mut font,
                    shape.kind == ShapeKind::Connector,
                )
            });
            let lines = match &shape.text {
                Some(text) if !text.is_empty() => {
                    let inset = style
                        .as_ref()
                        .map_or(options.cell_padding_px, |style| style.inset_x);
                    let width = (rect.width - inset * 2.0).max(1.0);
                    let measure = Rc::clone(&options.measure);
                    let font_for_measure = font.clone();
                    text.split('\n')
                        .flat_map(|paragraph| {
                            if paragraph.is_empty() {
                                vec![String::new()]
                            } else {
                                wrap_text(
                                    paragraph,
                                    width,
                                    &mut |grapheme| measure.measure(grapheme, &font_for_measure),
                                    WrapOptions::default(),
                                )
                            }
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            Some(PlacedShape {
                id: shape.id,
                name: shape.name.clone(),
                kind: shape.kind,
                rect,
                chart: shape.chart.as_ref().map(|chart| {
                    chart::layout_chart(chart, rect, sheet, workbook, options, styles)
                }),
                picture: shape.picture.as_ref().map(|picture| {
                    let [left, top, right, bottom] =
                        picture.crop.map(|value| f64::from(value) / 100_000.0);
                    let width = 1.0 - left - right;
                    let height = 1.0 - top - bottom;
                    let sx = left.clamp(0.0, 1.0);
                    let sy = top.clamp(0.0, 1.0);
                    let sw = ((1.0 - right).clamp(0.0, 1.0) - sx).max(0.0);
                    let sh = ((1.0 - bottom).clamp(0.0, 1.0) - sy).max(0.0);
                    let [fill_left, fill_top, fill_right, fill_bottom] =
                        picture.fill.map(|value| f64::from(value) / 100_000.0);
                    let fill_width = (1.0 - fill_left - fill_right).max(0.0) * rect.width;
                    let fill_height = (1.0 - fill_top - fill_bottom).max(0.0) * rect.height;
                    let image_rect = if width > 0.0 && height > 0.0 {
                        Rect {
                            x: fill_left * rect.width + (sx - left) / width * fill_width,
                            y: fill_top * rect.height + (sy - top) / height * fill_height,
                            width: sw / width * fill_width,
                            height: sh / height * fill_height,
                        }
                    } else {
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 0.0,
                            height: 0.0,
                        }
                    };
                    PlacedPicture {
                        media: picture.media.clone(),
                        source: [sx, sy, sw, sh],
                        image_rect,
                        rotation_radians: (f64::from(picture.rotation) / 60_000.0).to_radians(),
                        flip_h: picture.flip_h,
                        flip_v: picture.flip_v,
                        opacity: (f64::from(picture.opacity) / 100_000.0).clamp(0.0, 1.0),
                        supported: picture.unsupported_properties.is_empty(),
                    }
                }),
                lines,
                font,
                style,
            })
        })
        .collect()
}

/// Where the sheet's noted cells are, so a painter can mark them.
///
/// A note on a cell past the used range still gets a marker in Excel, but
/// only where the axes reach; the axes already stretch to cover merges and
/// drawing anchors, and a note beyond even those has no geometry to mark.
fn place_notes(sheet: &Sheet, columns: &[AxisEntry], rows: &[AxisEntry]) -> Vec<PlacedNote> {
    sheet
        .comments
        .iter()
        .filter_map(|comment| {
            Some(PlacedNote {
                reference: comment.reference,
                rect: cell_rect(comment.reference, sheet, columns, rows)?,
                text: comment.text.clone(),
            })
        })
        .collect()
}

/// One past the furthest column or row any drawing anchor reaches, so a
/// shape hanging past the last written cell still has geometry under it.
fn shape_extent(sheet: &Sheet, axis: Axis) -> u32 {
    sheet
        .shapes
        .iter()
        .map(|shape| {
            let cell = shape.to.unwrap_or(shape.from);
            match axis {
                Axis::Column => cell.column + 1,
                Axis::Row => cell.row + 1,
            }
        })
        .max()
        .unwrap_or(0)
}

/// One past the furthest column or row any merged range reaches, so a merge
/// that extends past the last written cell still has geometry.
fn merge_extent(sheet: &Sheet, axis: Axis) -> u32 {
    sheet
        .merges
        .iter()
        .map(|range| match axis {
            Axis::Column => range.end.column + 1,
            Axis::Row => range.end.row + 1,
        })
        .max()
        .unwrap_or(0)
}

fn column_axis(sheet: &Sheet, count: u32, options: &LayoutOptions) -> Vec<AxisEntry> {
    let mut entries = Vec::with_capacity(count as usize);
    extend_columns(sheet, count, 0.0, options, &mut entries);
    entries
}

fn extend_columns(
    sheet: &Sheet,
    count: u32,
    extent: f64,
    options: &LayoutOptions,
    entries: &mut Vec<AxisEntry>,
) {
    let default_width = sheet
        .default_column_width_chars
        .unwrap_or(units::DEFAULT_COLUMN_WIDTH_CHARS);
    let mut offset = entries
        .last()
        .map(|entry| entry.offset + entry.size)
        .unwrap_or_default();
    let mut index = entries.len() as u32;
    while index < jade_xlsx_reader::model::MAX_COLUMN_COUNT && (index < count || offset < extent) {
        let span = sheet.column_span(index);
        let hidden = span.map(|span| span.hidden).unwrap_or(false);
        let width_chars = span
            .and_then(|span| span.width_chars)
            .unwrap_or(default_width);
        let size = if hidden {
            0.0
        } else {
            units::column_width_chars_to_px(width_chars, options.max_digit_width_px)
        };
        entries.push(AxisEntry {
            index,
            offset,
            size,
            hidden,
        });
        offset += size;
        index += 1;
    }
}

/// `auto_heights` is the height in points each row needs for its wrapped text,
/// zero where nothing wrapped.
fn row_axis(sheet: &Sheet, count: u32, auto_heights: &[f32]) -> Vec<AxisEntry> {
    let mut entries = Vec::with_capacity(count as usize);
    extend_rows(sheet, count, 0.0, auto_heights, &mut entries);
    entries
}

fn extend_rows(
    sheet: &Sheet,
    count: u32,
    extent: f64,
    auto_heights: &[f32],
    entries: &mut Vec<AxisEntry>,
) {
    let default_height = sheet
        .default_row_height_points
        .unwrap_or(units::DEFAULT_ROW_HEIGHT_POINTS);
    let mut offset = entries
        .last()
        .map(|entry| entry.offset + entry.size)
        .unwrap_or_default();
    // `Sheet::rows` is sorted by index, so the two walk together. Searching the
    // row list per index instead would be quadratic, which a sheet used down to
    // row 10000 turns into a hundred million comparisons for one layout.
    let mut next_row = 0usize;
    let mut index = entries.len() as u32;
    // Hidden defaults cannot extend a viewport by adding implicit rows. Only
    // walk to a later explicit visible row when one exists.
    let visible_end = if sheet.default_rows_hidden {
        sheet
            .rows
            .iter()
            .rev()
            .find(|row| !row.hidden)
            .map(|row| row.index + 1)
            .unwrap_or(0)
    } else {
        0
    };
    while index < jade_xlsx_reader::model::MAX_ROW_COUNT
        && (index < count.max(visible_end) || (!sheet.default_rows_hidden && offset < extent))
    {
        while next_row < sheet.rows.len() && sheet.rows[next_row].index < index {
            next_row += 1;
        }
        let row = sheet.rows.get(next_row).filter(|row| row.index == index);
        let hidden = row
            .map(|row| row.hidden)
            .unwrap_or(sheet.default_rows_hidden);
        let stated = row
            .and_then(|row| row.height_points)
            .unwrap_or(default_height);
        // A height the user pinned wins: Excel stops auto-fitting a row once
        // its height is set by hand, even if the text no longer fits.
        let height_points = if row
            .map(|row| row.custom_height)
            .unwrap_or(sheet.default_row_custom_height)
        {
            stated
        } else {
            stated.max(
                auto_heights
                    .get(index as usize)
                    .copied()
                    .unwrap_or_default(),
            )
        };
        let size = if hidden {
            0.0
        } else {
            units::points_to_px(height_points)
        };
        entries.push(AxisEntry {
            index,
            offset,
            size,
            hidden,
        });
        offset += size;
        index += 1;
    }
}
