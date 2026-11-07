//! Sparklines as Excel's Insert ▸ Sparklines and the Sparkline tab make and
//! change them: a group over a data range and a location range, a
//! sparkline for each row or column of the data; then their data, type,
//! look and axes, grouped, ungrouped or cleared. Each is one undo step, the
//! sheet's sparkline groups written back whole.

use jade_xlsx_reader::sparkline::{Sparkline, SparklineGroup};
use jade_xlsx_reader::{CellRange, CellRef};

use crate::{parse_range, HeadlessError, HeadlessWorkbook, Result};

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

fn column_name(mut column: u32) -> String {
    let mut name = Vec::new();
    column += 1;
    while column > 0 {
        let rest = (column - 1) % 26;
        name.push(b'A' + rest as u8);
        column = (column - 1) / 26;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

fn a1(cell: CellRef) -> String {
    format!("{}{}", column_name(cell.column), cell.row + 1)
}

/// `data` as a sparkline's formula: on `sheet` unless it names one.
fn formula(sheet: &str, data: &str) -> Result<String> {
    let data = data.trim().trim_start_matches('=');
    let (on, range) = match data.rsplit_once('!') {
        Some((on, range)) => (on.trim_matches('\'').replace("''", "'"), range),
        None => (sheet.to_string(), data),
    };
    let range = parse_range(&range.replace('$', ""))?;
    Ok(format!(
        "{}!{}:{}",
        qualified(&on),
        a1(range.start),
        a1(range.end)
    ))
}

/// A new group as Excel's Create Sparklines makes one.
pub fn new_group(kind: &str) -> SparklineGroup {
    SparklineGroup {
        kind: kind.to_string(),
        line_weight_points: 0.75,
        empty_cells: "gap".into(),
        min_axis: "individual".into(),
        max_axis: "individual".into(),
        color_series: Some(jade_xlsx_reader::model::Color::Theme {
            index: 4,
            tint: -0.499_984_7,
            resolved: None,
        }),
        color_negative: Some(jade_xlsx_reader::model::Color::Theme {
            index: 5,
            tint: 0.0,
            resolved: None,
        }),
        color_axis: Some(jade_xlsx_reader::model::Color::Argb([255, 0, 0, 0])),
        color_markers: Some(jade_xlsx_reader::model::Color::Theme {
            index: 4,
            tint: -0.499_984_7,
            resolved: None,
        }),
        color_first: Some(jade_xlsx_reader::model::Color::Theme {
            index: 4,
            tint: 0.399_975_6,
            resolved: None,
        }),
        color_last: Some(jade_xlsx_reader::model::Color::Theme {
            index: 4,
            tint: 0.399_975_6,
            resolved: None,
        }),
        color_high: Some(jade_xlsx_reader::model::Color::Theme {
            index: 4,
            tint: 0.0,
            resolved: None,
        }),
        color_low: Some(jade_xlsx_reader::model::Color::Theme {
            index: 4,
            tint: 0.0,
            resolved: None,
        }),
        ..SparklineGroup::default()
    }
}

/// The sparklines of `data` drawn in `location`: a row or column of cells,
/// one sparkline for each row (or column) of the data, as Excel pairs
/// them.
pub fn pair(sheet: &str, data: &str, location: CellRange) -> Result<Vec<Sparkline>> {
    let data_formula = formula(sheet, data)?;
    let (on, range) = data_formula.rsplit_once('!').expect("a qualified formula");
    let range = parse_range(range)?;
    let cells: Vec<CellRef> = if location.start.row == location.end.row {
        (location.start.column..=location.end.column)
            .map(|column| CellRef::new(column, location.start.row))
            .collect()
    } else if location.start.column == location.end.column {
        (location.start.row..=location.end.row)
            .map(|row| CellRef::new(location.start.column, row))
            .collect()
    } else {
        return Err(HeadlessError::BadReference(
            "the location must be one row or one column".into(),
        ));
    };
    let rows = range.end.row - range.start.row + 1;
    let columns = range.end.column - range.start.column + 1;
    let count = cells.len() as u32;
    // A sparkline for each row when there are as many rows as cells (or
    // one cell); else for each column.
    let by_rows = if count == 1 {
        rows == 1 || columns != 1
    } else if rows == count {
        true
    } else if columns == count {
        false
    } else {
        return Err(HeadlessError::BadReference(
            "the data range does not match the location range".into(),
        ));
    };
    Ok(cells
        .into_iter()
        .enumerate()
        .map(|(i, cell)| {
            let i = i as u32;
            let part = if count == 1 {
                range
            } else if by_rows {
                CellRange {
                    start: CellRef::new(range.start.column, range.start.row + i),
                    end: CellRef::new(range.end.column, range.start.row + i),
                }
            } else {
                CellRange {
                    start: CellRef::new(range.start.column + i, range.start.row),
                    end: CellRef::new(range.start.column + i, range.end.row),
                }
            };
            Sparkline {
                formula: format!("{on}!{}:{}", a1(part.start), a1(part.end)),
                cell,
            }
        })
        .collect())
}

impl HeadlessWorkbook {
    /// The sheet's sparkline groups, as the file has them.
    pub fn sparkline_groups(&self, sheet: &str) -> Result<Vec<SparklineGroup>> {
        let index = self.sheet_index(sheet)?;
        Ok(self.workbook.sheets[index].sparkline_groups.clone())
    }

    /// The sheet's sparklines replaced by `groups`, each group's
    /// sparklines in cells no other has. One undo step; `false` when they
    /// are as they were.
    pub fn set_sparkline_groups(&mut self, sheet: &str, groups: &[SparklineGroup]) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let groups: Vec<SparklineGroup> = groups
            .iter()
            .filter(|group| !group.sparklines.is_empty())
            .cloned()
            .collect();
        if groups == self.workbook.sheets[index].sparkline_groups {
            return Ok(false);
        }
        let mut cells: Vec<CellRef> = groups
            .iter()
            .flat_map(|group| group.sparklines.iter().map(|one| one.cell))
            .collect();
        let total = cells.len();
        cells.sort_by_key(|cell| (cell.row, cell.column));
        cells.dedup();
        if cells.len() != total {
            return Err(HeadlessError::BadReference(
                "two sparklines cannot share a cell".into(),
            ));
        }
        for group in &groups {
            if !matches!(group.kind.as_str(), "line" | "column" | "stacked") {
                return Err(HeadlessError::BadReference(format!(
                    "no sparkline type {}",
                    group.kind
                )));
            }
            if let (Some(min), Some(max)) = (group.manual_min, group.manual_max) {
                if group.min_axis == "custom" && group.max_axis == "custom" && min > max {
                    return Err(HeadlessError::BadReference(
                        "the minimum is over the maximum".into(),
                    ));
                }
            }
        }
        let part_name = self.workbook.sheets[index].part_name.clone();
        let xml = self.sheet_part(&part_name)?;
        let patched = jade_xlsx_writer::sparklines::set_sparkline_groups(&xml, &groups)?;
        if patched == xml {
            return Ok(false);
        }
        self.replace_parts(vec![(part_name, patched)])?;
        Ok(true)
    }

    /// Insert ▸ Sparklines: a group of `kind` (`line`, `column`, `stacked`
    /// for win/loss) over `data`, drawn in `location`. Sparklines already
    /// in those cells go, as Excel replaces them.
    pub fn insert_sparklines(
        &mut self,
        sheet: &str,
        kind: &str,
        data: &str,
        location: &str,
    ) -> Result<()> {
        let location = parse_range(&location.replace('$', ""))?;
        let sparklines = pair(sheet, data, location)?;
        let mut groups = self.sparkline_groups(sheet)?;
        for group in &mut groups {
            group
                .sparklines
                .retain(|one| !sparklines.iter().any(|new| new.cell == one.cell));
        }
        let mut group = new_group(kind);
        group.sparklines = sparklines;
        groups.push(group);
        self.set_sparkline_groups(sheet, &groups)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_pairs_with_cells_by_rows_or_columns() {
        let at = |text: &str| parse_range(text).unwrap();
        let rows = pair("Data", "A2:E4", at("F2:F4")).unwrap();
        assert_eq!(rows[2].formula, "Data!A4:E4");
        assert_eq!(rows[2].cell, CellRef::parse("F4").unwrap());
        let columns = pair("My Data", "=B2:D9", at("B10:D10")).unwrap();
        assert_eq!(columns[1].formula, "'My Data'!C2:C9");
        let one = pair("Data", "Other!A1:A9", at("C3")).unwrap();
        assert_eq!(one[0].formula, "Other!A1:A9");
        assert!(pair("Data", "A1:C3", at("E1:E2")).is_err());
        assert!(pair("Data", "A1:C3", at("E1:F3")).is_err());
    }
}
