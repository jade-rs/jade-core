//! Track Changes as Word keeps it: deleting text that is still a tracked
//! insertion nests the deletion inside the insertion (`<w:ins><w:del>`), so
//! rejecting the deletion brings the insertion back and rejecting the
//! insertion removes the text; text typed with tracking off is never part of
//! an insertion; and a revision target can name part of a run.

use futures::executor::block_on;
use jade_docx_document_core::document::RevisionKind;
use jade_docx_document_core::headless_api::apply_headless_commands_to_document;
use jade_docx_document_core::word_document::WordDocument;
use jade_docx_document_core::Options;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessEditRequest, HeadlessRevisionKindFilter, HeadlessRevisionTarget,
};
use proptest::prelude::*;

fn hello() -> WordDocument {
    let bytes =
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hello.docx"))
            .expect("read hello.docx");
    block_on(WordDocument::load(&bytes, &Options::default())).expect("load hello.docx")
}

fn run(word: &mut WordDocument, commands: Vec<HeadlessCommand>) {
    apply_headless_commands_to_document(word, &HeadlessEditRequest { commands })
        .unwrap_or_else(|e| panic!("{e}"));
}

fn track(author: &str) -> HeadlessCommand {
    HeadlessCommand::StartChangeTracking {
        author: Some(author.into()),
        initials: None,
        date: Some("2026-10-18T00:00:00Z".into()),
        date_utc: None,
    }
}

fn delete(paragraph: usize, start: usize, end: usize) -> HeadlessCommand {
    HeadlessCommand::ReplaceRange {
        start_paragraph: paragraph,
        start_offset: start,
        end_paragraph: paragraph,
        end_offset: end,
        text: String::new(),
    }
}

fn insert(paragraph: usize, offset: usize, text: &str) -> HeadlessCommand {
    HeadlessCommand::InsertText {
        paragraph,
        offset,
        text: text.into(),
    }
}

/// `text[kind author]` per stretch of text with the same tracked changes,
/// `in ins author` for a deletion nested in an insertion.
fn runs(word: &WordDocument, paragraph: usize) -> Vec<String> {
    let mut stretches: Vec<(Option<String>, String)> = Vec::new();
    for r in &word.paragraph(paragraph).unwrap().runs {
        let mark = r.revision.as_ref().map(|m| {
            let kind = match m.kind {
                RevisionKind::Inserted => "ins",
                RevisionKind::Deleted => "del",
                RevisionKind::MovedFrom => "from",
                RevisionKind::MovedTo => "to",
            };
            let author = m.author.clone().unwrap_or_default();
            match &m.insertion {
                Some(i) => format!(
                    "[{kind} {author} in ins {}]",
                    i.author.clone().unwrap_or_default()
                ),
                None => format!("[{kind} {author}]"),
            }
        });
        match stretches.last_mut() {
            Some((last, text)) if *last == mark => text.push_str(&r.text),
            _ => stretches.push((mark, r.text.clone())),
        }
    }
    stretches
        .into_iter()
        .filter(|(_, text)| !text.is_empty())
        .map(|(mark, text)| format!("{text}{}", mark.unwrap_or_default()))
        .collect()
}

fn reload(word: &WordDocument) -> WordDocument {
    block_on(WordDocument::load(
        &word.export_docx().unwrap(),
        &Options::default(),
    ))
    .unwrap()
}

/// The `w:id` of the run whose text is `text`: of its deletion, or of the
/// insertion it sits in (`inner`).
fn id_of(word: &WordDocument, paragraph: usize, text: &str, inner: bool) -> String {
    let mark = word
        .paragraph(paragraph)
        .unwrap()
        .runs
        .iter()
        .find(|r| r.text == text)
        .and_then(|r| r.revision.clone())
        .unwrap();
    if inner {
        mark.insertion.unwrap().id.unwrap()
    } else {
        mark.id.unwrap()
    }
}

fn decide(
    accept: bool,
    kind: Option<HeadlessRevisionKindFilter>,
    targets: Option<Vec<(usize, usize, usize, String)>>,
) -> HeadlessCommand {
    let revision_targets = targets.map(|targets| {
        targets
            .into_iter()
            .map(
                |(paragraph, start_offset, end_offset, id)| HeadlessRevisionTarget {
                    paragraph,
                    start_offset,
                    end_offset,
                    paragraph_mark: false,
                    id: Some(id),
                    format_change: None,
                },
            )
            .collect()
    });
    if accept {
        HeadlessCommand::AcceptRevisions {
            kind,
            start_paragraph: None,
            end_paragraph: None,
            revision_id: None,
            revision_ids: None,
            revision_targets,
        }
    } else {
        HeadlessCommand::RejectRevisions {
            kind,
            start_paragraph: None,
            end_paragraph: None,
            revision_id: None,
            revision_ids: None,
            revision_targets,
        }
    }
}

#[test]
fn deleting_an_insertion_nests_the_deletion_in_it_and_survives_the_file() {
    let mut word = hello();
    run(&mut word, vec![track("A"), insert(1, 0, "XYZ")]);
    run(&mut word, vec![track("B"), delete(1, 1, 2)]);
    assert_eq!(
        runs(&word, 1),
        ["X[ins A]", "Y[del B in ins A]", "Z[ins A]", "World"]
    );
    let word = reload(&word);
    assert_eq!(
        runs(&word, 1),
        ["X[ins A]", "Y[del B in ins A]", "Z[ins A]", "World"]
    );
}

#[test]
fn rejecting_a_nested_deletion_restores_the_insertion() {
    let mut word = hello();
    run(&mut word, vec![track("A"), insert(1, 0, "XYZ")]);
    run(&mut word, vec![track("B"), delete(1, 0, 3)]);
    let deletion = id_of(&word, 1, "XYZ", false);
    run(
        &mut word,
        vec![decide(false, None, Some(vec![(1, 0, 3, deletion)]))],
    );
    assert_eq!(runs(&word, 1), ["XYZ[ins A]", "World"]);
}

#[test]
fn accepting_the_insertion_of_deleted_text_leaves_a_plain_deletion() {
    let mut word = hello();
    run(&mut word, vec![track("A"), insert(1, 0, "XYZ")]);
    run(&mut word, vec![track("B"), delete(1, 1, 2)]);
    let insertion = id_of(&word, 1, "Y", true);
    run(
        &mut word,
        vec![decide(
            true,
            Some(HeadlessRevisionKindFilter::Inserted),
            Some(vec![(1, 0, 3, insertion)]),
        )],
    );
    assert_eq!(runs(&word, 1), ["X", "Y[del B]", "ZWorld"]);
    let word = reload(&word);
    assert_eq!(runs(&word, 1), ["X", "Y[del B]", "ZWorld"]);
}

#[test]
fn deciding_everything_treats_both_layers() {
    for (accept, expected) in [(true, "XZWorld"), (false, "World")] {
        let mut word = hello();
        run(&mut word, vec![track("A"), insert(1, 0, "XYZ")]);
        run(&mut word, vec![track("B"), delete(1, 1, 2)]);
        run(&mut word, vec![decide(accept, None, None)]);
        let text: String = word
            .paragraph(1)
            .unwrap()
            .runs
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(text, expected, "accept: {accept}");
        assert!(word
            .paragraph(1)
            .unwrap()
            .runs
            .iter()
            .all(|r| r.revision.is_none()));
    }
}

#[test]
fn typing_with_tracking_off_beside_or_inside_an_insertion_is_plain() {
    let mut word = hello();
    run(&mut word, vec![track("A"), insert(1, 0, "XY")]);
    run(
        &mut word,
        vec![HeadlessCommand::StopChangeTracking, insert(1, 2, "u")],
    );
    run(&mut word, vec![insert(1, 1, "v")]);
    assert_eq!(runs(&word, 1), ["X[ins A]", "v", "Y[ins A]", "uWorld"]);
    // Tracked typing there is still an insertion of its own.
    run(&mut word, vec![track("B"), insert(1, 1, "w")]);
    assert_eq!(
        runs(&word, 1),
        ["X[ins A]", "w[ins B]", "v", "Y[ins A]", "uWorld"]
    );
}

#[test]
fn a_revision_target_decides_only_its_part_of_a_run() {
    let mut word = hello();
    run(
        &mut word,
        vec![track("A"), insert(1, 0, "ABCD"), delete(1, 4, 9)],
    );
    let insertion = id_of(&word, 1, "ABCD", false);
    let deletion = id_of(&word, 1, "World", false);
    run(
        &mut word,
        vec![decide(true, None, Some(vec![(1, 1, 3, insertion)]))],
    );
    assert_eq!(
        runs(&word, 1),
        ["A[ins A]", "BC", "D[ins A]", "World[del A]"]
    );
    run(
        &mut word,
        vec![decide(false, None, Some(vec![(1, 5, 7, deletion)]))],
    );
    assert_eq!(
        runs(&word, 1),
        ["A[ins A]", "BC", "D[ins A]", "W[del A]", "or", "ld[del A]"]
    );
    let word = reload(&word);
    assert_eq!(
        runs(&word, 1),
        ["A[ins A]", "BC", "D[ins A]", "W[del A]", "or", "ld[del A]"]
    );
}

#[derive(Clone, Debug)]
enum Step {
    Type {
        author: Option<u8>,
        at: usize,
        text: &'static str,
    },
    Delete {
        author: Option<u8>,
        at: usize,
        len: usize,
    },
    Decide {
        accept: bool,
    },
}

fn step() -> impl Strategy<Value = Step> {
    let author = prop::option::of(0u8..2);
    prop_oneof![
        3 => (author.clone(), 0usize..64, prop::sample::select(vec!["a", "bc", "日本"]))
            .prop_map(|(author, at, text)| Step::Type { author, at, text }),
        3 => (author, 0usize..64, 1usize..4).prop_map(|(author, at, len)| Step::Delete { author, at, len }),
        1 => any::<bool>().prop_map(|accept| Step::Decide { accept }),
    ]
}

/// The character boundary of `text` at or before `at`.
fn floor(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Whatever two authors type and delete, tracked or not, the file reads
    /// back with the same revisions after every step, and accepting or
    /// rejecting everything leaves no revision behind.
    #[test]
    fn revisions_survive_the_file_and_every_decision_is_complete(
        steps in prop::collection::vec(step(), 1..16),
    ) {
        let mut word = hello();
        for step in steps {
            let text: String = word.paragraph(1).unwrap().runs.iter().map(|r| r.text.as_str()).collect();
            let commands = match step {
                Step::Type { author, at, text: typed } => {
                    let at = floor(&text, at);
                    vec![
                        author.map_or(HeadlessCommand::StopChangeTracking, |a| track(["A", "B"][a as usize])),
                        insert(1, at, typed),
                    ]
                }
                Step::Delete { author, at, len } => {
                    let start = floor(&text, at);
                    let end = floor(&text, start + len);
                    if start == end {
                        continue;
                    }
                    vec![
                        author.map_or(HeadlessCommand::StopChangeTracking, |a| track(["A", "B"][a as usize])),
                        delete(1, start, end),
                    ]
                }
                Step::Decide { accept } => vec![decide(accept, None, None)],
            };
            run(&mut word, commands);
            let reloaded = reload(&word);
            prop_assert_eq!(runs(&reloaded, 1), runs(&word, 1));
        }
        for accept in [true, false] {
            let mut decided = word.clone();
            run(&mut decided, vec![decide(accept, None, None)]);
            prop_assert!(decided.paragraph(1).unwrap().runs.iter().all(|r| r.revision.is_none()));
        }
    }
}

fn fixture(name: &str) -> WordDocument {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../public/fixtures")
            .join(name),
    )
    .expect("read the fixture");
    block_on(WordDocument::load(&bytes, &Options::default())).expect("load the fixture")
}

/// Deciding a move by its target takes its `w:moveFrom` / `w:moveTo` out of
/// the file too, as for an insertion or a deletion — also once its paragraph
/// was joined with another.
#[test]
fn a_decided_move_leaves_the_file() {
    for (accept, join) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut word = fixture("review-kinds.docx");
        if join {
            // "moved words stays" + "Target moved words".
            run(
                &mut word,
                vec![HeadlessCommand::ReplaceRange {
                    start_paragraph: 1,
                    start_offset: 17,
                    end_paragraph: 2,
                    end_offset: 0,
                    text: String::new(),
                }],
            );
        }
        let mut targets = Vec::new();
        for paragraph in 0..word.paragraph_count() {
            let mut offset = 0;
            for r in &word.paragraph(paragraph).unwrap().runs {
                let end = offset + r.text.len();
                if let Some(mark) = r
                    .revision
                    .as_ref()
                    .filter(|m| matches!(m.kind, RevisionKind::MovedFrom | RevisionKind::MovedTo))
                {
                    targets.push((paragraph, offset, end, mark.id.clone().unwrap()));
                }
                offset = end;
            }
        }
        assert_eq!(targets.len(), 2);
        run(&mut word, vec![decide(accept, None, Some(targets))]);
        let expected = (runs(&word, 1), runs(&word, 2));
        let reloaded = reload(&word);
        assert_eq!(
            (runs(&reloaded, 1), runs(&reloaded, 2)),
            expected,
            "accept: {accept}, join: {join}"
        );
        let moved = if join {
            vec![expected.0]
        } else {
            vec![expected.0, expected.1]
        };
        assert!(
            moved.iter().flatten().all(|r| !r.contains('[')),
            "{moved:?}"
        );
    }
}

/// `textbox.docx` with its first paragraph rewritten as the text box
/// followed by deleted text: `[box]<w:del>a</w:del>`.
fn text_box_then_deletion() -> WordDocument {
    use std::io::{Read, Write};
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/textbox.docx");
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if entry.name().unwrap() == "word/document.xml" {
            let xml = String::from_utf8(bytes).unwrap();
            let hello_start = xml.find("<w:r><w:rPr><w:rtl").unwrap();
            let hello_end =
                xml[hello_start..].find("</w:r>").unwrap() + hello_start + "</w:r>".len();
            let xml = format!("{}{}", &xml[..hello_start], &xml[hello_end..]);
            let box_end = xml.find("</mc:AlternateContent></w:r>").unwrap()
                + "</mc:AlternateContent></w:r>".len();
            let deleted = r#"<w:del w:id="90" w:author="A" w:date="2026-10-18T00:00:00Z"><w:r><w:delText>a</w:delText></w:r></w:del>"#;
            bytes = format!("{}{deleted}{}", &xml[..box_end], &xml[box_end..]).into_bytes();
        }
        out.start_file(
            entry.name().unwrap(),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        out.write_all(&bytes).unwrap();
    }
    let bytes = out.finish().unwrap().into_inner();
    block_on(WordDocument::load(&bytes, &Options::default())).unwrap()
}

/// What paragraph 0 holds, in order: `[text box]` and run texts.
fn order(word: &WordDocument) -> Vec<String> {
    word.paragraph(0)
        .unwrap()
        .runs
        .iter()
        .filter(|r| r.text_box.is_some() || !r.text.is_empty())
        .map(|r| {
            if r.text_box.is_some() {
                "[text box]".to_string()
            } else {
                r.text.clone()
            }
        })
        .collect()
}

/// Typing at the start of a paragraph that begins with a text box goes
/// after the text box, as everywhere else — also when deleted text follows
/// it, where jade starts a run of its own rather than write into the
/// deletion.
#[test]
fn typing_before_deleted_text_at_a_paragraph_start_goes_after_its_text_box() {
    assert_eq!(order(&text_box_then_deletion()), ["[text box]", "a"]);
    for tracked in [false, true] {
        let mut word = text_box_then_deletion();
        let typing = if tracked {
            track("B")
        } else {
            HeadlessCommand::StopChangeTracking
        };
        run(&mut word, vec![typing, insert(0, 0, "xy")]);
        assert_eq!(
            order(&word),
            ["[text box]", "xy", "a"],
            "tracked: {tracked}"
        );
    }
}

/// Deleted text that ends (or starts) with a space keeps it in the file:
/// a `w:delText` cut out of a `w:t` that needed no `xml:space` gets one.
#[test]
fn deleted_text_keeps_its_edge_spaces_in_the_file() {
    let mut word = hello();
    run(&mut word, vec![insert(1, 0, "ab cd ")]);
    let mut word = reload(&word);
    assert_eq!(runs(&word, 1), ["ab cd World"]);
    run(
        &mut word,
        vec![track("A"), delete(1, 1, 3), delete(1, 5, 7)],
    );
    let expected = ["a", "b [del A]", "cd", " W[del A]", "orld"];
    assert_eq!(runs(&word, 1), expected);
    assert_eq!(runs(&reload(&word), 1), expected);
}

/// The same in a footnote, whose text lives in its own part.
#[test]
fn deleted_text_in_a_footnote_keeps_its_edge_spaces_in_the_file() {
    use jade_docx_document_core::selection::TextPosition;
    let mut word = fixture("footnote.docx");
    assert_eq!(runs(&word, 1), [" Hello World"]);
    let at = |word: &WordDocument, offset| {
        word.model_to_rendered_text_position(TextPosition::new(1, offset))
            .offset
    };
    let (start, end) = (at(&word, 2), at(&word, 7));
    run(&mut word, vec![track("A"), delete(1, start, end)]);
    let expected = [" H", "ello [del A]", "World"];
    assert_eq!(runs(&word, 1), expected);
    assert_eq!(runs(&reload(&word), 1), expected);
}

/// An insertion whose text was all deleted (tracking off) leaves an empty
/// run behind; typing there with tracking off is still plain text.
#[test]
fn typing_where_an_insertion_was_emptied_is_plain() {
    let mut word = hello();
    run(&mut word, vec![track("A"), insert(1, 0, "é")]);
    run(
        &mut word,
        vec![HeadlessCommand::StopChangeTracking, delete(1, 0, 2)],
    );
    run(&mut word, vec![insert(1, 0, "日本")]);
    assert_eq!(runs(&word, 1), ["日本World"]);
    assert_eq!(runs(&reload(&word), 1), ["日本World"]);
}

/// The same after a note reference that starts its paragraph.
#[test]
fn typing_where_an_insertion_was_emptied_after_a_note_reference_is_plain() {
    let mut word = fixture("footnote.docx");
    // "Hello[1]" → "Hello" / "[1]".
    run(
        &mut word,
        vec![HeadlessCommand::InsertParagraphBreak {
            paragraph: 0,
            offset: 5,
            count: 1,
        }],
    );
    run(&mut word, vec![track("A"), insert(1, 0, "é")]);
    // The reference renders one character before the typed text.
    run(
        &mut word,
        vec![HeadlessCommand::StopChangeTracking, delete(1, 1, 3)],
    );
    run(&mut word, vec![insert(1, 0, "日本")]);
    assert_eq!(runs(&word, 1), ["日本"]);
    assert_eq!(runs(&reload(&word), 1), ["日本"]);
}

fn formatting_revisions(
    word: &WordDocument,
) -> Vec<jade_docx_headless_api::HeadlessRevisionSnapshot> {
    use jade_docx_headless_api::{HeadlessQueryRequest, HeadlessQueryResult};
    let request: HeadlessQueryRequest =
        serde_json::from_value(serde_json::json!({"queries": [{"type": "revisions"}]})).unwrap();
    let response =
        jade_docx_document_core::headless_api::execute_headless_queries_on_document(word, &request)
            .unwrap();
    match response.results.into_iter().next().unwrap() {
        HeadlessQueryResult::Revisions { revisions, .. } => revisions,
        _ => unreachable!(),
    }
}

#[test]
fn tracked_formatting_survives_export_and_can_be_rejected_individually() {
    let mut word = hello();
    let original = word.paragraph(0).unwrap().properties.clone();
    run(
        &mut word,
        vec![
            track("Reviewer"),
            HeadlessCommand::SetBold {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: 5,
                value: true,
            },
            HeadlessCommand::SetParagraphIndent {
                start_paragraph: 0,
                end_paragraph: 0,
                left: Some(36.0),
                right: None,
                first_line: None,
                hanging: None,
                left_chars: None,
                right_chars: None,
                first_line_chars: None,
                hanging_chars: None,
                clear_special: false,
            },
        ],
    );
    let mut word = reload(&word);
    let revisions = formatting_revisions(&word);
    assert_eq!(revisions.len(), 2, "{revisions:?}");
    assert!(revisions
        .iter()
        .all(|r| r.author.as_deref() == Some("Reviewer") && r.date.is_some() && r.id.is_some()));
    for kind in ["runFormat", "paragraphFormat"] {
        let revision = revisions.iter().find(|r| r.kind == kind).unwrap();
        run(
            &mut word,
            vec![HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: None,
                end_paragraph: None,
                revision_id: None,
                revision_ids: None,
                revision_targets: Some(vec![HeadlessRevisionTarget {
                    paragraph: revision.paragraph,
                    start_offset: revision.start_offset,
                    end_offset: revision.end_offset,
                    id: revision.id.clone(),
                    paragraph_mark: false,
                    format_change: Some(kind.into()),
                }]),
            }],
        );
    }
    assert!(!word.paragraph(0).unwrap().runs[0]
        .properties
        .as_ref()
        .and_then(|props| props.bold)
        .unwrap_or(false));
    assert_eq!(
        word.paragraph(0)
            .unwrap()
            .properties
            .as_ref()
            .and_then(|props| props.indent.as_ref())
            .and_then(|indent| indent.left),
        original
            .as_ref()
            .and_then(|props| props.indent.as_ref())
            .and_then(|indent| indent.left)
    );
    assert!(formatting_revisions(&reload(&word)).is_empty());
}

#[test]
fn accepting_formatting_keeps_latest_value_and_repeated_edits_keep_original() {
    for accept in [false, true] {
        let mut word = hello();
        run(
            &mut word,
            vec![
                track("Reviewer"),
                HeadlessCommand::SetBold {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    value: true,
                },
                HeadlessCommand::SetBold {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    value: false,
                },
                HeadlessCommand::SetBold {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 5,
                    value: true,
                },
            ],
        );
        run(&mut word, vec![decide(accept, None, None)]);
        let word = reload(&word);
        assert_eq!(
            word.paragraph(0).unwrap().runs[0]
                .properties
                .as_ref()
                .and_then(|props| props.bold)
                .unwrap_or(false),
            accept
        );
        assert!(formatting_revisions(&word).is_empty());
    }
}

#[test]
fn rejecting_text_and_formatting_together_restores_original_properties() {
    let mut word = hello();
    let original = runs(&word, 0);
    run(
        &mut word,
        vec![
            track("Reviewer"),
            HeadlessCommand::SetBold {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: 5,
                value: true,
            },
            insert(0, 0, "new"),
            delete(0, 4, 6),
        ],
    );
    let mut word = reload(&word);
    run(&mut word, vec![decide(false, None, None)]);
    let word = reload(&word);
    assert_eq!(runs(&word, 0), original);
    assert!(word.paragraph(0).unwrap().runs.iter().all(|run| !run
        .properties
        .as_ref()
        .and_then(|props| props.bold)
        .unwrap_or(false)));
    assert!(formatting_revisions(&word).is_empty());
}

#[test]
fn tracked_paragraph_formatting_excludes_mark_properties_and_preserves_them_on_reject() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/math.docx");
    let bytes = std::fs::read(path).unwrap();
    let mut word = block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
    let original_properties = word.paragraph(0).unwrap().properties.clone();
    run(
        &mut word,
        vec![
            track("Reviewer"),
            HeadlessCommand::SetParagraphAlignment {
                start_paragraph: 0,
                end_paragraph: 0,
                alignment: "left".into(),
            },
        ],
    );
    let xml = word.raw_document.as_ref().unwrap().to_string().unwrap();
    let change = xml
        .split("<w:pPrChange")
        .nth(1)
        .unwrap()
        .split("</w:pPrChange>")
        .next()
        .unwrap();
    assert!(
        !change.contains("<w:rPr"),
        "paragraph-mark properties are outside CT_PPrBase: {change}"
    );
    assert!(!change.contains("<w:sectPr"));
    let mark_properties = |word: &WordDocument| {
        let xml = word.raw_document.as_ref().unwrap().to_string().unwrap();
        let props = xml
            .split("<w:pPr>")
            .nth(1)
            .unwrap()
            .split("<w:pPrChange")
            .next()
            .unwrap();
        props
            .split("<w:rPr>")
            .nth(1)
            .unwrap()
            .split("</w:rPr>")
            .next()
            .unwrap()
            .to_string()
    };
    let mark_before = mark_properties(&word);
    let mut word = reload(&word);
    run(&mut word, vec![decide(false, None, None)]);
    assert_eq!(mark_properties(&word), mark_before);
    assert_eq!(
        word.paragraph(0)
            .unwrap()
            .properties
            .as_ref()
            .and_then(|p| p.alignment.clone()),
        original_properties
            .as_ref()
            .and_then(|p| p.alignment.clone())
    );
    assert!(formatting_revisions(&reload(&word)).is_empty());
}

#[test]
fn legacy_format_kind_filter_selects_both_current_format_kinds() {
    use jade_docx_document_core::headless_api::{select_revisions, RevisionSelector};
    let mut word = hello();
    run(
        &mut word,
        vec![
            track("Reviewer"),
            HeadlessCommand::SetBold {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: 5,
                value: true,
            },
            HeadlessCommand::SetParagraphAlignment {
                start_paragraph: 0,
                end_paragraph: 0,
                alignment: "center".into(),
            },
        ],
    );
    let snapshots = formatting_revisions(&word);
    let selected = select_revisions(
        &snapshots,
        &RevisionSelector {
            kinds: Some(vec!["formatted".into()]),
            ..Default::default()
        },
    );
    assert_eq!(selected.len(), 2);
    assert!(selected.iter().any(|r| r.kind == "runFormat"));
    assert!(selected.iter().any(|r| r.kind == "paragraphFormat"));
}
