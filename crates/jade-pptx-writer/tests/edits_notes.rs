use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use jade_pptx_writer::{notes_text, set_slide_notes, EditSession};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn note_part(bytes: &[u8], index: usize) -> String {
    let deck = jade_pptx_reader::read_pptx(bytes).unwrap();
    let package = OpcPackage::open(bytes).unwrap();
    let slide = &deck.slides[index].part_name;
    let rel =
        parse_relationships(&package.required_part_str(&rels_part_name(slide)).unwrap()).unwrap();
    resolve_target(slide, rel.target_by_type_suffix("/notesSlide").unwrap())
}
#[test]
fn existing_notes_preserve_styles_fields_and_unrelated_parts() {
    let mut pkg = OpcPackage::open(DECK).unwrap();
    let part = note_part(DECK, 2);
    pkg.replace(&part,br#"<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="body"/><p:cNvSpPr/><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:pPr algn="ctr"/><a:r><a:rPr b="1"/><a:t>Hello </a:t></a:r><a:r><a:rPr i="1"/><a:t>World</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="number"/><p:cNvSpPr/><p:nvPr><p:ph type="sldNum"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:fld id="{D6501278-C1E9-4C38-B756-FC169E9FC84F}" type="slidenum"><a:t>3</a:t></a:fld></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#.to_vec());
    let bytes = pkg.write().unwrap();
    let changed = set_slide_notes(&bytes, 2, "Hello 新World\n次 🦀").unwrap();
    assert_eq!(
        notes_text(&changed, 2).unwrap().text,
        "Hello 新World\n次 🦀"
    );
    let after = OpcPackage::open(&changed).unwrap();
    let xml = after.required_part_str(&part).unwrap();
    assert!(xml.contains("type=\"slidenum\"><a:t>3</a:t>"));
    assert!(xml.contains("b=\"1\"") && xml.contains("i=\"1\""));
    for p in pkg.part_names() {
        if p != &part {
            assert_eq!(pkg.part(p), after.part(p), "{p}");
        }
    }
    assert_eq!(
        set_slide_notes(&changed, 2, &notes_text(&changed, 2).unwrap().text).unwrap(),
        changed
    );
}
#[test]
fn creates_notes_with_content_type_and_backlink_and_undo_redo_is_exact() {
    let mut session = EditSession::open(DECK).unwrap();
    assert!(!session.set_slide_notes(0, "").unwrap());
    assert_eq!(session.bytes(), DECK);
    session
        .set_slide_notes(0, "日本語 & <notes> 🦀\n次")
        .unwrap();
    let saved = session.bytes().to_vec();
    let part = note_part(&saved, 0);
    let pkg = OpcPackage::open(&saved).unwrap();
    assert!(pkg
        .required_part_str("[Content_Types].xml")
        .unwrap()
        .contains("notesSlide+xml"));
    let rel = parse_relationships(&pkg.required_part_str(&rels_part_name(&part)).unwrap()).unwrap();
    assert_eq!(
        resolve_target(&part, rel.target_by_type_suffix("/slide").unwrap()),
        "ppt/slides/slide1.xml"
    );
    assert_eq!(
        jade_pptx_reader::read_deck(&saved).unwrap().slides[0]
            .notes
            .as_deref(),
        Some("日本語 & <notes> 🦀\n次")
    );
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
    assert!(session.redo());
    assert_eq!(session.bytes(), saved);
    session.duplicate_slide(0).unwrap();
    session.set_slide_notes(1, "independent").unwrap();
    assert_eq!(
        notes_text(session.bytes(), 0).unwrap().text,
        "日本語 & <notes> 🦀\n次"
    );
}
#[test]
fn removing_note_text_keeps_note_part_and_other_placeholders() {
    let part = note_part(DECK, 2);
    let changed = set_slide_notes(DECK, 2, "").unwrap();
    assert!(OpcPackage::open(&changed).unwrap().part(&part).is_some());
    assert_eq!(notes_text(&changed, 2).unwrap().text, "");
    assert!(jade_pptx_reader::read_deck(&changed).unwrap().slides[2]
        .notes
        .is_none());
}
#[test]
fn invalid_characters_indexes_locks_and_body_fields_reject_without_history() {
    let mut session = EditSession::open(DECK).unwrap();
    assert!(session.set_slide_notes(0, "bad\0").is_err());
    assert!(session.set_slide_notes(99, "bad").is_err());
    assert_eq!(session.bytes(), DECK);
    assert!(!session.can_undo());
    let part = note_part(DECK, 2);
    let pkg = OpcPackage::open(DECK).unwrap();
    let xml = pkg.required_part_str(&part).unwrap();
    for altered in [
        xml.replace(
            "<p:cNvSpPr/>",
            "<p:cNvSpPr><a:spLocks noTextEdit=\"1\"/></p:cNvSpPr>",
        ),
        xml.replacen("<a:r>", "<a:fld type=\"datetime\" id=\"{x}\">", 1)
            .replacen("</a:r>", "</a:fld>", 1),
    ] {
        let mut pkg = OpcPackage::open(DECK).unwrap();
        pkg.replace(&part, altered.into_bytes());
        let bytes = pkg.write().unwrap();
        assert!(!notes_text(&bytes, 2).unwrap().can_edit);
        assert!(set_slide_notes(&bytes, 2, "bad").is_err());
    }
}
#[test]
fn existing_notes_without_a_body_get_one_without_losing_the_image_placeholder() {
    let part = note_part(DECK, 2);
    let mut pkg = OpcPackage::open(DECK).unwrap();
    let xml = pkg
        .required_part_str(&part)
        .unwrap()
        .replace("type=\"body\"", "type=\"sldImg\"");
    pkg.replace(&part, xml.into_bytes());
    let bytes = pkg.write().unwrap();
    assert_eq!(notes_text(&bytes, 2).unwrap().text, "");
    let changed = set_slide_notes(&bytes, 2, "added body").unwrap();
    assert_eq!(notes_text(&changed, 2).unwrap().text, "added body");
    assert!(OpcPackage::open(&changed)
        .unwrap()
        .required_part_str(&part)
        .unwrap()
        .contains("type=\"sldImg\""));
}

#[test]
fn an_inherited_body_without_text_can_be_filled_and_keeps_extensions() {
    let part = note_part(DECK, 2);
    let mut pkg = OpcPackage::open(DECK).unwrap();
    let xml = pkg.required_part_str(&part).unwrap();
    let start = xml.find("<p:txBody>").unwrap();
    let end = xml.find("</p:txBody>").unwrap() + "</p:txBody>".len();
    let mut xml = xml;
    xml.replace_range(
        start..end,
        "<p:extLst><p:ext uri=\"test\"><x:data xmlns:x=\"urn:test\"/></p:ext></p:extLst>",
    );
    pkg.replace(&part, xml.into_bytes());
    let bytes = pkg.write().unwrap();
    assert!(notes_text(&bytes, 2).unwrap().can_edit);
    let changed = set_slide_notes(&bytes, 2, "inherited body").unwrap();
    assert_eq!(notes_text(&changed, 2).unwrap().text, "inherited body");
    assert!(OpcPackage::open(&changed)
        .unwrap()
        .required_part_str(&part)
        .unwrap()
        .contains("<x:data xmlns:x=\"urn:test\"/>"));
}
#[test]
fn soft_breaks_and_root_level_slide_part_paths_remain_valid() {
    let mut pkg = OpcPackage::open(DECK).unwrap();
    let slide = pkg.part("ppt/slides/slide1.xml").unwrap().to_vec();
    pkg.replace("root-slide.xml", slide);
    let rels = pkg
        .required_part_str("ppt/_rels/presentation.xml.rels")
        .unwrap()
        .replace("slides/slide1.xml", "/root-slide.xml");
    pkg.replace("ppt/_rels/presentation.xml.rels", rels.into_bytes());
    let bytes = pkg.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    session.set_slide_notes(0, "one").unwrap();
    session
        .set_slide_notes_with_breaks(0, "one\ntwo", true)
        .unwrap();
    let part = note_part(session.bytes(), 0);
    assert!(!part.starts_with('/'));
    let pkg = OpcPackage::open(session.bytes()).unwrap();
    let xml = pkg.required_part_str(&part).unwrap();
    assert!(xml.contains(":br"));
    assert_eq!(notes_text(session.bytes(), 0).unwrap().text, "one\ntwo");
}
