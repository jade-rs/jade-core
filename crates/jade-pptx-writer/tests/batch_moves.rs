use jade_opc::{parse_relationships, resolve_target, OpcPackage};
use jade_pptx_writer::EditSession;
const DECK: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
fn preserved(before: &[u8], after: &[u8]) {
    let p = OpcPackage::open(before).unwrap();
    let q = OpcPackage::open(after).unwrap();
    assert_eq!(p.part_names(), q.part_names());
    for part in p
        .part_names()
        .iter()
        .filter(|p| !matches!(p.as_str(), "ppt/presentation.xml" | "docProps/app.xml"))
    {
        assert_eq!(p.part(part), q.part(part), "{part}");
    }
}
#[test]
fn noncontiguous_moves_use_original_order_preserve_all_content_and_one_exact_history() {
    let original = jade_pptx_reader::read_deck(DECK).unwrap();
    for (indices, to, expected) in [
        (vec![3, 1], 0, vec![1, 3, 0, 2]),
        (vec![2, 0], 2, vec![1, 3, 0, 2]),
        (vec![2, 0], 1, vec![1, 0, 2, 3]),
    ] {
        let mut session = EditSession::open(DECK).unwrap();
        assert!(session.move_slides(&indices, to).unwrap());
        let saved = session.bytes().to_vec();
        let deck = jade_pptx_reader::read_deck(&saved).unwrap();
        assert_eq!(
            deck.slides,
            expected
                .iter()
                .map(|&i| original.slides[i].clone())
                .collect::<Vec<_>>()
        );
        preserved(DECK, &saved);
        assert_eq!(session.history_depth(), (1, 0));
        assert!(session.undo());
        assert_eq!(session.bytes(), DECK);
        assert!(!session.undo());
        assert!(session.redo());
        assert_eq!(session.bytes(), saved);
        let targets: Vec<_> = (to..to + indices.len()).collect();
        assert!(!session.move_slides(&targets, to).unwrap());
        assert_eq!(session.bytes(), saved);
    }
}
#[test]
fn every_subset_and_destination_keeps_selected_and_unselected_order_and_exact_noops() {
    let source = jade_pptx_reader::read_deck(DECK).unwrap();
    let ids: Vec<_> = source.slides.iter().map(|s| s.id).collect();
    for mask in 1..16 {
        let indices: Vec<_> = (0..4).filter(|i| mask & (1 << i) != 0).collect();
        let selected: Vec<_> = indices.iter().map(|&i| ids[i]).collect();
        let unselected: Vec<_> = (0..4)
            .filter(|i| !indices.contains(i))
            .map(|i| ids[i])
            .collect();
        for to in 0..=4 - indices.len() {
            let mut session = EditSession::open(DECK).unwrap();
            let reversed: Vec<_> = indices.iter().rev().copied().collect();
            let changed = session.move_slides(&reversed, to).unwrap();
            let deck = jade_pptx_reader::read_deck(session.bytes()).unwrap();
            let actual: Vec<_> = deck.slides.iter().map(|s| s.id).collect();
            assert_eq!(actual[to..to + indices.len()], selected);
            assert_eq!(
                actual
                    .iter()
                    .filter(|id| !selected.contains(id))
                    .copied()
                    .collect::<Vec<_>>(),
                unselected
            );
            assert_eq!(changed, actual != ids);
            preserved(DECK, session.bytes());
            if changed {
                assert_eq!(session.history_depth(), (1, 0));
                assert!(session.undo());
                assert_eq!(session.bytes(), DECK);
            } else {
                assert_eq!(session.history_depth(), (0, 0));
                assert_eq!(session.bytes(), DECK);
            }
        }
    }
}
#[test]
fn batch_cross_section_moves_use_the_unselected_destination_and_preserve_custom_shows() {
    let original = jade_pptx_reader::read_deck(DECK).unwrap();
    let ids: Vec<_> = original.slides.iter().map(|s| s.id.unwrap()).collect();
    let mut package = OpcPackage::open(DECK).unwrap();
    let rels = parse_relationships(
        &package
            .required_part_str("ppt/_rels/presentation.xml.rels")
            .unwrap(),
    )
    .unwrap();
    let rids: Vec<_> = original
        .slides
        .iter()
        .map(|s| {
            rels.all()
                .iter()
                .find(|r| resolve_target("ppt/presentation.xml", &r.target) == s.part_name)
                .unwrap()
                .id
                .clone()
        })
        .collect();
    let show=format!("<p:custShowLst><p:custShow name=\"independent\" id=\"1\"><p:sldLst><p:sld r:id=\"{}\"/><p:sld r:id=\"{}\"/></p:sldLst></p:custShow></p:custShowLst>",rids[3],rids[1]);
    let sections=format!("<p:extLst><p:ext uri=\"{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}\"><q:sectionLst xmlns:q=\"http://schemas.microsoft.com/office/powerpoint/2010/main\"><q:section name=\"A\" id=\"{{00000000-0000-0000-0000-000000000001}}\"><q:sldIdLst><!--keep--><q:sldId id=\"{}\"/><q:sldId id=\"{}\"/></q:sldIdLst></q:section><q:section name=\"B\" id=\"{{00000000-0000-0000-0000-000000000002}}\"><q:sldIdLst><q:sldId id=\"{}\"/><q:sldId id=\"{}\"/></q:sldIdLst></q:section></q:sectionLst></p:ext></p:extLst>",ids[0],ids[1],ids[2],ids[3]);
    let xml = package.required_part_str("ppt/presentation.xml").unwrap();
    package.replace(
        "ppt/presentation.xml",
        xml.replace("<p:defaultTextStyle", &format!("{show}<p:defaultTextStyle"))
            .replace("</p:presentation>", &format!("{sections}</p:presentation>"))
            .into_bytes(),
    );
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    session.move_slides(&[2, 0], 2).unwrap();
    let saved = OpcPackage::open(session.bytes()).unwrap();
    let xml = saved.required_part_str("ppt/presentation.xml").unwrap();
    assert!(xml.contains(&show));
    assert!(xml.contains("<!--keep-->"));
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
    assert!(a.contains(&format!("<q:sldId id=\"{}\"", ids[1])));
    assert!(!a.contains(&format!("<q:sldId id=\"{}\"", ids[0])));
    let actual: Vec<u32> = b
        .split("<q:sldId ")
        .skip(1)
        .map(|node| {
            node.split("id=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(actual, [ids[3], ids[0], ids[2]]);
    preserved(&bytes, session.bytes());
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
}
#[test]
fn invalid_sets_destinations_and_strict_moves_preserve_history_and_namespace() {
    let mut session = EditSession::open(DECK).unwrap();
    for (ids, to) in [
        (&[][..], 0),
        (&[0, 0], 0),
        (&[0, 99], 0),
        (&[0, 2], 3),
        (&[0], 4),
        (&[0, 1, 2, 3], 1),
    ] {
        assert!(session.move_slides(ids, to).is_err());
        assert_eq!(session.bytes(), DECK);
        assert_eq!(session.history_depth(), (0, 0));
    }
    let mut package = OpcPackage::open(DECK).unwrap();
    for name in package.part_names().to_vec() {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let xml = package
                .required_part_str(&name)
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
            package.replace(&name, xml.into_bytes());
        }
    }
    let bytes = package.write().unwrap();
    let mut session = EditSession::open(&bytes).unwrap();
    assert!(session.move_slides(&[3, 1], 0).unwrap());
    preserved(&bytes, session.bytes());
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
}
