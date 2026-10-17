use jade_xlsx_layout::{
    layout_sheet, navigation::move_cell, LayoutOptions, Rect, ResolvedFont, TextMeasure,
};
use jade_xlsx_reader::{read_xlsx, CellRef, FreezePane, Sheet, StyleTable};
use std::rc::Rc;

struct Monospace;
impl TextMeasure for Monospace {
    fn measure(&self, text: &str, _: &ResolvedFont) -> f64 {
        text.chars().count() as f64 * 10.0
    }
}

fn fixture() -> (Sheet, StyleTable, jade_xlsx_layout::SheetLayout) {
    let mut workbook = read_xlsx(include_bytes!(
        "../../../public/fixtures/excel-grid-compat.xlsx"
    ))
    .unwrap();
    let sheet = workbook.sheets.remove(0);
    let layout = layout_sheet(
        &sheet,
        &workbook.styles,
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            measure: Rc::new(Monospace),
            ..Default::default()
        },
    );
    (sheet, workbook.styles, layout)
}

fn reference(value: &str) -> CellRef {
    CellRef::parse(value).unwrap()
}

#[test]
fn left_text_overflows_only_until_the_first_occupied_cell() {
    let (_, _, layout) = fixture();
    let cell = layout.cell(reference("A1")).unwrap();
    assert_eq!(cell.rect.width, 64.0);
    assert_eq!(cell.text_rect().right(), 192.0);
    let formula = layout.cell(reference("A8")).unwrap();
    assert_eq!(formula.text_rect().right(), 192.0);
    // An explicitly stored blank does not block; a formula returning "" does.
    assert_eq!(
        layout.cell(reference("A2")).unwrap().text_rect().right(),
        128.0
    );
}

#[test]
fn narrow_numbers_use_scientific_rounding_or_hashes_without_changing_values() {
    let (sheet, _, layout) = fixture();
    let large = layout.cell(reference("A6")).unwrap();
    assert_eq!(large.lines, ["1E+14"]);
    assert_eq!(large.text, "123456789012345");
    let fixed = layout.cell(reference("B6")).unwrap();
    assert_eq!(fixed.lines, ["#####"]);
    assert_eq!(fixed.text, "123456789012345.00");
    assert_eq!(layout.cell(reference("C6")).unwrap().lines, ["12.35"]);
    assert_eq!(layout.cell(reference("D6")).unwrap().lines, ["#####"]);
    assert_eq!(layout.cell(reference("D6")).unwrap().text, "03-15-23");
    assert_eq!(
        sheet.rows.iter().find(|row| row.index == 5).unwrap().cells[0].value,
        jade_xlsx_reader::CellValue::Number(123456789012345.0)
    );
}

#[test]
fn right_and_center_alignment_keep_the_original_alignment_box() {
    let (_, _, layout) = fixture();
    let right = layout.cell(reference("C3")).unwrap();
    assert_eq!(right.text_rect().x, 64.0);
    assert_eq!(right.text_rect().right(), right.rect.right());
    let center = layout.cell(reference("C4")).unwrap();
    assert_eq!(center.text_rect().x, 64.0);
    assert_eq!(center.text_rect().right(), 256.0);
}

#[test]
fn wrapping_numbers_merges_shrinking_and_rotation_do_not_overflow() {
    let (_, _, layout) = fixture();
    for address in ["A5", "A6", "A7", "A13", "A14"] {
        assert_eq!(
            layout.cell(reference(address)).unwrap().text_clip,
            None,
            "{address}"
        );
    }
    // A blank merged neighbour still blocks overflow.
    assert_eq!(
        layout.cell(reference("A15")).unwrap().text_rect().right(),
        128.0
    );
}

#[test]
fn overflow_from_an_offscreen_anchor_remains_visible_without_changing_hit_testing() {
    let (_, _, layout) = fixture();
    let viewport = Rect {
        x: 64.0,
        y: 0.0,
        width: 64.0,
        height: 20.0,
    };
    assert_eq!(layout.cells_in(viewport).count(), 0);
    assert_eq!(
        layout
            .text_cells_in(viewport)
            .map(|cell| cell.reference)
            .collect::<Vec<_>>(),
        vec![reference("A1")]
    );
    assert_eq!(layout.cell_at(80.0, 10.0), Some(reference("B1")));
}

#[test]
fn movement_leaves_and_enters_merges_at_their_edges() {
    let (sheet, _, layout) = fixture();
    let move_from = |from: &str, dc, dr| move_cell(&sheet, &layout, reference(from), dc, dr);
    assert_eq!(move_from("A11", 1, 0), reference("D11"));
    assert_eq!(move_from("D11", -1, 0), reference("A11"));
    assert_eq!(move_from("A11", 0, 1), reference("A13"));
    assert_eq!(move_from("B13", 0, -1), reference("A11"));
    assert_eq!(move_from("A11", 0, -1), reference("A10"));
    assert_eq!(move_from("A11", -1, 0), reference("A11"));
}

#[test]
fn blank_merges_have_a_surface_and_hit_target_even_without_a_stored_row() {
    let (sheet, _, layout) = fixture();
    for address in ["C15", "G18"] {
        let anchor = reference(address);
        let cell = layout.cell(anchor).unwrap();
        assert!(cell.text.is_empty());
        assert_eq!(cell.rect.width, 128.0);
        assert_eq!(
            layout.cell_at(cell.rect.right() - 2.0, cell.rect.bottom() - 2.0),
            Some(anchor)
        );
        let end = cell.merge.unwrap().end;
        assert_eq!(
            move_cell(&sheet, &layout, end, 1, 0),
            CellRef::new(end.column + 1, anchor.row)
        );
    }
}

#[test]
fn movement_skips_hidden_and_zero_size_axes_and_clamps_at_boundaries() {
    let (sheet, _, layout) = fixture();
    let move_from = |from: &str, dc, dr| move_cell(&sheet, &layout, reference(from), dc, dr);
    assert_eq!(move_from("E10", 1, 0), reference("G10"));
    assert_eq!(move_from("G10", -1, 0), reference("E10"));
    assert_eq!(move_from("B8", 0, 1), reference("B10"));
    assert_eq!(move_from("B10", 0, -1), reference("B8"));
    assert_eq!(move_from("B15", 0, 1), reference("B17"));
    assert_eq!(move_from("A1", -1, -1), reference("A1"));
    assert_eq!(move_from("L30", 1, 1), reference("L30"));
}

#[test]
fn revealing_a_cell_scrolls_minimally_and_is_stable_for_an_oversized_merge() {
    let (_, _, layout) = fixture();
    let window = Rect {
        x: 0.0,
        y: 0.0,
        width: 128.0,
        height: 100.0,
    };
    assert_eq!(layout.scroll_to_reveal(reference("B2"), window), (0.0, 0.0));
    let rect = layout.cell_rect(reference("E10")).unwrap();
    assert_eq!(
        layout.scroll_to_reveal(reference("E10"), window),
        (
            (rect.right() - 128.0).ceil(),
            (rect.bottom() - 100.0).ceil()
        )
    );
    let scrolled = Rect {
        x: 128.0,
        y: 200.0,
        ..window
    };
    assert_eq!(
        layout.scroll_to_reveal(reference("A1"), scrolled),
        (0.0, 0.0)
    );
    let small = Rect {
        width: 64.0,
        height: 20.0,
        ..scrolled
    };
    let (x, y) = layout.scroll_to_reveal(reference("A11"), small);
    assert_eq!(x, 0.0);
    assert_eq!(y, layout.cell_rect(reference("A11")).unwrap().y.floor());
    assert_eq!(
        layout.scroll_to_reveal(reference("A11"), Rect { x, y, ..small }),
        (x, y)
    );
}

#[test]
fn revealing_and_positioning_cells_respect_each_frozen_axis() {
    let (mut sheet, styles, _) = fixture();
    sheet.freeze_pane = Some(FreezePane {
        frozen_columns: 1,
        frozen_rows: 2,
        top_left_cell: Some(reference("B3")),
    });
    let layout = layout_sheet(
        &sheet,
        &styles,
        &LayoutOptions {
            min_column_count: 12,
            min_row_count: 30,
            ..Default::default()
        },
    );
    let viewport = Rect {
        x: 128.0,
        y: 120.0,
        width: 192.0,
        height: 100.0,
    };
    assert_eq!(
        layout.scroll_to_reveal(reference("A1"), viewport),
        (128.0, 120.0)
    );
    let rect = layout
        .cell_rect_in_viewport(reference("A1"), 128.0, 120.0)
        .unwrap();
    assert_eq!((rect.x, rect.y), (0.0, 0.0));
    assert_eq!(
        layout
            .cell_rect_in_viewport(reference("B1"), 128.0, 120.0)
            .unwrap()
            .y,
        0.0
    );
    assert_eq!(
        layout
            .cell_rect_in_viewport(reference("A10"), 128.0, 120.0)
            .unwrap()
            .x,
        0.0
    );
    // B3 must appear immediately beyond the frozen bands when returning from a scroll.
    assert_eq!(
        layout.scroll_to_reveal(reference("B3"), viewport),
        (0.0, 0.0)
    );
    let distant = layout.cell_rect(reference("G18")).unwrap();
    assert_eq!(
        layout.scroll_to_reveal(reference("G18"), viewport),
        (
            (distant.right() - 192.0).ceil(),
            (distant.bottom() - 100.0).ceil()
        )
    );
}
