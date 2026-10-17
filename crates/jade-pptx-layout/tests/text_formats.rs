use jade_pptx_layout::{layout_deck_slide, text_format_for_deck_shape, HeuristicMeasure};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn deck(body: &str) -> jade_pptx_reader::Deck {
    let mut deck = jade_pptx_reader::read_deck(DECK).unwrap();
    let xml = format!(
        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="7" name="text"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9144000" cy="4572000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr><a:normAutofit fontScale="50000"/></a:bodyPr><a:lstStyle><a:lvl1pPr><a:defRPr sz="2400" b="1" i="1" u="sng"/></a:lvl1pPr></a:lstStyle>{body}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
    );
    deck.slides[0].content = jade_pptx_reader::parse_slide_xml(&xml).unwrap();
    deck
}
#[test]
fn authoring_format_resolves_defaults_mixed_ranges_and_unscaled_size() {
    let deck = deck("<a:p><a:r><a:t>ab</a:t></a:r><a:r><a:rPr b=\"0\" i=\"0\" u=\"none\" sz=\"3600\"/><a:t>cd</a:t></a:r></a:p>");
    let all = text_format_for_deck_shape(&deck, 0, 7, 0, 4).unwrap();
    assert_eq!(
        (all.bold, all.italic, all.underline, all.size_pt),
        (None, None, None, None)
    );
    let caret = text_format_for_deck_shape(&deck, 0, 7, 2, 2).unwrap();
    assert_eq!(
        (caret.bold, caret.underline, caret.size_pt),
        (Some(true), Some(true), Some(24.0))
    );
    let right = text_format_for_deck_shape(&deck, 0, 7, 2, 4).unwrap();
    assert_eq!(
        (right.bold, right.italic, right.underline, right.size_pt),
        (Some(false), Some(false), Some(false), Some(36.0))
    );
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let runs = &layout
        .shapes
        .iter()
        .find(|s| s.source_id == Some(7))
        .unwrap()
        .lines[0]
        .runs;
    assert!(runs[0].underline);
    assert!(!runs[1].underline);
    assert_eq!((runs[0].size_pt, runs[1].size_pt), (12.0, 18.0));
}
#[test]
fn empty_paragraph_marks_and_soft_breaks_supply_typing_format() {
    let deck = deck("<a:p><a:endParaRPr b=\"0\" i=\"0\" u=\"none\" sz=\"1800\"/></a:p><a:p><a:br><a:rPr sz=\"4800\" b=\"0\"/></a:br><a:r><a:t>x</a:t></a:r></a:p>");
    let empty = text_format_for_deck_shape(&deck, 0, 7, 0, 0).unwrap();
    assert_eq!(
        (empty.bold, empty.italic, empty.underline, empty.size_pt),
        (Some(false), Some(false), Some(false), Some(18.0))
    );
    let after_break = text_format_for_deck_shape(&deck, 0, 7, 2, 2).unwrap();
    assert_eq!(
        (after_break.bold, after_break.size_pt),
        (Some(false), Some(48.0))
    );
}
#[test]
fn explicit_no_underline_overrides_hyperlink_default_and_inherited_underline() {
    let deck = deck("<a:p><a:r><a:rPr u=\"none\"><a:hlinkClick r:id=\"rId99\"/></a:rPr><a:t>link</a:t></a:r></a:p>");
    assert_eq!(
        text_format_for_deck_shape(&deck, 0, 7, 0, 4)
            .unwrap()
            .underline,
        Some(false)
    );
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    assert!(
        !layout
            .shapes
            .iter()
            .find(|s| s.source_id == Some(7))
            .unwrap()
            .lines[0]
            .runs[0]
            .underline
    );
}

#[test]
fn font_color_state_uses_render_inheritance_and_explicit_link_color() {
    let mut deck = deck(
        r#"<a:p><a:pPr><a:defRPr><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/></a:defRPr></a:pPr><a:r><a:t>a</a:t></a:r><a:r><a:rPr><a:solidFill><a:srgbClr val="FF8800"/></a:solidFill><a:latin typeface="Arial"/><a:ea typeface="游ゴシック"/><a:hlinkClick r:id="rId99"/></a:rPr><a:t>b</a:t></a:r></a:p>"#,
    );
    // Legacy manually supplied decks still resolve through deck-level defaults.
    deck.slides[0].theme = None;
    deck.theme_fonts.minor_latin = Some("Aptos".into());
    deck.theme_fonts.minor_east_asian = Some("Yu Gothic".into());
    deck.slides[0].content.color_map_override = Some(std::collections::HashMap::from([(
        "tx1".into(),
        "accent2".into(),
    )]));
    let left = text_format_for_deck_shape(&deck, 0, 7, 0, 1).unwrap();
    assert_eq!(left.font.as_deref(), Some("Aptos"));
    assert_eq!(left.east_asian_font.as_deref(), Some("Yu Gothic"));
    let right = text_format_for_deck_shape(&deck, 0, 7, 1, 2).unwrap();
    assert_eq!(right.color.as_deref(), Some("#FF8800"));
    let mixed = text_format_for_deck_shape(&deck, 0, 7, 0, 2).unwrap();
    assert_eq!(
        (mixed.font, mixed.east_asian_font, mixed.color),
        (None, None, None)
    );
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let runs = &layout
        .shapes
        .iter()
        .find(|s| s.source_id == Some(7))
        .unwrap()
        .lines[0]
        .runs;
    assert_eq!(left.color.as_deref(), Some(runs[0].color.as_str()));
    assert_eq!(runs[1].color, "#FF8800");
}
