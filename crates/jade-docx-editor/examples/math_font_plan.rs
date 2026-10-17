//! Read-only probe: explicitly bind a static font to OMML's authored family.
//! This does not register browser fonts or establish Word-compatible substitution.
use jade_docx_editor::canvas_renderer::paint_styled_math;
use jade_docx_layout::math_layout::font::OpenTypeMathFont;
use jade_docx_layout::math_layout::styled::font::OpenTypeMathMeasure;
use jade_docx_layout::math_layout::styled::{
    layout_styled_math, MathTextStyle, MathVariant, StyledMathMeasure,
};
use jade_docx_layout::math_layout::MathMetrics;
use jade_painter::recording::{Op, RecordingPainter};
use serde_json::json;

struct FontFallback<'a> {
    font: &'a OpenTypeMathFont<'a>,
    family: &'a str,
}
impl StyledMathMeasure for FontFallback<'_> {
    type Error = String;
    fn measure_glyph(&self, text: &str, style: MathTextStyle<'_>) -> Result<MathMetrics, String> {
        if style.family != self.family || !text.chars().all(char::is_whitespace) {
            return Err(format!("uncovered font/grapheme: {} {text}", style.family));
        }
        let scale = self.font.scale(style.size_px).ok_or("invalid font size")?;
        let mut width = 0.0;
        for character in text.chars() {
            let glyph = self
                .font
                .glyph_id(character)
                .ok_or("uncovered whitespace")?;
            width += f64::from(
                self.font
                    .glyph_advance_units(glyph)
                    .ok_or("missing advance")?,
            ) * scale;
        }
        Ok(MathMetrics {
            width,
            ascent: 0.0,
            descent: 0.0,
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: math_font_plan FONT OMML SIZE_PX AUTHORED_FAMILY")?;
    let omml_path = args.next().ok_or("missing OMML path")?;
    let size_px: f64 = args.next().ok_or("missing size in px")?.parse()?;
    let family = args.next().ok_or("missing authored-family binding")?;
    let bytes = std::fs::read(&path)?;
    let font = OpenTypeMathFont::parse(&bytes, 0).map_err(|e| format!("font: {e:?}"))?;
    let math =
        jade_docx_reader::parser::math::parse_math_paragraph(std::fs::read_to_string(&omml_path)?)?;
    let context = OpenTypeMathMeasure::new(
        &font,
        &family,
        FontFallback {
            font: &font,
            family: &family,
        },
        512,
        8192,
    )
    .with_display_style(true);
    let plan = layout_styled_math(
        &context,
        &math,
        MathTextStyle {
            family: &family,
            size_px,
            color: "000000",
            variant: MathVariant::Italic,
        },
    )
    .map_err(|e| format!("layout: {e:?}"))?;
    let painter = RecordingPainter::new();
    paint_styled_math(&painter, &plan, 10.0, 10.0 + plan.metrics.ascent)?;
    let glyphs: Vec<_> = plan.glyphs.iter().map(|g| {
        let prepared = g.font_glyph.as_ref();
        json!({"text": &plan.text[g.text_range.clone()], "x": g.x,
            "baseline": g.baseline, "width": g.width, "size_px": g.style.size_px,
            "glyph_id": prepared.map(|p| p.geometry.glyph_id),
            "ink_px": prepared.map(|p| [p.geometry.ink_px.x_min, p.geometry.ink_px.y_min, p.geometry.ink_px.x_max, p.geometry.ink_px.y_max]),
            "outline_commands": prepared.map(|p| p.outline.commands.len())})
    }).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "font_path": path, "omml_path": omml_path, "explicit_authored_family_binding": family,
            "word_font_equivalence_claimed": false, "body_pagination_integrated": false,
            "font_size_px": size_px, "text": plan.text,
            "metrics": {"width": plan.metrics.width, "ascent": plan.metrics.ascent, "descent": plan.metrics.descent},
            "line_metrics": {"width":plan.line_metrics.width,"ascent":plan.line_metrics.ascent,"descent":plan.line_metrics.descent},
            "cached_glyphs": context.cached_glyphs(), "glyphs": glyphs,
            "paint_measurements": painter.count(|o| matches!(o, Op::MeasureText(_))),
            "outline_fills": painter.count(|o| matches!(o, Op::Fill)),
            "text_fills": painter.count(|o| matches!(o, Op::FillText(..))),
            "draw_ops": painter.ops().iter().map(|o| format!("{o:?}")).collect::<Vec<_>>()
        }))?
    );
    Ok(())
}
