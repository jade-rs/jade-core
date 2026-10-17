use jade_pptx_layout::{
    layout_deck_slide, resolve_transform, text_format_for_deck_shape, HeuristicMeasure,
    InheritedContent, PlacedGeometry, PlacedOrigin,
};
use jade_pptx_reader::{parse_slide_xml, Autofit, ShapeItem, TextBody, TextBodyPropertyPresence};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/inherited-deck.pptx");
fn own(deck: &jade_pptx_reader::Deck, slide: usize) -> jade_pptx_layout::PlacedShape {
    layout_deck_slide(deck, slide, &HeuristicMeasure)
        .unwrap()
        .shapes
        .into_iter()
        .find(|s| s.origin == PlacedOrigin::Slide && s.source_id == Some(2))
        .unwrap()
}
#[test]
fn omitted_body_properties_and_shape_properties_inherit_from_master_without_copying_prompt_text() {
    let d = jade_pptx_reader::read_deck(DECK).unwrap();
    let s = own(&d, 0);
    assert_eq!(
        (s.rect.x, s.rect.y, s.rect.width, s.rect.height),
        (100.0, 100.0, 400.0, 180.0)
    );
    assert_eq!(s.fill.as_deref(), Some("#E6A010"));
    assert!(matches!(s.geometry, PlacedGeometry::RoundRect { .. }));
    assert_eq!(s.text_rotation_deg, 270.0);
    assert_eq!(s.lines.len(), 1);
    assert_eq!(s.lines[0].runs[0].size_pt, 15.0);
    assert_eq!(s.lines[0].runs[0].font.as_deref(), Some("serif"));
    assert_eq!(s.lines[0].runs[0].color, "#A020F0");
    let line = s.outline.unwrap();
    assert_eq!(line.color, "#1414F0");
    assert!((line.width_px - 16.0 / 3.0).abs() < 1e-8);
    assert!(!line.dash_px.is_empty());
    let state = text_format_for_deck_shape(&d, 0, 2, 0, 10).unwrap();
    assert_eq!(state.size_pt, Some(30.0));
    assert_eq!(state.color.as_deref(), Some("#A020F0"));
}
#[test]
fn explicit_horizontal_wrap_and_no_autofit_no_fill_no_line_block_inherited_values() {
    let d = jade_pptx_reader::read_deck(DECK).unwrap();
    let s = own(&d, 1);
    assert_eq!(s.text_rotation_deg, 0.0);
    assert!(s.lines.len() > 1);
    assert_eq!(s.lines[0].runs[0].size_pt, 30.0);
    assert!(matches!(s.geometry, PlacedGeometry::Rect));
    assert!(s.fill.is_none());
    assert!(s.outline.is_none());
}
#[test]
fn partial_outline_inherits_color_and_dash_while_local_width_wins() {
    let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
    let ShapeItem::Shape(shape) = &mut d.slides[0].content.shapes[0] else {
        panic!("shape")
    };
    shape.outline = Some(jade_pptx_reader::Outline {
        width_emu: Some(25400),
        ..Default::default()
    });
    let s = own(&d, 0);
    let l = s.outline.unwrap();
    assert_eq!(l.color, "#1414F0");
    assert!((l.width_px - 8.0 / 3.0).abs() < 1e-8);
    assert!(!l.dash_px.is_empty());
}
#[test]
fn picture_placeholder_frame_comes_from_a_layout_picture_and_matches_the_public_transform_query() {
    let d = jade_pptx_reader::read_deck(DECK).unwrap();
    let s = own(&d, 2);
    assert_eq!(
        (s.rect.x, s.rect.y, s.rect.width, s.rect.height),
        (650.0, 100.0, 300.0, 180.0)
    );
    assert_eq!(s.image_part.as_deref(), Some("ppt/media/inherited.png"));
    assert_eq!(s.source_rect.unwrap().left, 10000);
    let ShapeItem::Picture(pic) = &d.slides[2].content.shapes[0] else {
        panic!("picture")
    };
    let t = resolve_transform(
        pic.transform,
        pic.placeholder.as_ref(),
        &InheritedContent {
            layout: d.slides[2].layout_content.as_ref(),
            master: d.slides[2].master_content.as_ref(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(t.offset_emu, (650 * 9525, 100 * 9525));
    assert_eq!(t.extent_emu, (300 * 9525, 180 * 9525));
}
#[test]
fn reader_preserves_presence_and_legacy_models_keep_explicit_values() {
    for strict in [false, true] {
        let ns = if strict {
            "http://purl.oclc.org/ooxml"
        } else {
            "http://schemas.openxmlformats.org"
        };
        let (p, a) = if strict {
            (
                format!("{ns}/presentationml/main"),
                format!("{ns}/drawingml/main"),
            )
        } else {
            (
                format!("{ns}/presentationml/2006/main"),
                format!("{ns}/drawingml/2006/main"),
            )
        };
        for (body, presence) in [
            ("<a:bodyPr/>", TextBodyPropertyPresence::default()),
            (
                "<a:bodyPr wrap=\"square\" vert=\"horz\"><a:noAutofit/></a:bodyPr>",
                TextBodyPropertyPresence {
                    wrap: true,
                    autofit: true,
                    vertical: true,
                },
            ),
        ] {
            let xml=format!("<p:sld xmlns:p=\"{p}\" xmlns:a=\"{a}\"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"t\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody>{body}<a:p/></p:txBody></p:sp></p:spTree></p:cSld></p:sld>");
            let content = parse_slide_xml(&xml).unwrap();
            let ShapeItem::Shape(s) = &content.shapes[0] else {
                panic!("shape")
            };
            let t = s.text.as_ref().unwrap();
            assert_eq!(t.body_property_presence, Some(presence));
            assert_eq!(t.autofit, Autofit::None);
            assert!(t.vertical.is_none());
        }
    }
    let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
    let ShapeItem::Shape(shape) = &mut d.slides[0].content.shapes[0] else {
        panic!("shape")
    };
    shape.text.as_mut().unwrap().body_property_presence = None;
    let s = own(&d, 0);
    assert_eq!(s.text_rotation_deg, 0.0);
    assert_eq!(s.lines[0].runs[0].size_pt, 30.0);
    assert!(s.lines.len() > 1);
    assert!(TextBody::default().body_property_presence.is_none());
}

#[test]
fn inherited_shape_autofit_grows_the_native_frame_and_explicit_no_autofit_stops_growth() {
    let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
    let ShapeItem::Shape(master) = &mut d.slides[0].master_content.as_mut().unwrap().shapes[0]
    else {
        panic!("master")
    };
    master.transform.as_mut().unwrap().extent_emu.1 = 20 * 9525;
    let text = master.text.as_mut().unwrap();
    text.autofit = Autofit::Shape;
    text.wrap = true;
    text.vertical = None;
    let grown = own(&d, 0);
    assert_eq!(grown.text_rotation_deg, 0.0);
    assert!(grown.rect.height > 20.0);
    assert!(grown.lines.len() > 1);
    let ShapeItem::Shape(shape) = &mut d.slides[0].content.shapes[0] else {
        panic!("shape")
    };
    let text = shape.text.as_mut().unwrap();
    text.body_property_presence = Some(TextBodyPropertyPresence {
        autofit: true,
        ..Default::default()
    });
    text.autofit = Autofit::None;
    assert_eq!(own(&d, 0).rect.height, 20.0);
}
#[test]
fn inherited_image_fill_keeps_its_own_part_and_explicit_no_fill_blocks_it() {
    let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
    let ShapeItem::Shape(master) = &mut d.slides[0].master_content.as_mut().unwrap().shapes[0]
    else {
        panic!("master")
    };
    master.fill = None;
    master.image_rel_id = Some("master-image".into());
    master.image_part = Some("ppt/media/inherited.png".into());
    master.source_rect = Some(jade_pptx_reader::SourceRect {
        left: 10000,
        ..Default::default()
    });
    let image = own(&d, 0);
    assert_eq!(image.image_part.as_deref(), Some("ppt/media/inherited.png"));
    assert_eq!(image.source_rect.unwrap().left, 10000);
    assert!(image.fill.is_none());
    let ShapeItem::Shape(shape) = &mut d.slides[0].content.shapes[0] else {
        panic!("shape")
    };
    shape.fill = Some(jade_pptx_reader::Fill::None);
    let none = own(&d, 0);
    assert!(none.image_part.is_none());
    assert!(none.fill.is_none());
}

#[test]
fn omitted_index_is_zero_and_does_not_select_an_earlier_same_type_layout_slot() {
    let d = jade_pptx_reader::read_deck(DECK).unwrap();
    let ShapeItem::Shape(title) = &d.slides[2].content.shapes[1] else {
        panic!("title")
    };
    assert!(title.placeholder.as_ref().unwrap().index.is_none());
    let placed = layout_deck_slide(&d, 2, &HeuristicMeasure)
        .unwrap()
        .shapes
        .into_iter()
        .find(|s| s.origin == PlacedOrigin::Slide && s.source_id == Some(3))
        .unwrap();
    assert_eq!(
        (
            placed.rect.x,
            placed.rect.y,
            placed.rect.width,
            placed.rect.height
        ),
        (100.0, 400.0, 400.0, 100.0)
    );
    assert_eq!(placed.fill.as_deref(), Some("#1414F0"));
}
#[test]
fn master_type_wins_over_colliding_indexes_and_content_roles_use_the_body_family() {
    for kind in [
        "body", "obj", "pic", "chart", "tbl", "dgm", "media", "clipArt", "subTitle", "",
        "ctrTitle", "title",
    ] {
        let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
        let ShapeItem::Shape(source) = &mut d.slides[0].content.shapes[0] else {
            panic!("shape")
        };
        source.placeholder.as_mut().unwrap().kind = kind.into();
        let ShapeItem::Shape(layout) = &mut d.slides[0].layout_content.as_mut().unwrap().shapes[0]
        else {
            panic!("layout")
        };
        layout.placeholder.as_mut().unwrap().kind = kind.into();
        let placed = own(&d, 0);
        let title = matches!(kind, "title" | "ctrTitle");
        assert_eq!(placed.rect.x, if title { 20.0 } else { 100.0 }, "{kind}");
        assert_eq!(
            placed.fill.as_deref(),
            Some(if title { "#FF0000" } else { "#E6A010" }),
            "{kind}"
        );
    }
}
#[test]
fn missing_or_unbound_layout_index_never_borrows_a_different_slot_of_the_same_type() {
    for index in [99, u32::MAX] {
        let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
        let ShapeItem::Shape(source) = &mut d.slides[0].content.shapes[0] else {
            panic!("shape")
        };
        source.placeholder.as_mut().unwrap().index = Some(index);
        assert!(!layout_deck_slide(&d, 0, &HeuristicMeasure)
            .unwrap()
            .shapes
            .iter()
            .any(|s| s.origin == PlacedOrigin::Slide && s.source_id == Some(2)));
        let ShapeItem::Shape(source) = &mut d.slides[0].content.shapes[0] else {
            panic!("shape")
        };
        source.transform = Some(jade_pptx_reader::Transform {
            offset_emu: (50 * 9525, 50 * 9525),
            extent_emu: (300 * 9525, 100 * 9525),
            rotation_60000ths: 0,
            flip_h: false,
            flip_v: false,
        });
        let placed = own(&d, 0);
        assert_eq!(placed.rect.x, 50.0);
        assert!(placed.fill.is_none());
        let query = text_format_for_deck_shape(&d, 0, 2, 0, 10).unwrap();
        assert_eq!(query.size_pt, Some(18.0));
        assert_eq!(query.font, d.theme_for_slide(0).fonts.minor_latin);
    }
}
#[test]
fn ambiguous_layout_indexes_or_master_types_do_not_choose_an_arbitrary_definition() {
    for master in [false, true] {
        let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
        let content = if master {
            d.slides[0].master_content.as_mut().unwrap()
        } else {
            d.slides[0].layout_content.as_mut().unwrap()
        };
        let mut copy = content.shapes[0].clone();
        if let ShapeItem::Shape(s) = &mut copy {
            s.id = Some(123);
            if master {
                s.placeholder.as_mut().unwrap().index = Some(42);
            }
        }
        content.shapes.push(copy);
        assert!(!layout_deck_slide(&d, 0, &HeuristicMeasure)
            .unwrap()
            .shapes
            .iter()
            .any(|s| s.origin == PlacedOrigin::Slide && s.source_id == Some(2)));
    }
}

#[test]
fn index_only_slide_placeholder_uses_the_matched_layout_role_for_master_text_styles() {
    use jade_pptx_reader::{LevelStyle, ListStyle, RunProps, TextStyles};
    let mut d = jade_pptx_reader::read_deck(DECK).unwrap();
    let ShapeItem::Shape(source) = &mut d.slides[2].content.shapes[1] else {
        panic!("title")
    };
    source.placeholder.as_mut().unwrap().kind.clear();
    let styles = d.slides[2]
        .master_content
        .as_mut()
        .unwrap()
        .text_styles
        .get_or_insert_with(TextStyles::default);
    styles.title = ListStyle {
        levels: vec![Some(LevelStyle {
            default_run: RunProps {
                size_pt: Some(50.0),
                ..Default::default()
            },
            ..Default::default()
        })],
    };
    styles.body = ListStyle {
        levels: vec![Some(LevelStyle {
            default_run: RunProps {
                size_pt: Some(12.0),
                ..Default::default()
            },
            ..Default::default()
        })],
    };
    let query = text_format_for_deck_shape(&d, 2, 3, 0, 5).unwrap();
    assert_eq!(query.size_pt, Some(50.0));
    let placed = layout_deck_slide(&d, 2, &HeuristicMeasure)
        .unwrap()
        .shapes
        .into_iter()
        .find(|s| s.origin == PlacedOrigin::Slide && s.source_id == Some(3))
        .unwrap();
    assert_eq!(placed.lines[0].runs[0].size_pt, 50.0);
}
