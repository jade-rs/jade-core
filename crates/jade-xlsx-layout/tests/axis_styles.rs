use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{read_xlsx, CellFormat, CellRef, Color, ColumnSpan, Fill, Font, StyleTable};

fn styles() -> StyleTable {
    StyleTable {
        fonts: vec![
            Font::default(),
            Font {
                bold: true,
                ..Default::default()
            },
        ],
        fills: vec![
            Fill::default(),
            Fill {
                pattern: Some("solid".into()),
                foreground: Some(Color::Argb([255, 51, 170, 102])),
                ..Default::default()
            },
        ],
        cell_formats: vec![
            CellFormat {
                font_index: Some(0),
                fill_index: Some(0),
                ..Default::default()
            },
            CellFormat {
                font_index: Some(1),
                fill_index: Some(1),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn defaults_resolve_for_visible_unstored_cells_with_row_precedence() {
    let mut book = read_xlsx(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let sheet = &mut book.sheets[0];
    let mut row = sheet.rows[0].clone();
    row.index = 4;
    row.cells.clear();
    row.custom_format = true;
    row.style_index = Some(0);
    sheet.rows = vec![row];
    sheet.merges.clear();
    sheet.columns = vec![ColumnSpan {
        first: 1,
        last: 1,
        width_chars: None,
        custom_width: false,
        hidden: false,
        style_index: Some(1),
    }];
    let layout = layout_sheet(
        sheet,
        &styles(),
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            ..Default::default()
        },
    );
    assert!(layout.cells.is_empty());
    assert_eq!(layout.rows.len(), 30);
    let b2 = layout.styled_cell(CellRef::new(1, 1)).unwrap();
    assert!(b2.font.bold);
    assert_eq!(
        b2.fill.as_ref().unwrap().foreground,
        Some(Color::Argb([255, 51, 170, 102]))
    );
    let b5 = layout.styled_cell(CellRef::new(1, 4)).unwrap();
    assert!(!b5.font.bold);
    assert!(b5.fill.is_none());
    assert!(layout.styled_cell(CellRef::new(0, 1)).is_none());
    let visible = layout.blank_cells_in(Rect {
        x: 0.0,
        y: 0.0,
        width: 256.0,
        height: 120.0,
    });
    assert!(visible.len() < 30);
    assert!(visible.iter().all(|cell| cell.rect.intersects(&Rect {
        x: 0.0,
        y: 0.0,
        width: 256.0,
        height: 120.0
    })));
}

#[test]
fn conditional_blank_rules_overlay_axis_defaults_without_erasing_unspecified_font_properties() {
    let mut book = read_xlsx(include_bytes!(
        "../../../public/fixtures/conditional-formats-workbook.xlsx"
    ))
    .unwrap();
    let mut axis_styles = styles();
    axis_styles.differential_styles = book.styles.differential_styles.clone();
    let sheet = &mut book.sheets[0];
    sheet.columns.push(ColumnSpan {
        first: 3,
        last: 3,
        width_chars: None,
        custom_width: false,
        hidden: false,
        style_index: Some(1),
    });
    let layout = layout_sheet(
        sheet,
        &axis_styles,
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            ..Default::default()
        },
    );
    let d2 = layout.styled_cell(CellRef::new(3, 1)).unwrap();
    assert!(d2.font.bold);
    assert_eq!(
        d2.fill.as_ref().unwrap().foreground,
        Some(Color::Argb([255, 255, 255, 0]))
    );
}
