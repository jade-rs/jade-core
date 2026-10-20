//! Charts made and changed as Excel's Insert ▸ Chart and Chart Design do.
//! Each is one write and one undo step; a protected sheet refuses them
//! unless its protection allows editing objects.

use jade_xlsx_reader::chart::Chart;
use jade_xlsx_reader::{CellRange, CellRef, CellValue};
pub use jade_xlsx_writer::{ChartKind, LegendPosition};
use jade_xlsx_writer::{SeriesReferences, ShapeAnchor};

use crate::{find_cell, find_row, parse_range, HeadlessError, HeadlessWorkbook, Result};

const WHAT: &str = "editing objects is not allowed";

/// What Chart Design changes; what is left `None` stays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChartChange {
    pub kind: Option<ChartKind>,
    /// A new title, or (`Some(None)`) none.
    pub title: Option<Option<String>>,
    /// A legend somewhere, or (`Some(None)`) none.
    pub legend: Option<Option<LegendPosition>>,
    /// Select Data: the range the series come from, as a new chart's do.
    pub data: Option<String>,
}

/// A sheet's name as a reference writes it: quoted when it has to be.
fn qualified(sheet: &str) -> String {
    let plain = !sheet.is_empty()
        && sheet
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !sheet.starts_with(|c: char| c.is_ascii_digit())
        && CellRef::parse(sheet).is_err();
    if plain {
        sheet.to_string()
    } else {
        format!("'{}'", sheet.replace('\'', "''"))
    }
}

fn absolute(at: CellRef) -> String {
    format!(
        "${}${}",
        jade_xlsx_layout::column_label(at.column),
        at.row + 1
    )
}

fn reference(sheet: &str, start: CellRef, end: CellRef) -> String {
    if start == end {
        format!("{}!{}", qualified(sheet), absolute(start))
    } else {
        format!("{}!{}:{}", qualified(sheet), absolute(start), absolute(end))
    }
}

/// The series a chart over `range` has, as Excel lays them out: a series
/// per column — or per row, when the range is wider than tall — named from
/// the first row (column) and over the categories of the first column
/// (row) when those hold text, or when the corner is empty. With the text
/// each name shows.
fn series_from_range(
    book: &HeadlessWorkbook,
    index: usize,
    range: CellRange,
    kind: ChartKind,
) -> Result<Vec<(SeriesReferences, Option<String>)>> {
    let sheet = &book.workbook.sheets[index];
    let value = |at: CellRef| -> CellValue {
        find_row(sheet, at.row)
            .and_then(|row| find_cell(row, at.column))
            .map(|cell| cell.value.clone())
            .unwrap_or(CellValue::Empty)
    };
    let text = |at: CellRef| matches!(value(at), CellValue::Text(_));
    let rows = range.end.row - range.start.row + 1;
    let columns = range.end.column - range.start.column + 1;
    if rows * columns < 2 {
        return Err(HeadlessError::BadReference(format!(
            "a chart needs more than one cell: {}",
            crate::format_range(range)
        )));
    }
    let by_columns = rows >= columns;
    // In the series' own terms: `along` runs through a series, `across`
    // from one series to the next.
    let (along, across) = if by_columns {
        (rows, columns)
    } else {
        (columns, rows)
    };
    let at = |along_index: u32, across_index: u32| {
        if by_columns {
            CellRef::new(
                range.start.column + across_index,
                range.start.row + along_index,
            )
        } else {
            CellRef::new(
                range.start.column + along_index,
                range.start.row + across_index,
            )
        }
    };
    let corner_empty = matches!(value(range.start), CellValue::Empty);
    let names = along > 1 && (corner_empty || (1..across).all(|i| text(at(0, i))) && across > 1);
    let first = u32::from(names);
    let labels = across > 1
        && (corner_empty || kind == ChartKind::Scatter || (first..along).all(|i| text(at(i, 0))));
    let first_series = u32::from(labels);
    if first >= along || first_series >= across {
        return Err(HeadlessError::BadReference(format!(
            "no data to chart in {}",
            crate::format_range(range)
        )));
    }
    let name = sheet.name.clone();
    // A pie draws one series: the first.
    let last = if kind.is_round() {
        first_series + 1
    } else {
        across
    };
    Ok((first_series..last)
        .map(|series| {
            let shown = names.then(|| match value(at(0, series)) {
                CellValue::Text(text) => text,
                CellValue::Number(number) => number.to_string(),
                _ => String::new(),
            });
            (
                SeriesReferences {
                    name: names.then(|| reference(&name, at(0, series), at(0, series))),
                    categories: labels.then(|| reference(&name, at(first, 0), at(along - 1, 0))),
                    text_categories: labels && (first..along).any(|i| text(at(i, 0))),
                    values: reference(&name, at(first, series), at(along - 1, series)),
                },
                shown,
            )
        })
        .collect())
}

/// What a chart in the file is, as Insert ▸ Chart names it; `None` for
/// the types it does not offer (3-D, radar, a combination).
pub fn chart_kind(chart: &Chart) -> Option<ChartKind> {
    let stacked = matches!(chart.grouping.as_str(), "stacked" | "percentStacked");
    Some(match chart.kind.as_str() {
        "barChart" => match (chart.horizontal, stacked) {
            (false, false) => ChartKind::Column,
            (false, true) => ChartKind::StackedColumn,
            (true, false) => ChartKind::Bar,
            (true, true) => ChartKind::StackedBar,
        },
        "lineChart" if chart.series.iter().all(|series| series.marker == "none") => ChartKind::Line,
        "lineChart" => ChartKind::LineWithMarkers,
        "areaChart" => ChartKind::Area,
        "pieChart" => ChartKind::Pie,
        "doughnutChart" => ChartKind::Doughnut,
        "scatterChart" => ChartKind::Scatter,
        _ => return None,
    })
}

/// The chart's series by their references.
fn series_of(chart: &Chart) -> Vec<SeriesReferences> {
    chart
        .series
        .iter()
        .filter_map(|series| {
            let categories = if series.categories.formula.is_some() {
                &series.categories
            } else {
                &series.x_values
            };
            Some(SeriesReferences {
                name: series.name.formula.clone(),
                categories: categories.formula.clone(),
                text_categories: !categories.numeric,
                values: series.values.formula.clone()?,
            })
        })
        .collect()
}

impl HeadlessWorkbook {
    /// Insert ▸ Chart over `range`: a chart of `kind` beside the data — its
    /// top-left a column past the range and a row down, eight columns wide
    /// and fifteen rows tall. One series is its own title and needs no
    /// legend (a pie still names its slices); several are "Chart Title"
    /// with a legend at the bottom, as Excel makes them. Returns the new
    /// object's id.
    pub fn insert_chart(&mut self, sheet: &str, range: &str, kind: ChartKind) -> Result<u32> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, WHAT, |p| p.objects)?;
        let range = parse_range(range)?;
        let series = series_from_range(self, index, range, kind)?;
        let (title, legend) = match series.as_slice() {
            [(_, shown)] => (
                shown
                    .clone()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Chart Title".into()),
                kind.is_round().then_some(LegendPosition::Bottom),
            ),
            _ => ("Chart Title".to_string(), Some(LegendPosition::Bottom)),
        };
        let series: Vec<SeriesReferences> = series.into_iter().map(|(series, _)| series).collect();
        let part = jade_xlsx_writer::charts::new_chart_part(kind, Some(&title), legend, &series);
        let from = CellRef::new(
            (range.end.column + 2).min(jade_xlsx_reader::model::MAX_COLUMN_COUNT - 9),
            (range.start.row + 1).min(jade_xlsx_reader::model::MAX_ROW_COUNT - 16),
        );
        let anchor = ShapeAnchor {
            from,
            from_offset: (0, 0),
            to: CellRef::new(from.column + 8, from.row + 15),
            to_offset: (0, 0),
        };
        let sheet_part = self.workbook.sheets[index].part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        let id = jade_xlsx_writer::charts::insert_chart(&mut package, &sheet_part, &part, &anchor)?;
        self.replace_package(package)?;
        Ok(id)
    }

    /// Changes the chart whose id is `id`: its type, title, legend or data,
    /// as Chart Design's buttons do. A new type keeps the series; new data
    /// keeps the type, title and legend.
    pub fn set_chart(&mut self, sheet: &str, id: u32, change: &ChartChange) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        self.guard(index, WHAT, |p| p.objects)?;
        let missing = || HeadlessError::NoSuchShape(id.to_string());
        let chart = self.workbook.sheets[index]
            .shapes
            .iter()
            .find(|shape| shape.id == id)
            .and_then(|shape| shape.chart.clone())
            .ok_or_else(missing)?;
        let part = chart.part_name.clone().ok_or_else(missing)?;
        let plot = if change.kind.is_some() || change.data.is_some() {
            let kind = match change.kind.or_else(|| chart_kind(&chart)) {
                Some(kind) => kind,
                None => {
                    return Err(HeadlessError::BadReference(format!(
                        "a {} chart's data cannot be changed here",
                        chart.kind
                    )))
                }
            };
            let series = match &change.data {
                Some(range) => {
                    let range = parse_range(range)?;
                    series_from_range(self, index, range, kind)?
                        .into_iter()
                        .map(|(series, _)| series)
                        .collect()
                }
                None => series_of(&chart),
            };
            let series = if kind.is_round() {
                series.into_iter().take(1).collect()
            } else {
                series
            };
            Some((kind, series))
        } else {
            None
        };
        let edit = jade_xlsx_writer::charts::ChartEdit {
            plot,
            title: change.title.clone(),
            legend: change.legend,
        };
        let xml = jade_opc::OpcPackage::open(&self.source)?
            .part_str(&part)?
            .ok_or_else(|| jade_xlsx_writer::WriteError::MissingPart(part.clone()))?;
        let patched = jade_xlsx_writer::charts::edit_chart_part(&xml, &edit)?;
        self.replace_parts(vec![(part, patched)])
    }
}
