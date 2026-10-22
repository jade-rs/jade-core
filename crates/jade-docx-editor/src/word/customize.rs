//! リボンのユーザー設定 and the クイック アクセス ツール バー, as far as
//! both shells share them: the commands the toolbar and the user's own
//! ribbon group can hold, their names, labels and glyphs, and the
//! `key=value` text the choices are kept in between runs.

use std::collections::BTreeMap;

/// A command the toolbar or the user's group can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickCommand {
    Save,
    Undo,
    Redo,
    Print,
    ExportPdf,
    Find,
    AdvancedFind,
    Bold,
    Italic,
    Underline,
    Bullets,
    Numbering,
    AlignLeft,
    AlignCenter,
    AlignRight,
    Font,
    Paragraph,
    InsertTable,
    Picture,
    Link,
    Comment,
    TrackChanges,
    FormattingMarks,
    ZoomIn,
    ZoomOut,
    AutoCorrect,
    Overtype,
}

impl QuickCommand {
    pub const ALL: [QuickCommand; 27] = [
        QuickCommand::Save,
        QuickCommand::Undo,
        QuickCommand::Redo,
        QuickCommand::Print,
        QuickCommand::ExportPdf,
        QuickCommand::Find,
        QuickCommand::AdvancedFind,
        QuickCommand::Bold,
        QuickCommand::Italic,
        QuickCommand::Underline,
        QuickCommand::Bullets,
        QuickCommand::Numbering,
        QuickCommand::AlignLeft,
        QuickCommand::AlignCenter,
        QuickCommand::AlignRight,
        QuickCommand::Font,
        QuickCommand::Paragraph,
        QuickCommand::InsertTable,
        QuickCommand::Picture,
        QuickCommand::Link,
        QuickCommand::Comment,
        QuickCommand::TrackChanges,
        QuickCommand::FormattingMarks,
        QuickCommand::ZoomIn,
        QuickCommand::ZoomOut,
        QuickCommand::AutoCorrect,
        QuickCommand::Overtype,
    ];

    /// The name it is saved under.
    pub fn id(self) -> &'static str {
        match self {
            QuickCommand::Save => "save",
            QuickCommand::Undo => "undo",
            QuickCommand::Redo => "redo",
            QuickCommand::Print => "print",
            QuickCommand::ExportPdf => "export_pdf",
            QuickCommand::Find => "find",
            QuickCommand::AdvancedFind => "advanced_find",
            QuickCommand::Bold => "bold",
            QuickCommand::Italic => "italic",
            QuickCommand::Underline => "underline",
            QuickCommand::Bullets => "bullets",
            QuickCommand::Numbering => "numbering",
            QuickCommand::AlignLeft => "align_left",
            QuickCommand::AlignCenter => "align_center",
            QuickCommand::AlignRight => "align_right",
            QuickCommand::Font => "font",
            QuickCommand::Paragraph => "paragraph",
            QuickCommand::InsertTable => "insert_table",
            QuickCommand::Picture => "picture",
            QuickCommand::Link => "link",
            QuickCommand::Comment => "comment",
            QuickCommand::TrackChanges => "track_changes",
            QuickCommand::FormattingMarks => "formatting_marks",
            QuickCommand::ZoomIn => "zoom_in",
            QuickCommand::ZoomOut => "zoom_out",
            QuickCommand::AutoCorrect => "autocorrect",
            QuickCommand::Overtype => "overtype",
        }
    }

    pub fn from_id(id: &str) -> Option<QuickCommand> {
        QuickCommand::ALL
            .into_iter()
            .find(|command| command.id() == id)
    }

    /// Its label, an i18n key.
    pub fn label_key(self) -> &'static str {
        match self {
            QuickCommand::Save => "qat.save",
            QuickCommand::Undo => "qat.undo",
            QuickCommand::Redo => "qat.redo",
            QuickCommand::Print => "qat.print",
            QuickCommand::ExportPdf => "qat.export_pdf",
            QuickCommand::Find => "qat.find",
            QuickCommand::AdvancedFind => "find.menu",
            QuickCommand::Bold => "qat.bold",
            QuickCommand::Italic => "qat.italic",
            QuickCommand::Underline => "qat.underline",
            QuickCommand::Bullets => "context.bullets",
            QuickCommand::Numbering => "context.numbering",
            QuickCommand::AlignLeft => "qat.align_left",
            QuickCommand::AlignCenter => "qat.align_center",
            QuickCommand::AlignRight => "qat.align_right",
            QuickCommand::Font => "context.font",
            QuickCommand::Paragraph => "context.paragraph",
            QuickCommand::InsertTable => "qat.insert_table",
            QuickCommand::Picture => "qat.picture",
            QuickCommand::Link => "context.link",
            QuickCommand::Comment => "context.new_comment",
            QuickCommand::TrackChanges => "qat.track_changes",
            QuickCommand::FormattingMarks => "doc.marks_title",
            QuickCommand::ZoomIn => "doc.zoom_in",
            QuickCommand::ZoomOut => "doc.zoom_out",
            QuickCommand::AutoCorrect => "menu.tools.autocorrect",
            QuickCommand::Overtype => "options.overtype",
        }
    }

    /// The icon the toolbar shows for it, or a short text.
    pub fn glyph(self) -> (Option<&'static str>, &'static str) {
        match self {
            QuickCommand::Save => (Some("icons/file-text.svg"), ""),
            QuickCommand::Undo => (Some("icons/undo.svg"), ""),
            QuickCommand::Redo => (Some("icons/redo.svg"), ""),
            QuickCommand::Print => (Some("icons/printer.svg"), ""),
            QuickCommand::ExportPdf => (None, "PDF"),
            QuickCommand::Find => (Some("icons/filter.svg"), ""),
            QuickCommand::AdvancedFind => (None, "Aa…"),
            QuickCommand::Bold => (Some("icons/bold.svg"), ""),
            QuickCommand::Italic => (Some("icons/italic.svg"), ""),
            QuickCommand::Underline => (Some("icons/underline.svg"), ""),
            QuickCommand::Bullets => (Some("icons/list.svg"), ""),
            QuickCommand::Numbering => (Some("icons/list-ordered.svg"), ""),
            QuickCommand::AlignLeft => (Some("icons/align-left.svg"), ""),
            QuickCommand::AlignCenter => (Some("icons/align-center.svg"), ""),
            QuickCommand::AlignRight => (Some("icons/align-right.svg"), ""),
            QuickCommand::Font => (Some("icons/type.svg"), ""),
            QuickCommand::Paragraph => (None, "¶…"),
            QuickCommand::InsertTable => (Some("icons/table.svg"), ""),
            QuickCommand::Picture => (Some("icons/image.svg"), ""),
            QuickCommand::Link => (None, "Link"),
            QuickCommand::Comment => (None, "+C"),
            QuickCommand::TrackChanges => (None, "Rev"),
            QuickCommand::FormattingMarks => (None, "¶"),
            QuickCommand::ZoomIn => (None, "+"),
            QuickCommand::ZoomOut => (None, "-"),
            QuickCommand::AutoCorrect => (None, "ab✓"),
            QuickCommand::Overtype => (None, "Ins"),
        }
    }
}

/// The toolbar's commands as Word starts with them: 保存, 元に戻す, やり直し.
pub const DEFAULT_TOOLBAR: [QuickCommand; 3] =
    [QuickCommand::Save, QuickCommand::Undo, QuickCommand::Redo];

/// A saved list of commands (`save,undo,bold`); unknown names are skipped.
pub fn parse_commands(text: &str) -> Vec<QuickCommand> {
    text.split(',')
        .filter_map(|id| QuickCommand::from_id(id.trim()))
        .collect()
}

/// A list of commands as it is saved.
pub fn format_commands(commands: &[QuickCommand]) -> String {
    commands
        .iter()
        .map(|command| command.id())
        .collect::<Vec<_>>()
        .join(",")
}

/// A `key=value` settings text, one value a line.
pub fn parse_keyed(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// `key=value` lines, in key order.
pub fn format_keyed(values: &BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{key}={value}\n"))
        .collect()
}
