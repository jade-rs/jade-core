use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{effective_style, FormatChange, HeadlessWorkbook};
use jade_xlsx_reader::CellRef;
const ORIGINAL: &[u8] = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
fn book() -> HeadlessWorkbook {
    let mut package = OpcPackage::open(ORIGINAL).unwrap();
    let sheet = package.part_str("xl/worksheets/sheet1.xml").unwrap().unwrap()
        .replace("<sheetData>", "<cols><col min=\"2\" max=\"2\" width=\"18\" customWidth=\"1\"/><col min=\"3\" max=\"3\" width=\"24\" customWidth=\"1\"/></cols><sheetData>")
        .replace("<row r=\"2\">", "<row r=\"2\" ht=\"30\" customHeight=\"1\">");
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
    let workbook = package
        .part_str("xl/workbook.xml")
        .unwrap()
        .unwrap()
        .replace(
            "</sheets>",
            "<sheet name=\"Target\" sheetId=\"2\" r:id=\"rId999\"/></sheets>",
        );
    package.replace("xl/workbook.xml", workbook.into_bytes());
    let rels = package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap()
        .replace("</Relationships>", "<Relationship Id=\"rId999\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/></Relationships>");
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let types = package.part_str("[Content_Types].xml").unwrap().unwrap()
        .replace("</Types>", "<Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>");
    package.replace("[Content_Types].xml", types.into_bytes());
    package.replace("xl/worksheets/sheet2.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr defaultRowHeight="15"/><cols><col min="8" max="8" width="9" customWidth="1"/><col min="9" max="9" width="11" customWidth="1"/></cols><sheetData><row r="8" ht="21" customHeight="1"><c r="H8"><v>99</v></c><c r="Z8"><v>123</v></c></row></sheetData></worksheet>"#.to_vec());
    HeadlessWorkbook::open(&package.write().unwrap()).unwrap()
}
fn font(workbook: &HeadlessWorkbook, sheet: usize, at: &str) -> jade_xlsx_reader::Font {
    let sheet = &workbook.workbook().sheets[sheet];
    let styles = &workbook.workbook().styles;
    let format = styles.cell_format(Some(effective_style(sheet, CellRef::parse(at).unwrap())));
    format
        .and_then(|format| styles.font(format))
        .cloned()
        .unwrap_or_default()
}

#[test]
fn whole_sheet_copy_and_cut_use_sparse_rows_and_source_defaults() {
    let mut package = OpcPackage::open(book().bytes()).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("defaultRowHeight=\"15\"", "defaultRowHeight=\"18\"");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut workbook = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let clipboard = workbook
        .copy_snapshot("Operations", "A1:XFD1048576", true)
        .unwrap();
    workbook
        .paste_snapshot(&clipboard, "Target", "A1", true)
        .unwrap();
    assert_eq!(
        workbook.workbook().sheets[1].default_row_height_points,
        Some(18.0)
    );
    assert!(workbook
        .workbook()
        .sheets
        .iter()
        .all(|sheet| sheet.rows.len() < 10));
    assert_eq!(workbook.cell("Target", "B2").unwrap().number, Some(10.0));
    assert_eq!(workbook.cell("Target", "C2").unwrap().number, Some(20.0));
    save(&workbook, "clipboard-sheet.xlsx");
}

#[test]
fn error_literals_and_unsupported_formula_caches_survive_pasting() {
    let mut package = OpcPackage::open(book().bytes()).unwrap();
    let xml = package.part_str("xl/worksheets/sheet1.xml").unwrap().unwrap().replace("</sheetData>",
        "<row r=\"7\"><c r=\"D7\" t=\"b\"><f>UNSUPPORTED_CLIPBOARD()</f><v>1</v></c><c r=\"E7\" t=\"str\"><f>UNSUPPORTED_CLIPBOARD()</f><v>cached_x000D_&lt;text&gt;</v></c><c r=\"F7\" t=\"e\"><f>UNSUPPORTED_CLIPBOARD()</f><v>#N/A</v></c><c r=\"G7\" t=\"e\"><v>#DIV/0!</v></c><c r=\"H7\"><f>UNSUPPORTED_CLIPBOARD()</f><v>42</v></c></row></sheetData>");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut workbook = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let clipboard = workbook
        .copy_snapshot("Operations", "D7:H7", false)
        .unwrap();
    workbook
        .paste_snapshot(&clipboard, "Target", "D10", false)
        .unwrap();
    assert_eq!(workbook.cell("Target", "D10").unwrap().display, "TRUE");
    assert_eq!(
        workbook.cell("Target", "E10").unwrap().display,
        "cached\r<text>"
    );
    assert_eq!(workbook.cell("Target", "F10").unwrap().display, "#N/A");
    assert_eq!(workbook.cell("Target", "G10").unwrap().display, "#DIV/0!");
    assert_eq!(workbook.cell("Target", "H10").unwrap().number, Some(42.0));
    save(&workbook, "clipboard-caches.xlsx");
}

#[test]
fn cuts_update_complete_named_and_chart_references_after_the_cell_move() {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/charts-workbook.xlsx"
    ))
    .unwrap();
    let xml = package.part_str("xl/workbook.xml").unwrap().unwrap().replace("</sheets>", "</sheets><definedNames><definedName name=\"MovedAmounts\">Charts!$B$2:$B$5</definedName><definedName name=\"LocalAmount\" localSheetId=\"0\">$B$2</definedName></definedNames>");
    package.replace("xl/workbook.xml", xml.into_bytes());
    let mut workbook = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let clipboard = workbook.copy_snapshot("Charts", "B:B", true).unwrap();
    workbook
        .paste_snapshot(&clipboard, "Charts", "F1", false)
        .unwrap();
    let package = OpcPackage::open(workbook.bytes()).unwrap();
    let chart = package.part_str("xl/charts/chart1.xml").unwrap().unwrap();
    assert!(chart.contains("$F$2:$F$5"));
    assert!(!chart.contains("$B$2:$B$5"));
    assert!(workbook
        .workbook()
        .defined_names
        .iter()
        .find(|name| name.name == "MovedAmounts")
        .unwrap()
        .formula
        .contains("$F$2:$F$5"));
    assert_eq!(
        workbook
            .workbook()
            .defined_names
            .iter()
            .find(|name| name.name == "LocalAmount")
            .unwrap()
            .formula,
        "$F$2"
    );

    save(&workbook, "clipboard-charts.xlsx");
}

#[test]
fn overlapping_row_cuts_preserve_target_contents_and_update_dependencies() {
    let mut workbook = book();
    let clipboard = workbook.copy_snapshot("Operations", "2:3", true).unwrap();
    workbook
        .paste_snapshot(&clipboard, "Operations", "A3", false)
        .unwrap();
    assert!(workbook.cell("Operations", "B2").unwrap().number.is_none());
    assert_eq!(
        workbook.cell("Operations", "B3").unwrap().number,
        Some(10.0)
    );
    assert_eq!(
        workbook.cell("Operations", "C3").unwrap().number,
        Some(20.0)
    );
    assert_eq!(
        workbook.cell("Operations", "B4").unwrap().number,
        Some(20.0)
    );
    assert_eq!(
        workbook.cell("Operations", "C4").unwrap().number,
        Some(40.0)
    );
}
fn save(workbook: &HeadlessWorkbook, name: &str) {
    if let Some(dir) = std::env::var_os("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join(name),
            workbook.export_bytes().unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn a_copy_uses_the_value_and_formula_at_copy_time() {
    let mut workbook = book();
    let clipboard = workbook.copy_snapshot("Operations", "2:2", false).unwrap();
    workbook
        .set_cells("Operations", &[("B2".into(), CellInput::Number(999.0))])
        .unwrap();
    workbook
        .paste_snapshot(&clipboard, "Target", "A8", false)
        .unwrap();
    assert_eq!(workbook.cell("Target", "B8").unwrap().number, Some(10.0));
    assert_eq!(
        workbook.cell("Target", "C8").unwrap().formula.as_deref(),
        Some("B8*2")
    );
    assert_eq!(workbook.cell("Target", "C8").unwrap().number, Some(20.0));
    assert!(workbook.cell("Target", "Z8").unwrap().number.is_none());
    assert_eq!(
        workbook.workbook().sheets[1]
            .rows
            .iter()
            .find(|row| row.index == 7)
            .unwrap()
            .height_points,
        Some(21.0)
    );
    save(&workbook, "clipboard-rows.xlsx");
}
#[test]
fn historical_styles_survive_undo_id_collisions_and_repeated_pastes() {
    let mut workbook = book();
    workbook
        .set_format(
            "Operations",
            "B2",
            &FormatChange {
                bold: Some(true),
                fill_color: Some("AA5500".into()),
                number_format: Some("0.000\" copied\"".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let clipboard = workbook.copy_snapshot("Operations", "B2", false).unwrap();
    workbook.undo().unwrap();
    workbook
        .set_format(
            "Operations",
            "A9",
            &FormatChange {
                italic: Some(true),
                fill_color: Some("0055AA".into()),
                number_format: Some("0.0\" live\"".into()),
                ..Default::default()
            },
        )
        .unwrap();
    workbook
        .paste_snapshot(&clipboard, "Target", "E10", false)
        .unwrap();
    assert!(font(&workbook, 1, "E10").bold);
    assert!(!font(&workbook, 1, "E10").italic);
    assert_eq!(
        workbook.cell("Target", "E10").unwrap().display,
        "10.000 copied"
    );
    let styles = workbook.workbook().styles.clone();
    workbook
        .paste_snapshot(&clipboard, "Target", "E11", false)
        .unwrap();
    assert_eq!(workbook.workbook().styles, styles);
    save(&workbook, "clipboard-styles.xlsx");
}
#[test]
fn full_columns_replace_far_target_contents_and_override_target_row_defaults() {
    let mut workbook = book();
    workbook
        .set_format(
            "Operations",
            "B:C",
            &FormatChange {
                bold: Some(true),
                number_format: Some("0.00".into()),
                ..Default::default()
            },
        )
        .unwrap();
    workbook
        .set_format(
            "Target",
            "8:8",
            &FormatChange {
                italic: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    workbook
        .set_cells("Target", &[("H100".into(), CellInput::Number(88.0))])
        .unwrap();
    let clipboard = workbook.copy_snapshot("Operations", "B:C", false).unwrap();
    workbook
        .paste_snapshot(&clipboard, "Target", "H1", false)
        .unwrap();
    assert!(workbook.cell("Target", "H100").unwrap().number.is_none());
    assert!(font(&workbook, 1, "H8").bold);
    assert!(!font(&workbook, 1, "H8").italic);
    assert!(font(&workbook, 1, "Z8").italic);
    assert_eq!(
        workbook.workbook().sheets[1]
            .column_span(7)
            .unwrap()
            .width_chars,
        Some(9.0)
    );
    assert_eq!(
        workbook.workbook().sheets[1]
            .column_span(8)
            .unwrap()
            .width_chars,
        Some(11.0)
    );
    assert!(
        workbook.workbook().sheets[1]
            .rows
            .iter()
            .map(|row| row.cells.len())
            .sum::<usize>()
            < 30
    );
    workbook
        .paste_snapshot(&clipboard, "Target", "K1", true)
        .unwrap();
    assert_eq!(
        workbook.workbook().sheets[1]
            .column_span(10)
            .unwrap()
            .width_chars,
        Some(18.0)
    );
    assert_eq!(
        workbook.workbook().sheets[1]
            .column_span(11)
            .unwrap()
            .width_chars,
        Some(24.0)
    );
    assert_eq!(
        workbook.cell("Target", "L2").unwrap().formula.as_deref(),
        Some("K2*2")
    );
    save(&workbook, "clipboard-columns.xlsx");
}
#[test]
fn copied_rows_carry_merges_and_optional_heights_and_undo_is_atomic() {
    let mut workbook = book();
    let before = workbook.bytes().to_vec();
    let clipboard = workbook.copy_snapshot("Operations", "1:2", false).unwrap();
    workbook
        .paste_snapshot(&clipboard, "Target", "A10", true)
        .unwrap();
    assert!(workbook.workbook().sheets[1]
        .merges
        .iter()
        .any(|merge| merge.start == CellRef::new(4, 9) && merge.end == CellRef::new(5, 9)));
    assert_eq!(
        workbook.workbook().sheets[1]
            .rows
            .iter()
            .find(|row| row.index == 10)
            .unwrap()
            .height_points,
        Some(30.0)
    );
    save(&workbook, "clipboard-merges.xlsx");
    workbook.undo().unwrap();
    assert_eq!(workbook.bytes(), before);
    workbook.redo().unwrap();
    assert_eq!(workbook.cell("Target", "B11").unwrap().number, Some(10.0));
}
#[test]
fn partial_merges_and_out_of_bounds_targets_fail_without_writing() {
    let mut workbook = book();
    let before = workbook.bytes().to_vec();
    let clipboard = workbook.copy_snapshot("Operations", "E1", false).unwrap();
    assert!(workbook
        .paste_snapshot(&clipboard, "Target", "A1", false)
        .unwrap_err()
        .to_string()
        .contains("merged cell"));
    let clipboard = workbook.copy_snapshot("Operations", "B:C", false).unwrap();
    assert!(workbook
        .paste_snapshot(&clipboard, "Target", "XFD1", false)
        .is_err());
    assert_eq!(workbook.bytes(), before);
}

#[test]
fn oversized_dense_cuts_fail_before_building_source_and_target_grids() {
    let mut workbook = book();
    let clipboard = workbook
        .copy_snapshot("Operations", "A10:D200010", true)
        .unwrap();
    let before = workbook.bytes().to_vec();
    assert!(workbook
        .paste_snapshot(&clipboard, "Target", "A10", false)
        .unwrap_err()
        .to_string()
        .contains("limit"));
    assert_eq!(workbook.bytes(), before);
}
#[test]
fn a_cut_moves_data_in_one_write_and_cannot_clear_intervening_edits() {
    let mut workbook = book();
    let clipboard = workbook.copy_snapshot("Operations", "2:2", true).unwrap();
    workbook
        .paste_snapshot(&clipboard, "Target", "A10", true)
        .unwrap();
    assert!(workbook.cell("Operations", "B2").unwrap().number.is_none());
    assert_eq!(workbook.cell("Target", "B10").unwrap().number, Some(10.0));
    assert_eq!(
        workbook.cell("Target", "C10").unwrap().formula.as_deref(),
        Some("B10*2")
    );
    assert_eq!(workbook.cell("Target", "C10").unwrap().number, Some(20.0));
    save(&workbook, "clipboard-cut.xlsx");
    workbook.undo().unwrap();
    workbook
        .set_cells("Operations", &[("B2".into(), CellInput::Number(999.0))])
        .unwrap();
    let before = workbook.bytes().to_vec();
    assert!(workbook
        .paste_snapshot(&clipboard, "Target", "A10", true)
        .unwrap_err()
        .to_string()
        .contains("invalidated"));
    assert_eq!(workbook.bytes(), before);
}
