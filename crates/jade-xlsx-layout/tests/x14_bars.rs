use jade_xlsx_layout::{layout_sheet, LayoutOptions, SheetLayout};
use jade_xlsx_reader::{conditional_format::ConditionalVisual, read_xlsx, CellRef};
fn book() -> jade_xlsx_reader::Workbook {
    read_xlsx(include_bytes!(
        "../../../public/fixtures/x14-bars-workbook.xlsx"
    ))
    .unwrap()
}
fn layout(book: &jade_xlsx_reader::Workbook, sheet: usize) -> SheetLayout {
    layout_sheet(&book.sheets[sheet], &book.styles, &LayoutOptions::default())
}
fn cell<'a>(layout: &'a SheetLayout, name: &str) -> &'a jade_xlsx_layout::PlacedCell {
    layout.cell(CellRef::parse(name).unwrap()).unwrap()
}
#[test]
fn automatic_axis_separates_signs_zero_has_no_bar_and_the_border_colors_are_preserved() {
    let book = book();
    let layout = layout(&book, 0);
    for name in ["A1", "A2"] {
        let cell = cell(&layout, name);
        let bar = cell.data_bar.as_ref().unwrap();
        let axis = bar.axis.as_ref().unwrap();
        assert_eq!(bar.color, [255, 0, 0, 255]);
        assert!(bar.reverse);
        assert!((bar.rect.right() - (axis.rect.x + 0.5)).abs() < 1e-9);
        assert_eq!(bar.border, Some([0, 0, 0, 255]));
    }
    let positive = cell(&layout, "A5");
    let bar = positive.data_bar.as_ref().unwrap();
    assert!(!bar.reverse);
    assert_eq!(bar.color, [99, 142, 198, 255]);
    assert!((bar.rect.x - (bar.axis.as_ref().unwrap().rect.x + 0.5)).abs() < 1e-9);
    assert!(bar.rect.right() <= positive.rect.right());
    let zero = cell(&layout, "A3").data_bar.as_ref().unwrap();
    assert_eq!(zero.rect.width, 0.0);
    assert!(zero.axis.is_some());
    assert_eq!(
        cell(&layout, "B1").data_bar.as_ref().unwrap().border,
        Some([0, 128, 0, 255])
    );
}
#[test]
fn midpoint_none_all_positive_and_all_negative_ranges_have_distinct_origins() {
    let book = book();
    let layout = layout(&book, 0);
    let middle = cell(&layout, "B5");
    let bar = middle.data_bar.as_ref().unwrap();
    assert!((bar.rect.x - (middle.rect.x + middle.rect.width / 2.0).round()).abs() < 1e-9);
    assert!(bar.rect.width <= middle.rect.width / 2.0);
    assert!(cell(&layout, "C2")
        .data_bar
        .as_ref()
        .unwrap()
        .axis
        .is_none());
    let positive = cell(&layout, "E1");
    let bar = positive.data_bar.as_ref().unwrap();
    assert!(bar.axis.is_none());
    assert_eq!(bar.rect.width, (positive.rect.width * 0.1).round());
    let negative = cell(&layout, "F5");
    let bar = negative.data_bar.as_ref().unwrap();
    assert!(bar.axis.is_some());
    assert_eq!(bar.rect.width, (negative.rect.width * 0.1).round());
    assert!(bar.reverse);
    let longer = cell(&layout, "F1");
    let bar = longer.data_bar.as_ref().unwrap();
    assert_eq!(bar.rect.x, longer.rect.x + 2.0);
}
#[test]
fn explicit_rtl_and_sheet_context_mirror_bar_geometry_and_gradient_direction() {
    let book = book();
    let ltr = layout(&book, 0);
    let rtl = layout(&book, 1);
    for name in ["A1", "A2", "A3", "A4", "A5"] {
        let first = cell(&ltr, name);
        let second = cell(&rtl, name);
        let a = first.data_bar.as_ref().unwrap();
        let b = second.data_bar.as_ref().unwrap();
        assert_eq!(a.rect.width, b.rect.width);
        assert_eq!(a.reverse, !b.reverse);
        assert!(
            ((a.rect.x - first.rect.x) + (b.rect.right() - second.rect.x) - first.rect.width).abs()
                < 1e-9
        );
    }
    let a = cell(&ltr, "D5").data_bar.as_ref().unwrap();
    let b = cell(&rtl, "D5").data_bar.as_ref().unwrap();
    assert_eq!(a, b);
    assert!(a.gradient && a.reverse);
}
#[test]
fn numeric_xm_thresholds_override_the_fallback_and_unknown_predicates_are_reported() {
    use jade_xlsx_layout::conditional_format::ConditionalStyles;
    let mut book = book();
    let rule = &mut book.sheets[0].conditional_formats[4].rules[0];
    if let Some(ConditionalVisual::DataBar { thresholds, .. }) = &mut rule.visual {
        thresholds[0].kind = "num".into();
        thresholds[0].value = Some("0".into());
        thresholds[1].kind = "num".into();
        thresholds[1].value = Some("200".into());
    }
    let computed = layout(&book, 0);
    assert_eq!(
        cell(&computed, "E5").data_bar.as_ref().unwrap().rect.width,
        (cell(&computed, "E5").rect.width * 0.5).round()
    );
    // MS-XLSX ignores a bare numerical-constant formula operand and uses zero.
    let rule = &mut book.sheets[0].conditional_formats[4].rules[0];
    if let Some(ConditionalVisual::DataBar { thresholds, .. }) = &mut rule.visual {
        thresholds[0].kind = "formula".into();
        thresholds[0].value = Some("50".into());
    }
    let computed = layout(&book, 0);
    assert_eq!(
        cell(&computed, "E1").data_bar.as_ref().unwrap().rect.width,
        (cell(&computed, "E1").rect.width * 0.05).round()
    );
    // Missing numeric xm:f operands likewise use the format's specified zero.
    let rule = &mut book.sheets[0].conditional_formats[4].rules[0];
    if let Some(ConditionalVisual::DataBar { thresholds, .. }) = &mut rule.visual {
        thresholds[0].kind = "num".into();
        thresholds[0].value = None;
    }
    let computed = layout(&book, 0);
    assert_eq!(
        cell(&computed, "E1").data_bar.as_ref().unwrap().rect.width,
        (cell(&computed, "E1").rect.width * 0.05).round()
    );
    let rule = &mut book.sheets[0].conditional_formats[4].rules[0];
    if let Some(ConditionalVisual::DataBar { thresholds, .. }) = &mut rule.visual {
        thresholds[0].kind = "formula".into();
        thresholds[0].value = Some("$A$1".into());
    }
    let styles = ConditionalStyles::new(&book.sheets[0], &book.styles);
    assert!(styles
        .issue(&book.sheets[0].conditional_formats[4].rules[0])
        .is_some());
    assert!(cell(&layout(&book, 0), "E5").data_bar.is_none());
}
