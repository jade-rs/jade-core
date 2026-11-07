use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

fn empty_book(date1904: bool) -> HeadlessWorkbook {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#.to_vec());
    if date1904 {
        let xml = package
            .part_str("xl/workbook.xml")
            .unwrap()
            .unwrap()
            .replace("<sheets>", "<workbookPr date1904=\"1\"/><sheets>");
        package.replace("xl/workbook.xml", xml.into_bytes());
    }
    HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap()
}

#[test]
fn empty_holiday_reference_follows_history_and_saved_caches() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2024,1,5)".into())),
            ("C1", CellInput::Formula("DATE(2024,1,8)".into())),
            ("F1", CellInput::Formula("DAY(WORKDAY(B1,1,K1))".into())),
            (
                "G1",
                CellInput::Formula("DAY(WORKDAY.INTL(B1,1,1,K1))".into()),
            ),
            ("H1", CellInput::Formula("NETWORKDAYS(B1,C1,K1)".into())),
            (
                "I1",
                CellInput::Formula("NETWORKDAYS.INTL(B1,C1,1,K1)".into()),
            ),
            ("M1", CellInput::Formula("DAY(WORKDAY(B1,1,))".into())),
            ("N1", CellInput::Formula("NETWORKDAYS.INTL(B1,C1,,)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        for at in ["F1", "G1"] {
            assert_eq!(book.cell(name, at).unwrap().number, Some(8.0));
        }
        for at in ["H1", "I1"] {
            assert_eq!(book.cell(name, at).unwrap().number, Some(2.0));
        }
        book.set(name, "K1", CellInput::Formula("DATE(2024,1,8)".into()))
            .unwrap();
        for at in ["F1", "G1"] {
            assert_eq!(book.cell(name, at).unwrap().number, Some(9.0));
        }
        for at in ["H1", "I1"] {
            assert_eq!(book.cell(name, at).unwrap().number, Some(1.0));
        }
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(8.0));
        book.redo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(9.0));
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            for at in ["F1", "G1", "M1"] {
                assert_eq!(reopened.cell(name, at).unwrap().number, Some(8.0));
            }
            for at in ["H1", "I1", "N1"] {
                assert_eq!(reopened.cell(name, at).unwrap().number, Some(2.0));
            }
        }
    }
}

#[test]
fn networkdays_arrays_share_holidays_through_history_and_saved_caches() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2024,1,5)".into())),
            ("B2", CellInput::Formula("DATE(2024,1,8)".into())),
            ("C1", CellInput::Error("#N/A".into())),
            ("C2", CellInput::Formula("DATE(2024,1,9)".into())),
            ("D1", CellInput::Number(1.0)),
            ("E1", CellInput::Number(11.0)),
            ("K1", CellInput::Formula("DATE(2024,1,8)".into())),
            ("K2", CellInput::Formula("DATE(2024,1,15)".into())),
            (
                "F1",
                CellInput::Formula("NETWORKDAYS(B1:B2,C1:C2,K1:K2)".into()),
            ),
            (
                "H1",
                CellInput::Formula("NETWORKDAYS.INTL(B1:B2,C1:C2,D1:E1,K1:K2)".into()),
            ),
            ("M1", CellInput::Formula("ROWS(H1#)*COLUMNS(H1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "F1").unwrap().display, "#N/A");
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.0));
        book.set(name, "C1", CellInput::Formula("DATE(2024,1,8)".into()))
            .unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
        assert_eq!(book.cell(name, "H1").unwrap().number, Some(1.0));
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(2.0));
        book.set(name, "E1", CellInput::Number(1.0)).unwrap();
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(1.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(2.0));
        book.set(name, "K1", CellInput::Formula("DATE(2024,1,9)".into()))
            .unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(2.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
        book.redo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(2.0));
        book.undo().unwrap();
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            for at in ["F1", "H1", "I1"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
            }
            for at in ["F2", "H2", "I2"] {
                assert_eq!(reopened.cell(name, at).unwrap().number, Some(1.0));
            }
            assert_eq!(reopened.cell(name, "M1").unwrap().number, Some(4.0));
        }
    }
}

#[test]
fn workday_arrays_share_holiday_ranges_through_history_and_saved_caches() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2024,1,5)".into())),
            ("B2", CellInput::Formula("DATE(2024,1,8)".into())),
            ("C1", CellInput::Error("#N/A".into())),
            ("C2", CellInput::Number(1.0)),
            ("D1", CellInput::Number(1.0)),
            ("E1", CellInput::Number(11.0)),
            ("K1", CellInput::Formula("DATE(2024,1,8)".into())),
            ("K2", CellInput::Formula("DATE(2024,1,15)".into())),
            (
                "F1",
                CellInput::Formula("DAY(WORKDAY(B1:B2,C1:C2,K1:K2))".into()),
            ),
            (
                "H1",
                CellInput::Formula("DAY(WORKDAY.INTL(B1:B2,C1:C2,D1:E1,K1:K2))".into()),
            ),
            ("M1", CellInput::Formula("ROWS(H1#)*COLUMNS(H1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "F1").unwrap().display, "#N/A");
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(9.0));
        book.set(name, "C1", CellInput::Number(1.0)).unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(9.0));
        assert_eq!(book.cell(name, "H1").unwrap().number, Some(9.0));
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(6.0));
        book.set(name, "E1", CellInput::Number(1.0)).unwrap();
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(9.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(6.0));
        book.set(name, "K1", CellInput::Formula("DATE(2024,1,9)".into()))
            .unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(8.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(9.0));
        book.redo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(8.0));
        book.undo().unwrap();
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            for at in ["F1", "H1", "I1"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
            }
            for at in ["F2", "H2", "I2"] {
                assert_eq!(reopened.cell(name, at).unwrap().number, Some(9.0));
            }
            assert_eq!(reopened.cell(name, "M1").unwrap().number, Some(4.0));
        }
    }
}

#[test]
fn workday_out_of_range_offsets_follow_history_and_saved_caches() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2024,1,5)".into())),
            ("C1", CellInput::Number(1E100)),
            ("F1", CellInput::Formula("WORKDAY(B1,C1)".into())),
            ("I1", CellInput::Formula("WORKDAY.INTL(B1,C1)".into())),
            ("L1", CellInput::Formula("DATE(2024,1,8)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
        assert_eq!(book.cell(name, "I1").unwrap().display, "#NUM!");
        book.set(name, "C1", CellInput::Number(1.9)).unwrap();
        let expected = book.cell(name, "L1").unwrap().number;
        assert_eq!(book.cell(name, "F1").unwrap().number, expected);
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
        book.redo().unwrap();
        assert_eq!(book.cell(name, "I1").unwrap().number, expected);
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "#NUM!");
            assert_eq!(reopened.cell(name, "I1").unwrap().display, "#NUM!");
        }
    }
}

#[test]
fn time_component_ranges_follow_edits_history_and_saved_caches() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Number(-1.0)),
            ("B2", CellInput::Formula("TIME(13,14,15)".into())),
            ("F1", CellInput::Formula("HOUR(B1:B2)".into())),
            ("I1", CellInput::Formula("MINUTE(B1:B2)".into())),
            ("L1", CellInput::Formula("SECOND(B1:B2)".into())),
            ("O1", CellInput::Formula("ROWS(F1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        for at in ["F1", "I1", "L1"] {
            assert_eq!(book.cell(name, at).unwrap().display, "#NUM!");
        }
        book.set(name, "B1", CellInput::Number(0.5)).unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(12.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
        book.redo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(12.0));
        book.set(name, "B1", CellInput::Error("#N/A".into()))
            .unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            for at in ["F1", "I1", "L1"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
            }
            for (at, expected) in [("F2", 13.0), ("I2", 14.0), ("L2", 15.0), ("O1", 2.0)] {
                assert_eq!(reopened.cell(name, at).unwrap().number, Some(expected));
            }
        }
    }
}

#[test]
fn yearfrac_reversed_ranges_follow_history_and_saved_caches() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2021,1,1)".into())),
            ("C1", CellInput::Formula("DATE(2019,1,1)".into())),
            ("D1", CellInput::Number(5.0)),
            ("D2", CellInput::Number(1.0)),
            ("F1", CellInput::Formula("YEARFRAC(B1,C1,D1:D2)".into())),
            ("I1", CellInput::Formula("ROWS(F1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        let expected = 731.0 / (1096.0 / 3.0);
        assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(expected));
        book.set(name, "C1", CellInput::Formula("DATE(2020,1,1)".into()))
            .unwrap();
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(expected));
        book.redo().unwrap();
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.0));
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "#NUM!");
            assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(expected));
            assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(2.0));
        }
    }
}

#[test]
fn yearfrac_basis_ranges_follow_history_and_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2020,1,1)".into())),
            ("C1", CellInput::Formula("DATE(2021,1,1)".into())),
            ("D1", CellInput::Number(5.0)),
            ("D2", CellInput::Number(4.0)),
            ("F1", CellInput::Formula("YEARFRAC(B1,C1,D1:D2)".into())),
            ("I1", CellInput::Formula("ROWS(F1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.0));
        book.set(name, "D1", CellInput::Number(1.0)).unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
        book.redo().unwrap();
        book.set(name, "D1", CellInput::Error("#N/A".into()))
            .unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "#N/A");
            assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(1.0));
            assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(2.0));
        }
    }
}

#[test]
fn datedif_date_and_unit_ranges_follow_history_and_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Text("bad".into())),
            ("B2", CellInput::Text("2020-01-01".into())),
            ("C1", CellInput::Text("2021-01-01".into())),
            ("D1", CellInput::Text("D".into())),
            ("D2", CellInput::Text("Y".into())),
            ("F1", CellInput::Formula("DATEDIF(B1:B2,C1,D1:D2)".into())),
            ("I1", CellInput::Formula("ROWS(F1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "F1").unwrap().display, "#VALUE!");
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.0));
        book.set(name, "B1", CellInput::Text("2020-01-01".into()))
            .unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(366.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#VALUE!");
        book.redo().unwrap();
        book.set(name, "D1", CellInput::Text("M".into())).unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(12.0));
        book.set(name, "B1", CellInput::Error("#N/A".into()))
            .unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "#N/A");
            assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(1.0));
            assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(2.0));
        }
    }
}

#[test]
fn days360_method_ranges_follow_history_and_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2024,2,29)".into())),
            ("C1", CellInput::Formula("DATE(2024,3,31)".into())),
            ("D1", CellInput::Text("bad".into())),
            ("D2", CellInput::Bool(true)),
            ("F1", CellInput::Formula("DAYS360(B1,C1,D1:D2)".into())),
            ("I1", CellInput::Formula("ROWS(F1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "F1").unwrap().display, "#VALUE!");
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(31.0));
        book.set(name, "D1", CellInput::Bool(false)).unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(30.0));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#VALUE!");
        book.redo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(30.0));
        book.set(name, "D1", CellInput::Error("#N/A".into()))
            .unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "#N/A");
            assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(31.0));
            assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(2.0));
        }
    }
}

#[test]
fn weeknum_and_weekday_return_type_ranges_survive_history_and_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2021,1,1)".into())),
            ("B2", CellInput::Formula("DATE(2021,1,4)".into())),
            ("C1", CellInput::Number(3.0)),
            ("C2", CellInput::Number(21.0)),
            ("F1", CellInput::Formula("WEEKNUM(B1:B2,C1:C2)".into())),
            ("I1", CellInput::Formula("ROWS(F1#)".into())),
            ("L1", CellInput::Formula("WEEKDAY(B1:B2,C1:C2)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.0));
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(2.0));
        assert_eq!(book.cell(name, "L1").unwrap().number, Some(4.0));
        assert_eq!(book.cell(name, "L2").unwrap().display, "#NUM!");
        book.set(name, "C1", CellInput::Number(21.0)).unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(53.0));
        assert_eq!(book.cell(name, "L1").unwrap().display, "#NUM!");
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
        assert_eq!(book.cell(name, "L1").unwrap().number, Some(4.0));
        book.redo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(53.0));
        book.set(name, "B1", CellInput::Error("#N/A".into()))
            .unwrap();
        book.set(name, "C2", CellInput::Number(2.0)).unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "#N/A");
            assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(2.0));
            assert_eq!(reopened.cell(name, "L1").unwrap().display, "#N/A");
            assert_eq!(reopened.cell(name, "L2").unwrap().number, Some(1.0));
            assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(2.0));
        }
    }
}

#[test]
fn datevalue_epoch_bounds_survive_edits_and_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        book.set(name, "B1", CellInput::Text("1899-12-31".into()))
            .unwrap();
        book.set(name, "B2", CellInput::Text("9999-12-31".into()))
            .unwrap();
        book.set(name, "F1", CellInput::Formula("DATEVALUE(B1:B2)".into()))
            .unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#VALUE!");
        let maximum = if date1904 { 2957003.0 } else { 2958465.0 };
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(maximum));
        book.set(name, "B1", CellInput::Text("1904-01-01".into()))
            .unwrap();
        assert_eq!(
            book.cell(name, "F1").unwrap().number,
            Some(if date1904 { 0.0 } else { 1462.0 })
        );
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#VALUE!");
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "#VALUE!");
            assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(maximum));
        }
    }
}

#[test]
fn days_ranges_preserve_fractional_numbers_and_text_semantics_through_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        for (at, input) in [
            ("B1", CellInput::Text("2024-01-02 18:00".into())),
            ("C1", CellInput::Text("2024-01-01 06:00".into())),
            ("B2", CellInput::Formula("DATE(2024,1,2)+0.75".into())),
            ("C2", CellInput::Formula("DATE(2024,1,1)+0.25".into())),
            ("B3", CellInput::Error("#N/A".into())),
            ("C3", CellInput::Formula("DATE(2024,1,1)".into())),
            ("B4", CellInput::Text("bad".into())),
            ("C4", CellInput::Formula("DATE(2024,1,1)".into())),
            ("F1", CellInput::Formula("DAYS(B1:B4,C1:C4)".into())),
            ("I1", CellInput::Formula("ROWS(F1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.5));
        assert_eq!(book.cell(name, "F3").unwrap().display, "#N/A");
        assert_eq!(book.cell(name, "F4").unwrap().display, "#VALUE!");
        book.set(name, "B1", CellInput::Text("bad".into())).unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().display, "#VALUE!");
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.5));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(1.0));
        book.redo().unwrap();
        book.set(name, "C2", CellInput::Formula("DATE(2024,1,1)+0.5".into()))
            .unwrap();
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(1.25));
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "F1").unwrap().display, "#VALUE!");
            assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(1.5));
            assert_eq!(reopened.cell(name, "F3").unwrap().display, "#N/A");
            assert_eq!(reopened.cell(name, "F4").unwrap().display, "#VALUE!");
            assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(4.0));
        }
    }
}

#[test]
fn month_shift_ranges_keep_local_errors_through_history_and_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        let shift = if date1904 { 1462.0 } else { 0.0 };
        for (at, input) in [
            ("B1", CellInput::Formula("DATE(2024,1,31)".into())),
            ("C1", CellInput::Number(0.0)),
            ("D1", CellInput::Number(1.0)),
            ("E1", CellInput::Number(2.0)),
            ("F1", CellInput::Error("#N/A".into())),
            ("H1", CellInput::Formula("EDATE(B1,C1:F1)".into())),
            ("H3", CellInput::Formula("EOMONTH(B1,C1:F1)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(45351.0 - shift));
        assert_eq!(book.cell(name, "I3").unwrap().number, Some(45351.0 - shift));
        book.set(name, "C1", CellInput::Number(1E10)).unwrap();
        assert_eq!(book.cell(name, "H1").unwrap().display, "#NUM!");
        assert_eq!(book.cell(name, "H3").unwrap().display, "#NUM!");
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(45351.0 - shift));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "H1").unwrap().number, Some(45322.0 - shift));
        book.redo().unwrap();
        assert_eq!(book.cell(name, "H1").unwrap().display, "#NUM!");
        book.set(name, "B1", CellInput::Formula("DATE(2024,2,29)".into()))
            .unwrap();
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(45380.0 - shift));
        assert_eq!(book.cell(name, "I3").unwrap().number, Some(45382.0 - shift));
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            for at in ["H1", "H3"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
            }
            assert_eq!(
                reopened.cell(name, "I1").unwrap().number,
                Some(45380.0 - shift)
            );
            assert_eq!(
                reopened.cell(name, "I3").unwrap().number,
                Some(45382.0 - shift)
            );
            assert_eq!(
                reopened.cell(name, "J1").unwrap().number,
                Some(45411.0 - shift)
            );
            assert_eq!(
                reopened.cell(name, "J3").unwrap().number,
                Some(45412.0 - shift)
            );
            for at in ["K1", "K3"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
            }
        }
    }
}

#[test]
fn date_component_errors_and_array_growth_survive_history_and_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        let shift = if date1904 { 1462.0 } else { 0.0 };
        for (at, input) in [
            ("B1", CellInput::Error("#N/A".into())),
            ("B2", CellInput::Number(2025.0)),
            ("C1", CellInput::Number(1.0)),
            ("D1", CellInput::Number(2.0)),
            ("F1", CellInput::Formula("DATE(B1,C1:D1,1)".into())),
            ("I1", CellInput::Formula("COLUMNS(F1#)".into())),
            ("J1", CellInput::Formula("ROWS(F1#)".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        for at in ["F1", "G1"] {
            assert_eq!(book.cell(name, at).unwrap().display, "#N/A");
        }
        assert_eq!(book.cell(name, "I1").unwrap().number, Some(2.0));
        assert_eq!(book.cell(name, "J1").unwrap().number, Some(1.0));
        book.set(name, "B1", CellInput::Number(2024.0)).unwrap();
        assert_eq!(book.cell(name, "F1").unwrap().number, Some(45292.0 - shift));
        assert_eq!(book.cell(name, "G1").unwrap().number, Some(45323.0 - shift));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "G1").unwrap().display, "#N/A");
        book.redo().unwrap();
        book.set(name, "F1", CellInput::Formula("DATE(B1:B2,C1:D1,1)".into()))
            .unwrap();
        assert_eq!(book.cell(name, "J1").unwrap().number, Some(2.0));
        assert_eq!(book.cell(name, "F2").unwrap().number, Some(45658.0 - shift));
        assert_eq!(book.cell(name, "G2").unwrap().number, Some(45689.0 - shift));
        book.set(name, "B1", CellInput::Error("#DIV/0!".into()))
            .unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            for at in ["F1", "G1"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#DIV/0!");
            }
            assert_eq!(
                reopened.cell(name, "F2").unwrap().number,
                Some(45658.0 - shift)
            );
            assert_eq!(
                reopened.cell(name, "G2").unwrap().number,
                Some(45689.0 - shift)
            );
            assert_eq!(reopened.cell(name, "I1").unwrap().number, Some(2.0));
            assert_eq!(reopened.cell(name, "J1").unwrap().number, Some(2.0));
        }
    }
}

#[test]
fn time_component_ranges_broadcast_through_history_and_save() {
    let mut book = empty_book(false);
    let name = "売上サマリ";
    for (at, input) in [
        ("B1", CellInput::Number(0.0)),
        ("B2", CellInput::Number(12.0)),
        ("B3", CellInput::Error("#N/A".into())),
        ("C1", CellInput::Number(0.0)),
        ("D1", CellInput::Number(30.0)),
        ("F1", CellInput::Formula("TIME(B1:B3,C1:D1,0)".into())),
        ("I1", CellInput::Formula("ROWS(F1#)".into())),
        ("J1", CellInput::Formula("COLUMNS(F1#)".into())),
    ] {
        book.set(name, at, input).unwrap();
    }
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(0.0));
    assert_eq!(
        book.cell(name, "G1").unwrap().number,
        Some(1800.0 / 86400.0)
    );
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(0.5));
    assert_eq!(
        book.cell(name, "G2").unwrap().number,
        Some(45000.0 / 86400.0)
    );
    assert_eq!(book.cell(name, "I1").unwrap().number, Some(3.0));
    assert_eq!(book.cell(name, "J1").unwrap().number, Some(2.0));
    for at in ["F3", "G3"] {
        assert_eq!(book.cell(name, at).unwrap().display, "#N/A");
    }
    book.set(name, "B1", CellInput::Number(32768.0)).unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    assert_eq!(book.cell(name, "G1").unwrap().display, "#NUM!");
    assert_eq!(book.cell(name, "F2").unwrap().number, Some(0.5));
    book.undo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().number, Some(0.0));
    book.redo().unwrap();
    assert_eq!(book.cell(name, "F1").unwrap().display, "#NUM!");
    let saved = book.save_bytes().unwrap();
    for reopened in [
        HeadlessWorkbook::open(&saved).unwrap(),
        HeadlessWorkbook::open_for_editing(&saved).unwrap(),
    ] {
        for at in ["F1", "G1"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#NUM!");
        }
        assert_eq!(reopened.cell(name, "F2").unwrap().number, Some(0.5));
        assert_eq!(
            reopened.cell(name, "G2").unwrap().number,
            Some(45000.0 / 86400.0)
        );
        for at in ["F3", "G3"] {
            assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
        }
    }
}

#[test]
fn date_time_text_ranges_keep_error_spills_through_history_and_save() {
    for date1904 in [false, true] {
        let mut book = empty_book(date1904);
        let name = "売上サマリ";
        let shift = if date1904 { 1462.0 } else { 0.0 };
        for (at, input) in [
            ("B1", CellInput::Text("bad".into())),
            ("B2", CellInput::Text("2024-01-02".into())),
            ("B3", CellInput::Error("#N/A".into())),
            ("C1", CellInput::Text("bad".into())),
            ("C2", CellInput::Text("22 June 2000 12:00".into())),
            ("C3", CellInput::Error("#N/A".into())),
            ("A1", CellInput::Formula("DATEVALUE(B1:B4)".into())),
            ("D1", CellInput::Formula("TIMEVALUE(C1:C4)".into())),
            ("G1", CellInput::Formula("ROWS(DATEVALUE(B1:B4))".into())),
            ("H1", CellInput::Formula("TYPE(TIMEVALUE(C2:C2))".into())),
        ] {
            book.set(name, at, input).unwrap();
        }
        assert_eq!(book.cell(name, "A1").unwrap().display, "#VALUE!");
        assert_eq!(book.cell(name, "A2").unwrap().number, Some(45293.0 - shift));
        assert_eq!(book.cell(name, "D1").unwrap().display, "#VALUE!");
        assert_eq!(book.cell(name, "D2").unwrap().number, Some(0.5));
        assert_eq!(book.cell(name, "G1").unwrap().number, Some(4.0));
        assert_eq!(book.cell(name, "H1").unwrap().number, Some(64.0));
        for at in ["A3", "D3"] {
            assert_eq!(book.cell(name, at).unwrap().display, "#N/A");
        }
        for at in ["A4", "D4"] {
            assert_eq!(book.cell(name, at).unwrap().display, "#VALUE!");
        }
        let error_saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&error_saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&error_saved).unwrap(),
        ] {
            assert_eq!(reopened.cell(name, "A1").unwrap().display, "#VALUE!");
            assert_eq!(
                reopened.cell(name, "A2").unwrap().number,
                Some(45293.0 - shift)
            );
            assert_eq!(reopened.cell(name, "D1").unwrap().display, "#VALUE!");
            assert_eq!(reopened.cell(name, "D2").unwrap().number, Some(0.5));
            for at in ["A3", "D3"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
            }
        }
        book.set(name, "B1", CellInput::Text("2024-02-29".into()))
            .unwrap();
        book.set(name, "C1", CellInput::Text("22 JUNE 2000 6:00 AM".into()))
            .unwrap();
        assert_eq!(book.cell(name, "A1").unwrap().number, Some(45351.0 - shift));
        assert_eq!(book.cell(name, "D1").unwrap().number, Some(0.25));
        book.undo().unwrap();
        assert_eq!(book.cell(name, "D1").unwrap().display, "#VALUE!");
        book.redo().unwrap();
        assert_eq!(book.cell(name, "D1").unwrap().number, Some(0.25));
        book.set(name, "C2", CellInput::Text("bad".into())).unwrap();
        assert_eq!(book.cell(name, "D2").unwrap().display, "#VALUE!");
        assert_eq!(book.cell(name, "D1").unwrap().number, Some(0.25));
        book.undo().unwrap();
        let saved = book.save_bytes().unwrap();
        for reopened in [
            HeadlessWorkbook::open(&saved).unwrap(),
            HeadlessWorkbook::open_for_editing(&saved).unwrap(),
        ] {
            assert_eq!(
                reopened.cell(name, "A1").unwrap().number,
                Some(45351.0 - shift)
            );
            assert_eq!(
                reopened.cell(name, "A2").unwrap().number,
                Some(45293.0 - shift)
            );
            assert_eq!(reopened.cell(name, "D1").unwrap().number, Some(0.25));
            assert_eq!(reopened.cell(name, "D2").unwrap().number, Some(0.5));
            for at in ["A3", "D3"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#N/A");
            }
            for at in ["A4", "D4"] {
                assert_eq!(reopened.cell(name, at).unwrap().display, "#VALUE!");
            }
        }
    }
}
