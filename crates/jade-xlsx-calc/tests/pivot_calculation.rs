use jade_xlsx_calc::evaluate;
use jade_xlsx_reader::{read_xlsx, CellRef, CellValue, DefinedName, Workbook};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-cache-compat.xlsx");
fn book() -> Workbook {
    read_xlsx(FIXTURE).unwrap()
}
fn at(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}
fn formula(b: &mut Workbook, target: &str, text: &str) {
    let reference = at(target);
    let source = b.sheets[1]
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .find(|c| c.reference == at("F6"))
        .unwrap()
        .clone();
    let mut cell = source;
    cell.reference = reference;
    cell.formula = Some(text.into());
    cell.value = CellValue::Number(999.);
    let row = b.sheets[1]
        .rows
        .iter_mut()
        .find(|r| r.index == reference.row)
        .unwrap();
    row.cells.push(cell);
    row.cells.sort_by_key(|c| c.reference.column);
}
fn result(b: &Workbook, target: &str) -> CellValue {
    let answer = evaluate(b).unwrap();
    assert!(answer.skipped.is_empty(), "{:?}", answer.skipped);
    answer
        .cells
        .iter()
        .find(|c| c.sheet == 1 && c.reference == at(target))
        .unwrap()
        .value
        .clone()
}
#[test]
fn actual_visible_summary_cells_replace_stale_formula_caches() {
    let mut b = book();
    for (target, f, expected) in [
        ("H6", r#"GETPIVOTDATA("Amount",E3)"#, 42.),
        (
            "I6",
            r#"GETPIVOTDATA("Total Amount",E3,"Region","East")"#,
            15.,
        ),
        ("J6", r#"GETPIVOTDATA("Amount",E3,"Territory","West")"#, 27.),
    ] {
        formula(&mut b, target, f);
        assert_eq!(result(&b, target), CellValue::Number(expected));
    }
    // The source/cache detail is deliberately different: the visible report
    // is the query authority, so source aggregation would give the wrong result.
    b.sheets[0]
        .rows
        .iter_mut()
        .find(|r| r.index == 1)
        .unwrap()
        .cells
        .iter_mut()
        .find(|c| c.reference == at("B2"))
        .unwrap()
        .value = CellValue::Number(700.);
    assert_eq!(result(&b, "H6"), CellValue::Number(42.));
}
#[test]
fn invalid_and_not_visible_queries_return_real_ref_errors() {
    for f in [
        r#"GETPIVOTDATA("Absent",E3)"#,
        r#"GETPIVOTDATA("Amount",A1)"#,
        r#"GETPIVOTDATA("Amount",42)"#,
        r#"GETPIVOTDATA("Amount",E3,"Region","Historical")"#,
        r#"GETPIVOTDATA("Amount",E3,"Missing field","East")"#,
        r#"GETPIVOTDATA("Amount",E3,"When",DATE(2026,10,24))"#,
        r#"GETPIVOTDATA("Amount",E3,"Region","East","Territory","West")"#,
    ] {
        let mut b = book();
        formula(&mut b, "H6", f);
        assert_eq!(result(&b, "H6"), CellValue::Error("#REF!".into()), "{f}");
    }
}
#[test]
fn referenced_data_fields_items_and_named_pivot_ranges_remain_references_or_values_as_required() {
    let mut b = book();
    formula(&mut b, "P6", r#""Total Amount""#);
    formula(&mut b, "Q6", r#""East""#);
    b.defined_names.push(DefinedName {
        name: "PivotAnchor".into(),
        formula: "'Report'!$E$3".into(),
        local_sheet_index: None,
        hidden: false,
    });
    formula(&mut b, "H6", "GETPIVOTDATA(P6,E3,\"Region\",Q6)");
    assert_eq!(
        result(&b, "H6"),
        CellValue::Number(15.),
        "referenced field/item with literal pivot anchor"
    );
    formula(
        &mut b,
        "I6",
        r#"GETPIVOTDATA("Amount",PivotAnchor,"Region","East")"#,
    );
    assert_eq!(
        result(&b, "I6"),
        CellValue::Number(15.),
        "named pivot anchor"
    );
    formula(&mut b, "J6", "GETPIVOTDATA(P6,PivotAnchor,\"Region\",Q6)");
    assert_eq!(result(&b, "J6"), CellValue::Number(15.));
}
#[test]
fn array_item_criteria_spill_real_visible_values() {
    let mut b = book();
    formula(
        &mut b,
        "H6",
        r#"GETPIVOTDATA("Amount",E3,"Region",{"East","West"})"#,
    );
    let answer = evaluate(&b).unwrap();
    assert!(answer.skipped.is_empty(), "{:?}", answer.skipped);
    assert_eq!(
        answer
            .cells
            .iter()
            .find(|c| c.reference == at("H6"))
            .unwrap()
            .value,
        CellValue::Number(15.)
    );
    let spill = answer.spills.iter().find(|s| s.anchor == at("H6")).unwrap();
    assert_eq!(spill.cells[0].reference, at("I6"));
    assert_eq!(spill.cells[0].value, CellValue::Number(27.));
}
#[test]
fn selected_report_filter_dates_follow_both_workbook_epochs() {
    use std::sync::Arc;
    for epoch in [
        jade_xlsx_reader::DateSystem::Nineteen00,
        jade_xlsx_reader::DateSystem::Nineteen04,
    ] {
        let mut b = book();
        b.date_system = epoch;
        Arc::make_mut(&mut b.pivots.tables[0]).page_fields[0].item = Some(0);
        formula(
            &mut b,
            "H6",
            r#"GETPIVOTDATA("Amount",E3,"When",DATE(2026,10,24)+TIME(12,30,0))"#,
        );
        assert_eq!(result(&b, "H6"), CellValue::Number(42.), "{epoch:?}");
        formula(
            &mut b,
            "I6",
            r#"GETPIVOTDATA("Amount",E3,"When",DATE(2026,10,25)+TIME(12,30,0))"#,
        );
        assert_eq!(result(&b, "I6"), CellValue::Error("#REF!".into()));
    }
}
#[test]
fn visible_result_cells_preserve_text_boolean_errors_and_blank_value_semantics() {
    for (value, expected) in [
        (
            CellValue::Text("label".into()),
            CellValue::Text("label".into()),
        ),
        (CellValue::Bool(true), CellValue::Bool(true)),
        (
            CellValue::Error("#N/A".into()),
            CellValue::Error("#N/A".into()),
        ),
        (
            CellValue::Error("#NAME?".into()),
            CellValue::Error("#NAME?".into()),
        ),
        (CellValue::Empty, CellValue::Number(0.)),
    ] {
        let mut b = book();
        b.sheets[1]
            .rows
            .iter_mut()
            .flat_map(|r| &mut r.cells)
            .find(|c| c.reference == at("F4"))
            .unwrap()
            .value = value;
        formula(&mut b, "H6", r#"GETPIVOTDATA("Amount",E3,"Region","East")"#);
        assert_eq!(result(&b, "H6"), expected);
    }
}
#[test]
fn unavailable_metadata_holds_cached_results_but_absent_pivots_return_ref() {
    use std::sync::Arc;
    let mut b = book();
    Arc::make_mut(&mut b.pivots.tables[0]).row_items.clear();
    formula(&mut b, "H6", r#"GETPIVOTDATA("Amount",E3)"#);
    let answer = evaluate(&b).unwrap();
    assert_eq!(answer.skipped.len(), 1);
    assert_eq!(answer.skipped[0].reference, at("H6"));
    assert!(!answer.cells.iter().any(|c| c.reference == at("H6")));
    b.pivots.tables.clear();
    let answer = evaluate(&b).unwrap();
    assert!(answer.skipped.is_empty());
    assert_eq!(
        answer
            .cells
            .iter()
            .find(|c| c.reference == at("H6"))
            .unwrap()
            .value,
        CellValue::Error("#REF!".into())
    );
}
fn literal(b: &mut Workbook, target: &str, value: CellValue) {
    let reference = at(target);
    let template = b.sheets[1].rows[0].clone();
    if !b.sheets[1].rows.iter().any(|r| r.index == reference.row) {
        let mut row = template;
        row.index = reference.row;
        row.cells.clear();
        b.sheets[1].rows.push(row);
        b.sheets[1].rows.sort_by_key(|r| r.index);
    }
    let row = b.sheets[1]
        .rows
        .iter_mut()
        .find(|r| r.index == reference.row)
        .unwrap();
    if let Some(cell) = row.cells.iter_mut().find(|c| c.reference == reference) {
        cell.value = value;
        cell.formula = None;
    } else {
        let mut cell = row
            .cells
            .first()
            .cloned()
            .unwrap_or(jade_xlsx_reader::Cell {
                reference,
                value: CellValue::Empty,
                formula: None,
                style_index: None,
            });
        cell.reference = reference;
        cell.value = value;
        cell.formula = None;
        row.cells.push(cell);
        row.cells.sort_by_key(|c| c.reference.column);
    }
}
#[test]
fn exact_data_captions_take_precedence_over_another_measures_source_alias() {
    use std::sync::Arc;
    let mut b = book();
    let t = Arc::make_mut(&mut b.pivots.tables[0]);
    let mut count = t.data_fields[0].clone();
    count.name = Some("Count Amount".into());
    count.aggregation = "count".into();
    t.data_fields[0].name = Some("Amount".into());
    t.data_fields.push(count);
    t.location.reference = jade_xlsx_reader::CellRange::parse("E3:G6").unwrap();
    let mut column = t.column_items[0].clone();
    column.data_index = 1;
    t.column_items.push(column);
    for (target, value) in [("G4", 2.), ("G5", 2.), ("G6", 4.)] {
        literal(&mut b, target, CellValue::Number(value));
    }
    formula(&mut b, "H6", r#"GETPIVOTDATA("Count Amount",E3)"#);
    assert_eq!(result(&b, "H6"), CellValue::Number(4.));
    formula(&mut b, "I6", r#"GETPIVOTDATA("Amount",E3)"#);
    assert_eq!(result(&b, "I6"), CellValue::Number(42.));
    formula(
        &mut b,
        "J6",
        r#"GETPIVOTDATA("Count Amount",E3,"Region","East")"#,
    );
    assert_eq!(result(&b, "J6"), CellValue::Number(2.));
}
#[test]
fn compressed_row_prefixes_and_subtotals_keep_physical_result_coordinates() {
    use jade_xlsx_reader::pivot::AxisItem;
    use std::sync::Arc;
    let mut b = book();
    let t = Arc::make_mut(&mut b.pivots.tables[0]);
    t.row_fields = vec![0, 2];
    t.page_fields.clear();
    t.fields[2].axis = Some("axisRow".into());
    t.location.reference = jade_xlsx_reader::CellRange::parse("E3:F8").unwrap();
    t.row_items = vec![
        AxisItem {
            kind: "default".into(),
            repeat: 0,
            data_index: 0,
            items: vec![0],
        },
        AxisItem {
            kind: "data".into(),
            repeat: 1,
            data_index: 0,
            items: vec![0],
        },
        AxisItem {
            kind: "data".into(),
            repeat: 1,
            data_index: 0,
            items: vec![1],
        },
        AxisItem {
            kind: "data".into(),
            repeat: 0,
            data_index: 0,
            items: vec![1, 0],
        },
        AxisItem {
            kind: "grand".into(),
            repeat: 0,
            data_index: 0,
            items: vec![0],
        },
    ];
    for (target, value) in [
        ("F4", 15.),
        ("F5", 10.),
        ("F6", 5.),
        ("F7", 27.),
        ("F8", 42.),
    ] {
        literal(&mut b, target, CellValue::Number(value));
    }
    for (target, f, expected) in [
        ("H6", r#"GETPIVOTDATA("Amount",E3,"Region","East")"#, 15.),
        (
            "I6",
            r#"GETPIVOTDATA("Amount",E3,"When",DATE(2026,10,24)+TIME(12,30,0),"Region","East")"#,
            10.,
        ),
        (
            "J6",
            r#"GETPIVOTDATA("Amount",E3,"Region","East","When","(blank)")"#,
            5.,
        ),
        ("K6", r#"GETPIVOTDATA("Amount",E3)"#, 42.),
    ] {
        formula(&mut b, target, f);
        assert_eq!(result(&b, target), CellValue::Number(expected));
    }
}
#[test]
fn row_and_column_criteria_resolve_independently_and_use_visible_grand_totals() {
    use jade_xlsx_reader::pivot::AxisItem;
    use std::sync::Arc;
    let mut b = book();
    let t = Arc::make_mut(&mut b.pivots.tables[0]);
    t.column_fields = vec![2];
    t.page_fields.clear();
    t.fields[2].axis = Some("axisCol".into());
    t.location.reference = jade_xlsx_reader::CellRange::parse("E3:H6").unwrap();
    t.column_items = vec![
        AxisItem {
            kind: "data".into(),
            repeat: 0,
            data_index: 0,
            items: vec![0],
        },
        AxisItem {
            kind: "data".into(),
            repeat: 0,
            data_index: 0,
            items: vec![1],
        },
        AxisItem {
            kind: "grand".into(),
            repeat: 0,
            data_index: 0,
            items: vec![0],
        },
    ];
    for (target, value) in [
        ("F4", 10.),
        ("G4", 5.),
        ("H4", 15.),
        ("F5", 27.),
        ("G5", 0.),
        ("H5", 27.),
        ("F6", 37.),
        ("G6", 5.),
        ("H6", 42.),
    ] {
        literal(&mut b, target, CellValue::Number(value));
    }
    for (target, f, expected) in [
        (
            "J6",
            r#"GETPIVOTDATA("Amount",E3,"Region","East","When",DATE(2026,10,24)+TIME(12,30,0))"#,
            10.,
        ),
        ("K6", r#"GETPIVOTDATA("Amount",E3,"Region","East")"#, 15.),
        (
            "L6",
            r#"GETPIVOTDATA("Amount",E3,"When",DATE(2026,10,24)+TIME(12,30,0))"#,
            37.,
        ),
        ("M6", r#"GETPIVOTDATA("Amount",A1:Z20)"#, 42.),
    ] {
        formula(&mut b, target, f);
        assert_eq!(result(&b, target), CellValue::Number(expected));
    }
}

#[test]
fn report_filter_item_uses_cache_index_even_when_unused_values_leave_gaps() {
    use jade_xlsx_reader::pivot::{AxisItem, PageField};
    use std::sync::Arc;
    let mut b = book();
    let t = Arc::make_mut(&mut b.pivots.tables[0]);
    t.row_fields.clear();
    let mut page: PageField = t.page_fields[0].clone();
    page.field = 0;
    page.item = Some(2); // West is cache 2, but pivot field item 1.
    t.page_fields = vec![page];
    t.location.reference = jade_xlsx_reader::CellRange::parse("E3:F4").unwrap();
    t.row_items = vec![AxisItem {
        kind: "grand".into(),
        repeat: 0,
        data_index: 0,
        items: vec![],
    }];
    literal(&mut b, "F4", CellValue::Number(27.));
    for (target, f, expected) in [
        (
            "H6",
            r#"GETPIVOTDATA("Amount",E3,"Region","West")"#,
            CellValue::Number(27.),
        ),
        (
            "I6",
            r#"GETPIVOTDATA("Amount",E3,"Region","East")"#,
            CellValue::Error("#REF!".into()),
        ),
        ("J6", r#"GETPIVOTDATA("Amount",E3)"#, CellValue::Number(27.)),
    ] {
        formula(&mut b, target, f);
        assert_eq!(result(&b, target), expected);
    }
}
#[test]
fn mce_selected_workbook_cache_declarations_are_calculable_after_reading_the_package() {
    let mut package = jade_xlsx_reader::OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/workbook.xml";
    let xml = package.required_part_str(part).unwrap();
    let xml = xml.replace("<pivotCaches", "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Fallback><pivotCaches")
        .replace("</pivotCaches>", "</pivotCaches></mc:Fallback></mc:AlternateContent>");
    package.replace(part, xml.into_bytes());
    let mut b = read_xlsx(&package.write().unwrap()).unwrap();
    formula(&mut b, "H6", r#"GETPIVOTDATA("Amount",E3)"#);
    assert_eq!(result(&b, "H6"), CellValue::Number(42.));
}
#[test]
fn scalar_argument_errors_and_lambda_shadowing_keep_normal_formula_semantics() {
    for (f, expected) in [
        (r#"GETPIVOTDATA(NA(),42)"#, CellValue::Error("#N/A".into())),
        (
            r#"GETPIVOTDATA("Amount",E3,"Region",NA())"#,
            CellValue::Error("#N/A".into()),
        ),
        (
            r#"LET(_xlpm.GETPIVOTDATA,LAMBDA(x,y,x+y),_xlpm.GETPIVOTDATA(2,3))"#,
            CellValue::Number(5.),
        ),
        (
            r#"LAMBDA(x,GETPIVOTDATA("Amount",E3,"Region",x))("West")"#,
            CellValue::Number(27.),
        ),
        (
            r#"GETPIVOTDATA("Amount",E3,"Region")"#,
            CellValue::Error("#REF!".into()),
        ),
    ] {
        let mut b = book();
        formula(&mut b, "H6", f);
        assert_eq!(result(&b, "H6"), expected, "{f}");
    }
}
#[test]
fn array_dimensions_broadcast_and_incompatible_shapes_return_value() {
    let mut b = book();
    formula(
        &mut b,
        "H6",
        r#"GETPIVOTDATA({"Amount";"Total Amount"},E3,"Region",{"East","West"})"#,
    );
    let a = evaluate(&b).unwrap();
    assert!(a.skipped.is_empty(), "{:?}", a.skipped);
    let spill = a.spills.iter().find(|s| s.anchor == at("H6")).unwrap();
    for (reference, value) in [("I6", 27.), ("H7", 15.), ("I7", 27.)] {
        assert_eq!(
            spill
                .cells
                .iter()
                .find(|c| c.reference == at(reference))
                .unwrap()
                .value,
            CellValue::Number(value)
        );
    }
    let mut b = book();
    formula(
        &mut b,
        "H6",
        r#"GETPIVOTDATA({"Amount","Total Amount"},E3,"Region",{"East","West","East"})"#,
    );
    assert_eq!(result(&b, "H6"), CellValue::Error("#VALUE!".into()));
}

#[test]
fn ambiguous_table_ownership_preserves_imported_answers_instead_of_inventing_ref() {
    let mut package = jade_xlsx_reader::OpcPackage::open(FIXTURE).unwrap();
    package.replace("xl/worksheets/_rels/sheet1.xml.rels",br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="alsoPivot" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable" Target="../pivotTables/report.xml"/></Relationships>"#.to_vec());
    let mut b = read_xlsx(&package.write().unwrap()).unwrap();
    assert!(b.pivots.tables.is_empty());
    formula(&mut b, "H6", r#"GETPIVOTDATA("Amount",E3)"#);
    let answer = evaluate(&b).unwrap();
    assert_eq!(answer.skipped.len(), 1);
    assert!(!answer.cells.iter().any(|c| c.reference == at("H6")));
}

#[test]
fn uninterpreted_inner_alternatives_and_cube_hierarchies_hold_formula_caches() {
    for cube in [false, true] {
        let mut package = jade_xlsx_reader::OpcPackage::open(FIXTURE).unwrap();
        let part = "xl/pivotTables/report.xml";
        let mut xml = package.required_part_str(part).unwrap();
        if cube {
            xml = xml.replace(
                "</pivotTableDefinition>",
                "<pivotHierarchies count=\"0\"/></pivotTableDefinition>",
            );
        } else {
            xml = xml.replace("<pageFields", "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Fallback><pageFields")
                .replace("</pageFields>", "</pageFields></mc:Fallback></mc:AlternateContent>");
        }
        package.replace(part, xml.into_bytes());
        let mut b = read_xlsx(&package.write().unwrap()).unwrap();
        assert!(!b.pivots.tables[0].unsupported.is_empty());
        formula(
            &mut b,
            "H6",
            r#"GETPIVOTDATA("Amount",E3,"When",DATE(2026,10,24))"#,
        );
        let answer = evaluate(&b).unwrap();
        assert_eq!(answer.skipped.len(), 1, "cube={cube}: {:?}", answer.cells);
        assert!(!answer.cells.iter().any(|c| c.reference == at("H6")));
    }
}
#[test]
fn opaque_cache_extensions_keep_supported_visible_queries_calculable() {
    use std::sync::Arc;
    let mut b = book();
    Arc::make_mut(&mut b.pivots.caches[0])
        .unsupported
        .push("pivotCacheDefinition/extLst".into());
    formula(&mut b, "H6", r#"GETPIVOTDATA("Amount",E3)"#);
    assert_eq!(result(&b, "H6"), CellValue::Number(42.));
}
