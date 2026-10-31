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

mod stateful {
    use super::*;
    use jade_docx_editor::fields::FieldInfo;
    use proptest::prelude::*;

    #[derive(Clone, Copy, Debug)]
    enum Code {
        Name,
        Address,
        Number,
        Date,
        Fill,
    }

    impl Code {
        fn instruction(self) -> &'static str {
            match self {
                Self::Name => "USERNAME",
                Self::Address => "USERADDRESS",
                Self::Number => "= 6 * 7",
                Self::Date => "DATE \\@ \"yyyy-MM-dd\"",
                Self::Fill => "FILLIN \"宛先\" \\d \"初期値\"",
            }
        }

        // An independent oracle: no production field parser or evaluator.
        fn result(self, version: u8) -> String {
            match self {
                Self::Name => names(version).into(),
                Self::Address => addresses(version).into(),
                Self::Number => "42".into(),
                Self::Date => format!("2026-10-{:02}", 21 + version % 3),
                Self::Fill => "初期値".into(),
            }
        }

        fn name(self) -> &'static str {
            match self {
                Self::Name => "USERNAME",
                Self::Address => "USERADDRESS",
                Self::Number => "=",
                Self::Date => "DATE",
                Self::Fill => "FILLIN",
            }
        }
    }

    fn names(version: u8) -> &'static str {
        ["", "A", "山田 花子", "é🦀<&>\""][usize::from(version % 4)]
    }

    fn addresses(version: u8) -> &'static str {
        ["", "東京都", "一行目\n二行目", "a\tb<&>"][usize::from(version % 4)]
    }

    fn environment(version: u8) -> FieldHost {
        let mut host = host();
        host.user_name = names(version).into();
        host.user_address = addresses(version).into();
        host.now.day = 21 + u32::from(version % 3);
        host
    }

    #[derive(Clone, Debug)]
    enum Op {
        Insert(Code, u8),
        Edit(Code, u8),
        EmptyCode,
        MalformedCode(u8),
        Update(u8),
        Lock(bool),
        Unlink,
        Undo,
        Redo,
        Reload,
    }

    const MALFORMED_CODES: &[&str] = &[
        "{",
        "}",
        "= { = 1",
        "= 1 }",
        "= } { 1",
        "QUOTE \"a\u{0}b\"",
        "QUOTE \"a\u{1}b\"",
        "QUOTE \"a\u{b}b\"",
        "QUOTE \"a\u{c}b\"",
        "QUOTE \"a\u{1f}b\"",
        "QUOTE \"a\u{fffe}b\"",
        "QUOTE \"a\u{ffff}b\"",
    ];

    fn operations() -> impl Strategy<Value = Vec<Op>> {
        let code = prop_oneof![
            Just(Code::Name),
            Just(Code::Address),
            Just(Code::Number),
            Just(Code::Date),
            Just(Code::Fill),
        ];
        prop::collection::vec(
            prop_oneof![
                3 => (code.clone(), 0u8..4).prop_map(|(c, v)| Op::Insert(c, v)),
                3 => (code, 0u8..4).prop_map(|(c, v)| Op::Edit(c, v)),
                1 => Just(Op::EmptyCode),
                2 => (0u8..MALFORMED_CODES.len() as u8).prop_map(Op::MalformedCode),
                3 => (0u8..4).prop_map(Op::Update),
                2 => any::<bool>().prop_map(Op::Lock),
                2 => Just(Op::Unlink),
                3 => Just(Op::Undo),
                2 => Just(Op::Redo),
                2 => Just(Op::Reload),
            ],
            1..49,
        )
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    struct State {
        field: Option<FieldInfo>,
        plain: String,
    }

    fn observe(session: &mut Session) -> State {
        session.whole_paragraph();
        let fields = session.selected_fields();
        assert!(fields.len() <= 1, "unexpected extra fields: {fields:?}");
        State {
            field: fields.into_iter().next(),
            plain: session
                .document
                .paragraph(0)
                .unwrap()
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect(),
        }
    }

    // Opt-in corpus for the external Open XML SDK gate. This does no IO in
    // ordinary test runs and adds nothing to the editor/export path.
    fn capture(bytes: &[u8], context: &str) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        if let Some(directory) = std::env::var_os("JADE_FIELD_PBT_EXPORT_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let index = NEXT.fetch_add(1, Ordering::Relaxed);
            std::fs::write(
                directory.join(format!("field-sequence-{index:06}.docx")),
                bytes,
            )
            .unwrap();
            std::fs::write(
                directory.join(format!("field-sequence-{index:06}.txt")),
                context,
            )
            .unwrap();
        }
    }

    fn run(ops: &[Op]) {
        let mut session = Session::blank();
        let mut state = State::default();
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        for (step, op) in ops.iter().enumerate() {
            session.whole_paragraph();
            let before = state.clone();
            let mut records_history = true;
            let mut expected_success = true;
            let result = match *op {
                Op::Insert(code, version) => {
                    // Insertion without a caret must be atomic, including history.
                    if state.field.is_some() {
                        session.selection = None;
                        expected_success = false;
                    } else {
                        state.field = Some(FieldInfo {
                            code: code.instruction().into(),
                            name: code.name().into(),
                            result: code.result(version),
                            locked: false,
                        });
                        state.plain = if matches!(code, Code::Fill) {
                            code.result(version)
                        } else {
                            String::new()
                        };
                    }
                    session.insert_field_code(code.instruction(), &environment(version))
                }
                Op::Edit(code, version) => {
                    if let Some(field) = &mut state.field {
                        field.code = code.instruction().into();
                        field.name = code.name().into();
                        // Without a prompt answer, FILLIN preserves a previous
                        // nonempty answer; its default only fills an empty one.
                        if !field.locked && (!matches!(code, Code::Fill) || field.result.is_empty())
                        {
                            field.result = code.result(version);
                        }
                        state.plain = if matches!(code, Code::Fill) {
                            field.result.clone()
                        } else {
                            String::new()
                        };
                    } else {
                        expected_success = false;
                    }
                    session.edit_field_code(code.instruction(), &environment(version))
                }
                Op::EmptyCode => {
                    expected_success = false;
                    session.edit_field_code("  {}  ", &environment(0))
                }
                Op::MalformedCode(index) => {
                    expected_success = false;
                    let code = MALFORMED_CODES[usize::from(index)];
                    if state.field.is_some() {
                        session.edit_field_code(code, &environment(0))
                    } else {
                        session.insert_field_code(code, &environment(0))
                    }
                }
                Op::Update(version) => {
                    if let Some(field) = &mut state.field {
                        if !field.locked {
                            let code = match field.name.as_str() {
                                "USERNAME" => Code::Name,
                                "USERADDRESS" => Code::Address,
                                "=" => Code::Number,
                                "DATE" => Code::Date,
                                "FILLIN" => Code::Fill,
                                _ => unreachable!(),
                            };
                            if !matches!(code, Code::Fill) || field.result.is_empty() {
                                field.result = code.result(version);
                            }
                        }
                        if field.name == "FILLIN" {
                            state.plain = field.result.clone();
                        }
                    }
                    session.update_fields(
                        session.field_scope(),
                        &environment(version),
                        HashMap::new(),
                    )
                }
                Op::Lock(locked) => {
                    if let Some(field) = &mut state.field {
                        field.locked = locked;
                    }
                    session.lock_fields(locked)
                }
                Op::Unlink => {
                    if let Some(field) = state.field.take() {
                        state.plain = field.result;
                    }
                    session.unlink_fields()
                }
                Op::Undo | Op::Redo => {
                    records_history = false;
                    let (from, to) = if matches!(op, Op::Undo) {
                        (&mut undo, &mut redo)
                    } else {
                        (&mut redo, &mut undo)
                    };
                    let expected_changed = if let Some(previous) = from.pop() {
                        to.push(std::mem::replace(&mut state, previous));
                        true
                    } else {
                        false
                    };
                    let mut ctx =
                        CommandContext::new(&mut session.document, &mut session.selection);
                    let changed = if matches!(op, Op::Undo) {
                        session.executor.undo(&mut ctx)
                    } else {
                        session.executor.redo(&mut ctx)
                    }
                    .unwrap();
                    assert_eq!(
                        changed, expected_changed,
                        "step {step}: {op:?}, ops={ops:?}"
                    );
                    Ok(())
                }
                Op::Reload => {
                    records_history = false;
                    session.document = block_on(WordDocument::load(
                        &session.document.export_docx().unwrap(),
                        &Options::default(),
                    ))
                    .unwrap();
                    session.executor = CommandExecutor::new();
                    undo.clear();
                    redo.clear();
                    Ok(())
                }
            };
            assert_eq!(
                result.is_ok(),
                expected_success,
                "step {step}: {op:?}: {result:?}, before={before:?}, ops={ops:?}"
            );
            if records_history && expected_success {
                undo.push(before);
                redo.clear();
            }
            assert_eq!(
                observe(&mut session),
                state,
                "step {step}: {op:?}, ops={ops:?}"
            );
            assert_eq!(
                session.executor.undo_depth(),
                undo.len(),
                "step {step}: {op:?}"
            );
            // Cross the actual DOCX persistence boundary after every operation.
            let bytes = session.document.export_docx().unwrap();
            capture(&bytes, &format!("step {step}: {op:?}, ops={ops:?}"));
            let mut reloaded = Session {
                document: block_on(WordDocument::load(&bytes, &Options::default())).unwrap(),
                selection: None,
                executor: CommandExecutor::new(),
            };
            assert_eq!(
                observe(&mut reloaded),
                state,
                "reloaded at step {step}: {op:?}, ops={ops:?}"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"))),
            ..ProptestConfig::with_cases(64)
        })]
        #[test]
        fn field_operation_sequences_match_model(ops in operations()) {
            run(&ops);
        }

        #[test]
        fn trailing_nested_formula_is_preserved(value in -100i32..101, depth in 1usize..5, wrapped in any::<bool>()) {
            let mut code = format!("= {value}");
            for _ in 0..depth {
                code = format!("= {{ {code} }}");
            }
            let supplied = if wrapped { format!("{{ {code} }}") } else { code };
            let mut session = Session::blank();
            session.insert_field_code(&supplied, &host()).unwrap();
            session.whole_paragraph();
            prop_assert_eq!(&session.selected_fields()[0].result, &value.to_string(), "code={}", supplied);
            capture(&session.document.export_docx().unwrap(), &supplied);
            // Editing the same instruction and adding/removing an outer
            // format must also preserve the nested operand.
            let formatted = jade_docx_editor::word::fields::toggle_switch(&supplied, "\\*", Some("Arabic"));
            session.edit_field_code(&formatted, &host()).unwrap();
            session.whole_paragraph();
            prop_assert_eq!(&session.selected_fields()[0].result, &value.to_string());
            let restored = jade_docx_editor::word::fields::toggle_switch(&formatted, "\\*", Some("Arabic"));
            session.edit_field_code(&restored, &host()).unwrap();
            capture(&session.document.export_docx().unwrap(), &restored);
            session.document = block_on(WordDocument::load(
                &session.document.export_docx().unwrap(), &Options::default(),
            )).unwrap();
            session.whole_paragraph();
            prop_assert_eq!(&session.selected_fields()[0].result, &value.to_string());
        }

        #[test]
        fn outer_switches_preserve_nested_switches(value in 0u32..100, depth in 1usize..5) {
            use jade_docx_editor::word::fields::{code_has_switch, toggle_switch};
            let mut code = format!("= {value} \\* Roman");
            for _ in 0..depth {
                code = format!("= {{ {code} }}");
            }
            prop_assert!(!code_has_switch(&code, "\\*", Some("Roman")));
            let added = toggle_switch(&code, "\\*", Some("Arabic"));
            prop_assert_eq!(&added, &format!("{code} \\* Arabic"));
            prop_assert_eq!(toggle_switch(&added, "\\*", Some("Arabic")), code);
        }

        #[test]
        fn quoted_braces_and_escaped_quotes_are_literal(
            value in prop::collection::vec(prop::sample::select(vec!['a', ' ', '"', '{', '}', '山', '🦀']), 0..20),
        ) {
            let value: String = value.into_iter().collect();
            let code = format!("QUOTE \"{}\"", value.replace('"', "\\\""));
            let mut session = Session::blank();
            session.insert_field_code(&code, &host()).unwrap();
            session.whole_paragraph();
            prop_assert_eq!(&session.selected_fields()[0].result, &value, "code={}", code);
            // Replace a different instruction too, to cover code replacement
            // rather than just a newly constructed field.
            session.edit_field_code("= 42", &host()).unwrap();
            session.edit_field_code(&code, &host()).unwrap();
            session.whole_paragraph();
            prop_assert_eq!(&session.selected_fields()[0].result, &value);
            capture(&session.document.export_docx().unwrap(), &code);
        }

        #[test]
        fn host_values_omit_characters_xml_cannot_carry(
            value in prop::collection::vec(prop::sample::select(vec![
                'a', '山', '🦀', '&', '<', '>', '\t', '\n', '\r',
                '\u{0}', '\u{1}', '\u{b}', '\u{c}', '\u{1f}', '\u{fffe}', '\u{ffff}',
            ]), 0..20),
        ) {
            let value: String = value.into_iter().collect();
            let expected: String = value.chars().filter(|ch| !matches!(
                ch, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}'
            )).collect();
            let mut environment = host();
            environment.user_name = value.clone();
            let mut session = Session::blank();
            session.insert_field_code("USERNAME", &environment).unwrap();
            let bytes = session.document.export_docx().unwrap();
            capture(&bytes, &format!("user_name={value:?}"));
            prop_assert_eq!(&observe(&mut session).field.unwrap().result, &expected);
            session.document = block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
            prop_assert_eq!(&observe(&mut session).field.unwrap().result, &expected);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"))),
            ..ProptestConfig::with_cases(64)
        })]
        #[test]
        fn insertion_and_unlink_preserve_surrounding_text(
            before in prop::collection::vec(prop::sample::select(vec!['a', '山', '🦀', '\t', '\n']), 0..12),
            after in prop::collection::vec(prop::sample::select(vec!['b', '川', '🦀', '\t', '\n']), 0..12),
        ) {
            let before: String = before.into_iter().collect();
            let after: String = after.into_iter().collect();
            let text = format!("{before}{after}");
            let mut session = Session::blank();
            session.type_text(&text);
            prop_assert_eq!(&observe(&mut session).plain, &text);
            session.selection = Some(SelectionRange::collapsed(TextPosition::new(0, before.len())));
            session.insert_field_code("= 42", &host()).unwrap();
            prop_assert_eq!(&observe(&mut session).plain, &text);
            session.unlink_fields().unwrap();
            let expected = format!("{before}42{after}");
            prop_assert_eq!(&observe(&mut session).plain, &expected);
            let bytes = session.document.export_docx().unwrap();
            capture(&bytes, &format!("before={before:?}, after={after:?}"));
            session.document = block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
            prop_assert_eq!(&observe(&mut session).plain, &expected);
        }
    }

    #[test]
    fn malformed_codes_preserve_document_and_history() {
        for index in 0..MALFORMED_CODES.len() as u8 {
            run(&[
                Op::MalformedCode(index),
                Op::Insert(Code::Number, 0),
                Op::MalformedCode(index),
                Op::Undo,
                Op::Redo,
            ]);
        }
    }

    #[test]
    fn insertion_preserves_adjacent_tabs_and_breaks() {
        for text in ["\t", "\n", "山\t\n🦀"] {
            for offset in [0, text.len()] {
                let mut session = Session::blank();
                session.type_text(text);
                session.caret(offset);
                session.insert_field_code("= 42", &host()).unwrap();
                assert_eq!(observe(&mut session).plain, text);
                session.unlink_fields().unwrap();
                assert_eq!(
                    observe(&mut session).plain,
                    format!("{}42{}", &text[..offset], &text[offset..])
                );
            }
        }
    }

    #[test]
    fn insertion_preserves_text_matching_the_internal_placeholder() {
        let mut session = Session::blank();
        let text = "before\u{E000}jade-field\u{E001}after\u{E000}jade-field\u{E001}0";
        session.type_text(text);
        session.insert_field_code("= 42", &host()).unwrap();
        session.whole_paragraph();
        assert_eq!(observe(&mut session).plain, text);
        capture(&session.document.export_docx().unwrap(), text);
        session.unlink_fields().unwrap();
        assert_eq!(observe(&mut session).plain, format!("{text}42"));
    }
}
