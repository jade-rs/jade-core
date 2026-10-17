use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure, PlacedOrigin};
use jade_pptx_reader::{Color, Outline, ShapeItem};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
fn stroke(d: &jade_pptx_reader::Deck, slide: usize) -> Option<jade_pptx_layout::PlacedOutline> {
    layout_deck_slide(d, slide, &HeuristicMeasure)
        .unwrap()
        .shapes
        .into_iter()
        .find(|s| s.source_id == Some(4) && s.origin == PlacedOrigin::Slide)
        .unwrap()
        .outline
}
#[test]
fn each_master_and_override_line_matrix_supplies_width_dash_and_placeholder_color() {
    let d = jade_pptx_reader::read_deck(DECK).unwrap();
    for (i, w, color, dash) in [
        (0, 8.0, "#F01414", vec![24.0, 8.0]),
        (
            1,
            40.0 / 3.0,
            "#735008",
            vec![160.0 / 3.0, 40.0, 40.0 / 3.0, 40.0],
        ),
        (
            2,
            40.0 / 3.0,
            "#501078",
            vec![160.0 / 3.0, 40.0, 40.0 / 3.0, 40.0],
        ),
        (3, 16.0 / 3.0, "#F014C8", vec![]),
    ] {
        let s = stroke(&d, i).unwrap();
        assert_eq!(s.color, color);
        assert!((s.width_px - w).abs() < 1e-8);
        assert_eq!(s.dash_px.len(), dash.len());
        for (a, b) in s.dash_px.iter().zip(dash) {
            assert!((*a - b).abs() < 1e-8);
        }
    }
}
#[test]
fn local_color_width_and_explicit_solid_override_the_matrix_independently() {
    let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
    let ShapeItem::Shape(s) = &mut d.slides[0].content.shapes[2] else {
        panic!("shape")
    };
    s.outline = Some(Outline {
        color: Some(Color::Srgb("0000FF".into())),
        dash: Some("solid".into()),
        ..Default::default()
    });
    let line = stroke(&d, 0).unwrap();
    assert_eq!(line.color, "#0000FF");
    assert_eq!(line.width_px, 8.0);
    assert!(line.dash_px.is_empty());
    let ShapeItem::Shape(s) = &mut d.slides[0].content.shapes[2] else {
        panic!("shape")
    };
    s.outline = Some(Outline {
        width_emu: Some(25400),
        ..Default::default()
    });
    let line = stroke(&d, 0).unwrap();
    assert_eq!(line.color, "#F01414");
    assert!((line.width_px - 8.0 / 3.0).abs() < 1e-8);
    assert_eq!(line.dash_px.len(), 2);
}
#[test]
fn zero_missing_and_no_fill_entries_stop_theme_lines_but_explicit_color_can_override_no_fill() {
    let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
    for idx in [0, 3, 99] {
        let ShapeItem::Shape(s) = &mut d.slides[0].content.shapes[2] else {
            panic!("shape")
        };
        s.style.as_mut().unwrap().line_index = idx;
        assert!(stroke(&d, 0).is_none());
    }
    let ShapeItem::Shape(s) = &mut d.slides[0].content.shapes[2] else {
        panic!("shape")
    };
    s.style.as_mut().unwrap().line_index = 3;
    s.outline = Some(Outline {
        color: Some(Color::Srgb("123456".into())),
        ..Default::default()
    });
    let line = stroke(&d, 0).unwrap();
    assert_eq!(line.color, "#123456");
    assert_eq!(line.width_px, 12.0);
    let ShapeItem::Shape(s) = &mut d.slides[0].content.shapes[2] else {
        panic!("shape")
    };
    s.outline.as_mut().unwrap().no_fill = true;
    assert!(stroke(&d, 0).is_none());
}
