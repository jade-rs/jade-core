use jade_opc::OpcPackage;
use jade_painter::recording::{Op, RecordingPainter};
use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure};
use jade_pptx_render::{render_slide, RenderOptions};
use jade_pptx_writer::{shape_style, EditSession, ShapePaintInfo, ShapeStyleEdit};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/grouped-deck.pptx");
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}
fn edited(value: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(DECK).unwrap();
    p.replace("ppt/slides/slide1.xml", value.as_bytes().to_vec());
    p.write().unwrap()
}
fn all() -> ShapeStyleEdit {
    ShapeStyleEdit {
        fill: Some("#ab12cd".into()),
        outline: Some("#12cdab".into()),
        outline_width_pt: Some(3.75),
    }
}
#[test]
fn style_edit_preserves_geometry_text_parts_and_exact_undo_and_renders_native_style() {
    let before = jade_pptx_reader::read_deck(DECK).unwrap();
    let mut s = EditSession::open(DECK).unwrap();
    assert!(s.set_shape_style(0, 99, &all()).unwrap());
    let saved = s.bytes().to_vec();
    let state = shape_style(&saved, 0, 99).unwrap().unwrap();
    assert_eq!(
        state.fill,
        ShapePaintInfo::Solid {
            color: "#AB12CD".into()
        }
    );
    assert_eq!(state.outline_width_pt, Some(3.75));
    let after = jade_pptx_reader::read_deck(&saved).unwrap();
    let original = layout_deck_slide(&before, 0, &HeuristicMeasure).unwrap();
    let layout = layout_deck_slide(&after, 0, &HeuristicMeasure).unwrap();
    let a = original
        .shapes
        .iter()
        .find(|v| v.source_id == Some(99))
        .unwrap();
    let b = layout
        .shapes
        .iter()
        .find(|v| v.source_id == Some(99))
        .unwrap();
    assert_eq!(a.rect, b.rect);
    assert_eq!(a.lines, b.lines);
    assert_eq!(a.paint_transform(), b.paint_transform());
    assert_eq!(b.fill.as_deref(), Some("#AB12CD"));
    assert_eq!(b.outline.as_ref().unwrap().width_px, 5.0);
    let painter = RecordingPainter::new();
    render_slide(&painter, &layout, &RenderOptions::default()).unwrap();
    assert!(painter.ops().contains(&Op::SetFillStyle("#AB12CD".into())));
    assert!(painter
        .ops()
        .contains(&Op::SetStrokeStyle("#12CDAB".into())));
    assert!(painter.ops().contains(&Op::SetLineWidth(5.0)));
    let p = OpcPackage::open(DECK).unwrap();
    let q = OpcPackage::open(&saved).unwrap();
    for name in p
        .part_names()
        .iter()
        .filter(|n| n.as_str() != "ppt/slides/slide1.xml")
    {
        assert_eq!(p.part(name), q.part(name));
    }
    assert!(!s.set_shape_style(0, 99, &all()).unwrap());
    assert_eq!(s.bytes(), saved);
    assert!(s.undo());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    assert!(s.redo());
    assert_eq!(s.bytes(), saved);
}
#[test]
fn width_only_retains_gradient_alpha_dash_arrows_effects_and_unknown_xml() {
    let source = xml(DECK);
    let at = source.find("name=\"Outside target\"").unwrap();
    let pr = source[at..].find("<g:spPr>").map(|n| at + n).unwrap();
    let end = source[pr..].find("</g:spPr>").map(|n| pr + n).unwrap();
    let old = &source[pr..end + 9];
    let rich="<g:spPr><a:xfrm rot=\"5400000\"><a:off x=\"100000\" y=\"200000\"/><a:ext cx=\"300000\" cy=\"400000\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:gradFill><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"FF0000\"><a:alpha val=\"50000\"/></a:srgbClr></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"0000FF\"/></a:gs></a:gsLst><a:lin ang=\"0\"/></a:gradFill><a:ln w=\"12700\" cap=\"rnd\"><a:solidFill><a:schemeClr val=\"accent2\"><a:alpha val=\"75000\"/></a:schemeClr></a:solidFill><a:prstDash val=\"dash\"/><a:round/><a:headEnd type=\"triangle\"/><a:tailEnd type=\"oval\"/></a:ln><a:effectLst/><a:extLst><a:ext uri=\"test\"><q:payload xmlns:q=\"urn:test\" val=\"unchanged\"/></a:ext></a:extLst></g:spPr>";
    let bytes = edited(&source.replacen(old, rich, 1));
    let mut s = EditSession::open(&bytes).unwrap();
    let state = shape_style(&bytes, 0, 99).unwrap().unwrap();
    assert_eq!(state.fill, ShapePaintInfo::Other);
    assert_eq!(state.outline, ShapePaintInfo::Other);
    s.set_shape_style(
        0,
        99,
        &ShapeStyleEdit {
            outline_width_pt: Some(2.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        xml(s.bytes()),
        xml(&bytes).replacen("w=\"12700\" cap=\"rnd\"", "w=\"25400\" cap=\"rnd\"", 1)
    );
    s.set_shape_style(
        0,
        99,
        &ShapeStyleEdit {
            fill: Some("none".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let changed = xml(s.bytes());
    assert!(changed.contains("<a:prstDash val=\"dash\"/>"));
    assert!(changed.contains("<a:headEnd type=\"triangle\"/>"));
    assert!(changed.contains("<a:alpha val=\"75000\"/>"));
    assert!(changed.contains("val=\"unchanged\""));
    assert!(!changed.contains("<a:gradFill>"));
}
#[test]
fn strict_empty_properties_and_absent_line_insert_in_schema_order() {
    let source = xml(DECK);
    let at = source.find("name=\"Outside target\"").unwrap();
    let start = source[at..].find("<g:spPr>").map(|n| at + n).unwrap();
    let end = source[start..]
        .find("</g:spPr>")
        .map(|n| start + n + 9)
        .unwrap();
    let source = format!("{}<g:spPr/>{}", &source[..start], &source[end..])
        .replace(
            "http://schemas.openxmlformats.org/presentationml/2006/main",
            "http://purl.oclc.org/ooxml/presentationml/main",
        )
        .replace(
            "http://schemas.openxmlformats.org/drawingml/2006/main",
            "http://purl.oclc.org/ooxml/drawingml/main",
        );
    let bytes = edited(&source);
    let mut s = EditSession::open(&bytes).unwrap();
    s.set_shape_style(0, 99, &all()).unwrap();
    let text = xml(s.bytes());
    let start = text.find("name=\"Outside target\"").unwrap();
    let tail = &text[start..];
    assert!(tail.find("solidFill").unwrap() < tail.find("jadeStyle:ln").unwrap());
    assert!(tail.contains("http://purl.oclc.org/ooxml/drawingml/main"));
    assert_eq!(
        shape_style(s.bytes(), 0, 99)
            .unwrap()
            .unwrap()
            .outline_width_pt,
        Some(3.75)
    );
    assert!(!s.set_shape_style(0, 99, &all()).unwrap());
}
#[test]
fn invalid_style_locked_group_and_nested_shape_reject_without_changing_history() {
    let mut s = EditSession::open(DECK).unwrap();
    for color in ["red", "#1234", "#12345💥"] {
        assert!(s
            .set_shape_style(
                0,
                99,
                &ShapeStyleEdit {
                    fill: Some(color.into()),
                    ..Default::default()
                }
            )
            .is_err());
    }
    for width in [f64::NAN, f64::INFINITY, -1.0, 1584.001] {
        assert!(s
            .set_shape_style(
                0,
                99,
                &ShapeStyleEdit {
                    outline_width_pt: Some(width),
                    ..Default::default()
                }
            )
            .is_err());
    }
    for id in [50, 2, 12345] {
        assert!(s.set_shape_style(0, id, &all()).is_err());
    }
    assert!(!s
        .set_shape_style(0, 99, &ShapeStyleEdit::default())
        .unwrap());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    let bytes = edited(&xml(DECK).replacen(
        "<g:cNvSpPr/>",
        "<g:cNvSpPr><d:spLocks noSelect=\"1\"/></g:cNvSpPr>",
        1,
    ));
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.set_shape_style(0, 99, &all()).is_err());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
}
#[test]
fn alternate_content_updates_each_branch_without_changing_wrapper_or_refs() {
    let source = xml(DECK);
    let at = source.find("name=\"Outside target\"").unwrap();
    let start = source[..at].rfind("<g:sp ").unwrap();
    let end = source[at..].find("</g:sp>").map(|n| at + n + 7).unwrap();
    let shape = &source[start..end];
    let wrapper=format!("<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:q=\"urn:test\" Requires=\"q\">{shape}</mc:Choice><mc:Fallback>{shape}</mc:Fallback></mc:AlternateContent>");
    let bytes = edited(&format!(
        "{}{}{}",
        &source[..start],
        wrapper,
        &source[end..]
    ));
    let mut s = EditSession::open(&bytes).unwrap();
    s.set_shape_style(0, 99, &all()).unwrap();
    let changed = xml(s.bytes());
    assert_eq!(changed.matches("val=\"AB12CD\"").count(), 2);
    assert_eq!(changed.matches("w=\"47625\"").count(), 2);
    assert!(changed.contains("Requires=\"q\""));
    assert!(changed.contains("<p:timing>"));
}

#[test]
fn theme_line_matrix_reaches_the_recording_painter_and_width_edits_preserve_reference_and_exact_undo(
) {
    let bytes = include_bytes!("../../../public/fixtures/themed-deck.pptx");
    let mut s = EditSession::open(bytes).unwrap();
    for (index, width, color) in [
        (0, 8.0, "#F01414"),
        (1, 40.0 / 3.0, "#735008"),
        (2, 40.0 / 3.0, "#501078"),
        (3, 16.0 / 3.0, "#F014C8"),
    ] {
        let d = jade_pptx_reader::read_deck(s.bytes()).unwrap();
        let layout = layout_deck_slide(&d, index, &HeuristicMeasure).unwrap();
        let painter = RecordingPainter::new();
        render_slide(&painter, &layout, &RenderOptions::default()).unwrap();
        let ops = painter.ops();
        assert!(ops
            .iter()
            .any(|op| matches!(op,Op::SetLineWidth(w) if (*w-width).abs()<1e-8)));
        assert!(ops.contains(&Op::SetStrokeStyle(color.into())));
    }
    assert!(s
        .set_shape_style(
            0,
            4,
            &ShapeStyleEdit {
                outline_width_pt: Some(2.0),
                ..Default::default()
            }
        )
        .unwrap());
    let d = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let line = layout_deck_slide(&d, 0, &HeuristicMeasure)
        .unwrap()
        .shapes
        .into_iter()
        .find(|s| s.source_id == Some(4) && s.origin == jade_pptx_layout::PlacedOrigin::Slide)
        .unwrap()
        .outline
        .unwrap();
    assert_eq!(line.color, "#F01414");
    assert!((line.width_px - 8.0 / 3.0).abs() < 1e-8);
    assert_eq!(line.dash_px.len(), 2);
    let saved = s.bytes().to_vec();
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
    assert!(s.redo());
    assert_eq!(s.bytes(), saved);
}
