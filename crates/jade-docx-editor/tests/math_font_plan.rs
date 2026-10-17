#![cfg(feature = "opentype-math")]

use jade_docx_editor::canvas_renderer::paint_styled_math;
use jade_docx_layout::math_layout::font::{MathGlyphOutline, MathOutlineCommand, OpenTypeMathFont};
use jade_docx_layout::math_layout::styled::font::{
    MathFontData, MathFontDataError, MathFontLayoutError, OpenTypeMathMeasure,
};
use jade_docx_layout::math_layout::styled::{
    layout_styled_math, MathTextStyle, MathVariant, StyledMathMeasure,
};
use jade_docx_layout::math_layout::MathMetrics;
use jade_docx_reader::document::MathParagraph;
use jade_painter::recording::{Op, RecordingPainter};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

const FONT: &[u8] = include_bytes!("../../jade-docx-layout/tests/fixtures/math-font.ttf");
const NARY: &str = r#"<m:nary><m:naryPr><m:chr m:val="∫"/><m:limLoc m:val="subSup"/></m:naryPr><m:sup><m:r><m:t>x</m:t></m:r></m:sup><m:sub><m:r><m:t>x</m:t></m:r></m:sub><m:e><m:r><m:t>x</m:t></m:r></m:e></m:nary>"#;

#[derive(Default)]
struct Fallback(Rc<Cell<usize>>);
impl StyledMathMeasure for Fallback {
    type Error = &'static str;
    fn measure_glyph(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
    ) -> Result<MathMetrics, Self::Error> {
        self.0.set(self.0.get() + 1);
        if text == "!" {
            return Err("missing");
        }
        Ok(MathMetrics {
            width: style.size_px / 2.0,
            ascent: style.size_px * 0.8,
            descent: style.size_px * 0.2,
        })
    }
}
fn math(body: &str) -> MathParagraph {
    jade_docx_reader::parser::math::parse_math_paragraph(format!(r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{body}</m:oMath>"#)).unwrap()
}
fn style() -> MathTextStyle<'static> {
    MathTextStyle {
        family: "Host Math",
        size_px: 10.0,
        color: "112233",
        variant: MathVariant::Plain,
    }
}

#[test]
fn display_plan_uses_the_unencoded_glyph_advance_and_actual_script_scale() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let fallback = Fallback::default();
    let calls = Rc::clone(&fallback.0);
    let context =
        OpenTypeMathMeasure::new(&font, "Host Math", fallback, 8, 64).with_display_style(true);
    let source = math(NARY);
    let before = serde_json::to_value(&source).unwrap();
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert_eq!(plan.text, "∫xxx");
    assert_eq!(
        plan.glyphs[0]
            .font_glyph
            .as_ref()
            .unwrap()
            .geometry
            .glyph_id,
        3
    );
    assert_eq!(plan.glyphs[0].width, 5.5); // extension is 22px, not advance
    assert_eq!(plan.glyphs[1].style.size_px, 6.5);
    assert_eq!(plan.glyphs[1].width, 3.25);
    assert!(plan.metrics.ascent >= 18.0);
    assert!(plan.metrics.descent >= 4.0);
    assert_eq!(calls.get(), 0);
    assert_eq!(context.cached_glyphs(), 2);
    assert_eq!(serde_json::to_value(&source).unwrap(), before);
}

#[test]
fn inline_context_and_ordinary_integral_keep_the_base_glyph() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64);
    let source = math(NARY);
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert_eq!(
        plan.glyphs[0]
            .font_glyph
            .as_ref()
            .unwrap()
            .geometry
            .glyph_id,
        2
    );
    assert_eq!(plan.glyphs[0].width, 3.0);
    let context = context.with_display_style(true);
    let source = math("<m:r><m:t>∫</m:t></m:r>");
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert_eq!(
        plan.glyphs[0]
            .font_glyph
            .as_ref()
            .unwrap()
            .geometry
            .glyph_id,
        2
    );
}

#[test]
fn small_display_reserves_font_ink_beyond_a_single_18pt_paragraph_grid() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
        .with_display_style(true);
    let source = math(NARY);
    let small = MathTextStyle {
        size_px: 8.0 * 96.0 / 72.0,
        ..style()
    };
    let fallback = layout_styled_math(&Fallback::default(), &source, small).unwrap();
    let prepared = layout_styled_math(&context, &source, small).unwrap();
    let grid_px = 18.0 * 96.0 / 72.0;
    assert!(fallback.metrics.ascent + fallback.metrics.descent < grid_px);
    assert!(prepared.metrics.ascent + prepared.metrics.descent > grid_px);
    let plain_source = math("<m:r><m:t>x</m:t></m:r>");
    let plain = layout_styled_math(&context, &plain_source, small).unwrap();
    assert!(plain.metrics.ascent + plain.metrics.descent < grid_px);
}

#[test]
fn repeated_layout_at_another_size_reuses_identical_outline_storage() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
        .with_display_style(true);
    let source = math(NARY);
    let a = layout_styled_math(&context, &source, style()).unwrap();
    let b = layout_styled_math(
        &context,
        &source,
        MathTextStyle {
            size_px: 20.0,
            ..style()
        },
    )
    .unwrap();
    for (a, b) in a.glyphs.iter().zip(&b.glyphs) {
        let x = a.font_glyph.as_ref().unwrap();
        let y = b.font_glyph.as_ref().unwrap();
        assert!(Arc::ptr_eq(&x.outline, &y.outline));
        assert_eq!(y.geometry.advance_px, x.geometry.advance_px * 2.0);
        assert_eq!(y.geometry.ink_px.y_max, x.geometry.ink_px.y_max * 2.0);
    }
    assert_eq!(context.cached_glyphs(), 2);
}

#[test]
fn other_authored_faces_and_uncovered_graphemes_use_the_original_measurement() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let fallback = Fallback::default();
    let calls = Rc::clone(&fallback.0);
    let context =
        OpenTypeMathMeasure::new(&font, "Host Math", fallback, 8, 64).with_display_style(true);
    let source = math(NARY);
    let plan = layout_styled_math(
        &context,
        &source,
        MathTextStyle {
            family: "Other",
            ..style()
        },
    )
    .unwrap();
    assert!(plan.glyphs.iter().all(|g| g.font_glyph.is_none()));
    assert_eq!(calls.get(), 4);
    assert_eq!(context.cached_glyphs(), 0);
    let source = math("<m:r><m:t>y x́</m:t></m:r>");
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert_eq!(plan.text, "y x́");
    assert!(plan.glyphs.iter().all(|g| g.font_glyph.is_none()));
}

#[test]
fn preparation_budgets_and_fallback_errors_are_visible_to_the_caller() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let source = math(NARY);
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 0, 64)
        .with_display_style(true);
    assert!(matches!(
        layout_styled_math(&context, &source, style()),
        Err(MathFontLayoutError::GlyphCacheBudgetExceeded)
    ));
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 5)
        .with_display_style(true);
    assert!(matches!(
        layout_styled_math(&context, &source, style()),
        Err(MathFontLayoutError::Outline(_))
    ));
    assert_eq!(context.cached_glyphs(), 0);
    let source = math("<m:r><m:t>!</m:t></m:r>");
    assert!(matches!(
        layout_styled_math(&context, &source, style()),
        Err(MathFontLayoutError::Fallback("missing"))
    ));
}

#[test]
fn a_larger_required_operator_never_silently_uses_an_inadequate_prepared_glyph() {
    let mut bytes = FONT.to_vec();
    let n = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let p = (0..n)
        .map(|n| 12 + 16 * n)
        .find(|&p| &bytes[p..p + 4] == b"MATH")
        .unwrap();
    let p = u32::from_be_bytes(bytes[p + 8..p + 12].try_into().unwrap()) as usize;
    let constants = p + usize::from(u16::from_be_bytes([bytes[p + 4], bytes[p + 5]]));
    bytes[constants + 6..constants + 8].copy_from_slice(&65535_u16.to_be_bytes());
    let font = OpenTypeMathFont::parse(&bytes, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
        .with_display_style(true);
    assert!(matches!(
        layout_styled_math(&context, &math(NARY), style()),
        Err(MathFontLayoutError::AssemblyRequired)
    ));
    assert_eq!(context.cached_glyphs(), 0);
}

#[test]
fn generic_painter_draws_the_prepared_outline_with_y_inversion_without_remeasurement() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
        .with_display_style(true);
    let source = math(NARY);
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    let painter = RecordingPainter::new();
    paint_styled_math(&painter, &plan, 100.0, 200.0).unwrap();
    let ops = painter.ops();
    assert_eq!(ops.first(), Some(&Op::Save));
    assert_eq!(ops.last(), Some(&Op::Restore));
    assert!(ops.contains(&Op::MoveTo(99.6, 204.0)));
    assert!(ops.contains(&Op::LineTo(103.2, 182.0)));
    assert!(ops.contains(&Op::SetFillStyle("#112233".into())));
    assert_eq!(painter.count(|o| matches!(o, Op::Fill)), plan.glyphs.len());
    assert_eq!(
        painter.count(|o| matches!(o, Op::MeasureText(_) | Op::FillText(..) | Op::SetFont(_))),
        0
    );
}

#[test]
fn painter_consumes_quadratic_and_cubic_commands_and_keeps_existing_fraction_rules() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64);
    let source = math("<m:r><m:t>x</m:t></m:r>");
    let mut plan = layout_styled_math(&context, &source, style()).unwrap();
    let prepared = plan.glyphs[0].font_glyph.as_mut().unwrap();
    prepared.outline = Arc::new(MathGlyphOutline {
        bounds: prepared.outline.bounds,
        commands: vec![
            MathOutlineCommand::Move(0.0, 0.0),
            MathOutlineCommand::Quad(100.0, 200.0, 300.0, 400.0),
            MathOutlineCommand::Cubic(100.0, 200.0, 300.0, 400.0, 500.0, 600.0),
            MathOutlineCommand::Close,
        ],
    });
    let painter = RecordingPainter::new();
    paint_styled_math(&painter, &plan, 10.0, 20.0).unwrap();
    assert!(painter
        .ops()
        .contains(&Op::QuadraticCurveTo(11.0, 18.0, 13.0, 16.0)));
    assert!(painter
        .ops()
        .contains(&Op::BezierCurveTo(11.0, 18.0, 13.0, 16.0, 15.0, 14.0)));
    let source = math(
        "<m:f><m:num><m:r><m:t>x</m:t></m:r></m:num><m:den><m:r><m:t>x</m:t></m:r></m:den></m:f>",
    );
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    painter.clear();
    paint_styled_math(&painter, &plan, 10.0, 20.0).unwrap();
    assert_eq!(painter.count(|o| matches!(o, Op::Fill)), 2);
    assert_eq!(painter.count(|o| matches!(o, Op::Stroke)), 1);
}

#[test]
fn owned_font_data_reuses_paths_across_temporary_faces_and_outlives_the_parser() {
    let source = math(NARY);
    let data = MathFontData::new("Host Math".to_owned(), FONT.to_vec(), 8, 64).unwrap();
    assert_eq!(data.byte_len(), FONT.len());
    let a = data
        .layout(Fallback::default(), &source, style(), true)
        .unwrap();
    let b = data
        .layout(
            Fallback::default(),
            &source,
            MathTextStyle {
                size_px: 20.0,
                ..style()
            },
            true,
        )
        .unwrap();
    assert_eq!(data.cached_glyphs(), 2);
    assert_eq!(data.layouts(), 2);
    assert!(Arc::ptr_eq(
        &a.glyphs[0].font_glyph.as_ref().unwrap().outline,
        &b.glyphs[0].font_glyph.as_ref().unwrap().outline
    ));
    drop(data);
    let painter = RecordingPainter::new();
    paint_styled_math(&painter, &a, 10.0, 30.0).unwrap();
    assert_eq!(painter.count(|o| matches!(o, Op::Fill)), 4);
}

#[test]
fn owned_browser_face_rejects_invalid_family_collection_and_oversized_data() {
    assert!(matches!(
        MathFontData::new(" ".to_owned(), FONT.to_vec(), 8, 64),
        Err(MathFontDataError::InvalidFamily)
    ));
    assert_eq!(
        MathFontData::validate(b"ttcf"),
        Err(MathFontDataError::CollectionNotSupported)
    );
    assert_eq!(
        MathFontData::validate(&vec![0; MathFontData::MAX_BYTES + 1]),
        Err(MathFontDataError::TooLarge)
    );
    assert!(matches!(
        MathFontData::validate(b"broken"),
        Err(MathFontDataError::Font(_))
    ));
}

#[test]
fn owned_browser_face_rejects_invalid_script_constants_before_css_registration() {
    let mut bytes = FONT.to_vec();
    let n = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let record = (0..n)
        .map(|n| 12 + 16 * n)
        .find(|&p| &bytes[p..p + 4] == b"MATH")
        .unwrap();
    let table = u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize;
    let constants = table + usize::from(u16::from_be_bytes([bytes[table + 4], bytes[table + 5]]));
    bytes[constants..constants + 2].copy_from_slice(&101_u16.to_be_bytes());
    assert_eq!(
        MathFontData::validate(&bytes),
        Err(MathFontDataError::InvalidConstants)
    );
}

#[test]
fn font_bound_scripts_obey_baseline_drop_and_standard_shift() {
    let bytes = constants_changed(&[(5, 400), (9, 100)]);
    let font = OpenTypeMathFont::parse(&bytes, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64);
    let sup = math(
        "<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>x</m:t></m:r></m:sup></m:sSup>",
    );
    let plan = layout_styled_math(&context, &sup, style()).unwrap();
    assert!((plan.glyphs[1].baseline + 5.0).abs() < 1e-9);
    assert!((plan.metrics.width - 8.65).abs() < 1e-9);
    let sub = math(
        "<m:sSub><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>x</m:t></m:r></m:sub></m:sSub>",
    );
    let plan = layout_styled_math(&context, &sub, style()).unwrap();
    assert!((plan.glyphs[1].baseline - 2.1).abs() < 1e-9);
}

#[test]
fn display_side_limits_use_selected_operator_ink_and_italic_correction() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
        .with_display_style(true);
    let source = math(NARY);
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert!((plan.glyphs[1].baseline + 16.0).abs() < 1e-9);
    assert!((plan.glyphs[2].baseline - 5.0).abs() < 1e-9);
    assert!((plan.glyphs[1].x - 5.5).abs() < 1e-9);
    assert!((plan.glyphs[2].x - 4.8).abs() < 1e-9);
}

#[test]
fn stacked_limits_clear_operator_ink_with_font_gap_and_baseline_constraints() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
        .with_display_style(true);
    let source = math(&NARY.replace("subSup", "undOvr"));
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert!((plan.glyphs[1].baseline + 21.0).abs() < 1e-9);
    assert!((plan.glyphs[2].baseline - 10.7).abs() < 1e-9);
    assert!((plan.glyphs[1].x - 1.475).abs() < 1e-9);
    assert!((plan.glyphs[2].x - 0.775).abs() < 1e-9);
    assert!(
        (plan.glyphs[1].baseline.abs()
            - plan.glyphs[0].font_glyph.as_ref().unwrap().metrics().ascent)
            >= 3.0
    );
}

fn constants_changed(values: &[(usize, i16)]) -> Vec<u8> {
    let mut bytes = FONT.to_vec();
    let count = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let record = (0..count)
        .map(|i| 12 + 16 * i)
        .find(|&p| &bytes[p..p + 4] == b"MATH")
        .unwrap();
    let table = u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize;
    let constants = table + usize::from(u16::from_be_bytes([bytes[table + 4], bytes[table + 5]]));
    for &(index, value) in values {
        bytes[constants + 8 + index * 4..constants + 10 + index * 4]
            .copy_from_slice(&value.to_be_bytes());
    }
    bytes
}

#[test]
fn simultaneous_scripts_open_the_font_gap_superscript_first_then_subscript() {
    let bytes = constants_changed(&[(5, 400), (9, 100), (11, 800), (12, 900)]);
    let font = OpenTypeMathFont::parse(&bytes, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64);
    let source = math("<m:sSubSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>x</m:t></m:r></m:sub><m:sup><m:r><m:t>x</m:t></m:r></m:sup></m:sSubSup>");
    let before = serde_json::to_value(&source).unwrap();
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert_eq!(plan.glyphs[1].style.size_px, 6.5);
    assert!((plan.glyphs[1].baseline - 3.55).abs() < 1e-9);
    assert!((plan.glyphs[2].baseline + 9.0).abs() < 1e-9);
    assert!((plan.metrics.width - 8.65).abs() < 1e-9);
    assert_eq!(serde_json::to_value(&source).unwrap(), before);
}

#[test]
fn nested_scripts_use_root_relative_sizes_and_scaled_base_constants() {
    let bytes = constants_changed(&[(5, 400), (9, 100)]);
    let font = OpenTypeMathFont::parse(&bytes, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64);
    let source = math("<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>x</m:t></m:r></m:sup></m:sSup></m:sup></m:sSup>");
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert_eq!(
        plan.glyphs
            .iter()
            .map(|g| g.style.size_px)
            .collect::<Vec<_>>(),
        [10.0, 6.5, 4.0]
    );
    assert!((plan.glyphs[2].baseline - plan.glyphs[1].baseline + 3.25).abs() < 1e-9);
}

#[test]
fn long_stacked_limits_reserve_left_and_right_advance() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
        .with_display_style(true);
    let source = math(
        &NARY
            .replace("subSup", "undOvr")
            .replace("<m:t>x</m:t>", "<m:t>xxx</m:t>"),
    );
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert!((plan.glyphs[0].x - 2.475).abs() < 1e-9);
    assert!((plan.glyphs[1].x - 0.7).abs() < 1e-9);
    assert!((plan.glyphs[4].x).abs() < 1e-9);
    assert!((plan.glyphs[7].x - 11.45).abs() < 1e-9);
    assert!((plan.metrics.width - 26.45).abs() < 1e-9);
}

#[test]
fn hidden_nary_limits_skip_layout_without_mutating_source() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    for value in ["", " m:val=\"1\"", " m:val=\"true\"", " m:val=\"on\""] {
        let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
            .with_display_style(true);
        let xml = NARY.replace(
            "</m:naryPr>",
            &format!("<m:supHide{value}/><m:subHide{value}/></m:naryPr>"),
        );
        let source = math(&xml);
        let before = serde_json::to_value(&source).unwrap();
        let plan = layout_styled_math(&context, &source, style()).unwrap();
        assert_eq!(plan.text, "∫x");
        assert!((plan.glyphs[1].x - 6.5).abs() < 1e-9);
        assert_eq!(serde_json::to_value(&source).unwrap(), before);
    }
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64)
        .with_display_style(true);
    let source = math(&NARY.replace(
        "</m:naryPr>",
        "<m:supHide m:val=\"0\"/><m:subHide m:val=\"false\"/></m:naryPr>",
    ));
    assert_eq!(
        layout_styled_math(&context, &source, style()).unwrap().text,
        "∫xxx"
    );
}

#[test]
fn unbound_combined_scripts_preserve_property_skips_and_source_order() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Another Face", Fallback::default(), 8, 64);
    for children in [
        "<m:sSubSupPr><m:r><m:t>!</m:t></m:r></m:sSubSupPr><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>xx</m:t></m:r></m:sub><m:sup><m:r><m:t>xxx</m:t></m:r></m:sup>",
        "<m:sub><m:r><m:t>xx</m:t></m:r></m:sub><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>xxx</m:t></m:r></m:sup>",
    ] {
        let source = math(&format!("<m:sSubSup>{children}</m:sSubSup>"));
        let before = serde_json::to_value(&source).unwrap();
        let a = layout_styled_math(&Fallback::default(), &source, style()).unwrap();
        let b = layout_styled_math(&context, &source, style()).unwrap();
        assert_eq!(a.text, b.text);
        assert_eq!(a.metrics, b.metrics);
        assert_eq!(a.rules.len(), b.rules.len());
        assert_eq!(a.glyphs.len(), b.glyphs.len());
        for (a, b) in a.glyphs.iter().zip(&b.glyphs) {
            assert_eq!((a.x, a.baseline, a.width, a.style), (b.x, b.baseline, b.width, b.style));
        }
        assert_eq!(serde_json::to_value(&source).unwrap(), before);
    }
}

#[test]
fn bound_formula_line_box_reserves_font_leading_and_typographic_floor_without_moving_ink() {
    let font = OpenTypeMathFont::parse(FONT, 0).unwrap();
    let context = OpenTypeMathMeasure::new(&font, "Host Math", Fallback::default(), 8, 64);
    let source = math("<m:r><m:t>x</m:t></m:r>");
    let plan = layout_styled_math(&context, &source, style()).unwrap();
    assert_eq!(plan.metrics.ascent, 7.0);
    assert_eq!(plan.metrics.descent, 0.0);
    assert_eq!(plan.glyphs[0].baseline, 0.0);
    assert!((plan.line_metrics.ascent - 8.1).abs() < 1e-9);
    assert_eq!(plan.line_metrics.descent, 4.0);
    assert_eq!(plan.line_metrics.width, plan.metrics.width);
}

#[test]
fn bound_formula_grid_rounds_after_at_least_and_multiple_spacing_floor() {
    use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
    use jade_docx_layout::math_layout::math_block_line_box;
    use jade_docx_reader::{DocGrid, ParagraphProperties, Spacing};
    set_word_layout_platform(WordLayoutPlatform::Mac);
    let grid = DocGrid {
        grid_type: Some("lines".into()),
        line_pitch: Some(18.0),
        ..DocGrid::default()
    };
    let required = MathMetrics {
        width: 40.0,
        ascent: 25.0,
        descent: 8.0,
    };
    for (rule, line) in [("atLeast", 12.0), ("auto", 1.15)] {
        let properties = ParagraphProperties {
            spacing: Some(Spacing {
                line: Some(line),
                line_rule: Some(rule.into()),
                ..Spacing::default()
            }),
            ..ParagraphProperties::default()
        };
        let resolved =
            math_block_line_box(required, 8.0, Some(&properties), Some(&grid), true, None);
        assert_eq!(resolved.height, 48.0);
        assert_eq!(resolved.baseline, 32.5);
    }
    set_word_layout_platform(WordLayoutPlatform::Windows);
}

#[test]
fn formula_exact_spacing_keeps_authored_height_and_unbound_plan_has_no_math_leading() {
    use jade_docx_layout::math_layout::math_block_line_box;
    use jade_docx_reader::{ParagraphProperties, Spacing};
    let source = math("<m:r><m:t>x</m:t></m:r>");
    let plan = layout_styled_math(&Fallback::default(), &source, style()).unwrap();
    assert_eq!(plan.line_metrics, plan.metrics);
    let exact = ParagraphProperties {
        spacing: Some(Spacing {
            line: Some(12.0),
            line_rule: Some("exact".into()),
            ..Spacing::default()
        }),
        ..ParagraphProperties::default()
    };
    let line = math_block_line_box(
        MathMetrics {
            width: 0.0,
            ascent: 25.0,
            descent: 8.0,
        },
        8.0,
        Some(&exact),
        None,
        true,
        None,
    );
    assert_eq!(line.height, 16.0);
    assert_eq!(line.baseline, 25.0);
}
