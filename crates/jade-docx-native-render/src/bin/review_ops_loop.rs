//! Times review actions — accepting and rejecting a tracked change, adding a
//! comment, deleting a tracked selection — and their undo on a loaded DOCX,
//! so a sampling profiler has them to attach to.
//!
//! Usage: `review_ops_loop <input.docx> [iterations]`

use futures::executor::block_on;
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::{RevisionSession, WordDocument};
use jade_docx_editor::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .ok_or_else(|| "usage: review_ops_loop <input.docx> [iterations]".to_string())?;
    let iterations: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(10);
    let bytes = std::fs::read(&input).map_err(|err| format!("{input}: {err}"))?;
    let options = Options::default();
    let mut doc = block_on(WordDocument::load(&bytes, &options))
        .map_err(|err| format!("{input}: parse failed: {err}"))?;
    doc.ensure_roundtrip_xml().map_err(|err| err.to_string())?;

    let mut totals: BTreeMap<&'static str, (Duration, usize)> = BTreeMap::new();
    let mut time = |name: &'static str, run: &mut dyn FnMut()| {
        let started = Instant::now();
        run();
        let entry = totals.entry(name).or_default();
        entry.0 += started.elapsed();
        entry.1 += 1;
    };

    for _ in 0..iterations {
        time("snapshot", &mut || drop(doc.snapshot()));
        time("snapshot_including_package", &mut || {
            drop(doc.snapshot_including_package())
        });
        time("clone model", &mut || {
            drop(doc.document.as_ref().map(|rc| (**rc).clone()))
        });
        time("clone raw xml", &mut || drop(doc.raw_document.clone()));
        time("clone parts", &mut || drop(doc.parts.clone()));
        time("clone document_part", &mut || {
            drop(doc.document_part.clone())
        });
        time("clone styles_part", &mut || drop(doc.styles_part.clone()));
    }

    let session = RevisionSession {
        author: Some("perf".into()),
        date: Some("2026-10-05T00:00:00Z".into()),
        date_utc: None,
    };
    let mut executor = CommandExecutor::new();
    let run = |doc: &mut WordDocument,
               executor: &mut CommandExecutor,
               commands: Vec<HeadlessCommand>|
     -> Result<(), String> {
        apply_headless_commands_with_executor_to_document(
            doc,
            executor,
            None,
            &HeadlessEditRequest { commands },
        )
        .map(|_| ())
        .map_err(|err| format!("{err:?}"))
    };
    let undo = |doc: &mut WordDocument, executor: &mut CommandExecutor| {
        let mut selection = None;
        let mut ctx = CommandContext::new(doc, &mut selection);
        executor
            .undo(&mut ctx)
            .map(|_| ())
            .map_err(|err| err.to_string())
    };
    let decide = |accept: bool, paragraph: usize| {
        if accept {
            HeadlessCommand::AcceptRevisions {
                kind: None,
                start_paragraph: Some(paragraph),
                end_paragraph: Some(paragraph),
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }
        } else {
            HeadlessCommand::RejectRevisions {
                kind: None,
                start_paragraph: Some(paragraph),
                end_paragraph: Some(paragraph),
                revision_id: None,
                revision_ids: None,
                revision_targets: None,
            }
        }
    };

    let mut timed = BTreeMap::<&'static str, Vec<f64>>::new();
    let mut measure = |name: &'static str, result: &mut dyn FnMut() -> Result<(), String>| {
        let started = Instant::now();
        result()?;
        timed
            .entry(name)
            .or_default()
            .push(started.elapsed().as_secs_f64() * 1000.0);
        Ok::<(), String>(())
    };
    // Paragraphs with text of their own and two more after them, near the
    // top. Offsets are UTF-8 bytes, so they are taken at characters.
    let text = |doc: &WordDocument, index: usize| -> String {
        doc.paragraph(index)
            .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
            .unwrap_or_default()
    };
    let at = |doc: &WordDocument, index: usize, chars: usize| {
        let text = text(doc, index);
        text.char_indices()
            .nth(chars)
            .map_or(text.len(), |(offset, _)| offset)
    };
    let candidates: Vec<usize> = (0..doc.paragraph_count().saturating_sub(3))
        .filter(|&index| (0..3).all(|step| text(&doc, index + step).chars().count() >= 4))
        .step_by(3)
        .take(iterations)
        .collect();
    for paragraph in candidates {
        let (comment_end, delete_start, delete_end) = (
            at(&doc, paragraph, 2),
            at(&doc, paragraph, 1),
            at(&doc, paragraph + 2, 1),
        );
        for accept in [true, false] {
            doc.set_revision_session(Some(session.clone()));
            run(
                &mut doc,
                &mut executor,
                vec![HeadlessCommand::InsertText {
                    paragraph,
                    offset: 0,
                    text: "かきく".into(),
                }],
            )?;
            doc.set_revision_session(None);
            measure(if accept { "accept" } else { "reject" }, &mut || {
                run(&mut doc, &mut executor, vec![decide(accept, paragraph)])
            })?;
            measure("undo decision", &mut || undo(&mut doc, &mut executor))?;
        }
        measure("add comment", &mut || {
            run(
                &mut doc,
                &mut executor,
                vec![HeadlessCommand::AddComment {
                    start_paragraph: paragraph,
                    start_offset: Some(0),
                    end_paragraph: paragraph,
                    end_offset: Some(comment_end),
                    text: "確認".into(),
                    author: None,
                    initials: None,
                    date: None,
                }],
            )
        })?;
        measure("undo comment", &mut || undo(&mut doc, &mut executor))?;
        doc.set_revision_session(Some(session.clone()));
        measure("tracked range delete", &mut || {
            run(
                &mut doc,
                &mut executor,
                vec![HeadlessCommand::ReplaceRange {
                    start_paragraph: paragraph,
                    start_offset: delete_start,
                    end_paragraph: paragraph + 2,
                    end_offset: delete_end,
                    text: String::new(),
                }],
            )
        })?;
        measure("undo range delete", &mut || undo(&mut doc, &mut executor))?;
        doc.set_revision_session(None);
    }

    println!("{input}: {} paragraphs", doc.paragraph_count());
    for (name, (total, count)) in totals {
        println!(
            "{name:>28}: {:7.2} ms",
            total.as_secs_f64() * 1000.0 / count as f64
        );
    }
    for (name, mut values) in timed {
        values.sort_by(f64::total_cmp);
        println!(
            "{name:>28}: p50 {:7.2} ms  min {:7.2} ms",
            values[values.len() / 2],
            values[0]
        );
    }
    Ok(())
}
