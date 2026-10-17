//! Footnote / endnote layout regressions (#294) run against generated
//! fixtures in `public/fixtures`, rendered on a recording painter.

use super::recording_host::{recording_renderer, RecordingHost};
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::RecordingPainter;

fn load_fixture(name: &str) -> WordDocument {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("read {name}: {error}"));
    futures::executor::block_on(WordDocument::load(&bytes, &Options::default()))
        .unwrap_or_else(|error| panic!("load {name}: {error}"))
}

fn render_fixture(name: &str) -> (WordDocument, CanvasRenderer<RecordingPainter>) {
    let (model, renderer, _) = render_fixture_recorded(name);
    (model, renderer)
}

fn render_fixture_recorded(
    name: &str,
) -> (
    WordDocument,
    CanvasRenderer<RecordingPainter>,
    RecordingPainter,
) {
    render_fixture_with_painter(name, RecordingPainter::new())
}

fn render_fixture_with_painter(
    name: &str,
    painter: RecordingPainter,
) -> (
    WordDocument,
    CanvasRenderer<RecordingPainter>,
    RecordingPainter,
) {
    let model = load_fixture(name);
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("paged recording renderer");
    let document = model.document.as_ref().expect("document").clone();
    renderer
        .render(&document, &Options::default())
        .unwrap_or_else(|error| panic!("render {name}: {error:?}"));
    (model, renderer, painter)
}

type Renderer = CanvasRenderer<RecordingPainter>;

#[test]
fn imported_note_markers_preserve_the_authored_separator_and_inline_text() {
    for (fixture, is_footnote, marker) in
        [("footnote.docx", true, "1"), ("endnote.docx", false, "i")]
    {
        let model = load_fixture(fixture);
        let document = model.document.as_ref().expect("document");
        let notes = if is_footnote {
            &document.footnotes
        } else {
            &document.endnotes
        };
        let original = notes["1"].body[0].as_paragraph().expect("note paragraph");
        for text in ["Body", " Body", "  Body", "\tBody", ""] {
            for combined in [false, true] {
                let mut paragraph = original.clone();
                paragraph.runs[1].text = text.into();
                if combined {
                    paragraph.runs[0].text = text.into();
                    paragraph.runs.remove(1);
                }
                Renderer::prepare_note_paragraph(&mut paragraph, marker, true, is_footnote);
                let rendered: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
                assert_eq!(
                    rendered,
                    format!("{marker}{text}"),
                    "{fixture}, combined={combined}"
                );
                assert!(paragraph
                    .runs
                    .iter()
                    .all(|run| !run.has_footnote_ref && !run.has_endnote_ref));
            }
        }
    }
}

/// Page of the first line registered for surface `index`.
fn surface_page(renderer: &Renderer, index: usize) -> Option<usize> {
    let line = renderer.selection.paragraphs().get(index)?.lines.first()?;
    Some(renderer.page_for_y(line.absolute_top))
}

/// Index of the first registered surface whose text starts with `prefix`.
fn surface_starting_with(renderer: &Renderer, prefix: &str) -> usize {
    renderer
        .selection
        .paragraphs()
        .iter()
        .position(|paragraph| paragraph.text.starts_with(prefix))
        .unwrap_or_else(|| panic!("no surface starts with {prefix:?}"))
}

fn display_numbers(map: &HashMap<String, usize>) -> Vec<(String, usize)> {
    let mut numbers: Vec<_> = map.iter().map(|(id, n)| (id.clone(), *n)).collect();
    numbers.sort_by(|(left_id, left), (right_id, right)| {
        left.cmp(right).then_with(|| left_id.cmp(right_id))
    });
    numbers
}

/// #294: notes are numbered by reference position — body paragraphs, table
/// cells row by row. Text-box references are retained but ignored by Word.
/// Note ids run backwards, so id order cannot pass for position order.
#[test]
fn body_and_table_references_number_without_text_box_notes() {
    let (model, renderer) = render_fixture("footnote-table-textbox-order.docx");
    let expected: Vec<(String, usize)> = [
        ("9", 1),
        ("8", 2),
        ("7", 3),
        ("6", 4),
        ("5", 5),
        ("3", 6),
        ("2", 7),
    ]
    .into_iter()
    .map(|(id, n)| (id.to_string(), n))
    .collect();
    assert_eq!(display_numbers(&renderer.footnote_display_map), expected);
    assert_eq!(
        display_numbers(&renderer.endnote_display_map),
        vec![("5".to_string(), 1), ("4".to_string(), 2)]
    );

    // Main-story marks render; the box keeps only its authored text.
    let texts: Vec<&str> = renderer
        .selection
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect();
    for rendered in [
        "Intro1",
        "Cell A2",
        "Cell B3i",
        "Cell D4",
        "Host5 after box6",
        "Box text",
        "Tail7ii",
    ] {
        assert!(texts.contains(&rendered), "{rendered:?} not in {texts:?}");
    }

    // Every note body renders once, at the surface the model reserves for it,
    // led by the same number its reference shows.
    for (id, number) in &expected {
        let surface = model
            .footnote_paragraph_index(id)
            .unwrap_or_else(|| panic!("footnote {id} surface"));
        let text = &renderer.selection.paragraphs()[surface].text;
        assert!(
            text.starts_with(&format!("{number} ")),
            "footnote {id} at surface {surface}: {text:?}"
        );
        assert_eq!(
            texts
                .iter()
                .filter(|other| other.starts_with(&format!("{number} Note")))
                .count(),
            1,
            "footnote {id} rendered once"
        );
    }
    assert!(texts.contains(&"i Endnote for Cell B"), "{texts:?}");
    assert!(texts.contains(&"ii Endnote for Tail"), "{texts:?}");
    assert_surfaces_match_model("note order", &model, &renderer);
}

/// #294: with `w:numRestart="eachPage"` a table row continuing on a later
/// page numbers its references from that page's restart, and its notes are
/// registered there (so they are drawn at the bottom of that page) rather
/// than at the page where the table started. The next section restarts per
/// section (`eachSect`), whatever its first page.
#[test]
fn table_rows_number_notes_on_the_page_they_are_placed() {
    let (_model, renderer) = render_fixture("footnote-table-each-page.docx");
    // References in document order with the page their host lands on.
    let mut references: Vec<(String, usize)> = Vec::new();
    let page_of = |prefix: &str| {
        surface_page(&renderer, surface_starting_with(&renderer, prefix))
            .unwrap_or_else(|| panic!("{prefix:?} has no line"))
    };
    references.push(("1".into(), page_of("Before the table")));
    for row in 1..=60 {
        references.push(((row + 1).to_string(), page_of(&format!("Row {row} "))));
    }
    references.push(("70".into(), page_of("Last line of section one")));
    let first_page = references.first().unwrap().1;
    let last_page = references.last().unwrap().1;
    assert!(
        last_page > first_page,
        "the table must run over several pages ({first_page}..{last_page})"
    );

    let mut expected = HashMap::new();
    let mut previous_page = None;
    let mut ordinal = 0usize;
    for (id, page) in &references {
        if previous_page != Some(*page) {
            ordinal = 0;
            previous_page = Some(*page);
        }
        ordinal += 1;
        expected.insert(id.clone(), ordinal);
    }
    // Section two restarts per section even though it shares the page.
    expected.insert("71".into(), 1);
    expected.insert("72".into(), 2);
    let mut actual = renderer.footnote_display_map.clone();
    actual.retain(|id, _| expected.contains_key(id));
    assert_eq!(display_numbers(&actual), display_numbers(&expected));

    // The rendered marks agree with the numbering.
    for row in 1..=60 {
        let surface = surface_starting_with(&renderer, &format!("Row {row} "));
        let number = expected[&(row + 1).to_string()];
        let text = &renderer.selection.paragraphs()[surface].text;
        assert!(
            text.ends_with(&format!("space.{number}")),
            "row {row}: {text:?}"
        );
    }
}

/// A tentative row split can stop fitting once its notes reserve space.
/// Vary glyph advances to exercise those page-boundary placements.
#[test]
fn tentative_table_splits_keep_notes_on_the_reference_page() {
    for width in [4.0, 6.0, 8.0, 10.0] {
        let (model, renderer, _) = render_fixture_with_painter(
            "footnote-table-each-page.docx",
            RecordingPainter::with_width_per_char(width),
        );
        for row in 1..=60 {
            let reference = surface_starting_with(&renderer, &format!("Row {row} "));
            let note = model
                .footnote_paragraph_index(&(row + 1).to_string())
                .unwrap();
            let paragraph = &renderer.selection.paragraphs()[reference];
            let id = (row + 1).to_string();
            let mark = renderer.footnote_display_map[&id].to_string();
            assert!(
                paragraph.text.ends_with(&mark),
                "row {row}: {:?}",
                paragraph.text
            );
            let mark_start = paragraph.text.len() - mark.len();
            // A split paragraph can start on the previous page. Every fragment
            // of the actual reference mark must share its note's first page.
            let reference_pages = paragraph
                .lines
                .iter()
                .filter(|line| {
                    line.fragments.iter().any(|fragment| {
                        fragment.start < paragraph.text.len() && fragment.end > mark_start
                    })
                })
                .map(|line| renderer.page_for_y(line.absolute_top))
                .collect::<Vec<_>>();
            assert!(!reference_pages.is_empty(), "missing mark for row {row}");
            for page in reference_pages {
                assert_eq!(
                    surface_page(&renderer, note),
                    Some(page),
                    "row {row}, glyph advance {width}"
                );
            }
        }
    }
}

/// Page-local bottom of every line registered for `surface`.
fn surface_line_bottoms(renderer: &Renderer, surface: usize) -> Vec<(usize, f64)> {
    renderer.selection.paragraphs()[surface]
        .lines
        .iter()
        .map(|line| {
            (
                renderer.page_for_y(line.absolute_top),
                line.top + line.height,
            )
        })
        .collect()
}

/// #294: each page reserves its footnote area, so a note starts on the page
/// of its reference and every note line stays inside the body area.
#[test]
fn table_row_notes_stay_on_their_reference_page_inside_the_body() {
    let (model, renderer) = render_fixture("footnote-table-each-page.docx");
    let body_bottom = renderer.page_height - renderer.margin_bottom + 0.5;
    for row in 1..=60 {
        let id = (row + 1).to_string();
        let reference = surface_page(
            &renderer,
            surface_starting_with(&renderer, &format!("Row {row} ")),
        )
        .expect("row line");
        let note = model.footnote_paragraph_index(&id).expect("note surface");
        let lines = surface_line_bottoms(&renderer, note);
        assert!(!lines.is_empty(), "note {id} renders");
        assert_eq!(
            lines[0].0, reference,
            "note {id} starts on its reference page"
        );
        for (page, bottom) in lines {
            assert!(
                bottom <= body_bottom,
                "note {id} line on page {page} ends at {bottom}"
            );
        }
    }
    // Rows never run into the area: on every page the last row ends above
    // the first note line drawn there.
    let mut note_tops: HashMap<usize, f64> = HashMap::new();
    for row in 1..=60 {
        let note = model
            .footnote_paragraph_index(&(row + 1).to_string())
            .expect("note surface");
        for line in &renderer.selection.paragraphs()[note].lines {
            let page = renderer.page_for_y(line.absolute_top);
            let top = note_tops.entry(page).or_insert(f64::MAX);
            *top = top.min(line.top);
        }
    }
    for row in 1..=60 {
        let surface = surface_starting_with(&renderer, &format!("Row {row} "));
        for (page, bottom) in surface_line_bottoms(&renderer, surface) {
            if let Some(top) = note_tops.get(&page) {
                assert!(
                    bottom <= *top + 0.5,
                    "row {row} overlaps the notes on page {page}"
                );
            }
        }
    }
}

/// #294: a footnote too long for the space left continues on the next
/// page's footnote area below the continuation separator (a rule across the
/// whole column), an unlisted continuation notice is ignored,
/// and every part registers once with its lines inside the body area.
#[test]
fn a_long_footnote_continues_on_the_next_page() {
    let (model, renderer, painter) = render_fixture_recorded("footnote-long-continuation.docx");
    let document = model.document.as_ref().expect("document");
    assert!(
        !document.footnotes.contains_key("1"),
        "unlisted notice is not loaded"
    );
    assert!(
        document.footnotes.contains_key("-1"),
        "listed separator is loaded"
    );
    assert!(
        document.footnotes.contains_key("0"),
        "listed continuation separator is loaded"
    );
    let exported = model.export_docx().expect("export unlisted notice");
    let reloaded = futures::executor::block_on(WordDocument::load(&exported, &Options::default()))
        .expect("reload unlisted notice");
    assert_eq!(
        reloaded.parts["word/footnotes.xml"].content, model.parts["word/footnotes.xml"].content,
        "all source stories survive export unchanged"
    );
    assert!(!reloaded
        .document
        .as_ref()
        .expect("document")
        .footnotes
        .contains_key("1"));
    let reference_surface = surface_starting_with(&renderer, "Reference to a long note");
    let reference_page = surface_page(&renderer, reference_surface).expect("reference line");
    let first = model
        .footnote_paragraph_index("2")
        .expect("long note surface");
    let body_bottom = renderer.page_height - renderer.margin_bottom + 0.5;

    let mut pages = Vec::new();
    for offset in 0..40 {
        let surface = first + offset;
        let paragraph = &renderer.selection.paragraphs()[surface];
        let expected = format!("Long note line {}.", offset + 1);
        assert!(
            paragraph.text.ends_with(&expected) && paragraph.text.matches(&expected).count() == 1,
            "surface {surface}: {:?}",
            paragraph.text
        );
        let lines = surface_line_bottoms(&renderer, surface);
        assert_eq!(lines.len(), 1, "surface {surface} lines: {lines:?}");
        let (page, bottom) = lines[0];
        assert!(
            bottom <= body_bottom,
            "line {} at {bottom} on page {page}",
            offset + 1
        );
        pages.push(page);
    }
    assert_eq!(
        pages[0], reference_page,
        "the note starts with its reference"
    );
    assert!(
        pages.iter().any(|page| *page > reference_page),
        "the note continues on a later page: {pages:?}"
    );
    assert!(pages.windows(2).all(|pair| pair[0] <= pair[1]), "{pages:?}");

    // The short note referenced after the long one shares a later page's
    // area with the continuation, after it.
    let short = model.footnote_paragraph_index("3").expect("short note");
    let short_lines = surface_line_bottoms(&renderer, short);
    assert_eq!(short_lines.len(), 1);
    let short_reference = surface_page(
        &renderer,
        surface_starting_with(&renderer, "Page two reference"),
    )
    .unwrap();
    assert_eq!(short_lines[0].0, short_reference);

    let ops = painter.ops();
    let column = renderer.page_width - renderer.margin_left - renderer.margin_right;
    let rules: Vec<f64> = ops
        .windows(2)
        .filter_map(|pair| match pair {
            [jade_painter::recording::Op::MoveTo(x0, y0), jade_painter::recording::Op::LineTo(x1, y1)]
                if (y0 - y1).abs() < 0.01 && (x0 - renderer.margin_left).abs() < 0.01 =>
            {
                Some(x1 - x0)
            }
            _ => None,
        })
        .collect();
    assert!(
        rules.iter().any(|width| (width - column / 3.0).abs() < 0.5),
        "separator rule: {rules:?}"
    );
    assert!(
        rules.iter().any(|width| (width - column).abs() < 0.5),
        "continuation separator across the column: {rules:?}"
    );
    let drawn: String = ops
        .iter()
        .filter_map(|op| match op {
            jade_painter::recording::Op::FillText(text, _, _) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !drawn.contains("continued"),
        "unlisted continuation notice is not drawn"
    );
}

/// #294: whatever room a page has left when a reference line comes, the
/// reference and the start of its note share a page, the note stays inside
/// the body area, and body text ends above the footnote area. Sliding the
/// reference down the page walks it through every case: the note fits, the
/// note is split, and the reference line moves to the next page.
#[test]
fn a_reference_and_its_note_always_share_a_page() {
    let fixture = load_fixture("footnote-long-continuation.docx");
    let source = fixture.document.as_ref().expect("document");
    let elements = &source.body.as_ref().expect("body").elements;
    let filler = elements[0].clone();
    // "Page two reference" points at the one-line footnote 3.
    let reference = elements[39].clone();
    let trailing: Vec<BodyElement> = elements[31..35].to_vec();
    let mut outcomes = HashSet::new();
    for fillers in 40..60 {
        let mut document = (**source).clone();
        let body = document.body.as_mut().expect("body");
        body.elements = std::iter::repeat_n(filler.clone(), fillers)
            .chain(std::iter::once(reference.clone()))
            .chain(trailing.iter().cloned())
            .collect();
        let element_count = body.elements.len();
        assert_eq!(document.sections.len(), 1);
        document.sections[0].start = 0;
        document.sections[0].end = element_count;
        let model = WordDocument::from_document(document.clone());
        let mut renderer = CanvasRenderer::with_host(
            Box::new(RecordingHost {
                painter: RecordingPainter::new(),
            }),
            false,
        )
        .expect("paged recording renderer");
        renderer
            .render(&document, &Options::default())
            .expect("render");
        let body_bottom = renderer.page_height - renderer.margin_bottom + 0.5;
        let reference_surface = surface_starting_with(&renderer, "Page two reference");
        let reference_page = surface_page(&renderer, reference_surface).expect("reference line");
        let note = model.footnote_paragraph_index("3").expect("note surface");
        let lines = surface_line_bottoms(&renderer, note);
        assert_eq!(lines.len(), 1, "{fillers} fillers: {lines:?}");
        let (note_page, note_bottom) = lines[0];
        assert_eq!(note_page, reference_page, "{fillers} fillers");
        assert!(
            note_bottom <= body_bottom,
            "{fillers} fillers: note ends at {note_bottom}"
        );
        let note_top = renderer.selection.paragraphs()[note].lines[0].top;
        for (index, paragraph) in renderer.selection.paragraphs().iter().enumerate() {
            if index == note {
                continue;
            }
            for line in &paragraph.lines {
                if renderer.page_for_y(line.absolute_top) == note_page {
                    assert!(
                        line.top + line.height <= note_top + 0.5,
                        "{fillers} fillers: surface {index} runs into the notes"
                    );
                }
            }
        }
        let filler_pages: HashSet<_> = (0..fillers)
            .filter_map(|index| surface_page(&renderer, index))
            .collect();
        outcomes.insert(if filler_pages.contains(&reference_page) {
            "same page as the fillers"
        } else {
            "moved to the next page"
        });
    }
    assert_eq!(
        outcomes.len(),
        2,
        "both outcomes are exercised: {outcomes:?}"
    );
}

/// Every model surface is registered by the renderer at the same index with
/// the same text (reference marks and note numbers aside).
fn assert_surfaces_match_model(label: &str, model: &WordDocument, renderer: &Renderer) {
    let strip = |text: &str| -> String {
        text.chars()
            .filter(|ch| !ch.is_ascii_digit() && *ch != '\t')
            .collect()
    };
    let registered = renderer.selection.paragraphs();
    for index in 0..model.paragraph_count() {
        let Some(paragraph) = model.paragraph(index) else {
            continue;
        };
        // Notes without a main-story reference keep editable model surfaces
        // but do not have a painted Word story.
        if model.document.as_ref().is_some_and(|document| {
            document.footnotes.keys().any(|id| {
                model.footnote_paragraph_index(id) == Some(index)
                    && model.footnote_reference_mark(id).is_none()
            })
        }) {
            continue;
        }
        let text: String = paragraph.runs.iter().map(|run| run.text.as_str()).collect();
        if text.trim().is_empty() {
            continue;
        }
        let rendered = registered
            .get(index)
            .map(|paragraph| paragraph.text.as_str())
            .unwrap_or_default();
        assert!(
            strip(rendered).contains(&strip(&text)),
            "{label}: surface {index} is {rendered:?}, the model has {text:?}"
        );
    }
}

/// Re-ids the footnote references of a cloned table so each copy points at
/// its own notes.
fn retarget_table_notes(table: &mut jade_docx_reader::document::Table, offset: usize) {
    for row in &mut table.rows {
        for cell in &mut row.cells {
            for block in &mut cell.blocks {
                if let BodyElement::Paragraph(paragraph) = block {
                    for run in &mut paragraph.runs {
                        for id in &mut run.footnote_reference_ids {
                            *id = (id.parse::<usize>().unwrap() + offset).to_string();
                        }
                    }
                }
            }
        }
    }
}

/// #294: drawing a page's footnotes must not move the body's surface
/// counter. Once rows register their notes on later pages, a page break deep
/// in a table used to bump the counter by the notes drawn on earlier pages,
/// and every following surface registered at the wrong index.
#[test]
fn page_footnotes_leave_body_surface_indices_alone() {
    let fixture = load_fixture("footnote-table-each-page.docx");
    let mut document = (**fixture.document.as_ref().expect("document")).clone();
    let body = document.body.as_mut().expect("body");
    let BodyElement::Table(table) = body.elements[1].clone() else {
        panic!("table");
    };
    // Three more copies of the 60-row table, each with its own notes, so the
    // notes are drawn over several page breaks inside tables.
    for copy in 1..=3 {
        let mut clone = (*table).clone();
        retarget_table_notes(&mut clone, copy * 1000);
        body.elements
            .insert(1 + copy, BodyElement::Table(Box::new(clone)));
        for row in 1..=60 {
            let id = row + 1;
            let note = document.footnotes[&id.to_string()].clone();
            document
                .footnotes
                .insert((id + copy * 1000).to_string(), note);
        }
    }
    // Sections address body elements: shift the ranges past the copies.
    for section in &mut document.sections {
        if section.start > 1 {
            section.start += 3;
        }
        if section.end > 1 {
            section.end += 3;
        }
    }
    let model = WordDocument::from_document(document.clone());
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: RecordingPainter::new(),
        }),
        false,
    )
    .expect("paged recording renderer");
    renderer
        .render(&document, &Options::default())
        .expect("render");
    assert!(renderer.current_page_number > 3, "several pages");
    assert_surfaces_match_model("tables", &model, &renderer);

    // The note-order fixture surfaces too, with body text following the
    // text box over a page break: its note is reserved after the body.
    let fixture = load_fixture("footnote-table-textbox-order.docx");
    let mut document = (**fixture.document.as_ref().expect("document")).clone();
    let body = document.body.as_mut().expect("body");
    let filler = body.elements[0].clone();
    let at = body.elements.len() - 1;
    for _ in 0..90 {
        body.elements.insert(at, filler.clone());
    }
    let count = body.elements.len();
    document.sections[0].end = count;
    let model = WordDocument::from_document(document.clone());
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: RecordingPainter::new(),
        }),
        false,
    )
    .expect("paged recording renderer");
    renderer
        .render(&document, &Options::default())
        .expect("render");
    assert!(renderer.current_page_number > 1, "several pages");
    assert_surfaces_match_model("text box", &model, &renderer);
}

/// The continuation notice is page furniture laid out under a placeholder
/// surface index; no per-index cache may grow to that index (a render-cache
/// entry there would allocate `usize::MAX / 2` slots and abort the render).
#[test]
fn continuation_notice_leaves_per_surface_caches_alone() {
    // The editor renders without formatting marks, which makes plain
    // paragraphs eligible for the paragraph render cache.
    let mut model = load_fixture("footnote-long-continuation.docx");
    // Activate the source notice through the package settings. This must
    // still exercise the furniture cache guard after filtering unlisted notes.
    let settings = &mut model
        .parts
        .get_mut("word/settings.xml")
        .expect("settings")
        .content;
    assert!(settings.contains("</w:footnotePr>"));
    *settings = settings.replace(
        "</w:footnotePr>",
        r#"<w:footnote w:id="1"/></w:footnotePr>"#,
    );
    let bytes = model.export_docx().expect("export selected notice");
    let model = futures::executor::block_on(WordDocument::load(&bytes, &Options::default()))
        .expect("reload selected notice");
    assert_eq!(
        model.document.as_ref().expect("document").footnotes["1"]
            .note_type
            .as_deref(),
        Some("continuationNotice")
    );
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("paged recording renderer");
    let options = Options {
        show_formatting_marks: false,
        ..Options::default()
    };
    renderer
        .render(model.document.as_ref().expect("document"), &options)
        .expect("render");
    let drawn: String = painter
        .ops()
        .iter()
        .filter_map(|op| match op {
            jade_painter::recording::Op::FillText(text, _, _) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(drawn.contains("continued"), "the notice was laid out");
    let surfaces = model.paragraph_count();
    let render_cache = renderer.paragraph_render_cache.borrow();
    assert!(
        render_cache.entries.len() <= surfaces,
        "render cache spans {} slots for {surfaces} surfaces",
        render_cache.entries.len()
    );
    assert!(renderer.selection.paragraphs().len() <= surfaces);
}

/// #294: with a footer taller than the bottom margin the body (and with it
/// the footnote area) ends above the footer. Table rows must stop at the same
/// footer-aware edge the area is reserved against, or the rows after the
/// referencing ones run into the notes.
#[test]
fn table_rows_stay_above_the_footnote_area_under_a_tall_footer() {
    let fixture = load_fixture("footnote-table-each-page.docx");
    let mut document = (**fixture.document.as_ref().expect("document")).clone();
    // Only the first three rows reference notes; the rest fill the page.
    if let BodyElement::Table(table) = &mut document.body.as_mut().expect("body").elements[1] {
        for row in table.rows.iter_mut().skip(3) {
            for cell in &mut row.cells {
                for block in &mut cell.blocks {
                    if let BodyElement::Paragraph(paragraph) = block {
                        for run in &mut paragraph.runs {
                            run.footnote_reference_ids.clear();
                        }
                    }
                }
            }
        }
    } else {
        panic!("table");
    }
    let model = WordDocument::from_document(document.clone());
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: RecordingPainter::new(),
        }),
        false,
    )
    .expect("paged recording renderer");
    let footer: Vec<HeaderLine> = (0..8)
        .map(|index| HeaderLine {
            left: Some(format!("footer {index}")),
            ..HeaderLine::default()
        })
        .collect();
    renderer
        .render_with_numbering(
            &document,
            &Options::default(),
            None,
            None,
            Some(&footer),
            None,
        )
        .expect("render");
    assert!(
        renderer.unreserved_body_bottom() < renderer.page_height - renderer.margin_bottom - 1.0,
        "the footer must reach above the bottom margin"
    );
    let footer_top = renderer.unreserved_body_bottom() + 0.5;
    let notes: Vec<usize> = ["1", "2", "3", "4"]
        .iter()
        .map(|id| model.footnote_paragraph_index(id).expect("note surface"))
        .collect();
    let mut note_tops: HashMap<usize, f64> = HashMap::new();
    for note in &notes {
        for line in &renderer.selection.paragraphs()[*note].lines {
            assert!(
                line.top + line.height <= footer_top,
                "a note runs into the footer"
            );
            let top = note_tops
                .entry(renderer.page_for_y(line.absolute_top))
                .or_insert(f64::MAX);
            *top = top.min(line.top);
        }
    }
    let mut checked = 0;
    for row in 1..=60 {
        let surface = surface_starting_with(&renderer, &format!("Row {row} "));
        for (page, bottom) in surface_line_bottoms(&renderer, surface) {
            if let Some(top) = note_tops.get(&page) {
                checked += 1;
                assert!(
                    bottom <= *top + 0.5,
                    "row {row} ends at {bottom}, notes start at {top} on page {page}"
                );
            }
        }
    }
    assert!(checked > 3, "rows share the notes' page");
}

fn plain_run(text: &str) -> Run {
    Run {
        text: text.to_string(),
        properties: None,
        image: None,
        text_box: None,
        math: None,
        field: None,
        hyperlink: None,
        node_id: None,
        text_node_id: None,
        revision_node_id: None,
        comment_reference_ids: Vec::new(),
        footnote_reference_ids: Vec::new(),
        endnote_reference_ids: Vec::new(),
        note_separator: None,
        has_footnote_ref: false,
        has_endnote_ref: false,
        revision: None,
    }
}

/// A table-cell text box cannot affect the main-story note sequence. Its
/// authored reference and editable surface remain in the model.
#[test]
fn cell_text_box_references_do_not_number_main_story_notes() {
    use jade_docx_reader::document::{Note, Table, TableCell, TableRow};
    let reference = |text: &str, id: &str| {
        let mut run = plain_run(text);
        run.footnote_reference_ids.push(id.to_string());
        run
    };
    let paragraph = |runs: Vec<Run>| Paragraph {
        runs,
        properties: None,
        node_id: None,
        comment_ranges: Vec::new(),
        comment_markers: Vec::new(),
    };
    let mut anchor = plain_run("");
    let mut boxed_reference = reference("in box", "box");
    boxed_reference.footnote_reference_ids.push("a".into());
    boxed_reference
        .endnote_reference_ids
        .push("boxed-endnote".into());
    anchor.text_box = Some(RunTextBox {
        blocks: vec![BodyElement::from_paragraph(paragraph(vec![
            boxed_reference,
        ]))],
        ..Default::default()
    });
    let cell = |blocks: Vec<BodyElement>| TableCell {
        blocks,
        ..Default::default()
    };
    let table = Table {
        rows: vec![TableRow {
            cells: vec![
                cell(vec![BodyElement::from_paragraph(paragraph(vec![
                    reference("Cell A", "a"),
                    anchor,
                ]))]),
                cell(vec![BodyElement::from_paragraph(paragraph(vec![
                    reference("Cell B", "b"),
                ]))]),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut document = Document {
        body: Some(jade_docx_reader::document::Body {
            elements: vec![BodyElement::Table(Box::new(table))],
        }),
        ..Default::default()
    };
    for id in ["a", "box", "b"] {
        document.footnotes.insert(
            id.to_string(),
            Note {
                id: id.to_string(),
                body: vec![BodyElement::from_paragraph(paragraph(vec![plain_run(
                    &format!("note {id}"),
                )]))],
                ..Default::default()
            },
        );
    }
    document.endnotes.insert(
        "boxed-endnote".into(),
        Note {
            id: "boxed-endnote".into(),
            body: vec![BodyElement::from_paragraph(paragraph(vec![plain_run(
                "boxed endnote body",
            )]))],
            ..Default::default()
        },
    );
    let model = WordDocument::from_document(document.clone());
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: RecordingPainter::new(),
        }),
        false,
    )
    .expect("paged recording renderer");
    renderer
        .render(&document, &Options::default())
        .expect("render");
    for (id, number) in [("a", 1), ("b", 2)] {
        assert_eq!(renderer.footnote_display_map.get(id), Some(&number), "{id}");
        assert_eq!(
            model.footnote_reference_mark(id),
            Some(number.to_string()),
            "model mark for {id}"
        );
    }
    assert!(!renderer.footnote_display_map.contains_key("box"));
    assert_eq!(model.footnote_reference_mark("box"), None);
    assert!(model.footnote_paragraph_index("box").is_some());
    assert!(renderer.endnote_display_map.is_empty());
    let boxed_surface = (0..model.paragraph_count())
        .find(|&i| {
            model.paragraph(i).is_some_and(|p| {
                p.runs.iter().map(|r| r.text.as_str()).collect::<String>() == "in box"
            })
        })
        .unwrap();
    assert!(
        model.reference_marker_spans(boxed_surface).is_empty(),
        "a repeated body ID must not paint a marker in the box"
    );
}

/// #294: the area a continued footnote takes on the next page is measured
/// against that page. Here the reference ends a section with a small top
/// margin and the next section's pages have a much shorter body; the
/// continuation, taller than that body, must leave it a line for the text
/// (it used to be capped by the previous section's taller body, which left
/// the following text no room at all).
#[test]
fn carried_footnote_area_is_measured_on_the_page_it_continues_on() {
    use jade_docx_reader::document::{PageSettings, SectionRange};
    let fixture = load_fixture("footnote-long-continuation.docx");
    let mut document = (**fixture.document.as_ref().expect("document")).clone();
    let note = document.footnotes.get_mut("2").expect("long note");
    let lines = note.body.clone();
    for _ in 0..3 {
        note.body.extend(lines.iter().skip(1).cloned());
    }
    let base = document.default_section.clone().unwrap_or_default();
    let settings = base.page_settings.clone().unwrap_or_default();
    let element_count = document.body.as_ref().expect("body").elements.len();
    document.sections = vec![
        SectionRange {
            start: 0,
            end: 31,
            properties: Some(base.clone()),
        },
        SectionRange {
            start: 31,
            end: element_count,
            properties: Some(SectionProperties {
                section_type: Some("nextPage".into()),
                page_settings: Some(PageSettings {
                    margin_top: Some(300.0),
                    ..settings
                }),
                ..base
            }),
        },
    ];
    let model = WordDocument::from_document(document.clone());
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: RecordingPainter::new(),
        }),
        false,
    )
    .expect("paged recording renderer");
    renderer
        .render(&document, &Options::default())
        .expect("render");
    let first = model.footnote_paragraph_index("2").expect("note surface");
    let count = document.footnotes["2"].body.len();
    let mut pages = HashSet::new();
    for surface in first..first + count {
        for line in &renderer.selection.paragraphs()[surface].lines {
            pages.insert(renderer.page_for_y(line.absolute_top));
        }
    }
    assert!(
        pages.len() > 2,
        "the note runs over several pages: {pages:?}"
    );
    // The continuation leaves the body a line on each page it fills, so the
    // text after the reference starts on the very next page.
    let reference = surface_starting_with(&renderer, "Reference to a long note");
    let following = surface_starting_with(&renderer, "Following paragraph 1.");
    assert_eq!(
        surface_page(&renderer, following),
        surface_page(&renderer, reference).map(|page| page + 1),
        "body text resumes on the page after the reference"
    );
}

/// A body paragraph cloned from the fixture's "Page two reference"
/// paragraph, with its text replaced and its reference pointed at `id`.
fn reference_paragraph(source: &BodyElement, text: &str, id: &str) -> BodyElement {
    let mut element = source.clone();
    if let BodyElement::Paragraph(paragraph) = &mut element {
        paragraph.runs[0].text = text.to_string();
        for run in &mut paragraph.runs {
            if !run.footnote_reference_ids.is_empty() {
                run.footnote_reference_ids = vec![id.to_string()];
            }
        }
    }
    element
}

/// #294: under `w:numRestart="eachPage"` a reference takes the number of the
/// page its line lands on. A paragraph that starts on one page and carries
/// its reference over to the next (by wrapping, or because the reference
/// line moved with its note) used to keep the first page's number.
#[test]
fn each_page_numbers_follow_the_reference_line_across_pages() {
    use jade_docx_reader::document::NoteNumbering;
    let fixture = load_fixture("footnote-long-continuation.docx");
    let source = fixture.document.as_ref().expect("document");
    let elements = &source.body.as_ref().expect("body").elements;
    let filler = elements[0].clone();
    let host = elements[39].clone();
    let short_note = source.footnotes["3"].clone();
    let long_text = (0..60)
        .map(|index| format!("word{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut outcomes = HashSet::new();
    for fillers in 30..58 {
        let mut document = (**source).clone();
        let each_page = Some(NoteNumbering {
            number_restart: Some("eachpage".into()),
            ..Default::default()
        });
        document.footnote_properties = each_page.clone();
        for id in ["10", "11"] {
            let mut note = short_note.clone();
            note.id = id.to_string();
            document.footnotes.insert(id.to_string(), note);
        }
        let body = document.body.as_mut().expect("body");
        body.elements = std::iter::once(reference_paragraph(&host, "Early reference", "10"))
            .chain(std::iter::repeat_n(filler.clone(), fillers))
            .chain(std::iter::once(reference_paragraph(
                &host, &long_text, "11",
            )))
            .chain(elements[31..35].iter().cloned())
            .collect();
        let count = body.elements.len();
        document.sections[0].start = 0;
        document.sections[0].end = count;
        if let Some(properties) = document.sections[0].properties.as_mut() {
            properties.footnote_properties = each_page.clone();
        }
        if let Some(section) = document.default_section.as_mut() {
            section.footnote_properties = each_page;
        }
        let mut renderer = recording_renderer();
        renderer
            .render(&document, &Options::default())
            .expect("render");

        let early = surface_starting_with(&renderer, "Early reference");
        let early_page = surface_page(&renderer, early).expect("early line");
        let long = surface_starting_with(&renderer, "word0 ");
        let paragraph = &renderer.selection.paragraphs()[long];
        let first_page = renderer.page_for_y(paragraph.lines[0].absolute_top);
        let last_page = renderer.page_for_y(paragraph.lines.last().expect("lines").absolute_top);
        let expected = if last_page == early_page { 2 } else { 1 };
        assert_eq!(
            renderer.footnote_display_map.get("11"),
            Some(&expected),
            "{fillers} fillers: reference line on page {last_page}, early reference on {early_page}"
        );
        assert!(
            paragraph.text.ends_with(&format!("word59{expected}")),
            "{fillers} fillers: {:?}",
            &paragraph.text[paragraph.text.len().saturating_sub(12)..]
        );
        // The paragraph's text is laid out exactly once across its pages.
        let laid_out: String = renderer
            .layout_debug_lines_for_paragraphs(&[long])
            .iter()
            .map(|line| line.text.as_str())
            .collect::<String>()
            .split_whitespace()
            .collect();
        let registered: String = paragraph.text.split_whitespace().collect();
        assert_eq!(laid_out, registered, "{fillers} fillers");
        outcomes.insert((first_page != last_page, last_page != early_page));
    }
    assert!(
        outcomes.contains(&(true, true)),
        "a paragraph carrying its reference onto the next page is exercised: {outcomes:?}"
    );
}

/// #294: a footnote continued on a page of a different width resumes at the
/// text it did not place, re-wrapped for the new width — nothing dropped,
/// nothing repeated (it used to skip the number of lines placed before).
#[test]
fn a_continued_footnote_rewraps_for_a_wider_page() {
    use jade_docx_reader::document::{PageSettings, SectionRange};
    let fixture = load_fixture("footnote-long-continuation.docx");
    let mut document = (**fixture.document.as_ref().expect("document")).clone();
    let words = (0..500)
        .map(|index| format!("w{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    let note = document.footnotes.get_mut("2").expect("long note");
    note.body.truncate(1);
    if let BodyElement::Paragraph(paragraph) = &mut note.body[0] {
        if let Some(run) = paragraph
            .runs
            .iter_mut()
            .rev()
            .find(|run| !run.text.is_empty())
        {
            run.text = words.clone();
        }
    }
    let base = document.default_section.clone().unwrap_or_default();
    let settings = base.page_settings.clone().unwrap_or_default();
    let element_count = document.body.as_ref().expect("body").elements.len();
    document.sections = vec![
        SectionRange {
            start: 0,
            end: 31,
            properties: Some(base.clone()),
        },
        SectionRange {
            start: 31,
            end: element_count,
            properties: Some(SectionProperties {
                section_type: Some("nextPage".into()),
                page_settings: Some(PageSettings {
                    margin_left: Some(20.0),
                    margin_right: Some(20.0),
                    ..settings
                }),
                ..base
            }),
        },
    ];
    let model = WordDocument::from_document(document.clone());
    let mut renderer = recording_renderer();
    renderer
        .render(&document, &Options::default())
        .expect("render");
    let surface = model.footnote_paragraph_index("2").expect("note surface");
    let lines = renderer.layout_debug_lines_for_paragraphs(&[surface]);
    let pages: HashSet<_> = lines.iter().map(|line| line.page).collect();
    assert!(pages.len() > 1, "the note continues: {pages:?}");
    // Lines may break inside a word on the measuring host; compare the text
    // without whitespace, after the note number.
    let laid_out: String = lines
        .iter()
        .flat_map(|line| line.text.chars())
        .filter(|ch| !ch.is_whitespace())
        .collect();
    let expected: String = words.chars().filter(|ch| !ch.is_whitespace()).collect();
    assert_eq!(
        laid_out.trim_start_matches(|ch: char| ch.is_ascii_digit()),
        expected,
        "every character laid out once, in order"
    );
}

/// #294 performance guard: reserving each page's footnote area measures every
/// note once per render pass (plus once per page for the area actually
/// drawn), not every pending note again for each reference line or row.
#[test]
fn footnote_area_measures_each_note_once_per_render() {
    for name in [
        "footnote-table-each-page.docx",
        "footnote-long-continuation.docx",
    ] {
        let (model, renderer) = render_fixture(name);
        let notes = model
            .document
            .as_ref()
            .expect("document")
            .footnotes
            .values()
            .filter(|note| note.note_type.is_none())
            .count();
        let pages = renderer.current_page_number;
        eprintln!(
            "{name}: {} note layouts while measuring, {notes} notes, {pages} pages",
            renderer.note_measure_layouts
        );
        assert!(
            renderer.note_measure_layouts <= 2 * notes + pages,
            "{name}: {} note layouts while measuring, for {notes} notes on {pages} pages",
            renderer.note_measure_layouts
        );
    }
}

#[test]
fn original_word_text_box_reference_does_not_number_or_reserve_a_note() {
    let (model, renderer, _) = render_fixture_recorded("footnote-table-textbox-order.docx");
    assert_eq!(
        renderer.footnote_order.len(),
        7,
        "the original Word PDF contains seven footnotes"
    );
    assert!(
        !renderer.footnote_display_map.contains_key("4"),
        "Word ignores the text-box-only reference"
    );
    assert_eq!(renderer.footnote_display_map.get("2"), Some(&7));
    let texts: Vec<_> = renderer
        .selection
        .paragraphs()
        .iter()
        .map(|p| p.text.as_str())
        .collect();
    assert!(texts.contains(&"Box text"));
    assert!(texts.contains(&"Tail7ii"));
    assert!(!texts.iter().any(|t| t.contains("Note for Box text")));
    assert!(
        model.document.as_ref().unwrap().footnotes.contains_key("4"),
        "the authored note remains editable/exportable"
    );
    assert!(
        model.footnote_paragraph_index("4").is_some(),
        "model surface indices remain available"
    );
}

#[test]
fn model_word_text_box_reference_has_no_display_offset() {
    let model = load_fixture("footnote-table-textbox-order.docx");
    assert_eq!(model.footnote_reference_mark("4"), None);
    assert_eq!(model.footnote_reference_mark("2").as_deref(), Some("7"));
    let box_surface = (0..model.paragraph_count())
        .find(|&i| {
            model.paragraph(i).is_some_and(|p| {
                p.runs.iter().map(|r| r.text.as_str()).collect::<String>() == "Box text"
            })
        })
        .unwrap();
    assert!(model.reference_marker_spans(box_surface).is_empty());
    assert_eq!(
        model.rendered_to_model_text_position(TextPosition::new(box_surface, 8)),
        TextPosition::new(box_surface, 8)
    );
}
