//! Recover chart data when the chart part has a formula but no cache.

use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use jade_xlsx_layout::number_format::format_value;
use jade_xlsx_reader::{Cell, CellRange, CellRef, CellValue, Sheet, Workbook};

use super::{ChartSeries, XmlElement, MAX_CACHE_POINTS};

pub(super) fn open(package: &OpcPackage, part: &str, root: &XmlElement) -> Option<Workbook> {
    let id = root.child("externalData")?.attr("id")?;
    let xml = package.part_str(&rels_part_name(part)).ok()??;
    let relationships = parse_relationships(&xml).ok()?;
    let relationship = relationships.relationship(id)?;
    if relationship.external || !relationship.relationship_type.ends_with("/package") {
        return None;
    }
    let target = resolve_target(part, &relationship.target);
    jade_xlsx_reader::read_xlsx(package.part(&target)?).ok()
}

struct FlatSource {
    sheet_name: String,
    range: CellRange,
    count: usize,
    numeric: bool,
}

impl FlatSource {
    fn uncached(container: &XmlElement, allow_strings: bool) -> Option<Self> {
        let [reference] = container.children.as_slice() else {
            return None;
        };
        let numeric = match reference.name.as_str() {
            "numRef" if reference.child("numCache").is_none() => true,
            "strRef" if allow_strings && reference.child("strCache").is_none() => false,
            _ => return None,
        };
        let (sheet_name, range) = local_chart_range(&reference.child("f")?.text)?;
        let rows = (range.end.row - range.start.row) as usize + 1;
        let columns = (range.end.column - range.start.column) as usize + 1;
        let count = rows.max(columns);
        (rows.min(columns) == 1 && count <= MAX_CACHE_POINTS).then_some(Self {
            sheet_name,
            range,
            count,
            numeric,
        })
    }

    fn sheet<'a>(&self, book: &'a Workbook) -> Option<&'a Sheet> {
        book.sheets
            .iter()
            .find(|sheet| sheet.name.eq_ignore_ascii_case(&self.sheet_name))
    }

    fn cells<'a>(&self, sheet: &'a Sheet) -> impl Iterator<Item = Option<&'a Cell>> {
        let range = self.range;
        (0..self.count as u32).map(move |point| {
            let column = range.start.column
                + if range.start.column == range.end.column {
                    0
                } else {
                    point
                };
            let row = range.start.row
                + if range.start.row == range.end.row {
                    0
                } else {
                    point
                };
            sheet.cell_at(CellRef::new(column, row))
        })
    }

    fn numbers(&self, sheet: &Sheet) -> Vec<Option<f64>> {
        self.cells(sheet)
            .map(|cell| match &cell?.value {
                CellValue::Number(value) if value.is_finite() => Some(*value),
                CellValue::Text(value) if !self.numeric => {
                    value.trim().parse::<f64>().ok().filter(|n| n.is_finite())
                }
                _ => None,
            })
            .collect()
    }
}

pub(super) fn recover_numbers<'a>(
    ser: &XmlElement,
    series: &mut ChartSeries,
    workbook: impl Fn() -> Option<&'a Workbook>,
) {
    let resolve = |container: &XmlElement, allow_strings| {
        let source = FlatSource::uncached(container, allow_strings)?;
        let book = workbook()?;
        let sheet = source.sheet(book)?;
        Some((source, book, sheet))
    };
    let values = ser.child("val").or_else(|| ser.child("yVal"));
    if let Some((source, book, sheet)) = values.and_then(|values| resolve(values, false)) {
        series.values = source.numbers(sheet);
        let mut first_format = None;
        series.worksheet_value_labels = source
            .cells(sheet)
            .zip(&series.values)
            .map(|(cell, value)| {
                let (cell, _) = cell.zip(*value)?;
                let code = cell_format_code(book, sheet, cell);
                first_format.get_or_insert(code.unwrap_or("General"));
                Some(format_value(&cell.value, code, book.date_system).text)
            })
            .collect();
        series.format_code = first_format.map(str::to_owned);
    }
    for (container, points, allow_strings) in [
        (ser.child("xVal"), &mut series.x_values, true),
        (ser.child("bubbleSize"), &mut series.bubble_sizes, false),
    ] {
        if let Some((source, _, sheet)) =
            container.and_then(|element| resolve(element, allow_strings))
        {
            *points = source.numbers(sheet);
        }
    }
}

/// Parses a direct sheet-qualified chart reference within a workbook.
pub fn local_chart_range(formula: &str) -> Option<(String, CellRange)> {
    let formula = formula.trim();
    let (name, cells) = formula
        .strip_prefix('=')
        .unwrap_or(formula)
        .rsplit_once('!')?;
    let sheet = match name
        .strip_prefix('\'')
        .and_then(|name| name.strip_suffix('\''))
    {
        Some(quoted) => quoted.replace("''", "'"),
        None if name.contains(['\'', '!']) => return None,
        None => name.to_owned(),
    };
    if sheet.is_empty() || sheet.contains(['[', ']', ':']) {
        return None;
    }
    let range = CellRange::parse(cells).ok()?;
    (range.start.row <= range.end.row && range.start.column <= range.end.column)
        .then_some((sheet, range))
}

fn cell_format_code<'a>(book: &'a Workbook, sheet: &Sheet, cell: &Cell) -> Option<&'a str> {
    let style = sheet.effective_style_index(cell.reference, cell.style_index);
    book.styles
        .cell_format(style)
        .and_then(|format| book.styles.number_format_code(format))
}
