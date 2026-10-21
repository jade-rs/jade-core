//! Sparklines drawn in their cells, and a sheet's tab colour.
mod common;

use common::{number, styles, workbook, SheetPart};
use jade_xlsx_layout::{chart::ChartOp, layout_workbook_sheet, LayoutOptions, SheetLayout};
use jade_xlsx_reader::{read_xlsx, CellRef, Color};

fn lay_out(kind: &str, flags: &str) -> (SheetLayout, Option<Color>) {
    let row: String = [3.0, -1.0, 4.0, 1.0, 5.0]
        .iter()
        .enumerate()
        .map(|(index, value)| number(&format!("{}1", (b'A' + index as u8) as char), *value))
        .collect();
    let sheet = SheetPart::new(
        "Data",
        format!(
            r#"<sheetPr><tabColor rgb="FF00B050"/></sheetPr><sheetData><row r="1">{row}</row></sheetData><extLst><ext uri="{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}"><x14:sparklineGroups><x14:sparklineGroup type="{kind}" displayEmptyCellsAs="gap" {flags}><x14:colorSeries rgb="FF376092"/><x14:colorNegative rgb="FFD00000"/><x14:colorAxis rgb="FF000000"/><x14:colorMarkers rgb="FF00B050"/><x14:colorHigh rgb="FFFFC000"/><x14:sparklines><x14:sparkline><xm:f>Data!A1:E1</xm:f><xm:sqref>F1</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst>"#
        ),
    );
    let book = read_xlsx(&workbook(vec![sheet], &styles("", "", ""), &[])).unwrap();
    let tab = book.sheets[0].tab_color;
    (
        layout_workbook_sheet(&book, 0, &LayoutOptions::default()).unwrap(),
        tab,
    )
}

#[test]
fn a_line_sparkline_runs_through_its_cell_with_markers() {
    let (layout, tab) = lay_out("line", r#"markers="1" high="1" displayXAxis="1""#);
    assert_eq!(tab, Some(Color::Argb([255, 0, 176, 80])));
    assert_eq!(layout.sparklines.len(), 1);
    let sparkline = &layout.sparklines[0];
    assert_eq!(sparkline.reference, CellRef::parse("F1").unwrap());
    let line = sparkline
        .ops
        .iter()
        .find_map(|op| match op {
            ChartOp::Path {
                points,
                closed: false,
                stroke: Some(color),
                ..
            } if color == "rgba(55, 96, 146, 1)" => Some(points.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(line.len(), 5);
    let rect = sparkline.rect;
    assert!(line
        .iter()
        .all(|(x, y)| *x >= rect.x && *x <= rect.right() && *y >= rect.y && *y <= rect.bottom()));
    // The highest point (5) is the topmost.
    assert!(line[4].1 < line[2].1);
    let markers: Vec<_> = sparkline
        .ops
        .iter()
        .filter_map(|op| match op {
            ChartOp::Path {
                closed: true,
                fill: Some(color),
                ..
            } => Some(color.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(markers.len(), 5);
    assert_eq!(
        markers
            .iter()
            .filter(|c| *c == "rgba(255, 192, 0, 1)")
            .count(),
        1
    );
    // The axis crosses at zero.
    assert!(sparkline.ops.iter().any(|op| matches!(op,
        ChartOp::Path { stroke: Some(color), .. } if color == "rgba(0, 0, 0, 1)")));
}

#[test]
fn column_and_win_loss_sparklines_colour_negative_bars() {
    for kind in ["column", "stacked"] {
        let (layout, _) = lay_out(kind, r#"negative="1""#);
        let bars: Vec<_> = layout.sparklines[0]
            .ops
            .iter()
            .filter_map(|op| match op {
                ChartOp::Rectangle {
                    rect,
                    fill: Some(color),
                    ..
                } => Some((*rect, color.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 5, "{kind}");
        assert_eq!(bars[1].1, "rgba(208, 0, 0, 1)");
        assert_eq!(bars[0].1, "rgba(55, 96, 146, 1)");
        if kind == "stacked" {
            // Win/loss bars are all the same height.
            assert!((bars[0].0.height - bars[4].0.height).abs() < 1e-9);
        } else {
            assert!(bars[4].0.height > bars[0].0.height);
        }
    }
}
