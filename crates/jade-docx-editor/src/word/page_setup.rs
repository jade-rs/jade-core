//! Word's ページ設定 dialog — 文字数と行数, 余白, 用紙 and その他 — and the
//! 段組み, 行番号 and ページ番号の書式 dialogs beside it, over any
//! [`WordSession`]: what each shows for the section at the caret, and the
//! change applying it makes to the sections 設定対象 names.
//!
//! The dialog edits a copy of the section's layout and applies only what
//! was changed, so 文書全体 with a new top margin keeps each section's own
//! paper, columns and grid, as Word's does. Lengths are points; the shells
//! show them in millimetres, as Japanese Word does.

use jade_docx_headless_api::{
    HeadlessBorderEdge, HeadlessCommand, HeadlessCoverParagraph, HeadlessDocGrid,
    HeadlessLineNumbering, HeadlessPageBorderSet, HeadlessPageNumbering, HeadlessSectionBreakType,
    HeadlessSectionColumn, HeadlessSectionColumns, HeadlessSectionLayout,
    HeadlessSectionLayoutPatch, HeadlessSectionScope,
};

use super::page::Paper;
use super::WordSession;

/// Millimetres in a point.
pub const MM_PER_POINT: f32 = 25.4 / 72.0;

pub fn points_to_mm(points: f32) -> f32 {
    points * MM_PER_POINT
}

pub fn mm_to_points(mm: f32) -> f32 {
    mm / MM_PER_POINT
}

/// Word's default line pitch (`w:linePitch` 360): 18 points.
pub const DEFAULT_LINE_PITCH: f32 = 18.0;

/// The dialog's tabs, in Japanese Word's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SetupTab {
    /// 文字数と行数.
    Grid,
    /// 余白.
    #[default]
    Margins,
    /// 用紙.
    Paper,
    /// その他.
    Other,
}

impl SetupTab {
    pub const ALL: [SetupTab; 4] = [
        SetupTab::Grid,
        SetupTab::Margins,
        SetupTab::Paper,
        SetupTab::Other,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            SetupTab::Grid => "setup.tab.grid",
            SetupTab::Margins => "setup.tab.margins",
            SetupTab::Paper => "setup.tab.paper",
            SetupTab::Other => "setup.tab.other",
        }
    }
}

/// 設定対象: which sections the change reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyTo {
    /// 文書全体.
    WholeDocument,
    /// このセクション.
    ThisSection,
    /// 選択しているセクション.
    SelectedSections { first: usize, last: usize },
    /// これ以降: a section break at the caret, and the change from there
    /// to the end.
    ThisPointForward,
}

impl ApplyTo {
    pub fn label_key(self) -> &'static str {
        match self {
            ApplyTo::WholeDocument => "setup.apply.whole",
            ApplyTo::ThisSection => "setup.apply.section",
            ApplyTo::SelectedSections { .. } => "setup.apply.selected",
            ApplyTo::ThisPointForward => "setup.apply.forward",
        }
    }
}

/// 印刷の形式 (複数ページの印刷設定).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MultiplePages {
    /// 標準.
    #[default]
    Normal,
    /// 見開きページ (mirror margins).
    Mirror,
    /// 袋とじ (2 pages per sheet).
    TwoOnOne,
    /// 本 (縦方向に谷折り) (book fold).
    BookFold,
}

impl MultiplePages {
    pub const ALL: [MultiplePages; 4] = [
        MultiplePages::Normal,
        MultiplePages::Mirror,
        MultiplePages::TwoOnOne,
        MultiplePages::BookFold,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            MultiplePages::Normal => "setup.pages.normal",
            MultiplePages::Mirror => "setup.pages.mirror",
            MultiplePages::TwoOnOne => "setup.pages.two_on_one",
            MultiplePages::BookFold => "setup.pages.book_fold",
        }
    }

    fn of(layout: &HeadlessSectionLayout) -> Self {
        if layout.book_fold {
            MultiplePages::BookFold
        } else if layout.print_two_on_one {
            MultiplePages::TwoOnOne
        } else if layout.mirror_margins {
            MultiplePages::Mirror
        } else {
            MultiplePages::Normal
        }
    }
}

/// The 文字数と行数 tab's グリッド.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridMode {
    /// 標準の文字数を使う (`default`).
    Default,
    /// 行数だけを指定する (`lines`).
    Lines,
    /// 文字数と行数を指定する (`linesAndChars`).
    CharsAndLines,
    /// 原稿用紙の設定にする (`snapToChars`).
    Manuscript,
}

impl GridMode {
    pub const ALL: [GridMode; 4] = [
        GridMode::Default,
        GridMode::Lines,
        GridMode::CharsAndLines,
        GridMode::Manuscript,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            GridMode::Default => "setup.grid.default",
            GridMode::Lines => "setup.grid.lines",
            GridMode::CharsAndLines => "setup.grid.chars_and_lines",
            GridMode::Manuscript => "setup.grid.manuscript",
        }
    }

    fn ooxml(self) -> &'static str {
        match self {
            GridMode::Default => "default",
            GridMode::Lines => "lines",
            GridMode::CharsAndLines => "linesAndChars",
            GridMode::Manuscript => "snapToChars",
        }
    }

    fn of(grid_type: &str) -> Self {
        match grid_type {
            "lines" => GridMode::Lines,
            "linesAndChars" => GridMode::CharsAndLines,
            "snapToChars" => GridMode::Manuscript,
            _ => GridMode::Default,
        }
    }

    /// Whether the line count can be set.
    pub fn sets_lines(self) -> bool {
        self != GridMode::Default
    }

    /// Whether the character count can be set.
    pub fn sets_chars(self) -> bool {
        matches!(self, GridMode::CharsAndLines | GridMode::Manuscript)
    }
}

/// セクションの開始位置.
pub const SECTION_STARTS: [(&str, &str); 5] = [
    ("continuous", "setup.start.continuous"),
    ("nextColumn", "setup.start.next_column"),
    ("nextPage", "setup.start.next_page"),
    ("evenPage", "setup.start.even_page"),
    ("oddPage", "setup.start.odd_page"),
];

/// 垂直方向の配置.
pub const VERTICAL_ALIGNMENTS: [(&str, &str); 4] = [
    ("top", "setup.valign.top"),
    ("center", "setup.valign.center"),
    ("both", "setup.valign.both"),
    ("bottom", "setup.valign.bottom"),
];

/// 番号の付け方 (行番号).
pub const LINE_RESTARTS: [(&str, &str); 3] = [
    ("newPage", "setup.lines.new_page"),
    ("newSection", "setup.lines.new_section"),
    ("continuous", "setup.lines.continuous"),
];

/// The number fields the dialogs type into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    MarginTop,
    MarginBottom,
    MarginLeft,
    MarginRight,
    Gutter,
    PaperWidth,
    PaperHeight,
    HeaderDistance,
    FooterDistance,
    /// 段数.
    Columns,
    /// 文字数 (per line).
    CharsPerLine,
    /// 字送り.
    CharPitch,
    /// 行数 (per page).
    LinesPerPage,
    /// 行送り.
    LinePitch,
    /// 行番号の開始番号.
    LineStart,
    /// 行番号の間隔.
    LineCountBy,
    /// 行番号と文字列との間隔 (0 is 自動).
    LineDistance,
}

impl Field {
    /// Whether the field shows millimetres (rather than a count or points).
    pub fn is_length(self) -> bool {
        matches!(
            self,
            Field::MarginTop
                | Field::MarginBottom
                | Field::MarginLeft
                | Field::MarginRight
                | Field::Gutter
                | Field::PaperWidth
                | Field::PaperHeight
                | Field::HeaderDistance
                | Field::FooterDistance
                | Field::LineDistance
        )
    }

    /// What one step of the spinner adds: a millimetre, one, or half a
    /// point.
    fn step(self) -> f32 {
        match self {
            field if field.is_length() => 1.0,
            Field::CharPitch | Field::LinePitch => 0.5,
            _ => 1.0,
        }
    }

    /// The unit's string key.
    pub fn unit_key(self) -> &'static str {
        match self {
            field if field.is_length() => "setup.unit.mm",
            Field::Columns => "setup.unit.columns",
            Field::CharsPerLine => "setup.unit.chars",
            Field::LinesPerPage => "setup.unit.lines",
            Field::CharPitch | Field::LinePitch => "setup.unit.pt",
            _ => "setup.unit.none",
        }
    }
}

/// The ページ設定 dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSetupDialog {
    pub tab: SetupTab,
    /// What the dialog opened on.
    pub original: HeadlessSectionLayout,
    /// What it shows now.
    pub layout: HeadlessSectionLayout,
    pub apply_to: ApplyTo,
    /// The 設定対象 the caret or selection allows.
    pub apply_choices: Vec<ApplyTo>,
    /// The field being typed into and what has been typed.
    pub editing: Option<(Field, String)>,
    /// The 行番号 dialog, open over the その他 tab.
    pub line_numbers_open: bool,
}

impl PageSetupDialog {
    /// Opens on `layout`, the caret's section; `sections` is the range the
    /// selection spans, when it spans more than one.
    pub fn open(
        layout: HeadlessSectionLayout,
        sections: Option<(usize, usize)>,
        tab: SetupTab,
    ) -> Self {
        let apply_choices = match sections {
            Some((first, last)) if first != last => vec![
                ApplyTo::SelectedSections { first, last },
                ApplyTo::WholeDocument,
            ],
            _ if layout.section_count > 1 => vec![
                ApplyTo::ThisSection,
                ApplyTo::ThisPointForward,
                ApplyTo::WholeDocument,
            ],
            _ => vec![ApplyTo::WholeDocument, ApplyTo::ThisPointForward],
        };
        Self {
            tab,
            original: layout.clone(),
            layout,
            apply_to: apply_choices[0],
            apply_choices,
            editing: None,
            line_numbers_open: false,
        }
    }

    pub fn multiple_pages(&self) -> MultiplePages {
        MultiplePages::of(&self.layout)
    }

    /// 印刷の形式. 本 (縦方向に谷折り) turns the sheet landscape, as Word
    /// does; the others leave it.
    pub fn set_multiple_pages(&mut self, pages: MultiplePages) {
        self.layout.mirror_margins = pages == MultiplePages::Mirror;
        self.layout.print_two_on_one = pages == MultiplePages::TwoOnOne;
        self.layout.book_fold = pages == MultiplePages::BookFold;
        if pages == MultiplePages::BookFold {
            self.set_landscape(true);
        }
        if pages != MultiplePages::Normal {
            // The binding is the inside edge then, not a chosen one.
            self.layout.gutter_at_top = false;
        }
    }

    /// Whether 左/右 read 内側/外側 — facing pages, as Word relabels them.
    pub fn inside_outside(&self) -> bool {
        matches!(
            self.multiple_pages(),
            MultiplePages::Mirror | MultiplePages::BookFold
        )
    }

    /// Whether 上/下 read 外側/内側 — two upright pages on one sheet.
    pub fn outside_inside(&self) -> bool {
        self.multiple_pages() == MultiplePages::TwoOnOne && !self.layout.landscape
    }

    pub fn set_landscape(&mut self, landscape: bool) {
        if self.layout.landscape == landscape {
            return;
        }
        let layout = &mut self.layout;
        layout.landscape = landscape;
        std::mem::swap(&mut layout.width, &mut layout.height);
        // Word turns the margins with the page.
        let (top, bottom, left, right) = (
            layout.margin_top,
            layout.margin_bottom,
            layout.margin_left,
            layout.margin_right,
        );
        if landscape {
            layout.margin_top = left;
            layout.margin_bottom = right;
            layout.margin_left = bottom;
            layout.margin_right = top;
        } else {
            layout.margin_top = right;
            layout.margin_bottom = left;
            layout.margin_left = top;
            layout.margin_right = bottom;
        }
    }

    /// 用紙サイズ: a paper, kept the way up it is.
    pub fn set_paper(&mut self, paper: Paper) {
        let (short, long) = paper.size();
        let (width, height) = if self.layout.landscape {
            (long, short)
        } else {
            (short, long)
        };
        self.layout.width = width;
        self.layout.height = height;
    }

    pub fn paper(&self) -> Option<Paper> {
        Paper::of(self.layout.width, self.layout.height)
    }

    /// 文字方向: 縦書き turns the page, as Japanese Word does.
    pub fn set_vertical(&mut self, vertical: bool) {
        let was = self.layout.text_direction == "tbRl";
        if was == vertical {
            return;
        }
        self.layout.text_direction = if vertical { "tbRl" } else { "lrTb" }.to_string();
        self.set_landscape(vertical);
    }

    pub fn grid_mode(&self) -> GridMode {
        GridMode::of(&self.layout.doc_grid.grid_type)
    }

    pub fn set_grid_mode(&mut self, mode: GridMode) {
        self.layout.doc_grid.grid_type = mode.ooxml().to_string();
        if mode.sets_lines() && self.layout.doc_grid.line_pitch.is_none() {
            self.layout.doc_grid.line_pitch = Some(DEFAULT_LINE_PITCH);
        }
        if mode.sets_chars() && self.layout.doc_grid.char_pitch.is_none() {
            self.layout.doc_grid.char_pitch = Some(self.layout.default_font_size);
        }
    }

    /// The text's width: the page less its side margins and a side gutter.
    pub fn content_width(&self) -> f32 {
        let layout = &self.layout;
        let gutter = if layout.gutter_at_top {
            0.0
        } else {
            layout.gutter
        };
        (self.page_width() - layout.margin_left - layout.margin_right - gutter).max(1.0)
    }

    /// The text's height: the page less its top and bottom margins and a top
    /// gutter.
    pub fn content_height(&self) -> f32 {
        let layout = &self.layout;
        let gutter = if layout.gutter_at_top {
            layout.gutter
        } else {
            0.0
        };
        (self.page_height() - layout.margin_top - layout.margin_bottom - gutter).max(1.0)
    }

    /// One page's width: half the sheet when two pages share it side by
    /// side.
    fn page_width(&self) -> f32 {
        let layout = &self.layout;
        if layout.book_fold || (layout.print_two_on_one && layout.landscape) {
            layout.width / 2.0
        } else {
            layout.width
        }
    }

    fn page_height(&self) -> f32 {
        let layout = &self.layout;
        if layout.print_two_on_one && !layout.landscape && !layout.book_fold {
            layout.height / 2.0
        } else {
            layout.height
        }
    }

    pub fn char_pitch(&self) -> f32 {
        self.layout
            .doc_grid
            .char_pitch
            .unwrap_or(self.layout.default_font_size)
            .max(1.0)
    }

    pub fn line_pitch(&self) -> f32 {
        self.layout
            .doc_grid
            .line_pitch
            .unwrap_or(DEFAULT_LINE_PITCH)
            .max(1.0)
    }

    /// 文字数: how many characters a line holds on the grid. Columns divide
    /// the line, as Word counts per column.
    pub fn chars_per_line(&self) -> u32 {
        let columns = self.layout.columns.count.max(1) as f32;
        let spaces = self.layout.columns.space * (columns - 1.0);
        (((self.content_width() - spaces) / columns) / self.char_pitch() + 0.001).floor() as u32
    }

    /// 行数: how many lines a page holds.
    pub fn lines_per_page(&self) -> u32 {
        (self.content_height() / self.line_pitch() + 0.001).floor() as u32
    }

    /// A field's value as the dialog shows it.
    pub fn value(&self, field: Field) -> f32 {
        let layout = &self.layout;
        let mm = points_to_mm;
        match field {
            Field::MarginTop => mm(layout.margin_top),
            Field::MarginBottom => mm(layout.margin_bottom),
            Field::MarginLeft => mm(layout.margin_left),
            Field::MarginRight => mm(layout.margin_right),
            Field::Gutter => mm(layout.gutter),
            Field::PaperWidth => mm(layout.width),
            Field::PaperHeight => mm(layout.height),
            Field::HeaderDistance => mm(layout.header_distance),
            Field::FooterDistance => mm(layout.footer_distance),
            Field::Columns => layout.columns.count as f32,
            Field::CharsPerLine => self.chars_per_line() as f32,
            Field::CharPitch => self.char_pitch(),
            Field::LinesPerPage => self.lines_per_page() as f32,
            Field::LinePitch => self.line_pitch(),
            Field::LineStart => layout.line_numbering.start_at as f32,
            Field::LineCountBy => layout.line_numbering.count_by as f32,
            Field::LineDistance => layout.line_numbering.distance.map_or(0.0, mm),
        }
    }

    /// The field's value as text: what is being typed, or the value.
    pub fn text(&self, field: Field) -> String {
        if let Some((editing, typed)) = self.editing.as_ref() {
            if *editing == field {
                return typed.clone();
            }
        }
        let value = self.value(field);
        if field.is_length() || matches!(field, Field::CharPitch | Field::LinePitch) {
            let text = format!("{value:.1}");
            text.trim_end_matches(".0").to_string()
        } else {
            format!("{}", value.round() as i64)
        }
    }

    /// Sets a field from a value in its unit; out-of-range values are
    /// clamped to what Word accepts.
    pub fn set_value(&mut self, field: Field, value: f32) {
        if !value.is_finite() {
            return;
        }
        let points = mm_to_points(value.max(0.0));
        let layout = &mut self.layout;
        match field {
            Field::MarginTop => layout.margin_top = points,
            Field::MarginBottom => layout.margin_bottom = points,
            Field::MarginLeft => layout.margin_left = points,
            Field::MarginRight => layout.margin_right = points,
            Field::Gutter => layout.gutter = points,
            Field::PaperWidth => {
                layout.width = points.max(mm_to_points(10.0));
                layout.landscape = layout.width > layout.height;
            }
            Field::PaperHeight => {
                layout.height = points.max(mm_to_points(10.0));
                layout.landscape = layout.width > layout.height;
            }
            Field::HeaderDistance => layout.header_distance = points,
            Field::FooterDistance => layout.footer_distance = points,
            Field::Columns => {
                let count = (value.round() as u32).clamp(1, 45);
                layout.columns.count = count;
                layout.columns.equal_width = true;
                layout.columns.columns.clear();
            }
            Field::CharsPerLine => {
                let count = value.round().max(1.0);
                let columns = self.layout.columns.count.max(1) as f32;
                let spaces = self.layout.columns.space * (columns - 1.0);
                let pitch = ((self.content_width() - spaces) / columns) / count;
                self.layout.doc_grid.char_pitch = Some(pitch);
            }
            Field::CharPitch => layout.doc_grid.char_pitch = Some(value.clamp(1.0, 1584.0)),
            Field::LinesPerPage => {
                let count = value.round().max(1.0);
                let pitch = self.content_height() / count;
                // Word keeps the pitch in twips.
                self.layout.doc_grid.line_pitch = Some((pitch * 20.0).floor() / 20.0);
            }
            Field::LinePitch => layout.doc_grid.line_pitch = Some(value.clamp(1.0, 1584.0)),
            Field::LineStart => layout.line_numbering.start_at = (value.round() as u32).max(1),
            Field::LineCountBy => {
                layout.line_numbering.count_by = (value.round() as u32).clamp(1, 100)
            }
            Field::LineDistance => layout.line_numbering.distance = (value > 0.0).then_some(points),
        }
    }

    /// The spinner: one step up or down.
    pub fn step(&mut self, field: Field, up: bool) {
        self.commit_typing();
        let step = field.step();
        let value = self.value(field) + if up { step } else { -step };
        self.set_value(field, value);
    }

    /// Starts typing into `field`, over its value.
    pub fn focus(&mut self, field: Field) {
        self.commit_typing();
        self.editing = Some((field, String::new()));
    }

    /// A typed character: digits and a decimal point.
    pub fn type_char(&mut self, text: &str) {
        if let Some((_, typed)) = self.editing.as_mut() {
            typed.extend(
                text.chars()
                    .map(|ch| match ch {
                        // Full-width digits, as a Japanese IME types them.
                        '０'..='９' => char::from(b'0' + (ch as u32 - '０' as u32) as u8),
                        '．' => '.',
                        other => other,
                    })
                    .filter(|ch| ch.is_ascii_digit() || *ch == '.'),
            );
        }
    }

    pub fn backspace(&mut self) {
        if let Some((_, typed)) = self.editing.as_mut() {
            typed.pop();
        }
    }

    /// What was typed becomes the field's value.
    pub fn commit_typing(&mut self) {
        if let Some((field, typed)) = self.editing.take() {
            if let Ok(value) = typed.parse::<f32>() {
                self.set_value(field, value);
            }
        }
    }

    /// Why OK cannot apply, if it cannot: the text has no room, as Word
    /// says before it closes.
    pub fn problem(&self) -> Option<&'static str> {
        if self.content_width() < 12.0 || self.content_height() < 12.0 {
            return Some("setup.problem.margins");
        }
        None
    }

    /// What applying changes, field by field.
    pub fn patch(&self) -> HeadlessSectionLayoutPatch {
        let (now, then) = (&self.layout, &self.original);
        fn changed<T: PartialEq + Clone>(now: &T, then: &T) -> Option<T> {
            (now != then).then(|| now.clone())
        }
        fn length(now: f32, then: f32) -> Option<f32> {
            ((now - then).abs() > 0.004).then_some(now)
        }
        let paper_changed = length(now.width, then.width).is_some()
            || length(now.height, then.height).is_some()
            || now.landscape != then.landscape;
        HeadlessSectionLayoutPatch {
            width: paper_changed.then_some(now.width),
            height: paper_changed.then_some(now.height),
            landscape: paper_changed.then_some(now.landscape),
            margin_top: length(now.margin_top, then.margin_top),
            margin_bottom: length(now.margin_bottom, then.margin_bottom),
            margin_left: length(now.margin_left, then.margin_left),
            margin_right: length(now.margin_right, then.margin_right),
            header_distance: length(now.header_distance, then.header_distance),
            footer_distance: length(now.footer_distance, then.footer_distance),
            gutter: length(now.gutter, then.gutter),
            gutter_at_top: changed(&now.gutter_at_top, &then.gutter_at_top),
            rtl_gutter: changed(&now.rtl_gutter, &then.rtl_gutter),
            mirror_margins: changed(&now.mirror_margins, &then.mirror_margins),
            book_fold: changed(&now.book_fold, &then.book_fold),
            print_two_on_one: changed(&now.print_two_on_one, &then.print_two_on_one),
            start: changed(&now.start, &then.start),
            vertical_alignment: changed(&now.vertical_alignment, &then.vertical_alignment),
            different_first_page: changed(&now.different_first_page, &then.different_first_page),
            different_odd_even: changed(&now.different_odd_even, &then.different_odd_even),
            columns: changed(&now.columns, &then.columns),
            line_numbering: changed(&now.line_numbering, &then.line_numbering),
            doc_grid: changed(&now.doc_grid, &then.doc_grid),
            text_direction: changed(&now.text_direction, &then.text_direction),
            bidi: changed(&now.bidi, &then.bidi),
            page_numbering: changed(&now.page_numbering, &then.page_numbering),
            page_borders: changed(&now.page_borders, &then.page_borders),
        }
    }
}

/// The 段組み dialog (段組みの詳細設定).
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnsDialog {
    pub original: HeadlessSectionColumns,
    pub columns: HeadlessSectionColumns,
    /// The width the columns share.
    pub content_width: f32,
    pub apply_to: ApplyTo,
    pub apply_choices: Vec<ApplyTo>,
}

/// The 段組み menu's presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnPreset {
    One,
    Two,
    Three,
    /// 1 段目を狭く (左).
    Left,
    /// 2 段目を狭く (右).
    Right,
}

impl ColumnsDialog {
    pub fn open(dialog: &PageSetupDialog) -> Self {
        let mut columns = dialog.layout.columns.clone();
        if columns.equal_width {
            columns.columns.clear();
        }
        Self {
            original: columns.clone(),
            columns,
            content_width: dialog.content_width(),
            apply_to: dialog.apply_to,
            apply_choices: dialog.apply_choices.clone(),
        }
    }

    /// Each column's width and the space after it, as the 幅と間隔 table
    /// lists them: equal columns share the width.
    pub fn widths(&self) -> Vec<HeadlessSectionColumn> {
        if !self.columns.equal_width && !self.columns.columns.is_empty() {
            return self.columns.columns.clone();
        }
        let count = self.columns.count.max(1);
        let space = self.columns.space;
        let width = (self.content_width - space * (count - 1) as f32) / count as f32;
        (0..count)
            .map(|index| HeadlessSectionColumn {
                width,
                space: if index + 1 < count { space } else { 0.0 },
            })
            .collect()
    }

    pub fn set_count(&mut self, count: u32) {
        let count = count.clamp(1, 12);
        self.columns.count = count;
        self.columns.equal_width = true;
        self.columns.columns.clear();
        if count == 1 {
            self.columns.separator = false;
        }
    }

    pub fn apply_preset(&mut self, preset: ColumnPreset) {
        match preset {
            ColumnPreset::One => self.set_count(1),
            ColumnPreset::Two => self.set_count(2),
            ColumnPreset::Three => self.set_count(3),
            ColumnPreset::Left | ColumnPreset::Right => {
                self.columns.count = 2;
                self.columns.equal_width = false;
                let space = self.columns.space;
                let narrow = (self.content_width - space) * 0.32;
                let wide = self.content_width - space - narrow;
                let (first, second) = if preset == ColumnPreset::Left {
                    (narrow, wide)
                } else {
                    (wide, narrow)
                };
                self.columns.columns = vec![
                    HeadlessSectionColumn {
                        width: first,
                        space,
                    },
                    HeadlessSectionColumn {
                        width: second,
                        space: 0.0,
                    },
                ];
            }
        }
    }

    /// 段の幅をすべて同じにする.
    pub fn set_equal(&mut self, equal: bool) {
        if equal {
            let count = self.columns.count;
            self.set_count(count);
        } else {
            self.columns.columns = self.widths();
            self.columns.equal_width = false;
        }
    }

    /// A column's width, the others narrowing or widening to keep the line,
    /// as Word's table does: the last column takes up the difference.
    pub fn set_width(&mut self, index: usize, width_mm: f32) {
        self.set_equal(false);
        let width = mm_to_points(width_mm).max(mm_to_points(5.0));
        let columns = &mut self.columns.columns;
        if index >= columns.len() {
            return;
        }
        columns[index].width = width;
        self.fit_last(index);
    }

    /// The space after a column.
    pub fn set_space(&mut self, index: usize, space_mm: f32) {
        let space = mm_to_points(space_mm).max(0.0);
        if self.columns.equal_width || self.columns.columns.is_empty() {
            self.columns.space = space;
            return;
        }
        let columns = &mut self.columns.columns;
        if index + 1 >= columns.len() {
            return;
        }
        columns[index].space = space;
        self.fit_last(index);
    }

    fn fit_last(&mut self, changed: usize) {
        let columns = &mut self.columns.columns;
        let last = columns.len() - 1;
        let target = if changed == last {
            last.saturating_sub(1)
        } else {
            last
        };
        let used: f32 = columns
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != target)
            .map(|(index, column)| column.width + if index < last { column.space } else { 0.0 })
            .sum::<f32>()
            + if target < last {
                columns[target].space
            } else {
                0.0
            };
        columns[target].width = (self.content_width - used).max(mm_to_points(5.0));
    }

    pub fn changed(&self) -> bool {
        self.columns != self.original
    }
}

/// Word's ページ番号の書式 formats, as its list shows them.
pub const PAGE_NUMBER_FORMATS: [(&str, &str); 8] = [
    ("decimal", "1, 2, 3, …"),
    ("numberInDash", "- 1 -, - 2 -, - 3 -, …"),
    ("lowerLetter", "a, b, c, …"),
    ("upperLetter", "A, B, C, …"),
    ("lowerRoman", "i, ii, iii, …"),
    ("upperRoman", "I, II, III, …"),
    ("decimalFullWidth", "１, ２, ３, …"),
    ("japaneseCounting", "一, 二, 三, …"),
];

/// 区切り文字 for 章番号を含める.
pub const CHAPTER_SEPARATORS: [(&str, &str); 5] = [
    ("hyphen", "- (ハイフン)"),
    ("period", ". (ピリオド)"),
    ("colon", ": (コロン)"),
    ("emDash", "— (エム ダッシュ)"),
    ("enDash", "– (エン ダッシュ)"),
];

/// The ページ番号の書式 dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct PageNumberDialog {
    pub original: HeadlessPageNumbering,
    pub numbering: HeadlessPageNumbering,
    /// 開始番号, typed.
    pub start_text: String,
}

impl PageNumberDialog {
    pub fn open(numbering: HeadlessPageNumbering) -> Self {
        Self {
            start_text: numbering.start.unwrap_or(1).to_string(),
            original: numbering.clone(),
            numbering,
        }
    }

    /// 連続番号: 前のセクションから継続 (`None`) or 開始番号.
    pub fn set_continue(&mut self, continue_from_previous: bool) {
        self.numbering.start = if continue_from_previous {
            None
        } else {
            Some(self.start_text.parse().unwrap_or(1).max(1))
        };
    }

    pub fn set_start(&mut self, start: u32) {
        let start = start.max(1);
        self.start_text = start.to_string();
        self.numbering.start = Some(start);
    }

    /// 章番号を含める, from the heading level.
    pub fn set_chapter(&mut self, level: Option<u8>) {
        self.numbering.chapter_style = level;
        if level.is_some() && self.numbering.chapter_separator.is_none() {
            self.numbering.chapter_separator = Some("hyphen".to_string());
        }
    }

    pub fn changed(&self) -> bool {
        self.numbering != self.original
    }
}

/// ページ罫線's line styles, as `w:val` names them, and their labels: Word's
/// whole line list.
pub const PAGE_BORDER_STYLES: [(&str, &str); 24] = super::borders::LINE_STYLES;

/// ページ罫線's 絵柄 (art borders), and their labels.
pub const PAGE_BORDER_ARTS: [(&str, &str); 16] = super::borders::ART_STYLES;

/// 線の太さ, in points.
pub const PAGE_BORDER_WIDTHS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0];

/// The colours the page tools offer: Word's standard colours, `RRGGBB`.
pub const PAGE_COLORS: [(&str, &str); 16] = [
    ("FFFFFF", "setup.color.white"),
    ("F2F2F2", "setup.color.light_gray"),
    ("FFF9E6", "setup.color.cream"),
    ("FDE9D9", "setup.color.rose"),
    ("EAF1DD", "setup.color.pale_green"),
    ("DBE5F1", "setup.color.pale_blue"),
    ("E5DFEC", "setup.color.lavender"),
    ("C00000", "setup.color.dark_red"),
    ("FF0000", "setup.color.red"),
    ("FFC000", "setup.color.orange"),
    ("FFFF00", "setup.color.yellow"),
    ("92D050", "setup.color.light_green"),
    ("00B050", "setup.color.green"),
    ("00B0F0", "setup.color.light_blue"),
    ("0070C0", "setup.color.blue"),
    ("7030A0", "setup.color.purple"),
];

/// ページ罫線's 種類.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderSetting {
    /// 罫線なし.
    None,
    /// 囲む.
    Box,
    /// 指定: the sides chosen one by one.
    Custom,
}

/// The ページ罫線 dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct PageBorderDialog {
    pub original: HeadlessPageBorderSet,
    pub style: String,
    /// 線の太さ, in points.
    pub width: f32,
    /// `RRGGBB`, or `auto`.
    pub color: String,
    /// Top, bottom, left, right.
    pub sides: [bool; 4],
    /// 基準: `text` (本文) or `page` (ページの端).
    pub offset_from: String,
    pub apply_to: ApplyTo,
    pub apply_choices: Vec<ApplyTo>,
}

impl PageBorderDialog {
    pub fn open(setup: &PageSetupDialog) -> Self {
        let borders = setup.layout.page_borders.clone();
        let first = [&borders.top, &borders.bottom, &borders.left, &borders.right]
            .into_iter()
            .flatten()
            .next()
            .cloned();
        Self {
            style: first
                .as_ref()
                .and_then(|edge| edge.style.clone())
                .unwrap_or_else(|| "single".to_string()),
            width: first.as_ref().and_then(|edge| edge.width).unwrap_or(0.5),
            color: first
                .as_ref()
                .and_then(|edge| edge.color.clone())
                .map(|color| color.trim_start_matches('#').to_string())
                .unwrap_or_else(|| "auto".to_string()),
            sides: [
                borders.top.is_some(),
                borders.bottom.is_some(),
                borders.left.is_some(),
                borders.right.is_some(),
            ],
            offset_from: borders
                .offset_from
                .clone()
                .unwrap_or_else(|| "text".to_string()),
            original: borders,
            apply_to: if setup.layout.section_count > 1 {
                ApplyTo::ThisSection
            } else {
                ApplyTo::WholeDocument
            },
            apply_choices: if setup.layout.section_count > 1 {
                vec![ApplyTo::ThisSection, ApplyTo::WholeDocument]
            } else {
                vec![ApplyTo::WholeDocument]
            },
        }
    }

    /// Picks a line style or an art border. Their widths mean different
    /// things — a line's in points up to 6, an art tile's in points 1–31 —
    /// so switching between them starts from Word's default for the kind.
    pub fn set_style(&mut self, style: &str) {
        let was_art = super::borders::is_art(&self.style);
        let is_art = super::borders::is_art(style);
        self.style = style.to_string();
        if is_art && !was_art {
            self.width = super::borders::DEFAULT_ART_WIDTH;
        } else if was_art && !is_art {
            self.width = 0.5;
        }
    }

    /// Whether the chosen style is an art border.
    pub fn is_art(&self) -> bool {
        super::borders::is_art(&self.style)
    }

    /// The widths offered for the chosen style, in points.
    pub fn widths(&self) -> &'static [f32] {
        if self.is_art() {
            &super::borders::ART_WIDTHS
        } else {
            &PAGE_BORDER_WIDTHS
        }
    }

    pub fn setting(&self) -> BorderSetting {
        match self.sides {
            [false, false, false, false] => BorderSetting::None,
            [true, true, true, true] => BorderSetting::Box,
            _ => BorderSetting::Custom,
        }
    }

    pub fn set_setting(&mut self, setting: BorderSetting) {
        match setting {
            BorderSetting::None => self.sides = [false; 4],
            BorderSetting::Box => self.sides = [true; 4],
            BorderSetting::Custom => {}
        }
    }

    /// The frame as it would be written.
    pub fn borders(&self) -> HeadlessPageBorderSet {
        let edge = |on: bool| {
            on.then(|| HeadlessBorderEdge {
                style: Some(self.style.clone()),
                width: Some(self.width),
                color: Some(self.color.clone()),
            })
        };
        HeadlessPageBorderSet {
            top: edge(self.sides[0]),
            bottom: edge(self.sides[1]),
            left: edge(self.sides[2]),
            right: edge(self.sides[3]),
            offset_from: Some(self.offset_from.clone()),
        }
    }

    pub fn changed(&self) -> bool {
        let now = self.borders();
        if now.is_empty() {
            return !self.original.is_empty();
        }
        now != self.original
    }
}

/// The 表紙 gallery's designs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverPage {
    /// Title and subtitle centred, the author and date below.
    Simple,
    /// A coloured band behind the title.
    Banner,
    /// Rules above and below a left-aligned title.
    Lined,
}

impl CoverPage {
    pub const ALL: [CoverPage; 3] = [CoverPage::Simple, CoverPage::Banner, CoverPage::Lined];

    pub fn label_key(self) -> &'static str {
        match self {
            CoverPage::Simple => "setup.cover.simple",
            CoverPage::Banner => "setup.cover.banner",
            CoverPage::Lined => "setup.cover.lined",
        }
    }

    /// The cover's paragraphs, with Word's placeholder texts (`japanese`
    /// picks their language).
    pub fn paragraphs(self, japanese: bool) -> Vec<HeadlessCoverParagraph> {
        let (title, subtitle, author, date) = if japanese {
            (
                "[文書のタイトル]",
                "[文書のサブタイトル]",
                "[作成者名]",
                "[日付]",
            )
        } else {
            (
                "[Document title]",
                "[Document subtitle]",
                "[Author name]",
                "[Date]",
            )
        };
        let paragraph = |text: &str| HeadlessCoverParagraph {
            text: text.to_string(),
            bold: false,
            italic: false,
            color: None,
            font_size: None,
            alignment: None,
            space_before: None,
            space_after: None,
            shading_color: None,
            border_top: false,
            border_bottom: false,
            border_left: false,
            border_right: false,
            indent_left: None,
            indent_right: None,
        };
        match self {
            CoverPage::Simple => vec![
                HeadlessCoverParagraph {
                    font_size: Some(36.0),
                    bold: true,
                    alignment: Some("center".to_string()),
                    space_before: Some(200.0),
                    space_after: Some(12.0),
                    ..paragraph(title)
                },
                HeadlessCoverParagraph {
                    font_size: Some(18.0),
                    color: Some("595959".to_string()),
                    alignment: Some("center".to_string()),
                    space_after: Some(180.0),
                    ..paragraph(subtitle)
                },
                HeadlessCoverParagraph {
                    font_size: Some(14.0),
                    alignment: Some("center".to_string()),
                    ..paragraph(author)
                },
                HeadlessCoverParagraph {
                    font_size: Some(14.0),
                    alignment: Some("center".to_string()),
                    ..paragraph(date)
                },
            ],
            CoverPage::Banner => vec![
                HeadlessCoverParagraph {
                    font_size: Some(40.0),
                    bold: true,
                    color: Some("FFFFFF".to_string()),
                    shading_color: Some("1F4E79".to_string()),
                    space_before: Some(160.0),
                    indent_left: Some(12.0),
                    ..paragraph(title)
                },
                HeadlessCoverParagraph {
                    font_size: Some(18.0),
                    color: Some("FFFFFF".to_string()),
                    shading_color: Some("1F4E79".to_string()),
                    space_after: Some(200.0),
                    indent_left: Some(12.0),
                    ..paragraph(subtitle)
                },
                HeadlessCoverParagraph {
                    font_size: Some(14.0),
                    alignment: Some("right".to_string()),
                    ..paragraph(author)
                },
                HeadlessCoverParagraph {
                    font_size: Some(14.0),
                    alignment: Some("right".to_string()),
                    ..paragraph(date)
                },
            ],
            CoverPage::Lined => vec![
                HeadlessCoverParagraph {
                    font_size: Some(34.0),
                    border_top: true,
                    space_before: Some(220.0),
                    ..paragraph(title)
                },
                HeadlessCoverParagraph {
                    font_size: Some(16.0),
                    italic: true,
                    border_bottom: true,
                    space_after: Some(220.0),
                    ..paragraph(subtitle)
                },
                HeadlessCoverParagraph {
                    font_size: Some(12.0),
                    ..paragraph(author)
                },
                HeadlessCoverParagraph {
                    font_size: Some(12.0),
                    ..paragraph(date)
                },
            ],
        }
    }
}

/// What a column break is in a paragraph's text, as Word's own text has
/// it: the character the engine writes as `<w:br w:type="column"/>`.
pub const COLUMN_BREAK: &str = "\u{000E}";

/// The 区切り menu's breaks inside a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageBreak {
    /// 改ページ (⌘↩).
    Page,
    /// 段区切り (⇧⌘↩).
    Column,
    /// 文字列の折り返し (⇧↩).
    TextWrapping,
}

/// The 行番号 menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineNumbersPreset {
    /// なし.
    None,
    /// 連続番号.
    Continuous,
    /// ページごとに振り直し.
    RestartEachPage,
    /// セクションごとに振り直し.
    RestartEachSection,
}

/// Page setup by section, for every session.
pub trait WordPageSetup: WordSession {
    /// The section the caret is in, by index.
    fn caret_section(&self) -> Option<usize> {
        let range = self.selection_range()?;
        self.word_document()
            .section_index_for_paragraph(range.focus.paragraph)
    }

    /// The sections the selection spans, first and last.
    fn selected_sections(&self) -> Option<(usize, usize)> {
        let range = self.selection_range()?;
        let (start, end) = range.normalized();
        let document = self.word_document();
        let first = document.section_index_for_paragraph(start.paragraph)?;
        let last = document.section_index_for_paragraph(end.paragraph)?;
        Some((first.min(last), first.max(last)))
    }

    /// The caret's section's layout (the final section without a caret).
    fn section_layout(&self) -> HeadlessSectionLayout {
        self.word_document().section_layout(self.caret_section())
    }

    /// Opens the ページ設定 dialog on the caret's section.
    fn page_setup_dialog(&self, tab: SetupTab) -> PageSetupDialog {
        PageSetupDialog::open(self.section_layout(), self.selected_sections(), tab)
    }

    /// Applies `patch` to the sections `apply_to` names, as one undo step.
    fn apply_section_layout(
        &mut self,
        patch: HeadlessSectionLayoutPatch,
        apply_to: ApplyTo,
    ) -> Result<(), String> {
        if patch.is_empty() {
            return Ok(());
        }
        let mut commands = Vec::new();
        let scope = match apply_to {
            ApplyTo::WholeDocument => HeadlessSectionScope::All,
            ApplyTo::ThisSection => HeadlessSectionScope::Caret,
            ApplyTo::SelectedSections { first, last } => {
                HeadlessSectionScope::Range { first, last }
            }
            ApplyTo::ThisPointForward => {
                let range = self.selection_range().ok_or("no caret")?;
                let (start, _) = range.normalized();
                let section = self.caret_section().unwrap_or(0);
                commands.push(HeadlessCommand::InsertSectionBreak {
                    paragraph: start.paragraph,
                    offset: start.offset,
                    break_type: HeadlessSectionBreakType::NextPage,
                });
                HeadlessSectionScope::Range {
                    first: section + 1,
                    last: usize::MAX,
                }
            }
        };
        commands.push(HeadlessCommand::SetSectionLayout {
            scope,
            patch: Box::new(patch),
        });
        let keep = apply_to != ApplyTo::ThisPointForward;
        self.apply_commands(commands, keep)
    }

    /// OK on the ページ設定 dialog.
    fn apply_page_setup(&mut self, dialog: &PageSetupDialog) -> Result<(), String> {
        let mut dialog = dialog.clone();
        dialog.commit_typing();
        if let Some(problem) = dialog.problem() {
            return Err(problem.to_string());
        }
        self.apply_section_layout(dialog.patch(), dialog.apply_to)
    }

    /// OK on the 段組み dialog.
    fn apply_columns(&mut self, dialog: &ColumnsDialog) -> Result<(), String> {
        if !dialog.changed() {
            return Ok(());
        }
        let mut columns = dialog.columns.clone();
        if !columns.equal_width {
            columns.count = columns.columns.len() as u32;
        }
        self.apply_section_layout(
            HeadlessSectionLayoutPatch {
                columns: Some(columns),
                ..Default::default()
            },
            dialog.apply_to,
        )
    }

    /// The 段組み menu's presets, on the caret's section.
    fn set_column_preset(&mut self, preset: ColumnPreset) -> Result<(), String> {
        let setup = self.page_setup_dialog(SetupTab::Grid);
        let mut dialog = ColumnsDialog::open(&setup);
        dialog.apply_to = ApplyTo::ThisSection;
        dialog.apply_preset(preset);
        self.apply_columns(&dialog)
    }

    /// OK on the ページ番号の書式 dialog: the caret's section.
    fn apply_page_numbering(&mut self, dialog: &PageNumberDialog) -> Result<(), String> {
        if !dialog.changed() {
            return Ok(());
        }
        self.apply_section_layout(
            HeadlessSectionLayoutPatch {
                page_numbering: Some(dialog.numbering.clone()),
                ..Default::default()
            },
            ApplyTo::ThisSection,
        )
    }

    /// The 行番号 menu, on the caret's section.
    fn set_line_numbers(&mut self, preset: LineNumbersPreset) -> Result<(), String> {
        let current = self.section_layout().line_numbering;
        let numbering = match preset {
            LineNumbersPreset::None => HeadlessLineNumbering {
                enabled: false,
                ..current
            },
            LineNumbersPreset::Continuous
            | LineNumbersPreset::RestartEachPage
            | LineNumbersPreset::RestartEachSection => HeadlessLineNumbering {
                enabled: true,
                start_at: current.start_at.max(1),
                count_by: current.count_by.max(1),
                distance: current.distance,
                restart: match preset {
                    LineNumbersPreset::Continuous => "continuous",
                    LineNumbersPreset::RestartEachSection => "newSection",
                    _ => "newPage",
                }
                .to_string(),
            },
        };
        self.apply_section_layout(
            HeadlessSectionLayoutPatch {
                line_numbering: Some(numbering),
                ..Default::default()
            },
            ApplyTo::ThisSection,
        )
    }

    /// Which 行番号 entry the caret's section is.
    fn line_numbers_preset(&self) -> LineNumbersPreset {
        let numbering = self.section_layout().line_numbering;
        if !numbering.enabled {
            return LineNumbersPreset::None;
        }
        match numbering.restart.as_str() {
            "continuous" => LineNumbersPreset::Continuous,
            "newSection" => LineNumbersPreset::RestartEachSection,
            _ => LineNumbersPreset::RestartEachPage,
        }
    }

    /// 文字列の方向: 縦書き or 横書き for the caret's section.
    fn set_vertical_text(&mut self, vertical: bool) -> Result<(), String> {
        let mut dialog = self.page_setup_dialog(SetupTab::Grid);
        dialog.apply_to = ApplyTo::ThisSection;
        dialog.set_vertical(vertical);
        self.apply_section_layout(dialog.patch(), ApplyTo::ThisSection)
    }

    fn vertical_text(&self) -> bool {
        self.section_layout().text_direction == "tbRl"
    }

    /// 右から左 / 左から右: the selected paragraphs' direction.
    fn set_paragraph_rtl(&mut self, rtl: bool) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, end) = range.normalized();
        self.apply_command(
            HeadlessCommand::SetParagraphDirection {
                start_paragraph: start.paragraph,
                end_paragraph: end.paragraph,
                bidi: rtl,
            },
            true,
        )
    }

    /// Whether the caret's paragraph reads right to left.
    fn paragraph_rtl(&self) -> bool {
        let Some(range) = self.selection_range() else {
            return false;
        };
        self.word_document()
            .paragraph(range.focus.paragraph)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.bidi)
            .unwrap_or(false)
    }

    /// 1 ページの行数を指定時に文字を行グリッド線に合わせる for the
    /// selected paragraphs.
    fn set_snap_to_grid(&mut self, snap: bool) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, end) = range.normalized();
        self.apply_command(
            HeadlessCommand::SetParagraphSnapToGrid {
                start_paragraph: start.paragraph,
                end_paragraph: end.paragraph,
                snap,
            },
            true,
        )
    }

    /// Whether the caret's paragraph snaps to the document grid (the
    /// default).
    fn snap_to_grid(&self) -> bool {
        let Some(range) = self.selection_range() else {
            return true;
        };
        self.word_document()
            .paragraph(range.focus.paragraph)
            .and_then(|paragraph| paragraph.properties.as_ref())
            .and_then(|properties| properties.snap_to_grid)
            .unwrap_or(true)
    }

    /// OK on the ページ罫線 dialog.
    fn apply_page_borders(&mut self, dialog: &PageBorderDialog) -> Result<(), String> {
        if !dialog.changed() {
            return Ok(());
        }
        self.apply_section_layout(
            HeadlessSectionLayoutPatch {
                page_borders: Some(dialog.borders()),
                ..Default::default()
            },
            dialog.apply_to,
        )
    }

    /// ページの色: `RRGGBB`, or none.
    fn set_page_color(&mut self, color: Option<&str>) -> Result<(), String> {
        self.apply_command(
            HeadlessCommand::SetPageBackgroundColor {
                color: color.map(|color| format!("#{}", color.trim_start_matches('#'))),
            },
            true,
        )
    }

    /// The page colour, `RRGGBB`.
    fn page_color(&self) -> Option<String> {
        self.word_document()
            .document
            .as_deref()
            .and_then(|document| document.background_color.clone())
            .map(|color| color.trim_start_matches('#').to_ascii_uppercase())
    }

    /// 表紙: a cover page before the first page, its header and footer
    /// kept off it (先頭ページのみ別指定), as Word's built-in covers do.
    fn insert_cover_page(&mut self, cover: CoverPage, japanese: bool) -> Result<(), String> {
        let layout = self.word_document().section_layout(Some(0));
        let mut commands = vec![HeadlessCommand::InsertCoverPage {
            paragraphs: cover.paragraphs(japanese),
        }];
        if !layout.different_first_page {
            commands.push(HeadlessCommand::SetHeaderFooterVariants {
                different_first_page: true,
                different_odd_even_pages: layout.different_odd_even,
                section: Some(0),
            });
        }
        self.apply_commands(commands, false)
    }

    /// 改ページ, 段区切り and 文字列の折り返し at the caret, over the
    /// selection.
    fn insert_page_break(&mut self, kind: PageBreak) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, end) = range.normalized();
        let text = match kind {
            PageBreak::Page => "\u{000C}",
            PageBreak::Column => COLUMN_BREAK,
            PageBreak::TextWrapping => "\n",
        };
        let command = if start == end {
            HeadlessCommand::InsertText {
                paragraph: start.paragraph,
                offset: start.offset,
                text: text.to_string(),
            }
        } else {
            HeadlessCommand::ReplaceRange {
                start_paragraph: start.paragraph,
                start_offset: start.offset,
                end_paragraph: end.paragraph,
                end_offset: end.offset,
                text: text.to_string(),
            }
        };
        self.apply_command(command, false)
    }
}

impl<T: WordSession + ?Sized> WordPageSetup for T {}

/// A layout the tests build dialogs over: A4, Word's Japanese margins.
#[doc(hidden)]
pub fn a4_layout() -> HeadlessSectionLayout {
    HeadlessSectionLayout {
        section: 0,
        section_count: 1,
        width: 595.3,
        height: 841.9,
        landscape: false,
        margin_top: 99.25,
        margin_bottom: 85.05,
        margin_left: 85.05,
        margin_right: 85.05,
        header_distance: 42.55,
        footer_distance: 49.6,
        gutter: 0.0,
        start: "nextPage".to_string(),
        vertical_alignment: "top".to_string(),
        columns: HeadlessSectionColumns {
            count: 1,
            space: 21.25,
            equal_width: true,
            separator: false,
            columns: Vec::new(),
        },
        line_numbering: HeadlessLineNumbering {
            enabled: false,
            start_at: 1,
            count_by: 1,
            distance: None,
            restart: "newPage".to_string(),
        },
        doc_grid: HeadlessDocGrid {
            grid_type: "lines".to_string(),
            line_pitch: Some(18.0),
            char_pitch: None,
        },
        default_font_size: 10.5,
        text_direction: "lrTb".to_string(),
        page_numbering: HeadlessPageNumbering {
            format: "decimal".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_art_page_border_takes_art_widths_and_switching_back_restores_a_line() {
        let setup = PageSetupDialog::open(a4_layout(), None, SetupTab::Margins);
        let mut dialog = PageBorderDialog::open(&setup);
        dialog.set_setting(BorderSetting::Box);
        dialog.set_style("thinThickMediumGap");
        assert_eq!(
            dialog.borders().top.unwrap().style.as_deref(),
            Some("thinThickMediumGap")
        );
        dialog.set_style("hearts");
        assert!(dialog.is_art());
        assert_eq!(dialog.width, crate::word::borders::DEFAULT_ART_WIDTH);
        assert!(dialog.widths().contains(&31.0));
        let frame = dialog.borders();
        assert_eq!(frame.left.unwrap().width, Some(20.0));
        dialog.set_style("double");
        assert_eq!(dialog.width, 0.5);
        assert_eq!(PAGE_BORDER_STYLES.len(), 24);
    }

    #[test]
    fn only_what_changed_is_applied() {
        let mut dialog = PageSetupDialog::open(a4_layout(), None, SetupTab::Margins);
        assert!(dialog.patch().is_empty());
        dialog.set_value(Field::MarginTop, 30.0);
        let patch = dialog.patch();
        assert!((patch.margin_top.unwrap() - mm_to_points(30.0)).abs() < 0.01);
        assert_eq!(patch.margin_left, None);
        assert_eq!(patch.width, None);
    }

    #[test]
    fn chars_and_lines_set_the_pitches() {
        let mut dialog = PageSetupDialog::open(a4_layout(), None, SetupTab::Grid);
        assert_eq!(dialog.lines_per_page(), 36, "Word's A4 page holds 36 lines");
        dialog.set_grid_mode(GridMode::CharsAndLines);
        dialog.set_value(Field::CharsPerLine, 40.0);
        assert_eq!(dialog.chars_per_line(), 40);
        dialog.set_value(Field::LinesPerPage, 30.0);
        assert_eq!(dialog.lines_per_page(), 30);
        let grid = dialog.patch().doc_grid.unwrap();
        assert_eq!(grid.grid_type, "linesAndChars");
        assert!((grid.char_pitch.unwrap() - 10.6).abs() < 0.1);
    }

    #[test]
    fn turning_the_page_turns_the_margins() {
        let mut dialog = PageSetupDialog::open(a4_layout(), None, SetupTab::Margins);
        dialog.set_landscape(true);
        assert!(dialog.layout.width > dialog.layout.height);
        assert_eq!(dialog.layout.margin_left, 85.05);
        assert_eq!(dialog.layout.margin_top, 85.05);
        dialog.set_vertical(false);
        assert!(dialog.layout.landscape);
    }

    #[test]
    fn vertical_text_turns_the_page_and_book_fold_turns_the_sheet() {
        let mut dialog = PageSetupDialog::open(a4_layout(), None, SetupTab::Grid);
        dialog.set_vertical(true);
        assert_eq!(dialog.layout.text_direction, "tbRl");
        assert!(dialog.layout.landscape);
        let mut dialog = PageSetupDialog::open(a4_layout(), None, SetupTab::Margins);
        dialog.set_multiple_pages(MultiplePages::BookFold);
        assert!(dialog.layout.landscape && dialog.inside_outside());
        dialog.set_multiple_pages(MultiplePages::TwoOnOne);
        assert!(!dialog.layout.book_fold && dialog.layout.print_two_on_one);
    }

    #[test]
    fn typing_replaces_the_value() {
        let mut dialog = PageSetupDialog::open(a4_layout(), None, SetupTab::Margins);
        dialog.focus(Field::MarginLeft);
        dialog.type_char("2");
        dialog.type_char("5x");
        assert_eq!(dialog.text(Field::MarginLeft), "25");
        dialog.commit_typing();
        assert!((dialog.value(Field::MarginLeft) - 25.0).abs() < 0.01);
        dialog.step(Field::MarginLeft, true);
        assert!((dialog.value(Field::MarginLeft) - 26.0).abs() < 0.01);
    }

    #[test]
    fn apply_choices_follow_the_sections() {
        let one = PageSetupDialog::open(a4_layout(), None, SetupTab::Margins);
        assert_eq!(one.apply_to, ApplyTo::WholeDocument);
        let mut layout = a4_layout();
        layout.section_count = 3;
        let caret = PageSetupDialog::open(layout.clone(), Some((1, 1)), SetupTab::Margins);
        assert_eq!(caret.apply_to, ApplyTo::ThisSection);
        let selected = PageSetupDialog::open(layout, Some((0, 2)), SetupTab::Margins);
        assert_eq!(
            selected.apply_to,
            ApplyTo::SelectedSections { first: 0, last: 2 }
        );
    }

    #[test]
    fn unequal_columns_keep_the_line() {
        let setup = PageSetupDialog::open(a4_layout(), None, SetupTab::Grid);
        let mut dialog = ColumnsDialog::open(&setup);
        dialog.apply_preset(ColumnPreset::Left);
        let widths = dialog.widths();
        assert_eq!(widths.len(), 2);
        assert!(widths[0].width < widths[1].width);
        let total = widths[0].width + widths[0].space + widths[1].width;
        assert!((total - dialog.content_width).abs() < 0.5);
        dialog.set_width(0, 50.0);
        let widths = dialog.widths();
        let total = widths[0].width + widths[0].space + widths[1].width;
        assert!((total - dialog.content_width).abs() < 0.5);
    }
}
