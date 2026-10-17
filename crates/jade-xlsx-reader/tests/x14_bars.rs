use jade_xlsx_reader::{
    conditional_format::{ConditionalVisual, DataBarAxisPosition, DataBarDirection},
    read_xlsx, OpcPackage,
};
fn fixture() -> &'static [u8] {
    include_bytes!("../../../public/fixtures/x14-bars-workbook.xlsx")
}
fn edit(edit: impl FnOnce(String) -> String) -> jade_xlsx_reader::Workbook {
    let mut package = OpcPackage::open(fixture()).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    package.replace("xl/worksheets/sheet1.xml", edit(xml).into_bytes());
    read_xlsx(&package.write().unwrap()).unwrap()
}
#[test]
fn linked_and_standalone_bars_keep_priorities_thresholds_colors_and_view_context() {
    let book = read_xlsx(fixture()).unwrap();
    for (index, sheet) in book.sheets.iter().enumerate() {
        assert_eq!(sheet.conditional_formats.len(), 8);
        for (column, format) in sheet.conditional_formats.iter().enumerate() {
            let rule = &format.rules[0];
            assert_eq!(rule.priority, column as u32 + 1);
            assert!(rule.issues.is_empty(), "{:?}", rule.issues);
            let Some(ConditionalVisual::DataBar {
                thresholds,
                colors,
                options: Some(options),
                ..
            }) = &rule.visual
            else {
                panic!("x14 bar")
            };
            assert_eq!(thresholds[0].kind, "autoMin");
            assert_eq!(thresholds[1].kind, "autoMax");
            assert_eq!(colors.len(), 1);
            assert_eq!(options.linked_id.is_some(), column != 7);
            assert_eq!(options.context_right_to_left, index == 1);
            assert_eq!(options.gradient, column == 3);
            assert!(options.negative_color.is_some());
        }
        let Some(ConditionalVisual::DataBar {
            options: Some(options),
            ..
        }) = &sheet.conditional_formats[1].rules[0].visual
        else {
            panic!()
        };
        assert_eq!(options.axis_position, DataBarAxisPosition::Middle);
        assert!(options.negative_border_color.is_some());
        let Some(ConditionalVisual::DataBar {
            options: Some(options),
            ..
        }) = &sheet.conditional_formats[3].rules[0].visual
        else {
            panic!()
        };
        assert_eq!(options.direction, DataBarDirection::RightToLeft);
    }
}
#[test]
fn links_are_case_insensitive_namespace_owned_and_reject_duplicate_or_mismatched_definitions() {
    let book = edit(|xml| {
        xml.replace(
            "<x14:id>{01234567-89AB-CDEF-0123-000000000001}</x14:id>",
            "<x14:id><![CDATA[{01234567-89ab-cdef-0123-000000000001}]]></x14:id>",
        )
    });
    assert!(book.sheets[0].conditional_formats[0].rules[0]
        .issues
        .is_empty());
    for (before, after, reason) in [
        (
            "<xm:sqref>A1:A5</xm:sqref>",
            "<xm:sqref>A1:A4</xm:sqref>",
            "ranges disagree",
        ),
        (
            "id=\"{01234567-89AB-CDEF-0123-000000000002}\"",
            "id=\"{01234567-89AB-CDEF-0123-000000000001}\"",
            "duplicate",
        ),
        (
            "xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"",
            "xmlns:x14=\"urn:foreign\"",
            "extensions",
        ),
    ] {
        let book = edit(|xml| xml.replace(before, after));
        assert!(
            book.sheets[0].conditional_formats[0].rules[0]
                .issues
                .iter()
                .any(|issue| issue.contains(reason)),
            "{reason}"
        );
    }
    let book = edit(|xml| {
        xml.replace(
            "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
            "http://purl.oclc.org/ooxml/spreadsheetml/main",
        )
    });
    assert!(book.sheets[0].conditional_formats[0].rules[0]
        .issues
        .is_empty());
}
#[test]
fn isolated_extension_ids_are_ignored_and_unknown_standalone_rules_remain_visible() {
    let book = edit(|xml| {
        xml.replace(
            "<x14:id>{01234567-89AB-CDEF-0123-000000000001}</x14:id>",
            "<x14:id>{FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF}</x14:id>",
        )
    });
    assert_eq!(book.sheets[0].conditional_formats.len(), 8);
    assert!(book.sheets[0].conditional_formats[0].rules[0]
        .issues
        .iter()
        .any(|issue| issue.contains("missing")));
    let book = edit(|xml| {
        xml.replace(
            "<x14:cfRule type=\"dataBar\" priority=\"8\">",
            "<x14:cfRule type=\"iconSet\" priority=\"8\">",
        )
    });
    let rule = &book.sheets[0].conditional_formats[7].rules[0];
    assert_eq!(rule.rule_type, "iconSet");
    assert!(!rule.issues.is_empty());
}
#[test]
fn foreign_wrappers_and_additional_extensions_cannot_override_a_linked_bar() {
    let book = edit(|xml| {
        xml.replace(
            "<x14:conditionalFormattings>",
            "<other xmlns=\"urn:foreign\"><x14:conditionalFormattings>",
        )
        .replace(
            "</x14:conditionalFormattings>",
            "</x14:conditionalFormattings></other>",
        )
    });
    assert_eq!(book.sheets[0].conditional_formats.len(), 7);
    assert!(!book.sheets[0].conditional_formats[0].rules[0]
        .issues
        .is_empty());
    let book = edit(|xml| {
        xml.replacen(
            "<cfvo type=\"min\"/>",
            "<cfvo type=\"min\"><extLst><ext uri=\"urn:future\"/></extLst></cfvo>",
            1,
        )
    });
    assert!(!book.sheets[0].conditional_formats[0].rules[0]
        .issues
        .is_empty());
}

#[test]
fn xm_threshold_text_is_retained_and_invalid_boolean_priority_and_fallback_records_are_reported() {
    let book = edit(|xml| {
        xml.replacen(
            "<x14:cfvo type=\"autoMin\"/>",
            "<x14:cfvo type=\"num\"><xm:f><![CDATA[0]]></xm:f></x14:cfvo>",
            1,
        )
    });
    let Some(ConditionalVisual::DataBar { thresholds, .. }) =
        &book.sheets[0].conditional_formats[0].rules[0].visual
    else {
        panic!()
    };
    assert_eq!(
        (thresholds[0].kind.as_str(), thresholds[0].value.as_deref()),
        ("num", Some("0"))
    );
    for (before, after, reason) in [
        (
            "gradient=\"0\" border=\"1\"",
            "gradient=\"bogus\" border=\"1\"",
            "gradient",
        ),
        (
            "<x14:cfRule type=\"dataBar\" priority=\"8\">",
            "<x14:cfRule type=\"dataBar\" priority=\"1\">",
            "priority",
        ),
        (
            "<dataBar><cfvo type=\"min\"/>",
            "<dataBar minLength=\"0\"><cfvo type=\"min\"/>",
            "fallback",
        ),
    ] {
        let book = edit(|xml| xml.replacen(before, after, 1));
        assert!(
            book.sheets[0]
                .conditional_formats
                .iter()
                .flat_map(|format| &format.rules)
                .any(|rule| rule.issues.iter().any(|issue| issue.contains(reason))),
            "{reason}"
        );
    }
}
