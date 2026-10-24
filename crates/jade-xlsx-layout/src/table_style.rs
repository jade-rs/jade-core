//! Excel table styles: the banded look of a table (`ListObject`), drawn
//! beneath the cells' own formats.
//!
//! A table style is a stack of parts — the whole table, its header and total
//! rows, the first and last columns, row and column stripes — each with a
//! fill, a font and borders. Parts later in [`PRECEDENCE`] win where they
//! overlap. Built-in styles (`TableStyleLight1`–`21`, `Medium1`–`28`,
//! `Dark1`–`11`) are defined here in terms of the workbook's theme, as Excel
//! defines them; custom styles come from the file's `<tableStyles>`.

use crate::ResolvedFill;
use jade_xlsx_reader::{
    conditional_format::{DifferentialBorder, DifferentialStyle},
    BorderEdge, CellRange, CellRef, Color, Sheet, StyleTable, Table,
};
use serde::{Deserialize, Serialize};

/// Table parts from lowest to highest precedence.
pub const PRECEDENCE: [&str; 13] = [
    "wholeTable",
    "firstColumnStripe",
    "secondColumnStripe",
    "firstRowStripe",
    "secondRowStripe",
    "lastColumn",
    "firstColumn",
    "headerRow",
    "totalRow",
    "firstHeaderCell",
    "lastHeaderCell",
    "firstTotalCell",
    "lastTotalCell",
];

/// One part's look. A border of `Some(None)` clears what a lower part drew.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TablePart {
    pub fill: Option<Color>,
    pub text_color: Option<Color>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub left: Option<Option<BorderEdge>>,
    pub right: Option<Option<BorderEdge>>,
    pub top: Option<Option<BorderEdge>>,
    pub bottom: Option<Option<BorderEdge>>,
    pub inside_vertical: Option<Option<BorderEdge>>,
    pub inside_horizontal: Option<Option<BorderEdge>>,
}

/// A table on the sheet with its style resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyledTable {
    pub range: CellRange,
    pub header_rows: u32,
    pub totals_rows: u32,
    pub first_column: bool,
    pub last_column: bool,
    pub row_stripes: bool,
    pub column_stripes: bool,
    /// Parts in precedence order: kind, look, stripe size.
    pub parts: Vec<(String, TablePart, u32)>,
}

/// What the table style gives one cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableCellStyle {
    pub fill: Option<ResolvedFill>,
    pub text_color: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub left: Option<BorderEdge>,
    pub right: Option<BorderEdge>,
    pub top: Option<BorderEdge>,
    pub bottom: Option<BorderEdge>,
}

/// The rows (or columns) a part covers around one cell.
#[derive(Clone, Copy)]
struct Region {
    top: u32,
    bottom: u32,
    left: u32,
    right: u32,
}

impl StyledTable {
    fn data_rows(&self) -> Option<(u32, u32)> {
        let first = self.range.start.row + self.header_rows;
        let last = self.range.end.row.checked_sub(self.totals_rows)?;
        (first <= last).then_some((first, last))
    }

    /// The region `kind` covers at `cell`, if it covers the cell at all.
    fn region(&self, kind: &str, size: u32, cell: CellRef, first_size: u32) -> Option<Region> {
        let range = self.range;
        let whole = Region {
            top: range.start.row,
            bottom: range.end.row,
            left: range.start.column,
            right: range.end.column,
        };
        let header = (self.header_rows > 0).then_some(range.start.row);
        let total = (self.totals_rows > 0).then_some(range.end.row);
        let row_of = |row: u32| Region {
            top: row,
            bottom: row,
            ..whole
        };
        let column_of = |column: u32| Region {
            left: column,
            right: column,
            ..whole
        };
        let stripe = |position: u32, start: u32, first: u32, second: u32| {
            let cycle = first + second;
            let offset = (position - start) % cycle;
            let begin = position - offset;
            if offset < first {
                (true, begin, begin + first - 1)
            } else {
                (false, begin + first, begin + cycle - 1)
            }
        };
        match kind {
            "wholeTable" => Some(whole),
            "headerRow" => (header == Some(cell.row)).then(|| row_of(cell.row)),
            "totalRow" => (total == Some(cell.row)).then(|| row_of(cell.row)),
            "firstColumn" => (self.first_column && cell.column == range.start.column)
                .then(|| column_of(cell.column)),
            "lastColumn" => (self.last_column && cell.column == range.end.column)
                .then(|| column_of(cell.column)),
            "firstHeaderCell" => {
                (self.first_column && header == Some(cell.row) && cell.column == range.start.column)
                    .then_some(Region {
                        top: cell.row,
                        bottom: cell.row,
                        left: cell.column,
                        right: cell.column,
                    })
            }
            "lastHeaderCell" => {
                (self.last_column && header == Some(cell.row) && cell.column == range.end.column)
                    .then_some(Region {
                        top: cell.row,
                        bottom: cell.row,
                        left: cell.column,
                        right: cell.column,
                    })
            }
            "firstTotalCell" => {
                (self.first_column && total == Some(cell.row) && cell.column == range.start.column)
                    .then_some(Region {
                        top: cell.row,
                        bottom: cell.row,
                        left: cell.column,
                        right: cell.column,
                    })
            }
            "lastTotalCell" => {
                (self.last_column && total == Some(cell.row) && cell.column == range.end.column)
                    .then_some(Region {
                        top: cell.row,
                        bottom: cell.row,
                        left: cell.column,
                        right: cell.column,
                    })
            }
            "firstRowStripe" | "secondRowStripe" if self.row_stripes => {
                let (first, last) = self.data_rows()?;
                if cell.row < first || cell.row > last {
                    return None;
                }
                let (second_size, first_size) = if kind == "firstRowStripe" {
                    (self.size_of("secondRowStripe"), size)
                } else {
                    (size, first_size)
                };
                let (is_first, top, bottom) = stripe(cell.row, first, first_size, second_size);
                (is_first == (kind == "firstRowStripe")).then_some(Region {
                    top,
                    bottom: bottom.min(last),
                    ..whole
                })
            }
            "firstColumnStripe" | "secondColumnStripe" if self.column_stripes => {
                let (first, last) = self.data_rows()?;
                if cell.row < first || cell.row > last {
                    return None;
                }
                let (second_size, first_size) = if kind == "firstColumnStripe" {
                    (self.size_of("secondColumnStripe"), size)
                } else {
                    (size, first_size)
                };
                let (is_first, left, right) =
                    stripe(cell.column, range.start.column, first_size, second_size);
                (is_first == (kind == "firstColumnStripe")).then_some(Region {
                    top: first,
                    bottom: last,
                    left,
                    right: right.min(range.end.column),
                })
            }
            _ => None,
        }
    }

    fn size_of(&self, kind: &str) -> u32 {
        self.parts
            .iter()
            .find(|(name, _, _)| name == kind)
            .map_or(1, |(_, _, size)| (*size).max(1))
    }

    /// The style's look for `cell`, when the cell is in the table.
    pub fn style_at(&self, cell: CellRef) -> Option<TableCellStyle> {
        if !self.range.contains(cell) {
            return None;
        }
        let mut style = TableCellStyle::default();
        let mut fill = None;
        let mut text_color = None;
        let (mut left, mut right, mut top, mut bottom) = (None, None, None, None);
        for (kind, part, size) in &self.parts {
            let first_size = if kind.starts_with("secondRow") {
                self.size_of("firstRowStripe")
            } else {
                self.size_of("firstColumnStripe")
            };
            let Some(region) = self.region(kind, *size, cell, first_size) else {
                continue;
            };
            if part.fill.is_some() {
                fill = part.fill;
            }
            if part.text_color.is_some() {
                text_color = part.text_color;
            }
            if let Some(bold) = part.bold {
                style.bold = bold;
            }
            if let Some(italic) = part.italic {
                style.italic = italic;
            }
            let pick = |outer: &Option<Option<BorderEdge>>,
                        inner: &Option<Option<BorderEdge>>,
                        at_edge: bool| {
                if at_edge {
                    outer.clone()
                } else {
                    inner.clone()
                }
            };
            if let Some(edge) = pick(
                &part.left,
                &part.inside_vertical,
                cell.column == region.left,
            ) {
                left = Some(edge);
            }
            if let Some(edge) = pick(
                &part.right,
                &part.inside_vertical,
                cell.column == region.right,
            ) {
                right = Some(edge);
            }
            if let Some(edge) = pick(&part.top, &part.inside_horizontal, cell.row == region.top) {
                top = Some(edge);
            }
            if let Some(edge) = pick(
                &part.bottom,
                &part.inside_horizontal,
                cell.row == region.bottom,
            ) {
                bottom = Some(edge);
            }
        }
        style.fill = fill.map(|color| ResolvedFill {
            pattern: "solid".into(),
            foreground: Some(color),
            background: None,
        });
        style.text_color = text_color;
        style.left = left.flatten();
        style.right = right.flatten();
        style.top = top.flatten();
        style.bottom = bottom.flatten();
        Some(style)
    }
}

/// The sheet's tables that carry a style, resolved against the workbook's
/// theme and custom styles.
pub fn styled_tables(sheet: &Sheet, styles: &StyleTable) -> Vec<StyledTable> {
    sheet
        .tables
        .iter()
        .filter_map(|table| styled_table(table, styles))
        .collect()
}

fn styled_table(table: &Table, styles: &StyleTable) -> Option<StyledTable> {
    let name = table.style_name.as_deref()?;
    let mut parts = custom_parts(name, styles).or_else(|| builtin_parts(name, styles))?;
    parts.sort_by_key(|(kind, _, _)| {
        PRECEDENCE
            .iter()
            .position(|candidate| candidate == kind)
            .unwrap_or(0)
    });
    Some(StyledTable {
        range: table.reference,
        header_rows: table.header_row_count.min(1),
        totals_rows: table.totals_row_count.min(1),
        first_column: table.show_first_column,
        last_column: table.show_last_column,
        row_stripes: table.show_row_stripes,
        column_stripes: table.show_column_stripes,
        parts,
    })
}

fn custom_parts(name: &str, styles: &StyleTable) -> Option<Vec<(String, TablePart, u32)>> {
    let definition = styles
        .table_styles
        .iter()
        .find(|style| style.name.eq_ignore_ascii_case(name))?;
    Some(
        definition
            .elements
            .iter()
            .filter_map(|element| {
                let dxf = styles
                    .differential_styles
                    .get(element.differential_style_index? as usize)?;
                Some((element.kind.clone(), from_differential(dxf), element.size))
            })
            .collect(),
    )
}

fn from_differential(style: &DifferentialStyle) -> TablePart {
    let edge = |edge: &Option<DifferentialBorder>| {
        edge.as_ref().map(|edge| match edge {
            DifferentialBorder::Clear => None,
            DifferentialBorder::Edge(edge) => Some(edge.clone()),
        })
    };
    TablePart {
        fill: style.fill.as_ref().and_then(|fill| {
            (fill.pattern.as_deref() != Some("none"))
                .then(|| fill.background.or(fill.foreground))
                .flatten()
        }),
        text_color: style.text_color,
        bold: style.bold,
        italic: style.italic,
        left: edge(&style.left),
        right: edge(&style.right),
        top: edge(&style.top),
        bottom: edge(&style.bottom),
        inside_vertical: edge(&style.inside_vertical),
        inside_horizontal: edge(&style.inside_horizontal),
    }
}

/// The Office theme's colours, for a workbook without a theme part.
const OFFICE: [[u8; 3]; 10] = [
    [255, 255, 255],
    [0, 0, 0],
    [231, 230, 230],
    [68, 84, 106],
    [68, 114, 196],
    [237, 125, 49],
    [165, 165, 165],
    [255, 192, 0],
    [91, 155, 213],
    [112, 173, 71],
];

/// A theme colour, `tint` lighter (positive) or darker (negative).
fn theme(styles: &StyleTable, index: u32, tint: f32) -> Color {
    let resolved = styles
        .theme_colors
        .get(index as usize)
        .copied()
        .flatten()
        .or_else(|| OFFICE.get(index as usize).copied());
    Color::Theme {
        index,
        tint,
        resolved,
    }
}

fn thin(color: Color) -> Option<Option<BorderEdge>> {
    edge("thin", color)
}

fn edge(style: &str, color: Color) -> Option<Option<BorderEdge>> {
    Some(Some(BorderEdge {
        style: style.into(),
        color: Some(color),
    }))
}

/// The parts of a built-in table style.
///
/// Each family of seven runs through black (Text 1) and then the six
/// accents; Dark 8–11 pair black, accents 1 and 2, 3 and 4, 5 and 6.
fn builtin_parts(name: &str, styles: &StyleTable) -> Option<Vec<(String, TablePart, u32)>> {
    let (family, number) = if let Some(n) = name.strip_prefix("TableStyleLight") {
        ("light", n.parse::<u32>().ok()?)
    } else if let Some(n) = name.strip_prefix("TableStyleMedium") {
        ("medium", n.parse::<u32>().ok()?)
    } else if let Some(n) = name.strip_prefix("TableStyleDark") {
        ("dark", n.parse::<u32>().ok()?)
    } else {
        return None;
    };
    if number == 0 {
        return None;
    }
    let group = (number - 1) / 7;
    let variant = (number - 1) % 7;
    // Black, or accent 1–6.
    let base_index = if variant == 0 { 1 } else { 3 + variant };
    let black = variant == 0;
    let color = |tint: f32| {
        if black && tint > 0.0 {
            // Tints of black read as greys a shade lighter than an accent's.
            theme(styles, 1, (tint + 0.05).min(0.95))
        } else {
            theme(styles, base_index, tint)
        }
    };
    let text = theme(styles, 1, 0.0);
    let white = theme(styles, 0, 0.0);
    let bold = |part: TablePart| TablePart {
        bold: Some(true),
        ..part
    };
    let banded = |fill: Color| {
        vec![
            (
                "firstRowStripe".to_string(),
                TablePart {
                    fill: Some(fill),
                    ..Default::default()
                },
                1,
            ),
            (
                "firstColumnStripe".to_string(),
                TablePart {
                    fill: Some(fill),
                    ..Default::default()
                },
                1,
            ),
        ]
    };
    let emphasis = |total: TablePart, first: TablePart, last: TablePart| {
        vec![
            ("totalRow".to_string(), total, 1),
            ("firstColumn".to_string(), first, 1),
            ("lastColumn".to_string(), last, 1),
        ]
    };
    let double_top = |color: Color| TablePart {
        bold: Some(true),
        top: edge("double", color),
        ..Default::default()
    };
    let strong = TablePart {
        bold: Some(true),
        ..Default::default()
    };
    let mut parts: Vec<(String, TablePart, u32)> = Vec::new();
    match (family, group) {
        // Light 1–7: rules above and below, banded fills, coloured text.
        ("light", 0) => {
            parts.push((
                "wholeTable".into(),
                TablePart {
                    text_color: Some(if black { text } else { color(-0.25) }),
                    top: thin(color(0.0)),
                    bottom: thin(color(0.0)),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "headerRow".into(),
                bold(TablePart {
                    bottom: thin(color(0.0)),
                    ..Default::default()
                }),
                1,
            ));
            parts.extend(banded(color(0.8)));
            parts.extend(emphasis(
                double_top(color(0.0)),
                strong.clone(),
                strong.clone(),
            ));
        }
        // Light 8–14: a filled header and ruled bands.
        ("light", 1) => {
            parts.push((
                "wholeTable".into(),
                TablePart {
                    left: thin(color(0.0)),
                    right: thin(color(0.0)),
                    top: thin(color(0.0)),
                    bottom: thin(color(0.0)),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "headerRow".into(),
                TablePart {
                    fill: Some(color(0.0)),
                    text_color: Some(white),
                    bold: Some(true),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "firstRowStripe".into(),
                TablePart {
                    top: thin(color(0.0)),
                    bottom: thin(color(0.0)),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "firstColumnStripe".into(),
                TablePart {
                    left: thin(color(0.0)),
                    right: thin(color(0.0)),
                    ..Default::default()
                },
                1,
            ));
            parts.extend(emphasis(
                double_top(color(0.0)),
                strong.clone(),
                strong.clone(),
            ));
        }
        // Light 15–21: a full grid with banded fills.
        ("light", 2) => {
            parts.push((
                "wholeTable".into(),
                TablePart {
                    left: thin(color(0.0)),
                    right: thin(color(0.0)),
                    top: thin(color(0.0)),
                    bottom: thin(color(0.0)),
                    inside_vertical: thin(color(0.0)),
                    inside_horizontal: thin(color(0.0)),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "headerRow".into(),
                bold(TablePart {
                    bottom: thin(color(0.0)),
                    ..Default::default()
                }),
                1,
            ));
            parts.extend(banded(color(0.8)));
            parts.extend(emphasis(
                double_top(color(0.0)),
                strong.clone(),
                strong.clone(),
            ));
        }
        // Medium 1–7: a filled header, banded fills, light rules between rows.
        ("medium", 0) => {
            parts.push((
                "wholeTable".into(),
                TablePart {
                    text_color: Some(text),
                    top: thin(color(0.4)),
                    bottom: thin(color(0.4)),
                    inside_horizontal: thin(color(0.4)),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "headerRow".into(),
                TablePart {
                    fill: Some(color(0.0)),
                    text_color: Some(white),
                    bold: Some(true),
                    ..Default::default()
                },
                1,
            ));
            parts.extend(banded(color(0.8)));
            parts.extend(emphasis(
                double_top(color(0.0)),
                strong.clone(),
                strong.clone(),
            ));
        }
        // Medium 8–14: tinted throughout, white rules, solid header and edges.
        ("medium", 1) => {
            let solid = TablePart {
                fill: Some(color(0.0)),
                text_color: Some(white),
                bold: Some(true),
                ..Default::default()
            };
            parts.push((
                "wholeTable".into(),
                TablePart {
                    fill: Some(color(0.8)),
                    text_color: Some(text),
                    inside_vertical: thin(white),
                    inside_horizontal: thin(white),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "headerRow".into(),
                TablePart {
                    bottom: edge("medium", white),
                    ..solid.clone()
                },
                1,
            ));
            parts.extend(banded(color(0.6)));
            parts.extend(emphasis(
                TablePart {
                    top: edge("medium", white),
                    ..solid.clone()
                },
                solid.clone(),
                solid,
            ));
        }
        // Medium 15–21: dark rules, a coloured header and grey bands.
        ("medium", 2) => {
            let solid = TablePart {
                fill: Some(color(0.0)),
                text_color: Some(white),
                bold: Some(true),
                ..Default::default()
            };
            parts.push((
                "wholeTable".into(),
                TablePart {
                    text_color: Some(text),
                    left: thin(text),
                    right: thin(text),
                    top: thin(text),
                    bottom: thin(text),
                    inside_horizontal: thin(text),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "headerRow".into(),
                TablePart {
                    bottom: thin(text),
                    ..solid.clone()
                },
                1,
            ));
            parts.extend(banded(theme(styles, 0, -0.15)));
            parts.extend(emphasis(double_top(text), solid.clone(), solid));
        }
        // Medium 22–28: tinted throughout with a light grid.
        ("medium", 3) => {
            parts.push((
                "wholeTable".into(),
                TablePart {
                    fill: Some(color(0.8)),
                    text_color: Some(text),
                    left: thin(color(0.4)),
                    right: thin(color(0.4)),
                    top: thin(color(0.4)),
                    bottom: thin(color(0.4)),
                    inside_vertical: thin(color(0.4)),
                    inside_horizontal: thin(color(0.4)),
                    ..Default::default()
                },
                1,
            ));
            parts.push(("headerRow".into(), strong.clone(), 1));
            parts.extend(banded(color(0.6)));
            parts.extend(emphasis(
                double_top(color(0.0)),
                strong.clone(),
                strong.clone(),
            ));
        }
        // Dark 1–7: dark fills with white text and a black header.
        ("dark", 0) => {
            let deep = if black {
                theme(styles, 1, 0.25)
            } else {
                color(-0.5)
            };
            let solid = TablePart {
                fill: Some(deep),
                text_color: Some(white),
                bold: Some(true),
                ..Default::default()
            };
            parts.push((
                "wholeTable".into(),
                TablePart {
                    fill: Some(if black {
                        theme(styles, 1, 0.45)
                    } else {
                        color(-0.25)
                    }),
                    text_color: Some(white),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "headerRow".into(),
                TablePart {
                    fill: Some(text),
                    text_color: Some(white),
                    bold: Some(true),
                    bottom: edge("medium", white),
                    ..Default::default()
                },
                1,
            ));
            parts.extend(banded(deep));
            parts.extend(emphasis(
                TablePart {
                    top: edge("double", white),
                    ..solid.clone()
                },
                TablePart {
                    right: edge("medium", white),
                    ..solid.clone()
                },
                TablePart {
                    left: edge("medium", white),
                    ..solid
                },
            ));
        }
        // Dark 8–11: a black header over pale bands of a pair of colours.
        ("dark", 1) if variant < 4 => {
            let (first, second) = match variant {
                0 => (1, 1),
                1 => (4, 5),
                2 => (6, 7),
                _ => (8, 9),
            };
            let pale = |index: u32, tint: f32| {
                if index == 1 {
                    theme(styles, 1, (tint + 0.05).min(0.95))
                } else {
                    theme(styles, index, tint)
                }
            };
            parts.push((
                "wholeTable".into(),
                TablePart {
                    fill: Some(pale(first, 0.8)),
                    text_color: Some(text),
                    ..Default::default()
                },
                1,
            ));
            parts.push((
                "headerRow".into(),
                TablePart {
                    fill: Some(text),
                    text_color: Some(white),
                    bold: Some(true),
                    ..Default::default()
                },
                1,
            ));
            parts.extend(banded(pale(first, 0.6)));
            let edge_part = TablePart {
                fill: Some(theme(styles, second, if second == 1 { 0.35 } else { 0.0 })),
                text_color: Some(white),
                bold: Some(true),
                ..Default::default()
            };
            parts.extend(emphasis(double_top(text), edge_part.clone(), edge_part));
        }
        _ => return None,
    }
    Some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_style_resolves() {
        let styles = StyleTable::default();
        for (prefix, count) in [("Light", 21), ("Medium", 28), ("Dark", 11)] {
            for number in 1..=count {
                let name = format!("TableStyle{prefix}{number}");
                assert!(builtin_parts(&name, &styles).is_some(), "{name}");
            }
        }
        assert!(builtin_parts("TableStyleDark12", &styles).is_none());
        assert!(builtin_parts("TableStyleMedium29", &styles).is_none());
    }
}
