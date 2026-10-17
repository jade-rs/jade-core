use jade_xlsx_headless_api::{AutoFilterChange, AutoFilterTarget, CellInput};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::OpcPackage;
fn fixture(table: bool) -> Vec<u8> {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/filtered-subtotal-workbook.xlsx"
    ))
    .unwrap();
    if table {
        let mut xml = package
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .unwrap();
        let start = xml.find("<autoFilter").unwrap();
        let end = start + xml[start..].find("</autoFilter>").unwrap() + 13;
        let filter = xml[start..end].to_string();
        xml.replace_range(start..end, "");
        package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
        let xml = package
            .part_str("xl/tables/table1.xml")
            .unwrap()
            .unwrap()
            .replace("<autoFilter ref=\"A1:C4\"/>", &filter);
        package.replace("xl/tables/table1.xml", xml.into_bytes());
    }
    package.write().unwrap()
}
fn target(table: bool) -> AutoFilterTarget {
    if table {
        AutoFilterTarget::Table {
            name: "OperationsTable".into(),
        }
    } else {
        AutoFilterTarget::Worksheet
    }
}
fn rank(table: bool, value: u32, top: bool, percent: bool) -> AutoFilterChange {
    AutoFilterChange::Rank {
        target: target(table),
        reference: None,
        column: 1,
        value,
        top,
        percent,
    }
}
fn text(book: &HeadlessWorkbook, reference: &str) -> String {
    book.cell("Operations", reference).unwrap().display
}
fn hidden(book: &HeadlessWorkbook, row: u32) -> bool {
    book.workbook().sheets[0]
        .rows
        .iter()
        .find(|entry| entry.index == row)
        .is_some_and(|entry| entry.hidden)
}
fn threshold(book: &HeadlessWorkbook, table: bool) -> String {
    book.auto_filters("Operations")
        .unwrap()
        .iter()
        .find(|filter| filter.target == target(table))
        .unwrap()
        .columns
        .iter()
        .find(|column| column.column_id == 1)
        .unwrap()
        .criterion
        .as_ref()
        .unwrap()
        .attributes["filterVal"]
        .clone()
}
fn export(book: &HeadlessWorkbook, name: &str) {
    if let Ok(dir) = std::env::var("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join(name),
            book.export_bytes().unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn top_bottom_items_and_percent_combine_with_other_columns_and_exact_history() {
    for table in [false, true] {
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(table)).unwrap();
        let before = book.bytes().to_vec();
        let summary = book
            .change_auto_filter("Operations", &rank(table, 1, true, false))
            .unwrap();
        assert_eq!(summary.changed_rows, 1);
        assert_eq!(summary.active_columns, 2);
        assert_eq!(threshold(&book, table), "30");
        assert_eq!(text(&book, "C6"), "30");
        assert!(hidden(&book, 7));
        let after = book.bytes().to_vec();
        book.undo().unwrap();
        assert_eq!(book.bytes(), before);
        book.redo().unwrap();
        assert_eq!(book.bytes(), after);
        let reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
        assert_eq!(threshold(&reopened, table), "30");
        assert_eq!(text(&reopened, "C6"), "30");
        export(
            &book,
            if table {
                "rank-table.xlsx"
            } else {
                "rank-worksheet.xlsx"
            },
        );
        book.change_auto_filter("Operations", &rank(table, 50, true, true))
            .unwrap();
        assert_eq!(threshold(&book, table), "20");
        assert_eq!(text(&book, "C6"), "50");
        book.change_auto_filter("Operations", &rank(table, 1, false, false))
            .unwrap();
        assert_eq!(threshold(&book, table), "10");
        assert_eq!(text(&book, "C6"), "0");
        if !table {
            export(&book, "rank-bottom.xlsx");
        }
    }
}
#[test]
fn reapply_refreshes_thresholds_while_edits_calculation_options_and_export_do_not() {
    for table in [false, true] {
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(table)).unwrap();
        book.change_auto_filter("Operations", &rank(table, 2, true, false))
            .unwrap();
        book.set("Operations", "B2", CellInput::Number(100.0))
            .unwrap();
        book.recalculate().unwrap();
        assert_eq!(threshold(&book, table), "20");
        assert_eq!(text(&book, "C6"), "50");
        book.auto_filter_options("Operations", &target(table), 1, None)
            .unwrap();
        assert_eq!(threshold(&book, table), "20");
        let mut book = HeadlessWorkbook::open_for_editing(&book.export_bytes().unwrap()).unwrap();
        let before = book.bytes().to_vec();
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(table),
            },
        )
        .unwrap();
        assert_eq!(threshold(&book, table), "30");
        assert_eq!(text(&book, "C6"), "30");
        let after = book.bytes().to_vec();
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(table),
            },
        )
        .unwrap();
        assert_eq!(book.bytes(), after);
        book.undo().unwrap();
        assert_eq!(book.bytes(), before);
        book.redo().unwrap();
        assert_eq!(book.bytes(), after);
        export(
            &book,
            if table {
                "rank-reapplied-table.xlsx"
            } else {
                "rank-reapplied-worksheet.xlsx"
            },
        );
    }
}
fn mixed(criterion: &str) -> Vec<u8> {
    let mut package = OpcPackage::open(&fixture(false)).unwrap();
    let xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetPr filterMode="0"/><sheetData>
    <row r="1"><c r="B1"><v>999999</v></c></row><row r="2"><c r="B2"><v>-5</v></c></row><row r="3"><c r="B3"><v>0</v></c></row>
    <row r="4"><c r="B4"><f>5+5</f><v>10</v></c></row><row r="5"><c r="B5"><v>10</v></c></row>
    <row r="6"><c r="B6" t="inlineStr"><is><t>99999</t></is></c></row><row r="7"><c r="B7" t="b"><v>1</v></c></row><row r="8"><c r="B8"/></row>
    <row r="9"><c r="B9" t="e"><v>#DIV/0!</v></c></row></sheetData><autoFilter ref="B1:B9">{criterion}</autoFilter></worksheet>"#
    );
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    package.replace(
        "xl/worksheets/_rels/sheet1.xml.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#
            .to_vec(),
    );
    package.write().unwrap()
}
fn mixed_rank(value: u32, top: bool, percent: bool) -> AutoFilterChange {
    AutoFilterChange::Rank {
        target: target(false),
        reference: None,
        column: 0,
        value,
        top,
        percent,
    }
}
#[test]
fn ranking_uses_numbers_and_cached_numeric_formulas_includes_ties_and_ceil_percent() {
    let mut book = HeadlessWorkbook::open_for_editing(&mixed("")).unwrap();
    book.change_auto_filter("Operations", &mixed_rank(1, true, false))
        .unwrap();
    assert_eq!(
        (1..=8)
            .filter(|row| !hidden(&book, *row))
            .collect::<Vec<_>>(),
        [3, 4]
    );
    book.change_auto_filter("Operations", &mixed_rank(26, false, true))
        .unwrap();
    assert_eq!(
        (1..=8)
            .filter(|row| !hidden(&book, *row))
            .collect::<Vec<_>>(),
        [1, 2]
    );
    book.change_auto_filter("Operations", &mixed_rank(500, true, false))
        .unwrap();
    assert_eq!(
        (1..=8)
            .filter(|row| !hidden(&book, *row))
            .collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    export(&book, "rank-mixed.xlsx");
}
#[test]
fn missing_filter_value_uses_excel_zero_default_until_reapply() {
    let mut book = HeadlessWorkbook::open_for_editing(&mixed(
        "<filterColumn colId=\"0\"><top10 val=\"1\"/></filterColumn>",
    ))
    .unwrap();
    let before = book.bytes().to_vec();
    let options = book
        .auto_filter_options("Operations", &target(false), 0, None)
        .unwrap();
    assert_eq!(
        options
            .options
            .iter()
            .filter(|entry| entry.selected == Some(true))
            .count(),
        2
    );
    assert_eq!(book.bytes(), before);
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Reapply {
            target: target(false),
        },
    )
    .unwrap();
    assert_eq!(
        (1..=8)
            .filter(|row| !hidden(&book, *row))
            .collect::<Vec<_>>(),
        [3, 4]
    );
}
#[test]
fn manually_hidden_numeric_rows_still_rank_but_stay_hidden() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    book.set_axis_size("Operations", jade_xlsx_reader::Axis::Rows, 3, 3, 0.0)
        .unwrap();
    book.change_auto_filter("Operations", &rank(false, 1, true, false))
        .unwrap();
    assert_eq!(threshold(&book, false), "30");
    assert!(hidden(&book, 2));
    assert!(hidden(&book, 3));
    assert_eq!(text(&book, "C6"), "30");
    assert_eq!(text(&book, "B6"), "0");
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Clear {
            target: target(false),
            column: Some(1),
        },
    )
    .unwrap();
    assert!(!hidden(&book, 2));
    assert!(hidden(&book, 3));
}
#[test]
fn invalid_metadata_requests_and_fractional_reapply_fail_without_mutation() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    let before = book.bytes().to_vec();
    for change in [
        rank(false, 0, true, false),
        rank(false, 501, true, false),
        rank(false, 101, true, true),
    ] {
        assert!(book.change_auto_filter("Operations", &change).is_err());
        assert_eq!(book.bytes(), before);
        assert!(!book.can_undo());
    }
    for attrs in [
        "val=\"0\"",
        "val=\"501\"",
        "val=\"1\" top=\"2\"",
        "val=\"1\" filterVal=\"NaN\"",
    ] {
        let mut book = HeadlessWorkbook::open_for_editing(&mixed(&format!(
            "<filterColumn colId=\"0\"><top10 {attrs}/></filterColumn>"
        )))
        .unwrap();
        let before = book.bytes().to_vec();
        assert!(book
            .change_auto_filter(
                "Operations",
                &AutoFilterChange::Reapply {
                    target: target(false)
                }
            )
            .is_err());
        assert_eq!(book.bytes(), before);
    }
    let mut book = HeadlessWorkbook::open_for_editing(&mixed(
        "<filterColumn colId=\"0\"><top10 val=\"1.5\" filterVal=\"10\"/></filterColumn>",
    ))
    .unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(false)
            }
        )
        .unwrap_err()
        .to_string()
        .contains("fractional"));
    assert_eq!(book.bytes(), before);
    let change: AutoFilterChange = serde_json::from_str(
        r#"{"operation":"rank","target":{"kind":"worksheet"},"column":1,"value":1}"#,
    )
    .unwrap();
    assert_eq!(change, rank(false, 1, true, false));
}

#[test]
fn cache_refresh_preserves_strict_prefixes_foreign_attributes_and_foreign_rank_nodes() {
    let mut package = OpcPackage::open(&mixed(
        "<filterColumn colId=\"0\"><top10 val=\"1\" filterVal=\"0\"/></filterColumn>",
    ))
    .unwrap();
    let mut xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"",
            "xmlns:s=\"http://purl.oclc.org/ooxml/spreadsheetml/main\" xmlns:foreign=\"urn:test\"",
        );
    for name in [
        "worksheet",
        "sheetPr",
        "sheetData",
        "row",
        "c",
        "v",
        "is",
        "t",
        "autoFilter",
        "filterColumn",
        "top10",
        "f",
    ] {
        xml = xml
            .replace(&format!("<{name}"), &format!("<s:{name}"))
            .replace(&format!("</{name}"), &format!("</s:{name}"));
    }
    xml=xml.replace("<s:top10 val=\"1\" filterVal=\"0\"/>","<foreign:top10 filterVal='888'/><s:top10 val=\"1\" filterVal=\"0\" foreign:note='kept' foreign:filterVal='777'/>");
    xml=xml.replace("</s:worksheet>","<foreign:autoFilter><foreign:filterColumn colId='0'><foreign:top10 filterVal='999'/></foreign:filterColumn></foreign:autoFilter></s:worksheet>");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Reapply {
            target: target(false),
        },
    )
    .unwrap();
    let xml = OpcPackage::open(book.bytes())
        .unwrap()
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(xml.contains("filterVal=\"10\" foreign:note='kept' foreign:filterVal='777'"));
    assert!(xml.contains("<foreign:top10 filterVal='888'/>"));
    assert!(xml.contains("<foreign:top10 filterVal='999'/>"));
}
#[test]
fn table_total_row_is_outside_ranking_samples() {
    let mut package = OpcPackage::open(&fixture(true)).unwrap();
    let xml = package
        .part_str("xl/tables/table1.xml")
        .unwrap()
        .unwrap()
        .replace("ref=\"A1:C4\"", "ref=\"A1:C5\"");
    // Keep the filter's range before the total row, as Excel writes it.
    let xml = xml
        .replace("totalsRowShown=\"0\"", "totalsRowShown=\"1\"")
        .replace("<table ", "<table totalsRowCount=\"1\" ")
        .replace("<autoFilter ref=\"A1:C5\"", "<autoFilter ref=\"A1:C4\"");
    package.replace("xl/tables/table1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.set("Operations", "B5", CellInput::Number(999999.0))
        .unwrap();
    book.change_auto_filter("Operations", &rank(true, 1, true, false))
        .unwrap();
    assert_eq!(threshold(&book, true), "30");
    assert!(!hidden(&book, 4));
    export(&book, "rank-totals.xlsx");
}
#[test]
fn empty_numeric_population_and_manual_calculation_have_stable_caches() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    book.set_calculation_properties(jade_xlsx_reader::CalculationMode::Manual, false)
        .unwrap();
    book.change_auto_filter("Operations", &rank(false, 1, true, false))
        .unwrap();
    assert_eq!(text(&book, "C6"), "50");
    book.recalculate().unwrap();
    assert_eq!(text(&book, "C6"), "30");
    let mut book = HeadlessWorkbook::open_for_editing(&mixed("")).unwrap();
    for reference in ["B2", "B3", "B4", "B5"] {
        book.set("Operations", reference, CellInput::Text("text".into()))
            .unwrap();
    }
    book.change_auto_filter("Operations", &mixed_rank(1, true, false))
        .unwrap();
    assert!((1..=8).all(|row| hidden(&book, row)));
    let before = book.bytes().to_vec();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Reapply {
            target: target(false),
        },
    )
    .unwrap();
    assert_eq!(book.bytes(), before);
}

#[test]
fn ranking_preparation_has_a_deterministic_aggregate_sample_bound() {
    let columns=(0..11).map(|column|format!("<filterColumn colId=\"{column}\"><top10 val=\"1\" filterVal=\"0\"/></filterColumn>")).collect::<String>();
    let mut package = OpcPackage::open(&mixed(&columns)).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("ref=\"B1:B9\"", "ref=\"B1:L100001\"");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(false)
            }
        )
        .unwrap_err()
        .to_string()
        .contains("rank sample evaluation limit"));
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
}
