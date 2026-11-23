//! Host flows that change notes away from the caret, then seek: commands
//! that rewrite a note by id, and note numbering or endnote position
//! settings applied with a bounded repaint. Later pages must paint, and
//! number their notes, like a render from the document start.
use super::*;
use jade_docx_reader::document::{Note, NoteNumbering, SectionRange};
use jade_painter::recording::{Op, RecordingPainter};

const N: usize = 300;

fn rnote(id: &str, words: usize) -> Note {
    Note {
        id: id.into(),
        body: vec![BodyElement::from_paragraph(note_test_paragraph(
            &format!("footnote {id} {}", "note ".repeat(words)),
            None,
        ))],
        ..Default::default()
    }
}

fn rparagraph(document: &mut Document, index: usize) -> &mut Paragraph {
    let BodyElement::Paragraph(paragraph) = &mut document.body.as_mut().unwrap().elements[index]
    else {
        unreachable!()
    };
    paragraph
}

fn rdoc(
    footnotes: &[(usize, &str, usize)],
    split: Option<usize>,
    restart: Option<&str>,
) -> Document {
    let owned: Vec<String> = (0..N)
        .map(|index| format!("p{index} {}", "word ".repeat(12 + (index * 7) % 11)))
        .collect();
    let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
    let mut document = doc_with_paragraph_texts(&texts);
    for &(element, id, words) in footnotes {
        rparagraph(&mut document, element)
            .runs
            .last_mut()
            .unwrap()
            .footnote_reference_ids
            .push(id.into());
        document.footnotes.insert(id.into(), rnote(id, words));
    }
    let split = split.unwrap_or(N);
    document.sections = [(0, split), (split, N)]
        .into_iter()
        .filter(|(start, end)| start < end)
        .map(|(start, end)| SectionRange {
            start,
            end,
            properties: Some(SectionProperties {
                section_type: Some("nextPage".into()),
                footnote_properties: restart.map(|restart| NoteNumbering {
                    number_restart: Some(restart.into()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        })
        .collect();
    document
}

/// Surface index of body element `element` (one paragraph per footnote,
/// after the paragraph first referencing it).
fn rsurface(document: &Document, element: usize) -> usize {
    let mut seen = HashSet::new();
    let notes: usize = document.body.as_ref().unwrap().elements[..element]
        .iter()
        .filter_map(BodyElement::as_paragraph)
        .flat_map(|paragraph| &paragraph.runs)
        .flat_map(|run| &run.footnote_reference_ids)
        .filter(|id| document.footnotes.contains_key(*id) && seen.insert(*id))
        .count();
    element + notes
}

fn roptions() -> Options {
    let mut options = options_for_seek_tests();
    options.render_footnotes = true;
    options.render_endnotes = true;
    options
}

/// What is on the page: the paint after its last whole-page clear.
fn final_paint(painter: &RecordingPainter) -> Vec<String> {
    let ops = painter.ops();
    let start = ops
        .iter()
        .rposition(|op| matches!(op, Op::ClearRect(x, y, w, h) if *x == 0.0 && *y == 0.0 && *w > 500.0 && *h > 500.0))
        .map_or(0, |index| index + 1);
    ops.into_iter()
        .skip(start)
        .filter_map(|op| match op {
            Op::FillText(text, x, y) => Some(format!("{text}@{x:.1},{y:.1}")),
            Op::FillShapedText(text, _, y) => Some(format!("{text}@{y:.1}")),
            Op::FillRect(..) | Op::StrokeRect(..) | Op::Rect(..) | Op::LineTo(..) => {
                Some(format!("{op:?}"))
            }
            _ => None,
        })
        .collect()
}

type R = CanvasRenderer<RecordingPainter>;

fn rrenderer() -> (R, Vec<RecordingPainter>) {
    let (host, pages) = PerPageRecordingHost::new(400);
    (R::with_host(Box::new(host), false).unwrap(), pages)
}

fn numbers(renderer: &R) -> Vec<(String, usize)> {
    let mut entries: Vec<_> = renderer
        .footnote_display_map
        .iter()
        .map(|(id, n)| (id.clone(), *n))
        .collect();
    entries.sort();
    entries
}

fn describe_diff(ours: &[String], full: &[String]) -> String {
    let first = ours
        .iter()
        .zip(full)
        .position(|(a, b)| a != b)
        .unwrap_or(ours.len().min(full.len()));
    format!(
        "ours {} ops, full {} ops, first diff at {first}: ours {:?} full {:?}",
        ours.len(),
        full.len(),
        &ours[first.min(ours.len())..(first + 4).min(ours.len())],
        &full[first.min(full.len())..(first + 4).min(full.len())]
    )
}

/// `deleteFootnote(id)` / `updateFootnote(id, text)` from the notes list
/// while the caret sits on a later page. `canvas_view` treats a note command
/// by id as dirty from the document start and structural (no fast replay),
/// repaints the window at the start, and the user then scrolls back.
fn by_id_case(delete: bool) -> Option<String> {
    let footnotes = [
        (2, "1", 8),
        (5, "2", 30),
        (9, "3", 4),
        (14, "4", 12),
        (N - 18, "5", 6),
    ];
    let mut document = rdoc(&footnotes, None, None);
    let options = roptions();
    let (mut renderer, pages) = rrenderer();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    renderer.clear_dirty_regions();
    let caret_element = N - 20;
    let caret = rsurface(&document, caret_element);
    let page = renderer.cached_paragraph_start_page(caret).unwrap();
    if delete {
        for run in &mut rparagraph(&mut document, 5).runs {
            run.footnote_reference_ids.clear();
        }
        document.footnotes.remove("2");
    } else {
        document.footnotes.insert("2".into(), rnote("2", 400));
    }
    // canvas_view::apply_paragraph_style_headless_many_with, lazy mode.
    renderer.set_visible_page_range(Some(1..=2));
    renderer.set_layout_page_limit(Some(2));
    renderer.set_incremental_dirty_from(0);
    for index in 0..=caret {
        renderer.mark_paragraph_dirty(index);
    }
    renderer.invalidate_layout_from(0);
    renderer.mark_full_repaint();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    renderer.clear_dirty_regions();
    // Back to the caret page.
    let visible = page..=page + 1;
    renderer.set_visible_page_range(Some(visible.clone()));
    renderer.set_layout_page_limit(Some(page + 1));
    renderer.skip_change_detection_once();
    renderer.prefer_visible_window_seek_once();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();

    let (mut fresh, fresh_pages) = rrenderer();
    fresh.set_visible_page_range(Some(visible.clone()));
    fresh.set_layout_page_limit(Some(page + 1));
    fresh
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let mut problems = Vec::new();
    for p in visible {
        let (ours, full) = (final_paint(&pages[p - 1]), final_paint(&fresh_pages[p - 1]));
        if ours != full {
            problems.push(format!("page {p}: {}", describe_diff(&ours, &full)));
        }
    }
    if numbers(&renderer) != numbers(&fresh) {
        problems.push(format!(
            "footnote numbers ours {:?} full {:?}",
            numbers(&renderer),
            numbers(&fresh)
        ));
    }
    let name = if delete {
        "deleteFootnote(2)"
    } else {
        "updateFootnote(2, 400 words)"
    };
    (!problems.is_empty()).then(|| format!("{name}: caret page {page}; {}", problems.join(" | ")))
}

#[test]
fn note_commands_by_id_then_scrolling_to_the_caret_match_a_from_start_render() {
    let failures: Vec<_> = [false, true].into_iter().filter_map(by_id_case).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

/// `setNoteNumbering`: `prepare_hf_edit_rerender` repaints a bounded window
/// from the start and drops no snapshot; the user then scrolls on.
fn numbering_case(
    follow: usize,
    from: Option<&'static str>,
    to: Option<&'static str>,
    invalidate: bool,
) -> Option<String> {
    let footnotes = [
        (2, "1", 4),
        (5, "2", 4),
        (12, "3", 4),
        (14, "4", 4),
        (250, "5", 6),
        (270, "6", 6),
    ];
    let mut document = rdoc(&footnotes, Some(10), from);
    let options = roptions();
    let (mut renderer, pages) = rrenderer();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    renderer.clear_dirty_regions();
    let deep = renderer
        .cached_paragraph_start_page(rsurface(&document, 250))
        .unwrap();
    let near = renderer
        .cached_paragraph_start_page(rsurface(&document, 14))
        .unwrap();
    for section in &mut document.sections {
        section.properties.as_mut().unwrap().footnote_properties =
            to.map(|restart| NoteNumbering {
                number_restart: Some(restart.into()),
                ..Default::default()
            });
    }
    renderer.set_visible_page_range(Some(near..=near));
    renderer.set_layout_page_limit(Some(near + 1));
    renderer.mark_full_repaint();
    if invalidate {
        renderer.invalidate_layout_from(0);
    }
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    renderer.clear_dirty_regions();
    let registered = renderer.footnote_order.len();
    let mode = match follow {
        0 => {
            renderer.set_visible_page_range(Some(deep..=deep));
            renderer.set_layout_page_limit(Some(deep + 1));
            renderer.skip_change_detection_once();
            renderer.prefer_visible_window_seek_once();
            "scroll (window seek)".to_string()
        }
        1 => {
            renderer.set_visible_page_range(Some(deep..=deep));
            renderer.set_layout_page_limit(Some(deep + 1));
            renderer.skip_change_detection_once();
            format!(
                "page bitmap rerender (seek prepared: {})",
                renderer.try_prepare_rerender_for_page(deep - 1)
            )
        }
        _ => {
            let previous = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(deep + 1));
            let seeded = renderer.prepare_layout_extension_seek(previous, deep + 1, &options);
            renderer
                .render_layout_with_numbering_rc(
                    Rc::new(document.clone()),
                    &options,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            renderer.set_visible_page_range(Some(deep..=deep));
            renderer.skip_change_detection_once();
            renderer.prefer_visible_window_seek_once();
            format!("layout extension (seeded: {seeded}) then paint")
        }
    };
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let walked = renderer.last_render_body_element_count();
    let (mut fresh, fresh_pages) = rrenderer();
    fresh.set_visible_page_range(Some(deep..=deep));
    fresh.set_layout_page_limit(Some(deep + 1));
    fresh
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let mut problems = Vec::new();
    let (ours, full) = (
        final_paint(&pages[deep - 1]),
        final_paint(&fresh_pages[deep - 1]),
    );
    if ours != full {
        problems.push(format!("page {deep}: {}", describe_diff(&ours, &full)));
    }
    if numbers(&renderer) != numbers(&fresh) {
        problems.push(format!(
            "footnote numbers ours {:?} full {:?}",
            numbers(&renderer),
            numbers(&fresh)
        ));
    }
    (!problems.is_empty()).then(|| {
        format!(
            "numRestart {from:?} -> {to:?} invalidate={invalidate}, then {mode}: bounded repaint at page {near} registered {registered} notes; deep page {deep}; last pass walked {walked}; {}",
            problems.join(" | ")
        )
    })
}

#[test]
fn note_numbering_change_then_seek_matches_a_from_start_render() {
    let mut failures = Vec::new();
    for invalidate in [false, true] {
        for (from, to) in [
            (None, Some("eachSect")),
            (Some("eachSect"), None),
            (Some("eachPage"), None),
            (None, Some("eachPage")),
        ] {
            for follow in 0..3 {
                if let Some(failure) = numbering_case(follow, from, to, invalidate) {
                    let cut: String = failure.chars().take(1400).collect();
                    eprintln!("NUMBERING-FAIL {cut}");
                    failures.push(failure);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// `setEndnotePosition`: the same bounded repaint (`prepare_hf_edit_rerender`),
/// while the endnotes move between the document end and a section end that
/// lies past the repainted window.
fn position_case(follow: usize, to_section_end: bool, invalidate: bool) -> Option<String> {
    use jade_docx_reader::document::EndnotePosition;
    let footnotes = [(2, "1", 4), (5, "2", 4)];
    let mut document = rdoc(&footnotes, Some(150), None);
    // Section two continues on the page section one ends on.
    document.sections[1]
        .properties
        .as_mut()
        .unwrap()
        .section_type = Some("continuous".into());
    for (element, id) in [
        (3usize, "e1"),
        (6, "e2"),
        (20, "e3"),
        (40, "e4"),
        (200, "e5"),
    ] {
        rparagraph(&mut document, element)
            .runs
            .last_mut()
            .unwrap()
            .endnote_reference_ids
            .push(id.into());
        document.endnotes.insert(id.into(), rnote(id, 60));
    }
    let (before, after) = if to_section_end {
        (EndnotePosition::DocumentEnd, EndnotePosition::SectionEnd)
    } else {
        (EndnotePosition::SectionEnd, EndnotePosition::DocumentEnd)
    };
    document.endnote_position = Some(before);
    let options = roptions();
    let (mut renderer, pages) = rrenderer();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    renderer.clear_dirty_regions();
    let near = 2;
    document.endnote_position = Some(after);
    renderer.set_visible_page_range(Some(near..=near));
    renderer.set_layout_page_limit(Some(near + 1));
    renderer.mark_full_repaint();
    if invalidate {
        renderer.invalidate_layout_from(0);
    }
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    renderer.clear_dirty_regions();
    // Where element 220 really is now.
    let (mut whole, _) = rrenderer();
    whole
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let deep = whole
        .cached_paragraph_start_page(rsurface(&document, 220))
        .unwrap();
    let mode = match follow {
        0 => {
            renderer.set_visible_page_range(Some(deep..=deep));
            renderer.set_layout_page_limit(Some(deep + 1));
            renderer.skip_change_detection_once();
            renderer.prefer_visible_window_seek_once();
            "scroll (window seek)".to_string()
        }
        1 => {
            renderer.set_visible_page_range(Some(deep..=deep));
            renderer.set_layout_page_limit(Some(deep + 1));
            renderer.skip_change_detection_once();
            format!(
                "page bitmap rerender (seek prepared: {})",
                renderer.try_prepare_rerender_for_page(deep - 1)
            )
        }
        _ => {
            let previous = renderer.layout_page_limit();
            renderer.set_layout_page_limit(Some(deep + 1));
            let seeded = renderer.prepare_layout_extension_seek(previous, deep + 1, &options);
            renderer
                .render_layout_with_numbering_rc(
                    Rc::new(document.clone()),
                    &options,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            renderer.set_visible_page_range(Some(deep..=deep));
            renderer.skip_change_detection_once();
            renderer.prefer_visible_window_seek_once();
            format!("layout extension (seeded: {seeded}) then paint")
        }
    };
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let walked = renderer.last_render_body_element_count();
    let (mut fresh, fresh_pages) = rrenderer();
    fresh.set_visible_page_range(Some(deep..=deep));
    fresh.set_layout_page_limit(Some(deep + 1));
    fresh
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let (ours, full) = (
        final_paint(&pages[deep - 1]),
        final_paint(&fresh_pages[deep - 1]),
    );
    (ours != full).then(|| {
        format!(
            "endnotes {before:?} -> {after:?} invalidate={invalidate}, then {mode}: deep page {deep}; last pass walked {walked}; page {deep}: {}",
            describe_diff(&ours, &full)
        )
    })
}

#[test]
fn endnote_position_change_then_seek_matches_a_from_start_render() {
    let mut failures = Vec::new();
    for invalidate in [false, true] {
        for to_section_end in [true, false] {
            for follow in 0..3 {
                if let Some(failure) = position_case(follow, to_section_end, invalidate) {
                    let cut: String = failure.chars().take(1200).collect();
                    eprintln!("POSITION-FAIL {cut}");
                    failures.push(failure);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
