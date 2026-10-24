use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{
    HeadlessWorkbook, PivotCreate, PivotDestination, PivotSource, PivotUpdate,
};
use jade_xlsx_reader::{read_xlsx, CellRange, CellRef, CellValue, OpcPackage};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-chart-compat.xlsx");
fn at(text: &str) -> CellRef {
    CellRef::parse(text).unwrap()
}
fn options() -> PivotCreate {
    PivotCreate {
        name: None,
        source: PivotSource::Range {
            sheet: "Data".into(),
            range: CellRange::parse("A1:C5").unwrap(),
        },
        destination: PivotDestination::Existing {
            sheet: "Report".into(),
            origin: at("K24"),
        },
        settings: PivotUpdate::default(),
    }
}
fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open_for_editing(FIXTURE).unwrap()
}
fn value(bytes: &[u8], sheet: &str, reference: &str) -> CellValue {
    let b = read_xlsx(bytes).unwrap();
    b.sheets
        .iter()
        .find(|s| s.name == sheet)
        .unwrap()
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .find(|c| c.reference == at(reference))
        .map(|c| c.value.clone())
        .unwrap_or(CellValue::Empty)
}
fn transform(edit: impl FnOnce(&mut OpcPackage)) -> Vec<u8> {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    edit(&mut p);
    p.write().unwrap()
}
#[test]
fn creating_a_real_report_adds_complete_parts_values_queries_and_one_history_edit() {
    let mut b = open();
    let before = b.bytes().to_vec();
    let result = b.create_pivot(&options()).unwrap();
    assert_eq!(result.name, "PivotTable1");
    assert_eq!(result.source_rows, 4);
    let parsed = read_xlsx(b.bytes()).unwrap();
    assert!(parsed.pivots.issues.is_empty());
    assert_eq!(parsed.pivots.tables.len(), 2);
    assert_eq!(parsed.pivots.caches.len(), 2);
    assert_eq!(value(b.bytes(), "Report", "L25"), CellValue::Number(15.));
    assert_eq!(value(b.bytes(), "Report", "L26"), CellValue::Number(27.));
    assert_eq!(value(b.bytes(), "Report", "L27"), CellValue::Number(42.));
    let after = b.bytes().to_vec();
    let old = OpcPackage::open(&before).unwrap();
    let new = OpcPackage::open(&after).unwrap();
    assert_eq!(
        old.part("xl/pivotTables/report.xml"),
        new.part("xl/pivotTables/report.xml")
    );
    assert_eq!(
        old.part("xl/charts/pivot.xml"),
        new.part("xl/charts/pivot.xml")
    );
    let mut reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
    reopened
        .set(
            "Report",
            "N30",
            CellInput::Formula(r#"GETPIVOTDATA("Amount",K24)"#.into()),
        )
        .unwrap();
    assert_eq!(
        value(reopened.bytes(), "Report", "N30"),
        CellValue::Number(42.)
    );
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}
#[test]
fn new_destination_sheet_and_report_are_created_and_removed_by_one_undo() {
    let mut b = open();
    let before = b.bytes().to_vec();
    let mut o = options();
    o.destination = PivotDestination::NewSheet {
        name: "Summary_x0041_".into(),
        after: Some("Data".into()),
        origin: at("A3"),
    };
    let created = b.create_pivot(&o).unwrap();
    assert_eq!(created.sheet, "Summary_x0041_");
    let parsed = read_xlsx(b.bytes()).unwrap();
    assert_eq!(
        parsed
            .sheets
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Data", "Summary_x0041_", "Report"]
    );
    assert_eq!(
        value(b.bytes(), "Summary_x0041_", "B6"),
        CellValue::Number(42.)
    );
    let after = b.bytes().to_vec();
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}
#[test]
fn failed_creation_keeps_origin_cells_source_caches_and_the_redo_branch() {
    let mut b = open();
    b.set("Report", "K24", CellInput::Text("Keep origin".into()))
        .unwrap();
    b.set("Report", "Z1", CellInput::Text("Redo".into()))
        .unwrap();
    assert!(b.undo().unwrap());
    let before = b.bytes().to_vec();
    assert!(b.create_pivot(&options()).is_err());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(
        value(b.bytes(), "Report", "K24"),
        CellValue::Text("Keep origin".into())
    );
    let mut o = options();
    o.source = PivotSource::Range {
        sheet: "Data".into(),
        range: CellRange::parse("A1:D5").unwrap(),
    };
    let before = b.bytes().to_vec();
    assert!(b.create_pivot(&o).is_err());
    assert_eq!(b.bytes(), before);
}
#[test]
fn evaluated_formula_sources_are_used_for_creation_in_the_same_transaction() {
    let mut b = open();
    b.set("Data", "B2", CellInput::Formula("5*9".into()))
        .unwrap();
    let before = b.bytes().to_vec();
    b.create_pivot(&options()).unwrap();
    assert_eq!(value(b.bytes(), "Report", "L27"), CellValue::Number(77.));
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
}
#[test]
fn deleting_the_last_report_clears_only_owned_cells_and_freezes_its_chart() {
    let mut b = open();
    b.set("Report", "G10", CellInput::Text("Keep".into()))
        .unwrap();
    b.set(
        "Report",
        "N30",
        CellInput::Formula(r#"GETPIVOTDATA("Amount",E3)"#.into()),
    )
    .unwrap();
    assert_eq!(value(b.bytes(), "Report", "N30"), CellValue::Number(42.));
    let before = b.bytes().to_vec();
    let result = b.delete_pivot("Report", "RegionalSales").unwrap();
    assert!(result.removed_cache);
    assert_eq!(result.detached_charts, 1);
    let parsed = read_xlsx(b.bytes()).unwrap();
    assert!(parsed.pivots.issues.is_empty());
    assert!(parsed.pivots.tables.is_empty());
    assert_eq!(
        value(b.bytes(), "Report", "N30"),
        CellValue::Error("#REF!".into())
    );
    assert!(parsed.pivots.caches.is_empty());
    assert_eq!(value(b.bytes(), "Report", "F4"), CellValue::Empty);
    assert_eq!(
        value(b.bytes(), "Report", "G10"),
        CellValue::Text("Keep".into())
    );
    let chart = parsed.sheets[1].shapes[0].chart.as_ref().unwrap();
    assert!(chart.pivot_source.is_none());
    assert!(chart.series[0].values.formula.is_none());
    assert!(chart.series[0].categories.formula.is_none());
    assert_eq!(
        chart.series[0].values.points,
        [Some("15".into()), Some("27".into())]
    );
    assert_eq!(chart.series[0].color.rgb, Some([51, 102, 153]));
    let p = OpcPackage::open(b.bytes()).unwrap();
    assert!(p.part("xl/pivotTables/report.xml").is_none());
    assert!(p.part("xl/pivotCache/cache.xml").is_none());
    assert!(p.part("xl/pivotCache/records.xml").is_none());
    assert!(p
        .required_part_str("xl/charts/pivot.xml")
        .unwrap()
        .contains("j:caption=\"preserved\""));
    let after = b.bytes().to_vec();
    assert!(b.undo().unwrap());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
    assert_eq!(b.bytes(), after);
}
fn shared_fixture() -> Vec<u8> {
    transform(|p| {
        let xml = p
            .required_part_str("xl/pivotTables/report.xml")
            .unwrap()
            .replace("RegionalSales", "Other")
            .replace("E3:F6", "K24:L27");
        p.replace("xl/pivotTables/other.xml", xml.into_bytes());
        let rels = p
            .required_part_str("xl/pivotTables/_rels/report.xml.rels")
            .unwrap();
        p.replace("xl/pivotTables/_rels/other.xml.rels", rels.into_bytes());
        let body=p.required_part_str("xl/worksheets/_rels/sheet2.xml.rels").unwrap().replace("</Relationships>","<Relationship Id=\"other\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable\" Target=\"../pivotTables/other.xml\"/></Relationships>");
        p.replace("xl/worksheets/_rels/sheet2.xml.rels", body.into_bytes());
        let types=p.required_part_str("[Content_Types].xml").unwrap().replace("</Types>","<Override PartName=\"/xl/pivotTables/other.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml\"/></Types>");
        p.replace("[Content_Types].xml", types.into_bytes());
    })
}
#[test]
fn removing_one_shared_report_keeps_the_other_definition_cache_and_records_identical() {
    let bytes = shared_fixture();
    let original = OpcPackage::open(&bytes).unwrap();
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let result = b.delete_pivot("Report", "RegionalSales").unwrap();
    assert!(!result.removed_cache);
    let new = OpcPackage::open(b.bytes()).unwrap();
    for part in [
        "xl/pivotTables/other.xml",
        "xl/pivotCache/cache.xml",
        "xl/pivotCache/records.xml",
    ] {
        assert_eq!(original.part(part), new.part(part));
    }
    let parsed = read_xlsx(b.bytes()).unwrap();
    assert_eq!(parsed.pivots.tables.len(), 1);
    assert_eq!(parsed.pivots.tables[0].name, "Other");
}
#[test]
fn a_cache_referenced_by_another_package_part_is_retained_after_the_last_report() {
    let bytes = transform(|p| {
        let body=p.required_part_str("xl/_rels/workbook.xml.rels").unwrap().replace("</Relationships>","<Relationship Id=\"customCacheUser\" Type=\"urn:jade:test:cache-user\" Target=\"pivotCache/cache.xml\"/></Relationships>");
        p.replace("xl/_rels/workbook.xml.rels", body.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let result = b.delete_pivot("Report", "RegionalSales").unwrap();
    assert!(!result.removed_cache);
    let parsed = read_xlsx(b.bytes()).unwrap();
    assert_eq!(parsed.pivots.caches.len(), 1);
    assert!(parsed.pivots.tables.is_empty());
}
#[test]
fn an_opaque_chart_conversion_failure_preserves_the_report_bytes_and_redo() {
    let bytes = transform(|p| {
        let xml = p
            .required_part_str("xl/charts/pivot.xml")
            .unwrap()
            .replace("<c:numRef>", "<c:numRef j:keep=\"yes\">");
        p.replace("xl/charts/pivot.xml", xml.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.set("Report", "Z1", CellInput::Text("Redo".into()))
        .unwrap();
    assert!(b.undo().unwrap());
    let before = b.bytes().to_vec();
    assert!(b.delete_pivot("Report", "RegionalSales").is_err());
    assert_eq!(b.bytes(), before);
    assert!(b.redo().unwrap());
}
#[test]
fn workbook_structure_protection_refuses_new_sheet_creation_without_a_partial_report() {
    let bytes = transform(|p| {
        let xml = p.required_part_str("xl/workbook.xml").unwrap().replace(
            "<sheets>",
            "<workbookProtection lockStructure=\"1\"/><sheets>",
        );
        p.replace("xl/workbook.xml", xml.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = b.bytes().to_vec();
    let mut o = options();
    o.destination = PivotDestination::NewSheet {
        name: "Summary".into(),
        after: None,
        origin: at("A3"),
    };
    assert!(b.create_pivot(&o).is_err());
    assert_eq!(b.bytes(), before);
    assert_eq!(read_xlsx(b.bytes()).unwrap().sheets.len(), 2);
}

#[test]
fn deleting_one_of_two_independent_caches_keeps_a_valid_uncounted_declaration_list() {
    let mut b = open();
    let created = b.create_pivot(&options()).unwrap();
    assert!(
        b.delete_pivot("Report", &created.name)
            .unwrap()
            .removed_cache
    );
    let p = OpcPackage::open(b.bytes()).unwrap();
    let workbook = p.required_part_str("xl/workbook.xml").unwrap();
    assert!(workbook.contains("<pivotCaches>"));
    assert!(!workbook.contains("<pivotCaches count="));
    assert_eq!(read_xlsx(b.bytes()).unwrap().pivots.tables.len(), 1);
}
#[test]
fn deleting_a_legacy_report_removes_its_owned_marker_and_keeps_manual_styles() {
    let mut b = HeadlessWorkbook::open_for_editing(include_bytes!(
        "../../../public/fixtures/pivot-style-compat.xlsx"
    ))
    .unwrap();
    let before = read_xlsx(b.bytes()).unwrap();
    let style = before.sheets[1]
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .find(|c| c.reference == at("F4"))
        .unwrap()
        .style_index;
    b.delete_pivot("Report", "RegionalSales").unwrap();
    let after = read_xlsx(b.bytes()).unwrap();
    let cell = after.sheets[1]
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .find(|c| c.reference == at("F4"))
        .unwrap();
    assert_eq!(cell.style_index, style);
    assert_eq!(cell.value, CellValue::Empty);
    let p = OpcPackage::open(b.bytes()).unwrap();
    assert!(!p
        .required_part_str("xl/worksheets/sheet2.xml")
        .unwrap()
        .contains("pivotTableParts"));
}
#[test]
fn a_slicer_binding_refuses_removal_instead_of_leaving_a_dangling_reference() {
    let bytes = transform(|p| {
        p.replace("xl/slicerCaches/custom.xml",b"<slicerCacheDefinition xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><pivotTables><pivotTable tabId=\"2\" name=\"RegionalSales\"/></pivotTables></slicerCacheDefinition>".to_vec());
        let types=p.required_part_str("[Content_Types].xml").unwrap().replace("</Types>","<Override PartName=\"/xl/slicerCaches/custom.xml\" ContentType=\"application/vnd.ms-excel.slicerCache+xml\"/></Types>");
        p.replace("[Content_Types].xml", types.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = b.bytes().to_vec();
    assert!(b
        .delete_pivot("Report", "RegionalSales")
        .unwrap_err()
        .to_string()
        .contains("slicers/timelines"));
    assert_eq!(b.bytes(), before);
}
#[test]
fn report_protection_blocks_creation_and_deletion_without_losing_redo() {
    let bytes = transform(|p| {
        let body = p
            .required_part_str("xl/worksheets/sheet2.xml")
            .unwrap()
            .replace(
                "</sheetData>",
                "</sheetData><sheetProtection sheet=\"1\" pivotTables=\"1\"/>",
            );
        p.replace("xl/worksheets/sheet2.xml", body.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = b.bytes().to_vec();
    assert!(b.create_pivot(&options()).is_err());
    assert!(b.delete_pivot("Report", "RegionalSales").is_err());
    assert_eq!(b.bytes(), before);
}

#[test]
fn initial_member_filters_use_actual_new_cache_indices() {
    use jade_xlsx_headless_editor::PivotMemberFilter;
    let mut b = open();
    let mut o = options();
    o.settings.member_filters = vec![PivotMemberFilter {
        field: 0,
        visible: Some([0].into_iter().collect()),
    }];
    b.create_pivot(&o).unwrap();
    assert_eq!(value(b.bytes(), "Report", "L25"), CellValue::Number(15.));
    assert_eq!(value(b.bytes(), "Report", "L26"), CellValue::Number(15.));
    let parsed = read_xlsx(b.bytes()).unwrap();
    let t = parsed
        .pivots
        .tables
        .iter()
        .find(|t| t.name == "PivotTable1")
        .unwrap();
    let c = parsed
        .pivots
        .caches
        .iter()
        .find(|c| c.id == t.cache_id)
        .unwrap();
    assert_eq!(
        c.fields[0].shared_items[0].value,
        jade_xlsx_reader::pivot::PivotScalar::Text("East".into())
    );
}

#[test]
fn opaque_uri_aliases_do_not_let_cache_collection_delete_a_potential_dependency() {
    for target in [
        "PIVOTCACHE/CACHE.XML",
        "pivotCache/%63ache.xml",
        "pivotCache/cache.xml#payload",
    ] {
        let bytes = transform(|p| {
            let body=p.required_part_str("xl/_rels/workbook.xml.rels").unwrap().replace("</Relationships>",&format!("<Relationship Id=\"customCacheUser\" Type=\"urn:jade:test:cache-user\" Target=\"{target}\"/></Relationships>"));
            p.replace("xl/_rels/workbook.xml.rels", body.into_bytes());
        });
        let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
        assert!(
            !b.delete_pivot("Report", "RegionalSales")
                .unwrap()
                .removed_cache,
            "{target}"
        );
        assert_eq!(read_xlsx(b.bytes()).unwrap().pivots.caches.len(), 1);
    }
}
#[test]
fn creation_reserves_cache_ids_in_retained_markup_without_promoting_its_bindings() {
    let bytes = transform(|p| {
        let body=p.required_part_str("xl/workbook.xml").unwrap().replace("</workbook>","<j:payload xmlns:j=\"urn:jade:keep\"><pivotCache cacheId=\"0\"/></j:payload></workbook>");
        p.replace("xl/workbook.xml", body.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let summary = b.create_pivot(&options()).unwrap();
    assert_eq!(summary.cache_id, 1);
    let parsed = read_xlsx(b.bytes()).unwrap();
    assert_eq!(parsed.pivots.caches.len(), 2);
    let p = OpcPackage::open(b.bytes()).unwrap();
    assert!(p
        .required_part_str("xl/workbook.xml")
        .unwrap()
        .contains("<j:payload xmlns:j=\"urn:jade:keep\"><pivotCache cacheId=\"0\"/></j:payload>"));
}

#[test]
fn pivot_chart_format_deltas_are_not_silently_discarded_on_removal() {
    let bytes = transform(|p| {
        let xml=p.required_part_str("xl/charts/pivot.xml").unwrap().replace("<c:plotArea>","<c:pivotFmts><c:pivotFmt><c:idx val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"123456\"/></a:solidFill></c:spPr></c:pivotFmt></c:pivotFmts><c:plotArea>");
        p.replace("xl/charts/pivot.xml", xml.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = b.bytes().to_vec();
    assert!(b
        .delete_pivot("Report", "RegionalSales")
        .unwrap_err()
        .to_string()
        .contains("format deltas"));
    assert_eq!(b.bytes(), before);
}

#[test]
fn an_unrepresentable_cached_title_cannot_be_written_as_invalid_drawingml_text() {
    let bytes = transform(|p| {
        let mut xml = p.required_part_str("xl/charts/pivot.xml").unwrap();
        let start = xml.find("<c:title>").unwrap();
        let end = xml.find("</c:title>").unwrap() + "</c:title>".len();
        xml.replace_range(start..end,"<c:title><c:tx><c:strRef><c:f>'Report'!$F$3</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>Title_x0001_</c:v></c:pt></c:strCache></c:strRef></c:tx></c:title>");
        p.replace("xl/charts/pivot.xml", xml.into_bytes());
        let sheet = p
            .required_part_str("xl/worksheets/sheet2.xml")
            .unwrap()
            .replace("<t>Total Amount</t>", "<t>Title_x0001_</t>");
        p.replace("xl/worksheets/sheet2.xml", sheet.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = b.bytes().to_vec();
    assert!(b.delete_pivot("Report", "RegionalSales").is_err());
    assert_eq!(b.bytes(), before);
}

#[test]
fn detached_titles_keep_carriage_returns_literal_escape_text_and_unicode() {
    let bytes = transform(|p| {
        let mut xml = p.required_part_str("xl/charts/pivot.xml").unwrap();
        let start = xml.find("<c:title>").unwrap();
        let end = xml.find("</c:title>").unwrap() + "</c:title>".len();
        xml.replace_range(start..end,"<c:title><c:tx><c:strRef><c:f>'Report'!$F$3</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>Title_x000D__x000A__x005F_x0041_𠮷</c:v></c:pt></c:strCache></c:strRef></c:tx></c:title>");
        p.replace("xl/charts/pivot.xml", xml.into_bytes());
        let sheet = p
            .required_part_str("xl/worksheets/sheet2.xml")
            .unwrap()
            .replace(
                "<t>Total Amount</t>",
                "<t>Title_x000D__x000A__x005F_x0041_𠮷</t>",
            );
        p.replace("xl/worksheets/sheet2.xml", sheet.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.delete_pivot("Report", "RegionalSales").unwrap();
    let parsed = read_xlsx(b.bytes()).unwrap();
    assert_eq!(
        parsed.sheets[1].shapes[0].chart.as_ref().unwrap().title,
        "Title\r\n_x0041_𠮷"
    );
}

#[test]
fn stale_or_empty_chart_caches_freeze_the_current_displayed_worksheet_points() {
    for mode in 0..4 {
        let bytes = transform(|p| {
            let mut xml = p
                .required_part_str("xl/charts/pivot.xml")
                .unwrap()
                .replace("<c:v>15</c:v>", "<c:v>999</c:v>")
                .replace("<c:v>27</c:v>", "<c:v>888</c:v>")
                .replace("<c:v>East</c:v>", "<c:v>Old east</c:v>");
            let start = xml.find("<c:title>").unwrap();
            let end = xml.find("</c:title>").unwrap() + "</c:title>".len();
            xml.replace_range(start..end,"<c:title><c:tx><c:strRef><c:f>'Report'!$F$3</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>Title_x0001_</c:v></c:pt></c:strCache></c:strRef></c:tx></c:title>");
            if mode >= 2 {
                let start = xml.find("<c:numCache>").unwrap();
                let end = xml.find("</c:numCache>").unwrap() + "</c:numCache>".len();
                xml.replace_range(start..end,if mode==2{"<c:numCache><c:formatCode>0.00</c:formatCode><c:ptCount val=\"0\"/></c:numCache>"}else{""});
            }
            p.replace("xl/charts/pivot.xml", xml.into_bytes());
            if mode == 1 {
                let sheet = p
                    .required_part_str("xl/worksheets/sheet2.xml")
                    .unwrap()
                    .replace(
                        "<c r=\"F4\"><v>15</v></c>",
                        "<c r=\"F4\"><f>1000+1000</f><v>15</v></c>",
                    );
                p.replace("xl/worksheets/sheet2.xml", sheet.into_bytes());
            }
        });
        let original = read_xlsx(&bytes).unwrap();
        let old = &original.sheets[1].shapes[0].chart.as_ref().unwrap().series[0];
        if mode < 2 {
            assert_eq!(old.values.points[0].as_deref(), Some("999"));
        } else {
            assert!(old.values.points.is_empty());
        }
        // The byte writer takes a snapshot of the displayed saved cells, not
        // the stale chart cache or a falsely claimed verified calculation.
        let result =
            jade_xlsx_writer::pivot_refresh::delete_pivot(&bytes, "Report", "RegionalSales")
                .unwrap();
        let parsed = read_xlsx(&result.bytes).unwrap();
        let chart = parsed.sheets[1].shapes[0].chart.as_ref().unwrap();
        let series = &chart.series[0];
        assert_eq!(series.values.points, [Some("15".into()), Some("27".into())]);
        assert_eq!(
            series.categories.points,
            [Some("East".into()), Some("West".into())]
        );
        assert_eq!(chart.title, "Total Amount");
    }
}
#[test]
fn numeric_category_snapshots_use_the_format_currently_shown_by_the_source_cells() {
    let bytes = transform(|p| {
        let sheet = p
            .required_part_str("xl/worksheets/sheet2.xml")
            .unwrap()
            .replace(
                "<c r=\"E4\" t=\"inlineStr\"><is><t>East</t></is></c>",
                "<c r=\"E4\"><v>100</v></c>",
            )
            .replace(
                "<c r=\"E5\" t=\"inlineStr\"><is><t>West</t></is></c>",
                "<c r=\"E5\"><v>200</v></c>",
            );
        p.replace("xl/worksheets/sheet2.xml", sheet.into_bytes());
        let mut chart = p.required_part_str("xl/charts/pivot.xml").unwrap();
        let start = chart.find("<c:cat>").unwrap();
        let end = chart.find("</c:cat>").unwrap() + "</c:cat>".len();
        chart.replace_range(start..end,"<c:cat><c:numRef><c:f>'Report'!$E$4:$E$5</c:f><c:numCache><c:formatCode>0.00</c:formatCode><c:ptCount val=\"2\"/><c:pt idx=\"0\"><c:v>100</c:v></c:pt><c:pt idx=\"1\"><c:v>200</c:v></c:pt></c:numCache></c:numRef></c:cat>");
        p.replace("xl/charts/pivot.xml", chart.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    b.delete_pivot("Report", "RegionalSales").unwrap();
    let parsed = read_xlsx(b.bytes()).unwrap();
    let cat = &parsed.sheets[1].shapes[0].chart.as_ref().unwrap().series[0].categories;
    assert!(cat.numeric);
    assert_eq!(cat.points, [Some("100".into()), Some("200".into())]);
    assert_eq!(cat.format_code.as_deref(), Some("General"));
}
#[test]
fn hidden_visible_only_chart_points_refuse_until_static_indices_can_be_reconciled() {
    let bytes = transform(|p| {
        let body = p
            .required_part_str("xl/worksheets/sheet2.xml")
            .unwrap()
            .replace("<row r=\"4\">", "<row r=\"4\" hidden=\"1\">");
        p.replace("xl/worksheets/sheet2.xml", body.into_bytes());
    });
    let mut b = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = b.bytes().to_vec();
    assert!(b
        .delete_pivot("Report", "RegionalSales")
        .unwrap_err()
        .to_string()
        .contains("hidden chart points"));
    assert_eq!(b.bytes(), before);
}
