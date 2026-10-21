//! Paints a laid-out worksheet through [`jade_painter::Painter`].
//!
//! Nothing here knows which backend it is talking to, so the same code draws
//! the sheet on the browser's canvas and through Skia in the native shell — the
//! arrangement the DOCX renderer already uses.
//!
//! Paint order matches Excel: cell fills, then grid lines, then cell borders on
//! top of them, then text clipped to its cell, then the drawing objects over
//! the grid — text boxes, embedded pictures and chart geometry from shared
//! layout. Unavailable objects retain named frames.

pub mod color;
mod icons;

use jade_painter::Painter;
use jade_xlsx_layout::units::{line_height_points, points_to_px};
use jade_xlsx_layout::{
    column_label, HeaderMetrics, PlacedCell, PlacedShape, Rect, ResolvedFill, ResolvedFont,
    SheetLayout,
};
use jade_xlsx_layout::{HeuristicMeasure, TextMeasure};
use jade_xlsx_reader::{
    Border, BorderEdge, CellRange, CellRef, Color, HorizontalAlignment, ShapeKind,
    VerticalAlignment,
};
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Painted under the whole grid. `None` leaves whatever is already there,
    /// which is what an incremental repaint of one region wants.
    pub background: Option<String>,
    pub grid_line_color: String,
    /// Used for text whose style names no colour, and for borders that name
    /// none.
    pub default_text_color: String,
    /// Where the grid's top-left corner lands in painter coordinates.
    ///
    /// With headers this is the corner *inside* them, which is what
    /// [`render_grid`] sets it to.
    pub origin: (f64, f64),
    /// How the row and column bands look, when they are drawn.
    pub header: HeaderStyle,
    /// The translucent wash over the selected range's cells.
    pub selection_fill: String,
    /// The border drawn around the selected range.
    pub selection_border: String,
    /// A dark theme: the theme's Text 1 and Background 1 slots (and the
    /// automatic/system colours) swap to the dark surfaces and light text,
    /// the way Excel's dark mode reads a workbook. Every colour a file states
    /// explicitly is left as the author chose.
    pub dark: bool,
}

/// The look of the row-number and column-letter bands.
#[derive(Debug, Clone)]
pub struct HeaderStyle {
    pub background: String,
    pub text_color: String,
    pub line_color: String,
    /// The band behind a header whose row or column holds the selection.
    pub highlight: String,
    pub font_size_points: f32,
}

impl Default for HeaderStyle {
    fn default() -> Self {
        Self {
            background: "#f5f5f5".to_string(),
            text_color: "#555555".to_string(),
            line_color: "#b0b0b0".to_string(),
            highlight: "#dbe4f0".to_string(),
            font_size_points: 9.0,
        }
    }
}

impl HeaderStyle {
    /// The bands of the dark theme.
    pub fn dark() -> Self {
        Self {
            background: "#26262c".to_string(),
            text_color: "#b4b4bc".to_string(),
            line_color: "#3e3e46".to_string(),
            highlight: "#34405c".to_string(),
            font_size_points: 9.0,
        }
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            background: Some("#ffffff".to_string()),
            grid_line_color: "#d4d4d4".to_string(),
            default_text_color: "#000000".to_string(),
            origin: (0.0, 0.0),
            header: HeaderStyle::default(),
            selection_fill: "rgba(47, 85, 151, 0.12)".to_string(),
            selection_border: "#2f5597".to_string(),
            dark: false,
        }
    }
}

impl RenderOptions {
    /// The dark theme.
    pub fn dark() -> Self {
        Self {
            background: Some("#1b1b1f".to_string()),
            grid_line_color: "#34343b".to_string(),
            default_text_color: "#e6e6ea".to_string(),
            header: HeaderStyle::dark(),
            selection_fill: "rgba(111, 155, 224, 0.18)".to_string(),
            selection_border: "#6f9be0".to_string(),
            dark: true,
            ..Self::default()
        }
    }

    /// A colour as CSS, read through the theme: in the dark theme the
    /// theme's Background 1 becomes the sheet background and Text 1 the
    /// default text colour (tints included — a dimmed Text 1 is still text),
    /// as do the automatic and system colours. Explicit RGB is untouched.
    pub fn color_css(&self, color: Color, fallback: &str) -> String {
        if self.dark {
            let background = self
                .background
                .clone()
                .unwrap_or_else(|| "#1b1b1f".to_string());
            match color {
                Color::Theme { index: 0, .. } => return background,
                Color::Theme { index: 1, .. } => return self.default_text_color.clone(),
                Color::Indexed(65) => return background,
                Color::Indexed(64) | Color::Auto => return self.default_text_color.clone(),
                _ => {}
            }
        }
        color::to_css(color, fallback)
    }
}

/// Paints the whole sheet.
///
/// Convenient for a fixed-size surface such as the PNG the native binary
/// writes. A browser cannot use it on a real workbook — a sheet used down to
/// row 10000 is a quarter of a million pixels tall, past what a canvas will
/// allocate — so the interactive path calls [`render_viewport`] instead. Both
/// go through the same drawing code, so the two cannot drift.
pub fn render_sheet<P: Painter>(
    painter: &P,
    layout: &SheetLayout,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    render_viewport(
        painter,
        layout,
        Rect {
            x: 0.0,
            y: 0.0,
            width: layout.content_width,
            height: layout.content_height,
        },
        options,
    )
}

/// Paints the part of the sheet inside `viewport`.
///
/// `viewport` is in sheet coordinates: its origin is the scroll position, and
/// its size is the surface being painted. What lands on the surface is
/// translated so the viewport's top-left sits at [`RenderOptions::origin`].
pub fn render_viewport<P: Painter>(
    painter: &P,
    layout: &SheetLayout,
    viewport: Rect,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    painter.save();
    let result = render_body(painter, layout, viewport, options);
    painter.restore();
    result
}

fn render_body<P: Painter>(
    painter: &P,
    layout: &SheetLayout,
    viewport: Rect,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    painter.translate(options.origin.0, options.origin.1)?;
    // The body is clipped to its own rectangle so that, with headers drawn
    // around it, a scrolled sheet cannot paint over them.
    painter.begin_path();
    painter.rect(0.0, 0.0, viewport.width, viewport.height);
    painter.clip();
    // Everything below is in sheet coordinates, which is what the layout speaks
    // and what a test can assert without knowing the scroll position.
    painter.translate(-viewport.x, -viewport.y)?;

    if let Some(background) = &options.background {
        painter.set_fill_style_str(background);
        painter.fill_rect(viewport.x, viewport.y, viewport.width, viewport.height);
    }

    // Excel fills hide gridlines; explicit cell borders are painted afterwards.
    if layout.show_grid_lines {
        render_grid_lines(painter, layout, viewport, options);
    }
    let blank_cells = layout.blank_cells_in(viewport);
    for cell in layout.cells_in(viewport).chain(blank_cells.iter()) {
        if let Some(fill) = &cell.fill {
            let sheet_background = options.background.as_deref().unwrap_or("#ffffff");
            render_cell_fill(painter, fill, cell.rect, sheet_background, options);
        }
    }

    if layout.show_grid_lines {
        // A merged cell is one surface: Excel does not draw the grid lines the
        // merged-away columns and rows would otherwise put through it. Painting
        // the background back over the anchor's box erases them without having
        // to split every grid line around the merge.
        if let Some(background) = &options.background {
            for cell in layout.cells_in(viewport) {
                if cell.merge.is_none() || cell.fill.is_some() {
                    continue;
                }
                painter.set_fill_style_str(background);
                painter.fill_rect(cell.rect.x, cell.rect.y, cell.rect.width, cell.rect.height);
            }
        }
    }

    for cell in layout.cells_in(viewport) {
        if let Some(bar) = &cell.data_bar {
            if bar.rect.width > 0.0 && bar.rect.height > 0.0 {
                let color = |rgba: [u8; 4]| {
                    format!(
                        "rgba({}, {}, {}, {})",
                        rgba[0],
                        rgba[1],
                        rgba[2],
                        f64::from(rgba[3]) / 255.0
                    )
                };
                if bar.gradient {
                    let white = [255, 255, 255, bar.color[3]];
                    let (start, end) = if bar.reverse {
                        (white, bar.color)
                    } else {
                        (bar.color, white)
                    };
                    painter.fill_horizontal_gradient_rect(
                        jade_painter::ImageRect {
                            x: bar.rect.x,
                            y: bar.rect.y,
                            width: bar.rect.width,
                            height: bar.rect.height,
                        },
                        start,
                        end,
                    )?;
                } else {
                    painter.set_fill_style_str(&color(bar.color));
                    painter.fill_rect(bar.rect.x, bar.rect.y, bar.rect.width, bar.rect.height);
                }
                if let Some(border) = bar.border {
                    painter.set_stroke_style_str(&color(border));
                    painter.set_line_width(1.0);
                    painter.set_line_dash(&[]);
                    painter.stroke_rect(
                        bar.rect.x + 0.5,
                        bar.rect.y + 0.5,
                        (bar.rect.width - 1.0).max(0.0),
                        (bar.rect.height - 1.0).max(0.0),
                    );
                }
            }
            if let Some(axis) = &bar.axis {
                painter.set_fill_style_str(&format!(
                    "rgba({}, {}, {}, {})",
                    axis.color[0],
                    axis.color[1],
                    axis.color[2],
                    f64::from(axis.color[3]) / 255.0
                ));
                painter.fill_rect(axis.rect.x, axis.rect.y, axis.rect.width, axis.rect.height);
            }
        }
    }

    for cell in layout.cells_in(viewport).chain(blank_cells.iter()) {
        render_borders(painter, &cell.border, cell.rect, options);
    }

    for cell in layout.cells_in(viewport) {
        if let Some(icon) = &cell.icon {
            icons::render_icon(painter, &icon.icon_set, icon.index, icon.rect)?;
        }
    }

    for cell in layout.text_cells_in(viewport) {
        if cell.data_bar.as_ref().is_some_and(|bar| !bar.show_value)
            || cell.icon.as_ref().is_some_and(|icon| !icon.show_value)
        {
            continue;
        }
        render_text(painter, cell, layout.cell_padding_px, options)?;
    }

    for button in layout
        .filter_buttons
        .iter()
        .filter(|button| button.rect.intersects(&viewport))
    {
        render_filter_button(painter, button, options);
    }

    for note in layout.notes_in(viewport) {
        render_note_marker(painter, note.rect);
    }

    for shape in layout.shapes_in(viewport) {
        painter.save();
        let result = render_shape(painter, shape, layout.cell_padding_px, options);
        painter.restore();
        result?;
    }

    Ok(())
}

fn render_filter_button<P: Painter>(
    painter: &P,
    button: &jade_xlsx_layout::filter_button::FilterButton,
    options: &RenderOptions,
) {
    let rect = button.rect;
    painter.set_fill_style_str(&options.header.background);
    painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
    painter.set_stroke_style_str(&options.header.line_color);
    painter.set_line_width(1.0);
    painter.set_line_dash(&[]);
    painter.stroke_rect(
        rect.x + 0.5,
        rect.y + 0.5,
        (rect.width - 1.0).max(0.0),
        (rect.height - 1.0).max(0.0),
    );
    painter.set_fill_style_str(if button.active {
        &options.selection_border
    } else {
        &options.header.text_color
    });
    let x = rect.x + rect.width / 2.0;
    let y = rect.y + rect.height / 2.0;
    let size = (rect.width.min(rect.height) / 2.0 - 2.0).clamp(1.0, 4.0);
    painter.begin_path();
    if button.active {
        painter.move_to(x - size, y - size);
        painter.line_to(x + size, y - size);
        painter.line_to(x + size / 4.0, y);
        painter.line_to(x + size / 4.0, y + size);
        painter.line_to(x - size / 4.0, y + size);
        painter.line_to(x - size / 4.0, y);
    } else {
        painter.move_to(x - size, y - size / 2.0);
        painter.line_to(x + size, y - size / 2.0);
        painter.line_to(x, y + size / 2.0);
    }
    painter.close_path();
    painter.fill();
}

/// The size of a cell's note marker, in px. Excel draws a small right
/// triangle in the top-right corner of a cell that carries a note.
const NOTE_MARKER_PX: f64 = 5.0;

/// Excel paints the marker red whatever the theme is, so this is a constant
/// rather than a themed option: recolouring it would say something about the
/// note that Excel does not say.
const NOTE_MARKER_COLOR: &str = "#ff0000";

/// The red corner triangle marking a cell that carries a note.
fn render_note_marker<P: Painter>(painter: &P, rect: Rect) {
    let size = NOTE_MARKER_PX.min(rect.width).min(rect.height);
    if size <= 0.0 {
        return;
    }
    let right = rect.right();
    painter.set_fill_style_str(NOTE_MARKER_COLOR);
    painter.begin_path();
    painter.move_to(right - size, rect.y);
    painter.line_to(right, rect.y);
    painter.line_to(right, rect.y + size);
    painter.close_path();
    painter.fill();
}

/// A drawing object over the grid. A text box or shape is drawn as Excel's
/// default — a white box with a grey line — and its text. Pictures and charts
/// consume shared layout; unavailable objects retain named frames.
fn render_shape<P: Painter>(
    painter: &P,
    shape: &PlacedShape,
    padding: f64,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    if let Some(chart) = shape.chart.as_ref().filter(|chart| !chart.ops.is_empty()) {
        painter.begin_path();
        painter.rect(
            shape.rect.x,
            shape.rect.y,
            shape.rect.width,
            shape.rect.height,
        );
        painter.clip();
        for op in &chart.ops {
            use jade_xlsx_layout::chart::ChartOp;
            match op {
                ChartOp::Clip { rect } => {
                    painter.save();
                    painter.begin_path();
                    painter.rect(rect.x, rect.y, rect.width, rect.height);
                    painter.clip();
                }
                ChartOp::Restore => painter.restore(),
                ChartOp::Rectangle { rect, fill, stroke } => {
                    if let Some(fill) = fill {
                        painter.set_fill_style_str(fill);
                        painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
                    }
                    if let Some(stroke) = stroke {
                        painter.set_stroke_style_str(stroke);
                        painter.set_line_width(1.0);
                        painter.set_line_dash(&[]);
                        painter.stroke_rect(rect.x, rect.y, rect.width, rect.height);
                    }
                }
                ChartOp::Path {
                    points,
                    closed,
                    fill,
                    stroke,
                    width,
                    dash,
                } => {
                    painter.begin_path();
                    if let Some((x, y)) = points.first() {
                        painter.move_to(*x, *y);
                    }
                    for (x, y) in points.iter().skip(1) {
                        painter.line_to(*x, *y);
                    }
                    if *closed {
                        painter.close_path();
                    }
                    if let Some(fill) = fill {
                        painter.set_fill_style_str(fill);
                        painter.fill();
                    }
                    if let Some(stroke) = stroke {
                        painter.set_stroke_style_str(stroke);
                        painter.set_line_width(*width);
                        painter.set_line_dash(dash);
                        painter.stroke();
                        painter.set_line_dash(&[]);
                    }
                }
                ChartOp::Text {
                    text,
                    x,
                    y,
                    font,
                    color,
                    align: _,
                    rotation,
                } => {
                    painter.set_font(&font_shorthand(font));
                    painter.set_text_baseline("alphabetic");
                    painter.set_fill_style_str(color);
                    if *rotation == 0.0 {
                        painter.fill_text(text, *x, *y)?;
                    } else {
                        painter.save();
                        painter.translate(*x, *y)?;
                        painter.rotate(*rotation)?;
                        let result = painter.fill_text(text, 0.0, 0.0);
                        painter.restore();
                        result?;
                    }
                }
            }
        }
        return Ok(());
    }
    let mut rect = shape.rect;
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return Ok(());
    }
    if let Some(picture) = shape.picture.as_ref() {
        if picture.opacity <= 0.0 {
            return Ok(());
        }
        if picture.supported
            && picture.media.is_some()
            && (picture.image_rect.width <= 0.0 || picture.image_rect.height <= 0.0)
        {
            return Ok(());
        }
        painter.translate(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)?;
        painter.rotate(picture.rotation_radians)?;
        painter.scale(
            if picture.flip_h { -1.0 } else { 1.0 },
            if picture.flip_v { -1.0 } else { 1.0 },
        )?;
        painter.translate(-rect.width / 2.0, -rect.height / 2.0)?;
        rect.x = 0.0;
        rect.y = 0.0;
        if let Some(media) = picture.media.as_ref().filter(|_| {
            picture.supported && picture.image_rect.width > 0.0 && picture.image_rect.height > 0.0
        }) {
            painter.begin_path();
            painter.rect(0.0, 0.0, rect.width, rect.height);
            painter.clip();
            painter.set_fill_style_str("#ffffff");
            painter.set_global_alpha(picture.opacity);
            let destination = picture.image_rect;
            if painter.draw_image(
                jade_painter::ImageSource {
                    key: &media.cache_key,
                    bytes: &media.bytes,
                },
                Some(picture.source),
                jade_painter::ImageRect {
                    x: destination.x,
                    y: destination.y,
                    width: destination.width,
                    height: destination.height,
                },
            )? {
                return Ok(());
            }
            painter.set_global_alpha(1.0);
        }
    }
    if let Some(style) = shape.style.as_ref() {
        return render_styled_shape(painter, shape, style, rect);
    }
    let background = options
        .background
        .clone()
        .unwrap_or_else(|| "#ffffff".to_string());
    match shape.kind {
        ShapeKind::Connector => {
            painter.set_stroke_style_str(&options.default_text_color);
            painter.set_line_width(1.0);
            painter.set_line_dash(&[]);
            painter.begin_path();
            painter.move_to(rect.x, rect.y);
            painter.line_to(rect.right(), rect.bottom());
            painter.stroke();
            return Ok(());
        }
        ShapeKind::TextBox | ShapeKind::Shape => {
            painter.set_fill_style_str(&background);
            painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
            painter.set_stroke_style_str(&options.grid_line_color);
            painter.set_line_width(1.0);
            painter.set_line_dash(&[]);
            painter.stroke_rect(
                rect.x.floor() + 0.5,
                rect.y.floor() + 0.5,
                rect.width.floor(),
                rect.height.floor(),
            );
        }
        ShapeKind::Picture | ShapeKind::Chart | ShapeKind::GraphicFrame | ShapeKind::Group => {
            painter.set_fill_style_str(&background);
            painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
            painter.set_fill_style_str("rgba(128, 128, 128, 0.08)");
            painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
            painter.set_stroke_style_str(&options.header.line_color);
            painter.set_line_width(1.0);
            painter.set_line_dash(&[4.0, 2.0]);
            painter.stroke_rect(
                rect.x.floor() + 0.5,
                rect.y.floor() + 0.5,
                rect.width.floor(),
                rect.height.floor(),
            );
            painter.set_line_dash(&[]);
        }
    }

    painter.begin_path();
    painter.rect(rect.x, rect.y, rect.width, rect.height);
    painter.clip();
    painter.set_font(&font_shorthand(&shape.font));
    painter.set_text_baseline("alphabetic");
    painter.set_fill_style_str(&options.default_text_color);
    let line_height = points_to_px(line_height_points(shape.font.size_points));
    let ascent = f64::from(shape.font.size_points) * 96.0 / 72.0 * 0.8;
    if shape.lines.is_empty() {
        // A frame with no text of its own says what it stands for.
        if matches!(
            shape.kind,
            ShapeKind::Picture | ShapeKind::Chart | ShapeKind::GraphicFrame | ShapeKind::Group
        ) && !shape.name.is_empty()
        {
            painter.set_fill_style_str(&options.header.text_color);
            let width = painter.measure_text_width(&shape.name)?;
            painter.fill_text(
                &shape.name,
                rect.x + (rect.width - width) / 2.0,
                rect.y + (rect.height + ascent) / 2.0,
            )?;
        }
        return Ok(());
    }
    for (index, line) in shape.lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let y = rect.y + padding + line_height * index as f64 + ascent;
        painter.fill_text(line, rect.x + padding, y)?;
    }
    Ok(())
}

fn rgba_css(color: [u8; 4]) -> String {
    format!(
        "rgba({}, {}, {}, {})",
        color[0],
        color[1],
        color[2],
        f64::from(color[3]) / 255.0
    )
}

/// A shape with its own geometry, fill and outline, and its text placed by
/// its alignment and anchor.
fn render_styled_shape<P: Painter>(
    painter: &P,
    shape: &PlacedShape,
    style: &jade_xlsx_layout::shape_style::PlacedShapeStyle,
    rect: Rect,
) -> Result<(), P::Error> {
    use jade_xlsx_layout::shape_style::PathOp;
    let turned = style.rotation_radians != 0.0 || style.flip_h || style.flip_v;
    painter.save();
    painter.translate(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)?;
    painter.rotate(style.rotation_radians)?;
    painter.scale(
        if style.flip_h { -1.0 } else { 1.0 },
        if style.flip_v { -1.0 } else { 1.0 },
    )?;
    painter.translate(-rect.width / 2.0, -rect.height / 2.0)?;
    painter.begin_path();
    for op in &style.path {
        match *op {
            PathOp::Move(x, y) => painter.move_to(x, y),
            PathOp::Line(x, y) => painter.line_to(x, y),
            PathOp::Cubic(a, b, c, d, e, f) => painter.bezier_curve_to(a, b, c, d, e, f),
            PathOp::Close => painter.close_path(),
        }
    }
    if let Some(fill) = style.fill {
        painter.set_fill_style_str(&rgba_css(fill));
        painter.fill();
    }
    if let Some(stroke) = style.stroke {
        painter.set_stroke_style_str(&rgba_css(stroke));
        painter.set_line_width(style.stroke_width);
        painter.set_line_dash(&style.dash);
        painter.stroke();
        painter.set_line_dash(&[]);
        painter.set_fill_style_str(&rgba_css(stroke));
        for arrow in &style.arrows {
            painter.begin_path();
            for (index, (x, y)) in arrow.iter().enumerate() {
                if index == 0 {
                    painter.move_to(*x, *y);
                } else {
                    painter.line_to(*x, *y);
                }
            }
            painter.close_path();
            painter.fill();
        }
    }
    painter.restore();
    if shape.lines.is_empty() {
        return Ok(());
    }
    // Text turns with the shape but is never mirrored.
    painter.save();
    if turned {
        painter.translate(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)?;
        painter.rotate(style.rotation_radians)?;
        painter.translate(-rect.x - rect.width / 2.0, -rect.y - rect.height / 2.0)?;
    }
    painter.set_font(&font_shorthand(&shape.font));
    painter.set_text_baseline("alphabetic");
    painter.set_fill_style_str(&rgba_css(style.text_color));
    let line_height = points_to_px(line_height_points(shape.font.size_points));
    let ascent = f64::from(shape.font.size_points) * 96.0 / 72.0 * 0.8;
    let height = line_height * shape.lines.len() as f64;
    let top = match style.text_anchor.as_str() {
        "ctr" => rect.y + (rect.height - height) / 2.0,
        "b" => rect.bottom() - style.inset_y - height,
        _ => rect.y + style.inset_y,
    };
    for (index, line) in shape.lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let width = painter.measure_text_width(line)?;
        let x = match style.text_align.as_str() {
            "ctr" => rect.x + (rect.width - width) / 2.0,
            "r" => rect.right() - style.inset_x - width,
            _ => rect.x + style.inset_x,
        };
        let y = top + line_height * index as f64 + ascent;
        if let Err(error) = painter.fill_text(line, x, y) {
            painter.restore();
            return Err(error);
        }
    }
    painter.restore();
    Ok(())
}

fn render_cell_fill<P: Painter>(
    painter: &P,
    fill: &ResolvedFill,
    rect: Rect,
    fallback: &str,
    options: &RenderOptions,
) {
    if fill.pattern == "solid" {
        let color = fill
            .foreground
            .map(|color| options.color_css(color, fallback))
            .unwrap_or_else(|| fallback.to_string());
        painter.set_fill_style_str(&color);
        painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
        return;
    }

    let background = fill
        .background
        .map(|color| options.color_css(color, fallback))
        .unwrap_or_else(|| fallback.to_string());
    painter.set_fill_style_str(&background);
    painter.fill_rect(rect.x, rect.y, rect.width, rect.height);

    let foreground = fill
        .foreground
        .map(|color| options.color_css(color, &options.default_text_color))
        .unwrap_or_else(|| options.default_text_color.clone());
    painter.save();
    painter.begin_path();
    painter.rect(rect.x, rect.y, rect.width, rect.height);
    painter.clip();

    match fill.pattern.as_str() {
        "darkGray" => {
            painter.set_fill_style_str(&foreground);
            painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
            render_stipple(painter, rect, &background, 2.0, 2.0, 1.0);
        }
        "mediumGray" => render_stipple(painter, rect, &foreground, 2.0, 1.0, 1.0),
        "lightGray" => render_stipple(painter, rect, &foreground, 2.0, 2.0, 1.0),
        "gray125" => render_stipple(painter, rect, &foreground, 4.0, 2.0, 1.0),
        "gray0625" => render_stipple(painter, rect, &foreground, 4.0, 4.0, 1.0),
        pattern => {
            let dark = pattern.starts_with("dark");
            let spacing = if dark { 4.0 } else { 8.0 };
            painter.set_stroke_style_str(&foreground);
            painter.set_line_width(1.0);
            painter.set_line_dash(&[]);
            painter.begin_path();
            match pattern {
                "darkHorizontal" | "lightHorizontal" => pattern_horizontal(painter, rect, spacing),
                "darkVertical" | "lightVertical" => pattern_vertical(painter, rect, spacing),
                "darkDown" | "lightDown" => pattern_diagonal(painter, rect, spacing, false),
                "darkUp" | "lightUp" => pattern_diagonal(painter, rect, spacing, true),
                "darkGrid" | "lightGrid" => {
                    pattern_horizontal(painter, rect, spacing);
                    pattern_vertical(painter, rect, spacing);
                }
                "darkTrellis" | "lightTrellis" => {
                    pattern_diagonal(painter, rect, spacing, false);
                    pattern_diagonal(painter, rect, spacing, true);
                }
                _ => {}
            }
            painter.stroke();
        }
    }
    painter.restore();
}

fn render_stipple<P: Painter>(
    painter: &P,
    rect: Rect,
    foreground: &str,
    x_spacing: f64,
    y_spacing: f64,
    size: f64,
) {
    painter.set_fill_style_str(foreground);
    let mut y = rect.y + 1.0;
    let mut row = 0usize;
    while y < rect.bottom() {
        let mut x = rect.x
            + 1.0
            + if row.is_multiple_of(2) {
                0.0
            } else {
                x_spacing / 2.0
            };
        while x < rect.right() {
            painter.fill_rect(x, y, size, size);
            x += x_spacing;
        }
        y += y_spacing;
        row += 1;
    }
}

fn pattern_horizontal<P: Painter>(painter: &P, rect: Rect, spacing: f64) {
    let mut y = rect.y + spacing + 0.5;
    while y < rect.bottom() {
        painter.move_to(rect.x, y);
        painter.line_to(rect.right(), y);
        y += spacing;
    }
}

fn pattern_vertical<P: Painter>(painter: &P, rect: Rect, spacing: f64) {
    let mut x = rect.x + spacing + 0.5;
    while x < rect.right() {
        painter.move_to(x, rect.y);
        painter.line_to(x, rect.bottom());
        x += spacing;
    }
}

fn pattern_diagonal<P: Painter>(painter: &P, rect: Rect, spacing: f64, rising: bool) {
    let mut x = rect.x - rect.height;
    while x < rect.right() {
        if rising {
            painter.move_to(x, rect.bottom());
            painter.line_to(x + rect.height, rect.y);
        } else {
            painter.move_to(x, rect.y);
            painter.line_to(x + rect.height, rect.bottom());
        }
        x += spacing;
    }
}

/// Grid lines sit on the half-pixel so a 1px stroke lands on one device pixel
/// instead of straddling two and rendering as a 2px blur.
///
/// Only the lines the viewport touches are drawn. Drawing the whole sheet's
/// worth and letting the clip discard them would put a path segment per row of
/// the workbook into every frame.
fn render_grid_lines<P: Painter>(
    painter: &P,
    layout: &SheetLayout,
    viewport: Rect,
    options: &RenderOptions,
) {
    let Some(range) = layout.visible_range(viewport) else {
        return;
    };
    let top = viewport.y.max(0.0);
    let bottom = viewport.bottom().min(layout.content_height);
    let left = viewport.x.max(0.0);
    let right = viewport.right().min(layout.content_width);
    if bottom <= top || right <= left {
        return;
    }

    painter.set_stroke_style_str(&options.grid_line_color);
    painter.set_line_width(1.0);
    painter.set_line_dash(&[]);
    painter.begin_path();
    for index in range.first_column..=range.last_column {
        let Some(entry) = layout.column(index) else {
            continue;
        };
        let x = entry.offset.floor() + 0.5;
        painter.move_to(x, top);
        painter.line_to(x, bottom);
        // The last column also needs the line that closes it.
        if index == range.last_column {
            let closing = (entry.offset + entry.size).floor() + 0.5;
            painter.move_to(closing, top);
            painter.line_to(closing, bottom);
        }
    }
    for index in range.first_row..=range.last_row {
        let Some(entry) = layout.row(index) else {
            continue;
        };
        let y = entry.offset.floor() + 0.5;
        painter.move_to(left, y);
        painter.line_to(right, y);
        if index == range.last_row {
            let closing = (entry.offset + entry.size).floor() + 0.5;
            painter.move_to(left, closing);
            painter.line_to(right, closing);
        }
    }
    painter.stroke();
}

fn render_borders<P: Painter>(painter: &P, border: &Border, rect: Rect, options: &RenderOptions) {
    let edges = [
        (
            &border.left,
            (rect.x, rect.y),
            (rect.x, rect.bottom()),
            BorderSide::Left,
        ),
        (
            &border.right,
            (rect.right(), rect.y),
            (rect.right(), rect.bottom()),
            BorderSide::Right,
        ),
        (
            &border.top,
            (rect.x, rect.y),
            (rect.right(), rect.y),
            BorderSide::Top,
        ),
        (
            &border.bottom,
            (rect.x, rect.bottom()),
            (rect.right(), rect.bottom()),
            BorderSide::Bottom,
        ),
    ];
    for (edge, start, end, side) in edges {
        let Some(edge) = edge else { continue };
        painter.set_stroke_style_str(
            &edge
                .color
                .map(|color| options.color_css(color, &options.default_text_color))
                .unwrap_or_else(|| options.default_text_color.clone()),
        );
        if edge.style == "double" {
            render_double_border(painter, rect, side);
            continue;
        }
        let Some(stroke) = stroke_for(edge) else {
            continue;
        };
        painter.set_line_width(stroke.width);
        painter.set_line_dash(&stroke.dash);
        painter.begin_path();
        // Odd-width strokes straddle the coordinate, so nudge them the same way
        // grid lines are nudged; even widths are already pixel-aligned.
        let offset = if stroke.width % 2.0 == 1.0 { 0.5 } else { 0.0 };
        painter.move_to(start.0.floor() + offset, start.1.floor() + offset);
        painter.line_to(end.0.floor() + offset, end.1.floor() + offset);
        painter.stroke();
    }
    render_diagonal_borders(painter, border, rect, options);
}

fn render_diagonal_borders<P: Painter>(
    painter: &P,
    border: &Border,
    rect: Rect,
    options: &RenderOptions,
) {
    if !border.diagonal_up && !border.diagonal_down {
        return;
    }
    let Some(edge) = &border.diagonal else {
        return;
    };
    let stroke = if edge.style == "double" {
        Stroke {
            width: 3.0,
            dash: Vec::new(),
        }
    } else {
        let Some(stroke) = stroke_for(edge) else {
            return;
        };
        stroke
    };
    painter.set_stroke_style_str(
        &edge
            .color
            .map(|color| options.color_css(color, &options.default_text_color))
            .unwrap_or_else(|| options.default_text_color.clone()),
    );
    painter.set_line_width(stroke.width);
    painter.set_line_dash(&stroke.dash);
    painter.begin_path();
    if border.diagonal_down {
        painter.move_to(rect.x, rect.y);
        painter.line_to(rect.right(), rect.bottom());
    }
    if border.diagonal_up {
        painter.move_to(rect.x, rect.bottom());
        painter.line_to(rect.right(), rect.y);
    }
    painter.stroke();
}

#[derive(Clone, Copy)]
enum BorderSide {
    Left,
    Right,
    Top,
    Bottom,
}

fn render_double_border<P: Painter>(painter: &P, rect: Rect, side: BorderSide) {
    let left = rect.x.floor() + 0.5;
    let right = rect.right().floor() - 0.5;
    let top = rect.y.floor() + 0.5;
    let bottom = rect.bottom().floor() - 0.5;
    let segments = match side {
        BorderSide::Left => [
            ((left, top), (left, bottom)),
            ((left + 2.0, top), (left + 2.0, bottom)),
        ],
        BorderSide::Right => [
            ((right - 2.0, top), (right - 2.0, bottom)),
            ((right, top), (right, bottom)),
        ],
        BorderSide::Top => [
            ((left, top), (right, top)),
            ((left, top + 2.0), (right, top + 2.0)),
        ],
        BorderSide::Bottom => [
            ((left, bottom - 2.0), (right, bottom - 2.0)),
            ((left, bottom), (right, bottom)),
        ],
    };
    painter.set_line_width(1.0);
    painter.set_line_dash(&[]);
    painter.begin_path();
    for (start, end) in segments {
        painter.move_to(start.0, start.1);
        painter.line_to(end.0, end.1);
    }
    painter.stroke();
}

struct Stroke {
    width: f64,
    dash: Vec<f64>,
}

/// Maps `ST_BorderStyle` to a stroke. `none` produces nothing, so a styleless
/// edge never paints.
fn stroke_for(edge: &BorderEdge) -> Option<Stroke> {
    let (width, dash) = match edge.style.as_str() {
        "none" => return None,
        "hair" => (1.0, vec![1.0, 1.0]),
        "dotted" => (1.0, vec![1.0, 2.0]),
        "dashed" => (1.0, vec![3.0, 2.0]),
        "dashDot" => (1.0, vec![3.0, 2.0, 1.0, 2.0]),
        "dashDotDot" => (1.0, vec![3.0, 2.0, 1.0, 2.0, 1.0, 2.0]),
        "medium" => (2.0, vec![]),
        "mediumDashed" => (2.0, vec![3.0, 2.0]),
        "mediumDashDot" => (2.0, vec![3.0, 2.0, 1.0, 2.0]),
        "mediumDashDotDot" => (2.0, vec![3.0, 2.0, 1.0, 2.0, 1.0, 2.0]),
        "thick" => (3.0, vec![]),
        "double" => return None,
        // `thin` and anything unrecognised.
        _ => (1.0, vec![]),
    };
    Some(Stroke { width, dash })
}

fn render_text<P: Painter>(
    painter: &P,
    cell: &PlacedCell,
    padding: f64,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    if cell.lines.iter().all(String::is_empty) || cell.rect.width <= 0.0 || cell.rect.height <= 0.0
    {
        return Ok(());
    }

    painter.save();
    let result = render_text_body(painter, cell, padding, options);
    painter.restore();
    result
}

fn render_text_body<P: Painter>(
    painter: &P,
    cell: &PlacedCell,
    padding: f64,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    let clip = cell.text_rect();
    painter.begin_path();
    painter.rect(clip.x, clip.y, clip.width, clip.height);
    painter.clip();

    painter.set_font(&font_shorthand(&cell.font));
    painter.set_text_baseline("alphabetic");
    let text_color = cell
        .text_color
        .map(|color| options.color_css(color, &options.default_text_color))
        .unwrap_or_else(|| options.default_text_color.clone());
    painter.set_fill_style_str(&text_color);
    painter.set_stroke_style_str(&text_color);

    let metrics = painter.measure_text_metrics(&cell.lines[0])?;
    // A backend with no real metrics reports zero; fall back to the usual
    // ascent share of an em so placement stays sane rather than collapsing
    // onto the cell's top edge.
    let ascent = if metrics.font_bounding_box_ascent > 0.0 {
        metrics.font_bounding_box_ascent
    } else {
        f64::from(cell.font.size_points) * 96.0 / 72.0 * 0.8
    };

    // The block of lines is what vertical alignment positions; the first
    // baseline then sits one ascent into it. Line spacing comes from layout so
    // that what is drawn matches what the row was sized for.
    let line_height = points_to_px(line_height_points(cell.font.size_points));
    let block_height = line_height * cell.lines.len() as f64;
    let block_top = match cell.alignment.vertical {
        VerticalAlignment::Top => cell.rect.y + padding,
        VerticalAlignment::Center => cell.rect.y + (cell.rect.height - block_height) / 2.0,
        VerticalAlignment::Bottom => cell.rect.bottom() - padding - block_height,
    };
    // An auto-fitted row is exactly as tall as its lines, so the padding has
    // nowhere to go and bottom alignment would push the text out of the cell.
    // Keeping the block inside costs nothing when there is slack and is the
    // difference between clipped and not when there is none.
    let block_top = block_top.clamp(
        cell.rect.y,
        (cell.rect.bottom() - block_height).max(cell.rect.y),
    );
    // Dark view's automatic foreground can cross a light bar/explicit fill.
    // Preserve its color with a thin outline; authored RGB colors stay intact.
    let outline = options.dark
        && matches!(
            cell.text_color,
            None | Some(Color::Auto | Color::Indexed(64) | Color::Theme { index: 1, .. })
        )
        && (cell.fill.is_some() || cell.data_bar.is_some());
    let inner_left = cell.rect.x + padding + cell.text_inset_px;
    let content_right = cell.rect.right().min(cell.text_rect().right());
    let inner_right = content_right - padding - cell.text_inset_px;
    for (index, line) in cell.lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let width = painter.measure_text_width(line)?;
        let x = match cell.alignment.horizontal {
            HorizontalAlignment::Right => inner_right - width,
            HorizontalAlignment::Center => {
                cell.rect.x + (content_right - cell.rect.x - width) / 2.0
            }
            // `General` is resolved during layout, so anything left is left.
            _ => inner_left,
        };
        let y = block_top + line_height * index as f64 + ascent;
        if let Some(angle) = text_rotation_radians(cell.alignment.text_rotation) {
            painter.save();
            let result = (|| {
                painter.translate(x, y)?;
                painter.rotate(angle)?;
                paint_cell_text(painter, line, 0.0, 0.0, outline)?;
                render_text_decorations(painter, cell, 0.0, 0.0, width, ascent);
                Ok(())
            })();
            painter.restore();
            result?;
        } else {
            paint_cell_text(painter, line, x, y, outline)?;
            render_text_decorations(painter, cell, x, y, width, ascent);
        }
    }
    Ok(())
}

fn paint_cell_text<P: Painter>(
    painter: &P,
    line: &str,
    x: f64,
    y: f64,
    outline: bool,
) -> Result<(), P::Error> {
    if outline {
        painter.save();
        painter.set_stroke_style_str("#000000");
        painter.set_line_width(0.75);
        let result = painter.stroke_text(line, x, y);
        painter.restore();
        result?;
    }
    painter.fill_text(line, x, y)
}

fn render_text_decorations<P: Painter>(
    painter: &P,
    cell: &PlacedCell,
    x: f64,
    baseline: f64,
    width: f64,
    ascent: f64,
) {
    if !cell.font.underline && !cell.font.strike {
        return;
    }
    let em = points_to_px(cell.font.size_points);
    painter.set_line_width((em / 16.0).max(0.75));
    painter.set_line_dash(&[]);
    for y in [
        cell.font
            .underline
            .then_some(baseline + (em * 0.08).max(1.0)),
        cell.font.strike.then_some(baseline - ascent * 0.35),
    ]
    .into_iter()
    .flatten()
    {
        painter.begin_path();
        painter.move_to(x, y);
        painter.line_to(x + width, y);
        painter.stroke();
    }
}

/// Converts SpreadsheetML's two positive ranges into Canvas rotation, whose
/// positive direction is clockwise. 1..=90 is above the horizon; 91..=180
/// encodes 1..=90 degrees below it. 255 is stacked text, not an angle.
fn text_rotation_radians(rotation: u16) -> Option<f64> {
    let degrees = match rotation {
        1..=90 => -f64::from(rotation),
        91..=180 => f64::from(rotation - 90),
        _ => return None,
    };
    Some(degrees.to_radians())
}

/// Measures text with a painter, so wrapping happens against the backend that
/// will draw it.
///
/// One implementation serves both shells: the browser passes a canvas painter
/// and the native one passes Skia, and each then wraps where its own glyphs
/// land. Measuring changes the painter's font state, so a caller that draws
/// afterwards has to set its font again — which the renderer does per cell
/// anyway.
pub struct PainterMeasure<P: Painter> {
    painter: P,
    /// The font descriptor last pushed to the painter. Setting it per call is
    /// the expensive part of measuring a grapheme at a time.
    last_font: RefCell<String>,
}

impl<P: Painter> PainterMeasure<P> {
    pub fn new(painter: P) -> Self {
        Self {
            painter,
            last_font: RefCell::new(String::new()),
        }
    }
}

impl<P: Painter> TextMeasure for PainterMeasure<P> {
    fn measure(&self, text: &str, font: &ResolvedFont) -> f64 {
        let shorthand = font_shorthand(font);
        if *self.last_font.borrow() != shorthand {
            self.painter.set_font(&shorthand);
            *self.last_font.borrow_mut() = shorthand;
        }
        self.painter
            .measure_text_width(text)
            // A backend that cannot measure is not a reason to stop laying out;
            // the stand-in wraps plausibly and the failure shows as line ends
            // that do not match the glyphs.
            .unwrap_or_else(|_| HeuristicMeasure.measure(text, font))
    }
}

/// Paints a sheet with its row and column headers.
///
/// `viewport` is the *body*'s window in sheet coordinates; the surface is the
/// headers plus that window. A spreadsheet without headers reads as a table
/// rather than a sheet, and a cell reference has nothing to point at.
/// What the user has selected: a rectangle of cells, and the active cell
/// inside it — the anchor a drag started from, which is the one edits land in
/// and the one the wash leaves clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSelection {
    /// Normalised: `start` is the top-left corner.
    pub range: CellRange,
    pub active: CellRef,
    /// While the fill handle is being dragged: the rectangle the drop would
    /// fill, drawn as a dashed outline until it lands.
    pub fill_preview: Option<CellRange>,
    pub fill_handle: bool,
}

impl GridSelection {
    /// A single selected cell.
    pub fn cell(reference: CellRef) -> Self {
        Self {
            range: CellRange {
                start: reference,
                end: reference,
            },
            active: reference,
            fill_preview: None,
            fill_handle: true,
        }
    }

    /// The rectangle between an anchor and a focus, whichever way the drag
    /// went. The anchor stays active, as it does in Excel.
    pub fn range(anchor: CellRef, focus: CellRef) -> Self {
        Self {
            range: CellRange {
                start: CellRef::new(anchor.column.min(focus.column), anchor.row.min(focus.row)),
                end: CellRef::new(anchor.column.max(focus.column), anchor.row.max(focus.row)),
            },
            active: anchor,
            fill_preview: None,
            fill_handle: true,
        }
    }

    /// The same selection with a fill drag in progress over `range`.
    pub fn with_fill_preview(self, range: CellRange) -> Self {
        Self {
            fill_preview: Some(range),
            ..self
        }
    }

    pub fn without_fill_handle(self) -> Self {
        Self {
            fill_handle: false,
            ..self
        }
    }

    /// The range the selection reads as on the grid: a single selected cell
    /// that anchors a merge is the whole merged rectangle, because that is
    /// the one surface the user selected.
    fn effective_range(&self, layout: &SheetLayout) -> CellRange {
        if self.range.start == self.range.end {
            if let Some(merge) = layout.cell(self.active).and_then(|cell| cell.merge) {
                return merge;
            }
        }
        self.range
    }

    fn contains_column(&self, layout: &SheetLayout, index: u32) -> bool {
        let range = self.effective_range(layout);
        (range.start.column..=range.end.column).contains(&index)
    }

    fn contains_row(&self, layout: &SheetLayout, index: u32) -> bool {
        let range = self.effective_range(layout);
        (range.start.row..=range.end.row).contains(&index)
    }
}

pub fn render_grid<P: Painter>(
    painter: &P,
    layout: &SheetLayout,
    viewport: Rect,
    selected: Option<GridSelection>,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    let metrics = layout.header_metrics();
    let body_origin = (
        options.origin.0 + metrics.row_width,
        options.origin.1 + metrics.column_height,
    );
    let regions = layout.viewport_regions(viewport);
    for (region, offset) in [
        (regions.main, (regions.frozen_width, regions.frozen_height)),
        (regions.frozen_rows, (regions.frozen_width, 0.0)),
        (regions.frozen_columns, (0.0, regions.frozen_height)),
        (regions.corner, (0.0, 0.0)),
    ] {
        if region.width <= 0.0 || region.height <= 0.0 {
            continue;
        }
        let body = RenderOptions {
            origin: (body_origin.0 + offset.0, body_origin.1 + offset.1),
            ..options.clone()
        };
        render_viewport(painter, layout, region, &body)?;
        if let Some(selection) = selected {
            painter.save();
            let result = render_selection(painter, layout, region, selection, &body);
            painter.restore();
            result?;
        }
    }

    painter.save();
    let result = render_headers(painter, layout, viewport, metrics, selected, options);
    painter.restore();
    result
}

/// The resize preview is a shared layout guide, without mutating the workbook
/// or performing a second calculation of the column/row geometry.
pub fn render_resize_guide<P: Painter>(
    painter: &P,
    layout: &SheetLayout,
    viewport: Rect,
    gesture: jade_xlsx_layout::resize::ResizeGesture,
) {
    if let Some(rect) = jade_xlsx_layout::resize::guide_rect(layout, gesture, viewport) {
        let metrics = layout.header_metrics();
        painter.save();
        painter.begin_path();
        painter.rect(
            0.0,
            0.0,
            viewport.width + metrics.row_width,
            viewport.height + metrics.column_height,
        );
        painter.clip();
        painter.set_fill_style_str("#2563eb");
        painter.fill_rect(rect.x, rect.y, rect.width, rect.height);
        painter.restore();
    }
}

/// The selection overlay for one region: a wash over the range's cells with
/// the active one left clear, and a border around the whole range. Clipped to
/// the region the way the body is, so a range that crosses a frozen boundary
/// draws its part in each pane without bleeding into the other.
fn render_selection<P: Painter>(
    painter: &P,
    layout: &SheetLayout,
    viewport: Rect,
    selection: GridSelection,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    let range = selection.effective_range(layout);
    let (Some(first_column), Some(first_row)) = (
        layout.column(range.start.column),
        layout.row(range.start.row),
    ) else {
        return Ok(());
    };
    let (Some(last_column), Some(last_row)) = (
        layout.column(
            range
                .end
                .column
                .min(layout.columns.len().saturating_sub(1) as u32),
        ),
        layout.row(
            range
                .end
                .row
                .min(layout.rows.len().saturating_sub(1) as u32),
        ),
    ) else {
        return Ok(());
    };
    let rect = Rect {
        x: first_column.offset,
        y: first_row.offset,
        width: last_column.offset + last_column.size - first_column.offset,
        height: last_row.offset + last_row.size - first_row.offset,
    };
    if !rect.intersects(&viewport) {
        return Ok(());
    }

    painter.translate(options.origin.0, options.origin.1)?;
    painter.begin_path();
    painter.rect(0.0, 0.0, viewport.width, viewport.height);
    painter.clip();
    painter.translate(-viewport.x, -viewport.y)?;

    // The wash, in strips that leave the active cell clear: the rows above
    // and below it across the full width, and the rest of its own row band.
    // The active cell's rectangle covers its whole merge when it anchors one,
    // so a merged surface is never tinted over.
    let active_rect = match layout.cell(selection.active) {
        Some(placed) => placed.rect,
        None => {
            let (Some(active_column), Some(active_row)) = (
                layout.column(selection.active.column),
                layout.row(selection.active.row),
            ) else {
                return Ok(());
            };
            Rect {
                x: active_column.offset,
                y: active_row.offset,
                width: active_column.size,
                height: active_row.size,
            }
        }
    };
    painter.set_fill_style_str(&options.selection_fill);
    let above = active_rect.y - rect.y;
    if above > 0.0 {
        painter.fill_rect(rect.x, rect.y, rect.width, above);
    }
    let below = rect.y + rect.height - active_rect.bottom();
    if below > 0.0 {
        painter.fill_rect(rect.x, active_rect.bottom(), rect.width, below);
    }
    let left = active_rect.x - rect.x;
    if left > 0.0 {
        painter.fill_rect(rect.x, active_rect.y, left, active_rect.height);
    }
    let right = rect.x + rect.width - active_rect.right();
    if right > 0.0 {
        painter.fill_rect(
            active_rect.right(),
            active_rect.y,
            right,
            active_rect.height,
        );
    }

    painter.set_stroke_style_str(&options.selection_border);
    painter.set_line_width(2.0);
    painter.set_line_dash(&[]);
    painter.stroke_rect(rect.x, rect.y, rect.width, rect.height);

    // The fill handle: a small square on the bottom-right corner, the grip
    // a drag starts from to fill neighbouring cells.
    painter.set_fill_style_str(&options.selection_border);
    if selection.fill_handle {
        painter.fill_rect(
            rect.right() - FILL_HANDLE_SIZE / 2.0,
            rect.bottom() - FILL_HANDLE_SIZE / 2.0,
            FILL_HANDLE_SIZE,
            FILL_HANDLE_SIZE,
        );
    }

    // A fill in progress: the rectangle the drop would fill, dashed so it
    // reads as a proposal rather than a selection.
    if let Some(preview) = selection.fill_preview {
        if let (Some(first_column), Some(first_row), Some(last_column), Some(last_row)) = (
            layout.column(preview.start.column),
            layout.row(preview.start.row),
            layout.column(preview.end.column),
            layout.row(preview.end.row),
        ) {
            painter.set_line_width(1.0);
            painter.set_line_dash(&[4.0, 2.0]);
            painter.stroke_rect(
                first_column.offset,
                first_row.offset,
                last_column.offset + last_column.size - first_column.offset,
                last_row.offset + last_row.size - first_row.offset,
            );
            painter.set_line_dash(&[]);
        }
    }
    Ok(())
}

/// The side of the fill handle square, in px.
const FILL_HANDLE_SIZE: f64 = 6.0;

fn render_headers<P: Painter>(
    painter: &P,
    layout: &SheetLayout,
    viewport: Rect,
    metrics: HeaderMetrics,
    selected: Option<GridSelection>,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    painter.translate(options.origin.0, options.origin.1)?;

    let style = &options.header;
    let surface_width = metrics.row_width + viewport.width;
    let surface_height = metrics.column_height + viewport.height;

    painter.set_fill_style_str(&style.background);
    painter.fill_rect(0.0, 0.0, surface_width, metrics.column_height);
    painter.fill_rect(0.0, 0.0, metrics.row_width, surface_height);

    painter.set_font(&format!("{:.2}pt 'Calibri'", style.font_size_points));
    painter.set_text_baseline("alphabetic");

    let regions = layout.viewport_regions(viewport);
    let range = layout.visible_range(regions.main);
    let em = points_to_px(style.font_size_points);

    if let Some(range) = range {
        // Column letters in the scrolling pane.
        for index in range.first_column..=range.last_column {
            let Some(entry) = layout.column(index) else {
                continue;
            };
            if entry.size <= 0.0 {
                continue;
            }
            let x = metrics.row_width + regions.frozen_width + entry.offset - regions.main.x;
            if selected.is_some_and(|selection| selection.contains_column(layout, index)) {
                painter.set_fill_style_str(&style.highlight);
                painter.fill_rect(x, 0.0, entry.size, metrics.column_height);
            }
            let label = column_label(index);
            painter.set_fill_style_str(&style.text_color);
            let width = painter.measure_text_width(&label)?;
            painter.fill_text(
                &label,
                x + (entry.size - width) / 2.0,
                (metrics.column_height + em * 0.7) / 2.0,
            )?;
        }

        // Row numbers in the scrolling pane.
        for index in range.first_row..=range.last_row {
            let Some(entry) = layout.row(index) else {
                continue;
            };
            if entry.size <= 0.0 {
                continue;
            }
            let y = metrics.column_height + regions.frozen_height + entry.offset - regions.main.y;
            if selected.is_some_and(|selection| selection.contains_row(layout, index)) {
                painter.set_fill_style_str(&style.highlight);
                painter.fill_rect(0.0, y, metrics.row_width, entry.size);
            }
            let label = (index + 1).to_string();
            painter.set_fill_style_str(&style.text_color);
            let width = painter.measure_text_width(&label)?;
            painter.fill_text(
                &label,
                (metrics.row_width - width - 4.0).max(2.0),
                y + (entry.size + em * 0.7) / 2.0,
            )?;
        }
    }

    // Fixed headers sit above the overlays painted for their cells.
    for entry in layout
        .columns
        .iter()
        .take(layout.frozen_columns as usize)
        .take_while(|entry| entry.offset < regions.frozen_width)
    {
        if entry.size <= 0.0 {
            continue;
        }
        let x = metrics.row_width + entry.offset;
        if selected.is_some_and(|selection| selection.contains_column(layout, entry.index)) {
            painter.set_fill_style_str(&style.highlight);
            painter.fill_rect(x, 0.0, entry.size, metrics.column_height);
        }
        let label = column_label(entry.index);
        painter.set_fill_style_str(&style.text_color);
        let width = painter.measure_text_width(&label)?;
        painter.fill_text(
            &label,
            x + (entry.size - width) / 2.0,
            (metrics.column_height + em * 0.7) / 2.0,
        )?;
    }
    for entry in layout
        .rows
        .iter()
        .take(layout.frozen_rows as usize)
        .take_while(|entry| entry.offset < regions.frozen_height)
    {
        if entry.size <= 0.0 {
            continue;
        }
        let y = metrics.column_height + entry.offset;
        if selected.is_some_and(|selection| selection.contains_row(layout, entry.index)) {
            painter.set_fill_style_str(&style.highlight);
            painter.fill_rect(0.0, y, metrics.row_width, entry.size);
        }
        let label = (entry.index + 1).to_string();
        painter.set_fill_style_str(&style.text_color);
        let width = painter.measure_text_width(&label)?;
        painter.fill_text(
            &label,
            (metrics.row_width - width - 4.0).max(2.0),
            y + (entry.size + em * 0.7) / 2.0,
        )?;
    }

    // The lines that close the bands, and the corner between them.
    painter.set_stroke_style_str(&style.line_color);
    painter.set_line_width(1.0);
    painter.set_line_dash(&[]);
    painter.begin_path();
    let bottom = metrics.column_height.floor() + 0.5;
    painter.move_to(0.0, bottom);
    painter.line_to(surface_width, bottom);
    let right = metrics.row_width.floor() + 0.5;
    painter.move_to(right, 0.0);
    painter.line_to(right, surface_height);
    painter.stroke();
    Ok(())
}

/// The CSS font shorthand the painters parse: optional `bold`/`italic`, a size
/// in points, then a quoted family. Same shape the DOCX renderer emits, so both
/// formats exercise one code path in every backend.
pub fn font_shorthand(font: &ResolvedFont) -> String {
    let mut shorthand = String::new();
    if font.bold {
        shorthand.push_str("bold ");
    }
    if font.italic {
        shorthand.push_str("italic ");
    }
    shorthand.push_str(&format!("{:.2}pt '{}'", font.size_points, font.family));
    shorthand
}

#[cfg(test)]
mod tests {
    use super::text_rotation_radians;

    #[test]
    fn spreadsheetml_rotation_ranges_map_above_and_below_the_horizon() {
        let above = text_rotation_radians(45).expect("45 degrees above");
        let below = text_rotation_radians(135).expect("45 degrees below");

        assert!((above + std::f64::consts::FRAC_PI_4).abs() < 0.000_001);
        assert!((below - std::f64::consts::FRAC_PI_4).abs() < 0.000_001);
        assert!(text_rotation_radians(0).is_none());
        assert!(text_rotation_radians(255).is_none());
    }
}
