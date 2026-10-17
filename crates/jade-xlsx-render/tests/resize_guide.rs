use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{
    layout_sheet,
    resize::{ResizeGesture, ResizeHit},
    LayoutOptions, Rect,
};
use jade_xlsx_reader::{read_xlsx, Axis};
#[test]
fn resize_guide_consumes_shared_layout_coordinates() {
    let book = read_xlsx(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let layout = layout_sheet(&book.sheets[0], &book.styles, &LayoutOptions::default());
    let painter = RecordingPainter::default();
    jade_xlsx_render::render_resize_guide(
        &painter,
        &layout,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 500.0,
            height: 300.0,
        },
        ResizeGesture {
            hit: ResizeHit {
                axis: Axis::Columns,
                index: 0,
                size_px: 64.0,
            },
            pointer_start: 88.0,
            size_px: 100.0,
        },
    );
    assert!(painter
        .ops()
        .contains(&Op::FillRect(124.0, 0.0, 1.0, 320.0)));
}

#[test]
fn hidden_default_rows_do_not_paint_text_while_explicit_reveals_do() {
    let sheet = jade_xlsx_reader::sheet::parse_sheet("Sheet", "sheet.xml", r#"<worksheet><sheetFormatPr defaultRowHeight="25" zeroHeight="1"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Hidden</t></is></c></row><row r="3" hidden="0"><c r="A3" t="inlineStr"><is><t>Visible</t></is></c></row></sheetData></worksheet>"#, &[]).unwrap();
    let layout = layout_sheet(
        &sheet,
        &Default::default(),
        &LayoutOptions {
            min_row_count: 30,
            ..Default::default()
        },
    );
    let painter = RecordingPainter::default();
    jade_xlsx_render::render_sheet(&painter, &layout, &Default::default());
    let texts: Vec<_> = painter
        .ops()
        .iter()
        .filter_map(|op| match op {
            Op::FillText(text, _, _) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|text| text == "Visible"));
    assert!(!texts.iter().any(|text| text == "Hidden"));
}
