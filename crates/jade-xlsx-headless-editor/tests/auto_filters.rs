use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::OpcPackage;

fn fixture(table: bool, unknown: bool, clear: bool) -> Vec<u8> {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/filtered-subtotal-workbook.xlsx"
    ))
    .unwrap();
    let mut xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    let start = xml.find("<autoFilter").unwrap();
    let end = xml[start..].find("</autoFilter>").unwrap() + start + "</autoFilter>".len();
    let filter = xml[start..end].to_string();
    if table || unknown {
        xml.replace_range(start..end, "");
    }
    if clear {
        xml = xml.replace("filterMode=\"1\"", "filterMode=\"0\"");
    }
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    if table {
        let xml = package
            .part_str("xl/tables/table1.xml")
            .unwrap()
            .unwrap()
            .replace("<autoFilter ref=\"A1:C4\"/>", &filter);
        package.replace("xl/tables/table1.xml", xml.into_bytes());
    }
    package.write().unwrap()
}
fn display(book: &HeadlessWorkbook, reference: &str) -> String {
    book.cell("Operations", reference).unwrap().display
}

#[test]
fn saved_worksheet_and_table_filter_bodies_affect_subtotals_without_reapplying_conditions() {
    for table in [false, true] {
        let bytes = fixture(table, false, false);
        let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
        let before_view = book.bytes().to_vec();
        let views = book.auto_filters("Operations").unwrap();
        assert_eq!(book.bytes(), before_view);
        let view = views.iter().find(|view| view.has_criteria).unwrap();
        assert_eq!(
            view.owner,
            if table {
                "OperationsTable"
            } else {
                "worksheet"
            }
        );
        assert_eq!(view.reference.as_deref(), Some("A1:C4"));
        assert!(view.issues.is_empty());
        assert_eq!(
            view.columns[0].criterion.as_ref().unwrap().children[0].attributes["val"],
            "Beta"
        );
        assert_eq!(display(&book, "B6"), "50");
        assert_eq!(display(&book, "C6"), "50");
        assert_eq!(display(&book, "B10"), "90");
        assert_eq!(display(&book, "C10"), "50");
        let before = book.bytes().to_vec();
        book.set("Operations", "A2", CellInput::Text("Beta".into()))
            .unwrap();
        assert!(
            book.workbook().sheets[0]
                .rows
                .iter()
                .find(|row| row.index == 1)
                .unwrap()
                .hidden
        );
        assert_eq!(display(&book, "C6"), "50");
        book.undo().unwrap();
        assert_eq!(book.bytes(), before);
        let reopened = HeadlessWorkbook::open_for_editing(book.bytes()).unwrap();
        assert_eq!(display(&reopened, "C6"), "50");
        if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
            std::fs::write(
                std::path::Path::new(&dir).join(if table {
                    "filter-table.xlsx"
                } else {
                    "filter-worksheet.xlsx"
                }),
                book.export_bytes().unwrap(),
            )
            .unwrap();
        }
    }
}

#[test]
fn unknown_filter_ranges_are_reported_and_keep_stored_full_subtotal_results() {
    let bytes = fixture(false, true, false);
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    let result = book.recalculate().unwrap();
    assert!(!result.skipped.is_empty());
    assert!(book.calculation_status().requires_recalculation);
    assert_eq!(display(&book, "C6"), "50");
    assert_eq!(display(&book, "B10"), "90");
    assert_eq!(display(&book, "B6"), "50");
    assert!(book
        .auto_filters("Operations")
        .unwrap()
        .iter()
        .any(|view| view
            .issues
            .iter()
            .any(|issue| issue == "filtered row range is unavailable")));
}

#[test]
fn an_explicit_unfiltered_mode_keeps_hidden_rows_manual_and_does_not_reveal_them() {
    let book = HeadlessWorkbook::open_for_editing(&fixture(false, false, true)).unwrap();
    assert_eq!(display(&book, "C6"), "60");
    assert_eq!(display(&book, "B6"), "50");
    assert_eq!(display(&book, "B10"), "100");
    assert_eq!(
        book.auto_filters("Operations").unwrap()[0].filter_mode,
        Some(false)
    );
}

#[test]
fn table_filter_range_inference_excludes_totals_and_filter_dropdowns_alone_do_not_filter() {
    let mut package = OpcPackage::open(&fixture(true, false, false)).unwrap();
    let xml = package
        .part_str("xl/tables/table1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "totalsRowShown=\"0\"",
            "totalsRowShown=\"1\" totalsRowCount=\"1\"",
        )
        .replace("<autoFilter ref=\"A1:C4\">", "<autoFilter>");
    package.replace("xl/tables/table1.xml", xml.into_bytes());
    let mut sheet = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("<row r=\"4\">", "<row r=\"4\" hidden=\"1\">");
    package.replace("xl/worksheets/sheet1.xml", sheet.clone().into_bytes());
    let book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert_eq!(
        book.auto_filters("Operations").unwrap()[0]
            .reference
            .as_deref(),
        Some("A1:C3")
    );
    assert_eq!(display(&book, "C6"), "50");
    assert_eq!(display(&book, "B6"), "20");
    let original_table = package.part_str("xl/tables/table1.xml").unwrap().unwrap();
    package.replace(
        "xl/tables/table1.xml",
        original_table
            .replace("colId=\"0\"", "colId=\"5\"")
            .into_bytes(),
    );
    package.replace(
        "xl/worksheets/sheet1.xml",
        sheet.replace(" filterMode=\"1\"", "").into_bytes(),
    );
    let invalid = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert!(invalid.calculation_status().requires_recalculation);
    assert_eq!(display(&invalid, "C6"), "50");
    assert!(!invalid.auto_filters("Operations").unwrap()[0]
        .issues
        .is_empty());
    package.replace("xl/tables/table1.xml", original_table.into_bytes());
    sheet = sheet.replace("filterMode=\"1\"", "filterMode=\"0\"");
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
    let book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert_eq!(display(&book, "C6"), "60");
    let mut table = package.part_str("xl/tables/table1.xml").unwrap().unwrap();
    let start = table.find("<autoFilter").unwrap();
    let end = table[start..].find("</autoFilter>").unwrap() + start + "</autoFilter>".len();
    table.replace_range(start..end, "<autoFilter/>");
    package.replace("xl/tables/table1.xml", table.into_bytes());
    let book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert!(!book.auto_filters("Operations").unwrap()[0].has_criteria);
    assert_eq!(display(&book, "C6"), "60");
}

#[test]
fn table_filters_without_headers_include_the_first_data_row_in_the_filter_body() {
    let mut package = OpcPackage::open(&fixture(true, false, false)).unwrap();
    let table = package
        .part_str("xl/tables/table1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "totalsRowShown=\"0\"",
            "totalsRowShown=\"0\" headerRowCount=\"0\"",
        );
    package.replace("xl/tables/table1.xml", table.into_bytes());
    let sheet = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("<row r=\"1\">", "<row r=\"1\" hidden=\"1\">")
        .replace(
            "<c r=\"B1\" t=\"inlineStr\"><is><t>Amount</t></is></c>",
            "<c r=\"B1\"><v>5</v></c>",
        )
        .replace("SUBTOTAL(9,B2:B4)", "SUBTOTAL(9,B1:B4)");
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
    let book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert_eq!(display(&book, "C6"), "50");
}
