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
    },
    /// `<a:pattFill prst>`: a hatch of `foreground` over `background`.
    Pattern {
        preset: String,
        foreground: Color,
        background: Color,
    },
    /// `<a:noFill/>`: explicitly transparent, distinct from "inherit".
    None,
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
            .and_then(parse_fill),
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
        custom_geometry: sp_pr
            .and_then(|pr| pr.child("custGeom"))
            .map(parse_custom_geometry),
        fill: sp_pr.and_then(parse_fill),
        outline: sp_pr.and_then(|pr| pr.child("ln")).map(parse_outline),
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
    }
}

fn parse_grp(grp: &XmlElement) -> GroupShape {
    let mut children = Vec::new();
    parse_shape_children(grp, &mut children);
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

fn parse_custom_geometry(cust_geom: &XmlElement) -> CustomGeometry {
    let paths = cust_geom
        .child("pathLst")
        .map(|list| {
            list.children
                .iter()
                .filter(|child| child.name == "path")
                .map(parse_geometry_path)
                .collect()
        })
        .unwrap_or_default();
    CustomGeometry { paths }
}

fn parse_geometry_path(path: &XmlElement) -> GeometryPath {
    let number = |value: Option<&str>| -> i64 { value.and_then(|v| v.parse().ok()).unwrap_or(0) };
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
                width_radius: number(command.attr("wR")),
                height_radius: number(command.attr("hR")),
                start_60000ths: number(command.attr("stAng")) as i32,
                sweep_60000ths: number(command.attr("swAng")) as i32,
            }),
            "close" => Some(PathCommand::Close),
            _ => None,
        })
        .collect();
    GeometryPath {
        width: number(path.attr("w")),
        height: number(path.attr("h")),
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
    Some(Fill::Gradient { stops, angle_deg })
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
            _ => None,
        };
        let Some(base) = base else { continue };
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

fn preset_color_hex(name: &str) -> &'static str {
    match name {
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "gray" | "grey" => "808080",
        "ltGray" | "lightGray" => "D3D3D3",
        "dkGray" | "darkGray" => "A9A9A9",
        _ => "000000",
    }
}

pub(crate) fn parse_outline(ln: &XmlElement) -> Outline {
    Outline {
        color: ln.child("solidFill").and_then(parse_color),
        width_emu: ln.attr("w").and_then(|w| w.parse().ok()),
        dash: ln
            .child("prstDash")
            .and_then(|dash| dash.attr("val"))
            .map(str::to_string),
        no_fill: ln.child("noFill").is_some(),
    }
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

fn parse_run_props(r_pr: &XmlElement) -> RunProps {
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
        color: r_pr.child("solidFill").and_then(parse_color),
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
    TextParagraph {
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
