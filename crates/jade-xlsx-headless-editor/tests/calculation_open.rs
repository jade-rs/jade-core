use jade_xlsx_headless_api::{CellInput, FormulaState};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::OpcPackage;

fn fixture(mode: &str, sheet: impl FnOnce(String) -> String) -> Vec<u8> {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let xml = package.part_str("xl/workbook.xml").unwrap().unwrap();
    package.replace(
        "xl/workbook.xml",
        xml.replace(
            "</workbook>",
            &format!(
                "<calcPr calcId=\"999999\" calcMode=\"{mode}\" fullCalcOnLoad=\"0\"/></workbook>"
            ),
        )
        .into_bytes(),
    );
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    package.replace("xl/worksheets/sheet1.xml", sheet(xml).into_bytes());
    package.write().unwrap()
}

#[test]
fn automatic_open_refreshes_caches_even_without_a_saved_dirty_flag_and_sets_the_undo_baseline() {
    for mode in ["auto", "autoNoTable"] {
        let bytes = fixture(mode, |xml| xml.replace("<v>10</v>", "<v>40</v>"));
        let raw = HeadlessWorkbook::open(&bytes).unwrap();
        assert_eq!(raw.cell("Operations", "C2").unwrap().display, "20");
        assert_eq!(raw.bytes(), bytes);
        let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
        assert_eq!(book.cell("Operations", "C2").unwrap().display, "80");
        assert!(!book.calculation_status().requires_recalculation);
        assert!(!book.can_undo());
        let baseline = book.bytes().to_vec();
        book.set("Operations", "B2", CellInput::Number(50.0))
            .unwrap();
        assert_eq!(book.cell("Operations", "C2").unwrap().display, "100");
        assert!(book.undo().unwrap());
        assert_eq!(book.bytes(), baseline);
        assert!(!book.can_undo());
        let before = OpcPackage::open(&bytes).unwrap();
        let after = OpcPackage::open(book.bytes()).unwrap();
        assert_eq!(
            before.part("xl/tables/table1.xml"),
            after.part("xl/tables/table1.xml")
        );
        if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("calculation-open-{mode}.xlsx")),
                book.export_bytes().unwrap(),
            )
            .unwrap();
        }
    }
}

#[test]
fn manual_open_retains_missing_and_stale_caches_and_the_exact_package() {
    let bytes = fixture("manual", |xml| {
        xml.replace("<v>10</v>", "<v>40</v>")
            .replace("<f>B3*2</f><v>40</v>", "<f>B3*2</f>")
    });
    let bytes = jade_xlsx_writer::mark_for_recalculation(&bytes).unwrap();
    let mut book = HeadlessWorkbook::open_for_editing_in_time_zone(&bytes, "Asia/Tokyo").unwrap();
    assert_eq!(book.cell("Operations", "C2").unwrap().display, "20");
    assert_eq!(
        book.cell("Operations", "C2").unwrap().formula_state,
        Some(FormulaState::Stale)
    );
    assert_eq!(
        book.cell("Operations", "C3").unwrap().formula_state,
        Some(FormulaState::Uncalculated)
    );
    assert!(book.recalculate_on_open().unwrap().is_none());
    assert_eq!(book.bytes(), bytes);
    assert!(!book.can_undo());
    book.recalculate().unwrap();
    assert_eq!(book.cell("Operations", "C2").unwrap().display, "80");
    assert_eq!(book.cell("Operations", "C3").unwrap().display, "40");
}

#[test]
fn automatic_open_refreshes_missing_results_and_preserves_unsupported_results_and_dependents() {
    let bytes = fixture("auto", |xml| {
        xml.replace("<f>B2*2</f>", "<f>UNSUPPORTED_FOR_COMPAT()</f>").replace("</row><row r=\"3\">", "<c r=\"D2\"><f>C2+1</f></c><c r=\"E2\"><f>UNSUPPORTED_FOR_COMPAT()</f></c></row><row r=\"3\">")
    });
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    let summary = book.recalculate_on_open().unwrap().unwrap();
    assert_eq!(summary.skipped.len(), 2);
    assert_eq!(book.cell("Operations", "C2").unwrap().display, "20");
    assert_eq!(book.cell("Operations", "D2").unwrap().display, "21");
    // Nothing was cached for E2: it shows the engine's `#NAME?`, as a
    // misspelt function does in Excel, held only until the full
    // calculation the file now asks for.
    let e2 = book.cell("Operations", "E2").unwrap();
    assert_eq!(e2.display, "#NAME?");
    assert_eq!(e2.formula_state, Some(FormulaState::Stale));
    assert!(book.calculation_status().requires_recalculation);
    assert!(!book.can_undo());
    assert_eq!(
        HeadlessWorkbook::open(book.bytes())
            .unwrap()
            .cell("Operations", "D2")
            .unwrap()
            .display,
        "21"
    );
}

#[test]
fn opening_uses_the_configured_zone_and_refreshes_volatile_functions_in_both_date_epochs() {
    ironcalc_base::clock::with_milliseconds(1782873000000, || {
        for date1904 in [false, true] {
            let bytes = fixture("auto", |xml| {
                xml.replace("<f>B2*2</f>", "<f>TEXT(NOW(),\"yyyy-mm-dd hh:mm:ss\")</f>")
            });
            let mut package = OpcPackage::open(&bytes).unwrap();
            if date1904 {
                let xml = package
                    .part_str("xl/workbook.xml")
                    .unwrap()
                    .unwrap()
                    .replace("<sheets>", "<workbookPr date1904=\"1\"/><sheets>");
                package.replace("xl/workbook.xml", xml.into_bytes());
            }
            let bytes = package.write().unwrap();
            let book =
                HeadlessWorkbook::open_for_editing_in_time_zone(&bytes, "America/Los_Angeles")
                    .unwrap();
            assert_eq!(
                book.cell("Operations", "C2").unwrap().display,
                "2026-06-30 19:30:00"
            );
            let book =
                HeadlessWorkbook::open_for_editing_in_time_zone(&bytes, "Asia/Tokyo").unwrap();
            assert_eq!(
                book.cell("Operations", "C2").unwrap().display,
                "2026-07-01 11:30:00"
            );
        }
    });
}

#[test]
fn opening_a_workbook_without_formulas_does_not_rewrite_it() {
    let bytes = fixture("auto", |xml| {
        xml.replace("<f>B2*2</f>", "")
            .replace("<f>B3*2</f>", "")
            .replace("<f>B4*2</f>", "")
    });
    let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    assert!(book.recalculate_on_open().unwrap().is_none());
    assert_eq!(book.bytes(), bytes);
    assert!(!book.can_undo());
}

#[test]
fn automatic_open_evaluates_dependencies_through_hidden_sheets() {
    let bytes = fixture("auto", |xml| {
        xml.replace("<v>10</v>", "<v>40</v>").replace(
            "</row><row r=\"3\">",
            "<c r=\"D2\"><f>'Hidden Calc'!A1+1</f><v>0</v></c></row><row r=\"3\">",
        )
    });
    let mut package = OpcPackage::open(&bytes).unwrap();
    let xml = package
        .part_str("xl/workbook.xml")
        .unwrap()
        .unwrap()
        .replace(
            "</sheets>",
            "<sheet name=\"Hidden Calc\" sheetId=\"2\" state=\"hidden\" r:id=\"hidden\"/></sheets>",
        );
    package.replace("xl/workbook.xml", xml.into_bytes());
    let xml = package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace(
        "</Relationships>",
        "<Relationship Id=\"hidden\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/></Relationships>",
    );
    package.replace("xl/_rels/workbook.xml.rels", xml.into_bytes());
    let xml = package.part_str("[Content_Types].xml").unwrap().unwrap().replace(
        "</Types>",
        "<Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>",
    );
    package.replace("[Content_Types].xml", xml.into_bytes());
    package.replace("xl/worksheets/sheet2.xml", b"<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\"><f>Operations!C2*3</f><v>1</v></c></row></sheetData></worksheet>".to_vec());
    let book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert_eq!(book.cell("Hidden Calc", "A1").unwrap().display, "240");
    assert_eq!(book.cell("Operations", "D2").unwrap().display, "241");
    assert!(!book.calculation_status().requires_recalculation);
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("calculation-open-hidden.xlsx"),
            book.export_bytes().unwrap(),
        )
        .unwrap();
    }
}
