use jade_xlsx_headless_api::{AutoFilterChange, AutoFilterTarget, CellInput, SortKey, SortRequest};
use jade_xlsx_headless_editor::{FormatChange, HeadlessWorkbook};
use jade_xlsx_reader::{Axis, OpcPackage};
const SHEET: &str = "xl/worksheets/sheet1.xml";
fn source() -> Vec<u8> {
    include_bytes!("../../../public/fixtures/filtered-subtotal-workbook.xlsx").to_vec()
}
fn book() -> HeadlessWorkbook {
    HeadlessWorkbook::open_for_editing(&source()).unwrap()
}
fn sort(reference: &str, header: bool, keys: &[(u32, bool)]) -> SortRequest {
    SortRequest {
        reference: reference.into(),
        header,
        case_sensitive: false,
        locale: None,
        phonetic: None,
        keys: keys
            .iter()
            .map(|(column, descending)| SortKey {
                column: *column,
                descending: *descending,
                color: None,
            })
            .collect(),
    }
}
fn text(book: &HeadlessWorkbook, reference: &str) -> String {
    book.cell("Operations", reference).unwrap().display
}
fn part(book: &HeadlessWorkbook, part: &str) -> String {
    OpcPackage::open(book.bytes())
        .unwrap()
        .part_str(part)
        .unwrap()
        .unwrap()
}
fn hidden(book: &HeadlessWorkbook, row: u32) -> bool {
    let s = &book.workbook().sheets[0];
    s.rows
        .iter()
        .find(|r| r.index == row)
        .map(|r| r.hidden)
        .unwrap_or(s.default_rows_hidden)
}
fn export(book: &HeadlessWorkbook, name: &str) {
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join(format!("sort-{name}.xlsx")),
            book.export_bytes().unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn table_sort_moves_records_relative_formulas_styles_filters_and_exact_history() {
    let mut book = book();
    book.set_format(
        "Operations",
        "A4",
        &FormatChange {
            bold: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    let before = book.bytes().to_vec();
    let result = book
        .sort_range("Operations", &sort("A1:C4", true, &[(1, true)]))
        .unwrap();
    assert_eq!(result.moved_rows, 2);
    assert_eq!(result.reference, "A2:C4");
    assert_eq!(text(&book, "A1"), "Name");
    assert_eq!(text(&book, "A2"), "Gamma");
    assert_eq!(text(&book, "A4"), "Alpha");
    assert_eq!(
        book.cell("Operations", "C2").unwrap().formula.as_deref(),
        Some("B2*2")
    );
    assert_eq!(text(&book, "C2"), "60");
    assert_eq!(text(&book, "C6"), "50");
    assert!(!hidden(&book, 1));
    assert!(hidden(&book, 3));
    let s = &book.workbook().sheets[0];
    let style = s
        .rows
        .iter()
        .find(|r| r.index == 1)
        .unwrap()
        .cells
        .iter()
        .find(|c| c.reference.column == 0)
        .unwrap()
        .style_index;
    assert!(
        book.workbook()
            .styles
            .font(book.workbook().styles.cell_format(style).unwrap())
            .unwrap()
            .bold
    );
    assert!(part(&book, "xl/tables/table1.xml").contains("sortState ref=\"A2:C4\""));
    let saved = book.bytes().to_vec();
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    assert_eq!(book.bytes(), saved);
    let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(text(&reopened, "C6"), "50");
    export(&book, "table");
}
#[test]
fn multiple_keys_are_stable_keep_blanks_last_and_do_not_move_outside_cells() {
    let mut book = book();
    for (at, value) in [
        ("H1", "Group"),
        ("I1", "Score"),
        ("J1", "Id"),
        ("H2", "beta"),
        ("H3", "ALPHA"),
        ("H4", "alpha"),
        ("H5", ""),
        ("H6", "alpha"),
        ("J2", "b"),
        ("J3", "first"),
        ("J4", "second"),
        ("J5", "blank"),
        ("J6", "last"),
    ] {
        book.set("Operations", at, CellInput::Text(value.into()))
            .unwrap();
    }
    for (at, value) in [
        ("I2", 2.0),
        ("I3", 3.0),
        ("I4", 3.0),
        ("I5", 100.0),
        ("I6", 1.0),
    ] {
        book.set("Operations", at, CellInput::Number(value))
            .unwrap();
    }
    book.set("Operations", "K2", CellInput::Text("outside".into()))
        .unwrap();
    book.sort_range("Operations", &sort("H1:J6", true, &[(0, false), (1, true)]))
        .unwrap();
    assert_eq!(
        (2..=6)
            .map(|row| text(&book, &format!("J{row}")))
            .collect::<Vec<_>>(),
        ["first", "second", "last", "b", "blank"]
    );
    assert_eq!(text(&book, "K2"), "outside");
    book.sort_range("Operations", &sort("H1:J6", true, &[(0, true), (1, true)]))
        .unwrap();
    assert_eq!(text(&book, "J6"), "blank");
    export(&book, "multi");
}
#[test]
fn manually_hidden_positions_survive_filter_record_movement_and_clear() {
    let mut book = book();
    book.set_axis_size("Operations", Axis::Rows, 2, 2, 0.0)
        .unwrap();
    book.sort_range("Operations", &sort("A1:C4", true, &[(1, true)]))
        .unwrap();
    assert!(hidden(&book, 2));
    assert!(hidden(&book, 3));
    assert_eq!(text(&book, "C6"), "50");
    let saved = book.export_bytes().unwrap();
    let mut book = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Clear {
            target: AutoFilterTarget::Worksheet,
            column: None,
        },
    )
    .unwrap();
    assert!(!hidden(&book, 3));
    assert!(hidden(&book, 2));
    assert_eq!(text(&book, "C6"), "60");
    assert_eq!(text(&book, "B6"), "40");
    export(&book, "manual");
}
#[test]
fn manual_calculation_preserves_cached_formula_results_and_absolute_references() {
    let mut book = book();
    book.set("Operations", "C2", CellInput::Formula("B2+$B$2".into()))
        .unwrap();
    book.set_calculation_properties(jade_xlsx_reader::CalculationMode::Manual, false)
        .unwrap();
    let before = book.bytes().to_vec();
    book.sort_range("Operations", &sort("A1:C4", true, &[(1, true)]))
        .unwrap();
    assert_eq!(
        book.cell("Operations", "C4").unwrap().formula.as_deref(),
        Some("B4+$B$2")
    );
    assert_eq!(text(&book, "C4"), "20");
    assert!(book.calculation_status().requires_recalculation);
    book.recalculate().unwrap();
    assert_eq!(text(&book, "C4"), "40");
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    assert_eq!(book.calculation_status().calculation_mode, "manual");
}
#[test]
fn rich_text_notes_links_and_namespace_aliases_follow_the_record() {
    let mut package = OpcPackage::open(&source()).unwrap();
    let xml = package.part_str(SHEET).unwrap().unwrap();
    let xml=xml.replace("<row r=\"4\">","<row r=\"4\" xmlns:local=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:foreign=\"urn:test\">")
        .replace("<c r=\"A4\" t=\"inlineStr\"><is><t>Gamma</t></is></c>","<local:c r=\"A4\" t=\"inlineStr\" foreign:flag=\"keep\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"foreign\"><local:is><local:r><local:rPr><local:b/></local:rPr><local:t>Gamma</local:t></local:r></local:is><foreign:keep/></local:c>")
        .replace("<tableParts", "<hyperlinks><hyperlink ref=\"A4\" location=\"A1\" tooltip=\"keep\"/></hyperlinks><tableParts");
    package.replace(SHEET, xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.set_note("Operations", "A4", "tester", "Gamma note")
        .unwrap();
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    let rels = jade_opc::parse_relationships(
        &package
            .part_str("xl/worksheets/_rels/sheet1.xml.rels")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let vml = jade_opc::resolve_target(SHEET, rels.target_by_type_suffix("/vmlDrawing").unwrap());
    let foreign = r#"<foreign:ClientData ObjectType="Note"><foreign:Row>3</foreign:Row><foreign:Column>0</foreign:Column></foreign:ClientData>"#;
    let raw=package.part_str(&vml).unwrap().unwrap().replacen("<xml ","<xml xmlns:foreign=\"urn:test\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"foreign\" ",1)
        .replace("<x:ClientData ObjectType=\"Note\">",&format!("{foreign}<x:ClientData ObjectType=\"Note\"><foreign:Row>3</foreign:Row>"));
    package.replace(&vml, raw.into_bytes());
    book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.sort_range("Operations", &sort("A1:C4", true, &[(1, true)]))
        .unwrap();
    assert_eq!(text(&book, "A2"), "Gamma");
    assert_eq!(book.workbook().sheets[0].comments[0].reference.row, 1);
    let xml = part(&book, SHEET);
    assert!(xml.contains("foreign:flag=\"keep\""));
    assert!(xml.contains("<foreign:keep/>"));
    assert!(xml.contains("<local:b/>"));
    assert!(xml.contains("hyperlink ref=\"A2\""));
    let package = OpcPackage::open(book.bytes()).unwrap();
    let rels = jade_opc::parse_relationships(
        &package
            .part_str("xl/worksheets/_rels/sheet1.xml.rels")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let vml = jade_opc::resolve_target(SHEET, rels.target_by_type_suffix("/vmlDrawing").unwrap());
    assert!(part(&book, &vml).contains("<x:Row>1</x:Row>"));
    assert!(part(&book, &vml).contains(foreign));
    assert!(part(&book, &vml).contains("<foreign:Row>3</foreign:Row>"));
    export(&book, "notes");
}
#[test]
fn invalid_keys_partial_tables_merged_ranges_and_missing_formula_results_are_atomic() {
    let mut book = book();
    let before = book.bytes().to_vec();
    for request in [
        sort("A1:C4", true, &[]),
        sort("A1:C4", true, &[(3, false)]),
        sort("A1:C4", true, &[(0, false), (0, true)]),
        sort("A2:B3", false, &[(0, false)]),
        sort("E1:G4", false, &[(0, false)]),
        sort("H1:J100002", false, &[(0, false)]),
    ] {
        assert!(
            book.sort_range("Operations", &request).is_err(),
            "{}",
            request.reference
        );
        assert_eq!(book.bytes(), before);
        assert!(!book.can_undo());
    }
    book.set_calculation_properties(jade_xlsx_reader::CalculationMode::Manual, false)
        .unwrap();
    book.set("Operations", "H2", CellInput::Formula("1+1".into()))
        .unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .sort_range("Operations", &sort("H2:I4", false, &[(0, false)]))
        .is_err());
    assert_eq!(book.bytes(), before);
}

#[test]
fn table_totals_are_excluded_and_existing_sort_extensions_survive() {
    let mut package = OpcPackage::open(&source()).unwrap();
    let table = package
        .part_str("xl/tables/table1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "ref=\"A1:C4\" headerRowCount=\"1\"",
            "ref=\"A1:C5\" headerRowCount=\"1\" totalsRowCount=\"1\"",
        );
    let table = if table.contains("ref=\"A1:C5\"") {
        table
    } else {
        table.replacen("ref=\"A1:C4\"", "ref=\"A1:C5\" totalsRowCount=\"1\"", 1)
    };
    let table=table.replace("<tableColumns", "<sortState ref=\"A2:C4\" xmlns:foreign=\"urn:test\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"foreign\" foreign:flag=\"keep\"><sortCondition ref=\"A2:A4\"/><extLst><ext uri=\"urn:test\"><foreign:keep/></ext></extLst></sortState><tableColumns");
    let foreign = r#"<foreign:autoFilter><sortState xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" ref="A2:C4"><sortCondition ref="A2:A4"/></sortState></foreign:autoFilter>"#;
    let table=table.replacen("<table ","<table xmlns:foreign=\"urn:test\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"foreign\" ",1).replace("<autoFilter",&format!("{foreign}<autoFilter"));
    package.replace("xl/tables/table1.xml", table.into_bytes());
    let xml=package.part_str(SHEET).unwrap().unwrap().replace("<row r=\"6\">","<row r=\"5\"><c r=\"A5\" t=\"inlineStr\"><is><t>Table total</t></is></c><c r=\"B5\"><f>SUBTOTAL(109,B2:B4)</f><v>50</v></c></row><row r=\"6\">");
    package.replace(SHEET, xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let result = book
        .sort_range("Operations", &sort("A1:C5", true, &[(1, true)]))
        .unwrap();
    assert_eq!(result.rows, 3);
    assert_eq!(text(&book, "A5"), "Table total");
    assert_eq!(text(&book, "B5"), "50");
    let table = part(&book, "xl/tables/table1.xml");
    assert!(table.contains("foreign:flag=\"keep\""));
    assert!(table.contains("<foreign:keep/>"));
    assert!(table.contains("ref=\"B2:B4\""));
    assert!(table.contains(foreign));
    assert_eq!(table.matches("sortCondition ref=\"A2:A4\"").count(), 1);
    export(&book, "totals");
}
#[test]
fn shared_formulas_detach_all_members_including_outside_the_sorted_body() {
    let mut package = OpcPackage::open(&source()).unwrap();
    let xml = package
        .part_str(SHEET)
        .unwrap()
        .unwrap()
        .replace(
            "<f>B2*2</f>",
            "<f t=\"shared\" si=\"0\" ref=\"C2:C6\">B2*2</f>",
        )
        .replace("<f>B3*2</f>", "<f t=\"shared\" si=\"0\"/>")
        .replace("<f>B4*2</f>", "<f t=\"shared\" si=\"0\"/>")
        .replace("<f>SUBTOTAL(9,B2:B4)</f>", "<f t=\"shared\" si=\"0\"/>");
    package.replace(SHEET, xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.sort_range("Operations", &sort("A1:C4", true, &[(1, true)]))
        .unwrap();
    assert_eq!(
        book.cell("Operations", "C6").unwrap().formula.as_deref(),
        Some("B6*2")
    );
    assert_eq!(text(&book, "C6"), "100");
    assert!(!part(&book, SHEET).contains("t=\"shared\""));
    assert_eq!(
        book.cell("Operations", "C4").unwrap().formula.as_deref(),
        Some("B4*2")
    );
    export(&book, "shared");
}
#[test]
fn numeric_text_boolean_and_error_categories_sort_by_values_and_keep_empty_cells_last() {
    let mut book = book();
    for (at, value) in [
        ("H1", CellInput::Text("Key".into())),
        ("H2", CellInput::Number(3.0)),
        ("H3", CellInput::Text("2".into())),
        ("H4", CellInput::Bool(false)),
        ("H5", CellInput::Bool(true)),
        ("H6", CellInput::Formula("NA()".into())),
        ("H8", CellInput::Number(-0.0)),
        ("H9", CellInput::Number(0.0)),
    ] {
        book.set("Operations", at, value).unwrap();
    }
    for row in 2..=9 {
        book.set(
            "Operations",
            &format!("I{row}"),
            CellInput::Number(row as f64),
        )
        .unwrap();
    }
    book.sort_range("Operations", &sort("H1:I9", true, &[(0, false)]))
        .unwrap();
    assert_eq!(
        (2..=9)
            .map(|row| text(&book, &format!("I{row}")))
            .collect::<Vec<_>>(),
        ["8", "9", "2", "3", "4", "5", "6", "7"]
    );
    book.sort_range("Operations", &sort("H1:I9", true, &[(0, true)]))
        .unwrap();
    assert_eq!(text(&book, "I9"), "7");
}

#[test]
fn inherited_blank_formats_move_and_array_regions_outside_the_range_are_rejected() {
    let mut book = book();
    book.set_format(
        "Operations",
        "H2:J2",
        &FormatChange {
            bold: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    book.set("Operations", "H2", CellInput::Number(9.0))
        .unwrap();
    book.set("Operations", "H3", CellInput::Number(1.0))
        .unwrap();
    book.sort_range("Operations", &sort("H2:J3", false, &[(0, false)]))
        .unwrap();
    let at = jade_xlsx_reader::CellRef::parse("I3").unwrap();
    let sheet = &book.workbook().sheets[0];
    let style = jade_xlsx_headless_editor::effective_style(sheet, at);
    assert!(
        book.workbook()
            .styles
            .font(book.workbook().styles.cell_format(Some(style)).unwrap())
            .unwrap()
            .bold
    );
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    let xml=package.part_str(SHEET).unwrap().unwrap().replace("</sheetData>","<row r=\"12\"><c r=\"H12\"><f t=\"array\" ref=\"H12:J14\">ROW(H12:J14)</f><v>12</v></c></row></sheetData>");
    package.replace(SHEET, xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .sort_range("Operations", &sort("H13:J14", false, &[(0, false)]))
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
}

#[test]
fn numeric_sheet_selectors_resolve_before_filter_state_and_sort_transactions() {
    let mut named = book();
    let mut indexed = book();
    assert_eq!(
        indexed.sort_columns("0", "A1:C4", true).unwrap()[1].label,
        "Amount (B)"
    );
    let change = AutoFilterChange::Values {
        target: AutoFilterTarget::Worksheet,
        reference: None,
        column: 0,
        values: vec!["Gamma".into()],
        include_blank: false,
    };
    named.change_auto_filter("Operations", &change).unwrap();
    indexed.change_auto_filter("0", &change).unwrap();
    assert_eq!(indexed.bytes(), named.bytes());
    let request = sort("A1:C4", true, &[(1, true)]);
    named.sort_range("Operations", &request).unwrap();
    indexed.sort_range("0", &request).unwrap();
    assert_eq!(indexed.bytes(), named.bytes());
    assert_eq!(text(&indexed, "C6"), "30");
    indexed.undo().unwrap();
    assert_eq!(text(&indexed, "A2"), "Alpha");
    indexed.redo().unwrap();
    assert_eq!(indexed.bytes(), named.bytes());
}

fn text_records(values: &[&str]) -> HeadlessWorkbook {
    let mut book = book();
    book.set("Operations", "H1", CellInput::Text("Key".into()))
        .unwrap();
    for (index, value) in values.iter().enumerate() {
        book.set(
            "Operations",
            &format!("H{}", index + 2),
            CellInput::Text((*value).into()),
        )
        .unwrap();
        book.set(
            "Operations",
            &format!("I{}", index + 2),
            CellInput::Number(index as f64),
        )
        .unwrap();
    }
    book
}
fn ids(book: &HeadlessWorkbook, count: usize) -> Vec<String> {
    (2..count + 2)
        .map(|row| text(book, &format!("I{row}")))
        .collect()
}
#[test]
fn excel_ascii_symbols_case_order_and_ignored_apostrophes_hyphens_follow_documented_rules() {
    let mut book = text_records(&[
        "B", "A", "a", "a-b", "ab", "a'b", ">", "+", "~", " ", "9", "0",
    ]);
    let mut request = sort("H1:I13", true, &[(0, false)]);
    book.sort_range("Operations", &request).unwrap();
    assert_eq!(
        ids(&book, 12),
        ["11", "10", "9", "8", "7", "6", "1", "2", "4", "5", "3", "0"]
    );
    request.case_sensitive = true;
    book.sort_range("Operations", &request).unwrap();
    assert_eq!(
        ids(&book, 12),
        ["11", "10", "9", "8", "7", "6", "2", "4", "5", "3", "1", "0"]
    );
    assert_eq!(text(&book, "C6"), "50");
    assert!(hidden(&book, 1));
}
#[test]
fn locale_comparison_distinguishes_swedish_letters_german_umlauts_and_turkish_case() {
    for (locale, values, expected) in [
        (
            "sv-SE",
            vec!["ö", "ä", "å", "z", "a"],
            vec!["4", "3", "2", "1", "0"],
        ),
        ("de-DE", vec!["z", "ä", "a", "b"], vec!["2", "1", "3", "0"]),
        ("tr-TR", vec!["i", "I", "ı", "İ"], vec!["1", "2", "0", "3"]),
    ] {
        let mut book = text_records(&values);
        let mut request = sort(&format!("H1:I{}", values.len() + 1), true, &[(0, false)]);
        request.locale = Some(locale.into());
        let result = book.sort_range("Operations", &request).unwrap();
        assert_eq!(result.locale, locale);
        assert_eq!(ids(&book, values.len()), expected, "{locale}");
    }
}
#[test]
fn japanese_saved_phonetic_keys_move_without_changing_display_text_and_can_be_disabled() {
    let book = text_records(&["大橋", "大野", "小野"]);
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    let mut xml = package.part_str(SHEET).unwrap().unwrap();
    for (text, reading) in [("大橋", "オオハシ"), ("大野", "オオノ"), ("小野", "オノ")]
    {
        xml = xml.replace(
            &format!(">{text}</t></is>"),
            &format!(">{text}</t><rPh sb=\"0\" eb=\"2\"><t>{reading}</t></rPh></is>"),
        );
    }
    package.replace(SHEET, xml.into_bytes());
    let bytes = package.write().unwrap();
    let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let mut request = sort("H1:I4", true, &[(0, false)]);
    request.locale = Some("ja-JP".into());
    let result = book.sort_range("Operations", &request).unwrap();
    assert!(result.phonetic);
    assert_eq!(ids(&book, 3), ["1", "0", "2"]);
    assert_eq!(text(&book, "H2"), "大野");
    let saved = book.export_bytes().unwrap();
    let read = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(text(&read, "H2"), "大野");
    assert!(part(&book, SHEET).contains("<t>オオノ</t>"));
    book.undo().unwrap();
    assert_eq!(book.bytes(), bytes);
    book.redo().unwrap();
    assert_eq!(ids(&book, 3), ["1", "0", "2"]);
    request.phonetic = Some(false);
    let mut literal = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    literal.sort_range("Operations", &request).unwrap();
    assert_ne!(ids(&literal, 3), ["1", "0", "2"]);
    export(&book, "phonetic");
}
#[test]
fn invalid_locale_and_malformed_guides_fail_before_mutating_history() {
    let mut book = text_records(&["東京", "大阪"]);
    let mut request = sort("H1:I3", true, &[(0, false)]);
    request.locale = Some("not a language tag".into());
    let before = book.bytes().to_vec();
    assert!(book.sort_range("Operations", &request).is_err());
    assert_eq!(book.bytes(), before);
    let mut package = OpcPackage::open(&before).unwrap();
    let xml = package.part_str(SHEET).unwrap().unwrap().replace(
        ">東京</t></is>",
        ">東京</t><rPh sb=\"0\" eb=\"8\"><t>invalid</t></rPh></is>",
    );
    package.replace(SHEET, xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    request.locale = Some("ja-JP".into());
    assert!(book.sort_range("Operations", &request).is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    request.phonetic = Some(false);
    assert!(book.sort_range("Operations", &request).is_ok());
}

#[test]
fn relocated_shared_string_readings_are_preserved_and_unused_bad_hints_do_not_block_sorting() {
    let book = text_records(&["大橋", "大野", "小野"]);
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    let mut xml = package.part_str(SHEET).unwrap().unwrap();
    for row in 2..=4 {
        let start = xml.find(&format!("<c r=\"H{row}\"")).unwrap();
        let end = start + xml[start..].find("</c>").unwrap() + 4;
        xml.replace_range(
            start..end,
            &format!("<c r=\"H{row}\" t=\"s\"><v>{}</v></c>", row - 1),
        );
    }
    package.replace(SHEET, xml.into_bytes());
    let strings = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="3" uniqueCount="4"><si><t>unused</t><rPh sb="0" eb="99"><t>invalid</t></rPh></si><si><t>大橋</t><rPh sb="0" eb="2"><t>オオハシ</t></rPh></si><si><r><t>大野</t></r><rPh sb="0" eb="2"><t>ｵｵﾉ</t></rPh></si><si><t>小野</t><rPh sb="0" eb="2"><t>オノ</t></rPh></si></sst>"#;
    // The unrelated guide is malformed deliberately; exports omit it below.
    package.replace("xl/strings/custom.xml", strings.as_bytes().to_vec());
    let rels=package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace("</Relationships>","<Relationship Id=\"strings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"strings/custom.xml\"/></Relationships>");
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let types=package.part_str("[Content_Types].xml").unwrap().unwrap().replace("</Types>","<Override PartName=\"/xl/strings/custom.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/></Types>");
    package.replace("[Content_Types].xml", types.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let mut request = sort("H1:I4", true, &[(0, false)]);
    request.locale = Some("ja-JP".into());
    book.sort_range("Operations", &request).unwrap();
    assert_eq!(ids(&book, 3), ["1", "0", "2"]);
    assert_eq!(text(&book, "H2"), "大野");
    assert_eq!(part(&book, "xl/strings/custom.xml"), strings);
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    package.replace(
        "xl/strings/custom.xml",
        strings
            .replace("<rPh sb=\"0\" eb=\"99\"><t>invalid</t></rPh>", "")
            .into_bytes(),
    );
    let book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    export(&book, "shared-phonetic");
}

#[test]
fn unrelated_empty_filter_owners_do_not_move_hidden_rows_or_block_active_owner_sorting() {
    let mut book = text_records(&["Z", "X", "Y"]);
    book.set("Operations", "I1", CellInput::Text("Id".into()))
        .unwrap();
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    let xml = package
        .part_str(SHEET)
        .unwrap()
        .unwrap()
        .replace("<autoFilter ref=\"A1:C4\">", "<autoFilter ref=\"H1:I4\">")
        .replace("val=\"Beta\"", "val=\"Y\"")
        .replace("<filter val=\"Gamma\"/>", "");
    package.replace(SHEET, xml.into_bytes());
    let source = package.write().unwrap();
    let mut book = HeadlessWorkbook::open_for_editing(&source).unwrap();
    book.sort_range("Operations", &sort("A1:C4", true, &[(1, true)]))
        .unwrap();
    assert!(hidden(&book, 1));
    assert!(!hidden(&book, 3));
    let mut book = HeadlessWorkbook::open_for_editing(&source).unwrap();
    book.sort_range("Operations", &sort("H1:I4", true, &[(0, false)]))
        .unwrap();
    assert!(!hidden(&book, 1));
    assert!(hidden(&book, 3));
    let mut package = OpcPackage::open(&source).unwrap();
    let table=package.part_str("xl/tables/table1.xml").unwrap().unwrap().replace("<autoFilter ref=\"A1:C4\"/>","<autoFilter ref=\"A1:C4\"><filterColumn colId=\"0\"><filters><filter val=\"Beta\"/><filter val=\"Gamma\"/></filters></filterColumn></autoFilter>");
    package.replace("xl/tables/table1.xml", table.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .sort_range("Operations", &sort("H1:I4", true, &[(0, false)]))
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
}

/// Sort On Cell Color or Font Color: the cells of the colour first (On Top)
/// or last (On Bottom), every other row keeping its order; the same column
/// once per colour, as Excel's levels allow.
#[test]
fn sorting_by_colour_puts_that_colour_on_top_or_bottom() {
    use jade_xlsx_headless_api::SortColor;
    const NAME: &str = "売上サマリ";
    let mut book = HeadlessWorkbook::open_for_editing(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let names = ["a", "b", "c", "d", "e", "f"];
    let cells: Vec<(String, CellInput)> = names
        .iter()
        .enumerate()
        .map(|(row, name)| (format!("K{}", 31 + row), CellInput::Text(name.to_string())))
        .collect();
    book.set_cells(NAME, &cells).unwrap();
    let fill = |book: &mut HeadlessWorkbook, reference: &str, rgb: &str| {
        let change = FormatChange {
            fill_color: Some(rgb.into()),
            ..FormatChange::default()
        };
        book.set_format(NAME, reference, &change).unwrap();
    };
    // b and e yellow, d red.
    fill(&mut book, "K32", "FFFF00");
    fill(&mut book, "K35", "FFFF00");
    fill(&mut book, "K34", "FF0000");
    let column = |book: &HeadlessWorkbook| {
        (31..=36)
            .map(|row| book.cell(NAME, &format!("K{row}")).unwrap().display)
            .collect::<Vec<_>>()
    };
    let by = |rgb: &str, bottom: bool| SortKey {
        column: 0,
        descending: bottom,
        color: Some(SortColor {
            font: false,
            rgb: rgb.into(),
        }),
    };
    let request = |keys: Vec<SortKey>| SortRequest {
        reference: "K31:K36".into(),
        header: false,
        case_sensitive: false,
        locale: None,
        phonetic: None,
        keys,
    };
    book.sort_range(NAME, &request(vec![by("FFFF00", false)]))
        .unwrap();
    assert_eq!(column(&book), ["b", "e", "a", "c", "d", "f"]);
    // Red on top, then yellow: one column, two levels.
    book.sort_range(
        NAME,
        &request(vec![by("FF0000", false), by("FFFF00", false)]),
    )
    .unwrap();
    assert_eq!(column(&book), ["d", "b", "e", "a", "c", "f"]);
    // No fill on bottom.
    book.sort_range(NAME, &request(vec![by("", true)])).unwrap();
    assert_eq!(column(&book), ["d", "b", "e", "a", "c", "f"]);
    book.sort_range(NAME, &request(vec![by("", false)]))
        .unwrap();
    assert_eq!(column(&book), ["a", "c", "f", "d", "b", "e"]);
    // The font's colour.
    let red_text = FormatChange {
        font_color: Some("C00000".into()),
        ..FormatChange::default()
    };
    book.set_format(NAME, "K36", &red_text).unwrap();
    let font = SortKey {
        column: 0,
        descending: false,
        color: Some(SortColor {
            font: true,
            rgb: "C00000".into(),
        }),
    };
    book.sort_range(NAME, &request(vec![font])).unwrap();
    assert_eq!(column(&book)[0], "e");
    // The same colour twice is refused, as the same column twice is.
    assert!(book
        .sort_range(
            NAME,
            &request(vec![by("FFFF00", false), by("FFFF00", true)])
        )
        .is_err());
    // A sort by colour writes no sort state to sort by again.
    let sheet = book.workbook().sheets[0].part_name.clone();
    assert!(!part(&book, &sheet).contains("sortState"));
}
