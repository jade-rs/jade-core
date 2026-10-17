use jade_opc::OpcPackage;
use jade_pptx_reader::read_deck;
use std::sync::Arc;
const DECK: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
#[test]
fn every_slide_uses_its_own_master_and_partial_overrides_keep_other_schemes() {
    let deck = read_deck(DECK).unwrap();
    assert_eq!(deck.slides.len(), 4);
    let a = deck.theme_for_slide(0);
    let b = deck.theme_for_slide(1);
    let c = deck.theme_for_slide(2);
    let d = deck.theme_for_slide(3);
    assert_eq!(a.colors.get("accent1"), Some("F01414"));
    assert_eq!(a.fonts.minor_latin.as_deref(), Some("monospace"));
    assert_eq!(b.colors.get("accent1"), Some("E6A010"));
    assert_eq!(b.colors.get("tx1"), Some("1414F0"));
    assert_eq!(b.fonts.minor_latin.as_deref(), Some("serif"));
    assert_eq!(c.fonts.minor_latin.as_deref(), Some("sans-serif"));
    assert_eq!(c.colors.get("tx1"), Some("A020F0"));
    assert_eq!(d.fonts.minor_latin.as_deref(), Some("monospace"));
    assert_eq!(d.colors, c.colors);
    assert_ne!(d.fill_styles, c.fill_styles);
    assert!(deck.slides[3].content.use_master_color_map);
    assert!(!deck.slides[2].content.use_master_color_map);
}
#[test]
fn shared_theme_chains_share_one_snapshot_and_missing_themes_do_not_borrow_another_master() {
    let mut p = OpcPackage::open(DECK).unwrap();
    p.replace(
        "ppt/slides/slide5.xml",
        p.part("ppt/slides/slide1.xml").unwrap().to_vec(),
    );
    p.replace(
        "ppt/slides/_rels/slide5.xml.rels",
        p.part("ppt/slides/_rels/slide1.xml.rels").unwrap().to_vec(),
    );
    let pres = p
        .required_part_str("ppt/presentation.xml")
        .unwrap()
        .replace(
            "</p:sldIdLst>",
            "<p:sldId id=\"260\" r:id=\"rId8\"/></p:sldIdLst>",
        );
    p.replace("ppt/presentation.xml", pres.into_bytes());
    let rels=p.required_part_str("ppt/_rels/presentation.xml.rels").unwrap().replace("</Relationships>","<Relationship Id=\"rId8\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide5.xml\"/></Relationships>");
    p.replace("ppt/_rels/presentation.xml.rels", rels.into_bytes());
    let d = read_deck(&p.write().unwrap()).unwrap();
    assert!(Arc::ptr_eq(
        d.slides[0].theme.as_ref().unwrap(),
        d.slides[4].theme.as_ref().unwrap()
    ));
    p.remove("ppt/theme/theme2.xml");
    let d = read_deck(&p.write().unwrap()).unwrap();
    assert_eq!(d.theme_for_slide(1).colors.get("accent1"), None);
    assert_eq!(d.theme_for_slide(1).fonts.minor_latin, None);
    assert_eq!(d.theme_for_slide(0).colors.get("accent1"), Some("F01414"));
}
#[test]
fn missing_or_external_overrides_keep_the_inherited_schemes() {
    let mut p = OpcPackage::open(DECK).unwrap();
    p.remove("ppt/theme/slideOverride.xml");
    let d = read_deck(&p.write().unwrap()).unwrap();
    assert_eq!(
        d.theme_for_slide(3).fonts.minor_latin.as_deref(),
        Some("sans-serif")
    );
    let mut p = OpcPackage::open(DECK).unwrap();
    let rels = p
        .required_part_str("ppt/slides/_rels/slide4.xml.rels")
        .unwrap()
        .replace(
            "Target=\"../theme/slideOverride.xml\"",
            "Target=\"../theme/slideOverride.xml\" TargetMode=\"External\"",
        );
    p.replace("ppt/slides/_rels/slide4.xml.rels", rels.into_bytes());
    let d = read_deck(&p.write().unwrap()).unwrap();
    assert_eq!(
        d.theme_for_slide(3).fonts.minor_latin.as_deref(),
        Some("sans-serif")
    );
    assert_eq!(
        d.theme_for_slide(3).fill_styles,
        d.theme_for_slide(2).fill_styles
    );
}
#[test]
fn strict_namespaces_resolve_the_same_theme_chains_and_explicit_master_mapping() {
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
    let strict = read_deck(&p.write().unwrap()).unwrap();
    let original = read_deck(DECK).unwrap();
    for index in 0..4 {
        assert_eq!(strict.slides[index].theme, original.slides[index].theme);
        assert_eq!(
            strict.slides[index].content.use_master_color_map,
            original.slides[index].content.use_master_color_map
        );
    }
}
