//! What a sheet's Page Setup says: paper, orientation, scaling, margins and
//! the rows and columns repeated on every page.

use jade_xlsx_reader::{read_xlsx, PageMargins, PageOrientation, PageSetup, Workbook};
use std::path::PathBuf;

fn workbook() -> Workbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/page-setup-workbook.xlsx"
    ));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    read_xlsx(&bytes).expect("read")
}

fn setup(name: &str) -> PageSetup {
    workbook()
        .sheet_by_name(name)
        .expect(name)
        .page_setup
        .clone()
}

#[test]
fn reads_the_paper_orientation_and_margins_not_those_of_a_custom_view() {
    let setup = setup("Letter Landscape");
    assert_eq!(setup.paper_size, Some(1));
    assert_eq!(setup.orientation, Some(PageOrientation::Landscape));
    assert_eq!(setup.scale, None);
    assert_eq!(setup.fit_to(), None);
    assert!(!setup.over_then_down);
    assert_eq!(
        setup.margins,
        Some(PageMargins {
            left: 0.25,
            right: 0.25,
            top: 0.75,
            bottom: 0.75,
            header: 0.3,
            footer: 0.3,
        })
    );
    assert_eq!(setup.title_rows, None);
    assert_eq!(setup.title_columns, None);
}

#[test]
fn reads_a_scale_and_the_page_order() {
    let setup = setup("Half");
    assert_eq!(setup.paper_size, Some(9));
    assert_eq!(setup.orientation, Some(PageOrientation::Portrait));
    assert_eq!(setup.scale, Some(50));
    assert!(setup.over_then_down);
    assert_eq!(setup.margins, None);
}

#[test]
fn fit_to_page_reads_its_pages_wide_and_tall_one_by_default() {
    let wide = setup("Fit Wide");
    assert!(wide.fit_to_page);
    assert_eq!(wide.fit_to(), Some((1, 0)));
    let one = setup("One Page");
    assert_eq!((one.fit_to_width, one.fit_to_height), (None, None));
    assert_eq!(one.fit_to(), Some((1, 1)));
}

#[test]
fn print_titles_are_the_rows_and_columns_of_the_sheets_own_name() {
    let book = workbook();
    let titles = &book.sheet_by_name("Repeat Titles").unwrap().page_setup;
    assert_eq!(titles.title_rows, Some((0, 1)));
    assert_eq!(titles.title_columns, Some((0, 0)));
    for sheet in book.sheets.iter().filter(|s| s.name != "Repeat Titles") {
        assert_eq!(sheet.page_setup.title_rows, None, "{}", sheet.name);
    }
}

#[test]
fn a_sheet_without_page_setup_reads_excels_defaults() {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    let book = read_xlsx(&std::fs::read(path).unwrap()).unwrap();
    let setup = &book.sheets[0].page_setup;
    assert_eq!(setup.fit_to(), None);
    assert_eq!(setup.title_rows, None);
}
