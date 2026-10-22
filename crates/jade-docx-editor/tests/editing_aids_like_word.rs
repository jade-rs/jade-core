//! Word's 検索と置換 with its オプション, AutoCorrect's edits and the
//! drag-and-drop move, through the shared model on a blank document — as
//! either shell drives them.

use std::future::Future;

use jade_docx_editor::commands::{Command, CommandContext, CommandExecutor};
use jade_docx_editor::word::autocorrect::{
    corrections, list_start, on_enter, AutoCorrectOptions, WordAutoCorrect,
};
use jade_docx_editor::word::clipboard::WordClipboard;
use jade_docx_editor::word::new::{blank_document, Blank};
use jade_docx_editor::word::search::{Direction, FormatFilter, Scope, SearchOptions, WordSearch};
use jade_docx_editor::word::{CellLayout, WordSession};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::HeadlessCommand;
use jade_selection::{SelectionRange, TextPosition};

struct Session {
    document: WordDocument,
    selection: Option<SelectionRange>,
    executor: CommandExecutor,
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
    }
}

impl WordSession for Session {
    fn word_document(&self) -> &WordDocument {
        &self.document
    }
    fn selection_range(&self) -> Option<SelectionRange> {
        self.selection
    }
    fn set_selection_range(&mut self, range: Option<SelectionRange>) {
        self.selection = range;
    }
    fn apply_command(&mut self, command: HeadlessCommand, keep: bool) -> Result<(), String> {
        self.apply_commands(vec![command], keep)
    }
    fn apply_commands(
        &mut self,
        commands: Vec<HeadlessCommand>,
        _keep: bool,
    ) -> Result<(), String> {
        jade_docx_editor::headless_api::apply_headless_commands_to_document(
            &mut self.document,
            &jade_docx_headless_api::HeadlessEditRequest { commands },
        )
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
    }
    fn execute_command(&mut self, command: Box<dyn Command>, keep: bool) -> Result<(), String> {
        let before = self.selection;
        let mut ctx = CommandContext::new(&mut self.document, &mut self.selection);
        self.executor
            .execute(command, &mut ctx)
            .map_err(|error| format!("{error:?}"))?;
        if keep {
            self.selection = before;
        }
        Ok(())
    }
    fn cell_id_for_paragraph(&self, _paragraph: usize) -> Option<usize> {
        None
    }
    fn cell_layout(&self, _cell_id: usize) -> Option<&CellLayout> {
        None
    }
    fn cell_layouts(&self) -> &[CellLayout] {
        &[]
    }
    fn table_order_for(&self, _table_id: usize) -> Option<usize> {
        None
    }
    fn hyperlink_regions(&self) -> Vec<jade_docx_editor::canvas_renderer::HyperlinkRegion> {
        Vec::new()
    }
    fn select_cells(&mut self, _anchor_cell: usize, _focus_cell: usize) -> Result<(), String> {
        Err("no tables in this session".into())
    }
    fn page_count(&self) -> Option<usize> {
        Some(2)
    }
    fn paragraph_page(&self, _paragraph: usize) -> Option<usize> {
        Some(1)
    }
}

impl Session {
    fn with(paragraphs: &[&str]) -> Self {
        let document = block_on(WordDocument::load(
            &blank_document(Blank::Japanese),
            &Options::default(),
        ))
        .expect("a blank document loads");
        let mut session = Self {
            document,
            selection: Some(SelectionRange::collapsed(TextPosition::new(0, 0))),
            executor: CommandExecutor::new(),
        };
        for (index, text) in paragraphs.iter().enumerate() {
            if index > 0 {
                let previous = paragraphs[index - 1].len();
                session
                    .apply_command(
                        HeadlessCommand::InsertParagraphBreak {
                            paragraph: index - 1,
                            offset: previous,
                            count: 1,
                        },
                        false,
                    )
                    .unwrap();
            }
            session
                .apply_command(
                    HeadlessCommand::InsertText {
                        paragraph: index,
                        offset: 0,
                        text: text.to_string(),
                    },
                    false,
                )
                .unwrap();
        }
        session
    }

    fn text(&self, index: usize) -> String {
        self.document
            .paragraph(index)
            .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
            .unwrap_or_default()
    }
}

fn wildcards() -> SearchOptions {
    SearchOptions {
        wildcards: true,
        ..SearchOptions::default()
    }
}

#[test]
fn wildcards_find_and_groups_replace() {
    let mut session = Session::with(&["Smith, John", "Doe, Jane"]);
    let hits = session.search("(<*>), (<*>)", &wildcards()).unwrap();
    assert_eq!(hits.len(), 2);
    let replaced = session
        .replace_every("(<*>), (<*>)", "\\2 \\1", &wildcards())
        .unwrap();
    assert_eq!(replaced, 2);
    assert_eq!(session.text(0), "John Smith");
    assert_eq!(session.text(1), "Jane Doe");
}

#[test]
fn paragraph_marks_are_found_and_put_in() {
    let mut session = Session::with(&["one", "two"]);
    let options = SearchOptions::default();
    let hits = session.search("e^pt", &options).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].start, hits[0].end), ((0, 2), (1, 1)));
    // Joining two paragraphs: their mark replaced with a space.
    session.replace_every("^p", " ", &options).unwrap();
    assert_eq!(session.text(0), "one two");
    // And a mark put in where a comma was.
    let mut session = Session::with(&["a,b"]);
    session.replace_every(",", "^p", &options).unwrap();
    assert_eq!(
        (session.text(0), session.text(1)),
        ("a".to_string(), "b".to_string())
    );
}

#[test]
fn formatting_is_found_and_given() {
    let mut session = Session::with(&["plain bold plain"]);
    session
        .apply_command(
            HeadlessCommand::ApplyBold {
                start_paragraph: 0,
                start_offset: 6,
                end_paragraph: 0,
                end_offset: 10,
                enabled: true,
            },
            false,
        )
        .unwrap();
    let bold = SearchOptions {
        format: FormatFilter {
            bold: Some(true),
            ..FormatFilter::default()
        },
        ..SearchOptions::default()
    };
    // Bold text alone, with no words to find.
    let hits = session.search("", &bold).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].found, "bold");
    // "plain" only where it is bold: nowhere.
    assert!(session.search("plain", &bold).unwrap().is_empty());
    // Replace with formatting: every "plain" italic, its text kept.
    let italic = SearchOptions {
        replace_format: FormatFilter {
            italic: Some(true),
            ..FormatFilter::default()
        },
        ..SearchOptions::default()
    };
    assert_eq!(session.replace_every("plain", "", &italic).unwrap(), 2);
    assert_eq!(session.text(0), "plain bold plain");
    let italic_found = SearchOptions {
        format: FormatFilter {
            italic: Some(true),
            ..FormatFilter::default()
        },
        ..SearchOptions::default()
    };
    assert_eq!(session.search("", &italic_found).unwrap().len(), 2);
}

#[test]
fn direction_and_selection_scope() {
    let mut session = Session::with(&["a x a x a"]);
    let mut options = SearchOptions::default();
    let first = session
        .search_next("a", &options, (0, 1), None)
        .unwrap()
        .unwrap();
    assert_eq!(first.start, (0, 4));
    options.direction = Direction::Up;
    let back = session
        .search_next("a", &options, (0, 4), None)
        .unwrap()
        .unwrap();
    assert_eq!(back.start, (0, 0));
    options.direction = Direction::Down;
    assert!(session
        .search_next("a", &options, (0, 9), None)
        .unwrap()
        .is_none());
    options.direction = Direction::All;
    let wrapped = session
        .search_next("a", &options, (0, 9), None)
        .unwrap()
        .unwrap();
    assert_eq!(wrapped.start, (0, 0));
    // Within the selection only.
    session.selection = Some(SelectionRange::new(
        TextPosition::new(0, 2),
        TextPosition::new(0, 7),
    ));
    let within = SearchOptions {
        scope: Scope::Selection,
        ..SearchOptions::default()
    };
    let hits = session.search("a", &within).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].start, (0, 4));
}

#[test]
fn autocorrect_edits_the_document() {
    let mut session = Session::with(&["1. "]);
    let options = AutoCorrectOptions::default();
    let list = list_start(&session.text(0), 3, &options).unwrap();
    session.start_list(0, &list).unwrap();
    assert_eq!(session.text(0), "");
    let numbered = session
        .document
        .paragraph(0)
        .and_then(|p| p.properties.as_ref())
        .and_then(|props| props.numbering.as_ref())
        .is_some();
    assert!(numbered);

    let mut session = Session::with(&["the 2nd "]);
    let fixes = corrections(&session.text(0), 8, &options);
    session.apply_fixes(0, &fixes, 8).unwrap();
    assert_eq!(session.text(0), "the 2nd ");
    let raised = session
        .document
        .paragraph(0)
        .unwrap()
        .runs
        .iter()
        .any(|run| {
            run.text == "nd"
                && run
                    .properties
                    .as_ref()
                    .and_then(|props| props.baseline.as_deref())
                    == Some("superscript")
        });
    assert!(
        raised,
        "{:?}",
        session
            .document
            .paragraph(0)
            .unwrap()
            .runs
            .iter()
            .map(|r| (
                &r.text,
                r.properties.as_ref().and_then(|p| p.baseline.clone())
            ))
            .collect::<Vec<_>>()
    );

    let mut session = Session::with(&["記", ""]);
    let format = on_enter("記", "", &options).unwrap();
    session.format_on_enter(0, &format).unwrap();
    assert_eq!(session.text(2), "以上");
}

#[test]
fn a_drop_moves_text_in_one_step() {
    let mut session = Session::with(&["abc def ghi"]);
    session.selection = Some(SelectionRange::new(
        TextPosition::new(0, 0),
        TextPosition::new(0, 4),
    ));
    let clip = session.copy_rich().unwrap();
    let end = session
        .drop_rich(&clip, Some(((0, 0), (0, 4))), (0, 8))
        .unwrap();
    assert_eq!(session.text(0), "def abc ghi");
    assert_eq!(end, (0, 8));
}

#[test]
fn a_language_is_set_and_detected() {
    use jade_docx_editor::word::language::{LanguageChoice, WordLanguage};
    let mut session = Session::with(&["日本語 and English"]);
    let length = session.text(0).len();
    session
        .set_language(
            (0, 0),
            (0, length),
            &LanguageChoice {
                language: "en-US".to_string(),
                no_proof: false,
                detect: true,
            },
        )
        .unwrap();
    assert_eq!(session.language_at((0, 0)).as_deref(), Some("ja-JP"));
    assert_eq!(session.language_at((0, 12)).as_deref(), Some("en-US"));
    session
        .set_language(
            (0, 0),
            (0, length),
            &LanguageChoice {
                language: "en-GB".to_string(),
                no_proof: true,
                detect: false,
            },
        )
        .unwrap();
    assert_eq!(session.language_at((0, 12)).as_deref(), Some("en-GB"));
}
