use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use jade_pptx_writer::EditSession;
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn count(bytes: &[u8]) -> usize {
    jade_pptx_reader::read_deck(bytes).unwrap().slides.len()
}
fn notes_part(bytes: &[u8], index: usize) -> String {
    let deck = jade_pptx_reader::read_deck(bytes).unwrap();
    let package = OpcPackage::open(bytes).unwrap();
    let slide = &deck.slides[index].part_name;
    let rels =
        parse_relationships(&package.required_part_str(&rels_part_name(slide)).unwrap()).unwrap();
    resolve_target(slide, rels.target_by_type_suffix("/notesSlide").unwrap())
}
#[test]
fn move_and_hide_preserve_ids_parts_and_history() {
    let before = jade_pptx_reader::read_deck(DECK).unwrap();
    let mut session = EditSession::open(DECK).unwrap();
    assert!(!session.move_slide(0, 0).unwrap());
    assert!(!session.set_slide_hidden(0, false).unwrap());
    assert!(session.move_slide(0, 2).unwrap());
    let moved = jade_pptx_reader::read_deck(session.bytes()).unwrap();
    assert_eq!(
        moved.slides.iter().map(|s| s.id).collect::<Vec<_>>(),
        [
            before.slides[1].id,
            before.slides[2].id,
            before.slides[0].id
        ]
    );
    let old = OpcPackage::open(DECK).unwrap();
    let new = OpcPackage::open(session.bytes()).unwrap();
    for p in old.part_names() {
        if p != "ppt/presentation.xml" && p != "docProps/app.xml" {
            assert_eq!(old.part(p), new.part(p), "{p}");
        }
    }
    assert!(session.set_slide_hidden(2, true).unwrap());
    assert!(
        jade_pptx_reader::read_deck(session.bytes()).unwrap().slides[2]
            .content
            .hidden
    );
    assert!(session.undo());
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
    assert!(session.redo());
}
#[test]
fn duplication_clones_notes_and_rewires_their_backlink_without_changing_source_parts() {
    let mut session = EditSession::open(DECK).unwrap();
    let original = jade_pptx_reader::read_deck(DECK).unwrap();
    assert!(session.duplicate_slide(2).unwrap());
    assert_eq!(count(session.bytes()), 4);
    let copied = jade_pptx_reader::read_deck(session.bytes()).unwrap();
    assert_eq!(copied.slides[2].content, copied.slides[3].content);
    assert_eq!(copied.slides[2].notes, copied.slides[3].notes);
    assert_ne!(copied.slides[2].id, copied.slides[3].id);
    assert_ne!(copied.slides[2].part_name, copied.slides[3].part_name);
    let old_note = notes_part(DECK, 2);
    let new_note = notes_part(session.bytes(), 3);
    assert_ne!(old_note, new_note);
    let package = OpcPackage::open(session.bytes()).unwrap();
    let rels = parse_relationships(
        &package
            .required_part_str(&rels_part_name(&new_note))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        resolve_target(&new_note, rels.target_by_type_suffix("/slide").unwrap()),
        copied.slides[3].part_name
    );
    let old = OpcPackage::open(DECK).unwrap();
    for p in old.part_names() {
        if p != "ppt/presentation.xml"
            && p != "ppt/_rels/presentation.xml.rels"
            && p != "[Content_Types].xml"
            && p != "docProps/app.xml"
        {
            assert_eq!(old.part(p), package.part(p), "{p}");
        }
    }
    assert_eq!(copied.slides[..3], original.slides);
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
    assert!(session.redo());
}
#[test]
fn deletion_removes_owned_notes_but_keeps_shared_media_and_can_remove_the_last_slide() {
    let mut session = EditSession::open(DECK).unwrap();
    let note = notes_part(DECK, 2);
    session.delete_slide(2).unwrap();
    assert_eq!(count(session.bytes()), 2);
    let package = OpcPackage::open(session.bytes()).unwrap();
    assert!(package.part(&note).is_none());
    assert!(package.part("ppt/media/image1.png").is_some());
    session.delete_slide(1).unwrap();
    session.delete_slide(0).unwrap();
    assert_eq!(count(session.bytes()), 0);
    assert!(session.undo());
    assert_eq!(count(session.bytes()), 1);
    assert!(session.undo());
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
}
#[test]
fn new_slides_are_editable_and_empty_decks_can_recover_with_insert() {
    let mut session = EditSession::open(DECK).unwrap();
    session.insert_slide(1).unwrap();
    assert_eq!(count(session.bytes()), 4);
    let runs = jade_pptx_writer::editable_shapes(session.bytes(), 1).unwrap();
    assert!(!runs.is_empty());
    let id = runs[0].id;
    session.set_shape_text(1, id, "new slide", false).unwrap();
    assert_eq!(
        jade_pptx_writer::shape_text(session.bytes(), 1, id)
            .unwrap()
            .text,
        "new slide"
    );
    while count(session.bytes()) > 0 {
        session.delete_slide(0).unwrap();
    }
    session.insert_slide(0).unwrap();
    assert_eq!(count(session.bytes()), 1);
}
#[test]
fn invalid_indexes_never_change_bytes_or_history() {
    let mut session = EditSession::open(DECK).unwrap();
    assert!(session.delete_slide(99).is_err());
    assert!(session.duplicate_slide(3).is_err());
    assert!(session.insert_slide(4).is_err());
    assert!(session.move_slide(0, 3).is_err());
    assert!(session.set_slide_hidden(9, true).is_err());
    assert_eq!(session.bytes(), DECK);
    assert!(!session.can_undo());
}

#[test]
fn duplicate_owned_dependency_graphs_are_independent_and_keep_cycles_and_shared_images() {
    let mut package = OpcPackage::open(DECK).unwrap();
    let part = "ppt/slides/slide1.xml";
    let rels = package.required_part_str(&rels_part_name(part)).unwrap();
    package.replace(&rels_part_name(part),rels.replace("</Relationships>","<Relationship Id=\"rOwned\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/tags\" Target=\"../tags/tag1.xml\"/><Relationship Id=\"rShared\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"/ppt/media/image1.png\"/></Relationships>").into_bytes());
    package.replace("ppt/tags/tag1.xml",b"<p:tagLst xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:tag name=\"edit\" val=\"source\"/></p:tagLst>".to_vec());
    package.replace("ppt/tags/_rels/tag1.xml.rels",b"<pkg:Relationships xmlns:pkg=\"http://schemas.openxmlformats.org/package/2006/relationships\" xmlns:x=\"urn:keep\"><pkg:Relationship Id='child' x:keep='a &amp; b' Type='urn:owned/package' Target='../embeddings/workbook.bin'/></pkg:Relationships>".to_vec());
    package.replace(
        "ppt/embeddings/workbook.bin",
        b"independent embedded content".to_vec(),
    );
    package.replace("ppt/embeddings/_rels/workbook.bin.rels",b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"cycle\" Type=\"urn:owned/parent\" Target=\"../tags/tag1.xml\"/></Relationships>".to_vec());
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    session.duplicate_slide(0).unwrap();
    let copy = jade_pptx_reader::read_deck(session.bytes()).unwrap().slides[1]
        .part_name
        .clone();
    let package = OpcPackage::open(session.bytes()).unwrap();
    let rels =
        parse_relationships(&package.required_part_str(&rels_part_name(&copy)).unwrap()).unwrap();
    let tag = resolve_target(&copy, rels.target("rOwned").unwrap());
    assert_ne!(tag, "ppt/tags/tag1.xml");
    assert_eq!(
        resolve_target(&copy, rels.target("rShared").unwrap()),
        "ppt/media/image1.png"
    );
    let tag_rels_xml = package.required_part_str(&rels_part_name(&tag)).unwrap();
    assert!(tag_rels_xml.contains("x:keep='a &amp; b'"));
    let tag_rels = parse_relationships(&tag_rels_xml).unwrap();
    let embedded = resolve_target(&tag, tag_rels.target("child").unwrap());
    assert_ne!(embedded, "ppt/embeddings/workbook.bin");
    assert_eq!(
        package.part(&embedded),
        package.part("ppt/embeddings/workbook.bin")
    );
    let embedded_rels = parse_relationships(
        &package
            .required_part_str(&rels_part_name(&embedded))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        resolve_target(&embedded, embedded_rels.target("cycle").unwrap()),
        tag
    );
}
#[test]
fn custom_shows_sections_aliases_and_inbound_hyperlinks_follow_slide_operations() {
    let mut package = OpcPackage::open(DECK).unwrap();
    let xml = package.required_part_str("ppt/presentation.xml").unwrap();
    let ids = jade_pptx_reader::read_pptx(DECK)
        .unwrap()
        .slides
        .iter()
        .map(|s| s.id.unwrap())
        .collect::<Vec<_>>();
    let rels = parse_relationships(
        &package
            .required_part_str("ppt/_rels/presentation.xml.rels")
            .unwrap(),
    )
    .unwrap();
    let removed_rel = rels
        .all()
        .iter()
        .find(|r| resolve_target("ppt/presentation.xml", &r.target) == "ppt/slides/slide1.xml")
        .unwrap()
        .id
        .clone();
    let additions = format!("<p:custShowLst><p:custShow name=\"selected\" id=\"1\"><p:sldLst><p:sld r:id=\"{removed_rel}\"/></p:sldLst></p:custShow></p:custShowLst><p:extLst><p:ext uri=\"{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}\"><q:sectionLst xmlns:q=\"http://schemas.microsoft.com/office/powerpoint/2010/main\"><q:section name=\"A\" id=\"{{00000000-0000-0000-0000-000000000001}}\"><q:sldIdLst><!--keep--><q:sldId id=\"{}\"/><q:sldId id=\"{}\"/></q:sldIdLst></q:section><q:section name=\"B\" id=\"{{00000000-0000-0000-0000-000000000002}}\"><q:sldIdLst><q:sldId id=\"{}\"/></q:sldIdLst></q:section></q:sectionLst></p:ext></p:extLst>",ids[0],ids[1],ids[2]);
    // Insert optional presentation children before the default text style.
    let at = xml
        .find("<p:defaultTextStyle")
        .unwrap_or_else(|| xml.find("</p:presentation>").unwrap());
    let mut xml = xml;
    xml.insert_str(at, &additions);
    package.replace("ppt/presentation.xml", xml.into_bytes());
    let mut slide = package.required_part_str("ppt/slides/slide2.xml").unwrap();
    let start = slide.find("<a:rPr").unwrap();
    let end = start + slide[start..].find('>').unwrap() + 1;
    let link = "<a:hlinkClick r:id=\"jump\" action=\"ppaction://hlinksldjump\"/>";
    if slide[..end].ends_with("/>") {
        slide.replace_range(end - 2..end, &format!(">{link}</a:rPr>"));
    } else {
        slide.insert_str(end, link);
    }
    package.replace("ppt/slides/slide2.xml", slide.into_bytes());
    let rels = package.required_part_str("ppt/slides/_rels/slide2.xml.rels").unwrap().replace("</Relationships>","<Relationship Id=\"jump\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slide1.xml\"/></Relationships>");
    package.replace("ppt/slides/_rels/slide2.xml.rels", rels.into_bytes());
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    let mut batch = EditSession::open(&bytes).unwrap();
    batch.delete_slides(&[2, 0]).unwrap();
    let saved = OpcPackage::open(batch.bytes()).unwrap();
    let presentation = saved.required_part_str("ppt/presentation.xml").unwrap();
    assert!(presentation.contains("<!--keep-->"));
    assert!(!presentation.contains(&format!("<p:sld r:id=\"{removed_rel}\"")));
    for deleted in [ids[0], ids[2]] {
        assert!(!presentation.contains(&format!("<q:sldId id=\"{deleted}\"")));
    }
    assert!(presentation.contains(&format!("<q:sldId id=\"{}\"", ids[1])));
    assert!(!saved
        .required_part_str("ppt/slides/slide2.xml")
        .unwrap()
        .contains("r:id=\"jump\""));
    assert_eq!(batch.history_depth(), (1, 0));
    assert!(batch.undo());
    assert_eq!(batch.bytes(), bytes);
    session.duplicate_slide(0).unwrap();
    let p = OpcPackage::open(session.bytes())
        .unwrap()
        .required_part_str("ppt/presentation.xml")
        .unwrap();
    assert!(p.contains("<!--keep-->"));
    session.move_slide(0, 3).unwrap();
    session.delete_slide(3).unwrap();
    let p = OpcPackage::open(session.bytes()).unwrap();
    let xml = p.required_part_str("ppt/presentation.xml").unwrap();
    assert!(!xml.contains(&format!("<p:sld r:id=\"{removed_rel}\"")));
    assert!(!xml.contains(&format!("<q:sldId id=\"{}\"", ids[0])));
    assert!(!p
        .required_part_str("ppt/slides/slide2.xml")
        .unwrap()
        .contains("r:id=\"jump\""));
    assert!(parse_relationships(
        &p.required_part_str("ppt/slides/_rels/slide2.xml.rels")
            .unwrap()
    )
    .unwrap()
    .relationship("jump")
    .is_none());
}

#[test]
fn hidden_boolean_false_is_read_and_preserved_as_a_noop() {
    let mut package = OpcPackage::open(DECK).unwrap();
    let xml = package.required_part_str("ppt/slides/slide1.xml").unwrap();
    package.replace(
        "ppt/slides/slide1.xml",
        xml.replacen("<p:sld ", "<p:sld show='false' ", 1)
            .into_bytes(),
    );
    let bytes = package.write().unwrap();
    assert!(
        jade_pptx_reader::read_deck(&bytes).unwrap().slides[0]
            .content
            .hidden
    );
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(!session.set_slide_hidden(0, true).unwrap());
    assert_eq!(session.bytes(), bytes);
}
#[test]
fn inserting_from_a_layout_clears_prompt_text_preserves_insets_and_copies_needed_relationships() {
    let mut package = OpcPackage::open(DECK).unwrap();
    let rels=package.required_part_str("ppt/slides/_rels/slide1.xml.rels").unwrap().replace("</Relationships>","<Relationship Id=\"layout\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/layout1.xml\"/></Relationships>");
    package.replace("ppt/slides/_rels/slide1.xml.rels", rels.into_bytes());
    package.replace("ppt/slideLayouts/layout1.xml",br#"<p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="4" name="Layout body"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="100" y="200"/><a:ext cx="2000000" cy="1000000"/></a:xfrm><a:blipFill><a:blip r:embed="img"/></a:blipFill></p:spPr><p:txBody><a:bodyPr lIns="444" anchor="ctr"/><a:lstStyle><a:lvl1pPr><a:defRPr sz="2700"/></a:lvl1pPr></a:lstStyle><a:p><a:r><a:t>Layout prompt</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sldLayout>"#.to_vec());
    package.replace("ppt/slideLayouts/_rels/layout1.xml.rels",br#"<pkg:Relationships xmlns:pkg="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:x="urn:keep"><pkg:Relationship Id='img' x:keep='yes' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/image' Target='../media/image1.png'/></pkg:Relationships>"#.to_vec());
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    session.insert_slide(0).unwrap();
    let deck = jade_pptx_reader::read_deck(session.bytes()).unwrap();
    assert_eq!(
        jade_pptx_writer::shape_text(session.bytes(), 0, 4)
            .unwrap()
            .text,
        ""
    );
    let jade_pptx_reader::ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("shape")
    };
    assert_eq!(shape.image_part.as_deref(), Some("ppt/media/image1.png"));
    assert_eq!(shape.text.as_ref().unwrap().insets.left_emu, Some(444));
    assert_eq!(
        jade_pptx_layout::text_format_for_deck_shape(&deck, 0, 4, 0, 0)
            .unwrap()
            .size_pt,
        Some(27.0)
    );
    let copied = OpcPackage::open(session.bytes()).unwrap();
    assert!(copied
        .required_part_str(&rels_part_name(&deck.slides[0].part_name))
        .unwrap()
        .contains("x:keep='yes'"));
}

#[test]
fn batch_deletion_preserves_survivor_notes_shared_parts_and_exact_single_history() {
    let mut prepared = EditSession::open(DECK).unwrap();
    prepared.set_slide_notes(1, "Surviving notes").unwrap();
    let bytes = prepared.bytes().to_vec();
    let original = jade_pptx_reader::read_deck(&bytes).unwrap();
    let removed_note = notes_part(&bytes, 2);
    let kept_note = notes_part(&bytes, 1);
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.delete_slides(&[2, 0]).unwrap());
    let saved = session.bytes().to_vec();
    let deck = jade_pptx_reader::read_deck(&saved).unwrap();
    assert_eq!(deck.slides, vec![original.slides[1].clone()]);
    let old = OpcPackage::open(&bytes).unwrap();
    let package = OpcPackage::open(&saved).unwrap();
    for p in [
        &kept_note,
        &rels_part_name(&kept_note),
        &original.slides[1].part_name,
        &rels_part_name(&original.slides[1].part_name),
    ] {
        assert_eq!(package.part(p), old.part(p), "{p}");
    }
    assert!(package.part(&removed_note).is_none());
    assert_eq!(
        package.part("ppt/media/image1.png"),
        old.part("ppt/media/image1.png")
    );
    assert_eq!(session.history_depth(), (1, 0));
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    assert!(session.redo());
    assert_eq!(session.bytes(), saved);
    assert!(session.delete_slides(&[0]).unwrap());
    assert_eq!(count(session.bytes()), 0);
    assert!(session.undo());
    assert_eq!(session.bytes(), saved);
}

#[test]
fn invalid_batch_deletion_and_late_inbound_reference_preserve_all_bytes_and_history() {
    let mut package = OpcPackage::open(DECK).unwrap();
    let source = "ppt/slides/slide2.xml";
    let xml = package.required_part_str(source).unwrap();
    package.replace(
        source,
        xml.replace("</p:cSld>", "<p:unsupported r:id=\"batchJump\"/></p:cSld>")
            .into_bytes(),
    );
    let rels = package.required_part_str(&rels_part_name(source)).unwrap();
    package.replace(&rels_part_name(source), rels.replace("</Relationships>", "<Relationship Id=\"batchJump\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slide3.xml\"/></Relationships>").into_bytes());
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    for indices in [&[][..], &[0, 0], &[0, 99], &[0, 2]] {
        assert!(session.delete_slides(indices).is_err());
        assert_eq!(session.bytes(), bytes);
        assert_eq!(session.history_depth(), (0, 0));
    }
    // Unsupported cross-references between slides that are all being removed are harmless.
    assert!(session.delete_slides(&[2, 1]).unwrap());
    assert_eq!(count(session.bytes()), 1);
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
}

#[test]
fn strict_batch_deletion_uses_original_indexes_and_restores_exact_archive() {
    let mut package = OpcPackage::open(DECK).unwrap();
    for name in package.part_names().to_vec() {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let xml = package.required_part_str(&name).unwrap();
            package.replace(
                &name,
                xml.replace(
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
                )
                .into_bytes(),
            );
        }
    }
    let bytes = package.write().unwrap();
    let original = jade_pptx_reader::read_deck(&bytes).unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.delete_slides(&[2, 0]).unwrap());
    assert_eq!(
        jade_pptx_reader::read_deck(session.bytes()).unwrap().slides,
        vec![original.slides[1].clone()]
    );
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
}
