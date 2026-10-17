use jade_opc::OpcPackage;
use jade_painter::recording::RecordingPainter;
use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure, PlacedOrigin};
use jade_pptx_reader::Transform;
use jade_pptx_render::{render_slide, RenderOptions};
use jade_pptx_writer::{EditSession, ShapeStyleEdit, TextFormat, TextFormatEdit};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
const THEMES: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
const SLIDE: &str = "ppt/slides/slide1.xml";
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
fn transform(bytes: &[u8], id: u32) -> Transform {
    jade_pptx_writer::editable_shapes(bytes, 0)
        .unwrap()
        .into_iter()
        .find(|s| s.id == id)
        .unwrap()
        .transform
        .unwrap()
}
fn paint(bytes: &[u8]) -> Vec<jade_painter::recording::Op> {
    let deck = jade_pptx_reader::read_deck(bytes).unwrap();
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let p = RecordingPainter::new();
    render_slide(&p, &layout, &RenderOptions::default()).unwrap();
    p.ops()
}
fn edited() -> Vec<u8> {
    let mut s = EditSession::open(DECK).unwrap();
    let mut t = transform(DECK, 2);
    t.offset_emu = (100000, 100000);
    t.extent_emu = (3000000, 1200000);
    t.rotation_60000ths = 1800000;
    t.flip_h = true;
    s.set_shape_transform(0, 2, t).unwrap();
    s.format_text_range(
        0,
        &TextFormatEdit {
            shape_id: 2,
            start: 0,
            end: 11,
            format: TextFormat {
                bold: Some(true),
                size_pt: Some(68.0),
                color: Some("#FF0000".into()),
                font: Some("serif".into()),
                ..Default::default()
            },
        },
    )
    .unwrap();
    s.set_shape_style(
        0,
        2,
        &ShapeStyleEdit {
            fill: Some("#FF0000".into()),
            outline: Some("#0000FF".into()),
            outline_width_pt: Some(4.0),
        },
    )
    .unwrap();
    s.bytes().to_vec()
}
#[test]
fn reset_restores_inherited_frame_and_text_style_with_exact_undo_and_painter_agreement() {
    let bytes = edited();
    let mut expected = EditSession::open(DECK).unwrap();
    expected.reset_slide(0).unwrap();
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.reset_slide(0).unwrap());
    assert_eq!(transform(s.bytes(), 2), transform(DECK, 2));
    assert_eq!(
        jade_pptx_writer::shape_text(s.bytes(), 0, 2).unwrap().text,
        "Jade Slides"
    );
    assert_eq!(paint(s.bytes()), paint(expected.bytes()));
    let deck = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let title = layout
        .shapes
        .iter()
        .find(|s| s.source_id == Some(2) && s.origin == PlacedOrigin::Slide)
        .unwrap();
    assert_eq!(title.lines[0].runs[0].size_pt, 44.0);
    assert!(title.fill.is_none());
    let saved = s.bytes().to_vec();
    assert!(!s.reset_slide(0).unwrap());
    assert_eq!(s.history_depth(), (1, 0));
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
    assert!(s.redo());
    assert_eq!(s.bytes(), saved);
    let p = OpcPackage::open(&bytes).unwrap();
    let q = OpcPackage::open(&saved).unwrap();
    for part in p.part_names().iter().filter(|p| p.as_str() != SLIDE) {
        assert_eq!(p.part(part), q.part(part), "{part}");
    }
}
#[test]
fn reset_preserves_content_fields_links_levels_extensions_and_unmatched_objects() {
    let source = xml(&edited(), SLIDE);
    let source=source.replace("<a:p>","<a:p><a:pPr lvl=\"2\" algn=\"r\"><a:defRPr b=\"1\"/><a:extLst><a:ext uri=\"keep\"><z:data xmlns:z=\"urn:test\"/></a:ext></a:extLst></a:pPr>");
    let source = source.replace("<a:r>", "<a:r data-test=\"keep\">");
    let source=source.replace("</a:p>","<a:fld id=\"{00000000-0000-0000-0000-000000000001}\" type=\"slidenum\"><a:rPr lang=\"ja-JP\" sz=\"8800\"><a:hlinkClick xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId99\"/><a:extLst><a:ext uri=\"keep\"><z:run xmlns:z=\"urn:test\"/></a:ext></a:extLst></a:rPr><a:t>1</a:t></a:fld></a:p>");
    let bytes = replace(&edited(), SLIDE, &source);
    let before = jade_pptx_writer::shape_text(&bytes, 0, 2).unwrap().text;
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.reset_slide(0).unwrap());
    let result = xml(s.bytes(), SLIDE);
    for preserved in [
        "lvl=\"2\"",
        "<z:data xmlns:z=\"urn:test\"/>",
        "<z:run xmlns:z=\"urn:test\"/>",
        "data-test=\"keep\"",
        "r:id=\"rId99\"",
        "lang=\"ja-JP\"",
        "type=\"slidenum\"",
    ] {
        assert!(result.contains(preserved), "{preserved}");
    }
    assert!(!result.contains("sz=\"8800\""));
    assert!(!result.contains("algn=\"r\""));
    assert_eq!(
        jade_pptx_writer::shape_text(s.bytes(), 0, 2).unwrap().text,
        before
    );
    let mut s = EditSession::open(THEMES).unwrap();
    let original = xml(THEMES, SLIDE);
    s.reset_slide(0).unwrap();
    let after = xml(s.bytes(), SLIDE);
    let ordinary = original
        .split("<p:sp>")
        .nth(1)
        .unwrap()
        .split("</p:sp>")
        .next()
        .unwrap();
    assert!(after.contains(ordinary));
    assert_eq!(
        jade_pptx_writer::shape_text(s.bytes(), 0, 5).unwrap().text,
        ""
    );
}
#[test]
fn invalid_layout_extensions_or_locks_leave_package_and_history_untouched() {
    let bytes = edited();
    for source in [
        xml(&bytes, SLIDE).replace(
            "<p:cNvSpPr/>",
            "<p:cNvSpPr><a:spLocks noMove=\"1\"/></p:cNvSpPr>",
        ),
        xml(&bytes, SLIDE).replace(
            "<p:cNvSpPr/>",
            "<p:cNvSpPr><a:spLocks noTextEdit=\"1\"/></p:cNvSpPr>",
        ),
        xml(&bytes, SLIDE).replace(
            "<p:spPr>",
            "<p:spPr><a:custGeom><z:data xmlns:z=\"urn:test\"/></a:custGeom>",
        ),
    ] {
        let bytes = replace(&bytes, SLIDE, &source);
        let mut s = EditSession::open(&bytes).unwrap();
        assert!(s.reset_slide(0).is_err());
        assert_eq!(s.bytes(), bytes);
        assert_eq!(s.history_depth(), (0, 0));
    }
    let mut s = EditSession::open(DECK).unwrap();
    assert!(s.reset_slide(99).is_err());
    assert_eq!(s.bytes(), DECK);
}
#[test]
fn reset_uses_strict_scoped_template_namespaces() {
    let mut p = OpcPackage::open(&edited()).unwrap();
    for part in p.part_names().to_vec() {
        if part.ends_with(".xml") || part.ends_with(".rels") {
            let value = p
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
            p.replace(&part, value.into_bytes());
        }
    }
    let bytes = p.write().unwrap();
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.reset_slide(0).unwrap());
    assert_eq!(transform(s.bytes(), 2), transform(DECK, 2));
    assert!(xml(s.bytes(), SLIDE).contains("http://purl.oclc.org/ooxml/drawingml/main"));
    assert!(!s.reset_slide(0).unwrap());
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
}

#[test]
fn filled_picture_placeholder_resets_frame_but_preserves_crop_media_and_content() {
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    let mut image = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([240, 20, 20])))
        .write_to(&mut image, ImageFormat::Png)
        .unwrap();
    let mut s = EditSession::open(DECK).unwrap();
    let mut frame = transform(DECK, 3);
    frame.offset_emu = (100000, 100000);
    frame.rotation_60000ths = 1800000;
    frame.flip_h = true;
    s.delete_shape(0, 3).unwrap();
    let id = s.insert_picture(0, frame, &image.into_inner()).unwrap().id;
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
    let source = xml(s.bytes(), SLIDE).replace(
        "<jadeSp:nvPr/>",
        "<jadeSp:nvPr><jadeSp:ph type=\"subTitle\" idx=\"1\"/></jadeSp:nvPr>",
    );
    let bytes = replace(s.bytes(), SLIDE, &source);
    let mut s = EditSession::open(&bytes).unwrap();
    s.reset_slide(0).unwrap();
    assert_eq!(transform(s.bytes(), id), transform(DECK, 3));
    let before = jade_pptx_reader::read_deck(&bytes).unwrap();
    let after = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let picture = |deck: &jade_pptx_reader::Deck| {
        deck.slides[0]
            .content
            .shapes
            .iter()
            .find_map(|s| match s {
                jade_pptx_reader::ShapeItem::Picture(p) => Some(p.clone()),
                _ => None,
            })
            .unwrap()
    };
    let a = picture(&before);
    let b = picture(&after);
    assert_eq!(a.image_part, b.image_part);
    assert_eq!(a.source_rect, b.source_rect);
    assert_eq!(
        OpcPackage::open(&bytes)
            .unwrap()
            .part(a.image_part.as_ref().unwrap()),
        OpcPackage::open(s.bytes())
            .unwrap()
            .part(b.image_part.as_ref().unwrap())
    );
    assert!(!s.reset_slide(0).unwrap());
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
}
#[test]
fn existing_footer_fields_reset_without_recreating_hidden_footer_slots() {
    let footer = r#"<p:sp><p:nvSpPr><p:cNvPr id="99" name="Page"/><p:cNvSpPr/><p:nvPr><p:ph type="sldNum" idx="9"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="400000" y="400000"/><a:ext cx="500000" cy="500000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:fld id="{00000000-0000-0000-0000-000000000099}" type="slidenum"><a:rPr sz="3800"/><a:t>1</a:t></a:fld></a:p></p:txBody></p:sp>"#;
    let source = xml(DECK, SLIDE).replace("</p:spTree>", &format!("{footer}</p:spTree>"));
    let bytes = replace(DECK, SLIDE, &source);
    let layout = "ppt/slideLayouts/slideLayout1.xml";
    let target = xml(&bytes, layout).replace(
        "</p:spTree>",
        &format!(
            "{}</p:spTree>",
            footer
                .replace("x=\"400000\"", "x=\"800000\"")
                .replace("sz=\"3800\"", "sz=\"1200\"")
        ),
    );
    let bytes = replace(&bytes, layout, &target);
    let mut s = EditSession::open(&bytes).unwrap();
    s.reset_slide(0).unwrap();
    assert_eq!(transform(s.bytes(), 99).offset_emu.0, 800000);
    let result = xml(s.bytes(), SLIDE);
    assert!(result.contains("type=\"slidenum\""));
    assert!(!result.contains("sz=\"3800\""));
    assert!(result.contains("<a:t>1</a:t>"));
    let without = replace(&bytes, SLIDE, &xml(DECK, SLIDE));
    let mut s = EditSession::open(&without).unwrap();
    s.reset_slide(0).unwrap();
    assert!(!jade_pptx_writer::editable_shapes(s.bytes(), 0)
        .unwrap()
        .iter()
        .any(|s| s.id == 99));
}

#[test]
fn reset_removes_optional_style_and_keeps_geometry_before_preserved_image_fill() {
    let bytes = edited();
    let source=xml(&bytes,SLIDE).replace("</p:spPr>",r#"<a:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:embed="rId99"/><a:stretch><a:fillRect/></a:stretch></a:blipFill></p:spPr><p:style><a:lnRef idx="0"><a:schemeClr val="accent1"/></a:lnRef><a:fillRef idx="0"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="accent1"/></a:fontRef></p:style>"#);
    let bytes = replace(&bytes, SLIDE, &source);
    let mut s = EditSession::open(&bytes).unwrap();
    s.reset_slide(0).unwrap();
    let result = xml(s.bytes(), SLIDE);
    assert!(!result.contains("<p:style>"));
    let geometry = result.find(":prstGeom").unwrap();
    let image = result.find("<a:blipFill>").unwrap();
    assert!(geometry < image);
    assert!(result.contains("r:embed=\"rId99\""));
    assert!(!s.reset_slide(0).unwrap());
}

#[test]
fn reset_cannot_drop_a_visible_frame_when_layout_inheritance_is_unresolved() {
    let bytes = edited();
    let layout = "ppt/slideLayouts/slideLayout1.xml";
    let target = xml(&bytes, layout);
    let start = target.find("<p:sp>").unwrap();
    let end = target[start..].find("</p:sp>").unwrap() + start + "</p:sp>".len();
    let mut shape = target[start..end]
        .replace("p:sp>", "p:pic>")
        .replace("p:nvSpPr", "p:nvPicPr")
        .replace("p:cNvSpPr", "p:cNvPicPr");
    let x_start = shape.find("<a:xfrm>").unwrap();
    let x_end = shape.find("</a:xfrm>").unwrap() + "</a:xfrm>".len();
    shape.replace_range(x_start..x_end, "");
    let body_start = shape.find("<p:txBody>").unwrap();
    let body_end = shape.find("</p:txBody>").unwrap() + "</p:txBody>".len();
    shape.replace_range(body_start..body_end, "");
    let target = format!("{}{}{}", &target[..start], shape, &target[end..]);
    // Remove the master's title transform too, so no supported inherited frame exists.
    let master = "ppt/slideMasters/slideMaster1.xml";
    let master_xml = xml(&bytes, master);
    let master_xml = master_xml.replace("<p:ph type=\"title\"/>", "<p:ph type=\"ftr\"/>");
    let bytes = replace(&replace(&bytes, layout, &target), master, &master_xml);
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.reset_slide(0).is_err());
    assert_eq!(s.bytes(), bytes);
    assert_eq!(s.history_depth(), (0, 0));
}

#[test]
fn reset_uses_master_visual_and_body_defaults_and_retains_layout_picture_frame() {
    const INHERITED: &[u8] = include_bytes!("../../../public/fixtures/inherited-deck.pptx");
    let mut s = EditSession::open(INHERITED).unwrap();
    assert!(s.reset_slide(1).unwrap());
    let d = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let one = layout_deck_slide(&d, 0, &HeuristicMeasure).unwrap();
    let two = layout_deck_slide(&d, 1, &HeuristicMeasure).unwrap();
    let p = RecordingPainter::new();
    let q = RecordingPainter::new();
    render_slide(&p, &one, &RenderOptions::default()).unwrap();
    render_slide(&q, &two, &RenderOptions::default()).unwrap();
    assert_eq!(p.ops(), q.ops());
    assert!(!s.reset_slide(1).unwrap());
    assert!(s.undo());
    assert_eq!(s.bytes(), INHERITED);
    assert!(s.reset_slide(2).unwrap());
    let d = jade_pptx_reader::read_deck(s.bytes()).unwrap();
    let picture = layout_deck_slide(&d, 2, &HeuristicMeasure)
        .unwrap()
        .shapes
        .into_iter()
        .find(|s| s.source_id == Some(2) && s.origin == PlacedOrigin::Slide)
        .unwrap();
    assert_eq!(
        (
            picture.rect.x,
            picture.rect.y,
            picture.rect.width,
            picture.rect.height
        ),
        (650.0, 100.0, 300.0, 180.0)
    );
    assert_eq!(
        picture.image_part.as_deref(),
        Some("ppt/media/inherited.png")
    );
    assert_eq!(picture.source_rect.unwrap().left, 10000);
    assert!(!s.reset_slide(2).unwrap());
    assert!(s.undo());
    assert_eq!(s.bytes(), INHERITED);
}
