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

/// `text[kind author]` per run, `in ins author` for a nested insertion.
fn runs(word: &WordDocument, paragraph: usize) -> Vec<String> {
    word.paragraph(paragraph)
        .unwrap()
        .runs
        .iter()
        .filter(|r| !r.text.is_empty())
        .map(|r| match &r.revision {
            None => r.text.clone(),
            Some(m) => {
                let kind = match m.kind {
                    RevisionKind::Inserted => "ins",
                    RevisionKind::Deleted => "del",
                    RevisionKind::MovedFrom => "from",
                    RevisionKind::MovedTo => "to",
                };
                let author = m.author.clone().unwrap_or_default();
                match &m.insertion {
                    Some(i) => format!(
                        "{}[{kind} {author} in ins {}]",
                        r.text,
                        i.author.clone().unwrap_or_default()
                    ),
                    None => format!("{}[{kind} {author}]", r.text),
                }
            }
        })
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
