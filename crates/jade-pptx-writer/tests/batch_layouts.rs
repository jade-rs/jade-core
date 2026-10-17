use jade_opc::OpcPackage;
use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure};
use jade_pptx_writer::{set_slide_layout, shape_text, slide_layouts, EditSession};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
const INHERITED: &[u8] = include_bytes!("../../../public/fixtures/inherited-deck.pptx");
const TARGET: &str = "ppt/slideLayouts/slideLayout2.xml";
fn unchanged(before: &[u8], after: &[u8], allowed: &[&str]) {
    let p = OpcPackage::open(before).unwrap();
    let q = OpcPackage::open(after).unwrap();
    assert_eq!(p.part_names(), q.part_names());
    for name in p
        .part_names()
        .iter()
        .filter(|n| !allowed.contains(&n.as_str()))
    {
        assert_eq!(p.part(name), q.part(name), "{name}");
    }
}
#[test]
fn noncontiguous_batch_layout_has_one_exact_history_entry_and_matches_individual_edits() {
    let individual =
        set_slide_layout(&set_slide_layout(DECK, 0, TARGET).unwrap(), 2, TARGET).unwrap();
    let mut s = EditSession::open(DECK).unwrap();
    assert!(s.set_slide_layouts(&[2, 0], TARGET).unwrap());
    let saved = s.bytes().to_vec();
    let p = OpcPackage::open(&individual).unwrap();
    let q = OpcPackage::open(&saved).unwrap();
    for name in p.part_names() {
        assert_eq!(p.part(name), q.part(name), "{name}");
    }
    for slide in [0, 2] {
        assert!(slide_layouts(&saved, slide)
            .unwrap()
            .iter()
            .any(|l| l.current && l.part_name == TARGET));
        assert_eq!(
            shape_text(&saved, slide, 2).unwrap().text,
            shape_text(DECK, slide, 2).unwrap().text
        );
    }
    unchanged(
        DECK,
        &saved,
        &[
            "ppt/slides/slide1.xml",
            "ppt/slides/slide3.xml",
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/slides/_rels/slide3.xml.rels",
        ],
    );
    assert_eq!(s.history_depth(), (1, 0));
    assert!(!s.set_slide_layouts(&[0, 2], TARGET).unwrap());
    assert_eq!(s.bytes(), saved);
    assert!(s.undo());
    assert_eq!(s.bytes(), DECK);
    assert!(!s.undo());
    assert!(s.redo());
    assert_eq!(s.bytes(), saved);
}
#[test]
fn multi_slide_reset_restores_both_frames_and_painter_style_in_one_commit() {
    let mut prep = EditSession::open(INHERITED).unwrap();
    for slide in [0, 1] {
        let mut t = jade_pptx_writer::editable_shapes(INHERITED, slide)
            .unwrap()
            .into_iter()
            .find(|s| s.id == 2)
            .unwrap()
            .transform
            .unwrap();
        t.offset_emu.0 += 95250;
        prep.set_shape_transform(slide, 2, t).unwrap();
    }
    let before = prep.bytes().to_vec();
    let mut s = EditSession::open(&before).unwrap();
    assert!(s.reset_slides(&[0, 1]).unwrap());
    let saved = s.bytes().to_vec();
    let deck = jade_pptx_reader::read_deck(&saved).unwrap();
    let one = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let two = layout_deck_slide(&deck, 1, &HeuristicMeasure).unwrap();
    let a = jade_painter::recording::RecordingPainter::new();
    let b = jade_painter::recording::RecordingPainter::new();
    jade_pptx_render::render_slide(&a, &one, &Default::default()).unwrap();
    jade_pptx_render::render_slide(&b, &two, &Default::default()).unwrap();
    assert_eq!(a.ops(), b.ops());
    unchanged(
        &before,
        &saved,
        &["ppt/slides/slide1.xml", "ppt/slides/slide2.xml"],
    );
    assert_eq!(s.history_depth(), (1, 0));
    assert!(!s.reset_slides(&[0, 1]).unwrap());
    assert!(s.undo());
    assert_eq!(s.bytes(), before);
    assert!(s.redo());
    assert_eq!(s.bytes(), saved);
}
#[test]
fn invalid_late_target_locked_placeholder_and_invalid_sets_are_atomic() {
    let mut s = EditSession::open(DECK).unwrap();
    for ids in [vec![], vec![0, 0], vec![0, 99]] {
        assert!(s.set_slide_layouts(&ids, TARGET).is_err());
        assert!(s.reset_slides(&ids).is_err());
        assert_eq!(s.bytes(), DECK);
        assert_eq!(s.history_depth(), (0, 0));
    }
    assert!(s.set_slide_layouts(&[0, 1], "unregistered").is_err());
    assert_eq!(s.bytes(), DECK);
    let mut p = OpcPackage::open(INHERITED).unwrap();
    let part = "ppt/slides/slide2.xml";
    let xml = p.required_part_str(part).unwrap().replace(
        "<p:cNvSpPr/>",
        "<p:cNvSpPr><a:spLocks noTextEdit=\"1\"/></p:cNvSpPr>",
    );
    p.replace(part, xml.into_bytes());
    let bytes = p.write().unwrap();
    let mut prep = EditSession::open(&bytes).unwrap();
    let mut t = jade_pptx_writer::editable_shapes(&bytes, 0)
        .unwrap()
        .into_iter()
        .find(|i| i.id == 2)
        .unwrap()
        .transform
        .unwrap();
    t.offset_emu.0 += 95250;
    prep.set_shape_transform(0, 2, t).unwrap();
    let bytes = prep.bytes().to_vec();
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.reset_slides(&[0, 1]).is_err());
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
}
#[test]
fn mixed_noop_and_changed_targets_only_touch_changed_parts() {
    let mut s = EditSession::open(INHERITED).unwrap();
    assert!(s.reset_slides(&[0, 1]).unwrap());
    unchanged(INHERITED, s.bytes(), &["ppt/slides/slide2.xml"]);
    assert_eq!(s.history_depth(), (1, 0));
    assert!(s.undo());
    assert_eq!(s.bytes(), INHERITED);
}

#[test]
fn strict_batch_layout_and_late_alternate_placeholder_failure_preserve_one_transaction() {
    let mut p = OpcPackage::open(DECK).unwrap();
    for part in p.part_names().to_vec() {
        if part.ends_with(".xml") || part.ends_with(".rels") {
            let xml = p
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
            p.replace(&part, xml.into_bytes());
        }
    }
    let bytes = p.write().unwrap();
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.set_slide_layouts(&[0, 2], TARGET).unwrap());
    assert_eq!(s.history_depth(), (1, 0));
    assert!(s.undo());
    assert_eq!(s.bytes(), bytes);
    let mut p = OpcPackage::open(INHERITED).unwrap();
    let part = "ppt/slides/slide2.xml";
    let xml=p.required_part_str(part).unwrap().replace("<p:sp>","<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:q=\"urn:unsupported\"><mc:Choice Requires=\"q\"><q:shape/></mc:Choice><mc:Fallback><p:sp>").replace("</p:sp>","</p:sp></mc:Fallback></mc:AlternateContent>");
    p.replace(part, xml.into_bytes());
    let bytes = p.write().unwrap();
    assert_ne!(set_slide_layout(&bytes, 0, TARGET).unwrap(), bytes);
    let mut s = EditSession::open(&bytes).unwrap();
    assert!(s.set_slide_layouts(&[0, 1], TARGET).is_err());
    assert_eq!(s.bytes(), bytes);
    assert_eq!(s.history_depth(), (0, 0));
}
