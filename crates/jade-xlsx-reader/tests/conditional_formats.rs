use jade_xlsx_reader::{
    conditional_format::parse_conditional_formats, read_xlsx, styles::parse_styles, Color,
};

#[test]
fn reads_rules_priorities_partial_styles_and_keeps_normal_number_formats_separate() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/conditional-formats-workbook.xlsx"
    ))
    .unwrap();
    assert_eq!(book.styles.number_formats.get(&164).unwrap(), "0.00");
    let styles = &book.styles.differential_styles;
    assert_eq!(styles.len(), 5);
    assert_eq!(styles[0].bold, None);
    assert_eq!(styles[2].bold, Some(false));
    assert_eq!(styles[2].italic, Some(true));
    assert_eq!(styles[3].number_format_code.as_deref(), Some("$0.00"));
    assert!(matches!(
        styles[3].bottom,
        Some(jade_xlsx_reader::conditional_format::DifferentialBorder::Edge(_))
    ));
    let formats = &book.sheets[0].conditional_formats;
    assert_eq!(formats.len(), 8);
    assert_eq!(formats[6].ranges.len(), 3);
    assert_eq!(formats[0].rules[2].priority, 1);
    assert!(formats[2].rules[0].stop_if_true);
    assert_eq!(formats[7].rules[0].formulas, ["A1>1"]);
}

#[test]
fn namespace_resolution_accepts_strict_prefixes_and_ignores_extension_rules() {
    let xml = r#"<x:worksheet xmlns:x="http://purl.oclc.org/ooxml/spreadsheetml/main" xmlns:ext="urn:other"><ext:conditionalFormatting sqref="Z1"><ext:cfRule type="cellIs" priority="1"/></ext:conditionalFormatting><x:conditionalFormatting sqref="A1 B2:C3"><x:cfRule type="cellIs" dxfId="2" priority="3" operator="equal"><x:formula><![CDATA["A&B"]]></x:formula></x:cfRule></x:conditionalFormatting></x:worksheet>"#;
    let formats = parse_conditional_formats(xml).unwrap();
    assert_eq!(formats.len(), 1);
    assert_eq!(formats[0].ranges.len(), 2);
    assert_eq!(formats[0].rules[0].formulas, ["\"A&B\""]);
}

#[test]
fn dxf_font_color_and_border_color_do_not_cross_contaminate() {
    let styles = parse_styles(r#"<styleSheet><dxfs><dxf><font><color rgb="FF112233"/><u val="none"/></font><border><left/><right style="thin"><color rgb="FF445566"/></right></border></dxf></dxfs></styleSheet>"#).unwrap();
    let style = &styles.differential_styles[0];
    assert_eq!(style.text_color, Some(Color::Argb([255, 17, 34, 51])));
    assert_eq!(style.underline, Some(false));
    assert_eq!(
        style.left,
        Some(jade_xlsx_reader::conditional_format::DifferentialBorder::Clear)
    );
    assert_eq!(
        match style.right.as_ref().unwrap() {
            jade_xlsx_reader::conditional_format::DifferentialBorder::Edge(edge) => edge.color,
            _ => None,
        },
        Some(Color::Argb([255, 68, 85, 102]))
    );
    assert!(styles.fonts.is_empty());
    assert!(styles.borders.is_empty());
}

#[test]
fn advanced_underline_variants_are_retained_as_unsupported_properties() {
    let styles = parse_styles(
        r#"<styleSheet><dxfs><dxf><font><u val="double"/></font></dxf></dxfs></styleSheet>"#,
    )
    .unwrap();
    assert_eq!(
        styles.differential_styles[0].unsupported_properties,
        ["underline variant"]
    );
}

#[test]
fn visual_definitions_use_owned_direct_children_and_keep_invalid_flags_visible() {
    use jade_xlsx_reader::conditional_format::ConditionalVisual;
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        let xml = format!(
            r#"<x:worksheet xmlns:x="{namespace}" xmlns:e="urn:foreign"><e:wrapper><x:conditionalFormatting sqref="Z1"><x:cfRule type="dataBar" priority="0"/></x:conditionalFormatting></e:wrapper><x:conditionalFormatting sqref="A1:A5"><x:cfRule type="colorScale" priority="1"><e:wrapper><x:colorScale><x:cfvo type="num" val="999"/><x:color rgb="FFFFFFFF"/><x:extLst/></x:colorScale><x:conditionalFormatting><x:cfRule type="bogus" priority="0"/></x:conditionalFormatting></e:wrapper><x:colorScale><x:cfvo type="min"/><e:color rgb="FF000000"/><e:wrapper><x:colorScale><x:cfvo type="num" val="500"/><x:color rgb="FFFFFFFF"/></x:colorScale></e:wrapper><x:cfvo type="max"/><x:color theme="4" tint="0.4"/><x:color rgb="FFFF0000"/></x:colorScale></x:cfRule><x:cfRule type="dataBar" priority="2"><x:dataBar minLength="25" maxLength="75" showValue="false"><x:cfvo type="num" val="0"/><x:cfvo type="percentile" val="90"/><x:color rgb="FF638EC6"/></x:dataBar></x:cfRule><x:cfRule type="cellIs" priority="3" operator="equal"><x:formula>1<e:wrapper><x:formula>99</x:formula></e:wrapper>0</x:formula></x:cfRule></x:conditionalFormatting></x:worksheet>"#
        );
        let parsed = parse_conditional_formats(&xml).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].rules.len(), 3);
        assert!(parsed[0].rules[0].issues.is_empty());
        let Some(ConditionalVisual::ColorScale { thresholds, colors }) = &parsed[0].rules[0].visual
        else {
            panic!("color scale")
        };
        assert_eq!(thresholds.len(), 2);
        assert_eq!(thresholds[0].kind, "min");
        assert_eq!(thresholds[1].kind, "max");
        assert_eq!(colors.len(), 2);
        assert!(matches!(colors[0],Color::Theme{index:4,tint,..} if (tint-0.4).abs()<1e-6));
        let Some(ConditionalVisual::DataBar {
            min_length,
            max_length,
            show_value,
            ..
        }) = &parsed[0].rules[1].visual
        else {
            panic!("data bar")
        };
        assert_eq!((*min_length, *max_length, *show_value), (25, 75, false));
        assert_eq!(parsed[0].rules[2].formulas, ["10"]);
        let malformed = xml
            .replace("minLength=\"25\"", "minLength=\"oops\"")
            .replace("showValue=\"false\"", "showValue=\"maybe\"");
        let parsed = parse_conditional_formats(&malformed).unwrap();
        assert_eq!(parsed[0].rules[1].issues.len(), 2);
    }
}

#[test]
fn theme_parts_resolve_visual_colors_without_losing_their_theme_or_tint() {
    use jade_xlsx_reader::{conditional_format::ConditionalVisual, OpcPackage};
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/visual-conditional-workbook.xlsx"
    ))
    .unwrap();
    let sheet = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replacen(
            "<color rgb=\"FFFF0000\"/>",
            "<color theme=\"4\" tint=\"0.4\"/>",
            1,
        );
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
    let rels=package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace("</Relationships>","<Relationship Id=\"theme\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/></Relationships>");
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    package.replace("xl/theme/theme1.xml",br#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="fixture"><a:accent1><a:srgbClr val="4472C4"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#.to_vec());
    let book = read_xlsx(&package.write().unwrap()).unwrap();
    let Some(ConditionalVisual::ColorScale { colors, .. }) =
        &book.sheets[0].conditional_formats[0].rules[0].visual
    else {
        panic!("color scale")
    };
    assert!(
        matches!(colors[0],Color::Theme{index:4,tint,resolved:Some([0x44,0x72,0xc4])} if (tint-0.4).abs()<1e-6)
    );
}
