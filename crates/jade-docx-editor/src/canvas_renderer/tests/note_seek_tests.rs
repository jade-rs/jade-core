//! Seeks in documents with footnotes and endnotes. Snapshots carry the note
//! order lengths, counters and numbering scopes at their resume point, so a
//! resumed walk must paint, and number its notes, exactly like a walk from
//! the document start. Paragraph text is unique and aperiodic, so a
//! whole-paragraph shift changes the painted page.

use super::*;
use jade_docx_reader::document::{EndnotePosition, Note, NoteNumbering, SectionRange};
use jade_painter::recording::{Op, RecordingPainter};

const PARAGRAPHS: usize = 300;

#[derive(Clone, Default)]
struct NoteLayout {
    /// `(body element, footnote id, words in the note)`.
    footnotes: Vec<(usize, &'static str, usize)>,
    /// `(body element, endnote id)`.
    endnotes: Vec<(usize, &'static str)>,
    /// `w:numRestart` of every section's footnotes.
    restart: Option<&'static str>,
    /// Splits the body into two sections (next page) at this element.
    split: Option<usize>,
    endnote_position: Option<EndnotePosition>,
}

fn note(id: &str, text: String) -> Note {
    Note {
        id: id.into(),
        body: vec![BodyElement::from_paragraph(note_test_paragraph(
            &text, None,
        ))],
        ..Default::default()
    }
}

fn note_document(layout: &NoteLayout) -> Document {
    let owned: Vec<String> = (0..PARAGRAPHS)
        .map(|index| format!("p{index} {}", "word ".repeat(12 + (index * 7) % 11)))
        .collect();
    let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
    let mut document = doc_with_paragraph_texts(&texts);
    for &(element, id, words) in &layout.footnotes {
        add_footnote_reference(&mut document, element, id);
        document.footnotes.insert(
            id.into(),
            note(id, format!("footnote {id} {}", "note ".repeat(words))),
        );
    }
    for &(element, id) in &layout.endnotes {
        paragraph_mut(&mut document, element)
            .runs
            .last_mut()
            .unwrap()
            .endnote_reference_ids
            .push(id.into());
        document
            .endnotes
            .insert(id.into(), note(id, format!("endnote {id} text")));
    }
    let numbering = layout.restart.map(|restart| NoteNumbering {
        number_restart: Some(restart.into()),
        ..Default::default()
    });
    let properties = || SectionProperties {
        section_type: Some("nextPage".into()),
        footnote_properties: numbering.clone(),
        ..Default::default()
    };
    let split = layout.split.unwrap_or(PARAGRAPHS);
    document.sections = [(0, split), (split, PARAGRAPHS)]
        .into_iter()
        .filter(|(start, end)| start < end)
        .map(|(start, end)| SectionRange {
            start,
            end,
            properties: Some(properties()),
        })
        .collect();
    document.endnote_position = layout.endnote_position;
    document
}

fn add_footnote_reference(document: &mut Document, element: usize, id: &str) {
    paragraph_mut(document, element)
        .runs
        .last_mut()
        .unwrap()
        .footnote_reference_ids
        .push(id.into());
}

fn paragraph_mut(document: &mut Document, index: usize) -> &mut Paragraph {
    let BodyElement::Paragraph(paragraph) = &mut document.body.as_mut().unwrap().elements[index]
    else {
        unreachable!()
    };
    paragraph
}

/// The paragraph surface index of body element `element`: each footnote's
/// one paragraph enumerates after the body paragraph first referencing it.
fn surface(document: &Document, element: usize) -> usize {
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

fn options() -> Options {
    let mut options = options_for_seek_tests();
    options.render_footnotes = true;
    options.render_endnotes = true;
    options
}

/// What is on the page: the drawing after its last whole-page clear (a
/// pass may clear a page and draw its chrome more than once).
fn painted(painter: &RecordingPainter) -> Vec<Op> {
    let ops = painter.ops();
    let start = ops
        .iter()
        .rposition(|op| {
            matches!(op, Op::ClearRect(x, y, w, h)
                if *x == 0.0 && *y == 0.0 && *w > 500.0 && *h > 500.0)
        })
        .map_or(0, |index| index + 1);
    ops.into_iter()
        .skip(start)
        .filter(|op| {
            matches!(
                op,
                Op::FillText(..)
                    | Op::FillShapedText(..)
                    | Op::FillRect(..)
                    | Op::StrokeRect(..)
                    | Op::Rect(..)
                    | Op::MoveTo(..)
                    | Op::LineTo(..)
                    | Op::Arc(..)
            )
        })
        .collect()
}

type Renderer = CanvasRenderer<RecordingPainter>;

fn renderer_with_pages() -> (Renderer, Vec<RecordingPainter>) {
    let (host, pages) = PerPageRecordingHost::new(400);
    (Renderer::with_host(Box::new(host), false).unwrap(), pages)
}

fn page_of(renderer: &Renderer, document: &Document, element: usize) -> usize {
    renderer
        .cached_paragraph_start_page(surface(document, element))
        .unwrap()
}

/// What a page window shows: its paint and the numbers of the notes
/// registered up to it.
#[derive(Debug, PartialEq)]
struct Window {
    page: Vec<Op>,
    footnote_order: Vec<String>,
    endnote_order: Vec<String>,
    footnote_numbers: Vec<(String, usize)>,
    endnote_numbers: Vec<(String, usize)>,
    footnote_sections: Vec<(String, usize)>,
    endnote_sections: Vec<(String, usize)>,
}

fn sorted(map: &HashMap<String, usize>) -> Vec<(String, usize)> {
    let mut entries: Vec<_> = map.iter().map(|(id, n)| (id.clone(), *n)).collect();
    entries.sort();
    entries
}

fn window(renderer: &Renderer, pages: &[RecordingPainter], page: usize) -> Window {
    Window {
        page: painted(&pages[page - 1]),
        footnote_order: renderer.footnote_order.clone(),
        endnote_order: renderer.endnote_order.clone(),
        footnote_numbers: sorted(&renderer.footnote_display_map),
        endnote_numbers: sorted(&renderer.endnote_display_map),
        footnote_sections: sorted(&renderer.footnote_section_map),
        endnote_sections: sorted(&renderer.endnote_section_map),
    }
}

/// Page `page` (1-based) of an independent from-start render.
fn reference(document: &Document, page: usize, limit: Option<usize>) -> Window {
    reference_with(document, page, limit, &options())
}

fn reference_with(
    document: &Document,
    page: usize,
    limit: Option<usize>,
    options: &Options,
) -> Window {
    let (mut renderer, pages) = renderer_with_pages();
    renderer.set_visible_page_range(Some(page..=page));
    renderer.set_layout_page_limit(limit);
    renderer
        .render_with_numbering(document, options, None, None, None, None)
        .unwrap();
    window(&renderer, &pages, page)
}

#[derive(Debug)]
struct SeekOutcome {
    walked: u64,
    hits: u64,
    resume_page: Option<usize>,
}

enum Edit {
    /// Types into the paragraph at this body element.
    Type(usize),
    /// Types into `.0`, and (re)moves a footnote reference at `.1`, before it.
    Reference(usize, usize, bool),
}

/// Lays the document out, edits it, repaints the edited page with the
/// lazy-edit seek and then scrolls one page on. Both pages are compared,
/// paint and note numbers, with independent from-start renders.
fn edit_and_compare(
    mut document: Document,
    edit: Edit,
    invalidate: bool,
    context: &str,
) -> SeekOutcome {
    let options = options();
    let (mut renderer, pages) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let last_page = page_of(&renderer, &document, PARAGRAPHS - 1);
    let (typed, dirty_element) = match edit {
        Edit::Type(element) => (element, element),
        Edit::Reference(typed, at, _) => (typed, at),
    };
    let visible = page_of(&renderer, &document, typed);
    if let Edit::Reference(_, at, insert) = edit {
        if insert {
            add_footnote_reference(&mut document, at, "99");
            document
                .footnotes
                .insert("99".into(), note("99", "inserted footnote".into()));
        } else {
            for run in &mut paragraph_mut(&mut document, at).runs {
                run.footnote_reference_ids.clear();
            }
        }
    }
    assert!(
        visible > 2,
        "{context}: the edit must sit deep in the document"
    );
    paragraph_mut(&mut document, typed)
        .runs
        .last_mut()
        .unwrap()
        .text
        .insert_str(0, "typed ");
    let dirty = surface(&document, dirty_element);
    let mut outcome = None;
    for page in (visible..=visible + 1).filter(|&page| page <= last_page) {
        for painter in &pages {
            painter.clear();
        }
        let limit = (page < last_page).then_some(page + 1);
        renderer.set_visible_page_range(Some(page..=page));
        renderer.set_layout_page_limit(limit);
        renderer.prefer_visible_window_seek_once();
        if page == visible {
            renderer.set_incremental_dirty_from(dirty);
            if invalidate {
                renderer.invalidate_layout_from(dirty);
                renderer.invalidate_paragraph_wrap_cache_from(dirty);
            }
            renderer.mark_paragraph_dirty(dirty);
        } else {
            renderer.skip_change_detection_once();
        }
        // The snapshot the render below consumes (same inputs, same choice).
        let chosen = renderer.try_prepare_visible_window_page_seek(&options);
        let before = renderer.window_seek_stats();
        let spans_before = renderer.span_seek_consumed.get();
        renderer
            .render_with_numbering(&document, &options, None, None, None, None)
            .unwrap();
        let after = renderer.window_seek_stats();
        let ours = window(&renderer, &pages, page);
        assert!(!ours.page.is_empty(), "{context}: page {page} was painted");
        let expected = reference(&document, page, limit);
        assert!(
            ours.page == expected.page,
            "{context}: page {page} differs from a from-start render"
        );
        assert_eq!(
            Window {
                page: Vec::new(),
                ..ours
            },
            Window {
                page: Vec::new(),
                ..expected
            },
            "{context}: page {page} note numbers"
        );
        if page == visible {
            outcome = Some(SeekOutcome {
                walked: renderer.last_render_body_element_count(),
                hits: (after.0 + after.1) - (before.0 + before.1)
                    + (renderer.span_seek_consumed.get() - spans_before),
                resume_page: chosen.as_ref().map(|snapshot| snapshot.page_number),
            });
        }
    }
    outcome.unwrap()
}

fn early_footnotes() -> Vec<(usize, &'static str, usize)> {
    vec![(2, "1", 8), (5, "2", 30), (9, "3", 4), (14, "4", 12)]
}

fn assert_bounded(outcome: &SeekOutcome, context: &str) {
    eprintln!("{context}: walked {} body elements", outcome.walked);
    assert!(outcome.hits > 0, "{context}: {outcome:?}");
    assert!(outcome.walked < 100, "{context}: {outcome:?}");
}

#[test]
fn footnotes_on_early_pages_keep_the_visible_window_seek() {
    let layout = NoteLayout {
        footnotes: early_footnotes(),
        ..Default::default()
    };
    let outcome = edit_and_compare(
        note_document(&layout),
        Edit::Type(PARAGRAPHS - 20),
        true,
        "continuous",
    );
    assert_bounded(&outcome, "continuous");
}

#[test]
fn a_footnote_on_the_visible_page_takes_the_next_number_after_a_seek() {
    let mut footnotes = early_footnotes();
    footnotes.push((PARAGRAPHS - 18, "5", 6));
    let layout = NoteLayout {
        footnotes,
        endnotes: vec![(3, "1"), (PARAGRAPHS - 17, "2")],
        ..Default::default()
    };
    let outcome = edit_and_compare(
        note_document(&layout),
        Edit::Type(PARAGRAPHS - 20),
        true,
        "late footnote",
    );
    assert_bounded(&outcome, "late footnote");
}

#[test]
fn per_page_restart_matches_a_from_start_render() {
    let mut footnotes = early_footnotes();
    footnotes.extend([(40, "5", 3), (41, "6", 3), (PARAGRAPHS - 18, "7", 6)]);
    let layout = NoteLayout {
        footnotes,
        restart: Some("eachPage"),
        ..Default::default()
    };
    let outcome = edit_and_compare(
        note_document(&layout),
        Edit::Type(PARAGRAPHS - 20),
        true,
        "eachPage",
    );
    assert_bounded(&outcome, "eachPage");
}

#[test]
fn per_section_restart_matches_a_from_start_render() {
    let mut footnotes = early_footnotes();
    footnotes.extend([(151, "5", 3), (160, "6", 5), (PARAGRAPHS - 18, "7", 6)]);
    let layout = NoteLayout {
        footnotes,
        restart: Some("eachSect"),
        split: Some(150),
        ..Default::default()
    };
    let outcome = edit_and_compare(
        note_document(&layout),
        Edit::Type(PARAGRAPHS - 20),
        true,
        "eachSect",
    );
    assert_bounded(&outcome, "eachSect");
}

#[test]
fn section_end_endnotes_before_the_seek_are_not_drawn_again() {
    for position in [EndnotePosition::SectionEnd, EndnotePosition::DocumentEnd] {
        let layout = NoteLayout {
            footnotes: early_footnotes(),
            endnotes: vec![(3, "1"), (20, "2"), (200, "3"), (PARAGRAPHS - 2, "4")],
            split: Some(150),
            endnote_position: Some(position),
            ..Default::default()
        };
        // The last page: the resumed walk reaches the document end and draws
        // the endnotes still owed there.
        let outcome = edit_and_compare(
            note_document(&layout),
            Edit::Type(PARAGRAPHS - 1),
            true,
            &format!("{position:?}"),
        );
        assert_bounded(&outcome, &format!("{position:?}"));
    }
}

#[test]
fn a_footnote_continuing_onto_a_page_records_no_snapshot_at_its_top() {
    let mut exercised = 0;
    for words in [250, 500, 900, 1500] {
        let layout = NoteLayout {
            footnotes: vec![(30, "1", words)],
            ..Default::default()
        };
        let document = note_document(&layout);
        let (mut renderer, pages) = renderer_with_pages();
        renderer
            .render_with_numbering(&document, &options(), None, None, None, None)
            .unwrap();
        let carried = page_of(&renderer, &document, 30) + 1;
        // Body text is "p<n> word word …": an "n" or "t" can only be note text.
        let continues = painted(&pages[carried - 1]).iter().any(|op| {
            matches!(op, Op::FillText(text, ..) | Op::FillShapedText(text, ..)
                if text.contains('n') || text.contains('t'))
        });
        let typed =
            (0..PARAGRAPHS).find(|&element| page_of(&renderer, &document, element) == carried);
        let (true, Some(typed)) = (continues, typed) else {
            continue;
        };
        exercised += 1;
        assert!(
            renderer
                .page_start_seek_cache_by_page
                .borrow()
                .get(&carried)
                .is_none_or(|snapshot| snapshot.page_number < carried),
            "words={words}: page {carried} continues a footnote, so no snapshot resumes at its top"
        );
        // An edit on the continued page resumes on an earlier page (or at
        // the document start) and still paints what a full walk paints.
        let outcome =
            edit_and_compare(document, Edit::Type(typed), true, &format!("words={words}"));
        assert!(
            outcome.resume_page.is_none_or(|page| page < carried),
            "words={words}: {outcome:?}"
        );
    }
    assert!(
        exercised > 0,
        "no layout continued a footnote onto a page with body text"
    );
}

#[test]
fn inserting_or_deleting_a_reference_before_the_resume_point_walks_from_the_start() {
    for invalidate in [true, false] {
        for insert in [true, false] {
            let layout = NoteLayout {
                footnotes: early_footnotes(),
                ..Default::default()
            };
            let context = format!("insert={insert} invalidate={invalidate}");
            let at = if insert { 7 } else { 5 };
            let outcome = edit_and_compare(
                note_document(&layout),
                Edit::Reference(PARAGRAPHS - 20, at, insert),
                invalidate,
                &context,
            );
            assert!(
                outcome.resume_page.is_none_or(|page| page <= 1),
                "{context}: {outcome:?}"
            );
            assert!(outcome.walked > 200, "{context}: {outcome:?}");
        }
    }
}

#[test]
fn page_rerenders_seek_past_early_footnotes() {
    let layout = NoteLayout {
        footnotes: early_footnotes(),
        endnotes: vec![(3, "1")],
        ..Default::default()
    };
    let document = note_document(&layout);
    let options = options();
    let (mut renderer, pages) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, &options, None, None, None, None)
        .unwrap();
    let last_page = page_of(&renderer, &document, PARAGRAPHS - 1);
    let mut seeks = 0;
    for page in (3..last_page).rev() {
        for painter in &pages {
            painter.clear();
        }
        renderer.set_visible_page_range(Some(page..=page));
        renderer.set_layout_page_limit(Some(page + 1));
        renderer.skip_change_detection_once();
        if renderer.try_prepare_rerender_for_page(page - 1) {
            seeks += 1;
        }
        renderer
            .render_with_numbering(&document, &options, None, None, None, None)
            .unwrap();
        let ours = window(&renderer, &pages, page);
        let expected = reference(&document, page, Some(page + 1));
        assert!(ours.page == expected.page, "page {page}");
        let walked = renderer.last_render_body_element_count();
        eprintln!("page {page}: walked {walked} body elements");
        assert!(walked < 100, "page {page}: walked {walked}");
    }
    assert!(seeks > 0);
}

#[test]
fn snapshots_record_the_note_state_at_their_resume_point() {
    let layout = NoteLayout {
        footnotes: early_footnotes(),
        endnotes: vec![(3, "1"), (12, "2")],
        restart: Some("eachPage"),
        ..Default::default()
    };
    let document = note_document(&layout);
    let (mut renderer, _pages) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, &options(), None, None, None, None)
        .unwrap();
    let deep = renderer
        .page_start_seek_cache_by_page
        .borrow()
        .values()
        .max_by_key(|snapshot| snapshot.page_number)
        .cloned()
        .expect("a seek snapshot");
    assert_eq!(deep.notes.footnotes, 4);
    assert_eq!(deep.notes.endnotes, 2);
    assert_eq!(deep.notes.flags, NOTE_SEEK_FOOTNOTES | NOTE_SEEK_ENDNOTES);
    assert_eq!(deep.endnote_counter, 2);
    assert!(deep.notes.footnote_scope.page >= 1);
    // A snapshot recorded with other note kinds cannot seed this pass.
    let mut other = deep.clone();
    other.notes.flags = NOTE_SEEK_ENDNOTES;
    assert!(!renderer.seek_snapshot_notes_are_current(&other, &options()));
    assert!(renderer.seek_snapshot_notes_are_current(&deep, &options()));
    // Nor one that needs more notes than the previous pass registered.
    other = deep.clone();
    other.notes.footnotes = 5;
    assert!(!renderer.seek_snapshot_notes_are_current(&other, &options()));
    // Nor one recorded under other note settings.
    other = deep.clone();
    other.notes.epoch += 1;
    assert!(!renderer.seek_snapshot_notes_are_current(&other, &options()));
    // Nor one past an edit before it.
    renderer.forced_dirty_from = Some(deep.paragraph_index - 1);
    assert!(!renderer.seek_snapshot_notes_are_current(&deep, &options()));
    renderer.forced_dirty_from = Some(deep.paragraph_index);
    assert!(renderer.seek_snapshot_notes_are_current(&deep, &options()));
    renderer.forced_dirty_from = None;
}

fn check_history(
    visible_from_end: usize,
    edits: &[(usize, String)],
    restart: Option<&'static str>,
) {
    check_history_with(visible_from_end, edits, restart, &options());
}

fn texts(ops: &[Op]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| match op {
            Op::FillText(text, x, y) => Some(format!("{text}@{x:.1},{y:.1}")),
            Op::FillShapedText(text, _, y) => Some(format!("{text}@{y:.1}")),
            Op::FillRect(..) | Op::LineTo(..) => Some(format!("{op:?}")),
            _ => None,
        })
        .collect()
}

fn check_history_with(
    visible_from_end: usize,
    edits: &[(usize, String)],
    restart: Option<&'static str>,
    options: &Options,
) {
    let mut footnotes = early_footnotes();
    footnotes.push((PARAGRAPHS - 12, "5", 6));
    let layout = NoteLayout {
        footnotes,
        endnotes: vec![(4, "1"), (PARAGRAPHS - 11, "2")],
        restart,
        split: restart
            .is_some_and(|restart| restart == "eachSect")
            .then_some(120),
        ..Default::default()
    };
    let mut document = note_document(&layout);
    let (mut renderer, pages) = renderer_with_pages();
    renderer
        .render_with_numbering(&document, options, None, None, None, None)
        .unwrap();
    let last_page = page_of(&renderer, &document, PARAGRAPHS - 1);
    let visible = last_page.saturating_sub(visible_from_end).max(3);
    let on_page: Vec<usize> = (0..PARAGRAPHS)
        .filter(|&element| page_of(&renderer, &document, element) == visible)
        .collect();
    for (step, (pick, text)) in edits.iter().enumerate() {
        let element = on_page[pick % on_page.len()];
        paragraph_mut(&mut document, element)
            .runs
            .last_mut()
            .unwrap()
            .text
            .insert_str(0, text);
        let dirty = surface(&document, element);
        for painter in &pages {
            painter.clear();
        }
        let limit = (visible < last_page).then_some(visible + 1);
        renderer.set_visible_page_range(Some(visible..=visible));
        renderer.set_layout_page_limit(limit);
        renderer.prefer_visible_window_seek_once();
        renderer.set_incremental_dirty_from(dirty);
        renderer.invalidate_layout_from(dirty);
        renderer.invalidate_paragraph_wrap_cache_from(dirty);
        renderer.mark_paragraph_dirty(dirty);
        renderer
            .render_with_numbering(&document, options, None, None, None, None)
            .unwrap();
        let ours = window(&renderer, &pages, visible);
        let expected = reference_with(&document, visible, limit, options);
        if ours.page != expected.page {
            let (a, b) = (texts(&ours.page), texts(&expected.page));
            let first = a
                .iter()
                .zip(&b)
                .position(|(a, b)| a != b)
                .unwrap_or(a.len().min(b.len()));
            panic!(
                "step {step}: page {visible} (of {last_page}) differs at {first}:\nours {:?}\nfull {:?}",
                &a[first.saturating_sub(3)..(first + 6).min(a.len())],
                &b[first.saturating_sub(3)..(first + 6).min(b.len())]
            );
        }
        assert_eq!(
            Window {
                page: Vec::new(),
                ..ours
            },
            Window {
                page: Vec::new(),
                ..expected
            },
            "step {step}"
        );
    }
}

#[test]
fn editing_the_first_paragraph_of_a_next_page_section_matches_a_from_start_render() {
    // Page 6 is the first page of the second section: the window seek clears
    // it and draws its chrome twice, and the final page must still match.
    check_history(7, &[(0, "0".to_string())], Some("eachSect"));
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(24))]
    #[test]
    fn note_window_edit_histories_match_full_render(
        visible_from_end in 0usize..8,
        edits in proptest::collection::vec((0usize..64, "[a-zA-Z0-9 ]{1,40}"), 1..6),
        restart in proptest::sample::select(vec![None, Some("eachPage"), Some("eachSect")]),
    ) {
        check_history(visible_from_end, &edits, restart);
    }
}
