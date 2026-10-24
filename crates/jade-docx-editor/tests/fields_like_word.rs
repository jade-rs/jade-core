//! Word's fields through the shared model: inserting from the Field
//! dialog's code, updating with F9 (prompts answered), codes toggled,
//! locked, unlinked, and the document's properties feeding the property
//! fields — in a session over a blank Japanese document, as either shell
//! drives it.

use std::collections::HashMap;
use std::future::Future;

use jade_docx_editor::commands::{Command, CommandContext, CommandExecutor};
use jade_docx_editor::field_datetime::FieldDateTime;
use jade_docx_editor::word::fields::{
    build_code, CustomProperty, CustomValue, FieldCodeView, FieldHost, WordFields,
};
use jade_docx_editor::word::new::{blank_document, Blank};
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
    fn blank() -> Self {
        let document = block_on(WordDocument::load(
            &blank_document(Blank::Japanese),
            &Options::default(),
        ))
        .expect("a blank document loads");
        Self {
            document,
            selection: Some(SelectionRange::collapsed(TextPosition::new(0, 0))),
            executor: CommandExecutor::new(),
        }
    }

    fn caret(&mut self, offset: usize) {
        self.selection = Some(SelectionRange::collapsed(TextPosition::new(0, offset)));
    }

    fn type_text(&mut self, text: &str) {
        let at = self.selection.unwrap().focus;
        self.apply_command(
            HeadlessCommand::InsertText {
                paragraph: at.paragraph,
                offset: at.offset,
                text: text.to_string(),
            },
            false,
        )
        .unwrap();
        self.caret(at.offset + text.len());
    }

    fn whole_paragraph(&mut self) {
        let length = self
            .document
            .paragraph(0)
            .map(|paragraph| paragraph.runs.iter().map(|run| run.text.len()).sum())
            .unwrap_or(0);
        self.selection = Some(SelectionRange::new(
            TextPosition::new(0, 0),
            TextPosition::new(0, length),
        ));
    }

    fn undo(&mut self) {
        let mut ctx = CommandContext::new(&mut self.document, &mut self.selection);
        assert!(self.executor.undo(&mut ctx).unwrap());
    }
}

fn host() -> FieldHost {
    FieldHost {
        now: FieldDateTime {
            year: 2026,
            month: 10,
            day: 21,
            hour: 9,
            minute: 41,
            second: 0,
            weekday: 3,
        },
        utc_offset_seconds: 9 * 3600,
        user_name: "山田 花子".into(),
        user_initials: String::new(),
        user_address: "東京都".into(),
        file: None,
        japanese: true,
        merge: None,
    }
}

#[test]
fn the_field_dialog_inserts_a_date_that_updates() {
    let mut session = Session::blank();
    session.type_text("作成日: ");
    let code = build_code(
        "DATE",
        &[],
        &[("\\@".into(), Some("ggge年M月d日(aaa)".into()))],
        true,
    );
    session.insert_field_code(&code, &host()).unwrap();
    session.whole_paragraph();
    let fields = session.selected_fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(
        fields[0].code,
        "DATE \\@ \"ggge年M月d日(aaa)\" \\* MERGEFORMAT"
    );
    assert_eq!(fields[0].result, "令和8年10月21日(水)");

    // A day later, F9 brings it up to date.
    let mut tomorrow = host();
    tomorrow.now.day = 22;
    tomorrow.now.weekday = 4;
    let scope = session.field_scope();
    session
        .update_fields(scope, &tomorrow, HashMap::new())
        .unwrap();
    session.whole_paragraph();
    assert_eq!(session.selected_fields()[0].result, "令和8年10月22日(木)");

    // ⌘F11 locks it: an update leaves it be. ⌘⇧F11 lets it go again.
    session.lock_fields(true).unwrap();
    session.whole_paragraph();
    assert!(session.selected_fields()[0].locked);
    let scope = session.field_scope();
    session
        .update_fields(scope, &host(), HashMap::new())
        .unwrap();
    session.whole_paragraph();
    assert_eq!(session.selected_fields()[0].result, "令和8年10月22日(木)");
    session.lock_fields(false).unwrap();
    session.whole_paragraph();
    assert!(!session.selected_fields()[0].locked);

    // ⌘⇧F9 leaves the date as text; undo brings the field back.
    session.unlink_fields().unwrap();
    session.whole_paragraph();
    assert!(session.selected_fields().is_empty());
    session.undo();
    session.whole_paragraph();
    assert_eq!(session.selected_fields().len(), 1);
}

#[test]
fn user_information_and_page_fields_come_from_the_host_and_layout() {
    let mut session = Session::blank();
    for code in ["USERNAME", "USERINITIALS", "PAGE", "NUMPAGES \\* KANJINUM1"] {
        session.insert_field_code(code, &host()).unwrap();
    }
    session.whole_paragraph();
    let results: Vec<(String, String)> = session
        .selected_fields()
        .into_iter()
        .map(|field| (field.name, field.result))
        .collect();
    assert!(
        results.contains(&("USERNAME".into(), "山田 花子".into())),
        "{results:?}"
    );
    assert!(
        results.contains(&("USERINITIALS".into(), "山花".into())),
        "{results:?}"
    );
    assert!(
        results.contains(&("PAGE".into(), "1".into())),
        "{results:?}"
    );
    assert!(
        results.contains(&("NUMPAGES".into(), "二".into())),
        "{results:?}"
    );
}

#[test]
fn fill_in_prompts_are_answered_before_the_update() {
    let mut session = Session::blank();
    session
        .insert_field_code("FILLIN \"宛先は?\" \\d \"御中\"", &host())
        .unwrap();
    session.whole_paragraph();
    let scope = session.field_scope();
    let prompts = session.field_prompts(scope);
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].prompt, "宛先は?");
    let mut answers = HashMap::new();
    answers.insert(prompts[0].key.clone(), "株式会社ジェイド".to_string());
    session.update_fields(scope, &host(), answers).unwrap();
    session.whole_paragraph();
    assert_eq!(session.selected_fields()[0].result, "株式会社ジェイド");
}

#[test]
fn properties_feed_property_fields_and_codes_toggle() {
    let mut session = Session::blank();
    let mut properties = session.document_properties();
    properties.title = "見積書".into();
    properties.company = "Jade".into();
    properties.custom.push(CustomProperty {
        name: "案件番号".into(),
        value: CustomValue::Text("Q-42".into()),
    });
    session.set_document_properties(properties).unwrap();
    assert_eq!(session.document_properties().title, "見積書");
    session.insert_field_code("TITLE", &host()).unwrap();
    session
        .insert_field_code("DOCPROPERTY Company", &host())
        .unwrap();
    session
        .insert_field_code("DOCPROPERTY 案件番号", &host())
        .unwrap();
    session.whole_paragraph();
    let results: Vec<String> = session
        .selected_fields()
        .into_iter()
        .map(|field| field.result)
        .collect();
    assert_eq!(results.len(), 3);
    assert!(results.contains(&"見積書".to_string()));
    assert!(results.contains(&"Jade".to_string()));
    assert!(results.contains(&"Q-42".to_string()));

    // ⇧F9 on the selection toggles those fields' codes, and back.
    let view = session.toggled_field_codes(&FieldCodeView::default());
    assert_eq!(view.toggled.len(), 3);
    assert!(session.toggled_field_codes(&view).toggled.is_empty());

    session
        .set_document_variables(vec![("顧客".into(), "ACME".into())])
        .unwrap();
    assert_eq!(
        session.document_variables(),
        vec![("顧客".to_string(), "ACME".to_string())]
    );
}

#[test]
fn editing_a_code_reevaluates_the_field() {
    let mut session = Session::blank();
    session.insert_field_code("= 6 * 7", &host()).unwrap();
    session.whole_paragraph();
    assert_eq!(session.selected_fields()[0].result, "42");
    session
        .edit_field_code("= 6 * 7 \\* KANJINUM1", &host())
        .unwrap();
    session.whole_paragraph();
    assert_eq!(session.selected_fields()[0].result, "四十二");
}
