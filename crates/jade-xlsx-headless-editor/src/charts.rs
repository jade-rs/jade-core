//! Charts made and changed as Excel's Insert ▸ Chart and Chart Design do.
//! Each is one write and one undo step; a protected sheet refuses them
//! unless its protection allows editing objects.

use jade_xlsx_reader::chart::Chart;
use jade_xlsx_reader::{CellRange, CellRef, CellValue};
pub use jade_xlsx_writer::{ChartKind, LegendPosition};
use jade_xlsx_writer::chart_format::ChartFormat;
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
    /// Select Data Source's series, each by its references: what the
    /// chart draws, in its order — some edited, added or removed.
    pub series: Option<Vec<SeriesReferences>>,
    /// Switch Row/Column: the chart's range read the other way, a series
    /// for each row where it had one for each column, or the other way.
    pub switch_rows: bool,
    /// Chart Elements, Chart Styles and the Format panes: made after the
    /// rest, in their order.
    pub format: Vec<ChartFormat>,
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
/// per column — or per row, when the range is wider than tall, or as
/// `by_columns` says — named from the first row (column) and over the
/// categories of the first column (row) when those hold text, or when the
/// corner is empty. A bubble chart's first column is its X values and the
/// rest go in pairs, the values then the sizes. With the text each name
/// shows.
fn series_from_range(
    book: &HeadlessWorkbook,
    index: usize,
    range: CellRange,
    kind: ChartKind,
    by_columns: Option<bool>,
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
    let by_columns = by_columns.unwrap_or(rows >= columns);
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
    let bubble = kind == ChartKind::Bubble;
    let labels = if bubble {
        across > 2
    } else {
        across > 1 && (corner_empty || kind.is_xy() || (first..along).all(|i| text(at(i, 0))))
    };
    let first_series = u32::from(labels);
    if first >= along || first_series >= across || (bubble && first_series + 1 >= across) {
        return Err(HeadlessError::BadReference(format!(
            "no data to chart in {}",
            crate::format_range(range)
        )));
    }
    let name = sheet.name.clone();
    // A pie draws one series: the first; a bubble takes two lines a series.
    let step = if bubble { 2 } else { 1 };
    let last = if kind.is_round() {
        first_series + 1
    } else if bubble {
        across - 1
    } else {
        across
    };
    Ok((first_series..last)
        .step_by(step)
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
                    bubble_sizes: bubble.then(|| {
                        reference(&name, at(first, series + 1), at(along - 1, series + 1))
                    }),
                    kind: None,
                    secondary: false,
                },
                shown,
            )
        })
        .collect())
}

/// What one plot of a chart is, as Insert ▸ Chart names it.
fn plot_kind(
    element: &str,
    horizontal: bool,
    grouping: &str,
    style: &str,
    series: &[jade_xlsx_reader::chart::ChartSeries],
) -> Option<ChartKind> {
    let markers = series.iter().any(|series| series.marker != "none");
    Some(match element {
        "barChart" => match (horizontal, grouping) {
            (false, "stacked") => ChartKind::StackedColumn,
            (false, "percentStacked") => ChartKind::PercentStackedColumn,
            (false, _) => ChartKind::Column,
            (true, "stacked") => ChartKind::StackedBar,
            (true, "percentStacked") => ChartKind::PercentStackedBar,
            (true, _) => ChartKind::Bar,
        },
        "lineChart" => match (markers, grouping) {
            (false, "stacked") => ChartKind::StackedLine,
            (false, "percentStacked") => ChartKind::PercentStackedLine,
            (false, _) => ChartKind::Line,
            (true, "stacked") => ChartKind::StackedLineWithMarkers,
            (true, "percentStacked") => ChartKind::PercentStackedLineWithMarkers,
            (true, _) => ChartKind::LineWithMarkers,
        },
        "areaChart" => match grouping {
            "stacked" => ChartKind::StackedArea,
            "percentStacked" => ChartKind::PercentStackedArea,
            _ => ChartKind::Area,
        },
        "pieChart" => ChartKind::Pie,
        "doughnutChart" => ChartKind::Doughnut,
        "scatterChart" => {
            let lines = style != "marker" && series.iter().any(|series| !series.line.none);
            let smooth = style.starts_with("smooth") || series.iter().any(|series| series.smooth);
            match (lines, smooth, markers) {
                (false, _, _) => ChartKind::Scatter,
                (true, true, true) => ChartKind::ScatterSmoothMarkers,
                (true, true, false) => ChartKind::ScatterSmooth,
                (true, false, true) => ChartKind::ScatterLinesMarkers,
                (true, false, false) => ChartKind::ScatterLines,
            }
        }
        "bubbleChart" => ChartKind::Bubble,
        "radarChart" => match (style, markers) {
            ("filled", _) => ChartKind::FilledRadar,
            (_, true) => ChartKind::RadarWithMarkers,
            (_, false) => ChartKind::Radar,
        },
        _ => return None,
    })
}

/// What a chart in the file is, as Insert ▸ Chart names it: a
/// combination when it has plots of more than one type; `None` for the
/// types it does not offer (3-D, stock, surface).
pub fn chart_kind(chart: &Chart) -> Option<ChartKind> {
    let style = if chart.kind == "radarChart" {
        &chart.radar_style
    } else {
        &chart.scatter_style
    };
    let first = plot_kind(
        &chart.kind,
        chart.horizontal,
        &chart.grouping,
        style,
        &chart.series,
    )?;
    if chart.extra_plots.is_empty() {
        return Some(first);
    }
    chart
        .extra_plots
        .iter()
        .all(|plot| {
            plot_kind(
                &plot.kind,
                plot.horizontal,
                &plot.grouping,
                &plot.scatter_style,
                &plot.series,
            )
            .is_some()
        })
        .then_some(ChartKind::Combo)
}

/// The chart's series by their references, in their order; in a
/// combination each with its own type and axis.
pub fn chart_series(chart: &Chart) -> Vec<SeriesReferences> {
    let combination = !chart.extra_plots.is_empty();
    let style = if chart.kind == "radarChart" {
        &chart.radar_style
    } else {
        &chart.scatter_style
    };
    let mut plots = vec![(
        plot_kind(
            &chart.kind,
            chart.horizontal,
            &chart.grouping,
            style,
            &chart.series,
        ),
        false,
        &chart.series,
    )];
    for plot in &chart.extra_plots {
        plots.push((
            plot_kind(
                &plot.kind,
                plot.horizontal,
                &plot.grouping,
                &plot.scatter_style,
                &plot.series,
            ),
            plot.secondary,
            &plot.series,
        ));
    }
    let mut series: Vec<(u32, SeriesReferences)> = plots
        .into_iter()
        .flat_map(|(kind, secondary, series)| {
            series.iter().filter_map(move |series| {
                let categories = if series.categories.formula.is_some() {
                    &series.categories
                } else {
                    &series.x_values
                };
                Some((
                    series.order,
                    SeriesReferences {
                        name: series.name.formula.clone().or_else(|| {
                            let text = series.name.points.first().cloned().flatten()?;
                            Some(format!("\"{}\"", text.replace('"', "\"\"")))
                        }),
                        categories: categories.formula.clone(),
                        text_categories: !categories.numeric,
                        values: series.values.formula.clone()?,
                        bubble_sizes: series.bubble_sizes.formula.clone(),
                        kind: if combination { kind } else { None },
                        secondary: combination && secondary,
                    },
                ))
            })
        })
        .collect();
    series.sort_by_key(|(order, _)| *order);
    series.into_iter().map(|(_, series)| series).collect()
}

/// The range a chart's series come from on `sheet`, and whether they run
/// down its columns: `None` when they are not all on it.
fn chart_range(chart: &Chart, sheet: &str) -> Option<(CellRange, bool)> {
    let mut bounds: Option<CellRange> = None;
    let mut by_columns = None;
    let all = chart
        .series
        .iter()
        .chain(chart.extra_plots.iter().flat_map(|plot| plot.series.iter()));
    for series in all {
        for (data, values) in [
            (&series.name, false),
            (&series.categories, false),
            (&series.x_values, false),
            (&series.values, true),
            (&series.bubble_sizes, false),
        ] {
            let Some(formula) = data.formula.as_deref() else {
                continue;
            };
            let (on, at) = formula.rsplit_once('!')?;
            let on = on.trim_matches('\'').replace("''", "'");
            if !on.eq_ignore_ascii_case(sheet) {
                return None;
            }
            let range = CellRange::parse(&at.replace('$', "")).ok()?;
            if values && by_columns.is_none() {
                by_columns = Some(range.start.column == range.end.column);
            }
            bounds = Some(match bounds {
                None => range,
                Some(around) => CellRange {
                    start: CellRef::new(
                        around.start.column.min(range.start.column),
                        around.start.row.min(range.start.row),
                    ),
                    end: CellRef::new(
                        around.end.column.max(range.end.column),
                        around.end.row.max(range.end.row),
                    ),
                },
            });
        }
    }
    Some((bounds?, by_columns.unwrap_or(true)))
}

impl HeadlessWorkbook {
    /// Insert ▸ Chart over `range`: a chart of `kind` beside the data — its
    /// top-left a column past the range and a row down, eight columns wide
    /// and fifteen rows tall. One series is its own title and needs no
    /// legend (a pie still names its slices); several are "Chart Title"
    /// with a legend at the bottom, as Excel makes them. A combination is
    /// Excel's Clustered Column - Line. Returns the new object's id.
    pub fn insert_chart(&mut self, sheet: &str, range: &str, kind: ChartKind) -> Result<u32> {
        if kind == ChartKind::Combo {
            return self.insert_combo_chart(
                sheet,
                range,
                ChartKind::Column,
                ChartKind::Line,
                false,
            );
        }
        self.insert_chart_as(sheet, range, kind, None)
    }

    /// Insert ▸ Chart ▸ Combo over `range`: the first series drawn as
    /// `first`, the others as `rest` — on the secondary axis when
    /// `secondary` — as Excel's Clustered Column - Line (on Secondary
    /// Axis) and Stacked Area - Clustered Column are.
    pub fn insert_combo_chart(
        &mut self,
        sheet: &str,
        range: &str,
        first: ChartKind,
        rest: ChartKind,
        secondary: bool,
    ) -> Result<u32> {
        self.insert_chart_as(
            sheet,
            range,
            ChartKind::Combo,
            Some((first, rest, secondary)),
        )
    }

    fn insert_chart_as(
        &mut self,
        sheet: &str,
        range: &str,
        kind: ChartKind,
        combination: Option<(ChartKind, ChartKind, bool)>,
    ) -> Result<u32> {
        let index = self.sheet_index(sheet)?;
        self.guard_objects(index, WHAT)?;
        let range = parse_range(range)?;
        let mut series = series_from_range(self, index, range, kind, None)?;
        if let Some((first, rest, secondary)) = combination {
            for (at, (one, _)) in series.iter_mut().enumerate() {
                one.kind = Some(if at == 0 { first } else { rest });
                one.secondary = at > 0 && secondary;
            }
        }
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

    /// Insert ▸ Pictures: `image` (PNG, JPEG or GIF) on the sheet with its
    /// top-left at `at`'s, `size` px wide and tall, as Excel puts a picture
    /// from a file; returns the new object's id.
    pub fn insert_picture(
        &mut self,
        sheet: &str,
        at: CellRef,
        image: &[u8],
        size: (f64, f64),
    ) -> Result<u32> {
        let index = self.sheet_index(sheet)?;
        self.guard_objects(index, WHAT)?;
        let count = self.workbook.sheets[index]
            .shapes
            .iter()
            .filter(|shape| shape.kind == jade_xlsx_reader::ShapeKind::Picture)
            .count();
        let sheet_part = self.workbook.sheets[index].part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        let id = jade_xlsx_writer::pictures::insert_picture(
            &mut package,
            &sheet_part,
            image,
            at,
            size,
            &format!("Picture {}", count + 1),
        )?;
        self.replace_package(package)?;
        Ok(id)
    }

    /// Insert ▸ Shapes: a `preset` shape (one of
    /// [`jade_xlsx_writer::shapes::SHAPES`], `rect` or `line` say) on the
    /// sheet with its top-left at `at`'s, `size` px wide and tall, named as
    /// Excel names a new one; returns the object's id.
    pub fn insert_shape(
        &mut self,
        sheet: &str,
        at: CellRef,
        preset: &str,
        size: (f64, f64),
    ) -> Result<u32> {
        let index = self.sheet_index(sheet)?;
        self.guard_objects(index, WHAT)?;
        let name = jade_xlsx_writer::shapes::SHAPES
            .iter()
            .find(|(known, _)| *known == preset)
            .map(|(_, name)| *name)
            .ok_or_else(|| HeadlessError::BadReference(format!("no shape called {preset}")))?;
        let sheet_part = self.workbook.sheets[index].part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        let id = jade_xlsx_writer::shapes::insert_shape(
            &mut package,
            &sheet_part,
            preset,
            at,
            size,
            name,
        )?;
        self.replace_package(package)?;
        Ok(id)
    }

    /// Insert ▸ Shapes ▸ Freeform: a shape through `points` (px from its
    /// top-left), closed and filled when `closed`, an open line otherwise,
    /// its top-left `offset` px into `at`, named as Excel names one;
    /// returns the object's id.
    pub fn insert_freeform(
        &mut self,
        sheet: &str,
        at: CellRef,
        offset: (f64, f64),
        points: &[(f64, f64)],
        closed: bool,
    ) -> Result<u32> {
        let index = self.sheet_index(sheet)?;
        self.guard_objects(index, WHAT)?;
        let sheet_part = self.workbook.sheets[index].part_name.clone();
        let mut package = jade_opc::OpcPackage::open(&self.source)?;
        let id = jade_xlsx_writer::shapes::insert_freeform(
            &mut package,
            &sheet_part,
            at,
            offset,
            points,
            closed,
            "Freeform: Shape",
        )?;
        self.replace_package(package)?;
        Ok(id)
    }

    /// Changes the chart whose id is `id`: its type, title, legend or data,
    /// as Chart Design's buttons do. A new type keeps the series; new data
    /// keeps the type, title and legend.
    pub fn set_chart(&mut self, sheet: &str, id: u32, change: &ChartChange) -> Result<()> {
        let index = self.sheet_index(sheet)?;
        let (drawing, _, source) = self.guarded_drawing(index, id, false, WHAT)?;
        let missing = || HeadlessError::NoSuchShape(id.to_string());
        let chart = self.workbook.sheets[index]
            .shapes
            .iter()
            .find(|shape| shape.id == id)
            .and_then(|shape| shape.chart.clone())
            .ok_or_else(missing)?;
        let part = chart.part_name.clone().ok_or_else(missing)?;
        let package = jade_opc::OpcPackage::open(&self.source)?;
        let rels = package.required_part_str(&jade_opc::rels_part_name(&drawing))?;
        let rels = jade_xlsx_reader::object_protection::owned_relationships(&rels)?;
        let relationship = source
            .chart_relationship
            .as_deref()
            .and_then(|id| rels.relationship(id))
            .filter(|r| Self::owned_relationship(r, "chart"))
            .ok_or_else(missing)?;
        if jade_opc::resolve_target(&drawing, &relationship.target) != part {
            return Err(missing());
        }
        let rebuilt = change.kind.is_some()
            || change.data.is_some()
            || change.series.is_some()
            || change.switch_rows;
        let plot = if rebuilt {
            let kind = match change.kind.or_else(|| chart_kind(&chart)) {
                Some(kind) => kind,
                None => {
                    return Err(HeadlessError::BadReference(format!(
                        "a {} chart's data cannot be changed here",
                        chart.kind
                    )))
                }
            };
            let mut series = if let Some(series) = &change.series {
                series.clone()
            } else if let Some(range) = &change.data {
                let range = parse_range(range)?;
                series_from_range(self, index, range, kind, None)?
                    .into_iter()
                    .map(|(series, _)| series)
                    .collect()
            } else if change.switch_rows {
                let sheet_name = self.workbook.sheets[index].name.clone();
                let (range, by_columns) = chart_range(&chart, &sheet_name).ok_or_else(|| {
                    HeadlessError::BadReference(
                        "the chart's series do not come from one range on its sheet".into(),
                    )
                })?;
                series_from_range(self, index, range, kind, Some(!by_columns))?
                    .into_iter()
                    .map(|(series, _)| series)
                    .collect()
            } else {
                chart_series(&chart)
            };
            if series.is_empty() {
                return Err(HeadlessError::BadReference("a chart needs a series".into()));
            }
            // Only a combination draws its series as types of their own.
            if kind != ChartKind::Combo {
                for one in &mut series {
                    one.kind = None;
                    one.secondary = false;
                }
            }
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
        let mut patched = jade_xlsx_writer::charts::edit_chart_part(&xml, &edit)?;
        if !change.format.is_empty() {
            patched = jade_xlsx_writer::chart_format::format_chart_part(&patched, &change.format)?;
        }
        self.replace_parts(vec![(part, patched)])
    }
}
