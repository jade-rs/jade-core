use jade_xlsx_headless_api::{AutoFilterChange, AutoFilterTarget, CellInput};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, OpcPackage};

fn fixture(table: bool, manual: bool) -> Vec<u8> {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/filtered-subtotal-workbook.xlsx"
    ))
    .unwrap();
    let mut sheet = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    if manual {
        sheet = sheet.replace("filterMode=\"1\"", "filterMode=\"0\"");
    }
    if table {
        let start = sheet.find("<autoFilter").unwrap();
        let end = start + sheet[start..].find("</autoFilter>").unwrap() + "</autoFilter>".len();
        let filter = sheet[start..end].to_string();
        sheet.replace_range(start..end, "");
        let xml = package
            .part_str("xl/tables/table1.xml")
            .unwrap()
            .unwrap()
            .replace("<autoFilter ref=\"A1:C4\"/>", &filter);
        package.replace("xl/tables/table1.xml", xml.into_bytes());
    }
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
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
fn values(table: bool, items: &[&str]) -> AutoFilterChange {
    AutoFilterChange::Values {
        target: target(table),
        reference: None,
        column: 0,
        values: items.iter().map(|value| value.to_string()).collect(),
        include_blank: false,
    }
}
fn text(book: &HeadlessWorkbook, reference: &str) -> String {
    book.cell("Operations", reference).unwrap().display
}
fn hidden(book: &HeadlessWorkbook, row: u32) -> bool {
    book.workbook().sheets[0]
        .rows
        .iter()
        .find(|item| item.index == row)
        .map(|row| row.hidden)
        .unwrap_or(book.workbook().sheets[0].default_rows_hidden)
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
fn conditions_visibility_caches_and_original_metadata_move_as_one_history_step() {
    for table in [false, true] {
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(table, false)).unwrap();
        let before = book.bytes().to_vec();
        let options = book
            .auto_filter_options("Operations", &target(table), 0, None)
            .unwrap();
        assert_eq!(
            options
                .options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "Beta", "Gamma"]
        );
        assert_eq!(book.bytes(), before);
        let original = OpcPackage::open(book.bytes()).unwrap();
        let result = book
            .change_auto_filter("Operations", &values(table, &["Gamma"]))
            .unwrap();
        assert_eq!(result.changed_rows, 1);
        assert_eq!(result.active_columns, 1);
        assert!(hidden(&book, 1));
        assert!(hidden(&book, 2));
        assert!(!hidden(&book, 3));
        assert!(hidden(&book, 7));
        assert_eq!(text(&book, "C6"), "30");
        assert_eq!(text(&book, "B6"), "30");
        assert_eq!(text(&book, "B10"), "70");
        let after = book.bytes().to_vec();
        let written = OpcPackage::open(&after).unwrap();
        assert_eq!(
            original.part("xl/styles.xml"),
            written.part("xl/styles.xml")
        );
        assert_eq!(
            original.part("xl/worksheets/_rels/sheet1.xml.rels"),
            written.part("xl/worksheets/_rels/sheet1.xml.rels")
        );
        book.undo().unwrap();
        assert_eq!(book.bytes(), before);
        assert!(!book.can_undo());
        book.redo().unwrap();
        assert_eq!(book.bytes(), after);
        let reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
        assert_eq!(text(&reopened, "C6"), "30");
        export(
            &book,
            if table {
                "filter-values-table.xlsx"
            } else {
                "filter-values-worksheet.xlsx"
            },
        );
        let before = book.bytes().to_vec();
        book.change_auto_filter("Operations", &values(table, &["Gamma"]))
            .unwrap();
        assert_eq!(book.bytes(), before);
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(table),
                column: None,
            },
        )
        .unwrap();
        assert!(!hidden(&book, 1));
        assert!(!hidden(&book, 2));
        assert!(hidden(&book, 7));
        assert_eq!(text(&book, "C6"), "60");
        assert_eq!(text(&book, "B6"), "60");
        assert_eq!(text(&book, "B10"), "100");
    }
}

#[test]
fn edits_keep_filters_nondynamic_until_reapply_without_changing_the_saved_predicate() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false, false)).unwrap();
    book.set("Operations", "A2", CellInput::Text("Beta".into()))
        .unwrap();
    assert!(hidden(&book, 1));
    assert_eq!(text(&book, "C6"), "50");
    let filter = book.auto_filters("Operations").unwrap();
    let before = book.bytes().to_vec();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Reapply {
            target: target(false),
        },
    )
    .unwrap();
    assert!(!hidden(&book, 1));
    assert_eq!(text(&book, "C6"), "60");
    let after = book.auto_filters("Operations").unwrap();
    for (before, after) in filter.iter().zip(&after) {
        assert_eq!(before.reference, after.reference);
        assert_eq!(before.columns, after.columns);
        assert_eq!(after.filter_mode, Some(false));
    }
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
}

#[test]
fn manual_hides_survive_filter_changes_clear_and_history_and_stay_in_full_subtotals() {
    for table in [false, true] {
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(table, true)).unwrap();
        book.change_auto_filter("Operations", &values(table, &["Beta", "Gamma"]))
            .unwrap();
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(table),
                column: None,
            },
        )
        .unwrap();
        assert!(hidden(&book, 1));
        assert_eq!(text(&book, "C6"), "60");
        assert_eq!(text(&book, "B6"), "50");
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(table, false)).unwrap();
        book.set_axis_size("Operations", Axis::Rows, 2, 2, 0.0)
            .unwrap();
        assert_eq!(text(&book, "C6"), "50");
        assert_eq!(text(&book, "B6"), "30");
        let before = book.bytes().to_vec();
        book.change_auto_filter("Operations", &values(table, &["Gamma"]))
            .unwrap();
        assert_eq!(text(&book, "C6"), "30");
        book.undo().unwrap();
        assert_eq!(book.bytes(), before);
        book.redo().unwrap();
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(table),
                column: None,
            },
        )
        .unwrap();
        assert!(!hidden(&book, 1));
        assert!(hidden(&book, 2));
        assert_eq!(text(&book, "C6"), "60");
        assert_eq!(text(&book, "B6"), "40");
        export(&book, "filter-manual.xlsx");
    }
}

#[test]
fn a_new_explicit_range_can_enable_dropdowns_then_filter_and_empty_choices_hide_all_data() {
    let mut package = OpcPackage::open(&fixture(false, true)).unwrap();
    let mut sheet = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    let start = sheet.find("<autoFilter").unwrap();
    let end = start + sheet[start..].find("</autoFilter>").unwrap() + 13;
    sheet.replace_range(start..end, "");
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Enable {
            target: target(false),
            reference: Some("A1:C4".into()),
        },
    )
    .unwrap();
    assert!(book
        .auto_filters("Operations")
        .unwrap()
        .iter()
        .any(|filter| filter.owner == "worksheet" && !filter.has_criteria));
    book.change_auto_filter("Operations", &values(false, &[]))
        .unwrap();
    assert!(hidden(&book, 1));
    assert!(hidden(&book, 2));
    assert!(hidden(&book, 3));
    assert_eq!(text(&book, "C6"), "0");
}

#[test]
fn unsupported_conditions_invalid_columns_and_oversized_ranges_fail_without_mutating_history() {
    let mut package = OpcPackage::open(&fixture(false, false)).unwrap();
    let sheet = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "<filters><filter val=\"Beta\"/><filter val=\"Gamma\"/></filters>",
            "<colorFilter dxfId=\"0\"/>",
        );
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(false),
                column: None
            }
        )
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    let mut book = HeadlessWorkbook::open(&fixture(false, false)).unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Values {
                target: target(false),
                reference: None,
                column: 9,
                values: vec!["x".into()],
                include_blank: false
            }
        )
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    let mut package = OpcPackage::open(&fixture(false, true)).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "<autoFilter ref=\"A1:C4\">",
            "<autoFilter ref=\"A1:C100002\">",
        );
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(false),
                column: None
            }
        )
        .unwrap_err()
        .to_string()
        .contains("limit"));
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
}

#[test]
fn blank_values_and_manual_calculation_keep_filter_state_and_history_consistent() {
    let mut package = OpcPackage::open(&fixture(false, false)).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("<autoFilter ref=\"A1:C4\">", "<autoFilter ref=\"A1:C5\">");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let choices = book
        .auto_filter_options("Operations", &target(false), 0, None)
        .unwrap();
    assert_eq!(
        choices
            .options
            .iter()
            .find(|option| option.blank)
            .unwrap()
            .count,
        1
    );
    book.set_calculation_properties(jade_xlsx_reader::CalculationMode::Manual, false)
        .unwrap();
    let before = book.bytes().to_vec();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Values {
            target: target(false),
            reference: None,
            column: 0,
            values: vec![],
            include_blank: true,
        },
    )
    .unwrap();
    assert!(hidden(&book, 1));
    assert!(hidden(&book, 2));
    assert!(hidden(&book, 3));
    assert!(!hidden(&book, 4));
    assert_eq!(text(&book, "C6"), "50");
    assert!(book.calculation_status().requires_recalculation);
    book.recalculate().unwrap();
    assert_eq!(text(&book, "C6"), "0");
    assert_eq!(text(&book, "B10"), "40");
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    assert_eq!(book.calculation_status().calculation_mode, "manual");
    book.redo().unwrap();
    assert_eq!(text(&book, "C6"), "0");
}

#[test]
fn saved_manual_provenance_survives_reopening_and_direct_calculation_without_reapplying_edits() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false, false)).unwrap();
    book.set_axis_size("Operations", Axis::Rows, 2, 2, 0.0)
        .unwrap();
    assert_eq!(text(&book, "C6"), "50");
    let saved = book.export_bytes().unwrap();
    let raw = jade_xlsx_reader::read_xlsx(&saved).unwrap();
    assert!(!raw.sheets[0].filter_row_state.as_ref().unwrap().invalid);
    let result = jade_xlsx_calc::evaluate(&raw).unwrap();
    assert_eq!(
        result
            .cells
            .iter()
            .find(|cell| cell.reference == jade_xlsx_reader::CellRef::parse("C6").unwrap())
            .unwrap()
            .value,
        jade_xlsx_reader::CellValue::Number(50.0)
    );
    let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(text(&reopened, "C6"), "50");
    reopened
        .set("Operations", "A3", CellInput::Text("Alpha".into()))
        .unwrap();
    assert_eq!(text(&reopened, "C6"), "50");
    let again = HeadlessWorkbook::open_for_editing(&reopened.export_bytes().unwrap()).unwrap();
    assert_eq!(text(&again, "C6"), "50");
    let before = reopened.bytes().to_vec();
    reopened
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(false),
            },
        )
        .unwrap();
    assert_eq!(text(&reopened, "C6"), "30");
    reopened.undo().unwrap();
    assert_eq!(reopened.bytes(), before);
    assert_eq!(text(&reopened, "C6"), "50");
    reopened.redo().unwrap();
    let mut after = HeadlessWorkbook::open_for_editing(&reopened.export_bytes().unwrap()).unwrap();
    after
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(false),
                column: None,
            },
        )
        .unwrap();
    assert!(hidden(&after, 2));
    assert_eq!(text(&after, "C6"), "60");
    assert_eq!(text(&after, "B6"), "40");
    export(&after, "filter-provenance.xlsx");
}

#[test]
fn an_invalid_visibility_snapshot_preserves_full_subtotal_caches_and_blocks_destructive_filter_changes(
) {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false, false)).unwrap();
    book.set_axis_size("Operations", Axis::Rows, 2, 2, 0.0)
        .unwrap();
    let mut package = OpcPackage::open(&book.export_bytes().unwrap()).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("version=\"1\" signature=", "version=\"2\" signature=");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert_eq!(text(&book, "C6"), "50");
    assert_eq!(text(&book, "B6"), "30");
    assert!(book.calculation_status().requires_recalculation);
    assert!(book.auto_filters("Operations").unwrap()[0]
        .issues
        .iter()
        .any(|issue| issue.contains("snapshot")));
    let before = book.bytes().to_vec();
    assert!(book
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(false),
                column: None
            }
        )
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
}

#[test]
fn formatted_value_groups_preserve_their_raw_numeric_filter_values() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false, false)).unwrap();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Clear {
            target: target(false),
            column: None,
        },
    )
    .unwrap();
    for (cell, value) in [("B2", 0.123), ("B3", 0.124), ("B4", 2.0)] {
        book.set("Operations", cell, CellInput::Number(value))
            .unwrap();
    }
    book.set_format(
        "Operations",
        "B2:B4",
        &jade_xlsx_headless_editor::FormatChange {
            number_format: Some("$0.00".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let options = book
        .auto_filter_options("Operations", &target(false), 1, None)
        .unwrap();
    let rounded = options
        .options
        .iter()
        .find(|option| option.label == "$0.12")
        .unwrap();
    assert_eq!(rounded.count, 2);
    assert_eq!(rounded.values, ["0.123", "0.124"]);
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Values {
            target: target(false),
            reference: None,
            column: 1,
            values: rounded.values.clone(),
            include_blank: false,
        },
    )
    .unwrap();
    assert!(!hidden(&book, 1));
    assert!(!hidden(&book, 2));
    assert!(hidden(&book, 3));
    let filter = book.auto_filters("Operations").unwrap()[0]
        .columns
        .iter()
        .find(|column| column.column_id == 1)
        .unwrap()
        .criterion
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(
        filter
            .children
            .iter()
            .map(|node| node.attributes["val"].as_str())
            .collect::<Vec<_>>(),
        ["0.123", "0.124"]
    );
    export(&book, "filter-formatted-values.xlsx");
}

#[test]
fn filter_edits_respect_namespace_aliases_and_preserve_foreign_criteria_and_extensions() {
    let mut package = OpcPackage::open(&fixture(false, false)).unwrap();
    let xml=package.part_str("xl/worksheets/sheet1.xml").unwrap().unwrap()
        .replace("<worksheet xmlns=","<worksheet xmlns:p=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:q=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:z=\"urn:foreign\" xmlns=")
        .replace("<autoFilter ","<p:autoFilter z:ref=\"A100:C200\" ").replace("</autoFilter>","</p:autoFilter>")
        .replace("<filterColumn colId=\"0\">","<q:filterColumn colId=\"0\" z:colId=\"99\" hiddenButton=\"1\" showButton=\"0\"><z:filters><z:filter val='keep'/></z:filters>")
        .replace("</filterColumn>","<q:extLst><z:keep k='v'/></q:extLst></q:filterColumn>");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.change_auto_filter("Operations", &values(false, &["Gamma"]))
        .unwrap();
    let xml = OpcPackage::open(book.bytes())
        .unwrap()
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(xml.contains("z:ref=\"A100:C200\""));
    assert!(xml.contains("z:colId=\"99\""));
    assert!(xml.contains("<z:filters><z:filter val='keep'/></z:filters>"));
    assert!(xml.contains("<q:extLst><z:keep k='v'/></q:extLst>"));
    assert_eq!(text(&book, "C6"), "30");
    let view = &book.auto_filters("Operations").unwrap()[0];
    assert!(view.columns[0].hidden_button);
    assert!(!view.columns[0].show_button);
    assert_eq!(
        view.columns[0].criterion.as_ref().unwrap().children[0].attributes["val"],
        "Gamma"
    );
}

#[test]
fn tampered_or_foreign_snapshot_records_do_not_change_manual_hide_provenance_silently() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false, false)).unwrap();
    book.set_axis_size("Operations", Axis::Rows, 2, 2, 0.0)
        .unwrap();
    let bytes = book.export_bytes().unwrap();
    for replace in ["manual=\"0\" filtered=\"0\"", "manual=\"1\" filtered=\"1\""] {
        let mut package = OpcPackage::open(&bytes).unwrap();
        let xml = package
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .unwrap()
            .replace("manual=\"1\" filtered=\"0\"", replace);
        package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
        let book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
        assert!(
            book.workbook().sheets[0]
                .filter_row_state
                .as_ref()
                .unwrap()
                .invalid
        );
        assert_eq!(text(&book, "C6"), "50");
        assert!(book.calculation_status().requires_recalculation);
    }
}

#[test]
fn disabling_filters_preserves_manual_hides_other_owners_and_exact_history() {
    for table in [false, true] {
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(table, false)).unwrap();
        let before = book.bytes().to_vec();
        let original = OpcPackage::open(&before).unwrap();
        let summary = book
            .change_auto_filter(
                "Operations",
                &AutoFilterChange::Disable {
                    target: target(table),
                },
            )
            .unwrap();
        assert_eq!(summary.changed_rows, 1);
        assert_eq!(summary.active_columns, 0);
        assert!(!hidden(&book, 1));
        assert!(hidden(&book, 7));
        assert_eq!(text(&book, "C6"), "60");
        assert_eq!(text(&book, "B10"), "100");
        assert!(!book
            .auto_filters("Operations")
            .unwrap()
            .iter()
            .any(|filter| filter.target == target(table)));
        let after = book.bytes().to_vec();
        let updated = OpcPackage::open(&after).unwrap();
        assert_eq!(
            original.part("xl/styles.xml"),
            updated.part("xl/styles.xml")
        );
        assert_eq!(
            original.part("xl/worksheets/_rels/sheet1.xml.rels"),
            updated.part("xl/worksheets/_rels/sheet1.xml.rels")
        );
        if table {
            let xml = updated.part_str("xl/tables/table1.xml").unwrap().unwrap();
            assert!(xml.contains("tableColumns"));
            let original_xml = original.part_str("xl/tables/table1.xml").unwrap().unwrap();
            let start = original_xml.find("<autoFilter").unwrap();
            let end = start + original_xml[start..].find("</autoFilter>").unwrap() + 13;
            let mut expected = original_xml;
            expected.replace_range(start..end, "");
            assert_eq!(xml, expected);
        }
        let mut reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
        reopened
            .change_auto_filter(
                "Operations",
                &AutoFilterChange::Disable {
                    target: target(table),
                },
            )
            .unwrap();
        assert_eq!(reopened.bytes(), after);
        assert!(!reopened.can_undo());
        book.undo().unwrap();
        assert_eq!(book.bytes(), before);
        book.redo().unwrap();
        assert_eq!(book.bytes(), after);
        export(
            &book,
            if table {
                "filter-disabled-table.xlsx"
            } else {
                "filter-disabled-worksheet.xlsx"
            },
        );
    }
    // Removing an empty overlapping table filter must not turn another owner's
    // excluded rows into manual hides.
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false, false)).unwrap();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Disable {
            target: target(true),
        },
    )
    .unwrap();
    assert!(hidden(&book, 1));
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Disable {
            target: target(false),
        },
    )
    .unwrap();
    assert!(!hidden(&book, 1));
    assert!(hidden(&book, 7));
}

#[test]
fn empty_filter_enable_does_not_reapply_an_inactive_saved_filter() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false, true)).unwrap();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Disable {
            target: target(true),
        },
    )
    .unwrap();
    let before = (1..=7).map(|row| hidden(&book, row)).collect::<Vec<_>>();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Enable {
            target: target(true),
            reference: None,
        },
    )
    .unwrap();
    assert_eq!(
        (1..=7).map(|row| hidden(&book, row)).collect::<Vec<_>>(),
        before
    );
    assert_eq!(book.workbook().sheets[0].filter_mode, Some(false));
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Disable {
            target: target(false),
        },
    )
    .unwrap();
    assert_eq!(
        (1..=7).map(|row| hidden(&book, row)).collect::<Vec<_>>(),
        before
    );
    assert_eq!(book.workbook().sheets[0].filter_mode, Some(false));
}

#[test]
fn a_whole_unsupported_filter_can_be_removed_without_evaluating_its_predicate() {
    let mut package = OpcPackage::open(&fixture(false, false)).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    let start = xml.find("<filters").unwrap();
    let end = start + xml[start..].find("</filters>").unwrap() + 10;
    let mut xml = xml;
    xml.replace_range(start..end, "<colorFilter dxfId=\"0\"/>");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Disable {
            target: target(false),
        },
    )
    .unwrap();
    assert!(!hidden(&book, 1));
    assert!(hidden(&book, 7));
}

#[test]
fn protection_allows_only_permitted_changes_to_existing_filters() {
    for attributes in [
        "sheet=\"1\"",
        "sheet=\"true\" autoFilter=\"1\"",
        "sheet=\"1\" autoFilter=\"0\"",
        "sheet=\"0\"",
        "autoFilter=\"1\"",
    ] {
        let mut package = OpcPackage::open(&fixture(false, false)).unwrap();
        let xml = package
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .unwrap()
            .replace(
                "<autoFilter",
                &format!("<sheetProtection {attributes}/><autoFilter"),
            );
        package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
        let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
        let protected = attributes.contains("sheet=\"1\"") || attributes.contains("sheet=\"true\"");
        let allow_existing = !protected || attributes.contains("autoFilter=\"0\"");
        let before = book.bytes().to_vec();
        assert_eq!(
            book.change_auto_filter("Operations", &values(false, &["Gamma"]))
                .is_ok(),
            allow_existing
        );
        if !allow_existing {
            assert_eq!(book.bytes(), before);
            assert!(!book.can_undo());
        }
        assert_eq!(
            book.change_auto_filter(
                "Operations",
                &AutoFilterChange::Disable {
                    target: target(false)
                }
            )
            .is_ok(),
            !protected
        );
        assert_eq!(book.current_region("Operations", "A1").is_ok(), !protected);
    }
}

#[test]
fn removing_filters_preserves_foreign_namespace_nodes_and_strict_prefixes() {
    for strict in [false, true] {
        let mut package = OpcPackage::open(&fixture(false, false)).unwrap();
        let mut xml = package
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .unwrap();
        xml = xml.replace(
            "</worksheet>",
            "<foreign:autoFilter xmlns:foreign=\"urn:test\" ref=\"Z1:Z3\"/></worksheet>",
        );
        if strict {
            xml = xml.replace(
                "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                "http://purl.oclc.org/ooxml/spreadsheetml/main",
            );
        }
        package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
        let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Disable {
                target: target(false),
            },
        )
        .unwrap();
        let xml = OpcPackage::open(book.bytes())
            .unwrap()
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .unwrap();
        assert!(xml.contains("<foreign:autoFilter xmlns:foreign=\"urn:test\" ref=\"Z1:Z3\"/>"));
        assert!(book.workbook().sheets[0].auto_filter.is_none());
    }
}

#[test]
fn disabling_one_active_owner_preserves_overlapping_exclusions_from_another() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false, false)).unwrap();
    book.change_auto_filter("Operations", &values(true, &["Alpha"]))
        .unwrap();
    assert!((1..=3).all(|row| hidden(&book, row)));
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Disable {
            target: target(true),
        },
    )
    .unwrap();
    assert!(hidden(&book, 1));
    assert!(!hidden(&book, 2));
    assert!(!hidden(&book, 3));
    assert_eq!(text(&book, "C6"), "50");
    let saved = book.export_bytes().unwrap();
    let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    reopened
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Disable {
                target: target(false),
            },
        )
        .unwrap();
    assert!(!hidden(&reopened, 1));
    assert!(hidden(&reopened, 7));
    assert_eq!(text(&reopened, "C6"), "60");
}
