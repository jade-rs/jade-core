use jade_opc::OpcPackage;
use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure};
use jade_pptx_writer::{validate_slides_hidden, EditSession};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn visibility_source(values: &[&str]) -> Vec<u8> {
    let mut package = OpcPackage::open(DECK).unwrap();
    for (i, value) in values.iter().enumerate() {
        let part = format!("ppt/slides/slide{}.xml", i + 1);
        let xml = package.required_part_str(&part).unwrap();
        package.replace(
            &part,
            xml.replacen("<p:sld ", &format!("<p:sld show='{value}' "), 1)
                .into_bytes(),
        );
    }
    package.write().unwrap()
}
#[test]
fn mixed_batch_visibility_preserves_content_paint_and_unchanged_parts_in_one_history() {
    let mut package = OpcPackage::open(&visibility_source(&["false"])).unwrap();
    package.replace("docProps/app.xml", br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Slides>3</Slides><HiddenSlides>1</HiddenSlides><Notes>17</Notes></Properties>"#.to_vec());
    let bytes = package.write().unwrap();
    let original = jade_pptx_reader::read_deck(&bytes).unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.set_slides_hidden(&[2, 0], true).unwrap());
    let saved = session.bytes().to_vec();
    let deck = jade_pptx_reader::read_deck(&saved).unwrap();
    let mut expected = original.clone();
    expected.slides[2].content.hidden = true;
    assert_eq!(deck, expected);
    let old = OpcPackage::open(&bytes).unwrap();
    let new = OpcPackage::open(&saved).unwrap();
    assert_eq!(old.part_names(), new.part_names());
    for part in old
        .part_names()
        .iter()
        .filter(|p| !matches!(p.as_str(), "ppt/slides/slide3.xml" | "docProps/app.xml"))
    {
        assert_eq!(old.part(part), new.part(part), "{part}");
    }
    let properties = new.required_part_str("docProps/app.xml").unwrap();
    assert!(properties.contains("<Slides>3</Slides>"));
    assert!(properties.contains("<HiddenSlides>2</HiddenSlides>"));
    assert!(properties.contains("<Notes>17</Notes>"));
    // Hidden slides remain fully drawn in the editor, including their notes and content.
    for slide in 0..3 {
        let before = jade_painter::recording::RecordingPainter::new();
        let after = jade_painter::recording::RecordingPainter::new();
        jade_pptx_render::render_slide(
            &before,
            &layout_deck_slide(&original, slide, &HeuristicMeasure).unwrap(),
            &Default::default(),
        )
        .unwrap();
        jade_pptx_render::render_slide(
            &after,
            &layout_deck_slide(&deck, slide, &HeuristicMeasure).unwrap(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(before.ops(), after.ops());
    }
    assert_eq!(session.history_depth(), (1, 0));
    assert!(!session.set_slides_hidden(&[0, 2], true).unwrap());
    assert_eq!(session.bytes(), saved);
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    assert!(!session.undo());
    assert!(session.redo());
    assert_eq!(session.bytes(), saved);
}
#[test]
fn boolean_lexical_variants_and_repeat_noops_keep_exact_archive_and_history() {
    for (values, hidden) in [(["0", "false", "0"], true), (["1", "true", "1"], false)] {
        let bytes = visibility_source(&values);
        let mut session = EditSession::open(&bytes).unwrap();
        assert!(!session.set_slides_hidden(&[2, 0, 1], hidden).unwrap());
        assert_eq!(session.bytes(), bytes);
        assert_eq!(session.history_depth(), (0, 0));
        assert!(session.set_slides_hidden(&[2, 1], !hidden).unwrap());
        let saved = session.bytes().to_vec();
        let deck = jade_pptx_reader::read_deck(&saved).unwrap();
        assert_eq!(
            deck.slides
                .iter()
                .map(|s| s.content.hidden)
                .collect::<Vec<_>>(),
            vec![hidden, !hidden, !hidden]
        );
        let old = OpcPackage::open(&bytes).unwrap();
        let new = OpcPackage::open(&saved).unwrap();
        assert_eq!(
            old.part("ppt/slides/slide1.xml"),
            new.part("ppt/slides/slide1.xml")
        );
        assert!(!session.set_slides_hidden(&[1, 2], !hidden).unwrap());
        assert_eq!(session.bytes(), saved);
        assert_eq!(session.history_depth(), (1, 0));
        assert!(session.undo());
        assert_eq!(session.bytes(), bytes);
        assert!(session.redo());
        assert_eq!(session.bytes(), saved);
    }
}
#[test]
fn invalid_sets_and_late_non_slide_roots_reject_without_partial_changes() {
    let mut session = EditSession::open(DECK).unwrap();
    for targets in [&[][..], &[0, 0], &[0, 99]] {
        assert!(session.set_slides_hidden(targets, true).is_err());
        assert_eq!(session.bytes(), DECK);
        assert_eq!(session.history_depth(), (0, 0));
    }
    let mut package = OpcPackage::open(DECK).unwrap();
    let part = "ppt/slides/slide3.xml";
    let xml = package
        .required_part_str(part)
        .unwrap()
        .replace("<p:sld ", "<p:sldLayout ")
        .replace("</p:sld>", "</p:sldLayout>");
    package.replace(part, xml.into_bytes());
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(validate_slides_hidden(&bytes, &[0, 2], true).is_err());
    assert!(session.set_slides_hidden(&[0, 2], true).is_err());
    assert!(session.set_slide_hidden(2, true).is_err());
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.history_depth(), (0, 0));
}
#[test]
fn strict_visibility_edits_preserve_unknown_slide_attributes_and_exact_history() {
    let mut package = OpcPackage::open(DECK).unwrap();
    for part in package.part_names().to_vec() {
        if part.ends_with(".xml") || part.ends_with(".rels") {
            let xml = package
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
            package.replace(&part, xml.into_bytes());
        }
    }
    let part = "ppt/slides/slide3.xml";
    let xml = package.required_part_str(part).unwrap().replacen(
        "<p:sld ",
        "<p:sld xmlns:v='urn:vendor' v:keep='one &amp; two' ",
        1,
    );
    package.replace(part, xml.into_bytes());
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.set_slides_hidden(&[2, 0], true).unwrap());
    let new = OpcPackage::open(session.bytes()).unwrap();
    let xml = new.required_part_str(part).unwrap();
    assert!(xml.contains("v:keep='one &amp; two'"));
    assert!(xml.contains("http://purl.oclc.org/ooxml/presentationml/main"));
    assert_eq!(
        jade_pptx_reader::read_deck(session.bytes())
            .unwrap()
            .slides
            .iter()
            .map(|s| s.content.hidden)
            .collect::<Vec<_>>(),
        vec![true, false, true]
    );
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    assert!(session.redo());
}
