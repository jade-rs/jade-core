//! A dialog described once and drawn by both shells: its rows — fields,
//! choices, check boxes, lists, a grid — and its buttons, as data, with what
//! a click or a key on one of them does coming back as a [`FormEvent`].
//!
//! The mail-merge dialogs and the content control dialogs have many rows
//! and little layout of their own; describing them here keeps the native
//! app and the browser shell drawing the same dialog from the same state,
//! with only the drawing of each kind of row written per shell.

use crate::text_edit::TextEdit;

/// A control of a dialog: its name and, for one of a repeated set, which.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FormId(pub &'static str, pub usize);

impl FormId {
    pub const fn new(name: &'static str) -> Self {
        Self(name, 0)
    }

    pub const fn at(name: &'static str, index: usize) -> Self {
        Self(name, index)
    }
}

/// A one-line text field as it is drawn: its text, split at the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldView {
    pub before: String,
    pub after: String,
    /// All of it selected: what is typed replaces it.
    pub whole: bool,
}

impl FieldView {
    pub fn of(edit: &TextEdit) -> Self {
        let (before, after) = edit.split();
        Self {
            before: before.to_string(),
            after: after.to_string(),
            whole: edit.whole,
        }
    }

    pub fn text(&self) -> String {
        format!("{}{}", self.before, self.after)
    }
}

/// A row of a grid: its cells and, when the grid has them, its check box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridRow {
    pub cells: Vec<String>,
    pub checked: Option<bool>,
}

/// One row of a dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum FormRow {
    /// A section title.
    Heading(String),
    /// Text to read.
    Note(String),
    /// Text set in a box, line by line: a preview.
    Preview(Vec<String>),
    /// A labelled text field; `focused` while typing goes into it.
    Field {
        id: FormId,
        label: String,
        value: FieldView,
        focused: bool,
        /// Its width in points.
        width: f32,
    },
    /// A labelled choice of one of several, drawn as a row of chips.
    Choice {
        id: FormId,
        label: String,
        options: Vec<String>,
        selected: Option<usize>,
    },
    /// A labelled text field of several lines: Enter breaks a line in it.
    TextArea {
        id: FormId,
        label: String,
        value: FieldView,
        focused: bool,
        width: f32,
        lines: usize,
    },
    /// A check box and its label.
    Check { id: FormId, label: String, on: bool },
    /// A list to pick one entry from, `rows` tall.
    List {
        id: FormId,
        label: String,
        items: Vec<String>,
        selected: Option<usize>,
        rows: usize,
    },
    /// A table: its header (a click on one sorts by it), its rows (a check
    /// box each when `checked` is set), the cell selected and the one being
    /// typed into.
    Grid {
        id: FormId,
        headers: Vec<String>,
        rows: Vec<GridRow>,
        selected: Option<(usize, usize)>,
        editing: Option<(usize, usize, FieldView)>,
        /// The column sorted on and whether upwards.
        sorted: Option<(usize, bool)>,
    },
    /// Buttons in a line.
    Buttons(Vec<FormButton>),
    /// Rows side by side.
    Columns(Vec<Vec<FormRow>>),
}

/// A button of a dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormButton {
    pub id: FormId,
    pub label: String,
    /// Drawn as the dialog's default (Enter).
    pub primary: bool,
    pub enabled: bool,
}

impl FormButton {
    pub fn new(id: FormId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            primary: false,
            enabled: true,
        }
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// A dialog as it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct FormView {
    pub title: String,
    /// Its width in points.
    pub width: f32,
    pub rows: Vec<FormRow>,
    /// The buttons along its bottom: OK, Cancel and the like.
    pub buttons: Vec<FormButton>,
    /// Why the last OK did nothing.
    pub problem: Option<String>,
}

/// What the user did in a dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormEvent {
    /// A button, or a field or list clicked to give it the keyboard.
    Click(FormId),
    /// One of a choice's options, or a list's entry.
    Choose(FormId, usize),
    /// A check box.
    Toggle(FormId),
    /// A grid's cell (row, column).
    Cell(FormId, usize, usize),
    /// A grid's header: sort by that column.
    Header(FormId, usize),
    /// A grid row's check box.
    RowCheck(FormId, usize),
    /// Tab (or ⇧Tab): the next (previous) field.
    Tab(bool),
    /// Enter: the dialog's default.
    Enter,
    /// Escape.
    Escape,
}

/// What a dialog does after an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormOutcome<T> {
    /// It stays open.
    Stay,
    /// It closes and nothing happens.
    Close,
    /// It closes, or stays, and the shell does this.
    Do(T),
}

/// Pass the shell's translation of an i18n key.
pub type Translate = fn(&'static str) -> &'static str;

/// A group of a ribbon tab described once for both shells: its label and
/// its rows of buttons and menus. A button's click is
/// [`FormEvent::Click`]; a menu's button toggles it open, and an entry is
/// [`FormEvent::Choose`] with the menu's id.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolGroup {
    pub label: String,
    pub rows: Vec<Vec<ToolItem>>,
}

/// One control of a [`ToolGroup`].
#[derive(Clone, Debug, PartialEq)]
pub enum ToolItem {
    Button {
        id: FormId,
        label: String,
        title: String,
        on: bool,
        enabled: bool,
    },
    Menu {
        id: FormId,
        label: String,
        title: String,
        enabled: bool,
        open: bool,
        items: Vec<ToolMenuItem>,
    },
    /// Text shown in the row: a record number.
    Text(String),
}

/// An entry of a [`ToolItem::Menu`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolMenuItem {
    pub label: String,
    pub on: bool,
    pub enabled: bool,
}

impl ToolMenuItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            on: false,
            enabled: true,
        }
    }
}
