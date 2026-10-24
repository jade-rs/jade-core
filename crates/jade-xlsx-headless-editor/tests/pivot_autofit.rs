use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{read_xlsx, OpcPackage};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-style-compat.xlsx");
const LABEL: &str = "A long regional sales territory that needs a wider report column";
fn width(bytes: &[u8], column: u32) -> Option<f32> {
    read_xlsx(bytes).unwrap().sheets[1]
        .column_span(column)
        .and_then(|c| c.width_chars)
}
#[test]
fn refreshed_labels_fit_and_the_width_is_saved_in_the_same_history_step() {
    let mut b = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
    b.set("Data", "A2", CellInput::Text(LABEL.into())).unwrap();
    let before = b.bytes().to_vec();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    let after = b.bytes().to_vec();
    assert!(width(&after, 4).is_some_and(|w| w > 40.));
    assert!(width(&after, 5).is_some_and(|w| w > 5.));
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}
#[test]
fn opting_out_keeps_the_saved_column_width() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotTables/report.xml";
    let xml = p.required_part_str(part).unwrap().replace(
        "name=\"RegionalSales\"",
        "applyWidthHeightFormats=\"0\" name=\"RegionalSales\"",
    );
    p.replace(part, xml.into_bytes());
    let part = "xl/worksheets/sheet2.xml";
    let xml = p.required_part_str(part).unwrap().replace(
        "<sheetData>",
        "<cols><col min=\"5\" max=\"6\" width=\"9\" customWidth=\"1\"/></cols><sheetData>",
    );
    p.replace(part, xml.into_bytes());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    b.set("Data", "A2", CellInput::Text(LABEL.into())).unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(width(b.bytes(), 4), Some(9.));
    assert_eq!(width(b.bytes(), 5), Some(9.));
}

#[test]
fn pivot_permission_allows_refresh_while_column_protection_keeps_saved_widths() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/worksheets/sheet2.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace(
            "<sheetData>",
            "<cols><col min=\"5\" max=\"6\" width=\"9\" customWidth=\"1\"/></cols><sheetData>",
        )
        .replace(
            "</worksheet>",
            "<sheetProtection sheet=\"1\" pivotTables=\"0\" formatColumns=\"1\"/></worksheet>",
        );
    p.replace(part, xml.into_bytes());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    b.set("Data", "A2", CellInput::Text(LABEL.into())).unwrap();
    b.refresh_all_pivots().unwrap();
    assert_eq!(width(b.bytes(), 4), Some(9.));
    assert_eq!(width(b.bytes(), 5), Some(9.));
    assert_eq!(b.cell("Report", "F7").unwrap().number, Some(42.));
}

#[test]
fn failed_measurement_preserves_formula_caches_bytes_and_the_redo_branch() {
    use jade_xlsx_layout::{LayoutOptions, ResolvedFont, TextMeasure};
    use std::rc::Rc;
    struct Bad;
    impl TextMeasure for Bad {
        fn measure(&self, _: &str, _: &ResolvedFont) -> f64 {
            f64::NAN
        }
    }
    for all in [false, true] {
        let mut b = HeadlessWorkbook::open_for_editing(FIXTURE).unwrap();
        b.set("Data", "B2", CellInput::Formula("50+50".into()))
            .unwrap();
        let before = b.bytes().to_vec();
        b.set("Data", "B3", CellInput::Number(99.)).unwrap();
        let future = b.bytes().to_vec();
        assert!(b.undo().unwrap());
        let options = LayoutOptions {
            measure: Rc::new(Bad),
            ..Default::default()
        };
        let failed = if all {
            b.refresh_all_pivots_with_layout(&options).map(|_| ())
        } else {
            b.refresh_pivot_with_layout("Report", "RegionalSales", &options)
                .map(|_| ())
        };
        assert!(failed.is_err());
        assert_eq!(b.bytes(), before);
        assert!(b.redo().unwrap());
        assert_eq!(b.bytes(), future);
    }
}

#[test]
fn disabled_fitting_does_not_require_a_working_font_measurer() {
    use jade_xlsx_layout::{LayoutOptions, ResolvedFont, TextMeasure};
    use std::rc::Rc;
    struct Bad;
    impl TextMeasure for Bad {
        fn measure(&self, _: &str, _: &ResolvedFont) -> f64 {
            panic!("disabled fitting must not measure")
        }
    }
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotTables/report.xml";
    let xml = p.required_part_str(part).unwrap().replace(
        "name=\"RegionalSales\"",
        "applyWidthHeightFormats=\"0\" name=\"RegionalSales\"",
    );
    p.replace(part, xml.into_bytes());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    let o = LayoutOptions {
        measure: Rc::new(Bad),
        max_digit_width_px: f64::NAN,
        ..Default::default()
    };
    b.refresh_all_pivots_with_layout(&o).unwrap();
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(42.));
    assert!(width(b.bytes(), 4).is_none());
}
