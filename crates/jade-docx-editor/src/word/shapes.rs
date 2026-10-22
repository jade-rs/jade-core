//! 挿入 → 図形, テキスト ボックス and ワードアート, and 図形の書式's 図形の
//! スタイル, 塗りつぶし and 文字の効果, over any [`WordSession`]: a shape
//! drawn where the pointer dragged (or at a default size at the caret),
//! styled as Word styles a new one — the theme's first accent, its line a
//! shade darker, its text white — and the theme style gallery, fills,
//! WordArt and its text transforms, connectors glued to shapes.

use crate::commands::edit_drawing::edit_drawing_command;
use crate::commands::insert_image::InsertImageCommand;
use crate::commands::insert_shape::InsertShapeCommand;
use crate::commands::transform_object::TransformObjectCommand;
use crate::commands::{Command, CommandContext, CommandError, CommandResult};
pub use crate::document::TextBoxShape;
use crate::docx_xml::{XmlElement, XmlNode};
use crate::drawing_xml::{
    self, child, child_mut, descendant_mut, get_attr, put_shape_property, remove_children, set_attr,
};
use crate::word_document::{
    FloatingObjectTarget, FloatingObjectTransform, ShapeInsertSpec, WordDocument,
};
use jade_selection::{SelectionRange, TextPosition};

use super::objects::{fragment, object_run, WordObjects};
use super::WordSession;

/// A command that puts an object in, and the commands that then finish it
/// (place it, style it) once it is known where it went — one undo step.
pub struct InsertThen {
    insert: Inserted,
    then: Box<dyn FnMut(FloatingObjectTarget) -> Vec<Box<dyn Command>>>,
    applied: Vec<Box<dyn Command>>,
    selection: Option<SelectionRange>,
}

/// What [`InsertThen`] puts in.
// One per command in the undo history; the shape command's size is fine.
#[allow(clippy::large_enum_variant)]
pub enum Inserted {
    Shape(InsertShapeCommand),
    /// Boxed: an image command carries the picture's run.
    Image(Box<InsertImageCommand>),
}

impl Inserted {
    fn command(&mut self) -> &mut dyn Command {
        match self {
            Inserted::Shape(command) => command,
            Inserted::Image(command) => command.as_mut(),
        }
    }

    fn target(&self) -> Option<FloatingObjectTarget> {
        match self {
            Inserted::Shape(command) => command.inserted_target(),
            Inserted::Image(command) => command.inserted_target(),
        }
    }
}

impl InsertThen {
    pub fn new(
        insert: Inserted,
        then: impl FnMut(FloatingObjectTarget) -> Vec<Box<dyn Command>> + 'static,
    ) -> Self {
        Self {
            insert,
            then: Box::new(then),
            applied: Vec::new(),
            selection: None,
        }
    }
}

impl Command for InsertThen {
    fn name(&self) -> &'static str {
        "insert_object"
    }

    fn apply(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.insert.command().apply(ctx)?;
        let target = self
            .insert
            .target()
            .ok_or_else(|| CommandError::Custom("the object did not go in".into()))?;
        let selection = *ctx.selection;
        self.applied.clear();
        for mut command in (self.then)(target) {
            if let Err(error) = command.apply(ctx) {
                for done in self.applied.iter_mut().rev() {
                    let _ = done.undo(ctx);
                }
                self.applied.clear();
                let _ = self.insert.command().undo(ctx);
                return Err(error);
            }
            self.applied.push(command);
        }
        *ctx.selection = selection;
        self.selection = selection;
        Ok(())
    }

    fn undo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        for command in self.applied.iter_mut().rev() {
            command.undo(ctx)?;
        }
        self.insert.command().undo(ctx)
    }

    fn redo(&mut self, ctx: &mut CommandContext<'_>) -> CommandResult<()> {
        self.insert.command().redo(ctx)?;
        for command in self.applied.iter_mut() {
            command.redo(ctx)?;
        }
        *ctx.selection = self.selection;
        Ok(())
    }
}

/// A colour as a shape style writes it: one of the theme's (`accent1`,
/// `tx1`, `bg1`, …) with a shade or tint, or a fixed one (`RRGGBB`).
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Theme {
        slot: &'static str,
        /// `shade` below 1.0 darkens, `tint` lightens; 1.0 neither.
        shade: f64,
        tint: f64,
    },
    Rgb(String),
}

impl Paint {
    pub const fn theme(slot: &'static str) -> Self {
        Paint::Theme {
            slot,
            shade: 1.0,
            tint: 1.0,
        }
    }

    pub const fn shaded(slot: &'static str, shade: f64) -> Self {
        Paint::Theme {
            slot,
            shade,
            tint: 1.0,
        }
    }

    pub const fn tinted(slot: &'static str, tint: f64) -> Self {
        Paint::Theme {
            slot,
            shade: 1.0,
            tint,
        }
    }

    /// The colour element, with `alpha` (0.0–1.0) when not opaque.
    pub fn xml(&self, alpha: Option<f64>) -> String {
        let alpha = alpha
            .filter(|alpha| *alpha < 1.0)
            .map(|alpha| format!(r#"<a:alpha val="{}"/>"#, (alpha * 100_000.0).round() as i64))
            .unwrap_or_default();
        match self {
            Paint::Theme { slot, shade, tint } => {
                let mut transforms = String::new();
                if *shade < 1.0 {
                    transforms.push_str(&format!(
                        r#"<a:shade val="{}"/>"#,
                        (shade * 100_000.0).round() as i64
                    ));
                }
                if *tint < 1.0 {
                    transforms.push_str(&format!(
                        r#"<a:tint val="{}"/>"#,
                        (tint * 100_000.0).round() as i64
                    ));
                }
                format!(r#"<a:schemeClr val="{slot}">{transforms}{alpha}</a:schemeClr>"#)
            }
            Paint::Rgb(rgb) => format!(
                r#"<a:srgbClr val="{}">{alpha}</a:srgbClr>"#,
                rgb.trim_start_matches('#').to_ascii_uppercase()
            ),
        }
    }
}

/// A shape's look, as the style gallery and a new shape set it.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeStyle {
    pub fill: Option<Paint>,
    pub line: Option<Paint>,
    /// Line width in points.
    pub line_width: f64,
    pub text: Paint,
    /// An outer shadow under the shape (the gallery's 光沢/強調 rows).
    pub shadow: bool,
}

/// The theme colours down the style gallery's columns.
pub const STYLE_COLORS: [&str; 7] = [
    "tx1", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
];

impl ShapeStyle {
    /// A new shape: filled with `accent1`, outlined a shade darker, white
    /// text — Word's 既定の図形.
    pub fn default_shape() -> Self {
        Self::gallery(1, 1)
    }

    /// A new text box: white, a thin black outline, black text.
    pub fn text_box() -> Self {
        ShapeStyle {
            fill: Some(Paint::theme("lt1")),
            line: Some(Paint::theme("tx1")),
            line_width: 0.5,
            text: Paint::theme("tx1"),
            shadow: false,
        }
    }

    /// A new line or connector: `accent1`, half a point.
    pub fn line() -> Self {
        ShapeStyle {
            fill: None,
            line: Some(Paint::theme("accent1")),
            line_width: 0.5,
            text: Paint::theme("tx1"),
            shadow: false,
        }
    }

    /// The theme style gallery: `row` (枠線のみ, 塗りつぶし, 枠線あり, パステル,
    /// グラデーション風, 光沢) by `column` ([`STYLE_COLORS`]).
    pub fn gallery(row: usize, column: usize) -> Self {
        let color = STYLE_COLORS[column.min(STYLE_COLORS.len() - 1)];
        let on_color = if color == "tx1" { "bg1" } else { "lt1" };
        match row {
            0 => ShapeStyle {
                fill: Some(Paint::theme("lt1")),
                line: Some(Paint::theme(color)),
                line_width: 1.0,
                text: Paint::theme(color),
                shadow: false,
            },
            1 => ShapeStyle {
                fill: Some(Paint::theme(color)),
                line: Some(Paint::shaded(color, 0.5)),
                line_width: 1.0,
                text: Paint::theme(on_color),
                shadow: false,
            },
            2 => ShapeStyle {
                fill: Some(Paint::theme(color)),
                line: Some(Paint::theme("lt1")),
                line_width: 3.0,
                text: Paint::theme(on_color),
                shadow: true,
            },
            3 => ShapeStyle {
                fill: Some(Paint::tinted(color, 0.4)),
                line: Some(Paint::theme(color)),
                line_width: 1.0,
                text: Paint::theme("tx1"),
                shadow: false,
            },
            4 => ShapeStyle {
                fill: Some(Paint::tinted(color, 0.2)),
                line: None,
                line_width: 0.0,
                text: Paint::shaded(color, 0.5),
                shadow: false,
            },
            _ => ShapeStyle {
                fill: Some(Paint::shaded(color, 0.75)),
                line: None,
                line_width: 0.0,
                text: Paint::theme(on_color),
                shadow: true,
            },
        }
    }

    pub const GALLERY_ROWS: usize = 6;
}

/// Writes `style` into a shape's drawing: the fill and line in `wps:spPr`,
/// Word's `wps:style` references, and the text colour of the text inside.
pub fn apply_style(drawing: &mut XmlElement, style: &ShapeStyle) -> Result<(), String> {
    let sp_pr = drawing_xml::shape_properties_mut(drawing).ok_or("the object is not a shape")?;
    let fill = match &style.fill {
        Some(paint) => format!("<a:solidFill>{}</a:solidFill>", paint.xml(None)),
        None => "<a:noFill/>".to_string(),
    };
    put_shape_property(
        sp_pr,
        fragment(&fill)?,
        &[
            "a:noFill",
            "a:solidFill",
            "a:gradFill",
            "a:blipFill",
            "a:pattFill",
            "a:grpFill",
        ],
    );
    let line = match &style.line {
        Some(paint) => format!(
            r#"<a:ln w="{}"><a:solidFill>{}</a:solidFill></a:ln>"#,
            (style.line_width * 12_700.0).round() as i64,
            paint.xml(None)
        ),
        None => "<a:ln><a:noFill/></a:ln>".to_string(),
    };
    // A line keeps its arrowheads.
    let ends: Vec<XmlElement> = child(sp_pr, "a:ln")
        .map(|line| {
            drawing_xml::elements(line)
                .filter(|element| matches!(element.name.as_str(), "a:headEnd" | "a:tailEnd"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut line = fragment(&line)?;
    line.children.extend(ends.into_iter().map(XmlNode::Element));
    put_shape_property(sp_pr, line, &["a:ln"]);
    remove_children(sp_pr, &["a:effectLst", "a:effectDag"]);
    if style.shadow {
        put_shape_property(
            sp_pr,
            fragment(
                r#"<a:effectLst><a:outerShdw blurRad="40000" dist="23000" dir="5400000" rotWithShape="0"><a:srgbClr val="000000"><a:alpha val="35000"/></a:srgbClr></a:outerShdw></a:effectLst>"#,
            )?,
            &["a:effectLst"],
        );
    }
    // Word's style references, for Word's own style gallery.
    let reference = |paint: &Option<Paint>| {
        paint
            .as_ref()
            .map(|paint| paint.xml(None))
            .unwrap_or_else(|| r#"<a:schemeClr val="accent1"/>"#.to_string())
    };
    let wps_style = fragment(&format!(
        r#"<wps:style><a:lnRef idx="{}">{}</a:lnRef><a:fillRef idx="{}">{}</a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor">{}</a:fontRef></wps:style>"#,
        if style.line.is_some() { 2 } else { 0 },
        reference(&style.line),
        if style.fill.is_some() { 1 } else { 0 },
        reference(&style.fill),
        style.text.xml(None),
    ))?;
    if let Some(shape) = descendant_mut(drawing, "wps:wsp") {
        let before = drawing_xml::child_index(shape, &["wps:txbx", "wps:linkedTxbx", "wps:bodyPr"])
            .unwrap_or(shape.children.len());
        remove_children(shape, &["wps:style"]);
        let before = before.min(shape.children.len());
        shape.children.insert(before, XmlNode::Element(wps_style));
    }
    // The text inside takes the style's colour.
    set_text_color(drawing, &style.text)?;
    Ok(())
}

/// Colours every run inside a shape's text: `w:color` on each `w:r`.
fn set_text_color(drawing: &mut XmlElement, paint: &Paint) -> Result<(), String> {
    let value = match paint {
        Paint::Rgb(rgb) => rgb.trim_start_matches('#').to_ascii_uppercase(),
        // Word writes the theme colour with its resolved value; jade writes
        // the theme reference and `auto` falls back where it cannot resolve.
        Paint::Theme { slot, .. } => match *slot {
            "lt1" | "bg1" => "FFFFFF".to_string(),
            _ => "000000".to_string(),
        },
    };
    let theme = match paint {
        Paint::Theme { slot, .. } => Some(match *slot {
            "lt1" => "light1",
            "bg1" => "background1",
            "tx1" => "text1",
            "dk1" => "dark1",
            other => other,
        }),
        Paint::Rgb(_) => None,
    };
    fn walk(element: &mut XmlElement, value: &str, theme: Option<&str>) -> Result<(), String> {
        if element.name == "w:r" {
            if child(element, "w:rPr").is_none() {
                element
                    .children
                    .insert(0, XmlNode::Element(fragment("<w:rPr/>")?));
            }
            let properties = child_mut(element, "w:rPr").expect("added above");
            remove_children(properties, &["w:color"]);
            let mut color = format!(r#"<w:color w:val="{value}""#);
            if let Some(theme) = theme {
                color.push_str(&format!(r#" w:themeColor="{theme}""#));
            }
            color.push_str("/>");
            properties
                .children
                .push(XmlNode::Element(fragment(&color)?));
            return Ok(());
        }
        for child in &mut element.children {
            if let XmlNode::Element(child) = child {
                walk(child, value, theme)?;
            }
        }
        Ok(())
    }
    if let Some(content) = descendant_mut(drawing, "w:txbxContent") {
        walk(content, &value, theme)?;
        // The paragraph mark too, so typing in an empty box is in colour.
        for paragraph in content.children.iter_mut() {
            let XmlNode::Element(paragraph) = paragraph else {
                continue;
            };
            if paragraph.name != "w:p" {
                continue;
            }
            if child(paragraph, "w:pPr").is_none() {
                paragraph
                    .children
                    .insert(0, XmlNode::Element(fragment("<w:pPr/>")?));
            }
            let properties = child_mut(paragraph, "w:pPr").expect("added above");
            if child(properties, "w:rPr").is_none() {
                properties
                    .children
                    .push(XmlNode::Element(fragment("<w:rPr/>")?));
            }
            let mark = child_mut(properties, "w:rPr").expect("added above");
            remove_children(mark, &["w:color"]);
            mark.children.push(XmlNode::Element(fragment(&match theme {
                Some(theme) => format!(r#"<w:color w:val="{value}" w:themeColor="{theme}"/>"#),
                None => format!(r#"<w:color w:val="{value}"/>"#),
            })?));
        }
    }
    Ok(())
}

/// 図形の塗りつぶし.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    None,
    Paint(Paint),
    /// 透明度 of the current solid fill, 0.0–1.0.
    Transparency(f64),
}

pub fn apply_fill(drawing: &mut XmlElement, fill: &Fill) -> Result<(), String> {
    let sp_pr = drawing_xml::shape_properties_mut(drawing).ok_or("the object is not a shape")?;
    const FILLS: [&str; 6] = [
        "a:noFill",
        "a:solidFill",
        "a:gradFill",
        "a:blipFill",
        "a:pattFill",
        "a:grpFill",
    ];
    match fill {
        Fill::None => put_shape_property(sp_pr, fragment("<a:noFill/>")?, &FILLS),
        Fill::Paint(paint) => put_shape_property(
            sp_pr,
            fragment(&format!("<a:solidFill>{}</a:solidFill>", paint.xml(None)))?,
            &FILLS,
        ),
        Fill::Transparency(transparency) => {
            let solid = child_mut(sp_pr, "a:solidFill").ok_or("the shape has no solid fill")?;
            let color = solid
                .children
                .iter_mut()
                .find_map(|node| match node {
                    XmlNode::Element(element) => Some(element),
                    _ => None,
                })
                .ok_or("the fill has no colour")?;
            remove_children(color, &["a:alpha"]);
            if *transparency > 0.0 {
                color.children.push(XmlNode::Element(fragment(&format!(
                    r#"<a:alpha val="{}"/>"#,
                    ((1.0 - transparency.clamp(0.0, 1.0)) * 100_000.0).round() as i64
                ))?));
            }
        }
    }
    Ok(())
}

/// ワードアートのスタイル: the gallery Word offers when inserting WordArt,
/// as fill, outline and shadow of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordArtStyle {
    /// 塗りつぶし: 黒、文字色 1; 影.
    BlackShadow,
    /// 塗りつぶし: 青、アクセントカラー 1; 影.
    AccentShadow,
    /// 塗りつぶし: 白; 輪郭: 青、アクセントカラー 1; 光彩.
    WhiteOutlineGlow,
    /// 塗りつぶしなし; 輪郭: 青、アクセントカラー 1.
    OutlineOnly,
    /// 塗りつぶし: オレンジ、アクセントカラー 2; 輪郭: オレンジ.
    OrangeOutline,
    /// 塗りつぶし: 灰色; 面取り (サブトル).
    GrayBevel,
}

impl WordArtStyle {
    pub const ALL: [WordArtStyle; 6] = [
        WordArtStyle::BlackShadow,
        WordArtStyle::AccentShadow,
        WordArtStyle::WhiteOutlineGlow,
        WordArtStyle::OutlineOnly,
        WordArtStyle::OrangeOutline,
        WordArtStyle::GrayBevel,
    ];

    /// The run properties Word writes for the style: `w:color` and its
    /// Word 2010 text effects.
    pub fn run_properties(self) -> String {
        let (color, effects) = match self {
            WordArtStyle::BlackShadow => (
                r#"<w:color w:val="000000" w:themeColor="text1"/>"#,
                r#"<w14:shadow w14:blurRad="38100" w14:dist="19050" w14:dir="2700000" w14:sx="100000" w14:sy="100000" w14:kx="0" w14:ky="0" w14:algn="tl"><w14:srgbClr w14:val="000000"><w14:alpha w14:val="60000"/></w14:srgbClr></w14:shadow>"#,
            ),
            WordArtStyle::AccentShadow => (
                r#"<w:color w:val="4472C4" w:themeColor="accent1"/>"#,
                r#"<w14:shadow w14:blurRad="38100" w14:dist="19050" w14:dir="2700000" w14:sx="100000" w14:sy="100000" w14:kx="0" w14:ky="0" w14:algn="tl"><w14:srgbClr w14:val="000000"><w14:alpha w14:val="60000"/></w14:srgbClr></w14:shadow>"#,
            ),
            WordArtStyle::WhiteOutlineGlow => (
                r#"<w:color w:val="FFFFFF" w:themeColor="background1"/>"#,
                r#"<w14:glow w14:rad="38100"><w14:schemeClr w14:val="accent1"><w14:alpha w14:val="60000"/></w14:schemeClr></w14:glow><w14:textOutline w14:w="9525" w14:cap="flat" w14:cmpd="sng" w14:algn="ctr"><w14:solidFill><w14:schemeClr w14:val="accent1"/></w14:solidFill><w14:prstDash w14:val="solid"/><w14:round/></w14:textOutline>"#,
            ),
            WordArtStyle::OutlineOnly => (
                r#"<w:color w:val="FFFFFF" w:themeColor="background1"/>"#,
                r#"<w14:textOutline w14:w="9525" w14:cap="flat" w14:cmpd="sng" w14:algn="ctr"><w14:solidFill><w14:schemeClr w14:val="accent1"/></w14:solidFill><w14:prstDash w14:val="solid"/><w14:round/></w14:textOutline><w14:textFill><w14:noFill/></w14:textFill>"#,
            ),
            WordArtStyle::OrangeOutline => (
                r#"<w:color w:val="ED7D31" w:themeColor="accent2"/>"#,
                r#"<w14:textOutline w14:w="9525" w14:cap="flat" w14:cmpd="sng" w14:algn="ctr"><w14:solidFill><w14:schemeClr w14:val="accent2"><w14:lumMod w14:val="75000"/></w14:schemeClr></w14:solidFill><w14:prstDash w14:val="solid"/><w14:round/></w14:textOutline>"#,
            ),
            WordArtStyle::GrayBevel => (
                r#"<w:color w:val="7F7F7F" w:themeColor="text1" w:themeTint="80"/>"#,
                r#"<w14:props3d w14:extrusionH="0" w14:contourW="0" w14:prstMaterial="warmMatte"><w14:bevelT w14:w="38100" w14:h="38100" w14:prst="circle"/></w14:props3d>"#,
            ),
        };
        format!(
            r#"<w:rPr xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:b/><w:noProof/>{color}<w:sz w:val="72"/><w:szCs w:val="72"/>{effects}</w:rPr>"#
        )
    }
}

/// 文字の効果 → 変形: the text transforms Word offers, by `a:prstTxWarp`
/// preset, in the gallery's order.
pub const TEXT_WARPS: [&str; 20] = [
    "textNoShape",
    "textArchUp",
    "textArchDown",
    "textCircle",
    "textButton",
    "textWave1",
    "textWave2",
    "textDoubleWave1",
    "textSlantUp",
    "textSlantDown",
    "textTriangle",
    "textTriangleInverted",
    "textChevron",
    "textChevronInverted",
    "textInflate",
    "textDeflate",
    "textFadeRight",
    "textFadeLeft",
    "textCanUp",
    "textCanDown",
];

/// Where a new object goes when the user drew it: a box on a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Default sizes, in CSS px, of an object put in with a click rather than a
/// drag: Word's 2.54 cm square for shapes, its wider box for text.
pub const DEFAULT_SHAPE: (f64, f64) = (96.0, 96.0);
pub const DEFAULT_TEXT_BOX: (f64, f64) = (216.0, 96.0);

/// The shapes' tools, for every session.
pub trait WordShapes: WordSession + WordObjects {
    /// 挿入 → 図形: `shape` anchored at the caret's paragraph, at `drawn`
    /// on the page (or the margins' top left at its default size), styled as
    /// a new shape — or, with `text_box`, as a new text box.
    fn insert_shape(
        &mut self,
        shape: TextBoxShape,
        drawn: Option<Drawn>,
        text_box: bool,
    ) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, _) = range.normalized();
        let (width, height) = match drawn {
            Some(drawn) => (drawn.width.max(12.0), drawn.height.max(12.0)),
            None if text_box => DEFAULT_TEXT_BOX,
            None => DEFAULT_SHAPE,
        };
        // A line has no height to speak of when drawn level.
        let (width, height) = if shape.is_line() {
            (width.max(12.0), height.max(12.0))
        } else {
            (width, height)
        };
        let insert = InsertShapeCommand::new(
            start.paragraph,
            start.offset,
            ShapeInsertSpec {
                shape,
                width_px: width.min(4096.0),
                height_px: height.min(4096.0),
                fill_color: None,
                border_color: Some("000000".into()),
                border_width_px: 1.0,
                text: String::new(),
                text_color: None,
                text_font_size_pt: None,
                text_bold: None,
                text_italic: None,
                text_alignment: None,
            },
        );
        let style = if shape.is_line() {
            ShapeStyle::line()
        } else if text_box {
            ShapeStyle::text_box()
        } else {
            ShapeStyle::default_shape()
        };
        let name = if text_box {
            "テキスト ボックス"
        } else if shape.is_line() {
            "直線コネクタ"
        } else {
            "正方形/長方形"
        };
        self.execute_command(
            Box::new(InsertThen::new(Inserted::Shape(insert), move |target| {
                let style = style.clone();
                let mut then: Vec<Box<dyn Command>> = Vec::new();
                if let Some(drawn) = drawn {
                    then.push(Box::new(TransformObjectCommand::new(
                        target,
                        FloatingObjectTransform {
                            x: drawn.x,
                            y: drawn.y,
                            width,
                            height,
                            rotation_deg: 0.0,
                        },
                    )));
                }
                then.push(Box::new(edit_drawing_command(
                    "shape_style",
                    target,
                    move |_, drawing| {
                        apply_style(drawing, &style)?;
                        if let Some(doc_pr) = descendant_mut(drawing, "wp:docPr") {
                            let id = get_attr(doc_pr, "id").unwrap_or("1").to_string();
                            set_attr(doc_pr, "name", Some(format!("{name} {id}")));
                        }
                        if !text_box {
                            // A shape's text sits in its middle.
                            if let Some(body) = descendant_mut(drawing, "wps:bodyPr") {
                                set_attr(body, "anchor", Some("ctr".into()));
                            }
                            if let Some(content) = descendant_mut(drawing, "w:txbxContent") {
                                for paragraph in content.children.iter_mut() {
                                    if let XmlNode::Element(paragraph) = paragraph {
                                        center_paragraph(paragraph)?;
                                    }
                                }
                            }
                        } else if let Some(shape) = descendant_mut(drawing, "wps:cNvSpPr") {
                            set_attr(shape, "txBox", Some("1".into()));
                        }
                        Ok(())
                    },
                )));
                then
            })),
            false,
        )
    }

    /// 図形のスタイル: one of the theme style gallery's.
    fn apply_shape_style(
        &mut self,
        target: FloatingObjectTarget,
        style: ShapeStyle,
    ) -> Result<(), String> {
        self.edit_object(target, "shape_style", move |_, drawing| {
            apply_style(drawing, &style)
        })
    }

    /// 図形の塗りつぶし.
    fn set_shape_fill(&mut self, target: FloatingObjectTarget, fill: Fill) -> Result<(), String> {
        self.edit_object(target, "shape_fill", move |_, drawing| {
            apply_fill(drawing, &fill)
        })
    }

    /// 文字の塗りつぶし: the colour of all the shape's text.
    fn set_shape_text_color(
        &mut self,
        target: FloatingObjectTarget,
        paint: Paint,
    ) -> Result<(), String> {
        self.edit_object(target, "shape_text_color", move |_, drawing| {
            set_text_color(drawing, &paint)
        })
    }

    /// 文字の配置: the text's place in the box — `top`, `center` or
    /// `bottom` (`a:bodyPr@anchor`).
    fn set_text_anchor(&mut self, target: FloatingObjectTarget, align: &str) -> Result<(), String> {
        use crate::commands::format_text_box::{FormatTextBoxCommand, TextBoxFormatSpec};
        use crate::document::TextBoxVerticalAlignment;
        let alignment = match align {
            "top" => TextBoxVerticalAlignment::Top,
            "bottom" => TextBoxVerticalAlignment::Bottom,
            _ => TextBoxVerticalAlignment::Center,
        };
        self.execute_command(
            Box::new(FormatTextBoxCommand::new(
                target,
                TextBoxFormatSpec {
                    text_vertical_align: Some(alignment),
                    ..Default::default()
                },
            )),
            true,
        )
    }

    /// 挿入 → ワードアート: `text` in a box with no fill or line, in
    /// `style`, at the caret's paragraph.
    fn insert_wordart(
        &mut self,
        style: WordArtStyle,
        text: &str,
        drawn: Option<Drawn>,
    ) -> Result<(), String> {
        let range = self.selection_range().ok_or("no caret")?;
        let (start, _) = range.normalized();
        let text = if text.is_empty() {
            "ここに文字を入力".to_string()
        } else {
            text.to_string()
        };
        let insert = InsertShapeCommand::new(
            start.paragraph,
            start.offset,
            ShapeInsertSpec {
                shape: TextBoxShape::Rectangle,
                width_px: 360.0,
                height_px: 72.0,
                fill_color: None,
                border_color: None,
                border_width_px: 0.0,
                text: text.clone(),
                text_color: None,
                text_font_size_pt: Some(36.0),
                text_bold: Some(true),
                text_italic: None,
                text_alignment: Some("center".into()),
            },
        );
        self.execute_command(
            Box::new(InsertThen::new(Inserted::Shape(insert), move |target| {
                let text = text.clone();
                let mut then: Vec<Box<dyn Command>> = Vec::new();
                if let Some(drawn) = drawn {
                    then.push(Box::new(TransformObjectCommand::new(
                        target,
                        FloatingObjectTransform {
                            x: drawn.x,
                            y: drawn.y,
                            width: 360.0,
                            height: 72.0,
                            rotation_deg: 0.0,
                        },
                    )));
                }
                then.push(Box::new(edit_drawing_command("insert_wordart", target, move |_, drawing| {
                    let sp_pr = drawing_xml::shape_properties_mut(drawing).ok_or("no shape")?;
                    apply_fill_none_line_none(sp_pr)?;
                    let content = descendant_mut(drawing, "w:txbxContent").ok_or("no text")?;
                    content.children = vec![XmlNode::Element(fragment(&format!(
                        r#"<w:p><w:pPr><w:jc w:val="center"/>{}</w:pPr><w:r>{}<w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                        style.run_properties(),
                        style.run_properties(),
                        escape(&text)
                    ))?)];
                    if let Some(body) = descendant_mut(drawing, "wps:bodyPr") {
                        set_attr(body, "wrap", Some("none".into()));
                        remove_children(body, &["a:noAutofit", "a:normAutofit", "a:spAutoFit"]);
                        body.children.push(XmlNode::Element(fragment(
                            r#"<a:prstTxWarp prst="textNoShape"><a:avLst/></a:prstTxWarp>"#,
                        )?));
                        body.children.push(XmlNode::Element(fragment("<a:spAutoFit/>")?));
                    }
                    if let Some(shape) = descendant_mut(drawing, "wps:cNvSpPr") {
                        set_attr(shape, "txBox", Some("1".into()));
                    }
                    if let Some(doc_pr) = descendant_mut(drawing, "wp:docPr") {
                        let id = get_attr(doc_pr, "id").unwrap_or("1").to_string();
                        set_attr(doc_pr, "name", Some(format!("テキスト ボックス {id}")));
                    }
                    Ok(())
                })));
                then
            })),
            false,
        )
    }

    /// 文字の効果 → 変形: WordArt's text transform; `textNoShape` takes it
    /// off.
    fn set_text_warp(
        &mut self,
        target: FloatingObjectTarget,
        preset: &'static str,
    ) -> Result<(), String> {
        self.edit_object(target, "text_warp", move |_, drawing| {
            let body = descendant_mut(drawing, "wps:bodyPr").ok_or("the object has no text")?;
            remove_children(body, &["a:prstTxWarp"]);
            let warp = fragment(&format!(
                r#"<a:prstTxWarp prst="{preset}"><a:avLst/></a:prstTxWarp>"#
            ))?;
            // `a:prstTxWarp` comes first in `wps:bodyPr`.
            body.children.insert(0, XmlNode::Element(warp));
            Ok(())
        })
    }

    /// テキスト ボックスのリンクの作成: the empty text box `to` takes the text
    /// that does not fit in `from` (and the boxes already linked after it).
    fn link_text_boxes(
        &mut self,
        from: FloatingObjectTarget,
        to: FloatingObjectTarget,
    ) -> Result<(), String> {
        let document = self.word_document();
        let box_of = |target: FloatingObjectTarget| {
            object_run(document, target).and_then(|run| run.text_box.clone())
        };
        let source = box_of(from).ok_or("the first object is not a text box")?;
        let destination = box_of(to).ok_or("the second object is not a text box")?;
        if !destination.text.trim().is_empty() {
            return Err("a text box can only be linked to an empty one".into());
        }
        // The chain's id, and the next place in it.
        let links: Vec<crate::document::TextBoxLink> = super::objects::body_objects(document)
            .iter()
            .filter_map(|object| object_run(document, object.target))
            .filter_map(|run| {
                run.text_box
                    .as_ref()
                    .and_then(|text_box| text_box.extras().link)
            })
            .collect();
        let (id, start_chain) = match source.extras().link {
            Some(link) => (link.id, false),
            None => (
                links.iter().map(|link| link.id).max().unwrap_or(0) + 1,
                true,
            ),
        };
        let sequence = links
            .iter()
            .filter(|link| link.id == id)
            .map(|link| link.sequence)
            .max()
            .map(|last| last + 1)
            .unwrap_or(1);
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        // The follower first: it is emptied of its text box paragraphs,
        // which renumbers what follows it but not what comes before.
        let follower = crate::commands::edit_drawing::replace_drawing_command(
            "link_text_boxes",
            to,
            move |_, drawing| {
                let shape =
                    descendant_mut(drawing, "wps:wsp").ok_or("the object is not a shape")?;
                remove_children(shape, &["wps:txbx", "wps:linkedTxbx"]);
                let at = drawing_xml::child_index(shape, &["wps:bodyPr"])
                    .unwrap_or(shape.children.len());
                shape.children.insert(
                    at,
                    XmlNode::Element(fragment(&format!(
                        r#"<wps:linkedTxbx id="{id}" seq="{sequence}"/>"#
                    ))?),
                );
                Ok(())
            },
        );
        let head = start_chain.then(|| {
            edit_drawing_command("link_text_boxes", from, move |_, drawing| {
                let story =
                    descendant_mut(drawing, "wps:txbx").ok_or("the text box has no text")?;
                set_attr(story, "id", Some(id.to_string()));
                Ok(())
            })
        });
        let (from_first, to_first) = (
            (from.paragraph_index, from.run_index),
            (to.paragraph_index, to.run_index),
        );
        if to_first > from_first {
            commands.push(Box::new(follower));
            commands.extend(head.map(|head| Box::new(head) as Box<dyn Command>));
        } else {
            commands.extend(head.map(|head| Box::new(head) as Box<dyn Command>));
            commands.push(Box::new(follower));
        }
        self.execute_command(
            Box::new(super::objects::Sequence::new("link_text_boxes", commands)),
            true,
        )
    }

    /// The object at `target`'s shape geometry changed to `shape`
    /// (図形の変更).
    fn change_shape(
        &mut self,
        target: FloatingObjectTarget,
        shape: TextBoxShape,
    ) -> Result<(), String> {
        self.edit_object(target, "change_shape", move |_, drawing| {
            let sp_pr =
                drawing_xml::shape_properties_mut(drawing).ok_or("the object is not a shape")?;
            put_shape_property(
                sp_pr,
                fragment(&format!(
                    r#"<a:prstGeom prst="{}"><a:avLst/></a:prstGeom>"#,
                    shape.preset_name()
                ))?,
                &["a:custGeom", "a:prstGeom"],
            );
            Ok(())
        })
    }
}

impl<T: WordSession + ?Sized> WordShapes for T {}

fn apply_fill_none_line_none(sp_pr: &mut XmlElement) -> Result<(), String> {
    put_shape_property(
        sp_pr,
        fragment("<a:noFill/>")?,
        &[
            "a:noFill",
            "a:solidFill",
            "a:gradFill",
            "a:blipFill",
            "a:pattFill",
        ],
    );
    put_shape_property(sp_pr, fragment("<a:ln><a:noFill/></a:ln>")?, &["a:ln"]);
    Ok(())
}

fn center_paragraph(paragraph: &mut XmlElement) -> Result<(), String> {
    if paragraph.name != "w:p" {
        return Ok(());
    }
    if child(paragraph, "w:pPr").is_none() {
        paragraph
            .children
            .insert(0, XmlNode::Element(fragment("<w:pPr/>")?));
    }
    let properties = child_mut(paragraph, "w:pPr").expect("added above");
    remove_children(properties, &["w:jc"]);
    let at = drawing_xml::child_index(properties, &["w:rPr", "w:sectPr", "w:pPrChange"])
        .unwrap_or(properties.children.len());
    properties
        .children
        .insert(at, XmlNode::Element(fragment(r#"<w:jc w:val="center"/>"#)?));
    Ok(())
}

pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The caret at `position`, for a command that inserts there.
pub fn caret(position: (usize, usize)) -> SelectionRange {
    SelectionRange::collapsed(TextPosition::new(position.0, position.1))
}

/// Whether the object at `target` is a shape holding text.
pub fn has_text(document: &WordDocument, target: FloatingObjectTarget) -> bool {
    object_run(document, target)
        .and_then(|run| run.text_box.as_ref())
        .is_some_and(|text_box| !text_box.shape.is_line())
}
