use super::*;
use jade_docx_reader::document::MathElementKind;
use std::cell::RefCell;

#[test]
fn break_after_reserves_the_trailing_operator_width() {
    let math = parse("<m:r><m:t>aa+bb+cc+dd+ee+ff+gg+hh</m:t></m:r>");
    for width in [40.0, 48.0, 60.0, 80.0] {
        for indent in [0.0, 8.0] {
            let plan = plan_with(
                &math,
                MathLayoutOptions {
                    max_width: Some(width),
                    settings: MathSettings {
                        break_binary: BreakBinary::After,
                        wrap_indent_px: indent,
                        ..Default::default()
                    },
                    ..display()
                },
            );
            assert!(plan.lines.len() > 1);
            for line in &plan.lines {
                assert!(
                    line.x + line.width <= width + 1e-9,
                    "width={width}, indent={indent}: {line:?}"
                );
            }
            for line in &plan.lines[..plan.lines.len() - 1] {
                let last = plan
                    .glyphs
                    .iter()
                    .rev()
                    .find(|g| g.baseline == line.baseline)
                    .unwrap();
                assert_eq!(&plan.text[last.text_range.clone()], "+");
            }
        }
    }
}

#[test]
fn break_repeat_places_the_same_operator_on_both_lines() {
    let math = parse("<m:r><m:t>aa+bb+cc+dd+ee+ff+gg+hh</m:t></m:r>");
    // A continuation between two plus signs is about 34 px before indent.
    for width in [44.0, 48.0, 60.0, 80.0] {
        for indent in [0.0, 8.0] {
            let plan = plan_with(
                &math,
                MathLayoutOptions {
                    max_width: Some(width),
                    settings: MathSettings {
                        break_binary: BreakBinary::Repeat,
                        wrap_indent_px: indent,
                        ..Default::default()
                    },
                    ..display()
                },
            );
            assert!(plan.lines.len() > 1);
            assert_eq!(plan.text.matches('+').count(), 7);
            let edges = |line: &MathLine| {
                let mut glyphs: Vec<_> = plan
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.visible && glyph.baseline == line.baseline)
                    .collect();
                glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
                (
                    &plan.text[glyphs.first().unwrap().text_range.clone()],
                    &plan.text[glyphs.last().unwrap().text_range.clone()],
                )
            };
            for (index, line) in plan.lines.iter().enumerate() {
                assert!(
                    line.x + line.width <= width + 1e-9,
                    "width={width}, indent={indent}: {line:?}"
                );
                let (first, last) = edges(line);
                if index > 0 {
                    assert_eq!(first, "+");
                }
                if index + 1 < plan.lines.len() {
                    assert_eq!(last, "+");
                }
            }
        }
    }
}

#[test]
fn repeated_subtraction_displays_the_requested_signs_without_changing_the_equation() {
    struct Signs;
    impl StyledMathMeasure for Signs {
        type Error = std::convert::Infallible;

        fn measure_glyph(
            &self,
            text: &str,
            _: MathTextStyle<'_>,
        ) -> Result<MathMetrics, Self::Error> {
            Ok(MathMetrics {
                width: if text == "+" { 10.0 } else { 4.0 },
                ascent: if text == "+" { 12.0 } else { 8.0 },
                descent: 2.0,
            })
        }
    }

    let math = parse("<m:r><m:t>aa-bb−cc-dd-ee-ff+gg</m:t></m:r>");
    for (setting, end_sign, start_sign) in [
        (BreakBinarySubtraction::MinusMinus, "−", "−"),
        (BreakBinarySubtraction::PlusMinus, "+", "−"),
        (BreakBinarySubtraction::MinusPlus, "−", "+"),
    ] {
        let options = MathLayoutOptions {
            max_width: Some(38.0),
            settings: MathSettings {
                break_binary: BreakBinary::Repeat,
                break_binary_subtraction: setting,
                wrap_indent_px: 2.0,
                ..Default::default()
            },
            ..display()
        };
        let plan = layout_styled_math_with_options(&Signs, &math, style(), &options).unwrap();
        assert_eq!(plan.text.matches('−').count(), 5);
        assert_eq!(plan.text.matches('+').count(), 1);
        let mut minus_breaks = 0;
        for pair in plan.lines.windows(2) {
            let [left, right] = pair else { unreachable!() };
            let last = plan
                .glyphs
                .iter()
                .filter(|g| g.baseline == left.baseline)
                .max_by(|a, b| a.x.total_cmp(&b.x))
                .unwrap();
            let first = plan
                .glyphs
                .iter()
                .filter(|g| g.baseline == right.baseline)
                .min_by(|a, b| a.x.total_cmp(&b.x))
                .unwrap();
            assert_eq!(last.text_range, first.text_range);
            if &plan.text[first.text_range.clone()] == "−" {
                minus_breaks += 1;
                assert_eq!(last.display_text(&plan.text), end_sign);
                assert_eq!(first.display_text(&plan.text), start_sign);
                for (glyph, line) in [(last, left), (first, right)] {
                    if glyph.display_text(&plan.text) == "+" {
                        assert_eq!(glyph.width, 10.0);
                        assert!(line.ascent >= 12.0);
                    }
                }
            } else {
                assert_eq!(last.display_text(&plan.text), "+");
                assert_eq!(first.display_text(&plan.text), "+");
            }
        }
        assert!(minus_breaks > 0);
        assert!(plan
            .lines
            .iter()
            .all(|line| line.x + line.width <= 38.0 + 1e-9));

        let unwrapped = layout_styled_math_with_options(
            &Signs,
            &math,
            style(),
            &MathLayoutOptions {
                max_width: None,
                ..options
            },
        )
        .unwrap();
        assert_eq!(unwrapped.lines.len(), 1);
        assert!(unwrapped
            .glyphs
            .iter()
            .all(|g| g.display_text(&unwrapped.text) == &unwrapped.text[g.text_range.clone()]));
    }
}

#[derive(Default)]
struct Measure {
    calls: RefCell<Vec<(String, f64)>>,
}
impl StyledMathMeasure for Measure {
    type Error = &'static str;
    fn measure_glyph(
        &self,
        text: &str,
        style: MathTextStyle<'_>,
    ) -> Result<MathMetrics, Self::Error> {
        self.calls
            .borrow_mut()
            .push((text.to_string(), style.size_px));
        if text == "!" {
            return Err("unavailable");
        }
        Ok(MathMetrics {
            width: style.size_px * 0.5,
            ascent: style.size_px * 0.8,
            descent: style.size_px * 0.2,
        })
    }
}
fn parse(body: &str) -> MathParagraph {
    jade_docx_reader::parser::math::parse_math_paragraph(format!(r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{body}</m:oMath>"#)).unwrap()
}
fn style() -> MathTextStyle<'static> {
    MathTextStyle {
        family: "Cambria Math",
        size_px: 14.0,
        color: "#000000",
        variant: MathVariant::Italic,
    }
}
#[test]
fn styled_runs_preserve_authored_font_color_and_unicode_math_alphabetics() {
    let math = parse(
        r#"<m:r><w:rPr><w:rFonts w:ascii="Other Math"/><w:sz w:val="18"/><w:color w:val="112233"/></w:rPr><m:rPr><m:sty m:val="bi"/></m:rPr><m:t>Az2</m:t></m:r><m:r><m:rPr><m:nor/></m:rPr><m:t>xy</m:t></m:r><m:r><m:t>h</m:t></m:r>"#,
    );
    let m = Measure::default();
    let p = layout_styled_math(&m, &math, style()).unwrap();
    assert_eq!(p.text, "𝑨𝒛𝟐xyℎ");
    assert_eq!(m.calls.borrow().len(), p.glyphs.len());
    assert_eq!(p.glyphs[0].style.family, "Other Math");
    assert_eq!(p.glyphs[0].style.size_px, 12.0);
    assert_eq!(p.glyphs[0].style.color, "112233");
    assert_eq!(p.glyphs[3].style.size_px, 14.0);
    for g in &p.glyphs {
        assert!(!p.text[g.text_range.clone()].is_empty());
    }
}
#[test]
fn structured_nary_delimiters_and_scripts_have_measured_utf8_glyph_ranges() {
    let math = parse(
        r#"<m:r><m:t>P</m:t></m:r><m:d><m:e><m:r><m:t>x</m:t></m:r></m:e></m:d><m:nary><m:naryPr><m:chr m:val="∑"/><m:limLoc m:val="subSup"/></m:naryPr><m:sub><m:r><m:t>a</m:t></m:r></m:sub><m:sup><m:r><m:t>b</m:t></m:r></m:sup><m:e><m:r><m:t>f</m:t></m:r></m:e></m:nary>"#,
    );
    let m = Measure::default();
    let p = layout_styled_math(&m, &math, style()).unwrap();
    assert_eq!(p.text, "𝑃(𝑥)∑𝑏𝑎𝑓");
    let glyph = |text: &str| {
        p.glyphs
            .iter()
            .find(|g| &p.text[g.text_range.clone()] == text)
            .unwrap()
    };
    assert!(glyph("𝑏").baseline < glyph("∑").baseline);
    assert!(glyph("𝑎").baseline > glyph("∑").baseline);
    assert_eq!(glyph("𝑏").style.size_px, 14.0 * 0.8);
    assert!(glyph("𝑏").x > glyph("∑").x);
    assert_eq!(m.calls.borrow().len(), p.glyphs.len());
    for g in &p.glyphs {
        assert!(g.x >= 0.0 && g.x + g.width <= p.metrics.width + 1e-9);
        assert!(g.baseline - g.style.size_px * 0.8 >= -p.metrics.ascent - 1e-9);
        assert!(g.baseline + g.style.size_px * 0.2 <= p.metrics.descent + 1e-9);
    }
}
#[test]
fn styled_fraction_rules_and_children_fit_bounds_and_errors_propagate() {
    let math = parse(
        "<m:f><m:num><m:r><m:t>ab</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f>",
    );
    let p = layout_styled_math(&Measure::default(), &math, style()).unwrap();
    assert_eq!(p.rules.len(), 1);
    assert!(p.glyphs[0].baseline < 0.0 && p.glyphs[2].baseline > 0.0);
    for g in &p.glyphs {
        assert!(g.x >= 0.0 && g.x + g.width <= p.metrics.width + 1e-9);
    }
    assert_eq!(
        layout_styled_math(
            &Measure::default(),
            &parse("<m:r><m:t>!</m:t></m:r>"),
            style()
        )
        .unwrap_err(),
        "unavailable"
    );
}

// --- Every OMML structure, on a measurer whose glyphs are half an em
// wide, 0.8 em tall and 0.2 em deep.

fn leak(body: &str) -> &'static MathParagraph {
    Box::leak(Box::new(parse(body)))
}
fn plan_with<'a>(math: &'a MathParagraph, options: MathLayoutOptions<'a>) -> StyledMathLayout<'a> {
    layout_styled_math_with_options(&Measure::default(), math, style(), &options).unwrap()
}
fn glyph<'p>(p: &'p StyledMathLayout<'_>, text: &str) -> &'p MathGlyph<'p> {
    p.glyphs
        .iter()
        .find(|g| &p.text[g.text_range.clone()] == text)
        .unwrap_or_else(|| panic!("no glyph {text} in {}", p.text))
}
fn glyphs<'p>(p: &'p StyledMathLayout<'_>, text: &str) -> Vec<&'p MathGlyph<'p>> {
    p.glyphs
        .iter()
        .filter(|g| &p.text[g.text_range.clone()] == text)
        .collect()
}
fn display() -> MathLayoutOptions<'static> {
    MathLayoutOptions::default()
}

#[test]
fn operators_are_spaced_by_class_and_a_leading_minus_is_unary() {
    let p = plan_with(leak("<m:r><m:t>a=b+c</m:t></m:r>"), display());
    let em = 14.0;
    let half = em * 0.5;
    let (a, eq, b, plus) = (
        glyph(&p, "𝑎"),
        glyph(&p, "="),
        glyph(&p, "𝑏"),
        glyph(&p, "+"),
    );
    assert!((eq.x - a.x - half - em * 5.0 / 18.0).abs() < 1e-9);
    assert!((b.x - eq.x - half - em * 5.0 / 18.0).abs() < 1e-9);
    assert!((plus.x - b.x - half - em * 4.0 / 18.0).abs() < 1e-9);
    let p = plan_with(leak("<m:r><m:t>-x</m:t></m:r>"), display());
    assert_eq!(p.text, "−𝑥");
    assert!((glyph(&p, "𝑥").x - half).abs() < 1e-9);
}

#[test]
fn display_fractions_keep_their_size_and_inline_ones_shrink() {
    let math = parse(
        "<m:f><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>",
    );
    let shown = plan_with(&math, display());
    assert_eq!(glyph(&shown, "𝑎").style.size_px, 14.0);
    assert!(glyph(&shown, "𝑎").baseline < 0.0 && glyph(&shown, "𝑏").baseline > 0.0);
    assert_eq!(shown.rules.len(), 1);
    let inline = plan_with(&math, MathLayoutOptions::inline());
    assert!((glyph(&inline, "𝑎").style.size_px - 14.0 * 0.8).abs() < 1e-9);
    let skewed = plan_with(leak("<m:f><m:fPr><m:type m:val=\"skw\"/></m:fPr><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>"),
            display(),
        );
    assert!(glyph(&skewed, "𝑎").x < glyph(&skewed, "𝑏").x);
    assert_eq!(skewed.shapes.len(), 1);
    let linear = plan_with(leak("<m:f><m:fPr><m:type m:val=\"lin\"/></m:fPr><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>"),
            display(),
        );
    assert_eq!(linear.text, "𝑎/𝑏");
    assert!(linear.glyphs.iter().all(|g| g.baseline == 0.0));
    let stacked = plan_with(leak("<m:f><m:fPr><m:type m:val=\"noBar\"/></m:fPr><m:num><m:r><m:t>n</m:t></m:r></m:num><m:den><m:r><m:t>k</m:t></m:r></m:den></m:f>"),
            display(),
        );
    assert!(stacked.rules.is_empty());
}

#[test]
fn radicals_draw_a_sign_over_the_base_with_a_small_degree() {
    let p = plan_with(leak("<m:rad><m:deg><m:r><m:t>3</m:t></m:r></m:deg><m:e><m:r><m:t>x</m:t></m:r></m:e></m:rad>"),
            display(),
        );
    let degree = glyph(&p, "3");
    let base = glyph(&p, "𝑥");
    assert!(degree.style.size_px < base.style.size_px);
    assert!(degree.baseline < 0.0);
    assert!(base.x > degree.x);
    // No face: the sign and its vinculum are one drawn stroke.
    assert_eq!(p.shapes.len(), 1);
    assert!(!glyph(&p, "√").visible);
    let hidden = plan_with(leak("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e><m:r><m:t>x</m:t></m:r></m:e></m:rad>"),
            display(),
        );
    assert!(!hidden.text.contains('3'));
}

#[test]
fn delimiters_grow_around_tall_content_and_separate_their_arguments() {
    let tall = plan_with(leak("<m:d><m:e><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:e></m:d>"),
            display(),
        );
    assert!(!glyph(&tall, "(").visible && !glyph(&tall, ")").visible);
    assert_eq!(tall.shapes.len(), 2);
    let close = glyph(&tall, ")");
    assert!(close.ascent + close.descent > 14.0);
    let flat = plan_with(leak("<m:d><m:dPr><m:begChr m:val=\"[\"/><m:sepChr m:val=\"|\"/><m:endChr m:val=\"\"/></m:dPr><m:e><m:r><m:t>a</m:t></m:r></m:e><m:e><m:r><m:t>b</m:t></m:r></m:e></m:d>"),
            display(),
        );
    assert_eq!(flat.text, "[𝑎|𝑏");
    assert!(flat.glyphs.iter().all(|g| g.visible));
    assert!(glyph(&flat, "|").x > glyph(&flat, "𝑎").x + 7.0);
}

#[test]
fn accents_bars_and_group_characters_go_over_and_under() {
    let p = plan_with(leak("<m:acc><m:accPr><m:chr m:val=\"\u{20D7}\"/></m:accPr><m:e><m:r><m:t>v</m:t></m:r></m:e></m:acc>"),
            display(),
        );
    let (_, top, _, _) = shapes::bounds(&p.shapes[0].commands).unwrap();
    assert!(top < -glyph(&p, "𝑣").ascent);
    let p = plan_with(leak("<m:bar><m:barPr><m:pos m:val=\"top\"/></m:barPr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:bar>"),
            display(),
        );
    assert!(p.rules[0].y < -11.2);
    let p = plan_with(
        leak("<m:bar><m:e><m:r><m:t>x</m:t></m:r></m:e></m:bar>"),
        display(),
    );
    assert!(p.rules[0].y > 2.8);
    let p = plan_with(
        leak("<m:groupChr><m:e><m:r><m:t>abc</m:t></m:r></m:e></m:groupChr>"),
        display(),
    );
    let (left, top, right, _) = shapes::bounds(&p.shapes[0].commands).unwrap();
    assert!(top > 2.8, "the brace hangs under the base");
    assert!((right - left - 21.0).abs() < 1e-6, "as wide as the base");
}

#[test]
fn limits_and_prescripts_are_placed_around_their_base() {
    let p = plan_with(leak("<m:limLow><m:e><m:r><m:rPr><m:sty m:val=\"p\"/></m:rPr><m:t>lim</m:t></m:r></m:e><m:lim><m:r><m:t>n</m:t></m:r></m:lim></m:limLow>"),
            display(),
        );
    assert_eq!(p.text, "lim𝑛");
    let n = glyph(&p, "𝑛");
    assert!(n.baseline > 0.0);
    assert!(
        (n.x + n.width / 2.0 - 21.0 / 2.0).abs() < 1e-6,
        "centred under lim"
    );
    let p = plan_with(leak("<m:sPre><m:sub><m:r><m:t>1</m:t></m:r></m:sub><m:sup><m:r><m:t>2</m:t></m:r></m:sup><m:e><m:r><m:t>F</m:t></m:r></m:e></m:sPre>"),
            display(),
        );
    assert!(glyph(&p, "1").x < glyph(&p, "𝐹").x && glyph(&p, "2").x < glyph(&p, "𝐹").x);
    assert!(glyph(&p, "1").baseline > 0.0 && glyph(&p, "2").baseline < 0.0);
}

#[test]
fn n_ary_limits_go_over_sums_and_beside_integrals_and_inline() {
    let sum = "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub><m:r><m:t>i</m:t></m:r></m:sub><m:sup><m:r><m:t>n</m:t></m:r></m:sup><m:e><m:r><m:t>x</m:t></m:r></m:e></m:nary>";
    let p = plan_with(leak(sum), display());
    let (op, n) = (glyph(&p, "∑"), glyph(&p, "𝑛"));
    assert!(
        n.x > op.x && n.x + n.width < op.x + op.width + 1e-9,
        "over the sum"
    );
    assert!(op.style.size_px > 14.0, "drawn larger in display math");
    let p = plan_with(leak(sum), MathLayoutOptions::inline());
    assert!(glyph(&p, "𝑛").x >= glyph(&p, "∑").x + glyph(&p, "∑").width - 1e-9);
    let integral = sum.replace("<m:chr m:val=\"∑\"/>", "");
    let p = plan_with(leak(&integral), display());
    assert!(glyph(&p, "𝑛").x >= glyph(&p, "∫").x + glyph(&p, "∫").width - 1e-9);
}

#[test]
fn matrices_align_columns_and_arrays_align_their_ampersands() {
    let p = plan_with(leak("<m:m><m:mr><m:e><m:r><m:t>1</m:t></m:r></m:e><m:e><m:r><m:t>0</m:t></m:r></m:e></m:mr><m:mr><m:e><m:r><m:t>22</m:t></m:r></m:e><m:e><m:r><m:t>3</m:t></m:r></m:e></m:mr></m:m>"),
            display(),
        );
    let one = glyph(&p, "1");
    let twos = glyphs(&p, "2");
    let centre_one = one.x + one.width / 2.0;
    let centre_22 = (twos[0].x + twos[1].x + twos[1].width) / 2.0;
    assert!((centre_one - centre_22).abs() < 1e-9, "column centred");
    assert!(one.baseline < glyph(&p, "3").baseline);
    let p = plan_with(leak("<m:eqArr><m:e><m:r><m:t>x+y&amp;=1</m:t></m:r></m:e><m:e><m:r><m:t>2x-y&amp;=3</m:t></m:r></m:e></m:eqArr>"),
            display(),
        );
    let equals = glyphs(&p, "=");
    assert!((equals[0].x - equals[1].x).abs() < 1e-9);
    assert!(glyphs(&p, "&").iter().all(|g| !g.visible && g.width == 0.0));
}

#[test]
fn display_equation_array_distributes_space_between_aligned_column_pairs() {
    let render = |properties: &str, width: Option<f64>| {
        let math = leak(&format!(
            r#"<m:eqArr><m:eqArrPr>{properties}</m:eqArrPr><m:e><m:r><m:t>11&amp;=1&amp;33&amp;=3</m:t></m:r></m:e><m:e><m:r><m:t>222&amp;=2&amp;444&amp;=4</m:t></m:r></m:e></m:eqArr>"#
        ));
        plan_with(
            math,
            MathLayoutOptions {
                max_width: width,
                ..display()
            },
        )
    };
    let width = 180.0;
    let compact = render("", Some(width));
    let compact_equals = glyphs(&compact, "=");
    assert_eq!(compact_equals.len(), 4);
    let max_dist = r#"<m:maxDist/>"#;
    let obj_dist = r#"<m:objDist m:val="1"/>"#;
    for (properties, pads) in [
        ("", (0.0, 0.0, 0.0)),
        (obj_dist, (0.0, 0.0, 0.0)),
        (max_dist, (1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0)),
        (&format!("{max_dist}{obj_dist}"), (0.0, 1.0, 0.0)),
    ] {
        let plan = render(properties, Some(width));
        let equals = glyphs(&plan, "=");
        assert_eq!(equals.len(), compact_equals.len());
        assert_eq!(plan.text, compact.text);
        let extra = width - compact.metrics.width;
        let (left, between, right) = (pads.0 * extra, pads.1 * extra, pads.2 * extra);
        assert!((plan.metrics.width - compact.metrics.width - left - between - right).abs() < 1e-9);
        for (i, (old, new)) in compact_equals.iter().zip(equals).enumerate() {
            let shift = left + if i % 2 == 0 { 0.0 } else { between };
            assert!((new.x - old.x - shift).abs() < 1e-9, "{properties}");
            assert_eq!(old.baseline, new.baseline);
        }
        assert!(glyphs(&plan, "&")
            .iter()
            .all(|g| !g.visible && g.width == 0.0));
    }
    assert_eq!(render(max_dist, None).metrics, compact.metrics);
    assert_eq!(render(max_dist, Some(20.0)).metrics, compact.metrics);
}

#[test]
fn distribution_does_not_expand_arrays_without_alignment_or_their_own_display_line() {
    let array = |row: &str| {
        format!("<m:eqArr><m:eqArrPr><m:maxDist/></m:eqArrPr><m:e><m:r><m:t>{row}</m:t></m:r></m:e></m:eqArr>")
    };
    let unmarked = array("11=2");
    let marked = array("11&amp;=2&amp;33&amp;=4");
    let nested =
        format!("<m:f><m:num>{marked}</m:num><m:den><m:r><m:t>5</m:t></m:r></m:den></m:f>");
    let with_neighbor = format!("{marked}<m:r><m:t>5</m:t></m:r>");
    for (body, display) in [
        (&unmarked, true),
        (&nested, true),
        (&with_neighbor, true),
        (&marked, false),
    ] {
        let math = parse(body);
        let options = MathLayoutOptions {
            display,
            ..Default::default()
        };
        let compact = plan_with(&math, options);
        let constrained = plan_with(
            &math,
            MathLayoutOptions {
                max_width: Some(500.0),
                ..options
            },
        );
        assert_eq!(compact.metrics, constrained.metrics, "{body}");
        assert!(compact
            .glyphs
            .iter()
            .zip(&constrained.glyphs)
            .all(|(a, b)| a.x == b.x));
    }
}

#[test]
fn boxes_phantoms_and_border_boxes() {
    let p = plan_with(leak("<m:phant><m:phantPr><m:show m:val=\"0\"/></m:phantPr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:phant><m:r><m:t>y</m:t></m:r>"),
            display(),
        );
    assert!(!glyph(&p, "𝑥").visible);
    assert!(
        (glyph(&p, "𝑦").x - 7.0).abs() < 1e-9,
        "the phantom keeps its room"
    );
    let p = plan_with(leak("<m:phant><m:phantPr><m:zeroWid m:val=\"1\"/></m:phantPr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:phant><m:r><m:t>y</m:t></m:r>"),
            display(),
        );
    assert_eq!(glyph(&p, "𝑦").x, 0.0);
    let p = plan_with(leak("<m:borderBox><m:borderBoxPr><m:hideLeft m:val=\"1\"/><m:strikeBLTR m:val=\"1\"/></m:borderBoxPr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:borderBox>"),
            display(),
        );
    // Top, bottom, right and the diagonal: four strokes.
    let moves = p.shapes[0]
        .commands
        .iter()
        .filter(|c| matches!(c, MathPathCommand::Move(..)))
        .count();
    assert_eq!(moves, 4);
    assert!(glyph(&p, "𝑥").x > 0.0);
}

#[test]
fn normal_text_and_alphabets_follow_their_run_properties() {
    let p = plan_with(leak("<m:r><m:rPr><m:nor/></m:rPr><m:t>if x=1</m:t></m:r><m:r><m:rPr><m:scr m:val=\"double-struck\"/><m:sty m:val=\"p\"/></m:rPr><m:t>R</m:t></m:r>"),
            MathLayoutOptions {
                text_family: Some("Times New Roman"),
                ..display()
            },
        );
    assert!(p.text.starts_with("if x=1"));
    assert_eq!(glyph(&p, "x").style.family, "Times New Roman");
    // No operator spacing in normal text.
    assert!((glyph(&p, "=").x - glyph(&p, "x").x - 7.0).abs() < 1e-9);
    assert!(p.text.ends_with('ℝ'));
}

#[test]
fn display_lines_break_where_asked_and_where_they_overflow() {
    let math = parse("<m:r><m:t>a=b</m:t></m:r><m:r><m:rPr><m:brk/></m:rPr><m:t>+c</m:t></m:r>");
    let p = plan_with(&math, display());
    assert_eq!(p.lines.len(), 2);
    assert!(glyph(&p, "+").baseline > 0.0);
    // The continuation aligns under the first relation.
    assert!((glyph(&p, "+").x - glyph(&p, "=").x).abs() < 1e-9);
    let long = parse("<m:r><m:t>a=b+c+d+e+f+g+h</m:t></m:r>");
    let p = plan_with(
        &long,
        MathLayoutOptions {
            max_width: Some(80.0),
            settings: MathSettings {
                wrap_indent_px: 10.0,
                ..MathSettings::default()
            },
            ..display()
        },
    );
    assert!(p.lines.len() >= 2);
    for line in &p.lines[1..] {
        assert!(line.baseline > 0.0);
    }
    let p = plan_with(&long, MathLayoutOptions::inline());
    assert_eq!(p.lines.len(), 1);
}

#[test]
fn boxes_record_every_element_and_empty_arguments() {
    let math = parse("<m:f><m:num><m:r><m:t>ab</m:t></m:r></m:num><m:den/></m:f>");
    let p = plan_with(&math, display());
    // oMath (0), fraction (0,0), numerator (0,0,0), run, text …
    let numerator = p.box_for(&[0, 0, 0]).unwrap();
    assert_eq!(numerator.kind, MathElementKind::Numerator);
    let text = p.box_for(&[0, 0, 0, 0, 0]).unwrap();
    assert_eq!(text.glyphs.len(), 2);
    let denominator = p.box_for(&[0, 0, 1]).unwrap();
    assert!(denominator.empty && denominator.width > 0.0);
    assert!(denominator.baseline > 0.0);
    let hit = p
        .box_at(denominator.x + 1.0, denominator.baseline - 1.0)
        .unwrap();
    assert_eq!(hit.path, vec![0, 0, 1]);
}

#[test]
fn eq_fields_lay_out_upright_at_one_size() {
    let elements =
        crate::math_layout::eq_field::eq_field_elements(r"EQ \f(1,\r(x)) \s\up8(2)").unwrap();
    let p = layout_styled_elements(
        &Measure::default(),
        &elements,
        style(),
        &MathLayoutOptions::eq_field(),
    )
    .unwrap();
    assert!(p.glyphs.iter().all(|g| g.style.size_px == 14.0));
    assert!(p.text.contains('x'), "upright: {}", p.text);
    assert!(glyph(&p, "1").baseline < 0.0);
    let raised = glyph(&p, "2");
    assert!((raised.baseline + 8.0 * 96.0 / 72.0).abs() < 1e-9);
}

#[test]
fn display_math_margins_narrow_wrapping_and_drop_an_excess_left_margin() {
    let settings = MathSettings {
        display_margins_px: (40.0, 20.0),
        ..Default::default()
    };
    assert_eq!(settings.display_content_box(10.0, 140.0), (50.0, 80.0));
    assert_eq!(settings.display_content_box(10.0, 50.0), (10.0, 30.0));
    assert_eq!(settings.display_content_box(10.0, 10.0), (10.0, 1.0));
    assert_eq!(
        MathSettings {
            use_display_defaults: false,
            ..settings
        }
        .display_content_box(10.0, 140.0),
        (10.0, 140.0)
    );
    let math = parse("<m:r><m:t>a=b+c+d+e+f+g+h</m:t></m:r>");
    let options = MathLayoutOptions {
        max_width: Some(400.0),
        ..display()
    };
    let wide = plan_with(&math, options);
    let narrow = plan_with(
        &math,
        MathLayoutOptions {
            max_width: Some(settings.display_content_box(0.0, 140.0).1),
            settings: MathSettings {
                wrap_indent_px: 10.0,
                ..settings
            },
            ..options
        },
    );
    assert_eq!(wide.lines.len(), 1);
    assert!(narrow.lines.len() > wide.lines.len());
}
