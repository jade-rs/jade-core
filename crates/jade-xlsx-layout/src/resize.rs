//! Header resize geometry and content fitting shared by all browser hosts.
use super::{units, AxisEntry, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{Axis, Sheet, StyleTable};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResizeHit {
    pub axis: Axis,
    pub index: u32,
    pub size_px: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResizeGesture {
    pub hit: ResizeHit,
    pub pointer_start: f64,
    pub size_px: f64,
}
impl ResizeGesture {
    pub fn update(&mut self, x: f64, y: f64) {
        let pointer = if self.hit.axis == Axis::Columns { x } else { y };
        if pointer.is_finite() {
            self.size_px = (self.hit.size_px + pointer - self.pointer_start).max(0.0);
        }
    }
    pub fn stored_size(self, mdw: f64) -> f32 {
        if self.hit.axis == Axis::Columns {
            units::column_width_px_to_chars(self.size_px, mdw)
        } else {
            (self.size_px.round() * 72.0 / 96.0).clamp(0.0, 409.0) as f32
        }
    }
}

fn edge_hit(
    entries: &[AxisEntry],
    coordinate: f64,
    scroll: f64,
    frozen: u32,
    frozen_size: f64,
) -> Option<ResizeHit> {
    let mut candidate: Option<(f64, u32, f64)> = None;
    for sheet_coordinate in [coordinate, coordinate + scroll] {
        let first =
            entries.partition_point(|entry| entry.offset + entry.size < sheet_coordinate - 4.0);
        let last = entries.partition_point(|entry| entry.offset <= sheet_coordinate + 4.0);
        for entry in &entries[first.min(last)..last] {
            let frozen_entry = entry.index < frozen;
            let edge = entry.offset + entry.size - if frozen_entry { 0.0 } else { scroll };
            if !frozen_entry
                && (edge < frozen_size
                    || (edge == frozen_size && (entry.size > 0.0 || scroll > 0.0)))
            {
                continue;
            }
            if frozen_entry && coordinate > frozen_size + 4.0 {
                continue;
            }
            let distance = (edge - coordinate).abs();
            if distance <= 4.0
                && candidate.is_none_or(|(best, index, _)| {
                    distance < best || (distance == best && entry.index > index)
                })
            {
                candidate = Some((distance, entry.index, entry.size));
            }
        }
    }
    candidate.map(|(_, index, size_px)| ResizeHit {
        axis: Axis::Columns,
        index,
        size_px,
    })
}

/// Merged-cell hit targets do not alter the raw axis boundary. Hidden axes at
/// the boundary remain targets so dragging can reveal them.
pub fn header_resize_hit(
    layout: &SheetLayout,
    x: f64,
    y: f64,
    scroll_x: f64,
    scroll_y: f64,
) -> Option<ResizeHit> {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return None;
    }
    let metrics = layout.header_metrics();
    if y < metrics.column_height && x >= metrics.row_width {
        edge_hit(
            &layout.columns,
            x - metrics.row_width,
            scroll_x,
            layout.frozen_columns,
            layout.frozen_width(),
        )
    } else if x < metrics.row_width && y >= metrics.column_height {
        edge_hit(
            &layout.rows,
            y - metrics.column_height,
            scroll_y,
            layout.frozen_rows,
            layout.frozen_height(),
        )
        .map(|hit| ResizeHit {
            axis: Axis::Rows,
            ..hit
        })
    } else {
        None
    }
}

/// A one-pixel guide in full-canvas coordinates, clipped by the renderer.
pub fn guide_rect(layout: &SheetLayout, gesture: ResizeGesture, viewport: Rect) -> Option<Rect> {
    let metrics = layout.header_metrics();
    let (entries, scroll, frozen) = if gesture.hit.axis == Axis::Columns {
        (&layout.columns, viewport.x, layout.frozen_columns)
    } else {
        (&layout.rows, viewport.y, layout.frozen_rows)
    };
    let entry = entries.get(gesture.hit.index as usize)?;
    let edge = entry.offset + gesture.size_px - if entry.index < frozen { 0.0 } else { scroll };
    Some(if gesture.hit.axis == Axis::Columns {
        Rect {
            x: metrics.row_width + edge.floor(),
            y: 0.0,
            width: 1.0,
            height: metrics.column_height + viewport.height,
        }
    } else {
        Rect {
            x: 0.0,
            y: metrics.column_height + edge.floor(),
            width: metrics.row_width + viewport.width,
            height: 1.0,
        }
    })
}

/// Fits base-sized fonts and the full number-format text, excluding merged
/// cells. It never derives a width from scientific/hash overflow placeholders.
pub fn auto_fit_columns(
    sheet: &Sheet,
    styles: &StyleTable,
    first: u32,
    last: u32,
    options: &LayoutOptions,
) -> BTreeMap<u32, f32> {
    let mut pixels = BTreeMap::<u32, f64>::new();
    let conditional = super::conditional_format::ConditionalStyles::new(sheet, styles);
    let normal_font = super::resolve_normal_font(styles, &options.default_font);
    for row in &sheet.rows {
        for cell in &row.cells {
            let column = cell.reference.column;
            if column < first
                || column > last
                || cell.value.is_empty()
                || sheet.merge_at(cell.reference).is_some()
            {
                continue;
            }
            let format = styles.cell_format(Some(
                sheet
                    .effective_style_index_in_row(cell.reference, cell.style_index, Some(row))
                    .unwrap_or(0),
            ));
            let rule = conditional.for_cell(cell.reference, &cell.value);
            let mut font = super::resolve_font(
                format.and_then(|format| styles.font(format)),
                &options.default_font,
            );
            super::conditional_format::apply_font(&rule, &mut font);
            let code = rule
                .number_format_code
                .as_deref()
                .or_else(|| format.and_then(|format| styles.number_format_code(format)));
            let text =
                super::number_format::format_value(&cell.value, code, options.date_system).text;
            let alignment = format.map(|format| format.alignment).unwrap_or_default();
            let width = if alignment.text_rotation == 255 {
                use unicode_segmentation::UnicodeSegmentation;
                text.graphemes(true)
                    .map(|text| options.measure.measure(text, &font))
                    .fold(0.0, f64::max)
            } else {
                text.lines()
                    .map(|line| options.measure.measure(line, &font))
                    .fold(0.0, f64::max)
            };
            let indent = options.measure.measure("   ", &normal_font) * f64::from(alignment.indent);
            let measured = width + indent + options.cell_padding_px * 2.0;
            let slot = pixels.entry(column).or_default();
            *slot = slot.max(measured);
        }
    }
    (first..=last)
        .map(|column| {
            (
                column,
                pixels
                    .get(&column)
                    .map(|width| {
                        units::column_width_px_to_chars(width.ceil(), options.max_digit_width_px)
                    })
                    .unwrap_or_else(|| {
                        sheet
                            .default_column_width_chars
                            .unwrap_or(units::DEFAULT_COLUMN_WIDTH_CHARS)
                    }),
            )
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowHeightFit {
    pub default_points: f32,
    pub overrides: BTreeMap<u32, f32>,
}

/// Fits the worksheet using sparse overrides rather than its full row range.
pub fn auto_fit_all_rows(
    sheet: &Sheet,
    styles: &StyleTable,
    options: &LayoutOptions,
) -> RowHeightFit {
    measure_fitted_rows(
        sheet,
        styles,
        0,
        jade_xlsx_reader::model::MAX_ROW_COUNT - 1,
        options,
    )
}

pub fn auto_fit_rows(
    sheet: &Sheet,
    styles: &StyleTable,
    first: u32,
    last: u32,
    options: &LayoutOptions,
) -> BTreeMap<u32, f32> {
    let fit = measure_fitted_rows(sheet, styles, first, last, options);
    (first..=last)
        .map(|row| {
            (
                row,
                fit.overrides
                    .get(&row)
                    .copied()
                    .unwrap_or(fit.default_points),
            )
        })
        .collect()
}

fn measure_fitted_rows(
    sheet: &Sheet,
    styles: &StyleTable,
    first: u32,
    last: u32,
    options: &LayoutOptions,
) -> RowHeightFit {
    let mut measured = sheet.clone();
    let normal_font = super::resolve_normal_font(styles, &options.default_font);
    let default = if sheet.default_row_custom_height {
        units::line_height_points(normal_font.size_points)
    } else {
        sheet
            .default_row_height_points
            .unwrap_or(units::DEFAULT_ROW_HEIGHT_POINTS)
    }
    .min(409.0);
    measured.default_row_height_points = Some(default);
    measured.default_row_custom_height = false;
    for row in measured
        .rows
        .iter_mut()
        .filter(|row| row.index >= first && row.index <= last)
    {
        row.height_points = None;
        row.custom_height = false;
        row.hidden = false;
    }
    let layout = super::layout_sheet(&measured, styles, options);
    let mut heights = BTreeMap::<u32, f32>::new();
    for cell in layout.cells.iter().filter(|cell| {
        cell.reference.row >= first && cell.reference.row <= last && cell.merge.is_none()
    }) {
        let height =
            units::line_height_points(cell.font.size_points) * cell.lines.len().max(1) as f32;
        let slot = heights.entry(cell.reference.row).or_insert(default);
        *slot = slot.max(height).min(409.0);
    }
    heights.retain(|_, height| *height != default);
    RowHeightFit {
        default_points: default,
        overrides: heights,
    }
}
