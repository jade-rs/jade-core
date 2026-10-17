use crate::document::MathParagraph;
#[cfg(test)]
use crate::document::{MathElement, MathElementKind};
use jade_docx_layout::math_layout::MathMetrics;
use jade_docx_layout::math_layout::{layout_math, MathCommand, MathLayout, MathMeasureContext};
pub(crate) use jade_docx_layout::math_layout::{math_text, MathParagraphAlignment};
use jade_painter::Painter;

const DEFAULT_FONT_FAMILY: &str = "'Times New Roman', serif";
const DEFAULT_FONT_PX: f64 = 16.0;

struct MathPainterMeasure<'a, P>(&'a P);
impl<P: Painter> MathMeasureContext for MathPainterMeasure<'_, P> {
    type Error = P::Error;
    fn measure_text(&self, text: &str) -> Result<f64, Self::Error> {
        self.0.measure_text_width(text)
    }
}

pub struct MathRenderer<'a, P: Painter> {
    context: &'a P,
    font_px: f64,
    text_color: &'a str,
}

impl<'a, P: Painter> MathRenderer<'a, P> {
    pub fn new(context: &'a P) -> Self {
        Self {
            context,
            font_px: DEFAULT_FONT_PX,
            text_color: "#1a237e",
        }
    }
    pub fn with_font_size(mut self, font_px: f64) -> Self {
        if font_px > 0.0 {
            self.font_px = font_px;
        }
        self
    }
    pub fn with_text_color(mut self, color: &'a str) -> Self {
        self.text_color = color;
        self
    }
    pub fn layout<'m>(&self, math: &'m MathParagraph) -> Result<MathLayout<'m>, P::Error> {
        self.apply_font();
        layout_math(&MathPainterMeasure(self.context), math, self.font_px)
    }
    pub fn render_layout(
        &self,
        layout: &MathLayout<'_>,
        x: f64,
        baseline_y: f64,
    ) -> Result<(), P::Error> {
        self.apply_font();
        self.context.set_fill_style_str(self.text_color);
        for command in &layout.commands {
            match command {
                MathCommand::Text {
                    text,
                    x: dx,
                    baseline,
                    ..
                } => {
                    self.context
                        .fill_text(text, x + dx, baseline_y + baseline)?;
                }
                MathCommand::Rule {
                    start_x,
                    end_x,
                    y,
                    thickness,
                } => {
                    self.context.set_line_width(*thickness);
                    self.context.begin_path();
                    self.context.move_to(x + start_x, baseline_y + y);
                    self.context.line_to(x + end_x, baseline_y + y);
                    self.context.stroke();
                }
            }
        }
        Ok(())
    }
    #[cfg(test)]
    fn measure(&self, math: &MathParagraph) -> Result<MathMetrics, P::Error> {
        Ok(self.layout(math)?.metrics)
    }
    #[cfg(test)]
    fn render(
        &self,
        math: &MathParagraph,
        x: f64,
        baseline_y: f64,
    ) -> Result<MathMetrics, P::Error> {
        let layout = self.layout(math)?;
        self.render_layout(&layout, x, baseline_y)?;
        Ok(layout.metrics)
    }
    fn apply_font(&self) {
        let font = format!("italic {:.2}px {}", self.font_px, DEFAULT_FONT_FAMILY);
        self.context.set_font(&font);
    }
}

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
        for glyph in &plan.glyphs {
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
        Ok(())
    })();
    painter.restore();
    result
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
    use jade_painter::recording::RecordingPainter;
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
    fn mathematical_measurement_matches_painted_geometry() {
        for body in [
            "<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>",
            "<m:sSub><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>2</m:t></m:r></m:sub></m:sSub>",
            "<m:rad><m:e><m:r><m:t>x</m:t></m:r></m:e></m:rad>",
        ] {
            let math = jade_docx_reader::parser::math::parse_math_paragraph(format!(
                "<m:oMath xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">{body}</m:oMath>"
            )).unwrap();
            let painter = RecordingPainter::with_width_per_char(10.0);
            let renderer = MathRenderer::new(&painter);
            let measured = renderer.measure(&math).unwrap();
            let painted = renderer.render(&math, 40.0, 80.0).unwrap();
            assert_eq!(
                (measured.width, measured.ascent, measured.descent),
                (painted.width, painted.ascent, painted.descent),
                "measured and painted math disagree: {body}"
            );
        }
    }

    #[test]
    fn fraction_metrics_contain_painted_numerator_and_denominator() {
        let text = |value: &str| MathElement {
            kind: MathElementKind::Text,
            text: Some(value.to_string()),
            ..MathElement::default()
        };
        let math = MathParagraph {
            elements: vec![MathElement {
                kind: MathElementKind::Fraction,
                children: vec![
                    MathElement {
                        kind: MathElementKind::Numerator,
                        children: vec![text("1")],
                        ..MathElement::default()
                    },
                    MathElement {
                        kind: MathElementKind::Denominator,
                        children: vec![text("2")],
                        ..MathElement::default()
                    },
                ],
                ..MathElement::default()
            }],
            ..MathParagraph::default()
        };
        let painter = RecordingPainter::with_width_per_char(8.0);
        let renderer = MathRenderer::new(&painter);
        let layout = renderer.layout(&math).expect("measure fraction");
        let measurements = painter
            .ops()
            .iter()
            .filter(|op| matches!(op, jade_painter::recording::Op::MeasureText(_)))
            .count();
        assert_eq!(measurements, 2);
        renderer
            .render_layout(&layout, 40.0, 80.0)
            .expect("paint fraction");
        let metrics = layout.metrics;
        assert!((metrics.width - 16.0).abs() < 0.01);
        let mut baselines = Vec::new();
        for op in painter.ops() {
            if let jade_painter::recording::Op::FillText(_, x, baseline) = op {
                assert!(x >= 40.0 && x + 8.0 <= 40.0 + metrics.width);
                assert!(baseline - 16.0 * 0.8 >= 80.0 - metrics.ascent - 1e-9);
                assert!(baseline + 16.0 * 0.2 <= 80.0 + metrics.descent + 1e-9);
                baselines.push(baseline);
            }
        }
        assert_eq!(baselines.len(), 2);
        assert!(baselines[0] < 80.0 && baselines[1] > 80.0);
        assert_eq!(
            painter
                .ops()
                .iter()
                .filter(|op| matches!(op, jade_painter::recording::Op::MeasureText(_)))
                .count(),
            measurements,
            "painting must reuse the measured plan"
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
