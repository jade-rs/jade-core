use jade_docx_headless_editor::{apply_headless_commands, HeadlessCommand, HeadlessEditRequest};
use proptest::prelude::*;
use std::io::{Cursor, Read, Write};

const NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
    xml
}

fn fixture(runs: &str) -> Vec<u8> {
    let input = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/hello.docx"
    ))
    .unwrap();
    let mut source = zip::ZipArchive::new(Cursor::new(input)).unwrap();
    let mut target = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..source.len() {
        let mut entry = source.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if entry.name().unwrap() == "word/document.xml" {
            bytes = format!(r#"<w:document xmlns:w="{NS}"><w:body><w:p>{runs}</w:p><w:sectPr/></w:body></w:document>"#).into_bytes();
        }
        if entry.name().unwrap() == "word/styles.xml" {
            bytes = format!(r#"<w:styles xmlns:w="{NS}"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Cambria"/><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:rFonts w:ascii="MS Mincho" w:eastAsia="MS Mincho"/><w:sz w:val="21"/></w:rPr></w:style></w:styles>"#).into_bytes();
        }
        target
            .start_file(
                entry.name().unwrap(),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        target.write_all(&bytes).unwrap();
    }
    target.finish().unwrap().into_inner()
}

fn edit(bytes: &[u8], commands: Vec<HeadlessCommand>) -> Vec<u8> {
    apply_headless_commands(bytes, &HeadlessEditRequest { commands }).unwrap()
}

/// Independent package oracle: Word's final view excludes text inside w:del.
fn final_text(bytes: &[u8]) -> String {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    document
        .descendants()
        .filter(|n| {
            n.has_tag_name((NS, "t"))
                && !n.ancestors().any(|a| {
                    a.has_tag_name((NS, "del"))
                        || a.has_tag_name((NS, "moveFrom"))
                        || (a.has_tag_name((NS, "tr"))
                            && a.children()
                                .filter(|p| p.has_tag_name((NS, "trPr")))
                                .any(|p| p.children().any(|mark| mark.has_tag_name((NS, "del")))))
                })
        })
        .filter_map(|n| n.text())
        .collect()
}

fn tracking() -> HeadlessCommand {
    HeadlessCommand::StartChangeTracking {
        author: Some("LAWGUE".into()),
        initials: Some("LG".into()),
        date: Some("2026-10-24T00:00:00Z".into()),
        date_utc: None,
    }
}

fn decide(accept: bool, ids: Vec<String>) -> HeadlessCommand {
    if accept {
        HeadlessCommand::AcceptRevisions {
            kind: None,
            start_paragraph: None,
            end_paragraph: None,
            revision_id: None,
            revision_ids: Some(ids),
            revision_targets: None,
        }
    } else {
        HeadlessCommand::RejectRevisions {
            kind: None,
            start_paragraph: None,
            end_paragraph: None,
            revision_id: None,
            revision_ids: Some(ids),
            revision_targets: None,
        }
    }
}

fn lg_ids(bytes: &[u8]) -> Vec<String> {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    document
        .descendants()
        .filter(|n| n.attribute((NS, "author")) == Some("LAWGUE"))
        .filter_map(|n| n.attribute((NS, "id")).map(str::to_owned))
        .collect()
}

fn assert_other_author(bytes: &[u8]) {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let insertion = document
        .descendants()
        .find(|n| n.has_tag_name((NS, "ins")) && n.attribute((NS, "id")) == Some("41"))
        .expect("other author's insertion survives");
    assert_eq!(insertion.attribute((NS, "author")), Some("Reviewer"));
    assert_eq!(
        insertion.attribute((NS, "date")),
        Some("2025-01-01T00:00:00Z")
    );
    assert_eq!(
        insertion
            .descendants()
            .filter(|n| n.has_tag_name((NS, "t")))
            .filter_map(|n| n.text())
            .collect::<String>(),
        "R"
    );
}

#[test]
fn replacement_at_run_boundary_uses_replaced_run_format() {
    let input = fixture(
        r#"<w:r><w:rPr><w:b/></w:rPr><w:t>前</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>30日</w:t></w:r>"#,
    );
    let output = edit(
        &input,
        vec![HeadlessCommand::ReplaceRange {
            start_paragraph: 0,
            start_offset: 3,
            end_paragraph: 0,
            end_offset: 5,
            text: "60".into(),
        }],
    );
    let xml = part(&output, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let replacement = document
        .descendants()
        .find(|n| n.has_tag_name((NS, "t")) && n.text().is_some_and(|t| t.contains("6")))
        .unwrap()
        .parent()
        .unwrap();
    assert!(
        replacement.descendants().any(|n| n.has_tag_name((NS, "i"))),
        "replacement must inherit italic: {xml}"
    );
    assert!(
        !replacement.descendants().any(|n| n.has_tag_name((NS, "b"))),
        "replacement must not inherit preceding bold run: {xml}"
    );
    assert!(
        !xml.contains("Cambria"),
        "editing must not materialize docDefaults over Normal: {xml}"
    );
    assert_eq!(
        part(&input, "word/styles.xml"),
        part(&output, "word/styles.xml")
    );
}

fn body_paragraph_count(bytes: &[u8]) -> usize {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    document
        .descendants()
        .find(|n| n.has_tag_name((NS, "body")))
        .unwrap()
        .children()
        .filter(|n| n.has_tag_name((NS, "p")))
        .count()
}

// Synthetic contracts reproduced from lawgue#19947. These fixtures include
// inherited fonts, numbering, a table, and (review) foreign revisions/comments.
#[test]
fn deleting_a_range_containing_a_whole_table_removes_its_grid() {
    let input = include_bytes!("fixtures/staging-base.docx");
    let result = apply_headless_commands(
        input,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::ReplaceRange {
                start_paragraph: 24,
                start_offset: 0,
                end_paragraph: 36,
                end_offset: 27,
                text: String::new(),
            }],
        },
    );
    let output = result.unwrap();
    let xml = part(&output, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let cells: Vec<_> = document
        .descendants()
        .filter(|node| node.has_tag_name((NS, "tc")))
        .collect();
    assert!(cells.is_empty());
    assert_eq!(table_shape(&output), None);
    assert!(!final_text(&output).contains("別表（委託料）"));
    for accept in [false, true] {
        let tracked = edit(
            input,
            vec![
                tracking(),
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 24,
                    start_offset: 0,
                    end_paragraph: 36,
                    end_offset: 27,
                    text: String::new(),
                },
            ],
        );
        assert_eq!(tracked_rows(&tracked), Some(vec![2; 4]));
        assert!(!final_text(&tracked).contains("別表（委託料）"));
        let decided = edit(&tracked, vec![decide(accept, lg_ids(&tracked))]);
        assert_eq!(
            table_shape(&decided),
            if accept { None } else { Some((4, 3)) }
        );
        if !accept {
            assert_eq!(final_text(&decided), final_text(input));
        }
    }
}

fn range_table(left: &str, right: &str) -> String {
    format!(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>{left}</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>{right}</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
    )
}

fn range_delete(
    start: usize,
    start_offset: usize,
    end: usize,
    end_offset: usize,
) -> HeadlessCommand {
    HeadlessCommand::ReplaceRange {
        start_paragraph: start,
        start_offset,
        end_paragraph: end,
        end_offset,
        text: String::new(),
    }
}

#[test]
fn range_deletion_removes_multiple_and_nested_tables_but_keeps_partial_cells() {
    let one = range_table("AB", "CD");
    let nested = range_table(&format!("A</w:t></w:r></w:p>{one}<w:p><w:r><w:t>B"), "CD");
    for table in [format!("{one}{one}"), nested] {
        let input = fixture(&format!(
            "<w:r><w:t>HEAD</w:t></w:r></w:p>{table}<w:p><w:r><w:t>TAIL</w:t></w:r>"
        ));
        let paragraph_count = {
            let xml = part(&input, "word/document.xml");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            doc.descendants()
                .filter(|n| n.has_tag_name((NS, "p")))
                .count()
        };
        for tracked in [false, true] {
            let mut commands = vec![];
            if tracked {
                commands.push(tracking());
            }
            commands.push(range_delete(0, 0, paragraph_count - 2, 2));
            let output = edit(&input, commands);
            assert_eq!(final_text(&output), "TAIL");
            let accepted = if tracked {
                edit(&output, vec![decide(true, lg_ids(&output))])
            } else {
                output
            };
            assert_eq!(table_shape(&accepted), None);
        }
    }
    let input = fixture(&format!(
        "<w:r><w:t>HEAD</w:t></w:r></w:p>{one}<w:p><w:r><w:t>TAIL</w:t></w:r>"
    ));
    let partial = edit(&input, vec![range_delete(1, 1, 2, 1)]);
    assert_eq!(table_shape(&partial), Some((1, 2)));
    assert_eq!(final_text(&partial), "HEADADTAIL");
}

// Independent model: each one-row table has a plain/inserted/deleted mark and
// two cell strings. Every public operation exports and reloads a DOCX package.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn persisted_range_table_deletion_matches_model(actions in prop::collection::vec((0u8..7, any::<u8>(), any::<bool>()), 1..25)) {
        let table = range_table("AB", "CD");
        let mut bytes = fixture(&format!(r#"<w:ins w:id="40" w:author="Reviewer"><w:r><w:t>R</w:t></w:r></w:ins></w:p>{table}<w:p><w:r><w:t>TAIL</w:t></w:r>"#));
        let mut model = vec![(0u8, ["AB".to_string(), "CD".to_string()])];
        for (step, (kind, index, tracked)) in actions.into_iter().enumerate() {
            let mut expected = model.clone();
            let mut invalid = false;
            let command = match kind {
                0 => {
                    expected.insert(0, (u8::from(tracked), [String::new(), String::new()]));
                    HeadlessCommand::InsertTable { after_paragraph: 0, rows: 1, columns: 2, column_widths: vec![], caret_offset: None }
                },
                1 if !expected.is_empty() => {
                    let table = index as usize % expected.len();
                    let end = expected[table].1[1].len();
                    if !tracked || expected[table].0 == 1 { expected.remove(table); }
                    else { expected[table].0 = 2; }
                    range_delete(1 + table * 2, 0, 2 + table * 2, end)
                },
                2 if !expected.is_empty() => {
                    let table = index as usize % expected.len();
                    let end = usize::from(!expected[table].1[0].is_empty());
                    expected[table].1[0].replace_range(..end, "");
                    // A partial cell edit never removes the grid, even if empty.
                    let command = range_delete(1 + table * 2, 0, 1 + table * 2, end);
                    let output = apply_headless_commands(&bytes, &HeadlessEditRequest { commands: vec![HeadlessCommand::StopChangeTracking, command] });
                    prop_assert!(output.is_ok(), "partial cell step {}: {:?}", step, output.as_ref().err());
                    bytes = output.unwrap();
                    model = expected.clone();
                    HeadlessCommand::StopChangeTracking
                },
                3 | 4 => {
                    let accept = kind == 3;
                    expected.retain(|(state, _)| *state != if accept { 2 } else { 1 });
                    for (state, _) in &mut expected { *state = 0; }
                    decide(accept, lg_ids(&bytes))
                },
                6 => decide(tracked, vec![]),
                _ => { invalid = true; range_delete(99, 0, 99, 0) },
            };
            let output = apply_headless_commands(&bytes, &HeadlessEditRequest { commands: vec![if tracked { tracking() } else { HeadlessCommand::StopChangeTracking }, command] });
            if invalid { prop_assert!(output.is_err(), "invalid range at step {}", step); }
            else { prop_assert!(output.is_ok(), "step {} kind {}: {:?}", step, kind, output.as_ref().err()); bytes = output.unwrap(); model = expected; }
            let xml = part(&bytes, "word/document.xml");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let actual: Vec<_> = doc.descendants().filter(|n| n.has_tag_name((NS, "tbl"))).map(|table| {
                let row = table.children().find(|n| n.has_tag_name((NS, "tr"))).unwrap();
                let state = row.children().find(|n| n.has_tag_name((NS, "trPr"))).map_or(0, |p| {
                    if p.children().any(|n| n.has_tag_name((NS, "ins"))) { 1 } else if p.children().any(|n| n.has_tag_name((NS, "del"))) { 2 } else { 0 }
                });
                let cells: Vec<String> = row.children().filter(|n| n.has_tag_name((NS, "tc"))).map(|c| c.descendants().filter(|n| n.has_tag_name((NS, "t"))).filter_map(|n| n.text()).collect()).collect();
                (state, cells)
            }).collect();
            let expected: Vec<_> = model.iter().map(|(state, cells)| (*state, cells.to_vec())).collect();
            prop_assert_eq!(actual, expected, "persisted table state at step {} kind {}", step, kind);
            let expected_text = format!("R{}TAIL", model.iter().filter(|(state, _)| *state != 2).flat_map(|(_, cells)| cells.iter()).cloned().collect::<String>());
            prop_assert_eq!(final_text(&bytes), expected_text, "visible text at step {}", step);
            prop_assert!(doc.descendants().any(|n| n.has_tag_name((NS, "ins")) && n.attribute((NS, "author")) == Some("Reviewer") && n.attribute((NS, "id")) == Some("40")), "foreign revision at step {}", step);
        }
    }
}

#[test]
fn staging_contract_workflows_preserve_styles_and_foreign_parts() {
    let base = include_bytes!("fixtures/staging-base.docx");
    let review = include_bytes!("fixtures/staging-review.docx");
    for (name, input, tracked) in [
        ("T00", base.as_slice(), false),
        ("T01", base.as_slice(), true),
        ("T02", base.as_slice(), true),
        ("T03", review.as_slice(), true),
        ("T04", review.as_slice(), true),
    ] {
        let mut commands = Vec::new();
        if tracked {
            commands.push(tracking());
        }
        commands.push(HeadlessCommand::ReplaceRange {
            start_paragraph: 12,
            start_offset: 52,
            end_paragraph: 12,
            end_offset: 63,
            text: "60日以内".into(),
        });
        commands.push(HeadlessCommand::InsertParagraphBreak {
            paragraph: 9,
            offset: 0,
            count: 1,
        });
        commands.push(HeadlessCommand::InsertText {
            paragraph: 9,
            offset: 0,
            text: "(4)　分析結果に基づく改善提案".into(),
        });
        let output = edit(input, commands);
        assert!(final_text(&output).contains("60日以内"));
        assert!(final_text(&output).contains("(4)　分析結果に基づく改善提案"));
        assert_eq!(
            part(input, "word/styles.xml"),
            part(&output, "word/styles.xml")
        );
        assert_eq!(
            part(input, "word/numbering.xml"),
            part(&output, "word/numbering.xml")
        );
        assert_eq!(table_shape(&output), Some((4, 3)));
        let xml = part(&output, "word/document.xml");
        let document = roxmltree::Document::parse(&xml).unwrap();
        let replacement = document
            .descendants()
            .find(|n| n.has_tag_name((NS, "t")) && n.text().is_some_and(|t| t.contains("6")))
            .unwrap()
            .parent()
            .unwrap();
        assert!(
            replacement.descendants().any(|n| n.has_tag_name((NS, "u"))),
            "{name}: underline survives"
        );
        if name == "T03" || name == "T04" {
            for part_name in ["word/header1.xml", "word/footer1.xml", "word/comments.xml"] {
                assert_eq!(
                    part(input, part_name),
                    part(&output, part_name),
                    "{name}: {part_name}"
                );
            }
            let rejected = edit(&output, vec![decide(false, lg_ids(&output))]);
            assert_eq!(final_text(&rejected), final_text(input));
            let before_xml = part(input, "word/document.xml");
            let before = roxmltree::Document::parse(&before_xml).unwrap();
            let foreign = |doc: &roxmltree::Document<'_>| {
                doc.descendants()
                    .filter(|n| n.attribute((NS, "author")) == Some("相手方法務"))
                    .map(|n| {
                        (
                            n.tag_name().name().to_owned(),
                            n.attribute((NS, "id")).map(str::to_owned),
                            n.attribute((NS, "date")).map(str::to_owned),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(foreign(&before), foreign(&document));
        }
        if let Ok(directory) = std::env::var("DOCX_VERIFICATION_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::Path::new(&directory).join(format!("{name}.docx")),
                &output,
            )
            .unwrap();
            if tracked {
                let accepted = edit(&output, vec![decide(true, lg_ids(&output))]);
                std::fs::write(
                    std::path::Path::new(&directory).join(format!("{name}-accepted.docx")),
                    accepted,
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn replacing_another_authors_insertion_nests_our_deletion() {
    let input = fixture(
        r#"<w:r><w:t>直近</w:t></w:r><w:ins w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>6か月</w:t></w:r></w:ins>"#,
    );
    let output = edit(
        &input,
        vec![
            tracking(),
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 6,
                end_paragraph: 0,
                end_offset: 7,
                text: "9".into(),
            },
        ],
    );
    assert_eq!(final_text(&output), "直近9か月");
    let xml = part(&output, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let deletion = document
        .descendants()
        .find(|n| n.has_tag_name((NS, "del")) && n.attribute((NS, "author")) == Some("LAWGUE"))
        .expect("our deletion remains tracked");
    assert!(
        deletion.ancestors().any(|n| n.has_tag_name((NS, "ins"))
            && n.attribute((NS, "author")) == Some("Reviewer")
            && n.attribute((NS, "id")) == Some("41")),
        "source insertion and authorship survive: {xml}"
    );
    let rejected = edit(&output, vec![decide(false, lg_ids(&output))]);
    assert_eq!(final_text(&rejected), "直近6か月");
    assert!(part(&rejected, "word/document.xml").contains("Reviewer"));
}

#[test]
fn tracked_paragraph_split_and_join_survive_reload_and_decisions() {
    let input = fixture("<w:r><w:t>契約本文</w:t></w:r>");
    for accept in [false, true] {
        let split = edit(
            &input,
            vec![
                tracking(),
                HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset: 6,
                    count: 1,
                },
            ],
        );
        assert_eq!(body_paragraph_count(&split), 2);
        assert_eq!(final_text(&split), "契約本文");
        let ids = lg_ids(&split);
        assert!(!ids.is_empty(), "paragraph mark must be tracked");
        let decided = edit(&split, vec![decide(accept, ids)]);
        assert_eq!(body_paragraph_count(&decided), if accept { 2 } else { 1 });
        assert_eq!(final_text(&decided), "契約本文");
    }
    let split = edit(
        &input,
        vec![HeadlessCommand::InsertParagraphBreak {
            paragraph: 0,
            offset: 6,
            count: 1,
        }],
    );
    for accept in [false, true] {
        let joined = edit(
            &split,
            vec![
                tracking(),
                HeadlessCommand::ReplaceRange {
                    start_paragraph: 0,
                    start_offset: 6,
                    end_paragraph: 1,
                    end_offset: 0,
                    text: String::new(),
                },
            ],
        );
        let ids = lg_ids(&joined);
        assert!(!ids.is_empty(), "deleted paragraph mark must be tracked");
        let decided = edit(&joined, vec![decide(accept, ids)]);
        assert_eq!(body_paragraph_count(&decided), if accept { 1 } else { 2 });
        assert_eq!(final_text(&decided), "契約本文");
    }
}

#[test]
fn tracked_table_rows_survive_reload_and_decisions() {
    let input = edit(
        &fixture("<w:r><w:t>契約</w:t></w:r>"),
        vec![HeadlessCommand::InsertTable {
            after_paragraph: 0,
            rows: 2,
            columns: 2,
            column_widths: vec![],
            caret_offset: None,
        }],
    );
    for accept in [false, true] {
        let inserted = edit(
            &input,
            vec![
                tracking(),
                HeadlessCommand::InsertTableRow {
                    table_order_index: 0,
                    row_index: 1,
                    template_row: None,
                },
            ],
        );
        let ids = lg_ids(&inserted);
        assert!(!ids.is_empty());
        let decided = edit(&inserted, vec![decide(accept, ids)]);
        assert_eq!(table_shape(&decided), Some((if accept { 3 } else { 2 }, 2)));
        let deleted = edit(
            &input,
            vec![
                tracking(),
                HeadlessCommand::DeleteTableRow {
                    table_order_index: 0,
                    row_index: 0,
                },
            ],
        );
        assert_eq!(
            table_shape(&deleted),
            Some((2, 2)),
            "tracked deletion retains source row"
        );
        let ids = lg_ids(&deleted);
        assert!(!ids.is_empty());
        let decided = edit(&deleted, vec![decide(accept, ids)]);
        assert_eq!(table_shape(&decided), Some((if accept { 1 } else { 2 }, 2)));
    }
}

#[test]
fn deleting_another_authors_tracked_row_requires_a_revision_decision() {
    let input = edit(
        &fixture("<w:r><w:t>契約</w:t></w:r>"),
        vec![
            HeadlessCommand::InsertTable {
                after_paragraph: 0,
                rows: 2,
                columns: 2,
                column_widths: vec![],
                caret_offset: None,
            },
            HeadlessCommand::StartChangeTracking {
                author: Some("Reviewer".into()),
                initials: None,
                date: Some("2025-01-01T00:00:00Z".into()),
                date_utc: None,
            },
            HeadlessCommand::InsertTableRow {
                table_order_index: 0,
                row_index: 1,
                template_row: None,
            },
        ],
    );
    let error = apply_headless_commands(
        &input,
        &HeadlessEditRequest {
            commands: vec![
                tracking(),
                HeadlessCommand::DeleteTableRow {
                    table_order_index: 0,
                    row_index: 1,
                },
            ],
        },
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("decide the existing row revision"));
    let unchanged = edit(&input, vec![HeadlessCommand::StopChangeTracking]);
    assert_eq!(table_shape(&unchanged), Some((3, 2)));
    let xml = part(&unchanged, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    assert!(document
        .descendants()
        .any(|node| node.has_tag_name((NS, "ins"))
            && node.attribute((NS, "author")) == Some("Reviewer")));
}

fn tracked_rows(bytes: &[u8]) -> Option<Vec<u8>> {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let table = document
        .descendants()
        .find(|node| node.has_tag_name((NS, "tbl")))?;
    Some(
        table
            .children()
            .filter(|node| node.has_tag_name((NS, "tr")))
            .map(|row| {
                let properties = row.children().find(|node| node.has_tag_name((NS, "trPr")));
                if properties
                    .is_some_and(|p| p.children().any(|node| node.has_tag_name((NS, "ins"))))
                {
                    1
                } else if properties
                    .is_some_and(|p| p.children().any(|node| node.has_tag_name((NS, "del"))))
                {
                    2
                } else {
                    0
                }
            })
            .collect(),
    )
}

#[test]
fn tracked_whole_tables_and_last_rows_survive_revision_decisions() {
    let input =
        fixture("<w:ins w:id=\"40\" w:author=\"Reviewer\"><w:r><w:t>契約</w:t></w:r></w:ins>");
    for rows in [1, 3] {
        let inserted = edit(
            &input,
            vec![
                tracking(),
                HeadlessCommand::InsertTable {
                    after_paragraph: 0,
                    rows,
                    columns: 2,
                    column_widths: vec![],
                    caret_offset: None,
                },
            ],
        );
        assert_eq!(tracked_rows(&inserted), Some(vec![1; rows]));
        let rejected = edit(&inserted, vec![decide(false, lg_ids(&inserted))]);
        assert_eq!(table_shape(&rejected), None);
        let accepted = edit(&inserted, vec![decide(true, lg_ids(&inserted))]);
        assert_eq!(tracked_rows(&accepted), Some(vec![0; rows]));
        let deleted = edit(
            &accepted,
            vec![
                tracking(),
                HeadlessCommand::DeleteTable {
                    table_order_index: 0,
                },
            ],
        );
        assert_eq!(tracked_rows(&deleted), Some(vec![2; rows]));
        assert_eq!(
            table_shape(&edit(&deleted, vec![decide(true, lg_ids(&deleted))])),
            None
        );
        assert_eq!(
            tracked_rows(&edit(&deleted, vec![decide(false, lg_ids(&deleted))])),
            Some(vec![0; rows])
        );
        let cancelled = edit(
            &inserted,
            vec![
                tracking(),
                HeadlessCommand::DeleteTable {
                    table_order_index: 0,
                },
            ],
        );
        assert_eq!(table_shape(&cancelled), None);
        assert_eq!(final_text(&cancelled), "契約");
        let xml = part(&cancelled, "word/document.xml");
        let document = roxmltree::Document::parse(&xml).unwrap();
        assert!(document
            .descendants()
            .any(|node| node.has_tag_name((NS, "ins"))
                && node.attribute((NS, "author")) == Some("Reviewer")
                && node.attribute((NS, "id")) == Some("40")));
    }
}

#[test]
fn tracked_table_deletion_preserves_foreign_cell_history_and_nested_tables() {
    let input = fixture(
        r#"<w:r><w:t>before</w:t></w:r></w:p>
        <w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:tc>
        <w:p><w:ins w:id="40" w:author="Reviewer"><w:r><w:t>reviewed</w:t></w:r></w:ins></w:p>
        <w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>nested</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>
        </w:tc></w:tr></w:tbl><w:p><w:r><w:t>after</w:t></w:r>"#,
    );
    let deleted = edit(
        &input,
        vec![
            tracking(),
            HeadlessCommand::DeleteTable {
                table_order_index: 0,
            },
        ],
    );
    assert_eq!(final_text(&deleted), "beforeafter");
    let restored = edit(&deleted, vec![decide(false, lg_ids(&deleted))]);
    assert_eq!(final_text(&restored), "beforereviewednestedafter");
    let xml = part(&restored, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    assert_eq!(
        document
            .descendants()
            .filter(|n| n.has_tag_name((NS, "tbl")))
            .count(),
        2
    );
    assert!(document.descendants().any(|n| n.has_tag_name((NS, "ins"))
        && n.attribute((NS, "author")) == Some("Reviewer")
        && n.attribute((NS, "id")) == Some("40")));
    let accepted = edit(&deleted, vec![decide(true, lg_ids(&deleted))]);
    assert_eq!(table_shape(&accepted), None);
    assert_eq!(final_text(&accepted), "beforeafter");

    // Deciding two nested structural insertions must not leave a zero-row table.
    let inserted = fixture(
        r#"</w:p><w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:trPr><w:ins w:id="41" w:author="LAWGUE"/></w:trPr><w:tc>
        <w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:trPr><w:ins w:id="42" w:author="LAWGUE"/></w:trPr><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p/>
        </w:tc></w:tr></w:tbl><w:p>"#,
    );
    assert_eq!(
        table_shape(&edit(
            &inserted,
            vec![decide(false, vec!["41".into(), "42".into()])]
        )),
        None
    );
}

#[test]
fn cancelling_our_inserted_rows_keeps_other_authors_later_changes() {
    for rows in [1, 2] {
        let foreign = r#"<w:tr><w:trPr><w:ins w:id="41" w:author="LAWGUE"/></w:trPr><w:tc><w:p><w:ins w:id="42" w:author="Reviewer"><w:r><w:t>reviewed</w:t></w:r></w:ins></w:p></w:tc></w:tr>"#;
        let input = fixture(&format!(
            r#"</w:p><w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid>{foreign}{}</w:tbl><w:p>"#,
            if rows == 2 {
                "<w:tr><w:tc><w:p/></w:tc></w:tr>"
            } else {
                ""
            }
        ));
        for command in [
            HeadlessCommand::DeleteTable {
                table_order_index: 0,
            },
            HeadlessCommand::DeleteTableRow {
                table_order_index: 0,
                row_index: 0,
            },
        ] {
            let error = apply_headless_commands(
                &input,
                &HeadlessEditRequest {
                    commands: vec![tracking(), command],
                },
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("other author's revisions"),
                "{error}"
            );
        }
        assert_eq!(final_text(&input), "reviewed");
        assert_eq!(
            tracked_rows(&input),
            Some(if rows == 1 { vec![1] } else { vec![1, 0] })
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn persisted_tracked_tables_match_row_model(actions in prop::collection::vec((0u8..7, any::<u8>()), 1..25)) {
        let mut bytes = fixture("<w:ins w:id=\"40\" w:author=\"Reviewer\"><w:r><w:t>契約</w:t></w:r></w:ins>");
        let mut model: Option<(Vec<u8>, usize)> = None;
        for (step, (kind, index)) in actions.into_iter().enumerate() {
            let mut expected = model.clone();
            let mut invalid = false;
            let command = match (kind, expected.as_mut()) {
                (0, None) => {
                    let rows = 1 + index as usize % 3;
                    let columns = 1 + index as usize % 2;
                    expected = Some((vec![1; rows], columns));
                    HeadlessCommand::InsertTable { after_paragraph: 0, rows, columns, column_widths: vec![], caret_offset: None }
                },
                (1, Some((rows, _))) => {
                    rows.retain(|state| *state != 1);
                    for state in rows { if *state == 0 { *state = 2; } }
                    HeadlessCommand::DeleteTable { table_order_index: 0 }
                },
                (2, Some((rows, _))) => {
                    let row = index as usize % (rows.len() + 1);
                    rows.insert(row, 1);
                    HeadlessCommand::InsertTableRow { table_order_index: 0, row_index: row, template_row: None }
                },
                (3, Some((rows, _))) => {
                    let row = index as usize % rows.len();
                    if rows[row] == 1 { rows.remove(row); } else { rows[row] = 2; }
                    HeadlessCommand::DeleteTableRow { table_order_index: 0, row_index: row }
                },
                (4 | 5, state) => {
                    let accept = kind == 4;
                    if let Some((rows, _)) = state {
                        rows.retain(|state| *state != if accept { 2 } else { 1 });
                        rows.fill(0);
                    }
                    decide(accept, lg_ids(&bytes))
                },
                _ => { invalid = true; HeadlessCommand::DeleteTable { table_order_index: 99 } },
            };
            if expected.as_ref().is_some_and(|(rows, _)| rows.is_empty()) { expected = None; }
            let output = apply_headless_commands(&bytes, &HeadlessEditRequest { commands: vec![tracking(), command] });
            if invalid { prop_assert!(output.is_err(), "step {} invalid table", step); }
            else { prop_assert!(output.is_ok(), "step {} kind {}: {:?}", step, kind, output.as_ref().err()); bytes = output.unwrap(); model = expected; }
            prop_assert_eq!(tracked_rows(&bytes), model.as_ref().map(|(rows, _)| rows.clone()), "row states at step {} kind {}", step, kind);
            prop_assert_eq!(table_shape(&bytes), model.as_ref().map(|(rows, columns)| (rows.len(), *columns)), "grid at step {}", step);
            prop_assert_eq!(final_text(&bytes), "契約", "body text at step {}", step);
            let xml = part(&bytes, "word/document.xml");
            let document = roxmltree::Document::parse(&xml).unwrap();
            prop_assert!(document.descendants().any(|node| node.has_tag_name((NS, "ins")) && node.attribute((NS, "author")) == Some("Reviewer") && node.attribute((NS, "id")) == Some("40")), "foreign revision at step {}", step);
        }
    }
}

#[derive(Clone, Debug)]
enum Action {
    Replace {
        from: u8,
        to: u8,
        text: String,
        tracked: bool,
        accept: bool,
    },
    InvalidBoundary,
    NoopDecision(bool),
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        8 => (any::<u8>(), any::<u8>(), prop::collection::vec(prop_oneof![Just('日'), Just('😀'), Just('é'), Just('A')], 0..5), any::<bool>(), any::<bool>()).prop_map(|(from,to,text,tracked,accept)| Action::Replace { from,to,text:text.into_iter().collect(),tracked,accept }),
        1 => Just(Action::InvalidBoundary),
        1 => any::<bool>().prop_map(Action::NoopDecision),
    ]
}

fn table_shape(bytes: &[u8]) -> Option<(usize, usize)> {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let table = document
        .descendants()
        .find(|n| n.has_tag_name((NS, "tbl")))?;
    let rows: Vec<_> = table
        .children()
        .filter(|n| n.has_tag_name((NS, "tr")))
        .collect();
    let columns = rows
        .first()?
        .children()
        .filter(|n| n.has_tag_name((NS, "tc")))
        .count();
    assert!(rows
        .iter()
        .all(|r| r.children().filter(|n| n.has_tag_name((NS, "tc"))).count() == columns));
    Some((rows.len(), columns))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn persisted_table_structure_matches_grid_model(actions in prop::collection::vec((0u8..6, any::<u8>()), 1..31)) {
        let mut bytes = fixture("<w:r><w:t>契約</w:t></w:r>");
        let mut model: Option<(usize,usize)> = None;
        for (step,(kind,index)) in actions.into_iter().enumerate() {
            let mut expected = model;
            let command = match (kind,model) {
                (0,None) => {
                    expected = Some((2,2));
                    HeadlessCommand::InsertTable { after_paragraph:0, rows:2, columns:2, column_widths:vec![], caret_offset:None }
                }
                (0,Some(_)) => { expected=None; HeadlessCommand::DeleteTable { table_order_index:0 } }
                (1,Some((r,c))) => {
                    expected=Some((r+1,c));
                    HeadlessCommand::InsertTableRow { table_order_index:0, row_index:index as usize % (r+1), template_row:None }
                }
                (2,Some((r,c))) if r>1 => {
                    expected=Some((r-1,c));
                    HeadlessCommand::DeleteTableRow { table_order_index:0, row_index:index as usize % r }
                }
                (3,Some((r,c))) => {
                    expected=Some((r,c+1));
                    HeadlessCommand::InsertTableColumn { table_order_index:0, column_index:index as usize % (c+1), width:None, template_column:None }
                }
                (4,Some((r,c))) if c>1 => {
                    expected=Some((r,c-1));
                    HeadlessCommand::DeleteTableColumn { table_order_index:0, column_index:index as usize % c }
                }
                _ => HeadlessCommand::DeleteTableRow { table_order_index:99, row_index:0 },
            };
            let invalid = matches!(command, HeadlessCommand::DeleteTableRow { table_order_index:99, .. });
            let output = apply_headless_commands(&bytes,&HeadlessEditRequest {commands:vec![command]});
            if invalid { prop_assert!(output.is_err(), "step {} invalid table must fail", step); }
            else { bytes=output.unwrap(); model=expected; }
            prop_assert_eq!(table_shape(&bytes), model, "persisted grid at step {}", step);
            prop_assert_eq!(final_text(&bytes), "契約", "body text at step {}", step);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn persisted_revision_workflows_match_string_model(actions in prop::collection::vec(action(), 1..41)) {
        // The protected insertion shares the editable paragraph, exposing accidental
        // adoption/flattening of another author's revision at run boundaries.
        let mut bytes = fixture(r#"<w:ins w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>R</w:t></w:r></w:ins><w:r><w:t>契約😀éA</w:t></w:r>"#);
        let mut model = "契約😀éA".to_string();
        for (step, action) in actions.into_iter().enumerate() {
            match action {
                Action::Replace { from, to, text, tracked, accept } => {
                    let boundaries: Vec<usize> = model.char_indices().map(|(i,_)| i).chain(std::iter::once(model.len())).collect();
                    let a = boundaries[from as usize % boundaries.len()];
                    let b = boundaries[to as usize % boundaries.len()];
                    let (a,b) = (a.min(b), a.max(b));
                    let before = model.clone();
                    let mut after = model.clone();
                    after.replace_range(a..b, &text);
                    let mut commands = vec![if tracked { tracking() } else { HeadlessCommand::StopChangeTracking }];
                    commands.push(HeadlessCommand::ReplaceRange { start_paragraph: 0, start_offset: a+1, end_paragraph: 0, end_offset: b+1, text });
                    bytes = edit(&bytes, commands);
                    prop_assert_eq!(final_text(&bytes), format!("R{after}"), "step {} before decision", step);
                    assert_other_author(&bytes);
                    if tracked {
                        bytes = edit(&bytes, vec![decide(accept, lg_ids(&bytes))]);
                        model = if accept { after } else { before };
                    } else { model = after; }
                }
                Action::InvalidBoundary => {
                    // Byte 2 bisects a Japanese character after a stable ASCII prefix.
                    let offset = model.char_indices().find(|(_,c)| c.len_utf8()>1).map(|(i,_)| i+2);
                    if let Some(offset) = offset {
                        let result = apply_headless_commands(&bytes, &HeadlessEditRequest { commands: vec![HeadlessCommand::InsertText { paragraph: 0, offset, text: "bad".into() }] });
                        prop_assert!(result.is_err(), "step {} must reject a non-UTF8 boundary", step);
                    }
                }
                Action::NoopDecision(accept) => { bytes = edit(&bytes, vec![decide(accept, vec![])]); }
            }
            prop_assert_eq!(final_text(&bytes), format!("R{model}"), "persisted step {}", step);
            assert_other_author(&bytes);
            prop_assert!(lg_ids(&bytes).is_empty(), "each decision removes only our revisions");
        }
    }
}

fn comment(query: &str, occurrence: usize) -> HeadlessCommand {
    HeadlessCommand::AddCommentByText {
        query: query.into(),
        occurrence: Some(occurrence),
        text: "確認".into(),
        author: Some("LAWGUE".into()),
        initials: Some("LG".into()),
        date: Some("2026-10-24T00:00:00Z".into()),
    }
}

fn comment_anchors(bytes: &[u8]) -> Vec<String> {
    let xml = part(bytes, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let mut anchors = std::collections::BTreeMap::<u64, String>::new();
    let mut active = std::collections::BTreeSet::new();
    for node in document.descendants() {
        if node.has_tag_name((NS, "commentRangeStart")) {
            let id = node.attribute((NS, "id")).unwrap().parse::<u64>().unwrap();
            anchors.entry(id).or_default();
            active.insert(id);
        } else if node.has_tag_name((NS, "commentRangeEnd")) {
            active.remove(&node.attribute((NS, "id")).unwrap().parse::<u64>().unwrap());
        } else if node.has_tag_name((NS, "t"))
            && !node
                .ancestors()
                .any(|a| a.has_tag_name((NS, "del")) || a.has_tag_name((NS, "moveFrom")))
        {
            for id in &active {
                anchors
                    .get_mut(id)
                    .unwrap()
                    .push_str(node.text().unwrap_or(""));
            }
        }
    }
    anchors.into_values().collect()
}

#[test]
fn comments_search_final_text_and_preserve_foreign_revisions() {
    let input = fixture(
        r#"<w:del w:id="40" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:delText>対象</w:delText></w:r></w:del><w:r><w:t>前</w:t></w:r><w:del w:id="42" w:author="Reviewer"><w:r><w:delText>旧</w:delText></w:r></w:del><w:r><w:t>後😀対象</w:t></w:r><w:ins w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>R</w:t></w:r></w:ins>"#,
    );
    let output = edit(
        &input,
        vec![comment("対象", 0), comment("前後😀", 0), comment("R", 0)],
    );
    assert_eq!(comment_anchors(&output), vec!["対象", "前後😀", "R"]);
    assert_eq!(final_text(&output), "前後😀対象R");
    assert_other_author(&output);
    let xml = part(&output, "word/document.xml");
    assert!(xml.contains("旧"));
    assert!(apply_headless_commands(
        &output,
        &HeadlessEditRequest {
            commands: vec![comment("旧", 0)]
        }
    )
    .is_err());
    assert!(apply_headless_commands(
        &output,
        &HeadlessEditRequest {
            commands: vec![comment("対象", 1)]
        }
    )
    .is_err());
}

#[test]
fn full_paragraph_replacement_tracks_only_changed_middle() {
    let input = fixture(
        r#"<w:r><w:rPr><w:b/></w:rPr><w:t>契約😀</w:t></w:r><w:r><w:rPr><w:u w:val="double"/></w:rPr><w:t>30</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>日é</w:t></w:r>"#,
    );
    let output = edit(
        &input,
        vec![
            tracking(),
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: "契約😀30日é".len(),
                text: "契約😀60日é".into(),
            },
        ],
    );
    let xml = part(&output, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let changed: Vec<_> = document
        .descendants()
        .filter(|n| n.has_tag_name((NS, "t")) || n.has_tag_name((NS, "delText")))
        .filter(|n| {
            n.ancestors()
                .any(|a| a.attribute((NS, "author")) == Some("LAWGUE"))
        })
        .filter_map(|n| n.text())
        .collect();
    assert_eq!(changed, vec!["3", "6"]);
    assert_eq!(final_text(&output), "契約😀60日é");
    assert!(xml.contains("w:b"));
    assert!(xml.contains("w:i"));
    assert!(xml.contains("double"));
    assert_eq!(
        final_text(&edit(&output, vec![decide(false, lg_ids(&output))])),
        "契約😀30日é"
    );
    assert_eq!(
        final_text(&edit(&output, vec![decide(true, lg_ids(&output))])),
        "契約😀60日é"
    );
    let noop = edit(
        &input,
        vec![
            tracking(),
            HeadlessCommand::ReplaceRange {
                start_paragraph: 0,
                start_offset: 0,
                end_paragraph: 0,
                end_offset: "契約😀30日é".len(),
                text: "契約😀30日é".into(),
            },
        ],
    );
    assert!(lg_ids(&noop).is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn persisted_comments_and_tracked_edits_match_model(actions in prop::collection::vec((0u8..5, any::<bool>(), "[a-z😀é]{0,3}"), 1..25)) {
        let mut bytes = fixture(r#"<w:del w:id="40" w:author="Reviewer"><w:r><w:delText>対象</w:delText></w:r></w:del><w:ins w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>R</w:t></w:r></w:ins><w:r><w:t>対象😀</w:t></w:r>"#);
        let mut model = "R対象😀".to_string();
        let mut expected_anchors = Vec::<String>::new();
        for (step, (action, accept, text)) in actions.into_iter().enumerate() {
            match action {
                0 | 1 => {
                    let query = if action == 0 { "対象" } else { "R" };
                    bytes = edit(&bytes, vec![comment(query, 0)]);
                    expected_anchors.push(query.into());
                }
                2 => {
                    let before = model.clone();
                    // The independent string model supplies the changed suffix; the
                    // public request deliberately replaces the complete paragraph.
                    let after = format!("{model}{text}");
                    let original_len = {
                        let xml = part(&bytes, "word/document.xml");
                        let doc = roxmltree::Document::parse(&xml).unwrap();
                        doc.descendants().filter(|n| n.has_tag_name((NS,"t")) || n.has_tag_name((NS,"delText"))).filter_map(|n|n.text()).map(str::len).sum()
                    };
                    // Preserve the pre-existing deletion outside the requested range.
                    bytes = edit(&bytes, vec![tracking(), HeadlessCommand::ReplaceRange {start_paragraph:0, start_offset:"対象".len(), end_paragraph:0, end_offset:original_len, text:after.clone()}]);
                    prop_assert_eq!(final_text(&bytes), after.clone(), "step {} before decision", step);
                    bytes = edit(&bytes, vec![decide(accept, lg_ids(&bytes))]);
                    model = if accept { after } else { before };
                }
                3 => {
                    let result = apply_headless_commands(&bytes, &HeadlessEditRequest {commands:vec![comment("存在しない",0)]});
                    prop_assert!(result.is_err(), "step {} absent text must fail", step);
                }
                _ => {
                    let result = apply_headless_commands(&bytes, &HeadlessEditRequest {commands:vec![comment("対象",1)]});
                    prop_assert!(result.is_err(), "step {} deleted occurrence must not count", step);
                }
            }
            prop_assert_eq!(final_text(&bytes), model.clone(), "persisted text step {}", step);
            prop_assert_eq!(comment_anchors(&bytes), expected_anchors.clone(), "persisted anchors step {}", step);
            assert_other_author(&bytes);
        }
    }
}

#[test]
fn comments_skip_moved_text_and_nested_deleted_rows() {
    let input = fixture(
        r#"<w:moveFrom w:id="44" w:author="Reviewer"><w:r><w:t>対象</w:t></w:r></w:moveFrom></w:p>
        <w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid>
        <w:tr><w:trPr><w:del w:id="45" w:author="Reviewer"/></w:trPr><w:tc>
        <w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>対象</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>
        </w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>対象</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p>"#,
    );
    let output = edit(&input, vec![comment("対象", 0)]);
    assert_eq!(comment_anchors(&output), vec!["対象"]);
    let xml = part(&output, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let anchor = document
        .descendants()
        .find(|n| n.has_tag_name((NS, "commentRangeStart")))
        .unwrap();
    let row = anchor
        .ancestors()
        .find(|n| n.has_tag_name((NS, "tr")))
        .unwrap();
    assert!(
        !row.descendants().any(|n| n.has_tag_name((NS, "del"))),
        "anchor must be in the visible row: {xml}"
    );
    assert!(apply_headless_commands(
        &output,
        &HeadlessEditRequest {
            commands: vec![comment("対象", 1)]
        }
    )
    .is_err());
}

#[test]
fn whole_table_revision_ids_remain_stable_after_reload() {
    let base = fixture("<w:r><w:t>契約</w:t></w:r>");
    let inserted = edit(
        &base,
        vec![
            tracking(),
            HeadlessCommand::InsertTable {
                after_paragraph: 0,
                rows: 3,
                columns: 2,
                column_widths: vec![],
                caret_offset: None,
            },
        ],
    );
    let accepted = edit(&inserted, vec![decide(true, lg_ids(&inserted))]);
    let deleted = edit(
        &accepted,
        vec![
            tracking(),
            HeadlessCommand::DeleteTable {
                table_order_index: 0,
            },
        ],
    );
    for bytes in [&inserted, &deleted] {
        let ids = lg_ids(bytes);
        assert_eq!(ids.len(), 3);
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            3,
            "each row revision needs its own ID"
        );
        assert_eq!(
            lg_ids(&edit(bytes, vec![HeadlessCommand::StopChangeTracking])),
            ids,
            "reopening must not renumber IDs"
        );
        // A saved revision ID must still target exactly that row after reopening.
        let decided = edit(bytes, vec![decide(false, vec![ids[1].clone()])]);
        let remaining = lg_ids(&decided);
        assert_eq!(remaining.len(), 2);
        assert!(!remaining.contains(&ids[1]));
    }
}
