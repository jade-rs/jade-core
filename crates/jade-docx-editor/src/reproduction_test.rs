
#[test]
fn compute_selection_rects_fully_selected_intermediate_paragraph() {
    let line = sample_line();
    // Paragraph 1 is fully selected (start: 0, end: 2)
    // We are testing paragraph 1, which is between 0 and 2.
    let range = SelectionRange {
        start: TextPosition {
            paragraph: 0,
            offset: 0,
        },
        end: TextPosition {
            paragraph: 2,
            offset: 0,
        },
    };

    // Mock renderer with minimal state
    let renderer = CanvasRenderer::with_host(Box::new(crate::render_host::NoOpRenderHost), false).unwrap();
    
    // Test paragraph 1 (intermediate)
    let rects = renderer.compute_selection_rects(1, &line, &range);
    
    // Should have 1 rect covering the whole line (or margins)
    assert_eq!(rects.len(), 1, "Intermediate paragraph should be selected");
}
