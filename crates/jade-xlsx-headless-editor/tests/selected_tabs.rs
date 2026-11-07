//! Grouped sheets saved as Excel saves them: each grouped sheet's tab
//! selected, the others not, and read back so.

use jade_xlsx_headless_editor::HeadlessWorkbook;

fn selected(book: &HeadlessWorkbook) -> Vec<bool> {
    book.workbook()
        .sheets
        .iter()
        .map(|sheet| sheet.tab_selected)
        .collect()
}

#[test]
fn grouped_tabs_are_saved_selected() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    book.add_sheet("Second", None).unwrap();
    book.add_sheet("Third", None).unwrap();
    let history = book.can_undo();
    let saved = book.save_bytes_selecting(&[0, 2]).unwrap();
    assert_eq!(book.can_undo(), history, "a group is not an edit");
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    let count = reopened.workbook().sheets.len();
    let only =
        |picked: &[usize]| -> Vec<bool> { (0..count).map(|i| picked.contains(&i)).collect() };
    assert_eq!(selected(&reopened), only(&[0, 2]));
    // Saved again with only the active sheet, the others lose theirs.
    let mut reopened = reopened;
    let again = reopened.save_bytes_selecting(&[0]).unwrap();
    let back = HeadlessWorkbook::open(&again).unwrap();
    assert_eq!(selected(&back), only(&[0]));
}

const NO_VIEWS: &[u8] = include_bytes!("../../../public/fixtures/pivot-cache-compat.xlsx");

#[test]
fn saving_selected_tabs_preserves_sheets_without_views() {
    let original = jade_xlsx_reader::OpcPackage::open(NO_VIEWS).unwrap();
    let mut book = HeadlessWorkbook::open(NO_VIEWS).unwrap();
    let history = book.can_undo();
    for selected in [&[0][..], &[1][..], &[0, 1][..], &[][..]] {
        let saved = book.save_bytes_selecting(selected).unwrap();
        let package = jade_xlsx_reader::OpcPackage::open(&saved).unwrap();
        for part in ["xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
            assert_eq!(
                package.part(part).unwrap(),
                original.part(part).unwrap(),
                "{part}"
            );
        }
        assert_eq!(book.can_undo(), history);
    }
}

#[test]
fn saving_zoom_and_selection_creates_only_the_requested_view() {
    let original = jade_xlsx_reader::OpcPackage::open(NO_VIEWS).unwrap();
    let mut book = HeadlessWorkbook::open(NO_VIEWS).unwrap();
    let history = book.can_undo();
    let saved = book.save_bytes_viewing(&[0], &[(0, 85)]).unwrap();
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.workbook().sheets[0].zoom_percent, Some(85));
    assert_eq!(selected(&reopened), vec![true, false]);
    let package = jade_xlsx_reader::OpcPackage::open(&saved).unwrap();
    let untouched = "xl/worksheets/sheet2.xml";
    assert_eq!(
        package.part(untouched).unwrap(),
        original.part(untouched).unwrap()
    );
    assert_eq!(book.can_undo(), history);
}
