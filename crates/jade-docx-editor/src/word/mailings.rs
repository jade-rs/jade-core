//! The 差し込み文書 tab over any [`WordSession`]: Word's mail merge from
//! starting one — レター, 封筒, ラベル, 名簿 — through choosing recipients
//! (a list typed here, or a CSV or an Excel workbook), picking and sorting
//! and filtering them, putting merge fields, an address block, a greeting
//! line and rules in, matching fields, previewing each recipient and
//! checking for errors, to finishing: a new document, a PDF, the printer.
//! The 作成 group makes envelopes and labels on their own.
//!
//! What the fields come to and what each step does to the file are
//! `jade-docx-editor`'s (`word_document::mail_merge`). This module keeps
//! the merge's state for a document — its recipients, the record shown —
//! describes the tab and the dialogs ([`ToolGroup`], [`FormView`]) for both
//! shells to draw, and says what a shell has to do that only it can
//! ([`MailOutcome`]): ask for a file, save one, open a document, print.

mod dialogs;
mod sources;

pub use crate::word_document::mail_merge::{
    field_name, AddressBlockOptions, AddressField, EnvelopeSize, EnvelopeSpec, FieldMapping,
    GreetingLineOptions, LabelProduct, MailMergeAction, MailMergeSettings, MainDocumentType,
    MergeData, MergeFieldSpec, MergeProblem, MergeRequest, MergeRule, NameFormat, ENVELOPE_SIZES,
    LABEL_PRODUCTS, RULE_OPERATORS,
};
pub use dialogs::{
    AddressBlockDialog, EnvelopeDialog, ErrorCheckDialog, FinishDialog, GreetingDialog,
    LabelDialog, ListEditor, MatchFieldsDialog, RecipientsDialog, RuleDialog,
};
pub use sources::{csv_bytes, read_data_source, Filter, FilterOp, Recipients};

use jade_selection::{SelectionRange, TextPosition};

use super::form_view::{
    FormEvent, FormId, FormOutcome, FormView, ToolGroup, ToolItem, ToolMenuItem, Translate,
};
use super::WordSession;
use crate::text_edit::TextEdit;

/// What a shell does for the mail merge that the model cannot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailOutcome {
    /// Nothing to do but draw again.
    None,
    /// The document changed: repaint it.
    Edited,
    /// 既存のリストを使用: ask for a CSV, a text file or a workbook, and
    /// hand it to [`MailMerge::load_data_source`].
    PickDataSource,
    /// A list made here: save it as `name` (the bytes are CSV) and tell
    /// [`MailMerge::saved_data_source`] where it went.
    SaveDataSource { name: String, bytes: Vec<u8> },
    /// A new document: the merged letters, an envelope, a sheet of labels.
    OpenDocument { name: String, bytes: Vec<u8> },
    /// The merged document, to save as a PDF.
    SavePdf { name: String, bytes: Vec<u8> },
    /// The merged document (or an envelope, labels), to print.
    Print { bytes: Vec<u8> },
    /// Why it did nothing.
    Problem(String),
}

/// The 差し込み文書 tab's dialogs.
#[allow(clippy::large_enum_variant)]
pub enum MailDialog {
    Recipients(RecipientsDialog),
    List(ListEditor),
    AddressBlock(AddressBlockDialog),
    Greeting(GreetingDialog),
    Match(MatchFieldsDialog),
    Rule(RuleDialog),
    Finish(FinishDialog),
    Envelope(EnvelopeDialog),
    Labels(LabelDialog),
    Errors(ErrorCheckDialog),
}

/// Where a finished merge goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    /// 個々のドキュメントの編集: a new document.
    Document,
    /// 文書の印刷.
    Printer,
    /// PDF に差し込み.
    Pdf,
}

/// What a dialog's OK asks for.
#[derive(Clone, Debug)]
pub enum DialogAction {
    Insert(MergeFieldSpec),
    Recipients(Recipients),
    SaveList {
        data: MergeData,
        name: String,
    },
    Mapping(FieldMapping),
    Finish {
        records: Vec<usize>,
        destination: Destination,
    },
    /// An envelope or labels: made the main document (a merge), or a new
    /// document, or printed.
    Envelope {
        spec: EnvelopeSpec,
        output: LayoutOutput,
    },
    Labels {
        product: LabelProduct,
        text: Option<String>,
        output: LayoutOutput,
    },
    /// The address block's or greeting line's dialog asked to match fields.
    OpenMatch,
}

/// What an envelope or a sheet of labels becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutOutput {
    /// The main document of a merge.
    MainDocument,
    NewDocument,
    Print,
}

/// The 差し込み文書 tab's menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailMenu {
    Start,
    Recipients,
    InsertField,
    Rules,
    Finish,
}

/// The merge for one document.
pub struct MailMerge {
    pub japanese: bool,
    pub kind: Option<MainDocumentType>,
    pub recipients: Option<Recipients>,
    pub mapping: FieldMapping,
    /// 結果のプレビュー.
    pub preview: bool,
    /// The recipient shown: an index into the chosen ones.
    pub record: usize,
    /// 差し込みフィールドの強調表示.
    pub highlight: bool,
    pub dialog: Option<MailDialog>,
    pub menu: Option<MailMenu>,
    /// The data source the document names, to load when the shell can.
    pub wanted_source: Option<String>,
    /// The column names the document's mapping names, for when the data
    /// arrives.
    saved_mapping: Vec<(AddressField, String)>,
    /// A list typed here waiting to be saved before it is used.
    pending_list: Option<MergeData>,
}

const ENVELOPES: FormId = FormId::new("mail-envelopes");
const LABELS: FormId = FormId::new("mail-labels");
const START: FormId = FormId::new("mail-start");
const RECIPIENTS: FormId = FormId::new("mail-recipients");
const EDIT_RECIPIENTS: FormId = FormId::new("mail-edit-recipients");
const HIGHLIGHT: FormId = FormId::new("mail-highlight");
const ADDRESS_BLOCK: FormId = FormId::new("mail-address-block");
const GREETING: FormId = FormId::new("mail-greeting");
const INSERT_FIELD: FormId = FormId::new("mail-insert-field");
const RULES: FormId = FormId::new("mail-rules");
const MATCH: FormId = FormId::new("mail-match");
const UPDATE_LABELS: FormId = FormId::new("mail-update-labels");
const PREVIEW: FormId = FormId::new("mail-preview");
const FIRST: FormId = FormId::new("mail-first");
const PREVIOUS: FormId = FormId::new("mail-previous");
const NEXT: FormId = FormId::new("mail-next");
const LAST: FormId = FormId::new("mail-last");
const CHECK: FormId = FormId::new("mail-check");
const FINISH: FormId = FormId::new("mail-finish");

/// The rules' menu entries, in Word's order.
const RULE_ENTRIES: [&str; 9] = [
    "mail.rule.ask",
    "mail.rule.fill_in",
    "mail.rule.if",
    "mail.rule.merge_record",
    "mail.rule.merge_sequence",
    "mail.rule.next",
    "mail.rule.next_if",
    "mail.rule.set",
    "mail.rule.skip_if",
];

/// The 差し込み印刷の開始 menu's entries.
const START_ENTRIES: [(&str, Option<MainDocumentType>); 6] = [
    ("mail.start.letters", Some(MainDocumentType::Letters)),
    ("mail.start.envelopes", Some(MainDocumentType::Envelopes)),
    ("mail.start.labels", Some(MainDocumentType::Labels)),
    ("mail.start.directory", Some(MainDocumentType::Directory)),
    ("mail.start.email", Some(MainDocumentType::Email)),
    ("mail.start.normal", None),
];

fn selection_of<S: WordSession + ?Sized>(session: &S) -> SelectionRange {
    session
        .selection_range()
        .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::new(0, 0)))
}

impl MailMerge {
    /// The merge as the document keeps it: its type, preview, the record,
    /// and the data source it names (loaded later, by the shell).
    pub fn read<S: WordSession + ?Sized>(session: &S, japanese: bool) -> Self {
        let settings = session.word_document().mail_merge_settings();
        Self {
            japanese,
            kind: settings.kind,
            recipients: None,
            mapping: FieldMapping::default(),
            preview: settings.preview,
            record: settings.active_record.saturating_sub(1),
            highlight: false,
            dialog: None,
            menu: None,
            wanted_source: settings.kind.and(settings.data_source),
            saved_mapping: settings.mapping,
            pending_list: None,
        }
    }

    /// Whether a dialog is open over the pages.
    pub fn is_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// The field typing goes into while a dialog is open.
    pub fn field(&self) -> Option<&TextEdit> {
        match self.dialog.as_ref()? {
            MailDialog::Recipients(dialog) => dialog.field(),
            MailDialog::List(dialog) => dialog.field(),
            MailDialog::AddressBlock(_) | MailDialog::Match(_) | MailDialog::Errors(_) => None,
            MailDialog::Greeting(dialog) => dialog.field(),
            MailDialog::Rule(dialog) => dialog.field(),
            MailDialog::Finish(dialog) => dialog.field(),
            MailDialog::Envelope(dialog) => dialog.field(),
            MailDialog::Labels(dialog) => dialog.field(),
        }
    }

    pub fn field_mut(&mut self) -> Option<&mut TextEdit> {
        match self.dialog.as_mut()? {
            MailDialog::Recipients(dialog) => dialog.field_mut(),
            MailDialog::List(dialog) => dialog.field_mut(),
            MailDialog::AddressBlock(_) | MailDialog::Match(_) | MailDialog::Errors(_) => None,
            MailDialog::Greeting(dialog) => dialog.field_mut(),
            MailDialog::Rule(dialog) => dialog.field_mut(),
            MailDialog::Finish(dialog) => dialog.field_mut(),
            MailDialog::Envelope(dialog) => dialog.field_mut(),
            MailDialog::Labels(dialog) => dialog.field_mut(),
        }
    }

    /// The recipients merged, in order: indices into the data's records.
    pub fn chosen(&self) -> Vec<usize> {
        self.recipients
            .as_ref()
            .map(Recipients::chosen)
            .unwrap_or_default()
    }

    /// The merge and the recipient 結果のプレビュー shows, while it is on:
    /// what an update of the fields (F9) evaluates merge fields for.
    pub fn previewed(&self) -> Option<(MergeRequest, usize)> {
        if !self.preview {
            return None;
        }
        Some((self.request()?, self.record))
    }

    /// What a merge or a preview is over.
    pub fn request(&self) -> Option<MergeRequest> {
        let recipients = self.recipients.as_ref()?;
        Some(MergeRequest {
            data: recipients.data.clone(),
            records: recipients.chosen(),
            mapping: self.mapping.clone(),
        })
    }

    /// The columns of the data source, for 差し込みフィールドの挿入.
    pub fn columns(&self) -> Vec<String> {
        self.recipients
            .as_ref()
            .map(|recipients| recipients.data.columns.clone())
            .unwrap_or_default()
    }

    /// A data source the shell read: the recipients, all chosen; the
    /// fields matched by name and by the document's saved mapping.
    pub fn load_data_source<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        name: &str,
        path: Option<String>,
        bytes: &[u8],
    ) -> Result<MailOutcome, String> {
        let data = read_data_source(name, bytes)?;
        self.use_data(session, data, name, path, false)
    }

    fn use_data<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        data: MergeData,
        name: &str,
        path: Option<String>,
        editable: bool,
    ) -> Result<MailOutcome, String> {
        let mut mapping = FieldMapping::matched(&data);
        for (field, column) in &self.saved_mapping {
            if let Some(index) = data.column(column) {
                mapping.set(*field, Some(index));
            }
        }
        self.mapping = mapping;
        self.recipients = Some(Recipients::new(data, name, path, editable));
        self.record = self.record.min(self.chosen().len().saturating_sub(1));
        self.wanted_source = None;
        if self.kind.is_none() {
            self.kind = Some(MainDocumentType::Letters);
        }
        self.write_settings(session)?;
        if self.preview {
            self.show_record(session)?;
        }
        Ok(MailOutcome::Edited)
    }

    /// The data source the document names, read when it is opened: the
    /// recipients and the matching, the document left as it was saved.
    pub fn attach_data_source(
        &mut self,
        name: &str,
        path: Option<String>,
        bytes: &[u8],
    ) -> Result<(), String> {
        let data = read_data_source(name, bytes)?;
        let mut mapping = FieldMapping::matched(&data);
        for (field, column) in &self.saved_mapping {
            if let Some(index) = data.column(column) {
                mapping.set(*field, Some(index));
            }
        }
        self.mapping = mapping;
        self.recipients = Some(Recipients::new(data, name, path, false));
        self.record = self.record.min(self.chosen().len().saturating_sub(1));
        self.wanted_source = None;
        Ok(())
    }

    /// The list made here was saved at `path`: the document names it.
    pub fn saved_data_source<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        name: &str,
        path: Option<String>,
    ) -> Result<MailOutcome, String> {
        let data = self.pending_list.take().unwrap_or_default();
        self.use_data(session, data, name, path, true)
    }

    /// The merge's settings written to the document.
    fn write_settings<S: WordSession + ?Sized>(&self, session: &mut S) -> Result<(), String> {
        let mapping = self
            .recipients
            .as_ref()
            .map(|recipients| {
                AddressField::ALL
                    .into_iter()
                    .filter_map(|field| {
                        let column = self.mapping.column(field)?;
                        let name = recipients.data.columns.get(column)?;
                        // A column of the field's own name needs no entry.
                        (!name.eq_ignore_ascii_case(field.odso_name()))
                            .then(|| (field, name.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let settings = MailMergeSettings {
            kind: self.kind,
            data_source: self
                .recipients
                .as_ref()
                .and_then(|recipients| recipients.path.clone()),
            preview: self.preview,
            active_record: self.record + 1,
            mapping,
            check_errors: None,
        };
        if session.word_document().mail_merge_settings() == settings {
            return Ok(());
        }
        let range = selection_of(session);
        session.execute_command(
            Box::new(MailMergeAction::Settings(settings).command(range)),
            true,
        )
    }

    /// The fields showing the record previewed, or their «names».
    fn show_record<S: WordSession + ?Sized>(&mut self, session: &mut S) -> Result<(), String> {
        let range = selection_of(session);
        let action = match self.request() {
            Some(request) if self.preview && !request.records.is_empty() => {
                self.record = self.record.min(request.records.len() - 1);
                MailMergeAction::Preview(request, self.record)
            }
            _ => MailMergeAction::FieldCodes,
        };
        session.execute_command(Box::new(action.command(range)), true)
    }

    /// The tab, as both shells draw it.
    pub fn ribbon<S: WordSession + ?Sized>(&self, session: &S, t: Translate) -> Vec<ToolGroup> {
        let started = self.kind.is_some();
        let has_data = self.recipients.is_some();
        let count = self.chosen().len();
        let fields_used = !session.word_document().merge_field_names().is_empty();
        let button =
            |id: FormId, label: &'static str, title: &'static str, on: bool, enabled: bool| {
                ToolItem::Button {
                    id,
                    label: t(label).to_string(),
                    title: t(title).to_string(),
                    on,
                    enabled,
                }
            };
        let menu = |id: FormId,
                    which: MailMenu,
                    label: &'static str,
                    title: &'static str,
                    enabled: bool,
                    items: Vec<ToolMenuItem>| {
            ToolItem::Menu {
                id,
                label: t(label).to_string(),
                title: t(title).to_string(),
                enabled,
                open: self.menu == Some(which),
                items,
            }
        };
        let start_items = START_ENTRIES
            .iter()
            .map(|(key, kind)| ToolMenuItem {
                label: t(key).to_string(),
                on: self.kind == *kind,
                enabled: true,
            })
            .collect();
        let recipient_items = vec![
            ToolMenuItem::new(t("mail.recipients.new")),
            ToolMenuItem::new(t("mail.recipients.existing")),
        ];
        let field_items: Vec<ToolMenuItem> = if has_data {
            self.columns().into_iter().map(ToolMenuItem::new).collect()
        } else {
            vec![ToolMenuItem {
                label: t("mail.no_data").to_string(),
                on: false,
                enabled: false,
            }]
        };
        let rule_items = RULE_ENTRIES
            .iter()
            .map(|key| ToolMenuItem {
                label: t(key).to_string(),
                on: false,
                enabled: started,
            })
            .collect();
        let finish_items = vec![
            ToolMenuItem::new(t("mail.finish.document")),
            ToolMenuItem::new(t("mail.finish.print")),
            ToolMenuItem::new(t("mail.finish.pdf")),
        ];
        let record_text = if has_data && count > 0 {
            t("mail.record")
                .replace("{n}", &(self.record + 1).to_string())
                .replace("{count}", &count.to_string())
        } else {
            t("mail.record_none").to_string()
        };
        let navigable = self.preview && count > 0;
        vec![
            ToolGroup {
                label: t("mail.group.create").to_string(),
                rows: vec![vec![
                    button(
                        ENVELOPES,
                        "mail.envelopes",
                        "mail.envelopes_title",
                        false,
                        true,
                    ),
                    button(LABELS, "mail.labels", "mail.labels_title", false, true),
                ]],
            },
            ToolGroup {
                label: t("mail.group.start").to_string(),
                rows: vec![
                    vec![
                        menu(
                            START,
                            MailMenu::Start,
                            "mail.start",
                            "mail.start_title",
                            true,
                            start_items,
                        ),
                        menu(
                            RECIPIENTS,
                            MailMenu::Recipients,
                            "mail.recipients",
                            "mail.recipients_title",
                            true,
                            recipient_items,
                        ),
                    ],
                    vec![button(
                        EDIT_RECIPIENTS,
                        "mail.edit_recipients",
                        "mail.edit_recipients_title",
                        false,
                        has_data,
                    )],
                ],
            },
            ToolGroup {
                label: t("mail.group.fields").to_string(),
                rows: vec![
                    vec![
                        button(
                            HIGHLIGHT,
                            "mail.highlight",
                            "mail.highlight_title",
                            self.highlight,
                            fields_used,
                        ),
                        button(
                            ADDRESS_BLOCK,
                            "mail.address_block",
                            "mail.address_block_title",
                            false,
                            started,
                        ),
                        button(
                            GREETING,
                            "mail.greeting",
                            "mail.greeting_title",
                            false,
                            started,
                        ),
                    ],
                    vec![
                        menu(
                            INSERT_FIELD,
                            MailMenu::InsertField,
                            "mail.insert_field",
                            "mail.insert_field_title",
                            started,
                            field_items,
                        ),
                        menu(
                            RULES,
                            MailMenu::Rules,
                            "mail.rules",
                            "mail.rules_title",
                            started,
                            rule_items,
                        ),
                        button(MATCH, "mail.match", "mail.match_title", false, has_data),
                        button(
                            UPDATE_LABELS,
                            "mail.update_labels",
                            "mail.update_labels_title",
                            false,
                            self.kind == Some(MainDocumentType::Labels),
                        ),
                    ],
                ],
            },
            ToolGroup {
                label: t("mail.group.preview").to_string(),
                rows: vec![
                    vec![
                        button(
                            PREVIEW,
                            "mail.preview",
                            "mail.preview_title",
                            self.preview,
                            has_data && started,
                        ),
                        button(CHECK, "mail.check", "mail.check_title", false, started),
                    ],
                    vec![
                        button(
                            FIRST,
                            "mail.first",
                            "mail.first_title",
                            false,
                            navigable && self.record > 0,
                        ),
                        button(
                            PREVIOUS,
                            "mail.previous",
                            "mail.previous_title",
                            false,
                            navigable && self.record > 0,
                        ),
                        ToolItem::Text(record_text),
                        button(
                            NEXT,
                            "mail.next",
                            "mail.next_title",
                            false,
                            navigable && self.record + 1 < count,
                        ),
                        button(
                            LAST,
                            "mail.last",
                            "mail.last_title",
                            false,
                            navigable && self.record + 1 < count,
                        ),
                    ],
                ],
            },
            ToolGroup {
                label: t("mail.group.finish").to_string(),
                rows: vec![vec![menu(
                    FINISH,
                    MailMenu::Finish,
                    "mail.finish",
                    "mail.finish_title",
                    started && has_data,
                    finish_items,
                )]],
            },
        ]
    }

    /// A click on the tab.
    pub fn ribbon_event<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        event: FormEvent,
    ) -> Result<MailOutcome, String> {
        let menu_of = |id: FormId| match id {
            START => Some(MailMenu::Start),
            RECIPIENTS => Some(MailMenu::Recipients),
            INSERT_FIELD => Some(MailMenu::InsertField),
            RULES => Some(MailMenu::Rules),
            FINISH => Some(MailMenu::Finish),
            _ => None,
        };
        match event {
            FormEvent::Click(id) if menu_of(id).is_some() => {
                let menu = menu_of(id);
                self.menu = if self.menu == menu { None } else { menu };
                Ok(MailOutcome::None)
            }
            FormEvent::Choose(id, index) => {
                self.menu = None;
                self.menu_entry(session, id, index)
            }
            FormEvent::Click(id) => {
                self.menu = None;
                self.button(session, id)
            }
            _ => Ok(MailOutcome::None),
        }
    }

    fn button<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        id: FormId,
    ) -> Result<MailOutcome, String> {
        match id {
            ENVELOPES => {
                self.dialog = Some(MailDialog::Envelope(EnvelopeDialog::new(
                    self.japanese,
                    false,
                )));
            }
            LABELS => {
                self.dialog = Some(MailDialog::Labels(LabelDialog::new(self.japanese, false)));
            }
            EDIT_RECIPIENTS => {
                if let Some(recipients) = &self.recipients {
                    self.dialog = Some(MailDialog::Recipients(RecipientsDialog::new(
                        recipients.clone(),
                    )));
                }
            }
            HIGHLIGHT => self.highlight ^= true,
            ADDRESS_BLOCK => {
                self.dialog = Some(MailDialog::AddressBlock(AddressBlockDialog::new(
                    self.japanese,
                    self.request(),
                    self.record,
                )));
            }
            GREETING => {
                self.dialog = Some(MailDialog::Greeting(GreetingDialog::new(
                    self.japanese,
                    self.request(),
                    self.record,
                )));
            }
            MATCH => {
                if let Some(recipients) = &self.recipients {
                    self.dialog = Some(MailDialog::Match(MatchFieldsDialog::new(
                        recipients.data.clone(),
                        self.mapping.clone(),
                    )));
                }
            }
            UPDATE_LABELS => {
                let range = selection_of(session);
                session.execute_command(
                    Box::new(MailMergeAction::PropagateLabels.command(range)),
                    true,
                )?;
                if self.preview {
                    self.show_record(session)?;
                }
                return Ok(MailOutcome::Edited);
            }
            PREVIEW => {
                self.preview ^= true;
                self.show_record(session)?;
                self.write_settings(session)?;
                return Ok(MailOutcome::Edited);
            }
            FIRST | PREVIOUS | NEXT | LAST => {
                let count = self.chosen().len();
                if count == 0 {
                    return Ok(MailOutcome::None);
                }
                self.record = match id {
                    FIRST => 0,
                    PREVIOUS => self.record.saturating_sub(1),
                    NEXT => (self.record + 1).min(count - 1),
                    _ => count - 1,
                };
                self.show_record(session)?;
                return Ok(MailOutcome::Edited);
            }
            CHECK => {
                let request = self
                    .request()
                    .unwrap_or_else(|| MergeRequest::all(MergeData::default()));
                let problems = session.word_document().merge_problems(&request)?;
                self.dialog = Some(MailDialog::Errors(ErrorCheckDialog::new(problems)));
            }
            _ => {}
        }
        Ok(MailOutcome::None)
    }

    fn menu_entry<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        id: FormId,
        index: usize,
    ) -> Result<MailOutcome, String> {
        match id {
            START => {
                let Some((_, kind)) = START_ENTRIES.get(index) else {
                    return Ok(MailOutcome::None);
                };
                self.kind = *kind;
                match kind {
                    Some(MainDocumentType::Envelopes) => {
                        self.dialog = Some(MailDialog::Envelope(EnvelopeDialog::new(
                            self.japanese,
                            true,
                        )));
                    }
                    Some(MainDocumentType::Labels) => {
                        self.dialog =
                            Some(MailDialog::Labels(LabelDialog::new(self.japanese, true)));
                    }
                    None => {
                        self.preview = false;
                        let range = selection_of(session);
                        session.execute_command(
                            Box::new(MailMergeAction::Normal.command(range)),
                            true,
                        )?;
                        return Ok(MailOutcome::Edited);
                    }
                    _ => {}
                }
                self.write_settings(session)?;
                Ok(MailOutcome::Edited)
            }
            RECIPIENTS => match index {
                0 => {
                    self.dialog = Some(MailDialog::List(ListEditor::new(self.japanese, None)));
                    Ok(MailOutcome::None)
                }
                _ => Ok(MailOutcome::PickDataSource),
            },
            INSERT_FIELD => {
                let Some(column) = self.columns().get(index).cloned() else {
                    return Ok(MailOutcome::None);
                };
                self.insert(session, MergeFieldSpec::Column(column))
            }
            RULES => {
                let Some(key) = RULE_ENTRIES.get(index) else {
                    return Ok(MailOutcome::None);
                };
                match *key {
                    "mail.rule.merge_record" => {
                        self.insert(session, MergeFieldSpec::Rule(MergeRule::MergeRecord))
                    }
                    "mail.rule.merge_sequence" => {
                        self.insert(session, MergeFieldSpec::Rule(MergeRule::MergeSequence))
                    }
                    "mail.rule.next" => self.insert(session, MergeFieldSpec::Rule(MergeRule::Next)),
                    key => {
                        self.dialog = Some(MailDialog::Rule(RuleDialog::new(key, self.columns())));
                        Ok(MailOutcome::None)
                    }
                }
            }
            FINISH => {
                let destination = match index {
                    0 => Destination::Document,
                    1 => Destination::Printer,
                    _ => Destination::Pdf,
                };
                self.dialog = Some(MailDialog::Finish(FinishDialog::new(
                    destination,
                    self.record,
                    self.chosen().len(),
                )));
                Ok(MailOutcome::None)
            }
            _ => Ok(MailOutcome::None),
        }
    }

    /// A merge field at the caret, showing the previewed recipient's
    /// values when preview is on.
    fn insert<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        field: MergeFieldSpec,
    ) -> Result<MailOutcome, String> {
        if self.kind.is_none() {
            self.kind = Some(MainDocumentType::Letters);
            self.write_settings(session)?;
        }
        let preview = self
            .request()
            .filter(|request| self.preview && !request.records.is_empty())
            .map(|request| (request, self.record));
        let range = selection_of(session);
        session.execute_command(
            Box::new(MailMergeAction::InsertField { field, preview }.command(range)),
            false,
        )?;
        Ok(MailOutcome::Edited)
    }

    /// The open dialog, as both shells draw it.
    pub fn dialog_view(&self, t: Translate) -> Option<FormView> {
        Some(match self.dialog.as_ref()? {
            MailDialog::Recipients(dialog) => dialog.view(t),
            MailDialog::List(dialog) => dialog.view(t),
            MailDialog::AddressBlock(dialog) => dialog.view(t),
            MailDialog::Greeting(dialog) => dialog.view(t),
            MailDialog::Match(dialog) => dialog.view(t),
            MailDialog::Rule(dialog) => dialog.view(t),
            MailDialog::Finish(dialog) => dialog.view(t),
            MailDialog::Envelope(dialog) => dialog.view(t),
            MailDialog::Labels(dialog) => dialog.view(t),
            MailDialog::Errors(dialog) => dialog.view(t),
        })
    }

    /// A click or a key in the open dialog.
    pub fn dialog_event<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        event: FormEvent,
    ) -> Result<MailOutcome, String> {
        let Some(dialog) = self.dialog.as_mut() else {
            return Ok(MailOutcome::None);
        };
        let outcome = match dialog {
            MailDialog::Recipients(dialog) => dialog.event(event),
            MailDialog::List(dialog) => dialog.event(event),
            MailDialog::AddressBlock(dialog) => dialog.event(event),
            MailDialog::Greeting(dialog) => dialog.event(event),
            MailDialog::Match(dialog) => dialog.event(event),
            MailDialog::Rule(dialog) => dialog.event(event),
            MailDialog::Finish(dialog) => dialog.event(event),
            MailDialog::Envelope(dialog) => dialog.event(event),
            MailDialog::Labels(dialog) => dialog.event(event),
            MailDialog::Errors(dialog) => dialog.event(event),
        };
        match outcome {
            FormOutcome::Stay => Ok(MailOutcome::None),
            FormOutcome::Close => {
                self.dialog = None;
                Ok(MailOutcome::None)
            }
            FormOutcome::Do(action) => {
                // The address block's and greeting line's フィールドの対応
                // opens over them; everything else closes the dialog.
                if !matches!(action, DialogAction::OpenMatch) {
                    self.dialog = None;
                }
                self.dialog_action(session, action)
            }
        }
    }

    fn dialog_action<S: WordSession + ?Sized>(
        &mut self,
        session: &mut S,
        action: DialogAction,
    ) -> Result<MailOutcome, String> {
        match action {
            DialogAction::Insert(field) => self.insert(session, field),
            DialogAction::Recipients(recipients) => {
                self.recipients = Some(recipients);
                self.record = self.record.min(self.chosen().len().saturating_sub(1));
                if self.preview {
                    self.show_record(session)?;
                }
                Ok(MailOutcome::Edited)
            }
            DialogAction::SaveList { data, name } => {
                let bytes = csv_bytes(&data);
                self.pending_list = Some(data);
                Ok(MailOutcome::SaveDataSource { name, bytes })
            }
            DialogAction::Mapping(mapping) => {
                self.mapping = mapping;
                self.write_settings(session)?;
                if self.preview {
                    self.show_record(session)?;
                }
                Ok(MailOutcome::Edited)
            }
            DialogAction::OpenMatch => {
                if let Some(recipients) = &self.recipients {
                    self.dialog = Some(MailDialog::Match(MatchFieldsDialog::new(
                        recipients.data.clone(),
                        self.mapping.clone(),
                    )));
                }
                Ok(MailOutcome::None)
            }
            DialogAction::Finish {
                records,
                destination,
            } => {
                let mut request = self
                    .request()
                    .ok_or_else(|| "宛先が選択されていません".to_string())?;
                let chosen = request.records.clone();
                request.records = records
                    .into_iter()
                    .filter_map(|index| chosen.get(index).copied())
                    .collect();
                let bytes = session.word_document().merged_document(&request)?;
                let name = if self.japanese {
                    "差し込み文書.docx"
                } else {
                    "Merged Document.docx"
                }
                .to_string();
                Ok(match destination {
                    Destination::Document => MailOutcome::OpenDocument { name, bytes },
                    Destination::Printer => MailOutcome::Print { bytes },
                    Destination::Pdf => MailOutcome::SavePdf {
                        name: name.replace(".docx", ".pdf"),
                        bytes,
                    },
                })
            }
            DialogAction::Envelope { spec, output } => {
                let range = selection_of(session);
                match output {
                    LayoutOutput::MainDocument => {
                        self.kind = Some(MainDocumentType::Envelopes);
                        session.execute_command(
                            Box::new(MailMergeAction::MakeEnvelope(spec).command(range)),
                            false,
                        )?;
                        self.write_settings(session)?;
                        if self.preview {
                            self.show_record(session)?;
                        }
                        Ok(MailOutcome::Edited)
                    }
                    LayoutOutput::NewDocument | LayoutOutput::Print => {
                        let body = crate::word_document::mail_merge::layouts::envelope_body(&spec);
                        let bytes = session.word_document().document_with_body(&body)?;
                        Ok(if output == LayoutOutput::Print {
                            MailOutcome::Print { bytes }
                        } else {
                            MailOutcome::OpenDocument {
                                name: if self.japanese {
                                    "封筒.docx"
                                } else {
                                    "Envelope.docx"
                                }
                                .to_string(),
                                bytes,
                            }
                        })
                    }
                }
            }
            DialogAction::Labels {
                product,
                text,
                output,
            } => {
                let range = selection_of(session);
                match output {
                    LayoutOutput::MainDocument => {
                        self.kind = Some(MainDocumentType::Labels);
                        session.execute_command(
                            Box::new(
                                MailMergeAction::MakeLabels {
                                    product,
                                    text: None,
                                    merge: true,
                                    japanese: self.japanese,
                                }
                                .command(range),
                            ),
                            false,
                        )?;
                        self.write_settings(session)?;
                        if self.preview {
                            self.show_record(session)?;
                        }
                        Ok(MailOutcome::Edited)
                    }
                    LayoutOutput::NewDocument | LayoutOutput::Print => {
                        let body = crate::word_document::mail_merge::layouts::labels_body(
                            &product,
                            text.as_deref(),
                            false,
                            self.japanese,
                        );
                        let bytes = session.word_document().document_with_body(&body)?;
                        Ok(if output == LayoutOutput::Print {
                            MailOutcome::Print { bytes }
                        } else {
                            MailOutcome::OpenDocument {
                                name: if self.japanese {
                                    "ラベル.docx"
                                } else {
                                    "Labels.docx"
                                }
                                .to_string(),
                                bytes,
                            }
                        })
                    }
                }
            }
        }
    }

    /// Where the pages show merge fields, for 差し込みフィールドの強調表示:
    /// the text of each MERGEFIELD.
    pub fn highlight_regions<S: WordSession + ?Sized>(
        &self,
        session: &S,
    ) -> Vec<crate::canvas_renderer::TextRegion> {
        use crate::document::RunFieldKind;
        if !self.highlight {
            return Vec::new();
        }
        let document = session.word_document();
        let mut regions = Vec::new();
        for index in 0..document.paragraph_count() {
            let Some(paragraph) = document.paragraph(index) else {
                continue;
            };
            let mut offset = 0;
            for run in &paragraph.runs {
                let end = offset + run.text.len();
                if end > offset
                    && run.field.as_ref().is_some_and(|field| {
                        matches!(field.kind.base_kind(), RunFieldKind::MergeField { .. })
                    })
                {
                    let start =
                        document.model_to_rendered_text_position(TextPosition::new(index, offset));
                    let stop =
                        document.model_to_rendered_text_position(TextPosition::new(index, end));
                    regions.extend(session.text_regions(
                        (start.paragraph, start.offset),
                        (stop.paragraph, stop.offset),
                    ));
                }
                offset = end;
            }
        }
        regions
    }
}
