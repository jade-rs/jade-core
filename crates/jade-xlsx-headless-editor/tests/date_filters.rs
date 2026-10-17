use jade_xlsx_headless_api::{
    AutoFilterChange, AutoFilterTarget, CellInput, DateFilterGroup, DateFilterGrouping,
};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CalculationMode, OpcPackage};

fn fixture(epoch: u32, table: bool, criteria: Option<&str>) -> Vec<u8> {
    let bytes: &[u8] = if epoch == 1904 {
        include_bytes!("../../../public/fixtures/date-group-1904-workbook.xlsx")
    } else {
        include_bytes!("../../../public/fixtures/date-group-1900-workbook.xlsx")
    };
    let mut package = OpcPackage::open(bytes).unwrap();
    if let Some(criteria) = criteria {
        let part = if table {
            "xl/tables/table1.xml"
        } else {
            "xl/worksheets/sheet1.xml"
        };
        let xml = package.part_str(part).unwrap().unwrap().replace("<autoFilter ref=\"A1:C4\"/>", &format!("<autoFilter ref=\"A1:C4\"><filterColumn colId=\"0\" showButton=\"1\"><filters>{criteria}</filters></filterColumn></autoFilter>"));
        package.replace(part, xml.into_bytes());
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
fn group(grouping: DateFilterGrouping) -> DateFilterGroup {
    DateFilterGroup {
        grouping,
        year: 2024,
        month: Some(2),
        day: Some(1),
        hour: Some(12),
        minute: Some(34),
        second: Some(56),
    }
}
fn apply(book: &mut HeadlessWorkbook, table: bool, groups: Vec<DateFilterGroup>) {
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Dates {
            target: target(table),
            reference: None,
            column: 0,
            groups,
            include_blank: false,
        },
    )
    .unwrap();
}
fn hidden(book: &HeadlessWorkbook) -> Vec<bool> {
    (1..=3)
        .map(|index| {
            book.workbook().sheets[0]
                .rows
                .iter()
                .any(|row| row.index == index && row.hidden)
        })
        .collect()
}
fn text(book: &HeadlessWorkbook, cell: &str) -> String {
    book.cell("Operations", cell).unwrap().display
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
fn every_granularity_supports_both_epochs_and_owners_with_exact_history() {
    for epoch in [1900, 1904] {
        for table in [false, true] {
            for grouping in [
                DateFilterGrouping::Year,
                DateFilterGrouping::Month,
                DateFilterGrouping::Day,
                DateFilterGrouping::Hour,
                DateFilterGrouping::Minute,
                DateFilterGrouping::Second,
            ] {
                let mut book =
                    HeadlessWorkbook::open_for_editing(&fixture(epoch, table, None)).unwrap();
                let before = book.bytes().to_vec();
                apply(&mut book, table, vec![group(grouping)]);
                let all = grouping == DateFilterGrouping::Year;
                assert_eq!(
                    hidden(&book),
                    if all {
                        vec![false, false, false]
                    } else {
                        vec![true, false, true]
                    }
                );
                assert_eq!(text(&book, "C6"), if all { "60" } else { "20" });
                let options = book
                    .auto_filter_options("Operations", &target(table), 0, None)
                    .unwrap();
                assert!(options.unsupported.is_none());
                assert_eq!(
                    options
                        .options
                        .iter()
                        .filter(|item| item.selected == Some(true))
                        .count(),
                    if all { 3 } else { 1 }
                );
                let after = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
                let reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
                assert_eq!(hidden(&reopened), hidden(&book));
                if grouping == DateFilterGrouping::Second {
                    export(
                        &book,
                        &format!(
                            "filter-dates-{epoch}-{}.xlsx",
                            if table { "table" } else { "worksheet" }
                        ),
                    );
                }
            }
        }
    }
}

#[test]
fn saved_groups_union_with_values_and_blanks_without_rewriting_criteria() {
    for table in [false, true] {
        let criterion = "<dateGroupItem year=\"2024\" month=\"1\" dateTimeGrouping=\"month\"/><dateGroupItem year=\"2024\" month=\"2\" dateTimeGrouping=\"month\"/><filter val=\"45352\"/>";
        let mut book =
            HeadlessWorkbook::open_for_editing(&fixture(1900, table, Some(criterion))).unwrap();
        let before = book.auto_filters("Operations").unwrap();
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(table),
            },
        )
        .unwrap();
        assert_eq!(hidden(&book), vec![false, false, false]);
        assert_eq!(
            book.auto_filters("Operations").unwrap()[if table { 1 } else { 0 }].columns,
            before[if table { 1 } else { 0 }].columns
        );
        book.set("Operations", "A4", CellInput::Blank).unwrap();
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(table),
            },
        )
        .unwrap();
        assert_eq!(hidden(&book), vec![false, false, true]);
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Dates {
                target: target(table),
                reference: None,
                column: 0,
                groups: vec![group(DateFilterGrouping::Month)],
                include_blank: true,
            },
        )
        .unwrap();
        assert_eq!(hidden(&book), vec![true, false, false]);
        assert_eq!(text(&book, "C6"), "50");
    }
}

#[test]
fn reapply_preserves_manual_hides_and_manual_calculation_through_save() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(1900, false, None)).unwrap();
    book.set_axis_size("Operations", Axis::Rows, 2, 2, 0.0)
        .unwrap();
    book.set_calculation_properties(CalculationMode::Manual, false)
        .unwrap();
    apply(&mut book, false, vec![group(DateFilterGrouping::Month)]);
    assert_eq!(hidden(&book), vec![true, true, true]);
    // Manual caches remain stale until F9.
    book.recalculate().unwrap();
    assert_eq!(text(&book, "B6"), "0");
    assert_eq!(text(&book, "C6"), "20");
    book.set("Operations", "A2", CellInput::Number(45323.0))
        .unwrap();
    assert!(hidden(&book)[0]);
    book.change_auto_filter(
        "Operations",
        &AutoFilterChange::Reapply {
            target: target(false),
        },
    )
    .unwrap();
    assert_eq!(hidden(&book), vec![false, true, true]);
    let saved = book.export_bytes().unwrap();
    let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    reopened
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Clear {
                target: target(false),
                column: None,
            },
        )
        .unwrap();
    assert_eq!(hidden(&reopened), vec![false, true, false]);
    reopened.recalculate().unwrap();
    assert_eq!(text(&reopened, "B6"), "40");
    assert_eq!(text(&reopened, "C6"), "60");
}

#[test]
fn malformed_fields_and_unsupported_calendars_fail_without_history() {
    let invalid = [
        "<dateGroupItem year=\"2024\" dateTimeGrouping=\"month\"/>",
        "<dateGroupItem year=\"2023\" month=\"2\" day=\"29\" dateTimeGrouping=\"day\"/>",
        "<dateGroupItem year=\"2024\" month=\"2\" day=\"1\" hour=\"24\" dateTimeGrouping=\"hour\"/>",
        "<dateGroupItem year=\"2024\" month=\"2\" day=\"1\" hour=\"12\" minute=\"60\" dateTimeGrouping=\"minute\"/>",
        "<dateGroupItem year=\"2024\" month=\"2\" day=\"1\" hour=\"12\" minute=\"34\" second=\"60\" dateTimeGrouping=\"second\"/>",
        "<dateGroupItem year=\"2024\" dateTimeGrouping=\"week\"/>",
    ];
    for criterion in invalid {
        let mut book =
            HeadlessWorkbook::open_for_editing(&fixture(1900, false, Some(criterion))).unwrap();
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
        assert!(!book.can_undo());
    }
    let bytes = fixture(
        1900,
        false,
        Some("<dateGroupItem year=\"2024\" dateTimeGrouping=\"year\"/>"),
    );
    let mut package = OpcPackage::open(&bytes).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("<filters>", "<filters calendarType=\"japan\">");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert!(book
        .auto_filter_options("Operations", &target(false), 0, None)
        .unwrap()
        .unsupported
        .is_some());
    assert!(book
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Reapply {
                target: target(false)
            }
        )
        .is_err());
    assert!(!book.can_undo());
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(1900, false, None)).unwrap();
    let before = book.bytes().to_vec();
    let mut invalid = group(DateFilterGrouping::Day);
    invalid.day = Some(30);
    assert!(book
        .change_auto_filter(
            "Operations",
            &AutoFilterChange::Dates {
                target: target(false),
                reference: None,
                column: 0,
                groups: vec![invalid],
                include_blank: false
            }
        )
        .is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
}

#[test]
fn finer_groups_distinguish_neighbours_and_intersect_other_columns() {
    for epoch in [1900, 1904] {
        let offset = if epoch == 1904 { 1462.0 } else { 0.0 };
        let day = 45323.0 - offset;
        for (grouping, delta) in [
            (DateFilterGrouping::Day, 1.0),
            (DateFilterGrouping::Hour, 1.0 / 24.0),
            (DateFilterGrouping::Minute, 1.0 / 1440.0),
            (DateFilterGrouping::Second, 1.0 / 86400.0),
        ] {
            let mut book =
                HeadlessWorkbook::open_for_editing(&fixture(epoch, false, None)).unwrap();
            let selected = day + 45296.0 / 86400.0;
            for (cell, value) in [
                ("A2", selected - delta),
                ("A3", selected),
                ("A4", selected + delta),
            ] {
                book.set("Operations", cell, CellInput::Number(value))
                    .unwrap();
            }
            apply(&mut book, false, vec![group(grouping)]);
            assert_eq!(
                hidden(&book),
                vec![true, false, true],
                "{grouping:?} {epoch}"
            );
        }
        let mut book = HeadlessWorkbook::open_for_editing(&fixture(epoch, false, None)).unwrap();
        apply(&mut book, false, vec![group(DateFilterGrouping::Year)]);
        book.change_auto_filter(
            "Operations",
            &AutoFilterChange::Values {
                target: target(false),
                reference: None,
                column: 1,
                values: vec!["20".into()],
                include_blank: false,
            },
        )
        .unwrap();
        let options = book
            .auto_filter_options("Operations", &target(false), 0, None)
            .unwrap();
        assert_eq!(options.options.len(), 1);
        assert_eq!(options.options[0].count, 1);
        apply(
            &mut book,
            false,
            vec![DateFilterGroup {
                month: Some(1),
                ..group(DateFilterGrouping::Month)
            }],
        );
        assert_eq!(hidden(&book), vec![true, true, true]);
    }
}

#[test]
fn phantom_leap_day_and_empty_date_groups_have_explicit_semantics() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(1900, false, None)).unwrap();
    for (cell, value) in [("A2", 59.0), ("A3", 60.0), ("A4", 61.0)] {
        book.set("Operations", cell, CellInput::Number(value))
            .unwrap();
    }
    apply(
        &mut book,
        false,
        vec![DateFilterGroup {
            year: 1900,
            month: Some(2),
            day: Some(29),
            ..group(DateFilterGrouping::Day)
        }],
    );
    assert_eq!(hidden(&book), vec![true, false, true]);
    apply(&mut book, false, vec![]);
    assert_eq!(hidden(&book), vec![true, true, true]);
    let reopened = HeadlessWorkbook::open_for_editing(&book.export_bytes().unwrap()).unwrap();
    assert_eq!(hidden(&reopened), vec![true, true, true]);
}
