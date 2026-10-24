use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{
    HeadlessWorkbook, TableOptions, TableStyle, TableTotal, TableUpdate, TotalFunction,
};
use jade_xlsx_reader::CellRange;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../public/fixtures/{name}.xlsx",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open(&fixture("sample-workbook")).unwrap()
}
fn range(s: &str) -> CellRange {
    CellRange::parse(s).unwrap()
}
fn seed(book: &mut HeadlessWorkbook) -> String {
    let sheet = book.workbook().sheets[0].name.clone();
    book.set_cells(
        &sheet,
        &[
            ("H2".into(), CellInput::Text("Item".into())),
            ("I2".into(), CellInput::Text("Qty".into())),
            ("J2".into(), CellInput::Text("Amount".into())),
            ("H3".into(), CellInput::Text("A".into())),
            ("I3".into(), CellInput::Number(2.)),
            ("J3".into(), CellInput::Number(20.)),
            ("H4".into(), CellInput::Text("B".into())),
            ("I4".into(), CellInput::Number(3.)),
            ("J4".into(), CellInput::Number(45.)),
        ],
    )
    .unwrap();
    sheet
}
#[test]
fn create_style_and_reopen_keep_headers_relationships_structured_calculation_and_history() {
    let mut book = open();
    let sheet = seed(&mut book);
    let before = book.bytes().to_vec();
    let name = book
        .create_table(
            &sheet,
            range("H2:J4"),
            &TableOptions {
                name: Some("Sales".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(name, "Sales");
    let table = &book.workbook().sheets[0].tables[0];
    assert_eq!(table.columns, ["Item", "Qty", "Amount"]);
    assert!(table.auto_filter);
    assert_eq!(table.style_name.as_deref(), Some("TableStyleMedium2"));
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    book.set(
        &sheet,
        "L2",
        CellInput::Formula("SUM(Sales[Amount])".into()),
    )
    .unwrap();
    assert_eq!(book.cell(&sheet, "L2").unwrap().number, Some(65.));
    let bytes = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&bytes).unwrap();
    assert_eq!(
        reopened.workbook().sheets[0].tables[0].columns,
        table_columns()
    );
    let p = jade_opc::OpcPackage::open(&bytes).unwrap();
    let rels = jade_opc::parse_relationships(
        &p.required_part_str(&jade_opc::rels_part_name(
            &book.workbook().sheets[0].part_name,
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(rels.by_type_suffix("/table").len(), 1);
    assert!(p
        .required_part_str("[Content_Types].xml")
        .unwrap()
        .contains("spreadsheetml.table+xml"));
    let data_before = book.cell(&sheet, "J3").unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            style: Some(TableStyle {
                name: Some("TableStyleDark8".into()),
                first_column: true,
                column_stripes: true,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(book.cell(&sheet, "J3").unwrap().number, data_before.number);
    assert!(book.workbook().sheets[0].tables[0].show_first_column);
    assert!(book.workbook().sheets[0].tables[0].show_column_stripes);
}
fn table_columns() -> Vec<String> {
    vec!["Item".into(), "Qty".into(), "Amount".into()]
}
#[test]
fn headerless_creation_moves_data_and_formulas_and_uniquifies_text_headers() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.set(&sheet, "J3", CellInput::Formula("I3*10".into()))
        .unwrap();
    let outside = book.cell(&sheet, "A1").unwrap();
    book.create_table(
        &sheet,
        range("H3:J4"),
        &TableOptions {
            name: Some("NoHeaders".into()),
            has_headers: false,
            ..Default::default()
        },
    )
    .unwrap();
    let t = &book.workbook().sheets[0].tables[0];
    assert_eq!(t.reference, range("H3:J5"));
    assert_eq!(t.columns, ["Column1", "Column2", "Column3"]);
    assert_eq!(
        book.cell(&sheet, "J4").unwrap().formula.as_deref(),
        Some("I4*10")
    );
    assert_eq!(book.cell(&sheet, "J4").unwrap().number, Some(20.));
    assert_eq!(book.cell(&sheet, "H5").unwrap().display, "B");
    assert_eq!(book.cell(&sheet, "A1").unwrap(), outside);
    let mut book = open();
    let sheet = seed(&mut book);
    book.set_cells(
        &sheet,
        &[
            ("H2".into(), CellInput::Text("Name".into())),
            ("I2".into(), CellInput::Text("name".into())),
            ("J2".into(), CellInput::Blank),
        ],
    )
    .unwrap();
    book.create_table(&sheet, range("H2:J4"), &Default::default())
        .unwrap();
    assert_eq!(
        book.workbook().sheets[0].tables[0].columns,
        ["Name", "name2", "Column3"]
    );
    assert_eq!(book.cell(&sheet, "I2").unwrap().display, "name2");
}
#[test]
fn rename_table_updates_cross_sheet_cells_names_and_preserves_string_and_external_refs() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:J4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let other = book.workbook().sheets[1].name.clone();
    book.set_cells(
        &other,
        &[
            ("H2".into(), CellInput::Formula("SUM(Sales[Amount])".into())),
            ("H3".into(), CellInput::Formula("\"Sales[Amount]\"".into())),
        ],
    )
    .unwrap();
    book.set_defined_name(&jade_xlsx_reader::DefinedName {
        name: "GrandTotal".into(),
        formula: "SUM(Sales[Amount])".into(),
        local_sheet_index: None,
        hidden: false,
    })
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            name: Some("売上".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        book.cell(&other, "H2").unwrap().formula.as_deref(),
        Some("SUM(売上[Amount])")
    );
    assert_eq!(book.cell(&other, "H2").unwrap().number, Some(65.));
    assert_eq!(book.cell(&other, "H3").unwrap().display, "Sales[Amount]");
    assert_eq!(
        book.workbook()
            .defined_names
            .iter()
            .find(|n| n.name == "GrandTotal")
            .unwrap()
            .formula,
        "SUM(売上[Amount])"
    );
}
#[test]
fn totals_all_functions_and_calculated_columns_recalculate_resize_and_undo() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:K4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            calculated_columns: vec![(3, Some("I3*J3".into()))],
            totals_row: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(book.cell(&sheet, "K3").unwrap().number, Some(40.));
    assert_eq!(book.cell(&sheet, "K4").unwrap().number, Some(135.));
    assert_eq!(
        book.workbook().sheets[0].tables[0].column_properties[3]
            .calculated_formula
            .as_deref(),
        Some("I3*J3")
    );
    for function in [
        TotalFunction::Sum,
        TotalFunction::Average,
        TotalFunction::Count,
        TotalFunction::CountNumbers,
        TotalFunction::Max,
        TotalFunction::Min,
        TotalFunction::StandardDeviation,
        TotalFunction::Variance,
    ] {
        book.update_table(
            &sheet,
            "Sales",
            &TableUpdate {
                totals: vec![(1, TableTotal::Function(function))],
                ..Default::default()
            },
        )
        .unwrap();
        let c = book.cell(&sheet, "I5").unwrap();
        assert_eq!(
            c.formula,
            Some(format!("SUBTOTAL({},Sales[Qty])", function.code()))
        );
        assert!(c.number.is_some(), "{c:?}");
    }
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            totals: vec![
                (2, TableTotal::Formula("SUM(Sales[Amount])*2".into())),
                (0, TableTotal::Label("合計".into())),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(book.cell(&sheet, "J5").unwrap().number, Some(130.));
    assert_eq!(book.cell(&sheet, "H5").unwrap().display, "合計");
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            totals_row: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(book.cell(&sheet, "H5").unwrap().display, "");
    assert_eq!(
        book.workbook().sheets[0].tables[0].reference,
        range("H2:K4")
    );
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            reference: Some(range("H2:K6")),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        book.cell(&sheet, "K6").unwrap().formula.as_deref(),
        Some("I6*J6")
    );
    book.undo().unwrap();
    assert_eq!(
        book.workbook().sheets[0].tables[0].reference,
        range("H2:K4")
    );
    assert_eq!(book.cell(&sheet, "K6").unwrap().display, "");
}
#[test]
fn invalid_overlapping_merged_protected_names_and_occupied_rows_are_atomic() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:J4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let before = book.bytes().to_vec();
    for (r, name) in [("I3:K6", "Other"), ("L2:M4", "A1"), ("L2:M4", "sales")] {
        assert!(book
            .create_table(
                &sheet,
                range(r),
                &TableOptions {
                    name: Some(name.into()),
                    ..Default::default()
                }
            )
            .is_err());
        assert_eq!(book.bytes(), before);
    }
    assert!(book
        .update_table(
            &sheet,
            "Sales",
            &TableUpdate {
                reference: Some(range("H3:J5")),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(book.bytes(), before);
    let mut book = open();
    let sheet = seed(&mut book);
    book.set(&sheet, "J5", CellInput::Number(99.)).unwrap();
    book.create_table(
        &sheet,
        range("H2:J4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let before = book.bytes().to_vec();
    assert!(book
        .update_table(
            &sheet,
            "Sales",
            &TableUpdate {
                totals_row: Some(true),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(book.bytes(), before);
    let mut protected = HeadlessWorkbook::open(&fixture("protected-sheets-workbook")).unwrap();
    assert!(protected
        .create_table("Locked", range("H2:J4"), &Default::default())
        .is_err());
    assert!(!protected.can_undo());
}

#[test]
fn direct_header_edits_update_structured_refs_and_calculated_metadata_in_the_same_undo() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:K4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            calculated_columns: vec![(3, Some("[@Qty]*[@Amount]".into()))],
            ..Default::default()
        },
    )
    .unwrap();
    book.set(
        &sheet,
        "M2",
        CellInput::Formula("SUM(Sales[Amount])".into()),
    )
    .unwrap();
    let before = book.bytes().to_vec();
    book.set(&sheet, "J2", CellInput::Text("Price ] #".into()))
        .unwrap();
    assert_eq!(
        book.cell(&sheet, "M2").unwrap().formula.as_deref(),
        Some("SUM(Sales[Price '] '#])")
    );
    assert_eq!(book.cell(&sheet, "M2").unwrap().number, Some(65.));
    assert_eq!(book.cell(&sheet, "K3").unwrap().number, Some(40.));
    assert!(book.recalculate().unwrap().skipped.is_empty());
    let table = &book.workbook().sheets[0].tables[0];
    assert_eq!(
        table.column_properties[3].calculated_formula.as_deref(),
        Some("[@Qty]*[@Price '] '#]")
    );
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
}
#[test]
fn entries_expand_tables_and_fill_empty_calculated_columns_with_one_undo() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:J4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let before = book.bytes().to_vec();
    book.set(&sheet, "K3", CellInput::Formula("I3*J3".into()))
        .unwrap();
    assert_eq!(
        book.workbook().sheets[0].tables[0].reference,
        range("H2:K4")
    );
    assert_eq!(book.cell(&sheet, "K4").unwrap().number, Some(135.));
    assert_eq!(
        book.cell(&sheet, "K4").unwrap().formula.as_deref(),
        Some("I4*J4")
    );
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    let before = book.bytes().to_vec();
    book.set_cells(
        &sheet,
        &[
            ("H5".into(), CellInput::Text("C".into())),
            ("I5".into(), CellInput::Number(4.)),
            ("J5".into(), CellInput::Number(10.)),
            ("H6".into(), CellInput::Text("D".into())),
            ("I6".into(), CellInput::Number(5.)),
            ("J6".into(), CellInput::Number(11.)),
        ],
    )
    .unwrap();
    assert_eq!(
        book.workbook().sheets[0].tables[0].reference,
        range("H2:K6")
    );
    assert_eq!(book.cell(&sheet, "K6").unwrap().number, Some(55.));
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
}
#[test]
fn conversion_keeps_values_style_and_equivalent_structured_references() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:K4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            calculated_columns: vec![(3, Some("[@Qty]*[@Amount]".into()))],
            totals_row: Some(true),
            totals: vec![(2, TableTotal::Function(TotalFunction::Sum))],
            ..Default::default()
        },
    )
    .unwrap();
    book.set(
        &sheet,
        "M2",
        CellInput::Formula("SUM(Sales[Amount])".into()),
    )
    .unwrap();
    let before = book.bytes().to_vec();
    let old_total = book.cell(&sheet, "J5").unwrap().number;
    book.convert_table_to_range(&sheet, "Sales").unwrap();
    assert!(book.workbook().sheets[0].tables.is_empty());
    assert_eq!(book.cell(&sheet, "K4").unwrap().number, Some(135.));
    assert_eq!(book.cell(&sheet, "M2").unwrap().number, Some(65.));
    assert_eq!(book.cell(&sheet, "J5").unwrap().number, old_total);
    assert!(!book
        .cell(&sheet, "K4")
        .unwrap()
        .formula
        .unwrap()
        .contains('['));
    assert_eq!(book.format_at(&sheet, "H2").unwrap().bold, Some(true));
    let bytes = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&bytes).unwrap();
    assert!(reopened.workbook().sheets[0].tables.is_empty());
    assert_eq!(reopened.cell(&sheet, "M2").unwrap().number, Some(65.));
    let p = jade_opc::OpcPackage::open(&bytes).unwrap();
    assert!(!p
        .required_part_str("[Content_Types].xml")
        .unwrap()
        .contains("spreadsheetml.table+xml"));
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
}

#[test]
fn strict_aliases_unknown_metadata_column_ids_and_formula_attributes_survive() {
    let mut p = jade_opc::OpcPackage::open(&fixture("objects-workbook")).unwrap();
    let xml = r#"<s:table xmlns:s="http://purl.oclc.org/ooxml/spreadsheetml/main" xmlns:a="http://purl.oclc.org/ooxml/spreadsheetml/main" xmlns:f="urn:foreign" id="1" name="Table1" displayName="Table1" ref="A1:C5" totalsRowCount="1" f:keep="root"><s:autoFilter ref="A1:C4"/><a:tableColumns count="3" f:keep="columns"><f:tableColumn name="ignored"/><s:tableColumn id="4" name="Item" totalsRowLabel="Total" f:keep="column"><f:calculatedColumnFormula>opaque</f:calculatedColumnFormula></s:tableColumn><s:tableColumn id="5" name="Qty"><a:calculatedColumnFormula f:keep="formula">ROW()&lt;10</a:calculatedColumnFormula></s:tableColumn><s:tableColumn id="6" name="Amount" totalsRowFunction="sum"/></a:tableColumns><s:tableStyleInfo name="TableStyleMedium2" showRowStripes="1" f:keep="style"/><s:extLst><f:opaque/></s:extLst></s:table>"#;
    p.replace("xl/tables/table1.xml", xml.as_bytes().to_vec());
    let mut book = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    let t = &book.workbook().sheets[0].tables[0];
    assert_eq!(t.columns, ["Item", "Qty", "Amount"]);
    assert_eq!(
        t.column_properties[1].calculated_formula.as_deref(),
        Some("ROW()<10")
    );
    book.update_table(
        &sheet,
        "Table1",
        &TableUpdate {
            style: Some(TableStyle {
                name: Some("TableStyleDark3".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let p = jade_opc::OpcPackage::open(book.bytes()).unwrap();
    let after = p.required_part_str("xl/tables/table1.xml").unwrap();
    for keep in [
        "f:keep=\"root\"",
        "f:keep=\"columns\"",
        "f:keep=\"column\"",
        "f:keep=\"formula\"",
        "f:keep=\"style\"",
        "<f:tableColumn name=\"ignored\"/>",
        "<f:calculatedColumnFormula>opaque</f:calculatedColumnFormula>",
        "<f:opaque/>",
    ] {
        assert!(after.contains(keep), "{keep}: {after}");
    }
    assert_eq!(
        book.workbook().sheets[0].tables[0]
            .column_properties
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [4, 5, 6]
    );
}
#[test]
fn headers_filter_buttons_and_totals_can_toggle_without_losing_data() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:J4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            header_row: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    let t = &book.workbook().sheets[0].tables[0];
    assert_eq!(t.reference, range("H3:J4"));
    assert_eq!(t.header_row_count, 0);
    assert!(!t.auto_filter);
    assert_eq!(book.cell(&sheet, "H2").unwrap().display, "");
    assert_eq!(book.cell(&sheet, "J3").unwrap().number, Some(20.));
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            header_row: Some(true),
            filter_buttons: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(book.cell(&sheet, "H2").unwrap().display, "Item");
    assert!(!book.workbook().sheets[0].tables[0].auto_filter);
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            filter_buttons: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(book.workbook().sheets[0].tables[0].auto_filter);
}

#[test]
fn editing_or_clearing_total_cells_updates_metadata_without_coercing_numeric_values() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:J4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            totals_row: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    book.set(
        &sheet,
        "J5",
        CellInput::Formula("SUM(Sales[Amount])*2".into()),
    )
    .unwrap();
    let c = &book.workbook().sheets[0].tables[0].column_properties[2];
    assert_eq!(c.totals_function.as_deref(), Some("custom"));
    assert_eq!(c.totals_formula.as_deref(), Some("SUM(Sales[Amount])*2"));
    assert_eq!(book.cell(&sheet, "J5").unwrap().number, Some(130.));
    book.set(&sheet, "J5", CellInput::Number(42.)).unwrap();
    assert_eq!(book.cell(&sheet, "J5").unwrap().number, Some(42.));
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            style: Some(TableStyle {
                name: Some("TableStyleDark4".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(book.cell(&sheet, "J5").unwrap().number, Some(42.));
    let c = &book.workbook().sheets[0].tables[0].column_properties[2];
    assert!(c.totals_function.is_none());
    assert!(c.totals_formula.is_none());
    book.set(&sheet, "J5", CellInput::Blank).unwrap();
    assert_eq!(book.cell(&sheet, "J5").unwrap().display, "");
    assert!(book.workbook().sheets[0].tables[0].column_properties[2]
        .totals_label
        .is_none());
    book.undo().unwrap();
    assert_eq!(book.cell(&sheet, "J5").unwrap().number, Some(42.));
}

#[test]
fn snapshot_paste_expands_table_rows_fills_calculated_columns_and_undoes_atomically() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:K4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            calculated_columns: vec![(3, Some("I3*J3".into()))],
            ..Default::default()
        },
    )
    .unwrap();
    book.set_cells(
        &sheet,
        &[
            ("M2".into(), CellInput::Text("C".into())),
            ("N2".into(), CellInput::Number(4.)),
            ("O2".into(), CellInput::Number(10.)),
            ("M3".into(), CellInput::Text("D".into())),
            ("N3".into(), CellInput::Number(5.)),
            ("O3".into(), CellInput::Number(12.)),
        ],
    )
    .unwrap();
    let snapshot = book.copy_snapshot(&sheet, "M2:O3", false).unwrap();
    let before = book.bytes().to_vec();
    assert_eq!(
        book.paste_snapshot(&snapshot, &sheet, "H5", false).unwrap(),
        range("H5:J6")
    );
    assert_eq!(
        book.workbook().sheets[0].tables[0].reference,
        range("H2:K6")
    );
    assert_eq!(book.cell(&sheet, "K5").unwrap().number, Some(40.));
    assert_eq!(book.cell(&sheet, "K6").unwrap().number, Some(60.));
    let saved = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert_eq!(
        saved.workbook().sheets[0].tables[0].reference,
        range("H2:K6")
    );
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    assert_eq!(book.cell(&sheet, "K6").unwrap().number, Some(60.));
}

#[test]
fn snapshot_header_paste_follows_explicit_and_implicit_column_references() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:K4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            calculated_columns: vec![(3, Some("[@Qty]*[@Amount]".into()))],
            ..Default::default()
        },
    )
    .unwrap();
    book.set_cells(
        &sheet,
        &[
            ("M2".into(), CellInput::Text("Units".into())),
            ("Q2".into(), CellInput::Formula("SUM(Sales[Qty])".into())),
            ("R2".into(), CellInput::Formula("\"Sales[Qty]\"".into())),
        ],
    )
    .unwrap();
    let snapshot = book.copy_snapshot(&sheet, "M2", false).unwrap();
    book.paste_snapshot(&snapshot, &sheet, "I2", false).unwrap();
    assert_eq!(book.workbook().sheets[0].tables[0].columns[1], "Units");
    assert_eq!(
        book.cell(&sheet, "Q2").unwrap().formula.as_deref(),
        Some("SUM(Sales[Units])")
    );
    assert_eq!(book.cell(&sheet, "R2").unwrap().display, "Sales[Qty]");
    assert_eq!(
        book.cell(&sheet, "K4").unwrap().formula.as_deref(),
        Some("[@Units]*[@Amount]")
    );
    assert_eq!(book.cell(&sheet, "K4").unwrap().number, Some(135.));
    assert_eq!(book.cell(&sheet, "Q2").unwrap().number, Some(5.));
}

#[test]
fn snapshot_total_paste_and_cut_keep_column_metadata_and_cell_values_in_step() {
    let mut book = open();
    let sheet = seed(&mut book);
    book.create_table(
        &sheet,
        range("H2:J4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.update_table(
        &sheet,
        "Sales",
        &TableUpdate {
            totals_row: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    book.set(
        &sheet,
        "M2",
        CellInput::Formula("SUM(Sales[Amount])*2".into()),
    )
    .unwrap();
    let snapshot = book.copy_snapshot(&sheet, "M2", false).unwrap();
    book.paste_snapshot(&snapshot, &sheet, "J5", false).unwrap();
    let column = &book.workbook().sheets[0].tables[0].column_properties[2];
    assert_eq!(column.totals_function.as_deref(), Some("custom"));
    assert_eq!(
        column.totals_formula.as_deref(),
        Some("SUM(Sales[Amount])*2")
    );
    assert_eq!(book.cell(&sheet, "J5").unwrap().number, Some(130.));
    let snapshot = book.copy_snapshot(&sheet, "J5", true).unwrap();
    book.paste_snapshot(&snapshot, &sheet, "M5", false).unwrap();
    let column = &book.workbook().sheets[0].tables[0].column_properties[2];
    assert!(column.totals_function.is_none());
    assert!(column.totals_formula.is_none());
    assert_eq!(book.cell(&sheet, "J5").unwrap().display, "");
    assert_eq!(book.cell(&sheet, "M5").unwrap().number, Some(130.));
    book.undo().unwrap();
    assert_eq!(book.cell(&sheet, "J5").unwrap().number, Some(130.));
    assert_eq!(
        book.workbook().sheets[0].tables[0].column_properties[2]
            .totals_function
            .as_deref(),
        Some("custom")
    );
}
