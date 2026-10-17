use jade_pptx_layout::{
    layout_deck_slide, paragraph_format_for_deck_shape, text_format_for_deck_shape,
    HeuristicMeasure,
};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/themed-deck.pptx");
#[test]
fn master_and_override_palette_fonts_and_fill_styles_drive_layout_and_authoring_state_together() {
    let deck = jade_pptx_reader::read_deck(DECK).unwrap();
    for (i, font, text_color, fill, styled) in [
        (0, "monospace", "#14C814", "#F01414", "#F01414"),
        (1, "serif", "#1414F0", "#E6A010", "#E6A010"),
        (2, "sans-serif", "#14C8C8", "#A020F0", "#A020F0"),
        (3, "monospace", "#A020F0", "#14C8C8", "#F014C8"),
    ] {
        let layout = layout_deck_slide(&deck, i, &HeuristicMeasure).unwrap();
        let text = layout
            .shapes
            .iter()
            .find(|s| s.source_id == Some(2) && s.origin == jade_pptx_layout::PlacedOrigin::Slide)
            .unwrap();
        let run = text.lines[0].runs.last().unwrap();
        assert_eq!(run.font.as_deref(), Some(font));
        assert_eq!(run.color, text_color);
        assert_eq!(text.lines[0].runs[0].font.as_deref(), Some(font));
        let query = text_format_for_deck_shape(&deck, i, 2, 0, 8).unwrap();
        assert_eq!(query.font.as_deref(), Some(font));
        assert_eq!(query.color.as_deref(), Some(text_color));
        assert_eq!(
            paragraph_format_for_deck_shape(&deck, i, 2, 0, 8)
                .unwrap()
                .bullet_font
                .as_deref(),
            Some(font)
        );
        for (id, color) in [(3, fill), (4, styled)] {
            let shape = layout
                .shapes
                .iter()
                .find(|s| {
                    s.source_id == Some(id) && s.origin == jade_pptx_layout::PlacedOrigin::Slide
                })
                .unwrap();
            assert_eq!(shape.fill.as_deref(), Some(color));
        }
    }
}
#[test]
fn explicit_master_color_mapping_cancels_layout_mapping_for_geometry_and_query() {
    let mut deck = jade_pptx_reader::read_deck(DECK).unwrap();
    deck.slides[3].content.use_master_color_map = false;
    let inherited = text_format_for_deck_shape(&deck, 3, 2, 0, 8).unwrap();
    assert_eq!(inherited.color.as_deref(), Some("#14C8C8"));
    deck.slides[3].content.use_master_color_map = true;
    let master = text_format_for_deck_shape(&deck, 3, 2, 0, 8).unwrap();
    assert_eq!(master.color.as_deref(), Some("#A020F0"));
    let layout = layout_deck_slide(&deck, 3, &HeuristicMeasure).unwrap();
    assert_eq!(
        layout
            .shapes
            .iter()
            .find(|s| s.source_id == Some(2) && s.origin == jade_pptx_layout::PlacedOrigin::Slide)
            .unwrap()
            .lines[0]
            .runs
            .last()
            .unwrap()
            .color,
        "#A020F0"
    );
}
