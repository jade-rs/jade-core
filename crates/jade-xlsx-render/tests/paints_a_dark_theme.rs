//! A dark theme, the way Excel's dark mode reads a workbook: the sheet's own
//! surfaces go dark and the default text goes light, the theme's Text 1 and
//! Background 1 slots follow them, and every colour a file states explicitly
//! — a red fill, a blue heading — is left exactly as the author chose.

use std::rc::Rc;

use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{
    Cell, CellFormat, CellRef, CellValue, Color, Fill, Font, Row, Sheet, StyleTable,
};
use jade_xlsx_render::{render_grid, RenderOptions};

const WINDOW: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 400.0,
    height: 200.0,
};

/// A1: text in Text 1 (theme 1, black). A2: Background 1 fill (theme 0,
/// white). A3: an explicit red fill. A4: explicit red text.
fn laid_out() -> SheetLayout {
    let mut styles = StyleTable::default();
    styles.fonts.push(Font {
        color: Some(Color::Theme {
            index: 1,
            tint: 0.0,
            resolved: Some([0x00, 0x00, 0x00]),
        }),
        ..Font::default()
    });
    styles.fonts.push(Font {
        color: Some(Color::Argb([0xFF, 0xFF, 0x00, 0x00])),
        ..Font::default()
    });
    styles.fills.push(Fill {
        pattern: Some("solid".into()),
        foreground: Some(Color::Theme {
            index: 0,
            tint: 0.0,
            resolved: Some([0xFF, 0xFF, 0xFF]),
        }),
        background: None,
    });
    styles.fills.push(Fill {
        pattern: Some("solid".into()),
        foreground: Some(Color::Argb([0xFF, 0xFF, 0x00, 0x00])),
        background: None,
    });
    styles.cell_formats.push(CellFormat {
        font_index: Some(0),
        ..Default::default()
    });
    styles.cell_formats.push(CellFormat {
        fill_index: Some(0),
        ..Default::default()
    });
    styles.cell_formats.push(CellFormat {
        fill_index: Some(1),
        ..Default::default()
    });
    styles.cell_formats.push(CellFormat {
        font_index: Some(1),
        ..Default::default()
    });
    let cell = |row: u32, style: u32, text: &str| Row {
        index: row,
        height_points: None,
        custom_height: false,
        custom_format: false,
        hidden: false,
        style_index: None,
        cells: vec![Cell {
            reference: CellRef::new(0, row),
            value: CellValue::Text(text.into()),
            style_index: Some(style),
            formula: None,
        }],
    };
    let sheet = Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: Vec::new(),
        rows: vec![
            cell(0, 0, "text1"),
            cell(1, 1, "bg1"),
            cell(2, 2, "red-fill"),
            cell(3, 3, "red-text"),
        ],
        merges: Vec::new(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: Vec::new(),
        conditional_formats: Vec::new(),
        auto_filter: None,
        filter_mode: None,
        filter_row_state: None,
        default_column_width_chars: None,
        default_row_height_points: Some(15.0),
        default_row_custom_height: false,
        default_rows_hidden: false,
        show_grid_lines: true,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        rich_text: Default::default(),
        protection: None,
    };
    layout_sheet(
        &sheet,
        &styles,
        &LayoutOptions {
            min_column_count: 4,
            measure: Rc::new(jade_xlsx_layout::HeuristicMeasure),
            ..LayoutOptions::default()
        },
    )
}

fn paint(options: &RenderOptions) -> Vec<Op> {
    let painter = RecordingPainter::with_width_per_char(7.0);
    render_grid(&painter, &laid_out(), WINDOW, None, options).expect("render");
    painter.ops()
}

/// The fill style in force when `text` was drawn.
fn text_colour(ops: &[Op], text: &str) -> Option<String> {
    let mut current = None;
    for op in ops {
        match op {
            Op::SetFillStyle(color) => current = Some(color.clone()),
            Op::FillText(drawn, _, _) if drawn == text => return current,
            _ => {}
        }
    }
    None
}

#[test]
fn the_dark_theme_paints_dark_surfaces_and_light_default_text() {
    let options = RenderOptions::dark();
    let ops = paint(&options);

    // The sheet background and the header bands are the theme's own darks,
    // not the light defaults.
    assert_ne!(options.background.as_deref(), Some("#ffffff"));
    assert!(ops.iter().any(|op| matches!(op, Op::SetFillStyle(color) if Some(color.as_str()) == options.background.as_deref())));
    assert!(ops
        .iter()
        .any(|op| matches!(op, Op::SetFillStyle(color) if *color == options.header.background)));
    assert_ne!(options.header.background, header_style_light_background());

    // Text 1 follows the theme: black in the file, light on a dark sheet.
    assert_eq!(
        text_colour(&ops, "text1").as_deref(),
        Some(options.default_text_color.as_str())
    );
    // A colour the author chose explicitly is untouched.
    assert_eq!(text_colour(&ops, "red-text").as_deref(), Some("#ff0000"));
}

fn header_style_light_background() -> String {
    jade_xlsx_render::HeaderStyle::default().background
}

#[test]
fn background_1_fills_become_the_dark_sheet_and_explicit_fills_stay() {
    let options = RenderOptions::dark();
    let ops = paint(&options);
    let fills: Vec<String> = ops
        .iter()
        .filter_map(|op| match op {
            Op::SetFillStyle(color) => Some(color.clone()),
            _ => None,
        })
        .collect();

    // No white surface survives: Background 1 reads as the dark sheet.
    assert!(!fills.iter().any(|color| color == "#ffffff"), "{fills:?}");
    // The explicit red fill is still red.
    assert!(fills.iter().any(|color| color == "#ff0000"), "{fills:?}");
}

#[test]
fn the_light_theme_is_unchanged() {
    let ops = paint(&RenderOptions::default());
    assert_eq!(text_colour(&ops, "text1").as_deref(), Some("#000000"));
    assert!(ops
        .iter()
        .any(|op| matches!(op, Op::SetFillStyle(color) if color == "#ffffff")));
}
