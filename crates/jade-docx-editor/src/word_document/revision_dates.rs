//! Every tracked change jade records carries `w:date`, as Word's do.

use super::review_tools::tests::docx_with_body;
use super::*;
use crate::commands::delete_text::{DeleteDirection, DeleteTextCommand};
use crate::commands::insert_paragraph_break::InsertParagraphBreakCommand;
use crate::commands::insert_text::InsertTextCommand;
use crate::commands::set_bold::SetBoldCommand;
use crate::commands::set_paragraph_alignment::SetParagraphAlignmentCommand;
use crate::commands::table_structure::{TableStructureCommand, TableStructureOp};
use crate::commands::{Command, CommandContext, CommandExecutor};
use crate::field_datetime::FieldDateTime;
use crate::revision_clock::set_revision_clock;
use futures::executor;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

const EXISTING_DATE: &str = "2001-02-03T04:05:00Z";

fn host_clock() -> FieldDateTime {
    FieldDateTime {
        year: 2026,
        month: 11,
        day: 20,
        hour: 21,
        minute: 7,
        second: 42,
        weekday: 5,
    }
}

fn document() -> WordDocument {
    let body = format!(
        r#"<w:p><w:r><w:t>Hello world</w:t></w:r></w:p>
<w:p><w:r><w:t>Second</w:t></w:r></w:p>
<w:p><w:r><w:t>Third</w:t></w:r></w:p>
<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>
<w:tr><w:tc><w:p><w:r><w:t>one</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>two</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
<w:p><w:ins w:id="90" w:author="Old" w:date="{EXISTING_DATE}"><w:r><w:t>Kept</w:t></w:r></w:ins></w:p>"#
    );
    executor::block_on(WordDocument::load(
        &docx_with_body(&body),
        &Options::default(),
    ))
    .unwrap()
}

fn at(paragraph: usize, offset: usize) -> TextPosition {
    TextPosition::new(paragraph, offset)
}

fn range(start: TextPosition, end: TextPosition) -> SelectionRange {
    SelectionRange::new(start, end)
}

/// Records one change of every kind jade tracks and returns the saved
/// `word/document.xml`.
fn record_every_kind(session: RevisionSession) -> String {
    let mut document = document();
    document.set_revision_session(Some(session));
    let mut executor = CommandExecutor::new();
    let mut selection = None;
    let commands: Vec<Box<dyn Command>> = vec![
        // The paragraph mark between "Hello world" and "Second" deleted.
        Box::new(DeleteTextCommand::new(
            SelectionRange::collapsed(at(1, 0)),
            DeleteDirection::Backward,
        )),
        Box::new(InsertTextCommand::new(
            SelectionRange::collapsed(at(0, 0)),
            "X".into(),
            None,
        )),
        // "world" deleted.
        Box::new(DeleteTextCommand::new(
            range(at(0, 7), at(0, 12)),
            DeleteDirection::Forward,
        )),
        Box::new(SetBoldCommand::new(range(at(1, 0), at(1, 6)), true)),
        Box::new(SetParagraphAlignmentCommand::new(1, 1, "center")),
        Box::new(InsertParagraphBreakCommand::new(SelectionRange::collapsed(
            at(2, 2),
        ))),
        Box::new(TableStructureCommand::new(
            0,
            TableStructureOp::InsertRow {
                row_index: 2,
                template_row: Some(1),
            },
        )),
        Box::new(TableStructureCommand::new(
            0,
            TableStructureOp::DeleteRow { row_index: 0 },
        )),
    ];
    for command in commands {
        let name = command.name();
        executor
            .execute(
                command,
                &mut CommandContext::new(&mut document, &mut selection),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
    let saved = document.export_docx().unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(saved)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("word/document.xml").unwrap(), &mut xml)
        .unwrap();
    xml
}

/// A revision element in the saved XML: where it sits, and its attributes.
struct Revision {
    /// `parent/name`, e.g. `p/ins`, `rPr/del` (a paragraph mark), `trPr/ins`.
    path: String,
    author: Option<String>,
    date: Option<String>,
}

fn revisions(xml: &str) -> Vec<Revision> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut found = Vec::new();
    loop {
        let (element, empty) = match reader.read_event().unwrap() {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => {
                stack.pop();
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let name = String::from_utf8(element.name().as_ref().to_vec()).unwrap();
        let local = name.trim_start_matches("w:").to_string();
        if matches!(
            local.as_str(),
            "ins" | "del" | "rPrChange" | "pPrChange" | "trPrChange" | "tcPrChange"
        ) {
            let attribute = |key: &str| {
                element
                    .attributes()
                    .flatten()
                    .find(|attribute| attribute.key.as_ref() == key.as_bytes())
                    .map(|attribute| String::from_utf8(attribute.value.to_vec()).unwrap())
            };
            found.push(Revision {
                path: format!("{}/{local}", stack.last().map_or("", String::as_str)),
                author: attribute("w:author"),
                date: attribute("w:date"),
            });
        }
        if !empty {
            stack.push(local);
        }
    }
    found
}

/// `YYYY-MM-DDThh:mm:ssZ`, the form Word writes.
fn is_word_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 20
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
}

fn reviewer() -> RevisionSession {
    RevisionSession {
        author: Some("Reviewer".into()),
        date: None,
        date_utc: None,
    }
}

#[test]
fn every_recorded_revision_is_dated_by_the_host_clock() {
    set_revision_clock(Some(host_clock));
    let xml = record_every_kind(reviewer());
    set_revision_clock(None);
    let ours: Vec<_> = revisions(&xml)
        .into_iter()
        .filter(|revision| revision.author.as_deref() == Some("Reviewer"))
        .collect();
    for path in [
        "p/ins",
        "p/del",
        "rPr/rPrChange",
        "pPr/pPrChange",
        "rPr/ins",
        "rPr/del",
        "trPr/ins",
        "trPr/del",
    ] {
        assert!(
            ours.iter().any(|revision| revision.path == path),
            "no {path} recorded:\n{xml}"
        );
    }
    for revision in &ours {
        let date = revision
            .date
            .as_deref()
            .unwrap_or_else(|| panic!("{} has no w:date:\n{xml}", revision.path));
        assert!(is_word_date(date), "{}: {date}", revision.path);
    }
}

#[test]
fn a_revision_read_from_the_document_keeps_its_date() {
    set_revision_clock(Some(host_clock));
    let xml = record_every_kind(reviewer());
    set_revision_clock(None);
    let old: Vec<_> = revisions(&xml)
        .into_iter()
        .filter(|revision| revision.author.as_deref() == Some("Old"))
        .collect();
    assert_eq!(old.len(), 1, "{xml}");
    assert_eq!(old[0].date.as_deref(), Some(EXISTING_DATE));
}

#[test]
fn a_session_with_a_fixed_date_keeps_it() {
    set_revision_clock(Some(host_clock));
    let xml = record_every_kind(RevisionSession {
        date: Some("2026-02-03T00:00:00Z".into()),
        ..reviewer()
    });
    set_revision_clock(None);
    for revision in revisions(&xml)
        .iter()
        .filter(|revision| revision.author.as_deref() == Some("Reviewer"))
    {
        assert_eq!(
            revision.date.as_deref(),
            Some("2026-02-03T00:00:00Z"),
            "{}",
            revision.path
        );
    }
}

#[test]
fn the_host_clock_is_written_to_the_minute_with_a_z_suffix() {
    set_revision_clock(Some(host_clock));
    let date = crate::revision_clock::revision_date_now();
    set_revision_clock(None);
    // Word's local wall clock with `Z` (the comment convention), seconds
    // dropped as Word does for revisions.
    assert_eq!(date.as_deref(), Some("2026-11-20T21:07:00Z"));
}
