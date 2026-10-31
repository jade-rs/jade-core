//! Drawing a cell that wrapped.

use std::rc::Rc;

use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{
    layout_sheet, units, LayoutOptions, ResolvedFont, SheetLayout, TextMeasure,
};
use jade_xlsx_reader::{
    Alignment, Cell, CellFormat, CellRef, CellValue, ColumnSpan, Row, Sheet, StyleTable,
    VerticalAlignment,
};
use jade_xlsx_render::{render_sheet, RenderOptions};

struct Monospace;

impl TextMeasure for Monospace {
    fn measure(&self, text: &str, _font: &ResolvedFont) -> f64 {
        text.chars()
            .map(|c| if c.is_ascii() { 7.0 } else { 14.0 })
            .sum()
    }
}

fn wrapped_layout(vertical: VerticalAlignment) -> SheetLayout {
    wrapped_layout_with_height(vertical, None)
}

fn wrapped_layout_with_height(
    vertical: VerticalAlignment,
    pinned_height_points: Option<f32>,
) -> SheetLayout {
    let mut styles = StyleTable::default();
    styles.cell_formats.push(CellFormat {
        alignment: Alignment {
            wrap_text: true,
            vertical,
            ..Alignment::default()
        },
        ..Default::default()
    });

    let sheet = Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: vec![ColumnSpan {
            first: 0,
            last: 0,
            width_chars: Some(10.0),
            custom_width: true,
            hidden: false,
            style_index: None,
        }],
        rows: vec![Row {
            index: 0,
            height_points: pinned_height_points,
            custom_height: pinned_height_points.is_some(),
            custom_format: false,
            hidden: false,
            style_index: None,
            cells: vec![Cell {
                reference: CellRef::new(0, 0),
                value: CellValue::Text("hello there world".into()),
                style_index: Some(0),
                formula: None,
            }],
        }],
        merges: Vec::new(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: Vec::new(),
        conditional_formats: Vec::new(),
        auto_filter: None,
        filter_mode: None,
        filter_row_state: None,
        default_column_width_chars: None,
        default_row_height_points: Some(15.0),
        default_row_custom_height: false,
        default_rows_hidden: false,
        show_grid_lines: false,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        rich_text: Default::default(),
        tab_color: None,
        sparkline_groups: Vec::new(),
        outline: Default::default(),
        protection: None,
    };

    layout_sheet(
        &sheet,
        &styles,
        &LayoutOptions {
            measure: Rc::new(Monospace),
            ..LayoutOptions::default()
        },
    )
}

fn text_ops(layout: &SheetLayout) -> Vec<(String, f64, f64)> {
    let painter = RecordingPainter::with_width_per_char(7.0);
    render_sheet(&painter, layout, &RenderOptions::default()).expect("render");
    painter
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            Op::FillText(text, x, y) => Some((text, x, y)),
            _ => None,
        })
        .collect()
}

#[test]
fn every_wrapped_line_is_drawn_once() {
    let layout = wrapped_layout(VerticalAlignment::Top);
    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    let drawn: Vec<String> = text_ops(&layout)
        .into_iter()
        .map(|(text, _, _)| text)
        .collect();

    assert!(cell.lines.len() > 1, "the fixture must wrap");
    assert_eq!(drawn, cell.lines);
}

#[test]
fn lines_are_spaced_by_the_height_the_row_was_sized_for() {
    let layout = wrapped_layout(VerticalAlignment::Top);
    let ops = text_ops(&layout);

    // The spacing the renderer uses has to be the spacing layout grew the row
    // by, or a wrapped cell overflows the row it just sized.
    let expected = units::points_to_px(units::line_height_points(11.0));
    for pair in ops.windows(2) {
        assert!((pair[1].2 - pair[0].2 - expected).abs() < 0.001);
    }
    assert_eq!(
        layout.row(0).expect("row").size,
        expected * ops.len() as f64
    );
}

#[test]
fn vertical_alignment_positions_the_block_of_lines() {
    // A row with slack, so the two alignments have somewhere to differ.
    let top = text_ops(&wrapped_layout_with_height(
        VerticalAlignment::Top,
        Some(80.0),
    ));
    let bottom = text_ops(&wrapped_layout_with_height(
        VerticalAlignment::Bottom,
        Some(80.0),
    ));

    assert_eq!(top.len(), bottom.len());
    for (top_line, bottom_line) in top.iter().zip(bottom.iter()) {
        assert!(
            bottom_line.2 > top_line.2 + 10.0,
            "bottom-aligned text should sit lower"
        );
    }
}

#[test]
fn an_auto_fitted_row_keeps_bottom_aligned_text_inside_the_cell() {
    // The row is exactly as tall as its lines, so there is no room for the
    // padding to push the block down; it must not leave the cell.
    let layout = wrapped_layout(VerticalAlignment::Bottom);
    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    let ops = text_ops(&layout);

    let line_height = units::points_to_px(units::line_height_points(11.0));
    for (index, (_, _, baseline)) in ops.iter().enumerate() {
        assert!(*baseline >= cell.rect.y, "line {index} is above the cell");
        assert!(
            *baseline <= cell.rect.bottom() + line_height,
            "line {index} is below the cell"
        );
    }
}

#[test]
fn a_cell_that_does_not_wrap_still_draws_one_line() {
    let mut styles = StyleTable::default();
    styles.cell_formats.push(CellFormat::default());
    let sheet = Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: Vec::new(),
        rows: vec![Row {
            index: 0,
            height_points: None,
            custom_height: false,
            custom_format: false,
            hidden: false,
            style_index: None,
            cells: vec![Cell {
                reference: CellRef::new(0, 0),
                value: CellValue::Text("hello there world".into()),
                style_index: Some(0),
                formula: None,
            }],
        }],
        merges: Vec::new(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: Vec::new(),
        conditional_formats: Vec::new(),
        auto_filter: None,
        filter_mode: None,
        filter_row_state: None,
        default_column_width_chars: None,
        default_row_height_points: Some(15.0),
        default_row_custom_height: false,
        default_rows_hidden: false,
        show_grid_lines: false,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        rich_text: Default::default(),
        tab_color: None,
        sparkline_groups: Vec::new(),
        outline: Default::default(),
        protection: None,
    };
    let layout = layout_sheet(
        &sheet,
        &styles,
        &LayoutOptions {
            measure: Rc::new(Monospace),
            ..LayoutOptions::default()
        },
    );

    let drawn = text_ops(&layout);
    assert_eq!(drawn.len(), 1);
    assert_eq!(drawn[0].0, "hello there world");
}

#[test]
fn distributed_text_keeps_grapheme_clusters_in_one_paint_call() {
    use jade_xlsx_reader::HorizontalAlignment;

    for cluster in ["👩‍💻", "👍🏽", "🇯🇵", "कि", "a\u{20dd}", "か\u{3099}"] {
        let mut layout = wrapped_layout(VerticalAlignment::Top);
        let cell = &mut layout.cells[0];
        cell.text = format!("{cluster}X");
        cell.lines = vec![cell.text.clone()];
        cell.rect.width = 200.0;
        cell.alignment.horizontal = HorizontalAlignment::Distributed;
        let drawn: Vec<_> = text_ops(&layout)
            .into_iter()
            .map(|(text, _, _)| text)
            .collect();
        assert_eq!(drawn, [cluster, "X"], "split a grapheme: {cluster}");
    }
}
