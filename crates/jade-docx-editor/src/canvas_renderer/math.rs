use crate::document::MathParagraph;
#[cfg(test)]
use crate::document::{MathElement, MathElementKind};
use jade_docx_layout::math_layout::MathMetrics;
pub(crate) use jade_docx_layout::math_layout::math_text;
use jade_docx_layout::units::points_to_px;
use jade_docx_reader::{ParagraphProperties, StylesPart, ThemePart};
use jade_painter::Painter;
#[cfg(feature = "opentype-math")]
use std::rc::Rc;

use jade_docx_layout::math_layout::styled::{MathTextStyle, StyledMathLayout, StyledMathMeasure};
use std::cell::RefCell;

struct StyledPainterMeasure<'a, P> {
    painter: &'a P,
    font: RefCell<Option<(String, f64)>>,
}
fn cambria_math_metrics(text: &str, style: MathTextStyle<'_>) -> Option<(f64, f64)> {
    use jade_docx_layout::line_metrics::{WordLayoutPlatform, word_layout_platform};
    if !style.family.eq_ignore_ascii_case("Cambria Math")
        || word_layout_platform() != WordLayoutPlatform::Mac
    {
        return None;
    }
    // Cambria Math advances in 2048 units per em, also observed in Word's
    // printed display equations. Canvas may be measuring a substituted face.
    const ITALIC_LATIN: [(u16, u16); 52] = [
        (1297, 20), // A
        (1344, 55), // B
        (1222, 90), // C
        (1418, 60), // D
        (1279, 80), // E
        (1251, 80), // F
        (1290, 90), // G
        (1491, 60), // H
        (776, 65),  // I
        (782, 40),  // J
        (1430, 62), // K
        (1089, 42), // L
        (1743, 50), // M
        (1500, 50), // N
        (1385, 50), // O
        (1270, 50), // P
        (1385, 60), // Q
        (1305, 60), // R
        (1084, 40), // S
        (1219, 45), // T
        (1411, 40), // U
        (1281, 65), // V
        (1945, 80), // W
        (1321, 50), // X
        (1234, 50), // Y
        (1213, 50), // Z
        (1141, 50), // a
        (1104, 45), // b
        (942, 65),  // c
        (1187, 65), // d
        (1016, 45), // e
        (1126, 60), // f
        (1253, 35), // g
        (1136, 35), // h
        (649, 65),  // i
        (736, 35),  // j
        (1130, 60), // k
        (647, 65),  // l
        (1717, 35), // m
        (1175, 35), // n
        (1091, 45), // o
        (1142, 25), // p
        (1103, 62), // q
        (974, 45),  // r
        (946, 42),  // s
        (808, 50),  // t
        (1175, 52), // u
        (1114, 60), // v
        (1513, 60), // w
        (1090, 62), // x
        (1146, 35), // y
        (990, 35),  // z
    ];
    let mut characters = text.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    let (advance, italic) = match character {
        '0'..='9' => (1134, 0),
        '+' | '−' | '=' => (1530, 0),
        // Unicode uses the earlier PLANCK CONSTANT codepoint for italic h.
        'ℎ' => ITALIC_LATIN[33],
        '𝐴'..='𝑔' | '𝑖'..='𝑧' => ITALIC_LATIN[(character as u32 - 0x1D434) as usize],
        _ => return None,
    };
    let scale = style.size_px / 2048.0;
    Some((f64::from(advance) * scale, f64::from(italic) * scale))
}

impl<P: Painter> StyledMathMeasure for StyledPainterMeasure<'_, P> {
    type Error = P::Error;
    fn measure_glyph(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
    ) -> Result<MathMetrics, Self::Error> {
        let mut font = self.font.borrow_mut();
        if !font
            .as_ref()
            .is_some_and(|(family, size)| family == style.family && *size == style.size_px)
        {
            self.painter.set_font(&styled_font(style));
            *font = Some((style.family.to_owned(), style.size_px));
        }
        drop(font);
        let measured = self.painter.measure_text_metrics(text)?;
        let ascent = if measured.actual_bounding_box_ascent.is_finite()
            && measured.actual_bounding_box_ascent > 0.0
        {
            measured.actual_bounding_box_ascent
        } else {
            style.size_px * 0.8
        };
        let descent = if measured.actual_bounding_box_descent.is_finite()
            && measured.actual_bounding_box_descent >= 0.0
        {
            measured.actual_bounding_box_descent
        } else {
            style.size_px * 0.2
        };
        Ok(MathMetrics {
            width: cambria_math_metrics(text, style).map_or(measured.width, |(advance, _)| advance),
            ascent,
            descent,
        })
    }
    fn fallback_italic_correction(&self, text: &str, style: MathTextStyle<'_>) -> f64 {
        cambria_math_metrics(text, style).map_or(0.0, |(_, italic)| italic)
    }

    fn script_scale(&self, style: MathTextStyle<'_>, level: u32) -> f64 {
        match level {
            0 => 1.0,
            // Cambria's published typical scale is supported by Word captures
            // at two base sizes. This remains a fallback, not MATH-table shaping.
            1 if style.family == "Cambria Math" => 0.7,
            1 => 0.8,
            _ => 0.6,
        }
    }
}
fn styled_font(style: MathTextStyle<'_>) -> String {
    // Prefer a math face for missing authored glyphs instead of mixing generic
    // serif operators with a separate Unicode alphabetic fallback.
    format!(
        "{:.2}px '{}', 'STIX Two Math', math, serif",
        style.size_px,
        style.family.replace('\'', "\\'")
    )
}
fn styled_color(color: &str) -> std::borrow::Cow<'_, str> {
    if color.starts_with('#') {
        std::borrow::Cow::Borrowed(color)
    } else {
        std::borrow::Cow::Owned(format!("#{color}"))
    }
}
/// The layout settings a document's `m:mathPr` asks for.
pub(super) fn settings_from(
    properties: &jade_docx_reader::settings::MathProperties,
) -> jade_docx_layout::math_layout::styled::MathSettings {
    use jade_docx_layout::math_layout::styled::{
        BreakBinary, BreakBinarySubtraction, MathJustification, MathSettings,
    };
    let mut settings = MathSettings {
        use_display_defaults: properties.display_default != Some(false),
        ..Default::default()
    };
    if settings.use_display_defaults {
        let px = |twips: Option<f32>| f64::from(twips.unwrap_or(0.0)) / 15.0;
        settings.display_margins_px = (px(properties.left_margin), px(properties.right_margin));
    }
    if let Some(value) = properties.integral_limits.as_deref() {
        settings.integral_limits_side = value != "undOvr";
    }
    if let Some(value) = properties.nary_limits.as_deref() {
        settings.nary_limits_over = value != "subSup";
    }
    if let Some(small) = properties.small_fractions {
        settings.small_fractions = small;
    }
    if let Some(indent) = properties.wrap_indent {
        settings.wrap_indent_px = f64::from(indent) / 20.0 * 96.0 / 72.0;
    }
    if let Some(right) = properties.wrap_right {
        settings.wrap_right = right;
    }
    settings.break_binary = match properties.break_binary.as_deref() {
        Some("after") => BreakBinary::After,
        Some("repeat") => BreakBinary::Repeat,
        _ => BreakBinary::Before,
    };
    settings.break_binary_subtraction = match properties.break_binary_subtraction.as_deref() {
        Some("+-") => BreakBinarySubtraction::PlusMinus,
        Some("-+") => BreakBinarySubtraction::MinusPlus,
        _ => BreakBinarySubtraction::MinusMinus,
    };
    if let Some(jc) = properties
        .default_justification
        .as_deref()
        .and_then(MathJustification::from_omml)
    {
        settings.default_justification = jc;
    }
    settings
}

/// Lays out a list of math elements (an `m:oMath`, an `m:oMathPara`, an EQ
/// field's equation) in the bound math face when one is registered for the
/// style's family, else through the painter's measurement.
pub(super) fn layout_math_elements<'a, P: Painter>(
    painter: &P,
    elements: &'a [jade_docx_reader::document::MathElement],
    style: MathTextStyle<'a>,
    options: &jade_docx_layout::math_layout::styled::MathLayoutOptions<'a>,
    #[cfg(feature = "opentype-math")] font: Option<
        &jade_docx_layout::math_layout::styled::font::MathFontData,
    >,
) -> Result<StyledMathLayout<'a>, wasm_bindgen::JsValue>
where
    P::Error: jade_canvas_painter::PainterError,
{
    use jade_canvas_painter::PainterError;
    painter.save();
    let measure = StyledPainterMeasure {
        painter,
        font: RefCell::new(None),
    };
    #[cfg(feature = "opentype-math")]
    let result = match font {
        Some(font) => font
            .layout_with_options(measure, elements, style, options)
            .map_err(|error| match error {
                jade_docx_layout::math_layout::styled::font::MathFontLayoutError::Fallback(e) => {
                    e.into_js_value()
                }
                other => wasm_bindgen::JsValue::from_str(&format!(
                    "MATH font preparation: {}",
                    other.kind()
                )),
            }),
        None => jade_docx_layout::math_layout::styled::layout_styled_elements(
            &measure, elements, style, options,
        )
        .map_err(PainterError::into_js_value),
    };
    #[cfg(not(feature = "opentype-math"))]
    let result = jade_docx_layout::math_layout::styled::layout_styled_elements(
        &measure, elements, style, options,
    )
    .map_err(PainterError::into_js_value);
    painter.restore();
    result
}

/// A math plan that owns what it paints: kept by the renderer for an inline
/// equation between laying a line out and drawing it, and for an equation
/// editor that needs to know where everything went.
#[derive(Debug, Clone)]
pub struct OwnedMathPlan {
    pub alignment: jade_docx_layout::math_layout::MathParagraphAlignment,
    pub metrics: MathMetrics,
    /// The paragraph box display math reserves (font leading included).
    pub line_metrics: MathMetrics,
    pub text: String,
    pub glyphs: Vec<OwnedMathGlyph>,
    /// x, y, width, thickness, colour.
    pub rules: Vec<(f64, f64, f64, f64, String)>,
    pub shapes: Vec<OwnedMathShape>,
    pub boxes: Vec<jade_docx_layout::math_layout::styled::MathBox>,
    pub lines: Vec<jade_docx_layout::math_layout::styled::MathLine>,
}

#[derive(Debug, Clone)]
pub struct OwnedMathGlyph {
    pub text_range: std::ops::Range<usize>,
    pub rendered_text: Option<&'static str>,
    pub x: f64,
    pub baseline: f64,
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
    pub family: String,
    pub size_px: f64,
    pub color: String,
    pub visible: bool,
    #[cfg(feature = "opentype-math")]
    pub font_glyph: Option<jade_docx_layout::math_layout::styled::font::PreparedMathGlyph>,
}

#[derive(Debug, Clone)]
pub struct OwnedMathShape {
    pub commands: Vec<jade_docx_layout::math_layout::styled::MathPathCommand>,
    pub stroke: Option<f64>,
    pub color: String,
}

impl OwnedMathPlan {
    pub fn from_plan(plan: &StyledMathLayout<'_>) -> Self {
        Self {
            alignment: plan.alignment,
            metrics: plan.metrics,
            line_metrics: plan.line_metrics,
            text: plan.text.clone(),
            glyphs: plan
                .glyphs
                .iter()
                .map(|g| OwnedMathGlyph {
                    text_range: g.text_range.clone(),
                    rendered_text: g.rendered_text,
                    x: g.x,
                    baseline: g.baseline,
                    width: g.width,
                    ascent: g.ascent,
                    descent: g.descent,
                    family: g.style.family.to_string(),
                    size_px: g.style.size_px,
                    color: g.style.color.to_string(),
                    visible: g.visible,
                    #[cfg(feature = "opentype-math")]
                    font_glyph: g.font_glyph.clone(),
                })
                .collect(),
            rules: plan
                .rules
                .iter()
                .map(|r| (r.x, r.y, r.width, r.thickness, r.color.to_string()))
                .collect(),
            shapes: plan
                .shapes
                .iter()
                .map(|s| OwnedMathShape {
                    commands: s.commands.clone(),
                    stroke: s.stroke,
                    color: s.color.to_string(),
                })
                .collect(),
            boxes: plan.boxes.clone(),
            lines: plan.lines.clone(),
        }
    }

    /// Paints the plan with its origin (first baseline, left edge) at
    /// (`x`, `baseline`).
    pub fn paint<P: Painter>(&self, painter: &P, x: f64, baseline: f64) -> Result<(), P::Error> {
        self.paint_with_automatic_color(painter, x, baseline, "#000000")
    }

    pub(super) fn paint_with_automatic_color<P: Painter>(
        &self,
        painter: &P,
        x: f64,
        baseline: f64,
        automatic_color: &str,
    ) -> Result<(), P::Error> {
        painter.save();
        let result = (|| {
            let mut last_font: Option<(&str, f64)> = None;
            let mut last_color: Option<&str> = None;
            for glyph in self.glyphs.iter().filter(|g| g.visible) {
                #[cfg(feature = "opentype-math")]
                if let Some(prepared) = &glyph.font_glyph {
                    if last_color != Some(glyph.color.as_str()) {
                        painter
                            .set_fill_style_str(&owned_math_color(&glyph.color, automatic_color));
                        last_color = Some(glyph.color.as_str());
                    }
                    paint_math_outline(painter, prepared, x + glyph.x, baseline + glyph.baseline);
                    continue;
                }
                let font = (glyph.family.as_str(), glyph.size_px);
                if last_font != Some(font) {
                    painter.set_font(&styled_font(MathTextStyle {
                        family: &glyph.family,
                        size_px: glyph.size_px,
                        color: &glyph.color,
                        variant: jade_docx_layout::math_layout::styled::MathVariant::Plain,
                    }));
                    last_font = Some(font);
                }
                if last_color != Some(glyph.color.as_str()) {
                    painter.set_fill_style_str(&owned_math_color(&glyph.color, automatic_color));
                    last_color = Some(glyph.color.as_str());
                }
                painter.fill_text(
                    glyph
                        .rendered_text
                        .unwrap_or_else(|| &self.text[glyph.text_range.clone()]),
                    x + glyph.x,
                    baseline + glyph.baseline,
                )?;
            }
            for (rx, ry, width, thickness, color) in &self.rules {
                painter.set_stroke_style_str(&owned_math_color(color, automatic_color));
                painter.set_line_width(*thickness);
                painter.begin_path();
                painter.move_to(x + rx, baseline + ry);
                painter.line_to(x + rx + width, baseline + ry);
                painter.stroke();
            }
            for shape in &self.shapes {
                paint_owned_shape(painter, shape, x, baseline, automatic_color);
            }
            Ok(())
        })();
        painter.restore();
        result
    }
}

fn owned_math_color<'a>(color: &'a str, automatic_color: &'a str) -> std::borrow::Cow<'a, str> {
    if color == "auto" {
        std::borrow::Cow::Borrowed(automatic_color)
    } else {
        styled_color(color)
    }
}

fn paint_owned_shape<P: Painter>(
    painter: &P,
    shape: &OwnedMathShape,
    x: f64,
    baseline: f64,
    automatic_color: &str,
) {
    use jade_docx_layout::math_layout::styled::MathPathCommand::*;
    painter.begin_path();
    for command in &shape.commands {
        match *command {
            Move(a, b) => painter.move_to(x + a, baseline + b),
            Line(a, b) => painter.line_to(x + a, baseline + b),
            Quad(a, b, c, d) => {
                painter.quadratic_curve_to(x + a, baseline + b, x + c, baseline + d)
            }
            Cubic(a, b, c, d, e, f) => painter.bezier_curve_to(
                x + a,
                baseline + b,
                x + c,
                baseline + d,
                x + e,
                baseline + f,
            ),
            Close => painter.close_path(),
        }
    }
    match shape.stroke {
        Some(width) => {
            painter.set_stroke_style_str(&owned_math_color(&shape.color, automatic_color));
            painter.set_line_width(width);
            painter.stroke();
        }
        None => {
            painter.set_fill_style_str(&owned_math_color(&shape.color, automatic_color));
            painter.fill();
        }
    }
}

/// Where an equation was drawn at the last render: what an equation editor
/// reads to put its caret inside the structure.
#[derive(Debug, Clone)]
pub struct MathZone {
    /// The paragraph, in rendered coordinates (a display equation is a
    /// paragraph of its own).
    pub paragraph: usize,
    /// The equation's byte offset in the paragraph's rendered text (0 for a
    /// display equation).
    pub offset: usize,
    /// Its run in the model paragraph (inline equations).
    pub run_index: Option<usize>,
    pub display: bool,
    /// 1-based page.
    pub page: usize,
    /// The plan's origin on the page: its left edge and first baseline.
    pub x: f64,
    pub baseline: f64,
    /// The same baseline in document coordinates, pages stacked.
    pub absolute_baseline: f64,
    pub plan: std::rc::Rc<OwnedMathPlan>,
}

#[cfg(feature = "opentype-math")]
pub fn paint_boxed_math<P: Painter>(
    painter: &P,
    plan: &StyledMathLayout<'_>,
    x: f64,
    baseline: f64,
) -> Result<(), P::Error> {
    painter.save();
    let result = (|| {
        let mut last_font = None;
        let mut last_color = None;
        for glyph in plan.glyphs.iter().filter(|g| g.visible) {
            #[cfg(feature = "opentype-math")]
            if let Some(prepared) = &glyph.font_glyph {
                if last_color != Some(glyph.style.color) {
                    painter.set_fill_style_str(&styled_color(glyph.style.color));
                    last_color = Some(glyph.style.color);
                }
                paint_math_outline(painter, prepared, x + glyph.x, baseline + glyph.baseline);
                continue;
            }
            let font = (glyph.style.family, glyph.style.size_px);
            if last_font != Some(font) {
                painter.set_font(&styled_font(glyph.style));
                last_font = Some(font);
            }
            if last_color != Some(glyph.style.color) {
                painter.set_fill_style_str(&styled_color(glyph.style.color));
                last_color = Some(glyph.style.color);
            }
            painter.fill_text(
                glyph.display_text(&plan.text),
                x + glyph.x,
                baseline + glyph.baseline,
            )?;
        }
        for rule in &plan.rules {
            painter.set_stroke_style_str(&styled_color(rule.color));
            painter.set_line_width(rule.thickness);
            painter.begin_path();
            painter.move_to(x + rule.x, baseline + rule.y);
            painter.line_to(x + rule.x + rule.width, baseline + rule.y);
            painter.stroke();
        }
        for shape in &plan.shapes {
            paint_math_shape(painter, shape, x, baseline);
        }
        Ok(())
    })();
    painter.restore();
    result
}

/// Draws a stretched delimiter, a radical sign, a wide accent or a border.
#[cfg(feature = "opentype-math")]
fn paint_math_shape<P: Painter>(
    painter: &P,
    shape: &jade_docx_layout::math_layout::styled::MathShape<'_>,
    x: f64,
    baseline: f64,
) {
    use jade_docx_layout::math_layout::styled::MathPathCommand::*;
    painter.begin_path();
    for command in &shape.commands {
        match *command {
            Move(a, b) => painter.move_to(x + a, baseline + b),
            Line(a, b) => painter.line_to(x + a, baseline + b),
            Quad(a, b, c, d) => {
                painter.quadratic_curve_to(x + a, baseline + b, x + c, baseline + d)
            }
            Cubic(a, b, c, d, e, f) => painter.bezier_curve_to(
                x + a,
                baseline + b,
                x + c,
                baseline + d,
                x + e,
                baseline + f,
            ),
            Close => painter.close_path(),
        }
    }
    match shape.stroke {
        Some(width) => {
            painter.set_stroke_style_str(&styled_color(shape.color));
            painter.set_line_width(width);
            painter.stroke();
        }
        None => {
            painter.set_fill_style_str(&styled_color(shape.color));
            painter.fill();
        }
    }
}

#[cfg(feature = "opentype-math")]
fn paint_math_outline<P: Painter>(
    painter: &P,
    glyph: &jade_docx_layout::math_layout::styled::font::PreparedMathGlyph,
    x: f64,
    baseline: f64,
) {
    use jade_docx_layout::math_layout::font::MathOutlineCommand::*;
    let px = |v: f32| x + f64::from(v) * glyph.scale;
    let py = |v: f32| baseline - f64::from(v) * glyph.scale;
    painter.begin_path();
    for command in &glyph.outline.commands {
        match *command {
            Move(a, b) => painter.move_to(px(a), py(b)),
            Line(a, b) => painter.line_to(px(a), py(b)),
            Quad(a, b, c, d) => painter.quadratic_curve_to(px(a), py(b), px(c), py(d)),
            Cubic(a, b, c, d, e, f) => {
                painter.bezier_curve_to(px(a), py(b), px(c), py(d), px(e), py(f))
            }
            Close => painter.close_path(),
        }
    }
    painter.fill();
}

/// Shared font and paragraph rules for display equations in every story.
pub(super) struct DisplayMathStyle<'a> {
    pub(super) styles: Option<&'a StylesPart>,
    pub(super) theme: Option<&'a ThemePart>,
    pub(super) settings: jade_docx_layout::math_layout::styled::MathSettings,
    pub(super) dark_mode_text: bool,
}

impl DisplayMathStyle<'_> {
    pub(super) fn line_properties(
        styles: Option<&StylesPart>,
        math: &MathParagraph,
    ) -> ParagraphProperties {
        let mut properties = math.properties.as_deref().cloned().unwrap_or_default();
        let Some(styles) = styles else {
            return properties;
        };
        let style = properties
            .style
            .as_deref()
            .or_else(|| styles.default_paragraph_style_id())
            .and_then(|id| styles.resolve_paragraph_properties(id));
        let defaults = styles.doc_default_paragraph_properties();
        for spacing in [style.as_ref(), defaults]
            .into_iter()
            .flatten()
            .filter_map(|properties| properties.spacing.as_ref())
        {
            if let Some(direct) = properties.spacing.as_mut() {
                crate::word_document::WordDocument::fill_missing_spacing(direct, spacing);
            } else {
                properties.spacing = Some(spacing.clone());
            }
        }
        properties
    }

    /// The text face inherited by `m:nor` (normal text) runs from the
    /// containing paragraph style and document defaults.
    pub(super) fn math_text_family(&self, paragraph_style: Option<&str>) -> String {
        let mut properties = self
            .styles
            .and_then(|styles| styles.resolve_effective_run_properties(paragraph_style, None));
        if let (Some(properties), Some(theme)) = (properties.as_mut(), self.theme) {
            if theme.has_resolvable_font_reference(properties) {
                theme.apply_to_run_properties(properties);
            }
        }
        properties
            .as_ref()
            .and_then(|properties| {
                properties
                    .fonts
                    .as_ref()
                    .and_then(|fonts| fonts.ascii.as_deref().or(fonts.h_ansi.as_deref()))
                    .or(properties.font_family.as_deref())
            })
            .filter(|family| !family.trim().is_empty())
            .unwrap_or("Cambria")
            .to_owned()
    }

    pub(super) fn body_math_style<'a>(
        &self,
        math: &'a MathParagraph,
        family: &'a str,
    ) -> jade_docx_layout::math_layout::styled::MathTextStyle<'a> {
        let mut style = self.boxed_math_style(family);
        if let Some(properties) = math.properties.as_deref() {
            let mark = properties.mark_run_properties.as_ref();
            if let Some(size) = mark
                .and_then(|p| p.font_size)
                .or_else(|| {
                    properties
                        .style
                        .as_deref()
                        .and_then(|id| self.styles.and_then(|s| s.resolve_font_size(id)))
                })
                .filter(|size| size.is_finite() && *size > 0.0)
            {
                style.size_px = points_to_px(size);
            }
            if let Some(family) = mark
                .and_then(|p| p.fonts.as_ref())
                .and_then(|f| f.ascii.as_deref().or(f.h_ansi.as_deref()))
            {
                style.family = family;
            }
        }
        style
    }

    pub(super) fn display_math_settings(
        &self,
        math: &MathParagraph,
    ) -> jade_docx_layout::math_layout::styled::MathSettings {
        use jade_docx_layout::math_layout::styled::MathJustification;
        let mut settings = self.settings;
        if !settings.use_display_defaults {
            // dispDef=false inherits paragraph settings for alignment and wrapping.
            let properties = math.properties.as_deref();
            let styles = self.styles;
            let inherited = styles.and_then(|styles| {
                styles.resolve_paragraph_properties(
                    properties.and_then(|p| p.style.as_deref()).unwrap_or(""),
                )
            });
            let defaults = styles.and_then(StylesPart::doc_default_paragraph_properties);
            let bidi = properties
                .and_then(|p| p.bidi)
                .or_else(|| inherited.as_ref().and_then(|p| p.bidi))
                .or_else(|| defaults.and_then(|p| p.bidi))
                .unwrap_or(false);
            settings.word_mac_rtl_paragraph = bidi
                && jade_docx_layout::line_metrics::word_layout_platform()
                    == jade_docx_layout::line_metrics::WordLayoutPlatform::Mac;
            let alignment = properties
                .and_then(|p| p.alignment.as_deref())
                .or_else(|| inherited.as_ref().and_then(|p| p.alignment.as_deref()))
                .or_else(|| defaults.and_then(|p| p.alignment.as_deref()));
            settings.default_justification = match alignment {
                Some("center") => MathJustification::Center,
                Some("right") => MathJustification::Right,
                Some("left") => MathJustification::Left,
                Some("end") if !bidi => MathJustification::Right,
                Some("end") => MathJustification::Left,
                _ if bidi => MathJustification::Right,
                _ => MathJustification::Left,
            };
            settings.wrap_indent_px = 0.0;
            settings.wrap_right = false;
        }
        settings
    }

    pub(super) fn boxed_math_style<'a>(
        &self,
        family: &'a str,
    ) -> jade_docx_layout::math_layout::styled::MathTextStyle<'a> {
        use jade_docx_layout::math_layout::styled::{MathTextStyle, MathVariant};
        let size = self
            .styles
            .and_then(|s| s.doc_default_font_size())
            .unwrap_or(10.5);
        MathTextStyle {
            family,
            size_px: points_to_px(size),
            color: if self.dark_mode_text {
                super::DARK_MODE_TEXT_COLOR
            } else {
                "#000000"
            },
            variant: MathVariant::Italic,
        }
    }
}

#[cfg(feature = "opentype-math")]
pub(super) type MathFonts = std::collections::HashMap<
    String,
    Rc<jade_docx_layout::math_layout::styled::font::MathFontData>,
>;

#[cfg(test)]
mod tests {
    use super::*;
    use jade_docx_layout::math_layout::MathParagraphAlignment;
    use std::collections::BTreeMap;

    #[test]
    fn styled_math_font_preserves_authored_family_before_math_fallbacks() {
        let style = MathTextStyle {
            family: "Authored Font",
            size_px: 14.0,
            color: "000000",
            variant: jade_docx_layout::math_layout::styled::MathVariant::Italic,
        };
        assert_eq!(
            styled_font(style),
            "14.00px 'Authored Font', 'STIX Two Math', math, serif"
        );
        assert_eq!(
            styled_font(MathTextStyle {
                family: "Author's Math",
                ..style
            }),
            "14.00px 'Author\\'s Math', 'STIX Two Math', math, serif"
        );
    }

    #[test]
    fn repeated_subtraction_settings_reach_available_math_painters() {
        use jade_docx_layout::math_layout::styled::MathLayoutOptions;
        use jade_docx_reader::{parser::math::parse_math_paragraph, settings::DocumentSettings};
        use jade_painter::recording::{Op, RecordingPainter};

        let math = parse_math_paragraph(
            r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:r><m:t>11-22-33-44-55-66</m:t></m:r></m:oMath>"#.into(),
        )
        .unwrap();
        let style = MathTextStyle {
            family: "Cambria Math",
            size_px: 14.0,
            color: "000000",
            variant: jade_docx_layout::math_layout::styled::MathVariant::Italic,
        };
        for (mode, trailing, leading) in [("+-", "+", "−"), ("-+", "−", "+")] {
            let settings = DocumentSettings::from_xml(&format!(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:mathPr><m:brkBin m:val="repeat"/><m:brkBinSub m:val="{mode}"/><m:wrapIndent m:val="0"/></m:mathPr></w:settings>"#
            ))
            .unwrap();
            let painter = RecordingPainter::with_width_per_char(8.0);
            let plan = layout_math_elements(
                &painter,
                &math.elements,
                style,
                &MathLayoutOptions {
                    max_width: Some(64.0),
                    settings: settings_from(&settings.math),
                    ..MathLayoutOptions::default()
                },
                #[cfg(feature = "opentype-math")]
                None,
            )
            .unwrap();
            assert!(plan.lines.len() > 1);
            assert_eq!(plan.text, "11−22−33−44−55−66");
            let owned = OwnedMathPlan::from_plan(&plan);
            for draw_owned in [
                #[cfg(feature = "opentype-math")]
                false,
                true,
            ] {
                painter.clear();
                if draw_owned {
                    owned.paint(&painter, 0.0, 0.0).unwrap();
                } else {
                    #[cfg(feature = "opentype-math")]
                    paint_boxed_math(&painter, &plan, 0.0, 0.0).unwrap();
                }
                let ops = painter.ops();
                let edge = |baseline: f64, last: bool| {
                    let mut text: Vec<_> = ops
                        .iter()
                        .filter_map(|op| match op {
                            Op::FillText(value, x, y) if *y == baseline => {
                                Some((*x, value.as_str()))
                            }
                            _ => None,
                        })
                        .collect();
                    text.sort_by(|a, b| a.0.total_cmp(&b.0));
                    if last { text.last() } else { text.first() }.unwrap().1
                };
                for pair in plan.lines.windows(2) {
                    assert_eq!(edge(pair[0].baseline, true), trailing, "{mode}");
                    assert_eq!(edge(pair[1].baseline, false), leading, "{mode}");
                }
            }
        }
    }

    fn paragraph_with_alignment(value: Option<&str>) -> MathParagraph {
        let properties = value.map(|value| MathElement {
            kind: MathElementKind::Unknown,
            tag: Some("oMathParaPr".to_string()),
            children: vec![MathElement {
                kind: MathElementKind::Unknown,
                tag: Some("jc".to_string()),
                attributes: BTreeMap::from([("val".to_string(), value.to_string())]),
                ..MathElement::default()
            }],
            ..MathElement::default()
        });
        MathParagraph {
            elements: vec![MathElement {
                kind: MathElementKind::MathParagraph,
                tag: Some("oMathPara".to_string()),
                children: properties.into_iter().collect(),
                ..MathElement::default()
            }],
            ..MathParagraph::default()
        }
    }

    #[test]
    fn block_alignment_honors_omml_justification() {
        assert_eq!(
            MathParagraphAlignment::from_math_paragraph(&paragraph_with_alignment(Some("center"))),
            MathParagraphAlignment::Center
        );
        assert_eq!(
            MathParagraphAlignment::from_math_paragraph(&paragraph_with_alignment(Some("right"))),
            MathParagraphAlignment::Right
        );
        assert_eq!(
            MathParagraphAlignment::from_math_paragraph(&paragraph_with_alignment(None)),
            MathParagraphAlignment::Left
        );
    }

    #[test]
    fn block_alignment_coordinates_stay_inside_the_column() {
        assert_eq!(MathParagraphAlignment::Left.x(10.0, 100.0, 20.0), 10.0);
        assert_eq!(MathParagraphAlignment::Center.x(10.0, 100.0, 20.0), 50.0);
        assert_eq!(MathParagraphAlignment::Right.x(10.0, 100.0, 20.0), 90.0);
        assert_eq!(MathParagraphAlignment::Center.x(10.0, 20.0, 40.0), 10.0);
    }
}
