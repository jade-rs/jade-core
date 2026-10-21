//! Excel tables draw their table style beneath the cells' own formats.
mod common;

use common::{number, styles, workbook, SheetPart};
use jade_xlsx_layout::{color::rgba, layout_workbook_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{read_xlsx, CellRef};

/// A 4×5 table at A1:C5 (header, three rows, totals) in `style`, with A2
/// filled yellow by its own format and C3 left blank.
fn lay_out(style: &str, info: &str, styles_xml: &str) -> SheetLayout {
    let mut rows = String::from(
        r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Q1</t></is></c><c r="C1" t="inlineStr"><is><t>Q2</t></is></c></row>"#,
    );
    for row in 2..=4 {
        rows.push_str(&format!(
            r#"<row r="{row}"><c r="A{row}"{}><v>{row}</v></c>{}{}</row>"#,
            if row == 2 { r#" s="1""# } else { "" },
            number(&format!("B{row}"), f64::from(row)),
            if row == 3 {
                String::new()
            } else {
                number(&format!("C{row}"), 1.0)
            }
        ));
    }
    rows.push_str(&format!(r#"<row r="5">{}</row>"#, number("B5", 9.0)));
    let sheet = SheetPart::new(
        "Sheet1",
        format!(r#"<sheetData>{rows}</sheetData><tableParts count="1"><tablePart r:id="rId1"/></tableParts>"#),
    )
    .rel("rId1", "table", "../tables/table1.xml");
    let table = format!(
        r#"<?xml version="1.0"?><table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="T" displayName="T" ref="A1:C5" totalsRowCount="1"><tableColumns count="3"><tableColumn id="1" name="Item"/><tableColumn id="2" name="Q1"/><tableColumn id="3" name="Q2"/></tableColumns><tableStyleInfo name="{style}" {info}/></table>"#
    );
    let book = read_xlsx(&workbook(
        vec![sheet],
        styles_xml,
        &[("xl/tables/table1.xml", &table)],
    ))
    .unwrap();
    layout_workbook_sheet(&book, 0, &LayoutOptions::default()).unwrap()
}

fn yellow_cell_styles(extra: &str) -> String {
    styles(extra, r#"<xf numFmtId="0" fontId="0" fillId="2" borderId="0" xfId="0" applyFill="1"/>"#, "")
        .replace(
            r#"<fill><patternFill patternType="gray125"/></fill></fills>"#,
            r#"<fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/></patternFill></fill></fills>"#,
        )
}

fn fill(layout: &SheetLayout, name: &str) -> Option<[u8; 4]> {
    let reference = CellRef::parse(name).unwrap();
    let fill = match layout.cell(reference) {
        Some(cell) => cell.fill.clone(),
        None => layout
            .blank_cells_in(Rect {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 1000.0,
            })
            .into_iter()
            .find(|cell| cell.reference == reference)
            .and_then(|cell| cell.fill),
    }?;
    rgba(fill.foreground?)
}

const ACCENT1: [u8; 4] = [68, 114, 196, 255];

#[test]
fn medium2_fills_the_header_and_bands_the_rows() {
    let layout = lay_out(
        "TableStyleMedium2",
        r#"showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0""#,
        &yellow_cell_styles(""),
    );
    let header = layout.cell(CellRef::parse("B1").unwrap()).unwrap();
    assert_eq!(fill(&layout, "B1"), Some(ACCENT1));
    assert!(header.font.bold);
    assert_eq!(header.text_color.and_then(rgba), Some([255, 255, 255, 255]));
    // The first band is a pale tint of the accent; the second is unfilled.
    let band = fill(&layout, "B2").unwrap();
    assert!(band[2] > band[0] && band[0] > 200, "{band:?}");
    assert_eq!(fill(&layout, "B3"), None);
    assert_eq!(fill(&layout, "B4"), Some(band));
    // A cell's own fill wins over the table's.
    assert_eq!(fill(&layout, "A2"), Some([255, 255, 0, 255]));
    // A blank cell inside the table takes the band too.
    assert_eq!(fill(&layout, "C3"), None);
    assert!(layout.cell(CellRef::parse("C3").unwrap()).is_none());
    // Totals: bold, under a double rule.
    let total = layout.cell(CellRef::parse("B5").unwrap()).unwrap();
    assert!(total.font.bold);
    assert_eq!(total.border.top.as_ref().unwrap().style, "double");
    // Rows are ruled between, in a lighter tint.
    assert_eq!(
        layout
            .cell(CellRef::parse("B3").unwrap())
            .unwrap()
            .border
            .top
            .as_ref()
            .unwrap()
            .style,
        "thin"
    );
}

#[test]
fn blank_cells_in_a_banded_row_are_painted() {
    let layout = lay_out(
        "TableStyleMedium2",
        r#"showFirstColumn="0" showLastColumn="0" showRowStripes="0" showColumnStripes="1""#,
        &yellow_cell_styles(""),
    );
    // Column stripes: A and C banded, B not; C3 has no cell in the file.
    assert!(fill(&layout, "C3").is_some());
    assert!(fill(&layout, "B3").is_none());
}

#[test]
fn first_column_and_dark_styles() {
    let layout = lay_out(
        "TableStyleDark1",
        r#"showFirstColumn="1" showLastColumn="0" showRowStripes="1" showColumnStripes="0""#,
        &yellow_cell_styles(""),
    );
    assert_eq!(fill(&layout, "B1"), Some([0, 0, 0, 255]));
    let first = layout.cell(CellRef::parse("A3").unwrap()).unwrap();
    assert!(first.font.bold);
    assert_eq!(first.text_color.and_then(rgba), Some([255, 255, 255, 255]));
}

#[test]
fn custom_table_styles_come_from_the_files_dxfs() {
    let dxfs = r#"<dxf><fill><patternFill><bgColor rgb="FF00FF00"/></patternFill></fill></dxf><dxf><font><b/><color rgb="FFFF0000"/></font><border><bottom style="thick"><color rgb="FF0000FF"/></bottom></border></dxf>"#;
    let styles_xml = yellow_cell_styles(dxfs).replace(
        "</dxfs>",
        r#"</dxfs><tableStyles count="1"><tableStyle name="Mine" count="2"><tableStyleElement type="wholeTable" dxfId="0"/><tableStyleElement type="headerRow" dxfId="1"/></tableStyle></tableStyles>"#,
    );
    let layout = lay_out(
        "Mine",
        r#"showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0""#,
        &styles_xml,
    );
    assert_eq!(fill(&layout, "B3"), Some([0, 255, 0, 255]));
    let header = layout.cell(CellRef::parse("B1").unwrap()).unwrap();
    assert!(header.font.bold);
    assert_eq!(header.text_color.and_then(rgba), Some([255, 0, 0, 255]));
    assert_eq!(header.border.bottom.as_ref().unwrap().style, "thick");
}
