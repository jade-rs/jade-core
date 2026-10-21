//! The slide body: the shape tree and everything a shape carries.
//!
//! This models the subset of DrawingML the renderer draws — transforms,
//! preset geometry, fills, outlines, pictures, groups, tables, and text down
//! to the list styles a placeholder inherits — while naming things the way
//! the XML does so the write-back slice can line up byte anchors later
//! without a vocabulary change.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::xml_tree::{parse_tree, XmlElement};

/// One slide's parsed shape tree. Layout and master parts parse into the
/// same type: they carry a shape tree too, plus (for masters) the text
/// styles every placeholder ultimately inherits from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlideContent {
    /// `<p:bg><p:bgPr>` fill; `None` when the slide inherits its background.
    pub background: Option<Fill>,
    pub shapes: Vec<ShapeItem>,
    /// `showMasterSp` on the root: whether the master's (and layout's)
    /// non-placeholder shapes draw under this slide. Defaults to true.
    pub show_master_shapes: bool,
    /// `<p:clrMapOvr><a:overrideClrMapping>`: this slide's (or layout's)
    /// own `bg1`/`tx1` mapping, replacing the master's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_map_override: Option<std::collections::HashMap<String, String>>,
    /// Explicit masterClrMapping cancels a layout-level override.
    #[serde(default)]
    pub use_master_color_map: bool,
    /// `<p:txStyles>` — only a slide master carries these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_styles: Option<TextStyles>,
    /// `show="0"`: the slide is skipped in a slide show.
    #[serde(default)]
    pub hidden: bool,
    /// `<p:bg><p:bgRef idx>`: the theme's background fill style at `idx`
    /// (1001 is the first) in the reference colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_ref: Option<(u32, Color)>,
}

impl SlideContent {
    pub fn effective_color_map<'a>(
        &'a self,
        layout: Option<&'a Self>,
    ) -> Option<&'a std::collections::HashMap<String, String>> {
        if self.use_master_color_map {
            None
        } else {
            self.color_map_override
                .as_ref()
                .or_else(|| layout.and_then(|l| l.color_map_override.as_ref()))
        }
    }
}

impl Default for SlideContent {
    fn default() -> Self {
        Self {
            background: None,
            shapes: Vec::new(),
            show_master_shapes: true,
            color_map_override: None,
            use_master_color_map: false,
            text_styles: None,
            hidden: false,
            background_ref: None,
        }
    }
}

// A shape carries far more than a picture; the tree is built once per slide
// and walked by reference, so the padding costs nothing that boxing would
// win back.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ShapeItem {
    Shape(Shape),
    Picture(Picture),
    Group(GroupShape),
    Table(Table),
    Chart(Chart),
}

/// `<a:xfrm>` — where a shape sits and how it is turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transform {
    pub offset_emu: (i64, i64),
    pub extent_emu: (i64, i64),
    /// Clockwise, in 60000ths of a degree, as `rot` states it.
    pub rotation_60000ths: i32,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// A group's transform: its own placement plus the child coordinate space
/// (`chOff`/`chExt`) that children's offsets are expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupTransform {
    pub outer: Transform,
    pub child_offset_emu: (i64, i64),
    pub child_extent_emu: (i64, i64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Color {
    /// `<a:srgbClr val="RRGGBB"/>`, hex without the `#`.
    Srgb(String),
    /// `<a:schemeClr val="accent1"/>` — resolved against the theme at layout
    /// time, because the same slide renders differently under another theme.
    Scheme(String),
    /// A colour with transform children (`<a:lumMod>`, `<a:alpha>`, …). Each
    /// is a fraction of 1: `lumMod val="75000"` is `0.75`.
    Adjusted {
        base: Box<Color>,
        alpha: Option<f32>,
        lum_mod: Option<f32>,
        lum_off: Option<f32>,
        shade: Option<f32>,
        tint: Option<f32>,
    },
    /// A colour whose transforms go beyond `Adjusted`'s (`satMod`, `hueOff`,
    /// `comp`, `gray`, …): every transform child in document order, the
    /// order PowerPoint applies them in. Values are fractions of 1 (angles
    /// in degrees); flag transforms carry `0`.
    Transformed {
        base: Box<Color>,
        ops: Vec<(String, f32)>,
    },
}

/// `<a:path>` of a gradient: its kind and the focus rectangle
/// (`fillToRect`, insets from each edge as fractions of the box).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientPath {
    pub kind: String,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// One `<a:gs>` of a gradient, position in thousandths of a percent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub position_thousandths: u32,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Fill {
    Solid(Color),
    /// `<a:gradFill>` stops, sorted by position, with the `<a:lin ang>`
    /// direction in degrees (0 is left to right, 90 top to bottom). `None`
    /// when the fill states no line — a path gradient, or nothing.
    Gradient {
        stops: Vec<GradientStop>,
        angle_deg: Option<f32>,
        /// `<a:path path="circle|rect|shape">`: the gradient radiates from
        /// its `fillToRect` instead of running along a line.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<GradientPath>,
    },
    /// `<a:pattFill prst>`: a hatch of `foreground` over `background`.
    Pattern {
        preset: String,
        foreground: Color,
        background: Color,
    },
    /// `<a:noFill/>`: explicitly transparent, distinct from "inherit".
    None,
    /// `<a:grpFill/>`: the enclosing group's fill. Groups resolve it for
    /// their children while parsing; one left at the top level fills
    /// nothing.
    Group,
    /// `<a:blipFill>` on a background: a picture stretched over the slide.
    /// `image_part` is filled in by `read_deck`.
    Picture {
        rel_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image_part: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Outline {
    pub color: Option<Color>,
    pub width_emu: Option<i64>,
    /// `<a:prstDash val="…"/>` name, e.g. `dash`, `sysDot`; `None` is solid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<String>,
    /// `<a:ln><a:noFill/></a:ln>`: the shape explicitly has no outline, which
    /// also stops a style's `lnRef` from supplying one.
    #[serde(default)]
    pub no_fill: bool,
    /// `<a:headEnd>`: the decoration at the line's start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_end: Option<LineEnd>,
    /// `<a:tailEnd>`: the decoration at the line's end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_end: Option<LineEnd>,
}

/// `<a:headEnd>` / `<a:tailEnd>`: an arrowhead's kind (`triangle`,
/// `stealth`, `diamond`, `oval`, `arrow`) and its `w`/`len` (`sm`, `med`,
/// `lg`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineEnd {
    pub kind: String,
    pub width: String,
    pub length: String,
}

/// `<a:custGeom>`: one or more paths in their own coordinate space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomGeometry {
    pub paths: Vec<GeometryPath>,
}

/// One `<a:path>`: commands in a `width` × `height` space that maps onto
/// the shape's box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeometryPath {
    pub width: i64,
    pub height: i64,
    /// `fill="none"` turns this off: the path is outline only.
    pub filled: bool,
    /// `stroke="0"` turns this off.
    pub stroked: bool,
    pub commands: Vec<PathCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathCommand {
    MoveTo(i64, i64),
    LineTo(i64, i64),
    QuadTo {
        control: (i64, i64),
        to: (i64, i64),
    },
    CubicTo {
        control1: (i64, i64),
        control2: (i64, i64),
        to: (i64, i64),
    },
    /// `<a:arcTo>`: an elliptical arc from the current point, angles in
    /// 60000ths of a degree, clockwise.
    ArcTo {
        width_radius: i64,
        height_radius: i64,
        start_60000ths: i32,
        sweep_60000ths: i32,
    },
    Close,
}

/// `<p:ph>` — which layout/master placeholder this shape inherits from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placeholder {
    /// `type`, e.g. `title`, `body`, `ctrTitle`. Empty string when absent
    /// (an index-only placeholder).
    pub kind: String,
    pub index: Option<u32>,
}

/// `<p:style>` — the theme-indexed fill, line and font a shape uses when its
/// own `spPr` leaves them out. PowerPoint writes one on every inserted shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeStyle {
    /// `fillRef idx`: 0 is no fill, 1–3 the theme's fill styles, ≥1000 a
    /// background fill style.
    pub fill_index: u32,
    pub fill_color: Option<Color>,
    /// `lnRef idx`: 0 is no line, 1–3 the theme's line styles.
    pub line_index: u32,
    pub line_color: Option<Color>,
    pub font_color: Option<Color>,
    /// `effectRef idx`: 0 is no effect, 1–3 the theme's effect styles.
    #[serde(default)]
    pub effect_index: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// `<p:cNvPr id>`: unique within the slide, the key a caller uses to
    /// match a parsed shape with what layout placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub name: String,
    pub placeholder: Option<Placeholder>,
    pub transform: Option<Transform>,
    /// `<a:prstGeom prst="…">` name; `None` for custom geometry.
    pub preset_geometry: Option<String>,
    /// `<a:avLst><a:gd name="adj" fmla="val N"/>`: the preset's adjustment
    /// values, in the preset's own units (usually hundred-thousandths).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<(String, i64)>,
    /// `<a:custGeom>`: a freeform's own outline, when it has no preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_geometry: Option<CustomGeometry>,
    pub fill: Option<Fill>,
    pub outline: Option<Outline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<ShapeStyle>,
    /// `<a:blipFill>` in `spPr`: the shape is filled with a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_rel_id: Option<String>,
    /// The image part the picture fill resolves to, filled in by `read_deck`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_part: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_rect: Option<SourceRect>,
    pub text: Option<TextBody>,
    /// `<a:effectLst><a:outerShdw>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    /// The rest of `<a:effectLst>`: inner shadow, glow, reflection, soft
    /// edge.
    #[serde(default, skip_serializing_if = "Effects::is_empty")]
    pub effects: Effects,
}

/// `<a:srcRect>` offsets in thousandths of a percent; negative is an outset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SourceRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl SourceRect {
    pub fn from_attributes(
        left: Option<&str>,
        top: Option<&str>,
        right: Option<&str>,
        bottom: Option<&str>,
    ) -> Self {
        let offset = |value: Option<&str>| {
            value
                .and_then(|value| {
                    if let Some(percent) = value.strip_suffix('%') {
                        percent
                            .parse::<f64>()
                            .ok()
                            .filter(|n| n.is_finite())
                            .map(|n| (n * 1000.0).round() as i32)
                    } else {
                        value.parse().ok()
                    }
                })
                .unwrap_or(0)
        };
        Self {
            left: offset(left),
            top: offset(top),
            right: offset(right),
            bottom: offset(bottom),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Picture {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub name: String,
    /// A picture placeholder (`<p:ph type="pic">`), whose geometry the
    /// layout supplies when the picture states none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Placeholder>,
    pub transform: Option<Transform>,
    /// The `r:embed` relationship id of the `<a:blip>`.
    pub blip_rel_id: Option<String>,
    /// The image part the id resolves to, filled in by `read_deck`.
    pub image_part: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_rect: Option<SourceRect>,
    /// `<a:prstGeom>` the picture is cropped to (rounded, oval, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_geometry: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<(String, i64)>,
    /// The picture's border.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Outline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    #[serde(default, skip_serializing_if = "Effects::is_empty")]
    pub effects: Effects,
    /// `<a:blip><a:alphaModFix amt>`: the picture's opacity, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f32>,
}

/// `<a:outerShdw>`: a soft copy of the shape's silhouette behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub color: Color,
    pub blur_emu: i64,
    pub distance_emu: i64,
    /// Direction the shadow falls, clockwise degrees from the +x axis.
    pub direction_deg: f32,
}

/// The effects of an `<a:effectLst>` beyond its outer shadow, which keeps
/// its own field for the shapes and runs that only ever had that one.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Effects {
    /// `<a:innerShdw>`: a shadow cast inside the silhouette's edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_shadow: Option<Shadow>,
    /// `<a:glow>`: a soft halo of colour around the silhouette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<Glow>,
    /// `<a:reflection>`: a fading mirror image beside the shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection: Option<Reflection>,
    /// `<a:softEdge rad>`: the silhouette fades out over this many EMU
    /// inward from its edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_edge_emu: Option<i64>,
}

impl Effects {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// `<a:glow rad>` and its colour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Glow {
    pub color: Color,
    pub radius_emu: i64,
}

/// `<a:reflection>` with every attribute at its ECMA-376 default when
/// unstated. Fractions are 0..1 (thousandths of a percent / 100000),
/// angles in degrees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reflection {
    pub blur_emu: i64,
    pub start_alpha: f32,
    pub start_position: f32,
    pub end_alpha: f32,
    pub end_position: f32,
    pub distance_emu: i64,
    pub direction_deg: f32,
    pub fade_direction_deg: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    /// `algn`: `tl`, `t`, `tr`, `l`, `ctr`, `r`, `bl`, `b` or `br`.
    pub align: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupShape {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub name: String,
    pub transform: Option<GroupTransform>,
    pub children: Vec<ShapeItem>,
    /// Set when the group is a SmartArt frame: the `r:dm` relationship of
    /// its `<dgm:relIds>`. `read_deck` fills `children` from the drawing
    /// PowerPoint caches beside the data part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagram_data_rel_id: Option<String>,
}

/// `<p:graphicFrame>` holding a `<c:chart>` reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub name: String,
    pub transform: Option<Transform>,
    /// The `r:id` of `<c:chart>`.
    pub chart_rel_id: Option<String>,
    /// The chart part it resolves to, filled in by `read_deck`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_part: Option<String>,
    /// The plot element under `c:plotArea`, e.g. `barChart`, `lineChart`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Bars run sideways (`c:barDir val="bar"`).
    #[serde(default)]
    pub horizontal: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<crate::chart::ChartSeries>,
    #[serde(default)]
    pub style: crate::chart::ChartStyle,
}

/// `<p:graphicFrame>` holding an `<a:tbl>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Table {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub name: String,
    pub transform: Option<Transform>,
    /// `<a:gridCol w>` per column, in EMU.
    pub column_widths_emu: Vec<i64>,
    pub rows: Vec<TableRow>,
    /// `<a:tblPr>`: the style reference and which of its parts apply.
    #[serde(default)]
    pub style: crate::table_style::TableStyleFlags,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    /// `<a:tr h>`, in EMU.
    pub height_emu: i64,
    pub cells: Vec<TableCell>,
}

/// The four `<a:lnL>`/`<a:lnR>`/`<a:lnT>`/`<a:lnB>` borders of a cell.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CellBorders {
    pub left: Option<Outline>,
    pub right: Option<Outline>,
    pub top: Option<Outline>,
    pub bottom: Option<Outline>,
    /// `<a:lnTlToBr>`: a diagonal from the top-left to the bottom-right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagonal_down: Option<Outline>,
    /// `<a:lnBlToTr>`: a diagonal from the bottom-left to the top-right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagonal_up: Option<Outline>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    pub text: Option<TextBody>,
    /// `<a:tcPr>` fill.
    pub fill: Option<Fill>,
    pub borders: CellBorders,
    /// Columns this cell covers; 1 when unstated.
    pub grid_span: u32,
    /// Rows this cell covers; 1 when unstated.
    pub row_span: u32,
    /// The cell is covered by a spanning cell to its left.
    pub h_merge: bool,
    /// The cell is covered by a spanning cell above it.
    pub v_merge: bool,
    /// `<a:tcPr anchor>`.
    pub anchor: Option<TextAnchor>,
    /// `<a:tcPr marL/marR/marT/marB>`.
    pub insets: TextInsets,
}

/// `<a:bodyPr vert="…">` — text that runs down the box instead of across.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextVertical {
    /// `eaVert` / `mongolianVert`: East Asian vertical, columns right to
    /// left with upright characters.
    EastAsian,
    /// `vert` / `wordArtVert`: the whole block turned a quarter turn
    /// clockwise.
    Rotated,
    /// `vert270`: turned a quarter turn anticlockwise.
    Rotated270,
}

/// `<a:bodyPr anchor="…">` — where the text block sits vertically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAnchor {
    Top,
    Center,
    Bottom,
}

/// `<a:bodyPr>` insets; `None` means the PowerPoint default for that edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextInsets {
    pub left_emu: Option<i64>,
    pub top_emu: Option<i64>,
    pub right_emu: Option<i64>,
    pub bottom_emu: Option<i64>,
}

/// How a text body fits its shape, as `<a:bodyPr>` states it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Autofit {
    /// No autofit element (or `<a:noAutofit/>`): text overflows the box.
    #[default]
    None,
    /// `<a:spAutoFit/>`: the shape grows to fit its text.
    Shape,
    /// `<a:normAutofit fontScale lnSpcReduction/>`: PowerPoint shrank the
    /// text to fit and recorded by how much. Both are fractions of 1;
    /// `font_scale` is `1.0` when the element carries no attribute.
    Normal {
        font_scale: f32,
        line_spacing_reduction: f32,
    },
}

/// Parsed body-property presence distinguishes omission from explicit defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextBodyPropertyPresence {
    pub wrap: bool,
    pub autofit: bool,
    pub vertical: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBody {
    /// None keeps explicitly supplied/legacy model values authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_property_presence: Option<TextBodyPropertyPresence>,
    pub anchor: Option<TextAnchor>,
    pub insets: TextInsets,
    /// `wrap="none"` turns this off: the text runs on a single line per
    /// paragraph however wide the shape is.
    pub wrap: bool,
    pub autofit: Autofit,
    /// `vert`: `None` for ordinary horizontal text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<TextVertical>,
    /// The body's own `<a:lstStyle>`, the first stop of paragraph-property
    /// inheritance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_style: Option<ListStyle>,
    pub paragraphs: Vec<TextParagraph>,
    /// `numCol` and `spcCol` (EMU): text flowing through side-by-side
    /// columns. `None` for a single column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<(u32, i64)>,
}

impl Default for TextBody {
    fn default() -> Self {
        Self {
            body_property_presence: None,
            anchor: None,
            insets: TextInsets::default(),
            wrap: true,
            autofit: Autofit::None,
            vertical: None,
            list_style: None,
            paragraphs: Vec::new(),
            columns: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
    Distributed,
}

/// A paragraph's bullet, as `<a:pPr>` states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bullet {
    /// `<a:buChar char="…"/>`.
    Char(String),
    /// `<a:buAutoNum type="…"/>` — the numbering scheme name, e.g.
    /// `arabicPeriod`.
    AutoNumber { scheme: String },
    /// `<a:buNone/>`: explicitly no bullet, distinct from "inherit".
    None,
    /// `<a:buBlip><a:blip r:embed/>`: a picture as the bullet. `image_part`
    /// is filled in by `read_deck` from the part's relationships.
    Picture {
        rel_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image_part: Option<String>,
    },
}

/// A line or paragraph spacing value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Spacing {
    /// `<a:spcPct val>`: a multiple of the line's font size. `100000` → `1.0`.
    Percent(f32),
    /// `<a:spcPts val>`: an absolute size in points. `1200` → `12.0`.
    Points(f32),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextParagraph {
    /// Outline level, 0-based as `lvl` states it.
    pub level: u8,
    pub align: Option<TextAlign>,
    /// `None` means the paragraph inherits its bullet.
    pub bullet: Option<Bullet>,
    /// `<a:buFont typeface>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bullet_font: Option<String>,
    #[serde(default)]
    pub bullet_font_follow_text: bool,
    /// `<a:buSzPct val>` as a fraction of the text size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bullet_size_percent: Option<f32>,
    /// `<a:buClr>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bullet_color: Option<Color>,
    /// `<a:pPr marL>`: where the text starts, from the inset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_left_emu: Option<i64>,
    /// `<a:pPr marR>`: the paragraph's right margin inside the body inset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_right_emu: Option<i64>,
    /// `<a:pPr indent>`: the first line's offset from `marL`, negative for a
    /// hanging bullet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent_emu: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_spacing: Option<Spacing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_before: Option<Spacing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_after: Option<Spacing>,
    /// `<a:endParaRPr sz>`: the size an empty paragraph takes up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_size_pt: Option<f32>,
    #[serde(default)]
    pub end_run_props: RunProps,
    /// `<a:buAutoNum startAt>`: the number this paragraph restarts at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_start: Option<u32>,
    pub runs: Vec<TextRun>,
    /// `<a:tabLst>` stop positions (EMU from the text's left inset), in
    /// order, then `defTabSz` stops beyond them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tab_stops_emu: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_tab_emu: Option<i64>,
}

/// `<a:hlinkClick r:id>` on a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hyperlink {
    pub rel_id: String,
    /// The relationship's target, filled in by `read_deck`.
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextRun {
    /// Run text; an explicit `<a:br/>` becomes a run of `"\n"`.
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    #[serde(default)]
    pub strike: bool,
    /// `cap="all"` (or `small`, drawn as capitals too): the run shows in
    /// upper case whatever its text says.
    #[serde(default)]
    pub all_caps: bool,
    /// `<a:fld type="…">`: the run is a field (`slidenum`, `datetime…`).
    /// `read_deck` replaces a slide number field's cached text with the
    /// slide's real number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// `sz` converted from hundredths of a point.
    pub size_pt: Option<f32>,
    pub color: Option<Color>,
    /// `<a:latin typeface="…">`.
    pub font: Option<String>,
    /// `<a:ea typeface="…">` — the face East Asian text draws with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub east_asian_font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hyperlink: Option<Hyperlink>,
    /// Whether the run stated `b`/`i`/`u` at all, so inheritance can tell
    /// "not bold" from "unsaid". `None` means unsaid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold_stated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic_stated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline_stated: Option<bool>,
    /// `baseline`: the run's rise as a fraction of its size (`30000` is
    /// superscript, `-25000` subscript).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f32>,
    /// `<a:highlight>`: the colour behind the run's glyphs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Color>,
    /// `spc`: extra space after each character, in points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing_pt: Option<f32>,
    /// `u` beyond a plain single line (`dbl`, `heavy`, `dotted`, `dash`,
    /// `wavy`, …); `None` for `sng` or no underline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline_style: Option<String>,
    /// `strike="dblStrike"`.
    #[serde(default)]
    pub double_strike: bool,
    /// `<a:ln>` in `rPr`: the glyphs' outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Outline>,
    /// `<a:effectLst><a:outerShdw>` in `rPr`: the glyphs' shadow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    /// `<a:effectLst><a:glow>` in `rPr`: a halo around the glyphs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<Glow>,
}

/// The run properties a level style supplies (`<a:defRPr>`), each optional
/// so a level can override one facet and inherit the rest.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RunProps {
    pub size_pt: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub color: Option<Color>,
    pub font: Option<String>,
    pub east_asian_font: Option<String>,
}

/// One `<a:lvlNpPr>` of a list style: paragraph properties plus the default
/// run properties for that outline level.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LevelStyle {
    pub align: Option<TextAlign>,
    pub margin_left_emu: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_right_emu: Option<i64>,
    pub indent_emu: Option<i64>,
    pub bullet: Option<Bullet>,
    pub bullet_font: Option<String>,
    #[serde(default)]
    pub bullet_font_follow_text: bool,
    pub bullet_size_percent: Option<f32>,
    pub bullet_color: Option<Color>,
    pub line_spacing: Option<Spacing>,
    pub space_before: Option<Spacing>,
    pub space_after: Option<Spacing>,
    pub default_run: RunProps,
}

/// `<a:lstStyle>`, `<p:titleStyle>`, `<p:bodyStyle>`, `<p:otherStyle>` or
/// `<p:defaultTextStyle>`: up to nine levels, each optional.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ListStyle {
    pub levels: Vec<Option<LevelStyle>>,
}

impl ListStyle {
    /// The style for a 0-based outline level, when this list states one.
    pub fn level(&self, level: usize) -> Option<&LevelStyle> {
        self.levels.get(level).and_then(Option::as_ref)
    }

    pub fn is_empty(&self) -> bool {
        self.levels.iter().all(Option::is_none)
    }
}

/// A master's `<p:txStyles>`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextStyles {
    pub title: ListStyle,
    pub body: ListStyle,
    pub other: ListStyle,
}

/// Parses one slide part's XML into its shape tree. Layout, master and
/// notes parts parse through the same function.
///
/// Picture relationship ids are left unresolved; `read_deck` resolves them
/// against the slide's `.rels` because only the package knows the parts.
pub fn parse_slide_xml(xml: &str) -> Result<SlideContent> {
    let root = parse_tree(xml)?;
    let c_sld = root.child("cSld");
    let mut shapes = Vec::new();
    if let Some(tree) = c_sld.and_then(|c_sld| c_sld.child("spTree")) {
        parse_shape_children(tree, &mut shapes);
    }
    Ok(SlideContent {
        background: c_sld
            .and_then(|c_sld| c_sld.child("bg"))
            .and_then(|bg| bg.child("bgPr"))
            .and_then(|bg_pr| match bg_pr.child("blipFill") {
                Some(blip_fill) => blip_fill
                    .child("blip")
                    .and_then(|blip| blip.attr("embed"))
                    .map(|rel_id| Fill::Picture {
                        rel_id: rel_id.to_string(),
                        image_part: None,
                    }),
                None => parse_fill(bg_pr),
            }),
        background_ref: c_sld
            .and_then(|c_sld| c_sld.child("bg"))
            .and_then(|bg| bg.child("bgRef"))
            .and_then(|bg_ref| {
                let index = bg_ref.attr("idx")?.parse().ok()?;
                Some((index, parse_color(bg_ref)?))
            }),
        shapes,
        show_master_shapes: root.attr("showMasterSp") != Some("0"),
        hidden: matches!(root.attr("show"), Some("0" | "false")),
        color_map_override: root
            .child("clrMapOvr")
            .and_then(|ovr| ovr.child("overrideClrMapping"))
            .map(|map| {
                map.attrs
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            }),
        use_master_color_map: root
            .child("clrMapOvr")
            .and_then(|o| o.child("masterClrMapping"))
            .is_some(),
        text_styles: root.child("txStyles").map(|styles| TextStyles {
            title: styles
                .child("titleStyle")
                .map(parse_list_style)
                .unwrap_or_default(),
            body: styles
                .child("bodyStyle")
                .map(parse_list_style)
                .unwrap_or_default(),
            other: styles
                .child("otherStyle")
                .map(parse_list_style)
                .unwrap_or_default(),
        }),
    })
}

/// Parses `<p:defaultTextStyle>` out of the presentation part.
pub fn parse_default_text_style(xml: &str) -> Result<ListStyle> {
    let root = parse_tree(xml)?;
    Ok(root
        .child("defaultTextStyle")
        .map(parse_list_style)
        .unwrap_or_default())
}

fn parse_shape_children(parent: &XmlElement, out: &mut Vec<ShapeItem>) {
    for child in &parent.children {
        // `<p:cNvPr hidden="1">`: the object is in the file but not shown.
        let hidden = child
            .children
            .iter()
            .find(|nv| nv.name.starts_with("nv"))
            .and_then(|nv| nv.child("cNvPr"))
            .and_then(|c_nv_pr| c_nv_pr.attr("hidden"))
            .is_some_and(|hidden| matches!(hidden, "1" | "true"));
        if hidden {
            continue;
        }
        match child.name.as_str() {
            // A connector carries the same spPr as a shape and draws like one.
            "sp" | "cxnSp" => out.push(ShapeItem::Shape(parse_sp(child))),
            "pic" => out.push(ShapeItem::Picture(parse_pic(child))),
            "grpSp" => out.push(ShapeItem::Group(parse_grp(child))),
            "graphicFrame" => {
                if let Some(item) = parse_graphic_frame(child) {
                    out.push(item);
                }
            }
            // Markup compatibility: the Fallback is what every reader can
            // draw; the Choice needs a feature this one does not declare.
            "AlternateContent" => {
                if let Some(fallback) = child.child("Fallback") {
                    parse_shape_children(fallback, out);
                }
            }
            _ => {}
        }
    }
}

fn non_visual_name(element: &XmlElement) -> String {
    element
        .descendant("cNvPr")
        .and_then(|c_nv_pr| c_nv_pr.attr("name"))
        .unwrap_or_default()
        .to_string()
}

fn non_visual_id(element: &XmlElement) -> Option<u32> {
    element
        .descendant("cNvPr")
        .and_then(|c_nv_pr| c_nv_pr.attr("id"))
        .and_then(|id| id.parse().ok())
}

fn parse_sp(sp: &XmlElement) -> Shape {
    let sp_pr = sp.child("spPr");
    Shape {
        id: non_visual_id(sp),
        name: non_visual_name(sp),
        placeholder: sp
            .child("nvSpPr")
            .or_else(|| sp.child("nvCxnSpPr"))
            .and_then(|nv| nv.child("nvPr"))
            .and_then(|nv_pr| nv_pr.child("ph"))
            .map(|ph| Placeholder {
                kind: ph.attr("type").unwrap_or_default().to_string(),
                index: ph.attr("idx").and_then(|idx| idx.parse().ok()),
            }),
        transform: sp_pr.and_then(|pr| pr.child("xfrm")).map(parse_transform),
        preset_geometry: sp_pr
            .and_then(|pr| pr.child("prstGeom"))
            .and_then(|geom| geom.attr("prst"))
            .map(str::to_string),
        adjustments: sp_pr
            .and_then(|pr| pr.child("prstGeom"))
            .and_then(|geom| geom.child("avLst"))
            .map(|av| {
                av.children
                    .iter()
                    .filter(|gd| gd.name == "gd")
                    .filter_map(|gd| {
                        let name = gd.attr("name")?;
                        let value = gd.attr("fmla")?.strip_prefix("val ")?.trim().parse().ok()?;
                        Some((name.to_string(), value))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        custom_geometry: sp_pr.and_then(|pr| pr.child("custGeom")).map(|geom| {
            let ext = sp_pr
                .and_then(|pr| pr.child("xfrm"))
                .map(|xfrm| extent_of(xfrm.child("ext")));
            parse_custom_geometry(geom, ext)
        }),
        fill: sp_pr.and_then(parse_fill),
        outline: sp_pr.and_then(|pr| pr.child("ln")).map(parse_outline),
        shadow: sp_pr.and_then(parse_shadow),
        effects: sp_pr.map(parse_effects).unwrap_or_default(),
        style: sp.child("style").map(parse_style),
        image_rel_id: sp_pr
            .and_then(|pr| pr.child("blipFill"))
            .and_then(|fill| fill.child("blip"))
            .and_then(|blip| blip.attr("embed"))
            .map(str::to_string),
        image_part: None,
        source_rect: sp_pr
            .and_then(|pr| pr.child("blipFill"))
            .and_then(parse_source_rect),
        text: sp.child("txBody").map(parse_text_body),
    }
}

fn parse_style(style: &XmlElement) -> ShapeStyle {
    let index = |name: &str| -> u32 {
        style
            .child(name)
            .and_then(|reference| reference.attr("idx"))
            .and_then(|idx| idx.parse().ok())
            .unwrap_or(0)
    };
    ShapeStyle {
        fill_index: index("fillRef"),
        fill_color: style.child("fillRef").and_then(parse_color),
        line_index: index("lnRef"),
        line_color: style.child("lnRef").and_then(parse_color),
        font_color: style.child("fontRef").and_then(parse_color),
        effect_index: index("effectRef"),
    }
}

fn parse_source_rect(fill: &XmlElement) -> Option<SourceRect> {
    let rect = fill.child("srcRect")?;
    Some(SourceRect::from_attributes(
        rect.attr("l"),
        rect.attr("t"),
        rect.attr("r"),
        rect.attr("b"),
    ))
}

fn parse_pic(pic: &XmlElement) -> Picture {
    Picture {
        id: non_visual_id(pic),
        name: non_visual_name(pic),
        placeholder: pic
            .child("nvPicPr")
            .and_then(|nv| nv.child("nvPr"))
            .and_then(|nv_pr| nv_pr.child("ph"))
            .map(|ph| Placeholder {
                kind: ph.attr("type").unwrap_or_default().to_string(),
                index: ph.attr("idx").and_then(|idx| idx.parse().ok()),
            }),
        transform: pic
            .child("spPr")
            .and_then(|pr| pr.child("xfrm"))
            .map(parse_transform),
        blip_rel_id: pic
            .child("blipFill")
            .and_then(|fill| fill.child("blip"))
            .and_then(|blip| blip.attr("embed"))
            .map(str::to_string),
        image_part: None,
        source_rect: pic.child("blipFill").and_then(parse_source_rect),
        preset_geometry: pic
            .child("spPr")
            .and_then(|pr| pr.child("prstGeom"))
            .and_then(|geom| geom.attr("prst"))
            .map(str::to_string),
        adjustments: pic
            .child("spPr")
            .and_then(|pr| pr.child("prstGeom"))
            .map(parse_adjustments)
            .unwrap_or_default(),
        outline: pic
            .child("spPr")
            .and_then(|pr| pr.child("ln"))
            .map(parse_outline),
        shadow: pic.child("spPr").and_then(parse_shadow),
        effects: pic.child("spPr").map(parse_effects).unwrap_or_default(),
        alpha: pic.child("blipFill").and_then(blip_alpha),
    }
}

/// The opacity `<a:alphaModFix amt>` gives a blip, when it is not opaque.
fn blip_alpha(blip_fill: &XmlElement) -> Option<f32> {
    fraction(blip_fill.child("blip")?.child("alphaModFix")?.attr("amt"))
        .map(|amt| amt.clamp(0.0, 1.0))
        .filter(|amt| *amt < 1.0)
}

/// `<a:avLst>` overrides of a `<a:prstGeom>`.
fn parse_adjustments(geom: &XmlElement) -> Vec<(String, i64)> {
    geom.child("avLst")
        .map(|av| {
            av.children
                .iter()
                .filter(|gd| gd.name == "gd")
                .filter_map(|gd| {
                    let name = gd.attr("name")?;
                    let value = gd.attr("fmla")?.strip_prefix("val ")?.trim().parse().ok()?;
                    Some((name.to_string(), value))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `<a:outerShdw>` of an `spPr`'s `<a:effectLst>`.
///
/// An `<a:effectLst>` without one is an explicit "no shadow" that still
/// overrides a style's `effectRef`: it reads as a fully transparent shadow,
/// which layout drops.
pub(crate) fn parse_shadow(sp_pr: &XmlElement) -> Option<Shadow> {
    let effects = sp_pr.child("effectLst")?;
    let Some(shadow) = effects.child("outerShdw") else {
        return Some(Shadow {
            color: Color::Adjusted {
                base: Box::new(Color::Srgb("000000".into())),
                alpha: Some(0.0),
                lum_mod: None,
                lum_off: None,
                shade: None,
                tint: None,
            },
            blur_emu: 0,
            distance_emu: 0,
            direction_deg: 0.0,
        });
    };
    let number = |name: &str| {
        shadow
            .attr(name)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    };
    Some(Shadow {
        color: parse_color(shadow)?,
        blur_emu: number("blurRad"),
        distance_emu: number("dist"),
        direction_deg: number("dir") as f32 / 60_000.0,
    })
}

/// Everything in an element's `<a:effectLst>` but the outer shadow.
pub(crate) fn parse_effects(container: &XmlElement) -> Effects {
    let Some(effects) = container.child("effectLst") else {
        return Effects::default();
    };
    let int = |element: &XmlElement, name: &str, default: i64| {
        element
            .attr(name)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(default)
    };
    let frac =
        |element: &XmlElement, name: &str, default: f32| fraction(element.attr(name)).unwrap_or(default);
    let angle = |element: &XmlElement, name: &str, default: f32| {
        element
            .attr(name)
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(default, |v| v / 60_000.0)
    };
    Effects {
        inner_shadow: effects.child("innerShdw").and_then(|shadow| {
            Some(Shadow {
                color: parse_color(shadow)?,
                blur_emu: int(shadow, "blurRad", 0),
                distance_emu: int(shadow, "dist", 0),
                direction_deg: angle(shadow, "dir", 0.0),
            })
        }),
        glow: effects.child("glow").and_then(|glow| {
            Some(Glow {
                color: parse_color(glow)?,
                radius_emu: int(glow, "rad", 0),
            })
            .filter(|glow| glow.radius_emu > 0)
        }),
        reflection: effects.child("reflection").map(|r| Reflection {
            blur_emu: int(r, "blurRad", 0),
            start_alpha: frac(r, "stA", 1.0),
            start_position: frac(r, "stPos", 0.0),
            end_alpha: frac(r, "endA", 0.0),
            end_position: frac(r, "endPos", 1.0),
            distance_emu: int(r, "dist", 0),
            direction_deg: angle(r, "dir", 0.0),
            fade_direction_deg: angle(r, "fadeDir", 90.0),
            scale_x: frac(r, "sx", 1.0),
            scale_y: frac(r, "sy", 1.0),
            align: r.attr("algn").unwrap_or("b").to_string(),
        }),
        soft_edge_emu: effects
            .child("softEdge")
            .map(|edge| int(edge, "rad", 0))
            .filter(|rad| *rad > 0),
    }
}

/// Hands a group's own fill to the children that asked for it with
/// `<a:grpFill/>`, nested groups included.
fn inherit_group_fill(children: &mut [ShapeItem], fill: &Fill) {
    for child in children {
        match child {
            ShapeItem::Shape(shape) if shape.fill == Some(Fill::Group) => {
                shape.fill = Some(fill.clone());
            }
            ShapeItem::Group(group) => inherit_group_fill(&mut group.children, fill),
            _ => {}
        }
    }
}

fn parse_grp(grp: &XmlElement) -> GroupShape {
    let mut children = Vec::new();
    parse_shape_children(grp, &mut children);
    if let Some(fill) = grp.child("grpSpPr").and_then(parse_fill) {
        inherit_group_fill(&mut children, &fill);
    }
    GroupShape {
        diagram_data_rel_id: None,
        id: non_visual_id(grp),
        name: non_visual_name(grp),
        transform: grp
            .child("grpSpPr")
            .and_then(|pr| pr.child("xfrm"))
            .map(|xfrm| GroupTransform {
                outer: parse_transform(xfrm),
                child_offset_emu: point_of(xfrm.child("chOff")),
                child_extent_emu: extent_of(xfrm.child("chExt")),
            }),
        children,
    }
}

/// A graphic frame holds a table, a chart reference, or a SmartArt
/// reference; OLE objects and the rest are not modelled and the frame is
/// skipped.
fn parse_graphic_frame(frame: &XmlElement) -> Option<ShapeItem> {
    let data = frame
        .child("graphic")
        .and_then(|graphic| graphic.child("graphicData"))?;
    if let Some(tbl) = data.child("tbl") {
        return parse_table_frame(frame, tbl).map(ShapeItem::Table);
    }
    if let Some(chart) = data.child("chart") {
        return Some(ShapeItem::Chart(Chart {
            id: non_visual_id(frame),
            name: non_visual_name(frame),
            transform: frame.child("xfrm").map(parse_transform),
            chart_rel_id: chart.attr("id").map(str::to_string),
            chart_part: None,
            kind: None,
            title: None,
            horizontal: false,
            series: Vec::new(),
            style: Default::default(),
        }));
    }
    if let Some(rel_ids) = data.child("relIds") {
        // SmartArt: the drawing PowerPoint caches gives the shapes; until
        // `read_deck` reads it, the frame is an empty group at its xfrm.
        let transform = frame.child("xfrm").map(parse_transform);
        return Some(ShapeItem::Group(GroupShape {
            id: non_visual_id(frame),
            name: non_visual_name(frame),
            transform: transform.map(|outer| GroupTransform {
                outer,
                child_offset_emu: (0, 0),
                child_extent_emu: outer.extent_emu,
            }),
            children: Vec::new(),
            diagram_data_rel_id: rel_ids.attr("dm").map(str::to_string),
        }));
    }
    // An embedded object (a workbook, a drawing) shows its cached preview
    // picture, inside markup compatibility or not.
    let ole_objects = std::iter::once(data.child("oleObj")).chain(
        data.children
            .iter()
            .filter(|child| child.name == "AlternateContent")
            .flat_map(|alternate| {
                alternate
                    .children
                    .iter()
                    .map(|branch| branch.child("oleObj"))
            }),
    );
    for ole in ole_objects.flatten() {
        if let Some(pic) = ole.child("pic") {
            let mut picture = parse_pic(pic);
            picture.id = non_visual_id(frame).or(picture.id);
            picture.name = non_visual_name(frame);
            if let Some(xfrm) = frame.child("xfrm") {
                picture.transform = Some(parse_transform(xfrm));
            }
            return Some(ShapeItem::Picture(picture));
        }
    }
    None
}

fn parse_table_frame(frame: &XmlElement, tbl: &XmlElement) -> Option<Table> {
    let column_widths_emu = tbl
        .child("tblGrid")
        .map(|grid| {
            grid.children
                .iter()
                .filter(|child| child.name == "gridCol")
                .map(|col| col.attr("w").and_then(|w| w.parse().ok()).unwrap_or(0))
                .collect()
        })
        .unwrap_or_default();
    let rows = tbl
        .children
        .iter()
        .filter(|child| child.name == "tr")
        .map(|tr| TableRow {
            height_emu: tr.attr("h").and_then(|h| h.parse().ok()).unwrap_or(0),
            cells: tr
                .children
                .iter()
                .filter(|child| child.name == "tc")
                .map(parse_table_cell)
                .collect(),
        })
        .collect();
    Some(Table {
        id: non_visual_id(frame),
        name: non_visual_name(frame),
        transform: frame.child("xfrm").map(parse_transform),
        column_widths_emu,
        rows,
        style: crate::table_style::parse_table_style_flags(tbl.child("tblPr")),
    })
}

fn parse_table_cell(tc: &XmlElement) -> TableCell {
    let tc_pr = tc.child("tcPr");
    let span = |name: &str| -> u32 {
        tc.attr(name)
            .and_then(|value| value.parse().ok())
            .filter(|value| *value >= 1)
            .unwrap_or(1)
    };
    let flag = |name: &str| -> bool { matches!(tc.attr(name), Some("1") | Some("true")) };
    let inset = |name: &str| -> Option<i64> {
        tc_pr
            .and_then(|pr| pr.attr(name))
            .and_then(|value| value.parse().ok())
    };
    let border =
        |name: &str| -> Option<Outline> { tc_pr.and_then(|pr| pr.child(name)).map(parse_outline) };
    TableCell {
        text: tc.child("txBody").map(parse_text_body),
        fill: tc_pr.and_then(parse_fill),
        borders: CellBorders {
            left: border("lnL"),
            right: border("lnR"),
            top: border("lnT"),
            bottom: border("lnB"),
            diagonal_down: border("lnTlToBr"),
            diagonal_up: border("lnBlToTr"),
        },
        grid_span: span("gridSpan"),
        row_span: span("rowSpan"),
        h_merge: flag("hMerge"),
        v_merge: flag("vMerge"),
        anchor: tc_pr
            .and_then(|pr| pr.attr("anchor"))
            .and_then(parse_anchor),
        insets: TextInsets {
            left_emu: inset("marL"),
            top_emu: inset("marT"),
            right_emu: inset("marR"),
            bottom_emu: inset("marB"),
        },
    }
}

fn parse_custom_geometry(cust_geom: &XmlElement, ext: Option<(i64, i64)>) -> CustomGeometry {
    let path_elements: Vec<&XmlElement> = cust_geom
        .child("pathLst")
        .map(|list| list.children.iter().filter(|c| c.name == "path").collect())
        .unwrap_or_default();
    // Guides see the shape's own extent; a path without w/h draws in it.
    let (w, h) = ext.filter(|(w, h)| *w > 0 && *h > 0).unwrap_or_else(|| {
        path_elements
            .first()
            .map(|p| {
                let n = |name: &str| p.attr(name).and_then(|v| v.parse().ok()).unwrap_or(0);
                (n("w"), n("h"))
            })
            .unwrap_or((0, 0))
    });
    let mut guides = crate::ShapeGuides::new(w as f64, h as f64);
    for list in ["avLst", "gdLst"] {
        for gd in cust_geom.child(list).map_or(&[][..], |l| &l.children[..]) {
            if let (Some(name), Some(fmla)) = (gd.attr("name"), gd.attr("fmla")) {
                guides.define(name, fmla);
            }
        }
    }
    let paths = path_elements
        .into_iter()
        .map(|path| parse_geometry_path(path, &guides, (w, h)))
        .collect();
    CustomGeometry { paths }
}

fn parse_geometry_path(
    path: &XmlElement,
    guides: &crate::ShapeGuides,
    shape_extent: (i64, i64),
) -> GeometryPath {
    let value = |v: Option<&str>| -> i64 { v.map_or(0, |v| guides.arg(v).round() as i64) };
    let point_of = |element: Option<&XmlElement>| -> (i64, i64) {
        (
            value(element.and_then(|el| el.attr("x"))),
            value(element.and_then(|el| el.attr("y"))),
        )
    };
    let point = |element: &XmlElement| -> (i64, i64) { point_of(element.child("pt")) };
    let points = |element: &XmlElement| -> Vec<(i64, i64)> {
        element
            .children
            .iter()
            .filter(|child| child.name == "pt")
            .map(|pt| point_of(Some(pt)))
            .collect()
    };
    let commands = path
        .children
        .iter()
        .filter_map(|command| match command.name.as_str() {
            "moveTo" => {
                let (x, y) = point(command);
                Some(PathCommand::MoveTo(x, y))
            }
            "lnTo" => {
                let (x, y) = point(command);
                Some(PathCommand::LineTo(x, y))
            }
            "quadBezTo" => {
                let pts = points(command);
                Some(PathCommand::QuadTo {
                    control: *pts.first()?,
                    to: *pts.get(1)?,
                })
            }
            "cubicBezTo" => {
                let pts = points(command);
                Some(PathCommand::CubicTo {
                    control1: *pts.first()?,
                    control2: *pts.get(1)?,
                    to: *pts.get(2)?,
                })
            }
            "arcTo" => Some(PathCommand::ArcTo {
                width_radius: value(command.attr("wR")),
                height_radius: value(command.attr("hR")),
                start_60000ths: value(command.attr("stAng")) as i32,
                sweep_60000ths: value(command.attr("swAng")) as i32,
            }),
            "close" => Some(PathCommand::Close),
            _ => None,
        })
        .collect();
    let number = |name: &str| {
        path.attr(name)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    };
    // A path without its own w/h uses the shape's coordinate space.
    let (width, height) = match (number("w"), number("h")) {
        (0, 0) => shape_extent,
        size => size,
    };
    GeometryPath {
        width,
        height,
        filled: path.attr("fill") != Some("none"),
        stroked: !matches!(path.attr("stroke"), Some("0") | Some("false")),
        commands,
    }
}

fn point_of(element: Option<&XmlElement>) -> (i64, i64) {
    let x = element
        .and_then(|el| el.attr("x"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let y = element
        .and_then(|el| el.attr("y"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    (x, y)
}

fn extent_of(element: Option<&XmlElement>) -> (i64, i64) {
    let cx = element
        .and_then(|el| el.attr("cx"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let cy = element
        .and_then(|el| el.attr("cy"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    (cx, cy)
}

fn parse_transform(xfrm: &XmlElement) -> Transform {
    Transform {
        offset_emu: point_of(xfrm.child("off")),
        extent_emu: extent_of(xfrm.child("ext")),
        rotation_60000ths: xfrm
            .attr("rot")
            .and_then(|rot| rot.parse().ok())
            .unwrap_or(0),
        flip_h: xfrm.attr("flipH") == Some("1"),
        flip_v: xfrm.attr("flipV") == Some("1"),
    }
}

/// Reads a fill from the direct children of a property container
/// (`spPr`, `ln`, `tcPr`, or `rPr`).
/// Reads one fill element itself (`solidFill`, `gradFill`, `noFill`).
pub(crate) fn parse_fill_element(element: &XmlElement) -> Option<Fill> {
    match element.name.as_str() {
        "solidFill" => parse_color(element).map(Fill::Solid),
        "gradFill" => parse_gradient(element),
        "pattFill" => parse_pattern(element),
        "noFill" => Some(Fill::None),
        _ => None,
    }
}

pub(crate) fn parse_fill(properties: &XmlElement) -> Option<Fill> {
    for child in &properties.children {
        match child.name.as_str() {
            "solidFill" => return parse_color(child).map(Fill::Solid),
            "gradFill" => return parse_gradient(child),
            "pattFill" => return parse_pattern(child),
            "noFill" => return Some(Fill::None),
            "grpFill" => return Some(Fill::Group),
            _ => {}
        }
    }
    None
}

fn parse_pattern(patt_fill: &XmlElement) -> Option<Fill> {
    Some(Fill::Pattern {
        preset: patt_fill.attr("prst").unwrap_or("pct50").to_string(),
        foreground: patt_fill.child("fgClr").and_then(parse_color)?,
        background: patt_fill.child("bgClr").and_then(parse_color)?,
    })
}

/// Parses the `dsp:spTree` of a cached SmartArt drawing: the same shape
/// grammar as a slide, under another prefix.
pub fn parse_diagram_drawing_xml(xml: &str) -> Result<Vec<ShapeItem>> {
    let root = parse_tree(xml)?;
    let mut shapes = Vec::new();
    if let Some(tree) = root.child("spTree") {
        parse_shape_children(tree, &mut shapes);
    }
    Ok(shapes)
}

/// The drawing relationship a SmartArt data part names in its
/// `dsp:dataModelExt` extension, when it does.
pub fn diagram_drawing_rel_id(data_xml: &str) -> Result<Option<String>> {
    let root = parse_tree(data_xml)?;
    Ok(root
        .descendant("dataModelExt")
        .and_then(|ext| ext.attr("relId"))
        .map(str::to_string))
}

fn parse_gradient(grad_fill: &XmlElement) -> Option<Fill> {
    let mut stops: Vec<GradientStop> = grad_fill
        .child("gsLst")?
        .children
        .iter()
        .filter(|child| child.name == "gs")
        .filter_map(|gs| {
            Some(GradientStop {
                position_thousandths: gs.attr("pos").and_then(|pos| pos.parse().ok())?,
                color: parse_color(gs)?,
            })
        })
        .collect();
    if stops.is_empty() {
        return None;
    }
    stops.sort_by_key(|stop| stop.position_thousandths);
    let angle_deg = grad_fill
        .child("lin")
        .and_then(|lin| lin.attr("ang"))
        .and_then(|ang| ang.parse::<f32>().ok())
        .filter(|ang| ang.is_finite())
        .map(|ang| ang / 60_000.0);
    let path = grad_fill.child("path").map(|path| {
        let rect = path.child("fillToRect");
        let inset = |name: &str| fraction(rect.and_then(|r| r.attr(name))).unwrap_or(0.0);
        GradientPath {
            kind: path.attr("path").unwrap_or("circle").to_string(),
            left: inset("l"),
            top: inset("t"),
            right: inset("r"),
            bottom: inset("b"),
        }
    });
    Some(Fill::Gradient {
        stops,
        angle_deg,
        path,
    })
}

/// A thousandths-of-a-percent attribute (`75000` → `0.75`).
fn fraction(value: Option<&str>) -> Option<f32> {
    value
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .map(|v| v / 100_000.0)
}

/// Reads the colour child of a fill-like element, with its transforms.
pub(crate) fn parse_color(container: &XmlElement) -> Option<Color> {
    for child in &container.children {
        let base = match child.name.as_str() {
            "srgbClr" => child.attr("val").map(|val| Color::Srgb(val.to_string())),
            "schemeClr" => child.attr("val").map(|val| Color::Scheme(val.to_string())),
            // `<a:prstClr val="black"/>` and `<a:sysClr>` carry their rendered
            // colour in `lastClr` (sysClr) or a name (prstClr); black is the
            // one preset real decks use.
            "sysClr" => child
                .attr("lastClr")
                .map(|val| Color::Srgb(val.to_string())),
            "prstClr" => Some(Color::Srgb(
                preset_color_hex(child.attr("val")?).to_string(),
            )),
            "scrgbClr" => {
                // Linear-light percentages; sRGB-encode them.
                let channel = |name: &str| {
                    let linear = fraction(child.attr(name)).unwrap_or(0.0).clamp(0.0, 1.0) as f64;
                    let encoded = if linear <= 0.003_130_8 {
                        linear * 12.92
                    } else {
                        1.055 * linear.powf(1.0 / 2.4) - 0.055
                    };
                    (encoded * 255.0).round() as u8
                };
                Some(Color::Srgb(format!(
                    "{:02X}{:02X}{:02X}",
                    channel("r"),
                    channel("g"),
                    channel("b")
                )))
            }
            "hslClr" => {
                let hue = child
                    .attr("hue")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    / 60_000.0;
                let sat = fraction(child.attr("sat")).unwrap_or(0.0) as f64;
                let lum = fraction(child.attr("lum")).unwrap_or(0.0) as f64;
                let (r, g, b) = hsl_to_rgb(hue, sat.clamp(0.0, 1.0), lum.clamp(0.0, 1.0));
                Some(Color::Srgb(format!("{r:02X}{g:02X}{b:02X}")))
            }
            _ => None,
        };
        let Some(base) = base else { continue };
        const LEGACY: [&str; 5] = ["alpha", "lumMod", "lumOff", "shade", "tint"];
        if child
            .children
            .iter()
            .any(|t| !LEGACY.contains(&t.name.as_str()))
        {
            let ops = child
                .children
                .iter()
                .map(|t| {
                    let value = match t.name.as_str() {
                        "hue" | "hueOff" => t
                            .attr("val")
                            .and_then(|v| v.parse::<f32>().ok())
                            .map_or(0.0, |v| v / 60_000.0),
                        _ => fraction(t.attr("val")).unwrap_or(0.0),
                    };
                    (t.name.clone(), value)
                })
                .collect();
            return Some(Color::Transformed {
                base: Box::new(base),
                ops,
            });
        }
        let transform = |name: &str| fraction(child.child(name).and_then(|el| el.attr("val")));
        let alpha = transform("alpha");
        let lum_mod = transform("lumMod");
        let lum_off = transform("lumOff");
        let shade = transform("shade");
        let tint = transform("tint");
        if alpha.is_none()
            && lum_mod.is_none()
            && lum_off.is_none()
            && shade.is_none()
            && tint.is_none()
        {
            return Some(base);
        }
        return Some(Color::Adjusted {
            base: Box::new(base),
            alpha,
            lum_mod,
            lum_off,
            shade,
            tint,
        });
    }
    None
}

/// `ST_PresetColorVal`: the CSS colour names in camel case, with `dk`,
/// `lt` and `med` short for dark, light and medium.
fn preset_color_hex(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let full = if let Some(rest) = lower.strip_prefix("dk") {
        format!("dark{rest}")
    } else if let Some(rest) = lower.strip_prefix("lt") {
        format!("light{rest}")
    } else if let Some(rest) = lower.strip_prefix("med").filter(|r| !r.starts_with("ium")) {
        format!("medium{rest}")
    } else {
        lower
    };
    match full.as_str() {
        "aliceblue" => "F0F8FF",
        "antiquewhite" => "FAEBD7",
        "aqua" => "00FFFF",
        "aquamarine" => "7FFFD4",
        "azure" => "F0FFFF",
        "beige" => "F5F5DC",
        "bisque" => "FFE4C4",
        "black" => "000000",
        "blanchedalmond" => "FFEBCD",
        "blue" => "0000FF",
        "blueviolet" => "8A2BE2",
        "brown" => "A52A2A",
        "burlywood" => "DEB887",
        "cadetblue" => "5F9EA0",
        "chartreuse" => "7FFF00",
        "chocolate" => "D2691E",
        "coral" => "FF7F50",
        "cornflowerblue" => "6495ED",
        "cornsilk" => "FFF8DC",
        "crimson" => "DC143C",
        "cyan" => "00FFFF",
        "darkblue" => "00008B",
        "darkcyan" => "008B8B",
        "darkgoldenrod" => "B8860B",
        "darkgray" => "A9A9A9",
        "darkgrey" => "A9A9A9",
        "darkgreen" => "006400",
        "darkkhaki" => "BDB76B",
        "darkmagenta" => "8B008B",
        "darkolivegreen" => "556B2F",
        "darkorange" => "FF8C00",
        "darkorchid" => "9932CC",
        "darkred" => "8B0000",
        "darksalmon" => "E9967A",
        "darkseagreen" => "8FBC8F",
        "darkslateblue" => "483D8B",
        "darkslategray" => "2F4F4F",
        "darkslategrey" => "2F4F4F",
        "darkturquoise" => "00CED1",
        "darkviolet" => "9400D3",
        "deeppink" => "FF1493",
        "deepskyblue" => "00BFFF",
        "dimgray" => "696969",
        "dimgrey" => "696969",
        "dodgerblue" => "1E90FF",
        "firebrick" => "B22222",
        "floralwhite" => "FFFAF0",
        "forestgreen" => "228B22",
        "fuchsia" => "FF00FF",
        "gainsboro" => "DCDCDC",
        "ghostwhite" => "F8F8FF",
        "gold" => "FFD700",
        "goldenrod" => "DAA520",
        "gray" => "808080",
        "grey" => "808080",
        "green" => "008000",
        "greenyellow" => "ADFF2F",
        "honeydew" => "F0FFF0",
        "hotpink" => "FF69B4",
        "indianred" => "CD5C5C",
        "indigo" => "4B0082",
        "ivory" => "FFFFF0",
        "khaki" => "F0E68C",
        "lavender" => "E6E6FA",
        "lavenderblush" => "FFF0F5",
        "lawngreen" => "7CFC00",
        "lemonchiffon" => "FFFACD",
        "lightblue" => "ADD8E6",
        "lightcoral" => "F08080",
        "lightcyan" => "E0FFFF",
        "lightgoldenrodyellow" => "FAFAD2",
        "lightgray" => "D3D3D3",
        "lightgrey" => "D3D3D3",
        "lightgreen" => "90EE90",
        "lightpink" => "FFB6C1",
        "lightsalmon" => "FFA07A",
        "lightseagreen" => "20B2AA",
        "lightskyblue" => "87CEFA",
        "lightslategray" => "778899",
        "lightslategrey" => "778899",
        "lightsteelblue" => "B0C4DE",
        "lightyellow" => "FFFFE0",
        "lime" => "00FF00",
        "limegreen" => "32CD32",
        "linen" => "FAF0E6",
        "magenta" => "FF00FF",
        "maroon" => "800000",
        "mediumaquamarine" => "66CDAA",
        "mediumblue" => "0000CD",
        "mediumorchid" => "BA55D3",
        "mediumpurple" => "9370DB",
        "mediumseagreen" => "3CB371",
        "mediumslateblue" => "7B68EE",
        "mediumspringgreen" => "00FA9A",
        "mediumturquoise" => "48D1CC",
        "mediumvioletred" => "C71585",
        "midnightblue" => "191970",
        "mintcream" => "F5FFFA",
        "mistyrose" => "FFE4E1",
        "moccasin" => "FFE4B5",
        "navajowhite" => "FFDEAD",
        "navy" => "000080",
        "oldlace" => "FDF5E6",
        "olive" => "808000",
        "olivedrab" => "6B8E23",
        "orange" => "FFA500",
        "orangered" => "FF4500",
        "orchid" => "DA70D6",
        "palegoldenrod" => "EEE8AA",
        "palegreen" => "98FB98",
        "paleturquoise" => "AFEEEE",
        "palevioletred" => "DB7093",
        "papayawhip" => "FFEFD5",
        "peachpuff" => "FFDAB9",
        "peru" => "CD853F",
        "pink" => "FFC0CB",
        "plum" => "DDA0DD",
        "powderblue" => "B0E0E6",
        "purple" => "800080",
        "red" => "FF0000",
        "rosybrown" => "BC8F8F",
        "royalblue" => "4169E1",
        "saddlebrown" => "8B4513",
        "salmon" => "FA8072",
        "sandybrown" => "F4A460",
        "seagreen" => "2E8B57",
        "seashell" => "FFF5EE",
        "sienna" => "A0522D",
        "silver" => "C0C0C0",
        "skyblue" => "87CEEB",
        "slateblue" => "6A5ACD",
        "slategray" => "708090",
        "slategrey" => "708090",
        "snow" => "FFFAFA",
        "springgreen" => "00FF7F",
        "steelblue" => "4682B4",
        "tan" => "D2B48C",
        "teal" => "008080",
        "thistle" => "D8BFD8",
        "tomato" => "FF6347",
        "turquoise" => "40E0D0",
        "violet" => "EE82EE",
        "wheat" => "F5DEB3",
        "white" => "FFFFFF",
        "whitesmoke" => "F5F5F5",
        "yellow" => "FFFF00",
        "yellowgreen" => "9ACD32",
        _ => "000000",
    }
}

/// HSL (hue in degrees, saturation and lightness 0..=1) to 8-bit RGB.
fn hsl_to_rgb(hue: f64, sat: f64, lum: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * lum - 1.0).abs()) * sat;
    let hp = hue.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = lum - c / 2.0;
    let byte = |v: f64| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (byte(r), byte(g), byte(b))
}

pub(crate) fn parse_outline(ln: &XmlElement) -> Outline {
    Outline {
        // A gradient stroke draws in its first stop's colour.
        color: ln.child("solidFill").and_then(parse_color).or_else(|| {
            ln.child("gradFill")
                .and_then(parse_gradient)
                .and_then(|fill| match fill {
                    Fill::Gradient { stops, .. } => stops.into_iter().next().map(|s| s.color),
                    _ => None,
                })
        }),
        width_emu: ln.attr("w").and_then(|w| w.parse().ok()),
        dash: ln
            .child("prstDash")
            .and_then(|dash| dash.attr("val"))
            .map(str::to_string),
        no_fill: ln.child("noFill").is_some(),
        head_end: ln.child("headEnd").and_then(parse_line_end),
        tail_end: ln.child("tailEnd").and_then(parse_line_end),
    }
}

fn parse_line_end(end: &XmlElement) -> Option<LineEnd> {
    let kind = end.attr("type").unwrap_or("none");
    (kind != "none").then(|| LineEnd {
        kind: kind.to_string(),
        width: end.attr("w").unwrap_or("med").to_string(),
        length: end.attr("len").unwrap_or("med").to_string(),
    })
}

fn parse_anchor(anchor: &str) -> Option<TextAnchor> {
    match anchor {
        "t" => Some(TextAnchor::Top),
        "ctr" => Some(TextAnchor::Center),
        "b" => Some(TextAnchor::Bottom),
        _ => None,
    }
}

fn parse_text_body(tx_body: &XmlElement) -> TextBody {
    let body_pr = tx_body.child("bodyPr");
    let inset = |name: &str| -> Option<i64> {
        body_pr
            .and_then(|pr| pr.attr(name))
            .and_then(|value| value.parse().ok())
    };
    let autofit = body_pr
        .and_then(|pr| {
            pr.children
                .iter()
                .find_map(|child| match child.name.as_str() {
                    "spAutoFit" => Some(Autofit::Shape),
                    "normAutofit" => Some(Autofit::Normal {
                        font_scale: fraction(child.attr("fontScale")).unwrap_or(1.0),
                        line_spacing_reduction: fraction(child.attr("lnSpcReduction"))
                            .unwrap_or(0.0),
                    }),
                    "noAutofit" => Some(Autofit::None),
                    _ => None,
                })
        })
        .unwrap_or_default();
    TextBody {
        body_property_presence: Some(TextBodyPropertyPresence {
            wrap: body_pr.is_some_and(|pr| pr.attr("wrap").is_some()),
            vertical: body_pr.is_some_and(|pr| pr.attr("vert").is_some()),
            autofit: body_pr.is_some_and(|pr| {
                pr.children
                    .iter()
                    .any(|n| matches!(n.name.as_str(), "spAutoFit" | "normAutofit" | "noAutofit"))
            }),
        }),
        anchor: body_pr
            .and_then(|pr| pr.attr("anchor"))
            .and_then(parse_anchor),
        insets: TextInsets {
            left_emu: inset("lIns"),
            top_emu: inset("tIns"),
            right_emu: inset("rIns"),
            bottom_emu: inset("bIns"),
        },
        wrap: body_pr.and_then(|pr| pr.attr("wrap")) != Some("none"),
        autofit,
        vertical: body_pr
            .and_then(|pr| pr.attr("vert"))
            .and_then(|vert| match vert {
                "eaVert" | "mongolianVert" => Some(TextVertical::EastAsian),
                "vert" | "wordArtVert" => Some(TextVertical::Rotated),
                "vert270" => Some(TextVertical::Rotated270),
                _ => None,
            }),
        list_style: tx_body
            .child("lstStyle")
            .map(parse_list_style)
            .filter(|style| !style.is_empty()),
        columns: body_pr
            .and_then(|pr| pr.attr("numCol"))
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|n| (2..=16).contains(n))
            .map(|n| {
                let spacing = body_pr
                    .and_then(|pr| pr.attr("spcCol"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                (n, spacing)
            }),
        paragraphs: tx_body
            .children
            .iter()
            .filter(|child| child.name == "p")
            .map(parse_paragraph)
            .collect(),
    }
}

/// Reads the `<a:lvlNpPr>` children of any list-style container.
fn parse_list_style(container: &XmlElement) -> ListStyle {
    let mut levels: Vec<Option<LevelStyle>> = Vec::new();
    for child in &container.children {
        let Some(level) = child
            .name
            .strip_prefix("lvl")
            .and_then(|rest| rest.strip_suffix("pPr"))
            .and_then(|digit| digit.parse::<usize>().ok())
            .filter(|level| (1..=9).contains(level))
        else {
            continue;
        };
        if levels.len() < level {
            levels.resize(level, None);
        }
        levels[level - 1] = Some(parse_level_style(child));
    }
    ListStyle { levels }
}

fn parse_level_style(p_pr: &XmlElement) -> LevelStyle {
    let props = parse_paragraph_props(p_pr);
    LevelStyle {
        align: props.align,
        margin_left_emu: props.margin_left_emu,
        margin_right_emu: props.margin_right_emu,
        indent_emu: props.indent_emu,
        bullet: props.bullet,
        bullet_font: props.bullet_font,
        bullet_font_follow_text: props.bullet_font_follow_text,
        bullet_size_percent: props.bullet_size_percent,
        bullet_color: props.bullet_color,
        line_spacing: props.line_spacing,
        space_before: props.space_before,
        space_after: props.space_after,
        default_run: p_pr
            .child("defRPr")
            .map(parse_run_props)
            .unwrap_or_default(),
    }
}

pub(crate) fn parse_run_props(r_pr: &XmlElement) -> RunProps {
    let flag = |name: &str| -> Option<bool> {
        match r_pr.attr(name) {
            Some("1") | Some("true") => Some(true),
            Some("0") | Some("false") => Some(false),
            _ => None,
        }
    };
    RunProps {
        size_pt: r_pr
            .attr("sz")
            .and_then(|sz| sz.parse::<f32>().ok())
            .filter(|sz| sz.is_finite() && *sz > 0.0)
            .map(|hundredths| hundredths / 100.0),
        bold: flag("b"),
        italic: flag("i"),
        underline: r_pr.attr("u").map(|u| u != "none"),
        // Gradient-filled text reads as its first stop, the colour most of
        // its glyphs show.
        color: r_pr
            .child("solidFill")
            .and_then(parse_color)
            .or_else(|| {
                r_pr.child("gradFill")
                    .and_then(parse_gradient)
                    .and_then(|fill| match fill {
                        Fill::Gradient { stops, .. } => stops.into_iter().next().map(|s| s.color),
                        _ => None,
                    })
            })
            // `<a:noFill/>` glyphs: only their outline, if any, shows.
            .or_else(|| {
                r_pr.child("noFill").map(|_| Color::Adjusted {
                    base: Box::new(Color::Srgb("000000".into())),
                    alpha: Some(0.0),
                    lum_mod: None,
                    lum_off: None,
                    shade: None,
                    tint: None,
                })
            }),
        font: r_pr
            .child("latin")
            .and_then(|latin| latin.attr("typeface"))
            .filter(|face| !face.is_empty())
            .map(str::to_string),
        east_asian_font: r_pr
            .child("ea")
            .and_then(|ea| ea.attr("typeface"))
            .filter(|face| !face.is_empty())
            .map(str::to_string),
    }
}

/// The paragraph-level facts `<a:pPr>` and `<a:lvlNpPr>` share.
#[derive(Default)]
struct ParagraphProps {
    align: Option<TextAlign>,
    margin_left_emu: Option<i64>,

    margin_right_emu: Option<i64>,
    indent_emu: Option<i64>,
    bullet: Option<Bullet>,
    bullet_font: Option<String>,
    bullet_font_follow_text: bool,
    bullet_size_percent: Option<f32>,
    bullet_color: Option<Color>,
    line_spacing: Option<Spacing>,
    space_before: Option<Spacing>,
    space_after: Option<Spacing>,
}

fn parse_paragraph_props(p_pr: &XmlElement) -> ParagraphProps {
    let emu = |name: &str| -> Option<i64> { p_pr.attr(name).and_then(|value| value.parse().ok()) };
    ParagraphProps {
        align: p_pr.attr("algn").and_then(parse_align),
        margin_left_emu: emu("marL"),
        margin_right_emu: emu("marR"),
        indent_emu: emu("indent"),
        bullet: parse_bullet(p_pr),
        bullet_font: p_pr
            .child("buFont")
            .and_then(|font| font.attr("typeface"))
            .map(str::to_string),
        bullet_font_follow_text: p_pr.child("buFontTx").is_some(),
        bullet_size_percent: fraction(p_pr.child("buSzPct").and_then(|el| el.attr("val"))),
        bullet_color: p_pr.child("buClr").and_then(parse_color),
        line_spacing: p_pr.child("lnSpc").and_then(parse_spacing),
        space_before: p_pr.child("spcBef").and_then(parse_spacing),
        space_after: p_pr.child("spcAft").and_then(parse_spacing),
    }
}

fn parse_spacing(container: &XmlElement) -> Option<Spacing> {
    for child in &container.children {
        match child.name.as_str() {
            "spcPct" => return fraction(child.attr("val")).map(Spacing::Percent),
            "spcPts" => {
                return child
                    .attr("val")
                    .and_then(|val| val.parse::<f32>().ok())
                    .filter(|val| val.is_finite())
                    .map(|hundredths| Spacing::Points(hundredths / 100.0));
            }
            _ => {}
        }
    }
    None
}

fn parse_paragraph(p: &XmlElement) -> TextParagraph {
    let p_pr = p.child("pPr");
    let mut runs = Vec::new();
    for child in &p.children {
        match child.name.as_str() {
            // A field (slide number, date) draws as its cached literal text.
            "r" | "fld" => {
                if let Some(mut run) = parse_run(child) {
                    if child.name == "fld" {
                        run.field = child.attr("type").map(str::to_string);
                    }
                    runs.push(run);
                }
            }
            "br" => runs.push(parse_styled_run("\n".to_string(), child.child("rPr"))),
            _ => {}
        }
    }
    let props = p_pr.map(parse_paragraph_props).unwrap_or_default();
    let mut tab_stops_emu: Vec<i64> = p_pr
        .and_then(|pr| pr.child("tabLst"))
        .map(|list| {
            list.children
                .iter()
                .filter(|tab| tab.name == "tab")
                .filter_map(|tab| tab.attr("pos")?.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    tab_stops_emu.sort_unstable();
    TextParagraph {
        tab_stops_emu,
        default_tab_emu: p_pr
            .and_then(|pr| pr.attr("defTabSz"))
            .and_then(|v| v.parse().ok())
            .filter(|v: &i64| *v > 0),
        level: p_pr
            .and_then(|pr| pr.attr("lvl"))
            .and_then(|lvl| lvl.parse().ok())
            .unwrap_or(0),
        align: props.align,
        bullet: props.bullet,
        bullet_font: props.bullet_font,
        bullet_font_follow_text: props.bullet_font_follow_text,
        bullet_size_percent: props.bullet_size_percent,
        bullet_color: props.bullet_color,
        margin_left_emu: props.margin_left_emu,
        margin_right_emu: props.margin_right_emu,
        indent_emu: props.indent_emu,
        line_spacing: props.line_spacing,
        space_before: props.space_before,
        space_after: props.space_after,
        end_size_pt: p
            .child("endParaRPr")
            .map(parse_run_props)
            .and_then(|props| props.size_pt),
        end_run_props: p
            .child("endParaRPr")
            .map(parse_run_props)
            .unwrap_or_default(),
        number_start: p_pr
            .and_then(|pr| pr.child("buAutoNum"))
            .and_then(|num| num.attr("startAt"))
            .and_then(|start| start.parse().ok()),
        runs,
    }
}

fn parse_bullet(p_pr: &XmlElement) -> Option<Bullet> {
    for child in &p_pr.children {
        match child.name.as_str() {
            "buChar" => {
                return child
                    .attr("char")
                    .map(|value| Bullet::Char(value.to_string()));
            }
            "buAutoNum" => {
                return child.attr("type").map(|value| Bullet::AutoNumber {
                    scheme: value.to_string(),
                });
            }
            "buNone" => return Some(Bullet::None),
            "buBlip" => {
                return Some(
                    child
                        .child("blip")
                        .and_then(|blip| blip.attr("embed"))
                        .map(|id| Bullet::Picture {
                            rel_id: id.to_string(),
                            image_part: None,
                        })
                        // A picture bullet without a picture reads as the
                        // round bullet it stands in for.
                        .unwrap_or_else(|| Bullet::Char("\u{2022}".to_string())),
                );
            }
            _ => {}
        }
    }
    None
}

fn parse_align(algn: &str) -> Option<TextAlign> {
    match algn {
        "l" => Some(TextAlign::Left),
        "ctr" => Some(TextAlign::Center),
        "r" => Some(TextAlign::Right),
        "just" => Some(TextAlign::Justify),
        "dist" => Some(TextAlign::Distributed),
        _ => None,
    }
}

fn parse_run(r: &XmlElement) -> Option<TextRun> {
    let text = r.child("t")?.text.clone();
    Some(parse_styled_run(text, r.child("rPr")))
}

fn parse_styled_run(text: String, r_pr: Option<&XmlElement>) -> TextRun {
    let props = r_pr.map(parse_run_props).unwrap_or_default();
    TextRun {
        text,
        bold: props.bold.unwrap_or(false),
        italic: props.italic.unwrap_or(false),
        underline: props.underline.unwrap_or(false),
        strike: r_pr
            .and_then(|pr| pr.attr("strike"))
            .is_some_and(|strike| strike != "noStrike"),
        all_caps: matches!(
            r_pr.and_then(|pr| pr.attr("cap")),
            Some("all") | Some("small")
        ),
        baseline: fraction(r_pr.and_then(|pr| pr.attr("baseline"))).filter(|b| *b != 0.0),
        highlight: r_pr
            .and_then(|pr| pr.child("highlight"))
            .and_then(parse_color),
        spacing_pt: r_pr
            .and_then(|pr| pr.attr("spc"))
            .and_then(|spc| spc.parse::<f32>().ok())
            .filter(|spc| spc.is_finite() && *spc != 0.0)
            .map(|spc| spc / 100.0),
        underline_style: r_pr
            .and_then(|pr| pr.attr("u"))
            .filter(|u| !matches!(*u, "sng" | "none" | "words"))
            .map(str::to_string),
        double_strike: r_pr.and_then(|pr| pr.attr("strike")) == Some("dblStrike"),
        outline: r_pr
            .and_then(|pr| pr.child("ln"))
            .map(parse_outline)
            .filter(|ln| !ln.no_fill && ln.color.is_some()),
        shadow: r_pr.and_then(parse_shadow),
        glow: r_pr.map(parse_effects).and_then(|effects| effects.glow),
        field: None,
        size_pt: props.size_pt,
        color: props.color,
        font: props.font,
        east_asian_font: props.east_asian_font,
        hyperlink: r_pr
            .and_then(|pr| pr.child("hlinkClick"))
            .and_then(|link| link.attr("id"))
            .map(|id| Hyperlink {
                rel_id: id.to_string(),
                target: None,
            }),
        bold_stated: props.bold,
        italic_stated: props.italic,
        underline_stated: props.underline,
    }
}
