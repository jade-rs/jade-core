//! The browser entry point for spreadsheets.
//!
//! Nothing here decides how a sheet looks. It opens the workbook with
//! `jade-xlsx-reader`, lays it out with `jade-xlsx-layout`, and hands the
//! result to `jade-xlsx-render`, which draws through `jade_painter::Painter` —
//! the same three crates the native Skia path uses. What this crate adds is
//! only the browser's half: a canvas, a device pixel ratio, and the JS types
//! either side of the boundary.
//!
//! That is the whole point of the `Painter` seam. A sheet renders in the
//! browser and in a native window from one implementation, and a difference
//! between the two would have to come from the backend rather than from a
//! second copy of the drawing code.

use jade_canvas_painter::Canvas2dPainter;
use jade_xlsx_layout::selection::{SelectionArea, SelectionKind};
use jade_xlsx_layout::{layout_workbook_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_render::{render_grid, PainterMeasure, RenderOptions};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(
    inline_js = "export function browser_time_zone() { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; }"
)]
extern "C" {
    fn browser_time_zone() -> String;
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_time_zone() -> String {
    "UTC".to_string()
}
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap};

/// Columns and rows drawn past the last one the sheet writes a cell in, so a
/// small sheet still reads as a grid rather than a floating block of cells.
const MIN_COLUMNS: u32 = 12;
const MIN_ROWS: u32 = 30;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// A workbook held open on the JS side.
///
/// Held as a [`jade_xlsx_headless_editor::HeadlessWorkbook`] rather than the
/// bare model: editing goes through the same writer-and-recalculate path the
/// headless callers use, so a value typed into the browser patches the
/// original bytes and moves its dependent formulas exactly the way an agent's
/// `set_cell` does.
#[wasm_bindgen]
pub struct Workbook {
    inner: jade_xlsx_headless_editor::HeadlessWorkbook,
    font_metrics_canvas: RefCell<Option<HtmlCanvasElement>>,
    font_metrics_families: RefCell<Option<std::collections::HashSet<String>>>,
    measured_normal_family: RefCell<Option<String>>,
    /// The selected range per sheet, as the drag anchor and the focus it was
    /// extended to. A single selected cell is the pair being equal; the anchor
    /// is the active cell — the one edits land in.
    selection: RefCell<BTreeMap<usize, (jade_xlsx_reader::CellRef, jade_xlsx_reader::CellRef)>>,
    selection_kinds: RefCell<BTreeMap<usize, SelectionKind>>,
    /// The last sheet laid out. Scrolling repaints many times against one
    /// sheet, and laying out ten thousand rows per frame would undo the point
    /// of only painting a screenful.
    cached: RefCell<Option<(usize, Rc<SheetLayout>)>>,
    /// What the last copy or cut took, so a paste of the very text we wrote
    /// to the clipboard can carry formulas rather than their display text.
    clipboard: RefCell<Option<InternalClipboard>>,
    /// A fill-handle drag in progress: the sheet and the rectangle the drop
    /// would fill (source range included).
    fill_preview: RefCell<Option<(usize, jade_xlsx_reader::CellRange)>>,
    resize_preview: RefCell<Option<PendingResize>>,
    /// Whether the sheet paints in the dark theme.
    dark: RefCell<bool>,
    images: RefCell<HashMap<String, ImageBitmap>>,
    image_errors: RefCell<HashMap<String, String>>,
}

#[derive(Clone, Copy)]
struct PendingResize {
    sheet: usize,
    first: u32,
    last: u32,
    gesture: jade_xlsx_layout::resize::ResizeGesture,
    digit_width_px: f64,
}

impl Drop for Workbook {
    fn drop(&mut self) {
        for image in self.images.get_mut().values() {
            image.close();
        }
    }
}

/// The source of the last copy or cut.
struct InternalClipboard {
    kind: SelectionKind,
    snapshot: jade_xlsx_headless_editor::ClipboardSnapshot,
    cut: bool,
    /// The TSV written to the system clipboard at copy time. A paste whose
    /// text differs came from another application and pastes as values.
    tsv: String,
}

/// The selected cell, as a formula bar wants it.
#[wasm_bindgen]
pub struct Selection {
    reference: String,
    range: String,
    text: String,
    formula: Option<String>,
    formula_state: Option<jade_xlsx_headless_api::FormulaState>,
    row: u32,
    column: u32,
    kind: SelectionKind,
    selected_range: jade_xlsx_reader::CellRange,
}

#[wasm_bindgen]
impl Selection {
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.as_str().to_string()
    }
    #[wasm_bindgen(getter,js_name=startRow)]
    pub fn start_row(&self) -> u32 {
        self.selected_range.start.row
    }
    #[wasm_bindgen(getter,js_name=endRow)]
    pub fn end_row(&self) -> u32 {
        self.selected_range.end.row
    }
    #[wasm_bindgen(getter,js_name=startColumn)]
    pub fn start_column(&self) -> u32 {
        self.selected_range.start.column
    }
    #[wasm_bindgen(getter,js_name=endColumn)]
    pub fn end_column(&self) -> u32 {
        self.selected_range.end.column
    }
    /// `cached`, `stale`, or `uncalculated`; absent for a value cell.
    #[wasm_bindgen(getter, js_name = formulaState)]
    pub fn formula_state(&self) -> Option<String> {
        self.formula_state.as_ref().map(|state| {
            match state {
                jade_xlsx_headless_api::FormulaState::Cached => "cached",
                jade_xlsx_headless_api::FormulaState::Stale => "stale",
                jade_xlsx_headless_api::FormulaState::Uncalculated => "uncalculated",
            }
            .to_string()
        })
    }
    /// `B3`, the way a spreadsheet names the cell.
    #[wasm_bindgen(getter)]
    pub fn reference(&self) -> String {
        self.reference.clone()
    }

    /// `B3:D5` when more than one cell is selected, otherwise the same as
    /// [`Selection::reference`].
    #[wasm_bindgen(getter)]
    pub fn range(&self) -> String {
        self.range.clone()
    }

    /// What the cell shows, after number formatting.
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> String {
        self.text.clone()
    }

    /// The formula behind the value, without its `=`.
    #[wasm_bindgen(getter)]
    pub fn formula(&self) -> Option<String> {
        self.formula.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn row(&self) -> u32 {
        self.row
    }

    #[wasm_bindgen(getter)]
    pub fn column(&self) -> u32 {
        self.column
    }
}

/// A cell's rectangle in canvas coordinates, for positioning an editor.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct CellRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[wasm_bindgen]
impl CellRect {
    #[wasm_bindgen(getter)]
    pub fn x(&self) -> f64 {
        self.x
    }

    #[wasm_bindgen(getter)]
    pub fn y(&self) -> f64 {
        self.y
    }

    #[wasm_bindgen(getter)]
    pub fn width(&self) -> f64 {
        self.width
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> f64 {
        self.height
    }
}

#[wasm_bindgen]
pub struct ScrollPosition {
    scroll_x: f64,
    scroll_y: f64,
}

#[wasm_bindgen]
impl ScrollPosition {
    #[wasm_bindgen(getter, js_name = scrollX)]
    pub fn scroll_x(&self) -> f64 {
        self.scroll_x
    }

    #[wasm_bindgen(getter, js_name = scrollY)]
    pub fn scroll_y(&self) -> f64 {
        self.scroll_y
    }
}

/// A drawing object a click landed on: what it is, what it says, and where
/// it is on the canvas, so a shell can put an editor over it.
#[wasm_bindgen]
pub struct ShapeHit {
    id: u32,
    name: String,
    kind: String,
    text: Option<String>,
    rect: CellRect,
}

#[wasm_bindgen]
impl ShapeHit {
    /// The drawing's own id for the object, stable across edits.
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> u32 {
        self.id
    }

    /// `TextBox 1`, as Excel names it — and what `setShapeText` takes.
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// `text_box`, `shape`, `picture`, `chart`, `graphic_frame`, `group` or
    /// `connector` — the names the headless summary uses.
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.clone()
    }

    /// The object's text, paragraphs joined with newlines. Absent for a
    /// picture or a chart, which have no text body to edit.
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> Option<String> {
        self.text.clone()
    }

    /// Where the object is on the canvas, the header bands included.
    #[wasm_bindgen(getter)]
    pub fn rect(&self) -> CellRect {
        self.rect
    }
}

/// The name the headless summary gives a drawing object's kind, so a shell
/// reading `shapeAt` and one reading `sheetObjects` see the same word.
fn shape_kind_name(kind: jade_xlsx_reader::ShapeKind) -> &'static str {
    match kind {
        jade_xlsx_reader::ShapeKind::TextBox => "text_box",
        jade_xlsx_reader::ShapeKind::Shape => "shape",
        jade_xlsx_reader::ShapeKind::Picture => "picture",
        jade_xlsx_reader::ShapeKind::Chart => "chart",
        jade_xlsx_reader::ShapeKind::GraphicFrame => "graphic_frame",
        jade_xlsx_reader::ShapeKind::Group => "group",
        jade_xlsx_reader::ShapeKind::Connector => "connector",
    }
}

/// What an editor drawn over the active cell should look like: the cell's
/// font, and the colours the cell is painted with under the current theme.
#[wasm_bindgen]
pub struct CellStyle {
    font: String,
    background: String,
    color: String,
    text_decoration: String,
}

#[wasm_bindgen]
impl CellStyle {
    #[wasm_bindgen(getter, js_name = textDecoration)]
    pub fn text_decoration(&self) -> String {
        self.text_decoration.clone()
    }
    /// A CSS font shorthand.
    #[wasm_bindgen(getter)]
    pub fn font(&self) -> String {
        self.font.clone()
    }

    /// The cell's painted background, as CSS.
    #[wasm_bindgen(getter)]
    pub fn background(&self) -> String {
        self.background.clone()
    }

    /// The cell's text colour, as CSS.
    #[wasm_bindgen(getter)]
    pub fn color(&self) -> String {
        self.color.clone()
    }
}

/// A sheet's full size, for sizing a scroll area without painting anything.
#[wasm_bindgen]
pub struct SheetSize {
    width_px: f64,
    height_px: f64,
}

#[wasm_bindgen]
impl SheetSize {
    #[wasm_bindgen(getter, js_name = widthPx)]
    pub fn width_px(&self) -> f64 {
        self.width_px
    }

    #[wasm_bindgen(getter, js_name = heightPx)]
    pub fn height_px(&self) -> f64 {
        self.height_px
    }
}

/// What one `renderViewport` call painted, so the page can report it and a test
/// can assert something other than pixels.
#[wasm_bindgen]
pub struct ViewportRender {
    drawn_cells: usize,
    first_row: u32,
    last_row: u32,
    first_column: u32,
    last_column: u32,
    visible: bool,
}

#[wasm_bindgen]
impl ViewportRender {
    /// Cells painted. A merged range counts once.
    #[wasm_bindgen(getter, js_name = drawnCells)]
    pub fn drawn_cells(&self) -> usize {
        self.drawn_cells
    }

    /// Whether the viewport touched the grid at all.
    #[wasm_bindgen(getter)]
    pub fn visible(&self) -> bool {
        self.visible
    }

    #[wasm_bindgen(getter, js_name = firstRow)]
    pub fn first_row(&self) -> u32 {
        self.first_row
    }

    #[wasm_bindgen(getter, js_name = lastRow)]
    pub fn last_row(&self) -> u32 {
        self.last_row
    }

    #[wasm_bindgen(getter, js_name = firstColumn)]
    pub fn first_column(&self) -> u32 {
        self.first_column
    }

    #[wasm_bindgen(getter, js_name = lastColumn)]
    pub fn last_column(&self) -> u32 {
        self.last_column
    }
}

#[wasm_bindgen]
impl Workbook {
    /// Reads `.xlsx` bytes.
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: &[u8]) -> Result<Workbook, JsValue> {
        let inner = jade_xlsx_headless_editor::HeadlessWorkbook::open_in_time_zone(
            bytes,
            &browser_time_zone(),
        )
        .map_err(|error| JsValue::from_str(&format!("cannot read the workbook: {error}")))?;
        Ok(Workbook {
            inner,
            font_metrics_canvas: RefCell::new(None),
            font_metrics_families: RefCell::new(None),
            measured_normal_family: RefCell::new(None),
            selection: RefCell::new(BTreeMap::new()),
            selection_kinds: RefCell::new(BTreeMap::new()),
            cached: RefCell::new(None),
            clipboard: RefCell::new(None),
            fill_preview: RefCell::new(None),
            resize_preview: RefCell::new(None),
            dark: RefCell::new(false),
            images: RefCell::new(HashMap::new()),
            image_errors: RefCell::new(HashMap::new()),
        })
    }

    #[wasm_bindgen(js_name = calculationTimeZone)]
    pub fn calculation_time_zone(&self) -> String {
        self.inner.calculation_time_zone().to_string()
    }

    /// Configure future evaluations without changing stored workbook bytes.
    #[wasm_bindgen(js_name = setCalculationTimeZone)]
    pub fn set_calculation_time_zone(&mut self, time_zone: &str) -> Result<(), JsValue> {
        self.inner
            .set_calculation_time_zone(time_zone)
            .map_err(|error| {
                JsValue::from_str(&format!("cannot set calculation time zone: {error}"))
            })
    }

    /// Sheet names in workbook order, which is the order the tabs appear in.
    #[wasm_bindgen(js_name = sheetNames)]
    pub fn sheet_names(&self) -> Vec<String> {
        self.model()
            .sheets
            .iter()
            .map(|sheet| sheet.name.clone())
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = headerResizeAxisAt)]
    pub fn header_resize_axis_at(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<Option<String>, JsValue> {
        let layout = self.layout(sheet, canvas)?;
        Ok(
            jade_xlsx_layout::resize::header_resize_hit(&layout, x, y, scroll_x, scroll_y).map(
                |hit| {
                    if hit.axis == jade_xlsx_reader::Axis::Rows {
                        "rows"
                    } else {
                        "columns"
                    }
                    .into()
                },
            ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = beginResizeAt)]
    pub fn begin_resize_at(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<bool, JsValue> {
        let layout = self.layout(sheet, canvas)?;
        let Some(hit) =
            jade_xlsx_layout::resize::header_resize_hit(&layout, x, y, scroll_x, scroll_y)
        else {
            return Ok(false);
        };
        if self.selected_area(sheet).is_none() {
            let reference = if hit.axis == jade_xlsx_reader::Axis::Rows {
                jade_xlsx_reader::CellRef::new(0, hit.index)
            } else {
                jade_xlsx_reader::CellRef::new(hit.index, 0)
            };
            self.choose_area(sheet, SelectionKind::Cells, reference, reference);
        }
        let (first, last) = self
            .selected_area(sheet)
            .filter(|area| {
                area.kind == SelectionKind::Sheet
                    || (hit.axis == jade_xlsx_reader::Axis::Rows
                        && area.kind == SelectionKind::Rows)
                    || (hit.axis == jade_xlsx_reader::Axis::Columns
                        && area.kind == SelectionKind::Columns)
            })
            .map(|area| {
                let range = area.range();
                if hit.axis == jade_xlsx_reader::Axis::Rows {
                    (range.start.row, range.end.row)
                } else {
                    (range.start.column, range.end.column)
                }
            })
            .filter(|(first, last)| hit.index >= *first && hit.index <= *last)
            .unwrap_or((hit.index, hit.index));
        *self.resize_preview.borrow_mut() = Some(PendingResize {
            sheet,
            first,
            last,
            digit_width_px: layout.max_digit_width_px,
            gesture: jade_xlsx_layout::resize::ResizeGesture {
                hit,
                pointer_start: if hit.axis == jade_xlsx_reader::Axis::Rows {
                    y
                } else {
                    x
                },
                size_px: hit.size_px,
            },
        });
        *self.fill_preview.borrow_mut() = None;
        Ok(true)
    }
    #[wasm_bindgen(js_name = previewResizeTo)]
    pub fn preview_resize_to(&self, x: f64, y: f64) -> bool {
        let mut pending = self.resize_preview.borrow_mut();
        let Some(resize) = pending.as_mut() else {
            return false;
        };
        resize.gesture.update(x, y);
        let size = resize.gesture.stored_size(resize.digit_width_px);
        resize.gesture.size_px = if resize.gesture.hit.axis == jade_xlsx_reader::Axis::Rows {
            jade_xlsx_layout::units::points_to_px(size)
        } else {
            jade_xlsx_layout::units::column_width_chars_to_px(size, resize.digit_width_px)
        };
        true
    }
    #[wasm_bindgen(js_name = cancelResize)]
    pub fn cancel_resize(&self) {
        *self.resize_preview.borrow_mut() = None;
    }

    #[wasm_bindgen(js_name = commitResize)]
    pub fn commit_resize(&mut self) -> Result<bool, JsValue> {
        let Some(resize) = self.resize_preview.borrow_mut().take() else {
            return Ok(false);
        };
        let size = resize.gesture.stored_size(resize.digit_width_px);
        let original = jade_xlsx_layout::resize::ResizeGesture {
            size_px: resize.gesture.hit.size_px,
            ..resize.gesture
        }
        .stored_size(resize.digit_width_px);
        if size == original {
            return Ok(false);
        }
        let name = self.model().sheets[resize.sheet].name.clone();
        self.inner
            .set_axis_size(
                &name,
                resize.gesture.hit.axis,
                resize.first,
                resize.last,
                size,
            )
            .map_err(|error| JsValue::from_str(&format!("cannot resize: {error}")))?;
        *self.cached.borrow_mut() = None;
        Ok(true)
    }

    #[wasm_bindgen(js_name = setColumnWidth)]
    pub fn set_column_width(
        &mut self,
        sheet: usize,
        first: u32,
        last: u32,
        width: f32,
    ) -> Result<(), JsValue> {
        self.set_axis_size(sheet, jade_xlsx_reader::Axis::Columns, first, last, width)
    }
    #[wasm_bindgen(js_name = setRowHeight)]
    pub fn set_row_height(
        &mut self,
        sheet: usize,
        first: u32,
        last: u32,
        height: f32,
    ) -> Result<(), JsValue> {
        self.set_axis_size(sheet, jade_xlsx_reader::Axis::Rows, first, last, height)
    }
    fn set_axis_size(
        &mut self,
        sheet: usize,
        axis: jade_xlsx_reader::Axis,
        first: u32,
        last: u32,
        size: f32,
    ) -> Result<(), JsValue> {
        let name = self
            .model()
            .sheets
            .get(sheet)
            .ok_or_else(|| JsValue::from_str("sheet is gone"))?
            .name
            .clone();
        self.inner
            .set_axis_size(&name, axis, first, last, size)
            .map_err(|error| JsValue::from_str(&format!("cannot resize: {error}")))?;
        *self.cached.borrow_mut() = None;
        *self.resize_preview.borrow_mut() = None;
        Ok(())
    }

    #[wasm_bindgen(js_name = refreshAutomaticRowHeights)]
    pub fn refresh_automatic_row_heights(
        &mut self,
        canvas: &HtmlCanvasElement,
    ) -> Result<bool, JsValue> {
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("canvas context missing"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let options = self.layout_options(context);
        let changed = self
            .inner
            .refresh_automatic_row_heights(&options)
            .map_err(|error| JsValue::from_str(&format!("cannot refresh row heights: {error}")))?;
        if changed {
            *self.cached.borrow_mut() = None;
        }
        Ok(changed)
    }

    #[wasm_bindgen(js_name = usesFontFamily)]
    pub fn uses_font_family(&self, family: &str) -> bool {
        let family = family.trim().trim_matches(['\'', '"']);
        family.eq_ignore_ascii_case("Calibri")
            || self.model().styles.fonts.iter().any(|font| {
                font.name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(family))
            })
    }

    /// Font metrics are external state, retained across exact history restores.
    #[wasm_bindgen(js_name = refreshFontMetrics)]
    pub fn refresh_font_metrics(&self, canvas: &HtmlCanvasElement, family: Option<String>) {
        let initial = self.font_metrics_canvas.borrow().is_none();
        let mut families = self.font_metrics_families.borrow_mut();
        if let Some(family) = family {
            let family = family.trim().trim_matches(['\'', '"']).to_string();
            let normal =
                jade_xlsx_layout::resolve_normal_font(&self.model().styles, &Default::default());
            if normal.family.eq_ignore_ascii_case(&family) {
                *self.measured_normal_family.borrow_mut() = Some(family.clone());
            }
            if initial {
                *families = Some(std::collections::HashSet::new());
            }
            if let Some(families) = families.as_mut() {
                families.insert(family);
            }
        } else {
            *families = None;
        }
        *self.font_metrics_canvas.borrow_mut() = Some(canvas.clone());
        *self.cached.borrow_mut() = None;
        *self.resize_preview.borrow_mut() = None;
    }

    #[wasm_bindgen(js_name = recalculate)]
    pub fn recalculate(&mut self, canvas: &HtmlCanvasElement) -> Result<String, JsValue> {
        let result = self
            .inner
            .recalculate()
            .map_err(|error| JsValue::from_str(&format!("cannot recalculate: {error}")))?;
        *self.cached.borrow_mut() = None;
        self.refresh_automatic_row_heights(canvas)?;
        *self.resize_preview.borrow_mut() = None;
        *self.fill_preview.borrow_mut() = None;
        serde_json::to_string(&result).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Call after configuring the host zone and before the first editable view.
    #[wasm_bindgen(js_name = recalculateOnOpen)]
    pub fn recalculate_on_open(
        &mut self,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<String>, JsValue> {
        let Some(result) = self.inner.recalculate_on_open().map_err(|error| {
            JsValue::from_str(&format!("cannot initialize calculation: {error}"))
        })?
        else {
            return Ok(None);
        };
        *self.cached.borrow_mut() = None;
        self.refresh_automatic_row_heights(canvas)?;
        serde_json::to_string(&result)
            .map(Some)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = setCalculationProperties)]
    pub fn set_calculation_properties(
        &mut self,
        canvas: &HtmlCanvasElement,
        mode: &str,
        calculate_on_save: bool,
    ) -> Result<(), JsValue> {
        let mode = jade_xlsx_reader::CalculationMode::parse(mode)
            .ok_or_else(|| JsValue::from_str("invalid calculation mode"))?;
        self.inner
            .set_calculation_properties(mode, calculate_on_save)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        *self.cached.borrow_mut() = None;
        *self.resize_preview.borrow_mut() = None;
        *self.fill_preview.borrow_mut() = None;
        self.refresh_automatic_row_heights(canvas)?;
        Ok(())
    }

    #[wasm_bindgen(js_name = saveBytes)]
    pub fn save_bytes(&mut self, canvas: &HtmlCanvasElement) -> Result<Vec<u8>, JsValue> {
        self.inner
            .prepare_for_save()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        *self.cached.borrow_mut() = None;
        *self.resize_preview.borrow_mut() = None;
        *self.fill_preview.borrow_mut() = None;
        self.refresh_automatic_row_heights(canvas)?;
        self.export_bytes()
    }

    #[wasm_bindgen(js_name = autoFitColumns)]
    pub fn auto_fit_columns(
        &mut self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        first: u32,
        last: u32,
    ) -> Result<(), JsValue> {
        self.auto_fit_axes(sheet, canvas, jade_xlsx_reader::Axis::Columns, first, last)
    }
    #[wasm_bindgen(js_name = autoFitRows)]
    pub fn auto_fit_rows(
        &mut self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        first: u32,
        last: u32,
    ) -> Result<(), JsValue> {
        self.auto_fit_axes(sheet, canvas, jade_xlsx_reader::Axis::Rows, first, last)
    }
    fn auto_fit_axes(
        &mut self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        axis: jade_xlsx_reader::Axis,
        first: u32,
        last: u32,
    ) -> Result<(), JsValue> {
        let maximum = if axis == jade_xlsx_reader::Axis::Rows {
            jade_xlsx_reader::model::MAX_ROW_COUNT
        } else {
            jade_xlsx_reader::model::MAX_COLUMN_COUNT
        };
        let all_rows = axis == jade_xlsx_reader::Axis::Rows && first == 0 && last == maximum - 1;
        if first > last || last >= maximum || (!all_rows && last - first >= 1_000_000) {
            return Err(JsValue::from_str("axis range exceeds the resize limit"));
        }
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("canvas context missing"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let options = self.layout_options(context);
        let model = self.model();
        let source = model
            .sheets
            .get(sheet)
            .ok_or_else(|| JsValue::from_str("sheet is gone"))?;
        let name = source.name.clone();
        let written = if all_rows {
            let fit = jade_xlsx_layout::resize::auto_fit_all_rows(source, &model.styles, &options);
            self.inner
                .set_row_sizes_with_default(&name, fit.default_points, &fit.overrides, false)
        } else if axis == jade_xlsx_reader::Axis::Rows {
            let sizes = jade_xlsx_layout::resize::auto_fit_rows(
                source,
                &model.styles,
                first,
                last,
                &options,
            );
            self.inner.set_fitted_row_sizes(&name, &sizes)
        } else {
            let sizes = jade_xlsx_layout::resize::auto_fit_columns(
                source,
                &model.styles,
                first,
                last,
                &options,
            );
            self.inner.set_axis_sizes(&name, axis, &sizes)
        };
        written.map_err(|error| JsValue::from_str(&format!("cannot auto-fit: {error}")))?;
        *self.cached.borrow_mut() = None;
        *self.resize_preview.borrow_mut() = None;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = autoFitAt)]
    pub fn auto_fit_at(
        &mut self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<bool, JsValue> {
        if !self.begin_resize_at(sheet, canvas, x, y, scroll_x, scroll_y)? {
            return Ok(false);
        }
        let resize = self
            .resize_preview
            .borrow_mut()
            .take()
            .expect("resize started");
        self.auto_fit_axes(
            sheet,
            canvas,
            resize.gesture.hit.axis,
            resize.first,
            resize.last,
        )?;
        Ok(true)
    }

    /// Workbook indices of sheets a spreadsheet exposes as selectable tabs.
    /// Hidden and very-hidden sheets remain in the model so formulas can refer
    /// to them, but opening them from the normal tab strip would disagree with
    /// Excel.
    #[wasm_bindgen(js_name = visibleSheetIndices)]
    pub fn visible_sheet_indices(&self) -> Vec<u32> {
        self.model()
            .sheets
            .iter()
            .enumerate()
            .filter_map(|(index, sheet)| {
                (sheet.visibility == jade_xlsx_reader::SheetVisibility::Visible)
                    .then_some(index as u32)
            })
            .collect()
    }

    /// The tab Excel saved as active, if it still names a sheet in this file.
    #[wasm_bindgen(js_name = activeSheetIndex)]
    pub fn active_sheet_index(&self) -> Option<u32> {
        self.model()
            .active_sheet_index
            .filter(|index| *index < self.model().sheets.len())
            .map(|index| index as u32)
    }

    /// The sheet's full size in CSS pixels, for sizing a scroll area.
    ///
    /// Takes the canvas because the size depends on the layout, and the layout
    /// depends on measuring text against the font that will draw it: a wrapped
    /// row's height is whatever its lines come to.
    #[wasm_bindgen(js_name = sheetSize)]
    pub fn sheet_size(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<SheetSize, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;
        Ok(SheetSize {
            width_px: layout.content_width,
            height_px: layout.content_height,
        })
    }

    /// The size of the header bands, so the page can inset its scroll area to
    /// match the part of the sheet that is actually on show.
    #[wasm_bindgen(js_name = headerMetrics)]
    pub fn header_metrics(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<SheetSize, JsValue> {
        let metrics = self.layout(sheet_index, canvas)?.header_metrics();
        Ok(SheetSize {
            width_px: metrics.row_width,
            height_px: metrics.column_height,
        })
    }

    fn selected_area(&self, sheet: usize) -> Option<SelectionArea> {
        let (anchor, focus) = self.selection.borrow().get(&sheet).copied()?;
        Some(SelectionArea {
            kind: self
                .selection_kinds
                .borrow()
                .get(&sheet)
                .copied()
                .unwrap_or_default(),
            anchor,
            focus,
        })
    }
    fn choose_area(
        &self,
        sheet: usize,
        kind: SelectionKind,
        anchor: jade_xlsx_reader::CellRef,
        focus: jade_xlsx_reader::CellRef,
    ) {
        self.selection.borrow_mut().insert(sheet, (anchor, focus));
        self.selection_kinds.borrow_mut().insert(sheet, kind);
        *self.fill_preview.borrow_mut() = None;
    }
    #[wasm_bindgen(js_name=selectRows)]
    pub fn select_rows(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        start: u32,
        end: u32,
    ) -> Result<Option<Selection>, JsValue> {
        self.select_axes(sheet, canvas, SelectionKind::Rows, start, end)
    }
    #[wasm_bindgen(js_name=selectColumns)]
    pub fn select_columns(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        start: u32,
        end: u32,
    ) -> Result<Option<Selection>, JsValue> {
        self.select_axes(sheet, canvas, SelectionKind::Columns, start, end)
    }
    #[wasm_bindgen(js_name=selectAll)]
    pub fn select_all(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<Selection>, JsValue> {
        let layout = self.layout(sheet, canvas)?;
        let active = self
            .selected_area(sheet)
            .map(|area| area.anchor)
            .unwrap_or(jade_xlsx_reader::CellRef::new(0, 0));
        self.choose_area(sheet, SelectionKind::Sheet, active, active);
        Ok(Some(self.describe(sheet, active, &layout)))
    }
    fn select_axes(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        kind: SelectionKind,
        start: u32,
        end: u32,
    ) -> Result<Option<Selection>, JsValue> {
        let limit = if kind == SelectionKind::Rows {
            jade_xlsx_reader::model::MAX_ROW_COUNT
        } else {
            jade_xlsx_reader::model::MAX_COLUMN_COUNT
        };
        if start >= limit || end >= limit {
            return Err(JsValue::from_str("selection is outside the worksheet"));
        }
        let layout = self.layout(sheet, canvas)?;
        let active = self
            .selected_area(sheet)
            .map(|area| area.anchor)
            .unwrap_or(jade_xlsx_reader::CellRef::new(0, 0));
        let (anchor, focus) = if kind == SelectionKind::Rows {
            (
                jade_xlsx_reader::CellRef::new(active.column, start),
                jade_xlsx_reader::CellRef::new(active.column, end),
            )
        } else {
            (
                jade_xlsx_reader::CellRef::new(start, active.row),
                jade_xlsx_reader::CellRef::new(end, active.row),
            )
        };
        self.choose_area(sheet, kind, anchor, focus);
        Ok(Some(self.describe(sheet, anchor, &layout)))
    }

    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name=contextSelectAt)]
    pub fn context_select_at(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<Option<Selection>, JsValue> {
        let layout = self.layout(sheet, canvas)?;
        if let Some(hit) =
            jade_xlsx_layout::selection::header_hit(&layout, x, y, scroll_x, scroll_y)
        {
            if let Some(area) = self
                .selected_area(sheet)
                .filter(|area| area.kind == hit.kind && area.range().contains(hit.reference))
            {
                return Ok(Some(self.describe(sheet, area.anchor, &layout)));
            }
            return self.select_header_at(sheet, canvas, x, y, scroll_x, scroll_y, false);
        }
        let metrics = layout.header_metrics();
        let (sx, sy) = layout.sheet_point_in_viewport(
            x - metrics.row_width,
            y - metrics.column_height,
            scroll_x,
            scroll_y,
        );
        if let (Some(column), Some(row), Some(area)) = (
            layout.column_at(sx),
            layout.row_at(sy),
            self.selected_area(sheet),
        ) {
            if area
                .range()
                .contains(jade_xlsx_reader::CellRef::new(column, row))
            {
                return Ok(Some(self.describe(sheet, area.anchor, &layout)));
            }
        }
        self.select_at(sheet, canvas, x, y, scroll_x, scroll_y)
    }
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name=selectHeaderAt)]
    pub fn select_header_at(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
        extend: bool,
    ) -> Result<Option<Selection>, JsValue> {
        let layout = self.layout(sheet, canvas)?;
        let Some(hit) = jade_xlsx_layout::selection::header_hit(&layout, x, y, scroll_x, scroll_y)
        else {
            return Ok(None);
        };
        if hit.kind == SelectionKind::Sheet {
            return self.select_all(sheet, canvas);
        }
        let old = self.selected_area(sheet);
        let anchor = if extend {
            old.map(|area| area.anchor).unwrap_or(hit.reference)
        } else {
            hit.reference
        };
        let focus = match hit.kind {
            SelectionKind::Rows => jade_xlsx_reader::CellRef::new(anchor.column, hit.reference.row),
            SelectionKind::Columns => {
                jade_xlsx_reader::CellRef::new(hit.reference.column, anchor.row)
            }
            _ => anchor,
        };
        self.choose_area(sheet, hit.kind, anchor, focus);
        Ok(Some(self.describe(sheet, anchor, &layout)))
    }

    /// Selects the cell under a point on the canvas, and reports it.
    ///
    /// `x`/`y` are in canvas coordinates — the bands included — and the scroll
    /// position turns them into the sheet's. A point in a band selects nothing,
    /// which is why the return is optional.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = selectAt)]
    pub fn select_at(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<Option<Selection>, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;
        let metrics = layout.header_metrics();
        if x < metrics.row_width || y < metrics.column_height {
            self.selection.borrow_mut().remove(&sheet_index);
            return Ok(None);
        }
        let (sheet_x, sheet_y) = layout.sheet_point_in_viewport(
            x - metrics.row_width,
            y - metrics.column_height,
            scroll_x,
            scroll_y,
        );
        let Some(reference) = layout.cell_at(sheet_x, sheet_y) else {
            self.selection.borrow_mut().remove(&sheet_index);
            return Ok(None);
        };
        self.choose_area(sheet_index, SelectionKind::Cells, reference, reference);
        Ok(Some(self.describe(sheet_index, reference, &layout)))
    }

    /// Extends the selection from its anchor to the cell under a point,
    /// leaving the anchor — and with it the active cell — where it was.
    /// A point in the bands, or with nothing selected, changes nothing.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = extendTo)]
    pub fn extend_to(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<Option<Selection>, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;
        let Some((anchor, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let metrics = layout.header_metrics();
        if x < metrics.row_width || y < metrics.column_height {
            if self.selected_area(sheet_index).is_some_and(|area| {
                matches!(area.kind, SelectionKind::Rows | SelectionKind::Columns)
            }) {
                return self.select_header_at(sheet_index, canvas, x, y, scroll_x, scroll_y, true);
            }
            return Ok(None);
        }
        let (sheet_x, sheet_y) = layout.sheet_point_in_viewport(
            x - metrics.row_width,
            y - metrics.column_height,
            scroll_x,
            scroll_y,
        );
        let kind = self
            .selected_area(sheet_index)
            .map(|area| area.kind)
            .unwrap_or_default();
        let focus = if kind == SelectionKind::Rows {
            jade_xlsx_reader::CellRef::new(
                anchor.column,
                layout
                    .row_at(sheet_y)
                    .ok_or_else(|| JsValue::from_str("row is outside the layout"))?,
            )
        } else if kind == SelectionKind::Columns {
            jade_xlsx_reader::CellRef::new(
                layout
                    .column_at(sheet_x)
                    .ok_or_else(|| JsValue::from_str("column is outside the layout"))?,
                anchor.row,
            )
        } else {
            let Some(focus) = layout.cell_at(sheet_x, sheet_y) else {
                return Ok(None);
            };
            focus
        };
        self.choose_area(sheet_index, kind, anchor, focus);
        Ok(Some(self.describe(sheet_index, anchor, &layout)))
    }

    /// What is in the selected cell, or `None` when nothing is selected.
    #[wasm_bindgen(js_name = selection)]
    pub fn selection(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<Selection>, JsValue> {
        let Some((anchor, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let layout = self.layout(sheet_index, canvas)?;
        Ok(Some(self.describe(sheet_index, anchor, &layout)))
    }

    /// A row's height in CSS pixels.
    ///
    /// A row that wraps is as tall as its text came to, which is only known
    /// after measuring — so this reports what layout decided rather than what
    /// the file said.
    #[wasm_bindgen(js_name = rowHeightPx)]
    pub fn row_height_px(
        &self,
        sheet_index: usize,
        row_index: u32,
        canvas: &HtmlCanvasElement,
    ) -> Result<f64, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;
        Ok(layout
            .row(row_index)
            .map(|row| row.size)
            .unwrap_or_default())
    }

    /// Paints the part of a sheet under `scroll_x`/`scroll_y` onto a canvas the
    /// size of the viewport.
    ///
    /// The canvas stays the size of the window rather than the size of the
    /// sheet. A sheet used down to row 10000 is a quarter of a million pixels
    /// tall — past what a browser will allocate — so painting the whole grid is
    /// not an option that gets slower, it is one that fails.
    ///
    /// `device_pixel_ratio` scales the backing store so grid lines land on
    /// device pixels; the canvas keeps its CSS size in layout pixels.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = renderViewport)]
    pub fn render_viewport(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        device_pixel_ratio: f64,
        scroll_x: f64,
        scroll_y: f64,
        viewport_width: f64,
        viewport_height: f64,
    ) -> Result<ViewportRender, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;

        let ratio = if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
            device_pixel_ratio
        } else {
            1.0
        };
        let metrics = layout.header_metrics();
        // The canvas holds the bands as well as the sheet, so the part of the
        // sheet on show is smaller than the surface by exactly the bands.
        let viewport = Rect {
            x: scroll_x.max(0.0),
            y: scroll_y.max(0.0),
            width: (viewport_width - metrics.row_width).max(0.0),
            height: (viewport_height - metrics.column_height).max(0.0),
        };
        size_canvas(
            canvas,
            viewport_width.max(0.0),
            viewport_height.max(0.0),
            ratio,
        )?;

        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("the canvas has no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        context.scale(ratio, ratio)?;

        let images = self.images.borrow();
        let painter = Canvas2dPainter::with_images(&context, &images);
        let selection =
            self.selection
                .borrow()
                .get(&sheet_index)
                .copied()
                .map(|(anchor, _focus)| {
                    let area = self.selected_area(sheet_index).expect("selection exists");
                    let range = area.range();
                    let mut selection =
                        jade_xlsx_render::GridSelection::range(range.start, range.end);
                    selection.active = anchor;
                    if area.kind != SelectionKind::Cells {
                        selection = selection.without_fill_handle();
                    }
                    match *self.fill_preview.borrow() {
                        Some((sheet, preview)) if sheet == sheet_index => {
                            selection.with_fill_preview(preview)
                        }
                        _ => selection,
                    }
                });
        render_grid(
            &painter,
            &layout,
            viewport,
            selection,
            &self.render_options(),
        )?;
        if let Some(resize) = self
            .resize_preview
            .borrow()
            .as_ref()
            .filter(|resize| resize.sheet == sheet_index)
        {
            jade_xlsx_render::render_resize_guide(&painter, &layout, viewport, resize.gesture);
        }
        // The scale is part of the context's state, not the painter's, so it
        // has to come back off or the next frame would compound it.
        context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)?;

        let regions = layout.viewport_regions(viewport);
        let range = layout.visible_range(regions.main);
        let drawn_cells = layout
            .cells
            .iter()
            .filter(|cell| {
                [
                    regions.main,
                    regions.frozen_rows,
                    regions.frozen_columns,
                    regions.corner,
                ]
                .iter()
                .any(|region| cell.rect.intersects(region))
            })
            .count();
        Ok(ViewportRender {
            drawn_cells,
            first_row: range.map(|range| range.first_row).unwrap_or_default(),
            last_row: range.map(|range| range.last_row).unwrap_or_default(),
            first_column: range.map(|range| range.first_column).unwrap_or_default(),
            last_column: range.map(|range| range.last_column).unwrap_or_default(),
            visible: range.is_some(),
        })
    }

    /// Moves the selection by a whole-cell delta, clamped to the laid-out
    /// grid, and reports the active cell. A plain move collapses the range and
    /// steps from the active cell; an extending move keeps the anchor and
    /// steps the focus, the way Shift with an arrow does. With nothing
    /// selected there is nowhere to move from, so nothing happens.
    #[wasm_bindgen(js_name = moveSelection)]
    pub fn move_selection(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        d_column: i32,
        d_row: i32,
        extend: bool,
    ) -> Result<Option<Selection>, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;
        let Some((anchor, focus)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let model = self.model();
        let sheet = model
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str("sheet index is out of bounds"))?;
        let step =
            |from| jade_xlsx_layout::navigation::move_cell(sheet, &layout, from, d_column, d_row);
        let (anchor, focus) = if extend {
            (anchor, step(focus))
        } else {
            let landed = step(anchor);
            (landed, landed)
        };
        let kind = self
            .selected_area(sheet_index)
            .map(|area| area.kind)
            .unwrap_or_default();
        let kind = if extend
            && ((kind == SelectionKind::Rows && d_column == 0)
                || (kind == SelectionKind::Columns && d_row == 0))
        {
            kind
        } else {
            SelectionKind::Cells
        };
        self.choose_area(sheet_index, kind, anchor, focus);
        Ok(Some(self.describe(sheet_index, anchor, &layout)))
    }

    /// The selected cell's rectangle in canvas coordinates — header bands
    /// included, scroll subtracted — so the page can put an editor over it.
    /// A merged cell answers with its whole merged rectangle.
    #[wasm_bindgen(js_name = selectionRect)]
    pub fn selection_rect(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<Option<CellRect>, JsValue> {
        let Some((reference, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let layout = self.layout(sheet_index, canvas)?;
        let metrics = layout.header_metrics();
        let Some(rect) = layout.cell_rect_in_viewport(reference, scroll_x, scroll_y) else {
            return Ok(None);
        };
        Ok(Some(CellRect {
            x: rect.x + metrics.row_width,
            y: rect.y + metrics.column_height,
            width: rect.width,
            height: rect.height,
        }))
    }

    /// Scroll offsets that reveal the selection focus, including Shift ranges.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = selectionScroll)]
    pub fn selection_scroll(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        scroll_x: f64,
        scroll_y: f64,
        width: f64,
        height: f64,
    ) -> Result<Option<ScrollPosition>, JsValue> {
        let Some((_, focus)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let layout = self.layout(sheet_index, canvas)?;
        let (scroll_x, scroll_y) = layout.scroll_to_reveal(
            focus,
            Rect {
                x: scroll_x,
                y: scroll_y,
                width,
                height,
            },
        );
        Ok(Some(ScrollPosition { scroll_x, scroll_y }))
    }

    /// What editing the selected cell should start from: the formula with its
    /// `=`, or the stored value the way you would retype it — never the
    /// formatted text, which is what the cell shows rather than what it holds.
    #[wasm_bindgen(js_name = editText)]
    pub fn edit_text(&self, sheet_index: usize) -> Option<String> {
        let (reference, _) = self.selection.borrow().get(&sheet_index).copied()?;
        let cell = self
            .model()
            .sheets
            .get(sheet_index)?
            .rows
            .iter()
            .find(|row| row.index == reference.row)?
            .cells
            .iter()
            .find(|cell| cell.reference == reference)?;
        if let Some(formula) = &cell.formula {
            return Some(format!("={formula}"));
        }
        Some(match &cell.value {
            jade_xlsx_reader::CellValue::Empty => String::new(),
            jade_xlsx_reader::CellValue::Number(number) => {
                if number.fract() == 0.0 && number.abs() < 1e15 {
                    format!("{}", *number as i64)
                } else {
                    number.to_string()
                }
            }
            jade_xlsx_reader::CellValue::Text(text) => text.clone(),
            jade_xlsx_reader::CellValue::Bool(flag) => {
                if *flag { "TRUE" } else { "FALSE" }.to_string()
            }
            jade_xlsx_reader::CellValue::Error(error) => error.clone(),
        })
    }

    /// Commits typed text into the selected cell.
    ///
    /// The text is read the way a spreadsheet reads what you type — `=` starts
    /// a formula, a number looks like one, empty clears the cell — and the
    /// write goes through the same patch-and-recalculate path the headless
    /// callers use, so dependent formulas move before the next paint.
    #[wasm_bindgen(js_name = commitInput)]
    pub fn commit_input(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        text: &str,
    ) -> Result<Option<Selection>, JsValue> {
        let Some((reference, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let sheet_name = self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str(&format!("no sheet at index {sheet_index}")))?
            .name
            .clone();
        let reference_text = format!(
            "{}{}",
            jade_xlsx_layout::column_label(reference.column),
            reference.row + 1
        );
        let input = match text.strip_prefix('=') {
            Some(formula) if !formula.is_empty() => {
                jade_xlsx_headless_api::CellInput::Formula(formula.to_string())
            }
            _ => jade_xlsx_headless_api::CellInput::parse(text),
        };
        self.inner
            .set(&sheet_name, &reference_text, input)
            .map_err(|error| JsValue::from_str(&format!("cannot write the cell: {error}")))?;
        // The bytes changed and recalculation may have moved cells on any
        // sheet, so every cached layout is stale.
        *self.cached.borrow_mut() = None;
        let layout = self.layout(sheet_index, canvas)?;
        Ok(Some(self.describe(sheet_index, reference, &layout)))
    }

    /// The selected range as tab-separated display text, row by row — what a
    /// clipboard should carry when the user copies what they can see.
    #[wasm_bindgen(js_name = rangeText)]
    pub fn range_text(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<String>, JsValue> {
        let Some((_anchor, _focus)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let layout = self.layout(sheet_index, canvas)?;
        let range = self
            .selected_area(sheet_index)
            .expect("selection exists")
            .clipboard_range(&self.model().sheets[sheet_index]);
        let (start, end) = (range.start, range.end);
        if u64::from(end.column - start.column + 1) * u64::from(end.row - start.row + 1)
            > jade_xlsx_headless_editor::RANGE_CELL_LIMIT
        {
            return Err(JsValue::from_str(
                "selected range exceeds the clipboard limit",
            ));
        }
        let mut rows = Vec::new();
        for row in start.row..=end.row {
            let mut cells = Vec::new();
            for column in start.column..=end.column {
                cells.push(
                    layout
                        .cell(jade_xlsx_reader::CellRef::new(column, row))
                        .map(|cell| cell.text.clone())
                        .unwrap_or_default(),
                );
            }
            rows.push(cells.join("\t"));
        }
        Ok(Some(rows.join("\n")))
    }

    /// Clears every cell in the selected range, in one write — so the ranged
    /// delete is as atomic, and recalculates as promptly, as a single edit.
    #[wasm_bindgen(js_name = clearRange)]
    pub fn clear_range(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<Selection>, JsValue> {
        let Some((anchor, _focus)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let area = self.selected_area(sheet_index).expect("selection exists");
        let sheet = self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str("no sheet"))?;
        let refs = jade_xlsx_layout::selection::occupied_cells(sheet, area.range());
        if refs.len() > jade_xlsx_headless_editor::RANGE_CELL_LIMIT as usize {
            return Err(JsValue::from_str(
                "selected range exceeds the cell operation limit",
            ));
        }
        let values = refs
            .into_iter()
            .map(|cell| {
                (
                    format!(
                        "{}{}",
                        jade_xlsx_layout::column_label(cell.column),
                        cell.row + 1
                    ),
                    jade_xlsx_headless_api::CellInput::Blank,
                )
            })
            .collect::<Vec<_>>();
        if values.is_empty() {
            let layout = self.layout(sheet_index, canvas)?;
            return Ok(Some(self.describe(sheet_index, anchor, &layout)));
        }
        self.write_batch(sheet_index, canvas, values, anchor)
    }

    /// Pastes tab-separated text anchored at the active cell, and selects the
    /// rectangle it filled. Each token is read the way a spreadsheet reads
    /// what you type, so `=SUM(...)` pastes as a formula and `12` as a number.
    #[wasm_bindgen(js_name = pasteText)]
    pub fn paste_text(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        text: &str,
    ) -> Result<Option<Selection>, JsValue> {
        let Some((anchor, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let mut values = Vec::new();
        let mut focus = anchor;
        for (row_offset, line) in text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .enumerate()
        {
            // A trailing newline is a copy convention, not an empty last row.
            if line.is_empty() && row_offset > 0 && text.ends_with(['\n', '\r']) {
                continue;
            }
            for (column_offset, token) in line.split('\t').enumerate() {
                let cell = jade_xlsx_reader::CellRef::new(
                    anchor.column + column_offset as u32,
                    anchor.row + row_offset as u32,
                );
                focus = jade_xlsx_reader::CellRef::new(
                    focus.column.max(cell.column),
                    focus.row.max(cell.row),
                );
                let input = match token.strip_prefix('=') {
                    Some(formula) if !formula.is_empty() => {
                        jade_xlsx_headless_api::CellInput::Formula(formula.to_string())
                    }
                    _ => jade_xlsx_headless_api::CellInput::parse(token),
                };
                values.push((
                    format!(
                        "{}{}",
                        jade_xlsx_layout::column_label(cell.column),
                        cell.row + 1
                    ),
                    input,
                ));
            }
        }
        self.choose_area(sheet_index, SelectionKind::Cells, anchor, focus);
        self.write_batch(sheet_index, canvas, values, anchor)
    }

    /// Takes back the last write, recalculation included. Returns whether
    /// there was one; the caller repaints.
    #[wasm_bindgen(js_name = undo)]
    pub fn undo(&mut self) -> Result<bool, JsValue> {
        let undone = self
            .inner
            .undo()
            .map_err(|error| JsValue::from_str(&format!("cannot undo: {error}")))?;
        if undone {
            *self.cached.borrow_mut() = None;
            *self.fill_preview.borrow_mut() = None;
            *self.resize_preview.borrow_mut() = None;
        }
        Ok(undone)
    }

    /// Reapplies the last undone write. Returns whether there was one.
    #[wasm_bindgen(js_name = redo)]
    pub fn redo(&mut self) -> Result<bool, JsValue> {
        let redone = self
            .inner
            .redo()
            .map_err(|error| JsValue::from_str(&format!("cannot redo: {error}")))?;
        if redone {
            *self.cached.borrow_mut() = None;
            *self.fill_preview.borrow_mut() = None;
            *self.resize_preview.borrow_mut() = None;
        }
        Ok(redone)
    }

    /// Switches the sheet between the light and dark themes. The next paint
    /// uses it; the caller repaints.
    #[wasm_bindgen(js_name = setDarkTheme)]
    pub fn set_dark_theme(&self, dark: bool) {
        *self.dark.borrow_mut() = dark;
    }

    /// The active cell's font and painted colours, so an editor laid over it
    /// looks like the cell itself rather than like the page around it — in
    /// either theme.
    #[wasm_bindgen(js_name = selectionStyle)]
    pub fn selection_style(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<CellStyle>, JsValue> {
        let Some((active, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let layout = self.layout(sheet_index, canvas)?;
        let options = self.render_options();
        let sheet_background = options
            .background
            .clone()
            .unwrap_or_else(|| "#ffffff".to_string());
        let styled = layout.styled_cell(active);
        let placed = styled.as_deref();
        let font = placed.map(|cell| cell.font.clone()).unwrap_or_default();
        let background = placed
            .and_then(|cell| cell.fill.as_ref())
            .filter(|fill| fill.pattern == "solid")
            .and_then(|fill| fill.foreground)
            .map(|color| options.color_css(color, &sheet_background))
            .unwrap_or(sheet_background);
        let color = placed
            .and_then(|cell| cell.text_color)
            .map(|color| options.color_css(color, &options.default_text_color))
            .unwrap_or_else(|| options.default_text_color.clone());
        Ok(Some(CellStyle {
            font: jade_xlsx_render::font_shorthand(&font),
            text_decoration: match (font.underline, font.strike) {
                (true, true) => "underline line-through",
                (true, false) => "underline",
                (false, true) => "line-through",
                (false, false) => "none",
            }
            .into(),
            background,
            color,
        }))
    }

    /// The CSS font shorthand of the active cell, so an editor drawn over it
    /// uses the glyphs the cell itself is painted with. Falls back to the
    /// layout's default font for a cell the sheet does not style.
    #[wasm_bindgen(js_name = selectionFont)]
    pub fn selection_font(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<String>, JsValue> {
        let Some((active, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let layout = self.layout(sheet_index, canvas)?;
        let font = layout
            .styled_cell(active)
            .map(|cell| cell.font.clone())
            .unwrap_or_default();
        Ok(Some(jade_xlsx_render::font_shorthand(&font)))
    }

    /// Takes the selection for the clipboard and returns its display-text
    /// TSV. The source stays in the workbook — a cut, like Excel's, only
    /// clears it when the paste lands.
    #[wasm_bindgen(js_name = copySelection)]
    pub fn copy_selection(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        cut: bool,
    ) -> Result<Option<String>, JsValue> {
        let Some((_anchor, _focus)) = self.selection.borrow().get(&sheet_index).copied() else {
            *self.clipboard.borrow_mut() = None;
            return Ok(None);
        };
        let Some(tsv) = self.range_text(sheet_index, canvas)? else {
            *self.clipboard.borrow_mut() = None;
            return Ok(None);
        };
        let area = self.selected_area(sheet_index).expect("selection exists");
        let range = if area.kind == SelectionKind::Cells {
            area.clipboard_range(&self.model().sheets[sheet_index])
        } else {
            area.range()
        };
        let name_of = |cell: jade_xlsx_reader::CellRef| {
            format!(
                "{}{}",
                jade_xlsx_layout::column_label(cell.column),
                cell.row + 1
            )
        };
        let snapshot = self
            .inner
            .copy_snapshot(
                &self.model().sheets[sheet_index].name,
                &format!("{}:{}", name_of(range.start), name_of(range.end)),
                cut,
            )
            .map_err(|error| JsValue::from_str(&format!("cannot copy: {error}")))?;
        *self.clipboard.borrow_mut() = Some(InternalClipboard {
            kind: area.kind,
            snapshot,
            cut,
            tsv: tsv.clone(),
        });
        Ok(Some(tsv))
    }

    #[wasm_bindgen(js_name = clipboardKind)]
    pub fn clipboard_kind(&self) -> Option<String> {
        self.clipboard
            .borrow()
            .as_ref()
            .map(|payload| payload.kind.as_str().to_string())
    }

    /// Explicitly pastes this editor's captured selection; used by source-size
    /// paste controls rather than by ordinary system clipboard events.
    #[wasm_bindgen(js_name = pasteCopiedSelection)]
    pub fn paste_copied_selection(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        keep_dimensions: bool,
    ) -> Result<bool, JsValue> {
        let text = self
            .clipboard
            .borrow()
            .as_ref()
            .map(|payload| payload.tsv.clone());
        let Some(text) = text else {
            return Ok(false);
        };
        self.paste_internal(sheet_index, canvas, &text, Some(keep_dimensions))
    }

    /// Pastes from the internal clipboard when `text` is the very TSV the
    /// last copy wrote — formulas move with the offset the way shared
    /// formulas do, each cell's format comes along, and a cut clears its
    /// source (format included) in the same atomic write.
    /// Returns false when the text came from elsewhere, so the caller can
    /// paste it as values instead.
    #[wasm_bindgen(js_name = pasteInternal)]
    pub fn paste_internal(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        text: &str,
        keep_dimensions: Option<bool>,
    ) -> Result<bool, JsValue> {
        let Some((anchor, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(false);
        };
        let (cut, kind) = {
            let clipboard = self.clipboard.borrow();
            let Some(payload) = clipboard.as_ref() else {
                return Ok(false);
            };
            if payload.tsv != text {
                return Ok(false);
            }
            (payload.cut, payload.kind)
        };
        let anchor = match kind {
            SelectionKind::Rows => {
                if self
                    .selected_area(sheet_index)
                    .is_some_and(|a| a.kind == SelectionKind::Rows)
                    || anchor.column == 0
                {
                    jade_xlsx_reader::CellRef::new(0, anchor.row)
                } else {
                    return Err(JsValue::from_str(
                        "paste whole rows at column A or a row selection",
                    ));
                }
            }
            SelectionKind::Columns => {
                if self
                    .selected_area(sheet_index)
                    .is_some_and(|a| a.kind == SelectionKind::Columns)
                    || anchor.row == 0
                {
                    jade_xlsx_reader::CellRef::new(anchor.column, 0)
                } else {
                    return Err(JsValue::from_str(
                        "paste whole columns at row 1 or a column selection",
                    ));
                }
            }
            SelectionKind::Sheet => {
                if anchor.column != 0 || anchor.row != 0 {
                    return Err(JsValue::from_str("paste the worksheet at A1"));
                }
                anchor
            }
            SelectionKind::Cells => anchor,
        };
        let target_name = self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str(&format!("no sheet at index {sheet_index}")))?
            .name
            .clone();
        let anchor_name = format!(
            "{}{}",
            jade_xlsx_layout::column_label(anchor.column),
            anchor.row + 1
        );
        let range = {
            let clipboard = self.clipboard.borrow();
            self.inner
                .paste_snapshot(
                    &clipboard.as_ref().expect("clipboard checked").snapshot,
                    &target_name,
                    &anchor_name,
                    keep_dimensions.unwrap_or(false),
                )
                .map_err(|error| JsValue::from_str(&format!("cannot paste: {error}")))?
        };
        *self.cached.borrow_mut() = None;
        let focus = match kind {
            SelectionKind::Rows => jade_xlsx_reader::CellRef::new(anchor.column, range.end.row),
            SelectionKind::Columns => jade_xlsx_reader::CellRef::new(range.end.column, anchor.row),
            SelectionKind::Sheet => anchor,
            SelectionKind::Cells => range.end,
        };
        self.choose_area(sheet_index, kind, anchor, focus);
        if cut {
            *self.clipboard.borrow_mut() = None;
        }
        let _ = self.layout(sheet_index, canvas)?;
        Ok(true)
    }

    /// Moves a fill-handle drag to the cell under a point. The fill extends
    /// the selection along whichever axis the pointer has gone further on,
    /// the way Excel constrains a fill to a row run or a column run, and the
    /// result is only previewed until [`Workbook::commit_fill`].
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = previewFillTo)]
    pub fn preview_fill_to(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<bool, JsValue> {
        let Some((anchor, focus)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(false);
        };
        if self
            .selected_area(sheet_index)
            .is_some_and(|area| area.kind != SelectionKind::Cells)
        {
            return Ok(false);
        }
        let layout = self.layout(sheet_index, canvas)?;
        let metrics = layout.header_metrics();
        if x < metrics.row_width || y < metrics.column_height {
            return Ok(false);
        }
        let (sheet_x, sheet_y) = layout.sheet_point_in_viewport(
            x - metrics.row_width,
            y - metrics.column_height,
            scroll_x,
            scroll_y,
        );
        let Some(target) = layout.cell_at(sheet_x, sheet_y) else {
            return Ok(false);
        };
        let (start, end) = corners(anchor, focus);
        let union = fill_union(start, end, target);
        *self.fill_preview.borrow_mut() = Some((sheet_index, union));
        Ok(true)
    }

    /// Drops the fill: every cell the preview covers outside the source gets
    /// the source run continued — a series of numbers or counting text goes
    /// on, a lone date steps a day, anything else repeats with formulas moved
    /// along. The filled rectangle becomes the selection.
    #[wasm_bindgen(js_name = commitFill)]
    pub fn commit_fill(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<Selection>, JsValue> {
        let preview = self.fill_preview.borrow_mut().take();
        let Some((sheet, union)) = preview.filter(|(sheet, _)| *sheet == sheet_index) else {
            return Ok(None);
        };
        let Some((anchor, focus)) = self.selection.borrow().get(&sheet).copied() else {
            return Ok(None);
        };
        let (start, end) = corners(anchor, focus);
        let values = {
            let model = self.model();
            jade_xlsx_headless_editor::fill::plan_fill(
                &model.sheets[sheet],
                &model.styles,
                jade_xlsx_reader::CellRange { start, end },
                union,
            )
            .into_iter()
            .map(|write| {
                (
                    format!(
                        "{}{}",
                        jade_xlsx_layout::column_label(write.at.column),
                        write.at.row + 1
                    ),
                    write.value,
                    write.style_index,
                )
            })
            .collect::<Vec<_>>()
        };

        // The filled rectangle becomes the selection. The anchor stays active
        // when it sits on a corner of it; otherwise the top-left takes over,
        // since an anchor/focus pair can only name a rectangle by its corners.
        let far = jade_xlsx_reader::CellRef::new(
            if anchor.column - union.start.column >= union.end.column - anchor.column {
                union.start.column
            } else {
                union.end.column
            },
            if anchor.row - union.start.row >= union.end.row - anchor.row {
                union.start.row
            } else {
                union.end.row
            },
        );
        let (anchor, focus) = if corners(anchor, far) == (union.start, union.end) {
            (anchor, far)
        } else {
            (union.start, union.end)
        };
        self.choose_area(sheet, SelectionKind::Cells, anchor, focus);
        if values.is_empty() {
            let layout = self.layout(sheet, canvas)?;
            return Ok(Some(self.describe(sheet, anchor, &layout)));
        }
        self.write_batch_styled(sheet, canvas, values, anchor)
    }

    /// Formats the selection: `change` is a JSON `FormatChange` — `{"bold":
    /// true}`, `{"number_format": "0.00"}` and so on — applied to every
    /// selected cell over what it already has. Values stay; one undo step.
    #[wasm_bindgen(js_name = setFormat)]
    pub fn set_format(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        change: &str,
    ) -> Result<Option<Selection>, JsValue> {
        let Some((anchor, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let change: jade_xlsx_headless_editor::FormatChange = serde_json::from_str(change)
            .map_err(|error| JsValue::from_str(&format!("not a format change: {error}")))?;
        let area = self
            .selected_area(sheet_index)
            .expect("selected pair exists");
        let range = area.range();
        let (start, end) = (range.start, range.end);
        let sheet_name = self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str(&format!("no sheet at index {sheet_index}")))?
            .name
            .clone();
        let range = format!(
            "{}{}:{}{}",
            jade_xlsx_layout::column_label(start.column),
            start.row + 1,
            jade_xlsx_layout::column_label(end.column),
            end.row + 1
        );
        self.inner
            .set_format(&sheet_name, &range, &change)
            .map_err(|error| JsValue::from_str(&format!("cannot format the cells: {error}")))?;
        *self.cached.borrow_mut() = None;
        let layout = self.layout(sheet_index, canvas)?;
        Ok(Some(self.describe(sheet_index, anchor, &layout)))
    }

    /// The active cell's underlying formatting, without conditional or dark-theme colors.
    #[wasm_bindgen(js_name = selectionFormat)]
    pub fn selection_format(&self, sheet_index: usize) -> Result<Option<String>, JsValue> {
        let Some((active, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let sheet = self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str("sheet is unavailable"))?;
        let reference = format!(
            "{}{}",
            jade_xlsx_layout::column_label(active.column),
            active.row + 1
        );
        let format = self
            .inner
            .format_at(&sheet.name, &reference)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        serde_json::to_string(&format)
            .map(Some)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Toggles `bold`, `italic` or `underline` on the selection, the way
    /// Ctrl+B does: off when the active cell has it, on otherwise.
    #[wasm_bindgen(js_name = toggleFormat)]
    pub fn toggle_format(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        key: &str,
    ) -> Result<Option<Selection>, JsValue> {
        let Some((active, _)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let sheet = self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str("sheet is unavailable"))?;
        let reference = format!(
            "{}{}",
            jade_xlsx_layout::column_label(active.column),
            active.row + 1
        );
        let base = self
            .inner
            .format_at(&sheet.name, &reference)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let change = match key {
            "bold" => format!("{{\"bold\": {}}}", !base.bold.unwrap_or(false)),
            "italic" => format!("{{\"italic\": {}}}", !base.italic.unwrap_or(false)),
            "underline" => format!("{{\"underline\": {}}}", !base.underline.unwrap_or(false)),
            other => return Err(JsValue::from_str(&format!("no format called {other}"))),
        };
        self.set_format(sheet_index, canvas, &change)
    }

    /// Inserts `count` rows above the selection. The selection stays where
    /// it is, on the new rows, the way Excel leaves it.
    #[wasm_bindgen(js_name = insertRows)]
    pub fn insert_rows(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        count: u32,
    ) -> Result<Option<Selection>, JsValue> {
        self.restructure(
            sheet_index,
            canvas,
            jade_xlsx_reader::Axis::Rows,
            true,
            count,
        )
    }

    /// Deletes the rows the selection covers.
    #[wasm_bindgen(js_name = deleteRows)]
    pub fn delete_rows(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<Selection>, JsValue> {
        self.restructure(sheet_index, canvas, jade_xlsx_reader::Axis::Rows, false, 0)
    }

    /// Inserts `count` columns to the left of the selection.
    #[wasm_bindgen(js_name = insertColumns)]
    pub fn insert_columns(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        count: u32,
    ) -> Result<Option<Selection>, JsValue> {
        self.restructure(
            sheet_index,
            canvas,
            jade_xlsx_reader::Axis::Columns,
            true,
            count,
        )
    }

    /// Deletes the columns the selection covers.
    #[wasm_bindgen(js_name = deleteColumns)]
    pub fn delete_columns(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Option<Selection>, JsValue> {
        self.restructure(
            sheet_index,
            canvas,
            jade_xlsx_reader::Axis::Columns,
            false,
            0,
        )
    }

    fn restructure(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        axis: jade_xlsx_reader::Axis,
        inserted: bool,
        count: u32,
    ) -> Result<Option<Selection>, JsValue> {
        let Some((anchor, focus)) = self.selection.borrow().get(&sheet_index).copied() else {
            return Ok(None);
        };
        let kind = self
            .selected_area(sheet_index)
            .expect("selection exists")
            .kind;
        if kind == SelectionKind::Sheet
            || (kind == SelectionKind::Rows && axis == jade_xlsx_reader::Axis::Columns)
            || (kind == SelectionKind::Columns && axis == jade_xlsx_reader::Axis::Rows)
        {
            return Err(JsValue::from_str(
                "select matching rows or columns for this operation",
            ));
        }
        let (start, end) = corners(anchor, focus);
        let sheet_name = self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str(&format!("no sheet at index {sheet_index}")))?
            .name
            .clone();
        let result = match (axis, inserted) {
            (jade_xlsx_reader::Axis::Rows, true) => {
                self.inner.insert_rows(&sheet_name, start.row + 1, count)
            }
            (jade_xlsx_reader::Axis::Rows, false) => {
                self.inner
                    .delete_rows(&sheet_name, start.row + 1, end.row - start.row + 1)
            }
            (jade_xlsx_reader::Axis::Columns, true) => self.inner.insert_columns(
                &sheet_name,
                &jade_xlsx_layout::column_label(start.column),
                count,
            ),
            (jade_xlsx_reader::Axis::Columns, false) => self.inner.delete_columns(
                &sheet_name,
                &jade_xlsx_layout::column_label(start.column),
                end.column - start.column + 1,
            ),
        };
        result.map_err(|error| JsValue::from_str(&format!("cannot change the grid: {error}")))?;
        *self.cached.borrow_mut() = None;
        *self.fill_preview.borrow_mut() = None;
        if kind != SelectionKind::Cells {
            let focus = if inserted {
                match kind {
                    SelectionKind::Rows => {
                        jade_xlsx_reader::CellRef::new(start.column, start.row + count - 1)
                    }
                    SelectionKind::Columns => {
                        jade_xlsx_reader::CellRef::new(start.column + count - 1, start.row)
                    }
                    _ => start,
                }
            } else {
                start
            };
            self.choose_area(sheet_index, kind, start, focus);
        } else if !inserted {
            self.choose_area(sheet_index, SelectionKind::Cells, start, start);
        }
        let active = self
            .selection
            .borrow()
            .get(&sheet_index)
            .map(|(anchor, _)| *anchor)
            .unwrap_or(start);
        let layout = self.layout(sheet_index, canvas)?;
        Ok(Some(self.describe(sheet_index, active, &layout)))
    }

    /// Abandons a fill-handle drag without writing anything.
    #[wasm_bindgen(js_name = cancelFill)]
    pub fn cancel_fill(&self) {
        *self.fill_preview.borrow_mut() = None;
    }

    /// Everything hanging off the sheet — its Excel tables, page header and
    /// footer, page breaks, drawing objects and notes — as JSON, in the same
    /// shape the headless callers read.
    ///
    /// JSON rather than a class per object: these are lists, and a shell that
    /// only reads them should not have to `free()` a hundred handles.
    #[wasm_bindgen(js_name = sheetObjects)]
    pub fn sheet_objects(&self, sheet_index: usize) -> Result<String, JsValue> {
        let sheet = self
            .inner
            .summary()
            .sheets
            .into_iter()
            .nth(sheet_index)
            .ok_or_else(|| JsValue::from_str(&format!("no sheet at index {sheet_index}")))?;
        serde_json::to_string(&sheet)
            .map_err(|error| JsValue::from_str(&format!("cannot describe the sheet: {error}")))
    }

    /// The drawing object under a point on the canvas.
    ///
    /// A drawing sits over the grid, so a shell has to ask this before it
    /// treats a click as a cell click: in Excel, clicking a text box selects
    /// the box, not the cell behind it.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = shapeAt)]
    pub fn shape_at(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<Option<ShapeHit>, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;
        let metrics = layout.header_metrics();
        if x < metrics.row_width || y < metrics.column_height {
            return Ok(None);
        }
        let (sheet_x, sheet_y) = layout.sheet_point_in_viewport(
            x - metrics.row_width,
            y - metrics.column_height,
            scroll_x,
            scroll_y,
        );
        let Some(shape) = layout.shape_at(sheet_x, sheet_y) else {
            return Ok(None);
        };
        // The text comes from the model, never from the laid-out lines: those
        // are wrapped for painting, and writing them back would freeze this
        // viewport's wrapping into the file.
        let text = self
            .model()
            .sheets
            .get(sheet_index)
            .and_then(|sheet| sheet.shapes.iter().find(|model| model.id == shape.id))
            .and_then(|model| model.text.clone());
        Ok(Some(ShapeHit {
            id: shape.id,
            name: shape.name.clone(),
            kind: shape_kind_name(shape.kind).to_string(),
            text,
            rect: CellRect {
                x: shape.rect.x - scroll_x + metrics.row_width,
                y: shape.rect.y - scroll_y + metrics.column_height,
                width: shape.rect.width,
                height: shape.rect.height,
            },
        }))
    }

    /// Where a shape sits on the canvas now, so a shell can keep an editor
    /// over it while the sheet scrolls or a column widens. The shape is named
    /// the way `shapeAt` reports it, or by its id.
    #[wasm_bindgen(js_name = shapeRect)]
    pub fn shape_rect(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        shape: &str,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<Option<CellRect>, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;
        let metrics = layout.header_metrics();
        let id = shape.trim().parse::<u32>().ok();
        let Some(placed) = layout
            .shapes
            .iter()
            .find(|placed| placed.name == shape || Some(placed.id) == id)
        else {
            return Ok(None);
        };
        Ok(Some(CellRect {
            x: placed.rect.x - scroll_x + metrics.row_width,
            y: placed.rect.y - scroll_y + metrics.column_height,
            width: placed.rect.width,
            height: placed.rect.height,
        }))
    }

    /// Replaces a shape's text, one paragraph per line. The shape is named
    /// the way Excel names it (`TextBox 1`) or by its id.
    ///
    /// A picture, chart or group has no text body; asking to write into one
    /// is an error rather than a silent no-op, so a shell cannot report a
    /// write that never happened.
    #[wasm_bindgen(js_name = setShapeText)]
    pub fn set_shape_text(
        &mut self,
        sheet_index: usize,
        shape: &str,
        text: &str,
    ) -> Result<(), JsValue> {
        let sheet_name = self.sheet_name(sheet_index)?;
        self.inner
            .set_shape_text(&sheet_name, shape, text)
            .map_err(|error| JsValue::from_str(&format!("cannot write the shape: {error}")))?;
        *self.cached.borrow_mut() = None;
        Ok(())
    }

    /// Changes the sheet's page header and footer.
    ///
    /// `change` is the JSON of a headless `HeaderFooterChange`: a section
    /// given is set, `null` clears it, and one left out is kept. Returns the
    /// header and footer as it stands afterwards, as JSON.
    #[wasm_bindgen(js_name = setHeaderFooter)]
    pub fn set_header_footer(
        &mut self,
        sheet_index: usize,
        change: &str,
    ) -> Result<String, JsValue> {
        let parsed: jade_xlsx_headless_api::HeaderFooterChange = serde_json::from_str(change)
            .map_err(|error| {
                JsValue::from_str(&format!("cannot read the header/footer change: {error}"))
            })?;
        let sheet_name = self.sheet_name(sheet_index)?;
        let view = self
            .inner
            .set_header_footer(&sheet_name, &parsed)
            .map_err(|error| {
                JsValue::from_str(&format!("cannot write the header/footer: {error}"))
            })?;
        // Nothing on the grid moved, but the cached layout holds the sheet
        // the model described a moment ago; the model has been reread.
        *self.cached.borrow_mut() = None;
        serde_json::to_string(&view)
            .map_err(|error| JsValue::from_str(&format!("cannot describe the result: {error}")))
    }

    /// Puts a note on a cell, replacing the note it had. An empty `author`
    /// means the one the note already has, or Excel's own fallback.
    #[wasm_bindgen(js_name = setNote)]
    pub fn set_note(
        &mut self,
        sheet_index: usize,
        reference: &str,
        text: &str,
        author: &str,
    ) -> Result<(), JsValue> {
        let sheet_name = self.sheet_name(sheet_index)?;
        self.inner
            .set_note(&sheet_name, reference, text, author)
            .map_err(|error| JsValue::from_str(&format!("cannot write the note: {error}")))?;
        *self.cached.borrow_mut() = None;
        Ok(())
    }

    /// Removes a cell's note. `false`, changing nothing, when it had none.
    #[wasm_bindgen(js_name = deleteNote)]
    pub fn delete_note(&mut self, sheet_index: usize, reference: &str) -> Result<bool, JsValue> {
        let sheet_name = self.sheet_name(sheet_index)?;
        let removed = self
            .inner
            .delete_note(&sheet_name, reference)
            .map_err(|error| JsValue::from_str(&format!("cannot remove the note: {error}")))?;
        if removed {
            *self.cached.borrow_mut() = None;
        }
        Ok(removed)
    }

    /// The note on the cell under a point on the canvas, for the tooltip
    /// Excel shows when the pointer rests on a marked cell. Selecting is not
    /// involved, so hovering does not move the selection.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = noteAt)]
    pub fn note_at(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<Option<String>, JsValue> {
        let layout = self.layout(sheet_index, canvas)?;
        let metrics = layout.header_metrics();
        if x < metrics.row_width || y < metrics.column_height {
            return Ok(None);
        }
        let (sheet_x, sheet_y) = layout.sheet_point_in_viewport(
            x - metrics.row_width,
            y - metrics.column_height,
            scroll_x,
            scroll_y,
        );
        let Some(reference) = layout.cell_at(sheet_x, sheet_y) else {
            return Ok(None);
        };
        Ok(self
            .model()
            .sheets
            .get(sheet_index)
            .and_then(|sheet| sheet.comment_at(reference))
            .map(|comment| comment.text.clone()))
    }

    /// The workbook as `.xlsx` bytes, every committed edit included.
    #[wasm_bindgen(js_name = exportBytes)]
    pub fn export_bytes(&self) -> Result<Vec<u8>, JsValue> {
        let canvas = self.font_metrics_canvas.borrow();
        let bytes = if let Some(canvas) = canvas.as_ref() {
            let context = canvas
                .get_context("2d")?
                .ok_or_else(|| JsValue::from_str("canvas context missing"))?
                .dyn_into::<CanvasRenderingContext2d>()?;
            self.inner.export_bytes_with_row_metrics_for_fonts(
                &self.layout_options(context),
                self.font_metrics_families.borrow().as_ref(),
            )
        } else {
            self.inner.export_bytes()
        };
        bytes.map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Workbook-wide formula cache state, including hidden worksheets.
    #[wasm_bindgen(js_name = calculationStatus)]
    pub fn calculation_status(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.inner.calculation_status())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = chartStatus)]
    pub fn chart_status(&self) -> Result<String, JsValue> {
        serde_json::to_string(&jade_xlsx_layout::chart::chart_status(self.model()))
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = conditionalFormattingStatus)]
    pub fn conditional_formatting_status(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.inner.conditional_formatting_status())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = filterButtons)]
    pub fn filter_buttons(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<String, JsValue> {
        let layout = self.layout(sheet, canvas)?;
        serde_json::to_string(&layout.filter_buttons)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = filterButtonAt)]
    pub fn filter_button_at(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
    ) -> Result<String, JsValue> {
        let layout = self.layout(sheet, canvas)?;
        let metrics = layout.header_metrics();
        let button = if x < metrics.row_width || y < metrics.column_height {
            None
        } else {
            let (x, y) = layout.sheet_point_in_viewport(
                x - metrics.row_width,
                y - metrics.column_height,
                scroll_x,
                scroll_y,
            );
            layout.filter_button_at(x, y)
        };
        serde_json::to_string(&button).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = filterButtonForSelection)]
    pub fn filter_button_for_selection(
        &self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<String, JsValue> {
        let layout = self.layout(sheet, canvas)?;
        let button = self
            .selected_area(sheet)
            .and_then(|area| layout.filter_button_for_cell(area.anchor));
        serde_json::to_string(&button).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = autoFilters)]
    pub fn auto_filters(&self, sheet: usize) -> Result<String, JsValue> {
        let name = self
            .model()
            .sheets
            .get(sheet)
            .ok_or_else(|| JsValue::from_str("invalid sheet index"))?
            .name
            .clone();
        let filters = self
            .inner
            .auto_filters(&name)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        serde_json::to_string(&filters).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = autoFilterOptions)]
    pub fn auto_filter_options(
        &self,
        sheet: usize,
        target: &str,
        column: u32,
        reference: Option<String>,
    ) -> Result<String, JsValue> {
        let name = self
            .model()
            .sheets
            .get(sheet)
            .ok_or_else(|| JsValue::from_str("invalid sheet index"))?
            .name
            .clone();
        let target = serde_json::from_str::<jade_xlsx_headless_api::AutoFilterTarget>(target)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let options = self
            .inner
            .auto_filter_options(&name, &target, column, reference.as_deref())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        serde_json::to_string(&options).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = currentRegion)]
    pub fn current_region(&self, sheet: usize, reference: &str) -> Result<String, JsValue> {
        self.inner
            .current_region(&self.sheet_name(sheet)?, reference)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = toggleAutoFilter)]
    pub fn toggle_auto_filter(
        &mut self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        reference: Option<String>,
    ) -> Result<String, JsValue> {
        let reference = reference.unwrap_or_else(|| {
            let area = self.selected_area(sheet);
            let range = area
                .map(|area| {
                    if area.kind == SelectionKind::Cells {
                        area.range()
                    } else {
                        jade_xlsx_reader::CellRange {
                            start: area.anchor,
                            end: area.anchor,
                        }
                    }
                })
                .unwrap_or(jade_xlsx_reader::CellRange {
                    start: jade_xlsx_reader::CellRef::new(0, 0),
                    end: jade_xlsx_reader::CellRef::new(0, 0),
                });
            let cell = |reference: jade_xlsx_reader::CellRef| {
                format!(
                    "{}{}",
                    jade_xlsx_layout::column_label(reference.column),
                    reference.row + 1
                )
            };
            if range.start == range.end {
                cell(range.start)
            } else {
                format!("{}:{}", cell(range.start), cell(range.end))
            }
        });
        let summary = self
            .inner
            .toggle_auto_filter(&self.sheet_name(sheet)?, &reference)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.finish_filter_edit(sheet, canvas)?;
        serde_json::to_string(&summary).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = changeAutoFilter)]
    pub fn change_auto_filter(
        &mut self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        change: &str,
    ) -> Result<String, JsValue> {
        let name = self
            .model()
            .sheets
            .get(sheet)
            .ok_or_else(|| JsValue::from_str("invalid sheet index"))?
            .name
            .clone();
        let change = serde_json::from_str::<jade_xlsx_headless_api::AutoFilterChange>(change)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let summary = self
            .inner
            .change_auto_filter(&name, &change)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        self.finish_filter_edit(sheet, canvas)?;
        serde_json::to_string(&summary).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    fn finish_filter_edit(&self, sheet: usize, canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
        *self.cached.borrow_mut() = None;
        *self.resize_preview.borrow_mut() = None;
        let selected = self.selection.borrow().get(&sheet).copied();
        if let Some((anchor, _)) = selected {
            let layout = self.layout(sheet, canvas)?;
            if layout
                .row(anchor.row)
                .is_some_and(|row| row.hidden || row.size <= 0.0)
            {
                let model = &self.model().sheets[sheet];
                let mut landed =
                    jade_xlsx_layout::navigation::move_cell(model, &layout, anchor, 0, -1);
                if landed == anchor {
                    landed = jade_xlsx_layout::navigation::move_cell(model, &layout, anchor, 0, 1);
                }
                self.choose_area(sheet, SelectionKind::Cells, landed, landed);
            }
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = sortColumns)]
    pub fn sort_columns(
        &self,
        sheet: usize,
        reference: &str,
        header: bool,
    ) -> Result<String, JsValue> {
        let name = &self
            .model()
            .sheets
            .get(sheet)
            .ok_or_else(|| JsValue::from_str("invalid sheet index"))?
            .name;
        let columns = self
            .inner
            .sort_columns(name, reference, header)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        serde_json::to_string(&columns).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = sortRange)]
    pub fn sort_range(
        &mut self,
        sheet: usize,
        canvas: &HtmlCanvasElement,
        request: &str,
    ) -> Result<String, JsValue> {
        let name = self
            .model()
            .sheets
            .get(sheet)
            .ok_or_else(|| JsValue::from_str("invalid sheet index"))?
            .name
            .clone();
        let request = serde_json::from_str::<jade_xlsx_headless_api::SortRequest>(request)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let summary = self
            .inner
            .sort_range(&name, &request)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        *self.cached.borrow_mut() = None;
        *self.resize_preview.borrow_mut() = None;
        let selected = self.selection.borrow().get(&sheet).copied();
        if let Some((anchor, _)) = selected {
            let layout = self.layout(sheet, canvas)?;
            if layout
                .row(anchor.row)
                .is_some_and(|row| row.hidden || row.size <= 0.0)
            {
                let model = &self.model().sheets[sheet];
                let mut landed =
                    jade_xlsx_layout::navigation::move_cell(model, &layout, anchor, 0, -1);
                if landed == anchor {
                    landed = jade_xlsx_layout::navigation::move_cell(model, &layout, anchor, 0, 1);
                }
                self.choose_area(sheet, SelectionKind::Cells, landed, landed);
            }
        }
        serde_json::to_string(&summary).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[wasm_bindgen(js_name = imageParts)]
    pub fn image_parts(&self, sheet_index: usize) -> Vec<String> {
        self.model()
            .sheets
            .get(sheet_index)
            .into_iter()
            .flat_map(|sheet| &sheet.shapes)
            .filter_map(|shape| {
                shape
                    .picture
                    .as_ref()
                    .and_then(|picture| picture.media.as_ref())
            })
            .map(|media| media.part_name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[wasm_bindgen(js_name = imageBytes)]
    pub fn image_bytes(&self, part: &str) -> Option<Vec<u8>> {
        self.media(part).map(|media| media.bytes.as_ref().clone())
    }

    #[wasm_bindgen(js_name = imageContentType)]
    pub fn image_content_type(&self, part: &str) -> Option<String> {
        self.media(part).map(|media| media.content_type.clone())
    }

    #[wasm_bindgen(js_name = hasImage)]
    pub fn has_image(&self, part: &str) -> bool {
        self.media(part)
            .is_some_and(|media| self.images.borrow().contains_key(&media.cache_key))
    }

    /// Transfers ownership of the bitmap until the workbook is freed.
    #[wasm_bindgen(js_name = setImage)]
    pub fn set_image(&self, part: &str, image: ImageBitmap) -> Result<(), JsValue> {
        let media = self
            .media(part)
            .ok_or_else(|| JsValue::from_str("image part is not in the workbook"))?;
        if let Some(previous) = self
            .images
            .borrow_mut()
            .insert(media.cache_key.clone(), image)
        {
            previous.close();
        }
        self.image_errors.borrow_mut().remove(&media.cache_key);
        Ok(())
    }

    #[wasm_bindgen(js_name = setImageError)]
    pub fn set_image_error(&self, part: &str, message: &str) {
        if let Some(media) = self.media(part) {
            self.image_errors
                .borrow_mut()
                .insert(media.cache_key.clone(), message.to_string());
        }
    }

    #[wasm_bindgen(js_name = pictureStatus)]
    pub fn picture_status(&self) -> Result<String, JsValue> {
        let images = self.images.borrow();
        let errors = self.image_errors.borrow();
        let mut pictures = Vec::new();
        for (sheet_index, sheet) in self.model().sheets.iter().enumerate() {
            for shape in &sheet.shapes {
                if let Some(picture) = shape.picture.as_ref() {
                    let media = picture.media.as_ref();
                    let ready = picture.unsupported_properties.is_empty()
                        && media.is_some_and(|media| images.contains_key(&media.cache_key));
                    let error = media
                        .and_then(|media| errors.get(&media.cache_key).cloned())
                        .or_else(|| {
                            picture
                                .external_target
                                .as_ref()
                                .map(|_| "linked image is not embedded".to_string())
                        })
                        .or_else(|| {
                            media.is_none().then(|| {
                                "image relationship or package part is missing".to_string()
                            })
                        });
                    pictures.push(
                        serde_json::json!({ "sheet_index": sheet_index, "name": shape.name,
                        "part": media.map(|media| &media.part_name), "ready": ready, "error": error,
                        "unsupported_properties": picture.unsupported_properties }),
                    );
                }
            }
        }
        serde_json::to_string(&pictures).map_err(|error| JsValue::from_str(&error.to_string()))
    }
}

/// The rectangle a fill drag from `start..=end` to `target` would cover: the
/// source extended along one axis — rows or columns, whichever the pointer
/// has overshot further — never both, so a diagonal drag still fills a run.
fn fill_union(
    start: jade_xlsx_reader::CellRef,
    end: jade_xlsx_reader::CellRef,
    target: jade_xlsx_reader::CellRef,
) -> jade_xlsx_reader::CellRange {
    let overshoot_rows = target
        .row
        .saturating_sub(end.row)
        .max(start.row.saturating_sub(target.row));
    let overshoot_columns = target
        .column
        .saturating_sub(end.column)
        .max(start.column.saturating_sub(target.column));
    if overshoot_rows >= overshoot_columns {
        jade_xlsx_reader::CellRange {
            start: jade_xlsx_reader::CellRef::new(start.column, start.row.min(target.row)),
            end: jade_xlsx_reader::CellRef::new(end.column, end.row.max(target.row)),
        }
    } else {
        jade_xlsx_reader::CellRange {
            start: jade_xlsx_reader::CellRef::new(start.column.min(target.column), start.row),
            end: jade_xlsx_reader::CellRef::new(end.column.max(target.column), end.row),
        }
    }
}

/// The top-left and bottom-right of the rectangle between two cells.
fn corners(
    anchor: jade_xlsx_reader::CellRef,
    focus: jade_xlsx_reader::CellRef,
) -> (jade_xlsx_reader::CellRef, jade_xlsx_reader::CellRef) {
    (
        jade_xlsx_reader::CellRef::new(anchor.column.min(focus.column), anchor.row.min(focus.row)),
        jade_xlsx_reader::CellRef::new(anchor.column.max(focus.column), anchor.row.max(focus.row)),
    )
}

impl Workbook {
    fn media(&self, part: &str) -> Option<&jade_xlsx_reader::model::MediaPart> {
        self.model()
            .sheets
            .iter()
            .flat_map(|sheet| &sheet.shapes)
            .filter_map(|shape| {
                shape
                    .picture
                    .as_ref()
                    .and_then(|picture| picture.media.as_ref())
            })
            .find(|media| media.part_name == part)
    }
    /// The render options for the current theme.
    fn render_options(&self) -> RenderOptions {
        if *self.dark.borrow() {
            RenderOptions::dark()
        } else {
            RenderOptions::default()
        }
    }

    /// The parsed model behind the headless workbook.
    fn model(&self) -> &jade_xlsx_reader::Workbook {
        self.inner.workbook()
    }

    /// The name of a sheet by index — what the headless calls take.
    fn sheet_name(&self, sheet_index: usize) -> Result<String, JsValue> {
        Ok(self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str(&format!("no sheet at index {sheet_index}")))?
            .name
            .clone())
    }

    /// One atomic multi-cell write, then a fresh layout and description.
    fn write_batch(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        values: Vec<(String, jade_xlsx_headless_api::CellInput)>,
        active: jade_xlsx_reader::CellRef,
    ) -> Result<Option<Selection>, JsValue> {
        let values = values
            .into_iter()
            .map(|(reference, value)| (reference, value, None))
            .collect();
        self.write_batch_styled(sheet_index, canvas, values, active)
    }

    /// [`Self::write_batch`] with a style index per cell.
    fn write_batch_styled(
        &mut self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
        values: Vec<(String, jade_xlsx_headless_api::CellInput, Option<u32>)>,
        active: jade_xlsx_reader::CellRef,
    ) -> Result<Option<Selection>, JsValue> {
        let sheet_name = self
            .model()
            .sheets
            .get(sheet_index)
            .ok_or_else(|| JsValue::from_str(&format!("no sheet at index {sheet_index}")))?
            .name
            .clone();
        self.inner
            .set_cells_styled(&sheet_name, &values)
            .map_err(|error| JsValue::from_str(&format!("cannot write the cells: {error}")))?;
        *self.cached.borrow_mut() = None;
        let layout = self.layout(sheet_index, canvas)?;
        Ok(Some(self.describe(sheet_index, active, &layout)))
    }

    /// Describes a cell for the formula bar: what it shows, and what is behind
    /// it.
    fn describe(
        &self,
        sheet_index: usize,
        reference: jade_xlsx_reader::CellRef,
        layout: &SheetLayout,
    ) -> Selection {
        let name = |cell: jade_xlsx_reader::CellRef| {
            format!(
                "{}{}",
                jade_xlsx_layout::column_label(cell.column),
                cell.row + 1
            )
        };
        let range = match self.selection.borrow().get(&sheet_index).copied() {
            Some((anchor, focus)) if anchor != focus => {
                let start = jade_xlsx_reader::CellRef::new(
                    anchor.column.min(focus.column),
                    anchor.row.min(focus.row),
                );
                let end = jade_xlsx_reader::CellRef::new(
                    anchor.column.max(focus.column),
                    anchor.row.max(focus.row),
                );
                format!("{}:{}", name(start), name(end))
            }
            _ => name(reference),
        };
        let area = self.selected_area(sheet_index).unwrap_or(SelectionArea {
            kind: SelectionKind::Cells,
            anchor: reference,
            focus: reference,
        });
        let bounds = area.range();
        let range = match area.kind {
            SelectionKind::Rows => format!("{}:{}", bounds.start.row + 1, bounds.end.row + 1),
            SelectionKind::Columns => format!(
                "{}:{}",
                jade_xlsx_layout::column_label(bounds.start.column),
                jade_xlsx_layout::column_label(bounds.end.column)
            ),
            SelectionKind::Sheet => format!(
                "A1:{}{}",
                jade_xlsx_layout::column_label(bounds.end.column),
                bounds.end.row + 1
            ),
            SelectionKind::Cells => range,
        };
        let formula = self
            .model()
            .sheets
            .get(sheet_index)
            .and_then(|sheet| sheet.rows.iter().find(|row| row.index == reference.row))
            .and_then(|row| {
                row.cells
                    .iter()
                    .find(|cell| cell.reference.column == reference.column)
            })
            .and_then(|cell| cell.formula.clone());
        Selection {
            reference: name(reference),
            range,
            text: layout
                .cell(reference)
                .map(|cell| cell.text.clone())
                .unwrap_or_default(),
            formula,
            formula_state: self.inner.formula_state(sheet_index, reference),
            row: reference.row,
            column: reference.column,
            kind: area.kind,
            selected_range: bounds,
        }
    }

    fn layout_options(&self, context: CanvasRenderingContext2d) -> LayoutOptions {
        let options = LayoutOptions {
            date_system: self.model().date_system,
            measure: Rc::new(PainterMeasure::new(Canvas2dPainter::new(context))),
            ..Default::default()
        };
        let normal =
            jade_xlsx_layout::resolve_normal_font(&self.model().styles, &options.default_font);
        if self
            .measured_normal_family
            .borrow()
            .as_deref()
            .is_some_and(|family| family.eq_ignore_ascii_case(&normal.family))
        {
            options.with_measured_normal_font(&self.model().styles)
        } else {
            options.with_normal_font_metrics(&self.model().styles)
        }
    }

    /// The laid-out sheet, reusing the last one when the index has not changed.
    ///
    /// The canvas is only used to measure: wrapping has to happen against the
    /// same font the browser will draw with, or a wrapped row is the wrong
    /// height for its own text.
    fn layout(
        &self,
        sheet_index: usize,
        canvas: &HtmlCanvasElement,
    ) -> Result<Rc<SheetLayout>, JsValue> {
        if let Some((index, layout)) = self.cached.borrow().as_ref() {
            if *index == sheet_index {
                return Ok(Rc::clone(layout));
            }
        }
        if self.model().sheets.get(sheet_index).is_none() {
            return Err(JsValue::from_str(&format!(
                "no sheet at index {sheet_index}"
            )));
        }
        // A separate context handle from the one that paints: measuring pushes
        // a font onto it, and the painting path sets its own per cell.
        let measuring_context = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("the canvas has no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let options = LayoutOptions {
            min_column_count: MIN_COLUMNS,
            min_row_count: MIN_ROWS,
            ..self.layout_options(measuring_context)
        };
        let measured;
        let model = if self.font_metrics_canvas.borrow().is_some() {
            measured = self.inner.workbook_with_refreshed_row_metrics_for_fonts(
                &options,
                self.font_metrics_families.borrow().as_ref(),
            );
            &measured
        } else {
            self.model()
        };
        let layout = Rc::new(
            layout_workbook_sheet(model, sheet_index, &options).expect("checked sheet index"),
        );
        *self.cached.borrow_mut() = Some((sheet_index, Rc::clone(&layout)));
        Ok(layout)
    }
}

fn size_canvas(
    canvas: &HtmlCanvasElement,
    viewport_width: f64,
    viewport_height: f64,
    device_pixel_ratio: f64,
) -> Result<(), JsValue> {
    let width = viewport_width.max(1.0);
    let height = viewport_height.max(1.0);
    canvas.set_width((width * device_pixel_ratio).ceil() as u32);
    canvas.set_height((height * device_pixel_ratio).ceil() as u32);
    let style = canvas.style();
    style.set_property("width", &format!("{width}px"))?;
    style.set_property("height", &format!("{height}px"))?;
    Ok(())
}
