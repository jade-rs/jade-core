use jade_xlsx_reader::{sheet::parse_sheet, CellRef, CellValue};
use jade_xlsx_writer::{patch_sheet, CellEdit, NewValue};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
fn sheet(body: &str) -> String {
    format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:mc="{MC}" xmlns:u="urn:foreign">{body}</s:worksheet>"#
    )
}
fn edit(xml: &str) -> String {
    patch_sheet(
        xml,
        &[CellEdit {
            reference: CellRef::new(0, 0),
            value: NewValue::Number(8.0),
            style_index: None,
        }],
    )
    .unwrap()
}
fn number(xml: &str) -> CellValue {
    let s = parse_sheet("Sheet", "sheet.xml", xml, &[]).unwrap();
    s.rows
        .iter()
        .flat_map(|r| &r.cells)
        .find(|c| c.reference == CellRef::new(0, 0))
        .unwrap()
        .value
        .clone()
}

#[test]
fn editing_a_real_cell_does_not_consume_the_edit_in_a_foreign_grid_or_cell() {
    let foreign_grid = r#"<u:sheetData><u:row r="2"><u:c r="A2"><u:v>200</u:v></u:c></u:row><u:row r="1"><u:c r="A1"><u:v>100</u:v></u:c></u:row></u:sheetData>"#;
    let foreign_cell = r#"<u:c r="A1"><u:v>900</u:v></u:c>"#;
    let xml = sheet(&format!(
        r#"{foreign_grid}<s:sheetData><s:row r="1">{foreign_cell}<s:c r="A1" u:r="Z99"><s:v>2</s:v></s:c></s:row></s:sheetData>"#
    ));
    let out = edit(&xml);
    assert_eq!(number(&out), CellValue::Number(8.0));
    assert!(out.contains(foreign_grid));
    assert!(out.contains(foreign_cell));
}

#[test]
fn editing_the_selected_cell_updates_known_fallbacks_and_keeps_unknown_choices_opaque() {
    let inactive = r#"<mc:Choice Requires="u"><s:row r="1"><s:c r="A1"><s:v>900</s:v></s:c></s:row></mc:Choice>"#;
    let fallback = r#"<mc:Fallback><s:c r="A1"><s:v>700</s:v></s:c></mc:Fallback>"#;
    let xml = sheet(&format!(
        r#"<s:sheetData><mc:AlternateContent>{inactive}<mc:Fallback><s:row r="1"><mc:AlternateContent><mc:Choice Requires="s"><s:c r="A1"><s:v>2</s:v></s:c></mc:Choice>{fallback}</mc:AlternateContent></s:row></mc:Fallback></mc:AlternateContent></s:sheetData>"#
    ));
    let out = edit(&xml);
    assert_eq!(number(&out), CellValue::Number(8.0));
    assert!(out.contains(inactive));
    assert!(out.contains(r#"<mc:Fallback><s:c r="A1"><s:v>8</s:v></s:c></mc:Fallback>"#));
}

#[test]
fn replacing_a_cell_keeps_its_local_namespace_declarations() {
    for root_ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        let xml = sheet(&format!(r#"<s:sheetData><s:row r="1"><q:c xmlns:q="{root_ns}" r="A1"><q:v>2</q:v></q:c></s:row></s:sheetData>"#)).replace(MAIN, root_ns);
        let out = edit(&xml);
        assert_eq!(number(&out), CellValue::Number(8.0));
        assert!(out.contains(&format!(r#"xmlns:q="{root_ns}""#)));
    }
}

#[test]
fn a_foreign_worksheet_namespace_is_not_an_editable_grid() {
    let xml = r#"<x:worksheet xmlns:x="urn:sheet"><x:sheetData/></x:worksheet>"#;
    assert!(patch_sheet(
        xml,
        &[CellEdit {
            reference: CellRef::new(0, 0),
            value: NewValue::Number(8.0),
            style_index: None
        }]
    )
    .is_err());
}

#[test]
fn known_alternate_cells_share_implicit_coordinates_without_consuming_the_next_cell() {
    let xml = sheet(
        r#"<s:sheetData><s:row><mc:AlternateContent><mc:Choice Requires="s"><s:c><s:v>1</s:v></s:c></mc:Choice><mc:Fallback><s:c><s:v>1</s:v></s:c></mc:Fallback></mc:AlternateContent><s:c><s:v>2</s:v></s:c></s:row></s:sheetData>"#,
    );
    let out = edit(&xml);
    assert_eq!(number(&out), CellValue::Number(8.0));
    assert_eq!(
        number(&out.replace("Requires=\"s\"", "Requires=\"u\"")),
        CellValue::Number(8.0)
    );
    let parsed = parse_sheet("Sheet", "sheet.xml", &out, &[]).unwrap();
    assert_eq!(parsed.rows[0].cells[1].reference, CellRef::new(1, 0));
    assert_eq!(parsed.rows[0].cells[1].value, CellValue::Number(2.0));
}

#[test]
fn detaching_shared_formulas_does_not_rewrite_foreign_formula_lookalikes() {
    let foreign =
        r#"<u:wrapper><s:c r="A1"><s:f t="shared" si="0">100+200</s:f></s:c></u:wrapper>"#;
    let xml = sheet(&format!(
        r#"<s:sheetData><s:row r="1">{foreign}<s:c r="A1"><s:f t="shared" si="0" ref="A1:B1">1+1</s:f><s:v>2</s:v></s:c><s:c r="B1"><s:f t="shared" si="0"/><s:v>2</s:v></s:c></s:row></s:sheetData>"#
    ));
    let out = edit(&xml);
    assert_eq!(number(&out), CellValue::Number(8.0));
    assert!(out.contains(foreign));
    let parsed = parse_sheet("Sheet", "sheet.xml", &out, &[]).unwrap();
    assert_eq!(parsed.rows[0].cells[1].formula.as_deref(), Some("1+1"));
}

#[test]
fn cache_refresh_targets_the_selected_formula_and_keeps_foreign_type_attributes() {
    let foreign = r#"<u:wrapper><s:c r="A1"><s:f>100+200</s:f><s:v>300</s:v></s:c></u:wrapper>"#;
    let inactive = r#"<mc:Fallback><s:f>SUM(1,2)</s:f><s:v>90</s:v></mc:Fallback>"#;
    let xml = sheet(&format!(
        r#"<s:sheetData><s:row r="1">{foreign}<s:c r="A1" t="str" u:t="opaque"><mc:AlternateContent><mc:Choice Requires="s"><s:f>1+2</s:f><s:v>old</s:v></mc:Choice>{inactive}</mc:AlternateContent></s:c></s:row></s:sheetData>"#
    ));
    let out = jade_xlsx_writer::refresh_sheet(
        &xml,
        &[jade_xlsx_writer::CachedValueEdit {
            reference: CellRef::new(0, 0),
            value: jade_xlsx_writer::CachedValue::Number(3.0),
        }],
    )
    .unwrap();
    assert_eq!(number(&out), CellValue::Number(3.0));
    assert!(out.contains(foreign));
    assert!(out.contains("<s:f>SUM(1,2)</s:f>"));
    assert_eq!(out.matches("<s:v>3</s:v>").count(), 2);
    let fallback_view = out.replace("Requires=\"s\"", "Requires=\"u\"");
    assert_eq!(number(&fallback_view), CellValue::Number(3.0));
    assert!(out.contains(r#"u:t="opaque""#));
}

#[test]
fn cache_refresh_updates_each_known_cell_representation_and_keeps_attribute_values() {
    let xml = sheet(
        r#"<s:sheetData><s:row><mc:AlternateContent><mc:Choice Requires="s"><s:c t="str" u:note='say "yes" &amp; keep'><s:f>1+2</s:f><s:v>old</s:v></s:c></mc:Choice><mc:Fallback><s:c t="str"><s:f>SUM(1,2)</s:f><s:v>old</s:v></s:c></mc:Fallback></mc:AlternateContent><s:c><s:v>2</s:v></s:c></s:row></s:sheetData>"#,
    );
    let out = jade_xlsx_writer::refresh_sheet(
        &xml,
        &[jade_xlsx_writer::CachedValueEdit {
            reference: CellRef::new(0, 0),
            value: jade_xlsx_writer::CachedValue::Number(3.0),
        }],
    )
    .unwrap();
    assert_eq!(number(&out), CellValue::Number(3.0));
    assert_eq!(
        number(&out.replace("Requires=\"s\"", "Requires=\"u\"")),
        CellValue::Number(3.0)
    );
    assert!(out.contains(r#"u:note="say &quot;yes&quot; &amp; keep""#));
    let parsed = parse_sheet("Sheet", "sheet.xml", &out, &[]).unwrap();
    assert_eq!(parsed.rows[0].cells[1].reference, CellRef::new(1, 0));
    assert_eq!(parsed.rows[0].cells[1].value, CellValue::Number(2.0));
}

#[test]
fn an_inactive_formula_does_not_turn_the_selected_stored_value_into_a_cache() {
    let xml = sheet(
        r#"<s:sheetData><s:row><s:c><mc:AlternateContent><mc:Choice Requires="s"><s:v>2</s:v></mc:Choice><mc:Fallback><s:f>1+1</s:f><s:v>2</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></s:sheetData>"#,
    );
    let out = jade_xlsx_writer::refresh_sheet(
        &xml,
        &[jade_xlsx_writer::CachedValueEdit {
            reference: CellRef::new(0, 0),
            value: jade_xlsx_writer::CachedValue::Number(3.0),
        }],
    )
    .unwrap();
    assert_eq!(out, xml);
}

#[test]
fn refreshing_known_sources_does_not_add_a_second_cache_to_unknown_representations() {
    let unknown = r#"<mc:Choice Requires="u"><s:f>SUM(1,2)</s:f><s:v>90</s:v></mc:Choice>"#;
    for original_value in ["<s:v>0</s:v>", ""] {
        let xml = sheet(&format!(
            r#"<s:sheetData><s:row r="1"><s:c r="A1"><mc:AlternateContent>{unknown}<mc:Choice Requires="s"><s:f>1+2</s:f>{original_value}</mc:Choice><mc:Fallback><s:f>SUM(1,2)</s:f><s:v>0</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></s:sheetData>"#
        ));
        let out = jade_xlsx_writer::refresh_sheet(
            &xml,
            &[jade_xlsx_writer::CachedValueEdit {
                reference: CellRef::new(0, 0),
                value: jade_xlsx_writer::CachedValue::Number(3.0),
            }],
        )
        .unwrap();
        assert_eq!(number(&out), CellValue::Number(3.0));
        assert!(out.contains(unknown));
        let future = out.replace("xmlns:u=\"urn:foreign\"", &format!("xmlns:u=\"{MAIN}\""));
        let owned = jade_xlsx_reader::cell_content::worksheet_source(&future).unwrap();
        let mut reader = quick_xml::Reader::from_str(&future);
        let mut count = 0;
        loop {
            let start = reader.buffer_position() as usize;
            match reader.read_event().unwrap() {
                quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e)
                    if e.local_name().as_ref() == b"v" && owned.owns_start(start) =>
                {
                    count += 1
                }
                quick_xml::events::Event::Eof => break,
                _ => {}
            }
        }
        assert_eq!(count, 1, "{future}");
        assert_eq!(number(&future), CellValue::Number(90.0));
    }
}

#[test]
fn a_missing_cache_uses_the_selected_empty_or_nested_value_branch() {
    let unknown = r#"<mc:Choice Requires="u"><s:v>90</s:v></mc:Choice>"#;
    let branches = [
        r#"<mc:Choice Requires="s"/><mc:Fallback><s:v>0</s:v></mc:Fallback>"#,
        r#"<mc:Choice Requires="s"><mc:AlternateContent><mc:Choice Requires="s"/><mc:Fallback><s:v>0</s:v></mc:Fallback></mc:AlternateContent></mc:Choice><mc:Fallback><s:v>0</s:v></mc:Fallback>"#,
    ];
    for branch in branches {
        let xml = sheet(&format!(
            r#"<s:sheetData><s:row r="1"><s:c r="A1"><s:f>1+2</s:f><mc:AlternateContent>{unknown}{branch}</mc:AlternateContent></s:c></s:row></s:sheetData>"#
        ));
        let out = jade_xlsx_writer::refresh_sheet(
            &xml,
            &[jade_xlsx_writer::CachedValueEdit {
                reference: CellRef::new(0, 0),
                value: jade_xlsx_writer::CachedValue::Number(3.0),
            }],
        )
        .unwrap();
        assert!(out.contains(unknown));
        let fallback = out.replace("Requires=\"s\"", "Requires=\"missing\"");
        let future = out.replace("xmlns:u=\"urn:foreign\"", &format!("xmlns:u=\"{MAIN}\""));
        for (view, expected) in [(&out, 3.0), (&fallback, 3.0), (&future, 90.0)] {
            assert_eq!(number(view), CellValue::Number(expected));
            let owned = jade_xlsx_reader::cell_content::worksheet_source(view).unwrap();
            let mut reader = quick_xml::Reader::from_str(view);
            let mut count = 0;
            loop {
                let start = reader.buffer_position() as usize;
                match reader.read_event().unwrap() {
                    quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e)
                        if e.local_name().as_ref() == b"v" && owned.owns_start(start) =>
                    {
                        count += 1
                    }
                    quick_xml::events::Event::Eof => break,
                    _ => {}
                }
            }
            assert_eq!(count, 1, "{view}");
        }
    }
}

#[test]
fn caches_keep_the_main_namespace_when_branch_prefixes_are_rebound() {
    for main in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        for value in ["<q:v>0</q:v>", ""] {
            let body = format!(
                r#"<s:sheetData><s:row r="1"><s:c r="A1"><mc:AlternateContent><mc:Choice xmlns:q="{main}" xmlns:s="urn:foreign" Requires="q"><q:f>1+2</q:f>{value}</mc:Choice><mc:Fallback><s:f>SUM(1,2)</s:f><s:v>0</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></s:sheetData>"#
            );
            let xml = sheet(&body).replace(MAIN, main);
            let out = jade_xlsx_writer::refresh_sheet(
                &xml,
                &[jade_xlsx_writer::CachedValueEdit {
                    reference: CellRef::new(0, 0),
                    value: jade_xlsx_writer::CachedValue::Number(3.0),
                }],
            )
            .unwrap();
            assert_eq!(number(&out), CellValue::Number(3.0), "{out}");
        }
        for value in [format!(r#"<v xmlns="{main}">0</v>"#), String::new()] {
            let body = format!(
                r#"<s:sheetData><s:row r="1"><s:c r="A1"><mc:AlternateContent><mc:Choice xmlns:q="{main}" xmlns:s="urn:foreign" xmlns="urn:other" Requires="q"><f xmlns="{main}">1+2</f>{value}</mc:Choice><mc:Fallback><s:f>SUM(1,2)</s:f><s:v>0</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></s:sheetData>"#
            );
            let xml = sheet(&body).replace(MAIN, main);
            let out = jade_xlsx_writer::refresh_sheet(
                &xml,
                &[jade_xlsx_writer::CachedValueEdit {
                    reference: CellRef::new(0, 0),
                    value: jade_xlsx_writer::CachedValue::Number(3.0),
                }],
            )
            .unwrap();
            assert_eq!(number(&out), CellValue::Number(3.0), "{out}");
        }
        let body = format!(
            r#"<s:sheetData><s:row r="1"><s:c r="A1"><s:f>1+2</s:f><mc:AlternateContent><mc:Choice xmlns:q="{main}" xmlns:s="urn:foreign" Requires="q"/><mc:Fallback><s:v>0</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></s:sheetData>"#
        );
        let xml = sheet(&body).replace(MAIN, main);
        let out = jade_xlsx_writer::refresh_sheet(
            &xml,
            &[jade_xlsx_writer::CachedValueEdit {
                reference: CellRef::new(0, 0),
                value: jade_xlsx_writer::CachedValue::Number(3.0),
            }],
        )
        .unwrap();
        assert_eq!(number(&out), CellValue::Number(3.0), "{out}");
    }
}
