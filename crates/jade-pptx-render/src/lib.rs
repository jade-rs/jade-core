//! Paints a laid-out slide through [`jade_painter::Painter`].
//!
//! Nothing here knows which backend it is talking to, so the same code draws
//! a deck on the browser's canvas and through Skia in the native shell — the
//! arrangement the DOCX and XLSX renderers already use.
//!
//! Paint order is the slide's z-order: the layout's shape list, back to front,
//! each shape filling, then stroking, then drawing its text.

use std::cell::RefCell;

use jade_painter::Painter;
use jade_pptx_layout::{
    HeuristicMeasure, PlacedGeometry, PlacedOutline, PlacedShape, RunStyle, SlideLayout,
    TextMeasure, PX_PER_PT,
};

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Painted under the whole slide. `None` leaves whatever is already there.
    pub background: Option<String>,
    /// Drawn where a picture sits, until image drawing lands in `Painter`.
    pub picture_placeholder: Option<String>,
    /// The face used when a run names none.
    pub default_font: String,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            background: Some("#FFFFFF".to_string()),
            picture_placeholder: Some("#E0E0E0".to_string()),
            default_font: "Calibri".to_string(),
        }
    }
}

/// Draws a picture where the painter cannot: the host gets the placed shape
/// (its `image_part` is `Some`) while the shape's transform is applied, and
/// returns whether it drew. `false` paints the placeholder instead.
pub type DrawImage<'a> = dyn FnMut(&PlacedShape) -> bool + 'a;

pub fn render_slide<P: Painter>(
    painter: &P,
    layout: &SlideLayout,
    options: &RenderOptions,
) -> Result<(), P::Error> {
    render_slide_with(painter, layout, options, &mut |_| false)
}

/// [`render_slide`] with a host hook for pictures. `Painter` has no image
/// drawing; a canvas host draws the bitmap itself, in z-order, when asked.
pub fn render_slide_with<P: Painter>(
    painter: &P,
    layout: &SlideLayout,
    options: &RenderOptions,
    draw_image: &mut DrawImage<'_>,
) -> Result<(), P::Error> {
    // The slide's own background wins; the option is the fallback ground.
    if let Some(gradient) = &layout.background_gradient {
        painter.save();
        painter.begin_path();
        painter.rect(0.0, 0.0, layout.width_px, layout.height_px);
        painter.clip();
        fill_gradient_rect(
            painter,
            0.0,
            0.0,
            layout.width_px,
            layout.height_px,
            gradient,
        )?;
        painter.restore();
    } else if let Some(background) = layout
        .background
        .as_deref()
        .or(options.background.as_deref())
    {
        painter.set_fill_style_str(background);
        painter.fill_rect(0.0, 0.0, layout.width_px, layout.height_px);
    }
    painter.set_text_baseline("alphabetic");
    for shape in &layout.shapes {
        render_shape(painter, shape, options, draw_image)?;
    }
    Ok(())
}

/// Sets the stroke state for an outline. A dash pattern is set only when
/// there is one, and the caller clears it after stroking so the next shape
/// starts solid.
fn apply_outline<P: Painter>(painter: &P, outline: &PlacedOutline) {
    painter.set_stroke_style_str(&outline.color);
    painter.set_line_width(outline.width_px);
    if !outline.dash_px.is_empty() {
        painter.set_line_dash(&outline.dash_px);
    }
}

fn clear_outline<P: Painter>(painter: &P, outline: &PlacedOutline) {
    if !outline.dash_px.is_empty() {
        painter.set_line_dash(&[]);
    }
}

fn render_shape<P: Painter>(
    painter: &P,
    shape: &PlacedShape,
    options: &RenderOptions,
    draw_image: &mut DrawImage<'_>,
) -> Result<(), P::Error> {
    let transformed = !shape.group_transforms.is_empty()
        || shape.rotation_rad != 0.0
        || shape.flip_h
        || shape.flip_v;
    if transformed {
        let transform = shape.paint_transform();
        let rect = shape.paint_rect();
        painter.save();
        painter.translate(transform.center.0, transform.center.1)?;
        if transform.rotation_rad != 0.0 {
            painter.rotate(transform.rotation_rad)?;
        }
        if transform.scale != (1.0, 1.0) {
            painter.scale(transform.scale.0, transform.scale.1)?;
        }
        painter.translate(-rect.x - rect.width / 2.0, -rect.y - rect.height / 2.0)?;
    }

    if shape.geometry == PlacedGeometry::Line {
        // A connector is only its stroke; one without an explicit outline
        // still draws, the way PowerPoint gives it the default line.
        let default_outline = PlacedOutline {
            color: "#000000".to_string(),
            width_px: 1.0,
            dash_px: Vec::new(),
        };
        let outline = shape.outline.as_ref().unwrap_or(&default_outline);
        apply_outline(painter, outline);
        painter.begin_path();
        painter.move_to(shape.paint_rect().x, shape.paint_rect().y);
        painter.line_to(
            shape.paint_rect().x + shape.paint_rect().width,
            shape.paint_rect().y + shape.paint_rect().height,
        );
        painter.stroke();
        clear_outline(painter, outline);
        if transformed {
            painter.restore();
        }
        return Ok(());
    }

    // A picture is the host's to draw; when it cannot, its footprint shows
    // as the placeholder colour rather than nothing.
    let host_drew = if shape.image_part.is_some() {
        painter.save();
        geometry_path(painter, shape)?;
        painter.clip();
        let drew = draw_image(shape);
        painter.restore();
        drew
    } else {
        false
    };
    let fill = match (&shape.fill, &shape.image_part) {
        (Some(fill), _) => Some(fill.as_str()),
        (None, Some(_)) if !host_drew => options.picture_placeholder.as_deref(),
        _ => None,
    };
    if let Some(gradient) = shape.gradient.as_ref().filter(|_| !host_drew) {
        // Bands across the shape's box, clipped to its outline: a gradient
        // through the painter's flat fills.
        painter.save();
        geometry_path(painter, shape)?;
        painter.clip();
        fill_gradient_rect(
            painter,
            shape.paint_rect().x,
            shape.paint_rect().y,
            shape.paint_rect().width,
            shape.paint_rect().height,
            gradient,
        )?;
        painter.restore();
    } else if let Some(fill) = fill.filter(|_| !host_drew) {
        painter.set_fill_style_str(fill);
        fill_geometry(painter, shape)?;
    }

    if let Some(outline) = &shape.outline {
        apply_outline(painter, outline);
        stroke_geometry(painter, shape)?;
        clear_outline(painter, outline);
    }

    // Vertical text: the lines were laid out in a frame turned inside the
    // shape, so turn the canvas the same way around the shape's centre and
    // draw them in frame coordinates.
    let turned = shape.text_rotation_deg != 0.0 && !shape.lines.is_empty();
    let (origin_x, origin_y) = if turned {
        let (frame_w, frame_h) = shape.text_frame();
        painter.save();
        painter.translate(
            shape.paint_rect().x + shape.paint_rect().width / 2.0,
            shape.paint_rect().y + shape.paint_rect().height / 2.0,
        )?;
        painter.rotate(shape.text_rotation_deg.to_radians())?;
        painter.translate(-frame_w / 2.0, -frame_h / 2.0)?;
        (0.0, 0.0)
    } else {
        (shape.paint_rect().x, shape.paint_rect().y)
    };
    for line in &shape.lines {
        for run in &line.runs {
            painter.set_fill_style_str(&run.color);
            painter.set_font(&font_shorthand(run, options));
            let x = origin_x + run.x;
            let baseline = origin_y + run.baseline_y;
            painter.fill_text(&run.text, x, baseline)?;
            if run.underline || run.strike {
                // Drawn as rects in the run's colour, the way canvas text
                // decorations have to be: `fillText` has no underline.
                let width = if run.width > 0.0 {
                    run.width
                } else {
                    painter.measure_text_width(&run.text)?
                };
                let size_px = run.size_pt as f64 * PX_PER_PT;
                let thickness = size_px / 15.0;
                if run.underline {
                    painter.fill_rect(x, baseline + size_px * 0.08, width, thickness);
                }
                if run.strike {
                    painter.fill_rect(x, baseline - size_px * 0.3, width, thickness);
                }
            }
        }
    }
    if turned {
        painter.restore();
    }

    if transformed {
        painter.restore();
    }
    Ok(())
}

/// A canvas font shorthand: the Latin face first, the East Asian face as its
/// fallback, so a Japanese run named through `<a:ea>` finds its glyphs even
/// when its `<a:latin>` face has none.
fn font_shorthand(run: &jade_pptx_layout::PlacedRun, options: &RenderOptions) -> String {
    shorthand_for(
        run.size_pt,
        run.bold,
        run.italic,
        run.font.as_deref(),
        run.east_asian_font.as_deref(),
        &options.default_font,
    )
}

/// The one place a run's faces become a font string. Painting and measuring
/// both come through here, so a line cannot be measured against one face and
/// then drawn with another.
fn shorthand_for(
    size_pt: f32,
    bold: bool,
    italic: bool,
    font: Option<&str>,
    east_asian_font: Option<&str>,
    default_font: &str,
) -> String {
    let mut shorthand = String::new();
    if italic {
        shorthand.push_str("italic ");
    }
    if bold {
        shorthand.push_str("bold ");
    }
    let mut faces: Vec<&str> = Vec::new();
    for face in [font, east_asian_font].into_iter().flatten() {
        if !faces.contains(&face) {
            faces.push(face);
        }
    }
    if (faces.is_empty() || font.is_none()) && !faces.contains(&default_font) {
        faces.push(default_font);
    }
    let family_list = faces
        .iter()
        .map(|face| {
            let escaped = face
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\a ")
                .replace('\r', "\\d ")
                .replace('\u{c}', "\\c ");
            format!("'{escaped}'")
        })
        .collect::<Vec<_>>()
        .join(", ");
    shorthand.push_str(&format!("{size_pt}pt {family_list}"));
    shorthand
}

/// Measures through a [`Painter`], so a slide wraps against the faces that
/// will draw it rather than against an approximation.
///
/// Layout runs before painting and cannot ask the painter mid-flight, so the
/// backend hands one of these to `layout_slide`; the same arrangement
/// `jade-xlsx-render` uses. A backend that cannot measure falls back to
/// [`HeuristicMeasure`] per call rather than failing the layout — the damage
/// then shows as line ends that do not match the glyphs, which is far better
/// than a blank slide.
pub struct PainterMeasure<P: Painter> {
    painter: P,
    default_font: String,
    /// The font last pushed to the painter. Setting it per call is the
    /// expensive half of measuring a grapheme at a time.
    last_font: RefCell<String>,
}

impl<P: Painter> PainterMeasure<P> {
    /// `default_font` must match the [`RenderOptions::default_font`] the same
    /// slide is painted with, or runs that name no face measure and draw
    /// differently.
    pub fn new(painter: P, default_font: impl Into<String>) -> Self {
        Self {
            painter,
            default_font: default_font.into(),
            last_font: RefCell::new(String::new()),
        }
    }
}

impl<P: Painter> TextMeasure for PainterMeasure<P> {
    fn measure(&self, text: &str, style: &RunStyle) -> f64 {
        let shorthand = shorthand_for(
            style.size_pt,
            style.bold,
            style.italic,
            style.font.as_deref(),
            style.east_asian_font.as_deref(),
            &self.default_font,
        );
        if *self.last_font.borrow() != shorthand {
            self.painter.set_font(&shorthand);
            *self.last_font.borrow_mut() = shorthand;
        }
        self.painter
            .measure_text_width(text)
            .unwrap_or_else(|_| HeuristicMeasure.measure(text, style))
    }
}

/// How many flat bands approximate a gradient. Enough that the steps are
/// below a pixel on a slide-sized shape at typical zoom.
const GRADIENT_BANDS: usize = 32;

/// Paints `gradient` across the rect as bands along its direction. The
/// caller has clipped to the shape, so the bands only need to cover the box:
/// they are drawn in a frame rotated by the angle around the box centre and
/// sized to the box's diagonal.
fn fill_gradient_rect<P: Painter>(
    painter: &P,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    gradient: &jade_pptx_layout::PlacedGradient,
) -> Result<(), P::Error> {
    let angle = gradient.angle_deg.to_radians();
    // The gradient runs along the rotated x axis; its extent is the box's
    // projection on that axis, so 0 and 1 land on the box's edges.
    let extent = (width * angle.cos()).abs() + (height * angle.sin()).abs();
    let diagonal = (width * width + height * height).sqrt();
    painter.save();
    painter.translate(x + width / 2.0, y + height / 2.0)?;
    painter.rotate(angle)?;
    let band_width = extent / GRADIENT_BANDS as f64;
    for band in 0..GRADIENT_BANDS {
        let position = (band as f64 + 0.5) / GRADIENT_BANDS as f64;
        painter.set_fill_style_str(&gradient_color_at(gradient, position));
        // Overlap the bands by a hair so no seam shows between them.
        painter.fill_rect(
            -extent / 2.0 + band as f64 * band_width,
            -diagonal / 2.0,
            band_width + 0.5,
            diagonal,
        );
    }
    painter.restore();
    Ok(())
}

/// The gradient's colour at `position` (0..1): the first stop before the
/// first position, the last after the last, linear between neighbours. The
/// first and last bands are the stops themselves so the ends stay exact.
fn gradient_color_at(gradient: &jade_pptx_layout::PlacedGradient, position: f64) -> String {
    let stops = &gradient.stops;
    let Some(first) = stops.first() else {
        return "#000000".to_string();
    };
    let Some(last) = stops.last() else {
        return first.color.clone();
    };
    let half_band = 0.5 / GRADIENT_BANDS as f64;
    if position <= first.position + half_band {
        return first.color.clone();
    }
    if position >= last.position - half_band {
        return last.color.clone();
    }
    for pair in stops.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if position <= b.position {
            let span = b.position - a.position;
            let t = if span > 0.0 {
                (position - a.position) / span
            } else {
                1.0
            };
            return mix_css_colors(&a.color, &b.color, t);
        }
    }
    last.color.clone()
}

fn parse_css_color(color: &str) -> Option<(f64, f64, f64, f64)> {
    if let Some(hex) = color.strip_prefix('#') {
        if hex.len() == 6 {
            let channel = |range: std::ops::Range<usize>| {
                u8::from_str_radix(hex.get(range)?, 16).ok().map(f64::from)
            };
            return Some((channel(0..2)?, channel(2..4)?, channel(4..6)?, 1.0));
        }
        return None;
    }
    let inner = color.strip_prefix("rgba(")?.strip_suffix(')')?;
    let mut parts = inner.split(',').map(|part| part.trim().parse::<f64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next()??,
        parts.next()??,
    ))
}

/// Linear interpolation between two CSS colours; an unparsable colour
/// yields the nearer stop unchanged.
fn mix_css_colors(a: &str, b: &str, t: f64) -> String {
    match (parse_css_color(a), parse_css_color(b)) {
        (Some(a), Some(b)) => {
            let mix = |x: f64, y: f64| x + (y - x) * t;
            let (r, g, bl, alpha) = (
                mix(a.0, b.0).round() as u8,
                mix(a.1, b.1).round() as u8,
                mix(a.2, b.2).round() as u8,
                mix(a.3, b.3),
            );
            if (alpha - 1.0).abs() < 1e-6 {
                format!("#{r:02X}{g:02X}{bl:02X}")
            } else {
                format!("rgba({r},{g},{bl},{alpha:.3})")
            }
        }
        _ => {
            if t < 0.5 {
                a.to_string()
            } else {
                b.to_string()
            }
        }
    }
}

/// Builds the shape's outline as the current path, for clipping.
fn geometry_path<P: Painter>(painter: &P, shape: &PlacedShape) -> Result<(), P::Error> {
    match &shape.geometry {
        PlacedGeometry::Rect | PlacedGeometry::Line => {
            painter.begin_path();
            painter.rect(
                shape.paint_rect().x,
                shape.paint_rect().y,
                shape.paint_rect().width,
                shape.paint_rect().height,
            );
        }
        PlacedGeometry::Ellipse => ellipse_path(painter, shape)?,
        PlacedGeometry::RoundRect { adjust } => round_rect_path(painter, shape, *adjust)?,
        PlacedGeometry::Polygon(points) => polygon_path(painter, shape, points),
        PlacedGeometry::Path(subpaths) => subpaths_path(painter, shape, subpaths),
    }
    Ok(())
}

fn subpaths_path<P: Painter>(painter: &P, shape: &PlacedShape, subpaths: &[Vec<(f64, f64)>]) {
    painter.begin_path();
    for points in subpaths {
        for (index, (nx, ny)) in points.iter().enumerate() {
            let x = shape.paint_rect().x + nx * shape.paint_rect().width;
            let y = shape.paint_rect().y + ny * shape.paint_rect().height;
            if index == 0 {
                painter.move_to(x, y);
            } else {
                painter.line_to(x, y);
            }
        }
        painter.close_path();
    }
}

/// The corner radius of a `roundRect`: its `adj` as a fraction of the
/// shorter side (16667 hundred-thousandths by default).
fn round_rect_radius(shape: &PlacedShape, adjust: f64) -> f64 {
    adjust * shape.paint_rect().width.min(shape.paint_rect().height)
}

fn fill_geometry<P: Painter>(painter: &P, shape: &PlacedShape) -> Result<(), P::Error> {
    match &shape.geometry {
        PlacedGeometry::Rect => {
            painter.fill_rect(
                shape.paint_rect().x,
                shape.paint_rect().y,
                shape.paint_rect().width,
                shape.paint_rect().height,
            );
        }
        PlacedGeometry::Ellipse => {
            ellipse_path(painter, shape)?;
            painter.fill();
        }
        PlacedGeometry::RoundRect { adjust } => {
            round_rect_path(painter, shape, *adjust)?;
            painter.fill();
        }
        PlacedGeometry::Polygon(points) => {
            polygon_path(painter, shape, points);
            painter.fill();
        }
        PlacedGeometry::Path(subpaths) => {
            subpaths_path(painter, shape, subpaths);
            painter.fill();
        }
        // A line has no interior; render_shape strokes it before this runs.
        PlacedGeometry::Line => {}
    }
    Ok(())
}

fn stroke_geometry<P: Painter>(painter: &P, shape: &PlacedShape) -> Result<(), P::Error> {
    match &shape.geometry {
        PlacedGeometry::Rect => {
            painter.stroke_rect(
                shape.paint_rect().x,
                shape.paint_rect().y,
                shape.paint_rect().width,
                shape.paint_rect().height,
            );
        }
        PlacedGeometry::Ellipse => {
            ellipse_path(painter, shape)?;
            painter.stroke();
        }
        PlacedGeometry::RoundRect { adjust } => {
            round_rect_path(painter, shape, *adjust)?;
            painter.stroke();
        }
        PlacedGeometry::Polygon(points) => {
            polygon_path(painter, shape, points);
            painter.stroke();
        }
        PlacedGeometry::Path(subpaths) => {
            subpaths_path(painter, shape, subpaths);
            painter.stroke();
        }
        PlacedGeometry::Line => {}
    }
    Ok(())
}

fn polygon_path<P: Painter>(painter: &P, shape: &PlacedShape, points: &[(f64, f64)]) {
    painter.begin_path();
    for (index, (nx, ny)) in points.iter().enumerate() {
        let x = shape.paint_rect().x + nx * shape.paint_rect().width;
        let y = shape.paint_rect().y + ny * shape.paint_rect().height;
        if index == 0 {
            painter.move_to(x, y);
        } else {
            painter.line_to(x, y);
        }
    }
    painter.close_path();
}

fn ellipse_path<P: Painter>(painter: &P, shape: &PlacedShape) -> Result<(), P::Error> {
    painter.begin_path();
    painter.ellipse(
        shape.paint_rect().x + shape.paint_rect().width / 2.0,
        shape.paint_rect().y + shape.paint_rect().height / 2.0,
        shape.paint_rect().width / 2.0,
        shape.paint_rect().height / 2.0,
        0.0,
        0.0,
        std::f64::consts::TAU,
    )
}

fn round_rect_path<P: Painter>(
    painter: &P,
    shape: &PlacedShape,
    adjust: f64,
) -> Result<(), P::Error> {
    use std::f64::consts::{FRAC_PI_2, PI, TAU};
    let radius = round_rect_radius(shape, adjust);
    let (x, y) = (shape.paint_rect().x, shape.paint_rect().y);
    let (w, h) = (shape.paint_rect().width, shape.paint_rect().height);
    painter.begin_path();
    if radius <= 0.0 {
        painter.rect(x, y, w, h);
        return Ok(());
    }
    painter.move_to(x + radius, y);
    painter.line_to(x + w - radius, y);
    painter.ellipse(
        x + w - radius,
        y + radius,
        radius,
        radius,
        0.0,
        -FRAC_PI_2,
        0.0,
    )?;
    painter.line_to(x + w, y + h - radius);
    painter.ellipse(
        x + w - radius,
        y + h - radius,
        radius,
        radius,
        0.0,
        0.0,
        FRAC_PI_2,
    )?;
    painter.line_to(x + radius, y + h);
    painter.ellipse(
        x + radius,
        y + h - radius,
        radius,
        radius,
        0.0,
        FRAC_PI_2,
        PI,
    )?;
    painter.line_to(x, y + radius);
    painter.ellipse(
        x + radius,
        y + radius,
        radius,
        radius,
        0.0,
        PI,
        TAU - FRAC_PI_2,
    )?;
    painter.close_path();
    Ok(())
}
