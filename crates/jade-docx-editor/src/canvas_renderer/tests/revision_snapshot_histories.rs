//! Revision-presence facts must follow the document snapshot, even while hidden.

use super::*;
use jade_painter::recording::{Op, RecordingPainter};
use proptest::prelude::*;

#[derive(Clone, Copy, Debug)]
enum Content {
    Plain,
    Inserted,
    Deleted,
    MovedTo,
    MovedFrom,
}

fn snapshot(content: Content) -> Rc<Document> {
    let mut document = doc_with_paragraph_texts(&["first", "second", "third"]);
    for element in &mut document.body.as_mut().unwrap().elements {
        let BodyElement::Paragraph(paragraph) = element else {
            unreachable!()
        };
        paragraph.properties = Some(ParagraphProperties {
            numbering: Some(Numbering {
                id: Some("1".into()),
                level: Some(0),
            }),
            ..Default::default()
        });
    }
    let kind = match content {
        Content::Plain => return Rc::new(document),
        Content::Inserted => RevisionKind::Inserted,
        Content::Deleted => RevisionKind::Deleted,
        Content::MovedTo => RevisionKind::MovedTo,
        Content::MovedFrom => RevisionKind::MovedFrom,
    };
    if let BodyElement::Paragraph(paragraph) = &mut document.body.as_mut().unwrap().elements[1] {
        paragraph.runs[0].revision = build_revision_run(kind).revision;
    }
    Rc::new(document)
}

fn text_ink(painter: &RecordingPainter) -> Vec<Op> {
    painter
        .ops()
        .into_iter()
        .filter(|op| matches!(op, Op::FillText(..) | Op::FillShapedText(..)))
        .collect()
}

fn check_sequence(operations: &[(u8, bool, bool)]) {
    let contents = [
        Content::Plain,
        Content::Inserted,
        Content::Deleted,
        Content::MovedTo,
        Content::MovedFrom,
    ];
    let numbering = markup_seek_numbering();
    let (host, pages) = PerPageRecordingHost::new(2);
    let mut renderer = CanvasRenderer::with_host(Box::new(host), false).unwrap();
    let mut current = None;
    let mut document = snapshot(Content::Plain);
    for (step, &(content_index, markup, reset)) in operations.iter().enumerate() {
        let content = contents[usize::from(content_index)];
        let context = format!("step {step}: {content:?}, markup={markup}, reset={reset}");
        if current != Some(content_index) {
            // Revision decisions are structural edits in the host: their
            // explicit invalidation must precede rendering the new snapshot.
            renderer.invalidate_layout_from(1);
            renderer.set_incremental_dirty_from(1);
            document = snapshot(content);
            current = Some(content_index);
        }
        if reset {
            renderer.drop_document_cache();
            renderer.restore_document_cache(Rc::clone(&document));
        }
        let mut options = options_for_seek_tests();
        options.render_changes = markup;
        options.show_crop_marks = false;
        renderer.set_revision_display_flags(markup, false);
        // Two renders exercise both replacement and same-Rc fact reuse.
        for _ in 0..2 {
            pages[0].clear();
            renderer.mark_full_repaint();
            renderer
                .render_with_numbering_rc(
                    Rc::clone(&document),
                    &options,
                    Some(&numbering),
                    None,
                    None,
                    None,
                )
                .unwrap();
            let (reference_host, reference_pages) = PerPageRecordingHost::new(2);
            let mut reference = CanvasRenderer::with_host(Box::new(reference_host), false).unwrap();
            reference
                .render_with_numbering_rc(
                    snapshot(content),
                    &options,
                    Some(&numbering),
                    None,
                    None,
                    None,
                )
                .unwrap();
            assert!(!text_ink(&reference_pages[0]).is_empty(), "{context}");
            assert_eq!(
                text_ink(&pages[0]),
                text_ink(&reference_pages[0]),
                "{context}: numbered text and positions"
            );
            let deletes_content = matches!(content, Content::Deleted | Content::MovedFrom);
            assert_eq!(
                renderer.is_forward_window_seek_compatible_options(&options),
                !markup || !deletes_content,
                "{context}: forward-seek safety"
            );
        }
    }
}

#[test]
fn hidden_revision_replacements_and_cache_resets_recheck_seek_safety() {
    check_sequence(&[
        (0, true, false),
        (2, false, false),
        (0, false, false),
        (2, false, false),
        (2, true, false),
        (1, false, true),
        (1, true, false),
        (4, false, false),
        (4, true, false),
        (3, true, true),
        (0, true, false),
    ]);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, max_shrink_iters: 512, ..ProptestConfig::default() })]

    #[test]
    fn revision_snapshot_and_display_histories_match_fresh_render(
        operations in prop::collection::vec((0u8..5, any::<bool>(), any::<bool>()), 1..=20)
    ) {
        check_sequence(&operations);
    }
}
