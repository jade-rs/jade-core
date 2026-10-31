//! 挿入 › フィールド and the field commands over any [`WordSession`]:
//! the catalogue Word's Field dialog lists — every field by category, with
//! its options and switches — building and reading field codes, and the
//! session operations behind F9 (update), ⌥F9/⇧F9 (codes), ⌘F11/⌘⇧F11
//! (lock), ⌘⇧F9 (unlink), the prompts ASK and FILLIN show, and the
//! document's properties.
//!
//! Labels are i18n keys (`field.*`) the shells translate, so the catalogue
//! is the same in both.

use std::collections::HashMap;
use std::path::PathBuf;

pub use crate::field_datetime::{
    datetime_from_unix, format_word_date, parse_core_property_datetime, unix_seconds, FieldDateTime,
};
pub use crate::fields::FieldScope;
use crate::fields::{self, FieldCommand, FieldEnvironment};
pub use crate::fields::{
    CustomProperty, CustomValue, DocumentProperties, FieldCodeView, FieldInfo, FieldPrompt,
};
pub use jade_selection::{SelectionRange, TextPosition};

use super::WordSession;

/// The Field dialog's categories, in Word for Mac's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldCategory {
    DateTime,
    DocumentAutomation,
    DocumentInformation,
    EquationsFormulas,
    IndexTables,
    LinksReferences,
    MailMerge,
    Numbering,
    UserInformation,
}

impl FieldCategory {
    pub const ALL: [FieldCategory; 9] = [
        FieldCategory::DateTime,
        FieldCategory::DocumentAutomation,
        FieldCategory::DocumentInformation,
        FieldCategory::EquationsFormulas,
        FieldCategory::IndexTables,
        FieldCategory::LinksReferences,
        FieldCategory::MailMerge,
        FieldCategory::Numbering,
        FieldCategory::UserInformation,
    ];

    /// The i18n key of the category's name.
    pub fn key(self) -> &'static str {
        match self {
            FieldCategory::DateTime => "field.category.date_time",
            FieldCategory::DocumentAutomation => "field.category.automation",
            FieldCategory::DocumentInformation => "field.category.information",
            FieldCategory::EquationsFormulas => "field.category.formulas",
            FieldCategory::IndexTables => "field.category.index",
            FieldCategory::LinksReferences => "field.category.links",
            FieldCategory::MailMerge => "field.category.mail_merge",
            FieldCategory::Numbering => "field.category.numbering",
            FieldCategory::UserInformation => "field.category.user",
        }
    }
}

/// What a switch takes after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchArgument {
    /// Nothing: the switch is on or off.
    None,
    /// Free text (quoted when it has spaces).
    Text,
    /// A number.
    Number,
    /// One of these (pictures, formats), or free text.
    Choice(&'static [&'static str]),
}

/// One switch of a field, as the Options list shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchSpec {
    /// As written, with the backslash: `\p`, `\@`, `\*`.
    pub switch: &'static str,
    /// The i18n key of what it does.
    pub key: &'static str,
    pub argument: SwitchArgument,
}

/// One operand a field takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperandSpec {
    /// The i18n key of what it is.
    pub key: &'static str,
    /// Written in quotes.
    pub quoted: bool,
    /// Where the dialog offers values from.
    pub source: OperandSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandSource {
    Free,
    Bookmarks,
    Styles,
    Properties,
    Variables,
    Choice(&'static [&'static str]),
}

/// A field the dialog offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: &'static str,
    pub category: FieldCategory,
    /// The i18n key of its description.
    pub key: &'static str,
    pub operands: &'static [OperandSpec],
    pub switches: &'static [SwitchSpec],
}

/// Word's date and time pictures, Japanese ones first.
pub const DATE_PICTURES: &[&str] = &[
    "yyyy/MM/dd",
    "yyyy年M月d日",
    "yyyy年M月d日(aaa)",
    "yyyy年MM月dd日",
    "ggge年M月d日",
    "ggge年M月d日(aaa)",
    "gggee年MM月dd日",
    "yyyy/M/d",
    "M/d/yyyy",
    "dddd, MMMM d, yyyy",
    "MMMM d, yyyy",
    "d-MMM-yy",
    "yyyy-MM-dd",
    "yyyy/MM/dd H:mm",
    "yyyy年M月d日H時m分",
    "H:mm",
    "H:mm:ss",
    "h:mm AM/PM",
    "H時m分",
];

/// Word's numeric pictures.
pub const NUMBER_PICTURES: &[&str] = &[
    "0",
    "#,##0",
    "#,##0.00",
    "¥#,##0;(¥#,##0)",
    "$#,##0.00;($#,##0.00)",
    "0%",
    "0.00%",
    "0.00",
];

/// Word's `\*` formats.
pub const TEXT_FORMATS: &[&str] = &[
    "Upper",
    "Lower",
    "FirstCap",
    "Caps",
    "Arabic",
    "ROMAN",
    "roman",
    "ALPHABETIC",
    "alphabetic",
    "CardText",
    "OrdText",
    "Ordinal",
    "Hex",
    "DollarText",
    "ArabicDash",
    "CIRCLENUM",
    "KANJINUM1",
    "KANJINUM2",
    "KANJINUM3",
    "DBNUM1",
    "DBNUM2",
    "DBNUM3",
    "DBNUM4",
    "AIUEO",
    "IROHA",
    "ZODIAC1",
    "ZODIAC2",
    "DBCHAR",
    "SBCHAR",
];

const fn switch(switch: &'static str, key: &'static str, argument: SwitchArgument) -> SwitchSpec {
    SwitchSpec {
        switch,
        key,
        argument,
    }
}

const fn operand(key: &'static str, quoted: bool, source: OperandSource) -> OperandSpec {
    OperandSpec {
        key,
        quoted,
        source,
    }
}

const DATE: SwitchSpec = switch(
    "\\@",
    "field.switch.date_picture",
    SwitchArgument::Choice(DATE_PICTURES),
);
const NUMBER: SwitchSpec = switch(
    "\\#",
    "field.switch.number_picture",
    SwitchArgument::Choice(NUMBER_PICTURES),
);
const FORMAT: SwitchSpec = switch(
    "\\*",
    "field.switch.format",
    SwitchArgument::Choice(TEXT_FORMATS),
);
const TEXT_SWITCHES: &[SwitchSpec] = &[FORMAT];
const NUMBER_SWITCHES: &[SwitchSpec] = &[FORMAT, NUMBER];
const DATE_SWITCHES: &[SwitchSpec] = &[DATE, FORMAT];
const NO_OPERANDS: &[OperandSpec] = &[];
const NEW_VALUE: &[OperandSpec] = &[operand(
    "field.operand.new_value",
    true,
    OperandSource::Free,
)];

macro_rules! field {
    ($name:literal, $category:ident, $operands:expr, $switches:expr) => {
        FieldSpec {
            name: $name,
            category: FieldCategory::$category,
            key: concat!("field.describe.", $name),
            operands: $operands,
            switches: $switches,
        }
    };
}

/// Every field the dialog offers, alphabetically, as Word lists them.
pub const FIELDS: &[FieldSpec] = &[
    field!(
        "=",
        EquationsFormulas,
        &[operand("field.operand.formula", false, OperandSource::Free)],
        NUMBER_SWITCHES
    ),
    field!(
        "ADVANCE",
        EquationsFormulas,
        NO_OPERANDS,
        &[
            switch("\\d", "field.switch.advance_down", SwitchArgument::Number),
            switch("\\u", "field.switch.advance_up", SwitchArgument::Number),
            switch("\\l", "field.switch.advance_left", SwitchArgument::Number),
            switch("\\r", "field.switch.advance_right", SwitchArgument::Number),
            switch("\\x", "field.switch.advance_x", SwitchArgument::Number),
            switch("\\y", "field.switch.advance_y", SwitchArgument::Number),
        ]
    ),
    field!(
        "ASK",
        MailMerge,
        &[
            operand("field.operand.bookmark", false, OperandSource::Bookmarks),
            operand("field.operand.prompt", true, OperandSource::Free),
        ],
        &[
            switch("\\d", "field.switch.default_response", SwitchArgument::Text),
            switch("\\o", "field.switch.prompt_once", SwitchArgument::None),
        ]
    ),
    field!("AUTHOR", DocumentInformation, NEW_VALUE, TEXT_SWITCHES),
    field!(
        "AUTONUM",
        Numbering,
        NO_OPERANDS,
        &[switch(
            "\\s",
            "field.switch.separator",
            SwitchArgument::Text
        ),]
    ),
    field!(
        "AUTONUMLGL",
        Numbering,
        NO_OPERANDS,
        &[
            switch("\\e", "field.switch.no_period", SwitchArgument::None),
            switch("\\s", "field.switch.separator", SwitchArgument::Text),
        ]
    ),
    field!("AUTONUMOUT", Numbering, NO_OPERANDS, NO_SWITCHES),
    field!(
        "AUTOTEXT",
        LinksReferences,
        &[operand(
            "field.operand.autotext",
            false,
            OperandSource::Free
        )],
        TEXT_SWITCHES
    ),
    field!(
        "AUTOTEXTLIST",
        LinksReferences,
        &[operand("field.operand.text", true, OperandSource::Free)],
        &[
            switch("\\s", "field.switch.style", SwitchArgument::Text),
            switch("\\t", "field.switch.tip", SwitchArgument::Text),
        ]
    ),
    field!(
        "BIBLIOGRAPHY",
        LinksReferences,
        NO_OPERANDS,
        &[switch(
            "\\l",
            "field.switch.language",
            SwitchArgument::Number
        ),]
    ),
    field!(
        "CITATION",
        LinksReferences,
        &[operand(
            "field.operand.source_tag",
            false,
            OperandSource::Free
        )],
        &[
            switch("\\p", "field.switch.citation_page", SwitchArgument::Text),
            switch("\\f", "field.switch.prefix", SwitchArgument::Text),
            switch("\\s", "field.switch.suffix", SwitchArgument::Text),
            switch("\\n", "field.switch.no_author", SwitchArgument::None),
            switch("\\y", "field.switch.no_year", SwitchArgument::None),
        ]
    ),
    field!("COMMENTS", DocumentInformation, NEW_VALUE, TEXT_SWITCHES),
    field!(
        "COMPARE",
        DocumentAutomation,
        &[operand(
            "field.operand.comparison",
            false,
            OperandSource::Free
        )],
        NO_SWITCHES
    ),
    field!("CREATEDATE", DateTime, NO_OPERANDS, DATE_SWITCHES),
    field!("DATE", DateTime, NO_OPERANDS, DATE_SWITCHES),
    field!(
        "DOCPROPERTY",
        DocumentInformation,
        &[operand(
            "field.operand.property",
            false,
            OperandSource::Properties
        )],
        NUMBER_SWITCHES
    ),
    field!(
        "DOCVARIABLE",
        DocumentAutomation,
        &[operand(
            "field.operand.variable",
            false,
            OperandSource::Variables
        )],
        TEXT_SWITCHES
    ),
    field!(
        "EQ",
        EquationsFormulas,
        &[operand(
            "field.operand.equation",
            false,
            OperandSource::Free
        )],
        NO_SWITCHES
    ),
    field!(
        "FILENAME",
        DocumentInformation,
        NO_OPERANDS,
        &[
            switch("\\p", "field.switch.full_path", SwitchArgument::None),
            FORMAT,
        ]
    ),
    field!(
        "FILESIZE",
        DocumentInformation,
        NO_OPERANDS,
        &[
            switch("\\k", "field.switch.kilobytes", SwitchArgument::None),
            switch("\\m", "field.switch.megabytes", SwitchArgument::None),
            FORMAT,
            NUMBER,
        ]
    ),
    field!(
        "FILLIN",
        MailMerge,
        &[operand("field.operand.prompt", true, OperandSource::Free)],
        &[
            switch("\\d", "field.switch.default_response", SwitchArgument::Text),
            switch("\\o", "field.switch.prompt_once", SwitchArgument::None),
        ]
    ),
    field!(
        "GOTOBUTTON",
        DocumentAutomation,
        &[
            operand("field.operand.destination", false, OperandSource::Bookmarks),
            operand("field.operand.display_text", false, OperandSource::Free),
        ],
        NO_SWITCHES
    ),
    field!(
        "HYPERLINK",
        LinksReferences,
        &[operand("field.operand.address", true, OperandSource::Free)],
        &[
            switch("\\l", "field.switch.location", SwitchArgument::Text),
            switch("\\o", "field.switch.tooltip", SwitchArgument::Text),
            switch("\\t", "field.switch.target", SwitchArgument::Text),
            switch("\\n", "field.switch.new_window", SwitchArgument::None),
        ]
    ),
    field!(
        "IF",
        DocumentAutomation,
        &[operand(
            "field.operand.condition",
            false,
            OperandSource::Free
        )],
        NO_SWITCHES
    ),
    field!(
        "INCLUDEPICTURE",
        LinksReferences,
        &[operand("field.operand.file", true, OperandSource::Free)],
        &[switch("\\d", "field.switch.no_store", SwitchArgument::None),]
    ),
    field!(
        "INCLUDETEXT",
        LinksReferences,
        &[
            operand("field.operand.file", true, OperandSource::Free),
            operand("field.operand.bookmark", false, OperandSource::Free),
        ],
        &[
            switch("\\!", "field.switch.no_update_inside", SwitchArgument::None),
            FORMAT,
        ]
    ),
    field!(
        "INDEX",
        IndexTables,
        NO_OPERANDS,
        &[
            switch("\\e", "field.switch.entry_separator", SwitchArgument::Text),
            switch("\\l", "field.switch.page_separator", SwitchArgument::Text),
            switch("\\f", "field.switch.entry_type", SwitchArgument::Text),
            switch("\\c", "field.switch.columns", SwitchArgument::Number),
        ]
    ),
    field!(
        "INFO",
        DocumentInformation,
        &[operand(
            "field.operand.info_type",
            false,
            OperandSource::Choice(&[
                "Author",
                "Comments",
                "CreateDate",
                "EditTime",
                "FileName",
                "FileSize",
                "Keywords",
                "LastSavedBy",
                "NumChars",
                "NumPages",
                "NumWords",
                "PrintDate",
                "RevNum",
                "SaveDate",
                "Subject",
                "Template",
                "Title",
            ])
        )],
        TEXT_SWITCHES
    ),
    field!("KEYWORDS", DocumentInformation, NEW_VALUE, TEXT_SWITCHES),
    field!(
        "LASTSAVEDBY",
        DocumentInformation,
        NO_OPERANDS,
        TEXT_SWITCHES
    ),
    field!(
        "LISTNUM",
        Numbering,
        &[operand(
            "field.operand.list_name",
            false,
            OperandSource::Choice(&["NumberDefault", "LegalDefault", "OutlineDefault",])
        )],
        &[
            switch("\\l", "field.switch.level", SwitchArgument::Number),
            switch("\\s", "field.switch.start_at", SwitchArgument::Number),
        ]
    ),
    field!(
        "MACROBUTTON",
        DocumentAutomation,
        &[
            operand("field.operand.macro", false, OperandSource::Free),
            operand("field.operand.display_text", false, OperandSource::Free),
        ],
        NO_SWITCHES
    ),
    field!(
        "MERGEFIELD",
        MailMerge,
        &[operand(
            "field.operand.merge_field",
            false,
            OperandSource::Free
        )],
        &[
            switch("\\b", "field.switch.text_before", SwitchArgument::Text),
            switch("\\f", "field.switch.text_after", SwitchArgument::Text),
            FORMAT,
        ]
    ),
    field!(
        "NOTEREF",
        LinksReferences,
        &[operand(
            "field.operand.bookmark",
            false,
            OperandSource::Bookmarks
        )],
        &[
            switch("\\f", "field.switch.note_format", SwitchArgument::None),
            switch("\\h", "field.switch.hyperlink", SwitchArgument::None),
            switch("\\p", "field.switch.relative", SwitchArgument::None),
        ]
    ),
    field!(
        "NUMCHARS",
        DocumentInformation,
        NO_OPERANDS,
        NUMBER_SWITCHES
    ),
    field!(
        "NUMPAGES",
        DocumentInformation,
        NO_OPERANDS,
        NUMBER_SWITCHES
    ),
    field!(
        "NUMWORDS",
        DocumentInformation,
        NO_OPERANDS,
        NUMBER_SWITCHES
    ),
    field!("PAGE", Numbering, NO_OPERANDS, NUMBER_SWITCHES),
    field!(
        "PAGEREF",
        LinksReferences,
        &[operand(
            "field.operand.bookmark",
            false,
            OperandSource::Bookmarks
        )],
        &[
            switch("\\h", "field.switch.hyperlink", SwitchArgument::None),
            switch("\\p", "field.switch.relative", SwitchArgument::None),
            FORMAT,
        ]
    ),
    field!("PRINTDATE", DateTime, NO_OPERANDS, DATE_SWITCHES),
    field!(
        "QUOTE",
        DocumentAutomation,
        &[operand("field.operand.text", true, OperandSource::Free)],
        TEXT_SWITCHES
    ),
    field!(
        "REF",
        LinksReferences,
        &[operand(
            "field.operand.bookmark",
            false,
            OperandSource::Bookmarks
        )],
        &[
            switch("\\f", "field.switch.note_format", SwitchArgument::None),
            switch("\\h", "field.switch.hyperlink", SwitchArgument::None),
            switch("\\n", "field.switch.paragraph_number", SwitchArgument::None),
            switch("\\p", "field.switch.relative", SwitchArgument::None),
            switch("\\r", "field.switch.relative_number", SwitchArgument::None),
            switch("\\w", "field.switch.full_number", SwitchArgument::None),
            FORMAT,
        ]
    ),
    field!("REVNUM", DocumentInformation, NO_OPERANDS, NUMBER_SWITCHES),
    field!("SAVEDATE", DateTime, NO_OPERANDS, DATE_SWITCHES),
    field!("SECTION", Numbering, NO_OPERANDS, NUMBER_SWITCHES),
    field!("SECTIONPAGES", Numbering, NO_OPERANDS, NUMBER_SWITCHES),
    field!(
        "SEQ",
        Numbering,
        &[operand(
            "field.operand.sequence",
            false,
            OperandSource::Choice(&["Figure", "Table", "Equation", "図", "表", "数式"])
        )],
        &[
            switch("\\c", "field.switch.repeat", SwitchArgument::None),
            switch("\\h", "field.switch.hide", SwitchArgument::None),
            switch("\\n", "field.switch.next", SwitchArgument::None),
            switch("\\r", "field.switch.reset_to", SwitchArgument::Number),
            switch(
                "\\s",
                "field.switch.reset_at_heading",
                SwitchArgument::Number
            ),
            FORMAT,
        ]
    ),
    field!(
        "SET",
        MailMerge,
        &[
            operand("field.operand.bookmark", false, OperandSource::Bookmarks),
            operand("field.operand.text", true, OperandSource::Free),
        ],
        NO_SWITCHES
    ),
    field!(
        "STYLEREF",
        LinksReferences,
        &[operand("field.operand.style", true, OperandSource::Styles)],
        &[
            switch("\\l", "field.switch.from_bottom", SwitchArgument::None),
            switch("\\n", "field.switch.paragraph_number", SwitchArgument::None),
            switch("\\p", "field.switch.relative", SwitchArgument::None),
            switch("\\r", "field.switch.relative_number", SwitchArgument::None),
            switch("\\w", "field.switch.full_number", SwitchArgument::None),
            FORMAT,
        ]
    ),
    field!("SUBJECT", DocumentInformation, NEW_VALUE, TEXT_SWITCHES),
    field!(
        "SYMBOL",
        EquationsFormulas,
        &[operand(
            "field.operand.character_code",
            false,
            OperandSource::Free
        )],
        &[
            switch("\\f", "field.switch.font", SwitchArgument::Text),
            switch("\\s", "field.switch.size", SwitchArgument::Number),
            switch("\\u", "field.switch.unicode", SwitchArgument::None),
            switch("\\j", "field.switch.shift_jis", SwitchArgument::None),
            switch("\\h", "field.switch.keep_line_height", SwitchArgument::None),
        ]
    ),
    field!(
        "TA",
        IndexTables,
        NO_OPERANDS,
        &[
            switch("\\l", "field.switch.long_citation", SwitchArgument::Text),
            switch("\\s", "field.switch.short_citation", SwitchArgument::Text),
            switch("\\c", "field.switch.category", SwitchArgument::Number),
            switch("\\b", "field.switch.bold", SwitchArgument::None),
            switch("\\i", "field.switch.italic", SwitchArgument::None),
        ]
    ),
    field!(
        "TC",
        IndexTables,
        &[operand("field.operand.text", true, OperandSource::Free)],
        &[
            switch("\\f", "field.switch.entry_type", SwitchArgument::Text),
            switch("\\l", "field.switch.level", SwitchArgument::Number),
            switch("\\n", "field.switch.no_page", SwitchArgument::None),
        ]
    ),
    field!(
        "TEMPLATE",
        DocumentInformation,
        NO_OPERANDS,
        &[
            switch("\\p", "field.switch.full_path", SwitchArgument::None),
            FORMAT,
        ]
    ),
    field!("TIME", DateTime, NO_OPERANDS, DATE_SWITCHES),
    field!("TITLE", DocumentInformation, NEW_VALUE, TEXT_SWITCHES),
    field!(
        "TOA",
        IndexTables,
        NO_OPERANDS,
        &[
            switch("\\c", "field.switch.category", SwitchArgument::Number),
            switch("\\e", "field.switch.entry_separator", SwitchArgument::Text),
            switch("\\p", "field.switch.passim", SwitchArgument::None),
            switch("\\h", "field.switch.category_heading", SwitchArgument::None),
        ]
    ),
    field!(
        "TOC",
        IndexTables,
        NO_OPERANDS,
        &[
            switch("\\o", "field.switch.outline_levels", SwitchArgument::Text),
            switch("\\h", "field.switch.hyperlinks", SwitchArgument::None),
            switch("\\z", "field.switch.hide_web_pages", SwitchArgument::None),
            switch(
                "\\u",
                "field.switch.outline_paragraphs",
                SwitchArgument::None
            ),
            switch("\\t", "field.switch.styles", SwitchArgument::Text),
            switch("\\n", "field.switch.no_page_numbers", SwitchArgument::Text),
            switch("\\c", "field.switch.sequence", SwitchArgument::Text),
        ]
    ),
    field!("USERADDRESS", UserInformation, NEW_VALUE, TEXT_SWITCHES),
    field!("USERINITIALS", UserInformation, NEW_VALUE, TEXT_SWITCHES),
    field!("USERNAME", UserInformation, NEW_VALUE, TEXT_SWITCHES),
    field!(
        "XE",
        IndexTables,
        &[operand("field.operand.text", true, OperandSource::Free)],
        &[
            switch("\\b", "field.switch.bold", SwitchArgument::None),
            switch("\\i", "field.switch.italic", SwitchArgument::None),
            switch("\\f", "field.switch.entry_type", SwitchArgument::Text),
            switch("\\r", "field.switch.page_range", SwitchArgument::Text),
            switch("\\t", "field.switch.cross_reference", SwitchArgument::Text),
            switch("\\y", "field.switch.reading", SwitchArgument::Text),
        ]
    ),
];

const NO_SWITCHES: &[SwitchSpec] = &[];

/// The field `name` names, if the dialog offers it.
pub fn field_spec(name: &str) -> Option<&'static FieldSpec> {
    FIELDS
        .iter()
        .find(|spec| spec.name.eq_ignore_ascii_case(name))
}

/// The fields a category lists, or every field for `None` ((すべて)).
pub fn fields_in(category: Option<FieldCategory>) -> Vec<&'static FieldSpec> {
    FIELDS
        .iter()
        .filter(|spec| category.is_none_or(|category| spec.category == category))
        .collect()
}

/// An operand as a field code writes it: quoted when asked, or when it has
/// spaces; quotes inside escaped.
fn quote(value: &str, quoted: bool) -> String {
    if quoted || value.chars().any(char::is_whitespace) || value.is_empty() {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// Builds a field code: the name, its operands, its switches and, with
/// `preserve_format`, `\* MERGEFORMAT` (フィールド更新時に書式を維持する).
pub fn build_code(
    name: &str,
    operands: &[(String, bool)],
    switches: &[(String, Option<String>)],
    preserve_format: bool,
) -> String {
    let mut code = name.to_ascii_uppercase();
    for (value, quoted) in operands {
        if value.is_empty() && !quoted {
            continue;
        }
        code.push(' ');
        if name == "=" {
            code.push_str(value);
        } else {
            code.push_str(&quote(value, *quoted));
        }
    }
    for (switch, value) in switches {
        code.push(' ');
        code.push_str(switch);
        if let Some(value) = value {
            code.push(' ');
            let picture = matches!(switch.as_str(), "\\@" | "\\#");
            code.push_str(&quote(value, picture));
        }
    }
    if preserve_format && !code.contains("MERGEFORMAT") {
        code.push_str(" \\* MERGEFORMAT");
    }
    code
}

/// A code's field name (`DATE`, `=`), for selecting it in the dialog.
pub fn code_field_name(code: &str) -> Option<String> {
    let code = code.trim().trim_start_matches('{').trim();
    if code.starts_with('=') {
        return Some("=".to_string());
    }
    code.split_whitespace()
        .next()
        .map(|name| name.to_ascii_uppercase())
}

/// A code's tokens: words, quoted strings and intact nested instructions.
fn code_tokens(code: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut depth = 0usize;
    let mut chars = code.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\\' if in_quotes && chars.peek() == Some(&'"') => {
                current.push(character);
                current.push(chars.next().expect("peeked"));
            }
            '"' => {
                current.push(character);
                in_quotes = !in_quotes;
            }
            '{' if !in_quotes => {
                depth += 1;
                current.push(character);
            }
            '}' if !in_quotes => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            c if c.is_whitespace() && !in_quotes && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Whether switch `switch` of field `name` takes a value.
fn takes_value(name: &str, switch: &str) -> bool {
    if matches!(switch, "\\*" | "\\#" | "\\@") {
        return true;
    }
    field_spec(name)
        .and_then(|spec| {
            spec.switches
                .iter()
                .find(|spec| spec.switch.eq_ignore_ascii_case(switch))
        })
        .is_some_and(|spec| spec.argument != SwitchArgument::None)
}

/// A code split into its name, operands and switches (each with its value).
struct ParsedCode {
    name: String,
    operands: Vec<String>,
    switches: Vec<(String, Option<String>)>,
}

fn parse_code(code: &str) -> ParsedCode {
    let tokens = code_tokens(crate::fields::unwrapped_code(code).unwrap_or_else(|_| code.trim()));
    let mut tokens = tokens.into_iter();
    let name = tokens.next().unwrap_or_default();
    let upper = name.to_ascii_uppercase();
    let mut parsed = ParsedCode {
        name,
        operands: Vec::new(),
        switches: Vec::new(),
    };
    let mut pending: Option<String> = None;
    for token in tokens {
        if let Some(switch) = pending.take() {
            parsed.switches.push((switch, Some(token)));
            continue;
        }
        if token.starts_with('\\') && !token.starts_with('"') {
            if takes_value(&upper, &token) {
                pending = Some(token);
            } else {
                parsed.switches.push((token, None));
            }
        } else {
            parsed.operands.push(token);
        }
    }
    if let Some(switch) = pending {
        parsed.switches.push((switch, None));
    }
    parsed
}

fn unparse(parsed: &ParsedCode) -> String {
    let mut parts = vec![parsed.name.clone()];
    parts.extend(parsed.operands.iter().cloned());
    // MERGEFORMAT stays last, as Word writes it.
    let (merge, others): (Vec<_>, Vec<_>) = parsed.switches.iter().partition(|(switch, value)| {
        switch == "\\*"
            && value
                .as_deref()
                .is_some_and(|v| v.eq_ignore_ascii_case("MERGEFORMAT"))
    });
    for (switch, value) in others.into_iter().chain(merge) {
        parts.push(switch.clone());
        if let Some(value) = value {
            parts.push(value.clone());
        }
    }
    parts.join(" ")
}

fn unquote(token: &str) -> String {
    token
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(token)
        .replace("\\\"", "\"")
}

/// Whether `code` has switch `switch` (with `value`, when given).
pub fn code_has_switch(code: &str, switch: &str, value: Option<&str>) -> bool {
    parse_code(code)
        .switches
        .iter()
        .any(|(existing, existing_value)| {
            existing.eq_ignore_ascii_case(switch)
                && value.is_none_or(|value| {
                    existing_value
                        .as_deref()
                        .is_some_and(|existing| unquote(existing) == value)
                })
        })
}

/// The Options list's click: adds switch `switch` (with `value`) to the
/// code, or takes it out when it is there with that value. A picture
/// (`\@`, `\#`) replaces the one there; a `\*` format is one of several.
pub fn toggle_switch(code: &str, switch: &str, value: Option<&str>) -> String {
    let mut parsed = parse_code(code);
    let rendered = value.map(|value| {
        if matches!(switch, "\\@" | "\\#") || value.chars().any(char::is_whitespace) {
            format!("\"{}\"", value.replace('"', "\\\""))
        } else {
            value.to_string()
        }
    });
    let position = parsed
        .switches
        .iter()
        .position(|(existing, existing_value)| {
            existing.eq_ignore_ascii_case(switch)
                && (switch != "\\*"
                    || existing_value.as_deref().map(unquote) == value.map(str::to_string))
        });
    match position {
        Some(at) => {
            let same = parsed.switches[at].1.as_deref().map(unquote) == value.map(str::to_string);
            if same || value.is_none() {
                parsed.switches.remove(at);
            } else {
                parsed.switches[at].1 = rendered;
            }
        }
        None => parsed.switches.push((switch.to_string(), rendered)),
    }
    unparse(&parsed)
}

/// Sets operand `index` of the code (a bookmark, a style, a property…).
pub fn set_operand(code: &str, index: usize, value: &str, quoted: bool) -> String {
    let mut parsed = parse_code(code);
    let value = quote(value, quoted);
    if index < parsed.operands.len() {
        parsed.operands[index] = value;
    } else {
        parsed.operands.push(value);
    }
    unparse(&parsed)
}

/// Reads a file a field names (INCLUDETEXT, INCLUDEPICTURE).
type FileReader = Box<dyn Fn(&str) -> Option<Vec<u8>>>;

/// What the host tells field evaluation: the clock, the user, the file.
#[derive(Clone, Debug)]
pub struct FieldHost {
    pub now: FieldDateTime,
    pub utc_offset_seconds: i64,
    pub user_name: String,
    pub user_initials: String,
    pub user_address: String,
    /// The document's file, once saved.
    pub file: Option<PathBuf>,
    pub japanese: bool,
    /// The merge 差し込み文書 previews and the recipient shown (an index
    /// into its chosen records): merge fields take its values on an update
    /// (`MailMerge::previewed`).
    pub merge: Option<(crate::word_document::mail_merge::MergeRequest, usize)>,
}

impl Default for FieldHost {
    fn default() -> Self {
        let environment = FieldEnvironment::default();
        Self {
            now: environment.now,
            utc_offset_seconds: 0,
            user_name: String::new(),
            user_initials: String::new(),
            user_address: String::new(),
            file: None,
            japanese: true,
            merge: None,
        }
    }
}

/// The initials Word derives from a name: the first letter of each word, or
/// the first character of a name without spaces (山田 → 山).
pub fn initials_of(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() > 1 {
        return words
            .iter()
            .filter_map(|word| word.chars().next())
            .collect::<String>()
            .to_uppercase();
    }
    name.chars().next().map(String::from).unwrap_or_default()
}

/// Whether `range` covers the whole document, as ⌘A leaves it: F9 then
/// updates every story, headers and notes too.
fn covers_document<S: WordSession + ?Sized>(session: &S, range: SelectionRange) -> bool {
    let (start, end) = if (range.anchor.paragraph, range.anchor.offset)
        <= (range.focus.paragraph, range.focus.offset)
    {
        (range.anchor, range.focus)
    } else {
        (range.focus, range.anchor)
    };
    let document = session.word_document();
    let last = document.paragraph_count().saturating_sub(1);
    let last_length = document
        .paragraph(last)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.len()).sum())
        .unwrap_or(0);
    start == TextPosition::new(0, 0) && end.paragraph >= last && end.offset >= last_length
}

/// The field commands over any [`WordSession`].
pub trait WordFields: WordSession {
    /// What evaluation needs, from the host and from the laid-out session.
    fn field_environment(
        &self,
        host: &FieldHost,
        answers: HashMap<String, String>,
    ) -> FieldEnvironment {
        let document = self.word_document();
        let mut paragraph_pages = HashMap::new();
        for paragraph in 0..document.paragraph_count() {
            if let Some(page) = self.paragraph_page(paragraph) {
                paragraph_pages.insert(paragraph, page);
            }
        }
        let folder = host
            .file
            .as_ref()
            .and_then(|file| file.parent().map(PathBuf::from));
        let read_file: Option<FileReader> = Some(Box::new(move |path: &str| {
            let path = std::path::Path::new(path);
            let path = match (&folder, path.is_absolute()) {
                (Some(folder), false) => folder.join(path),
                _ => path.to_path_buf(),
            };
            std::fs::read(path).ok()
        }));
        FieldEnvironment {
            now: host.now,
            utc_offset_seconds: host.utc_offset_seconds,
            user_name: host.user_name.clone(),
            user_initials: if host.user_initials.is_empty() {
                initials_of(&host.user_name)
            } else {
                host.user_initials.clone()
            },
            user_address: host.user_address.clone(),
            file_name: host
                .file
                .as_ref()
                .and_then(|file| file.file_name())
                .map(|name| name.to_string_lossy().into_owned()),
            file_path: host
                .file
                .as_ref()
                .map(|file| file.to_string_lossy().into_owned()),
            file_size: host
                .file
                .as_ref()
                .and_then(|file| std::fs::metadata(file).ok())
                .map(|metadata| metadata.len()),
            page_count: self.page_count(),
            paragraph_pages,
            prompt_answers: answers,
            read_file,
            japanese: host.japanese,
            merge: host.merge.clone(),
        }
    }

    /// The scope F9 has: the selection, or every story when the selection
    /// is the whole document (or there is none).
    fn field_scope(&self) -> FieldScope {
        match self.selection_range() {
            Some(range) if !covers_document(self, range) => FieldScope::Selection(range),
            _ => FieldScope::Document,
        }
    }

    /// The prompts an update of `scope` will show, in order.
    fn field_prompts(&self, scope: FieldScope) -> Vec<FieldPrompt> {
        fields::field_prompts(self.word_document(), scope)
    }

    /// F9: updates the fields in `scope` with the prompts' `answers`.
    fn update_fields(
        &mut self,
        scope: FieldScope,
        host: &FieldHost,
        answers: HashMap<String, String>,
    ) -> Result<(), String> {
        let environment = self.field_environment(host, answers);
        self.execute_command(Box::new(FieldCommand::update(scope, environment)), true)
    }

    /// The fields the selection touches.
    fn selected_fields(&self) -> Vec<FieldInfo> {
        match self.selection_range() {
            Some(range) => fields::fields_at(self.word_document(), range),
            None => Vec::new(),
        }
    }

    /// 挿入 › フィールド: inserts a field from its code at the selection.
    fn insert_field_code(&mut self, code: &str, host: &FieldHost) -> Result<(), String> {
        if self.selection_range().is_none() {
            return Err("there is no insertion point".to_string());
        }
        let environment = self.field_environment(host, HashMap::new());
        self.execute_command(
            Box::new(FieldCommand::insert(code.to_string(), environment)),
            false,
        )
    }

    /// フィールドの編集: replaces the selected field's code.
    fn edit_field_code(&mut self, code: &str, host: &FieldHost) -> Result<(), String> {
        let range = self
            .selection_range()
            .ok_or_else(|| "there is no field at the selection".to_string())?;
        let environment = self.field_environment(host, HashMap::new());
        self.execute_command(
            Box::new(FieldCommand::edit(range, code.to_string(), environment)),
            true,
        )
    }

    /// ⌘F11 / ⌘⇧F11: locks or unlocks the selected fields.
    fn lock_fields(&mut self, locked: bool) -> Result<(), String> {
        let scope = self.field_scope();
        self.execute_command(Box::new(FieldCommand::lock(scope, locked)), true)
    }

    /// ⌘⇧F9: turns the selected fields into their results.
    fn unlink_fields(&mut self) -> Result<(), String> {
        let scope = self.field_scope();
        self.execute_command(Box::new(FieldCommand::unlink(scope)), true)
    }

    /// ⇧F9: the view with the selected fields' codes toggled.
    fn toggled_field_codes(&self, view: &FieldCodeView) -> FieldCodeView {
        let mut view = view.clone();
        if let Some(range) = self.selection_range() {
            for id in fields::field_toggle_ids(self.word_document(), range) {
                if !view.toggled.remove(&id) {
                    view.toggled.insert(id);
                }
            }
        }
        view
    }

    /// ファイル › プロパティ.
    fn document_properties(&self) -> DocumentProperties {
        fields::document_properties(self.word_document())
    }

    fn set_document_properties(&mut self, properties: DocumentProperties) -> Result<(), String> {
        self.execute_command(Box::new(FieldCommand::properties(properties)), true)
    }

    /// The document variables DOCVARIABLE reads.
    fn document_variables(&self) -> Vec<(String, String)> {
        fields::document_variables(self.word_document())
    }

    fn set_document_variables(&mut self, variables: Vec<(String, String)>) -> Result<(), String> {
        self.execute_command(Box::new(FieldCommand::variables(variables)), true)
    }

    /// The bookmarks REF, PAGEREF and ASK can name.
    fn bookmark_names(&self) -> Vec<String> {
        self.word_document()
            .document
            .as_ref()
            .map(|document| {
                document
                    .bookmarks
                    .iter()
                    .map(|bookmark| bookmark.name.clone())
                    .filter(|name| !name.starts_with('_'))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The paragraph styles STYLEREF can name, by display name.
    fn paragraph_style_names(&self) -> Vec<String> {
        self.word_document()
            .styles_part
            .as_ref()
            .map(|styles| {
                styles
                    .paragraph_style_definitions()
                    .map(|style| style.name.clone().unwrap_or_else(|| style.style_id.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl<T: WordSession + ?Sized> WordFields for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_lists_fields_and_every_field_one_category() {
        for category in FieldCategory::ALL {
            assert!(!fields_in(Some(category)).is_empty(), "{category:?}");
        }
        assert_eq!(fields_in(None).len(), FIELDS.len());
        let mut names: Vec<&str> = FIELDS.iter().map(|spec| spec.name).collect();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "no field twice");
        assert!(field_spec("date").is_some());
        assert_eq!(
            field_spec("STYLEREF").unwrap().category,
            FieldCategory::LinksReferences
        );
    }

    #[test]
    fn codes_quote_operands_and_pictures() {
        assert_eq!(
            build_code(
                "date",
                &[],
                &[("\\@".into(), Some("ggge年M月d日".into()))],
                true
            ),
            "DATE \\@ \"ggge年M月d日\" \\* MERGEFORMAT"
        );
        assert_eq!(
            build_code(
                "STYLEREF",
                &[("見出し 1".into(), true)],
                &[("\\l".into(), None)],
                false
            ),
            "STYLEREF \"見出し 1\" \\l"
        );
        assert_eq!(
            build_code(
                "SEQ",
                &[("Figure".into(), false)],
                &[("\\* ".trim().into(), Some("ROMAN".into()))],
                false
            ),
            "SEQ Figure \\* ROMAN"
        );
        assert_eq!(
            build_code(
                "=",
                &[("SUM(ABOVE)".into(), false)],
                &[("\\#".into(), Some("#,##0".into()))],
                false
            ),
            "= SUM(ABOVE) \\# \"#,##0\""
        );
        assert_eq!(code_field_name("{ = 1+2 }").as_deref(), Some("="));
        assert_eq!(code_field_name(" page \\* roman").as_deref(), Some("PAGE"));
        assert_eq!(initials_of("Jade User"), "JU");
        assert_eq!(initials_of("山田"), "山");
    }

    #[test]
    fn the_options_list_adds_and_removes_switches() {
        let code = toggle_switch("DATE", "\\@", Some("yyyy/MM/dd"));
        assert_eq!(code, "DATE \\@ \"yyyy/MM/dd\"");
        let code = toggle_switch(&code, "\\@", Some("ggge年M月d日"));
        assert_eq!(code, "DATE \\@ \"ggge年M月d日\"");
        assert!(code_has_switch(&code, "\\@", Some("ggge年M月d日")));
        let code = toggle_switch(&code, "\\*", Some("MERGEFORMAT"));
        let code = toggle_switch(&code, "\\*", Some("DBNUM1"));
        assert_eq!(code, "DATE \\@ \"ggge年M月d日\" \\* DBNUM1 \\* MERGEFORMAT");
        let code = toggle_switch(&code, "\\*", Some("DBNUM1"));
        assert_eq!(code, "DATE \\@ \"ggge年M月d日\" \\* MERGEFORMAT");
        let code = toggle_switch("SEQ Figure", "\\r", Some("3"));
        assert_eq!(code, "SEQ Figure \\r 3");
        assert_eq!(toggle_switch(&code, "\\r", None), "SEQ Figure");
        assert_eq!(toggle_switch("FILENAME", "\\p", None), "FILENAME \\p");
        assert_eq!(toggle_switch("FILENAME \\p", "\\p", None), "FILENAME");
        assert_eq!(set_operand("REF", 0, "Total", false), "REF Total");
        assert_eq!(
            set_operand("STYLEREF \"x\" \\l", 0, "見出し 1", true),
            "STYLEREF \"見出し 1\" \\l"
        );
    }
}
