//! Word's table tools beyond the ribbon's row and column buttons, for both
//! shells: the テーブルデザイン tab's style options and style gallery, the
//! レイアウト tab's selection, cell insertion and deletion, AutoFit, cell
//! size, text direction, sorting, formulas and conversions, 表のプロパティ
//! (表 / 行 / 列 / セル), 線種とページ罫線と網かけ, the column and row borders
//! dragged with the mouse, and 罫線を引く / 消しゴム.
//!
//! Everything here works on the caret's table through the session's last
//! paint — which cell a paragraph was laid out in, and where — and applies
//! the engine's headless commands, so an action is one undo step and the
//! document repaints as Word would draw it.

use crate::commands::resize_table_column::ResizeTableColumnCommand;
use crate::document::{BodyElement, Table, TableLayoutMode, TableRowHeightRule};
use crate::word_document::BUILTIN_TABLE_STYLES;
use jade_docx_headless_api::{
    HeadlessBorderEdge, HeadlessCommand, HeadlessTableCellDeleteDirection,
    HeadlessTableCellInsertDirection, HeadlessTableCellSelection, HeadlessTableFitMode,
    HeadlessTableSortKey, HeadlessTableTextWrapping,
};

use super::{CellLayout, Position, TableCell, WordFormatting, WordSession};

/// What a 並べ替え key is made of, for the shells.
pub use jade_docx_headless_api::{
    HeadlessTableSortKey as SortKey, HeadlessTableSortKind as SortKind,
};

/// Points per layout pixel: the renderer lays pages out at 96 per inch.
const PT_PER_PX: f64 = 0.75;
/// Millimetres per point, for the dialogs' lengths.
pub const MM_PER_PT: f32 = 25.4 / 72.0;
/// Word's narrowest column.
const MIN_COLUMN_PX: f64 = 12.0;
/// How near a border the pointer must be to grab it, in layout pixels.
const BORDER_GRAB_PX: f64 = 3.0;

fn points(value: &str) -> Option<f32> {
    value.trim().strip_suffix("pt")?.trim().parse().ok()
}

fn percent(value: &str) -> Option<f32> {
    value.trim().strip_suffix('%')?.trim().parse().ok()
}

// ---- what the ribbon shows ---------------------------------------------------

/// 表スタイルのオプション: which of the style's regions are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableLookState {
    /// タイトル行.
    pub header_row: bool,
    /// 集計行.
    pub total_row: bool,
    /// 縞模様 (行).
    pub banded_rows: bool,
    /// 最初の列.
    pub first_column: bool,
    /// 最後の列.
    pub last_column: bool,
    /// 縞模様 (列).
    pub banded_columns: bool,
}

/// One 表スタイルのオプション checkbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookOption {
    HeaderRow,
    TotalRow,
    BandedRows,
    FirstColumn,
    LastColumn,
    BandedColumns,
}

impl LookOption {
    pub const ALL: [LookOption; 6] = [
        LookOption::HeaderRow,
        LookOption::FirstColumn,
        LookOption::TotalRow,
        LookOption::LastColumn,
        LookOption::BandedRows,
        LookOption::BandedColumns,
    ];

    pub fn is_on(self, state: &TableLookState) -> bool {
        match self {
            LookOption::HeaderRow => state.header_row,
            LookOption::TotalRow => state.total_row,
            LookOption::BandedRows => state.banded_rows,
            LookOption::FirstColumn => state.first_column,
            LookOption::LastColumn => state.last_column,
            LookOption::BandedColumns => state.banded_columns,
        }
    }
}

/// A table style the gallery offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableStyleChoice {
    /// What `ApplyTableNamedStyle` takes.
    pub id: String,
    /// Word's name (`Grid Table 4 - Accent 1`).
    pub name: String,
    /// Header fill, band fill and border colour, `RRGGBB`, for the swatch.
    pub preview: (Option<String>, Option<String>, Option<String>),
}

/// The built-in styles' swatch colours, by id.
fn builtin_preview(id: &str) -> (Option<String>, Option<String>, Option<String>) {
    let (header, band, border) = match id {
        "TableGrid" => (None, None, Some("000000")),
        "PlainTable1" => (None, Some("F2F2F2"), Some("BFBFBF")),
        "PlainTable3" => (None, Some("F2F2F2"), None),
        "GridTable1Light" => (None, None, Some("999999")),
        "GridTable4" => (Some("000000"), Some("CCCCCC"), Some("666666")),
        "GridTable4-Accent1" => (Some("4472C4"), Some("D9E2F3"), Some("8EAADB")),
        "GridTable4-Accent2" => (Some("ED7D31"), Some("FBE4D5"), Some("F4B083")),
        "GridTable4-Accent6" => (Some("70AD47"), Some("E2EFD9"), Some("A8D08D")),
        "GridTable5Dark-Accent1" => (Some("4472C4"), Some("B4C6E7"), Some("FFFFFF")),
        "ListTable3-Accent1" => (Some("4472C4"), None, Some("4472C4")),
        "ListTable4-Accent1" => (Some("4472C4"), Some("D9E2F3"), Some("8EAADB")),
        "ListTable4-Accent2" => (Some("ED7D31"), Some("FBE4D5"), Some("F4B083")),
        _ => (None, None, None),
    };
    (
        header.map(str::to_string),
        band.map(str::to_string),
        border.map(str::to_string),
    )
}

/// What the selection is in a table: the rows and columns it spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSpan {
    pub table_order_index: usize,
    pub rows: (usize, usize),
    pub columns: (usize, usize),
}

/// The レイアウト tab's 選択 menu and the mouse's selections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TablePart {
    Cell,
    Column,
    Row,
    Table,
}

/// セルの挿入's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellShift {
    /// 右方向にシフト / 左方向にシフト.
    Horizontal,
    /// 下方向にシフト / 上方向にシフト.
    Vertical,
    /// 行全体を挿入 / 行全体を削除.
    EntireRow,
    /// 列全体を挿入 / 列全体を削除.
    EntireColumn,
}

/// 自動調整.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoFit {
    /// 文字列の幅に合わせる.
    Contents,
    /// ウィンドウ幅に合わせる.
    Window,
    /// 列の幅を固定する.
    Fixed,
}

/// 罫線's sets, as the ribbon's 罫線 menu lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderSet {
    Bottom,
    Top,
    Left,
    Right,
    None,
    All,
    Outside,
    Inside,
    InsideHorizontal,
    InsideVertical,
}

impl BorderSet {
    pub const ALL: [BorderSet; 10] = [
        BorderSet::Bottom,
        BorderSet::Top,
        BorderSet::Left,
        BorderSet::Right,
        BorderSet::None,
        BorderSet::All,
        BorderSet::Outside,
        BorderSet::Inside,
        BorderSet::InsideHorizontal,
        BorderSet::InsideVertical,
    ];
}

/// A border line: Word's 線種, 太さ and 色.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    /// `single`, `double`, `dashed`, `dotted`, `thick`, `dotDash`, `triple`,
    /// `wave`.
    pub style: String,
    /// Points: ¼, ½, ¾, 1, 1½, 2¼, 3, 4½, 6.
    pub width: f32,
    /// `RRGGBB`, or `auto`.
    pub color: String,
}

impl Default for Pen {
    fn default() -> Self {
        Pen {
            style: "single".to_string(),
            width: 0.5,
            color: "auto".to_string(),
        }
    }
}

impl Pen {
    /// Word's line list (every `ST_Border` line style) and `thick`.
    pub const STYLES: [&'static str; 25] = [
        "single",
        "dotted",
        "dashSmallGap",
        "dashed",
        "dotDash",
        "dotDotDash",
        "double",
        "triple",
        "thick",
        "thinThickSmallGap",
        "thickThinSmallGap",
        "thinThickThinSmallGap",
        "thinThickMediumGap",
        "thickThinMediumGap",
        "thinThickThinMediumGap",
        "thinThickLargeGap",
        "thickThinLargeGap",
        "thinThickThinLargeGap",
        "wave",
        "doubleWave",
        "dashDotStroked",
        "threeDEmboss",
        "threeDEngrave",
        "outset",
        "inset",
    ];
    pub const WIDTHS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0];

    fn edge(&self) -> HeadlessBorderEdge {
        HeadlessBorderEdge {
            style: Some(self.style.clone()),
            width: Some(self.width),
            color: Some(if self.color.eq_ignore_ascii_case("auto") {
                "auto".to_string()
            } else {
                format!("#{}", self.color.trim_start_matches('#'))
            }),
        }
    }
}

fn no_edge() -> HeadlessBorderEdge {
    HeadlessBorderEdge {
        style: Some("none".to_string()),
        width: None,
        color: None,
    }
}

/// 並べ替え: up to three keys, and whether the first row is a title row.
#[derive(Clone, Debug, PartialEq)]
pub struct SortSpec {
    pub keys: Vec<HeadlessTableSortKey>,
    pub header_row: bool,
}

/// 文字列を表にする / 表の解除's 文字列の区切り.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextSeparator {
    Paragraph,
    Tab,
    Comma,
    Other(char),
}

impl TextSeparator {
    fn as_command(&self) -> String {
        match self {
            TextSeparator::Paragraph => "paragraph".to_string(),
            TextSeparator::Tab => "tab".to_string(),
            TextSeparator::Comma => "comma".to_string(),
            TextSeparator::Other(ch) => ch.to_string(),
        }
    }

    fn split(&self, text: &str) -> usize {
        match self {
            TextSeparator::Paragraph => 1,
            TextSeparator::Tab => text.matches('\t').count() + 1,
            TextSeparator::Comma => text.matches(',').count() + 1,
            TextSeparator::Other(ch) => text.matches(*ch).count() + 1,
        }
    }
}

// ---- 表のプロパティ ---------------------------------------------------------

/// A preferred width: 指定なし, a length, or a share of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreferredWidth {
    Points(f32),
    Percent(f32),
}

/// What the caret's table, row, column and cell are, as 表のプロパティ reads
/// them.
#[derive(Clone, Debug, PartialEq)]
pub struct TableProperties {
    pub cell: TableCell,
    // 表
    pub width: Option<PreferredWidth>,
    /// `left`, `center`, `right`.
    pub alignment: String,
    /// 左端からのインデント, points.
    pub indent: f32,
    /// 文字列の折り返し: する.
    pub floating: bool,
    pub position: TablePosition,
    /// 既定のセルの余白: top, bottom, left, right, points.
    pub margins: [f32; 4],
    /// セルの間隔, points as Word's dialog shows it.
    pub spacing: Option<f32>,
    /// 自動的にセルのサイズを変更する.
    pub autofit: bool,
    // 行
    pub row_height: Option<f32>,
    pub row_exact: bool,
    pub allow_break: bool,
    pub repeat_header: bool,
    // 列
    pub column_width: Option<f32>,
    // セル
    /// `top`, `center`, `bottom`.
    pub vertical_align: String,
    /// The cell's own margins, when it has any.
    pub cell_margins: Option<[f32; 4]>,
    /// `lrTb`, `tbRlV` (縦書き), `tbRl`, `btLr`.
    pub text_direction: String,
}

/// 表の位置.
#[derive(Clone, Debug, PartialEq)]
pub struct TablePosition {
    /// `left`, `center`, `right`, `inside`, `outside`, or `None` for an offset.
    pub horizontal_align: Option<String>,
    pub horizontal: f32,
    /// `margin`, `page`, `text` (列).
    pub horizontal_anchor: String,
    pub vertical_align: Option<String>,
    pub vertical: f32,
    /// `margin`, `page`, `text` (段落).
    pub vertical_anchor: String,
    /// 文字列との間隔: top, bottom, left, right.
    pub distances: [f32; 4],
    /// 重ねて配置する.
    pub allow_overlap: bool,
}

impl Default for TablePosition {
    fn default() -> Self {
        TablePosition {
            horizontal_align: Some("left".to_string()),
            horizontal: 0.0,
            horizontal_anchor: "text".to_string(),
            vertical_align: None,
            vertical: 0.0,
            vertical_anchor: "text".to_string(),
            distances: [0.0, 0.0, 9.0, 9.0],
            allow_overlap: true,
        }
    }
}

impl TableProperties {
    fn read(table: &Table, cell: TableCell) -> Self {
        let style = |key: &str| table.style.get(key).map(String::as_str);
        let width = style("width").and_then(|value| {
            percent(value)
                .map(PreferredWidth::Percent)
                .or_else(|| points(value).map(PreferredWidth::Points))
        });
        let margins = table.effective_cell_margins();
        let margin = |edge: Option<f32>, fallback: f32| edge.unwrap_or(fallback);
        let margins = [
            margin(margins.and_then(|m| m.top), 0.0),
            margin(margins.and_then(|m| m.bottom), 0.0),
            margin(
                margins.and_then(|m| m.physical_left(table.bidi_visual)),
                5.4,
            ),
            margin(
                margins.and_then(|m| m.physical_right(table.bidi_visual)),
                5.4,
            ),
        ];
        let row = table.rows.get(cell.row);
        let cell_model = row.and_then(|row| {
            let mut start = row.grid_before.unwrap_or(0) as usize;
            row.cells.iter().find(|candidate| {
                let span = candidate.span.unwrap_or(1).max(1) as usize;
                let found = (start..start + span).contains(&cell.column);
                start += span;
                found
            })
        });
        let floating = style("table-position") == Some("floating");
        let distance = |key: &str, fallback: f32| style(key).and_then(points).unwrap_or(fallback);
        let position = if floating {
            TablePosition {
                horizontal_align: style("table-position-x-align").map(str::to_string),
                horizontal: style("table-position-x").and_then(points).unwrap_or(0.0),
                horizontal_anchor: style("table-position-horizontal-anchor")
                    .unwrap_or("text")
                    .to_string(),
                vertical_align: style("table-position-y-align")
                    .filter(|align| *align != "inline")
                    .map(str::to_string),
                vertical: style("table-position-y").and_then(points).unwrap_or(0.0),
                vertical_anchor: style("table-position-vertical-anchor")
                    .unwrap_or("text")
                    .to_string(),
                distances: [
                    distance("table-distance-top", 0.0),
                    distance("table-distance-bottom", 0.0),
                    distance("table-distance-left", 9.0),
                    distance("table-distance-right", 9.0),
                ],
                allow_overlap: style("table-overlap") != Some("never"),
            }
        } else {
            TablePosition::default()
        };
        TableProperties {
            cell,
            width,
            alignment: style("text-align").unwrap_or("left").to_string(),
            indent: style("margin-left").and_then(points).unwrap_or(0.0),
            floating,
            position,
            margins,
            // Word stores half the spacing its dialog shows.
            spacing: table.cell_spacing.map(|value| value * 2.0),
            autofit: table.table_layout != Some(TableLayoutMode::Fixed),
            row_height: row
                .and_then(|row| row.height)
                .filter(|_| row.and_then(|row| row.height_rule) != Some(TableRowHeightRule::Auto)),
            row_exact: row.and_then(|row| row.height_rule) == Some(TableRowHeightRule::Exact),
            allow_break: !row.is_some_and(|row| row.cant_split),
            repeat_header: row.is_some_and(|row| row.is_header),
            column_width: table
                .columns
                .get(cell.column)
                .and_then(|column| column.width),
            vertical_align: cell_model
                .and_then(|cell| cell.vertical_align.clone())
                .unwrap_or_else(|| "top".to_string()),
            cell_margins: cell_model
                .and_then(|cell| cell.margins.as_ref())
                .map(|own| {
                    [
                        own.top.unwrap_or(margins[0]),
                        own.bottom.unwrap_or(margins[1]),
                        own.physical_left(table.bidi_visual).unwrap_or(margins[2]),
                        own.physical_right(table.bidi_visual).unwrap_or(margins[3]),
                    ]
                }),
            text_direction: cell_model
                .and_then(|cell| cell.text_direction)
                .map(|direction| match direction {
                    crate::document::TextDirection::TbRlV => "tbRlV",
                    crate::document::TextDirection::TbRl => "tbRl",
                    crate::document::TextDirection::BtLr
                    | crate::document::TextDirection::BtLrV => "btLr",
                    _ => "lrTb",
                })
                .unwrap_or("lrTb")
                .to_string(),
        }
    }
}

/// 表のプロパティ's four tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertiesTab {
    Table,
    Row,
    Column,
    Cell,
}

impl PropertiesTab {
    pub const ALL: [PropertiesTab; 4] = [
        PropertiesTab::Table,
        PropertiesTab::Row,
        PropertiesTab::Column,
        PropertiesTab::Cell,
    ];
}

/// A number box of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    TableWidth,
    Indent,
    RowHeight,
    ColumnWidth,
    CellWidth,
    MarginTop,
    MarginBottom,
    MarginLeft,
    MarginRight,
    Spacing,
    CellMarginTop,
    CellMarginBottom,
    CellMarginLeft,
    CellMarginRight,
    PositionHorizontal,
    PositionVertical,
    DistanceTop,
    DistanceBottom,
    DistanceLeft,
    DistanceRight,
}

/// Lengths are typed in millimetres, as Japanese Word shows them.
fn mm(points: f32) -> String {
    let value = (points * MM_PER_PT * 10.0).round() / 10.0;
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn from_mm(text: &str) -> Option<f32> {
    let value: f32 = text
        .trim()
        .trim_end_matches("mm")
        .trim()
        .replace('，', ".")
        .parse()
        .ok()?;
    value.is_finite().then_some(value / MM_PER_PT)
}

/// The dialog's state while it is open. OK turns what changed into commands.
#[derive(Clone, Debug, PartialEq)]
pub struct TablePropertiesDialog {
    pub tab: PropertiesTab,
    pub original: TableProperties,
    /// The box being typed into.
    pub focus: Option<Field>,
    pub text: std::collections::HashMap<Field, String>,
    // 表
    pub width_on: bool,
    pub width_percent: bool,
    pub alignment: String,
    pub floating: bool,
    pub position_open: bool,
    pub position_x_align: Option<String>,
    pub horizontal_anchor: String,
    pub position_y_align: Option<String>,
    pub vertical_anchor: String,
    pub allow_overlap: bool,
    pub options_open: bool,
    pub spacing_on: bool,
    pub autofit: bool,
    // 行
    pub row_height_on: bool,
    pub row_exact: bool,
    pub allow_break: bool,
    pub repeat_header: bool,
    // 列
    pub column_width_on: bool,
    // セル
    pub cell_width_on: bool,
    pub vertical_align: String,
    pub cell_options_open: bool,
    pub cell_margins_same: bool,
}

impl TablePropertiesDialog {
    pub fn new(properties: TableProperties, tab: PropertiesTab) -> Self {
        let mut text = std::collections::HashMap::new();
        let mut put = |field, value: String| {
            text.insert(field, value);
        };
        match properties.width {
            Some(PreferredWidth::Percent(value)) => put(Field::TableWidth, format!("{value:.0}")),
            Some(PreferredWidth::Points(value)) => put(Field::TableWidth, mm(value)),
            None => put(Field::TableWidth, "0".to_string()),
        }
        put(Field::Indent, mm(properties.indent));
        put(Field::RowHeight, mm(properties.row_height.unwrap_or(0.0)));
        put(
            Field::ColumnWidth,
            mm(properties.column_width.unwrap_or(0.0)),
        );
        put(Field::CellWidth, mm(properties.column_width.unwrap_or(0.0)));
        for (field, value) in [
            Field::MarginTop,
            Field::MarginBottom,
            Field::MarginLeft,
            Field::MarginRight,
        ]
        .into_iter()
        .zip(properties.margins)
        {
            put(field, mm(value));
        }
        put(Field::Spacing, mm(properties.spacing.unwrap_or(0.0)));
        let cell = properties.cell_margins.unwrap_or(properties.margins);
        for (field, value) in [
            Field::CellMarginTop,
            Field::CellMarginBottom,
            Field::CellMarginLeft,
            Field::CellMarginRight,
        ]
        .into_iter()
        .zip(cell)
        {
            put(field, mm(value));
        }
        let position = &properties.position;
        put(Field::PositionHorizontal, mm(position.horizontal));
        put(Field::PositionVertical, mm(position.vertical));
        for (field, value) in [
            Field::DistanceTop,
            Field::DistanceBottom,
            Field::DistanceLeft,
            Field::DistanceRight,
        ]
        .into_iter()
        .zip(position.distances)
        {
            put(field, mm(value));
        }
        TablePropertiesDialog {
            tab,
            focus: None,
            text,
            width_on: properties.width.is_some(),
            width_percent: matches!(properties.width, Some(PreferredWidth::Percent(_))),
            alignment: properties.alignment.clone(),
            floating: properties.floating,
            position_open: false,
            position_x_align: position.horizontal_align.clone(),
            horizontal_anchor: position.horizontal_anchor.clone(),
            position_y_align: position.vertical_align.clone(),
            vertical_anchor: position.vertical_anchor.clone(),
            allow_overlap: position.allow_overlap,
            options_open: false,
            spacing_on: properties.spacing.is_some(),
            autofit: properties.autofit,
            row_height_on: properties.row_height.is_some(),
            row_exact: properties.row_exact,
            allow_break: properties.allow_break,
            repeat_header: properties.repeat_header,
            column_width_on: properties.column_width.is_some(),
            cell_width_on: properties.column_width.is_some(),
            vertical_align: properties.vertical_align.clone(),
            cell_options_open: false,
            cell_margins_same: properties.cell_margins.is_none(),
            original: properties,
        }
    }

    /// The text of a box.
    pub fn value(&self, field: Field) -> &str {
        self.text.get(&field).map(String::as_str).unwrap_or("")
    }

    /// Typing into the focused box: digits, a point, Backspace.
    pub fn type_text(&mut self, typed: &str) {
        let Some(field) = self.focus else {
            return;
        };
        let entry = self.text.entry(field).or_default();
        for ch in typed.chars() {
            let ch = match ch {
                '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch),
                '．' => '.',
                other => other,
            };
            if ch.is_ascii_digit()
                || (ch == '.' && !entry.contains('.'))
                || (ch == '-' && entry.is_empty())
            {
                entry.push(ch);
            }
        }
        self.tick(field);
    }

    pub fn backspace(&mut self) {
        if let Some(field) = self.focus {
            self.text.entry(field).or_default().pop();
        }
    }

    /// Steps a box by `delta` millimetres (or percent), as its arrows do.
    pub fn step(&mut self, field: Field, delta: f32) {
        let current: f32 = self.value(field).parse().unwrap_or(0.0);
        let next = ((current + delta) * 10.0).round() / 10.0;
        let next = if matches!(field, Field::PositionHorizontal | Field::PositionVertical) {
            next
        } else {
            next.max(0.0)
        };
        self.text.insert(
            field,
            if next.fract() == 0.0 {
                format!("{next:.0}")
            } else {
                format!("{next:.1}")
            },
        );
        self.tick(field);
    }

    /// A value typed into a box turns its checkbox on, as in Word.
    fn tick(&mut self, field: Field) {
        match field {
            Field::TableWidth => self.width_on = true,
            Field::RowHeight => self.row_height_on = true,
            Field::ColumnWidth => self.column_width_on = true,
            Field::CellWidth => self.cell_width_on = true,
            Field::Spacing => self.spacing_on = true,
            Field::CellMarginTop
            | Field::CellMarginBottom
            | Field::CellMarginLeft
            | Field::CellMarginRight => self.cell_margins_same = false,
            Field::PositionHorizontal => self.position_x_align = None,
            Field::PositionVertical => self.position_y_align = None,
            _ => {}
        }
    }

    fn length(&self, field: Field) -> Option<f32> {
        from_mm(self.value(field))
    }

    /// The commands OK applies, for `cells` — the selection's rows and
    /// columns — in the order Word applies them.
    /// `column_widths` are the columns as drawn, in points, and
    /// `content_width` the text column a percentage is a share of.
    pub fn commands(
        &self,
        span: CellSpan,
        column_count: usize,
        column_widths: &[f32],
        content_width: f32,
    ) -> Vec<HeadlessCommand> {
        let original = &self.original;
        let table_order_index = span.table_order_index;
        let mut commands = Vec::new();
        // The boxes hold tenths of a millimetre, about 0.28 points.
        let changed = |a: f32, b: f32| (a - b).abs() > 0.3;

        // 表: AutoFit first — 文字列の幅に合わせる drops a preferred width.
        if self.autofit != original.autofit {
            commands.push(HeadlessCommand::SetTableFit {
                table_order_index,
                mode: if self.autofit {
                    HeadlessTableFitMode::Contents
                } else {
                    HeadlessTableFitMode::Fixed
                },
            });
        }
        let width = if self.width_on {
            let value: f32 = self.value(Field::TableWidth).parse().unwrap_or(0.0);
            if self.width_percent {
                (value > 0.0).then_some(PreferredWidth::Percent(value.min(600.0)))
            } else {
                self.length(Field::TableWidth)
                    .filter(|value| *value > 0.0)
                    .map(PreferredWidth::Points)
            }
        } else {
            None
        };
        let width_changed = match (width, original.width) {
            (Some(PreferredWidth::Points(a)), Some(PreferredWidth::Points(b))) => changed(a, b),
            (Some(PreferredWidth::Percent(a)), Some(PreferredWidth::Percent(b))) => changed(a, b),
            (None, None) => false,
            _ => true,
        };
        // The columns as they will be: a new preferred width scales them, as
        // Word's dialog does — a fixed grid would otherwise keep its width.
        let mut grid: Vec<f32> = column_widths.to_vec();
        let mut grid_changed = false;
        if width_changed || (self.autofit != original.autofit && width.is_some()) {
            commands.push(HeadlessCommand::SetTableWidth {
                table_order_index,
                width: match width {
                    Some(PreferredWidth::Points(value)) => format!("{value:.2}pt"),
                    Some(PreferredWidth::Percent(value)) => format!("{value:.2}%"),
                    None => "auto".to_string(),
                },
            });
            let target = match width {
                Some(PreferredWidth::Points(value)) => Some(value),
                Some(PreferredWidth::Percent(value)) => Some(content_width * value / 100.0),
                None => None,
            };
            let total: f32 = grid.iter().sum();
            if let Some(target) = target.filter(|target| *target > 0.0 && total > 0.0) {
                let scale = target / total;
                for width in &mut grid {
                    *width = (*width * scale).max(9.0);
                }
                grid_changed = column_widths.len() == column_count;
            }
        }
        if self.alignment != original.alignment {
            commands.push(HeadlessCommand::SetTableAlignment {
                table_order_index,
                alignment: self.alignment.clone(),
            });
        }
        if let Some(indent) = self.length(Field::Indent) {
            if changed(indent, original.indent) {
                commands.push(HeadlessCommand::SetTableIndent {
                    table_order_index,
                    indent,
                });
            }
        }
        let position = TablePosition {
            horizontal_align: self.position_x_align.clone(),
            horizontal: self.length(Field::PositionHorizontal).unwrap_or(0.0),
            horizontal_anchor: self.horizontal_anchor.clone(),
            vertical_align: self.position_y_align.clone(),
            vertical: self.length(Field::PositionVertical).unwrap_or(0.0),
            vertical_anchor: self.vertical_anchor.clone(),
            distances: [
                self.length(Field::DistanceTop).unwrap_or(0.0),
                self.length(Field::DistanceBottom).unwrap_or(0.0),
                self.length(Field::DistanceLeft).unwrap_or(0.0),
                self.length(Field::DistanceRight).unwrap_or(0.0),
            ],
            allow_overlap: self.allow_overlap,
        };
        if !self.floating && original.floating {
            commands.push(HeadlessCommand::SetTableTextWrapping {
                table_order_index,
                wrapping: HeadlessTableTextWrapping::None,
            });
        } else if self.floating && (!original.floating || position != original.position) {
            commands.push(HeadlessCommand::SetTablePosition {
                table_order_index,
                horizontal_align: position.horizontal_align.clone(),
                horizontal_offset: Some(position.horizontal),
                horizontal_anchor: position.horizontal_anchor.clone(),
                vertical_align: position.vertical_align.clone(),
                vertical_offset: Some(position.vertical),
                vertical_anchor: position.vertical_anchor.clone(),
                distance_top: position.distances[0],
                distance_bottom: position.distances[1],
                distance_left: position.distances[2],
                distance_right: position.distances[3],
                allow_overlap: position.allow_overlap,
            });
        }
        let margins = [
            Field::MarginTop,
            Field::MarginBottom,
            Field::MarginLeft,
            Field::MarginRight,
        ]
        .map(|field| self.length(field));
        if margins
            .iter()
            .zip(original.margins)
            .any(|(new, old)| new.is_some_and(|new| changed(new, old)))
        {
            commands.push(HeadlessCommand::SetTableCellMargins {
                table_order_index,
                row: None,
                column: None,
                top: margins[0],
                bottom: margins[1],
                left: margins[2],
                right: margins[3],
            });
        }
        let spacing = self
            .spacing_on
            .then(|| self.length(Field::Spacing))
            .flatten()
            .filter(|value| *value > 0.0);
        let spacing_changed = match (spacing, original.spacing) {
            (Some(a), Some(b)) => changed(a, b),
            (None, None) => false,
            _ => true,
        };
        if spacing_changed {
            commands.push(HeadlessCommand::SetTableCellSpacing {
                table_order_index,
                // Word stores half of what its dialog shows.
                spacing: spacing.map(|value| value / 2.0),
            });
        }

        // 行: every selected row.
        let height = self
            .row_height_on
            .then(|| self.length(Field::RowHeight))
            .flatten()
            .filter(|value| *value > 0.0);
        let height_changed = match (height, original.row_height) {
            (Some(a), Some(b)) => changed(a, b) || self.row_exact != original.row_exact,
            (None, None) => false,
            _ => true,
        };
        for row in span.rows.0..=span.rows.1 {
            if height_changed {
                commands.push(HeadlessCommand::SetTableRowHeightProperties {
                    table_order_index,
                    row,
                    mode: match height {
                        None => "auto".to_string(),
                        Some(_) if self.row_exact => "exact".to_string(),
                        Some(_) => "atLeast".to_string(),
                    },
                    height,
                });
            }
            if self.allow_break != original.allow_break
                || self.repeat_header != original.repeat_header
            {
                commands.push(HeadlessCommand::SetTableRowProperties {
                    table_order_index,
                    row,
                    repeat_header: (self.repeat_header != original.repeat_header)
                        .then_some(self.repeat_header),
                    cant_split: (self.allow_break != original.allow_break)
                        .then_some(!self.allow_break),
                });
            }
        }

        // 列 / セル: the selected columns' widths.
        let column_width = if self.tab == PropertiesTab::Cell {
            self.cell_width_on
                .then(|| self.length(Field::CellWidth))
                .flatten()
        } else {
            self.column_width_on
                .then(|| self.length(Field::ColumnWidth))
                .flatten()
        }
        .filter(|value| *value > 0.0);
        if let Some(width) = column_width {
            if original.column_width.is_none_or(|old| changed(width, old))
                && column_widths.len() == column_count
            {
                let last = span.columns.1.min(column_count.saturating_sub(1));
                for slot in grid.iter_mut().take(last + 1).skip(span.columns.0) {
                    *slot = width;
                }
                grid_changed = true;
            }
        }
        if grid_changed {
            commands.push(HeadlessCommand::SetTableColumnWidths {
                table_order_index,
                widths: grid,
            });
        }

        // セル: alignment and margins of every selected cell.
        let cell_margins = [
            Field::CellMarginTop,
            Field::CellMarginBottom,
            Field::CellMarginLeft,
            Field::CellMarginRight,
        ]
        .map(|field| self.length(field));
        let own_margins_changed = !self.cell_margins_same
            && match original.cell_margins {
                Some(old) => cell_margins
                    .iter()
                    .zip(old)
                    .any(|(new, old)| new.is_some_and(|new| changed(new, old))),
                None => true,
            };
        for row in span.rows.0..=span.rows.1 {
            for column in span.columns.0..=span.columns.1 {
                if self.vertical_align != original.vertical_align {
                    commands.push(HeadlessCommand::SetTableCellVerticalAlignment {
                        table_order_index,
                        row,
                        column,
                        alignment: self.vertical_align.clone(),
                    });
                }
                if self.cell_margins_same && original.cell_margins.is_some() {
                    commands.push(HeadlessCommand::ResetTableCellMargins {
                        table_order_index,
                        row,
                        column,
                    });
                } else if own_margins_changed {
                    commands.push(HeadlessCommand::SetTableCellMargins {
                        table_order_index,
                        row: Some(row),
                        column: Some(column),
                        top: cell_margins[0],
                        bottom: cell_margins[1],
                        left: cell_margins[2],
                        right: cell_margins[3],
                    });
                }
            }
        }
        commands
    }
}

// ---- 線種とページ罫線と網かけ ------------------------------------------------

/// What 罫線と網かけ applies to: 設定対象.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderTarget {
    Table,
    Cell,
}

/// The Borders and Shading dialog: a pen, the edges it is on, a fill.
#[derive(Clone, Debug, PartialEq)]
pub struct BordersDialog {
    pub pen: Pen,
    /// top, bottom, left, right, inside horizontal, inside vertical.
    pub edges: [bool; 6],
    pub target: BorderTarget,
    /// The fill: `None` leaves it, `Some(None)` clears it.
    pub fill: Option<Option<String>>,
}

impl BordersDialog {
    pub const EDGE_NAMES: [&'static str; 6] =
        ["top", "bottom", "left", "right", "insideH", "insideV"];

    pub fn new(target: BorderTarget) -> Self {
        BordersDialog {
            pen: Pen::default(),
            edges: [true; 6],
            target,
            fill: None,
        }
    }

    /// 囲む / 格子 / 指定しない: the dialog's presets.
    pub fn preset(&mut self, set: BorderSet) {
        self.edges = match set {
            BorderSet::None => [false; 6],
            BorderSet::Outside => [true, true, true, true, false, false],
            _ => [true; 6],
        };
    }

    pub fn commands(&self, span: CellSpan) -> Vec<HeadlessCommand> {
        let edge = |on: bool| Some(if on { self.pen.edge() } else { no_edge() });
        let [top, bottom, left, right, inside_h, inside_v] = self.edges;
        let table_order_index = span.table_order_index;
        let mut commands = Vec::new();
        match self.target {
            BorderTarget::Table => commands.push(HeadlessCommand::SetTableBorders {
                table_order_index,
                top: edge(top),
                bottom: edge(bottom),
                left: edge(left),
                right: edge(right),
                inside_h: edge(inside_h),
                inside_v: edge(inside_v),
            }),
            BorderTarget::Cell => {
                // The selection's outline and the lines inside it.
                for row in span.rows.0..=span.rows.1 {
                    for column in span.columns.0..=span.columns.1 {
                        let top = if row == span.rows.0 { top } else { inside_h };
                        let bottom = if row == span.rows.1 { bottom } else { inside_h };
                        let left = if column == span.columns.0 {
                            left
                        } else {
                            inside_v
                        };
                        let right = if column == span.columns.1 {
                            right
                        } else {
                            inside_v
                        };
                        commands.push(HeadlessCommand::SetTableCellBorders {
                            table_order_index,
                            row,
                            column,
                            top: edge(top),
                            bottom: edge(bottom),
                            left: edge(left),
                            right: edge(right),
                        });
                    }
                }
            }
        }
        if let Some(fill) = &self.fill {
            for row in span.rows.0..=span.rows.1 {
                for column in span.columns.0..=span.columns.1 {
                    commands.push(HeadlessCommand::SetTableCellShading {
                        table_order_index,
                        row,
                        column,
                        color: fill
                            .as_ref()
                            .map(|color| format!("#{}", color.trim_start_matches('#'))),
                    });
                }
            }
        }
        commands
    }
}

// ---- the mouse ---------------------------------------------------------------

/// What the pointer is over in a table, in layout pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TableHit {
    /// The line after grid column `boundary - 1`: a drag resizes the columns
    /// on either side, or the last one at the right edge.
    ColumnBorder {
        table_id: usize,
        boundary: usize,
        x: f64,
    },
    /// The line under `row`: a drag sets the row's height.
    RowBorder { table_id: usize, row: usize, y: f64 },
    /// Left of a row: a click selects the row.
    RowSelector { table_id: usize, row: usize },
    /// Over the top of a column: a click selects the column.
    ColumnSelector { table_id: usize, column: usize },
    /// The left edge inside a cell: a click selects the cell.
    CellSelector { cell_id: usize },
    /// The handle at the table's top-left corner: a click selects the table.
    TableHandle { table_id: usize },
}

impl TableHit {
    /// Whether a drag from here resizes (the pointer shows ↔ or ↕).
    pub fn is_border(&self) -> bool {
        matches!(
            self,
            TableHit::ColumnBorder { .. } | TableHit::RowBorder { .. }
        )
    }
}

/// A table's laid-out grid, from the last paint.
#[derive(Clone, Debug, PartialEq)]
pub struct TableGeometry {
    pub table_id: usize,
    /// The x of each grid line, left to right: one more than the columns.
    pub columns: Vec<f64>,
    /// Each row's top and bottom, by row index (a row split over two pages
    /// keeps its first piece's top and its last piece's bottom).
    pub rows: Vec<(f64, f64)>,
}

impl TableGeometry {
    pub fn left(&self) -> f64 {
        self.columns.first().copied().unwrap_or(0.0)
    }

    pub fn right(&self) -> f64 {
        self.columns.last().copied().unwrap_or(0.0)
    }

    pub fn top(&self) -> f64 {
        self.rows.first().map_or(0.0, |row| row.0)
    }

    /// Each column's rendered width, in points.
    pub fn widths_pt(&self) -> Vec<f32> {
        self.columns
            .windows(2)
            .map(|pair| ((pair[1] - pair[0]).max(0.0) * PT_PER_PX) as f32)
            .collect()
    }
}

fn geometry_of(cells: &[CellLayout], table_id: usize) -> Option<TableGeometry> {
    let cells: Vec<&CellLayout> = cells
        .iter()
        .filter(|cell| cell.table_id == table_id)
        .collect();
    if cells.is_empty() {
        return None;
    }
    let column_count = cells
        .iter()
        .map(|cell| cell.column + cell.column_span.max(1))
        .max()?;
    let mut columns: Vec<Option<f64>> = vec![None; column_count + 1];
    for cell in &cells {
        let end = cell.column + cell.column_span.max(1);
        columns[cell.column].get_or_insert(cell.left);
        columns[end].get_or_insert(cell.right);
    }
    // A line no cell starts or ends at (inside every row's span) is put
    // between its neighbours.
    for index in 0..columns.len() {
        if columns[index].is_none() {
            let before = (0..index).rev().find_map(|i| columns[i].map(|x| (i, x)));
            let after = (index + 1..columns.len()).find_map(|i| columns[i].map(|x| (i, x)));
            if let (Some((i0, x0)), Some((i1, x1))) = (before, after) {
                columns[index] = Some(x0 + (x1 - x0) * (index - i0) as f64 / (i1 - i0) as f64);
            }
        }
    }
    let columns: Vec<f64> = columns.into_iter().collect::<Option<Vec<_>>>()?;
    let row_count = cells.iter().map(|cell| cell.row + 1).max()?;
    let mut rows: Vec<Option<(f64, f64)>> = vec![None; row_count];
    for cell in &cells {
        let entry = rows[cell.row].get_or_insert((cell.top, cell.bottom));
        if cell.column_span <= 1 || entry.1 < cell.bottom {
            // Merged-down cells reach below their row; the row is where its
            // unmerged cells end.
        }
        entry.0 = entry.0.min(cell.top);
        if cell.bottom > entry.1 && cell.bottom - cell.top < 2.0 * (entry.1 - entry.0).max(1.0) {
            entry.1 = entry.1.max(cell.bottom);
        }
    }
    // A row whose cells all continue a merge down is between its neighbours.
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| row.unwrap_or((index as f64, index as f64)))
        .collect();
    Some(TableGeometry {
        table_id,
        columns,
        rows,
    })
}

// ---- the session's table tools -----------------------------------------------

/// What draws a table with the mouse: 罫線を引く's pencil and its 消しゴム.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawTool {
    Pencil,
    Eraser,
}

/// The table tools on every session.
pub trait WordTables: WordSession {
    /// The caret's table's style options.
    fn table_look(&self) -> Option<TableLookState> {
        let cell = self.caret_table_cell()?;
        let table = self
            .word_document()
            .table_by_order(cell.table_order_index)?;
        let look = table.look;
        Some(TableLookState {
            header_row: look.first_row_enabled(),
            total_row: look.last_row_enabled(),
            banded_rows: look.horizontal_banding_enabled(),
            first_column: look.first_column_enabled(),
            last_column: look.last_column_enabled(),
            banded_columns: look.vertical_banding_enabled(),
        })
    }

    /// Turns one 表スタイルのオプション on or off.
    fn set_look_option(&mut self, option: LookOption, on: bool) -> Result<(), String> {
        let cell = self
            .caret_table_cell()
            .ok_or_else(|| "the caret is not in a table".to_string())?;
        let flag = |wanted: LookOption| (wanted == option).then_some(on);
        self.apply_command(
            HeadlessCommand::SetTableLook {
                table_order_index: cell.table_order_index,
                first_row: flag(LookOption::HeaderRow),
                banded_rows: flag(LookOption::BandedRows),
                last_row: flag(LookOption::TotalRow),
                first_column: flag(LookOption::FirstColumn),
                last_column: flag(LookOption::LastColumn),
                banded_columns: flag(LookOption::BandedColumns),
            },
            true,
        )
    }

    /// The gallery: Word's built-in styles, then the document's own.
    fn table_styles(&self) -> Vec<TableStyleChoice> {
        let mut choices: Vec<TableStyleChoice> = BUILTIN_TABLE_STYLES
            .iter()
            .map(|(id, name)| TableStyleChoice {
                id: id.to_string(),
                name: name.to_string(),
                preview: builtin_preview(id),
            })
            .collect();
        for info in self.word_document().table_style_infos() {
            let key = |value: &str| -> String {
                value
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect()
            };
            if choices.iter().any(|choice| {
                key(&choice.name) == key(&info.name) || key(&choice.id) == key(&info.style_id)
            }) {
                continue;
            }
            let display = &info.display;
            choices.push(TableStyleChoice {
                id: info.style_id.clone(),
                name: info.name.clone(),
                preview: (
                    display
                        .first_row
                        .as_ref()
                        .and_then(|row| row.fill.clone())
                        .map(|fill| fill.trim_start_matches('#').to_string()),
                    display
                        .band1_fill
                        .clone()
                        .map(|fill| fill.trim_start_matches('#').to_string()),
                    None,
                ),
            });
        }
        choices
    }

    /// The caret's table's named style, if it has one.
    fn table_style_id(&self) -> Option<String> {
        let cell = self.caret_table_cell()?;
        self.word_document()
            .table_style_id(cell.table_order_index)
            .ok()
            .flatten()
    }

    /// Applies a gallery style; `None` is 表 (標準): no named style.
    fn apply_table_style(&mut self, style_id: Option<&str>) -> Result<(), String> {
        let cell = self
            .caret_table_cell()
            .ok_or_else(|| "the caret is not in a table".to_string())?;
        self.apply_command(
            HeadlessCommand::ApplyTableNamedStyle {
                table_order_index: cell.table_order_index,
                style_id: style_id.map(str::to_string),
            },
            true,
        )
    }

    // ---- selection --------------------------------------------------------

    /// The rows and columns the selection spans in its table, or the caret's
    /// cell.
    fn cell_span(&self) -> Option<CellSpan> {
        let range = self.selection_range()?;
        let caret = self.caret_table_cell()?;
        let anchor = self.cell_id_for_paragraph(range.anchor.paragraph);
        let focus = self.cell_id_for_paragraph(range.focus.paragraph);
        let (Some(anchor), Some(focus)) = (
            anchor.and_then(|id| self.cell_layout(id)),
            focus.and_then(|id| self.cell_layout(id)),
        ) else {
            return Some(CellSpan {
                table_order_index: caret.table_order_index,
                rows: (caret.row, caret.row),
                columns: (caret.column, caret.column),
            });
        };
        if anchor.table_id != focus.table_id {
            return Some(CellSpan {
                table_order_index: caret.table_order_index,
                rows: (caret.row, caret.row),
                columns: (caret.column, caret.column),
            });
        }
        let last = |cell: &CellLayout| cell.column + cell.column_span.max(1) - 1;
        Some(CellSpan {
            table_order_index: caret.table_order_index,
            rows: (anchor.row.min(focus.row), anchor.row.max(focus.row)),
            columns: (
                anchor.column.min(focus.column),
                last(anchor).max(last(focus)),
            ),
        })
    }

    /// The cell ids of a table's corners within a span: the first and last
    /// cell laid out for it.
    fn span_corners(
        &self,
        table_id: usize,
        rows: (usize, usize),
        columns: (usize, usize),
    ) -> Option<(usize, usize)> {
        let inside = |cell: &CellLayout| {
            cell.table_id == table_id
                && (rows.0..=rows.1).contains(&cell.row)
                && cell.column <= columns.1
                && cell.column + cell.column_span.max(1) > columns.0
        };
        let cells: Vec<(usize, &CellLayout)> = self
            .cell_layouts()
            .iter()
            .enumerate()
            .filter(|(_, cell)| inside(cell))
            .collect();
        let first = cells
            .iter()
            .min_by_key(|(_, cell)| (cell.row, cell.column))?
            .0;
        let last = cells
            .iter()
            .max_by_key(|(_, cell)| (cell.row, cell.column))?
            .0;
        Some((first, last))
    }

    /// 選択 ▸ セル / 列 / 行 / 表 for the caret's cell.
    fn select_table_part(&mut self, part: TablePart) -> Result<(), String> {
        let paragraph = self
            .selection_range()
            .map(|range| range.focus.paragraph)
            .ok_or_else(|| "the caret is not in a table".to_string())?;
        let cell = self
            .cell_id_for_paragraph(paragraph)
            .and_then(|id| self.cell_layout(id).cloned())
            .ok_or_else(|| "the caret is not in a table".to_string())?;
        self.select_in_table(cell.table_id, part, cell.row, cell.column)
    }

    /// Selects `part` of a laid-out table around (`row`, `column`).
    fn select_in_table(
        &mut self,
        table_id: usize,
        part: TablePart,
        row: usize,
        column: usize,
    ) -> Result<(), String> {
        let geometry =
            geometry_of(self.cell_layouts(), table_id).ok_or("the table is not laid out")?;
        let rows = geometry.rows.len().saturating_sub(1);
        let columns = geometry.columns.len().saturating_sub(2);
        let (row_span, column_span) = match part {
            TablePart::Cell => ((row, row), (column, column)),
            TablePart::Column => ((0, rows), (column, column)),
            TablePart::Row => ((row, row), (0, columns)),
            TablePart::Table => ((0, rows), (0, columns)),
        };
        let (anchor, focus) = self
            .span_corners(table_id, row_span, column_span)
            .ok_or("no cell there")?;
        self.select_cells(anchor, focus)
    }

    /// The laid-out grid of a table.
    fn table_geometry(&self, table_id: usize) -> Option<TableGeometry> {
        geometry_of(self.cell_layouts(), table_id)
    }

    /// What the pointer at (`x`, `y`) in layout pixels is over.
    fn table_hit(&self, x: f64, y: f64) -> Option<TableHit> {
        // Only the tables whose box, with the room the selectors and the
        // handle take round it, holds the point: a hit test runs on every
        // move of the pointer.
        const REACH: f64 = 26.0;
        let mut boxes: std::collections::BTreeMap<usize, (f64, f64, f64, f64)> =
            std::collections::BTreeMap::new();
        for cell in self.cell_layouts() {
            let entry = boxes.entry(cell.table_id).or_insert((
                cell.left,
                cell.top,
                cell.right,
                cell.bottom,
            ));
            entry.0 = entry.0.min(cell.left);
            entry.1 = entry.1.min(cell.top);
            entry.2 = entry.2.max(cell.right);
            entry.3 = entry.3.max(cell.bottom);
        }
        let tables: Vec<usize> = boxes
            .into_iter()
            .filter(|(_, (left, top, right, bottom))| {
                x >= left - REACH && x <= right + REACH && y >= top - REACH && y <= bottom + REACH
            })
            .map(|(table_id, _)| table_id)
            .collect();
        // The innermost table first: a nested table sits inside its cell.
        let mut best: Option<(f64, TableHit)> = None;
        for table_id in tables {
            let Some(geometry) = geometry_of(self.cell_layouts(), table_id) else {
                continue;
            };
            let area = (geometry.right() - geometry.left())
                * geometry
                    .rows
                    .last()
                    .map_or(0.0, |row| row.1 - geometry.top());
            let mut consider = |hit: TableHit| {
                if best.as_ref().is_none_or(|(size, _)| area < *size) {
                    best = Some((area, hit));
                }
            };
            let (left, right) = (geometry.left(), geometry.right());
            if (x - (left - 14.0)).abs() <= 6.0 && (y - (geometry.top() - 14.0)).abs() <= 6.0 {
                consider(TableHit::TableHandle { table_id });
                continue;
            }
            for (row, &(top, bottom)) in geometry.rows.iter().enumerate() {
                if y < top - BORDER_GRAB_PX || y > bottom + BORDER_GRAB_PX {
                    continue;
                }
                if x >= left - 24.0 && x < left - BORDER_GRAB_PX && y > top && y < bottom {
                    consider(TableHit::RowSelector { table_id, row });
                    continue;
                }
                if x < left - BORDER_GRAB_PX || x > right + BORDER_GRAB_PX {
                    continue;
                }
                if (y - bottom).abs() <= BORDER_GRAB_PX {
                    consider(TableHit::RowBorder {
                        table_id,
                        row,
                        y: bottom,
                    });
                    continue;
                }
                // The lines of this row's own cells.
                let row_cells = self
                    .cell_layouts()
                    .iter()
                    .filter(|cell| cell.table_id == table_id && cell.row == row);
                let mut hit = None;
                for cell in row_cells {
                    let end = cell.column + cell.column_span.max(1);
                    if (x - cell.right).abs() <= BORDER_GRAB_PX {
                        hit = Some(TableHit::ColumnBorder {
                            table_id,
                            boundary: end,
                            x: cell.right,
                        });
                    } else if cell.column > 0 && (x - cell.left).abs() <= BORDER_GRAB_PX {
                        hit = Some(TableHit::ColumnBorder {
                            table_id,
                            boundary: cell.column,
                            x: cell.left,
                        });
                    }
                }
                if let Some(hit) = hit {
                    consider(hit);
                    continue;
                }
                if row == 0 && y <= top + BORDER_GRAB_PX {
                    if let Some(column) = geometry
                        .columns
                        .windows(2)
                        .position(|pair| x > pair[0] && x < pair[1])
                    {
                        consider(TableHit::ColumnSelector { table_id, column });
                        continue;
                    }
                }
                for (id, cell) in self.cell_layouts().iter().enumerate() {
                    if cell.table_id == table_id
                        && cell.row == row
                        && x > cell.left + BORDER_GRAB_PX
                        && x <= cell.left + 9.0
                        && y > cell.top
                        && y < cell.bottom
                    {
                        consider(TableHit::CellSelector { cell_id: id });
                    }
                }
            }
            if y >= geometry.top() - 12.0 && y < geometry.top() - BORDER_GRAB_PX {
                if let Some(column) = geometry
                    .columns
                    .windows(2)
                    .position(|pair| x > pair[0] && x < pair[1])
                {
                    consider(TableHit::ColumnSelector { table_id, column });
                }
            }
        }
        best.map(|(_, hit)| hit)
    }

    /// A click on a selector: what it selects.
    fn click_table_hit(&mut self, hit: TableHit) -> Result<bool, String> {
        match hit {
            TableHit::RowSelector { table_id, row } => {
                self.select_in_table(table_id, TablePart::Row, row, 0)?
            }
            TableHit::ColumnSelector { table_id, column } => {
                self.select_in_table(table_id, TablePart::Column, 0, column)?
            }
            TableHit::CellSelector { cell_id } => {
                let cell = self.cell_layout(cell_id).cloned().ok_or("no such cell")?;
                self.select_in_table(cell.table_id, TablePart::Cell, cell.row, cell.column)?
            }
            TableHit::TableHandle { table_id } => {
                self.select_in_table(table_id, TablePart::Table, 0, 0)?
            }
            TableHit::ColumnBorder { .. } | TableHit::RowBorder { .. } => return Ok(false),
        }
        Ok(true)
    }

    /// A drag across cells selects whole cells, as in Word: from the cell
    /// at `anchor` to the one at `focus` (paragraphs).
    fn select_cells_between(&mut self, anchor: usize, focus: usize) -> Result<bool, String> {
        let (Some(a), Some(f)) = (
            self.cell_id_for_paragraph(anchor),
            self.cell_id_for_paragraph(focus),
        ) else {
            return Ok(false);
        };
        let (Some(anchor_cell), Some(focus_cell)) =
            (self.cell_layout(a).cloned(), self.cell_layout(f).cloned())
        else {
            return Ok(false);
        };
        if anchor_cell.table_id != focus_cell.table_id
            || (anchor_cell.row, anchor_cell.column) == (focus_cell.row, focus_cell.column)
        {
            return Ok(false);
        }
        let rows = (
            anchor_cell.row.min(focus_cell.row),
            anchor_cell.row.max(focus_cell.row),
        );
        let columns = (
            anchor_cell.column.min(focus_cell.column),
            (anchor_cell.column + anchor_cell.column_span.max(1) - 1)
                .max(focus_cell.column + focus_cell.column_span.max(1) - 1),
        );
        let (first, last) = self
            .span_corners(anchor_cell.table_id, rows, columns)
            .ok_or("no cells there")?;
        // The anchor stays where the drag began.
        let (anchor, focus) =
            if (anchor_cell.row, anchor_cell.column) <= (focus_cell.row, focus_cell.column) {
                (first, last)
            } else {
                (last, first)
            };
        self.select_cells(anchor, focus)?;
        Ok(true)
    }

    /// A column border dragged to `x`: the columns either side share the
    /// move, or the last column takes it at the right edge — and the table's
    /// grid takes the widths it is drawn with, as Word's drag does.
    fn drag_column_border(
        &mut self,
        table_id: usize,
        boundary: usize,
        x: f64,
    ) -> Result<(), String> {
        let table_order_index = self.table_order_for(table_id).ok_or("the table is gone")?;
        let geometry =
            geometry_of(self.cell_layouts(), table_id).ok_or("the table is not laid out")?;
        let last = geometry.columns.len() - 1;
        if boundary == 0 || boundary > last {
            return Err("the table's left edge does not resize".to_string());
        }
        let mut lines = geometry.columns.clone();
        let low = lines[boundary - 1] + MIN_COLUMN_PX;
        let high = if boundary == last {
            f64::MAX
        } else {
            lines[boundary + 1] - MIN_COLUMN_PX
        };
        lines[boundary] = x.clamp(low, high.max(low));
        let widths: Vec<f32> = lines
            .windows(2)
            .map(|pair| ((pair[1] - pair[0]) * PT_PER_PX) as f32)
            .collect();
        self.execute_command(
            Box::new(ResizeTableColumnCommand::rendered_columns(
                table_order_index,
                widths,
            )),
            true,
        )
    }

    /// A row border dragged to `y`: the row above takes the new height.
    fn drag_row_border(&mut self, table_id: usize, row: usize, y: f64) -> Result<(), String> {
        let table_order_index = self.table_order_for(table_id).ok_or("the table is gone")?;
        let geometry =
            geometry_of(self.cell_layouts(), table_id).ok_or("the table is not laid out")?;
        let (top, _) = *geometry.rows.get(row).ok_or("no such row")?;
        let height = ((y - top).max(MIN_COLUMN_PX) * PT_PER_PX) as f32;
        self.apply_command(
            HeadlessCommand::ResizeTableRowBottomEdge {
                table_order_index,
                row_index: row,
                new_height: height,
            },
            true,
        )
    }

    // ---- 表のプロパティ ------------------------------------------------------

    /// What the caret's table is, for 表のプロパティ.
    fn table_properties(&self) -> Option<TableProperties> {
        let cell = self.caret_table_cell()?;
        let table = self
            .word_document()
            .table_by_order(cell.table_order_index)?;
        Some(TableProperties::read(table, cell))
    }

    /// 表のプロパティ's OK.
    fn apply_table_properties(&mut self, dialog: &TablePropertiesDialog) -> Result<(), String> {
        let span = self.cell_span().ok_or("the caret is not in a table")?;
        let table = self
            .word_document()
            .table_by_order(span.table_order_index)
            .ok_or("the table is gone")?;
        let column_count = table.columns.len();
        let geometry = self
            .cell_id_for_paragraph(
                self.selection_range()
                    .map_or(0, |range| range.focus.paragraph),
            )
            .and_then(|id| self.cell_layout(id))
            .and_then(|cell| geometry_of(self.cell_layouts(), cell.table_id));
        let widths: Vec<f32> = match geometry {
            Some(geometry) if geometry.columns.len() == column_count + 1 => geometry.widths_pt(),
            _ => table
                .columns
                .iter()
                .map(|column| column.width.unwrap_or(72.0))
                .collect(),
        };
        let setup = super::page::WordPage::page_setup(self);
        let content_width = (setup.width - setup.margins.left - setup.margins.right).max(72.0);
        let commands = dialog.commands(span, column_count, &widths, content_width);
        self.apply_commands(commands, true)
    }

    /// 罫線と網かけ's OK, or a 罫線 menu entry with the ribbon's pen.
    fn apply_borders(&mut self, dialog: &BordersDialog) -> Result<(), String> {
        let span = self.cell_span().ok_or("the caret is not in a table")?;
        self.apply_commands(dialog.commands(span), true)
    }

    /// The ribbon's 罫線 menu: `set` drawn with `pen` on the selected cells
    /// (or, with nothing selected, the caret's cell), as Word does.
    fn apply_border_set(&mut self, set: BorderSet, pen: &Pen) -> Result<(), String> {
        let span = self.cell_span().ok_or("the caret is not in a table")?;
        let mut dialog = BordersDialog::new(BorderTarget::Cell);
        dialog.pen = pen.clone();
        let table_wide = span.rows.0 == span.rows.1 && span.columns.0 == span.columns.1;
        if table_wide
            && matches!(
                set,
                BorderSet::All
                    | BorderSet::None
                    | BorderSet::Outside
                    | BorderSet::Inside
                    | BorderSet::InsideHorizontal
                    | BorderSet::InsideVertical
            )
        {
            dialog.target = BorderTarget::Table;
        }
        let on = |edges: [bool; 6]| edges;
        let (edges, touched): ([bool; 6], [bool; 6]) = match set {
            BorderSet::Bottom => (
                on([false, true, false, false, false, false]),
                [false, true, false, false, false, false],
            ),
            BorderSet::Top => (
                [true, false, false, false, false, false],
                [true, false, false, false, false, false],
            ),
            BorderSet::Left => (
                [false, false, true, false, false, false],
                [false, false, true, false, false, false],
            ),
            BorderSet::Right => (
                [false, false, false, true, false, false],
                [false, false, false, true, false, false],
            ),
            BorderSet::None => ([false; 6], [true; 6]),
            BorderSet::All => ([true; 6], [true; 6]),
            BorderSet::Outside => (
                [true, true, true, true, false, false],
                [true, true, true, true, false, false],
            ),
            BorderSet::Inside => (
                [false, false, false, false, true, true],
                [false, false, false, false, true, true],
            ),
            BorderSet::InsideHorizontal => (
                [false, false, false, false, true, false],
                [false, false, false, false, true, false],
            ),
            BorderSet::InsideVertical => (
                [false, false, false, false, false, true],
                [false, false, false, false, false, true],
            ),
        };
        dialog.edges = edges;
        // Only the edges the entry names change; the others are left out.
        let commands: Vec<HeadlessCommand> = dialog
            .commands(span)
            .into_iter()
            .map(|command| strip_untouched(command, touched))
            .collect();
        self.apply_commands(commands, true)
    }

    /// 網かけ: the selected cells filled, or cleared with `None`.
    fn shade_cells(&mut self, color: Option<&str>) -> Result<(), String> {
        let span = self.cell_span().ok_or("the caret is not in a table")?;
        let mut commands = Vec::new();
        for row in span.rows.0..=span.rows.1 {
            for column in span.columns.0..=span.columns.1 {
                commands.push(HeadlessCommand::SetTableCellShading {
                    table_order_index: span.table_order_index,
                    row,
                    column,
                    color: color.map(|color| format!("#{}", color.trim_start_matches('#'))),
                });
            }
        }
        self.apply_commands(commands, true)
    }

    // ---- rows, columns and cells ---------------------------------------------

    /// セルの挿入 before the caret's cell.
    fn insert_cells(&mut self, shift: CellShift) -> Result<Position, String> {
        let cell = self
            .caret_table_cell()
            .ok_or("the caret is not in a table")?;
        let direction = match shift {
            CellShift::Horizontal => HeadlessTableCellInsertDirection::ShiftRight,
            CellShift::Vertical => HeadlessTableCellInsertDirection::ShiftDown,
            CellShift::EntireRow => return self.table_command(super::TableCommand::InsertRowAbove),
            CellShift::EntireColumn => {
                return self.table_command(super::TableCommand::InsertColumnLeft)
            }
        };
        self.structural_cell_command(HeadlessCommand::InsertTableCell {
            table_order_index: cell.table_order_index,
            row: cell.row,
            column: cell.column,
            direction,
        })
    }

    /// セルの削除 of the caret's cell.
    fn delete_cells(&mut self, shift: CellShift) -> Result<Position, String> {
        let cell = self
            .caret_table_cell()
            .ok_or("the caret is not in a table")?;
        let direction = match shift {
            CellShift::Horizontal => HeadlessTableCellDeleteDirection::ShiftLeft,
            CellShift::Vertical => HeadlessTableCellDeleteDirection::ShiftUp,
            CellShift::EntireRow => return self.table_command(super::TableCommand::DeleteRow),
            CellShift::EntireColumn => {
                return self.table_command(super::TableCommand::DeleteColumn)
            }
        };
        self.structural_cell_command(HeadlessCommand::DeleteTableCell {
            table_order_index: cell.table_order_index,
            row: cell.row,
            column: cell.column,
            direction,
        })
    }

    /// Runs a command that renumbers the table's paragraphs, and puts the
    /// caret back in the cell it was in.
    fn structural_cell_command(&mut self, command: HeadlessCommand) -> Result<Position, String> {
        let cell = self
            .caret_table_cell()
            .ok_or("the caret is not in a table")?;
        let caret = self
            .selection_range()
            .map(|range| (range.focus.paragraph, 0))
            .unwrap_or((0, 0));
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(caret.0, 0),
        )));
        self.apply_command(command, false)?;
        let paragraph = self
            .cell_layouts()
            .iter()
            .filter(|layout| self.table_order_for(layout.table_id) == Some(cell.table_order_index))
            .find(|layout| {
                layout.row == cell.row
                    && layout.column <= cell.column
                    && cell.column < layout.column + layout.column_span.max(1)
            })
            .and_then(|layout| layout.paragraphs.first().copied())
            .unwrap_or(caret.0);
        let position = (paragraph, 0);
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(paragraph, 0),
        )));
        Ok(position)
    }

    /// 自動調整.
    fn autofit(&mut self, fit: AutoFit) -> Result<(), String> {
        let cell = self
            .caret_table_cell()
            .ok_or("the caret is not in a table")?;
        self.apply_command(
            HeadlessCommand::SetTableFit {
                table_order_index: cell.table_order_index,
                mode: match fit {
                    AutoFit::Contents => HeadlessTableFitMode::Contents,
                    AutoFit::Window => HeadlessTableFitMode::Window,
                    AutoFit::Fixed => HeadlessTableFitMode::Fixed,
                },
            },
            true,
        )
    }

    /// セルのサイズ's 高さ: the selected rows at least this tall (points).
    fn set_row_height(&mut self, height: f32) -> Result<(), String> {
        let span = self.cell_span().ok_or("the caret is not in a table")?;
        let commands = (span.rows.0..=span.rows.1)
            .map(|row| HeadlessCommand::SetTableRowHeightProperties {
                table_order_index: span.table_order_index,
                row,
                mode: if height > 0.0 { "atLeast" } else { "auto" }.to_string(),
                height: (height > 0.0).then_some(height),
            })
            .collect();
        self.apply_commands(commands, true)
    }

    /// セルのサイズ's 幅: the selected columns this wide (points).
    fn set_column_width(&mut self, width: f32) -> Result<(), String> {
        let span = self.cell_span().ok_or("the caret is not in a table")?;
        let table_id = self
            .selection_range()
            .and_then(|range| self.cell_id_for_paragraph(range.focus.paragraph))
            .and_then(|id| self.cell_layout(id))
            .map(|cell| cell.table_id)
            .ok_or("the caret is not in a table")?;
        let mut widths = self
            .table_geometry(table_id)
            .ok_or("not laid out")?
            .widths_pt();
        for column in span.columns.0..=span.columns.1.min(widths.len().saturating_sub(1)) {
            widths[column] = width.max(9.0);
        }
        self.apply_command(
            HeadlessCommand::SetTableColumnWidths {
                table_order_index: span.table_order_index,
                widths,
            },
            true,
        )
    }

    /// The caret column's laid-out width and row's height, in points, for
    /// the ribbon's セルのサイズ boxes.
    fn cell_size(&self) -> Option<(f32, f32)> {
        let paragraph = self.selection_range()?.focus.paragraph;
        let cell = self.cell_layout(self.cell_id_for_paragraph(paragraph)?)?;
        let geometry = geometry_of(self.cell_layouts(), cell.table_id)?;
        let width = geometry.widths_pt().get(cell.column).copied()?;
        let (top, bottom) = *geometry.rows.get(cell.row)?;
        Some((width, ((bottom - top) * PT_PER_PX) as f32))
    }

    /// 文字列の方向 of the selected cells, as the button steps through
    /// Word's four: 横書き, 縦書き (upright Japanese, `tbRlV`), turned right
    /// (`tbRl`) and turned left (`btLr`).
    fn cycle_text_direction(&mut self) -> Result<(), String> {
        let properties = self
            .table_properties()
            .ok_or("the caret is not in a table")?;
        let next = match properties.text_direction.as_str() {
            "lrTb" => Some("tbRlV"),
            "tbRlV" => Some("tbRl"),
            "tbRl" => Some("btLr"),
            _ => None,
        };
        self.set_text_direction(next)
    }

    /// 文字列の方向 of the selected cells: `tbRl`, `btLr`, or `None` for
    /// horizontal text.
    fn set_text_direction(&mut self, direction: Option<&str>) -> Result<(), String> {
        let span = self.cell_span().ok_or("the caret is not in a table")?;
        let mut commands = Vec::new();
        for row in span.rows.0..=span.rows.1 {
            for column in span.columns.0..=span.columns.1 {
                commands.push(HeadlessCommand::SetTableCellTextDirection {
                    table_order_index: span.table_order_index,
                    row,
                    column,
                    direction: direction.map(str::to_string),
                });
            }
        }
        self.apply_commands(commands, true)
    }

    /// 配置's nine buttons: the paragraphs' alignment and the cells'
    /// vertical alignment of the selection at once.
    fn align_cells(&mut self, horizontal: &str, vertical: &str) -> Result<(), String> {
        let span = self.cell_span().ok_or("the caret is not in a table")?;
        let range = self.selection_range().ok_or("nothing is selected")?;
        let (start, end) = range.normalized();
        let mut commands = vec![HeadlessCommand::SetParagraphAlignment {
            start_paragraph: start.paragraph,
            end_paragraph: end.paragraph,
            alignment: horizontal.to_string(),
        }];
        for row in span.rows.0..=span.rows.1 {
            for column in span.columns.0..=span.columns.1 {
                commands.push(HeadlessCommand::SetTableCellVerticalAlignment {
                    table_order_index: span.table_order_index,
                    row,
                    column,
                    alignment: vertical.to_string(),
                });
            }
        }
        self.apply_commands(commands, true)
    }

    /// The selected cells as merge and split take them.
    fn selected_cell_list(&self) -> Option<Vec<HeadlessTableCellSelection>> {
        let span = self.cell_span()?;
        let table_id = self
            .selection_range()
            .and_then(|range| self.cell_id_for_paragraph(range.focus.paragraph))
            .and_then(|id| self.cell_layout(id))
            .map(|cell| cell.table_id)?;
        let mut cells: Vec<HeadlessTableCellSelection> = self
            .cell_layouts()
            .iter()
            .filter(|cell| {
                cell.table_id == table_id
                    && (span.rows.0..=span.rows.1).contains(&cell.row)
                    && (span.columns.0..=span.columns.1).contains(&cell.column)
            })
            .map(|cell| HeadlessTableCellSelection {
                row: cell.row,
                column: cell.column,
                column_span: cell.column_span.max(1),
            })
            .collect();
        cells.sort_by_key(|cell| (cell.row, cell.column));
        cells.dedup();
        Some(cells)
    }

    // ---- data ------------------------------------------------------------

    /// 並べ替え of the caret's table.
    fn sort_table(&mut self, spec: &SortSpec) -> Result<(), String> {
        let cell = self
            .caret_table_cell()
            .ok_or("the caret is not in a table")?;
        let caret = self.selection_range();
        self.apply_command(
            HeadlessCommand::SortTable {
                table_order_index: cell.table_order_index,
                keys: spec.keys.clone(),
                header_rows: usize::from(spec.header_row),
            },
            false,
        )?;
        self.set_selection_range(caret.map(|range| {
            jade_selection::SelectionRange::collapsed(jade_selection::TextPosition::new(
                range.focus.paragraph,
                0,
            ))
        }));
        Ok(())
    }

    /// The column headings 並べ替え offers: the first row's texts, or
    /// 列 1, 列 2 … when the table has no title row.
    fn sort_columns(&self, header_row: bool) -> Vec<String> {
        let Some(cell) = self.caret_table_cell() else {
            return Vec::new();
        };
        let Some(table) = self.word_document().table_by_order(cell.table_order_index) else {
            return Vec::new();
        };
        (0..cell.columns)
            .map(|column| {
                let heading = header_row
                    .then(|| table.rows.first())
                    .flatten()
                    .and_then(|row| row.cells.get(column))
                    .map(|cell| {
                        cell.blocks
                            .iter()
                            .filter_map(BodyElement::as_paragraph)
                            .flat_map(|paragraph| {
                                paragraph.runs.iter().map(|run| run.text.as_str())
                            })
                            .collect::<String>()
                    })
                    .filter(|text| !text.trim().is_empty());
                heading.unwrap_or_else(|| format!("{}", column + 1))
            })
            .collect()
    }

    /// Word's guess for 計算式: `=SUM(ABOVE)` when the cells above hold
    /// numbers, `=SUM(LEFT)` otherwise.
    fn suggested_formula(&self) -> String {
        let Some(cell) = self.caret_table_cell() else {
            return "=SUM(ABOVE)".to_string();
        };
        let Some(table) = self.word_document().table_by_order(cell.table_order_index) else {
            return "=SUM(ABOVE)".to_string();
        };
        let number_above = cell.row > 0
            && table
                .rows
                .get(cell.row - 1)
                .and_then(|row| row.cells.get(cell.column))
                .is_some_and(|above| {
                    above
                        .blocks
                        .iter()
                        .filter_map(BodyElement::as_paragraph)
                        .flat_map(|paragraph| paragraph.runs.iter())
                        .any(|run| run.text.chars().any(|ch| ch.is_ascii_digit()))
                });
        if number_above || cell.column == 0 {
            "=SUM(ABOVE)".to_string()
        } else {
            "=SUM(LEFT)".to_string()
        }
    }

    /// 計算式's OK: the field at the caret.
    fn insert_formula(&mut self, formula: &str, number_format: Option<&str>) -> Result<(), String> {
        let range = self.selection_range().ok_or("nothing is selected")?;
        let (start, _) = range.normalized();
        self.caret_table_cell()
            .ok_or("a formula goes in a table cell")?;
        self.apply_command(
            HeadlessCommand::InsertTableFormula {
                paragraph: start.paragraph,
                offset: start.offset,
                formula: formula.to_string(),
                number_format: number_format
                    .map(str::trim)
                    .filter(|format| !format.is_empty())
                    .map(str::to_string),
            },
            false,
        )
    }

    /// フィールド更新 of every table formula.
    fn update_formulas(&mut self) -> Result<(), String> {
        self.apply_command(HeadlessCommand::UpdateTableFormulas, true)
    }

    /// 文字列を表にする for the selected paragraphs: the columns and rows
    /// the separator gives, as the dialog previews them.
    fn text_table_shape(&self, separator: &TextSeparator) -> (usize, usize) {
        let Some(range) = self.selection_range() else {
            return (1, 1);
        };
        let (start, end) = range.normalized();
        let paragraphs = start.paragraph..=end.paragraph;
        match separator {
            TextSeparator::Paragraph => (1, paragraphs.count()),
            _ => {
                let columns = paragraphs
                    .clone()
                    .filter_map(|index| self.word_document().paragraph(index))
                    .map(|paragraph| {
                        let text: String =
                            paragraph.runs.iter().map(|run| run.text.as_str()).collect();
                        separator.split(&text)
                    })
                    .max()
                    .unwrap_or(1);
                (columns, paragraphs.count())
            }
        }
    }

    /// 文字列を表にする: the selected paragraphs become a table, and the caret
    /// goes to its first cell.
    fn convert_text_to_table(
        &mut self,
        separator: &TextSeparator,
        columns: Option<usize>,
    ) -> Result<Position, String> {
        let range = self.selection_range().ok_or("select the text to convert")?;
        let (start, end) = range.normalized();
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(start)));
        self.apply_command(
            HeadlessCommand::ConvertTextToTable {
                start_paragraph: start.paragraph,
                end_paragraph: end.paragraph,
                separator: separator.as_command(),
                columns,
            },
            false,
        )?;
        let position = (start.paragraph, 0);
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(position.0, 0),
        )));
        Ok(position)
    }

    /// 表の解除: the caret's table becomes text.
    fn convert_table_to_text(&mut self, separator: &TextSeparator) -> Result<Position, String> {
        let cell = self
            .caret_table_cell()
            .ok_or("the caret is not in a table")?;
        let first = self
            .cell_layouts()
            .iter()
            .filter(|layout| self.table_order_for(layout.table_id) == Some(cell.table_order_index))
            .filter_map(|layout| layout.paragraphs.first().copied())
            .min()
            .unwrap_or(0);
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(first, 0),
        )));
        self.apply_command(
            HeadlessCommand::ConvertTableToText {
                table_order_index: cell.table_order_index,
                separator: separator.as_command(),
            },
            false,
        )?;
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(first, 0),
        )));
        Ok((first, 0))
    }

    // ---- 罫線を引く ------------------------------------------------------------

    /// The pencil's stroke from `from` to `to` (layout pixels). Inside a
    /// cell, a mostly horizontal stroke splits it into two rows and a
    /// vertical one into two columns; outside any table, a rectangle drawn
    /// at `paragraph` becomes a one-cell table that wide.
    fn draw_stroke(
        &mut self,
        from: (f64, f64),
        to: (f64, f64),
        paragraph: Option<Position>,
    ) -> Result<Option<Position>, String> {
        let middle = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
        let inside = self.cell_layouts().iter().enumerate().find(|(_, cell)| {
            middle.0 > cell.left
                && middle.0 < cell.right
                && middle.1 > cell.top
                && middle.1 < cell.bottom
        });
        if let Some((_, cell)) = inside {
            let cell = cell.clone();
            let table_order_index = self
                .table_order_for(cell.table_id)
                .ok_or("the table is gone")?;
            let horizontal = (to.0 - from.0).abs() >= (to.1 - from.1).abs();
            self.apply_command(
                HeadlessCommand::SplitTableCell {
                    table_order_index,
                    row: cell.row,
                    column: cell.column,
                    columns: if horizontal { 1 } else { 2 },
                    rows: if horizontal { 2 } else { 1 },
                },
                false,
            )?;
            return Ok(None);
        }
        let Some((paragraph, offset)) = paragraph else {
            return Ok(None);
        };
        let width = ((to.0 - from.0).abs() * PT_PER_PX) as f32;
        let height = ((to.1 - from.1).abs() * PT_PER_PX) as f32;
        if width < 9.0 {
            return Ok(None);
        }
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(paragraph, offset),
        )));
        // A drawn table has Word's 表 (格子) lines, as one inserted does.
        self.execute_command(
            Box::new(
                crate::commands::insert_table::InsertTableCommand::at_caret(
                    paragraph,
                    offset,
                    1,
                    1,
                    vec![width],
                )
                .with_style("TableGrid"),
            ),
            false,
        )?;
        let first = [paragraph, paragraph + 1]
            .into_iter()
            .find(|candidate| self.cell_id_for_paragraph(*candidate).is_some())
            .unwrap_or(paragraph);
        if height > 12.0 {
            if let Some(cell) = self.caret_cell_at(first) {
                self.apply_command(
                    HeadlessCommand::SetTableRowHeightProperties {
                        table_order_index: cell.table_order_index,
                        row: 0,
                        mode: "atLeast".to_string(),
                        height: Some(height),
                    },
                    true,
                )?;
            }
        }
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(first, 0),
        )));
        Ok(Some((first, 0)))
    }

    /// The table cell paragraph `paragraph` is in.
    fn caret_cell_at(&self, paragraph: usize) -> Option<TableCell> {
        let layout = self.cell_layout(self.cell_id_for_paragraph(paragraph)?)?;
        let table_order_index = self.table_order_for(layout.table_id)?;
        let (rows, columns) = self.word_document().table_dimensions(table_order_index)?;
        Some(TableCell {
            table_order_index,
            row: layout.row,
            column: layout.column,
            rows,
            columns,
            can_merge: false,
            can_unmerge: false,
            repeat_header: false,
        })
    }

    /// The eraser on the border at (`x`, `y`): the two cells it separates
    /// become one.
    fn erase_border(&mut self, x: f64, y: f64) -> Result<bool, String> {
        let Some(hit) = self.table_hit(x, y) else {
            return Ok(false);
        };
        let (table_id, first, second) = match hit {
            TableHit::ColumnBorder {
                table_id, boundary, ..
            } => {
                let row = self
                    .cell_layouts()
                    .iter()
                    .find(|cell| cell.table_id == table_id && y > cell.top && y < cell.bottom)
                    .map(|cell| cell.row)
                    .ok_or("no row there")?;
                let left = self.cell_layouts().iter().find(|cell| {
                    cell.table_id == table_id
                        && cell.row == row
                        && cell.column + cell.column_span.max(1) == boundary
                });
                let right = self.cell_layouts().iter().find(|cell| {
                    cell.table_id == table_id && cell.row == row && cell.column == boundary
                });
                match (left, right) {
                    (Some(left), Some(right)) => (table_id, left.clone(), right.clone()),
                    _ => return Ok(false),
                }
            }
            TableHit::RowBorder { table_id, row, .. } => {
                let above = self.cell_layouts().iter().find(|cell| {
                    cell.table_id == table_id && cell.row == row && x > cell.left && x < cell.right
                });
                let below = self.cell_layouts().iter().find(|cell| {
                    cell.table_id == table_id
                        && cell.row == row + 1
                        && x > cell.left
                        && x < cell.right
                });
                match (above, below) {
                    (Some(above), Some(below)) => (table_id, above.clone(), below.clone()),
                    _ => return Ok(false),
                }
            }
            _ => return Ok(false),
        };
        let table_order_index = self.table_order_for(table_id).ok_or("the table is gone")?;
        let caret = first.paragraphs.first().copied().unwrap_or(0);
        self.set_selection_range(Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(caret, 0),
        )));
        self.apply_command(
            HeadlessCommand::MergeTableCells {
                table_order_index,
                selected_cells: vec![
                    HeadlessTableCellSelection {
                        row: first.row,
                        column: first.column,
                        column_span: first.column_span.max(1),
                    },
                    HeadlessTableCellSelection {
                        row: second.row,
                        column: second.column,
                        column_span: second.column_span.max(1),
                    },
                ],
            },
            false,
        )?;
        Ok(true)
    }
}

/// Leaves out the edges a 罫線 entry does not name, so the others keep
/// their lines.
fn strip_untouched(command: HeadlessCommand, touched: [bool; 6]) -> HeadlessCommand {
    let keep = |on: bool, edge: Option<HeadlessBorderEdge>| edge.filter(|_| on);
    match command {
        HeadlessCommand::SetTableBorders {
            table_order_index,
            top,
            bottom,
            left,
            right,
            inside_h,
            inside_v,
        } => HeadlessCommand::SetTableBorders {
            table_order_index,
            top: keep(touched[0], top),
            bottom: keep(touched[1], bottom),
            left: keep(touched[2], left),
            right: keep(touched[3], right),
            inside_h: keep(touched[4], inside_h),
            inside_v: keep(touched[5], inside_v),
        },
        HeadlessCommand::SetTableCellBorders {
            table_order_index,
            row,
            column,
            top,
            bottom,
            left,
            right,
        } => HeadlessCommand::SetTableCellBorders {
            table_order_index,
            row,
            column,
            // A cell's inner edges stand for the inside lines.
            top: top.filter(|_| touched[0] || touched[4]),
            bottom: bottom.filter(|_| touched[1] || touched[4]),
            left: left.filter(|_| touched[2] || touched[5]),
            right: right.filter(|_| touched[3] || touched[5]),
        },
        other => other,
    }
}

impl<T: WordSession + ?Sized> WordTables for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(
        table_id: usize,
        row: usize,
        column: usize,
        left: f64,
        right: f64,
        top: f64,
        bottom: f64,
    ) -> CellLayout {
        CellLayout {
            paragraphs: vec![row * 10 + column],
            left,
            right,
            top,
            bottom,
            table_id,
            row,
            column,
            column_span: 1,
            order: row * 10 + column,
        }
    }

    #[test]
    fn the_grid_comes_from_the_cells() {
        let cells = vec![
            layout(0, 0, 0, 100.0, 200.0, 50.0, 70.0),
            layout(0, 0, 1, 200.0, 350.0, 50.0, 70.0),
            layout(0, 1, 0, 100.0, 200.0, 70.0, 95.0),
            layout(0, 1, 1, 200.0, 350.0, 70.0, 95.0),
        ];
        let geometry = geometry_of(&cells, 0).unwrap();
        assert_eq!(geometry.columns, vec![100.0, 200.0, 350.0]);
        assert_eq!(geometry.rows, vec![(50.0, 70.0), (70.0, 95.0)]);
        assert_eq!(geometry.widths_pt(), vec![75.0, 112.5]);
    }

    #[test]
    fn a_dialog_sends_only_what_changed() {
        let properties = TableProperties {
            cell: TableCell {
                table_order_index: 0,
                row: 1,
                column: 0,
                rows: 2,
                columns: 2,
                can_merge: false,
                can_unmerge: false,
                repeat_header: false,
            },
            width: None,
            alignment: "left".to_string(),
            indent: 0.0,
            floating: false,
            position: TablePosition::default(),
            margins: [0.0, 0.0, 5.4, 5.4],
            spacing: None,
            autofit: true,
            row_height: None,
            row_exact: false,
            allow_break: true,
            repeat_header: false,
            column_width: Some(100.0),
            vertical_align: "top".to_string(),
            cell_margins: None,
            text_direction: "lrTb".to_string(),
        };
        let span = CellSpan {
            table_order_index: 0,
            rows: (1, 1),
            columns: (0, 0),
        };
        let mut dialog = TablePropertiesDialog::new(properties, PropertiesTab::Row);
        assert!(dialog.commands(span, 2, &[100.0, 100.0], 450.0).is_empty());
        dialog.focus = Some(Field::RowHeight);
        dialog.type_text("10");
        dialog.row_exact = true;
        dialog.allow_break = false;
        dialog.width_on = true;
        dialog.width_percent = true;
        dialog.text.insert(Field::TableWidth, "80".to_string());
        dialog.spacing_on = true;
        dialog.text.insert(Field::Spacing, "1".to_string());
        let commands = dialog.commands(span, 2, &[100.0, 100.0], 450.0);
        assert!(commands.contains(&HeadlessCommand::SetTableWidth {
            table_order_index: 0,
            width: "80.00%".to_string()
        }));
        assert!(commands.iter().any(|command| matches!(command,
            HeadlessCommand::SetTableRowHeightProperties { row: 1, mode, height: Some(height), .. }
                if mode == "exact" && (height - 28.35).abs() < 0.1)));
        assert!(commands.iter().any(|command| matches!(
            command,
            HeadlessCommand::SetTableRowProperties {
                row: 1,
                cant_split: Some(true),
                repeat_header: None,
                ..
            }
        )));
        assert!(commands.iter().any(|command| matches!(command,
            HeadlessCommand::SetTableCellSpacing { spacing: Some(spacing), .. } if (spacing - 1.417).abs() < 0.01)));
    }

    #[test]
    fn border_sets_name_only_their_edges() {
        let span = CellSpan {
            table_order_index: 0,
            rows: (0, 1),
            columns: (0, 1),
        };
        let mut dialog = BordersDialog::new(BorderTarget::Cell);
        dialog.edges = [false, true, false, false, false, false];
        let commands: Vec<_> = dialog
            .commands(span)
            .into_iter()
            .map(|command| strip_untouched(command, [false, true, false, false, false, false]))
            .collect();
        assert_eq!(commands.len(), 4);
        // Only the bottom row's cells get a bottom line; nothing else moves.
        for command in commands {
            let HeadlessCommand::SetTableCellBorders {
                row,
                top,
                bottom,
                left,
                right,
                ..
            } = command
            else {
                panic!("cell borders");
            };
            assert!(top.is_none() && left.is_none() && right.is_none());
            assert_eq!(
                bottom.unwrap().style.as_deref(),
                Some(if row == 1 { "single" } else { "none" })
            );
        }
    }
}
