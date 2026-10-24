//! Conditional formats a formula decides (`expression`, `cellIs` against a
//! reference, `timePeriod`) and icon sets.
mod common;

use common::{column_of_numbers, number, styles, workbook, SheetPart};
use jade_xlsx_layout::{layout_workbook_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{read_xlsx, CellRef, Color};

const DXFS: &str = r#"<dxf><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill></dxf><dxf><font><b/></font></dxf>"#;
const PINK: Color = Color::Argb([255, 255, 199, 206]);

fn lay_out(sheets: Vec<SheetPart>) -> SheetLayout {
    let book = read_xlsx(&workbook(sheets, &styles(DXFS, "", ""), &[])).unwrap();
    layout_workbook_sheet(
        &book,
        0,
        &LayoutOptions {
            min_column_count: 6,
            min_row_count: 12,
            ..Default::default()
        },
    )
    .unwrap()
}

fn at(name: &str) -> CellRef {
    CellRef::parse(name).unwrap()
}

fn filled(layout: &SheetLayout, name: &str) -> bool {
    let reference = at(name);
    let fill = match layout.cell(reference) {
        Some(cell) => cell.fill.clone(),
        None => layout
            .blank_cells_in(Rect {
                x: 0.0,
                y: 0.0,
                width: 2000.0,
                height: 2000.0,
            })
            .into_iter()
            .find(|cell| cell.reference == reference)
            .and_then(|cell| cell.fill),
    };
    fill.is_some_and(|fill| fill.foreground == Some(PINK))
}

#[test]
fn an_expression_rule_moves_its_relative_references_with_each_cell() {
    let rows: String = (1..=6)
        .map(|row| {
            format!(
                r#"<row r="{row}">{}{}</row>"#,
                number(&format!("A{row}"), f64::from(row)),
                number(&format!("B{row}"), 0.0)
            )
        })
        .collect();
    let layout = lay_out(vec![SheetPart::new(
        "Sheet1",
        format!(
            r#"<sheetData>{rows}</sheetData><conditionalFormatting sqref="B1:B6"><cfRule type="expression" dxfId="0" priority="1"><formula>A1&gt;3</formula></cfRule></conditionalFormatting>"#
        ),
    )]);
    for (cell, expected) in [("B1", false), ("B3", false), ("B4", true), ("B6", true)] {
        assert_eq!(filled(&layout, cell), expected, "{cell}");
    }
}

#[test]
fn a_banding_formula_paints_blank_cells_too() {
    let layout = lay_out(vec![SheetPart::new(
        "Sheet1",
        format!(
            r#"<sheetData>{}</sheetData><conditionalFormatting sqref="A1:C6"><cfRule type="expression" dxfId="0" priority="1"><formula>MOD(ROW(),2)=0</formula></cfRule></conditionalFormatting>"#,
            column_of_numbers("A", 1, &[1.0, 2.0, 3.0])
        ),
    )]);
    assert!(filled(&layout, "A2"));
    assert!(!filled(&layout, "A3"));
    // Blank cells, with no <c> in the file.
    assert!(filled(&layout, "C2"));
    assert!(filled(&layout, "B6"));
    assert!(!filled(&layout, "C5"));
}

#[test]
fn cell_is_compares_with_a_reference_and_another_sheet() {
    let layout = lay_out(vec![
        SheetPart::new(
            "Sheet1",
            format!(
                r#"<sheetData>{}</sheetData><conditionalFormatting sqref="A1:A4"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>$D$9</formula></cfRule><cfRule type="expression" dxfId="1" priority="2"><formula>A1=Other!$A$1</formula></cfRule></conditionalFormatting>"#,
                column_of_numbers("A", 1, &[1.0, 2.0, 3.0, 4.0])
                    + &format!(r#"<row r="9">{}</row>"#, number("D9", 2.0))
            ),
        ),
        SheetPart::new(
            "Other",
            format!(
                r#"<sheetData><row r="1">{}</row></sheetData>"#,
                number("A1", 1.0)
            ),
        ),
    ]);
    assert!(!filled(&layout, "A1"));
    assert!(!filled(&layout, "A2"));
    assert!(filled(&layout, "A3"));
    assert!(layout.cell(at("A1")).unwrap().font.bold);
    assert!(!layout.cell(at("A2")).unwrap().font.bold);
}

#[test]
fn dates_occurring_follow_today() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let today = (now / 86_400) as f64 + 25_569.0;
    let values = [today - 1.0, today, today - 5.0, today - 9.0, today + 0.5];
    let layout = lay_out(vec![SheetPart::new(
        "Sheet1",
        format!(
            r#"<sheetData>{}</sheetData><conditionalFormatting sqref="A1:A5"><cfRule type="timePeriod" dxfId="0" priority="1" timePeriod="yesterday"/><cfRule type="timePeriod" dxfId="1" priority="2" timePeriod="last7Days"><formula>AND(TODAY()-FLOOR(A1,1)&lt;=6,FLOOR(A1,1)&lt;=TODAY())</formula></cfRule></conditionalFormatting>"#,
            column_of_numbers("A", 1, &values)
        ),
    )]);
    assert!(filled(&layout, "A1"));
    assert!(!filled(&layout, "A2"));
    assert!(!filled(&layout, "A5"));
    let bold = |name| layout.cell(at(name)).unwrap().font.bold;
    assert!(bold("A2") && bold("A3") && bold("A5"));
    assert!(!bold("A4"));
}

fn icons(xml: &str) -> SheetLayout {
    lay_out(vec![SheetPart::new(
        "Sheet1",
        format!(
            "<sheetData>{}</sheetData>{xml}",
            column_of_numbers("A", 1, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0])
        ),
    )])
}

fn icon(layout: &SheetLayout, name: &str) -> Option<(String, u32, bool)> {
    let icon = layout.cell(at(name))?.icon.clone()?;
    Some((icon.icon_set, icon.index, icon.show_value))
}

#[test]
fn an_icon_set_picks_the_band_each_value_reaches() {
    let layout = icons(
        r#"<conditionalFormatting sqref="A1:A10"><cfRule type="iconSet" priority="1"><iconSet iconSet="3Arrows"><cfvo type="percent" val="0"/><cfvo type="percent" val="33"/><cfvo type="percent" val="67"/></iconSet></cfRule></conditionalFormatting>"#,
    );
    // 33% of 1..10 is 3.97 and 67% is 7.03.
    assert_eq!(icon(&layout, "A1"), Some(("3Arrows".into(), 0, true)));
    assert_eq!(icon(&layout, "A4").unwrap().1, 1);
    assert_eq!(icon(&layout, "A7").unwrap().1, 1);
    assert_eq!(icon(&layout, "A8").unwrap().1, 2);
    let rect = layout.cell(at("A1")).unwrap().rect;
    let placed = layout.cell(at("A1")).unwrap().icon.clone().unwrap().rect;
    assert_eq!(placed.width, 16.0);
    assert!(placed.x >= rect.x && placed.y >= rect.y && placed.bottom() <= rect.bottom());
}

#[test]
fn reversed_numbered_icon_sets_and_hidden_values() {
    let layout = icons(
        r#"<conditionalFormatting sqref="A1:A10"><cfRule type="iconSet" priority="1"><iconSet iconSet="4Rating" reverse="1" showValue="0"><cfvo type="percent" val="0"/><cfvo type="num" val="3"/><cfvo type="num" val="5" gte="0"/><cfvo type="num" val="8"/></iconSet></cfRule></conditionalFormatting>"#,
    );
    assert_eq!(icon(&layout, "A1"), Some(("4Rating".into(), 3, false)));
    assert_eq!(icon(&layout, "A3").unwrap().1, 2);
    // 5 is not past a threshold that excludes itself.
    assert_eq!(icon(&layout, "A5").unwrap().1, 2);
    assert_eq!(icon(&layout, "A6").unwrap().1, 1);
    assert_eq!(icon(&layout, "A10").unwrap().1, 0);
}

#[test]
fn x14_icon_sets_and_custom_icons() {
    let layout = icons(
        r#"<extLst><ext uri="{78C0D931-6437-407D-A8EE-F0AAD7539E65}"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="iconSet" priority="1" id="{00000000-0000-0000-0000-000000000001}"><x14:iconSet iconSet="3Stars" custom="1"><x14:cfvo type="percent"><xm:f>0</xm:f></x14:cfvo><x14:cfvo type="percent"><xm:f>33</xm:f></x14:cfvo><x14:cfvo type="percent"><xm:f>67</xm:f></x14:cfvo><x14:cfIcon iconSet="3Flags" iconId="0"/><x14:cfIcon iconSet="NoIcons" iconId="0"/><x14:cfIcon iconSet="3Stars" iconId="2"/></x14:iconSet></x14:cfRule><xm:sqref>A1:A10</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#,
    );
    assert_eq!(icon(&layout, "A1"), Some(("3Flags".into(), 0, true)));
    assert_eq!(icon(&layout, "A5"), None);
    assert_eq!(icon(&layout, "A10"), Some(("3Stars".into(), 2, true)));
}
