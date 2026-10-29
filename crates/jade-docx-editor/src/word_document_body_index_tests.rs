use super::*;
use crate::commands::insert_paragraph_break::InsertParagraphBreakCommand;
use crate::commands::insert_text::InsertTextCommand;
use crate::commands::{Command, CommandError};
use jade_docx_layout::selection::{SelectionRange, TextPosition};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Surface {
    body: bool,
    text: String,
}

fn surface(body: bool, text: &str) -> Surface {
    Surface { body, text: text.into() }
}

fn mixed_document() -> WordDocument {
    load_body_document(concat!(
        "<w:p><w:r><w:t>Host</w:t></w:r><w:r><w:pict>",
        "<v:shape xmlns:v=\"urn:schemas-microsoft-com:vml\" style=\"width:100pt;height:50pt\">",
        "<v:textbox><w:txbxContent><w:p><w:r><w:t>Box</w:t></w:r></w:p>",
        "</w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>",
        "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p>",
        "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Nested</w:t></w:r></w:p>",
        "</w:tc></w:tr></w:tbl></w:tc></w:tr></w:tbl>",
        "<w:p><w:r><w:t>After</w:t></w:r></w:p>",
    ))
}

#[derive(Clone, Debug)]
enum Step {
    Split(usize, usize),
    Insert(usize, usize, String),
    Undo,
    Redo,
    Reopen,
    Clone,
}

fn steps() -> impl Strategy<Value = Step> {
    prop_oneof![
        5 => (1usize..12, 0usize..16).prop_map(|(i, o)| Step::Split(i, o)),
        5 => (1usize..12, 0usize..16, "[a-z ]{1,6}")
            .prop_map(|(i, o, text)| Step::Insert(i, o, text)),
        3 => Just(Step::Undo),
        3 => Just(Step::Redo),
        1 => Just(Step::Reopen),
        1 => Just(Step::Clone),
    ]
}

#[test]
fn body_indices_exclude_equations_text_boxes_and_notes_after_structural_edits() {
    fn check(doc: &WordDocument, expected: &[usize]) {
        assert_eq!(doc.body_paragraph_count(), expected.len());
        for (body, surface) in expected.iter().enumerate() {
            assert_eq!(doc.surface_index_for_body(body), Some(*surface));
        }
        for surface in 0..=doc.paragraph_count() {
            assert_eq!(doc.body_index_for_surface(surface), expected.iter().position(|i| *i == surface));
        }
        assert_eq!(doc.surface_index_for_body(expected.len()), None);
        assert_eq!(doc.body_index_for_surface(usize::MAX), None);
    }

    let mut doc = mixed_document();
    check(&doc, &[0, 2, 3, 4]);
    doc.insert_math_paragraph_after(0, MathParagraph {
        raw_omml: "<m:oMathPara xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\"><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></m:oMathPara>".into(),
        ..MathParagraph::default()
    }).unwrap();
    check(&doc, &[0, 3, 4, 5]);
    let footnote = doc.insert_footnote_at(0, 4, "Footnote").unwrap();
    let endnote = doc.insert_endnote_at(0, 4, "Endnote").unwrap();
    check(&doc, &[0, 4, 5, 6]);
    assert!(doc.split_paragraph_at(6, 2));
    check(&doc, &[0, 4, 5, 6, 7]);
    assert_eq!(doc.document_text_metrics().words, 5);
    let exported = doc.export_docx().unwrap();
    let mut doc = executor::block_on(WordDocument::load(&exported, &Options::default())).unwrap();
    check(&doc.clone(), &[0, 4, 5, 6, 7]);
    doc.delete_footnote(&footnote).unwrap();
    check(&doc, &[0, 3, 4, 5, 6]);
    doc.delete_endnote(&endnote).unwrap();
    check(&doc, &[0, 3, 4, 5, 6]);
    assert!(doc.remove_paragraph_at(5));
    check(&doc, &[0, 3, 4, 5]);

    doc.document_mut().unwrap().body = None;
    doc.rebuild_paragraph_surfaces();
    check(&doc, &[]);
    doc.document = None;
    doc.rebuild_paragraph_surfaces();
    check(&doc, &[]);
    assert_eq!(doc.document_text_metrics(), DocumentTextMetrics::default());
}

// The oracle is a flat list of story/text values, not production paths or a
// cloned document. The first body paragraph owns the fixed text box; edits
// target the nested table and later body paragraphs. Undoing a split exercises
// joining, while reloads cross the actual DOCX persistence boundary.
proptest! {
    #![proptest_config(ProptestConfig { cases: 96, max_shrink_iters: 3000, ..ProptestConfig::default() })]
    #[test]
    fn property_body_indices_follow_edit_history(commands in prop::collection::vec(steps(), 1..81)) {
        let mut doc = mixed_document();
        let mut model = vec![surface(true, "Host"), surface(false, "Box"), surface(true, "Cell"), surface(true, "Nested"), surface(true, "After")];
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        let mut history = CommandExecutor::new();
        let mut selection = None;
        for (step, command) in commands.iter().enumerate() {
            match command {
                Step::Split(body, offset) | Step::Insert(body, offset, _) => {
                    let target = model.iter().enumerate().filter(|(_, value)| value.body).nth(*body).map(|(i, _)| i);
                    let position = target.map(|i| TextPosition::new(i, (*offset).min(model[i].text.len()))).unwrap_or(TextPosition::new(usize::MAX, 0));
                    let range = SelectionRange::collapsed(position);
                    let operation: Box<dyn Command> = match command {
                        Step::Split(..) => Box::new(InsertParagraphBreakCommand::new(range)),
                        Step::Insert(_, _, text) => Box::new(InsertTextCommand::new(range, text.clone(), None)),
                        _ => unreachable!(),
                    };
                    let result = history.execute(operation, &mut CommandContext::new(&mut doc, &mut selection));
                    if let Some(i) = target {
                        prop_assert!(result.is_ok(), "step {step}: {command:?}: {result:?}");
                        undo.push(model.clone());
                        redo.clear();
                        match command {
                            Step::Split(..) => {
                                let tail = model[i].text.split_off(position.offset);
                                model.insert(i + 1, surface(true, &tail));
                            }
                            Step::Insert(_, _, text) => model[i].text.insert_str(position.offset, text),
                            _ => unreachable!(),
                        }
                    } else {
                        prop_assert!(matches!(result, Err(CommandError::ParagraphUnavailable)), "step {step}: {command:?}: {result:?}");
                    }
                }
                Step::Undo => {
                    let expected = undo.pop();
                    let changed = history.undo(&mut CommandContext::new(&mut doc, &mut selection)).unwrap();
                    prop_assert_eq!(changed, expected.is_some(), "step {}: {:?}", step, command);
                    if let Some(previous) = expected { redo.push(model); model = previous; }
                }
                Step::Redo => {
                    let expected = redo.pop();
                    let changed = history.redo(&mut CommandContext::new(&mut doc, &mut selection)).unwrap();
                    prop_assert_eq!(changed, expected.is_some(), "step {}: {:?}", step, command);
                    if let Some(next) = expected { undo.push(model); model = next; }
                }
                Step::Reopen => {
                    doc = executor::block_on(WordDocument::load(&doc.export_docx().unwrap(), &Options::default())).unwrap();
                    history = CommandExecutor::new();
                    undo.clear(); redo.clear(); selection = None;
                }
                Step::Clone => doc = doc.clone(),
            }
            let expected_body = model.iter().enumerate().filter(|(_, value)| value.body).collect::<Vec<_>>();
            prop_assert_eq!(doc.body_paragraph_count(), expected_body.len(), "step {}: {:?}", step, command);
            prop_assert_eq!(doc.paragraph_count(), model.len(), "step {}: {:?}", step, command);
            for (body_index, (surface_index, value)) in expected_body.iter().enumerate() {
                prop_assert_eq!(doc.surface_index_for_body(body_index), Some(*surface_index), "step {}: {:?}", step, command);
                prop_assert_eq!(doc.body_index_for_surface(*surface_index), Some(body_index), "step {}: {:?}", step, command);
                let actual = doc.paragraph(*surface_index).unwrap().runs.iter().map(|run| run.text.as_str()).collect::<String>();
                prop_assert_eq!(&actual, &value.text, "step {}: {:?}", step, command);
            }
            for (index, value) in model.iter().enumerate().filter(|(_, value)| !value.body) {
                prop_assert_eq!(doc.body_index_for_surface(index), None);
                prop_assert_eq!(doc.paragraph(index).unwrap().runs.iter().map(|run| run.text.as_str()).collect::<String>(), value.text.as_str());
            }
            prop_assert_eq!(doc.surface_index_for_body(expected_body.len()), None);
            prop_assert_eq!(doc.body_index_for_surface(model.len()), None);
            let counts = doc.document_text_metrics();
            prop_assert_eq!(counts.words, expected_body.iter().map(|(_, value)| value.text.split_whitespace().count()).sum::<usize>());
            prop_assert_eq!(counts.chars_with_space, expected_body.iter().map(|(_, value)| value.text.len()).sum::<usize>());
            prop_assert_eq!(counts.chars_no_space, expected_body.iter().map(|(_, value)| value.text.bytes().filter(|c| !c.is_ascii_whitespace()).count()).sum::<usize>());
            prop_assert_eq!(counts.paragraphs, expected_body.iter().filter(|(_, value)| !value.text.trim().is_empty()).count());
        }
    }
}

// No timing thresholds in CI: run this ignored probe with the same optimized
// profile before and after a change to compare the scaling of each operation.
#[test]
#[ignore]
fn benchmark_body_surface_metrics_scaling() {
    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort_unstable();
        samples[samples.len() / 2]
    }

    for count in [512, 4_096, 16_384] {
        let mut doc = WordDocument::from_document(Document {
            body: Some(Body {
                elements: (0..count)
                    .map(|_| BodyElement::from_paragraph(make_paragraph("word data")))
                    .collect(),
            }),
            ..Document::default()
        });
        let mut initial = Vec::new();
        let mut cached = Vec::new();
        let mut dirty = Vec::new();
        let mut lookup = Vec::new();
        for _ in 0..9 {
            doc.text_metrics_cache.get_mut().invalidate_all();
            let start = Instant::now();
            assert_eq!(black_box(doc.document_text_metrics()).words, 2 * count);
            initial.push(start.elapsed());

            let start = Instant::now();
            for _ in 0..1_000 {
                black_box(doc.document_text_metrics());
            }
            cached.push(start.elapsed() / 1_000);

            let start = Instant::now();
            for _ in 0..1_000 {
                doc.sync_paragraph_text(black_box(count - 1));
                black_box(doc.document_text_metrics());
            }
            dirty.push(start.elapsed() / 1_000);

            let start = Instant::now();
            for _ in 0..10_000 {
                black_box(doc.surface_index_for_body(black_box(count - 1)));
                black_box(doc.body_index_for_surface(black_box(count - 1)));
            }
            lookup.push(start.elapsed() / 10_000);
        }
        println!(
            "body paragraphs={count}: initial={:?}, cached={:?}, dirty-last={:?}, lookup-pair={:?}",
            median(initial), median(cached), median(dirty), median(lookup)
        );
    }
}
