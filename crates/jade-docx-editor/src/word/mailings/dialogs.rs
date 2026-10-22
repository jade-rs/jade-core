//! The 差し込み文書 tab's dialogs as [`FormView`]s: 宛先リストの編集,
//! 新しいアドレス帳 (a list typed here), 住所ブロックの挿入, あいさつ文の挿入,
//! フィールドの対応, the ルール, 完了と差し込み, 封筒 and ラベル, and
//! エラーのチェック. Each keeps its own fields and returns what its OK asks
//! for as a [`DialogAction`].

use crate::word_document::mail_merge::address::{render_format, CountryMode};

use super::{
    AddressBlockOptions, AddressField, Destination, DialogAction, EnvelopeSpec, FieldMapping,
    GreetingLineOptions, LabelProduct, LayoutOutput, MergeData, MergeFieldSpec, MergeProblem,
    MergeRequest, MergeRule, NameFormat, Recipients, ENVELOPE_SIZES, LABEL_PRODUCTS,
    RULE_OPERATORS,
};
use crate::text_edit::TextEdit;
use crate::word::form_view::{
    FieldView, FormButton, FormEvent, FormId, FormOutcome, FormRow, FormView, GridRow, Translate,
};
use crate::word::mailings::{Filter, FilterOp};

const OK: FormId = FormId::new("ok");
const CANCEL: FormId = FormId::new("cancel");

fn ok_cancel(t: Translate) -> Vec<FormButton> {
    vec![
        FormButton::new(CANCEL, t("format.cancel")),
        FormButton::new(OK, t("format.ok")).primary(),
    ]
}

fn field(id: FormId, label: &str, edit: &TextEdit, focused: FormId, width: f32) -> FormRow {
    FormRow::Field {
        id,
        label: label.to_string(),
        value: FieldView::of(edit),
        focused: focused == id,
        width,
    }
}

fn area(id: FormId, label: &str, edit: &TextEdit, focused: FormId, lines: usize) -> FormRow {
    FormRow::TextArea {
        id,
        label: label.to_string(),
        value: FieldView::of(edit),
        focused: focused == id,
        width: 260.0,
        lines,
    }
}

/// Tab through `fields`, from `focus`.
fn tab(fields: &[FormId], focus: FormId, backwards: bool) -> FormId {
    if fields.is_empty() {
        return focus;
    }
    let at = fields.iter().position(|id| *id == focus).unwrap_or(0);
    let next = if backwards {
        (at + fields.len() - 1) % fields.len()
    } else {
        (at + 1) % fields.len()
    };
    fields[next]
}

/// A recipient's values the way an address block or a greeting line takes
/// them.
fn address_value(request: &MergeRequest, record: usize) -> impl Fn(AddressField) -> String + '_ {
    move |field| {
        let Some(&index) = request.records.get(record) else {
            return String::new();
        };
        request
            .mapping
            .column(field)
            .map(|column| request.data.value(index, column).to_string())
            .unwrap_or_default()
    }
}

// ---- 宛先リストの編集 -------------------------------------------------------

const GRID: FormId = FormId::new("grid");
const FILTER_COLUMN: FormId = FormId::new("filter-column");
const FILTER_OP: FormId = FormId::new("filter-op");
const FILTER_VALUE: FormId = FormId::new("filter-value");
const FILTER_ADD: FormId = FormId::new("filter-add");
const FILTER_CLEAR: FormId = FormId::new("filter-clear");
const SELECT_ALL: FormId = FormId::new("select-all");
const SELECT_NONE: FormId = FormId::new("select-none");

/// 宛先リストの編集: tick the recipients, sort by a column, filter.
pub struct RecipientsDialog {
    pub recipients: Recipients,
    filter_column: usize,
    filter_op: FilterOp,
    filter_value: TextEdit,
}

impl RecipientsDialog {
    pub fn new(recipients: Recipients) -> Self {
        Self {
            recipients,
            filter_column: 0,
            filter_op: FilterOp::Equal,
            filter_value: TextEdit::new(String::new()),
        }
    }

    pub fn field(&self) -> Option<&TextEdit> {
        Some(&self.filter_value)
    }

    pub fn field_mut(&mut self) -> Option<&mut TextEdit> {
        Some(&mut self.filter_value)
    }

    pub fn view(&self, t: Translate) -> FormView {
        let recipients = &self.recipients;
        let shown = recipients.shown();
        let mut rows = vec![
            FormRow::Note(t("mail.recipients.source").replace("{name}", &recipients.name)),
            FormRow::Grid {
                id: GRID,
                headers: recipients.data.columns.clone(),
                rows: shown
                    .iter()
                    .map(|&record| GridRow {
                        cells: recipients.data.records[record].clone(),
                        checked: Some(recipients.included.get(record).copied().unwrap_or(true)),
                    })
                    .collect(),
                selected: None,
                editing: None,
                sorted: recipients.sort,
            },
            FormRow::Buttons(vec![
                FormButton::new(SELECT_ALL, t("mail.recipients.all")),
                FormButton::new(SELECT_NONE, t("mail.recipients.none")),
            ]),
            FormRow::Heading(t("mail.filter").to_string()),
            FormRow::Choice {
                id: FILTER_COLUMN,
                label: t("mail.filter.column").to_string(),
                options: recipients.data.columns.clone(),
                selected: Some(self.filter_column),
            },
            FormRow::Choice {
                id: FILTER_OP,
                label: t("mail.filter.compare").to_string(),
                options: FilterOp::ALL
                    .iter()
                    .map(|op| t(op.label_key()).to_string())
                    .collect(),
                selected: FilterOp::ALL.iter().position(|op| *op == self.filter_op),
            },
            field(
                FILTER_VALUE,
                t("mail.filter.value"),
                &self.filter_value,
                FILTER_VALUE,
                200.0,
            ),
            FormRow::Buttons(vec![
                FormButton::new(FILTER_ADD, t("mail.filter.add")),
                FormButton::new(FILTER_CLEAR, t("mail.filter.clear"))
                    .enabled(!recipients.filters.is_empty()),
            ]),
        ];
        for filter in &recipients.filters {
            rows.push(FormRow::Note(format!(
                "{} {} {}",
                recipients
                    .data
                    .columns
                    .get(filter.column)
                    .cloned()
                    .unwrap_or_default(),
                t(filter.op.label_key()),
                filter.value
            )));
        }
        rows.push(FormRow::Note(
            t("mail.recipients.count")
                .replace("{n}", &recipients.chosen().len().to_string())
                .replace("{count}", &recipients.data.records.len().to_string()),
        ));
        FormView {
            title: t("mail.edit_recipients").to_string(),
            width: 720.0,
            rows,
            buttons: ok_cancel(t),
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        let recipients = &mut self.recipients;
        match event {
            FormEvent::Click(OK) | FormEvent::Enter => {
                return FormOutcome::Do(DialogAction::Recipients(recipients.clone()))
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            FormEvent::Header(GRID, column) => {
                recipients.sort = match recipients.sort {
                    Some((sorted, true)) if sorted == column => Some((column, false)),
                    Some((sorted, false)) if sorted == column => None,
                    _ => Some((column, true)),
                };
            }
            FormEvent::RowCheck(GRID, row) | FormEvent::Cell(GRID, row, _) => {
                if let Some(&record) = recipients.shown().get(row) {
                    if let Some(included) = recipients.included.get_mut(record) {
                        *included ^= true;
                    }
                }
            }
            FormEvent::Click(SELECT_ALL) | FormEvent::Click(SELECT_NONE) => {
                let on = event == FormEvent::Click(SELECT_ALL);
                for record in recipients.shown() {
                    if let Some(included) = recipients.included.get_mut(record) {
                        *included = on;
                    }
                }
            }
            FormEvent::Choose(FILTER_COLUMN, column) => self.filter_column = column,
            FormEvent::Choose(FILTER_OP, index) => {
                if let Some(op) = FilterOp::ALL.get(index) {
                    self.filter_op = *op;
                }
            }
            FormEvent::Click(FILTER_ADD) => {
                if self.filter_column < recipients.data.columns.len() {
                    recipients.filters.push(Filter {
                        column: self.filter_column,
                        op: self.filter_op,
                        value: self.filter_value.text.clone(),
                    });
                    self.filter_value = TextEdit::new(String::new());
                }
            }
            FormEvent::Click(FILTER_CLEAR) => recipients.filters.clear(),
            _ => {}
        }
        FormOutcome::Stay
    }
}

// ---- 新しいアドレス帳 -------------------------------------------------------

const LIST_NAME: FormId = FormId::new("list-name");
const COLUMN_NAME: FormId = FormId::new("column-name");
const ADD_ROW: FormId = FormId::new("add-row");
const REMOVE_ROW: FormId = FormId::new("remove-row");
const ADD_COLUMN: FormId = FormId::new("add-column");
const REMOVE_COLUMN: FormId = FormId::new("remove-column");
const RENAME_COLUMN: FormId = FormId::new("rename-column");

/// The columns Word gives a new list.
pub fn default_columns(japanese: bool) -> Vec<String> {
    let names: &[&str] = if japanese {
        &[
            "敬称",
            "姓",
            "名",
            "会社名",
            "郵便番号",
            "都道府県",
            "市区町村",
            "住所1",
            "住所2",
            "電話番号",
            "メール アドレス",
        ]
    } else {
        &[
            "Title",
            "First Name",
            "Last Name",
            "Company Name",
            "Address Line 1",
            "Address Line 2",
            "City",
            "State",
            "ZIP Code",
            "Country or Region",
            "Home Phone",
            "E-mail Address",
        ]
    };
    names.iter().map(|name| name.to_string()).collect()
}

/// 新しいアドレス帳: a table of recipients typed here, saved as a CSV the
/// merge then reads.
pub struct ListEditor {
    pub data: MergeData,
    /// The cell selected (row, column) and whether it is being typed in.
    selected: (usize, usize),
    cell: TextEdit,
    editing: bool,
    name: TextEdit,
    column_name: TextEdit,
    focus: FormId,
}

impl ListEditor {
    pub fn new(japanese: bool, data: Option<MergeData>) -> Self {
        let data = data.unwrap_or_else(|| {
            let columns = default_columns(japanese);
            MergeData {
                records: vec![vec![String::new(); columns.len()]],
                columns,
            }
        });
        let first = data
            .records
            .first()
            .and_then(|record| record.first())
            .cloned()
            .unwrap_or_default();
        Self {
            data,
            selected: (0, 0),
            cell: TextEdit::new(first),
            editing: true,
            name: TextEdit::new(
                if japanese {
                    "アドレス帳.csv"
                } else {
                    "Address List.csv"
                }
                .to_string(),
            ),
            column_name: TextEdit::new(String::new()),
            focus: GRID,
        }
    }

    pub fn field(&self) -> Option<&TextEdit> {
        Some(match self.focus {
            LIST_NAME => &self.name,
            COLUMN_NAME => &self.column_name,
            _ => &self.cell,
        })
    }

    pub fn field_mut(&mut self) -> Option<&mut TextEdit> {
        Some(match self.focus {
            LIST_NAME => &mut self.name,
            COLUMN_NAME => &mut self.column_name,
            _ => {
                self.editing = true;
                &mut self.cell
            }
        })
    }

    /// The typed text into its cell.
    fn commit(&mut self) {
        let (row, column) = self.selected;
        if let Some(slot) = self
            .data
            .records
            .get_mut(row)
            .and_then(|record| record.get_mut(column))
        {
            *slot = self.cell.text.clone();
        }
    }

    fn select(&mut self, row: usize, column: usize) {
        self.commit();
        let row = row.min(self.data.records.len().saturating_sub(1));
        let column = column.min(self.data.columns.len().saturating_sub(1));
        self.selected = (row, column);
        let text = self
            .data
            .records
            .get(row)
            .and_then(|record| record.get(column))
            .cloned()
            .unwrap_or_default();
        self.cell = TextEdit::selected(text);
        self.editing = true;
        self.focus = GRID;
    }

    pub fn view(&self, t: Translate) -> FormView {
        let (row, column) = self.selected;
        let editing =
            (self.focus == GRID && self.editing).then(|| (row, column, FieldView::of(&self.cell)));
        FormView {
            title: t("mail.list.title").to_string(),
            width: 760.0,
            rows: vec![
                FormRow::Note(t("mail.list.note").to_string()),
                FormRow::Grid {
                    id: GRID,
                    headers: self.data.columns.clone(),
                    rows: self
                        .data
                        .records
                        .iter()
                        .map(|record| GridRow {
                            cells: record.clone(),
                            checked: None,
                        })
                        .collect(),
                    selected: Some(self.selected),
                    editing,
                    sorted: None,
                },
                FormRow::Buttons(vec![
                    FormButton::new(ADD_ROW, t("mail.list.add_row")),
                    FormButton::new(REMOVE_ROW, t("mail.list.remove_row"))
                        .enabled(self.data.records.len() > 1),
                ]),
                field(
                    COLUMN_NAME,
                    t("mail.list.column_name"),
                    &self.column_name,
                    self.focus,
                    160.0,
                ),
                FormRow::Buttons(vec![
                    FormButton::new(ADD_COLUMN, t("mail.list.add_column")),
                    FormButton::new(RENAME_COLUMN, t("mail.list.rename_column")),
                    FormButton::new(REMOVE_COLUMN, t("mail.list.remove_column"))
                        .enabled(self.data.columns.len() > 1),
                ]),
                field(
                    LIST_NAME,
                    t("mail.list.name"),
                    &self.name,
                    self.focus,
                    240.0,
                ),
            ],
            buttons: ok_cancel(t),
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        let (row, column) = self.selected;
        match event {
            FormEvent::Click(OK) => {
                self.commit();
                let mut name = self.name.text.trim().to_string();
                if name.is_empty() {
                    name = "Address List.csv".to_string();
                }
                if !name.to_ascii_lowercase().ends_with(".csv") {
                    name.push_str(".csv");
                }
                return FormOutcome::Do(DialogAction::SaveList {
                    data: self.data.clone(),
                    name,
                });
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            FormEvent::Cell(GRID, row, column) => self.select(row, column),
            FormEvent::Click(LIST_NAME) | FormEvent::Click(COLUMN_NAME) => {
                self.commit();
                if let FormEvent::Click(id) = event {
                    self.focus = id;
                }
            }
            FormEvent::Tab(backwards) if self.focus == GRID => {
                let columns = self.data.columns.len().max(1);
                let rows = self.data.records.len();
                let flat = row * columns + column;
                let next = if backwards {
                    flat.saturating_sub(1)
                } else {
                    flat + 1
                };
                if next / columns >= rows {
                    // Tab past the last cell: a new entry, as Word's list does.
                    self.commit();
                    self.data.records.push(vec![String::new(); columns]);
                }
                self.select(next / columns, next % columns);
            }
            FormEvent::Enter if self.focus == GRID => {
                if row + 1 >= self.data.records.len() {
                    self.commit();
                    self.data
                        .records
                        .push(vec![String::new(); self.data.columns.len()]);
                }
                self.select(row + 1, column);
            }
            FormEvent::Tab(_) | FormEvent::Enter => {
                self.focus = GRID;
            }
            FormEvent::Click(ADD_ROW) => {
                self.commit();
                self.data
                    .records
                    .push(vec![String::new(); self.data.columns.len()]);
                self.select(self.data.records.len() - 1, 0);
            }
            FormEvent::Click(REMOVE_ROW) => {
                if self.data.records.len() > 1 {
                    self.data.records.remove(row);
                    self.cell = TextEdit::new(String::new());
                    self.selected = (row.min(self.data.records.len() - 1), column);
                    self.select(self.selected.0, column);
                }
            }
            FormEvent::Click(ADD_COLUMN) => {
                self.commit();
                let name = self.column_name.text.trim().to_string();
                if !name.is_empty() {
                    self.data.columns.push(name);
                    for record in &mut self.data.records {
                        record.push(String::new());
                    }
                    self.column_name = TextEdit::new(String::new());
                }
            }
            FormEvent::Click(RENAME_COLUMN) => {
                let name = self.column_name.text.trim().to_string();
                if let Some(slot) = self.data.columns.get_mut(column) {
                    if !name.is_empty() {
                        *slot = name;
                        self.column_name = TextEdit::new(String::new());
                    }
                }
            }
            FormEvent::Click(REMOVE_COLUMN) => {
                if self.data.columns.len() > 1 {
                    self.commit();
                    self.data.columns.remove(column);
                    for record in &mut self.data.records {
                        if column < record.len() {
                            record.remove(column);
                        }
                    }
                    self.select(row, column.min(self.data.columns.len() - 1));
                }
            }
            _ => {}
        }
        FormOutcome::Stay
    }
}

// ---- 住所ブロックの挿入 -----------------------------------------------------

const NAME_FORMAT: FormId = FormId::new("name-format");
const HONORIFIC: FormId = FormId::new("honorific");
const COMPANY: FormId = FormId::new("company");
const POSTAL: FormId = FormId::new("postal");
const COUNTRY: FormId = FormId::new("country");
const BY_COUNTRY: FormId = FormId::new("by-country");
const PREVIOUS: FormId = FormId::new("previous");
const NEXT: FormId = FormId::new("next");
const MATCH: FormId = FormId::new("match");

const HONORIFICS: [&str; 4] = ["様", "殿", "御中", ""];

fn preview_rows(
    request: &Option<MergeRequest>,
    record: usize,
    format: &str,
    country: &dyn Fn(&str) -> bool,
    invalid: Option<&str>,
    t: Translate,
) -> Vec<FormRow> {
    let lines = match request {
        Some(request) if !request.records.is_empty() => {
            let value = address_value(request, record);
            let named = [AddressField::LastName, AddressField::FirstName]
                .into_iter()
                .any(|field| !value(field).trim().is_empty());
            let text = match invalid {
                Some(invalid) if !named => invalid.to_string(),
                _ => render_format(format, &value, country),
            };
            text.lines().map(str::to_string).collect()
        }
        _ => vec![t("mail.preview_none").to_string()],
    };
    let count = request.as_ref().map_or(0, |request| request.records.len());
    vec![
        FormRow::Heading(t("mail.preview_heading").to_string()),
        FormRow::Preview(lines),
        FormRow::Buttons(vec![
            FormButton::new(PREVIOUS, "◀").enabled(record > 0),
            FormButton::new(
                FormId::new("record"),
                t("mail.record")
                    .replace("{n}", &(record + 1).min(count).to_string())
                    .replace("{count}", &count.to_string()),
            )
            .enabled(false),
            FormButton::new(NEXT, "▶").enabled(record + 1 < count),
            FormButton::new(MATCH, t("mail.match")).enabled(request.is_some()),
        ]),
    ]
}

/// 住所ブロックの挿入.
pub struct AddressBlockDialog {
    pub options: AddressBlockOptions,
    request: Option<MergeRequest>,
    record: usize,
}

impl AddressBlockDialog {
    pub fn new(japanese: bool, request: Option<MergeRequest>, record: usize) -> Self {
        Self {
            options: AddressBlockOptions::new(japanese),
            request,
            record,
        }
    }

    pub fn view(&self, t: Translate) -> FormView {
        let options = &self.options;
        let mut rows = vec![
            FormRow::Check {
                id: FormId::new("name"),
                label: t("mail.address.name").to_string(),
                on: options.name.is_some(),
            },
            FormRow::Choice {
                id: NAME_FORMAT,
                label: t("mail.address.name_format").to_string(),
                options: NameFormat::ALL
                    .iter()
                    .map(|format| format.sample(options.japanese).to_string())
                    .collect(),
                selected: options
                    .name
                    .and_then(|name| NameFormat::ALL.iter().position(|format| *format == name)),
            },
            FormRow::Choice {
                id: HONORIFIC,
                label: t("mail.address.honorific").to_string(),
                options: HONORIFICS
                    .iter()
                    .map(|honorific| {
                        if honorific.is_empty() {
                            t("cc.props.none").to_string()
                        } else {
                            honorific.to_string()
                        }
                    })
                    .collect(),
                selected: HONORIFICS
                    .iter()
                    .position(|honorific| *honorific == options.honorific),
            },
            FormRow::Check {
                id: COMPANY,
                label: t("mail.address.company").to_string(),
                on: options.company,
            },
            FormRow::Check {
                id: POSTAL,
                label: t("mail.address.postal").to_string(),
                on: options.postal_address,
            },
            FormRow::Choice {
                id: COUNTRY,
                label: t("mail.address.country").to_string(),
                options: vec![
                    t("mail.address.country_always").to_string(),
                    t("mail.address.country_never").to_string(),
                    t("mail.address.country_except").to_string(),
                ],
                selected: Some(match options.country {
                    CountryMode::Always => 0,
                    CountryMode::Never => 1,
                    CountryMode::ExceptFor(_) => 2,
                }),
            },
            FormRow::Check {
                id: BY_COUNTRY,
                label: t("mail.address.by_country").to_string(),
                on: options.by_country,
            },
        ];
        let country = options.country.clone();
        rows.extend(preview_rows(
            &self.request,
            self.record,
            &options.format(),
            &move |value: &str| match &country {
                CountryMode::Always => true,
                CountryMode::Never => false,
                CountryMode::ExceptFor(except) => value.trim() != except.trim(),
            },
            None,
            t,
        ));
        FormView {
            title: t("mail.address_block").to_string(),
            width: 520.0,
            rows,
            buttons: ok_cancel(t),
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        let options = &mut self.options;
        match event {
            FormEvent::Click(OK) | FormEvent::Enter => {
                return FormOutcome::Do(DialogAction::Insert(MergeFieldSpec::AddressBlock(
                    options.clone(),
                )))
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            FormEvent::Click(MATCH) => return FormOutcome::Do(DialogAction::OpenMatch),
            FormEvent::Toggle(FormId("name", 0)) => {
                options.name = match options.name {
                    Some(_) => None,
                    None => Some(NameFormat::default()),
                }
            }
            FormEvent::Choose(NAME_FORMAT, index) => {
                options.name = NameFormat::ALL.get(index).copied()
            }
            FormEvent::Choose(HONORIFIC, index) => {
                if let Some(honorific) = HONORIFICS.get(index) {
                    options.honorific = honorific.to_string();
                }
            }
            FormEvent::Toggle(COMPANY) => options.company ^= true,
            FormEvent::Toggle(POSTAL) => options.postal_address ^= true,
            FormEvent::Toggle(BY_COUNTRY) => options.by_country ^= true,
            FormEvent::Choose(COUNTRY, index) => {
                options.country = match index {
                    0 => CountryMode::Always,
                    1 => CountryMode::Never,
                    _ => CountryMode::ExceptFor(
                        if options.japanese {
                            "日本"
                        } else {
                            "United States"
                        }
                        .to_string(),
                    ),
                }
            }
            FormEvent::Click(PREVIOUS) => self.record = self.record.saturating_sub(1),
            FormEvent::Click(NEXT) => self.record += 1,
            _ => {}
        }
        FormOutcome::Stay
    }
}

// ---- あいさつ文の挿入 -------------------------------------------------------

const BEFORE: FormId = FormId::new("before");
const AFTER: FormId = FormId::new("after");
const INVALID: FormId = FormId::new("invalid");

/// あいさつ文の挿入.
pub struct GreetingDialog {
    pub options: GreetingLineOptions,
    invalid: TextEdit,
    request: Option<MergeRequest>,
    record: usize,
}

fn befores(japanese: bool) -> &'static [&'static str] {
    if japanese {
        &["", "拝啓", "前略"]
    } else {
        &["Dear", "To", ""]
    }
}

fn afters(japanese: bool) -> &'static [&'static str] {
    if japanese {
        &["様", "殿", "御中", ""]
    } else {
        &[",", ":", ""]
    }
}

impl GreetingDialog {
    pub fn new(japanese: bool, request: Option<MergeRequest>, record: usize) -> Self {
        let options = GreetingLineOptions::new(japanese);
        Self {
            invalid: TextEdit::new(options.invalid.clone()),
            options,
            request,
            record,
        }
    }

    pub fn field(&self) -> Option<&TextEdit> {
        Some(&self.invalid)
    }

    pub fn field_mut(&mut self) -> Option<&mut TextEdit> {
        Some(&mut self.invalid)
    }

    pub fn view(&self, t: Translate) -> FormView {
        let options = &self.options;
        let none = |text: &&str| {
            if text.is_empty() {
                t("cc.props.none").to_string()
            } else {
                text.to_string()
            }
        };
        let mut rows = vec![
            FormRow::Choice {
                id: BEFORE,
                label: t("mail.greeting.before").to_string(),
                options: befores(options.japanese).iter().map(none).collect(),
                selected: befores(options.japanese)
                    .iter()
                    .position(|before| *before == options.before),
            },
            FormRow::Choice {
                id: NAME_FORMAT,
                label: t("mail.address.name_format").to_string(),
                options: NameFormat::ALL
                    .iter()
                    .map(|format| format.sample(options.japanese).to_string())
                    .collect(),
                selected: NameFormat::ALL
                    .iter()
                    .position(|format| *format == options.name),
            },
            FormRow::Choice {
                id: AFTER,
                label: t("mail.greeting.after").to_string(),
                options: afters(options.japanese).iter().map(none).collect(),
                selected: afters(options.japanese)
                    .iter()
                    .position(|after| *after == options.after),
            },
            field(
                INVALID,
                t("mail.greeting.invalid"),
                &self.invalid,
                INVALID,
                260.0,
            ),
        ];
        let mut shown = options.clone();
        shown.invalid = self.invalid.text.clone();
        rows.extend(preview_rows(
            &self.request,
            self.record,
            &shown.format(),
            &|_| false,
            Some(&shown.invalid),
            t,
        ));
        FormView {
            title: t("mail.greeting").to_string(),
            width: 520.0,
            rows,
            buttons: ok_cancel(t),
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        let japanese = self.options.japanese;
        match event {
            FormEvent::Click(OK) | FormEvent::Enter => {
                let mut options = self.options.clone();
                options.invalid = self.invalid.text.clone();
                return FormOutcome::Do(DialogAction::Insert(MergeFieldSpec::GreetingLine(
                    options,
                )));
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            FormEvent::Click(MATCH) => return FormOutcome::Do(DialogAction::OpenMatch),
            FormEvent::Choose(BEFORE, index) => {
                if let Some(before) = befores(japanese).get(index) {
                    self.options.before = before.to_string();
                }
            }
            FormEvent::Choose(AFTER, index) => {
                if let Some(after) = afters(japanese).get(index) {
                    self.options.after = after.to_string();
                }
            }
            FormEvent::Choose(NAME_FORMAT, index) => {
                if let Some(name) = NameFormat::ALL.get(index) {
                    self.options.name = *name;
                }
            }
            FormEvent::Click(PREVIOUS) => self.record = self.record.saturating_sub(1),
            FormEvent::Click(NEXT) => self.record += 1,
            _ => {}
        }
        FormOutcome::Stay
    }
}

// ---- フィールドの対応 ---------------------------------------------------------

/// フィールドの対応: which column each of Word's address fields takes.
pub struct MatchFieldsDialog {
    data: MergeData,
    pub mapping: FieldMapping,
}

impl MatchFieldsDialog {
    pub fn new(data: MergeData, mapping: FieldMapping) -> Self {
        Self { data, mapping }
    }

    pub fn view(&self, t: Translate) -> FormView {
        let options: Vec<String> = std::iter::once(t("mail.match.none").to_string())
            .chain(self.data.columns.iter().cloned())
            .collect();
        let mut rows = vec![FormRow::Note(t("mail.match.note").to_string())];
        for (index, field) in AddressField::ALL.into_iter().enumerate() {
            rows.push(FormRow::Choice {
                id: FormId::at("map", index),
                label: t(field.label_key()).to_string(),
                options: options.clone(),
                selected: Some(self.mapping.column(field).map_or(0, |column| column + 1)),
            });
        }
        FormView {
            title: t("mail.match").to_string(),
            width: 640.0,
            rows,
            buttons: ok_cancel(t),
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        match event {
            FormEvent::Click(OK) | FormEvent::Enter => {
                FormOutcome::Do(DialogAction::Mapping(self.mapping.clone()))
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => FormOutcome::Close,
            FormEvent::Choose(FormId("map", field), choice) => {
                if let Some(field) = AddressField::ALL.get(field) {
                    self.mapping.set(*field, choice.checked_sub(1));
                }
                FormOutcome::Stay
            }
            _ => FormOutcome::Stay,
        }
    }
}

// ---- ルール -------------------------------------------------------------------

const RULE_FIELD: FormId = FormId::new("rule-field");
const RULE_OP: FormId = FormId::new("rule-op");
const RULE_VALUE: FormId = FormId::new("rule-value");
const RULE_THEN: FormId = FormId::new("rule-then");
const RULE_ELSE: FormId = FormId::new("rule-else");
const RULE_BOOKMARK: FormId = FormId::new("rule-bookmark");
const RULE_PROMPT: FormId = FormId::new("rule-prompt");
const RULE_DEFAULT: FormId = FormId::new("rule-default");
const RULE_ONCE: FormId = FormId::new("rule-once");

/// A ルール's dialog: If...Then...Else, Next Record If, Skip Record If,
/// Ask, Fill-in, Set Bookmark.
pub struct RuleDialog {
    /// The rule's menu key (`mail.rule.if`, …).
    pub key: &'static str,
    columns: Vec<String>,
    column: usize,
    operator: usize,
    value: TextEdit,
    then_text: TextEdit,
    else_text: TextEdit,
    bookmark: TextEdit,
    prompt: TextEdit,
    default: TextEdit,
    once: bool,
    focus: FormId,
}

impl RuleDialog {
    pub fn new(key: &'static str, columns: Vec<String>) -> Self {
        let focus = match key {
            "mail.rule.ask" | "mail.rule.set" => RULE_BOOKMARK,
            "mail.rule.fill_in" => RULE_PROMPT,
            _ => RULE_VALUE,
        };
        Self {
            key,
            columns,
            column: 0,
            operator: 0,
            value: TextEdit::new(String::new()),
            then_text: TextEdit::new(String::new()),
            else_text: TextEdit::new(String::new()),
            bookmark: TextEdit::new(String::new()),
            prompt: TextEdit::new(String::new()),
            default: TextEdit::new(String::new()),
            once: false,
            focus,
        }
    }

    fn fields(&self) -> Vec<FormId> {
        match self.key {
            "mail.rule.ask" => vec![RULE_BOOKMARK, RULE_PROMPT, RULE_DEFAULT],
            "mail.rule.fill_in" => vec![RULE_PROMPT, RULE_DEFAULT],
            "mail.rule.set" => vec![RULE_BOOKMARK, RULE_VALUE],
            "mail.rule.if" => vec![RULE_VALUE, RULE_THEN, RULE_ELSE],
            _ => vec![RULE_VALUE],
        }
    }

    pub fn field(&self) -> Option<&TextEdit> {
        Some(match self.focus {
            RULE_THEN => &self.then_text,
            RULE_ELSE => &self.else_text,
            RULE_BOOKMARK => &self.bookmark,
            RULE_PROMPT => &self.prompt,
            RULE_DEFAULT => &self.default,
            _ => &self.value,
        })
    }

    pub fn field_mut(&mut self) -> Option<&mut TextEdit> {
        Some(match self.focus {
            RULE_THEN => &mut self.then_text,
            RULE_ELSE => &mut self.else_text,
            RULE_BOOKMARK => &mut self.bookmark,
            RULE_PROMPT => &mut self.prompt,
            RULE_DEFAULT => &mut self.default,
            _ => &mut self.value,
        })
    }

    pub fn view(&self, t: Translate) -> FormView {
        let mut rows = Vec::new();
        let comparison = |rows: &mut Vec<FormRow>| {
            rows.push(FormRow::Choice {
                id: RULE_FIELD,
                label: t("mail.rule.field").to_string(),
                options: self.columns.clone(),
                selected: Some(self.column),
            });
            rows.push(FormRow::Choice {
                id: RULE_OP,
                label: t("mail.rule.operator").to_string(),
                options: RULE_OPERATORS
                    .iter()
                    .map(|operator| t(operator_key(operator)).to_string())
                    .collect(),
                selected: Some(self.operator),
            });
            rows.push(field(
                RULE_VALUE,
                t("mail.rule.value"),
                &self.value,
                self.focus,
                200.0,
            ));
        };
        match self.key {
            "mail.rule.ask" => {
                rows.push(field(
                    RULE_BOOKMARK,
                    t("mail.rule.bookmark"),
                    &self.bookmark,
                    self.focus,
                    200.0,
                ));
                rows.push(field(
                    RULE_PROMPT,
                    t("mail.rule.prompt"),
                    &self.prompt,
                    self.focus,
                    260.0,
                ));
                rows.push(field(
                    RULE_DEFAULT,
                    t("mail.rule.default"),
                    &self.default,
                    self.focus,
                    260.0,
                ));
                rows.push(FormRow::Check {
                    id: RULE_ONCE,
                    label: t("mail.rule.once").to_string(),
                    on: self.once,
                });
            }
            "mail.rule.fill_in" => {
                rows.push(field(
                    RULE_PROMPT,
                    t("mail.rule.prompt"),
                    &self.prompt,
                    self.focus,
                    260.0,
                ));
                rows.push(field(
                    RULE_DEFAULT,
                    t("mail.rule.default"),
                    &self.default,
                    self.focus,
                    260.0,
                ));
                rows.push(FormRow::Check {
                    id: RULE_ONCE,
                    label: t("mail.rule.once").to_string(),
                    on: self.once,
                });
            }
            "mail.rule.set" => {
                rows.push(field(
                    RULE_BOOKMARK,
                    t("mail.rule.bookmark"),
                    &self.bookmark,
                    self.focus,
                    200.0,
                ));
                rows.push(field(
                    RULE_VALUE,
                    t("mail.rule.value"),
                    &self.value,
                    self.focus,
                    260.0,
                ));
            }
            "mail.rule.if" => {
                comparison(&mut rows);
                rows.push(field(
                    RULE_THEN,
                    t("mail.rule.then"),
                    &self.then_text,
                    self.focus,
                    260.0,
                ));
                rows.push(field(
                    RULE_ELSE,
                    t("mail.rule.else"),
                    &self.else_text,
                    self.focus,
                    260.0,
                ));
            }
            _ => comparison(&mut rows),
        }
        FormView {
            title: t(self.key).to_string(),
            width: 480.0,
            rows,
            buttons: ok_cancel(t),
            problem: (self.columns.is_empty()
                && matches!(
                    self.key,
                    "mail.rule.if" | "mail.rule.next_if" | "mail.rule.skip_if"
                ))
            .then(|| t("mail.no_data").to_string()),
        }
    }

    fn rule(&self) -> Option<MergeRule> {
        let column = self.columns.get(self.column).cloned();
        let operator = RULE_OPERATORS
            .get(self.operator)
            .copied()
            .unwrap_or("=")
            .to_string();
        Some(match self.key {
            "mail.rule.ask" => MergeRule::Ask {
                bookmark: self.bookmark.text.trim().to_string(),
                prompt: self.prompt.text.clone(),
                default: self.default.text.clone(),
                once: self.once,
            },
            "mail.rule.fill_in" => MergeRule::FillIn {
                prompt: self.prompt.text.clone(),
                default: self.default.text.clone(),
                once: self.once,
            },
            "mail.rule.set" => MergeRule::Set {
                bookmark: self.bookmark.text.trim().to_string(),
                value: self.value.text.clone(),
            },
            "mail.rule.if" => MergeRule::If {
                field: column?,
                operator,
                value: self.value.text.clone(),
                then_text: self.then_text.text.clone(),
                else_text: self.else_text.text.clone(),
            },
            "mail.rule.next_if" => MergeRule::NextIf {
                field: column?,
                operator,
                value: self.value.text.clone(),
            },
            "mail.rule.skip_if" => MergeRule::SkipIf {
                field: column?,
                operator,
                value: self.value.text.clone(),
            },
            _ => return None,
        })
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        match event {
            FormEvent::Click(OK) | FormEvent::Enter => {
                if let Some(rule) = self.rule() {
                    return FormOutcome::Do(DialogAction::Insert(MergeFieldSpec::Rule(rule)));
                }
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            FormEvent::Click(id) if self.fields().contains(&id) => self.focus = id,
            FormEvent::Tab(backwards) => self.focus = tab(&self.fields(), self.focus, backwards),
            FormEvent::Choose(RULE_FIELD, index) => self.column = index,
            FormEvent::Choose(RULE_OP, index) => self.operator = index,
            FormEvent::Toggle(RULE_ONCE) => self.once ^= true,
            _ => {}
        }
        FormOutcome::Stay
    }
}

fn operator_key(operator: &str) -> &'static str {
    match operator {
        "=" => "mail.filter.equal",
        "<>" => "mail.filter.not_equal",
        "<" => "mail.filter.less",
        ">" => "mail.filter.greater",
        "<=" => "mail.filter.less_equal",
        ">=" => "mail.filter.greater_equal",
        "isblank" => "mail.filter.empty",
        _ => "mail.filter.not_empty",
    }
}

// ---- 完了と差し込み ----------------------------------------------------------

const RANGE: FormId = FormId::new("range");
const FROM: FormId = FormId::new("from");
const TO: FormId = FormId::new("to");

/// 新規文書への差し込み / プリンターに差し込み / PDF に差し込み: which
/// recipients.
pub struct FinishDialog {
    pub destination: Destination,
    current: usize,
    count: usize,
    range: usize,
    from: TextEdit,
    to: TextEdit,
    focus: FormId,
}

impl FinishDialog {
    pub fn new(destination: Destination, current: usize, count: usize) -> Self {
        Self {
            destination,
            current,
            count,
            range: 0,
            from: TextEdit::new("1".to_string()),
            to: TextEdit::new(count.max(1).to_string()),
            focus: FROM,
        }
    }

    pub fn field(&self) -> Option<&TextEdit> {
        Some(if self.focus == TO {
            &self.to
        } else {
            &self.from
        })
    }

    pub fn field_mut(&mut self) -> Option<&mut TextEdit> {
        Some(if self.focus == TO {
            &mut self.to
        } else {
            &mut self.from
        })
    }

    fn records(&self) -> Vec<usize> {
        match self.range {
            0 => (0..self.count).collect(),
            1 => vec![self.current.min(self.count.saturating_sub(1))],
            _ => {
                let from = self.from.text.trim().parse::<usize>().unwrap_or(1).max(1);
                let to = self
                    .to
                    .text
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(self.count)
                    .min(self.count);
                (from - 1..to).collect()
            }
        }
    }

    pub fn view(&self, t: Translate) -> FormView {
        let title = match self.destination {
            Destination::Document => "mail.finish.document",
            Destination::Printer => "mail.finish.print",
            Destination::Pdf => "mail.finish.pdf",
        };
        let mut rows = vec![FormRow::Choice {
            id: RANGE,
            label: t("mail.finish.records").to_string(),
            options: vec![
                t("mail.finish.all").to_string(),
                t("mail.finish.current").to_string(),
                t("mail.finish.range").to_string(),
            ],
            selected: Some(self.range),
        }];
        if self.range == 2 {
            rows.push(field(
                FROM,
                t("mail.finish.from"),
                &self.from,
                self.focus,
                60.0,
            ));
            rows.push(field(TO, t("mail.finish.to"), &self.to, self.focus, 60.0));
        }
        let records = self.records();
        FormView {
            title: t(title).to_string(),
            width: 420.0,
            rows,
            buttons: vec![
                FormButton::new(CANCEL, t("format.cancel")),
                FormButton::new(OK, t("format.ok"))
                    .primary()
                    .enabled(!records.is_empty()),
            ],
            problem: (self.count == 0).then(|| t("mail.no_recipients").to_string()),
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        match event {
            FormEvent::Click(OK) | FormEvent::Enter => {
                let records = self.records();
                if !records.is_empty() {
                    return FormOutcome::Do(DialogAction::Finish {
                        records,
                        destination: self.destination,
                    });
                }
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            FormEvent::Choose(RANGE, index) => self.range = index,
            FormEvent::Click(FROM) => self.focus = FROM,
            FormEvent::Click(TO) => self.focus = TO,
            FormEvent::Tab(_) => self.focus = if self.focus == FROM { TO } else { FROM },
            _ => {}
        }
        FormOutcome::Stay
    }
}

// ---- 封筒 ----------------------------------------------------------------------

const SIZE: FormId = FormId::new("size");
const DELIVERY: FormId = FormId::new("delivery");
const RETURN: FormId = FormId::new("return");
const OMIT_RETURN: FormId = FormId::new("omit-return");
const LEFT: FormId = FormId::new("left");
const TOP: FormId = FormId::new("top");
const PRINT: FormId = FormId::new("print");
const NEW_DOCUMENT: FormId = FormId::new("new-document");

fn millimetres(edit: &TextEdit) -> Option<f32> {
    edit.text.trim().trim_end_matches("mm").trim().parse().ok()
}

/// 封筒 (and 封筒オプション when a merge starts with envelopes).
pub struct EnvelopeDialog {
    japanese: bool,
    /// The merge's main document: its delivery address is the address
    /// block.
    main_document: bool,
    size: usize,
    delivery: TextEdit,
    return_address: TextEdit,
    omit_return: bool,
    left: TextEdit,
    top: TextEdit,
    focus: FormId,
}

impl EnvelopeDialog {
    pub fn new(japanese: bool, main_document: bool) -> Self {
        Self {
            japanese,
            main_document,
            size: if japanese { 0 } else { 6 },
            delivery: TextEdit::new(String::new()),
            return_address: TextEdit::new(String::new()),
            omit_return: false,
            left: TextEdit::new(String::new()),
            top: TextEdit::new(String::new()),
            focus: if main_document { RETURN } else { DELIVERY },
        }
    }

    fn fields(&self) -> Vec<FormId> {
        if self.main_document {
            vec![RETURN, LEFT, TOP]
        } else {
            vec![DELIVERY, RETURN, LEFT, TOP]
        }
    }

    pub fn field(&self) -> Option<&TextEdit> {
        Some(match self.focus {
            RETURN => &self.return_address,
            LEFT => &self.left,
            TOP => &self.top,
            _ => &self.delivery,
        })
    }

    pub fn field_mut(&mut self) -> Option<&mut TextEdit> {
        Some(match self.focus {
            RETURN => &mut self.return_address,
            LEFT => &mut self.left,
            TOP => &mut self.top,
            _ => &mut self.delivery,
        })
    }

    pub fn spec(&self) -> EnvelopeSpec {
        EnvelopeSpec {
            size: ENVELOPE_SIZES[self.size.min(ENVELOPE_SIZES.len() - 1)],
            delivery: self.delivery.text.clone(),
            return_address: self.return_address.text.clone(),
            omit_return: self.omit_return,
            merge: self.main_document,
            delivery_left_mm: millimetres(&self.left),
            delivery_top_mm: millimetres(&self.top),
            japanese: self.japanese,
        }
    }

    pub fn view(&self, t: Translate) -> FormView {
        let mut rows = Vec::new();
        if self.main_document {
            rows.push(FormRow::Note(t("mail.envelope.merge_note").to_string()));
        } else {
            rows.push(area(
                DELIVERY,
                t("mail.envelope.delivery"),
                &self.delivery,
                self.focus,
                4,
            ));
        }
        rows.push(area(
            RETURN,
            t("mail.envelope.return"),
            &self.return_address,
            self.focus,
            3,
        ));
        rows.push(FormRow::Check {
            id: OMIT_RETURN,
            label: t("mail.envelope.omit_return").to_string(),
            on: self.omit_return,
        });
        rows.push(FormRow::List {
            id: SIZE,
            label: t("mail.envelope.size").to_string(),
            items: ENVELOPE_SIZES
                .iter()
                .map(|size| format!("{} ({} × {} mm)", size.name, size.width_mm, size.height_mm))
                .collect(),
            selected: Some(self.size),
            rows: 6,
        });
        let (left, top) = self.spec().delivery_position();
        rows.push(field(
            LEFT,
            t("mail.envelope.left"),
            &self.left,
            self.focus,
            80.0,
        ));
        rows.push(field(
            TOP,
            t("mail.envelope.top"),
            &self.top,
            self.focus,
            80.0,
        ));
        rows.push(FormRow::Note(
            t("mail.envelope.auto")
                .replace("{left}", &format!("{left:.0}"))
                .replace("{top}", &format!("{top:.0}")),
        ));
        let buttons = if self.main_document {
            ok_cancel(t)
        } else {
            vec![
                FormButton::new(CANCEL, t("format.cancel")),
                FormButton::new(PRINT, t("mail.print")),
                FormButton::new(NEW_DOCUMENT, t("mail.new_document")).primary(),
            ]
        };
        FormView {
            title: t(if self.main_document {
                "mail.envelope.options"
            } else {
                "mail.envelopes"
            })
            .to_string(),
            width: 520.0,
            rows,
            buttons,
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        match event {
            FormEvent::Click(OK) if self.main_document => {
                return FormOutcome::Do(DialogAction::Envelope {
                    spec: self.spec(),
                    output: LayoutOutput::MainDocument,
                })
            }
            FormEvent::Click(NEW_DOCUMENT) => {
                return FormOutcome::Do(DialogAction::Envelope {
                    spec: self.spec(),
                    output: LayoutOutput::NewDocument,
                })
            }
            FormEvent::Click(PRINT) => {
                return FormOutcome::Do(DialogAction::Envelope {
                    spec: self.spec(),
                    output: LayoutOutput::Print,
                })
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            // Enter breaks a line in an address.
            FormEvent::Enter if matches!(self.focus, DELIVERY | RETURN) => {
                if let Some(field) = self.field_mut() {
                    field.insert("\n");
                }
            }
            FormEvent::Enter => {
                return FormOutcome::Do(DialogAction::Envelope {
                    spec: self.spec(),
                    output: if self.main_document {
                        LayoutOutput::MainDocument
                    } else {
                        LayoutOutput::NewDocument
                    },
                })
            }
            FormEvent::Click(id) if self.fields().contains(&id) => self.focus = id,
            FormEvent::Tab(backwards) => self.focus = tab(&self.fields(), self.focus, backwards),
            FormEvent::Choose(SIZE, index) => self.size = index.min(ENVELOPE_SIZES.len() - 1),
            FormEvent::Toggle(OMIT_RETURN) => self.omit_return ^= true,
            _ => {}
        }
        FormOutcome::Stay
    }
}

// ---- ラベル --------------------------------------------------------------------

const PRODUCT: FormId = FormId::new("product");
const ADDRESS: FormId = FormId::new("address");

/// ラベル (and ラベル オプション when a merge starts with labels).
pub struct LabelDialog {
    main_document: bool,
    product: usize,
    address: TextEdit,
}

impl LabelDialog {
    pub fn new(japanese: bool, main_document: bool) -> Self {
        Self {
            main_document,
            product: if japanese { 0 } else { 9 },
            address: TextEdit::new(String::new()),
        }
    }

    pub fn field(&self) -> Option<&TextEdit> {
        (!self.main_document).then_some(&self.address)
    }

    pub fn field_mut(&mut self) -> Option<&mut TextEdit> {
        (!self.main_document).then_some(&mut self.address)
    }

    pub fn product(&self) -> LabelProduct {
        LABEL_PRODUCTS[self.product.min(LABEL_PRODUCTS.len() - 1)]
    }

    pub fn view(&self, t: Translate) -> FormView {
        let product = self.product();
        let mut rows = Vec::new();
        if !self.main_document {
            rows.push(area(
                ADDRESS,
                t("mail.labels.address"),
                &self.address,
                ADDRESS,
                4,
            ));
        }
        rows.push(FormRow::List {
            id: PRODUCT,
            label: t("mail.labels.product").to_string(),
            items: LABEL_PRODUCTS.iter().map(LabelProduct::label).collect(),
            selected: Some(self.product),
            rows: 7,
        });
        rows.push(FormRow::Note(
            t("mail.labels.details")
                .replace("{columns}", &product.columns.to_string())
                .replace("{rows}", &product.rows.to_string())
                .replace("{width}", &format!("{:.1}", product.label_width_mm))
                .replace("{height}", &format!("{:.1}", product.label_height_mm)),
        ));
        let buttons = if self.main_document {
            ok_cancel(t)
        } else {
            vec![
                FormButton::new(CANCEL, t("format.cancel")),
                FormButton::new(PRINT, t("mail.print")),
                FormButton::new(NEW_DOCUMENT, t("mail.new_document")).primary(),
            ]
        };
        FormView {
            title: t(if self.main_document {
                "mail.labels.options"
            } else {
                "mail.labels"
            })
            .to_string(),
            width: 520.0,
            rows,
            buttons,
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        let text = (!self.address.text.is_empty()).then(|| self.address.text.clone());
        match event {
            FormEvent::Click(OK) if self.main_document => FormOutcome::Do(DialogAction::Labels {
                product: self.product(),
                text: None,
                output: LayoutOutput::MainDocument,
            }),
            FormEvent::Click(NEW_DOCUMENT) => FormOutcome::Do(DialogAction::Labels {
                product: self.product(),
                text,
                output: LayoutOutput::NewDocument,
            }),
            FormEvent::Click(PRINT) => FormOutcome::Do(DialogAction::Labels {
                product: self.product(),
                text,
                output: LayoutOutput::Print,
            }),
            FormEvent::Click(CANCEL) | FormEvent::Escape => FormOutcome::Close,
            FormEvent::Enter if !self.main_document => {
                self.address.insert("\n");
                FormOutcome::Stay
            }
            FormEvent::Enter => FormOutcome::Do(DialogAction::Labels {
                product: self.product(),
                text: None,
                output: LayoutOutput::MainDocument,
            }),
            FormEvent::Choose(PRODUCT, index) => {
                self.product = index.min(LABEL_PRODUCTS.len() - 1);
                FormOutcome::Stay
            }
            _ => FormOutcome::Stay,
        }
    }
}

// ---- エラーのチェック ----------------------------------------------------------

/// エラーのチェック: what is wrong with the merge.
pub struct ErrorCheckDialog {
    pub problems: Vec<MergeProblem>,
}

impl ErrorCheckDialog {
    pub fn new(problems: Vec<MergeProblem>) -> Self {
        Self { problems }
    }

    pub fn view(&self, t: Translate) -> FormView {
        let mut rows = vec![FormRow::Note(t("mail.check.note").to_string())];
        if self.problems.is_empty() {
            rows.push(FormRow::Heading(t("mail.check.none").to_string()));
        }
        for problem in &self.problems {
            rows.push(FormRow::Note(match problem {
                MergeProblem::EmptyDataSource => t("mail.check.empty").to_string(),
                MergeProblem::NoRecipients => t("mail.no_recipients").to_string(),
                MergeProblem::MissingColumn(name) => {
                    t("mail.check.missing").replace("{name}", name)
                }
                MergeProblem::UnmatchedAddress => t("mail.check.unmatched").to_string(),
            }));
        }
        FormView {
            title: t("mail.check").to_string(),
            width: 460.0,
            rows,
            buttons: vec![FormButton::new(OK, t("format.ok")).primary()],
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<DialogAction> {
        match event {
            FormEvent::Click(OK) | FormEvent::Enter | FormEvent::Escape => FormOutcome::Close,
            _ => FormOutcome::Stay,
        }
    }
}
