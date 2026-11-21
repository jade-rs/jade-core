#[path = "support/package_integrity.rs"]
mod package_integrity;
// Independent text model plus strict XML/package checks at every persistence boundary.
use futures::executor::block_on;
use jade_docx_document_core::{word_document::WordDocument, Options};
use jade_docx_headless_editor::{apply_headless_commands, HeadlessCommand, HeadlessEditRequest};
use proptest::prelude::*;
use std::io::{Cursor, Read, Write};

fn seed() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/document.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>seed</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"#,
        ),
        (
            "customXml/item1.xml",
            "<sentinel keep=\"yes\">untouched</sentinel>",
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn inspect(bytes: &[u8]) -> Vec<String> {
    package_integrity::assert_valid_package(bytes);
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("valid ZIP");
    let mut xml_paragraphs = Vec::new();
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).unwrap();
        let name = file.name().expect("valid ZIP entry name").into_owned();
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .expect("CRC and decompression");
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let xml = std::str::from_utf8(&content).expect("UTF-8 XML");
            let parsed = roxmltree::Document::parse(xml)
                .unwrap_or_else(|err| panic!("invalid XML in {}: {err}: {xml:?}", name));
            if name == "word/document.xml" {
                const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
                xml_paragraphs = parsed
                    .descendants()
                    .filter(|n| n.has_tag_name((W, "p")))
                    .map(|p| {
                        p.descendants()
                            .filter_map(|n| {
                                if n.has_tag_name((W, "t")) {
                                    Some(n.text().unwrap_or(""))
                                } else if n.has_tag_name((W, "tab")) {
                                    Some("\t")
                                } else if n.has_tag_name((W, "br")) {
                                    Some(if n.attribute((W, "type")) == Some("page") {
                                        "\u{c}"
                                    } else {
                                        "\n"
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect::<String>()
                    })
                    .collect();
            }
        }
        if name == "customXml/item1.xml" {
            assert_eq!(content, b"<sentinel keep=\"yes\">untouched</sentinel>");
        }
    }
    assert!(zip.by_name("customXml/item1.xml").is_ok());
    let doc = block_on(WordDocument::load(bytes, &Options::default())).expect("reopen DOCX");
    let paragraphs: Vec<String> = (0..doc.paragraph_count())
        .map(|i| {
            doc.paragraph(i)
                .unwrap()
                .runs
                .iter()
                .map(|r| r.text.as_str())
                .collect()
        })
        .collect();
    assert_eq!(
        paragraphs, xml_paragraphs,
        "independent XML text must match reader text"
    );
    paragraphs
}

#[derive(Debug, Clone)]
enum Edit {
    Insert(usize, usize, String),
    Split(usize, usize),
    Replace(usize, usize, usize, usize, String),
}

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just('a'),
            Just(' '),
            Just('&'),
            Just('<'),
            Just('>'),
            Just('"'),
            Just('日'),
            Just('😀'),
            Just('\u{301}'),
            Just('\t'),
            Just('\n'),
            Just('\r'),
            any::<char>().prop_filter("XML 1.0 text", |ch| matches!(ch, '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')),
        ],
        0..9,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        4 => (any::<usize>(), any::<usize>(), text()).prop_map(|(p,o,t)| Edit::Insert(p,o,t)),
        2 => (any::<usize>(), any::<usize>()).prop_map(|(p,o)| Edit::Split(p,o)),
        4 => (any::<usize>(), any::<usize>(), any::<usize>(), any::<usize>(), text())
            .prop_map(|(p,o,q,e,t)| Edit::Replace(p,o,q,e,t)),
    ]
}

fn position(model: &[String], p: usize, o: usize) -> (usize, usize) {
    let p = p % model.len();
    {
        let boundaries: Vec<_> = model[p]
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(model[p].len()))
            .collect();
        (p, boundaries[o % boundaries.len()])
    }
}

fn slice(s: &str, start: usize, end: usize) -> String {
    s[start..end].to_string()
}

fn transition(model: &mut Vec<String>, edit: Edit) -> HeadlessCommand {
    match edit {
        Edit::Insert(p, o, text) => {
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            let (p, o) = position(model, p, o);
            model[p] = format!(
                "{}{}{}",
                slice(&model[p], 0, o),
                normalized,
                slice(&model[p], o, model[p].len())
            );
            HeadlessCommand::InsertText {
                paragraph: p,
                offset: o,
                text,
            }
        }
        Edit::Split(p, o) => {
            let (p, o) = position(model, p, o);
            let tail = slice(&model[p], o, model[p].len());
            model[p] = slice(&model[p], 0, o);
            model.insert(p + 1, tail);
            HeadlessCommand::InsertParagraphBreak {
                paragraph: p,
                offset: o,
                count: 1,
            }
        }
        Edit::Replace(p, o, q, e, text) => {
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            let a = position(model, p, o);
            let b = position(model, q, e);
            let ((p, o), (q, e)) = if a <= b { (a, b) } else { (b, a) };
            let joined = format!(
                "{}{}{}",
                slice(&model[p], 0, o),
                normalized,
                slice(&model[q], e, model[q].len())
            );
            model.splice(p..=q, [joined]);
            HeadlessCommand::ReplaceRange {
                start_paragraph: p,
                start_offset: o,
                end_paragraph: q,
                end_offset: e,
                text,
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|value| value.parse().ok()).unwrap_or(128),
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"))),
        .. ProptestConfig::default()
    })]
    #[test]
    fn mixed_edits_survive_every_save(edits in prop::collection::vec(edit(),1..65)) {
        let mut bytes = seed();
        let mut model = vec!["seed".to_string()];
        for (step, edit) in edits.into_iter().enumerate() {
            let command = transition(&mut model,edit);
            bytes = apply_headless_commands(&bytes,&HeadlessEditRequest { commands:vec![command.clone()] })
                .unwrap_or_else(|err| panic!("step {step}, {command:?}: {err}"));
            prop_assert_eq!(inspect(&bytes),model.clone(),"step {}, {:?}",step,command);
        }
    }

    #[test]
    fn undo_redo_preserves_exported_state(operations in prop::collection::vec((0u8..7, any::<usize>(), any::<usize>(), any::<usize>(), any::<usize>(), text()), 1..65)) {
        use jade_docx_document_core::commands::{CommandContext, CommandExecutor};
        use jade_docx_document_core::headless_api::apply_headless_commands_with_executor_to_document;
        let mut doc = block_on(WordDocument::load(&seed(), &Options::default())).unwrap();
        let mut executor = CommandExecutor::default();
        let mut selection = None;
        let mut model = vec!["seed".to_string()];
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        for (step, (kind, p, o, q, e, mut text)) in operations.into_iter().enumerate() {
            match kind {
                0..=3 => {
                    let before = model.clone();
                    if text.is_empty() { text.push('a'); }
                    let edit = match kind {
                        2 => Edit::Split(p,o),
                        3 => Edit::Replace(p,o,q,e,text),
                        _ => Edit::Insert(p,o,text),
                    };
                    let command = transition(&mut model,edit);
                    if model != before { undo.push(before); redo.clear(); }
                    selection = apply_headless_commands_with_executor_to_document(
                        &mut doc, &mut executor, selection, &HeadlessEditRequest { commands:vec![command] }
                    ).unwrap();
                }
                4 | 5 => {
                    let expected = if let Some(previous) = undo.pop() {
                        redo.push(model.clone()); model = previous; true
                    } else { false };
                    let actual = executor.undo(&mut CommandContext::new(&mut doc, &mut selection)).unwrap();
                    prop_assert_eq!(actual,expected,"undo at step {}",step);
                }
                _ => {
                    let expected = if let Some(next) = redo.pop() {
                        undo.push(model.clone()); model = next; true
                    } else { false };
                    let actual = executor.redo(&mut CommandContext::new(&mut doc, &mut selection)).unwrap();
                    prop_assert_eq!(actual,expected,"redo at step {}",step);
                }
            }
            prop_assert_eq!(inspect(&doc.export_docx().unwrap()),model.clone(),"history step {}",step);
        }
    }
}

#[test]
fn carriage_returns_become_persistent_model_line_breaks() {
    let bytes = apply_headless_commands(
        &seed(),
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 2,
                text: "\r\nX\rY".to_string(),
            }],
        },
    )
    .unwrap();
    assert_eq!(inspect(&bytes), vec!["se\nX\nYed".to_string()]);
}

#[test]
fn equivalent_line_ending_replacement_keeps_history_unchanged() {
    use jade_docx_document_core::commands::{CommandContext, CommandExecutor};
    use jade_docx_document_core::headless_api::apply_headless_commands_with_executor_to_document;
    let mut doc = block_on(WordDocument::load(&seed(), &Options::default())).unwrap();
    let mut executor = CommandExecutor::default();
    let mut selection = apply_headless_commands_with_executor_to_document(
        &mut doc,
        &mut executor,
        None,
        &HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 0,
                    text: "\n".into(),
                },
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 0,
                    start_offset: 0,
                    end_paragraph: 0,
                    end_offset: 1,
                    text: "\r\n".into(),
                },
            ],
        },
    )
    .unwrap();
    assert_eq!(inspect(&doc.export_docx().unwrap()), vec!["\nseed"]);
    assert!(executor
        .undo(&mut CommandContext::new(&mut doc, &mut selection))
        .unwrap());
    assert_eq!(inspect(&doc.export_docx().unwrap()), vec!["seed"]);
    assert!(!executor
        .undo(&mut CommandContext::new(&mut doc, &mut selection))
        .unwrap());
}

#[test]
fn snapshot_offsets_use_normalized_line_break_lengths() {
    use jade_docx_document_core::commands::CommandExecutor;
    use jade_docx_document_core::headless_api::apply_snapshot_headless_commands_with_executor_to_document;
    for (first, expected) in [
        (
            HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: "\r\n".into(),
            },
            "\nseedX",
        ),
        (
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: 1,
                text: "\r\n".into(),
            },
            "\needX",
        ),
    ] {
        let mut doc = block_on(WordDocument::load(&seed(), &Options::default())).unwrap();
        apply_snapshot_headless_commands_with_executor_to_document(
            &mut doc,
            &mut CommandExecutor::default(),
            None,
            &HeadlessEditRequest {
                commands: vec![
                    first,
                    HeadlessCommand::InsertText {
                        paragraph: 0,
                        offset: 4,
                        text: "X".into(),
                    },
                ],
            },
        )
        .unwrap();
        assert_eq!(inspect(&doc.export_docx().unwrap()), vec![expected]);
    }
}

#[test]
fn comment_values_survive_xml_normalization() {
    let text = "comment\r\n\t😀<&";
    let author = "author\r\n\t😀<&\"";
    let bytes = apply_headless_commands(
        &seed(),
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::AddComment {
                start_paragraph: 0,
                start_offset: Some(0),
                end_paragraph: 0,
                end_offset: Some(2),
                text: text.to_string(),
                author: Some(author.to_string()),
                initials: None,
                date: None,
            }],
        },
    )
    .unwrap();
    inspect(&bytes);
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/comments.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    let parsed = roxmltree::Document::parse(&xml).unwrap();
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let comment = parsed
        .descendants()
        .find(|n| n.has_tag_name((W, "comment")))
        .unwrap();
    assert_eq!(comment.attribute((W, "author")), Some(author));
    let actual: String = comment
        .descendants()
        .filter(|n| n.has_tag_name((W, "t")))
        .filter_map(|n| n.text())
        .collect();
    assert_eq!(actual, text);
}

#[test]
fn arbitrary_controls_cannot_corrupt_exported_xml() {
    for ch in [
        '\0', '\u{8}', '\u{b}', '\u{c}', '\u{1f}', '\u{fffe}', '\u{ffff}',
    ] {
        let result = apply_headless_commands(
            &seed(),
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: 2,
                    text: ch.to_string(),
                }],
            },
        );
        if ch == '\u{c}' {
            // The editor represents a page break as form feed, serialized as w:br.
            assert_eq!(inspect(&result.unwrap()), vec!["se\u{c}ed"]);
        } else {
            assert!(result.is_err(), "must reject forbidden character {ch:?}");
        }
    }
}

#[test]
fn fixture_package_graphs_are_valid() {
    for bytes in [
        include_bytes!("../../../public/fixtures/hello.docx").as_slice(),
        include_bytes!("../../../public/fixtures/table2x2.docx").as_slice(),
        include_bytes!("../../../public/fixtures/comment.docx").as_slice(),
        include_bytes!("../../../public/fixtures/hyperlink.docx").as_slice(),
        include_bytes!("../../../public/fixtures/hyperlink-field-edge-cases.docx").as_slice(),
    ] {
        package_integrity::assert_valid_package(bytes);
    }
}

#[test]
#[should_panic(expected = "missing relationship target")]
fn package_oracle_rejects_dangling_relationships() {
    let bytes = seed();
    let mut input = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..input.len() {
        let mut file = input.by_index(index).unwrap();
        let name = file.name().expect("valid ZIP entry name").into_owned();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        if name == "_rels/.rels" {
            data = String::from_utf8(data)
                .unwrap()
                .replace("word/document.xml", "word/missing.xml")
                .into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&data).unwrap();
    }
    package_integrity::assert_valid_package(&output.finish().unwrap().into_inner());
}
