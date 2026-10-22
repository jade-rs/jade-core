//! What the table tabs, their dialogs and the mouse in a table do, apart
//! from how a shell draws them: both shells keep a [`TableTools`] with the
//! document, hand it each control's [`TableAction`], each key a dialog
//! takes and each press, drag and release over the pages, and draw what it
//! holds. The commands are [`WordTables`]'.

use super::table::{
    AutoFit, BorderSet, BorderTarget, BordersDialog, CellShift, DrawTool, LookOption, Pen,
    PropertiesTab, SortKey, SortKind, SortSpec, TableHit, TableLookState, TablePart,
    TablePropertiesDialog, TableStyleChoice, TextSeparator, WordTables, MM_PER_PT,
};
use super::{Position, TableCommand, WordFormatting};

/// What a table control does.
#[derive(Clone, Debug, PartialEq)]
pub enum TableAction {
    Look(LookOption, bool),
    Style(Option<String>),
    Select(TablePart),
    Properties(PropertiesTab),
    /// セルの配置's 余白: 表のプロパティ's table options.
    CellMargins,
    Borders,
    BorderSet(BorderSet),
    PenStyle(&'static str),
    PenWidth(f32),
    PenColor(&'static str),
    Shading(Option<&'static str>),
    Tool(DrawTool),
    InsertCells,
    DeleteCells,
    SplitCells,
    AutoFit(AutoFit),
    /// セルのサイズ's ▲▼: a millimetre taller or wider, or less.
    StepHeight(bool),
    StepWidth(bool),
    Align(&'static str, &'static str),
    TextDirection(Option<&'static str>),
    Sort,
    Formula,
    UpdateFormulas,
    ToText,
    ToTable,
}

/// A small dialog's boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogBox {
    Columns,
    Rows,
    Other,
}

/// The separator a conversion dialog has chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Separator {
    Paragraph,
    Tab,
    Comma,
    Other,
}

impl Separator {
    /// What the conversion takes, `other` being the その他 character.
    pub fn text_separator(self, other: &str) -> TextSeparator {
        match self {
            Separator::Paragraph => TextSeparator::Paragraph,
            Separator::Tab => TextSeparator::Tab,
            Separator::Comma => TextSeparator::Comma,
            Separator::Other => other
                .chars()
                .next()
                .map(TextSeparator::Other)
                .unwrap_or(TextSeparator::Tab),
        }
    }
}

/// One 並べ替え key as the dialog holds it: a column, its kind, and
/// whether it sorts descending; `None` for (なし).
pub type SortSlot = Option<(usize, SortKind, bool)>;

/// The table dialog that is open.
#[derive(Clone, Debug, PartialEq)]
pub enum TableDialog {
    Properties(Box<TablePropertiesDialog>),
    Borders(BordersDialog),
    Cells {
        delete: bool,
        shift: CellShift,
    },
    Split {
        columns: String,
        rows: String,
        focus: DialogBox,
    },
    ToTable {
        separator: Separator,
        other: String,
        columns: String,
        focus: DialogBox,
    },
    ToText {
        separator: Separator,
        other: String,
    },
    Sort {
        /// Up to three keys: a column, its kind, descending. The first is
        /// always set.
        keys: [SortSlot; 3],
        header: bool,
        headings: Vec<String>,
    },
    Formula {
        formula: String,
        format: &'static str,
    },
}

impl TableDialog {
    /// Typing into the focused box.
    pub fn type_text(&mut self, text: &str) {
        let digits = |target: &mut String| {
            for ch in text.chars() {
                let ch = match ch {
                    '０'..='９' => {
                        char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch)
                    }
                    other => other,
                };
                if ch.is_ascii_digit() {
                    target.push(ch);
                }
            }
        };
        match self {
            TableDialog::Properties(dialog) => dialog.type_text(text),
            TableDialog::Split {
                columns,
                rows,
                focus,
            } => match focus {
                DialogBox::Rows => digits(rows),
                _ => digits(columns),
            },
            TableDialog::ToTable {
                other,
                columns,
                focus,
                separator,
            } => match focus {
                DialogBox::Other => {
                    *other = text.chars().last().map(String::from).unwrap_or_default();
                    *separator = Separator::Other;
                }
                _ => digits(columns),
            },
            TableDialog::ToText { other, separator } => {
                *other = text.chars().last().map(String::from).unwrap_or_default();
                *separator = Separator::Other;
            }
            TableDialog::Formula { formula, .. } => formula.push_str(text),
            TableDialog::Borders(_) | TableDialog::Cells { .. } | TableDialog::Sort { .. } => {}
        }
    }

    pub fn backspace(&mut self) {
        match self {
            TableDialog::Properties(dialog) => dialog.backspace(),
            TableDialog::Split {
                columns,
                rows,
                focus,
            } => {
                match focus {
                    DialogBox::Rows => rows.pop(),
                    _ => columns.pop(),
                };
            }
            TableDialog::ToTable {
                other,
                columns,
                focus,
                ..
            } => {
                match focus {
                    DialogBox::Other => other.pop(),
                    _ => columns.pop(),
                };
            }
            TableDialog::ToText { other, .. } => {
                other.pop();
            }
            TableDialog::Formula { formula, .. } => {
                formula.pop();
            }
            TableDialog::Borders(_) | TableDialog::Cells { .. } | TableDialog::Sort { .. } => {}
        }
    }

    /// The i18n key of the dialog's title.
    pub fn title_key(&self) -> &'static str {
        match self {
            TableDialog::Properties(_) => "table.properties",
            TableDialog::Borders(_) => "table.borders_title",
            TableDialog::Cells { delete: false, .. } => "table.insert_cells_title",
            TableDialog::Cells { delete: true, .. } => "table.delete_cells_title",
            TableDialog::Split { .. } => "table.split_cells_title",
            TableDialog::ToTable { .. } => "table.to_table_title",
            TableDialog::ToText { .. } => "table.to_text_title",
            TableDialog::Sort { .. } => "table.sort_title",
            TableDialog::Formula { .. } => "table.formula_title",
        }
    }
}

/// A drag the mouse began in a table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TableDrag {
    Column {
        table_id: usize,
        boundary: usize,
    },
    Row {
        table_id: usize,
        row: usize,
    },
    /// The pencil, from where it went down.
    Stroke,
}

/// The pointer's shape over the pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableCursor {
    Text,
    Arrow,
    ResizeColumns,
    ResizeRows,
    Pencil,
    Eraser,
}

/// What a table control or a press did, for the shell to show.
#[derive(Clone, Debug, PartialEq)]
pub enum TableOutcome {
    /// Nothing for the shell: the click was not the table's.
    NotMine,
    /// Only the tools' own state changed — a pen, a tool, a dialog opened or
    /// closed, a drag begun or moved: draw again.
    Redraw,
    /// The document changed: the caret goes to the position given, or stays.
    Edited(Result<Option<Position>, String>),
    /// Cells were selected.
    Selected(Result<(), String>),
}

/// What the table tabs show as current: read once per frame they show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableTabState {
    pub look: TableLookState,
    pub styles: Vec<TableStyleChoice>,
    pub style_id: Option<String>,
    pub pen: Pen,
    pub tool: Option<DrawTool>,
    /// The caret column's width and row's height, in points.
    pub cell_size: (f32, f32),
    /// The caret cell's 文字列の方向.
    pub direction: String,
}

/// A drag in a table: what, on which page, from and to where in layout
/// pixels.
pub type DragState = (TableDrag, usize, (f64, f64), (f64, f64));

/// The table tools' state, kept with the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableTools {
    pub dialog: Option<TableDialog>,
    /// The pen 罫線 draws with.
    pub pen: Pen,
    /// 罫線を引く or 消しゴム, while one is on.
    pub tool: Option<DrawTool>,
    /// A border or a pencil stroke being dragged: what, on which page, from
    /// and to where in layout pixels.
    pub drag: Option<DragState>,
    /// What the pointer is over, for its shape.
    pub hover: Option<TableHit>,
}

impl TableTools {
    /// What the table tabs show for `document`.
    pub fn tab_state<S: WordTables + ?Sized>(&self, document: &S) -> TableTabState {
        TableTabState {
            look: document.table_look().unwrap_or_default(),
            styles: document.table_styles(),
            style_id: document.table_style_id(),
            pen: self.pen.clone(),
            tool: self.tool,
            cell_size: document.cell_size().unwrap_or((0.0, 0.0)),
            direction: document
                .table_properties()
                .map(|properties| properties.text_direction)
                .unwrap_or_default(),
        }
    }

    /// The pointer over the pages.
    pub fn cursor(&self) -> TableCursor {
        match (self.tool, self.drag.map(|drag| drag.0), self.hover) {
            (Some(DrawTool::Pencil), ..) => TableCursor::Pencil,
            (Some(DrawTool::Eraser), ..) => TableCursor::Eraser,
            (_, Some(TableDrag::Column { .. }), _)
            | (_, _, Some(TableHit::ColumnBorder { .. })) => TableCursor::ResizeColumns,
            (_, Some(TableDrag::Row { .. }), _) | (_, _, Some(TableHit::RowBorder { .. })) => {
                TableCursor::ResizeRows
            }
            (_, _, Some(_)) => TableCursor::Arrow,
            _ => TableCursor::Text,
        }
    }

    /// A table control was clicked.
    pub fn action<S: WordTables + ?Sized>(
        &mut self,
        document: &mut S,
        action: TableAction,
    ) -> TableOutcome {
        let edited = |result: Result<(), String>| TableOutcome::Edited(result.map(|_| None));
        match action {
            TableAction::Look(option, on) => edited(document.set_look_option(option, on)),
            TableAction::Style(style) => edited(document.apply_table_style(style.as_deref())),
            TableAction::Select(part) => TableOutcome::Selected(document.select_table_part(part)),
            TableAction::Properties(tab) => {
                self.open(document.table_properties().map(|properties| {
                    TableDialog::Properties(Box::new(TablePropertiesDialog::new(properties, tab)))
                }))
            }
            TableAction::CellMargins => self.open(document.table_properties().map(|properties| {
                let mut dialog = TablePropertiesDialog::new(properties, PropertiesTab::Table);
                dialog.options_open = true;
                TableDialog::Properties(Box::new(dialog))
            })),
            TableAction::Borders => {
                let mut dialog = BordersDialog::new(BorderTarget::Table);
                dialog.pen = self.pen.clone();
                self.open(Some(TableDialog::Borders(dialog)))
            }
            TableAction::BorderSet(set) => edited(document.apply_border_set(set, &self.pen)),
            TableAction::PenStyle(style) => {
                self.pen.style = style.to_string();
                TableOutcome::Redraw
            }
            TableAction::PenWidth(width) => {
                self.pen.width = width;
                TableOutcome::Redraw
            }
            TableAction::PenColor(color) => {
                self.pen.color = color.to_string();
                TableOutcome::Redraw
            }
            TableAction::Shading(color) => edited(document.shade_cells(color)),
            TableAction::Tool(tool) => {
                self.tool = (self.tool != Some(tool)).then_some(tool);
                TableOutcome::Redraw
            }
            TableAction::InsertCells => self.open(Some(TableDialog::Cells {
                delete: false,
                shift: CellShift::Vertical,
            })),
            TableAction::DeleteCells => self.open(Some(TableDialog::Cells {
                delete: true,
                shift: CellShift::Horizontal,
            })),
            TableAction::SplitCells => self.open(Some(TableDialog::Split {
                columns: "2".to_string(),
                rows: "1".to_string(),
                focus: DialogBox::Columns,
            })),
            TableAction::AutoFit(fit) => edited(document.autofit(fit)),
            TableAction::StepHeight(grow) => {
                let (_, height) = document.cell_size().unwrap_or((0.0, 0.0));
                let step = 1.0 / MM_PER_PT;
                edited(
                    document.set_row_height((height + if grow { step } else { -step }).max(step)),
                )
            }
            TableAction::StepWidth(grow) => {
                let (width, _) = document.cell_size().unwrap_or((0.0, 0.0));
                let step = 1.0 / MM_PER_PT;
                edited(document.set_column_width(width + if grow { step } else { -step }))
            }
            TableAction::Align(horizontal, vertical) => {
                edited(document.align_cells(horizontal, vertical))
            }
            TableAction::TextDirection(direction) => edited(document.set_text_direction(direction)),
            TableAction::Sort => {
                let header = document
                    .table_properties()
                    .is_some_and(|properties| properties.repeat_header)
                    || document.table_look().is_some_and(|look| look.header_row);
                let headings = document.sort_columns(header);
                self.open(Some(TableDialog::Sort {
                    keys: [Some((0, SortKind::Text, false)), None, None],
                    header,
                    headings,
                }))
            }
            TableAction::Formula => {
                let formula = document.suggested_formula();
                self.open(Some(TableDialog::Formula {
                    formula,
                    format: "",
                }))
            }
            TableAction::UpdateFormulas => edited(document.update_formulas()),
            TableAction::ToText => self.open(Some(TableDialog::ToText {
                separator: Separator::Tab,
                other: "-".to_string(),
            })),
            TableAction::ToTable => {
                let (columns, _) = document.text_table_shape(&TextSeparator::Tab);
                self.open(Some(TableDialog::ToTable {
                    separator: Separator::Tab,
                    other: "-".to_string(),
                    columns: columns.to_string(),
                    focus: DialogBox::Columns,
                }))
            }
        }
    }

    fn open(&mut self, dialog: Option<TableDialog>) -> TableOutcome {
        match dialog {
            Some(dialog) => {
                self.dialog = Some(dialog);
                TableOutcome::Redraw
            }
            None => TableOutcome::Edited(Err("the caret is not in a table".to_string())),
        }
    }

    /// OK in the open dialog.
    pub fn apply_dialog<S: WordTables + ?Sized>(&mut self, document: &mut S) -> TableOutcome {
        let Some(dialog) = self.dialog.take() else {
            return TableOutcome::NotMine;
        };
        TableOutcome::Edited(match dialog {
            TableDialog::Properties(dialog) => {
                document.apply_table_properties(&dialog).map(|_| None)
            }
            TableDialog::Borders(dialog) => document.apply_borders(&dialog).map(|_| None),
            TableDialog::Cells {
                delete: false,
                shift,
            } => document.insert_cells(shift).map(Some),
            TableDialog::Cells {
                delete: true,
                shift,
            } => document.delete_cells(shift).map(Some),
            TableDialog::Split { columns, rows, .. } => document
                .table_command(TableCommand::SplitCell {
                    columns: columns.parse().unwrap_or(2).clamp(1, 63),
                    rows: rows.parse().unwrap_or(1).clamp(1, 63),
                })
                .map(Some),
            TableDialog::ToTable {
                separator,
                other,
                columns,
                ..
            } => document
                .convert_text_to_table(
                    &separator.text_separator(&other),
                    columns.parse().ok().filter(|count: &usize| *count > 0),
                )
                .map(Some),
            TableDialog::ToText { separator, other } => document
                .convert_table_to_text(&separator.text_separator(&other))
                .map(Some),
            TableDialog::Sort { keys, header, .. } => document
                .sort_table(&SortSpec {
                    keys: keys
                        .iter()
                        .flatten()
                        .map(|(column, kind, descending)| SortKey {
                            column: *column,
                            kind: *kind,
                            descending: *descending,
                        })
                        .collect(),
                    header_row: header,
                })
                .map(|_| None),
            TableDialog::Formula { formula, format } => document
                .insert_formula(&formula, Some(format))
                .map(|_| None),
        })
    }

    /// A key while a dialog is open: Enter is OK, Escape cancels, Backspace
    /// edits the focused box, and text types into it. `None` when no dialog
    /// is open or the key is not the dialog's.
    pub fn dialog_key<S: WordTables + ?Sized>(
        &mut self,
        document: &mut S,
        key: &str,
        text: Option<&str>,
    ) -> Option<TableOutcome> {
        let dialog = self.dialog.as_mut()?;
        Some(match key {
            "escape" => {
                self.dialog = None;
                TableOutcome::Redraw
            }
            "enter" | "return" => self.apply_dialog(document),
            "backspace" => {
                dialog.backspace();
                TableOutcome::Redraw
            }
            _ => {
                if let Some(text) = text {
                    dialog.type_text(text);
                }
                TableOutcome::Redraw
            }
        })
    }

    // ---- the mouse ---------------------------------------------------------

    /// The pointer went down at (`x`, `y`) on `page`, layout pixels: a
    /// border to drag, a row, column, cell or the table to select, or the
    /// pencil or the eraser at work.
    pub fn mouse_down<S: WordTables + ?Sized>(
        &mut self,
        document: &mut S,
        page: usize,
        (x, y): (f64, f64),
        click_count: usize,
    ) -> TableOutcome {
        if click_count >= 2 {
            return TableOutcome::NotMine;
        }
        match self.tool {
            Some(DrawTool::Pencil) => {
                self.drag = Some((TableDrag::Stroke, page, (x, y), (x, y)));
                return TableOutcome::Redraw;
            }
            Some(DrawTool::Eraser) => {
                return match document.erase_border(x, y) {
                    Ok(true) => TableOutcome::Edited(Ok(None)),
                    Ok(false) => TableOutcome::Redraw,
                    Err(error) => TableOutcome::Edited(Err(error)),
                };
            }
            None => {}
        }
        let Some(hit) = document.table_hit(x, y) else {
            return TableOutcome::NotMine;
        };
        match hit {
            TableHit::ColumnBorder {
                table_id, boundary, ..
            } => {
                self.drag = Some((
                    TableDrag::Column { table_id, boundary },
                    page,
                    (x, y),
                    (x, y),
                ));
                TableOutcome::Redraw
            }
            TableHit::RowBorder { table_id, row, .. } => {
                self.drag = Some((TableDrag::Row { table_id, row }, page, (x, y), (x, y)));
                TableOutcome::Redraw
            }
            _ => TableOutcome::Selected(document.click_table_hit(hit).map(|_| ())),
        }
    }

    /// The pointer moved with the button down, to `point` (layout pixels):
    /// a border or the pencil follows it; a text drag from `anchor` into
    /// another cell selects whole cells.
    pub fn mouse_drag<S: WordTables + ?Sized>(
        &mut self,
        document: &mut S,
        point: Option<(f64, f64)>,
        anchor: Option<Position>,
    ) -> TableOutcome {
        if let Some(drag) = self.drag.as_mut() {
            if let Some(point) = point {
                drag.3 = point;
            }
            return TableOutcome::Redraw;
        }
        let (Some(anchor), Some((x, y))) = (anchor, point) else {
            return TableOutcome::NotMine;
        };
        let Some(focus) = document.cell_id_for_paragraph(anchor.0).and_then(|from| {
            let to = document.cell_layouts().iter().position(|cell| {
                x >= cell.left && x <= cell.right && y >= cell.top && y <= cell.bottom
            })?;
            (to != from).then(|| document.cell_layouts()[to].paragraphs.first().copied())?
        }) else {
            return TableOutcome::NotMine;
        };
        match document.select_cells_between(anchor.0, focus) {
            Ok(true) => TableOutcome::Selected(Ok(())),
            _ => TableOutcome::NotMine,
        }
    }

    /// The button came up: a dragged border lands, a pencil stroke draws —
    /// a table drawn outside one goes in at the text `position_at` finds
    /// at the stroke's top-left corner.
    pub fn mouse_up<S: WordTables + ?Sized>(
        &mut self,
        document: &mut S,
        position_at: impl Fn(&S, f64, f64) -> Option<Position>,
    ) -> TableOutcome {
        let Some((drag, _page, from, to)) = self.drag.take() else {
            return TableOutcome::NotMine;
        };
        TableOutcome::Edited(match drag {
            TableDrag::Column { table_id, boundary } => {
                if (to.0 - from.0).abs() < 1.0 {
                    return TableOutcome::Redraw;
                }
                document
                    .drag_column_border(table_id, boundary, to.0)
                    .map(|_| None)
            }
            TableDrag::Row { table_id, row } => {
                if (to.1 - from.1).abs() < 1.0 {
                    return TableOutcome::Redraw;
                }
                document.drag_row_border(table_id, row, to.1).map(|_| None)
            }
            TableDrag::Stroke => {
                let at = position_at(document, from.0.min(to.0), from.1.min(to.1));
                document.draw_stroke(from, to, at)
            }
        })
    }

    /// The pointer moved without a button: whether what it is over changed.
    pub fn hover<S: WordTables + ?Sized>(
        &mut self,
        document: &S,
        point: Option<(f64, f64)>,
    ) -> bool {
        let hover = point.and_then(|(x, y)| document.table_hit(x, y));
        if hover == self.hover {
            return false;
        }
        self.hover = hover;
        true
    }
}
