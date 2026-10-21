use crate::document::MathParagraph;
#[cfg(test)]
use crate::document::{MathElement, MathElementKind};
pub(crate) use jade_docx_layout::math_layout::math_text;
use jade_docx_layout::math_layout::MathMetrics;
use jade_painter::Painter;

use jade_docx_layout::math_layout::styled::{
    layout_styled_math, MathTextStyle, StyledMathLayout, StyledMathMeasure,
};
use std::cell::RefCell;

struct StyledPainterMeasure<'a, P> {
    painter: &'a P,
    font: RefCell<Option<(String, f64)>>,
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
            width: measured.width,
            ascent,
            descent,
        })
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
pub(super) fn layout_boxed_math<'a, P: Painter>(
    painter: &P,
    math: &'a MathParagraph,
    style: MathTextStyle<'a>,
) -> Result<StyledMathLayout<'a>, P::Error> {
    painter.save();
    let result = layout_styled_math(
        &StyledPainterMeasure {
            painter,
            font: RefCell::new(None),
        },
        math,
        style,
    );
    painter.restore();
    result
}

#[cfg(feature = "opentype-math")]
pub(super) fn layout_boxed_math_with_font<'a, P: Painter>(
    painter: &P,
    math: &'a MathParagraph,
    style: MathTextStyle<'a>,
    font: &jade_docx_layout::math_layout::styled::font::MathFontData,
) -> Result<
    StyledMathLayout<'a>,
    jade_docx_layout::math_layout::styled::font::MathFontLayoutError<P::Error>,
> {
    painter.save();
    let result = font.layout(
        StyledPainterMeasure {
            painter,
            font: RefCell::new(None),
        },
        math,
        style,
        true,
    );
    painter.restore();
    result
}
/// The layout settings a document's `m:mathPr` asks for.
pub(super) fn settings_from(
    properties: &jade_docx_reader::settings::MathProperties,
) -> jade_docx_layout::math_layout::styled::MathSettings {
    use jade_docx_layout::math_layout::styled::{BreakBinary, MathJustification, MathSettings};
    let mut settings = MathSettings::default();
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
            metrics: plan.metrics,
            line_metrics: plan.line_metrics,
            text: plan.text.clone(),
            glyphs: plan
                .glyphs
                .iter()
                .map(|g| OwnedMathGlyph {
                    text_range: g.text_range.clone(),
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
        painter.save();
        let result = (|| {
            let mut last_font: Option<(&str, f64)> = None;
            let mut last_color: Option<&str> = None;
            for glyph in self.glyphs.iter().filter(|g| g.visible) {
                #[cfg(feature = "opentype-math")]
                if let Some(prepared) = &glyph.font_glyph {
                    if last_color != Some(glyph.color.as_str()) {
                        painter.set_fill_style_str(&styled_color(&glyph.color));
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
                    painter.set_fill_style_str(&styled_color(&glyph.color));
                    last_color = Some(glyph.color.as_str());
                }
                painter.fill_text(
                    &self.text[glyph.text_range.clone()],
                    x + glyph.x,
                    baseline + glyph.baseline,
                )?;
            }
            for (rx, ry, width, thickness, color) in &self.rules {
                painter.set_stroke_style_str(&styled_color(color));
                painter.set_line_width(*thickness);
                painter.begin_path();
                painter.move_to(x + rx, baseline + ry);
                painter.line_to(x + rx + width, baseline + ry);
                painter.stroke();
            }
            for shape in &self.shapes {
                paint_owned_shape(painter, shape, x, baseline);
            }
            Ok(())
        })();
        painter.restore();
        result
    }
}

fn paint_owned_shape<P: Painter>(painter: &P, shape: &OwnedMathShape, x: f64, baseline: f64) {
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
            painter.set_stroke_style_str(&styled_color(&shape.color));
            painter.set_line_width(width);
            painter.stroke();
        }
        None => {
            painter.set_fill_style_str(&styled_color(&shape.color));
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
                &plan.text[glyph.text_range.clone()],
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
