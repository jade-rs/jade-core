use jade_xlsx_layout::{conditional_format::ConditionalStyles, layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue, Color};
fn book() -> jade_xlsx_reader::Workbook {
    read_xlsx(include_bytes!(
        "../../../public/fixtures/visual-conditional-workbook.xlsx"
    ))
    .unwrap()
}
fn layout(book: &jade_xlsx_reader::Workbook) -> jade_xlsx_layout::SheetLayout {
    layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default())
}
fn reference(cell: &str) -> CellRef {
    CellRef::parse(cell).unwrap()
}
fn fill(layout: &jade_xlsx_layout::SheetLayout, cell: &str) -> Option<Color> {
    layout
        .cell(reference(cell))
        .unwrap()
        .fill
        .as_ref()
        .and_then(|fill| fill.foreground)
}
#[test]
fn two_three_color_percent_and_percentile_interpolation_use_numeric_range_unions() {
    let book = book();
    let layout = layout(&book);
    assert_eq!(fill(&layout, "A1"), Some(Color::Argb([255, 255, 0, 0])));
    assert_eq!(fill(&layout, "A3"), Some(Color::Argb([255, 128, 128, 0])));
    assert_eq!(fill(&layout, "A5"), Some(Color::Argb([255, 0, 255, 0])));
    assert_eq!(fill(&layout, "B2"), Some(Color::Argb([255, 255, 128, 0])));
    assert_eq!(fill(&layout, "B3"), Some(Color::Argb([255, 255, 255, 0])));
    assert_eq!(fill(&layout, "F3"), Some(Color::Argb([255, 255, 255, 0])));
    assert_eq!(fill(&layout, "G3"), Some(Color::Argb([255, 255, 255, 0])));
    assert_eq!(fill(&layout, "I3"), None);
    assert_eq!(fill(&layout, "I5"), Some(Color::Argb([255, 0, 255, 0])));
    for cell in ["E1", "E2", "E3"] {
        assert_eq!(fill(&layout, cell), None);
    }
    // The only stored numeric value in E is its cached formula result.
    assert_eq!(fill(&layout, "E5"), Some(Color::Argb([255, 255, 0, 0])));
    assert!(layout.cells.len() < 100);
    assert!(layout.rows.len() < 100);
    assert!(layout
        .conditional_blank_cells_in(Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 300.0
        })
        .is_empty());
}
#[test]
fn bars_have_shared_geometry_show_value_and_priority_without_changing_text() {
    let book = book();
    let layout = layout(&book);
    for (cell, ratio) in [
        ("C1", 0.1),
        ("C3", 0.5),
        ("C5", 0.9),
        ("K1", 0.25),
        ("K5", 0.75),
    ] {
        let cell = layout.cell(reference(cell)).unwrap();
        let bar = cell.data_bar.as_ref().unwrap();
        assert_eq!(bar.rect.x, cell.rect.x + 2.0);
        assert_eq!(bar.rect.width, (cell.rect.width * ratio).round());
        assert_eq!(bar.rect.height, cell.rect.height - 4.0);
        assert!(bar.rect.right() <= cell.rect.right());
    }
    let c = layout.cell(reference("C3")).unwrap();
    assert_eq!(c.text, "50");
    assert!(!c.data_bar.as_ref().unwrap().show_value);
    assert!(
        layout
            .cell(reference("D3"))
            .unwrap()
            .data_bar
            .as_ref()
            .unwrap()
            .show_value
    );
    assert!(layout.cell(reference("H1")).unwrap().data_bar.is_none());
    assert_eq!(fill(&layout, "H1"), Some(Color::Argb([255, 255, 0, 255])));
    assert!(layout.cell(reference("H3")).unwrap().data_bar.is_some());
}
#[test]
fn invalid_formula_extensions_colors_and_inverted_thresholds_are_reported() {
    use jade_xlsx_reader::conditional_format::ConditionalVisual;
    let mut book = book();
    let conditional = ConditionalStyles::new(&book.sheets[0], &book.styles);
    for format in &book.sheets[0].conditional_formats {
        let rule = &format.rules[0];
        assert_eq!(conditional.issue(rule).is_some(), rule.priority == 12);
    }
    let rule = &mut book.sheets[0].conditional_formats[0].rules[0];
    if let Some(ConditionalVisual::ColorScale { thresholds, colors }) = &mut rule.visual {
        thresholds[0].kind = "num".into();
        thresholds[0].value = Some("200".into());
        colors[0] = Color::Theme {
            index: 4,
            tint: 0.4,
            resolved: Some([0x44, 0x72, 0xc4]),
        };
    }
    let conditional = ConditionalStyles::new(&book.sheets[0], &book.styles);
    assert!(conditional
        .issue(&book.sheets[0].conditional_formats[0].rules[0])
        .unwrap()
        .contains("increasing"));
    assert_eq!(fill(&layout(&book), "A3"), None);
}
#[test]
fn edits_hidden_rows_theme_colors_merges_and_extreme_values_recompute_range_results() {
    use jade_xlsx_reader::{conditional_format::ConditionalVisual, CellRange};
    let mut book = book();
    let rule = &mut book.sheets[0].conditional_formats[0].rules[0];
    if let Some(ConditionalVisual::ColorScale { colors, .. }) = &mut rule.visual {
        colors[0] = Color::Theme {
            index: 4,
            tint: 0.4,
            resolved: Some([0x44, 0x72, 0xc4]),
        };
    }
    assert_eq!(
        fill(&layout(&book), "A1"),
        Some(Color::Argb([255, 0x8f, 0xaa, 0xdc]))
    );
    book.sheets[0].rows[4].hidden = true;
    book.sheets[0].rows[4]
        .cells
        .iter_mut()
        .find(|cell| cell.reference == reference("A5"))
        .unwrap()
        .value = CellValue::Number(200.0);
    assert_eq!(
        fill(&layout(&book), "A3"),
        Some(Color::Argb([255, 107, 191, 165]))
    );
    book.sheets[0]
        .merges
        .push(CellRange::parse("K1:L1").unwrap());
    let merged = layout(&book);
    let cell = merged.cell(reference("K1")).unwrap();
    assert!(cell.data_bar.as_ref().unwrap().rect.width > 40.0);
    for (cell, number) in [("A1", -f64::MAX), ("A5", f64::MAX)] {
        book.sheets[0]
            .rows
            .iter_mut()
            .flat_map(|row| &mut row.cells)
            .find(|value| value.reference == reference(cell))
            .unwrap()
            .value = CellValue::Number(number);
    }
    let result = layout(&book);
    assert!(fill(&result, "A3").is_some());
}
