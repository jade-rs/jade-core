use jade_opc::OpcPackage;
use jade_painter::recording::{Op, RecordingPainter};
use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure, PlacedGeometry};
use jade_pptx_reader::Transform;
use jade_pptx_render::{render_slide, RenderOptions};
use jade_pptx_writer::{
    editable_shapes, shape_text, EditSession, ShapeInsert, ShapePreset, TextFormat,
};

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
const PRESETS: [ShapePreset; 6] = [
    ShapePreset::Rect,
    ShapePreset::RoundRect,
    ShapePreset::Ellipse,
    ShapePreset::Triangle,
    ShapePreset::Diamond,
    ShapePreset::Hexagon,
];
fn bounds() -> Transform {
    Transform {
        offset_emu: (952500, 952500),
        extent_emu: (3048000, 1524000),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}
fn value(preset: ShapePreset) -> ShapeInsert {
    ShapeInsert {
        preset,
        text: "日本語 & <shape> 🦀\n\n次の段落\n".into(),
        format: TextFormat {
            size_pt: Some(24.0),
            bold: Some(true),
            ..Default::default()
        },
        fill: Some("#12ab34".into()),
        outline: Some("#AB1234".into()),
        outline_width_pt: Some(2.25),
    }
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}
fn with_xml(value: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(DECK).unwrap();
    p.replace("ppt/slides/slide1.xml", value.as_bytes().to_vec());
    p.write().unwrap()
}

#[test]
fn each_preset_preserves_original_parts_and_has_editable_text_and_one_undo() {
    for preset in PRESETS {
        let mut session = EditSession::open(DECK).unwrap();
        let value = value(preset);
        let id = session.insert_shape(0, bounds(), &value).unwrap();
        let inserted = session.bytes().to_vec();
        let text = shape_text(&inserted, 0, id).unwrap();
        assert!(text.can_edit);
        assert_eq!(text.text, value.text);
        let objects = editable_shapes(&inserted, 0).unwrap();
        let shape = objects.last().unwrap();
        assert_eq!(shape.id, id);
        assert_eq!(shape.transform, Some(bounds()));
        assert!(shape.can_group && shape.can_duplicate && shape.can_delete);
        let changed = xml(&inserted);
        let start = changed.find("<jadeSp:sp ").unwrap();
        let end = start + changed[start..].find("</jadeSp:sp>").unwrap() + "</jadeSp:sp>".len();
        assert_eq!(
            format!("{}{}", &changed[..start], &changed[end..]),
            xml(DECK)
        );
        assert!(changed.contains(&format!("prst=\"{}\"", preset.as_str())));
        assert!(changed[start..end].contains("anchor=\"ctr\""));
        assert!(changed[start..end].contains("algn=\"ctr\""));
        assert!(!changed[start..end].contains("txBox="));
        let before = OpcPackage::open(DECK).unwrap();
        let after = OpcPackage::open(&inserted).unwrap();
        assert_eq!(before.part_names(), after.part_names());
        for part in before
            .part_names()
            .iter()
            .filter(|p| p.as_str() != "ppt/slides/slide1.xml")
        {
            assert_eq!(before.part(part), after.part(part), "{part}");
        }
        assert!(session.undo());
        assert_eq!(session.bytes(), DECK);
        assert!(!session.undo());
        assert!(session.redo());
        assert_eq!(session.bytes(), inserted);
        assert_eq!(
            shape_text(EditSession::open(&inserted).unwrap().bytes(), 0, id)
                .unwrap()
                .text,
            value.text
        );
    }
}

#[test]
fn inserted_shapes_render_their_native_geometry_fill_outline_and_text() {
    for preset in PRESETS {
        let mut session = EditSession::open(DECK).unwrap();
        let id = session.insert_shape(0, bounds(), &value(preset)).unwrap();
        let deck = jade_pptx_reader::read_deck(session.bytes()).unwrap();
        let mut layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
        layout.shapes.retain(|shape| shape.source_id == Some(id));
        let shape = &layout.shapes[0];
        assert_eq!(shape.fill.as_deref(), Some("#12AB34"));
        assert_eq!(shape.outline.as_ref().unwrap().width_px, 3.0);
        assert_eq!(shape.outline.as_ref().unwrap().color, "#AB1234");
        assert!(!shape.lines.is_empty());
        match preset {
            ShapePreset::Rect => assert_eq!(shape.geometry, PlacedGeometry::Rect),
            ShapePreset::Ellipse => assert_eq!(shape.geometry, PlacedGeometry::Ellipse),
            ShapePreset::RoundRect => {
                assert!(matches!(shape.geometry, PlacedGeometry::RoundRect { .. }))
            }
            _ => assert!(matches!(shape.geometry, PlacedGeometry::Polygon(_))),
        }
        let painter = RecordingPainter::new();
        render_slide(&painter, &layout, &RenderOptions::default()).unwrap();
        let ops = painter.ops();
        for line in &shape.lines {
            for run in &line.runs {
                assert!(ops.contains(&Op::FillText(
                    run.text.clone(),
                    shape.rect.x + run.x,
                    shape.rect.y + run.baseline_y
                )));
            }
        }
        assert!(ops.contains(&Op::SetFillStyle("#12AB34".into())));
        assert!(ops.contains(&Op::SetStrokeStyle("#AB1234".into())));
        assert!(ops.contains(&Op::SetLineWidth(3.0)));
        if preset == ShapePreset::Ellipse {
            assert!(ops.iter().any(|op| matches!(op, Op::Ellipse(x,y,rx,ry,..) if *x==260.0 && *y==180.0 && *rx==160.0 && *ry==80.0)));
        }
        assert_eq!(
            ops.iter().filter(|op| **op == Op::Save).count(),
            ops.iter().filter(|op| **op == Op::Restore).count()
        );
    }
}

#[test]
fn theme_defaults_and_transparent_shapes_remain_editable_after_typing() {
    for transparent in [false, true] {
        let mut value = value(ShapePreset::Ellipse);
        value.text.clear();
        value.fill = transparent.then(|| "none".into());
        value.outline = transparent.then(|| "none".into());
        value.outline_width_pt = None;
        let mut session = EditSession::open(DECK).unwrap();
        let id = session.insert_shape(0, bounds(), &value).unwrap();
        let created = session.bytes().to_vec();
        let deck = jade_pptx_reader::read_deck(&created).unwrap();
        let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
        let shape = layout
            .shapes
            .iter()
            .find(|s| s.source_id == Some(id))
            .unwrap();
        assert_eq!(shape.fill.is_none(), transparent);
        assert_eq!(shape.outline.is_none(), transparent);
        if !transparent {
            assert_eq!(shape.outline.as_ref().unwrap().width_px, 4.0 / 3.0);
        }
        session.set_shape_text(0, id, "typed", false).unwrap();
        assert_eq!(shape_text(session.bytes(), 0, id).unwrap().text, "typed");
        let run = jade_pptx_writer::editable_runs(session.bytes(), 0)
            .unwrap()
            .into_iter()
            .find(|r| r.address.shape_id == id)
            .unwrap();
        assert_eq!(run.text, "typed");
        assert!(xml(session.bytes()).contains("sz=\"2400\""));
        assert!(session.undo());
        assert_eq!(session.bytes(), created);
        assert!(session.undo());
        assert_eq!(session.bytes(), DECK);
    }
}

#[test]
fn invalid_styles_bounds_text_and_slide_do_not_mutate_history() {
    let mut session = EditSession::open(DECK).unwrap();
    for color in ["red", "#abc", "#12345G", "#123456\"", "#ééé", "None"] {
        let mut v = value(ShapePreset::Rect);
        v.fill = Some(color.into());
        assert!(session.insert_shape(0, bounds(), &v).is_err());
        v.fill = None;
        v.outline = Some(color.into());
        assert!(session.insert_shape(0, bounds(), &v).is_err());
    }
    for width in [-1.0, 1584.01, f64::NAN, f64::INFINITY] {
        let mut v = value(ShapePreset::Rect);
        v.outline_width_pt = Some(width);
        assert!(session.insert_shape(0, bounds(), &v).is_err());
    }
    for transform in [
        Transform {
            extent_emu: (0, 1),
            ..bounds()
        },
        Transform {
            offset_emu: (i64::MAX, 0),
            ..bounds()
        },
    ] {
        assert!(session
            .insert_shape(0, transform, &value(ShapePreset::Rect))
            .is_err());
    }
    let mut v = value(ShapePreset::Rect);
    v.text = "bad\u{1}".into();
    assert!(session.insert_shape(0, bounds(), &v).is_err());
    assert!(session
        .insert_shape(99, bounds(), &value(ShapePreset::Rect))
        .is_err());
    assert_eq!(session.bytes(), DECK);
    assert!(!session.undo());
}

#[test]
fn strict_aliases_extensions_and_reserved_child_ids_are_preserved() {
    for strict in [false, true] {
        let mut input = xml(DECK)
            .replace("<p:", "<s:")
            .replace("</p:", "</s:")
            .replace("xmlns:p=", "xmlns:s=");
        if strict {
            input = input
                .replace(
                    "http://schemas.openxmlformats.org/presentationml/2006/main",
                    "http://purl.oclc.org/ooxml/presentationml/main",
                )
                .replace(
                    "http://schemas.openxmlformats.org/drawingml/2006/main",
                    "http://purl.oclc.org/ooxml/drawingml/main",
                );
        }
        let ext="<s:extLst><s:ext uri=\"test\"><ref xmlns=\"urn:test\" shapeId=\"999\"/></s:ext></s:extLst>";
        input = input.replace("</s:spTree>", &format!("{ext}</s:spTree>"));
        let bytes = with_xml(&input);
        let mut session = EditSession::open(&bytes).unwrap();
        let first = session
            .insert_shape(0, bounds(), &value(ShapePreset::Triangle))
            .unwrap();
        let second = session
            .insert_shape(0, bounds(), &value(ShapePreset::Diamond))
            .unwrap();
        assert_eq!((first, second), (1000, 1001));
        let changed = xml(session.bytes());
        assert!(changed.contains("name=\"Shape 1\""));
        assert!(changed.contains("name=\"Shape 2\""));
        assert!(changed.contains(&format!("</jadeSp:sp>{ext}")));
        assert_eq!(
            changed.contains("xmlns:jadeSp=\"http://purl.oclc.org"),
            strict
        );
    }
}
