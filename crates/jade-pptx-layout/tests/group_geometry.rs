use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure};
use jade_pptx_reader::{read_deck, ShapeItem};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/grouped-deck.pptx");
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}
#[test]
fn nested_groups_keep_local_layout_and_map_rotation_flips_nonuniform_scale_and_hits() {
    let mut deck = read_deck(DECK).unwrap();
    let ShapeItem::Group(group) = &mut deck.slides[0].content.shapes[0] else {
        panic!("group");
    };
    let outer = &mut group.transform.as_mut().unwrap().outer;
    outer.extent_emu = (800 * 9525, 300 * 9525);
    outer.rotation_60000ths = 90 * 60000;
    outer.flip_h = true;
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let red = layout
        .shapes
        .iter()
        .find(|s| s.group_ids == [50, 51])
        .unwrap();
    assert_eq!(red.paint_rect().width, 120.0);
    assert_eq!(red.paint_rect().height, 80.0);
    let center = red.paint_to_slide((60.0, 40.0));
    near(center.0, 760.0);
    near(center.1, 590.0);
    assert!(red.contains_point(center.0, center.1, 0.0));
    assert!(!red.contains_point(380.0, 260.0, 0.0));
    let point = red.paint_to_slide((90.0, 20.0));
    // Cumulative axes: 2x/1.5x with H/V flips, then 120 degrees, no shear.
    near(point.0, 760.0 + 30.0 - 30.0 * (3.0f64).sqrt() / 2.0);
    near(point.1, 590.0 - 30.0 * (3.0f64).sqrt() - 15.0);
    let restored = red.slide_to_paint(point).unwrap();
    near(restored.0, 90.0);
    near(restored.1, 20.0);
    let blue = layout
        .shapes
        .iter()
        .find(|s| s.name.starts_with("Picture") && s.group_ids == [50])
        .unwrap();
    let center = blue.paint_to_slide((260.0, 140.0));
    near(center.0, 640.0);
    near(center.1, 230.0);
    assert_eq!(layout.groups[0].id, 50);
    near(layout.groups[0].transform.rotation_degrees, 90.0);
}
#[test]
fn group_resize_scales_text_at_paint_time_without_rewrapping_and_caret_hit_roundtrips() {
    let mut deck = read_deck(DECK).unwrap();
    let before = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let ShapeItem::Group(group) = &mut deck.slides[0].content.shapes[0] else {
        panic!("group");
    };
    let outer = &mut group.transform.as_mut().unwrap().outer;
    outer.extent_emu = (800 * 9525, 400 * 9525);
    outer.rotation_60000ths = 30 * 60000;
    outer.flip_v = true;
    let after = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let old = before.shapes.iter().find(|s| !s.lines.is_empty()).unwrap();
    let text = after.shapes.iter().find(|s| !s.lines.is_empty()).unwrap();
    assert_eq!(old.lines, text.lines);
    let caret = text
        .selection_geometry(3, 3, 3, false, &HeuristicMeasure)
        .unwrap()
        .caret;
    let hit = text
        .hit_text(
            (caret[0].x + caret[1].x) / 2.0,
            (caret[0].y + caret[1].y) / 2.0,
            &HeuristicMeasure,
        )
        .unwrap();
    assert_eq!(hit.offset, 3);
}
