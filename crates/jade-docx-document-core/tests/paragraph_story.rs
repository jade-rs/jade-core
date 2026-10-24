//! Which story each paragraph surface belongs to.
//!
//! jade lists every paragraph it can edit — body paragraphs (table cells
//! included), text-box paragraphs right after the paragraph that anchors the
//! box, footnotes after the paragraph that first references them, then the
//! remaining footnotes and the endnotes — as one sequence of *paragraph
//! surfaces*. A caller that edits by story (a collaborative model, say) needs
//! to name a surface by its story and back: `paragraph_story` and
//! `paragraph_story_surface`, and `paragraph_anchors` for where in a
//! paragraph the text boxes and note references sit.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

use futures::executor::block_on;
use jade_docx_document_core::document::BodyElement;
use jade_docx_document_core::headless_api::apply_headless_commands_to_document;
use jade_docx_document_core::selection::TextPosition;
use jade_docx_document_core::word_document::{
    ParagraphAnchor, ParagraphAnchorKind, ParagraphStory, WordDocument,
};
use jade_docx_document_core::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use proptest::prelude::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures")
}

fn load_bytes(bytes: &[u8]) -> WordDocument {
    block_on(WordDocument::load(bytes, &Options::default())).expect("jade loads the document")
}

fn load(name: &str) -> WordDocument {
    load_bytes(&std::fs::read(fixtures().join(name)).expect("fixture exists"))
}

fn body(index: usize) -> ParagraphStory {
    ParagraphStory::Body { index }
}

fn footnote(id: &str, paragraph: usize) -> ParagraphStory {
    ParagraphStory::Footnote {
        note_id: id.to_string(),
        paragraph,
    }
}

fn endnote(id: &str, paragraph: usize) -> ParagraphStory {
    ParagraphStory::Endnote {
        note_id: id.to_string(),
        paragraph,
    }
}

fn text_box(host: usize, text_box: usize, paragraph: usize) -> ParagraphStory {
    ParagraphStory::TextBox {
        host,
        text_box,
        paragraph,
    }
}

fn text(word: &WordDocument, surface: usize) -> String {
    word.paragraph(surface)
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .unwrap_or_default()
}

/// The fixture interleaves body paragraphs, table cells, a text box and
/// footnotes and endnotes referenced from all of them: every surface names
/// its story, in jade's order.
#[test]
fn every_surface_of_a_mixed_document_names_its_story() {
    let word = load("footnote-table-textbox-order.docx");
    let expected = vec![
        body(0),           // Intro
        footnote("9", 0),  // Note for Intro
        body(1),           // Cell A
        footnote("8", 0),  // Note for Cell A
        body(2),           // Cell B
        footnote("7", 0),  // Note for Cell B
        body(3),           // Cell C
        body(4),           // Cell D
        footnote("6", 0),  // Note for Cell D
        body(5),           // Host
        text_box(5, 0, 0), // Box text
        footnote("5", 0),  // Note for Host
        footnote("3", 0),  // Note after box
        body(6),           // Tail
        footnote("2", 0),  // Note for Tail
        footnote("-1", 0), // separator
        footnote("0", 0),  // continuation separator
        footnote("4", 0),  // Note for Box text (referenced from the box only)
        endnote("5", 0),   // Endnote for Cell B
        endnote("4", 0),   // Endnote for Tail
        endnote("-1", 0),
        endnote("0", 0),
    ];
    let stories: Vec<ParagraphStory> = (0..word.paragraph_count())
        .map(|s| word.paragraph_story(s).expect("not an equation"))
        .collect();
    assert_eq!(stories, expected);
    for (surface, story) in expected.iter().enumerate() {
        assert_eq!(
            word.paragraph_story_surface(story),
            Some(surface),
            "{story:?}"
        );
    }
    assert_eq!(text(&word, 10), "Box text");
    assert_eq!(word.paragraph_story(word.paragraph_count()), None);
    assert_eq!(word.paragraph_story_surface(&body(7)), None);
    assert_eq!(word.paragraph_story_surface(&footnote("9", 1)), None);
    assert_eq!(word.paragraph_story_surface(&text_box(5, 1, 0)), None);
    assert_eq!(word.paragraph_story_surface(&text_box(4, 0, 0)), None);
}

#[test]
fn anchors_are_listed_in_run_order_at_their_model_offsets() {
    let word = load("footnote-table-textbox-order.docx");
    let footnote_ref = |offset, id: &str| ParagraphAnchor {
        offset,
        rendered_len: 1,
        kind: ParagraphAnchorKind::Footnote {
            note_id: id.to_string(),
        },
    };
    // "Host" ¹ [box] " after box" ²
    assert_eq!(
        word.paragraph_anchors(9),
        vec![
            footnote_ref(4, "5"),
            ParagraphAnchor {
                offset: 4,
                rendered_len: 0,
                kind: ParagraphAnchorKind::TextBox { paragraphs: 1 },
            },
            footnote_ref(14, "3"),
        ]
    );
    // "Cell B" with a footnote and an endnote reference.
    assert_eq!(
        word.paragraph_anchors(4),
        vec![
            footnote_ref(6, "7"),
            ParagraphAnchor {
                offset: 6,
                rendered_len: 1,
                kind: ParagraphAnchorKind::Endnote {
                    note_id: "5".to_string(),
                },
            },
        ]
    );
    // Word draws no mark for a reference inside a text box.
    assert_eq!(
        word.paragraph_anchors(10),
        vec![ParagraphAnchor {
            offset: 8,
            rendered_len: 0,
            kind: ParagraphAnchorKind::Footnote {
                note_id: "4".to_string(),
            },
        }]
    );
    // A note's own number is not a reference to another story.
    assert!(word.paragraph_anchors(1).is_empty());
    assert!(word.paragraph_anchors(6).is_empty());
    assert!(word.paragraph_anchors(word.paragraph_count()).is_empty());
}

/// `textbox.docx` with a second text box ("Second box") in the same host
/// paragraph, after " and ".
fn two_text_boxes() -> Vec<u8> {
    let source = std::fs::read(fixtures().join("textbox.docx")).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(source)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let name = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "word/document.xml" {
            let xml = String::from_utf8(bytes).unwrap();
            let open = "<w:r><mc:AlternateContent>";
            let close = "</mc:AlternateContent></w:r>";
            let start = xml.find(open).expect("the text box run");
            let end = start + xml[start..].find(close).unwrap() + close.len();
            let first = &xml[start..end];
            let second = first.replace(">Hello<", ">Second box<");
            let joined =
                format!("{first}<w:r><w:t xml:space=\"preserve\"> and </w:t></w:r>{second}");
            bytes = format!("{}{}{}", &xml[..start], joined, &xml[end..]).into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&bytes).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn text_boxes_of_one_paragraph_are_numbered_in_run_order() {
    let word = load_bytes(&two_text_boxes());
    let stories: Vec<Option<ParagraphStory>> = (0..word.paragraph_count())
        .map(|s| word.paragraph_story(s))
        .collect();
    assert_eq!(
        stories,
        vec![
            Some(body(0)),
            Some(text_box(0, 0, 0)),
            Some(text_box(0, 1, 0)),
            Some(body(1)),
        ]
    );
    assert_eq!(text(&word, 1), "Hello");
    assert_eq!(text(&word, 2), "Second box");
    let boxes: Vec<(usize, ParagraphAnchorKind)> = word
        .paragraph_anchors(0)
        .into_iter()
        .map(|a| (a.offset, a.kind))
        .collect();
    assert_eq!(
        boxes,
        vec![
            (5, ParagraphAnchorKind::TextBox { paragraphs: 1 }),
            (10, ParagraphAnchorKind::TextBox { paragraphs: 1 }),
        ]
    );
}

#[test]
fn equations_have_no_story() {
    let word = load("math.docx");
    let mut equations = 0;
    for surface in 0..word.paragraph_count() {
        match word.paragraph_story(surface) {
            None => {
                assert!(
                    word.paragraph(surface).is_none(),
                    "{surface} has a paragraph"
                );
                equations += 1;
            }
            Some(story) => assert_eq!(word.paragraph_story_surface(&story), Some(surface)),
        }
    }
    assert!(equations > 0);
}

/// The invariants every document keeps: a story names exactly one surface,
/// body paragraphs are numbered in surface order, a note's or a text box's
/// paragraphs are numbered from 0 in surface order, and the text boxes listed
/// after a host paragraph are the ones its anchors name, in order.
fn assert_stories_are_consistent(word: &WordDocument, context: &str) {
    let mut next_body = 0;
    let mut next_in_story: BTreeMap<ParagraphStory, usize> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let doc = word.document.as_ref().expect("a document");
    for surface in 0..word.paragraph_count() {
        let Some(story) = word.paragraph_story(surface) else {
            continue;
        };
        assert!(seen.insert(story.clone()), "{story:?} twice {context}");
        // Anchors sit inside the text, in order; only a body paragraph's
        // note references render a mark.
        let anchors = word.paragraph_anchors(surface);
        let len = text(word, surface).len();
        for pair in anchors.windows(2) {
            assert!(pair[0].offset <= pair[1].offset, "{anchors:?} {context}");
        }
        for anchor in &anchors {
            assert!(anchor.offset <= len, "{anchor:?} past {len} {context}");
            if anchor.rendered_len > 0 {
                assert!(
                    matches!(story, ParagraphStory::Body { .. })
                        && !matches!(anchor.kind, ParagraphAnchorKind::TextBox { .. }),
                    "{anchor:?} in {story:?} {context}"
                );
            }
        }
        assert_eq!(
            word.paragraph_story_surface(&story),
            Some(surface),
            "{context}"
        );
        assert_eq!(
            word.is_body_paragraph_surface(surface),
            matches!(story, ParagraphStory::Body { .. }),
            "{context}"
        );
        // The story with its paragraph index zeroed, as a key.
        let (key, index) = match &story {
            ParagraphStory::Body { index } => {
                assert_eq!(*index, next_body, "body numbering {context}");
                next_body += 1;
                continue;
            }
            ParagraphStory::TextBox {
                host,
                text_box,
                paragraph,
            } => (text_box_key(*host, *text_box), *paragraph),
            ParagraphStory::Footnote { note_id, paragraph } => {
                assert!(doc.footnotes.contains_key(note_id), "{context}");
                (footnote(note_id, 0), *paragraph)
            }
            ParagraphStory::Endnote { note_id, paragraph } => {
                assert!(doc.endnotes.contains_key(note_id), "{context}");
                (endnote(note_id, 0), *paragraph)
            }
        };
        let next = next_in_story.entry(key).or_insert(0);
        assert_eq!(index, *next, "{story:?} numbering {context}");
        *next += 1;
    }
    // Each body paragraph's text-box anchors, in order, own the text-box
    // stories listed for it, with as many paragraphs as the anchor says.
    for index in 0..next_body {
        let surface = word.paragraph_story_surface(&body(index)).unwrap();
        let mut boxes = 0;
        for anchor in word.paragraph_anchors(surface) {
            if let ParagraphAnchorKind::TextBox { paragraphs } = anchor.kind {
                assert_eq!(
                    next_in_story
                        .get(&text_box_key(index, boxes))
                        .copied()
                        .unwrap_or(0),
                    paragraphs,
                    "text box {boxes} of body paragraph {index} {context}"
                );
                boxes += 1;
            }
        }
        assert_eq!(
            word.paragraph_story_surface(&text_box(index, boxes, 0)),
            None,
            "body paragraph {index} has only {boxes} text boxes {context}"
        );
    }
}

fn text_box_key(host: usize, text_box: usize) -> ParagraphStory {
    ParagraphStory::TextBox {
        host,
        text_box,
        paragraph: 0,
    }
}

#[test]
fn every_fixture_names_each_surface_once() {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "docx"))
        .collect();
    entries.sort();
    let mut checked = 0;
    for path in entries {
        let Ok(word) = block_on(WordDocument::load(
            &std::fs::read(&path).unwrap(),
            &Options::default(),
        )) else {
            continue;
        };
        assert_stories_are_consistent(&word, &path.display().to_string());
        // Every note with a paragraph lists it.
        let doc = word.document.as_ref().unwrap();
        for (notes, story) in [
            (
                &doc.footnotes,
                footnote as fn(&str, usize) -> ParagraphStory,
            ),
            (&doc.endnotes, endnote),
        ] {
            for (id, note) in notes {
                let has_paragraph = note
                    .body
                    .iter()
                    .any(|b| matches!(b, BodyElement::Paragraph(_)));
                assert_eq!(
                    word.paragraph_story_surface(&story(id, 0)).is_some(),
                    has_paragraph,
                    "note {id} of {}",
                    path.display()
                );
            }
        }
        checked += 1;
    }
    assert!(checked > 50, "only {checked} fixtures");
}

fn rendered(word: &WordDocument, surface: usize, offset: usize) -> usize {
    word.model_to_rendered_text_position(TextPosition::new(surface, offset))
        .offset
}

fn apply(word: &mut WordDocument, command: HeadlessCommand) -> bool {
    apply_headless_commands_to_document(
        word,
        &HeadlessEditRequest {
            commands: vec![command],
        },
    )
    .is_ok()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Splitting, typing and joining anywhere keeps every surface named once
    /// and keeps each host's text boxes listed after it.
    #[test]
    fn stories_stay_consistent_under_edits(
        fixture in prop::sample::select(vec![
            "footnote-table-textbox-order.docx",
            "footnote.docx",
            "endnote.docx",
            "textbox.docx",
            "footnote-long-continuation.docx",
        ]),
        edits in prop::collection::vec((0..3u8, any::<usize>(), any::<usize>()), 1..12),
    ) {
        let mut word = load(fixture);
        for (kind, at, offset) in edits {
            let surfaces: Vec<usize> = (0..word.paragraph_count())
                .filter(|s| word.paragraph(*s).is_some())
                .collect();
            let surface = surfaces[at % surfaces.len()];
            let len = text(&word, surface).len();
            let model = offset % (len + 1);
            if !text(&word, surface).is_char_boundary(model) {
                continue;
            }
            let offset = rendered(&word, surface, model);
            let command = match kind {
                0 => HeadlessCommand::InsertParagraphBreak { paragraph: surface, offset, count: 1 },
                1 => HeadlessCommand::InsertText { paragraph: surface, offset, text: "xy".into() },
                _ => {
                    // Join with the next paragraph of the same story.
                    let Some(story) = word.paragraph_story(surface) else { continue };
                    let next = match story {
                        ParagraphStory::Body { index } => body(index + 1),
                        ParagraphStory::Footnote { note_id, paragraph } => footnote(&note_id, paragraph + 1),
                        ParagraphStory::Endnote { note_id, paragraph } => endnote(&note_id, paragraph + 1),
                        ParagraphStory::TextBox { host, text_box: k, paragraph } => text_box(host, k, paragraph + 1),
                    };
                    let Some(next) = word.paragraph_story_surface(&next) else { continue };
                    if next != surface + 1 || !word.paragraphs_share_a_container(surface, next) {
                        continue;
                    }
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: surface,
                        start_offset: rendered(&word, surface, len),
                        end_paragraph: next,
                        end_offset: rendered(&word, next, 0),
                        text: String::new(),
                    }
                }
            };
            apply(&mut word, command);
            assert_stories_are_consistent(&word, fixture);
        }
    }
}
