//! Widths for refreshed reports, measured from their own displayed content.
use crate::{number_format, resolve_font, resolve_normal_font, units, LayoutOptions};
use jade_xlsx_reader::{pivot::PivotTable, CellRef, Sheet, Workbook};
use std::collections::{BTreeMap, BTreeSet};

fn flag(attrs: &BTreeMap<String, String>, name: &str, default: bool) -> Result<bool, String> {
    match attrs.get(name).map(|v| v.trim()) {
        None => Ok(default),
        Some("1" | "true") => Ok(true),
        Some("0" | "false") => Ok(false),
        _ => Err(format!("invalid pivot {name}")),
    }
}
fn visible(sheet: &Sheet, column: u32) -> bool {
    sheet
        .column_span(column)
        .is_none_or(|c| !c.hidden && c.width_chars != Some(0.))
}
fn compact(table: &PivotTable) -> Result<bool, String> {
    if table.location.first_data_column > 1 || !flag(&table.attributes, "compact", true)? {
        return Ok(false);
    }
    let Some(first) = table.row_fields.iter().find(|f| **f >= 0) else {
        return Ok(true);
    };
    let field = table
        .fields
        .get(*first as usize)
        .ok_or("missing row field for pivot fitting")?;
    Ok(flag(&field.attributes, "compact", true)? && flag(&field.attributes, "outline", true)?)
}
fn measured(
    options: &LayoutOptions,
    text: &str,
    font: &crate::ResolvedFont,
) -> Result<f64, String> {
    let width = options.measure.measure(text, font);
    if width.is_finite() && width >= 0. {
        Ok(width)
    } else {
        Err("invalid pivot text measurement".into())
    }
}
/// Cache refresh rebuilds every report sharing that cache, possibly on several
/// sheets. Shared columns use the widest requirement from those reports.
/// Headless callers may supply deterministic metrics; app callers supply fonts.
pub fn fit_columns(
    book: &Workbook,
    cache_ids: &BTreeSet<u32>,
    options: &LayoutOptions,
) -> Result<BTreeMap<String, BTreeMap<u32, f32>>, String> {
    let mut sheets = BTreeMap::<String, BTreeMap<u32, f64>>::new();
    let mut work = 0usize;
    for t in book
        .pivots
        .tables
        .iter()
        .filter(|t| cache_ids.contains(&t.cache_id))
    {
        if !flag(&t.attributes, "applyWidthHeightFormats", true)? {
            continue;
        }
        let range = t.location.reference;
        let height = range
            .end
            .row
            .checked_sub(range.start.row)
            .and_then(|n| n.checked_add(1))
            .ok_or("inverted pivot fitting range")?;
        let width = range
            .end
            .column
            .checked_sub(range.start.column)
            .and_then(|n| n.checked_add(1))
            .ok_or("inverted pivot fitting range")?;
        if range.end.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
            || range.end.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
            || t.location.first_header_row > t.location.first_data_row
            || t.location.first_data_row > height
            || t.location.first_data_column > width
        {
            return Err("invalid pivot fitting geometry".into());
        }
        if u64::from(height) * u64::from(width) > 1_000_000 {
            return Err("pivot fitting report exceeds cell budget".into());
        }
        let sheet = book
            .sheets
            .iter()
            .find(|s| s.part_name == t.sheet_part)
            .ok_or("missing pivot fitting owner")?;
        if sheet.protection.as_ref().is_some_and(|p| !p.format_columns) {
            continue;
        }
        if !options.max_digit_width_px.is_finite()
            || options.max_digit_width_px <= 0.
            || !options.cell_padding_px.is_finite()
            || options.cell_padding_px < 0.
        {
            return Err("invalid pivot fitting metrics".into());
        }
        let pages = t
            .page_cells()
            .ok_or("ambiguous pivot page-field geometry")?;
        let page_labels = pages.iter().map(|p| p.0).collect::<BTreeSet<_>>();
        let page_values = pages.iter().map(|p| p.1).collect::<BTreeSet<_>>();
        let keep_first = !compact(t)?;
        let normal = resolve_normal_font(&book.styles, &options.default_font);
        let show_headers = flag(&t.attributes, "showHeaders", true)?;
        let data_row = range
            .start
            .row
            .checked_add(t.location.first_data_row)
            .ok_or("pivot fitting row overflow")?;
        let corner = (t.location.first_data_column > 0
            && t.location.first_header_row < t.location.first_data_row)
            .then_some(CellRef::new(
                range.start.column,
                range.start.row + t.location.first_header_row,
            ));
        for row in &sheet.rows {
            for cell in &row.cells {
                work = work.saturating_add(1);
                if work > 20_000_000 {
                    return Err("pivot fitting exceeds sparse scan budget".into());
                }
                let at = cell.reference;
                if cell.value.is_empty()
                    || !visible(sheet, at.column)
                    || sheet.merge_at(at).is_some()
                {
                    continue;
                }
                let in_report = range.contains(at);
                let is_page = page_labels.contains(&at) || page_values.contains(&at);
                if (!in_report && !is_page)
                    || (in_report && corner == Some(at))
                    || (keep_first && at.column == range.start.column)
                {
                    continue;
                }
                let format = book.styles.cell_format(Some(
                    sheet
                        .effective_style_index_in_row(at, cell.style_index, Some(row))
                        .unwrap_or(0),
                ));
                // Automatic pivot font emphasis is not a cell's own font. Do not
                // measure the style overlay or width-dependent ### placeholders.
                let font = resolve_font(
                    format.and_then(|f| book.styles.font(f)),
                    &options.default_font,
                );
                let code = format.and_then(|f| book.styles.number_format_code(f));
                let text = number_format::format_value(&cell.value, code, book.date_system).text;
                if text.is_empty() {
                    continue;
                }
                let alignment = format.map(|f| f.alignment).unwrap_or_default();
                let width = if alignment.text_rotation == 255 {
                    use unicode_segmentation::UnicodeSegmentation;
                    text.graphemes(true)
                        .map(|s| measured(options, s, &font))
                        .try_fold(0., |a, b| b.map(|b| f64::max(a, b)))?
                } else {
                    text.lines()
                        .map(|s| measured(options, s, &font))
                        .try_fold(0., |a, b| b.map(|b| f64::max(a, b)))?
                };
                let indent = measured(options, "   ", &normal)? * f64::from(alignment.indent);
                // Reserve a standard dropdown target for label headers and page
                // values; there is no button in a measure's caption.
                let label_header = in_report
                    && at.row < data_row
                    && at.column < range.start.column + t.location.first_data_column;
                let button = if show_headers && (label_header || page_values.contains(&at)) {
                    20.
                } else {
                    0.
                };
                let width = width + indent + button + options.cell_padding_px * 2.;
                if !width.is_finite() {
                    return Err("invalid pivot text measurement".into());
                }

                let slot = sheets
                    .entry(sheet.name.clone())
                    .or_default()
                    .entry(at.column)
                    .or_default();
                *slot = slot.max(width);
            }
        }
    }
    Ok(sheets
        .into_iter()
        .map(|(name, widths)| {
            (
                name,
                widths
                    .into_iter()
                    .filter_map(|(column, pixels)| {
                        (pixels > 0.).then_some((
                            column,
                            units::column_width_px_to_chars(
                                pixels.ceil(),
                                options.max_digit_width_px,
                            ),
                        ))
                    })
                    .collect(),
            )
        })
        .collect())
}
