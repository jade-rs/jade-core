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

#[test]
fn tracked_table_range_keeps_heading_boundary_reviewable() {
    let input = fixture(&format!(
        "<w:r><w:t>HEAD</w:t></w:r></w:p>{}<w:p><w:r><w:t>TAIL</w:t></w:r>",
        range_table("AB", "CD")
    ));
    let tracked = edit(&input, vec![tracking(), range_delete(0, 0, 2, 2)]);
    let xml = part(&tracked, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let heading = document
        .descendants()
        .find(|n| n.has_tag_name((NS, "p")))
        .unwrap();
    assert!(
        heading
            .children()
            .filter(|n| n.has_tag_name((NS, "pPr")))
            .flat_map(|n| n.children())
            .filter(|n| n.has_tag_name((NS, "rPr")))
            .flat_map(|n| n.children())
            .any(|n| n.has_tag_name((NS, "del"))),
        "the selected heading's paragraph mark must be reviewable"
    );
    for accept in [false, true] {
        let decided = edit(&tracked, vec![decide(accept, lg_ids(&tracked))]);
        let xml = part(&decided, "word/document.xml");
        let document = roxmltree::Document::parse(&xml).unwrap();
        let body = document
            .descendants()
            .find(|n| n.has_tag_name((NS, "body")))
            .unwrap();
        assert_eq!(
            body.children()
                .filter(|n| n.has_tag_name((NS, "p")))
                .count(),
            if accept { 1 } else { 2 }
        );
        assert_eq!(
            final_text(&decided),
            if accept { "TAIL" } else { "HEADABCDTAIL" }
        );
    }
}

#[test]
fn tracked_heading_table_decisions_can_be_reviewed_separately() {
    let input = fixture(&format!(
        "<w:r><w:t>HEAD</w:t></w:r></w:p>{}<w:p><w:r><w:t>TAIL</w:t></w:r>",
        range_table("AB", "CD")
    ));
    let tracked = edit(&input, vec![tracking(), range_delete(0, 2, 2, 2)]);
    let xml = part(&tracked, "word/document.xml");
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let row_ids: Vec<_> = doc
        .descendants()
        .filter(|n| {
            n.has_tag_name((NS, "del")) && n.parent().is_some_and(|p| p.has_tag_name((NS, "trPr")))
        })
        .filter_map(|n| n.attribute((NS, "id")).map(str::to_owned))
        .collect();
    let heading_ids: Vec<_> = lg_ids(&tracked)
        .into_iter()
        .filter(|id| !row_ids.contains(id))
        .collect();
    // Joining the heading while its table survives would lose the grid.
    assert!(apply_headless_commands(
        &tracked,
        &HeadlessEditRequest {
            commands: vec![decide(true, heading_ids.clone())]
        }
    )
    .is_err());
    for accept in [false, true] {
        let rows_first = edit(&tracked, vec![decide(accept, row_ids.clone())]);
        let decided = edit(&rows_first, vec![decide(accept, heading_ids.clone())]);
        assert_eq!(body_paragraph_count(&decided), if accept { 1 } else { 2 });
        assert_eq!(
            final_text(&decided),
            if accept { "HETAIL" } else { "HEADABCDTAIL" }
        );
    }
    let heading_first = edit(&tracked, vec![decide(false, heading_ids)]);
    let decided = edit(&heading_first, vec![decide(false, row_ids)]);
    assert_eq!(body_paragraph_count(&decided), 2);
    assert_eq!(final_text(&decided), "HEADABCDTAIL");
}

#[test]
fn cancelling_our_inserted_table_keeps_the_heading_break_reviewable() {
    let table = range_table("AB", "CD").replace(
        "<w:tr>",
        r#"<w:tr><w:trPr><w:ins w:id="41" w:author="LAWGUE"/></w:trPr>"#,
    );
    let input = fixture(&format!(
        "<w:r><w:t>HEAD</w:t></w:r></w:p>{table}<w:p><w:r><w:t>TAIL</w:t></w:r>"
    ));
    let deleted = edit(&input, vec![tracking(), range_delete(0, 0, 2, 2)]);
    assert_eq!(table_shape(&deleted), None);
    assert_eq!(body_paragraph_count(&deleted), 2);
    let accepted = edit(&deleted, vec![decide(true, lg_ids(&deleted))]);
    assert_eq!(body_paragraph_count(&accepted), 1);
    assert_eq!(final_text(&accepted), "TAIL");
    let rejected = edit(&deleted, vec![decide(false, lg_ids(&deleted))]);
    assert_eq!(body_paragraph_count(&rejected), 2);
    assert_eq!(final_text(&rejected), "HEADTAIL");
}

#[test]
fn heading_table_review_and_delete_undo_redo_restore_the_package() {
    use jade_docx_document_core::{
        commands::{CommandContext, CommandExecutor},
        headless_api::apply_headless_commands_with_executor_to_document,
        word_document::WordDocument,
        Options,
    };
    let input = fixture(&format!(
        "<w:r><w:t>HEAD</w:t></w:r></w:p>{}<w:p><w:r><w:t>TAIL</w:t></w:r>",
        range_table("AB", "CD")
    ));
    for accept in [false, true] {
        let mut document =
            futures::executor::block_on(WordDocument::load(&input, &Options::default())).unwrap();
        let mut executor = CommandExecutor::new();
        let before = part(&document.export_docx().unwrap(), "word/document.xml");
        let mut selection = apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            None,
            &HeadlessEditRequest {
                commands: vec![tracking(), range_delete(0, 0, 2, 2)],
            },
        )
        .unwrap();
        let tracked = document.export_docx().unwrap();
        let tracked_xml = part(&tracked, "word/document.xml");
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).unwrap());
        }
        assert_eq!(
            part(&document.export_docx().unwrap(), "word/document.xml"),
            before
        );
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.redo(&mut ctx).unwrap());
        }
        assert_eq!(
            part(&document.export_docx().unwrap(), "word/document.xml"),
            tracked_xml
        );
        // A refused selective decision must leave the live document untouched.
        let xml = part(&tracked, "word/document.xml");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let heading_mark = doc
            .descendants()
            .find(|n| {
                n.has_tag_name((NS, "del"))
                    && n.parent().is_some_and(|p| p.has_tag_name((NS, "rPr")))
            })
            .unwrap()
            .attribute((NS, "id"))
            .unwrap()
            .to_owned();
        assert!(document
            .accept_revisions(None, None, None, Some(&heading_mark))
            .is_err());
        assert_eq!(
            part(&document.export_docx().unwrap(), "word/document.xml"),
            tracked_xml
        );
        selection = apply_headless_commands_with_executor_to_document(
            &mut document,
            &mut executor,
            selection,
            &HeadlessEditRequest {
                commands: vec![decide(accept, lg_ids(&tracked))],
            },
        )
        .unwrap();
        let decided = part(&document.export_docx().unwrap(), "word/document.xml");
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.undo(&mut ctx).unwrap());
        }
        assert_eq!(
            part(&document.export_docx().unwrap(), "word/document.xml"),
            tracked_xml
        );
        {
            let mut ctx = CommandContext::new(&mut document, &mut selection);
            assert!(executor.redo(&mut ctx).unwrap());
        }
        assert_eq!(
            part(&document.export_docx().unwrap(), "word/document.xml"),
            decided
        );
    }
}

#[derive(Clone, Debug, PartialEq)]
enum HeadingTableBlock {
    Paragraph {
        runs: Vec<(String, bool)>,
        deleted_break: bool,
    },
    Table {
        deleted: bool,
    },
}

fn normalize_heading_runs(runs: Vec<(String, bool)>) -> Vec<(String, bool)> {
    let mut result: Vec<(String, bool)> = Vec::new();
    for (text, deleted) in runs {
        if text.is_empty() {
            continue;
        }
        if let Some(last) = result.last_mut().filter(|last| last.1 == deleted) {
            last.0.push_str(&text);
        } else {
            result.push((text, deleted));
        }
    }
    result
}

// The model consists only of ordered body blocks, text chunks, and pending
// deletions. It does not call document/model helpers. Each step exports a DOCX
// and the next public operation reloads that package, including selective
// decisions, dependency errors, empty selections, and invalid coordinates.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn persisted_heading_table_decisions_match_block_model(
        headings in prop::collection::vec("[a-z甲乙😀]{1,8}", 1..5),
        cut in any::<usize>(),
        actions in prop::collection::vec((0u8..7, any::<bool>(), any::<usize>()), 1..33),
    ) {
        let mut source = String::new();
        let mut model = Vec::new();
        let offsets: Vec<_> = headings[0].char_indices().map(|(i, _)| i).chain(std::iter::once(headings[0].len())).collect();
        let offset = offsets[cut % offsets.len()];
        for (index, heading) in headings.iter().enumerate() {
            source.push_str(&format!("<w:r><w:t>{heading}</w:t></w:r></w:p>{}<w:p>", range_table("AB", "CD")));
            let from = if index == 0 { offset } else { 0 };
            model.push(HeadingTableBlock::Paragraph {
                runs: normalize_heading_runs(vec![(heading[..from].into(), false), (heading[from..].into(), true)]),
                deleted_break: true,
            });
            model.push(HeadingTableBlock::Table { deleted: true });
        }
        source.push_str(r#"<w:r><w:t>TAIL</w:t></w:r><w:ins w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>R</w:t></w:r></w:ins>"#);
        model.push(HeadingTableBlock::Paragraph { runs: vec![("TAILR".into(), false)], deleted_break: false });
        let input = fixture(&source);
        let mut bytes = edit(&input, vec![tracking(), range_delete(0, offset, headings.len()*3-1, 2)]);
        for (step, (scope, accept, index)) in actions.into_iter().enumerate() {
            let xml = part(&bytes, "word/document.xml");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let body = doc.descendants().find(|n| n.has_tag_name((NS, "body"))).unwrap();
            let blocks: Vec<_> = body.children().filter(|n| n.has_tag_name((NS, "p")) || n.has_tag_name((NS, "tbl"))).collect();
            let chosen = index % model.len();
            // (row deletion, paragraph break deletion, text deletion)
            let selected: Vec<_> = (0..model.len()).map(|i| (
                scope == 0 || scope == 1 || (scope == 4 && chosen == i),
                scope == 0 || scope == 2 || (scope == 4 && chosen == i),
                scope == 0 || scope == 3 || (scope == 4 && chosen == i),
            )).collect();
            let ids: Vec<_> = blocks.iter().enumerate().flat_map(|(i, block)| {
                let flags = selected[i];
                block.descendants().filter(move |n| {
                    if !n.has_tag_name((NS, "del")) || n.attribute((NS, "author")) != Some("LAWGUE") { return false; }
                    let parent = n.parent().unwrap();
                    if parent.has_tag_name((NS, "trPr")) { flags.0 }
                    else if parent.has_tag_name((NS, "rPr")) && parent.parent().is_some_and(|p| p.has_tag_name((NS, "pPr"))) { flags.1 }
                    else { flags.2 }
                }).filter_map(|n| n.attribute((NS, "id")).map(str::to_owned))
            }).collect();
            let mut expected = model.clone();
            let mut invalid = scope == 6;
            // A paragraph boundary can join only when every intervening table
            // is physically removed in this same decision (or already gone).
            if accept {
                for i in 0..model.len() {
                    if matches!(model[i], HeadingTableBlock::Paragraph { deleted_break: true, .. }) && selected[i].1 {
                        let mut next = i+1;
                        while next < model.len() && matches!(model[next], HeadingTableBlock::Table { deleted: true }) && selected[next].0 { next += 1; }
                        if !matches!(model.get(next), Some(HeadingTableBlock::Paragraph { .. })) { invalid = true; }
                    }
                }
            }
            let mut remove = Vec::new();
            for (i, block) in expected.iter_mut().enumerate() {
                match block {
                    HeadingTableBlock::Table { deleted } if *deleted && selected[i].0 => {
                        if accept { remove.push(i); } else { *deleted = false; }
                    }
                    HeadingTableBlock::Paragraph { runs, deleted_break } => {
                        if selected[i].2 {
                            *runs = normalize_heading_runs(runs.iter().filter(|(_, del)| !accept || !del).map(|(text, _)| (text.clone(), false)).collect());
                        }
                        if *deleted_break && selected[i].1 { *deleted_break = false; }
                    }
                    _ => {},
                }
            }
            if !invalid {
                // Join backwards before removing table placeholders so that
                // selected block indices are independent of engine IDs.
                for i in (0..model.len()).rev() {
                    if accept && selected[i].1 && matches!(model[i], HeadingTableBlock::Paragraph { deleted_break:true, .. }) {
                        let mut next = i+1;
                        while remove.contains(&next) { next += 1; }
                        let right = expected[next].clone();
                        if let (HeadingTableBlock::Paragraph { runs, deleted_break }, HeadingTableBlock::Paragraph { runs:right_runs, deleted_break:right_break }) = (&mut expected[i], right) {
                            runs.extend(right_runs);
                            *runs = normalize_heading_runs(std::mem::take(runs));
                            *deleted_break = right_break;
                            remove.push(next);
                        }
                    }
                }
                remove.sort_unstable(); remove.dedup();
                for i in remove.into_iter().rev() { expected.remove(i); }
            }
            let command = if scope == 6 { range_delete(999, 0, 999, 0) } else { decide(accept, ids) };
            let result = apply_headless_commands(&bytes, &HeadlessEditRequest { commands: vec![command] });
            if invalid { prop_assert!(result.is_err(), "invalid step {} scope {} accept {}", step, scope, accept); }
            else { prop_assert!(result.is_ok(), "step {} scope {} accept {}: {:?}", step, scope, accept, result.as_ref().err()); bytes = result.unwrap(); model = expected; }
            let xml = part(&bytes, "word/document.xml");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let body = doc.descendants().find(|n| n.has_tag_name((NS, "body"))).unwrap();
            let actual: Vec<_> = body.children().filter_map(|block| {
                if block.has_tag_name((NS, "tbl")) {
                    Some(HeadingTableBlock::Table { deleted: block.descendants().any(|n| n.has_tag_name((NS, "del")) && n.parent().is_some_and(|p| p.has_tag_name((NS, "trPr")))) })
                } else if block.has_tag_name((NS, "p")) {
                    let runs = block.descendants().filter(|n| n.has_tag_name((NS, "t")) || n.has_tag_name((NS, "delText")))
                        .filter_map(|n| n.text().map(|text| (text.to_owned(), n.ancestors().any(|a| a.has_tag_name((NS, "del")))))).collect();
                    let deleted_break = block.children().filter(|n| n.has_tag_name((NS, "pPr"))).flat_map(|n| n.descendants()).any(|n| n.has_tag_name((NS, "del")));
                    Some(HeadingTableBlock::Paragraph { runs: normalize_heading_runs(runs), deleted_break })
                } else { None }
            }).collect();
            prop_assert_eq!(&actual, &model, "saved block model at step {} scope {} accept {}", step, scope, accept);
            assert_other_author(&bytes);
        }
    }
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
fn whole_table_ranges_ignore_only_deleted_text_at_cell_edges() {
    for tag in ["del", "moveFrom", "ins", "moveTo"] {
        let hidden = matches!(tag, "del" | "moveFrom");
        let text_tag = if tag == "del" { "delText" } else { "t" };
        let edge = format!(
            r#"</w:t></w:r><w:{tag} w:id="40" w:author="Reviewer"><w:r><w:{text_tag}>旧</w:{text_tag}></w:r></w:{tag}><w:r><w:t>"#
        );
        let table = range_table(&format!("{edge}AB"), &format!("CD{edge}"));
        let input = fixture(&format!(
            "<w:r><w:t>HEAD</w:t></w:r></w:p>{table}<w:p><w:r><w:t>TAIL</w:t></w:r>"
        ));
        for tracked in [false, true] {
            let mut commands = vec![];
            if tracked {
                commands.push(tracking());
            }
            commands.push(range_delete(1, "旧".len(), 2, 2));
            let output = edit(&input, commands);
            if hidden {
                assert_eq!(table_shape(&output), tracked.then_some((1, 2)), "{tag}");
                assert_eq!(final_text(&output), "HEADTAIL", "{tag}");
                if tracked {
                    assert_eq!(tracked_rows(&output), Some(vec![2]));
                    let rejected = edit(&output, vec![decide(false, lg_ids(&output))]);
                    assert_eq!(final_text(&rejected), final_text(&input));
                    let accepted = edit(&output, vec![decide(true, lg_ids(&output))]);
                    assert_eq!(table_shape(&accepted), None);
                }
            } else {
                assert_eq!(table_shape(&output), Some((1, 2)), "{tag}");
                assert!(
                    final_text(&output).contains('旧'),
                    "visible edge must keep grid"
                );
            }
        }
    }
}

// Independent visible-grid model with foreign deleted/moved text at the two
// selection boundaries. Each operation exports, then the next reloads it.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn persisted_hidden_edge_table_ranges_match_model(
        prefix in 1usize..5,
        suffix in 1usize..5,
        moved in any::<bool>(),
        actions in prop::collection::vec((0u8..6, any::<bool>()), 1..25),
    ) {
        let prefix = "旧😀".repeat(prefix);
        let suffix = "移動前".repeat(suffix);
        let tag = if moved { "moveFrom" } else { "del" };
        let text_tag = if moved { "t" } else { "delText" };
        let edge = |id, text: &str| format!(r#"</w:t></w:r><w:{tag} w:id="{id}" w:author="Reviewer" w:date="2026-10-23T00:00:00Z"><w:r><w:{text_tag}>{text}</w:{text_tag}></w:r></w:{tag}><w:r><w:t>"#);
        let table = range_table(&format!("{}甲AB", edge(40, &prefix)), &format!("乙CD{}", edge(41, &suffix)));
        let mut bytes = fixture(&format!("<w:r><w:t>HEAD</w:t></w:r></w:p>{table}<w:p><w:r><w:t>TAIL</w:t></w:r>"));
        let mut exists = true;
        let mut deleted = false;
        for (step, (kind, tracked)) in actions.into_iter().enumerate() {
            let invalid = kind == 4 || (kind <= 1 && !exists);
            let command = match kind {
                0 | 1 if exists => range_delete(1, prefix.len(), 2, "乙CD".len()),
                2 | 3 => decide(kind == 2, lg_ids(&bytes)),
                5 if exists => range_delete(1, prefix.len(), 1, prefix.len()),
                5 => range_delete(0, 0, 0, 0),
                _ => range_delete(99, 0, 99, 0),
            };
            let output = apply_headless_commands(&bytes, &HeadlessEditRequest { commands: vec![if tracked { tracking() } else { HeadlessCommand::StopChangeTracking }, command] });
            if invalid {
                prop_assert!(output.is_err(), "invalid step {}", step);
            } else {
                prop_assert!(output.is_ok(), "step {} kind {}: {:?}", step, kind, output.as_ref().err());
                bytes = output.unwrap();
                match kind {
                    0 | 1 if tracked => deleted = true,
                    0 | 1 => { exists = false; deleted = false; },
                    2 if deleted => { exists = false; deleted = false; },
                    3 => deleted = false,
                    _ => {},
                }
            }
            prop_assert_eq!(table_shape(&bytes), exists.then_some((1, 2)), "grid at step {}", step);
            prop_assert_eq!(tracked_rows(&bytes), exists.then_some(vec![if deleted { 2 } else { 0 }]), "row at step {}", step);
            prop_assert_eq!(final_text(&bytes), if exists && !deleted { "HEAD甲AB乙CDTAIL" } else { "HEADTAIL" }, "visible model at step {}", step);
            if exists {
                let xml = part(&bytes, "word/document.xml");
                let doc = roxmltree::Document::parse(&xml).unwrap();
                for (id, text) in [("40", &prefix), ("41", &suffix)] {
                    let mark = doc.descendants().find(|n| n.has_tag_name((NS, tag)) && n.attribute((NS, "id")) == Some(id));
                    prop_assert!(mark.is_some(), "foreign mark {} at step {}", id, step);
                    let mark = mark.unwrap();
                    prop_assert_eq!(mark.attribute((NS, "author")), Some("Reviewer"));
                    prop_assert_eq!(mark.attribute((NS, "date")), Some("2026-10-23T00:00:00Z"));
                    let actual: String = mark.descendants().filter(|n| n.has_tag_name((NS, text_tag))).filter_map(|n| n.text()).collect();
                    prop_assert_eq!(&actual, text, "foreign text at step {}", step);
                }
            }
        }
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

#[test]
fn range_ending_at_an_empty_cell_start_preserves_the_table() {
    let input = fixture(
        r#"<w:del w:id="41" w:author="Reviewer"><w:r><w:delText>Hello</w:delText></w:r></w:del></w:p><w:tbl><w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl><w:p><w:r><w:t>TAIL</w:t></w:r>"#,
    );
    for tracked in [false, true] {
        let mut commands = vec![];
        if tracked {
            commands.push(tracking());
        }
        commands.push(range_delete(0, 0, 1, 0));
        let output = edit(&input, commands);
        assert_eq!(table_shape(&output), Some((1, 1)));
        assert_eq!(final_text(&output), "TAIL");
        assert!(part(&output, "word/document.xml").contains("<w:delText>Hello</w:delText>"));
    }
    // Extending the selection beyond the empty table does select its grid.
    let output = edit(&input, vec![range_delete(0, 0, 2, 0)]);
    assert_eq!(table_shape(&output), None);

    let input = fixture(&format!(
        "<w:r><w:t>HEAD</w:t></w:r></w:p>{}<w:p><w:r><w:t>TAIL</w:t></w:r>",
        range_table("AB", "")
    ));
    let output = edit(&input, vec![range_delete(1, 0, 2, 0)]);
    assert_eq!(table_shape(&output), Some((1, 2)));
    assert_eq!(final_text(&output), "HEADTAIL");
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
                    // To select a table with an empty final cell, extend past
                    // that cell's paragraph boundary into the next paragraph.
                    let end_paragraph = 2 + table * 2 + usize::from(end == 0);
                    range_delete(1 + table * 2, 0, end_paragraph, end)
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
fn plain_join_keeps_the_surviving_paragraph_mark_revision() {
    for revision in ["ins", "del", "moveFrom", "moveTo"] {
        let input = fixture(&format!(
            r#"<w:r><w:t>甲</w:t></w:r></w:p>
            <w:p><w:pPr><w:rPr><w:b/><w:{revision} w:id="71" w:author="Reviewer" w:date="2026-10-23T01:02:03Z"/></w:rPr></w:pPr><w:r><w:t>😀</w:t></w:r></w:p>
            <w:p><w:r><w:t>乙</w:t></w:r>"#
        ));
        let joined = edit(&input, vec![range_delete(0, 3, 1, 0)]);
        assert_eq!(body_paragraph_count(&joined), 2);
        assert_eq!(final_text(&joined), "甲😀乙");
        let xml = part(&joined, "word/document.xml");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let mark = doc
            .descendants()
            .find(|n| n.has_tag_name((NS, revision)))
            .expect("unselected final paragraph mark revision must survive the join");
        assert_eq!(mark.attribute((NS, "id")), Some("71"));
        assert_eq!(mark.attribute((NS, "author")), Some("Reviewer"));
        assert_eq!(mark.attribute((NS, "date")), Some("2026-10-23T01:02:03Z"));
        let paragraph = mark
            .ancestors()
            .find(|n| n.has_tag_name((NS, "p")))
            .unwrap();
        let text: String = paragraph
            .descendants()
            .filter(|n| n.has_tag_name((NS, "t")))
            .filter_map(|n| n.text())
            .collect();
        assert_eq!(text, "甲😀");
    }
}

#[test]
fn cancelling_adjacent_inserted_breaks_keeps_one_surviving_revision() {
    for offset in [0, 3] {
        let input = fixture("<w:r><w:t>甲😀乙</w:t></w:r>");
        let split = edit(
            &input,
            vec![
                tracking(),
                HeadlessCommand::InsertParagraphBreak {
                    paragraph: 0,
                    offset,
                    count: 1,
                },
                HeadlessCommand::InsertParagraphBreak {
                    paragraph: 1,
                    offset: if offset == 0 { 0 } else { 4 },
                    count: 1,
                },
            ],
        );
        assert_eq!(body_paragraph_count(&split), 3);
        assert_eq!(lg_ids(&split).len(), 2);
        let joined = edit(&split, vec![tracking(), range_delete(0, offset, 1, 0)]);
        assert_eq!(body_paragraph_count(&joined), 2);
        assert_eq!(lg_ids(&joined).len(), 1);
        let xml = part(&joined, "word/document.xml");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        assert_eq!(
            doc.descendants()
                .filter(|n| n.has_tag_name((NS, "ins")))
                .count(),
            1
        );
        assert_eq!(final_text(&joined), "甲😀乙");
        for accept in [false, true] {
            let decided = edit(&joined, vec![decide(accept, lg_ids(&joined))]);
            assert_eq!(body_paragraph_count(&decided), if accept { 2 } else { 1 });
            assert!(lg_ids(&decided).is_empty());
            assert_eq!(final_text(&decided), "甲😀乙");
        }
    }
}

// An independent boundary model: a paragraph break is stable, inserted, or
// deleted. Text is never edited in this property, so every split/join must
// preserve its exact Unicode character sequence, including across reloads.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn persisted_paragraph_boundaries_match_model(
        initial in prop::collection::vec(prop::sample::select(vec!['甲', '😀', '乙', 'x']), 1..17),
        actions in prop::collection::vec((0u8..7, any::<usize>(), any::<usize>(), any::<bool>()), 1..33),
    ) {
        let initial: String = initial.into_iter().collect();
        let mut bytes = fixture(&format!("<w:r><w:t>{initial}</w:t></w:r>"));
        let mut paragraphs = vec![initial.clone()];
        // None is stable, true is inserted, false is deleted; last has no mark.
        let mut boundaries: Vec<Option<bool>> = Vec::new();
        for (step, (kind, a, b, tracked)) in actions.into_iter().enumerate() {
            let p = a % paragraphs.len();
            let chars: Vec<_> = paragraphs[p].char_indices().map(|(i, _)| i)
                .chain(std::iter::once(paragraphs[p].len())).collect();
            let offset = chars[b % chars.len()];
            let xml = part(&bytes, "word/document.xml");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let actual_paragraphs: Vec<_> = doc.descendants().filter(|n| n.has_tag_name((NS, "p"))).collect();
            let mark_id = |index: usize| actual_paragraphs[index].children()
                .filter(|n| n.has_tag_name((NS, "pPr")))
                .flat_map(|n| n.descendants())
                .find(|n| n.has_tag_name((NS, "ins")) || n.has_tag_name((NS, "del")))
                .and_then(|n| n.attribute((NS, "id"))).map(str::to_owned);
            let join = a % paragraphs.len().saturating_sub(1).max(1);
            let invalid = kind == 5 || (kind == 1 && (paragraphs.len() == 1 || (tracked && boundaries[join] == Some(false))));
            let command = match kind {
                0 => HeadlessCommand::InsertParagraphBreak { paragraph:p, offset, count:1 },
                1 if paragraphs.len() > 1 => range_delete(join, paragraphs[join].len(), join+1, 0),
                2 | 3 => decide(kind == 2, if p < boundaries.len() { mark_id(p).into_iter().collect() } else { vec![] }),
                4 => decide(tracked, lg_ids(&bytes)),
                6 => range_delete(p, offset, p, offset),
                _ => range_delete(999, 0, 999, 0),
            };
            let result = apply_headless_commands(&bytes, &HeadlessEditRequest { commands: vec![
                if tracked { tracking() } else { HeadlessCommand::StopChangeTracking }, command,
            ] });
            if invalid {
                prop_assert!(result.is_err(), "invalid step {}", step);
            } else {
                prop_assert!(result.is_ok(), "step {} kind {} tracked {}: {:?}", step, kind, tracked, result.as_ref().err());
                bytes = result.unwrap();
                let mut remove = Vec::new();
                match kind {
                    0 => {
                        let right = paragraphs[p].split_off(offset);
                        paragraphs.insert(p+1, right);
                        boundaries.insert(p, tracked.then_some(true));
                    }
                    1 => {
                        if !tracked || boundaries[join] == Some(true) { remove.push(join); }
                        else { boundaries[join] = Some(false); }
                    }
                    2 | 3 if p < boundaries.len() => {
                        if let Some(inserted) = boundaries[p] {
                            if inserted == (kind == 3) { remove.push(p); }
                            else { boundaries[p] = None; }
                        }
                    }
                    4 => {
                        for (index, boundary) in boundaries.iter_mut().enumerate() {
                            if let Some(inserted) = *boundary {
                                if inserted != tracked { remove.push(index); }
                                else { *boundary = None; }
                            }
                        }
                    }
                    _ => {},
                }
                for index in remove.into_iter().rev() {
                    let right = paragraphs.remove(index+1);
                    paragraphs[index].push_str(&right);
                    boundaries.remove(index);
                }
            }
            let xml = part(&bytes, "word/document.xml");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let actual: Vec<_> = doc.descendants().filter(|n| n.has_tag_name((NS, "p"))).map(|p| {
                let text: String = p.descendants().filter(|n| n.has_tag_name((NS, "t"))).filter_map(|n| n.text()).collect();
                let marks: Vec<_> = p.children().filter(|n| n.has_tag_name((NS, "pPr")))
                    .flat_map(|n| n.descendants())
                    .filter(|n| n.has_tag_name((NS, "ins")) || n.has_tag_name((NS, "del"))).collect();
                (text, marks)
            }).collect();
            prop_assert_eq!(actual.len(), paragraphs.len(), "paragraph count at step {}", step);
            for (index, (text, marks)) in actual.iter().enumerate() {
                prop_assert_eq!(text, &paragraphs[index], "paragraph {} at step {}", index, step);
                let expected = boundaries.get(index).copied().flatten();
                prop_assert_eq!(marks.len(), usize::from(expected.is_some()), "mark count at paragraph {} step {}", index, step);
                if let Some(inserted) = expected {
                    prop_assert!(marks[0].has_tag_name((NS, if inserted { "ins" } else { "del" })), "revision kind at step {}", step);
                    prop_assert_eq!(marks[0].attribute((NS, "author")), Some("LAWGUE"));
                    prop_assert_eq!(marks[0].attribute((NS, "date")), Some("2026-10-24T00:00:00Z"));
                }
            }
            prop_assert_eq!(paragraphs.concat(), initial.clone(), "Unicode text at step {}", step);
            prop_assert_eq!(final_text(&bytes), initial.clone(), "saved text at step {}", step);
        }
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
fn deleting_another_authors_tracked_row_preserves_independent_revisions() {
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
    let deleted = edit(
        &input,
        vec![
            tracking(),
            HeadlessCommand::DeleteTableRow {
                table_order_index: 0,
                row_index: 1,
            },
        ],
    );
    assert_eq!(table_shape(&deleted), Some((3, 2)));
    let xml = part(&deleted, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let insertion = document
        .descendants()
        .find(|node| {
            node.has_tag_name((NS, "ins")) && node.attribute((NS, "author")) == Some("Reviewer")
        })
        .unwrap();
    let insertion_id = insertion.attribute((NS, "id")).unwrap().to_string();
    assert_eq!(
        insertion.attribute((NS, "date")),
        Some("2025-01-01T00:00:00Z")
    );
    let deletion_ids = lg_ids(&deleted);
    assert_eq!(deletion_ids.len(), 1);
    // Each author can decide their revision independently, even after saving.
    let restored = edit(&deleted, vec![decide(false, deletion_ids.clone())]);
    assert_eq!(tracked_rows(&restored), Some(vec![0, 1, 0]));
    let inserted = edit(&deleted, vec![decide(true, vec![insertion_id.clone()])]);
    assert_eq!(tracked_rows(&inserted), Some(vec![0, 2, 0]));
    assert_eq!(
        table_shape(&edit(&deleted, vec![decide(true, deletion_ids)])),
        Some((2, 2))
    );
    assert_eq!(
        table_shape(&edit(&deleted, vec![decide(false, vec![insertion_id])])),
        Some((2, 2))
    );
}

#[test]
fn overlapping_row_history_preserves_extensions_repeated_deletion_and_review_queries() {
    use jade_docx_headless_editor::{
        execute_headless_queries, HeadlessQuery, HeadlessQueryRequest, HeadlessQueryResult,
    };
    let input = fixture(
        r#"</w:p><w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid>
    <w:tr><w:trPr><w:ins xmlns:w16du="http://schemas.microsoft.com/office/word/2023/wordml/word16du" xmlns:ext="urn:revision-extension" w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z" w16du:dateUtc="2025-01-01T00:00:00Z" ext:source="kept"/></w:trPr><w:tc><w:p><w:r><w:t>reviewed</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl><w:p>"#,
    );
    for whole_table in [false, true] {
        let delete = || {
            if whole_table {
                HeadlessCommand::DeleteTable {
                    table_order_index: 0,
                }
            } else {
                HeadlessCommand::DeleteTableRow {
                    table_order_index: 0,
                    row_index: 0,
                }
            }
        };
        let deleted = edit(&input, vec![tracking(), delete(), delete()]);
        let response = execute_headless_queries(
            &deleted,
            &HeadlessQueryRequest {
                queries: vec![HeadlessQuery::Revisions {
                    start_revision: 0,
                    max_revisions: 10,
                    kind: None,
                    author: None,
                    date_from: None,
                    date_to: None,
                }],
            },
        )
        .unwrap();
        let HeadlessQueryResult::Revisions {
            total_revisions,
            revisions,
            ..
        } = &response.results[0]
        else {
            panic!("expected revision list")
        };
        assert_eq!(*total_revisions, 2);
        assert!(revisions
            .iter()
            .any(|revision| revision.author.as_deref() == Some("Reviewer")));
        assert!(revisions
            .iter()
            .any(|revision| revision.author.as_deref() == Some("LAWGUE")));
        let restored = edit(&deleted, vec![decide(false, lg_ids(&deleted))]);
        assert_eq!(final_text(&restored), "reviewed");
        let xml = part(&restored, "word/document.xml");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let insertion = doc
            .descendants()
            .find(|node| node.has_tag_name((NS, "ins")))
            .unwrap();
        assert_eq!(
            insertion.attribute(("urn:revision-extension", "source")),
            Some("kept")
        );
        assert_eq!(
            insertion.attribute((
                "http://schemas.microsoft.com/office/word/2023/wordml/word16du",
                "dateUtc"
            )),
            Some("2025-01-01T00:00:00Z")
        );
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RowHistory {
    inserted_by: Option<bool>,
    deleted_by: Option<bool>,
}

fn persisted_row_history(bytes: &[u8]) -> Option<Vec<RowHistory>> {
    let xml = part(bytes, "word/document.xml");
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let table = doc
        .descendants()
        .find(|node| node.has_tag_name((NS, "tbl")))?;
    Some(
        table
            .children()
            .filter(|node| node.has_tag_name((NS, "tr")))
            .map(|row| {
                let marks: Vec<_> = row
                    .children()
                    .filter(|node| node.has_tag_name((NS, "trPr")))
                    .flat_map(|node| node.children())
                    .filter(|node| node.has_tag_name((NS, "ins")) || node.has_tag_name((NS, "del")))
                    .collect();
                let author = |name| {
                    marks
                        .iter()
                        .find(|node| node.has_tag_name((NS, name)))
                        .map(|node| {
                            assert_eq!(
                                marks
                                    .iter()
                                    .filter(|mark| mark.tag_name().name() == name)
                                    .count(),
                                1
                            );
                            assert_eq!(node.attribute((NS, "date")), Some("2026-10-25T00:00:00Z"));
                            match node.attribute((NS, "author")) {
                                Some("Ann") => false,
                                Some("Bob") => true,
                                other => panic!("unexpected row author: {other:?}"),
                            }
                        })
                };
                RowHistory {
                    inserted_by: author("ins"),
                    deleted_by: author("del"),
                }
            })
            .collect(),
    )
}

#[derive(Clone, Debug)]
enum RowAction {
    EnsureTable(bool),
    Insert(bool, usize),
    Delete(bool, usize),
    DeleteTable(bool),
    Decide { accept: bool, kind: u8 },
    UnknownDecision,
}

fn row_action() -> impl Strategy<Value = RowAction> {
    prop_oneof![
        any::<bool>().prop_map(RowAction::EnsureTable),
        (any::<bool>(), 0usize..7).prop_map(|(author, row)| RowAction::Insert(author, row)),
        (any::<bool>(), 0usize..7).prop_map(|(author, row)| RowAction::Delete(author, row)),
        any::<bool>().prop_map(RowAction::DeleteTable),
        (any::<bool>(), 0u8..3).prop_map(|(accept, kind)| RowAction::Decide { accept, kind }),
        Just(RowAction::UnknownDecision),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn persisted_foreign_row_revisions_match_independent_model(actions in prop::collection::vec(row_action(), 1..33)) {
        use jade_docx_headless_editor::HeadlessRevisionKindFilter;
        let mut bytes = fixture("<w:r><w:t>契約</w:t></w:r>");
        let mut rows: Option<Vec<RowHistory>> = None;
        for (step, action) in actions.iter().enumerate() {
            let mut expected = rows.clone();
            let mut invalid = false;
            let author = match action {
                RowAction::EnsureTable(author) | RowAction::Insert(author, _) | RowAction::Delete(author, _) | RowAction::DeleteTable(author) => *author,
                _ => false,
            };
            let delete = |row: &mut RowHistory| {
                if row.deleted_by.is_some() { return false; }
                if row.inserted_by == Some(author) { return true; }
                row.deleted_by = Some(author);
                false
            };
            let command = match action {
                RowAction::EnsureTable(_) if expected.is_none() => {
                    expected = Some(vec![RowHistory { inserted_by: Some(author), deleted_by: None }; 2]);
                    HeadlessCommand::InsertTable { after_paragraph: 0, rows: 2, columns: 1, column_widths: vec![], caret_offset: None }
                }
                RowAction::Insert(_, index) if expected.as_ref().is_some_and(|rows| *index <= rows.len()) => {
                    expected.as_mut().unwrap().insert(*index, RowHistory { inserted_by: Some(author), deleted_by: None });
                    HeadlessCommand::InsertTableRow { table_order_index: 0, row_index: *index, template_row: None }
                }
                RowAction::Delete(_, index) if expected.as_ref().is_some_and(|rows| *index < rows.len()) => {
                    let rows = expected.as_mut().unwrap();
                    if delete(&mut rows[*index]) { rows.remove(*index); }
                    HeadlessCommand::DeleteTableRow { table_order_index: 0, row_index: *index }
                }
                RowAction::DeleteTable(_) if expected.is_some() => {
                    expected.as_mut().unwrap().retain_mut(|row| !delete(row));
                    HeadlessCommand::DeleteTable { table_order_index: 0 }
                }
                RowAction::Decide { accept, kind } => {
                    let insertion = *kind != 2;
                    let deletion = *kind != 1;
                    if let Some(rows) = &mut expected {
                        rows.retain_mut(|row| {
                            let remove = (insertion && row.inserted_by.is_some() && !accept)
                                || (deletion && row.deleted_by.is_some() && *accept);
                            if insertion { row.inserted_by = None; }
                            if deletion { row.deleted_by = None; }
                            !remove
                        });
                    }
                    let kind = match kind { 1 => Some(HeadlessRevisionKindFilter::Inserted), 2 => Some(HeadlessRevisionKindFilter::Deleted), _ => None };
                    if *accept {
                        HeadlessCommand::AcceptRevisions { kind, start_paragraph: None, end_paragraph: None, revision_id: None, revision_ids: None, revision_targets: None }
                    } else {
                        HeadlessCommand::RejectRevisions { kind, start_paragraph: None, end_paragraph: None, revision_id: None, revision_ids: None, revision_targets: None }
                    }
                }
                RowAction::UnknownDecision => decide(false, vec!["999999".into()]),
                RowAction::EnsureTable(_) => HeadlessCommand::StopChangeTracking,
                RowAction::Insert(_, index) => { invalid = true; HeadlessCommand::InsertTableRow { table_order_index: 0, row_index: *index, template_row: None } }
                RowAction::Delete(_, index) => { invalid = true; HeadlessCommand::DeleteTableRow { table_order_index: 0, row_index: *index } }
                RowAction::DeleteTable(_) => { invalid = true; HeadlessCommand::DeleteTable { table_order_index: 0 } }
            };
            if expected.as_ref().is_some_and(Vec::is_empty) { expected = None; }
            let output = apply_headless_commands(&bytes, &HeadlessEditRequest { commands: vec![
                HeadlessCommand::StartChangeTracking { author: Some(if author { "Bob" } else { "Ann" }.into()), initials: None, date: Some("2026-10-25T00:00:00Z".into()), date_utc: None }, command,
            ] });
            if invalid { prop_assert!(output.is_err(), "step {} {:?}: invalid operation succeeded", step, action); }
            else {
                prop_assert!(output.is_ok(), "step {} {:?}: {:?}", step, action, output.as_ref().err());
                bytes = output.unwrap();
                rows = expected;
            }
            prop_assert_eq!(persisted_row_history(&bytes), rows.clone(), "step {} {:?}", step, action);
            prop_assert_eq!(final_text(&bytes), "契約", "step {} {:?}", step, action);
        }
    }
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
            let deleted = edit(&input, vec![tracking(), command]);
            assert_eq!(table_shape(&deleted), Some((rows, 1)));
            let xml = part(&deleted, "word/document.xml");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            assert!(doc.descendants().any(|node| node.has_tag_name((NS, "ins"))
                && node.attribute((NS, "id")) == Some("42")
                && node.attribute((NS, "author")) == Some("Reviewer")));
            let deletion_ids: Vec<_> = doc
                .descendants()
                .filter(|node| node.has_tag_name((NS, "del")))
                .map(|node| node.attribute((NS, "id")).unwrap().to_string())
                .collect();
            let restored = edit(&deleted, vec![decide(false, deletion_ids)]);
            assert_eq!(final_text(&restored), "reviewed");
            assert!(part(&restored, "word/document.xml").contains("Reviewer"));
            assert_eq!(
                tracked_rows(&restored),
                Some(if rows == 1 { vec![1] } else { vec![1, 0] })
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

#[test]
fn deleting_the_last_grid_unit_removes_the_table_and_supports_undo() {
    use jade_docx_document_core::{
        commands::{CommandContext, CommandExecutor},
        headless_api::apply_headless_commands_with_executor_to_document,
        word_document::WordDocument,
        Options,
    };
    let single = String::from(
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
    );
    for nested in [false, true] {
        for column in [false, true] {
            for tracked in [false, true] {
                let table = if nested {
                    format!(
                        r#"<w:tbl><w:tblGrid><w:gridCol w:w="4800"/></w:tblGrid><w:tr><w:tc>{single}<w:p><w:r><w:t>outer</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#
                    )
                } else {
                    single.clone()
                };
                let input = fixture(&format!(
                    "<w:r><w:t>HEAD</w:t></w:r></w:p>{table}<w:p><w:r><w:t>TAIL</w:t></w:r>"
                ));
                let mut document =
                    futures::executor::block_on(WordDocument::load(&input, &Options::default()))
                        .unwrap();
                let mut executor = CommandExecutor::new();
                apply_headless_commands_with_executor_to_document(
                    &mut document,
                    &mut executor,
                    None,
                    &HeadlessEditRequest {
                        commands: vec![if tracked {
                            tracking()
                        } else {
                            HeadlessCommand::StopChangeTracking
                        }],
                    },
                )
                .unwrap();
                let before = part(&document.export_docx().unwrap(), "word/document.xml");
                let command = if column {
                    HeadlessCommand::DeleteTableColumn {
                        table_order_index: usize::from(nested),
                        column_index: 0,
                    }
                } else {
                    HeadlessCommand::DeleteTableRow {
                        table_order_index: usize::from(nested),
                        row_index: 0,
                    }
                };
                let mut selection = apply_headless_commands_with_executor_to_document(
                    &mut document,
                    &mut executor,
                    None,
                    &HeadlessEditRequest {
                        commands: vec![command],
                    },
                )
                .unwrap_or_else(|error| {
                    panic!("nested={nested} column={column} tracked={tracked}: {error}")
                });
                let output = document.export_docx().unwrap();
                let after = part(&output, "word/document.xml");
                assert_eq!(
                    final_text(&output),
                    if nested { "HEADouterTAIL" } else { "HEADTAIL" }
                );
                assert_eq!(lg_ids(&output).is_empty(), column || !tracked);
                let xml = roxmltree::Document::parse(&after).unwrap();
                assert_eq!(
                    xml.descendants()
                        .filter(|n| n.has_tag_name((NS, "tbl")))
                        .count(),
                    usize::from(nested) + usize::from(tracked && !column)
                );
                {
                    let mut ctx = CommandContext::new(&mut document, &mut selection);
                    assert!(executor.undo(&mut ctx).unwrap());
                }
                assert_eq!(
                    part(&document.export_docx().unwrap(), "word/document.xml"),
                    before
                );
                {
                    let mut ctx = CommandContext::new(&mut document, &mut selection);
                    assert!(executor.redo(&mut ctx).unwrap());
                }
                assert_eq!(
                    part(&document.export_docx().unwrap(), "word/document.xml"),
                    after
                );
            }
        }
    }
}

#[test]
fn bulk_cell_edits_and_row_clones_track_changes_and_support_undo() {
    use jade_docx_document_core::{
        commands::{CommandContext, CommandExecutor},
        headless_api::apply_headless_commands_with_executor_to_document,
        word_document::WordDocument,
        Options,
    };
    let input = fixture(&format!(
        r#"<w:ins w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>R</w:t></w:r></w:ins></w:p>{}<w:p>"#,
        range_table("甲", "乙")
    ));
    for clone in [false, true] {
        for tracked in [false, true] {
            let mut document =
                futures::executor::block_on(WordDocument::load(&input, &Options::default()))
                    .unwrap();
            let mut executor = CommandExecutor::new();
            apply_headless_commands_with_executor_to_document(
                &mut document,
                &mut executor,
                None,
                &HeadlessEditRequest {
                    commands: vec![if tracked {
                        tracking()
                    } else {
                        HeadlessCommand::StopChangeTracking
                    }],
                },
            )
            .unwrap();
            let before = part(&document.export_docx().unwrap(), "word/document.xml");
            let command = if clone {
                HeadlessCommand::CloneTableRow {
                    table_order_index: 0,
                    source_row: 0,
                    target_row: 1,
                    clear_text: false,
                }
            } else {
                HeadlessCommand::FillTableRow {
                    table_order_index: 0,
                    row: 0,
                    values: vec!["新😀".into(), "".into()],
                }
            };
            let mut selection = apply_headless_commands_with_executor_to_document(
                &mut document,
                &mut executor,
                None,
                &HeadlessEditRequest {
                    commands: vec![command],
                },
            )
            .unwrap();
            let output = document.export_docx().unwrap();
            let after = part(&output, "word/document.xml");
            assert_eq!(
                final_text(&output),
                if clone { "R甲乙甲乙" } else { "R新😀" }
            );
            assert_other_author(&output);
            assert_eq!(
                lg_ids(&output).is_empty(),
                !tracked,
                "clone={clone} tracked={tracked}"
            );
            if tracked {
                let rejected = edit(&output, vec![decide(false, lg_ids(&output))]);
                assert_eq!(final_text(&rejected), "R甲乙");
                assert_eq!(table_shape(&rejected), Some((1, 2)));
                assert_other_author(&rejected);
            }
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                assert!(
                    executor.undo(&mut ctx).unwrap(),
                    "clone={clone} tracked={tracked}"
                );
            }
            assert_eq!(
                part(&document.export_docx().unwrap(), "word/document.xml"),
                before
            );
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                assert!(executor.redo(&mut ctx).unwrap());
            }
            assert_eq!(
                part(&document.export_docx().unwrap(), "word/document.xml"),
                after
            );
        }
    }
}

/// Read saved cells independently of the editor model, excluding deleted text.
fn saved_grid(bytes: &[u8]) -> Option<Vec<Vec<String>>> {
    let xml = part(bytes, "word/document.xml");
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let table = doc.descendants().find(|n| n.has_tag_name((NS, "tbl")))?;
    let rows: Vec<Vec<String>> = table
        .children()
        .filter(|n| {
            n.has_tag_name((NS, "tr"))
                && !n
                    .children()
                    .filter(|p| p.has_tag_name((NS, "trPr")))
                    .any(|p| p.children().any(|mark| mark.has_tag_name((NS, "del"))))
        })
        .map(|row| {
            row.children()
                .filter(|n| n.has_tag_name((NS, "tc")))
                .map(|cell| {
                    cell.descendants()
                        .filter(|n| {
                            n.has_tag_name((NS, "t"))
                                && !n.ancestors().any(|a| {
                                    a.has_tag_name((NS, "del")) || a.has_tag_name((NS, "moveFrom"))
                                })
                        })
                        .filter_map(|n| n.text())
                        .collect()
                })
                .collect()
        })
        .collect();
    (!rows.is_empty()).then_some(rows)
}
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/revision_workflows.proptest-regressions")
        ))),
        ..ProptestConfig::default()
    })]
    #[test]
    fn mixed_persisted_grid_workflows_match_cell_model(
        actions in prop::collection::vec((0u8..10, any::<u8>(), any::<u8>(), prop_oneof![Just(""), Just("契約😀"), Just("a&<b"), Just("e\u{301}条"), Just("更新")], any::<bool>(), any::<bool>()), 1..49)
    ) {
        use jade_docx_document_core::{
            commands::{CommandContext, CommandExecutor},
            headless_api::apply_headless_commands_with_executor_to_document,
            word_document::WordDocument, Options,
        };
        let mut bytes = fixture(r#"<w:ins w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>R</w:t></w:r></w:ins>"#);
        let mut model: Option<Vec<Vec<String>>> = None;
        for (step, (kind, first, second, text, tracked, accept)) in actions.into_iter().enumerate() {
            let before_model = model.clone();
            let mut expected = model.clone();
            let mut invalid = false;
            let mut reversible_revision = true;
            let command = match (kind, expected.as_mut()) {
                (0, None) => {
                    expected = Some(vec![vec![String::new(); 2]; 2]);
                    HeadlessCommand::InsertTable { after_paragraph:0, rows:2, columns:2, column_widths:vec![], caret_offset:None }
                }
                (0, Some(_)) => {
                    expected = None;
                    HeadlessCommand::DeleteTable { table_order_index:0 }
                }
                (1, Some(grid)) => {
                    let row = first as usize % (grid.len()+1);
                    grid.insert(row, vec![String::new();grid[0].len()]);
                    HeadlessCommand::InsertTableRow { table_order_index:0, row_index:row, template_row:None }
                }
                (2, Some(grid)) => {
                    let row = first as usize % grid.len();
                    grid.remove(row);
                    if grid.is_empty() { expected = None; }
                    HeadlessCommand::DeleteTableRow { table_order_index:0, row_index:row }
                }
                (3, Some(grid)) => {
                    reversible_revision = false;
                    let column = first as usize % (grid[0].len()+1);
                    for row in grid { row.insert(column, String::new()); }
                    HeadlessCommand::InsertTableColumn { table_order_index:0, column_index:column, width:None, template_column:None }
                }
                (4, Some(grid)) => {
                    reversible_revision = false;
                    let column = first as usize % grid[0].len();
                    for row in grid.iter_mut() { row.remove(column); }
                    if grid[0].is_empty() { expected=None; }
                    HeadlessCommand::DeleteTableColumn { table_order_index:0, column_index:column }
                }
                (5, Some(grid)) => {
                    let row = first as usize % grid.len();
                    let column = second as usize % grid[0].len();
                    grid[row][column] = text.to_owned();
                    HeadlessCommand::SetTableCellText { table_order_index:0, row, column, text:text.into() }
                }
                (6, Some(grid)) => {
                    let row = first as usize % grid.len();
                    let values = vec![text.to_owned(); grid[0].len()];
                    grid[row] = values.clone();
                    HeadlessCommand::FillTableRow { table_order_index:0, row, values }
                }
                (7, Some(grid)) => {
                    let source_row = first as usize % grid.len();
                    let target_row = second as usize % (grid.len()+1);
                    let clear_text = first % 2 == 0;
                    let row = if clear_text { vec![String::new();grid[0].len()] } else { grid[source_row].clone() };
                    grid.insert(target_row,row);
                    HeadlessCommand::CloneTableRow { table_order_index:0, source_row, target_row, clear_text }
                }
                (8, Some(grid)) => {
                    invalid = true;
                    match first % 3 {
                        0 => HeadlessCommand::FillTableRow { table_order_index:0, row:0, values:vec![text.to_owned();grid[0].len()+1] },
                        1 => HeadlessCommand::CloneTableRow { table_order_index:0, source_row:usize::MAX, target_row:0, clear_text:false },
                        _ => HeadlessCommand::DeleteTableColumn { table_order_index:0, column_index:usize::MAX },
                    }
                }
                (9, Some(grid)) => HeadlessCommand::FillTableRow { table_order_index:0, row:first as usize % grid.len(), values:vec![] },
                _ => {
                    invalid = true;
                    HeadlessCommand::FillTableRow { table_order_index:0, row:usize::MAX, values:vec![text.into()] }
                }
            };
            let mut document = futures::executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
            let mut executor = CommandExecutor::new();
            apply_headless_commands_with_executor_to_document(&mut document, &mut executor, None, &HeadlessEditRequest { commands:vec![if tracked {tracking()} else {HeadlessCommand::StopChangeTracking}] }).unwrap();
            let before = part(&document.export_docx().unwrap(), "word/document.xml");
            let result = apply_headless_commands_with_executor_to_document(&mut document, &mut executor, None, &HeadlessEditRequest {commands:vec![command.clone()]});
            if invalid {
                prop_assert!(result.is_err(), "step {} {:?} must reject",step,command);
                prop_assert_eq!(part(&document.export_docx().unwrap(), "word/document.xml"), before, "step {} rejection is atomic",step);
                continue;
            }
            prop_assert!(result.is_ok(), "step {} {:?}: {:?}",step,command,result);
            let mut selection = result.unwrap();
            let output = document.export_docx().unwrap();
            prop_assert_eq!(saved_grid(&output), expected.clone(), "step {} {:?} final view before review",step,command);
            let after = part(&output, "word/document.xml");
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.undo(&mut ctx).unwrap(), "step {} {:?} undo",step,command);
            }
            prop_assert_eq!(part(&document.export_docx().unwrap(), "word/document.xml"), before, "step {} {:?} undo restores XML",step,command);
            {
                let mut ctx = CommandContext::new(&mut document, &mut selection);
                prop_assert!(executor.redo(&mut ctx).unwrap());
            }
            prop_assert_eq!(part(&document.export_docx().unwrap(), "word/document.xml"), after, "step {} {:?} redo restores XML",step,command);
            bytes = if tracked {
                edit(&output,vec![decide(accept,lg_ids(&output))])
            } else { output };
            model = if tracked && !accept && reversible_revision { before_model } else { expected };
            prop_assert_eq!(saved_grid(&bytes), model.clone(), "step {} {:?}, tracked={}, accept={}",step,command,tracked,accept);
            assert_other_author(&bytes);
            prop_assert!(lg_ids(&bytes).is_empty(), "step {} decisions remove our history",step);
        }
    }
}

#[test]
fn bulk_fill_on_foreign_inserted_row_preserves_history_when_rejected() {
    let table = range_table("甲", "乙")
        .replace("<w:tr>", r#"<w:tr><w:trPr><w:ins w:id="40" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"/></w:trPr>"#)
        .replace("<w:r><w:t>甲</w:t></w:r>", r#"<w:ins w:id="42" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>甲</w:t></w:r></w:ins>"#);
    let input = fixture(&format!(
        r#"<w:ins w:id="41" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"><w:r><w:t>R</w:t></w:r></w:ins></w:p>{table}<w:p>"#
    ));
    let output = edit(
        &input,
        vec![
            tracking(),
            HeadlessCommand::FillTableRow {
                table_order_index: 0,
                row: 0,
                values: vec!["新😀".into(), "更新".into()],
            },
        ],
    );
    assert_eq!(final_text(&output), "R新😀更新");
    let rejected = edit(&output, vec![decide(false, lg_ids(&output))]);
    assert_eq!(final_text(&rejected), "R甲乙");
    assert_other_author(&rejected);
    let xml = part(&rejected, "word/document.xml");
    let document = roxmltree::Document::parse(&xml).unwrap();
    for id in ["40", "42"] {
        let mark = document
            .descendants()
            .find(|n| n.has_tag_name((NS, "ins")) && n.attribute((NS, "id")) == Some(id))
            .unwrap();
        assert_eq!(mark.attribute((NS, "author")), Some("Reviewer"));
        assert_eq!(mark.attribute((NS, "date")), Some("2025-01-01T00:00:00Z"));
    }
}

fn foreign_revision_row_fixture() -> Vec<u8> {
    let row = r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:trPr><w:ins w:id="40" w:author="Reviewer" w:date="2025-01-01T00:00:00Z"/></w:trPr><w:tc><w:tcPr/><w:p><w:del w:id="42" w:author="Reviewer"><w:r><w:delText>旧</w:delText></w:r></w:del><w:ins w:id="43" w:author="Reviewer"><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>新😀</w:t></w:r></w:ins></w:p></w:tc></w:tr></w:tbl>"#;
    fixture(&format!("</w:p>{row}<w:p>"))
}

#[test]
fn cloning_before_or_after_a_row_preserves_source_revision_ids() {
    let input = foreign_revision_row_fixture();
    let original_xml = part(&input, "word/document.xml");
    let original_doc = roxmltree::Document::parse(&original_xml).unwrap();
    for target_row in [0, 1] {
        for clear_text in [false, true] {
            for tracked in [false, true] {
                let output = edit(
                    &input,
                    vec![
                        if tracked {
                            tracking()
                        } else {
                            HeadlessCommand::StopChangeTracking
                        },
                        HeadlessCommand::CloneTableRow {
                            table_order_index: 0,
                            source_row: 0,
                            target_row,
                            clear_text,
                        },
                    ],
                );
                let xml = part(&output, "word/document.xml");
                let doc = roxmltree::Document::parse(&xml).unwrap();
                for id in ["40", "42", "43"] {
                    let original = original_doc
                        .descendants()
                        .find(|n| n.attribute((NS, "id")) == Some(id))
                        .unwrap();
                    let copies: Vec<_> = doc
                        .descendants()
                        .filter(|n| n.attribute((NS, "id")) == Some(id))
                        .collect();
                    assert_eq!(copies.len(), 1, "foreign revision {id} lost or duplicated");
                    assert_eq!(&xml[copies[0].range()], &original_xml[original.range()],
                        "source revision changed: target={target_row} clear={clear_text} tracked={tracked}");
                }
                let mut expected = vec![vec!["新😀".into()]];
                expected.insert(
                    target_row,
                    vec![if clear_text {
                        "".into()
                    } else {
                        "新😀".into()
                    }],
                );
                assert_eq!(saved_grid(&output), Some(expected));
                if tracked {
                    let rejected = edit(&output, vec![decide(false, lg_ids(&output))]);
                    assert_eq!(saved_grid(&rejected), Some(vec![vec!["新😀".into()]]));
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/revision_workflows.proptest-regressions")
        ))),
        ..ProptestConfig::default()
    })]
    #[test]
    fn cloning_foreign_rows_survives_repeated_save_reload(
        actions in prop::collection::vec((any::<u8>(), any::<bool>(), any::<bool>(), any::<bool>()), 1..13)
    ) {
        let mut bytes = foreign_revision_row_fixture();
        let original_xml = part(&bytes, "word/document.xml");
        let original_doc = roxmltree::Document::parse(&original_xml).unwrap();
        let foreign_marks: Vec<_> = ["40", "42", "43"].into_iter().map(|id| {
            let node = original_doc.descendants().find(|n| n.attribute((NS, "id")) == Some(id)).unwrap();
            (id, original_xml[node.range()].to_owned())
        }).collect();
        let mut source_row = 0usize;
        let mut model = vec![vec!["新😀".to_string()]];
        for (target, clear_text, tracked, accept) in actions {
            let target_row = usize::from(target) % (model.len() + 1);
            let output = edit(&bytes, vec![
                if tracked { tracking() } else { HeadlessCommand::StopChangeTracking },
                HeadlessCommand::CloneTableRow { table_order_index: 0, source_row, target_row, clear_text },
            ]);
            let mut cloned = model.clone();
            cloned.insert(target_row, vec![if clear_text { String::new() } else { "新😀".to_string() }]);
            prop_assert_eq!(saved_grid(&output), Some(cloned.clone()));
            bytes = if tracked { edit(&output, vec![decide(accept, lg_ids(&output))]) } else { output };
            if !tracked || accept {
                model = cloned;
                if target_row <= source_row { source_row += 1; }
            }
            prop_assert_eq!(saved_grid(&bytes), Some(model.clone()));
            let xml = part(&bytes, "word/document.xml");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            for (id, original) in &foreign_marks {
                let nodes: Vec<_> = doc.descendants().filter(|n| n.attribute((NS, "id")) == Some(*id)).collect();
                prop_assert_eq!(nodes.len(), 1, "foreign revision {} was lost or duplicated", id);
                prop_assert_eq!(&xml[nodes[0].range()], original, "{}", xml);
            }
        }
    }
}
