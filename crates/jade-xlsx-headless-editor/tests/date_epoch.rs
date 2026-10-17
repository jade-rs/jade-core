use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{DateSystem, OpcPackage};

#[test]
fn date_epoch_survives_editing_dependency_recalculation_export_and_history() {
    let mut book = HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/date-1904-workbook.xlsx"
    ))
    .unwrap();
    let name = "1904 dates";
    book.set(name, "A2", CellInput::Number(2024.0)).unwrap();
    book.set(name, "B2", CellInput::Formula("DATE(A2,1,1)".into()))
        .unwrap();
    book.set(name, "C2", CellInput::Formula("YEAR(B2)".into()))
        .unwrap();
    book.set(
        name,
        "D2",
        CellInput::Formula("TEXT(B2,\"yyyy-mm-dd\")".into()),
    )
    .unwrap();
    let before = book.bytes().to_vec();
    assert_eq!(book.cell(name, "B2").unwrap().number, Some(43830.0));
    assert_eq!(book.cell(name, "B2").unwrap().display, "2024-01-01");
    book.set(name, "A2", CellInput::Number(2025.0)).unwrap();
    let after = book.bytes().to_vec();
    assert_eq!(book.cell(name, "B2").unwrap().number, Some(44196.0));
    assert_eq!(book.cell(name, "C2").unwrap().display, "2025");
    assert_eq!(book.cell(name, "D2").unwrap().display, "2025-01-01");
    assert_eq!(book.calculation_status().date_system, 1904);
    assert_eq!(book.calculation_status().cached, 3);
    assert!(!book.calculation_status().requires_recalculation);
    let exported = book.export_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&exported).unwrap();
    assert_eq!(reopened.workbook().date_system, DateSystem::Nineteen04);
    assert_eq!(reopened.cell(name, "B2").unwrap().display, "2025-01-01");
    assert_eq!(
        reopened.cell(name, "B2").unwrap().formula.as_deref(),
        Some("DATE(A2,1,1)")
    );
    let package = OpcPackage::open(&exported).unwrap();
    assert!(package
        .part_str("xl/workbook.xml")
        .unwrap()
        .unwrap()
        .contains("date1904=\"1\""));
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), before);
    assert!(book.redo().unwrap());
    assert_eq!(book.bytes(), after);
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("date-1904-edited.xlsx"),
            exported,
        )
        .unwrap();
    }
}

#[test]
fn host_time_zone_changes_refresh_history_without_adding_an_undo_step() {
    ironcalc_base::clock::with_milliseconds(1782873000000, || {
        let mut book = HeadlessWorkbook::open(include_bytes!(
            "../../../public/fixtures/date-1904-workbook.xlsx"
        ))
        .unwrap();
        let name = "1904 dates";
        book.set(name, "B2", CellInput::Formula("TODAY()".into()))
            .unwrap();
        book.set(
            name,
            "C2",
            CellInput::Formula("TEXT(NOW(),\"yyyy-mm-dd hh:mm:ss\")".into()),
        )
        .unwrap();
        book.set(name, "A2", CellInput::Number(1.0)).unwrap();
        let before = book.bytes().to_vec();
        assert_eq!(book.calculation_time_zone(), "UTC");
        assert_eq!(book.cell(name, "B2").unwrap().display, "2026-07-01");
        assert!(book.set_calculation_time_zone("Not/AZone").is_err());
        assert_eq!(book.bytes(), before);
        assert_eq!(book.calculation_time_zone(), "UTC");
        book.set_calculation_time_zone("America/Los_Angeles")
            .unwrap();
        assert_eq!(book.bytes(), before);
        assert!(book.calculation_status().requires_recalculation);
        assert_eq!(
            book.cell(name, "B2").unwrap().formula_state,
            Some(jade_xlsx_headless_api::FormulaState::Stale)
        );
        book.recalculate().unwrap();
        assert_eq!(book.cell(name, "B2").unwrap().display, "2026-06-30");
        assert_eq!(
            book.cell(name, "C2").unwrap().display,
            "2026-06-30 19:30:00"
        );
        assert!(!book.calculation_status().requires_recalculation);
        assert!(book.undo().unwrap());
        assert_eq!(book.cell(name, "A2").unwrap().display, "1904");
        assert_eq!(book.cell(name, "B2").unwrap().display, "2026-06-30");
        assert_eq!(book.calculation_time_zone(), "America/Los_Angeles");
        book.set_calculation_time_zone("Asia/Tokyo").unwrap();
        assert!(book.redo().unwrap());
        assert_eq!(book.cell(name, "A2").unwrap().display, "1");
        assert_eq!(
            book.cell(name, "C2").unwrap().display,
            "2026-07-01 11:30:00"
        );
        let exported = book.export_bytes().unwrap();
        let reopened = HeadlessWorkbook::open_in_time_zone(&exported, "Asia/Tokyo").unwrap();
        assert_eq!(
            reopened.cell(name, "C2").unwrap().display,
            "2026-07-01 11:30:00"
        );
        if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
            std::fs::write(
                std::path::Path::new(&dir).join("date-timezone-edited.xlsx"),
                exported,
            )
            .unwrap();
        }
    });
}

#[test]
fn incomplete_explicit_recalculation_keeps_the_cache_and_requests_recalculation_on_reopen() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/date-1904-workbook.xlsx"
    ))
    .unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("<v>0</v>", "<f>UNKNOWN_CLOCK()</f><v>0</v>");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    assert!(!book.calculation_status().requires_recalculation);
    book.set_calculation_time_zone("Asia/Tokyo").unwrap();
    let result = book.recalculate().unwrap();
    assert_eq!(result.skipped.len(), 1);
    let reopened = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert!(reopened.calculation_status().requires_recalculation);
    assert_eq!(reopened.cell("1904 dates", "B2").unwrap().number, Some(0.0));
}
