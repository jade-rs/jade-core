use jade_pptx_reader::{parse_theme_line_styles, read_deck};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
#[test]
fn line_parser_preserves_matrix_slots_no_fill_explicit_solid_and_strict_namespaces() {
    for ns in [
        "http://schemas.openxmlformats.org/drawingml/2006/main",
        "http://purl.oclc.org/ooxml/drawingml/main",
    ] {
        let xml=format!("<a:theme xmlns:a=\"{ns}\"><a:themeElements><a:fmtScheme name=\"lines\"><a:lnStyleLst><a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln><a:ln w=\"25400\"><a:noFill/></a:ln><a:ln w=\"50800\"><a:solidFill><a:srgbClr val=\"112233\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln></a:lnStyleLst></a:fmtScheme></a:themeElements></a:theme>");
        let ls = parse_theme_line_styles(&xml).unwrap();
        assert_eq!(ls.len(), 3);
        assert_eq!(ls[0].dash.as_deref(), Some("solid"));
        assert!(ls[1].no_fill);
        assert_eq!(ls[2].width_emu, Some(50800));
        assert_eq!(ls[2].dash.as_deref(), Some("dash"));
    }
}
#[test]
fn line_styles_follow_the_slide_theme_chain_and_partial_overrides_keep_inherited_format_scheme() {
    let d = read_deck(DECK).unwrap();
    for (i, w, dash) in [
        (0, 76200, "sysDash"),
        (1, 127000, "dashDot"),
        (2, 127000, "dashDot"),
        (3, 50800, "solid"),
    ] {
        let t = d.theme_for_slide(i);
        assert_eq!(t.line_styles.len(), 3);
        assert_eq!(t.line_styles[1].width_emu, Some(w));
        assert_eq!(t.line_styles[1].dash.as_deref(), Some(dash));
    }
    assert_eq!(
        d.theme_for_slide(1).line_styles,
        d.theme_for_slide(2).line_styles
    );
    assert_ne!(
        d.theme_for_slide(2).line_styles,
        d.theme_for_slide(3).line_styles
    );
}
