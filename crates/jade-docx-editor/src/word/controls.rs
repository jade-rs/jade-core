//! Content controls and legacy form fields over any [`WordSession`], as the
//! 開発 tab has them: putting one in, what a click on one does — a box
//! ticked, a list or a calendar dropped, a picture asked for, a placeholder
//! selected so typing replaces it — the box Word draws round the one the
//! caret is in, and the プロパティ dialogs.
//!
//! What a control is and what changing it does to the file are
//! `jade-docx-editor`'s (`word_document::content_controls`); every change is
//! one undo step there.

pub use crate::canvas_renderer::TextRegion;
pub use crate::word_document::content_controls::edit::{default_placeholder, FormFieldOptions};
pub use crate::word_document::content_controls::{
    CheckSymbol, ContentControl, ContentControlAction, ContentControlKind,
    ContentControlProperties, ControlAppearance, FormFieldKind, LegacyFormField, ListEntry,
    BUILDING_BLOCK_GALLERIES, DATE_FORMATS,
};
use jade_selection::{SelectionRange, TextPosition};

use super::form_view::{
    FieldView, FormButton, FormEvent, FormId, FormOutcome, FormRow, FormView, Translate,
};
use super::{Position, WordSession};
use crate::text_edit::TextEdit;

/// The controls the 開発 tab puts in, in its order.
pub const INSERTABLE: [ContentControlKind; 9] = [
    ContentControlKind::RichText,
    ContentControlKind::PlainText,
    ContentControlKind::Picture,
    ContentControlKind::BuildingBlockGallery,
    ContentControlKind::CheckBox,
    ContentControlKind::ComboBox,
    ContentControlKind::DropDownList,
    ContentControlKind::Date,
    ContentControlKind::RepeatingSection,
];

/// The i18n key of a kind's name.
pub fn kind_key(kind: &ContentControlKind) -> &'static str {
    match kind {
        ContentControlKind::RichText => "cc.rich_text",
        ContentControlKind::PlainText => "cc.plain_text",
        ContentControlKind::Picture => "cc.picture",
        ContentControlKind::CheckBox => "cc.check_box",
        ContentControlKind::ComboBox => "cc.combo_box",
        ContentControlKind::DropDownList => "cc.drop_down",
        ContentControlKind::Date => "cc.date",
        ContentControlKind::BuildingBlockGallery => "cc.building_block",
        ContentControlKind::RepeatingSection => "cc.repeating",
        ContentControlKind::RepeatingSectionItem => "cc.repeating_item",
        ContentControlKind::Group => "cc.group",
        ContentControlKind::Other(_) => "cc.rich_text",
    }
}

/// What a click on the pages did to a control or a form field.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlClick {
    /// Nothing: the click was not on one, or on one that takes typing.
    None,
    /// A check box was ticked or cleared.
    Toggled,
    /// A placeholder was selected, so what is typed replaces it.
    SelectedPlaceholder,
    /// A combo box's or drop-down list's entries, to drop under it.
    List {
        control: usize,
        entries: Vec<String>,
        selected: Option<usize>,
    },
    /// A date picker's calendar.
    Date(DatePicker),
    /// A picture control: the shell asks for a picture.
    Picture { control: usize },
    /// A building block gallery's entries.
    Gallery { control: usize, blocks: Vec<String> },
    /// A form drop-down's entries.
    FormList {
        field: usize,
        entries: Vec<String>,
        selected: usize,
    },
}

/// What the shells draw over the pages for the controls: boxes, title tabs
/// and the buttons on them, tags in design mode, and form fields' grey.
#[derive(Clone, Debug, PartialEq)]
pub enum ControlChrome {
    /// The box round the control the caret is in, with its title on a tab
    /// above its top-left corner, and a drop button at its right for one
    /// that drops a list or a calendar.
    Box {
        control: usize,
        regions: Vec<TextRegion>,
        title: Option<String>,
        /// `RRGGBB`.
        color: String,
        drop_button: bool,
        /// A repeating section item's ＋.
        add_button: bool,
    },
    /// Design mode's (or 開始/終了タグ's) tags at the control's ends.
    Tags {
        control: usize,
        start: Option<TextRegion>,
        end: Option<TextRegion>,
        title: String,
        color: String,
    },
    /// The grey Word shades a legacy form field with.
    Shading { regions: Vec<TextRegion> },
}

/// The colour Word draws a control's box in when it has none of its own.
pub const DEFAULT_CONTROL_COLOR: &str = "8EAADB";

/// A date picker's calendar: the month shown and the date chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatePicker {
    pub control: usize,
    pub year: i32,
    pub month: u32,
    pub selected: Option<(i32, u32, u32)>,
    pub today: (i32, u32, u32),
}

/// Days in a month of the Gregorian calendar.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

/// The weekday of a date, 0 for Sunday.
pub fn weekday(year: i32, month: u32, day: u32) -> u32 {
    // Zeller's congruence, shifted to Sunday = 0.
    let (y, m) = if month < 3 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let k = y.rem_euclid(100);
    let j = y.div_euclid(100);
    let h = (day as i32 + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    ((h + 6) % 7) as u32
}

impl DatePicker {
    fn new(control: usize, full_date: Option<&str>, today: (i32, u32, u32)) -> Self {
        let selected = full_date.and_then(|date| {
            let year = date.get(0..4)?.parse().ok()?;
            let month = date.get(5..7)?.parse().ok()?;
            let day = date.get(8..10)?.parse().ok()?;
            Some((year, month, day))
        });
        let (year, month, _) = selected.unwrap_or(today);
        Self {
            control,
            year,
            month,
            selected,
            today,
        }
    }

    /// The month's days in weeks from Sunday; `None` before the first and
    /// after the last.
    pub fn weeks(&self) -> Vec<[Option<u32>; 7]> {
        let first = weekday(self.year, self.month, 1) as usize;
        let days = days_in_month(self.year, self.month);
        let mut weeks = Vec::new();
        let mut week = [None; 7];
        let mut column = first;
        for day in 1..=days {
            week[column] = Some(day);
            column += 1;
            if column == 7 {
                weeks.push(week);
                week = [None; 7];
                column = 0;
            }
        }
        if column > 0 {
            weeks.push(week);
        }
        weeks
    }

    /// The month before (or after).
    pub fn step(&mut self, forwards: bool) {
        if forwards {
            if self.month == 12 {
                self.month = 1;
                self.year += 1;
            } else {
                self.month += 1;
            }
        } else if self.month == 1 {
            self.month = 12;
            self.year -= 1;
        } else {
            self.month -= 1;
        }
    }

    /// The action that puts `day` of the month shown in the control.
    pub fn choose(&self, day: u32) -> ContentControlAction {
        ContentControlAction::SetDate {
            control: self.control,
            date: format!("{:04}-{:02}-{:02}", self.year, self.month, day),
        }
    }

    /// The month as a title: `2026年10月`, or `October 2026`.
    pub fn title(&self, japanese: bool) -> String {
        const MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        if japanese {
            format!("{}年{}月", self.year, self.month)
        } else {
            format!(
                "{} {}",
                MONTHS[(self.month as usize).clamp(1, 12) - 1],
                self.year
            )
        }
    }
}

fn to_position(position: TextPosition) -> Position {
    (position.paragraph, position.offset)
}

/// Content controls and form fields over every session.
pub trait WordControls: WordSession {
    /// Every content control of the body, in document order.
    fn content_controls(&self) -> Vec<ContentControl> {
        self.word_document().content_controls()
    }

    /// Every legacy form field of the body, in document order.
    fn form_fields(&self) -> Vec<LegacyFormField> {
        self.word_document().legacy_form_fields()
    }

    /// The innermost control at a position.
    fn control_at(&self, (paragraph, offset): Position) -> Option<ContentControl> {
        self.word_document()
            .content_control_at(TextPosition::new(paragraph, offset))
    }

    /// The control the caret (the selection's focus) is in.
    fn caret_control(&self) -> Option<ContentControl> {
        let range = self.selection_range()?;
        self.word_document().content_control_at(range.focus)
    }

    /// The form field whose result or box is at `position`.
    fn form_field_at(&self, (paragraph, offset): Position) -> Option<LegacyFormField> {
        let position = TextPosition::new(paragraph, offset);
        let fields = self.form_fields();
        // A check box's box is drawn where the field is, one character wide;
        // it wins over a text box that ends right there.
        let check = fields.iter().find(|field| {
            field.kind == FormFieldKind::CheckBox
                && field.result_start.paragraph == paragraph
                && field.result_start.offset <= offset
                && offset <= field.result_start.offset + '\u{2610}'.len_utf8()
        });
        check
            .or_else(|| {
                fields.iter().find(|field| {
                    field.kind != FormFieldKind::CheckBox
                        && field.result_start <= position
                        && position <= field.result_end
                })
            })
            .cloned()
    }

    /// Whether the document is in デザイン モード.
    fn design_mode(&self) -> bool {
        self.word_document().forms_design_mode()
    }

    /// Whether the document is protected so only its forms can be filled in.
    fn protected_for_forms(&self) -> bool {
        self.word_document().protected_for_forms()
    }

    /// Runs a content-control action on the selection, one undo step.
    fn run_control_action(&mut self, action: ContentControlAction) -> Result<(), String> {
        let range = self
            .selection_range()
            .unwrap_or_else(|| SelectionRange::collapsed(TextPosition::new(0, 0)));
        self.execute_command(Box::new(action.command(range)), false)
    }

    /// 開発 → a control, around the selection or at the caret.
    fn insert_content_control(
        &mut self,
        kind: ContentControlKind,
        japanese: bool,
    ) -> Result<(), String> {
        self.run_control_action(ContentControlAction::Insert { kind, japanese })
    }

    /// What a click at `position` does to the control or form field there.
    /// The caret is already there. `today` is the local date, for a date
    /// picker with no date yet.
    fn click_control(&mut self, position: Position, today: (i32, u32, u32)) -> ControlClick {
        if let Some(field) = self.form_field_at(position) {
            match field.kind {
                FormFieldKind::CheckBox if field.enabled => {
                    let action = ContentControlAction::SetFormCheckBox {
                        field: field.index,
                        checked: !field.checked,
                    };
                    return match self.run_control_action(action) {
                        Ok(()) => ControlClick::Toggled,
                        Err(_) => ControlClick::None,
                    };
                }
                FormFieldKind::Text
                    if field.enabled
                        && field.result
                            == crate::word_document::content_controls::forms::EMPTY_TEXT_FIELD =>
                {
                    self.set_selection_range(Some(SelectionRange::new(
                        field.result_start,
                        field.result_end,
                    )));
                    return ControlClick::SelectedPlaceholder;
                }
                FormFieldKind::DropDown if field.enabled => {
                    return ControlClick::FormList {
                        field: field.index,
                        entries: field.entries,
                        selected: field.selected,
                    }
                }
                _ => {}
            }
        }
        let Some(control) = self.control_at(position) else {
            return ControlClick::None;
        };
        if self.design_mode() {
            return ControlClick::None;
        }
        match control.kind {
            ContentControlKind::CheckBox if !control.cannot_edit => {
                let action = ContentControlAction::SetChecked {
                    control: control.index,
                    checked: !control.checked,
                };
                match self.run_control_action(action) {
                    Ok(()) => ControlClick::Toggled,
                    Err(_) => ControlClick::None,
                }
            }
            ContentControlKind::ComboBox | ContentControlKind::DropDownList
                if !control.cannot_edit =>
            {
                let selected = control
                    .entries
                    .iter()
                    .position(|entry| entry.display == control.text);
                ControlClick::List {
                    control: control.index,
                    entries: control
                        .entries
                        .iter()
                        .map(|entry| entry.display.clone())
                        .collect(),
                    selected,
                }
            }
            ContentControlKind::Date if !control.cannot_edit => ControlClick::Date(
                DatePicker::new(control.index, control.full_date.as_deref(), today),
            ),
            ContentControlKind::Picture if !control.cannot_edit => ControlClick::Picture {
                control: control.index,
            },
            ContentControlKind::BuildingBlockGallery if !control.cannot_edit => {
                ControlClick::Gallery {
                    control: control.index,
                    blocks: self
                        .word_document()
                        .building_blocks(control.gallery.as_deref()),
                }
            }
            _ if control.showing_placeholder => {
                self.set_selection_range(Some(SelectionRange::new(control.start, control.end)));
                ControlClick::SelectedPlaceholder
            }
            _ => ControlClick::None,
        }
    }

    /// Typing while the selection is a control's placeholder: the text
    /// replaces it, out of the placeholder's grey, one undo step. `None`
    /// when the selection is not one, and typing goes on as usual.
    fn type_over_placeholder(&mut self, text: &str) -> Option<Result<(), String>> {
        let range = self.selection_range()?;
        let (start, end) = range.normalized();
        if !self
            .word_document()
            .paragraph_may_hold_controls(start.paragraph)
        {
            return None;
        }
        // A form text box whose result is selected, or still Word's five
        // spaces: the text replaces it.
        let field = self.form_fields().into_iter().find(|field| {
            field.kind == FormFieldKind::Text
                && field.result_start <= start
                && end <= field.result_end
                && ((start == field.result_start && end == field.result_end)
                    || field.result
                        == crate::word_document::content_controls::forms::EMPTY_TEXT_FIELD)
        });
        if let Some(field) = field {
            return Some(self.run_control_action(ContentControlAction::SetFormText {
                field: field.index,
                text: text.to_string(),
            }));
        }
        let control = self.word_document().content_control_at(start)?;
        if !control.showing_placeholder
            || !control.kind.takes_typing()
            || control.cannot_edit
            || control.block
            || control.start != start
            || control.end != end
        {
            return None;
        }
        Some(self.run_control_action(ContentControlAction::SetText {
            control: control.index,
            text: text.to_string(),
        }))
    }

    /// Tab in a document protected for forms: the next form field or
    /// content control after the caret (⇧Tab the one before), its content
    /// selected, round to the first after the last. `false` when the
    /// document is not protected so, and Tab does what it always does.
    fn tab_to_form_field(&mut self, backwards: bool) -> bool {
        if !self.protected_for_forms() {
            return false;
        }
        let mut stops: Vec<(TextPosition, TextPosition)> = self
            .form_fields()
            .into_iter()
            .filter(|field| field.enabled)
            .map(|field| (field.result_start, field.result_end))
            .chain(
                self.content_controls()
                    .into_iter()
                    .filter(|control| {
                        !control.block
                            && !control.cannot_edit
                            && control.kind != ContentControlKind::Group
                    })
                    .map(|control| (control.start, control.end)),
            )
            .collect();
        if stops.is_empty() {
            return true;
        }
        stops.sort();
        stops.dedup();
        let caret = self
            .selection_range()
            .map(|range| range.normalized().0)
            .unwrap_or_else(|| TextPosition::new(0, 0));
        let next = if backwards {
            stops
                .iter()
                .rev()
                .find(|(start, _)| *start < caret)
                .or(stops.last())
        } else {
            stops
                .iter()
                .find(|(start, _)| *start > caret)
                .or(stops.first())
        };
        if let Some((start, end)) = next {
            self.set_selection_range(Some(SelectionRange::new(*start, *end)));
        }
        true
    }

    /// What to draw over the pages: the caret control's box, and in design
    /// mode the tags of every control; the form fields' grey.
    fn control_chrome(&self) -> Vec<ControlChrome> {
        let controls = self.content_controls();
        let fields = self.form_fields();
        if controls.is_empty() && fields.is_empty() {
            return Vec::new();
        }
        let design = self.design_mode();
        let caret = self.selection_range().map(|range| range.focus);
        let color_of = |control: &ContentControl| {
            control
                .color
                .clone()
                .unwrap_or_else(|| DEFAULT_CONTROL_COLOR.to_string())
        };
        let mut chrome = Vec::new();
        // The caret's control and the controls round it.
        let mut boxed: Vec<usize> = Vec::new();
        if let Some(caret) = caret {
            let mut current = controls
                .iter()
                .filter(|control| control.contains(caret))
                .max_by_key(|control| control.depth)
                .map(|control| control.index);
            while let Some(index) = current {
                boxed.push(index);
                current = controls.get(index).and_then(|control| control.parent);
            }
        }
        for control in &controls {
            let tags = design || control.appearance == ControlAppearance::Tags;
            if tags {
                let start = self
                    .text_regions(to_position(control.start), to_position(control.start))
                    .into_iter()
                    .next();
                let end = self
                    .text_regions(to_position(control.end), to_position(control.end))
                    .into_iter()
                    .last();
                chrome.push(ControlChrome::Tags {
                    control: control.index,
                    start,
                    end,
                    title: control.title.clone().unwrap_or_default(),
                    color: color_of(control),
                });
                continue;
            }
            if control.appearance == ControlAppearance::Hidden || !boxed.contains(&control.index) {
                continue;
            }
            let regions = self.text_regions(to_position(control.start), to_position(control.end));
            if regions.is_empty() {
                continue;
            }
            chrome.push(ControlChrome::Box {
                control: control.index,
                regions,
                title: control.label().map(str::to_string),
                color: color_of(control),
                drop_button: matches!(
                    control.kind,
                    ContentControlKind::ComboBox
                        | ContentControlKind::DropDownList
                        | ContentControlKind::Date
                        | ContentControlKind::BuildingBlockGallery
                ),
                add_button: control.kind == ContentControlKind::RepeatingSectionItem
                    && !controls
                        .get(control.parent.unwrap_or(usize::MAX))
                        .is_some_and(|section| section.fixed_items),
            });
        }
        for field in &fields {
            let regions = if field.kind == FormFieldKind::CheckBox {
                // The box is the field's drawn result, one character on.
                let start = to_position(field.result_start);
                self.text_regions(start, (start.0, start.1 + 3))
            } else {
                self.text_regions(
                    to_position(field.result_start),
                    to_position(field.result_end),
                )
            };
            if !regions.is_empty() {
                chrome.push(ControlChrome::Shading { regions });
            }
        }
        chrome
    }
}

impl<T: WordSession + ?Sized> WordControls for T {}

/// The controls' property dialog: コンテンツ コントロールのプロパティ.
pub struct ControlPropertiesForm {
    pub control: usize,
    pub kind: ContentControlKind,
    pub properties: ContentControlProperties,
    /// The document's character styles, id and name, for スタイル.
    pub styles: Vec<(String, String)>,
    title: TextEdit,
    tag: TextEdit,
    section_title: TextEdit,
    category: TextEdit,
    date_format: TextEdit,
    entry_display: TextEdit,
    entry_value: TextEdit,
    focus: FormId,
    selected_entry: Option<usize>,
}

const TITLE: FormId = FormId::new("title");
const TAG: FormId = FormId::new("tag");
const SECTION_TITLE: FormId = FormId::new("section-title");
const CATEGORY: FormId = FormId::new("category");
const DATE_FORMAT: FormId = FormId::new("date-format");
const ENTRY_DISPLAY: FormId = FormId::new("entry-display");
const ENTRY_VALUE: FormId = FormId::new("entry-value");
const APPEARANCE: FormId = FormId::new("appearance");
const COLOR: FormId = FormId::new("color");
const STYLE: FormId = FormId::new("style");
const TEMPORARY: FormId = FormId::new("temporary");
const MULTI_LINE: FormId = FormId::new("multi-line");
const NO_DELETE: FormId = FormId::new("no-delete");
const NO_EDIT: FormId = FormId::new("no-edit");
const ENTRIES: FormId = FormId::new("entries");
const ENTRY_ADD: FormId = FormId::new("entry-add");
const ENTRY_CHANGE: FormId = FormId::new("entry-change");
const ENTRY_REMOVE: FormId = FormId::new("entry-remove");
const ENTRY_UP: FormId = FormId::new("entry-up");
const ENTRY_DOWN: FormId = FormId::new("entry-down");
const DATE_FORMATS_LIST: FormId = FormId::new("date-formats");
const CHECKED_SYMBOL: FormId = FormId::new("checked-symbol");
const UNCHECKED_SYMBOL: FormId = FormId::new("unchecked-symbol");
const GALLERY: FormId = FormId::new("gallery");
const FIXED_ITEMS: FormId = FormId::new("fixed-items");
const OK: FormId = FormId::new("ok");
const CANCEL: FormId = FormId::new("cancel");
const REMOVE_CONTROL: FormId = FormId::new("remove-control");

/// The colours the 色 choice offers: none (Word's automatic) and Word's
/// standard ones.
pub const CONTROL_COLORS: [Option<&str>; 7] = [
    None,
    Some("4472C4"),
    Some("ED7D31"),
    Some("70AD47"),
    Some("FFC000"),
    Some("FF0000"),
    Some("7030A0"),
];

/// The symbols a check box can show ticked and not.
pub const CHECKED_SYMBOLS: [char; 4] = ['\u{2612}', '\u{2611}', '\u{2714}', '\u{2716}'];
pub const UNCHECKED_SYMBOLS: [char; 3] = ['\u{2610}', '\u{25A1}', '\u{2B1C}'];

impl ControlPropertiesForm {
    pub fn new(control: &ContentControl, styles: Vec<(String, String)>) -> Self {
        let properties = ContentControlProperties::of(control);
        Self {
            control: control.index,
            kind: control.kind.clone(),
            title: TextEdit::new(properties.title.clone()),
            tag: TextEdit::new(properties.tag.clone()),
            section_title: TextEdit::new(properties.section_title.clone()),
            category: TextEdit::new(properties.category.clone()),
            date_format: TextEdit::new(properties.date_format.clone()),
            entry_display: TextEdit::new(String::new()),
            entry_value: TextEdit::new(String::new()),
            focus: TITLE,
            selected_entry: None,
            properties,
            styles,
        }
    }

    fn fields(&self) -> Vec<FormId> {
        let mut fields = vec![TITLE, TAG];
        match self.kind {
            ContentControlKind::ComboBox | ContentControlKind::DropDownList => {
                fields.extend([ENTRY_DISPLAY, ENTRY_VALUE]);
            }
            ContentControlKind::Date => fields.push(DATE_FORMAT),
            ContentControlKind::BuildingBlockGallery => fields.push(CATEGORY),
            ContentControlKind::RepeatingSection => fields.push(SECTION_TITLE),
            _ => {}
        }
        fields
    }

    /// The field typing goes into.
    pub fn field(&self) -> &TextEdit {
        match self.focus {
            TAG => &self.tag,
            SECTION_TITLE => &self.section_title,
            CATEGORY => &self.category,
            DATE_FORMAT => &self.date_format,
            ENTRY_DISPLAY => &self.entry_display,
            ENTRY_VALUE => &self.entry_value,
            _ => &self.title,
        }
    }

    pub fn field_mut(&mut self) -> &mut TextEdit {
        match self.focus {
            TAG => &mut self.tag,
            SECTION_TITLE => &mut self.section_title,
            CATEGORY => &mut self.category,
            DATE_FORMAT => &mut self.date_format,
            ENTRY_DISPLAY => &mut self.entry_display,
            ENTRY_VALUE => &mut self.entry_value,
            _ => &mut self.title,
        }
    }

    fn field_row(&self, id: FormId, label: &'static str, edit: &TextEdit, t: Translate) -> FormRow {
        FormRow::Field {
            id,
            label: t(label).to_string(),
            value: FieldView::of(edit),
            focused: self.focus == id,
            width: 220.0,
        }
    }

    /// The dialog as it is drawn.
    pub fn view(&self, t: Translate) -> FormView {
        let properties = &self.properties;
        let mut rows = vec![
            FormRow::Heading(t("cc.props.general").to_string()),
            self.field_row(TITLE, "cc.props.title", &self.title, t),
            self.field_row(TAG, "cc.props.tag", &self.tag, t),
            FormRow::Choice {
                id: APPEARANCE,
                label: t("cc.props.show_as").to_string(),
                options: vec![
                    t("cc.props.bounding_box").to_string(),
                    t("cc.props.tags").to_string(),
                    t("cc.props.none").to_string(),
                ],
                selected: Some(match properties.appearance {
                    ControlAppearance::BoundingBox => 0,
                    ControlAppearance::Tags => 1,
                    ControlAppearance::Hidden => 2,
                }),
            },
            FormRow::Choice {
                id: COLOR,
                label: t("cc.props.color").to_string(),
                options: CONTROL_COLORS
                    .iter()
                    .map(|color| match color {
                        None => t("cc.props.automatic").to_string(),
                        Some(color) => format!("#{color}"),
                    })
                    .collect(),
                selected: CONTROL_COLORS
                    .iter()
                    .position(|color| color.map(str::to_string) == properties.color),
            },
            FormRow::Choice {
                id: STYLE,
                label: t("cc.props.style").to_string(),
                options: std::iter::once(t("cc.props.default_style").to_string())
                    .chain(self.styles.iter().map(|(_, name)| name.clone()))
                    .collect(),
                selected: Some(
                    properties
                        .style
                        .as_ref()
                        .and_then(|style| self.styles.iter().position(|(id, _)| id == style))
                        .map_or(0, |index| index + 1),
                ),
            },
            FormRow::Check {
                id: TEMPORARY,
                label: t("cc.props.temporary").to_string(),
                on: properties.temporary,
            },
        ];
        if self.kind == ContentControlKind::PlainText {
            rows.push(FormRow::Check {
                id: MULTI_LINE,
                label: t("cc.props.multi_line").to_string(),
                on: properties.multi_line,
            });
        }
        rows.push(FormRow::Heading(t("cc.props.locking").to_string()));
        rows.push(FormRow::Check {
            id: NO_DELETE,
            label: t("cc.props.no_delete").to_string(),
            on: properties.cannot_delete,
        });
        rows.push(FormRow::Check {
            id: NO_EDIT,
            label: t("cc.props.no_edit").to_string(),
            on: properties.cannot_edit,
        });
        match self.kind {
            ContentControlKind::ComboBox | ContentControlKind::DropDownList => {
                rows.push(FormRow::Heading(t("cc.props.list").to_string()));
                rows.push(FormRow::List {
                    id: ENTRIES,
                    label: t("cc.props.entries").to_string(),
                    items: properties
                        .entries
                        .iter()
                        .map(|entry| format!("{}  —  {}", entry.display, entry.value))
                        .collect(),
                    selected: self.selected_entry,
                    rows: 5,
                });
                rows.push(self.field_row(
                    ENTRY_DISPLAY,
                    "cc.props.display_name",
                    &self.entry_display,
                    t,
                ));
                rows.push(self.field_row(ENTRY_VALUE, "cc.props.value", &self.entry_value, t));
                let selected = self.selected_entry.is_some();
                rows.push(FormRow::Buttons(vec![
                    FormButton::new(ENTRY_ADD, t("cc.props.add")),
                    FormButton::new(ENTRY_CHANGE, t("cc.props.modify")).enabled(selected),
                    FormButton::new(ENTRY_REMOVE, t("cc.props.remove")).enabled(selected),
                    FormButton::new(ENTRY_UP, t("cc.props.up")).enabled(selected),
                    FormButton::new(ENTRY_DOWN, t("cc.props.down")).enabled(selected),
                ]));
            }
            ContentControlKind::Date => {
                rows.push(FormRow::Heading(t("cc.props.date").to_string()));
                rows.push(self.field_row(
                    DATE_FORMAT,
                    "cc.props.date_format",
                    &self.date_format,
                    t,
                ));
                rows.push(FormRow::List {
                    id: DATE_FORMATS_LIST,
                    label: String::new(),
                    items: DATE_FORMATS
                        .iter()
                        .map(|format| format.to_string())
                        .collect(),
                    selected: DATE_FORMATS
                        .iter()
                        .position(|format| *format == self.date_format.text),
                    rows: 5,
                });
            }
            ContentControlKind::CheckBox => {
                rows.push(FormRow::Heading(t("cc.props.check_box").to_string()));
                rows.push(FormRow::Choice {
                    id: CHECKED_SYMBOL,
                    label: t("cc.props.checked_symbol").to_string(),
                    options: CHECKED_SYMBOLS.iter().map(char::to_string).collect(),
                    selected: CHECKED_SYMBOLS
                        .iter()
                        .position(|symbol| *symbol == properties.checked_symbol.symbol),
                });
                rows.push(FormRow::Choice {
                    id: UNCHECKED_SYMBOL,
                    label: t("cc.props.unchecked_symbol").to_string(),
                    options: UNCHECKED_SYMBOLS.iter().map(char::to_string).collect(),
                    selected: UNCHECKED_SYMBOLS
                        .iter()
                        .position(|symbol| *symbol == properties.unchecked_symbol.symbol),
                });
            }
            ContentControlKind::BuildingBlockGallery => {
                rows.push(FormRow::Heading(t("cc.props.gallery_heading").to_string()));
                rows.push(FormRow::Choice {
                    id: GALLERY,
                    label: t("cc.props.gallery").to_string(),
                    options: BUILDING_BLOCK_GALLERIES
                        .iter()
                        .map(|gallery| gallery.to_string())
                        .collect(),
                    selected: BUILDING_BLOCK_GALLERIES
                        .iter()
                        .position(|gallery| *gallery == properties.gallery),
                });
                rows.push(self.field_row(CATEGORY, "cc.props.category", &self.category, t));
            }
            ContentControlKind::RepeatingSection => {
                rows.push(FormRow::Heading(t("cc.props.repeating").to_string()));
                rows.push(self.field_row(
                    SECTION_TITLE,
                    "cc.props.section_title",
                    &self.section_title,
                    t,
                ));
                rows.push(FormRow::Check {
                    id: FIXED_ITEMS,
                    label: t("cc.props.allow_insert").to_string(),
                    on: !properties.fixed_items,
                });
            }
            _ => {}
        }
        FormView {
            title: t("cc.props.dialog").to_string(),
            width: 460.0,
            rows,
            buttons: vec![
                FormButton::new(REMOVE_CONTROL, t("cc.props.remove_control")),
                FormButton::new(CANCEL, t("format.cancel")),
                FormButton::new(OK, t("format.ok")).primary(),
            ],
            problem: None,
        }
    }

    fn commit_fields(&mut self) {
        self.properties.title = self.title.text.trim().to_string();
        self.properties.tag = self.tag.text.trim().to_string();
        self.properties.section_title = self.section_title.text.trim().to_string();
        self.properties.category = self.category.text.trim().to_string();
        let format = self.date_format.text.trim();
        if !format.is_empty() {
            self.properties.date_format = format.to_string();
        }
    }

    /// What a click or a key does.
    pub fn event(&mut self, event: FormEvent) -> FormOutcome<ContentControlAction> {
        match event {
            FormEvent::Click(id) if self.fields().contains(&id) => self.focus = id,
            FormEvent::Click(OK) | FormEvent::Enter => {
                self.commit_fields();
                return FormOutcome::Do(ContentControlAction::SetProperties {
                    control: self.control,
                    properties: self.properties.clone(),
                });
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            FormEvent::Click(REMOVE_CONTROL) => {
                return FormOutcome::Do(ContentControlAction::Remove {
                    control: self.control,
                })
            }
            FormEvent::Tab(backwards) => {
                let fields = self.fields();
                let at = fields.iter().position(|id| *id == self.focus).unwrap_or(0);
                let next = if backwards {
                    (at + fields.len() - 1) % fields.len()
                } else {
                    (at + 1) % fields.len()
                };
                self.focus = fields[next];
            }
            FormEvent::Choose(APPEARANCE, index) => {
                self.properties.appearance = match index {
                    1 => ControlAppearance::Tags,
                    2 => ControlAppearance::Hidden,
                    _ => ControlAppearance::BoundingBox,
                };
            }
            FormEvent::Choose(COLOR, index) => {
                self.properties.color = CONTROL_COLORS
                    .get(index)
                    .copied()
                    .flatten()
                    .map(str::to_string);
            }
            FormEvent::Choose(STYLE, index) => {
                self.properties.style = index
                    .checked_sub(1)
                    .and_then(|index| self.styles.get(index))
                    .map(|(id, _)| id.clone());
            }
            FormEvent::Toggle(TEMPORARY) => self.properties.temporary ^= true,
            FormEvent::Toggle(MULTI_LINE) => self.properties.multi_line ^= true,
            FormEvent::Toggle(NO_DELETE) => self.properties.cannot_delete ^= true,
            FormEvent::Toggle(NO_EDIT) => self.properties.cannot_edit ^= true,
            FormEvent::Toggle(FIXED_ITEMS) => self.properties.fixed_items ^= true,
            FormEvent::Choose(ENTRIES, index) => {
                self.selected_entry = Some(index);
                if let Some(entry) = self.properties.entries.get(index) {
                    self.entry_display = TextEdit::new(entry.display.clone());
                    self.entry_value = TextEdit::new(entry.value.clone());
                }
            }
            FormEvent::Click(ENTRY_ADD) => {
                let display = self.entry_display.text.trim().to_string();
                if !display.is_empty() {
                    let value = match self.entry_value.text.trim() {
                        "" => display.clone(),
                        value => value.to_string(),
                    };
                    self.properties.entries.push(ListEntry { display, value });
                    self.selected_entry = Some(self.properties.entries.len() - 1);
                    self.entry_display = TextEdit::new(String::new());
                    self.entry_value = TextEdit::new(String::new());
                    self.focus = ENTRY_DISPLAY;
                }
            }
            FormEvent::Click(ENTRY_CHANGE) => {
                if let Some(entry) = self
                    .selected_entry
                    .and_then(|index| self.properties.entries.get_mut(index))
                {
                    let display = self.entry_display.text.trim();
                    if !display.is_empty() {
                        entry.display = display.to_string();
                        entry.value = self.entry_value.text.trim().to_string();
                    }
                }
            }
            FormEvent::Click(ENTRY_REMOVE) => {
                if let Some(index) = self.selected_entry {
                    if index < self.properties.entries.len() {
                        self.properties.entries.remove(index);
                    }
                    self.selected_entry = None;
                }
            }
            FormEvent::Click(ENTRY_UP) | FormEvent::Click(ENTRY_DOWN) => {
                let up = event == FormEvent::Click(ENTRY_UP);
                if let Some(index) = self.selected_entry {
                    let entries = &mut self.properties.entries;
                    let target = if up {
                        index.checked_sub(1)
                    } else {
                        (index + 1 < entries.len()).then_some(index + 1)
                    };
                    if let Some(target) = target {
                        entries.swap(index, target);
                        self.selected_entry = Some(target);
                    }
                }
            }
            FormEvent::Choose(DATE_FORMATS_LIST, index) => {
                if let Some(format) = DATE_FORMATS.get(index) {
                    self.date_format = TextEdit::new(format.to_string());
                    self.properties.date_format = format.to_string();
                }
            }
            FormEvent::Choose(CHECKED_SYMBOL, index) => {
                if let Some(symbol) = CHECKED_SYMBOLS.get(index) {
                    self.properties.checked_symbol.symbol = *symbol;
                }
            }
            FormEvent::Choose(UNCHECKED_SYMBOL, index) => {
                if let Some(symbol) = UNCHECKED_SYMBOLS.get(index) {
                    self.properties.unchecked_symbol.symbol = *symbol;
                }
            }
            FormEvent::Choose(GALLERY, index) => {
                if let Some(gallery) = BUILDING_BLOCK_GALLERIES.get(index) {
                    self.properties.gallery = gallery.to_string();
                }
            }
            _ => {}
        }
        FormOutcome::Stay
    }
}

/// A legacy form field's オプション dialog.
pub struct FormFieldForm {
    pub field: usize,
    pub kind: FormFieldKind,
    pub options: FormFieldOptions,
    name: TextEdit,
    default_text: TextEdit,
    max_length: TextEdit,
    status_text: TextEdit,
    entry: TextEdit,
    focus: FormId,
    selected_entry: Option<usize>,
}

const NAME: FormId = FormId::new("name");
const DEFAULT_TEXT: FormId = FormId::new("default-text");
const MAX_LENGTH: FormId = FormId::new("max-length");
const STATUS_TEXT: FormId = FormId::new("status-text");
const ENTRY: FormId = FormId::new("entry");
const ENABLED: FormId = FormId::new("enabled");
const DEFAULT_CHECKED: FormId = FormId::new("default-checked");

impl FormFieldForm {
    pub fn new(field: &LegacyFormField) -> Self {
        let options = FormFieldOptions::of(field);
        Self {
            field: field.index,
            kind: field.kind,
            name: TextEdit::new(options.name.clone()),
            default_text: TextEdit::new(options.default_text.clone()),
            max_length: TextEdit::new(if options.max_length == 0 {
                String::new()
            } else {
                options.max_length.to_string()
            }),
            status_text: TextEdit::new(options.status_text.clone()),
            entry: TextEdit::new(String::new()),
            focus: NAME,
            selected_entry: None,
            options,
        }
    }

    fn fields(&self) -> Vec<FormId> {
        match self.kind {
            FormFieldKind::Text => vec![DEFAULT_TEXT, MAX_LENGTH, NAME, STATUS_TEXT],
            FormFieldKind::CheckBox => vec![NAME, STATUS_TEXT],
            FormFieldKind::DropDown => vec![ENTRY, NAME, STATUS_TEXT],
        }
    }

    pub fn field(&self) -> &TextEdit {
        match self.focus {
            DEFAULT_TEXT => &self.default_text,
            MAX_LENGTH => &self.max_length,
            STATUS_TEXT => &self.status_text,
            ENTRY => &self.entry,
            _ => &self.name,
        }
    }

    pub fn field_mut(&mut self) -> &mut TextEdit {
        match self.focus {
            DEFAULT_TEXT => &mut self.default_text,
            MAX_LENGTH => &mut self.max_length,
            STATUS_TEXT => &mut self.status_text,
            ENTRY => &mut self.entry,
            _ => &mut self.name,
        }
    }

    pub fn view(&self, t: Translate) -> FormView {
        let field = |id: FormId, label: &'static str, edit: &TextEdit| FormRow::Field {
            id,
            label: t(label).to_string(),
            value: FieldView::of(edit),
            focused: self.focus == id,
            width: 200.0,
        };
        let mut rows = Vec::new();
        let title = match self.kind {
            FormFieldKind::Text => {
                rows.push(field(DEFAULT_TEXT, "form.default_text", &self.default_text));
                rows.push(field(MAX_LENGTH, "form.max_length", &self.max_length));
                "form.text_options"
            }
            FormFieldKind::CheckBox => {
                rows.push(FormRow::Check {
                    id: DEFAULT_CHECKED,
                    label: t("form.default_checked").to_string(),
                    on: self.options.default_checked,
                });
                "form.check_options"
            }
            FormFieldKind::DropDown => {
                rows.push(field(ENTRY, "form.entry", &self.entry));
                rows.push(FormRow::List {
                    id: ENTRIES,
                    label: t("form.entries").to_string(),
                    items: self.options.entries.clone(),
                    selected: self.selected_entry,
                    rows: 5,
                });
                let selected = self.selected_entry.is_some();
                rows.push(FormRow::Buttons(vec![
                    FormButton::new(ENTRY_ADD, t("cc.props.add")),
                    FormButton::new(ENTRY_REMOVE, t("cc.props.remove")).enabled(selected),
                    FormButton::new(ENTRY_UP, t("cc.props.up")).enabled(selected),
                    FormButton::new(ENTRY_DOWN, t("cc.props.down")).enabled(selected),
                ]));
                "form.dropdown_options"
            }
        };
        rows.push(field(NAME, "form.bookmark", &self.name));
        rows.push(field(STATUS_TEXT, "form.help_text", &self.status_text));
        rows.push(FormRow::Check {
            id: ENABLED,
            label: t("form.enabled").to_string(),
            on: self.options.enabled,
        });
        FormView {
            title: t(title).to_string(),
            width: 420.0,
            rows,
            buttons: vec![
                FormButton::new(CANCEL, t("format.cancel")),
                FormButton::new(OK, t("format.ok")).primary(),
            ],
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<ContentControlAction> {
        match event {
            FormEvent::Click(id) if self.fields().contains(&id) => self.focus = id,
            FormEvent::Click(OK) | FormEvent::Enter => {
                self.options.name = self.name.text.trim().to_string();
                self.options.default_text = self.default_text.text.clone();
                self.options.max_length = self.max_length.text.trim().parse().unwrap_or(0);
                self.options.status_text = self.status_text.text.clone();
                return FormOutcome::Do(ContentControlAction::SetFormFieldOptions {
                    field: self.field,
                    options: self.options.clone(),
                });
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => return FormOutcome::Close,
            FormEvent::Tab(backwards) => {
                let fields = self.fields();
                let at = fields.iter().position(|id| *id == self.focus).unwrap_or(0);
                let next = if backwards {
                    (at + fields.len() - 1) % fields.len()
                } else {
                    (at + 1) % fields.len()
                };
                self.focus = fields[next];
            }
            FormEvent::Toggle(ENABLED) => self.options.enabled ^= true,
            FormEvent::Toggle(DEFAULT_CHECKED) => self.options.default_checked ^= true,
            FormEvent::Choose(ENTRIES, index) => self.selected_entry = Some(index),
            FormEvent::Click(ENTRY_ADD) => {
                let entry = self.entry.text.trim().to_string();
                if !entry.is_empty() {
                    self.options.entries.push(entry);
                    self.entry = TextEdit::new(String::new());
                }
            }
            FormEvent::Click(ENTRY_REMOVE) => {
                if let Some(index) = self.selected_entry.take() {
                    if index < self.options.entries.len() {
                        self.options.entries.remove(index);
                    }
                }
            }
            FormEvent::Click(ENTRY_UP) | FormEvent::Click(ENTRY_DOWN) => {
                let up = event == FormEvent::Click(ENTRY_UP);
                if let Some(index) = self.selected_entry {
                    let entries = &mut self.options.entries;
                    let target = if up {
                        index.checked_sub(1)
                    } else {
                        (index + 1 < entries.len()).then_some(index + 1)
                    };
                    if let Some(target) = target {
                        entries.swap(index, target);
                        self.selected_entry = Some(target);
                    }
                }
            }
            _ => {}
        }
        FormOutcome::Stay
    }
}

/// The document's character styles, id and name, for the properties
/// dialog's スタイル.
pub fn character_styles<S: WordSession + ?Sized>(session: &S) -> Vec<(String, String)> {
    let Some(styles) = session.word_document().styles_part.as_ref() else {
        return Vec::new();
    };
    styles
        .character_style_gallery_definitions()
        // 既定の段落フォント is the first choice already.
        .filter(|style| !style.is_default)
        .map(|style| {
            (
                style.style_id.clone(),
                style.name.clone().unwrap_or_else(|| style.style_id.clone()),
            )
        })
        .filter(|(_, name)| !name.eq_ignore_ascii_case("Placeholder Text"))
        .collect()
}

/// 選択範囲をギャラリーに保存: a name and a gallery for the selected
/// paragraphs, kept as a building block.
pub struct SaveBlockForm {
    pub name: TextEdit,
    pub gallery: usize,
}

impl Default for SaveBlockForm {
    fn default() -> Self {
        Self {
            name: TextEdit::new(String::new()),
            gallery: 0,
        }
    }
}

const BLOCK_NAME: FormId = FormId::new("block-name");
const BLOCK_GALLERY: FormId = FormId::new("block-gallery");

impl SaveBlockForm {
    pub fn view(&self, t: Translate) -> FormView {
        FormView {
            title: t("cc.save_block.title").to_string(),
            width: 420.0,
            rows: vec![
                FormRow::Field {
                    id: BLOCK_NAME,
                    label: t("cc.save_block.name").to_string(),
                    value: FieldView::of(&self.name),
                    focused: true,
                    width: 220.0,
                },
                FormRow::Choice {
                    id: BLOCK_GALLERY,
                    label: t("cc.props.gallery").to_string(),
                    options: BUILDING_BLOCK_GALLERIES
                        .iter()
                        .map(|gallery| gallery.to_string())
                        .collect(),
                    selected: Some(self.gallery),
                },
            ],
            buttons: vec![
                FormButton::new(CANCEL, t("format.cancel")),
                FormButton::new(OK, t("format.ok")).primary(),
            ],
            problem: None,
        }
    }

    pub fn event(&mut self, event: FormEvent) -> FormOutcome<ContentControlAction> {
        match event {
            FormEvent::Click(OK) | FormEvent::Enter if !self.name.text.trim().is_empty() => {
                FormOutcome::Do(ContentControlAction::SaveBuildingBlock {
                    name: self.name.text.trim().to_string(),
                    gallery: BUILDING_BLOCK_GALLERIES
                        [self.gallery.min(BUILDING_BLOCK_GALLERIES.len() - 1)]
                    .to_string(),
                })
            }
            FormEvent::Click(CANCEL) | FormEvent::Escape => FormOutcome::Close,
            FormEvent::Choose(BLOCK_GALLERY, index) => {
                self.gallery = index;
                FormOutcome::Stay
            }
            _ => FormOutcome::Stay,
        }
    }
}
