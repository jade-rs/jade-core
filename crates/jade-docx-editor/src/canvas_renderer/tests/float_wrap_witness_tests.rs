use super::*;

fn wrapping_element(kind: usize) -> BodyElement {
    if kind == 2 {
        let mut table = jade_docx_reader::document::Table::default();
        table
            .style
            .insert("table-position".into(), "floating".into());
        return BodyElement::Table(Box::new(table));
    }
    let mut run = band_plain_run();
    let layout = RunImageLayout {
        kind: RunImageKind::Floating,
        wrap: WrapMode::Square,
        ..Default::default()
    };
    if kind == 0 {
        run.image = Some(jade_docx_reader::document::RunImage {
            layout,
            ..Default::default()
        });
    } else {
        run.text_box = Some(RunTextBox {
            layout,
            ..Default::default()
        });
    }
    BodyElement::from_paragraph(band_test_paragraph(vec![run]))
}

#[test]
fn local_text_edits_revalidate_each_float_witness_without_scanning_the_document() {
    for kind in 0..3 {
        let mut document = doc_with_paragraph_texts(&vec!["plain"; 3000]);
        document
            .body
            .as_mut()
            .unwrap()
            .elements
            .push(wrapping_element(kind));
        let mut model = WordDocument::from_document(document);
        let (mut renderer, _) = recording_renderer();
        for step in 0..3 {
            let document = model.document.as_ref().unwrap();
            renderer.restore_document_cache(Rc::clone(document));
            assert!(renderer.horizontal_float_wrap_cache.get(document));
            assert_eq!(renderer.horizontal_float_wrap_cache.witness, Some(3000));
            let cache = renderer.local_text_edit_cache();
            renderer.drop_document_cache();
            let document = model.document.as_ref().unwrap();
            assert_eq!(Rc::strong_count(document), 1);
            assert_eq!(Rc::weak_count(document), 0);
            let address = Rc::as_ptr(document);
            model.paragraph_mut(0).unwrap().runs[0].text.push('x');
            assert_eq!(Rc::as_ptr(model.document.as_ref().unwrap()), address);
            renderer.restore_local_text_edit_cache(cache, &model, 0);
            assert!(renderer
                .horizontal_float_wrap_cache
                .get(model.document.as_ref().unwrap()));
            assert_eq!(
                renderer.horizontal_float_wrap_cache.scans, 0,
                "kind={kind} step={step}"
            );
        }
    }
}

#[test]
fn removed_or_moved_witnesses_fall_back_to_a_fresh_scan() {
    for kind in 0..3 {
        // Include changed wrapping, removed/out-of-bounds hosts, a moved host,
        // and replacement by another kind of valid wrapping element.
        for change in 0..6 {
            let mut document = doc_with_paragraph_texts(&["plain", "before", "after"]);
            document.body.as_mut().unwrap().elements[1] = wrapping_element(kind);
            let (mut renderer, _) = recording_renderer();
            let document = Rc::new(document);
            renderer.restore_document_cache(Rc::clone(&document));
            assert!(renderer.horizontal_float_wrap_cache.get(&document));
            let cache = renderer.local_text_edit_cache();
            renderer.drop_document_cache();
            let mut document = Rc::try_unwrap(document).unwrap();
            let elements = &mut document.body.as_mut().unwrap().elements;
            match change {
                0 => match &mut elements[1] {
                    BodyElement::Paragraph(paragraph) if kind == 0 => {
                        paragraph.runs[0].image.as_mut().unwrap().layout.wrap = WrapMode::None;
                    }
                    BodyElement::Paragraph(paragraph) => {
                        paragraph.runs[0].text_box.as_mut().unwrap().layout.kind =
                            RunImageKind::Inline;
                    }
                    BodyElement::Table(table) => {
                        table.style.clear();
                    }
                    BodyElement::Math(_) => unreachable!(),
                },
                1 => {
                    elements.remove(1);
                }
                2 => elements.truncate(1),
                3 => elements.swap(1, 2),
                4 => elements[1] = wrapping_element((kind + 1) % 3),
                _ => document.body = None,
            }
            let model = WordDocument::from_document(document);
            renderer.restore_local_text_edit_cache(cache, &model, 0);
            let expected = change == 3 || change == 4;
            assert_eq!(
                renderer
                    .horizontal_float_wrap_cache
                    .get(model.document.as_ref().unwrap()),
                expected,
                "kind={kind} change={change}"
            );
            assert_eq!(
                renderer.horizontal_float_wrap_cache.scans,
                usize::from(change != 4),
                "kind={kind} change={change}"
            );
        }
    }
}

#[test]
fn witness_capture_requires_the_current_rendered_snapshot() {
    let (mut renderer, _) = recording_renderer();
    let plain = Rc::new(doc_with_paragraph_texts(&["plain"]));
    renderer.restore_document_cache(plain);
    let model = WordDocument::from_document(horizontal_wrap_textbox_document(WrapMode::Square));
    assert!(renderer
        .horizontal_float_wrap_cache
        .get(model.document.as_ref().unwrap()));
    let cache = renderer.local_text_edit_cache();
    assert!(!cache.no_horizontal_float_wrap);
    assert_eq!(cache.horizontal_float_wrap_witness, None);
    renderer.drop_document_cache();
    renderer.restore_local_text_edit_cache(cache, &model, 0);
    assert!(renderer
        .horizontal_float_wrap_cache
        .get(model.document.as_ref().unwrap()));
    assert_eq!(renderer.horizontal_float_wrap_cache.scans, 1);
}
