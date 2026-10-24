use jade_xlsx_reader::{model::Color, sparkline::parse_sparklines, CellRef};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";

fn sheet(body: &str) -> String {
    format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:x="{X14}" xmlns:m="{XM}" xmlns:u="urn:foreign">{body}</s:worksheet>"#
    )
}
fn extension(body: &str) -> String {
    format!("<s:extLst><s:ext uri=\"spark\"><x:sparklineGroups>{body}</x:sparklineGroups></s:ext></s:extLst>")
}
fn spark(formula: &str, destination: &str) -> String {
    format!("<x:sparkline><m:f>{formula}</m:f><m:sqref>{destination}</m:sqref></x:sparkline>")
}

#[test]
fn foreign_and_misplaced_groups_do_not_create_sparklines() {
    let body = format!(
        "<u:sparklineGroup type=\"column\"><u:sparkline><u:f>A1:A3</u:f><u:sqref>D1</u:sqref></u:sparkline></u:sparklineGroup><x:sparklineGroups><x:sparklineGroup/></x:sparklineGroups><u:wrapper>{}</u:wrapper>",
        extension("<x:sparklineGroup/>")
    );
    assert!(parse_sparklines(&sheet(&body)).unwrap().is_empty());
}

#[test]
fn only_direct_owned_group_children_supply_settings_and_data() {
    let xml = sheet(&extension(&format!(
        "<x:sparklineGroup type=\"line\"><x:colorSeries rgb=\"FF123456\"/><u:wrapper><x:colorSeries rgb=\"FFFFFFFF\"/></u:wrapper><x:sparklines><x:sparkline><m:f>Data!A1:A3</m:f><m:sqref>D1</m:sqref><u:f>BAD!A1:A9</u:f><u:sqref>Z99</u:sqref><u:wrapper><m:f>WRAPPED!A1:A9</m:f><m:sqref>Z98</m:sqref></u:wrapper></x:sparkline><u:sparkline><m:f>FOREIGN!A1:A9</m:f><m:sqref>Z97</m:sqref></u:sparkline></x:sparklines>{}</x:sparklineGroup>",
        spark("MISPLACED!A1:A9", "Z96")
    )));
    let groups = parse_sparklines(&xml).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].color_series, Some(Color::Argb([255, 18, 52, 86])));
    assert_eq!(groups[0].sparklines.len(), 1);
    assert_eq!(groups[0].sparklines[0].formula, "Data!A1:A3");
    assert_eq!(groups[0].sparklines[0].cell, CellRef::new(3, 0));
}

#[test]
fn strict_and_locally_default_namespaces_are_supported() {
    let xml = format!(
        r#"<s:worksheet xmlns:s="{STRICT}"><s:extLst><s:ext uri="spark"><sparklineGroups xmlns="{X14}"><sparklineGroup type="column"><sparklines><sparkline><f xmlns="{XM}">Data!A1:A3</f><sqref xmlns="{XM}">D1</sqref></sparkline></sparklines></sparklineGroup></sparklineGroups></s:ext></s:extLst></s:worksheet>"#
    );
    let groups = parse_sparklines(&xml).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].kind, "column");
    assert_eq!(groups[0].sparklines.len(), 1);
    assert_eq!(groups[0].sparklines[0].cell, CellRef::new(3, 0));
}

#[test]
fn cdata_entities_comments_and_processing_instructions_keep_leaf_text() {
    let xml = sheet(&extension(&format!(
        "<x:sparklineGroup><x:sparklines>{}</x:sparklines></x:sparklineGroup>",
        spark(
            "<![CDATA['A&]]>&amp;<![CDATA[B'!A1]]><!--keep--><?hint value?>:A3",
            "<![CDATA[D]]>1"
        )
    )));
    let groups = parse_sparklines(&xml).unwrap();
    assert_eq!(groups[0].sparklines[0].formula, "'A&&B'!A1:A3");
    assert_eq!(groups[0].sparklines[0].cell, CellRef::new(3, 0));
}

#[test]
fn invalid_or_missing_display_references_do_not_default_to_a1() {
    let mut entries = String::from("<x:sparkline><m:f>Data!A1:A3</m:f></x:sparkline>");
    for invalid in ["", "garbage", "A1:B1", "A1 B1", "XFE1", "A1048577"] {
        entries.push_str(&spark("Data!A1:A3", invalid));
    }
    entries.push_str(&spark("", "D1"));
    entries.push_str(&spark("Data!A1:A3", "A1"));
    let xml = sheet(&extension(&format!(
        "<x:sparklineGroup><x:sparklines>{entries}</x:sparklines></x:sparklineGroup>"
    )));
    let groups = parse_sparklines(&xml).unwrap();
    assert_eq!(groups[0].sparklines.len(), 1);
    assert_eq!(groups[0].sparklines[0].cell, CellRef::new(0, 0));
}

#[test]
fn one_cell_ranges_and_absolute_destinations_resolve_to_their_cell() {
    let entries = ["D1:D1", "$E$2:$E$2", "$F$3"]
        .map(|destination| spark("Data!A1:A3", destination))
        .join("");
    let xml = sheet(&extension(&format!(
        "<x:sparklineGroup><x:sparklines>{entries}</x:sparklines></x:sparklineGroup>"
    )));
    let groups = parse_sparklines(&xml).unwrap();
    let cells: Vec<_> = groups[0].sparklines.iter().map(|s| s.cell).collect();
    assert_eq!(
        cells,
        [CellRef::new(3, 0), CellRef::new(4, 1), CellRef::new(5, 2)]
    );
}

#[test]
fn opaque_nested_groups_and_namespace_rebinding_do_not_replace_the_active_group() {
    let xml = sheet(&extension(&format!(
        "<x:sparklineGroup type=\"line\"><u:wrapper><x:sparklineGroup type=\"column\"><x:sparklines>{}</x:sparklines></x:sparklineGroup></u:wrapper><x:sparklines><u:wrapper xmlns:m=\"urn:foreign\">{}</u:wrapper>{}</x:sparklines></x:sparklineGroup><x:sparklineGroup type=\"column\"/>",
        spark("BAD!A1:A3", "Z99"), spark("BAD2!A1:A3", "Z98"), spark("Data!A1:A3", "D1")
    )));
    let groups = parse_sparklines(&xml).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].kind, "line");
    assert_eq!(groups[0].sparklines.len(), 1);
    assert_eq!(groups[0].sparklines[0].formula, "Data!A1:A3");
    assert_eq!(groups[1].kind, "column");
    assert!(groups[1].sparklines.is_empty());
}
