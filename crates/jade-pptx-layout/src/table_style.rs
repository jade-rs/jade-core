//! Which table style a table draws with, and what each of its cells takes
//! from it.
//!
//! PowerPoint's built-in styles are referenced by GUID and never written into
//! the package, so the ones decks actually use are defined here. A deck's own
//! styles (`tableStyles.xml`) take precedence when the GUID matches.

use jade_pptx_reader::{
    Color, Fill, Outline, RunProps, TablePartStyle, TableStyle, TableStyleFlags,
};

/// The Office defaults: "Medium Style 2 - Accent 1" is what every table gets
/// on insertion; the two "No Style" entries are the plain choices.
const MEDIUM_STYLE_2_ACCENT_1: &str = "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}";
const NO_STYLE_NO_GRID: &str = "{2D5ABB26-0587-4C30-8999-92F81FD0307C}";
const NO_STYLE_TABLE_GRID: &str = "{5940675A-B579-460E-94D1-54222C63F5DA}";

fn scheme(name: &str) -> Color {
    Color::Scheme(name.to_string())
}

fn tinted(name: &str, tint: f32) -> Color {
    Color::Adjusted {
        base: Box::new(scheme(name)),
        alpha: None,
        lum_mod: None,
        lum_off: None,
        shade: None,
        tint: Some(tint),
    }
}

fn line(color: Color, width_emu: i64) -> Option<Outline> {
    Some(Outline {
        color: Some(color),
        width_emu: Some(width_emu),
        ..Outline::default()
    })
}

fn all_edges(color: Color, width_emu: i64) -> TablePartStyle {
    TablePartStyle {
        left: line(color.clone(), width_emu),
        right: line(color.clone(), width_emu),
        top: line(color.clone(), width_emu),
        bottom: line(color.clone(), width_emu),
        inside_h: line(color.clone(), width_emu),
        inside_v: line(color, width_emu),
        ..TablePartStyle::default()
    }
}

/// Medium Style 2 with `accent` as its colour: a solid header, tinted body,
/// white grid.
fn medium_style_2(id: &str, accent: &str) -> TableStyle {
    let emphasised = TablePartStyle {
        fill: Some(Fill::Solid(scheme(accent))),
        bold: Some(true),
        text_color: Some(scheme("lt1")),
        ..TablePartStyle::default()
    };
    TableStyle {
        id: id.to_string(),
        name: Some("Medium Style 2".to_string()),
        whole: Some(TablePartStyle {
            fill: Some(Fill::Solid(tinted(accent, 0.2))),
            text_color: Some(scheme("dk1")),
            ..all_edges(scheme("lt1"), 12_700)
        }),
        first_row: Some(TablePartStyle {
            bottom: line(scheme("lt1"), 38_100),
            ..emphasised.clone()
        }),
        last_row: Some(TablePartStyle {
            top: line(scheme("lt1"), 38_100),
            ..emphasised.clone()
        }),
        first_col: Some(emphasised.clone()),
        last_col: Some(emphasised),
        band1_h: Some(TablePartStyle {
            fill: Some(Fill::Solid(tinted(accent, 0.4))),
            ..TablePartStyle::default()
        }),
        band2_h: None,
        band1_v: Some(TablePartStyle {
            fill: Some(Fill::Solid(tinted(accent, 0.4))),
            ..TablePartStyle::default()
        }),
        band2_v: None,
    }
}

/// A built-in style by GUID, when it is one this crate knows.
pub fn builtin_table_style(id: &str) -> Option<TableStyle> {
    match id {
        MEDIUM_STYLE_2_ACCENT_1 => Some(medium_style_2(id, "accent1")),
        // The other accents share the layout with a different colour.
        "{21E4AEA4-8DFA-4A89-87EB-49C32662AFE0}" => Some(medium_style_2(id, "accent2")),
        "{F5AB1C69-6EDB-4FF4-983F-18BD219EF322}" => Some(medium_style_2(id, "accent3")),
        "{00A15C55-8517-42AA-B614-E9B94910E393}" => Some(medium_style_2(id, "accent4")),
        "{7DF18680-E054-41AD-8BC1-D1AEF772440D}" => Some(medium_style_2(id, "accent5")),
        "{93296810-A885-4BE3-A3E7-6D5BEEA58F35}" => Some(medium_style_2(id, "accent6")),
        NO_STYLE_NO_GRID => Some(TableStyle {
            id: id.to_string(),
            name: Some("No Style, No Grid".to_string()),
            ..TableStyle::default()
        }),
        NO_STYLE_TABLE_GRID => Some(TableStyle {
            id: id.to_string(),
            name: Some("No Style, Table Grid".to_string()),
            whole: Some(all_edges(scheme("tx1"), 12_700)),
            ..TableStyle::default()
        }),
        _ => None,
    }
}

/// The style a table draws with: the deck's own definition first, then a
/// built-in, else none.
pub fn resolve_table_style(
    flags: &TableStyleFlags,
    deck_styles: Option<&[TableStyle]>,
) -> Option<TableStyle> {
    let id = flags.style_id.as_deref()?;
    deck_styles
        .and_then(|styles| styles.iter().find(|style| style.id == id))
        .cloned()
        .or_else(|| builtin_table_style(id))
}

/// Where a cell sits in its table, which decides which style parts apply.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CellPosition {
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
    pub rows: usize,
    pub columns: usize,
}

/// What a cell takes from the style: fill, its four borders, and the text
/// defaults. Each facet comes from the most specific part that states it.
#[derive(Debug, Clone, Default)]
pub(crate) struct CellStyle {
    pub fill: Option<Fill>,
    pub left: Option<Outline>,
    pub right: Option<Outline>,
    pub top: Option<Outline>,
    pub bottom: Option<Outline>,
    pub text: RunProps,
}

/// The cells a style part covers: rows `row_start..row_end`, columns
/// `col_start..col_end`. A part's `top`/`bottom`/`left`/`right` borders
/// apply on the region's edges and its `insideH`/`insideV` between the
/// cells within it — so the header row's thick bottom line is the
/// `firstRow` part's `bottom`, even though it is inside the table.
#[derive(Debug, Clone, Copy)]
struct Region {
    row_start: usize,
    row_end: usize,
    col_start: usize,
    col_end: usize,
}

pub(crate) fn cell_style(
    style: &TableStyle,
    flags: &TableStyleFlags,
    at: CellPosition,
) -> CellStyle {
    let is_first_row = flags.first_row && at.row == 0;
    let is_last_row = flags.last_row && at.row + at.row_span == at.rows;
    let is_first_col = flags.first_col && at.column == 0;
    let is_last_col = flags.last_col && at.column + at.column_span == at.columns;
    let all = Region {
        row_start: 0,
        row_end: at.rows,
        col_start: 0,
        col_end: at.columns,
    };
    let own_rows = Region {
        row_start: at.row,
        row_end: at.row + at.row_span,
        ..all
    };
    let own_cols = Region {
        col_start: at.column,
        col_end: at.column + at.column_span,
        ..all
    };

    // Most specific first. Banding counts data rows only: the header row,
    // when it is styled as one, does not shift the stripes.
    let mut parts: Vec<(&TablePartStyle, Region)> = Vec::new();
    fn pick<'a>(
        parts: &mut Vec<(&'a TablePartStyle, Region)>,
        part: &'a Option<TablePartStyle>,
        on: bool,
        region: Region,
    ) {
        if on {
            if let Some(part) = part.as_ref() {
                parts.push((part, region));
            }
        }
    }
    pick(
        &mut parts,
        &style.first_row,
        is_first_row,
        Region {
            row_start: 0,
            row_end: 1,
            ..all
        },
    );
    pick(
        &mut parts,
        &style.last_row,
        is_last_row,
        Region {
            row_start: at.rows.saturating_sub(1),
            ..all
        },
    );
    pick(
        &mut parts,
        &style.first_col,
        is_first_col,
        Region {
            col_start: 0,
            col_end: 1,
            ..all
        },
    );
    pick(
        &mut parts,
        &style.last_col,
        is_last_col,
        Region {
            col_start: at.columns.saturating_sub(1),
            ..all
        },
    );
    if flags.band_rows && !is_first_row && !is_last_row {
        let data_row = at.row - usize::from(flags.first_row);
        if data_row.is_multiple_of(2) {
            pick(&mut parts, &style.band1_h, true, own_rows);
        } else {
            pick(&mut parts, &style.band2_h, true, own_rows);
        }
    }
    if flags.band_cols && !is_first_col && !is_last_col {
        let data_col = at.column - usize::from(flags.first_col);
        if data_col.is_multiple_of(2) {
            pick(&mut parts, &style.band1_v, true, own_cols);
        } else {
            pick(&mut parts, &style.band2_v, true, own_cols);
        }
    }
    pick(&mut parts, &style.whole, true, all);

    let edge = |on_boundary: fn(&Region, &CellPosition) -> bool,
                outer: fn(&TablePartStyle) -> Option<&Outline>,
                inside: fn(&TablePartStyle) -> Option<&Outline>|
     -> Option<Outline> {
        parts
            .iter()
            .find_map(|(part, region)| {
                if on_boundary(region, &at) {
                    outer(part)
                } else {
                    inside(part)
                }
            })
            .cloned()
    };
    CellStyle {
        fill: parts.iter().find_map(|(part, _)| part.fill.clone()),
        left: edge(
            |region, at| at.column == region.col_start,
            |part| part.left.as_ref(),
            |part| part.inside_v.as_ref(),
        ),
        right: edge(
            |region, at| at.column + at.column_span == region.col_end,
            |part| part.right.as_ref(),
            |part| part.inside_v.as_ref(),
        ),
        top: edge(
            |region, at| at.row == region.row_start,
            |part| part.top.as_ref(),
            |part| part.inside_h.as_ref(),
        ),
        bottom: edge(
            |region, at| at.row + at.row_span == region.row_end,
            |part| part.bottom.as_ref(),
            |part| part.inside_h.as_ref(),
        ),
        text: RunProps {
            bold: parts.iter().find_map(|(part, _)| part.bold),
            italic: parts.iter().find_map(|(part, _)| part.italic),
            color: parts.iter().find_map(|(part, _)| part.text_color.clone()),
            ..RunProps::default()
        },
    }
}
