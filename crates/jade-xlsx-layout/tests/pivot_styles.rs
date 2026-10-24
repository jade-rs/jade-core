use jade_xlsx_layout::{color::rgba, layout_workbook_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{read_xlsx, CellRef, OpcPackage};
const FIXTURE: &[u8] = include_bytes!("../../../public/fixtures/pivot-cache-compat.xlsx");
fn at(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}
fn custom() -> Vec<u8> {
    let mut p = OpcPackage::open(FIXTURE).unwrap();
    let part = "xl/styles.xml";
    let xml = p.required_part_str(part).unwrap().replace("</styleSheet>", r#"<dxfs count="3"><dxf><fill><patternFill patternType="solid"><fgColor rgb="FFFF0000"/></patternFill></fill></dxf><dxf><font><b/></font><fill><patternFill patternType="solid"><fgColor rgb="FF0000FF"/></patternFill></fill></dxf><dxf><fill><patternFill patternType="solid"><fgColor rgb="FF00FF00"/></patternFill></fill></dxf></dxfs><tableStyles count="1"><tableStyle name="OwnedPivot" table="0" pivot="1"><tableStyleElement type="wholeTable" dxfId="0"/><tableStyleElement type="headerRow" dxfId="1"/><tableStyleElement type="totalRow" dxfId="2"/></tableStyle></tableStyles></styleSheet>"#);
    p.replace(part, xml.into_bytes());
    let part = "xl/pivotTables/report.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace("PivotStyleMedium9", "OwnedPivot")
        .replace(
            "showRowHeaders=\"1\"",
            "showRowHeaders=\"1\" showColHeaders=\"1\"",
        );
    p.replace(part, xml.into_bytes());
    p.write().unwrap()
}
fn layout(bytes: &[u8]) -> SheetLayout {
    layout_workbook_sheet(&read_xlsx(bytes).unwrap(), 1, &LayoutOptions::default()).unwrap()
}
fn fill(l: &SheetLayout, cell: &str) -> Option<[u8; 4]> {
    let at = at(cell);
    let c = l.cell(at).cloned().or_else(|| {
        l.blank_cells_in(Rect {
            x: 0.,
            y: 0.,
            width: 2000.,
            height: 2000.,
        })
        .into_iter()
        .find(|c| c.reference == at)
    })?;
    c.fill.and_then(|f| f.foreground).and_then(rgba)
}
#[test]
fn custom_pivot_styles_apply_to_headers_values_and_grand_totals() {
    let l = layout(&custom());
    assert_eq!(fill(&l, "F3"), Some([0, 0, 255, 255]));
    assert_eq!(fill(&l, "F4"), Some([255, 0, 0, 255]));
    assert_eq!(fill(&l, "F6"), Some([0, 255, 0, 255]));
    assert!(l.cell(at("F3")).unwrap().font.bold);
}
#[test]
fn an_unstored_cell_in_a_pivot_report_keeps_its_style_surface() {
    let mut p = OpcPackage::open(&custom()).unwrap();
    let part = "xl/worksheets/sheet2.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace("<c r=\"F5\"><v>27</v></c>", "");
    p.replace(part, xml.into_bytes());
    let l = layout(&p.write().unwrap());
    assert!(l.cell(at("F5")).is_none());
    assert_eq!(fill(&l, "F5"), Some([255, 0, 0, 255]));
}

#[test]
fn all_eighty_four_normative_pivot_presets_resolve() {
    for family in ["Light", "Medium", "Dark"] {
        for n in 1..=28 {
            let mut b = read_xlsx(FIXTURE).unwrap();
            std::sync::Arc::make_mut(&mut b.pivots.tables[0])
                .style
                .as_mut()
                .unwrap()
                .insert("name".into(), format!("PivotStyle{family}{n}"));
            let l = layout_workbook_sheet(&b, 1, &LayoutOptions::default()).unwrap();
            assert_eq!(l.pivots.len(), 1, "{family}{n}");
        }
    }
}
#[test]
fn builtin_theme_resolution_is_scoped_to_each_workbook() {
    let mut red = read_xlsx(FIXTURE).unwrap();
    std::sync::Arc::make_mut(&mut red.pivots.tables[0])
        .style
        .as_mut()
        .unwrap()
        .insert("showColHeaders".into(), "1".into());
    red.styles.theme_colors.resize(12, None);
    red.styles.theme_colors[4] = Some([255, 0, 0]);
    let mut blue = red.clone();
    blue.styles.theme_colors[4] = Some([0, 0, 255]);
    let r = layout_workbook_sheet(&red, 1, &LayoutOptions::default()).unwrap();
    let b = layout_workbook_sheet(&blue, 1, &LayoutOptions::default()).unwrap();
    let r = fill(&r, "F3").unwrap();
    let b = fill(&b, "F3").unwrap();
    assert!(r[0] > r[2], "{r:?}");
    assert!(b[2] > b[0], "{b:?}");
}
#[test]
fn a_creation_default_does_not_restyle_an_existing_unnamed_report() {
    let mut b = read_xlsx(FIXTURE).unwrap();
    b.styles.default_pivot_style = Some("PivotStyleMedium9".into());
    std::sync::Arc::make_mut(&mut b.pivots.tables[0])
        .style
        .as_mut()
        .unwrap()
        .remove("name");
    assert!(layout_workbook_sheet(&b, 1, &LayoutOptions::default())
        .unwrap()
        .pivots
        .is_empty());
    std::sync::Arc::make_mut(&mut b.pivots.tables[0])
        .style
        .as_mut()
        .unwrap()
        .insert("name".into(), String::new());
    assert!(layout_workbook_sheet(&b, 1, &LayoutOptions::default())
        .unwrap()
        .pivots
        .is_empty());
}
#[test]
fn explicit_cell_font_and_fill_override_automatic_pivot_formatting() {
    let mut p = OpcPackage::open(&custom()).unwrap();
    let part = "xl/styles.xml";
    let xml=p.required_part_str(part).unwrap().replace("<cellXfs count=\"1\">","<cellXfs count=\"2\">")
        .replace("</cellXfs>","<xf fontId=\"0\" fillId=\"2\" borderId=\"0\" applyFont=\"1\"/></cellXfs>")
        .replace("<fills count=\"2\">","<fills count=\"3\">")
        .replace("</fills>","<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFFFFF00\"/></patternFill></fill></fills>");
    p.replace(part, xml.into_bytes());
    let part = "xl/worksheets/sheet2.xml";
    let xml = p
        .required_part_str(part)
        .unwrap()
        .replace("<c r=\"F3\"", "<c s=\"1\" r=\"F3\"");
    p.replace(part, xml.into_bytes());
    let l = layout(&p.write().unwrap());
    assert_eq!(fill(&l, "F3"), Some([255, 255, 0, 255]));
    assert!(!l.cell(at("F3")).unwrap().font.bold);
}
#[test]
fn repeated_axis_prefixes_distinguish_subtotals_and_subheading_levels() {
    use jade_xlsx_reader::pivot::AxisItem;
    let mut b = read_xlsx(&custom()).unwrap();
    let t = std::sync::Arc::make_mut(&mut b.pivots.tables[0]);
    t.location.reference.end.row = 7;
    t.row_fields = vec![0, 2];
    t.fields[2].axis = Some("axisRow".into());
    t.page_fields.clear();
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
            kind: "default".into(),
            repeat: 0,
            data_index: 0,
            items: vec![1],
        },
        AxisItem {
            kind: "data".into(),
            repeat: 1,
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
    let def = &mut b.styles.table_styles[0];
    def.elements.push(jade_xlsx_reader::TableStyleElement {
        kind: "firstSubtotalRow".into(),
        differential_style_index: Some(2),
        size: 1,
    });
    def.elements.push(jade_xlsx_reader::TableStyleElement {
        kind: "firstRowSubheading".into(),
        differential_style_index: Some(1),
        size: 1,
    });
    let l = layout_workbook_sheet(&b, 1, &LayoutOptions::default()).unwrap();
    assert_eq!(fill(&l, "F4"), Some([0, 255, 0, 255]));
    assert_eq!(fill(&l, "E4"), Some([0, 0, 255, 255]));
}

#[test]
fn medium9_uses_zero_based_normative_pivot_dxf_ids() {
    let mut b = read_xlsx(FIXTURE).unwrap();
    std::sync::Arc::make_mut(&mut b.pivots.tables[0])
        .style
        .as_mut()
        .unwrap()
        .insert("showColHeaders".into(), "1".into());
    let l = layout_workbook_sheet(&b, 1, &LayoutOptions::default()).unwrap();
    assert_eq!(fill(&l, "F3"), Some([68, 114, 196, 255]));
    assert_eq!(
        l.cell(at("F3")).unwrap().text_color.and_then(rgba),
        Some([255, 255, 255, 255])
    );
    // Medium9's wholeTable is font-only, and its column stripe is a border.
    assert_eq!(fill(&l, "F4"), None);
}

#[test]
fn omitted_header_flags_do_not_enable_header_formatting() {
    let l = layout(FIXTURE);
    assert_eq!(fill(&l, "F3"), None);
    assert_eq!(
        l.cell(at("F4"))
            .unwrap()
            .border
            .left
            .as_ref()
            .unwrap()
            .style,
        "thin"
    );
    let mut b = read_xlsx(FIXTURE).unwrap();
    std::sync::Arc::make_mut(&mut b.pivots.tables[0])
        .style
        .as_mut()
        .unwrap()
        .insert("showColStripes".into(), "0".into());
    assert!(layout_workbook_sheet(&b, 1, &LayoutOptions::default())
        .unwrap()
        .cell(at("F4"))
        .unwrap()
        .border
        .left
        .is_none());
}
#[test]
fn the_normative_legacy_workbook_is_not_restyled_by_its_creation_default() {
    let bytes = include_bytes!("../../../public/fixtures/pivot-style-ecma-2016.xlsx");
    let b = read_xlsx(bytes).unwrap();
    assert_eq!(b.pivots.tables.len(), 22);
    let unnamed = b
        .pivots
        .tables
        .iter()
        .filter(|t| t.style.as_ref().is_some_and(|s| !s.contains_key("name")))
        .count();
    assert!(unnamed > 0);
    for t in b
        .pivots
        .tables
        .iter()
        .filter(|t| t.style.as_ref().is_some_and(|s| !s.contains_key("name")))
    {
        let index = b
            .sheets
            .iter()
            .position(|s| s.part_name == t.sheet_part)
            .unwrap();
        let l = layout_workbook_sheet(&b, index, &LayoutOptions::default()).unwrap();
        assert!(!l.pivots.iter().any(|p| p.range == t.location.reference));
    }
}

#[test]
fn conditional_formatting_overrides_manual_and_automatic_pivot_fills() {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/pivot-style-compat.xlsx"
    ))
    .unwrap();
    let part = "xl/worksheets/sheet2.xml";
    let xml = p.required_part_str(part).unwrap().replace("</worksheet>", r#"<conditionalFormatting sqref="F4:F5"><cfRule type="expression" dxfId="2" priority="1"><formula>TRUE</formula></cfRule></conditionalFormatting></worksheet>"#);
    p.replace(part, xml.into_bytes());
    let l = layout(&p.write().unwrap());
    assert_eq!(fill(&l, "F4"), Some([0, 255, 0, 255]));
    assert_eq!(fill(&l, "F5"), Some([0, 255, 0, 255]));
    assert_eq!(fill(&l, "F3"), Some([0, 0, 255, 255]));
}

#[test]
fn a_named_report_extends_axes_even_when_its_cells_are_unstored() {
    let mut b = read_xlsx(&custom()).unwrap();
    let t = std::sync::Arc::make_mut(&mut b.pivots.tables[0]);
    t.location.reference = jade_xlsx_reader::CellRange::parse("X100:Y103").unwrap();
    let l = layout_workbook_sheet(&b, 1, &LayoutOptions::default()).unwrap();
    assert!(l.cell(at("Y103")).is_none());
    let c = l.styled_cell(at("Y103")).expect("unstored report extent");
    assert_eq!(
        c.fill.as_ref().and_then(|f| f.foreground).and_then(rgba),
        Some([0, 255, 0, 255])
    );
    assert!(c.rect.width > 0. && c.rect.height > 0.);
    let c = l.styled_cell(at("Y100")).unwrap();
    assert_eq!(
        c.fill.as_ref().and_then(|f| f.foreground).and_then(rgba),
        Some([0, 0, 255, 255])
    );
}

#[test]
fn invalid_or_unbounded_report_geometry_does_not_expand_layout_axes() {
    for reference in ["A1:XFD1048576", "E3:F3"] {
        let mut b = read_xlsx(&custom()).unwrap();
        std::sync::Arc::make_mut(&mut b.pivots.tables[0])
            .location
            .reference = jade_xlsx_reader::CellRange::parse(reference).unwrap();
        let l = layout_workbook_sheet(&b, 1, &LayoutOptions::default()).unwrap();
        assert!(l.pivots.is_empty(), "{reference}");
        assert!(l.columns.len() < 100 && l.rows.len() < 100);
    }
}

#[test]
fn row_stripe_sizes_and_grand_totals_follow_pivot_precedence() {
    let mut b = read_xlsx(&custom()).unwrap();
    let d = &mut b.styles.table_styles[0];
    d.elements.push(jade_xlsx_reader::TableStyleElement {
        kind: "firstRowStripe".into(),
        differential_style_index: Some(1),
        size: 1,
    });
    d.elements.push(jade_xlsx_reader::TableStyleElement {
        kind: "secondRowStripe".into(),
        differential_style_index: Some(2),
        size: 1,
    });
    let t = std::sync::Arc::make_mut(&mut b.pivots.tables[0]);
    t.style
        .as_mut()
        .unwrap()
        .insert("showRowStripes".into(), "1".into());
    let l = layout_workbook_sheet(&b, 1, &LayoutOptions::default()).unwrap();
    assert_eq!(fill(&l, "F4"), Some([0, 0, 255, 255]));
    assert_eq!(fill(&l, "F5"), Some([0, 255, 0, 255]));
    assert_eq!(fill(&l, "F6"), Some([0, 255, 0, 255]));
    b.styles.table_styles[0]
        .elements
        .iter_mut()
        .find(|e| e.kind == "firstRowStripe")
        .unwrap()
        .size = 2;
    let l = layout_workbook_sheet(&b, 1, &LayoutOptions::default()).unwrap();
    assert_eq!(fill(&l, "F5"), Some([0, 0, 255, 255]));
    assert_eq!(fill(&l, "F6"), Some([0, 255, 0, 255]));
}
