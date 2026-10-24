use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::OpcPackage;
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-cache-compat.xlsx");
fn book(bytes: &[u8]) -> HeadlessWorkbook {
    HeadlessWorkbook::open_for_editing(bytes).unwrap()
}
fn formula(b: &mut HeadlessWorkbook, at: &str, f: &str) {
    b.set("Report", at, CellInput::Formula(f.into())).unwrap();
}
fn query(b: &mut HeadlessWorkbook, target: &str, f: &str, expected: f64) {
    formula(b, target, f);
    assert_eq!(
        b.cell("Report", target).unwrap().number,
        Some(expected),
        "{f}"
    );
}
fn edit_part(bytes: &[u8], part: &str, edit: impl FnOnce(String) -> String) -> Vec<u8> {
    let mut p = OpcPackage::open(bytes).unwrap();
    let xml = p.required_part_str(part).unwrap();
    p.replace(part, edit(xml).into_bytes());
    p.write().unwrap()
}
fn table(bytes: &[u8], edit: impl FnOnce(String) -> String) -> Vec<u8> {
    edit_part(bytes, "xl/pivotTables/report.xml", edit)
}
#[test]
fn refresh_is_one_history_edit_and_getpivotdata_observes_new_values_after_reopen() {
    let mut b = book(FIXTURE);
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    query(&mut b, "H10", r#"GETPIVOTDATA("Amount",E3)"#, 42.);
    let before = b.bytes().to_vec();
    let result = b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(result.source_rows, 4);
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(132.));
    assert_eq!(b.cell("Report", "F4").unwrap().display, "105.00");
    let saved = b.bytes().to_vec();
    assert_eq!(
        book(&saved).cell("Report", "H10").unwrap().number,
        Some(132.)
    );
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert_eq!(b.cell("Report", "H10").unwrap().number, Some(42.));
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), saved);
}
#[test]
fn source_formula_results_are_evaluated_before_cache_and_report_rebuild() {
    let mut b = book(FIXTURE);
    b.set("Data", "B2", CellInput::Formula("50+50".into()))
        .unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(b.cell("Report", "F4").unwrap().number, Some(105.));
    assert_eq!(
        b.workbook().pivots.caches[0].record_value(0, 1),
        Some(&jade_xlsx_reader::pivot::PivotScalar::Number(100.))
    );
}
#[test]
fn unverified_source_and_occupied_growth_fail_without_mutating_cells_cache_or_history() {
    for unknown_formula in [false, true] {
        let mut b = book(FIXTURE);
        if unknown_formula {
            b.set(
                "Data",
                "B2",
                CellInput::Formula("UNKNOWN_SOURCE_FUNCTION()".into()),
            )
            .unwrap();
        } else {
            b.set("Data", "A3", CellInput::Text("North".into()))
                .unwrap();
            b.set("Report", "F7", CellInput::Text("Keep".into()))
                .unwrap();
        }
        let before = b.bytes().to_vec();
        let checkpoint = b.edit_checkpoint();
        assert!(b.refresh_pivot("Report", "RegionalSales").is_err());
        assert_eq!(b.bytes(), before);
        assert!(b.restore_edit_checkpoint(checkpoint));
        assert_eq!(b.bytes(), before);
    }
}
#[test]
fn removed_members_clear_old_footprint_while_cache_indices_remain_stable() {
    let mut b = book(FIXTURE);
    b.set("Data", "A3", CellInput::Text("East".into())).unwrap();
    b.set("Data", "A5", CellInput::Text("East".into())).unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(b.cell("Report", "F4").unwrap().number, Some(42.));
    assert_eq!(b.cell("Report", "F5").unwrap().number, Some(42.));
    assert_eq!(
        b.cell("Report", "F6").unwrap().kind,
        jade_xlsx_headless_api::CellKind::Empty
    );
    assert!(b.workbook().pivots.caches[0].fields[0].shared_items[2].unused);
}
#[test]
fn manual_hidden_members_and_selected_page_filters_restrict_visible_totals() {
    for page in [false, true] {
        let bytes = table(FIXTURE, |s| {
            if page {
                s.replace(
                    "<pageField fld=\"2\"/>",
                    "<pageField fld=\"2\" item=\"0\"/>",
                )
            } else {
                s.replace("<item x=\"2\"/>", "<item x=\"2\" h=\"1\"/>")
            }
        });
        let mut b = book(&bytes);
        b.refresh_pivot("Report", "RegionalSales").unwrap();
        query(&mut b, "H10", r#"GETPIVOTDATA("Amount",E3)"#, 15.);
        assert_eq!(
            b.cell("Report", "F6").unwrap().kind,
            jade_xlsx_headless_api::CellKind::Empty
        );
    }
}
#[test]
fn all_standard_aggregations_rebuild_their_actual_cells_not_a_sum_placeholder() {
    let kinds = [
        ("sum", 42.),
        ("count", 4.),
        ("average", 10.5),
        ("min", 5.),
        ("max", 20.),
        ("product", 7000.),
        ("countNums", 4.),
        ("var", 133. / 3.),
        ("varp", 133. / 4.),
        ("stdDev", (133.0_f64 / 3.).sqrt()),
        ("stdDevp", (133.0_f64 / 4.).sqrt()),
    ];
    let bytes = table(FIXTURE, |s| {
        let start = s.find("<dataFields ").unwrap();
        let end = s.find("</dataFields>").unwrap() + "</dataFields>".len();
        let body = kinds
            .iter()
            .map(|(kind, _)| {
                format!("<dataField name=\"{kind} Amount\" fld=\"1\" subtotal=\"{kind}\"/>")
            })
            .collect::<String>();
        format!(
            "{}<dataFields count=\"{}\">{body}</dataFields>{}",
            &s[..start],
            kinds.len(),
            &s[end..]
        )
    });
    let mut b = book(&bytes);
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    for (i, (kind, expected)) in kinds.iter().enumerate() {
        let target = format!("W{}", i + 1);
        let actual_query = format!(r#"GETPIVOTDATA("{kind} Amount",E3)"#);
        formula(&mut b, &target, &actual_query);
        let actual = b.cell("Report", &target).unwrap().number.unwrap();
        assert!(
            (actual - expected).abs() < 1e-9,
            "{kind}: {actual} != {expected}"
        );
    }
}
#[test]
fn multiple_row_fields_keep_subtotals_dates_blank_items_and_typed_date_labels() {
    let bytes = table(FIXTURE, |s| {
        s.replace(
            "<rowFields count=\"1\"><field x=\"0\"/></rowFields>",
            "<rowFields count=\"2\"><field x=\"0\"/><field x=\"2\"/></rowFields>",
        )
        .replace("firstDataCol=\"1\"", "firstDataCol=\"2\"")
        .replace("E3:F6", "E3:G6")
        .replace(
            "<pageFields count=\"1\"><pageField fld=\"2\"/></pageFields>",
            "<pageFields count=\"0\"/>",
        )
    });
    let mut b = book(&bytes);
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    query(&mut b, "K10", r#"GETPIVOTDATA("Amount",E3)"#, 42.);
    query(
        &mut b,
        "L10",
        r#"GETPIVOTDATA("Amount",E3,"Region","East")"#,
        15.,
    );
    query(
        &mut b,
        "M10",
        r#"GETPIVOTDATA("Amount",E3,"Region","East","When",DATE(2026,10,24)+TIME(12,30,0))"#,
        15.,
    );
    query(
        &mut b,
        "N10",
        r#"GETPIVOTDATA("Amount",E3,"Region","East","When",F5)"#,
        15.,
    );
    assert!(b.cell("Report", "F5").unwrap().number.is_some());
    assert!(b.cell("Report", "F5").unwrap().display.contains("2026"));
}
#[test]
fn multiple_column_fields_and_measures_keep_header_levels_and_grand_totals() {
    let bytes = table(FIXTURE, |s| {
        s.replace(
            "<colFields count=\"1\"><field x=\"-2\"/></colFields>",
            "<colFields count=\"2\"><field x=\"2\"/><field x=\"-2\"/></colFields>",
        )
        .replace(
            "<pageFields count=\"1\"><pageField fld=\"2\"/></pageFields>",
            "<pageFields count=\"0\"/>",
        )
        .replace("<dataFields count=\"1\">", "<dataFields count=\"2\">")
        .replace(
            "</dataFields>",
            "<dataField name=\"Count Amount\" fld=\"1\" subtotal=\"count\"/></dataFields>",
        )
    });
    let mut b = book(&bytes);
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    query(&mut b, "N10", r#"GETPIVOTDATA("Total Amount",E3)"#, 42.);
    query(&mut b, "O10", r#"GETPIVOTDATA("Count Amount",E3)"#, 4.);
    query(
        &mut b,
        "P10",
        r#"GETPIVOTDATA("Total Amount",E3,"When",F3)"#,
        15.,
    );
    query(
        &mut b,
        "Q10",
        r#"GETPIVOTDATA("Count Amount",E3,"When",F3,"Region","East")"#,
        2.,
    );
}
#[test]
fn named_range_and_table_sources_are_resolved_instead_of_using_old_record_counts() {
    for table_source in [false, true] {
        let mut b = book(FIXTURE);
        if table_source {
            b.create_table(
                "Data",
                jade_xlsx_reader::CellRange::parse("A1:C5").unwrap(),
                &jade_xlsx_headless_editor::TableOptions {
                    name: Some("PivotSource".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        } else {
            b.set_defined_name(&jade_xlsx_reader::DefinedName {
                name: "PivotSource".into(),
                formula: "'Data'!$A$1:$C$5".into(),
                local_sheet_index: None,
                hidden: false,
            })
            .unwrap();
        }
        let bytes = edit_part(b.bytes(), "xl/pivotCache/cache.xml", |s| {
            s.replace(
                "<worksheetSource sheet=\"Data\" ref=\"A1:C5\"/>",
                "<worksheetSource name=\"PivotSource\"/>",
            )
        });
        let mut b = book(&bytes);
        b.set("Data", "B2", CellInput::Number(100.)).unwrap();
        b.refresh_pivot("Report", "RegionalSales").unwrap();
        query(&mut b, "H10", r#"GETPIVOTDATA("Amount",E3)"#, 132.);
    }
}

#[test]
fn compact_groups_keep_indentation_and_header_only_rows_are_not_queryable_subtotals() {
    for mode in ["top", "bottom", "none"] {
        let bytes = table(FIXTURE, |s| {
            s.replace(
                "<rowFields count=\"1\"><field x=\"0\"/></rowFields>",
                "<rowFields count=\"2\"><field x=\"0\"/><field x=\"2\"/></rowFields>",
            )
            .replace(
                "<pageFields count=\"1\"><pageField fld=\"2\"/></pageFields>",
                "<pageFields count=\"0\"/>",
            )
            .replace(
                "<pivotField axis=\"axisRow\" name=\"Territory\">",
                &format!(
                    "<pivotField axis=\"axisRow\" name=\"Territory\" {}>",
                    match mode {
                        "none" => "defaultSubtotal=\"0\"",
                        "bottom" => "subtotalTop=\"0\"",
                        _ => "",
                    }
                ),
            )
        });
        let mut b = book(&bytes);
        b.refresh_pivot("Report", "RegionalSales").unwrap();
        assert_eq!(b.workbook().pivots.tables[0].location.first_data_column, 1);
        query(&mut b, "K15", r#"GETPIVOTDATA("Amount",E3)"#, 42.);
        if mode == "none" {
            formula(
                &mut b,
                "L15",
                r#"GETPIVOTDATA("Amount",E3,"Region","East")"#,
            );
            assert_eq!(
                b.cell("Report", "L15").unwrap().text.as_deref(),
                Some("#REF!")
            );
        } else {
            query(
                &mut b,
                "L15",
                r#"GETPIVOTDATA("Amount",E3,"Region","East")"#,
                15.,
            );
        }
        query(
            &mut b,
            "M15",
            r#"GETPIVOTDATA("Amount",E3,"Region","East","When",E5)"#,
            15.,
        );
        let at = jade_xlsx_reader::CellRef::parse("E5").unwrap();
        let s = &b.workbook().sheets[1];
        let cell = s
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .find(|c| c.reference == at)
            .unwrap();
        let style = s.effective_style_index(at, cell.style_index).unwrap_or(0);
        assert_eq!(
            b.workbook().styles.cell_formats[style as usize]
                .alignment
                .indent,
            1
        );
    }
}
#[test]
fn member_label_sorting_keeps_physical_cells_and_query_coordinates_consistent() {
    for (sort, labels) in [
        ("ascending", ["East", "North", "West"]),
        ("descending", ["West", "North", "East"]),
    ] {
        let bytes = table(FIXTURE, |s| {
            s.replace(
                "<pivotField axis=\"axisRow\"",
                &format!("<pivotField sortType=\"{sort}\" axis=\"axisRow\""),
            )
        });
        let mut b = book(&bytes);
        b.set("Data", "A3", CellInput::Text("North".into()))
            .unwrap();
        b.refresh_pivot("Report", "RegionalSales").unwrap();
        for (i, label) in labels.iter().enumerate() {
            assert_eq!(
                b.cell("Report", &format!("E{}", i + 4)).unwrap().display,
                *label
            );
        }
        query(
            &mut b,
            "K10",
            r#"GETPIVOTDATA("Amount",E3,"Region","East")"#,
            15.,
        );
        query(
            &mut b,
            "L10",
            r#"GETPIVOTDATA("Amount",E3,"Region","North")"#,
            20.,
        );
    }
}
fn shared_fixture() -> Vec<u8> {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/pivotTables/secondary.xml";
    let definition = p
        .required_part_str("xl/pivotTables/report.xml")
        .unwrap()
        .replace("RegionalSales", "Secondary")
        .replace("E3:F6", "J3:K6");
    p.replace(part, definition.into_bytes());
    let sheet = "xl/worksheets/sheet2.xml";
    let xml = p
        .required_part_str(sheet)
        .unwrap()
        .replace(
            "<pivotTableParts count=\"1\">",
            "<pivotTableParts count=\"2\">",
        )
        .replace(
            "</pivotTableParts>",
            "<pivotTablePart r:id=\"secondary\"/></pivotTableParts>",
        );
    let edits = [
        ("J3", CellInput::Text("Territory".into())),
        ("K3", CellInput::Text("Total Amount".into())),
        ("J4", CellInput::Text("East".into())),
        ("K4", CellInput::Number(15.)),
        ("J5", CellInput::Text("West".into())),
        ("K5", CellInput::Number(27.)),
        ("J6", CellInput::Text("Grand Total".into())),
        ("K6", CellInput::Number(42.)),
    ]
    .into_iter()
    .map(|(r, v)| jade_xlsx_writer::CellEdit {
        reference: jade_xlsx_reader::CellRef::parse(r).unwrap(),
        value: match v {
            CellInput::Number(n) => jade_xlsx_writer::NewValue::Number(n),
            CellInput::Text(s) => jade_xlsx_writer::NewValue::Text(s),
            _ => unreachable!(),
        },
        style_index: None,
    })
    .collect::<Vec<_>>();
    p.replace(
        sheet,
        jade_xlsx_writer::patch_sheet(&xml, &edits)
            .unwrap()
            .into_bytes(),
    );
    let rel = "xl/worksheets/_rels/sheet2.xml.rels";
    let xml=p.required_part_str(rel).unwrap().replace("</Relationships>","<Relationship Id=\"secondary\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable\" Target=\"../pivotTables/secondary.xml\"/></Relationships>");
    p.replace(rel, xml.into_bytes());
    let xml=p.required_part_str("[Content_Types].xml").unwrap().replace("</Types>",&format!("<Override PartName=\"/{part}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml\"/></Types>"));
    p.replace("[Content_Types].xml", xml.into_bytes());
    p.write().unwrap()
}
#[test]
fn all_reports_sharing_a_cache_refresh_and_undo_as_one_edit() {
    let mut b = book(&shared_fixture());
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    query(&mut b, "N10", r#"GETPIVOTDATA("Amount",E3)"#, 42.);
    query(&mut b, "O10", r#"GETPIVOTDATA("Amount",J3)"#, 42.);
    let before = b.bytes().to_vec();
    let summary = b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(summary.tables, ["RegionalSales", "Secondary"]);
    assert_eq!(b.cell("Report", "N10").unwrap().number, Some(132.));
    assert_eq!(b.cell("Report", "O10").unwrap().number, Some(132.));
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
}
#[test]
fn a_collision_in_one_shared_report_refuses_the_entire_refresh() {
    let mut b = book(&shared_fixture());
    b.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    b.set("Report", "K7", CellInput::Text("Keep".into()))
        .unwrap();
    let before = b.bytes().to_vec();
    assert!(b.refresh_pivot("Report", "RegionalSales").is_err());
    assert_eq!(b.bytes(), before);
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(42.));
}
#[test]
fn refresh_recreates_missing_records_with_owned_internal_relationships() {
    for undeclared in [false, true] {
        let mut p = OpcPackage::open(FIXTURE).unwrap();
        p.remove("xl/pivotCache/records.xml");
        if undeclared {
            let part = "xl/pivotCache/cache.xml";
            let xml = p
                .required_part_str(part)
                .unwrap()
                .replace(" r:id=\"records\"", "");
            p.replace(part, xml.into_bytes());
            p.remove("xl/pivotCache/_rels/cache.xml.rels");
        }
        let mut b = book(&p.write().unwrap());
        b.refresh_pivot("Report", "RegionalSales").unwrap();
        let c = &b.workbook().pivots.caches[0];
        assert_eq!(c.records.as_ref().unwrap().rows.len(), 4);
        assert!(c.records_part.is_some());
        assert!(b.workbook().pivots.issues.is_empty());
    }
}
#[test]
fn strict_namespaces_and_qualified_aliases_keep_refresh_records_and_saved_queries_working() {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let names = p.part_names().to_vec();
    for name in names {
        if !name.ends_with(".xml") && !name.ends_with(".rels") {
            continue;
        }
        let Some(xml) = p.part_str(&name).unwrap() else {
            continue;
        };
        let xml = xml
            .replace(
                "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                "http://purl.oclc.org/ooxml/spreadsheetml/main",
            )
            .replace(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                "http://purl.oclc.org/ooxml/officeDocument/relationships",
            )
            .replace("xmlns:r=", "xmlns:link=")
            .replace("r:id=", "link:id=");
        p.replace(&name, xml.into_bytes());
    }
    let mut b = book(&p.write().unwrap());
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    query(&mut b, "N10", r#"GETPIVOTDATA("Amount",E3)"#, 132.);
}

#[test]
fn values_on_rows_preserve_measure_labels_coordinates_and_query_results() {
    let bytes = table(FIXTURE, |s| {
        s.replace(
            "<rowFields count=\"1\"><field x=\"0\"/></rowFields>",
            "<rowFields count=\"2\"><field x=\"0\"/><field x=\"-2\"/></rowFields>",
        )
        .replace(
            "<colFields count=\"1\"><field x=\"-2\"/></colFields>",
            "<colFields count=\"0\"/>",
        )
        .replace("E3:F6", "E3:G6")
        .replace("firstDataCol=\"1\"", "firstDataCol=\"2\"")
        .replace("<dataFields count=\"1\">", "<dataFields count=\"2\">")
        .replace(
            "</dataFields>",
            "<dataField name=\"Count Amount\" fld=\"1\" subtotal=\"count\"/></dataFields>",
        )
    });
    let mut b = book(&bytes);
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(b.cell("Report", "G3").unwrap().display, "Values");
    assert_eq!(b.cell("Report", "F4").unwrap().display, "Total Amount");
    assert_eq!(b.cell("Report", "F5").unwrap().display, "Count Amount");
    query(
        &mut b,
        "K15",
        r#"GETPIVOTDATA("Total Amount",E3,"Region","East")"#,
        15.,
    );
    query(
        &mut b,
        "L15",
        r#"GETPIVOTDATA("Count Amount",E3,"Region","East")"#,
        2.,
    );
    query(&mut b, "M15", r#"GETPIVOTDATA("Total Amount",E3)"#, 42.);
    query(&mut b, "N15", r#"GETPIVOTDATA("Count Amount",E3)"#, 4.);
}
#[test]
fn numeric_text_boolean_blank_and_error_inputs_keep_aggregation_type_semantics() {
    let bytes = table(FIXTURE, |s| {
        s.replace("<dataFields count=\"1\">","<dataFields count=\"3\">").replace("</dataFields>","<dataField name=\"Count Amount\" fld=\"1\" subtotal=\"count\"/><dataField name=\"Numeric Amount\" fld=\"1\" subtotal=\"countNums\"/></dataFields>")
    });
    let mut p = OpcPackage::open(&bytes).unwrap();
    let part = "xl/worksheets/sheet1.xml";
    let xml = p.required_part_str(part).unwrap();
    let changes = [
        ("B3", jade_xlsx_writer::NewValue::Text("text".into())),
        ("B4", jade_xlsx_writer::NewValue::Bool(true)),
        ("B5", jade_xlsx_writer::NewValue::Error("#N/A".into())),
    ]
    .into_iter()
    .map(|(s, v)| jade_xlsx_writer::CellEdit {
        reference: jade_xlsx_reader::CellRef::parse(s).unwrap(),
        value: v,
        style_index: None,
    })
    .collect::<Vec<_>>();
    p.replace(
        part,
        jade_xlsx_writer::patch_sheet(&xml, &changes)
            .unwrap()
            .into_bytes(),
    );
    let mut b = book(&p.write().unwrap());
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    query(&mut b, "L15", r#"GETPIVOTDATA("Count Amount",E3)"#, 4.);
    query(&mut b, "M15", r#"GETPIVOTDATA("Numeric Amount",E3)"#, 1.);
    query(
        &mut b,
        "N15",
        r#"GETPIVOTDATA("Total Amount",E3,"Region","East")"#,
        10.,
    );
    formula(&mut b, "O15", r#"GETPIVOTDATA("Total Amount",E3)"#);
    assert_eq!(
        b.cell("Report", "O15").unwrap().text.as_deref(),
        Some("#N/A")
    );
    b.set("Data", "B4", CellInput::Blank).unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(b.cell("Report", "L15").unwrap().number, Some(3.));
}
#[test]
fn refresh_keeps_the_same_iso_cache_member_and_date_criteria_in_the_1904_epoch() {
    let bytes = edit_part(FIXTURE, "xl/workbook.xml", |s| {
        s.replace("<sheets>", "<workbookPr date1904=\"1\"/><sheets>")
    });
    let bytes = table(&bytes, |s| {
        s.replace(
            "<rowFields count=\"1\"><field x=\"0\"/></rowFields>",
            "<rowFields count=\"1\"><field x=\"2\"/></rowFields>",
        )
        .replace(
            "<pageFields count=\"1\"><pageField fld=\"2\"/></pageFields>",
            "<pageFields count=\"0\"/>",
        )
    });
    let mut b = book(&bytes);
    let original = b.cell("Data", "C2").unwrap().number.unwrap();
    for reference in ["C2", "C4"] {
        b.set("Data", reference, CellInput::Number(original - 1462.))
            .unwrap();
    }
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(
        b.cell("Report", "E4").unwrap().number,
        Some(original - 1462.)
    );
    assert_eq!(
        b.workbook().pivots.caches[0].fields[2].shared_items[0].value,
        jade_xlsx_reader::pivot::PivotScalar::Date("2026-10-24T12:30:00".into())
    );
    query(
        &mut b,
        "L15",
        r#"GETPIVOTDATA("Amount",E3,"When",DATE(2026,10,24)+TIME(12,30,0))"#,
        15.,
    );
    query(&mut b, "M15", r#"GETPIVOTDATA("Amount",E3,"When",E4)"#, 15.);
    assert_eq!(
        b.workbook().pivots.tables[0].fields[2].axis.as_deref(),
        Some("axisRow")
    );
    assert!(b.workbook().pivots.tables[0].fields[0].axis.is_none());
}
#[test]
fn header_only_source_refresh_clears_old_output_but_retains_saved_item_indices() {
    let bytes = edit_part(FIXTURE, "xl/pivotCache/cache.xml", |s| {
        s.replace("ref=\"A1:C5\"", "ref=\"A1:C1\"")
    });
    let mut b = book(&bytes);
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(
        b.workbook().pivots.caches[0]
            .records
            .as_ref()
            .unwrap()
            .rows
            .len(),
        0
    );
    assert!(b.workbook().pivots.caches[0].fields[0]
        .shared_items
        .iter()
        .all(|v| v.unused));
    assert_eq!(
        b.cell("Report", "F5").unwrap().kind,
        jade_xlsx_headless_api::CellKind::Empty
    );
    assert_eq!(
        b.cell("Report", "F6").unwrap().kind,
        jade_xlsx_headless_api::CellKind::Empty
    );
    query(&mut b, "L15", r#"GETPIVOTDATA("Amount",E3)"#, 0.);
}
#[test]
fn report_refresh_obeys_the_sheet_pivot_permission_and_refusals_are_atomic() {
    for blocked in [false, true] {
        let bytes = edit_part(FIXTURE, "xl/worksheets/sheet2.xml", |s| {
            s.replace(
                "</worksheet>",
                &format!(
                    "<sheetProtection sheet=\"1\" pivotTables=\"{}\"/></worksheet>",
                    u8::from(blocked)
                ),
            )
        });
        let mut b = book(&bytes);
        b.set("Data", "B2", CellInput::Number(100.)).unwrap();
        let before = b.bytes().to_vec();
        let result = b.refresh_pivot("Report", "RegionalSales");
        if blocked {
            assert!(result.is_err());
            assert_eq!(b.bytes(), before);
        } else {
            result.unwrap();
            assert_eq!(b.cell("Report", "F4").unwrap().number, Some(105.));
        }
    }
}
#[test]
fn uninterpreted_replaced_entry_metadata_is_refused_instead_of_dropped() {
    for records in [false, true] {
        let bytes = if records {
            edit_part(FIXTURE, "xl/pivotCache/records.xml", |s| {
                s.replacen("<r>", "<r xmlns:f=\"urn:vendor\" f:identity=\"keep\">", 1)
            })
        } else {
            table(FIXTURE, |s| {
                s.replacen("<i>", "<i xmlns:f=\"urn:vendor\" f:identity=\"keep\">", 1)
            })
        };
        let mut b = book(&bytes);
        let before = b.bytes().to_vec();
        assert!(b.refresh_pivot("Report", "RegionalSales").is_err());
        assert_eq!(b.bytes(), before);
    }
}

#[test]
fn equivalent_cache_date_spellings_keep_selected_page_indices_and_results() {
    for fractional in [false, true] {
        let bytes = edit_part(FIXTURE, "xl/pivotCache/cache.xml", |s| {
            s.replace(
                "2026-10-24T12:30:00",
                if fractional {
                    "2026-10-24T12:30:00.5000"
                } else {
                    "2026-10-24T12:30:00.000Z"
                },
            )
        });
        let bytes = table(&bytes, |s| {
            s.replace(
                "<pageField fld=\"2\"/>",
                "<pageField fld=\"2\" item=\"0\"/>",
            )
        });
        let mut b = book(&bytes);
        if fractional {
            let original = b.cell("Data", "C2").unwrap().number.unwrap();
            for at in ["C2", "C4"] {
                b.set("Data", at, CellInput::Number(original + 0.5 / 86400.))
                    .unwrap();
            }
        }
        b.refresh_pivot("Report", "RegionalSales").unwrap();
        assert_eq!(
            b.cell("Report", "F4").unwrap().number,
            Some(15.),
            "fractional={fractional}"
        );
        assert_eq!(
            b.workbook().pivots.caches[0].records.as_ref().unwrap().rows[0][2].value,
            jade_xlsx_reader::pivot::PivotScalar::Shared(0)
        );
    }
}
#[test]
fn a_displayed_measure_caption_without_a_saved_name_becomes_a_queryable_field() {
    let bytes = table(FIXTURE, |s| s.replace("name=\"Total Amount\" ", ""));
    let mut b = book(&bytes);
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    query(&mut b, "H10", "GETPIVOTDATA(F3,E3)", 42.);
}

#[test]
fn new_member_attribute_text_keeps_newlines_tabs_and_literal_xstring_sequences() {
    let member = "North\nZone\t_x000A_";
    let mut b = book(FIXTURE);
    b.set("Data", "A3", CellInput::Text(member.into())).unwrap();
    b.refresh_pivot("Report", "RegionalSales").unwrap();
    assert_eq!(
        b.cell("Report", "E6").unwrap().text.as_deref(),
        Some(member)
    );
    query(
        &mut b,
        "L15",
        r#"GETPIVOTDATA("Amount",E3,"Region",E6)"#,
        20.,
    );
    let p = OpcPackage::open(b.bytes()).unwrap();
    let xml = p.required_part_str("xl/pivotCache/cache.xml").unwrap();
    assert!(xml.contains("North_x000A_Zone_x0009__x005F_x000A_"));
}

#[test]
fn a_shared_cache_report_on_another_protected_sheet_blocks_the_entire_refresh() {
    let mut package = OpcPackage::open(&shared_fixture()).unwrap();
    let report = "xl/worksheets/sheet2.xml";
    let xml = package
        .required_part_str(report)
        .unwrap()
        .replace("<pivotTablePart r:id=\"secondary\"/>", "")
        .replace(
            "<pivotTableParts count=\"2\">",
            "<pivotTableParts count=\"1\">",
        );
    package.replace(report, xml.into_bytes());
    let source = "xl/worksheets/sheet1.xml";
    let xml = package.required_part_str(source).unwrap().replace("</worksheet>",
        "<sheetProtection sheet=\"1\" pivotTables=\"1\"/><pivotTableParts count=\"1\"><pivotTablePart xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"secondary\"/></pivotTableParts></worksheet>");
    package.replace(source, xml.into_bytes());
    package.replace("xl/worksheets/_rels/sheet1.xml.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="secondary" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable" Target="../pivotTables/secondary.xml"/></Relationships>"#.to_vec());
    let rel = "xl/worksheets/_rels/sheet2.xml.rels";
    let xml = package.required_part_str(rel).unwrap().replace(
        "<Relationship Id=\"secondary\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable\" Target=\"../pivotTables/secondary.xml\"/>", "");
    package.replace(rel, xml.into_bytes());
    let bytes = package.write().unwrap();
    for blocked in [false, true] {
        let bytes = edit_part(&bytes, source, |s| {
            s.replace(
                "pivotTables=\"1\"",
                &format!("pivotTables=\"{}\"", u8::from(blocked)),
            )
        });
        let mut b = book(&bytes);
        assert_eq!(b.workbook().pivots.tables.len(), 2);
        let before = b.bytes().to_vec();
        let result = b.refresh_pivot("Report", "RegionalSales");
        if blocked {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("protected report sheet"), "{error}");
            assert_eq!(b.bytes(), before);
            assert!(!b.undo().unwrap());
        } else {
            assert_eq!(result.unwrap().tables.len(), 2);
            assert_eq!(b.cell("Data", "K6").unwrap().number, Some(42.));
            assert_eq!(b.cell("Report", "F6").unwrap().number, Some(42.));
            assert!(b.undo().unwrap());
            assert_eq!(b.bytes(), before);
        }
    }
}

fn separate_caches_fixture() -> Vec<u8> {
    let mut p = OpcPackage::open(&shared_fixture()).unwrap();
    for (part, original, edit) in [
        (
            "xl/pivotCache/second.xml",
            "xl/pivotCache/cache.xml",
            Some(("ref=\"A1:C5\"", "ref=\"D1:F5\"")),
        ),
        (
            "xl/pivotCache/second-records.xml",
            "xl/pivotCache/records.xml",
            None,
        ),
        (
            "xl/pivotCache/_rels/second.xml.rels",
            "xl/pivotCache/_rels/cache.xml.rels",
            Some(("Target=\"records.xml\"", "Target=\"second-records.xml\"")),
        ),
    ] {
        let mut xml = p.required_part_str(original).unwrap();
        if let Some((from, to)) = edit {
            xml = xml.replace(from, to);
        }
        p.replace(part, xml.into_bytes());
    }
    let part = "xl/pivotTables/secondary.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace("cacheId=\"7\"", "cacheId=\"9\"");
    p.replace(part, xml.into_bytes());
    let part = "xl/workbook.xml";
    let xml = p.required_part_str(part).unwrap().replace(
        "</pivotCaches>",
        "<pivotCache cacheId=\"9\" r:id=\"secondCache\"/></pivotCaches>",
    );
    p.replace(part, xml.into_bytes());
    let part = "xl/_rels/workbook.xml.rels";
    let xml = p.required_part_str(part).unwrap().replace("</Relationships>",
        "<Relationship Id=\"secondCache\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition\" Target=\"pivotCache/second.xml\"/></Relationships>");
    p.replace(part, xml.into_bytes());
    let part = "[Content_Types].xml";
    let xml = p.required_part_str(part).unwrap().replace("</Types>",
        "<Override PartName=\"/xl/pivotCache/second.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml\"/><Override PartName=\"/xl/pivotCache/second-records.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml\"/></Types>");
    p.replace(part, xml.into_bytes());
    let part = "xl/worksheets/sheet1.xml";
    let edits = [
        ("D1", jade_xlsx_writer::NewValue::Text("Region".into())),
        ("E1", jade_xlsx_writer::NewValue::Text("Amount".into())),
        ("F1", jade_xlsx_writer::NewValue::Text("When".into())),
        ("D2", jade_xlsx_writer::NewValue::Text("East".into())),
        ("E2", jade_xlsx_writer::NewValue::Number(2.)),
        ("D3", jade_xlsx_writer::NewValue::Text("West".into())),
        ("E3", jade_xlsx_writer::NewValue::Number(3.)),
        ("D4", jade_xlsx_writer::NewValue::Text("East".into())),
        ("E4", jade_xlsx_writer::NewValue::Number(4.)),
        ("D5", jade_xlsx_writer::NewValue::Text("West".into())),
        ("E5", jade_xlsx_writer::NewValue::Number(5.)),
    ]
    .into_iter()
    .map(|(at, value)| jade_xlsx_writer::CellEdit {
        reference: jade_xlsx_reader::CellRef::parse(at).unwrap(),
        value,
        style_index: None,
    })
    .collect::<Vec<_>>();
    let xml = jade_xlsx_writer::patch_sheet(&p.required_part_str(part).unwrap(), &edits).unwrap();
    p.replace(part, xml.into_bytes());
    p.write().unwrap()
}
#[test]
fn refresh_all_deduplicates_shared_caches_and_has_one_undo_redo_step() {
    let mut b = book(&shared_fixture());
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    let before = b.bytes().to_vec();
    let summaries = b.refresh_all_pivots().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].tables.len(), 2);
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(132.));
    assert_eq!(b.cell("Report", "K6").unwrap().number, Some(132.));
    let after = b.bytes().to_vec();
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}
#[test]
fn refresh_all_updates_distinct_caches_and_recalculates_between_sources() {
    let mut b = book(&separate_caches_fixture());
    assert!(b.workbook().pivots.issues.is_empty());
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    b.set("Data", "E2", CellInput::Formula("Report!F4".into()))
        .unwrap();
    let before = b.bytes().to_vec();
    let result = b.refresh_all_pivots().unwrap();
    assert_eq!(
        result.iter().map(|s| s.cache_id).collect::<Vec<_>>(),
        [7, 9]
    );
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(132.));
    assert_eq!(b.cell("Report", "K6").unwrap().number, Some(117.));
    let after = b.bytes().to_vec();
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}
#[test]
fn a_later_refresh_failure_restores_every_cache_formula_and_history_branch() {
    for unsupported in [false, true] {
        let mut b = book(&separate_caches_fixture());
        b.set("Data", "B2", CellInput::Number(100.)).unwrap();
        if unsupported {
            b.set(
                "Data",
                "E2",
                CellInput::Formula("NOT_IMPLEMENTED(1)".into()),
            )
            .unwrap();
        } else {
            b.set("Data", "D3", CellInput::Text("North".into()))
                .unwrap();
            b.set("Report", "K7", CellInput::Text("Keep".into()))
                .unwrap();
        }
        let before_edit = b.bytes().to_vec();
        b.set("Report", "N10", CellInput::Number(99.)).unwrap();
        let future = b.bytes().to_vec();
        assert!(b.undo().unwrap());
        assert_eq!(b.bytes(), before_edit);
        let before = b.bytes().to_vec();
        assert!(b.refresh_all_pivots().is_err());
        assert_eq!(b.bytes(), before);
        assert_eq!(b.cell("Report", "F6").unwrap().number, Some(42.));
        assert_eq!(b.cell("Report", "K6").unwrap().number, Some(42.));
        assert!(b.redo().unwrap());
        assert_eq!(b.bytes(), future);
    }
}
#[test]
fn refresh_all_without_reports_is_a_true_noop_and_unknown_catalogs_are_refused() {
    let empty = edit_part(FIXTURE, "xl/workbook.xml", |s| {
        s.replace(
            "<pivotCaches><pivotCache cacheId=\"7\" r:id=\"sharedCache\"/></pivotCaches>",
            "",
        )
    });
    let empty = edit_part(&empty, "xl/worksheets/sheet2.xml", |s| {
        s.replace(
            "<pivotTableParts count=\"1\"><pivotTablePart r:id=\"pivot\"/></pivotTableParts>",
            "",
        )
    });
    let mut package = OpcPackage::open(&empty).unwrap();
    package.remove("xl/worksheets/_rels/sheet2.xml.rels");
    let empty = package.write().unwrap();
    let mut b = book(&empty);
    let before = b.bytes().to_vec();
    assert!(b.refresh_all_pivots().unwrap().is_empty());
    assert_eq!(b.bytes(), before);
    assert!(!b.undo().unwrap());
    let bytes = edit_part(FIXTURE, "xl/worksheets/_rels/sheet2.xml.rels", |s| {
        s.replace("../pivotTables/report.xml", "../pivotTables/missing.xml")
    });
    let mut b = book(&bytes);
    assert!(!b.workbook().pivots.issues.is_empty());
    let before = b.bytes().to_vec();
    assert!(b.refresh_all_pivots().is_err());
    assert_eq!(b.bytes(), before);
}

fn reversed_cache_fixture() -> Vec<u8> {
    edit_part(
        &separate_caches_fixture(),
        "xl/worksheets/_rels/sheet2.xml.rels",
        |s| {
            let primary = r#"<Relationship Id="pivot" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable" Target="../pivotTables/report.xml"/>"#;
            let secondary = r#"<Relationship Id="secondary" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable" Target="../pivotTables/secondary.xml"/>"#;
            assert!(s.contains(&format!("{primary}{secondary}")));
            s.replace(
                &format!("{primary}{secondary}"),
                &format!("{secondary}{primary}"),
            )
        },
    )
}

#[test]
fn refresh_all_orders_reverse_declared_formula_dependencies_before_consumers() {
    let mut b = book(&reversed_cache_fixture());
    assert_eq!(b.workbook().pivots.tables[0].cache_id, 9);
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    b.set("Data", "E2", CellInput::Formula("Report!F4".into()))
        .unwrap();
    let before = b.bytes().to_vec();
    b.refresh_all_pivots().unwrap();
    assert_eq!(b.cell("Report", "K6").unwrap().number, Some(117.));
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(132.));
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
}
#[test]
fn refresh_all_refuses_cycles_between_cache_sources_without_changing_history() {
    let mut b = book(&separate_caches_fixture());
    b.set("Data", "B2", CellInput::Formula("Report!K4".into()))
        .unwrap();
    b.set("Data", "E2", CellInput::Formula("Report!F4".into()))
        .unwrap();
    let before = b.bytes().to_vec();
    assert!(b.refresh_all_pivots().is_err());
    assert_eq!(b.bytes(), before);
}

#[test]
fn reverse_dependencies_follow_names_dynamic_references_ranges_and_getpivotdata() {
    for formula in [
        "PivotEast",
        "INDIRECT(\"Report!F4\")",
        "SUM(OFFSET(Report!E4,0,1))",
        "SUM(Report!F4:F4)",
        "GETPIVOTDATA(\"Amount\",Report!E3,\"Region\",\"East\")",
    ] {
        let mut b = book(&reversed_cache_fixture());
        b.set_defined_name(&jade_xlsx_reader::DefinedName {
            name: "PivotEast".into(),
            formula: "Report!$F$4".into(),
            local_sheet_index: None,
            hidden: false,
        })
        .unwrap();
        b.set("Data", "B2", CellInput::Number(100.)).unwrap();
        b.set("Data", "E2", CellInput::Formula(formula.into()))
            .unwrap();
        b.refresh_all_pivots().unwrap();
        assert_eq!(
            b.cell("Report", "K6").unwrap().number,
            Some(117.),
            "{formula}"
        );
    }
}
#[test]
fn growth_into_a_previously_empty_dependency_invalidates_an_earlier_refresh() {
    let mut b = book(&reversed_cache_fixture());
    b.set("Data", "A3", CellInput::Text("North".into()))
        .unwrap();
    b.set("Data", "E2", CellInput::Formula("Report!F7".into()))
        .unwrap();
    b.refresh_all_pivots().unwrap();
    assert_eq!(b.cell("Report", "F7").unwrap().number, Some(42.));
    assert_eq!(b.cell("Report", "K6").unwrap().number, Some(54.));
}
#[test]
fn a_dynamic_dependency_that_becomes_circular_after_another_refresh_is_atomic() {
    let mut b = book(&reversed_cache_fixture());
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    b.set(
        "Data",
        "E2",
        CellInput::Formula("INDIRECT(IF(Report!F4<20,\"Report!F4\",\"Report!K4\"))".into()),
    )
    .unwrap();
    let before = b.bytes().to_vec();
    assert!(b.refresh_all_pivots().is_err());
    assert_eq!(b.bytes(), before);
}
#[test]
fn volatile_sources_do_not_require_value_convergence_or_repeat_unchanged_dependencies() {
    let mut b = book(&reversed_cache_fixture());
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    b.set("Data", "E2", CellInput::Formula("Report!F4+RAND()".into()))
        .unwrap();
    b.refresh_all_pivots().unwrap();
    let value = b.cell("Report", "K6").unwrap().number.unwrap();
    assert!((117.0..118.0).contains(&value), "{value}");
    let saved = b.bytes().to_vec();
    assert!(b.undo().unwrap());
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), saved);
}

#[test]
fn a_direct_report_source_is_ordered_before_the_dependent_cache() {
    let bytes = edit_part(&reversed_cache_fixture(), "xl/pivotCache/second.xml", |s| {
        s.replace(
            "sheet=\"Data\" ref=\"D1:F5\"",
            "sheet=\"Report\" ref=\"E3:G6\"",
        )
        .replace("name=\"Region\"", "name=\"Territory\"")
        .replace("name=\"Amount\"", "name=\"Total Amount\"")
    });
    let mut b = book(&bytes);
    b.set("Report", "G3", CellInput::Text("When".into()))
        .unwrap();
    b.set("Data", "B2", CellInput::Number(100.)).unwrap();
    b.refresh_all_pivots().unwrap();
    assert_eq!(b.cell("Report", "F6").unwrap().number, Some(132.));
    assert_eq!(b.cell("Report", "K7").unwrap().number, Some(264.));
}
#[test]
fn structured_reference_and_spill_intermediates_preserve_transitive_dependencies() {
    for spill in [false, true] {
        let mut b = book(&reversed_cache_fixture());
        b.set("Data", "B2", CellInput::Number(100.)).unwrap();
        if spill {
            b.set(
                "Report",
                "M2",
                CellInput::Formula("SEQUENCE(2,1,F4,0)".into()),
            )
            .unwrap();
            b.set("Data", "E2", CellInput::Formula("Report!M3".into()))
                .unwrap();
        } else {
            b.set("Report", "M1", CellInput::Text("Amount".into()))
                .unwrap();
            b.set("Report", "M2", CellInput::Formula("F4".into()))
                .unwrap();
            b.create_table(
                "Report",
                jade_xlsx_reader::CellRange::parse("M1:M2").unwrap(),
                &jade_xlsx_headless_editor::TableOptions {
                    name: Some("Dependent".into()),
                    ..Default::default()
                },
            )
            .unwrap();
            b.set(
                "Data",
                "E2",
                CellInput::Formula("SUM(Dependent[Amount])".into()),
            )
            .unwrap();
        }
        b.refresh_all_pivots().unwrap();
        assert_eq!(
            b.cell("Report", "K6").unwrap().number,
            Some(117.),
            "spill={spill}"
        );
    }
}
#[test]
fn named_and_table_cache_source_bounds_use_the_same_resolver_as_refresh() {
    for table in [false, true] {
        let mut b = book(&reversed_cache_fixture());
        if table {
            b.create_table(
                "Data",
                jade_xlsx_reader::CellRange::parse("D1:F5").unwrap(),
                &jade_xlsx_headless_editor::TableOptions {
                    name: Some("AlternateSource".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        } else {
            b.set_defined_name(&jade_xlsx_reader::DefinedName {
                name: "AlternateSource".into(),
                formula: "Data!$D$1:$F$5".into(),
                local_sheet_index: None,
                hidden: false,
            })
            .unwrap();
        }
        let bytes = edit_part(b.bytes(), "xl/pivotCache/second.xml", |s| {
            s.replace("sheet=\"Data\" ref=\"D1:F5\"", "name=\"AlternateSource\"")
        });
        let mut b = book(&bytes);
        b.set("Data", "B2", CellInput::Number(100.)).unwrap();
        b.set("Data", "E2", CellInput::Formula("Report!F4".into()))
            .unwrap();
        b.refresh_all_pivots().unwrap();
        assert_eq!(
            b.cell("Report", "K6").unwrap().number,
            Some(117.),
            "table={table}"
        );
    }
}

#[test]
fn refresh_all_preserves_the_widest_refreshed_report_on_shared_columns() {
    use jade_xlsx_writer::{CellEdit, NewValue};
    let mut p = OpcPackage::open(&separate_caches_fixture()).unwrap();
    let part = "xl/pivotTables/secondary.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace("J3:K6", "E10:F13");
    p.replace(part, xml.into_bytes());
    let mut edits = Vec::new();
    for (old, new) in [
        ("J3", "E10"),
        ("K3", "F10"),
        ("J4", "E11"),
        ("K4", "F11"),
        ("J5", "E12"),
        ("K5", "F12"),
        ("J6", "E13"),
        ("K6", "F13"),
    ] {
        let b = jade_xlsx_reader::read_xlsx(&p.write().unwrap()).unwrap();
        let cell = b.sheets[1]
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .find(|c| c.reference == jade_xlsx_reader::CellRef::parse(old).unwrap())
            .unwrap();
        let value = match &cell.value {
            jade_xlsx_reader::CellValue::Text(v) => NewValue::Text(v.clone()),
            jade_xlsx_reader::CellValue::Number(v) => NewValue::Number(*v),
            _ => panic!("saved report value"),
        };
        edits.push(CellEdit {
            reference: jade_xlsx_reader::CellRef::parse(old).unwrap(),
            value: NewValue::Remove,
            style_index: None,
        });
        edits.push(CellEdit {
            reference: jade_xlsx_reader::CellRef::parse(new).unwrap(),
            value,
            style_index: cell.style_index,
        });
    }
    let part = "xl/worksheets/sheet2.xml";
    let xml = jade_xlsx_writer::patch_sheet(&p.required_part_str(part).unwrap(), &edits).unwrap();
    p.replace(part, xml.into_bytes());
    let mut b = book(&p.write().unwrap());
    b.set(
        "Data",
        "A2",
        CellInput::Text(
            "A very long primary report label that must survive a later short report".into(),
        ),
    )
    .unwrap();
    let before = b.bytes().to_vec();
    assert_eq!(b.refresh_all_pivots().unwrap().len(), 2);
    let after = b.bytes().to_vec();
    let width = b.workbook().sheets[1]
        .column_span(4)
        .and_then(|c| c.width_chars)
        .unwrap();
    assert!(width > 40., "{width}");
    assert_eq!(b.cell("Report", "F7").unwrap().number, Some(42.));
    assert_eq!(b.cell("Report", "F13").unwrap().number, Some(14.));
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}

#[test]
fn a_later_cache_measurement_failure_restores_earlier_widths_and_the_redo_branch() {
    use jade_xlsx_layout::{LayoutOptions, ResolvedFont, TextMeasure};
    use std::rc::Rc;
    struct Measurement;
    impl TextMeasure for Measurement {
        fn measure(&self, text: &str, _: &ResolvedFont) -> f64 {
            if text == "Bad Metric" {
                f64::NAN
            } else {
                text.chars().count() as f64 * 8.
            }
        }
    }
    let bytes = edit_part(
        &separate_caches_fixture(),
        "xl/pivotTables/secondary.xml",
        |s| s.replace("Total Amount", "Bad Metric"),
    );
    let mut b = book(&bytes);
    b.set("Data", "B2", CellInput::Formula("50+50".into()))
        .unwrap();
    let before = b.bytes().to_vec();
    b.set("Data", "B3", CellInput::Number(99.)).unwrap();
    let future = b.bytes().to_vec();
    assert!(b.undo().unwrap());
    let options = LayoutOptions {
        measure: Rc::new(Measurement),
        ..Default::default()
    };
    assert!(b
        .refresh_all_pivots_with_layout(&options)
        .unwrap_err()
        .to_string()
        .contains("measurement"));
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), future);
}
