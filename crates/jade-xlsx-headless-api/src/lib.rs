//! The shapes a caller with no UI sees.
//!
//! Everything here is what a workbook *says*, not how it looks: a cell reports
//! the value the file stores, the formula behind it, the format code that
//! applies, and the text those three come to. A caller that wants to read a
//! sheet needs the last one; a caller that wants to reason about it needs the
//! first three.
//!
//! Deliberately a separate crate from the engine, as on the DOCX side: the
//! types travel across a process boundary (a CLI's stdout, an MCP server's
//! response), and a consumer should be able to depend on them without pulling
//! in a spreadsheet.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AutoFilterTarget {
    Worksheet,
    Table { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AutoFilterChange {
    Enable {
        target: AutoFilterTarget,
        reference: Option<String>,
    },
    Disable {
        target: AutoFilterTarget,
    },
    Values {
        target: AutoFilterTarget,
        reference: Option<String>,
        column: u32,
        values: Vec<String>,
        #[serde(default)]
        include_blank: bool,
    },
    Dates {
        target: AutoFilterTarget,
        reference: Option<String>,
        column: u32,
        groups: Vec<DateFilterGroup>,
        #[serde(default)]
        include_blank: bool,
    },
    Rank {
        target: AutoFilterTarget,
        reference: Option<String>,
        column: u32,
        value: u32,
        #[serde(default = "rank_top_default")]
        top: bool,
        #[serde(default)]
        percent: bool,
    },
    Custom {
        target: AutoFilterTarget,
        reference: Option<String>,
        column: u32,
        conditions: Vec<CustomFilterCondition>,
        #[serde(default)]
        and: bool,
    },
    Clear {
        target: AutoFilterTarget,
        column: Option<u32>,
    },
    Reapply {
        target: AutoFilterTarget,
    },
}

fn rank_top_default() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterComparisonOperator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}
impl FilterComparisonOperator {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equal => "equal",
            Self::NotEqual => "notEqual",
            Self::LessThan => "lessThan",
            Self::LessThanOrEqual => "lessThanOrEqual",
            Self::GreaterThan => "greaterThan",
            Self::GreaterThanOrEqual => "greaterThanOrEqual",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomFilterCondition {
    pub operator: FilterComparisonOperator,
    /// Raw numeric serial or text criterion. Equal/notEqual support text wildcards.
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DateFilterGrouping {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}
impl DateFilterGrouping {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Year => "year",
            Self::Month => "month",
            Self::Day => "day",
            Self::Hour => "hour",
            Self::Minute => "minute",
            Self::Second => "second",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateFilterGroup {
    pub grouping: DateFilterGrouping,
    pub year: u32,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub second: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFilterOption {
    pub value: String,
    pub values: Vec<String>,
    pub label: String,
    pub blank: bool,
    pub count: u32,
    pub selected: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFilterOptions {
    pub columns: Vec<AutoFilterColumnOption>,
    pub options: Vec<AutoFilterOption>,
    pub truncated: bool,
    pub unsupported: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFilterColumnOption {
    pub column: u32,
    pub worksheet_column: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFilterSummary {
    pub changed_rows: u32,
    pub active_columns: u32,
}

/// Row sorting; key columns are zero-based offsets within `reference`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortRequest {
    pub reference: String,
    #[serde(default)]
    pub header: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    /// BCP 47 language tag. Headless callers default to en-US.
    #[serde(default)]
    pub locale: Option<String>,
    /// Use saved phonetic guides; defaults to true for Japanese locales.
    #[serde(default)]
    pub phonetic: Option<bool>,
    pub keys: Vec<SortKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortKey {
    pub column: u32,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortSummary {
    pub reference: String,
    pub rows: u32,
    pub moved_rows: u32,
    pub locale: String,
    pub phonetic: bool,
}

/// What the workbook holds, without reading any of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkbookSummary {
    pub sheets: Vec<SheetSummary>,
    #[serde(default)]
    pub defined_names: Vec<DefinedNameSummary>,
    /// `1900` or `1904` — the epoch date serials count from.
    pub date_system: u16,
    /// The saved active tab's workbook index, when the file declares one.
    #[serde(default)]
    pub active_sheet_index: Option<usize>,
    /// Whether the file asks its opening spreadsheet to recalculate formulas.
    #[serde(default)]
    pub full_calculation_on_load: bool,
}

/// A workbook- or sheet-scoped name a formula can refer to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinedNameSummary {
    pub name: String,
    pub formula: String,
    /// The local sheet's tab-order index, or `None` for a workbook-global name.
    pub sheet_index: Option<usize>,
    /// The local sheet's name, resolved for callers that should not need to
    /// repeat workbook-index bookkeeping.
    pub sheet: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetSummary {
    pub index: usize,
    pub name: String,
    /// `visible`, `hidden`, or `very_hidden` as declared by the workbook.
    #[serde(default)]
    pub visibility: SheetVisibility,
    /// One past the last column any row writes a cell in.
    pub used_columns: u32,
    /// One past the last row the sheet writes.
    pub used_rows: u32,
    /// The used range in `A1:D10` form, or `None` for an empty sheet.
    pub used_range: Option<String>,
    pub merges: Vec<String>,
    pub show_grid_lines: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_pane: Option<FreezePaneSummary>,
    /// Excel tables on the sheet (Format as Table / `ListObject`s).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<TableSummary>,
    /// The page header and footer, when the sheet has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_footer: Option<HeaderFooterView>,
    /// Manual page breaks, when the sheet has any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_breaks: Option<PageBreaksView>,
    /// Shapes, text boxes, pictures and charts anchored on the sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<ShapeView>,
    /// The cells carrying a note, so a caller can find them without walking
    /// the grid asking every cell whether it has one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<SheetNoteView>,
}

/// A note, with the cell it is on. [`CommentView`] says the same thing about
/// a cell already being looked at; this says it about a sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetNoteView {
    /// The cell the note is on, as `B4`.
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub text: String,
}

/// An Excel table, by where it is and what its columns are called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSummary {
    /// The name structured references use: `Table1` in `Table1[Amount]`.
    pub name: String,
    /// What the ribbon shows; usually the same as `name`.
    pub display_name: String,
    /// The cells the table covers, header and totals rows included, as
    /// `A1:D10`.
    pub reference: String,
    /// The row the column names sit on, 1-based, when the table shows a
    /// header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_row: Option<u32>,
    /// The totals row, 1-based, when the table shows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totals_row: Option<u32>,
    /// Column names, left to right.
    pub columns: Vec<String>,
    /// `TableStyleMedium2` and the like.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Whether the header row has filter dropdowns.
    pub auto_filter: bool,
}

/// The table a cell belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCellView {
    pub name: String,
    /// The column the cell is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// Whether the cell is on the header row — where typing renames the
    /// column.
    pub header: bool,
    /// Whether the cell is on the totals row.
    pub totals: bool,
}

/// A sheet's page header and footer, in Excel's own code language: `&L`,
/// `&C`, `&R` open the left, centre and right sections; `&P` is the page
/// number, `&N` the page count, `&D` the date, `&T` the time, `&A` the
/// sheet name, `&F` the file name, `&&` a literal ampersand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HeaderFooterView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub odd_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub odd_footer: Option<String>,
    /// Used only when `different_odd_even` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub even_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub even_footer: Option<String>,
    /// Used only when `different_first` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_footer: Option<String>,
    pub different_odd_even: bool,
    pub different_first: bool,
    pub scale_with_doc: bool,
    pub align_with_margins: bool,
}

/// A change to a sheet's header and footer: a field given is set (`null`
/// clears that section), a field left out keeps what the sheet has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HeaderFooterChange {
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub odd_header: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub odd_footer: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub even_header: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub even_footer: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub first_header: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub first_footer: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub different_odd_even: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub different_first: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_with_doc: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align_with_margins: Option<bool>,
}

impl HeaderFooterChange {
    /// Whether the change names anything to change.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `null` and an absent field differ here: absent keeps, `null` clears.
fn double_option<'de, D>(deserializer: D) -> std::result::Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Where a sheet's manual page breaks fall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PageBreaksView {
    /// The 1-based rows a page starts on: `[6]` means rows 1-5 print on
    /// one page and row 6 starts the next.
    pub rows: Vec<u32>,
    /// The columns a page starts on, as letters.
    pub columns: Vec<String>,
}

/// What kind of object a drawing anchor holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKindView {
    TextBox,
    Shape,
    Picture,
    Chart,
    GraphicFrame,
    Group,
    Connector,
}

/// A shape, text box, picture or chart anchored on the sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeView {
    /// The drawing's own id for the object; stable across edits, unlike
    /// its position in the list.
    pub id: u32,
    /// `TextBox 1`, `Picture 2`, `Chart 3`, as Excel names them.
    pub name: String,
    pub kind: ShapeKindView,
    /// The cells the object is anchored to, as `B2:D5` (or a single cell
    /// for an object anchored at one).
    pub anchor: String,
    /// The object's text, paragraphs joined with newlines, for a shape that
    /// has a text body. Absent for a picture or a chart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SheetVisibility {
    #[default]
    Visible,
    Hidden,
    VeryHidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezePaneSummary {
    /// Columns pinned at the left of the sheet.
    pub frozen_columns: u32,
    /// Rows pinned at the top of the sheet.
    pub frozen_rows: u32,
    /// The first cell in the scrollable bottom-right pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_left_cell: Option<String>,
}

/// What kind of value a cell holds, before any formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellKind {
    Empty,
    Number,
    Text,
    Bool,
    Error,
}

/// What a formula's stored result means to the current headless session.
/// Jade preserves cached results but does not run Excel's calculation engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormulaState {
    /// The workbook supplied a cached result and does not request recalculation.
    Cached,
    /// The workbook requests recalculation, so this cached result may be old.
    Stale,
    /// The formula has no cached result, normally because this session wrote it.
    Uncalculated,
}

/// Formula cache state across the current workbook, including hidden sheets.
/// `cached` means a stored result without a recalculation request, not proof
/// that Excel has independently verified that result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalculationStatus {
    #[serde(default = "default_calculation_mode")]
    pub calculation_mode: String,
    #[serde(default = "default_calculate_on_save")]
    pub calculate_on_save: bool,
    pub date_system: u16,
    pub formula_count: usize,
    pub cached: usize,
    pub stale: usize,
    pub uncalculated: usize,
    pub requires_recalculation: bool,
}

fn default_calculation_mode() -> String {
    "auto".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterCriterionView {
    pub kind: String,
    pub attributes: std::collections::BTreeMap<String, String>,
    pub children: Vec<FilterCriterionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterColumnView {
    pub column_id: u32,
    pub hidden_button: bool,
    pub show_button: bool,
    pub criterion: Option<FilterCriterionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFilterView {
    pub target: AutoFilterTarget,
    /// `worksheet` or the table's name.
    pub owner: String,
    pub reference: Option<String>,
    pub has_criteria: bool,
    pub filter_mode: Option<bool>,
    pub columns: Vec<FilterColumnView>,
    pub issues: Vec<String>,
}
fn default_calculate_on_save() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConditionalFormattingStatus {
    pub rule_count: usize,
    pub supported_rules: usize,
    pub unsupported: Vec<ConditionalFormattingIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConditionalFormattingIssue {
    pub sheet: String,
    pub sheet_index: usize,
    pub ranges: Vec<String>,
    pub rule_type: String,
    pub priority: u32,
    pub reason: String,
}

/// One conditional formatting rule of a sheet, as
/// `HeadlessWorkbook::conditional_formats` lists them: in priority order,
/// the rule Excel evaluates first coming first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalFormatView {
    /// The ranges it applies to (`B2:B10`, `D4`), as the file lists them.
    pub ranges: Vec<String>,
    /// The file's `priority` (smaller is evaluated first).
    pub priority: u32,
    /// The file's rule type (`cellIs`, `expression`, `colorScale`, …).
    pub rule_type: String,
    /// The rule, when it is one jade writes — `None` for kinds it only
    /// keeps (icon sets, top-ten, text rules…) and for rules whose style it
    /// cannot express.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<ConditionalRuleSpec>,
    /// `false` for a rule with Excel 2010 extensions (an `x14` data bar or
    /// a rule kept only in the extension list): it can be moved in the
    /// priority order but not edited, re-ranged or deleted.
    pub editable: bool,
}

/// A conditional formatting rule jade writes. Formulas are written without
/// their `=` and are relative to the top-left cell of the rule's first
/// range, as Excel stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConditionalRuleSpec {
    /// "Format only cells that contain": the value compared with one
    /// formula, or two for `between` / `not_between`.
    CellIs {
        operator: ConditionalOperator,
        formulas: Vec<String>,
        #[serde(default, skip_serializing_if = "ConditionalStyle::is_empty")]
        style: ConditionalStyle,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        stop_if_true: bool,
    },
    /// "Use a formula to determine which cells to format".
    Expression {
        formula: String,
        #[serde(default, skip_serializing_if = "ConditionalStyle::is_empty")]
        style: ConditionalStyle,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        stop_if_true: bool,
    },
    /// A two- or three-colour scale.
    ColorScale { points: Vec<ColorScalePoint> },
    /// A solid data bar from `min` to `max`.
    DataBar {
        min: ConditionalThresholdSpec,
        max: ConditionalThresholdSpec,
        color: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionalOperator {
    Between,
    NotBetween,
    Equal,
    NotEqual,
    GreaterThan,
    LessThan,
    GreaterThanOrEqual,
    LessThanOrEqual,
}

impl ConditionalOperator {
    pub const ALL: [ConditionalOperator; 8] = [
        Self::Between,
        Self::NotBetween,
        Self::Equal,
        Self::NotEqual,
        Self::GreaterThan,
        Self::LessThan,
        Self::GreaterThanOrEqual,
        Self::LessThanOrEqual,
    ];

    /// As SpreadsheetML spells it (`greaterThanOrEqual`).
    pub fn xml_name(self) -> &'static str {
        match self {
            Self::Between => "between",
            Self::NotBetween => "notBetween",
            Self::Equal => "equal",
            Self::NotEqual => "notEqual",
            Self::GreaterThan => "greaterThan",
            Self::LessThan => "lessThan",
            Self::GreaterThanOrEqual => "greaterThanOrEqual",
            Self::LessThanOrEqual => "lessThanOrEqual",
        }
    }

    pub fn from_xml_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.xml_name() == name)
    }

    /// How many formulas the comparison takes.
    pub fn operands(self) -> usize {
        match self {
            Self::Between | Self::NotBetween => 2,
            _ => 1,
        }
    }
}

/// What a matching cell takes on top of its own format. `None` keeps the
/// cell's. Colours are `RRGGBB`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionalStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
}

impl ConditionalStyle {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Where a colour scale point or a data bar end sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionalThresholdSpec {
    pub kind: ThresholdKind,
    /// The number, percent, percentile or formula; none for `min` / `max`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdKind {
    Min,
    Max,
    Num,
    Percent,
    Percentile,
    Formula,
}

impl ThresholdKind {
    pub const ALL: [ThresholdKind; 6] = [
        Self::Min,
        Self::Max,
        Self::Num,
        Self::Percent,
        Self::Percentile,
        Self::Formula,
    ];

    pub fn xml_name(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
            Self::Num => "num",
            Self::Percent => "percent",
            Self::Percentile => "percentile",
            Self::Formula => "formula",
        }
    }

    pub fn from_xml_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.xml_name() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorScalePoint {
    pub kind: ThresholdKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub color: String,
}

impl ConditionalRuleSpec {
    /// The rule as jade reads it back — formulas without `=`, colours as
    /// upper-case `RRGGBB`, `General` for an empty number format — or why
    /// jade refuses to write it. Callers that must agree with jade on what
    /// it accepts (a collaboration model) call this too.
    pub fn canonical(&self) -> Result<Self, String> {
        let formula = |text: &str| -> Result<String, String> {
            let text = text.trim();
            let text = text.strip_prefix('=').unwrap_or(text).trim();
            if text.is_empty() {
                return Err("a conditional format formula cannot be empty".into());
            }
            if text.chars().any(|c| c.is_control()) {
                return Err("a conditional format formula cannot hold control characters".into());
            }
            Ok(text.to_string())
        };
        let threshold = |kind: ThresholdKind, value: &Option<String>| -> Result<_, String> {
            Ok(match kind {
                ThresholdKind::Min | ThresholdKind::Max => {
                    if value.is_some() {
                        return Err(format!("a {} point takes no value", kind.xml_name()));
                    }
                    None
                }
                ThresholdKind::Formula => Some(formula(
                    value
                        .as_deref()
                        .ok_or("a formula point needs its formula")?,
                )?),
                _ => {
                    let text = value
                        .as_deref()
                        .ok_or_else(|| format!("a {} point needs a number", kind.xml_name()))?
                        .trim();
                    let number: f64 = text
                        .parse()
                        .map_err(|_| format!("{text:?} is not a number"))?;
                    if !number.is_finite()
                        || (matches!(kind, ThresholdKind::Percent | ThresholdKind::Percentile)
                            && !(0.0..=100.0).contains(&number))
                    {
                        return Err(format!("{text:?} is out of range"));
                    }
                    Some(text.to_string())
                }
            })
        };
        Ok(match self {
            Self::CellIs {
                operator,
                formulas,
                style,
                stop_if_true,
            } => {
                if formulas.len() != operator.operands() {
                    return Err(format!(
                        "{} takes {} formula(s)",
                        operator.xml_name(),
                        operator.operands()
                    ));
                }
                Self::CellIs {
                    operator: *operator,
                    formulas: formulas
                        .iter()
                        .map(|f| formula(f))
                        .collect::<Result<_, _>>()?,
                    style: style.canonical()?,
                    stop_if_true: *stop_if_true,
                }
            }
            Self::Expression {
                formula: text,
                style,
                stop_if_true,
            } => Self::Expression {
                formula: formula(text)?,
                style: style.canonical()?,
                stop_if_true: *stop_if_true,
            },
            Self::ColorScale { points } => {
                if !(2..=3).contains(&points.len()) {
                    return Err("a colour scale has two or three points".into());
                }
                Self::ColorScale {
                    points: points
                        .iter()
                        .map(|point| {
                            Ok(ColorScalePoint {
                                kind: point.kind,
                                value: threshold(point.kind, &point.value)?,
                                color: rgb(&point.color)?,
                            })
                        })
                        .collect::<Result<_, String>>()?,
                }
            }
            Self::DataBar { min, max, color } => Self::DataBar {
                min: ConditionalThresholdSpec {
                    kind: min.kind,
                    value: threshold(min.kind, &min.value)?,
                },
                max: ConditionalThresholdSpec {
                    kind: max.kind,
                    value: threshold(max.kind, &max.value)?,
                },
                color: rgb(color)?,
            },
        })
    }

    /// The formulas a structural change rewrites (`<formula>` elements).
    pub fn formulas_mut(&mut self) -> Vec<&mut String> {
        match self {
            Self::CellIs { formulas, .. } => formulas.iter_mut().collect(),
            Self::Expression { formula, .. } => vec![formula],
            Self::ColorScale { .. } | Self::DataBar { .. } => Vec::new(),
        }
    }
}

impl ConditionalStyle {
    fn canonical(&self) -> Result<Self, String> {
        let mut style = self.clone();
        for color in [&mut style.font_color, &mut style.fill_color]
            .into_iter()
            .flatten()
        {
            *color = rgb(color)?;
        }
        if let Some(code) = &mut style.number_format {
            if code.is_empty() || code.eq_ignore_ascii_case("General") {
                *code = "General".to_string();
            }
        }
        Ok(style)
    }
}

/// `RRGGBB`, upper-case, from `RRGGBB` or `#rrggbb`.
fn rgb(text: &str) -> Result<String, String> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(hex.to_ascii_uppercase())
    } else {
        Err(format!("{text:?} is not an RRGGBB colour"))
    }
}

/// A worksheet hyperlink applying to a cell. External links use `target`;
/// jumps within the workbook use `location`. OOXML permits both together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyperlinkView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
}

/// A legacy cell comment, called a note by current Excel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub text: String,
}

/// The worksheet input rule applying to a cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataValidationView {
    pub ranges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    pub allow_blank: bool,
    pub show_input_message: bool,
    pub show_error_message: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula2: Option<String>,
}

/// How one edge of a cell is drawn, by its OOXML name (`thin`, `mediumDashed`,
/// …). `None` is no line at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BorderStyle {
    None,
    Thin,
    Medium,
    Thick,
    Dashed,
    Dotted,
    Double,
    Hair,
    MediumDashed,
    DashDot,
    MediumDashDot,
    DashDotDot,
    MediumDashDotDot,
    SlantDashDot,
}

impl BorderStyle {
    pub const ALL: [BorderStyle; 14] = [
        BorderStyle::None,
        BorderStyle::Thin,
        BorderStyle::Medium,
        BorderStyle::Thick,
        BorderStyle::Dashed,
        BorderStyle::Dotted,
        BorderStyle::Double,
        BorderStyle::Hair,
        BorderStyle::MediumDashed,
        BorderStyle::DashDot,
        BorderStyle::MediumDashDot,
        BorderStyle::DashDotDot,
        BorderStyle::MediumDashDotDot,
        BorderStyle::SlantDashDot,
    ];

    /// The `style` attribute value, as the file writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            BorderStyle::None => "none",
            BorderStyle::Thin => "thin",
            BorderStyle::Medium => "medium",
            BorderStyle::Thick => "thick",
            BorderStyle::Dashed => "dashed",
            BorderStyle::Dotted => "dotted",
            BorderStyle::Double => "double",
            BorderStyle::Hair => "hair",
            BorderStyle::MediumDashed => "mediumDashed",
            BorderStyle::DashDot => "dashDot",
            BorderStyle::MediumDashDot => "mediumDashDot",
            BorderStyle::DashDotDot => "dashDotDot",
            BorderStyle::MediumDashDotDot => "mediumDashDotDot",
            BorderStyle::SlantDashDot => "slantDashDot",
        }
    }

    /// The style a `style` attribute names; `None` for a name OOXML does not
    /// define.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.as_str() == name)
    }
}

/// One edge: its style and colour.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BorderLine {
    pub style: BorderStyle,
    /// `RRGGBB`; absent for Excel's automatic colour (and for no line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl BorderLine {
    /// No line.
    pub fn none() -> Self {
        Self {
            style: BorderStyle::None,
            color: None,
        }
    }

    pub fn is_none(&self) -> bool {
        self.style == BorderStyle::None
    }
}

/// The lines drawn around (and across) one cell. An edge that is absent has
/// no line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellBorderView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<BorderLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<BorderLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<BorderLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<BorderLine>,
    /// The line shared by both diagonals; drawn only in the directions
    /// `diagonal_up` / `diagonal_down` turn on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagonal: Option<BorderLine>,
    /// Bottom-left to top-right.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diagonal_up: bool,
    /// Top-left to bottom-right.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diagonal_down: bool,
}

impl CellBorderView {
    pub fn is_empty(&self) -> bool {
        *self == CellBorderView::default()
    }
}

/// One cell, as a caller without a screen needs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellView {
    /// `B3`.
    pub reference: String,
    pub row: u32,
    pub column: u32,
    pub kind: CellKind,
    /// The stored number, for a numeric or boolean cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<f64>,
    /// The stored text, for a text or error cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// What the cell shows: the value put through its number format. A date is
    /// only a date here — as a stored number it is indistinguishable from any
    /// other.
    pub display: String,
    /// The formula without its `=`, when there is one. OOXML shared formulas
    /// are expanded to this cell's actual A1 references rather than reported
    /// only on the master cell. The value beside it is what the file cached;
    /// nothing recalculates yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    /// Cache freshness for a formula. Absent on non-formula cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula_state: Option<FormulaState>,
    /// The format code that produced `display`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
    /// The link applying to this cell, including a worksheet hyperlink whose
    /// `ref` spans a range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hyperlink: Option<HyperlinkView>,
    /// The note attached to this cell, with rich-text runs flattened to text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<CommentView>,
    /// The standard input rule applying to this cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_validation: Option<DataValidationView>,
    /// The Excel table this cell is part of, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<TableCellView>,
    /// The cell's own border lines, when it has any (a merged region's
    /// outline is drawn from the edges of the cells along it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<CellBorderView>,
}

/// A rectangle of cells, in row-major order. Cells the file does not write are
/// present and empty, so a caller can index the grid rather than search it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeView {
    pub sheet: String,
    /// The range asked for, normalised to `A1:D10`.
    pub reference: String,
    pub rows: Vec<Vec<CellView>>,
}

/// A whole sheet as display text, which is what a reader usually wants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetText {
    pub sheet: String,
    /// The rectangle represented by `rows`, or `None` for an empty sheet.
    pub reference: Option<String>,
    pub rows: Vec<Vec<String>>,
}

/// Where a search matched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindHit {
    pub sheet: String,
    pub sheet_index: usize,
    pub reference: String,
    /// The cell's display text, so a caller can show the match in context.
    pub display: String,
    /// Formula source when the hit is a formula cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
}

/// What a recalculation did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecalculationSummary {
    /// Formula cells whose cached value was refreshed in the file.
    pub refreshed: usize,
    /// Formula cells the engine evaluated whose cache was already current.
    pub unchanged: usize,
    /// Formula cells left at their cached values, with the engine's reason.
    pub skipped: Vec<SkippedFormula>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedFormula {
    pub sheet: String,
    pub reference: String,
    pub reason: String,
}

/// What a fill wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillSummary {
    pub sheet: String,
    /// The run the fill continued, as `F1:F2`.
    pub source: String,
    /// The rectangle the fill now covers, source included.
    pub target: String,
    /// The cells written, in row order. Empty when the target was the source.
    pub cells: Vec<String>,
}

/// What a format copy wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatSummary {
    pub sheet: String,
    /// The cells whose formats were copied, as `B4:B6`.
    pub source: String,
    /// The cells that now wear them — the target given, or, for a single
    /// target cell, the source's shape from it.
    pub target: String,
    /// The cells written, in row order.
    pub cells: Vec<String>,
}

/// What a format change wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetFormatSummary {
    pub sheet: String,
    /// The cells formatted, as `B4:B6`.
    pub range: String,
    /// The cells written, in row order.
    pub cells: Vec<String>,
    /// How many `cellXfs` entries the change added to the styles part —
    /// zero when every look it needed was there already.
    pub styles_added: usize,
}

/// What a structural change did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructureSummary {
    pub sheet: String,
    /// `rows` or `columns`.
    pub axis: String,
    /// The first row (`5`) or column (`C`) inserted or deleted.
    pub at: String,
    pub count: u32,
    /// `true` for an insertion, `false` for a deletion.
    pub inserted: bool,
}

/// What merging a range did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeSummary {
    pub sheet: String,
    /// The merged range, top-left first, as `B2:C3`.
    pub range: String,
    /// The cells whose value or formula the merge discarded — every cell of
    /// the range but the top-left one that held something — in row order.
    pub cleared: Vec<String>,
}

/// A value to put in a cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum CellInput {
    Number(f64),
    Text(String),
    Bool(bool),
    /// A formula, with or without the leading `=`. Formula calculation is
    /// deferred to the spreadsheet that opens the exported workbook.
    Formula(String),
    /// Clears the value and keeps the cell, along with its format.
    Blank,
}

impl CellInput {
    /// Reads a value the way a spreadsheet reads what you type: a number if it
    /// looks like one, a boolean for `TRUE`/`FALSE`, blank for nothing at all,
    /// and text otherwise.
    ///
    /// A caller that means the text `007` has to say so — [`CellInput::Text`]
    /// exists for exactly that, and the CLI spells it `--text`.
    pub fn parse(input: &str) -> Self {
        if input.is_empty() {
            return CellInput::Blank;
        }
        if let Ok(number) = input.parse::<f64>() {
            if number.is_finite() {
                return CellInput::Number(number);
            }
        }
        match input.to_ascii_uppercase().as_str() {
            "TRUE" => CellInput::Bool(true),
            "FALSE" => CellInput::Bool(false),
            _ => CellInput::Text(input.to_string()),
        }
    }
}
