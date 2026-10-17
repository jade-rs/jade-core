use jade_opc::OpcPackage;
use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure, PlacedOrigin};
use jade_pptx_reader::{ShapeItem, Transform};
use jade_pptx_writer::{slide_layouts, EditSession};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
const THEMES: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
const LAYOUT: &str = "ppt/slideLayouts/slideLayout2.xml";
fn xml(bytes: &[u8], part: &str) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str(part)
        .unwrap()
}
fn replace(bytes: &[u8], part: &str, value: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(bytes).unwrap();
    p.replace(part, value.as_bytes().to_vec());
    p.write().unwrap()
}
fn own_bounds(bytes: &[u8], id: u32) -> jade_pptx_layout::Rect {
    let d = jade_pptx_reader::read_deck(bytes).unwrap();
    layout_deck_slide(&d, 0, &HeuristicMeasure)
        .unwrap()
        .shapes
        .into_iter()
        .find(|s| s.source_id == Some(id) && s.origin == PlacedOrigin::Slide)
        .unwrap()
        .paint_rect()
}
#[test]
fn changing_layout_retargets_placeholders_keeps_content_parts_and_one_exact_undo() {
    let layouts = slide_layouts(DECK, 0).unwrap();
    assert_eq!(layouts.len(), 2);
    assert!(layouts[0].current);
    assert_eq!(layouts[1].part_name, LAYOUT);
    let old = jade_pptx_writer::shape_text(DECK, 0, 2).unwrap();
    let old_box = own_bounds(DECK, 2);
    let mut s = EditSession::open(DECK).unwrap();
    assert!(s.set_slide_layout(0, LAYOUT).unwrap());
    let saved = s.bytes().to_vec();
    assert_eq!(
        jade_pptx_writer::shape_text(&saved, 0, 2).unwrap().text,
        old.text
    );
    assert_ne!(own_bounds(&saved, 2), old_box);
    let deck = jade_pptx_reader::read_deck(&saved).unwrap();
    let ShapeItem::Shape(title) = &deck.slides[0].content.shapes[0] else {
        panic!("title")
    };
    assert_eq!(title.id, Some(2));
    assert_eq!(title.placeholder.as_ref().unwrap().kind, "title");
    let ShapeItem::Shape(body) = &deck.slides[0].content.shapes[1] else {
        panic!("body")
    };
    assert_eq!(body.id, Some(3));
    assert_eq!(body.placeholder.as_ref().unwrap().kind, "body");
    let p = OpcPackage::open(DECK).unwrap();
    let q = OpcPackage::open(&saved).unwrap();
    for part in p.part_names().iter().filter(|p| {
        !matches!(
            p.as_str(),
            "ppt/slides/slide1.xml" | "ppt/slides/_rels/slide1.xml.rels"
        )
    }) {
        assert_eq!(p.part(part), q.part(part));
    }
    assert!(!s.set_slide_layout(0, LAYOUT).unwrap());
    assert_eq!(s.bytes(), saved);
    assert!(s.undo());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    assert!(s.redo());
    assert_eq!(s.bytes(), saved);
}
#[test]
fn reapplying_adds_missing_empty_placeholders_without_copying_layout_prompt_text() {
    let mut s = EditSession::open(THEMES).unwrap();
    let layout = slide_layouts(THEMES, 0)
        .unwrap()
        .into_iter()
        .find(|l| l.current)
        .unwrap();
    assert!(s.set_slide_layout(0, &layout.part_name).unwrap());
    let d = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    assert_eq!(d.slides[0].content.shapes.len(), 5);
    let ids: Vec<_> = d.slides[0]
        .content
        .shapes
        .iter()
        .filter_map(|v| {
            if let ShapeItem::Shape(s) = v {
                Some(s.id.unwrap())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids, vec![2, 3, 4, 5, 6]);
    for id in [5, 6] {
        let t = jade_pptx_writer::shape_text(s.bytes(), 0, id).unwrap();
        assert!(t.can_edit);
        assert!(t.text.is_empty());
        assert!(jade_pptx_writer::editable_shapes(s.bytes(), 0)
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .transform
            .is_some());
    }
    assert!(!s.set_slide_layout(0, &layout.part_name).unwrap());
    assert_eq!(s.history_depth(), (1, 0));
    assert!(s.undo());
    assert_eq!(s.bytes(), THEMES);
}
#[test]
fn blank_layout_preserves_unmatched_placeholder_positions_and_manual_geometry() {
    let blank = r#"<p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" type="blank"><p:cSld name="Blank"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld></p:sldLayout>"#;
    let bytes = replace(DECK, LAYOUT, blank);
    let old = [own_bounds(&bytes, 2), own_bounds(&bytes, 3)];
    let mut s = EditSession::open(&bytes).unwrap();
    s.set_slide_layout(0, LAYOUT).unwrap();
    assert_eq!(own_bounds(s.bytes(), 2), old[0]);
    assert_eq!(own_bounds(s.bytes(), 3), old[1]);
    assert!(xml(s.bytes(), "ppt/slides/slide1.xml").contains("jadeDraw:xfrm"));
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
    let mut s = EditSession::open(DECK).unwrap();
    let transform = Transform {
        offset_emu: (952500, 1905000),
        extent_emu: (2857500, 952500),
        rotation_60000ths: 1800000,
        flip_h: true,
        flip_v: false,
    };
    s.set_shape_transform(0, 2, transform).unwrap();
    s.set_slide_layout(0, LAYOUT).unwrap();
    assert_eq!(
        jade_pptx_writer::editable_shapes(s.bytes(), 0)
            .unwrap()
            .iter()
            .find(|s| s.id == 2)
            .unwrap()
            .transform,
        Some(transform)
    );
}
#[test]
fn cross_master_layout_assignment_uses_the_new_theme_and_preserves_existing_objects() {
    let mut s = EditSession::open(THEMES).unwrap();
    let old = s.bytes().to_vec();
    assert!(s.set_slide_layout(0, LAYOUT).unwrap());
    let d = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    assert_eq!(
        d.theme_for_slide(0).fonts.minor_latin.as_deref(),
        Some("serif")
    );
    let layout = layout_deck_slide(&d, 0, &HeuristicMeasure).unwrap();
    assert_eq!(
        layout
            .shapes
            .iter()
            .find(|s| s.source_id == Some(3) && s.origin == PlacedOrigin::Slide)
            .unwrap()
            .fill
            .as_deref(),
        Some("#E6A010")
    );
    assert!(s.undo());
    assert_eq!(s.bytes(), old);
}
#[test]
fn unregistered_ambiguous_or_external_layout_targets_fail_without_history_or_package_mutation() {
    let mut s = EditSession::open(DECK).unwrap();
    for target in [
        "../../bad.xml",
        "ppt/slides/slide2.xml",
        "missing",
        "https://example.com/layout.xml",
    ] {
        assert!(s.set_slide_layout(0, target).is_err());
    }
    assert!(s.set_slide_layout(99, LAYOUT).is_err());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    let original = xml(DECK, LAYOUT);
    let duplicate=original.replace("</p:spTree>","<p:sp><p:nvSpPr><p:cNvPr id=\"99\" name=\"Duplicate\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp></p:spTree>");
    let bytes = replace(DECK, LAYOUT, &duplicate);
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.set_slide_layout(0, LAYOUT).is_err());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
}

#[test]
fn filled_picture_placeholder_keeps_media_crop_and_explicit_transform_after_role_mapping() {
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    let mut data = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([240, 20, 20])))
        .write_to(&mut data, ImageFormat::Png)
        .unwrap();
    let mut s = EditSession::open(DECK).unwrap();
    s.delete_shape(0, 3).unwrap();
    let transform = Transform {
        offset_emu: (952500, 1905000),
        extent_emu: (1905000, 952500),
        rotation_60000ths: 1800000,
        flip_h: true,
        flip_v: false,
    };
    let inserted = s.insert_picture(0, transform, &data.into_inner()).unwrap();
    let id = inserted.id;
    s.set_picture_crop(
        0,
        id,
        jade_pptx_reader::SourceRect {
            left: 10000,
            top: 0,
            right: 0,
            bottom: 0,
        },
        true,
    )
    .unwrap();
    let text = xml(s.bytes(), "ppt/slides/slide1.xml");
    let start = text.find("<jadeSp:pic ").unwrap();
    let pic = &text[start..];
    let nv = pic.find("<jadeSp:nvPr/>").unwrap() + start;
    let changed = format!(
        "{}<jadeSp:nvPr><jadeSp:ph type=\"pic\" idx=\"1\"/></jadeSp:nvPr>{}",
        &text[..nv],
        &text[nv + "<jadeSp:nvPr/>".len()..]
    );
    let bytes = replace(s.bytes(), "ppt/slides/slide1.xml", &changed);
    let before = jade_pptx_reader::read_deck(&bytes).unwrap();
    let mut s = EditSession::open(&bytes).unwrap();
    s.set_slide_layout(0, LAYOUT).unwrap();
    let after = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let get = |deck: &jade_pptx_reader::Deck| {
        deck.slides[0]
            .content
            .shapes
            .iter()
            .find_map(|item| match item {
                ShapeItem::Picture(p) if p.id == Some(id) => Some(p.clone()),
                _ => None,
            })
            .unwrap()
    };
    let a = get(&before);
    let b = get(&after);
    assert_eq!(a.transform, b.transform);
    assert_eq!(a.source_rect, b.source_rect);
    assert_eq!(a.image_part, b.image_part);
    assert_eq!(b.placeholder.unwrap().kind, "body");
    let p = OpcPackage::open(&bytes).unwrap();
    let q = OpcPackage::open(s.bytes()).unwrap();
    assert_eq!(
        p.part(&a.image_part.unwrap()),
        q.part(&b.image_part.unwrap())
    );
}
#[test]
fn strict_layout_switch_uses_scoped_strict_placeholder_namespaces() {
    let mut p = OpcPackage::open(THEMES).unwrap();
    for part in p.part_names().to_vec() {
        if part.ends_with(".xml") || part.ends_with(".rels") {
            let text = p
                .required_part_str(&part)
                .unwrap()
                .replace(
                    "http://schemas.openxmlformats.org/presentationml/2006/main",
                    "http://purl.oclc.org/ooxml/presentationml/main",
                )
                .replace(
                    "http://schemas.openxmlformats.org/drawingml/2006/main",
                    "http://purl.oclc.org/ooxml/drawingml/main",
                )
                .replace(
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                    "http://purl.oclc.org/ooxml/officeDocument/relationships",
                );
            p.replace(&part, text.into_bytes());
        }
    }
    let bytes = p.write().unwrap();
    let mut s = EditSession::open(&bytes).unwrap();
    s.set_slide_layout(0, LAYOUT).unwrap();
    let text = xml(s.bytes(), "ppt/slides/slide1.xml");
    assert!(text.contains("xmlns:jadeLayout=\"http://purl.oclc.org/ooxml/presentationml/main\""));
    assert_eq!(
        jade_pptx_writer::shape_text(s.bytes(), 0, 5).unwrap().text,
        ""
    );
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
}

#[test]
fn new_placeholder_extensions_cannot_copy_layout_relative_relationships_into_the_slide() {
    let source = xml(THEMES, "ppt/slideLayouts/slideLayout1.xml");
    let xml=source.replace("<p:ph type=\"ctrTitle\"/>","<p:ph type=\"ctrTitle\"><p:extLst><p:ext uri=\"test\"><q:payload xmlns:q=\"urn:test\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId1\"/></p:ext></p:extLst></p:ph>");
    assert_ne!(source, xml);
    let bytes = replace(THEMES, "ppt/slideLayouts/slideLayout1.xml", &xml);
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s
        .set_slide_layout(0, "ppt/slideLayouts/slideLayout1.xml")
        .is_err());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
}
