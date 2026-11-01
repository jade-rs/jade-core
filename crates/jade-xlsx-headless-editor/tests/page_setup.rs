use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, PageMargins, PageOrientation, PageSetup};

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/page-setup-workbook.xlsx"
    ))
    .unwrap()
}

#[test]
fn page_setup_and_print_titles_round_trip_together_and_undo_in_one_step() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.rename_sheet(&sheet, "O'Brien, Sales").unwrap();
    let sheet = "O'Brien, Sales";
    let original = book.workbook().sheets[0].page_setup.clone();
    let setup = PageSetup {
        paper_size: Some(9),
        orientation: Some(PageOrientation::Landscape),
        scale: Some(80),
        fit_to_page: true,
        fit_to_width: Some(1),
        fit_to_height: Some(0),
        over_then_down: true,
        margins: Some(PageMargins {
            left: 0.25,
            right: 0.3,
            ..PageMargins::default()
        }),
        title_rows: Some((0, 2)),
        title_columns: Some((0, 26)),
        center_horizontally: true,
        center_vertically: false,
        print_grid_lines: true,
        print_headings: true,
    };
    book.set_page_setup(sheet, &setup).unwrap();
    assert_eq!(book.workbook().sheets[0].page_setup, setup);
    let saved = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.workbook().sheets[0].page_setup, setup);
    // printOptions has its place, just before the sheet's own pageMargins
    // (after the custom views, which keep theirs).
    let part = reopened.workbook().sheets[0].part_name.clone();
    let xml = OpcPackage::open(&saved)
        .unwrap()
        .required_part_str(&part)
        .unwrap();
    let options = xml.find("<printOptions").expect("printOptions written");
    assert!(options > xml.find("</customSheetViews>").unwrap());
    assert!(options < xml.rfind("<pageMargins").unwrap());
    assert!(xml[options..]
        .starts_with(r#"<printOptions horizontalCentered="1" gridLines="1" headings="1"/>"#));
    book.undo().unwrap();
    assert_eq!(book.workbook().sheets[0].page_setup, original);
    book.redo().unwrap();
    assert_eq!(book.workbook().sheets[0].page_setup, setup);
    book.set_page_setup(sheet, &PageSetup::default()).unwrap();
    assert_eq!(book.workbook().sheets[0].page_setup, PageSetup::default());
    assert!(!book
        .workbook()
        .defined_names
        .iter()
        .any(|n| n.name == "_xlnm.Print_Titles" && n.local_sheet_index == Some(0)));
}

#[test]
fn print_area_does_not_remove_titles_or_other_sheets_names() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    let others: Vec<_> = book
        .workbook()
        .defined_names
        .iter()
        .filter(|n| n.local_sheet_index != Some(0))
        .cloned()
        .collect();
    let range = CellRange::parse("B2:AA100").unwrap();
    book.set_print_area(&sheet, Some(range)).unwrap();
    let area = book
        .workbook()
        .defined_names
        .iter()
        .find(|n| n.name == "_xlnm.Print_Area" && n.local_sheet_index == Some(0))
        .unwrap();
    assert!(area.formula.ends_with("!$B$2:$AA$100"));
    for other in others {
        assert!(book.workbook().defined_names.contains(&other));
    }
    book.set_print_area(&sheet, None).unwrap();
    assert!(!book
        .workbook()
        .defined_names
        .iter()
        .any(|n| n.name == "_xlnm.Print_Area" && n.local_sheet_index == Some(0)));
    book.undo().unwrap();
    assert!(book
        .workbook()
        .defined_names
        .iter()
        .any(|n| n.name == "_xlnm.Print_Area" && n.local_sheet_index == Some(0)));
}

#[test]
fn existing_printer_attributes_sheet_properties_and_custom_views_survive() {
    let mut package = OpcPackage::open(&fixture()).unwrap();
    let part = "xl/worksheets/sheet1.xml";
    let xml = package.required_part_str(part).unwrap();
    // The existing pageSetup carries settings the model does not expose.
    let xml = xml.replace(
        "<pageSetup ",
        "<pageSetup blackAndWhite=\"1\" copies=\"3\" ",
    );
    package.replace(part, xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    let mut setup = book.workbook().sheets[0].page_setup.clone();
    setup.scale = Some(125);
    book.set_page_setup(&sheet, &setup).unwrap();
    let written = OpcPackage::open(book.bytes()).unwrap();
    let xml = written.required_part_str(part).unwrap();
    assert!(xml.contains("blackAndWhite=\"1\""));
    assert!(xml.contains("copies=\"3\""));
    assert!(xml.find("pageMargins").unwrap() < xml.find("<pageSetup ").unwrap());
    for name in package
        .part_names()
        .iter()
        .filter(|n| n.as_str() != part && n.as_str() != "xl/workbook.xml")
    {
        assert_eq!(written.part(name), package.part(name), "{name}");
    }
}

#[test]
fn bad_page_settings_are_atomic() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    let before = book.bytes().to_vec();
    for setup in [
        PageSetup {
            scale: Some(9),
            ..PageSetup::default()
        },
        PageSetup {
            title_rows: Some((5, 2)),
            ..PageSetup::default()
        },
        PageSetup {
            margins: Some(PageMargins {
                left: f64::NAN,
                ..PageMargins::default()
            }),
            ..PageSetup::default()
        },
    ] {
        assert!(book
            .set_page_setup(&sheet, &setup)
            .unwrap_err()
            .is_invalid_input());
        assert_eq!(book.bytes(), before);
    }
    assert!(!book.can_undo());
}
