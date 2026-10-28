use super::{edit, final_text, fixture, part, NS};
use jade_docx_headless_editor::{HeadlessCommand, HeadlessRevisionKindFilter};
use proptest::prelude::*;

const AUTHORS: [&str; 2] = ["Ann", "Bob"];
const EDIT_DATE: &str = "2026-10-28T00:00:00Z";
const ORIGINAL_DATES: [&str; 2] = ["2025-01-01T00:00:00Z", "2025-01-02T00:00:00Z"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Revision {
    author: bool,
    original: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Character {
    text: char,
    inserted_by: Option<Revision>,
    deleted_by: Option<Revision>,
}

fn saved_characters(bytes: &[u8]) -> Vec<Character> {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    document
        .descendants()
        .filter(|n| n.has_tag_name((NS, "t")) || n.has_tag_name((NS, "delText")))
        .flat_map(|node| {
            let author = |kind| {
                node.ancestors()
                    .find(|n| n.has_tag_name((NS, kind)))
                    .map(|n| {
                        let author = match n.attribute((NS, "author")) {
                            Some("Ann") => false,
                            Some("Bob") => true,
                            other => panic!("unexpected author {other:?}"),
                        };
                        let original = match n.attribute((NS, "date")) {
                            Some(EDIT_DATE) => false,
                            Some(date) if date == ORIGINAL_DATES[author as usize] => true,
                            other => panic!("unexpected revision date {other:?}"),
                        };
                        Revision { author, original }
                    })
            };
            let inserted_by = author("ins");
            let deleted_by = author("del");
            node.text()
                .unwrap_or_default()
                .chars()
                .map(move |text| Character {
                    text,
                    inserted_by,
                    deleted_by,
                })
        })
        .collect()
}

fn replace(
    model: &mut Vec<Character>,
    from: usize,
    to: usize,
    text: &str,
    author: bool,
    tracked: bool,
) {
    let replacement: Vec<_> = text.chars().collect();
    let selected = &model[from..to];
    let (prefix, suffix) = if selected.iter().any(|c| c.deleted_by.is_some()) {
        (0, 0)
    } else {
        let prefix = selected
            .iter()
            .zip(&replacement)
            .take_while(|(a, b)| a.text == **b)
            .count();
        let suffix = selected[prefix..]
            .iter()
            .rev()
            .zip(replacement[prefix..].iter().rev())
            .take_while(|(a, b)| a.text == **b)
            .count();
        (prefix, suffix)
    };
    let mut changed: Vec<_> = model[from + prefix..to - suffix]
        .iter()
        .filter_map(|c| {
            if c.deleted_by.is_some() {
                Some(c.clone())
            } else if tracked && c.inserted_by.map(|mark| mark.author) != Some(author) {
                Some(Character {
                    deleted_by: Some(Revision {
                        author,
                        original: false,
                    }),
                    ..c.clone()
                })
            } else {
                None
            }
        })
        .collect();
    changed.extend(
        replacement[prefix..replacement.len() - suffix]
            .iter()
            .map(|text| Character {
                text: *text,
                inserted_by: tracked.then_some(Revision {
                    author,
                    original: false,
                }),
                deleted_by: None,
            }),
    );
    model.splice(from + prefix..to - suffix, changed);
}

fn review(model: &mut Vec<Character>, accept: bool, author: Option<bool>, kind: u8) {
    model.retain_mut(|c| {
        let insertion = kind != 2
            && c.inserted_by
                .is_some_and(|mark| author.is_none_or(|filter| filter == mark.author));
        let deletion = kind != 1
            && c.deleted_by
                .is_some_and(|mark| author.is_none_or(|filter| filter == mark.author));
        if (insertion && !accept) || (deletion && accept) {
            return false;
        }
        if insertion {
            c.inserted_by = None;
        }
        if deletion {
            c.deleted_by = None;
        }
        true
    });
}

fn decision(bytes: &[u8], accept: bool, author: Option<bool>, kind: u8) -> HeadlessCommand {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let ids = document
        .descendants()
        .filter(|n| {
            (n.has_tag_name((NS, "ins")) || n.has_tag_name((NS, "del")))
                && author.is_none_or(|a| n.attribute((NS, "author")) == Some(AUTHORS[a as usize]))
        })
        .filter_map(|n| n.attribute((NS, "id")).map(str::to_owned))
        .collect();
    let mut command = super::decide(accept, ids);
    let filter = match kind {
        1 => Some(HeadlessRevisionKindFilter::Inserted),
        2 => Some(HeadlessRevisionKindFilter::Deleted),
        _ => None,
    };
    match &mut command {
        HeadlessCommand::AcceptRevisions { kind, .. }
        | HeadlessCommand::RejectRevisions { kind, .. } => *kind = filter,
        _ => unreachable!(),
    }
    command
}

#[derive(Clone, Debug)]
enum Action {
    Replace {
        from: u8,
        to: u8,
        text: String,
        author: bool,
        tracked: bool,
    },
    Review {
        accept: bool,
        author: Option<bool>,
        kind: u8,
    },
    UnknownReview(bool),
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        7 => (any::<u8>(),any::<u8>(),prop_oneof![Just(""), Just("😀"), Just("甲乙"), Just("e\u{301}"), Just("<&>")],any::<bool>(),any::<bool>())
            .prop_map(|(from,to,text,author,tracked)| Action::Replace { from,to,text:text.into(),author,tracked }),
        3 => (any::<bool>(),prop::option::of(any::<bool>()),0u8..3).prop_map(|(accept,author,kind)| Action::Review { accept,author,kind }),
        1 => any::<bool>().prop_map(Action::UnknownReview),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases:128,
        failure_persistence:Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"),"/tests/inline_authors.proptest-regressions")
        ))),
        ..ProptestConfig::default()
    })]
    #[test]
    fn mixed_author_edits_and_selective_reviews_match_character_model(actions in prop::collection::vec(action(),1..41)) {
        let mut bytes = fixture(r#"<w:r><w:t>甲</w:t></w:r><w:ins w:id="41" w:author="Ann" w:date="2025-01-01T00:00:00Z"><w:r><w:t>😀</w:t></w:r></w:ins><w:del w:id="42" w:author="Bob" w:date="2025-01-02T00:00:00Z"><w:r><w:delText>乙</w:delText></w:r></w:del><w:ins w:id="43" w:author="Bob" w:date="2025-01-02T00:00:00Z"><w:del w:id="44" w:author="Ann" w:date="2025-01-01T00:00:00Z"><w:r><w:delText>丙</w:delText></w:r></w:del></w:ins><w:r><w:t>終</w:t></w:r>"#);
        let mut model = vec![
            Character { text:'甲',inserted_by:None,deleted_by:None },
            Character { text:'😀',inserted_by:Some(Revision { author:false,original:true }),deleted_by:None },
            Character { text:'乙',inserted_by:None,deleted_by:Some(Revision { author:true,original:true }) },
            Character { text:'丙',inserted_by:Some(Revision { author:true,original:true }),deleted_by:Some(Revision { author:false,original:true }) },
            Character { text:'終',inserted_by:None,deleted_by:None },
        ];
        // Establish the serializer's canonical XML before checking exact no-ops.
        bytes = edit(&bytes,vec![HeadlessCommand::StopChangeTracking]);
        for (step,action) in actions.iter().enumerate() {
            match action {
                Action::Replace { from,to,text,author,tracked } => {
                    let a = *from as usize % (model.len()+1);
                    let b = *to as usize % (model.len()+1);
                    let (from,to) = (a.min(b),a.max(b));
                    let offset = |end| model[..end].iter().map(|c| c.text.len_utf8()).sum();
                    bytes = edit(&bytes,vec![
                        if *tracked { HeadlessCommand::StartChangeTracking { author:Some(AUTHORS[*author as usize].into()),initials:None,date:Some(EDIT_DATE.into()),date_utc:None } }
                            else { HeadlessCommand::StopChangeTracking },
                        HeadlessCommand::ReplaceRange { start_paragraph:0,start_offset:offset(from),end_paragraph:0,end_offset:offset(to),text:text.clone() },
                    ]);
                    replace(&mut model,from,to,text,*author,*tracked);
                }
                Action::Review { accept,author,kind } => {
                    bytes = edit(&bytes,vec![decision(&bytes,*accept,*author,*kind)]);
                    review(&mut model,*accept,*author,*kind);
                }
                Action::UnknownReview(accept) => {
                    let before = part(&bytes,"word/document.xml");
                    bytes = edit(&bytes,vec![super::decide(*accept,vec!["999999".into()])]);
                    prop_assert_eq!(part(&bytes,"word/document.xml"),before,"step {} {:?} unknown revision is a no-op",step,action);
                }
            }
            prop_assert_eq!(saved_characters(&bytes),model.clone(),"step {} {:?}",step,action);
            let visible: String = model.iter().filter(|c| c.deleted_by.is_none()).map(|c| c.text).collect();
            prop_assert_eq!(final_text(&bytes),visible,"step {} {:?} visible text",step,action);
        }
    }
}
