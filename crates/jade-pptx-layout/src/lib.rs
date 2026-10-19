//! Turns a parsed slide into px geometry and placed text.
//!
//! Everything positional a renderer needs is resolved here: EMU to px, theme
//! colours to CSS, group child coordinate spaces flattened, master and layout
//! shapes stacked under the slide, placeholder text styles inherited down to
//! the run, tables broken into cells, and text wrapped to its box through
//! `jade-text-layout`'s greedy filler — the same filler a spreadsheet cell
//! uses, because a slide text box wants the same wrap.

mod chart;
mod color;
mod geometry;
mod interaction;
mod measure;
mod paragraph_format;
mod placeholder;
mod preset_shapes;
mod presets;
mod style;
mod table;
mod table_style;
mod text;
mod text_format;
pub use paragraph_format::{
    paragraph_format_for_deck_shape, paragraph_indices_for_selection, ParagraphFormatState,
    ParagraphSpacingState,
};
mod text_selection;

use jade_pptx_reader::{
    emu_to_px, Color, Deck, Fill, ListStyle, Outline, Picture, Shape, ShapeItem, ShapeStyle,
    SlideContent, SlideSize, ThemeColors, ThemeFonts, Transform,
};
use serde::{Deserialize, Serialize};

pub use color::resolve_color;
pub use geometry::custom_geometry;
pub use interaction::{
    creation_rect, crop_transform, drag_transform, grouping_transform, DragKind, ObjectTransform,
};
pub use measure::{HeuristicMeasure, RunStyle, TextMeasure};
pub use preset_shapes::is_known as is_known_preset;
pub use table_style::{builtin_table_style, resolve_table_style};
pub use text::{format_number, PlacedLine, PlacedRun};
pub use text_format::{text_format_for_deck_shape, TextFormatState};
pub use text_selection::{Point, TextCaretMove, TextHit, TextSelectionGeometry};

/// CSS px per typographic point, the 96/72 every browser uses.
pub const PX_PER_PT: f64 = 96.0 / 72.0;
/// PowerPoint's default left/right text inset (0.1 inch).
pub const TEXT_INSET_X_EMU: i64 = 91_440;
/// PowerPoint's default top/bottom text inset (0.05 inch).
pub const TEXT_INSET_Y_EMU: i64 = 45_720;
/// Single line spacing as a multiple of the font size.
pub const LINE_SPACING: f64 = 1.2;
/// Where the baseline sits below the line top, as a fraction of the font size.
pub const ASCENT_RATIO: f64 = 0.8;
/// The size a run without `sz` draws at when nothing in the style chain
/// states one.
pub const DEFAULT_SIZE_PT: f32 = 18.0;
/// One outline level's worth of indent, and the width of a bullet's slot —
/// PowerPoint's default 0.375 inch list step, used when neither the paragraph
/// nor its inherited styles state `marL`/`indent`.
pub const LEVEL_INDENT_EMU: i64 = 342_900;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlacedGeometry {
    Rect,
    /// `adjust` is the corner radius as a fraction of the shorter side —
    /// PowerPoint's `adj` over 100000, 0.16667 by default.
    RoundRect {
        adjust: f64,
    },
    Ellipse,
    /// A connector: a stroke from the rect's top-left to its bottom-right
    /// (flips reorient it, as they do every shape).
    Line,
    /// A closed outline in normalized shape coordinates (0..1 × 0..1).
    Polygon(Vec<(f64, f64)>),
    /// Custom geometry: closed subpaths in normalized shape coordinates,
    /// curves already flattened.
    Path(Vec<Vec<(f64, f64)>>),
    /// A preset evaluated from its ECMA-376 definition: each DrawingML path
    /// keeps its own fill mode and stroke, and open subpaths stay open.
    Paths(Vec<PlacedPath>),
}

/// One DrawingML `<a:path>`, flattened, in normalized shape coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedPath {
    pub subpaths: Vec<PlacedSubpath>,
    pub fill: PathFill,
    pub stroked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedSubpath {
    pub points: Vec<(f64, f64)>,
    pub closed: bool,
}

/// A path's `fill` attribute: the shape's fill, none, or that fill shaded
/// (the darker face of a cube, the lit fold of a scroll).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathFill {
    None,
    Norm,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

/// A gradient as the renderer paints it: stops at 0..1 along a direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedGradient {
    /// Clockwise degrees: 0 runs left to right, 90 top to bottom.
    pub angle_deg: f64,
    pub stops: Vec<PlacedGradientStop>,
    /// A path gradient: stop 0 sits on the focus, stop 1 on the box's edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radial: Option<PlacedRadial>,
}

/// Where and how a path gradient radiates, in normalized box coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedRadial {
    /// `true` for `circle` (ellipses), `false` for `rect`/`shape`
    /// (rectangles growing from the focus to the box).
    pub elliptical: bool,
    /// The focus rectangle: left, top, right, bottom in 0..1.
    pub focus: (f64, f64, f64, f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedGradientStop {
    /// 0 at the start of the direction, 1 at its end.
    pub position: f64,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedOutline {
    pub color: String,
    pub width_px: f64,
    /// Canvas `setLineDash` segments in px; empty for a solid stroke.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dash_px: Vec<f64>,
    /// Arrowhead at the stroke's start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_end: Option<PlacedLineEnd>,
    /// Arrowhead at the stroke's end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_end: Option<PlacedLineEnd>,
}

/// An arrowhead ready to draw: its kind and its size in px.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedLineEnd {
    /// `triangle`, `stealth`, `diamond`, `oval` or `arrow`.
    pub kind: String,
    /// Across the line.
    pub width_px: f64,
    /// Along the line.
    pub length_px: f64,
}

/// PowerPoint sizes a line end as a multiple of the line's width, with a
/// floor so a hairline still shows its arrow.
fn place_line_end(end: &jade_pptx_reader::LineEnd, line_width_px: f64) -> PlacedLineEnd {
    let factor = |size: &str| match size {
        "sm" => 2.0,
        "lg" => 5.0,
        _ => 3.0,
    };
    // Like PowerPoint, thin lines size their ends from a ~2pt floor.
    let base = line_width_px.max(2.6);
    PlacedLineEnd {
        kind: end.kind.clone(),
        width_px: (base * factor(&end.width)).max(4.0),
        length_px: (base * factor(&end.length)).max(4.0),
    }
}

/// Which part a placed shape came from. Master and layout shapes draw under
/// the slide's own; a caller listing "what is on this slide" can leave them
/// out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacedOrigin {
    Slide,
    Layout,
    Master,
}

/// One shape, ready to draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedShape {
    /// Geometry's text rectangle in local text-frame pixels, before body insets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_region: Option<Rect>,
    pub name: String,
    pub rect: Rect,
    /// Local paint bounds and ancestor maps; `rect` remains a slide-space bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_rect: Option<Rect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_transforms: Vec<PlacedGroupTransform>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_ids: Vec<u32>,
    /// Clockwise radians around the rect's centre.
    pub rotation_rad: f64,
    pub flip_h: bool,
    pub flip_v: bool,
    pub geometry: PlacedGeometry,
    /// Resolved CSS colour; `None` paints no fill. For a gradient this is
    /// the first stop, the flat fallback.
    pub fill: Option<String>,
    /// The gradient to paint instead of the flat `fill`, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<PlacedGradient>,
    pub outline: Option<PlacedOutline>,
    /// The image part a picture draws, when the backend can.
    pub image_part: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_rect: Option<jade_pptx_reader::SourceRect>,
    pub lines: Vec<PlacedLine>,
    #[serde(default = "PlacedOrigin::slide")]
    pub origin: PlacedOrigin,
    /// The `cNvPr id` of the shape this came from, so a caller can match
    /// placed geometry back to the parsed tree. Table cells and borders
    /// carry their table's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<u32>,
    /// Clockwise degrees the text frame is turned inside the shape: 90 for
    /// vertical text, 270 for `vert270`, 0 otherwise. The lines' coordinates
    /// are in the turned frame, whose width is the shape's height.
    #[serde(default)]
    pub text_rotation_deg: f64,
    /// An outer shadow drawn under the shape's silhouette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<PlacedShadow>,
    /// A picture's opacity when it is translucent (`alphaModFix`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_alpha: Option<f64>,
    /// `<a:pattFill>`: the hatch to draw over `fill` (which holds the two
    /// colours' blend for renderers that draw no patterns).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<PlacedPattern>,
}

/// A preset pattern with its resolved colours.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedPattern {
    pub preset: String,
    pub foreground: String,
    pub background: String,
}

pub(crate) fn resolve_pattern(fill: &Fill, theme: &ThemeColors) -> Option<PlacedPattern> {
    match fill {
        Fill::Pattern {
            preset,
            foreground,
            background,
        } => Some(PlacedPattern {
            preset: preset.clone(),
            foreground: resolve_color(foreground, theme),
            background: resolve_color(background, theme),
        }),
        _ => None,
    }
}

/// `<a:outerShdw>` ready to draw: the offset (in the shape's unrotated
/// frame), the blur radius and the colour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedShadow {
    pub color: String,
    pub dx: f64,
    pub dy: f64,
    pub blur_px: f64,
}

pub(crate) fn place_shadow(
    shadow: Option<&jade_pptx_reader::Shadow>,
    theme: &ThemeColors,
) -> Option<PlacedShadow> {
    let shadow = shadow?;
    let color = resolve_color(&shadow.color, theme);
    if color.ends_with(",0)") {
        // Fully transparent: an explicit "no shadow".
        return None;
    }
    let distance = emu_to_px(shadow.distance_emu);
    let direction = (shadow.direction_deg as f64).to_radians();
    Some(PlacedShadow {
        color,
        dx: distance * direction.cos(),
        dy: distance * direction.sin(),
        blur_px: emu_to_px(shadow.blur_emu),
    })
}

impl PlacedOrigin {
    fn slide() -> Self {
        Self::Slide
    }
}

impl PlacedShape {
    /// Source pixels and destination slide pixels for a cropped bitmap.
    /// Outsets retain transparent margins rather than stretching the image.
    pub fn image_rectangles(&self, width: f64, height: f64) -> Option<(Rect, Rect)> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return None;
        }
        let crop = self.source_rect.unwrap_or_default();
        let left = f64::from(crop.left) / 100_000.0;
        let top = f64::from(crop.top) / 100_000.0;
        let right = 1.0 - f64::from(crop.right) / 100_000.0;
        let bottom = 1.0 - f64::from(crop.bottom) / 100_000.0;
        let crop_width = right - left;
        let crop_height = bottom - top;
        let x = left.max(0.0);
        let y = top.max(0.0);
        let w = right.min(1.0) - x;
        let h = bottom.min(1.0) - y;
        if crop_width <= 0.0 || crop_height <= 0.0 || w <= 0.0 || h <= 0.0 {
            return None;
        }
        Some((
            Rect {
                x: x * width,
                y: y * height,
                width: w * width,
                height: h * height,
            },
            Rect {
                x: self.paint_rect().x + (x - left) / crop_width * self.paint_rect().width,
                y: self.paint_rect().y + (y - top) / crop_height * self.paint_rect().height,
                width: w / crop_width * self.paint_rect().width,
                height: h / crop_height * self.paint_rect().height,
            },
        ))
    }

    /// How far the text block's bottom sits below the shape's top, in px —
    /// the line boxes, not the glyphs. Blank lines do not count: a
    /// decorative bar with an empty paragraph shows nothing, so it has
    /// nothing to overflow with. Zero for a shape without visible text.
    pub fn text_bottom_px(&self) -> f64 {
        self.lines
            .iter()
            .filter(|line| line.runs.iter().any(|run| !run.text.trim().is_empty()))
            .map(|line| line.top + line.height)
            .fold(0.0, f64::max)
    }

    /// How far the text overflows the geometry's text region, in px; zero when
    /// it fits.
    pub fn text_overflow_px(&self) -> f64 {
        let bottom = self
            .text_region
            .map_or(self.text_frame().1, |r| r.y + r.height);
        (self.text_bottom_px() - bottom).max(0.0)
    }

    /// The frame the text is laid out in: the rect's size, swapped when the
    /// text is turned.
    pub fn text_frame(&self) -> (f64, f64) {
        if self.text_rotation_deg == 90.0 || self.text_rotation_deg == 270.0 {
            (self.paint_rect().height, self.paint_rect().width)
        } else {
            (self.paint_rect().width, self.paint_rect().height)
        }
    }

    /// How far the widest line's text runs past its text region's right edge, in
    /// px; zero when every line fits. Only unwrapped text can do this.
    pub fn text_overflow_right_px(&self) -> f64 {
        self.lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .filter(|run| !run.text.trim().is_empty())
            .map(|run| {
                run.x + run.width
                    - self
                        .text_region
                        .map_or(self.text_frame().0, |r| r.x + r.width)
            })
            .fold(0.0, f64::max)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideLayout {
    pub width_px: f64,
    pub height_px: f64,
    /// Resolved CSS colour of the slide's background — its own, or the
    /// layout's, or the master's. For a gradient, its first stop.
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_gradient: Option<PlacedGradient>,
    /// In paint order: master shapes, layout shapes, then the slide's own.
    pub shapes: Vec<PlacedShape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<PlacedGroup>,
}

mod group;
mod justification;
pub use group::{PlacedGroupTransform, PlacedPaintTransform};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedGroup {
    pub id: u32,
    pub transform: ObjectTransform,
}

/// Resolves any fill to a flat CSS colour: a gradient's first stop.
pub(crate) fn resolve_fill(fill: &Fill, theme: &ThemeColors) -> Option<String> {
    match fill {
        Fill::Solid(color) => Some(resolve_color(color, theme)),
        Fill::Gradient { stops, .. } => stops.first().map(|stop| resolve_color(&stop.color, theme)),
        // A hatch reads as the blend of its two colours at slide scale.
        Fill::Pattern {
            foreground,
            background,
            ..
        } => Some(color::mix(
            &resolve_color(foreground, theme),
            &resolve_color(background, theme),
            0.5,
        )),
        Fill::None | Fill::Group | Fill::Picture { .. } => None,
    }
}

/// The gradient a fill paints, when it is one.
pub(crate) fn resolve_gradient(fill: &Fill, theme: &ThemeColors) -> Option<PlacedGradient> {
    match fill {
        Fill::Gradient {
            stops,
            angle_deg,
            path,
        } if !stops.is_empty() => Some(PlacedGradient {
            radial: path.as_ref().map(|path| {
                let left = (path.left as f64).clamp(0.0, 1.0);
                let top = (path.top as f64).clamp(0.0, 1.0);
                let right = (1.0 - path.right as f64).clamp(left, 1.0);
                let bottom = (1.0 - path.bottom as f64).clamp(top, 1.0);
                PlacedRadial {
                    elliptical: path.kind == "circle",
                    focus: (left, top, right, bottom),
                }
            }),
            angle_deg: angle_deg
                .filter(|angle| angle.is_finite())
                .map(|angle| angle as f64)
                .unwrap_or(0.0),
            stops: stops
                .iter()
                .map(|stop| PlacedGradientStop {
                    position: (stop.position_thousandths as f64 / 100_000.0).clamp(0.0, 1.0),
                    color: resolve_color(&stop.color, theme),
                })
                .collect(),
        }),
        _ => None,
    }
}

fn substitute_color(target: &Color, color: &Color) -> Color {
    match target {
        Color::Scheme(name) if name == "phClr" => color.clone(),
        Color::Adjusted {
            base,
            alpha,
            lum_mod,
            lum_off,
            shade,
            tint,
        } => Color::Adjusted {
            base: Box::new(substitute_color(base, color)),
            alpha: *alpha,
            lum_mod: *lum_mod,
            lum_off: *lum_off,
            shade: *shade,
            tint: *tint,
        },
        Color::Transformed { base, ops } => Color::Transformed {
            base: Box::new(substitute_color(base, color)),
            ops: ops.clone(),
        },
        other => other.clone(),
    }
}

/// A theme fill style with the reference colour in place of `phClr`.
fn substitute_placeholder_color(fill: &Fill, color: &Color) -> Fill {
    match fill {
        Fill::Solid(target) => Fill::Solid(substitute_color(target, color)),
        Fill::Gradient {
            stops,
            angle_deg,
            path,
        } => Fill::Gradient {
            path: path.clone(),
            stops: stops
                .iter()
                .map(|stop| jade_pptx_reader::GradientStop {
                    position_thousandths: stop.position_thousandths,
                    color: substitute_color(&stop.color, color),
                })
                .collect(),
            angle_deg: *angle_deg,
        },
        Fill::None => Fill::None,
        Fill::Picture { .. } | Fill::Group => fill.clone(),
        Fill::Pattern {
            preset,
            foreground,
            background,
        } => Fill::Pattern {
            preset: preset.clone(),
            foreground: substitute_color(foreground, color),
            background: substitute_color(background, color),
        },
    }
}

/// The fill a shape's style reference supplies: the theme's fill style at
/// that index with the reference colour, or a plain solid of the colour
/// when the theme does not define the style.
fn style_fill(style: &ShapeStyle, inherited: &InheritedContent<'_>) -> Option<Fill> {
    if style.fill_index == 0 {
        return None;
    }
    let color = style.fill_color.as_ref()?;
    let (styles, index) = if style.fill_index >= 1000 {
        (
            inherited.theme_background_fill_styles,
            style.fill_index - 1000,
        )
    } else {
        (inherited.theme_fill_styles, style.fill_index)
    };
    let from_theme = styles
        .and_then(|styles| styles.get(index.checked_sub(1)? as usize))
        .map(|fill| substitute_placeholder_color(fill, color));
    Some(from_theme.unwrap_or_else(|| Fill::Solid(color.clone())))
}

/// Where a slide's implicit facts come from: its layout's tree first, then
/// the master's, then the presentation.
#[derive(Debug, Clone, Copy, Default)]
pub struct InheritedContent<'a> {
    pub layout: Option<&'a SlideContent>,
    pub master: Option<&'a SlideContent>,
    /// Resolves `+mj-lt`-style typeface references.
    pub theme_fonts: Option<&'a ThemeFonts>,
    /// `<p:defaultTextStyle>`, the last stop for text outside placeholders.
    pub default_text_style: Option<&'a ListStyle>,
    /// The deck's own table styles, consulted before the built-ins.
    pub table_styles: Option<&'a [jade_pptx_reader::TableStyle]>,
    /// The theme's `fillStyleLst`, for `fillRef idx` 1–3.
    pub theme_fill_styles: Option<&'a [Fill]>,
    /// The theme's `bgFillStyleLst`, for `fillRef idx` 1001–1003.
    pub theme_background_fill_styles: Option<&'a [Fill]>,
    pub theme_line_styles: Option<&'a [Outline]>,
    /// The theme's effect styles, for a shape style's `effectRef`.
    pub theme_effect_styles: Option<&'a [Option<jade_pptx_reader::Shadow>]>,
}

pub fn layout_slide(
    slide_size: SlideSize,
    content: &SlideContent,
    theme: &ThemeColors,
    measure: &dyn TextMeasure,
) -> SlideLayout {
    layout_slide_inheriting(
        slide_size,
        content,
        InheritedContent::default(),
        theme,
        measure,
    )
}

/// Lays out one slide of a parsed deck, with everything the deck knows —
/// layout and master trees, theme fonts, the default text style. `index` is
/// 0-based; `None` when the deck has no such slide.
pub fn layout_deck_slide(
    deck: &Deck,
    index: usize,
    measure: &dyn TextMeasure,
) -> Option<SlideLayout> {
    let slide = deck.slides.get(index)?;
    let slide_theme = deck.theme_for_slide(index);
    Some(layout_slide_inheriting(
        deck.slide_size,
        &slide.content,
        InheritedContent {
            layout: slide.layout_content.as_ref(),
            master: slide.master_content.as_ref(),
            theme_fonts: Some(slide_theme.fonts),
            default_text_style: Some(&deck.default_text_style),
            table_styles: Some(&deck.table_styles),
            theme_fill_styles: Some(slide_theme.fill_styles),
            theme_background_fill_styles: Some(slide_theme.background_fill_styles),
            theme_line_styles: Some(slide_theme.line_styles),
            theme_effect_styles: Some(slide_theme.effect_styles),
        },
        slide_theme.colors,
        measure,
    ))
}

/// [`layout_slide`] with the slide's layout/master trees available, so a
/// placeholder that states no `a:xfrm` of its own — which is how real decks
/// position their titles and bodies — still lands where PowerPoint puts it,
/// its text inherits the master's styles, and the master's own shapes draw
/// underneath.
pub fn layout_slide_inheriting(
    slide_size: SlideSize,
    content: &SlideContent,
    inherited: InheritedContent<'_>,
    theme: &ThemeColors,
    measure: &dyn TextMeasure,
) -> SlideLayout {
    // A slide's (or its layout's) colour map override redirects bg1/tx1 for
    // everything on it, the master's own shapes included.
    let overridden_theme;
    let theme = match content.effective_color_map(inherited.layout) {
        Some(map) => {
            overridden_theme = theme.with_color_map(map.clone());
            &overridden_theme
        }
        None => theme,
    };
    let context = Context {
        inherited: &inherited,
        theme,
        measure,
    };
    let mut shapes = Vec::new();

    // Master and layout shapes that are not placeholders draw under the
    // slide (logos, bands, footers drawn as plain shapes). A placeholder on
    // a master is a template for the slide's own, never drawn itself.
    if content.show_master_shapes {
        let layout_shows_master = inherited
            .layout
            .map(|layout| layout.show_master_shapes)
            .unwrap_or(true);
        if layout_shows_master {
            if let Some(master) = inherited.master {
                place_items(
                    &master.shapes,
                    None,
                    &context,
                    PlacedOrigin::Master,
                    &mut shapes,
                );
            }
        }
        if let Some(layout) = inherited.layout {
            place_items(
                &layout.shapes,
                None,
                &context,
                PlacedOrigin::Layout,
                &mut shapes,
            );
        }
    }
    place_items(
        &content.shapes,
        None,
        &context,
        PlacedOrigin::Slide,
        &mut shapes,
    );

    // The nearest part that states a background wins, whether it states a
    // fill or a reference to the theme's background fill styles.
    let background_fill = [Some(content), inherited.layout, inherited.master]
        .into_iter()
        .flatten()
        .find(|tree| tree.background.is_some() || tree.background_ref.is_some())
        .and_then(|tree| match (&tree.background, &tree.background_ref) {
            (Some(fill), _) => Some(fill.clone()),
            (None, Some((index, color))) => Some(
                inherited
                    .theme_background_fill_styles
                    .and_then(|styles| styles.get(index.checked_sub(1001)? as usize))
                    .map(|fill| substitute_placeholder_color(fill, color))
                    .unwrap_or_else(|| Fill::Solid(color.clone())),
            ),
            (None, None) => None,
        });
    let background_fill = background_fill.as_ref();
    let background = background_fill.and_then(|fill| resolve_fill(fill, theme));
    let background_gradient = background_fill.and_then(|fill| resolve_gradient(fill, theme));
    // A picture background draws as a slide-sized picture under everything.
    if let Some(Fill::Picture {
        image_part: Some(part),
        ..
    }) = background_fill
    {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: slide_size.width_px(),
            height: slide_size.height_px(),
        };
        shapes.insert(
            0,
            PlacedShape {
                text_region: None,
                local_rect: None,
                group_transforms: Vec::new(),
                group_ids: Vec::new(),
                name: "Background".to_string(),
                rect,
                rotation_rad: 0.0,
                flip_h: false,
                flip_v: false,
                geometry: PlacedGeometry::Rect,
                fill: None,
                gradient: None,
                outline: None,
                image_part: Some(part.clone()),
                source_rect: None,
                lines: Vec::new(),
                origin: PlacedOrigin::Master,
                source_id: None,
                text_rotation_deg: 0.0,
                shadow: None,
                image_alpha: None,
                pattern: None,
            },
        );
    }

    SlideLayout {
        groups: content
            .shapes
            .iter()
            .filter_map(|item| match item {
                ShapeItem::Group(group) => Some(PlacedGroup {
                    id: group.id?,
                    transform: PlacedGroupTransform::from(group.transform.as_ref()?)
                        .object_transform(),
                }),
                _ => None,
            })
            .collect(),
        width_px: slide_size.width_px(),
        height_px: slide_size.height_px(),
        background,
        background_gradient,
        shapes,
    }
}

/// Everything shape placement needs beyond the shape itself.
pub(crate) struct Context<'a> {
    pub(crate) inherited: &'a InheritedContent<'a>,
    pub(crate) theme: &'a ThemeColors,
    pub(crate) measure: &'a dyn TextMeasure,
}

/// Resolve a selected placeholder chain before asking it for geometry.
fn inherited_transform(
    ph: &jade_pptx_reader::Placeholder,
    inherited: &InheritedContent<'_>,
) -> Option<Transform> {
    placeholder::parents(ph, inherited)
        .into_iter()
        .flatten()
        .find_map(|item| placeholder::definition(item).and_then(|(_, t)| t))
}

/// Authored transform, falling back to the same placeholder inheritance used
/// by layout. Writers use this before autofit changes the displayed rectangle.
pub fn resolve_transform(
    transform: Option<Transform>,
    placeholder: Option<&jade_pptx_reader::Placeholder>,
    inherited: &InheritedContent<'_>,
) -> Option<Transform> {
    transform
        .or_else(|| placeholder.and_then(|placeholder| inherited_transform(placeholder, inherited)))
}

/// Primitive placement basis. Group maps are resolved after local layout.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GroupSpace {
    offset_emu: (f64, f64),
    pub(crate) scale: (f64, f64),
}

impl GroupSpace {
    fn identity() -> Self {
        Self {
            offset_emu: (0.0, 0.0),
            scale: (1.0, 1.0),
        }
    }

    fn apply(&self, x: i64, y: i64) -> (f64, f64) {
        (
            self.offset_emu.0 + x as f64 * self.scale.0,
            self.offset_emu.1 + y as f64 * self.scale.1,
        )
    }
}

fn place_items(
    items: &[ShapeItem],
    space: Option<GroupSpace>,
    context: &Context<'_>,
    origin: PlacedOrigin,
    out: &mut Vec<PlacedShape>,
) {
    let space = space.unwrap_or_else(GroupSpace::identity);
    for item in items {
        match item {
            ShapeItem::Shape(shape) => {
                // Placeholders on a master or layout are templates, not
                // content: PowerPoint draws them only through the slide's own
                // placeholder that inherits from them.
                if origin != PlacedOrigin::Slide && shape.placeholder.is_some() {
                    continue;
                }
                if let Some(placed) = place_shape(shape, &space, context, origin) {
                    out.push(placed);
                }
            }
            ShapeItem::Picture(picture) => {
                if origin != PlacedOrigin::Slide && picture.placeholder.is_some() {
                    continue;
                }
                if let Some(placed) = place_picture(picture, &space, context, origin) {
                    out.push(placed);
                }
            }
            ShapeItem::Group(group) => {
                let start = out.len();
                place_items(&group.children, None, context, origin, out);
                if let Some(transform) = &group.transform {
                    for placed in &mut out[start..] {
                        placed.enter_group(transform.into(), group.id);
                    }
                }
            }
            ShapeItem::Table(table) => {
                table::place_table(table, &space, context, origin, out);
            }
            ShapeItem::Chart(chart) => {
                out.extend(place_chart(chart, &space, context, origin));
            }
        }
    }
}

/// The frame colour a chart draws with until charts draw as charts.
const CHART_FRAME_COLOR: &str = "#BFBFBF";
/// PowerPoint's default chart title size.
const CHART_TITLE_SIZE_PT: f32 = 14.0;

/// A chart places as its frame with the title across the top, then its
/// plot — bars, lines or sectors from the cached series.
fn place_chart(
    chart: &jade_pptx_reader::Chart,
    space: &GroupSpace,
    context: &Context<'_>,
    origin: PlacedOrigin,
) -> Vec<PlacedShape> {
    let Some(transform) = chart.transform.as_ref() else {
        return Vec::new();
    };
    let rect = placed_rect(transform, space);
    let has_title = chart
        .title
        .as_deref()
        .is_some_and(|title| !title.trim().is_empty());
    let lines = match &chart.title {
        Some(title) if !title.trim().is_empty() => {
            let body = jade_pptx_reader::TextBody {
                paragraphs: vec![jade_pptx_reader::TextParagraph {
                    align: Some(jade_pptx_reader::TextAlign::Center),
                    runs: vec![jade_pptx_reader::TextRun {
                        text: title.clone(),
                        bold: true,
                        bold_stated: Some(true),
                        size_pt: Some(CHART_TITLE_SIZE_PT),
                        ..jade_pptx_reader::TextRun::default()
                    }],
                    ..jade_pptx_reader::TextParagraph::default()
                }],
                ..jade_pptx_reader::TextBody::default()
            };
            let text_context = style::text_context_for_table(context);
            text::layout_text(&body, rect.width, rect.height, &text_context).0
        }
        _ => Vec::new(),
    };
    let mut placed = vec![PlacedShape {
        text_region: None,
        local_rect: None,
        group_transforms: Vec::new(),
        group_ids: Vec::new(),
        name: chart.name.clone(),
        rect,
        rotation_rad: rotation_rad(transform),
        flip_h: transform.flip_h,
        flip_v: transform.flip_v,
        geometry: PlacedGeometry::Rect,
        fill: None,
        gradient: None,
        outline: Some(PlacedOutline {
            color: CHART_FRAME_COLOR.to_string(),
            width_px: 1.0,
            dash_px: Vec::new(),
            head_end: None,
            tail_end: None,
        }),
        image_part: None,
        source_rect: None,
        lines,
        origin,
        source_id: chart.id,
        text_rotation_deg: 0.0,
        shadow: None,
        image_alpha: None,
        pattern: None,
    }];
    placed.extend(chart::plot_chart(chart, rect, has_title, context, origin));
    placed
}

pub(crate) fn placed_rect(transform: &Transform, space: &GroupSpace) -> Rect {
    let (x_emu, y_emu) = space.apply(transform.offset_emu.0, transform.offset_emu.1);
    let (cx, cy) = transform.extent_emu;
    Rect {
        x: emu_to_px_f(x_emu),
        y: emu_to_px_f(y_emu),
        width: emu_to_px_f(cx as f64 * space.scale.0),
        height: emu_to_px_f(cy as f64 * space.scale.1),
    }
}

pub(crate) fn emu_to_px_f(emu: f64) -> f64 {
    emu / jade_pptx_reader::EMU_PER_INCH * 96.0
}

fn rotation_rad(transform: &Transform) -> f64 {
    (transform.rotation_60000ths as f64 / 60_000.0).to_radians()
}

/// The Office theme's three line widths, indexed by `lnRef idx`, for a
/// style-supplied outline whose shape states no width of its own.
fn theme_line_width_emu(index: u32) -> i64 {
    match index {
        1 => 6_350,
        2 => 12_700,
        _ => 19_050,
    }
}

/// PowerPoint's preset dash patterns, in multiples of the line width.
fn dash_pattern(name: Option<&str>) -> &'static [f64] {
    match name {
        Some("dash") => &[4.0, 3.0],
        Some("dashDot") => &[4.0, 3.0, 1.0, 3.0],
        Some("dot") => &[1.0, 3.0],
        Some("lgDash") => &[8.0, 3.0],
        Some("lgDashDot") => &[8.0, 3.0, 1.0, 3.0],
        Some("lgDashDotDot") => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        Some("sysDash") => &[3.0, 1.0],
        Some("sysDashDot") => &[3.0, 1.0, 1.0, 1.0],
        Some("sysDashDotDot") => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        Some("sysDot") => &[1.0, 1.0],
        _ => &[],
    }
}

/// Resolves an outline, letting the shape's style supply colour and width
/// for what the outline itself leaves unsaid. `None` when nothing draws.
pub(crate) fn resolve_outline(
    outline: Option<&Outline>,
    style: Option<&ShapeStyle>,
    theme: &ThemeColors,
    styles: Option<&[Outline]>,
) -> Option<PlacedOutline> {
    if outline.is_some_and(|l| l.no_fill) {
        return None;
    }
    let legacy = styles.is_none_or(|styles| styles.is_empty());
    let reference = style.filter(|s| s.line_index > 0);
    let matrix =
        reference.and_then(|s| styles.and_then(|ls| ls.get(s.line_index.checked_sub(1)? as usize)));
    let own_color = outline.and_then(|l| l.color.as_ref());
    if own_color.is_none() && matrix.is_some_and(|l| l.no_fill) {
        return None;
    }
    let color = own_color
        .cloned()
        .or_else(|| {
            matrix.and_then(|l| l.color.as_ref()).map(|c| {
                reference
                    .and_then(|s| s.line_color.as_ref())
                    .map_or_else(|| c.clone(), |r| substitute_color(c, r))
            })
        })
        .or_else(|| {
            legacy
                .then(|| reference.and_then(|s| s.line_color.clone()))
                .flatten()
        })?;
    let width_emu = outline
        .and_then(|l| l.width_emu)
        .or_else(|| matrix.and_then(|l| l.width_emu))
        .unwrap_or_else(|| {
            if legacy && own_color.is_none() {
                reference.map_or(12700, |s| theme_line_width_emu(s.line_index))
            } else {
                12700
            }
        });
    let width_px = emu_to_px(width_emu);
    let dash = outline
        .and_then(|l| l.dash.as_deref())
        .or_else(|| matrix.and_then(|l| l.dash.as_deref()));
    Some(PlacedOutline {
        color: resolve_color(&color, theme),
        width_px,
        dash_px: dash_pattern(dash)
            .iter()
            .map(|unit| unit * width_px)
            .collect(),
        head_end: outline
            .and_then(|l| l.head_end.as_ref())
            .or_else(|| matrix.and_then(|l| l.head_end.as_ref()))
            .map(|end| place_line_end(end, width_px)),
        tail_end: outline
            .and_then(|l| l.tail_end.as_ref())
            .or_else(|| matrix.and_then(|l| l.tail_end.as_ref()))
            .map(|end| place_line_end(end, width_px)),
    })
}

fn place_shape(
    shape: &Shape,
    space: &GroupSpace,
    context: &Context<'_>,
    origin: PlacedOrigin,
) -> Option<PlacedShape> {
    let transform = resolve_transform(
        shape.transform,
        shape.placeholder.as_ref(),
        context.inherited,
    )?;
    let transform = &transform;
    let mut rect = placed_rect(transform, space);
    let visual = placeholder::visual(shape, context.inherited);
    let text_context = style::text_context_for_shape(shape, context);
    let text_rotation_deg = match text_context.vertical {
        Some(jade_pptx_reader::TextVertical::EastAsian)
        | Some(jade_pptx_reader::TextVertical::Rotated) => 90.0,
        Some(jade_pptx_reader::TextVertical::Rotated270) => 270.0,
        None => 0.0,
    };
    let mut text_region = None;
    let lines = match &shape.text {
        Some(text) => {
            let mut lines = Vec::new();
            // Preset margins can depend on the grown height. Reapply anchoring
            // and wrapping until spAutoFit also fits its inner text rectangle.
            for _ in 0..64 {
                let preset_layout = presets::resolve(
                    visual.geometry.preset_geometry.as_deref(),
                    &visual.geometry.adjustments,
                    rect.width,
                    rect.height,
                );
                text_region = preset_layout
                    .as_ref()
                    .filter(|_| visual.geometry.preset_geometry.as_deref() != Some("rect"))
                    .map(|preset| {
                        presets::turned(preset.text, rect.width, rect.height, text_rotation_deg)
                    });
                let (frame_w, frame_h) = if text_rotation_deg == 0.0 {
                    (rect.width, rect.height)
                } else {
                    (rect.height, rect.width)
                };
                let (placed, content_height) =
                    text::layout_text_in_region(text, frame_w, frame_h, text_region, &text_context);
                lines = placed;
                if text_context.autofit.unwrap_or(text.autofit) != jade_pptx_reader::Autofit::Shape
                    || text_rotation_deg != 0.0
                    || !content_height.is_finite()
                    || content_height <= rect.height + 1e-7
                {
                    break;
                }
                rect.height = preset_layout
                    .map_or(content_height, |preset| {
                        preset.height_for_content(rect.width, rect.height, content_height)
                    })
                    .max(content_height);
            }
            lines
        }
        None => Vec::new(),
    };
    let effective_fill: Option<Fill> = match visual.fill {
        Some(fill) => Some(fill.clone()),
        None if visual.image.is_some() => None,
        None => visual
            .style
            .and_then(|style| style_fill(style, context.inherited)),
    };
    let fill = effective_fill
        .as_ref()
        .and_then(|fill| resolve_fill(fill, context.theme));
    let gradient = effective_fill
        .as_ref()
        .and_then(|fill| resolve_gradient(fill, context.theme));
    let pattern = effective_fill
        .as_ref()
        .and_then(|fill| resolve_pattern(fill, context.theme));
    Some(PlacedShape {
        pattern,
        text_region,
        local_rect: None,
        group_transforms: Vec::new(),
        group_ids: Vec::new(),
        name: shape.name.clone(),
        rect,
        rotation_rad: rotation_rad(transform),
        flip_h: transform.flip_h,
        flip_v: transform.flip_v,
        geometry: match &visual.geometry.custom_geometry {
            Some(custom) if visual.geometry.preset_geometry.is_none() => {
                geometry::custom_geometry(custom).unwrap_or_else(|| placed_geometry(None))
            }
            _ => presets::resolve(
                visual.geometry.preset_geometry.as_deref(),
                &visual.geometry.adjustments,
                rect.width,
                rect.height,
            )
            .map(|preset| preset.geometry)
            .unwrap_or_else(|| placed_geometry(visual.geometry.preset_geometry.as_deref())),
        },
        fill,
        gradient,
        outline: resolve_outline(
            visual.outline.as_ref(),
            visual.style,
            context.theme,
            context.inherited.theme_line_styles,
        ),
        image_part: visual.image.and_then(|s| s.image_part.clone()),
        source_rect: visual.image.and_then(|s| s.source_rect),
        lines,
        origin,
        source_id: shape.id,
        text_rotation_deg,
        shadow: place_shadow(
            shape.shadow.as_ref().or_else(|| {
                // `<a:effectLst/>` stated empty would also stop the style;
                // without one, the style's effect reference applies.
                let index = visual.style?.effect_index.checked_sub(1)? as usize;
                context.inherited.theme_effect_styles?.get(index)?.as_ref()
            }),
            context.theme,
        ),
        image_alpha: None,
    })
}

/// Maps a preset geometry name to what the renderer can draw.
///
/// The polygon points are ECMA-376 preset outlines at their default
/// adjustment values, in normalized shape coordinates. Anything unmodelled
/// stays a rectangle so the shape at least shows its footprint.
fn placed_geometry(preset: Option<&str>) -> PlacedGeometry {
    match preset {
        // Bent and curved connectors draw as their straight diagonal until
        // elbow routing exists — visible beats invisible.
        Some("line")
        | Some("straightConnector1")
        | Some("bentConnector2")
        | Some("bentConnector3")
        | Some("curvedConnector2")
        | Some("curvedConnector3") => PlacedGeometry::Line,
        Some("rtTriangle") => PlacedGeometry::Polygon(vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0)]),
        Some("parallelogram") => {
            PlacedGeometry::Polygon(vec![(0.25, 0.0), (1.0, 0.0), (0.75, 1.0), (0.0, 1.0)])
        }
        Some("trapezoid") => {
            PlacedGeometry::Polygon(vec![(0.25, 0.0), (0.75, 0.0), (1.0, 1.0), (0.0, 1.0)])
        }
        Some("pentagon") => PlacedGeometry::Polygon(vec![
            (0.5, 0.0),
            (1.0, 0.38),
            (0.81, 1.0),
            (0.19, 1.0),
            (0.0, 0.38),
        ]),
        // A callout is its box plus a pointer; the box is what carries the
        // text, and it draws with a small default tail at the bottom-left.
        Some("wedgeRectCallout") => PlacedGeometry::Polygon(vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (0.3125, 1.0),
            (0.125, 1.25),
            (0.1875, 1.0),
            (0.0, 1.0),
        ]),
        Some("homePlate") => PlacedGeometry::Polygon(vec![
            (0.0, 0.0),
            (0.75, 0.0),
            (1.0, 0.5),
            (0.75, 1.0),
            (0.0, 1.0),
        ]),
        Some("chevron") => PlacedGeometry::Polygon(vec![
            (0.0, 0.0),
            (0.75, 0.0),
            (1.0, 0.5),
            (0.75, 1.0),
            (0.0, 1.0),
            (0.25, 0.5),
        ]),
        Some("rightArrow") => PlacedGeometry::Polygon(vec![
            (0.0, 0.25),
            (0.75, 0.25),
            (0.75, 0.0),
            (1.0, 0.5),
            (0.75, 1.0),
            (0.75, 0.75),
            (0.0, 0.75),
        ]),
        Some("leftArrow") => PlacedGeometry::Polygon(vec![
            (1.0, 0.25),
            (0.25, 0.25),
            (0.25, 0.0),
            (0.0, 0.5),
            (0.25, 1.0),
            (0.25, 0.75),
            (1.0, 0.75),
        ]),
        Some("downArrow") => PlacedGeometry::Polygon(vec![
            (0.25, 0.0),
            (0.75, 0.0),
            (0.75, 0.75),
            (1.0, 0.75),
            (0.5, 1.0),
            (0.0, 0.75),
            (0.25, 0.75),
        ]),
        Some("upArrow") => PlacedGeometry::Polygon(vec![
            (0.25, 1.0),
            (0.75, 1.0),
            (0.75, 0.25),
            (1.0, 0.25),
            (0.5, 0.0),
            (0.0, 0.25),
            (0.25, 0.25),
        ]),
        Some("octagon") => PlacedGeometry::Polygon(vec![
            (0.29, 0.0),
            (0.71, 0.0),
            (1.0, 0.29),
            (1.0, 0.71),
            (0.71, 1.0),
            (0.29, 1.0),
            (0.0, 0.71),
            (0.0, 0.29),
        ]),
        _ => PlacedGeometry::Rect,
    }
}

fn place_picture(
    picture: &Picture,
    space: &GroupSpace,
    context: &Context<'_>,
    origin: PlacedOrigin,
) -> Option<PlacedShape> {
    let transform = resolve_transform(
        picture.transform,
        picture.placeholder.as_ref(),
        context.inherited,
    )?;
    let transform = &transform;
    Some(PlacedShape {
        text_region: None,
        local_rect: None,
        group_transforms: Vec::new(),
        group_ids: Vec::new(),
        name: picture.name.clone(),
        rect: placed_rect(transform, space),
        rotation_rad: rotation_rad(transform),
        flip_h: transform.flip_h,
        flip_v: transform.flip_v,
        geometry: presets::resolve(
            picture.preset_geometry.as_deref(),
            &picture.adjustments,
            placed_rect(transform, space).width,
            placed_rect(transform, space).height,
        )
        .map_or(PlacedGeometry::Rect, |preset| preset.geometry),
        fill: None,
        gradient: None,
        outline: resolve_outline(picture.outline.as_ref(), None, context.theme, None),
        image_part: picture.image_part.clone(),
        source_rect: picture.source_rect,
        lines: Vec::new(),
        origin,
        source_id: picture.id,
        text_rotation_deg: 0.0,
        shadow: place_shadow(picture.shadow.as_ref(), context.theme),
        image_alpha: picture.alpha.map(f64::from),
        pattern: None,
    })
}
