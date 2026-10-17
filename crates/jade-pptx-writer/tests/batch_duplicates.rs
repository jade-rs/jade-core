use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use jade_pptx_writer::EditSession;
const DECK: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
fn target(package: &OpcPackage, part: &str, suffix: &str) -> String {
    let rels =
        parse_relationships(&package.required_part_str(&rels_part_name(part)).unwrap()).unwrap();
    resolve_target(part, rels.target_by_type_suffix(suffix).unwrap())
}
fn originals_unchanged(before: &[u8], after: &[u8]) {
    let p = OpcPackage::open(before).unwrap();
    let q = OpcPackage::open(after).unwrap();
    for part in p.part_names().iter().filter(|p| {
        !matches!(
            p.as_str(),
            "ppt/presentation.xml"
                | "ppt/_rels/presentation.xml.rels"
                | "[Content_Types].xml"
                | "docProps/app.xml"
        )
    }) {
        assert_eq!(p.part(part), q.part(part), "{part}");
    }
}
#[test]
fn reversed_noncontiguous_sources_copy_in_order_after_last_target_and_one_exact_undo() {
    let mut preparation = EditSession::open(DECK).unwrap();
    preparation.set_slide_notes(0, "First notes").unwrap();
    preparation.set_slide_notes(2, "Third notes").unwrap();
    preparation.set_slide_hidden(2, true).unwrap();
    let bytes = preparation.bytes().to_vec();
    let original = jade_pptx_reader::read_deck(&bytes).unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.duplicate_slides(&[2, 0]).unwrap());
    let saved = session.bytes().to_vec();
    let deck = jade_pptx_reader::read_deck(&saved).unwrap();
    assert_eq!(deck.slides.len(), 6);
    assert_eq!(deck.slides[..3], original.slides[..3]);
    assert_eq!(deck.slides[5], original.slides[3]);
    let package = OpcPackage::open(&saved).unwrap();
    let old = OpcPackage::open(&bytes).unwrap();
    for (source, copy) in [(0, 3), (2, 4)] {
        assert_ne!(deck.slides[source].id, deck.slides[copy].id);
        assert_ne!(deck.slides[source].part_name, deck.slides[copy].part_name);
        assert_eq!(deck.slides[source].content, deck.slides[copy].content);
        assert_eq!(deck.slides[source].notes, deck.slides[copy].notes);
        let before = jade_painter::recording::RecordingPainter::new();
        let after = jade_painter::recording::RecordingPainter::new();
        jade_pptx_render::render_slide(
            &before,
            &jade_pptx_layout::layout_deck_slide(
                &original,
                source,
                &jade_pptx_layout::HeuristicMeasure,
            )
            .unwrap(),
            &Default::default(),
        )
        .unwrap();
        jade_pptx_render::render_slide(
            &after,
            &jade_pptx_layout::layout_deck_slide(&deck, copy, &jade_pptx_layout::HeuristicMeasure)
                .unwrap(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(before.ops(), after.ops());
        let note = target(&package, &deck.slides[copy].part_name, "/notesSlide");
        assert_ne!(
            note,
            target(&old, &original.slides[source].part_name, "/notesSlide")
        );
        assert_eq!(
            target(&package, &note, "/slide"),
            deck.slides[copy].part_name
        );
    }
    originals_unchanged(&bytes, &saved);
    assert_eq!(session.history_depth(), (1, 0));
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    assert!(!session.undo());
    assert!(session.redo());
    assert_eq!(session.bytes(), saved);
    session.set_slide_notes(3, "Copy notes edited").unwrap();
    assert_eq!(
        jade_pptx_writer::notes_text(session.bytes(), 0)
            .unwrap()
            .text,
        "First notes"
    );
    session.set_shape_text(3, 2, "Copy only", false).unwrap();
    assert_ne!(
        jade_pptx_writer::shape_text(session.bytes(), 0, 2)
            .unwrap()
            .text,
        "Copy only"
    );
}
#[test]
fn shared_owned_dependency_graphs_clone_independently_with_cycles_and_content_types() {
    let mut package = OpcPackage::open(DECK).unwrap();
    for slide in [1, 3] {
        let part = format!("ppt/slides/slide{slide}.xml");
        let rel = rels_part_name(&part);
        let xml = package.required_part_str(&rel).unwrap();
        package.replace(&rel,xml.replace("</Relationships>","<Relationship Id=\"owned\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/tags\" Target=\"../tags/shared.xml\"/></Relationships>").into_bytes());
    }
    package.replace("ppt/tags/shared.xml",br#"<p:tagLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:tag name="source" val="keep"/></p:tagLst>"#.to_vec());
    package.replace("ppt/tags/_rels/shared.xml.rels",br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="child" Type="urn:owned" Target="../embeddings/shared.bin"/></Relationships>"#.to_vec());
    package.replace(
        "ppt/embeddings/shared.bin",
        b"owned binary payload".to_vec(),
    );
    package.replace("ppt/embeddings/_rels/shared.bin.rels",br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="parent" Type="urn:owned" Target="../tags/shared.xml"/></Relationships>"#.to_vec());
    let xml = package.required_part_str("[Content_Types].xml").unwrap();
    package.replace("[Content_Types].xml",xml.replace("</Types>","<Override PartName=\"/ppt/tags/shared.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.tags+xml\"/><Override PartName=\"/ppt/embeddings/shared.bin\" ContentType=\"application/octet-stream\"/></Types>").into_bytes());
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    session.duplicate_slides(&[0, 2]).unwrap();
    let deck = jade_pptx_reader::read_deck(session.bytes()).unwrap();
    let package = OpcPackage::open(session.bytes()).unwrap();
    let mut tags = Vec::new();
    let mut binaries = Vec::new();
    let types = package.required_part_str("[Content_Types].xml").unwrap();
    for copy in [3, 4] {
        let tag = target(&package, &deck.slides[copy].part_name, "/tags");
        let binary = target(&package, &tag, "urn:owned");
        assert_ne!(tag, "ppt/tags/shared.xml");
        assert_ne!(binary, "ppt/embeddings/shared.bin");
        assert_eq!(target(&package, &binary, "urn:owned"), tag);
        for part in [&tag, &binary] {
            assert!(types.contains(&format!("PartName=\"/{part}\"")));
        }
        assert_eq!(package.part(&binary), Some(&b"owned binary payload"[..]));
        tags.push(tag);
        binaries.push(binary);
    }
    assert_ne!(tags[0], tags[1]);
    assert_ne!(binaries[0], binaries[1]);
    originals_unchanged(&bytes, session.bytes());
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
}
#[test]
fn late_missing_owned_parts_and_invalid_sets_never_partially_commit() {
    let mut session = EditSession::open(DECK).unwrap();
    for targets in [&[][..], &[0, 0], &[0, 99]] {
        assert!(session.duplicate_slides(targets).is_err());
        assert_eq!(session.bytes(), DECK);
        assert_eq!(session.history_depth(), (0, 0));
    }
    let mut package = OpcPackage::open(DECK).unwrap();
    let rel = "ppt/slides/_rels/slide3.xml.rels";
    let xml = package.required_part_str(rel).unwrap();
    package.replace(rel,xml.replace("</Relationships>","<Relationship Id=\"missing\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/tags\" Target=\"../tags/missing.xml\"/></Relationships>").into_bytes());
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.duplicate_slides(&[0, 2]).is_err());
    assert_eq!(session.bytes(), bytes);
    assert_eq!(session.history_depth(), (0, 0));
}
#[test]
fn strict_batch_duplication_keeps_namespaces_and_adds_copies_to_insertion_section() {
    let mut package = OpcPackage::open(DECK).unwrap();
    let deck = jade_pptx_reader::read_deck(DECK).unwrap();
    let ids: Vec<_> = deck.slides.iter().map(|s| s.id.unwrap()).collect();
    let xml = package.required_part_str("ppt/presentation.xml").unwrap();
    let sections=format!("<p:extLst><p:ext uri=\"{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}\"><q:sectionLst xmlns:q=\"http://schemas.microsoft.com/office/powerpoint/2010/main\"><q:section name=\"A\" id=\"{{00000000-0000-0000-0000-000000000001}}\"><q:sldIdLst><!--keep--><q:sldId id=\"{}\"/><q:sldId id=\"{}\"/></q:sldIdLst></q:section><q:section name=\"B\" id=\"{{00000000-0000-0000-0000-000000000002}}\"><q:sldIdLst><q:sldId id=\"{}\"/><q:sldId id=\"{}\"/></q:sldIdLst></q:section></q:sectionLst></p:ext></p:extLst>",ids[0],ids[1],ids[2],ids[3]);
    package.replace(
        "ppt/presentation.xml",
        xml.replace("</p:presentation>", &format!("{sections}</p:presentation>"))
            .into_bytes(),
    );
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
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    session.duplicate_slides(&[2, 0]).unwrap();
    let deck = jade_pptx_reader::read_deck(session.bytes()).unwrap();
    let package = OpcPackage::open(session.bytes()).unwrap();
    let xml = package.required_part_str("ppt/presentation.xml").unwrap();
    assert!(xml.contains("<!--keep-->"));
    assert!(xml.contains("http://purl.oclc.org/ooxml/presentationml/main"));
    let a = xml
        .split("name=\"A\"")
        .nth(1)
        .unwrap()
        .split("</q:section>")
        .next()
        .unwrap();
    let b = xml
        .split("name=\"B\"")
        .nth(1)
        .unwrap()
        .split("</q:section>")
        .next()
        .unwrap();
    for slide in [3, 4] {
        let marker = format!("id=\"{}\"", deck.slides[slide].id.unwrap());
        assert!(!a.contains(&marker));
        assert!(b.contains(&marker));
    }
    originals_unchanged(&bytes, session.bytes());
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
}
