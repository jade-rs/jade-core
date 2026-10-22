//! Word's 表示 tab, as far as it is arithmetic both shells share: how far to
//! zoom for 1 ページ, 複数ページ and ページ幅を基準に表示, which band of a
//! page 余白を表示しない and 下書き show, and where the グリッド線 fall.
//!
//! Lengths come in points (as the section's page setup has them) and go out
//! in CSS pixels, the unit the pages are laid out and drawn in.

use crate::canvas_renderer::ViewGeometry;
use jade_docx_headless_api::HeadlessSectionLayout;

use super::xml::Document;

/// Points to CSS pixels.
pub fn points_to_px(points: f32) -> f32 {
    points * 96.0 / 72.0
}

/// Word's 表示 → 表示 group: how the document is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewKind {
    /// 閲覧モード: the text in screen-sized columns, no ribbon.
    Read,
    /// 印刷レイアウト.
    #[default]
    Print,
    /// Web レイアウト: the text wrapped to the window, no pages.
    Web,
    /// アウトライン.
    Outline,
    /// 下書き: the text band of each page, one after another.
    Draft,
}

impl ViewKind {
    pub const ALL: [ViewKind; 5] = [
        ViewKind::Read,
        ViewKind::Print,
        ViewKind::Web,
        ViewKind::Outline,
        ViewKind::Draft,
    ];

    /// The i18n key of the view's name.
    pub fn label_key(self) -> &'static str {
        match self {
            ViewKind::Read => "views.read",
            ViewKind::Print => "views.print",
            ViewKind::Web => "views.web",
            ViewKind::Outline => "views.outline",
            ViewKind::Draft => "views.draft",
        }
    }

    /// Whether the view lays the document out on its pages (印刷レイアウト
    /// and 下書き, which shows the same lines) rather than to the window.
    pub fn keeps_pages(self) -> bool {
        matches!(self, ViewKind::Print | ViewKind::Draft)
    }
}

/// ページ移動: 縦 or 横 (pages side by side, turned like a book's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageMovement {
    #[default]
    Vertical,
    SideToSide,
}

/// What the zoom follows as the window changes: a percentage the user set,
/// or one of ズーム's fits, which Word keeps as the window is resized.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ZoomFit {
    #[default]
    Custom,
    /// 1 ページ: a whole page in the window.
    OnePage,
    /// 複数ページ: this many pages across, each whole.
    MultiplePages(usize),
    /// ページ幅を基準に表示.
    PageWidth,
    /// 文字列の幅を基準に表示: the text column across the window.
    TextWidth,
}

/// The room around the pages: the canvas's padding and the gap between two
/// pages side by side.
pub const PAGE_PADDING: f32 = 32.0;
pub const PAGE_GAP: f32 = 24.0;

/// The zoom a fit asks for, for a page of `page` (width, height), whose
/// text column is `text` (left margin, width), in a window `area` wide and
/// high. `None` for [`ZoomFit::Custom`].
pub fn fit_zoom(fit: ZoomFit, page: (f32, f32), text: (f32, f32), area: (f32, f32)) -> Option<f32> {
    let (page_width, page_height) = (page.0.max(1.0), page.1.max(1.0));
    let width = (area.0 - 2.0 * PAGE_PADDING).max(1.0);
    let height = (area.1 - 2.0 * PAGE_PADDING).max(1.0);
    let zoom = match fit {
        ZoomFit::Custom => return None,
        ZoomFit::PageWidth => width / page_width,
        ZoomFit::TextWidth => width / text.1.max(1.0),
        ZoomFit::OnePage => (width / page_width).min(height / page_height),
        ZoomFit::MultiplePages(count) => {
            let count = count.max(1) as f32;
            let across = (width - PAGE_GAP * (count - 1.0)) / (page_width * count);
            across.min(height / page_height)
        }
    };
    Some(zoom.clamp(0.1, 5.0))
}

/// How many pages a row holds while pages are shown several across, or one
/// under another (`1`).
pub fn pages_per_row(fit: ZoomFit, movement: PageMovement, page_count: usize) -> usize {
    match (movement, fit) {
        (PageMovement::SideToSide, _) => page_count.max(1),
        (_, ZoomFit::MultiplePages(count)) => count.max(1),
        _ => 1,
    }
}

/// The page a scroll position shows, for pages laid `per_row` across with
/// `pitch` (x, y) from one to the next: the page whose middle is nearest
/// the viewport's top-left plus half a page.
pub fn page_at_scroll(
    scroll: (f32, f32),
    pitch: (f32, f32),
    per_row: usize,
    page_count: usize,
    horizontal: bool,
) -> usize {
    if page_count == 0 {
        return 0;
    }
    let page = if horizontal {
        ((scroll.0 + pitch.0 / 2.0) / pitch.0.max(1.0))
            .floor()
            .max(0.0) as usize
    } else {
        let row = ((scroll.1 + pitch.1 / 2.0) / pitch.1.max(1.0))
            .floor()
            .max(0.0) as usize;
        row * per_row.max(1)
    };
    page.min(page_count - 1)
}

/// Web レイアウト's strips: each this tall, closing up one under another.
pub const WEB_STRIP_HEIGHT: f64 = 1600.0;
/// Web レイアウト's text's distance from the window's sides.
pub const WEB_MARGIN: f64 = 24.0;
/// 閲覧モード's room round a column, and between two.
pub const READ_MARGIN: f64 = 48.0;
pub const READ_GAP: f32 = 32.0;
/// The widest a column of 閲覧モード reads comfortably at.
pub const READ_MAX_COLUMN: f64 = 760.0;

/// The page box view `kind` lays the document out in, for a window `area`
/// wide and high at `zoom`: Web レイアウト's strips as wide as the window,
/// 閲覧モード's screen-sized columns (two side by side when they fit).
/// `None` for the views that keep the sections' pages.
pub fn view_geometry(kind: ViewKind, area: (f32, f32), zoom: f32) -> Option<ViewGeometry> {
    let zoom = f64::from(zoom.max(0.1));
    let width = f64::from(area.0) / zoom;
    let height = f64::from(area.1) / zoom;
    match kind {
        ViewKind::Web => Some(ViewGeometry {
            page_width: (width - 2.0 * f64::from(PAGE_PADDING) / zoom)
                .max(240.0)
                .floor(),
            page_height: WEB_STRIP_HEIGHT,
            margin_top: 0.0,
            margin_right: WEB_MARGIN,
            margin_bottom: 0.0,
            margin_left: WEB_MARGIN,
            hide_page_chrome: true,
        }),
        ViewKind::Read => {
            let room = width - 2.0 * f64::from(PAGE_PADDING) / zoom;
            let two = room >= 2.0 * 520.0;
            let column = if two {
                ((room - f64::from(READ_GAP) / zoom) / 2.0).min(READ_MAX_COLUMN)
            } else {
                room.min(READ_MAX_COLUMN)
            };
            Some(ViewGeometry {
                page_width: column.max(240.0).floor(),
                page_height: (height - 2.0 * f64::from(PAGE_PADDING) / zoom)
                    .max(240.0)
                    .floor(),
                margin_top: READ_MARGIN,
                margin_right: READ_MARGIN,
                margin_bottom: READ_MARGIN,
                margin_left: READ_MARGIN,
                hide_page_chrome: true,
            })
        }
        _ => None,
    }
}

/// アウトライン's page box: the first section's width, the text from just
/// past the outline's symbols, in strips with nothing round them.
pub fn outline_geometry(layout: &HeadlessSectionLayout) -> Option<ViewGeometry> {
    if layout.width <= 0.0 {
        return None;
    }
    Some(ViewGeometry {
        page_width: f64::from(points_to_px(layout.width)),
        page_height: WEB_STRIP_HEIGHT,
        margin_top: 0.0,
        margin_right: f64::from(points_to_px(layout.margin_right)),
        margin_bottom: 0.0,
        margin_left: 8.0,
        hide_page_chrome: true,
    })
}

/// Whether two page boxes are close enough not to lay the document out
/// again for: a window dragged a pixel wider is not worth a re-flow.
pub fn geometry_close(a: Option<ViewGeometry>, b: Option<ViewGeometry>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            (a.page_width - b.page_width).abs() < 6.0
                && (a.page_height - b.page_height).abs() < 6.0
                && a.margin_left == b.margin_left
                && a.margin_top == b.margin_top
        }
        _ => false,
    }
}

/// Where a shell sets its pages out, in window pixels from the top-left of
/// the scrolled content: `per_row` across (one under another with 1), or
/// all in one row scrolled sideways (`horizontal`, 横 and 閲覧モード).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGrid {
    /// One page as shown: zoomed, and cut to its band when the view cuts.
    pub page: (f32, f32),
    pub gap: f32,
    pub padding: f32,
    pub per_row: usize,
    pub horizontal: bool,
}

impl PageGrid {
    /// The top-left of page `index` (a row's pages from the row's left).
    pub fn origin(&self, index: usize) -> (f32, f32) {
        let (width, height) = self.page;
        if self.horizontal {
            return (
                self.padding + index as f32 * (width + self.gap),
                self.padding,
            );
        }
        let per_row = self.per_row.max(1);
        let (row, column) = (index / per_row, index % per_row);
        (
            self.padding + column as f32 * (width + self.gap),
            self.padding + row as f32 * (height + self.gap),
        )
    }

    /// The pages a viewport `size` scrolled to `scroll` (both positive)
    /// shows, any part of them.
    pub fn visible(
        &self,
        scroll: (f32, f32),
        size: (f32, f32),
        page_count: usize,
    ) -> std::ops::Range<usize> {
        if page_count == 0 {
            return 0..0;
        }
        let (width, height) = (self.page.0.max(1.0), self.page.1.max(1.0));
        let (from, to, pitch) = if self.horizontal {
            (scroll.0, scroll.0 + size.0, width + self.gap)
        } else {
            (scroll.1, scroll.1 + size.1, height + self.gap)
        };
        let first = ((from - self.padding) / pitch).floor().max(0.0) as usize;
        let last = ((to - self.padding) / pitch).floor().max(0.0) as usize;
        let per = if self.horizontal {
            1
        } else {
            self.per_row.max(1)
        };
        let start = (first * per).min(page_count - 1);
        let end = ((last + 1) * per).min(page_count);
        start..end.max(start + 1)
    }

    /// The page counted current: the one a third of the way into the
    /// viewport, across for a row, down for a column.
    pub fn current(&self, scroll: (f32, f32), size: (f32, f32), page_count: usize) -> usize {
        if page_count == 0 {
            return 0;
        }
        let (width, height) = (self.page.0.max(1.0), self.page.1.max(1.0));
        if self.horizontal {
            let probe = scroll.0 + size.0 / 3.0 - self.padding;
            return ((probe / (width + self.gap)).floor().max(0.0) as usize).min(page_count - 1);
        }
        let probe = scroll.1 + size.1 / 3.0 - self.padding;
        let row = (probe / (height + self.gap)).floor().max(0.0) as usize;
        (row * self.per_row.max(1)).min(page_count - 1)
    }
}

/// The part of a page between its margins, in CSS pixels from the page's
/// top-left: what 余白を表示しない and 下書き keep of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageBand {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl PageBand {
    /// The band of a page in the section `layout`.
    pub fn of(layout: &HeadlessSectionLayout) -> PageBand {
        let gutter = if layout.gutter_at_top {
            0.0
        } else {
            layout.gutter
        };
        let top = layout.margin_top
            + if layout.gutter_at_top {
                layout.gutter
            } else {
                0.0
            };
        PageBand {
            top: points_to_px(top.max(0.0)),
            bottom: points_to_px((layout.height - layout.margin_bottom).max(top)),
            left: points_to_px(layout.margin_left + gutter),
            right: points_to_px((layout.width - layout.margin_right).max(0.0)),
        }
    }

    /// The band as kept by a view: 余白を表示しない keeps the page's width
    /// and cuts above and below; 下書き also cuts the left margin down to
    /// a strip.
    pub fn crop(&self, kind: ViewKind, page: (f32, f32)) -> PageCrop {
        let (top, bottom) = (self.top.min(page.1), self.bottom.clamp(self.top, page.1));
        match kind {
            ViewKind::Draft => PageCrop {
                top,
                height: bottom - top,
                left: (self.left - DRAFT_LEFT_STRIP).max(0.0),
                width: (self.right + DRAFT_LEFT_STRIP).min(page.0)
                    - (self.left - DRAFT_LEFT_STRIP).max(0.0),
            },
            _ => PageCrop {
                top,
                height: bottom - top,
                left: 0.0,
                width: page.0,
            },
        }
    }
}

/// What 下書き keeps of the left margin, so the caret at a line's start is
/// not on the window's edge.
pub const DRAFT_LEFT_STRIP: f32 = 16.0;

/// The rectangle of a page a view shows, in the page's CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageCrop {
    pub top: f32,
    pub height: f32,
    pub left: f32,
    pub width: f32,
}

/// Word's 描画グリッド (`w:drawingGrid*` in the settings part): the grid
/// グリッド線 shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawingGrid {
    /// 文字グリッド線の間隔 and 行グリッド線の間隔, in points.
    pub horizontal_spacing: f32,
    pub vertical_spacing: f32,
    /// グリッド線を表示する間隔: every so many grid units a line is drawn;
    /// 0 draws none that way.
    pub horizontal_every: u32,
    pub vertical_every: u32,
    /// Where the grid starts, in points from the page's top-left, when it
    /// does not start at the margins (`w:doNotUseMarginsForDrawingGridOrigin`).
    pub origin: Option<(f32, f32)>,
}

impl Default for DrawingGrid {
    /// Word's defaults: 180 twips (9 pt) both ways, every line shown, from
    /// the margins.
    fn default() -> Self {
        DrawingGrid {
            horizontal_spacing: 9.0,
            vertical_spacing: 9.0,
            horizontal_every: 1,
            vertical_every: 1,
            origin: None,
        }
    }
}

impl DrawingGrid {
    /// The grid a settings part (`word/settings.xml`) declares.
    pub fn from_settings(xml: &str) -> DrawingGrid {
        let mut grid = DrawingGrid::default();
        let Ok(document) = Document::parse(xml) else {
            return grid;
        };
        let value = |name: &str| -> Option<f32> {
            document
                .root
                .find(name)
                .and_then(|element| element.get("w:val"))
                .and_then(|value| value.trim().parse::<f32>().ok())
        };
        if let Some(twips) = value("w:drawingGridHorizontalSpacing").filter(|v| *v > 0.0) {
            grid.horizontal_spacing = twips / 20.0;
        }
        if let Some(twips) = value("w:drawingGridVerticalSpacing").filter(|v| *v > 0.0) {
            grid.vertical_spacing = twips / 20.0;
        }
        if let Some(every) = value("w:displayHorizontalDrawingGridEvery") {
            grid.horizontal_every = every.max(0.0) as u32;
        }
        if let Some(every) = value("w:displayVerticalDrawingGridEvery") {
            grid.vertical_every = every.max(0.0) as u32;
        }
        let no_margins = document
            .root
            .find("w:doNotUseMarginsForDrawingGridOrigin")
            .is_some_and(|element| {
                element
                    .get("w:val")
                    .is_none_or(|value| !matches!(value.as_str(), "0" | "false" | "off"))
            });
        if no_margins {
            let x = value("w:drawingGridHorizontalOrigin").unwrap_or(0.0) / 20.0;
            let y = value("w:drawingGridVerticalOrigin").unwrap_or(0.0) / 20.0;
            grid.origin = Some((x, y));
        }
        grid
    }
}

/// The drawing grid of a session's document, from its settings part.
pub fn document_drawing_grid<S: super::WordSession + ?Sized>(session: &S) -> DrawingGrid {
    let Ok(bytes) = session.word_document().export_docx() else {
        return DrawingGrid::default();
    };
    let Ok(package) = super::package::Package::open(&bytes) else {
        return DrawingGrid::default();
    };
    let main = package.main().to_string();
    let Some(name) = package.related(&main, "settings") else {
        return DrawingGrid::default();
    };
    match package.xml(&name) {
        Ok(document) => DrawingGrid::from_settings(&document.to_xml()),
        Err(_) => DrawingGrid::default(),
    }
}

/// The lines グリッド線 draws on one page, in CSS pixels from its top-left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridLines {
    /// The rectangle the lines are drawn across.
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// The y of each horizontal line and the x of each vertical one.
    pub rows: Vec<f32>,
    pub columns: Vec<f32>,
}

/// Where the grid lines of a page in section `layout` fall: horizontal ones
/// `vertical_spacing × vertical_every` apart and vertical ones
/// `horizontal_spacing × horizontal_every` apart, from the margins (or the
/// grid's own origin) across the text area, as Word draws them.
pub fn grid_lines(layout: &HeadlessSectionLayout, grid: &DrawingGrid) -> GridLines {
    let band = PageBand::of(layout);
    let (left, top, right, bottom) = match grid.origin {
        Some(_) => (
            0.0,
            0.0,
            points_to_px(layout.width),
            points_to_px(layout.height),
        ),
        None => (band.left, band.top, band.right, band.bottom),
    };
    let (origin_x, origin_y) = grid
        .origin
        .map(|(x, y)| (points_to_px(x), points_to_px(y)))
        .unwrap_or((left, top));
    let step = |spacing: f32, every: u32| points_to_px(spacing * every as f32);
    let mut lines = GridLines {
        left,
        top,
        right,
        bottom,
        ..GridLines::default()
    };
    let row_step = step(grid.vertical_spacing, grid.vertical_every);
    if row_step >= 2.0 {
        let mut y = origin_y;
        while y <= bottom + 0.01 {
            if y >= top - 0.01 {
                lines.rows.push(y);
            }
            y += row_step;
        }
    }
    let column_step = step(grid.horizontal_spacing, grid.horizontal_every);
    if column_step >= 2.0 {
        let mut x = origin_x;
        while x <= right + 0.01 {
            if x >= left - 0.01 {
                lines.columns.push(x);
            }
            x += column_step;
        }
    }
    lines
}

/// ズーム's percentages, as its dialog offers them.
pub const ZOOM_PERCENTS: [u32; 3] = [200, 100, 75];

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> HeadlessSectionLayout {
        HeadlessSectionLayout {
            width: 595.3,
            height: 841.9,
            margin_top: 99.25,
            margin_bottom: 85.05,
            margin_left: 85.05,
            margin_right: 85.05,
            ..HeadlessSectionLayout::default()
        }
    }

    #[test]
    fn fits_follow_the_window() {
        let page = (800.0, 1100.0);
        let text = (100.0, 600.0);
        let area = (864.0, 664.0);
        // Page width: the page across the window less the padding.
        assert_eq!(fit_zoom(ZoomFit::PageWidth, page, text, area), Some(1.0));
        assert_eq!(
            fit_zoom(ZoomFit::TextWidth, page, text, area),
            Some(800.0 / 600.0)
        );
        // One page: the height decides.
        assert_eq!(
            fit_zoom(ZoomFit::OnePage, page, text, area),
            Some(600.0 / 1100.0)
        );
        // Two pages across, each whole.
        let two = fit_zoom(ZoomFit::MultiplePages(2), page, text, (2000.0, 2000.0)).unwrap();
        assert!((two - (2000.0 - 64.0 - PAGE_GAP) / 1600.0).abs() < 1e-4);
        assert_eq!(fit_zoom(ZoomFit::Custom, page, text, area), None);
    }

    #[test]
    fn the_scrolled_to_page_follows_rows_and_columns() {
        // One under another, 1000 apart.
        assert_eq!(page_at_scroll((0.0, 0.0), (0.0, 1000.0), 1, 5, false), 0);
        assert_eq!(page_at_scroll((0.0, 1600.0), (0.0, 1000.0), 1, 5, false), 2);
        // Two across: the second row starts at page 2.
        assert_eq!(page_at_scroll((0.0, 1000.0), (0.0, 1000.0), 2, 5, false), 2);
        // Side to side.
        assert_eq!(page_at_scroll((2100.0, 0.0), (800.0, 0.0), 5, 5, true), 3);
        assert_eq!(page_at_scroll((99999.0, 0.0), (800.0, 0.0), 5, 5, true), 4);
    }

    #[test]
    fn the_band_is_the_page_between_its_margins() {
        let band = PageBand::of(&layout());
        assert!((band.top - points_to_px(99.25)).abs() < 1e-3);
        assert!((band.bottom - points_to_px(841.9 - 85.05)).abs() < 1e-3);
        let page = (points_to_px(595.3), points_to_px(841.9));
        let hidden = band.crop(ViewKind::Print, page);
        assert_eq!((hidden.left, hidden.width), (0.0, page.0));
        assert!((hidden.height - (band.bottom - band.top)).abs() < 1e-3);
        let draft = band.crop(ViewKind::Draft, page);
        assert!((draft.left - (band.left - DRAFT_LEFT_STRIP)).abs() < 1e-3);
        assert!(draft.left + draft.width <= page.0 + 1e-3);
    }

    #[test]
    fn the_grid_comes_from_the_settings() {
        // A Japanese template: 9 pt rows shown every other one (18 pt, the
        // line pitch), no vertical lines.
        let xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:drawingGridHorizontalSpacing w:val="105"/>
            <w:drawingGridVerticalSpacing w:val="180"/>
            <w:displayHorizontalDrawingGridEvery w:val="0"/>
            <w:displayVerticalDrawingGridEvery w:val="2"/>
        </w:settings>"#;
        let grid = DrawingGrid::from_settings(xml);
        assert_eq!(grid.vertical_spacing, 9.0);
        assert_eq!((grid.horizontal_every, grid.vertical_every), (0, 2));
        let lines = grid_lines(&layout(), &grid);
        assert!(lines.columns.is_empty());
        assert!((lines.rows[1] - lines.rows[0] - points_to_px(18.0)).abs() < 1e-3);
        assert!((lines.rows[0] - points_to_px(99.25)).abs() < 1e-3);
        assert!(*lines.rows.last().unwrap() <= points_to_px(841.9 - 85.05) + 0.01);
        // No settings: Word's 9 pt both ways.
        let default = grid_lines(&layout(), &DrawingGrid::from_settings("<w:settings/>"));
        assert!(!default.columns.is_empty());
        assert!((default.columns[1] - default.columns[0] - 12.0).abs() < 1e-3);
    }
}
