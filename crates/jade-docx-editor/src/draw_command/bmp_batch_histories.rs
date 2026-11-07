//! Compare batch-capable replay with a per-glyph contract after each operation.
use super::*;
use crate::draw_command::GlyphRunStep;
use jade_painter::recording::{Op, RecordingPainter};
use jade_painter::TextMetrics;
use proptest::prelude::*;
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug)]
enum BatchMode {
    Accept,
    Decline,
    Error,
}

type BatchCall = (String, Vec<f64>, Vec<u16>, usize);

struct BatchPainter {
    inner: RecordingPainter,
    mode: Cell<BatchMode>,
    calls: RefCell<Vec<BatchCall>>,
}

impl BatchPainter {
    fn new() -> Self {
        Self {
            inner: RecordingPainter::new(),
            mode: Cell::new(BatchMode::Accept),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn batch(
        &self,
        text: &str,
        xs: &[f64],
        indexes: &[u16],
        fonts: &[Rc<str>],
        y: f64,
    ) -> Result<bool, &'static str> {
        self.calls.borrow_mut().push((
            text.to_owned(),
            xs.to_vec(),
            indexes.to_vec(),
            text.as_ptr() as usize,
        ));
        match self.mode.get() {
            BatchMode::Decline => return Ok(false),
            BatchMode::Error => return Err("batch"),
            BatchMode::Accept => {}
        }
        assert_eq!(text.encode_utf16().count(), xs.len());
        assert!(text.chars().all(|ch| ch.len_utf16() == 1));
        assert!(indexes.is_empty() || indexes.len() == xs.len());
        for (index, (ch, x)) in text.chars().zip(xs).enumerate() {
            if !indexes.is_empty() {
                self.inner.set_font(&fonts[indexes[index] as usize]);
            }
            self.inner.fill_text(&ch.to_string(), *x, y).unwrap();
        }
        Ok(true)
    }
}

impl Painter for BatchPainter {
    type Error = &'static str;
    fn save(&self) {
        self.inner.save()
    }

    fn restore(&self) {
        self.inner.restore()
    }

    fn translate(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.inner.translate(x, y).map_err(|never| match never {})
    }

    fn scale(&self, x: f64, y: f64) -> Result<(), Self::Error> {
        self.inner.scale(x, y).map_err(|never| match never {})
    }

    fn rotate(&self, angle: f64) -> Result<(), Self::Error> {
        self.inner.rotate(angle).map_err(|never| match never {})
    }

    fn set_fill_style_str(&self, value: &str) {
        self.inner.set_fill_style_str(value)
    }

    fn set_stroke_style_str(&self, value: &str) {
        self.inner.set_stroke_style_str(value)
    }

    fn set_line_width(&self, width: f64) {
        self.inner.set_line_width(width)
    }

    fn set_global_alpha(&self, alpha: f64) {
        self.inner.set_global_alpha(alpha)
    }

    fn set_line_dash(&self, segments: &[f64]) {
        self.inner.set_line_dash(segments)
    }

    fn set_shadow_color(&self, value: &str) {
        self.inner.set_shadow_color(value)
    }

    fn set_shadow_blur(&self, value: f64) {
        self.inner.set_shadow_blur(value)
    }

    fn set_shadow_offset_x(&self, value: f64) {
        self.inner.set_shadow_offset_x(value)
    }

    fn set_shadow_offset_y(&self, value: f64) {
        self.inner.set_shadow_offset_y(value)
    }

    fn set_blur_filter(&self, radius_px: f64) {
        self.inner.set_blur_filter(radius_px)
    }

    fn set_font(&self, value: &str) {
        self.inner.set_font(value)
    }

    fn set_text_baseline(&self, value: &str) {
        self.inner.set_text_baseline(value)
    }

    fn fill_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.inner
            .fill_text(text, x, y)
            .map_err(|never| match never {})
    }

    fn stroke_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        self.inner
            .stroke_text(text, x, y)
            .map_err(|never| match never {})
    }

    fn measure_text_width(&self, text: &str) -> Result<f64, Self::Error> {
        self.inner
            .measure_text_width(text)
            .map_err(|never| match never {})
    }

    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, Self::Error> {
        self.inner
            .measure_text_metrics(text)
            .map_err(|never| match never {})
    }

    fn fill_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.inner.fill_rect(x, y, width, height)
    }

    fn clear_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.inner.clear_rect(x, y, width, height)
    }

    fn stroke_rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.inner.stroke_rect(x, y, width, height)
    }

    fn begin_path(&self) {
        self.inner.begin_path()
    }

    fn close_path(&self) {
        self.inner.close_path()
    }

    fn move_to(&self, x: f64, y: f64) {
        self.inner.move_to(x, y)
    }

    fn line_to(&self, x: f64, y: f64) {
        self.inner.line_to(x, y)
    }

    fn rect(&self, x: f64, y: f64, width: f64, height: f64) {
        self.inner.rect(x, y, width, height)
    }

    fn arc(
        &self,
        x: f64,
        y: f64,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.inner
            .arc(x, y, radius, start_angle, end_angle)
            .map_err(|never| match never {})
    }

    fn quadratic_curve_to(&self, cp_x: f64, cp_y: f64, x: f64, y: f64) {
        self.inner.quadratic_curve_to(cp_x, cp_y, x, y)
    }

    fn bezier_curve_to(&self, cp1_x: f64, cp1_y: f64, cp2_x: f64, cp2_y: f64, x: f64, y: f64) {
        self.inner.bezier_curve_to(cp1_x, cp1_y, cp2_x, cp2_y, x, y)
    }

    fn ellipse(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
    ) -> Result<(), Self::Error> {
        self.inner
            .ellipse(x, y, radius_x, radius_y, rotation, start_angle, end_angle)
            .map_err(|never| match never {})
    }

    fn ellipse_with_anticlockwise(
        &self,
        x: f64,
        y: f64,
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start_angle: f64,
        end_angle: f64,
        anticlockwise: bool,
    ) -> Result<(), Self::Error> {
        self.inner
            .ellipse_with_anticlockwise(
                x,
                y,
                radius_x,
                radius_y,
                rotation,
                start_angle,
                end_angle,
                anticlockwise,
            )
            .map_err(|never| match never {})
    }

    fn fill(&self) {
        self.inner.fill()
    }

    fn stroke(&self) {
        self.inner.stroke()
    }

    fn clip(&self) {
        self.inner.clip()
    }

    fn use_fill_style_for_stroke(&self) {
        self.inner.use_fill_style_for_stroke()
    }

    fn try_fill_glyph_run_batch(
        &self,
        text: &str,
        xs: &[f64],
        y: f64,
    ) -> Result<bool, Self::Error> {
        self.batch(text, xs, &[], &[], y)
    }
    fn try_fill_glyph_run_batch_multi_font(
        &self,
        text: &str,
        xs: &[f64],
        indexes: &[u16],
        fonts: &[Rc<str>],
        y: f64,
    ) -> Result<bool, Self::Error> {
        self.batch(text, xs, indexes, fonts, y)
    }
}

type DrawnGlyph = (String, String, u64, u64);

fn drawn_glyphs(painter: &RecordingPainter) -> Vec<DrawnGlyph> {
    let mut font = String::new();
    painter
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            Op::SetFont(value) => {
                font = value;
                None
            }
            Op::FillText(text, x, y) => Some((font.clone(), text, x.to_bits(), y.to_bits())),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug)]
enum ReplayOp {
    Draw {
        text: String,
        glyphs: Vec<GlyphRunStep>,
        mode: BatchMode,
        font_count: usize,
        x: f64,
        y: f64,
    },
    Invalidate,
    CloneTracker,
    ResetTracker,
}

fn replay_op() -> impl Strategy<Value = ReplayOp> {
    let draw = (
        proptest::collection::vec(
            prop_oneof![
                12 => proptest::sample::select(vec!['A', 'é', '税', '\0', '\u{0301}']),
                1 => Just('😀'),
            ],
            0..32,
        ),
        proptest::collection::vec((0u8..40, 0u8..40, 0u16..4, -300i16..300), 0..40),
        0u8..5,
        0u8..3,
        0usize..3,
        -300i16..300,
        -300i16..300,
    )
        .prop_map(
            |(characters, selections, range_mode, mode, font_count, x, y)| {
                let text: String = characters.iter().collect();
                let ranges: Vec<_> = text
                    .char_indices()
                    .map(|(start, ch)| (start as u32, (start + ch.len_utf8()) as u32))
                    .collect();
                let glyphs = match range_mode {
                    0 | 1 => ranges
                        .iter()
                        .enumerate()
                        .skip(if range_mode == 1 { ranges.len() / 3 } else { 0 })
                        .map(|(index, &(text_start, text_end))| GlyphRunStep {
                            text_start,
                            text_end,
                            font_index: if range_mode == 1 {
                                (index % 2) as u16
                            } else {
                                0
                            },
                            x_offset: index as f64 * 7.5,
                        })
                        .collect(),
                    2 => ranges
                        .iter()
                        .rev()
                        .enumerate()
                        .map(|(index, &(text_start, text_end))| GlyphRunStep {
                            text_start,
                            text_end,
                            font_index: (index % 2) as u16,
                            x_offset: index as f64 * -2.5,
                        })
                        .collect(),
                    _ => selections
                        .into_iter()
                        .map(|(start, end, font_index, x_offset)| {
                            let (text_start, text_end) = if range_mode == 3 {
                                ranges
                                    .get(start as usize % ranges.len().max(1))
                                    .copied()
                                    .unwrap_or((u32::MAX, u32::MAX))
                            } else {
                                (u32::from(start), u32::from(end))
                            };
                            GlyphRunStep {
                                text_start,
                                text_end,
                                font_index: if range_mode == 3 && font_count > 0 {
                                    font_index % font_count as u16
                                } else {
                                    font_index
                                },
                                x_offset: f64::from(x_offset),
                            }
                        })
                        .collect(),
                };
                ReplayOp::Draw {
                    text,
                    glyphs,
                    mode: match mode {
                        0 => BatchMode::Accept,
                        1 => BatchMode::Decline,
                        _ => BatchMode::Error,
                    },
                    font_count,
                    x: f64::from(x),
                    y: f64::from(y),
                }
            },
        );
    prop_oneof![7 => draw, 1 => Just(ReplayOp::Invalidate), 1 => Just(ReplayOp::CloneTracker), 1 => Just(ReplayOp::ResetTracker)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn batch_replay_matches_per_glyph_contract_after_every_operation(
        operations in proptest::collection::vec(replay_op(), 1..64),
    ) {
        let painter = BatchPainter::new();
        let mut tracker = FontStateTracker::new();
        // Reuse the same allocation while replacing its contents between draws.
        let mut source = String::with_capacity(256);
        let source_ptr = source.as_ptr();
        let available_fonts = [Rc::<str>::from("12pt Calibri"), Rc::from("11pt 'Noto Sans JP'")];
        let mut expected = Vec::new();
        for (step, operation) in operations.into_iter().enumerate() {
            match operation {
                ReplayOp::Invalidate => tracker.invalidate(),
                ReplayOp::CloneTracker => tracker = tracker.clone(),
                ReplayOp::ResetTracker => tracker = FontStateTracker::new(),
                ReplayOp::Draw { text, glyphs, mode, font_count, x, y } => {
                    source.clear();
                    source.push_str(&text);
                    prop_assert_eq!(source.as_ptr(), source_ptr);
                    painter.mode.set(mode);
                    let fonts = &available_fonts[..font_count];
                    // The contract uses UTF-16 unit counts, independently of
                    // the production byte-shape check and borrowing logic.
                    let valid: Vec<_> = glyphs.iter().map(|glyph| {
                        fonts.get(glyph.font_index as usize).zip(source.get(glyph.text_start as usize..glyph.text_end as usize))
                    }).collect();
                    let eligible = !glyphs.is_empty() && valid.iter().all(|value| value.is_some_and(|(_, text)| text.encode_utf16().count() == 1));
                    let should_error = eligible && matches!(mode, BatchMode::Error);
                    let calls_before = painter.calls.borrow().len();
                    let result = draw_text_run_glyphs(&painter, &source, x, y, &TextStyle::default(), fonts, &glyphs, &mut tracker);
                    prop_assert_eq!(result, if should_error { Err("batch") } else { Ok(()) }, "step {}", step);
                    let calls = painter.calls.borrow();
                    prop_assert_eq!(calls.len() - calls_before, usize::from(eligible), "step {}", step);
                    if eligible {
                        let (text, xs, indexes, _) = calls.last().unwrap();
                        let expected_text: String = valid.iter().map(|value| value.unwrap().1).collect();
                        let expected_xs: Vec<_> = glyphs.iter().map(|glyph| x + glyph.x_offset).collect();
                        let expected_indexes: Vec<_> = if glyphs.iter().all(|glyph| glyph.font_index == glyphs[0].font_index) { Vec::new() } else { glyphs.iter().map(|glyph| glyph.font_index).collect() };
                        prop_assert_eq!(text, &expected_text, "step {}", step);
                        prop_assert_eq!(xs, &expected_xs, "step {}", step);
                        prop_assert_eq!(indexes, &expected_indexes, "step {}", step);
                    }
                    if !should_error {
                        for (glyph, value) in glyphs.iter().zip(valid) {
                            if let Some((font, text)) = value {
                                expected.push((font.to_string(), text.to_owned(), (x + glyph.x_offset).to_bits(), y.to_bits()));
                            }
                        }
                    }
                }
            }
            prop_assert_eq!(drawn_glyphs(&painter.inner), expected.clone(), "step {}", step);
        }
    }
}

#[test]
fn batch_outcomes_preserve_the_next_draw_after_replacing_source_text() {
    let fonts = [
        Rc::<str>::from("12pt Calibri"),
        Rc::from("11pt 'Noto Sans JP'"),
    ];
    for mode in [BatchMode::Accept, BatchMode::Decline, BatchMode::Error] {
        for multi_font in [false, true] {
            let painter = BatchPainter::new();
            painter.mode.set(mode);
            let mut tracker = FontStateTracker::new();
            let mut source = String::with_capacity(64);
            source.push_str("A税");
            let source_ptr = source.as_ptr();
            let glyphs = [
                GlyphRunStep {
                    text_start: 0,
                    text_end: 1,
                    font_index: 0,
                    x_offset: 1.0,
                },
                GlyphRunStep {
                    text_start: 1,
                    text_end: 4,
                    font_index: u16::from(multi_font),
                    x_offset: 3.0,
                },
            ];
            let result = draw_text_run_glyphs(
                &painter,
                &source,
                10.0,
                20.0,
                &TextStyle::default(),
                &fonts,
                &glyphs,
                &mut tracker,
            );
            let mut expected = Vec::new();
            if matches!(mode, BatchMode::Error) {
                assert_eq!(result, Err("batch"));
            } else {
                result.unwrap();
                expected.extend([
                    (
                        fonts[0].to_string(),
                        "A".into(),
                        11.0f64.to_bits(),
                        20.0f64.to_bits(),
                    ),
                    (
                        fonts[usize::from(multi_font)].to_string(),
                        "税".into(),
                        13.0f64.to_bits(),
                        20.0f64.to_bits(),
                    ),
                ]);
            }
            assert_eq!(drawn_glyphs(&painter.inner), expected);
            source.clear();
            source.push('é');
            assert_eq!(source.as_ptr(), source_ptr);
            painter.mode.set(BatchMode::Accept);
            draw_text_run_glyphs(
                &painter,
                &source,
                30.0,
                40.0,
                &TextStyle::default(),
                &fonts,
                &[GlyphRunStep {
                    text_start: 0,
                    text_end: 2,
                    font_index: 0,
                    x_offset: -1.0,
                }],
                &mut tracker,
            )
            .unwrap();
            expected.push((
                fonts[0].to_string(),
                "é".into(),
                29.0f64.to_bits(),
                40.0f64.to_bits(),
            ));
            assert_eq!(
                drawn_glyphs(&painter.inner),
                expected,
                "{mode:?}, multi_font={multi_font}"
            );
        }
    }
}

#[test]
fn contiguous_spans_are_borrowed_and_other_orders_keep_the_exact_text() {
    let source = "😀Aé税終";
    let fonts = [
        Rc::<str>::from("12pt Calibri"),
        Rc::from("11pt 'Noto Sans JP'"),
    ];
    for (ranges, expected_text, borrowed) in [
        (vec![(4, 5), (5, 7), (7, 10)], "Aé税", true),
        (vec![(7, 10), (4, 5), (7, 10)], "税A税", false),
        (vec![(4, 5), (5, 7), (10, 13)], "Aé終", false),
        (vec![(4, 5), (4, 5), (5, 7)], "AAé", false),
    ] {
        for multi_font in [false, true] {
            let glyphs: Vec<_> = ranges
                .iter()
                .enumerate()
                .map(|(index, &(text_start, text_end))| GlyphRunStep {
                    text_start,
                    text_end,
                    font_index: if multi_font { (index % 2) as u16 } else { 0 },
                    x_offset: index as f64 * 3.0,
                })
                .collect();
            let painter = BatchPainter::new();
            draw_text_run_glyphs(
                &painter,
                source,
                -0.0,
                25.0,
                &TextStyle::default(),
                &fonts,
                &glyphs,
                &mut FontStateTracker::new(),
            )
            .unwrap();
            let calls = painter.calls.borrow();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, expected_text);
            if borrowed {
                assert_eq!(calls[0].3, source[4..10].as_ptr() as usize);
            }
        }
    }
}
