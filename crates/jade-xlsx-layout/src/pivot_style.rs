//! Pivot table style regions, separate from ListObject table geometry.
mod presets;
use crate::table_style::{from_differential, style_from_regions, TableCellStyle, TablePart};
use jade_xlsx_reader::{
    pivot::{AxisItem, PivotTable},
    CellRange, CellRef, StyleTable, Workbook,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Line {
    kind: String,
    members: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyledPivot {
    pub range: CellRange,
    data_row: u32,
    data_column: u32,
    header_row: u32,
    row_headers: bool,
    column_headers: bool,
    last_column: bool,
    row_stripes: bool,
    column_stripes: bool,
    row_dimensions: usize,
    column_dimensions: usize,
    compact: bool,
    rows: Vec<Line>,
    columns: Vec<Line>,
    parts: Vec<(String, TablePart, u32)>,
}
// The standard's PivotTable precedence differs from ordinary table precedence.
const ORDER: &[&str] = &[
    "wholeTable",
    "pageFieldLabels",
    "pageFieldValues",
    "firstColumnStripe",
    "secondColumnStripe",
    "firstRowStripe",
    "secondRowStripe",
    "firstColumn",
    "headerRow",
    "firstHeaderCell",
    "firstSubtotalColumn",
    "secondSubtotalColumn",
    "thirdSubtotalColumn",
    "blankRow",
    "firstSubtotalRow",
    "secondSubtotalRow",
    "thirdSubtotalRow",
    "firstColumnSubheading",
    "secondColumnSubheading",
    "thirdColumnSubheading",
    "firstRowSubheading",
    "secondRowSubheading",
    "thirdRowSubheading",
    "lastColumn",
    "totalRow",
];
fn flag(
    attrs: &std::collections::BTreeMap<String, String>,
    name: &str,
    default: bool,
) -> Option<bool> {
    match attrs.get(name).map(String::as_str) {
        None => Some(default),
        Some("0" | "false") => Some(false),
        Some("1" | "true") => Some(true),
        _ => None,
    }
}
fn axis(items: &[AxisItem], fields: &[i32], t: &PivotTable, expected: usize) -> Option<Vec<Line>> {
    if expected == 0 && items.is_empty() {
        return Some(Vec::new());
    }
    if items.is_empty() {
        return (expected == 1 && fields.iter().all(|f| *f < 0)).then(|| {
            vec![Line {
                kind: "grand".into(),
                members: Vec::new(),
            }]
        });
    }
    if items.len() != expected {
        return None;
    }
    let real = fields
        .iter()
        .copied()
        .filter(|f| *f >= 0)
        .collect::<Vec<_>>();
    let mut previous = Vec::new();
    let mut lines = Vec::new();
    for item in items {
        let mut raw = previous.get(..item.repeat as usize)?.to_vec();
        raw.extend_from_slice(&item.items);
        let positions = if fields.contains(&-2) && raw.len() == real.len() {
            &real[..]
        } else {
            fields
        };
        if raw.len() > positions.len() {
            return None;
        }
        let mut members = Vec::new();
        if !matches!(item.kind.as_str(), "grand" | "blank") {
            for (field, index) in positions.iter().zip(&raw) {
                if *field >= 0 {
                    t.fields.get(*field as usize)?.items.get(*index as usize)?;
                    members.push(*index);
                }
            }
        }
        lines.push(Line {
            kind: item.kind.clone(),
            members,
        });
        previous = raw;
    }
    Some(lines)
}
pub fn styled_pivots(book: &Workbook, sheet_part: &str) -> Vec<StyledPivot> {
    book.pivots
        .tables
        .iter()
        .filter(|t| t.sheet_part == sheet_part)
        .filter_map(|t| styled(t, &book.styles))
        .collect()
}
/// Names available for an explicit pivot style assignment. Custom definitions
/// override built-in names; gallery flags do not invalidate explicit references.
pub fn style_names(styles: &StyleTable) -> Vec<String> {
    let mut names = std::collections::BTreeMap::new();
    for name in presets::names() {
        names.insert(name.to_lowercase(), name.to_string());
    }
    for definition in &styles.table_styles {
        names.insert(definition.name.to_lowercase(), definition.name.clone());
    }
    names.into_values().collect()
}
fn styled(t: &PivotTable, styles: &StyleTable) -> Option<StyledPivot> {
    let info = t.style.as_ref()?;
    let name = info.get("name")?;
    if name.is_empty() {
        return None;
    }
    let definitions = styles
        .table_styles
        .iter()
        .filter(|d| d.name.eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    let mut parts = if definitions.is_empty() {
        presets::parts(name, styles)?
    } else {
        if definitions.len() != 1 {
            return None;
        }
        definitions[0]
            .elements
            .iter()
            .filter_map(|e| {
                Some((
                    e.kind.clone(),
                    from_differential(
                        styles
                            .differential_styles
                            .get(e.differential_style_index? as usize)?,
                    ),
                    e.size,
                ))
            })
            .collect()
    };
    parts.sort_by_key(|(kind, _, _)| ORDER.iter().position(|s| *s == kind).unwrap_or(usize::MAX));
    let range = t.location.reference;
    let height = range.end.row.checked_sub(range.start.row)?.checked_add(1)?;
    let width = range
        .end
        .column
        .checked_sub(range.start.column)?
        .checked_add(1)?;
    if u64::from(height) * u64::from(width) > 1_000_000
        || t.location.first_header_row > t.location.first_data_row
        || range.end.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
        || range.end.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
    {
        return None;
    }
    let rows = axis(
        &t.row_items,
        &t.row_fields,
        t,
        height.checked_sub(t.location.first_data_row)? as usize,
    )?;
    let columns = axis(
        &t.column_items,
        &t.column_fields,
        t,
        width.checked_sub(t.location.first_data_column)? as usize,
    )?;
    Some(StyledPivot {
        range,
        data_row: range.start.row.checked_add(t.location.first_data_row)?,
        data_column: range
            .start
            .column
            .checked_add(t.location.first_data_column)?,
        header_row: range.start.row.checked_add(t.location.first_header_row)?,
        row_headers: flag(info, "showRowHeaders", false)?,
        column_headers: flag(info, "showColHeaders", false)?,
        last_column: flag(info, "showLastColumn", false)?,
        row_stripes: flag(info, "showRowStripes", false)?,
        column_stripes: flag(info, "showColStripes", false)?,
        row_dimensions: t.row_fields.iter().filter(|f| **f >= 0).count(),
        column_dimensions: t.column_fields.iter().filter(|f| **f >= 0).count(),
        compact: flag(&t.attributes, "compact", true)? && t.location.first_data_column == 1,
        rows,
        columns,
        parts,
    })
}
fn depth(kind: &str, names: &[&str]) -> Option<usize> {
    names.iter().position(|name| *name == kind)
}
fn subtotal(l: &Line) -> bool {
    !matches!(l.kind.as_str(), "data" | "grand" | "blank")
}
impl StyledPivot {
    fn size(&self, name: &str) -> u32 {
        self.parts
            .iter()
            .find(|(n, _, _)| n == name)
            .map_or(1, |(_, _, size)| (*size).max(1))
    }
    fn region_with_lines(
        &self,
        kind: &str,
        size: u32,
        cell: CellRef,
        row: Option<&Line>,
        column: Option<&Line>,
        whole: CellRange,
    ) -> Option<CellRange> {
        let row_region = || CellRange {
            start: CellRef::new(whole.start.column, cell.row),
            end: CellRef::new(whole.end.column, cell.row),
        };
        let col_region = || CellRange {
            start: CellRef::new(cell.column, whole.start.row),
            end: CellRef::new(cell.column, whole.end.row),
        };
        match kind {
            "wholeTable" => Some(whole),
            "headerRow"
                if self.column_headers
                    && cell.row >= self.header_row
                    && cell.row < self.data_row =>
            {
                Some(CellRange {
                    start: CellRef::new(whole.start.column, self.header_row),
                    end: CellRef::new(whole.end.column, self.data_row - 1),
                })
            }
            "firstHeaderCell"
                if self.header_row < self.data_row
                    && self.column_headers
                    && self.row_headers
                    && cell.column == whole.start.column
                    && cell.row == self.header_row =>
            {
                Some(CellRange {
                    start: cell,
                    end: cell,
                })
            }
            "firstColumn"
                if self.row_headers
                    && cell.column < self.data_column
                    && cell.row >= self.data_row =>
            {
                Some(CellRange {
                    start: CellRef::new(whole.start.column, self.data_row),
                    end: CellRef::new(self.data_column - 1, whole.end.row),
                })
            }
            "lastColumn" if self.last_column && column.is_some_and(|l| l.kind == "grand") => {
                Some(col_region())
            }
            "totalRow" if row.is_some_and(|l| l.kind == "grand") => Some(row_region()),
            "blankRow" if row.is_some_and(|l| l.kind == "blank") => Some(row_region()),
            "firstRowStripe" | "secondRowStripe"
                if self.row_stripes && row.is_some_and(|l| l.kind != "grand") =>
            {
                let first = self.size("firstRowStripe");
                let second = self.size("secondRowStripe");
                let pos = (cell.row - self.data_row) % first.saturating_add(second);
                let on = if kind == "firstRowStripe" {
                    pos < first
                } else {
                    pos >= first
                };
                on.then(|| {
                    let start = cell.row - pos + if kind == "secondRowStripe" { first } else { 0 };
                    CellRange {
                        start: CellRef::new(whole.start.column, start),
                        end: CellRef::new(
                            whole.end.column,
                            start.saturating_add(size.max(1) - 1).min(whole.end.row),
                        ),
                    }
                })
            }
            "firstColumnStripe" | "secondColumnStripe"
                if self.column_stripes
                    && cell.row >= self.data_row
                    && cell.column >= self.data_column
                    && column.is_some_and(|l| l.kind != "grand") =>
            {
                let first = self.size("firstColumnStripe");
                let second = self.size("secondColumnStripe");
                let pos = (cell.column - self.data_column) % first.saturating_add(second);
                let on = if kind == "firstColumnStripe" {
                    pos < first
                } else {
                    pos >= first
                };
                on.then(|| {
                    let start = cell.column - pos
                        + if kind == "secondColumnStripe" {
                            first
                        } else {
                            0
                        };
                    CellRange {
                        start: CellRef::new(start, self.data_row),
                        end: CellRef::new(
                            start.saturating_add(size.max(1) - 1).min(whole.end.column),
                            whole.end.row,
                        ),
                    }
                })
            }
            k if depth(
                k,
                &["firstSubtotalRow", "secondSubtotalRow", "thirdSubtotalRow"],
            )
            .is_some() =>
            {
                let wanted = depth(
                    k,
                    &["firstSubtotalRow", "secondSubtotalRow", "thirdSubtotalRow"],
                )?;
                row.filter(|l| subtotal(l) && l.members.len().checked_sub(1) == Some(wanted))
                    .map(|_| row_region())
            }
            k if depth(
                k,
                &[
                    "firstSubtotalColumn",
                    "secondSubtotalColumn",
                    "thirdSubtotalColumn",
                ],
            )
            .is_some() =>
            {
                let wanted = depth(
                    k,
                    &[
                        "firstSubtotalColumn",
                        "secondSubtotalColumn",
                        "thirdSubtotalColumn",
                    ],
                )?;
                column
                    .filter(|l| subtotal(l) && l.members.len().checked_sub(1) == Some(wanted))
                    .map(|_| col_region())
            }
            k if depth(
                k,
                &[
                    "firstRowSubheading",
                    "secondRowSubheading",
                    "thirdRowSubheading",
                ],
            )
            .is_some()
                && self.row_headers
                && cell.column < self.data_column =>
            {
                let wanted = depth(
                    k,
                    &[
                        "firstRowSubheading",
                        "secondRowSubheading",
                        "thirdRowSubheading",
                    ],
                )?;
                let l = row?;
                let actual = if self.compact {
                    l.members.len().checked_sub(1)?
                } else {
                    (cell.column - whole.start.column) as usize
                };
                (actual == wanted
                    && actual < self.row_dimensions
                    && l.kind != "grand"
                    && l.kind != "blank")
                    .then_some(CellRange {
                        start: cell,
                        end: cell,
                    })
            }
            k if depth(
                k,
                &[
                    "firstColumnSubheading",
                    "secondColumnSubheading",
                    "thirdColumnSubheading",
                ],
            )
            .is_some()
                && self.column_headers
                && cell.row >= self.header_row
                && cell.row < self.data_row
                && cell.column >= self.data_column =>
            {
                let wanted = depth(
                    k,
                    &[
                        "firstColumnSubheading",
                        "secondColumnSubheading",
                        "thirdColumnSubheading",
                    ],
                )?;
                (wanted == (cell.row - self.header_row) as usize
                    && wanted < self.column_dimensions
                    && column.is_some_and(|l| l.kind != "grand"))
                .then_some(CellRange {
                    start: cell,
                    end: cell,
                })
            }
            _ => None,
        }
    }
    pub fn style_at(&self, cell: CellRef) -> Option<TableCellStyle> {
        if !self.range.contains(cell) {
            return None;
        }
        // Header coordinates precede data_row and must not short-circuit the
        // whole/header style just because there is no body-axis row there.
        let row = cell
            .row
            .checked_sub(self.data_row)
            .and_then(|i| self.rows.get(i as usize));
        let column = cell
            .column
            .checked_sub(self.data_column)
            .and_then(|i| self.columns.get(i as usize));
        Some(style_from_regions(
            cell,
            self.parts.iter().filter_map(|(kind, part, size)| {
                self.region_with_lines(kind, *size, cell, row, column, self.range)
                    .map(|r| (part, r))
            }),
        ))
    }
}
