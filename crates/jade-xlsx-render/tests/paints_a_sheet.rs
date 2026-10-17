//! Drives the renderer through `RecordingPainter`, so what is asserted is the
//! exact sequence of drawing operations a real backend would receive.

use std::io::Write;

use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions};
use jade_xlsx_reader::{read_xlsx, CellRef};
use jade_xlsx_render::{render_sheet, RenderOptions};

const SHEET_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetViews><sheetView workbookViewId="0"/></sheetViews>
  <sheetFormatPr defaultColWidth="9.140625" defaultRowHeight="15"/>
  <sheetData>
    <row r="1">
      <c r="A1" s="1" t="inlineStr"><is><t>Head</t></is></c>
      <c r="B1"><v>12</v></c>
      <c r="C1" s="2" t="inlineStr"><is><t>Tilt</t></is></c>
    </row>
    <row r="2">
      <c r="A2" t="inlineStr"><is><t>merged</t></is></c>
      <c r="C2" s="4" t="inlineStr"><is><t>Pattern</t></is></c>
    </row>
  </sheetData>
  <mergeCells count="1"><mergeCell ref="A2:B2"/></mergeCells>
</worksheet>"#;

const STYLES_XML: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="2">
    <font><sz val="11"/><name val="Calibri"/></font>
    <font><b/><sz val="11"/><color rgb="FF112233"/><name val="Meiryo"/></font>
  </fonts>
  <fills count="4">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
    <fill><patternFill patternType="solid"><fgColor theme="4"/></patternFill></fill>
    <fill><patternFill patternType="darkGrid"><fgColor rgb="FFCC0000"/><bgColor rgb="FFFFFFCC"/></patternFill></fill>
  </fills>
  <borders count="3">
    <border><left/><right/><top/><bottom/></border>
    <border><left/><right/><top style="medium"><color rgb="FF000000"/></top><bottom style="double"><color rgb="FF000000"/></bottom></border>
    <border diagonalUp="1" diagonalDown="1"><left/><right/><top/><bottom/><diagonal style="mediumDashDot"><color rgb="FF008000"/></diagonal></border>
  </borders>
  <cellXfs count="5">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0"/>
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment textRotation="45"/></xf>
    <xf numFmtId="0" fontId="0" fillId="3" borderId="0" xfId="0" applyFill="1"/>
    <xf numFmtId="0" fontId="0" fillId="3" borderId="2" xfId="0" applyFill="1" applyBorder="1"/>
  </cellXfs>
</styleSheet>"##;

fn workbook_bytes(show_grid_lines: bool) -> Vec<u8> {
    let sheet = if show_grid_lines {
        SHEET_XML.to_string()
    } else {
        SHEET_XML.replace(
            r#"<sheetView workbookViewId="0"/>"#,
            r#"<sheetView showGridLines="0" workbookViewId="0"/>"#,
        )
    };
    let parts: Vec<(&str, String)> = vec![
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_string(),
        ),
        (
            "xl/workbook.xml",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_string(),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/></Relationships>"#.to_string(),
        ),
        ("xl/styles.xml", STYLES_XML.to_string()),
        (
            "xl/theme/theme1.xml",
            r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="Test"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="222222"/></a:dk2><a:lt2><a:srgbClr val="EEEEEE"/></a:lt2><a:accent1><a:srgbClr val="FFEE00"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#.to_string(),
        ),
        ("xl/worksheets/sheet1.xml", sheet),
    ];

    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in parts {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("start part");
            writer.write_all(contents.as_bytes()).expect("write part");
        }
        writer.finish().expect("finish archive");
    }
    buffer
}

fn paint(show_grid_lines: bool) -> (RecordingPainter, jade_xlsx_layout::SheetLayout) {
    let workbook = read_xlsx(&workbook_bytes(show_grid_lines)).expect("read");
    let sheet = &workbook.sheets[0];
    let layout = layout_sheet(sheet, &workbook.styles, &LayoutOptions::default());
    // One unit of width per character keeps the expected text positions
    // arithmetic rather than font-dependent.
    let painter = RecordingPainter::with_width_per_char(10.0);
    render_sheet(&painter, &layout, &RenderOptions::default()).expect("render");
    (painter, layout)
}

fn index_of(ops: &[Op], predicate: impl Fn(&Op) -> bool) -> usize {
    ops.iter().position(predicate).expect("operation not found")
}

#[test]
fn paints_grid_lines_then_fills_then_borders_then_text() {
    let (painter, _) = paint(true);
    let ops = painter.ops();

    let fill = index_of(
        &ops,
        |op| matches!(op, Op::SetFillStyle(color) if color == "#ffee00"),
    );
    let grid = index_of(
        &ops,
        |op| matches!(op, Op::SetStrokeStyle(color) if color == "#d4d4d4"),
    );
    let border = index_of(
        &ops,
        |op| matches!(op, Op::SetLineWidth(width) if *width == 2.0),
    );
    let text = index_of(
        &ops,
        |op| matches!(op, Op::FillText(text, _, _) if text == "Head"),
    );

    assert!(grid < fill, "a cell fill covers the grid lines");
    assert!(fill < border, "explicit borders sit on top of fills");
    assert!(border < text, "text is painted last");
}

#[test]
fn a_double_border_paints_two_parallel_one_pixel_lines() {
    let (painter, layout) = paint(false);
    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    let expected_y = [cell.rect.bottom() - 2.5, cell.rect.bottom() - 0.5];
    let ops = painter.ops();
    let double_width = ops
        .iter()
        .rposition(|op| matches!(op, Op::SetLineWidth(width) if *width == 1.0))
        .expect("double border line width");
    let ys = ops[double_width..]
        .iter()
        .filter_map(|op| match op {
            Op::MoveTo(_, y) => Some(*y),
            _ => None,
        })
        .take(2)
        .collect::<Vec<_>>();

    assert_eq!(ys, expected_y, "double must retain its white centre gap");
}

#[test]
fn a_solid_fill_paints_its_cell_box() {
    let (painter, layout) = paint(true);
    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    let ops = painter.ops();

    let fill_index = index_of(
        &ops,
        |op| matches!(op, Op::SetFillStyle(c) if c == "#ffee00"),
    );
    assert_eq!(
        ops[fill_index + 1],
        Op::FillRect(cell.rect.x, cell.rect.y, cell.rect.width, cell.rect.height)
    );
}

#[test]
fn a_pattern_fill_paints_its_background_and_foreground_grid() {
    let (painter, layout) = paint(false);
    let cell = layout.cell(CellRef::new(2, 1)).expect("C2");
    let ops = painter.ops();

    let background = index_of(
        &ops,
        |op| matches!(op, Op::SetFillStyle(color) if color == "#ffffcc"),
    );
    assert_eq!(
        ops[background + 1],
        Op::FillRect(cell.rect.x, cell.rect.y, cell.rect.width, cell.rect.height)
    );

    let foreground = index_of(
        &ops,
        |op| matches!(op, Op::SetStrokeStyle(color) if color == "#cc0000"),
    );
    assert!(
        ops[foreground..]
            .iter()
            .any(|op| matches!(op, Op::MoveTo(x, y) if *x > cell.rect.x && *x < cell.rect.right() && *y == cell.rect.y)),
        "pattern must draw vertical foreground lines inside the cell"
    );
    assert!(
        ops[foreground..]
            .iter()
            .any(|op| matches!(op, Op::MoveTo(x, y) if *x == cell.rect.x && *y > cell.rect.y && *y < cell.rect.bottom())),
        "pattern must draw horizontal foreground lines inside the cell"
    );
}

#[test]
fn diagonal_up_and_down_borders_cross_the_cell() {
    let (painter, layout) = paint(false);
    let cell = layout.cell(CellRef::new(2, 1)).expect("C2");
    assert!(cell.border.diagonal_up);
    assert!(cell.border.diagonal_down);
    assert_eq!(
        cell.border
            .diagonal
            .as_ref()
            .map(|edge| edge.style.as_str()),
        Some("mediumDashDot")
    );

    let ops = painter.ops();
    let color = index_of(
        &ops,
        |op| matches!(op, Op::SetStrokeStyle(value) if value == "#008000"),
    );
    let lines = ops[color..]
        .windows(2)
        .filter_map(|ops| match (&ops[0], &ops[1]) {
            (Op::MoveTo(x1, y1), Op::LineTo(x2, y2)) => Some(((*x1, *y1), (*x2, *y2))),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(lines.contains(&(
        (cell.rect.x, cell.rect.y),
        (cell.rect.right(), cell.rect.bottom())
    )));
    assert!(lines.contains(&(
        (cell.rect.x, cell.rect.bottom()),
        (cell.rect.right(), cell.rect.y)
    )));
}

#[test]
fn numbers_are_right_aligned_inside_the_padding() {
    let (painter, layout) = paint(true);
    let cell = layout.cell(CellRef::new(1, 0)).expect("B1");
    let ops = painter.ops();

    let (x, _) = ops
        .iter()
        .find_map(|op| match op {
            Op::FillText(text, x, y) if text == "12" => Some((*x, *y)),
            _ => None,
        })
        .expect("B1 text");
    // Two characters at 10px each, right-aligned 3px inside the cell.
    assert_eq!(x, cell.rect.right() - 3.0 - 20.0);
}

#[test]
fn angled_text_uses_the_spreadsheetml_rotation() {
    let (painter, _) = paint(true);
    let ops = painter.ops();
    let rotation = ops
        .iter()
        .position(|op| matches!(op, Op::Rotate(angle) if (*angle + std::f64::consts::FRAC_PI_4).abs() < 0.000_001))
        .expect("45 degrees above the horizon");

    assert!(matches!(ops[rotation - 1], Op::Translate(_, _)));
    assert_eq!(ops[rotation + 1], Op::FillText("Tilt".into(), 0.0, 0.0));
}

#[test]
fn text_is_clipped_to_its_own_cell() {
    let (painter, layout) = paint(true);
    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    let ops = painter.ops();

    let rect = index_of(
        &ops,
        |op| matches!(op, Op::Rect(x, y, w, h) if *x == cell.rect.x && *y == cell.rect.y && *w == cell.rect.width && *h == cell.rect.height),
    );
    assert_eq!(ops[rect + 1], Op::Clip);
    // The clip is scoped, or every later cell would inherit it.
    assert!(ops[rect..].iter().any(|op| matches!(op, Op::Restore)));
}

#[test]
fn the_style_font_reaches_the_painter() {
    let (painter, _) = paint(true);
    assert!(painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::SetFont(font) if font == "bold 11.00pt 'Meiryo'")));
}

#[test]
fn a_merged_cell_is_drawn_once_across_its_range() {
    let (painter, layout) = paint(true);
    let anchor = layout.cell(CellRef::new(0, 1)).expect("A2");
    assert_eq!(anchor.rect.width, 128.0, "A2:B2 spans two default columns");

    let drawn = painter.count(|op| matches!(op, Op::FillText(text, _, _) if text == "merged"));
    assert_eq!(drawn, 1, "the covered cell must not repaint the text");
}

#[test]
fn hiding_grid_lines_removes_them_without_touching_the_rest() {
    let (with_lines, _) = paint(true);
    let (without_lines, _) = paint(false);

    let grid_strokes = |painter: &RecordingPainter| {
        painter.count(|op| matches!(op, Op::SetStrokeStyle(color) if color == "#d4d4d4"))
    };
    assert_eq!(grid_strokes(&with_lines), 1);
    assert_eq!(grid_strokes(&without_lines), 0);

    let texts = |painter: &RecordingPainter| painter.count(|op| matches!(op, Op::FillText(..)));
    assert_eq!(texts(&with_lines), texts(&without_lines));
}
