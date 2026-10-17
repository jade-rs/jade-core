use jade_pptx_layout::{layout_deck_slide, paragraph_format_for_deck_shape, HeuristicMeasure};
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
fn authoring_paragraph_values_resolve_inheritance_without_autofit_scaling() {
    let deck = deck(
        r#"<a:p><a:pPr algn="ctr" lvl="2" marL="457200" indent="-228600"><a:lnSpc><a:spcPct val="150000"/></a:lnSpc><a:spcBef><a:spcPts val="600"/></a:spcBef><a:spcAft><a:spcPts val="1200"/></a:spcAft><a:buChar char="•"/></a:pPr><a:r><a:t>one</a:t></a:r></a:p><a:p><a:pPr algn="r"/><a:r><a:t>two</a:t></a:r></a:p>"#,
    );
    let p = paragraph_format_for_deck_shape(&deck, 0, 7, 1, 1).unwrap();
    assert_eq!(p.alignment.as_deref(), Some("center"));
    assert_eq!(
        (p.level, p.margin_left_pt, p.indent_pt),
        (Some(2), Some(36.0), Some(-18.0))
    );
    assert_eq!(
        p.line_spacing,
        Some(jade_pptx_layout::ParagraphSpacingState::Percent(150.0))
    );
    let mixed = paragraph_format_for_deck_shape(&deck, 0, 7, 0, 7).unwrap();
    assert_eq!(
        (
            mixed.alignment,
            mixed.level,
            mixed.bullet,
            mixed.line_spacing
        ),
        (None, None, None, None)
    );
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let lines = &layout
        .shapes
        .iter()
        .find(|s| s.source_id == Some(7))
        .unwrap()
        .lines;
    assert!(lines[0].runs[1].x > lines[0].runs[0].x);
    assert!(lines[1].text_left > lines[0].text_left);
}

#[test]
fn bullet_font_follow_text_blocks_inherited_symbol_font_and_theme_fonts_resolve() {
    let mut deck = deck(
        r#"<a:p><a:pPr><a:buFontTx/><a:buChar char="•"/></a:pPr><a:r><a:rPr><a:latin typeface="Arial"/></a:rPr><a:t>one</a:t></a:r></a:p><a:p><a:pPr><a:buFont typeface="+mn-lt"/><a:buChar char="•"/></a:pPr><a:r><a:t>two</a:t></a:r></a:p>"#,
    );
    // Legacy manually supplied decks still resolve through deck-level defaults.
    deck.slides[0].theme = None;
    deck.theme_fonts.minor_latin = Some("Aptos".into());
    let jade_pptx_reader::ShapeItem::Shape(shape) = &mut deck.slides[0].content.shapes[0] else {
        panic!("shape");
    };
    let text = shape.text.as_mut().unwrap();
    text.list_style.as_mut().unwrap().levels[0]
        .as_mut()
        .unwrap()
        .bullet_font = Some("Wingdings".into());
    assert!(text.paragraphs[0].bullet_font_follow_text);
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let lines = &layout
        .shapes
        .iter()
        .find(|s| s.source_id == Some(7))
        .unwrap()
        .lines;
    assert_eq!(lines[0].runs[0].font.as_deref(), Some("Arial"));
    assert_eq!(lines[1].runs[0].font.as_deref(), Some("Aptos"));
    assert_eq!(
        paragraph_format_for_deck_shape(&deck, 0, 7, 4, 7)
            .unwrap()
            .bullet_font
            .as_deref(),
        Some("Aptos")
    );
    let jade_pptx_reader::ShapeItem::Shape(shape) = &mut deck.slides[0].content.shapes[0] else {
        panic!("shape");
    };
    let text = shape.text.as_mut().unwrap();
    text.paragraphs[0].bullet_font_follow_text = false;
    let own = text.list_style.as_mut().unwrap().levels[0]
        .as_mut()
        .unwrap();
    own.bullet_font = None;
    own.bullet_font_follow_text = true;
    deck.default_text_style.levels = vec![Some(jade_pptx_reader::LevelStyle {
        bullet_font: Some("Wingdings".into()),
        ..Default::default()
    })];
    let layout = layout_deck_slide(&deck, 0, &HeuristicMeasure).unwrap();
    let marker = &layout
        .shapes
        .iter()
        .find(|s| s.source_id == Some(7))
        .unwrap()
        .lines[0]
        .runs[0];
    assert_eq!(marker.font.as_deref(), Some("Arial"));
}

#[test]
fn paragraph_selection_boundaries_soft_breaks_and_overflow_are_deterministic() {
    use jade_pptx_layout::paragraph_indices_for_selection as selected;
    assert_eq!(selected(&[6, 0, 3, 0], 4, 4), Some(vec![0]));
    assert_eq!(selected(&[6, 0, 3, 0], 7, 7), Some(vec![1]));
    assert_eq!(selected(&[6, 0, 3, 0], 0, 8), Some(vec![0, 1]));
    assert_eq!(selected(&[6, 0, 3, 0], 12, 12), Some(vec![3]));
    assert_eq!(selected(&[6, 0, 3, 0], 8, 7), None);
    assert_eq!(selected(&[usize::MAX], 0, usize::MAX), Some(vec![0]));
    assert_eq!(selected(&[usize::MAX, 1], 0, 0), None);
}
