use jade_xlsx_headless_api::{
    AutoFilterChange, AutoFilterTarget, CustomFilterCondition, FilterComparisonOperator as Op,
};
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
fn custom(table: bool, column: u32, conditions: &[(Op, &str)], and: bool) -> AutoFilterChange {
    AutoFilterChange::Custom {
        target: target(table),
        reference: None,
        column,
        conditions: conditions
            .iter()
            .map(|(operator, value)| CustomFilterCondition {
                operator: *operator,
                value: (*value).into(),
            })
            .collect(),
        and,
    }
}
fn hidden(book: &HeadlessWorkbook, row: u32) -> bool {
    book.workbook().sheets[0]
        .rows
        .iter()
        .find(|item| item.index == row)
        .is_some_and(|row| row.hidden)
}
fn text(book: &HeadlessWorkbook, reference: &str) -> String {
    book.cell("Operations", reference).unwrap().display
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
fn numeric_comparisons_compose_with_other_columns_and_preserve_exact_history() {
    for table in [false, true] {
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(table)).unwrap();
        let before = book.bytes().to_vec();
        let summary = book
            .change_auto_filter(
                "Operations",
                &custom(
                    table,
                    1,
                    &[(Op::GreaterThan, "20"), (Op::LessThanOrEqual, "30")],
                    true,
                ),
            )
            .unwrap();
        assert_eq!(summary.active_columns, 2);
        assert_eq!(summary.changed_rows, 1);
        assert!(hidden(&book, 1));
        assert!(hidden(&book, 2));
        assert!(!hidden(&book, 3));
        assert!(hidden(&book, 7));
        assert_eq!(text(&book, "C6"), "30");
        assert_eq!(text(&book, "B10"), "70");
        let after = book.bytes().to_vec();
        book.undo().unwrap();
        assert_eq!(book.bytes(), before);
        book.redo().unwrap();
        assert_eq!(book.bytes(), after);
        let reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
        assert_eq!(text(&reopened, "C6"), "30");
        let options = reopened
            .auto_filter_options("Operations", &target(table), 1, None)
            .unwrap();
        assert_eq!(options.unsupported, None);
        assert_eq!(
            options
                .options
                .iter()
                .filter(|option| option.selected == Some(true))
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["30"]
        );
        export(
            &book,
            if table {
                "custom-filter-table.xlsx"
            } else {
                "custom-filter-worksheet.xlsx"
            },
        );
    }
}
#[test]
fn every_comparison_and_or_have_expected_numeric_boundaries() {
    let cases = [
        (Op::Equal, "20", vec![true, false, true]),
        (Op::NotEqual, "20", vec![false, true, false]),
        (Op::LessThan, "20", vec![false, true, true]),
        (Op::LessThanOrEqual, "20", vec![false, false, true]),
        (Op::GreaterThan, "20", vec![true, true, false]),
        (Op::GreaterThanOrEqual, "20", vec![true, false, false]),
    ];
    for (operator, value, expected) in cases {
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(false),
                column: None,
            },
        )
        .unwrap();
        book.change_auto_filter("Operations", &custom(false, 1, &[(operator, value)], false))
            .unwrap();
        assert_eq!(
            (1..=3).map(|row| hidden(&book, row)).collect::<Vec<_>>(),
            expected
        );
    }
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Clear {
            target: target(false),
            column: None,
        },
    )
    .unwrap();
    book.change_auto_filter(
        "Operations",
        &custom(
            false,
            1,
            &[(Op::LessThan, "15"), (Op::GreaterThan, "25")],
            false,
        ),
    )
    .unwrap();
    assert_eq!(text(&book, "C6"), "40");
    assert!(hidden(&book, 2));
    export(&book, "custom-filter-or.xlsx");
}
#[test]
fn text_wildcards_escape_literals_and_compare_raw_text_not_format_literals() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    for (pattern, visible) in [("*PH*", vec![1]), ("?eta", vec![2]), ("*a", vec![1, 2, 3])] {
        book.change_auto_filter(
            "Operations",
            &custom(false, 0, &[(Op::Equal, pattern)], false),
        )
        .unwrap();
        assert_eq!(
            (1..=3)
                .filter(|row| !hidden(&book, *row))
                .collect::<Vec<_>>(),
            visible
        );
    }
    book.set(
        "Operations",
        "A2",
        jade_xlsx_headless_api::CellInput::Text("a*b".into()),
    )
    .unwrap();
    book.set(
        "Operations",
        "A3",
        jade_xlsx_headless_api::CellInput::Text("a?b".into()),
    )
    .unwrap();
    book.change_auto_filter(
        "Operations",
        &custom(false, 0, &[(Op::Equal, "a~*b"), (Op::Equal, "a~?b")], false),
    )
    .unwrap();
    assert!(!hidden(&book, 1));
    assert!(!hidden(&book, 2));
    assert!(hidden(&book, 3));
    book.set_format(
        "Operations",
        "A2:A4",
        &jade_xlsx_headless_editor::FormatChange {
            number_format: Some("0;0;0;\"prefix\"@".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(text(&book, "A2").starts_with("prefix"));
    book.change_auto_filter(
        "Operations",
        &custom(false, 0, &[(Op::Equal, "*prefix*")], false),
    )
    .unwrap();
    assert!((1..=3).all(|row| hidden(&book, row)));
    book.change_auto_filter(
        "Operations",
        &custom(false, 0, &[(Op::Equal, "a~*b")], false),
    )
    .unwrap();
    assert!(!hidden(&book, 1));
    assert!(hidden(&book, 2));
    export(&book, "custom-filter-wildcards.xlsx");
}
#[test]
fn invalid_requests_and_expensive_patterns_leave_bytes_and_history_unchanged() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    let before = book.bytes().to_vec();
    for conditions in [vec![], vec![(Op::Equal, "a"); 3], vec![(Op::Equal, ""); 0]] {
        assert!(book
            .change_auto_filter("Operations", &custom(false, 0, &conditions, false))
            .is_err());
        assert_eq!(book.bytes(), before);
        assert!(!book.can_undo());
    }
    let huge = "a".repeat(131069);
    assert!(book
        .change_auto_filter(
            "Operations",
            &custom(false, 0, &[(Op::Equal, &huge)], false)
        )
        .is_err());
    assert_eq!(book.bytes(), before);
    book.set(
        "Operations",
        "A2",
        jade_xlsx_headless_api::CellInput::Text("a".repeat(22000)),
    )
    .unwrap();
    let before = book.bytes().to_vec();
    let pattern = format!("*{}b", "a".repeat(10000));
    let error = book
        .change_auto_filter(
            "Operations",
            &custom(false, 0, &[(Op::Equal, &pattern)], false),
        )
        .unwrap_err();
    assert!(error.to_string().contains("comparison work limit"));
    assert_eq!(book.bytes(), before);
    book.undo().unwrap();
    assert!(!book.can_undo());
}

#[test]
fn blank_and_literal_xstring_conditions_round_trip_and_remain_nondynamic() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    book.set(
        "Operations",
        "A2",
        jade_xlsx_headless_api::CellInput::Text("_x0041_ & <tag>".into()),
    )
    .unwrap();
    book.set(
        "Operations",
        "A3",
        jade_xlsx_headless_api::CellInput::Formula("\"\"".into()),
    )
    .unwrap();
    book.change_auto_filter("Operations", &custom(false, 0, &[(Op::Equal, "")], false))
        .unwrap();
    assert!(hidden(&book, 1));
    assert!(!hidden(&book, 2));
    assert!(hidden(&book, 3));
    book.change_auto_filter(
        "Operations",
        &custom(false, 0, &[(Op::Equal, "_x0041_ & <tag>")], false),
    )
    .unwrap();
    assert!(!hidden(&book, 1));
    assert!(hidden(&book, 2));
    let bytes = book.export_bytes().unwrap();
    let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let criterion = reopened.auto_filters("Operations").unwrap()[0].columns[0]
        .criterion
        .clone()
        .unwrap();
    assert_eq!(criterion.children[0].attributes["val"], "_x0041_ & <tag>");
    reopened
        .set(
            "Operations",
            "A4",
            jade_xlsx_headless_api::CellInput::Text("_x0041_ & <tag>".into()),
        )
        .unwrap();
    assert!(hidden(&reopened, 3));
    reopened
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(false),
            },
        )
        .unwrap();
    assert!(!hidden(&reopened, 3));
    export(&reopened, "custom-filter-strings.xlsx");
}
#[test]
fn manual_calculation_and_protection_are_shared_with_other_filter_changes() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    book.set_calculation_properties(jade_xlsx_reader::CalculationMode::Manual, false)
        .unwrap();
    book.change_auto_filter(
        "Operations",
        &custom(false, 1, &[(Op::GreaterThanOrEqual, "30")], false),
    )
    .unwrap();
    assert_eq!(text(&book, "C6"), "50");
    book.recalculate().unwrap();
    assert_eq!(text(&book, "C6"), "30");
    let mut package = OpcPackage::open(&fixture(false)).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("<autoFilter", "<sheetProtection sheet=\"1\"/><autoFilter");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .change_auto_filter("Operations", &custom(false, 1, &[(Op::Equal, "20")], false))
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
}
#[test]
fn typed_json_defaults_and_operator_validation_are_stable() {
    let change:AutoFilterChange=serde_json::from_str(r#"{"operation":"custom","target":{"kind":"worksheet"},"column":1,"conditions":[{"operator":"greaterThan","value":"20"}]}"#).unwrap();
    assert_eq!(change, custom(false, 1, &[(Op::GreaterThan, "20")], false));
    assert!(serde_json::from_str::<AutoFilterChange>(r#"{"operation":"custom","target":{"kind":"worksheet"},"column":1,"conditions":[{"operator":"contains","value":"20"}]}"#).is_err());
}

#[test]
fn tilde_only_escapes_wildcards_and_other_tildes() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(false)).unwrap();
    for (reference, value) in [("A2", "~a"), ("A3", "a"), ("A4", "~")] {
        book.set(
            "Operations",
            reference,
            jade_xlsx_headless_api::CellInput::Text(value.into()),
        )
        .unwrap();
    }
    for (pattern, visible) in [("~a", vec![1]), ("~", vec![3]), ("~~", vec![3])] {
        book.change_auto_filter(
            "Operations",
            &custom(false, 0, &[(Op::Equal, pattern)], false),
        )
        .unwrap();
        assert_eq!(
            (1..=3)
                .filter(|row| !hidden(&book, *row))
                .collect::<Vec<_>>(),
            visible
        );
    }
}
