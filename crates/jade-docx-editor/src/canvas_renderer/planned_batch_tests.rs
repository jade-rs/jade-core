//! Compare live batches against a scalar-based, always-owned text contract.
use super::recording_host::RecordingHost;
use super::*;
use crate::draw_command::executor::bmp_batch_histories::{BatchMode, BatchPainter};
use jade_painter::recording::RecordingPainter;
use proptest::prelude::*;

fn renderer() -> CanvasRenderer<RecordingPainter> {
    CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: RecordingPainter::new(),
        }),
        true,
    )
    .unwrap()
}

fn style(alias: bool) -> ResolvedRunStyle {
    ResolvedRunStyle::new(
        Rc::from("12pt Latin"),
        Rc::from(if alias { "12pt Latin" } else { "12pt HAnsi" }),
        Rc::from("11pt East"),
        Rc::from("10pt Complex"),
        0.0,
        1.0,
        false,
        None,
    )
}

// Font state is modeled independently, including the existing rule that a
// rejected or failed multi-font batch does not invalidate the tracked font.
fn reference_batch(
    painter: &BatchPainter,
    tracked_font: &mut Option<String>,
    source: &str,
    steps: &[PlannedGlyphLayoutStep],
    style: &ResolvedRunStyle,
) -> Result<bool, &'static str> {
    if steps.is_empty() {
        return Ok(false);
    }
    let mut text = String::new();
    let mut fonts: Vec<Rc<str>> = Vec::new();
    let mut indexes = Vec::new();
    let mut xs = Vec::new();
    for step in steps {
        let mut chars = source[step.text_start as usize..step.text_end as usize].chars();
        let Some(ch) = chars.next() else {
            return Ok(false);
        };
        if ch.len_utf16() != 1 || chars.next().is_some() {
            return Ok(false);
        }
        text.push(ch);
        let descriptor = style.descriptor_for_slot(step.font_slot);
        let index = match fonts
            .iter()
            .position(|font| font.as_ref() == descriptor.as_ref())
        {
            Some(index) => index,
            None => {
                fonts.push(descriptor.clone());
                fonts.len() - 1
            }
        };
        indexes.push(index as u16);
        xs.push(7.25 + step.x_offset);
    }
    if fonts.len() == 1 {
        if tracked_font.as_deref() != Some(fonts[0].as_ref()) {
            painter.set_font(&fonts[0]);
            *tracked_font = Some(fonts[0].to_string());
        }
        painter.try_fill_glyph_run_batch(&text, &xs, 25.0)
    } else {
        let accepted =
            painter.try_fill_glyph_run_batch_multi_font(&text, &xs, &indexes, &fonts, 25.0)?;
        if accepted {
            *tracked_font = None;
        }
        Ok(accepted)
    }
}

fn compare_step(
    renderer: &CanvasRenderer<RecordingPainter>,
    actual: &BatchPainter,
    reference: &BatchPainter,
    tracked_font: &mut Option<String>,
    source: &str,
    steps: &[PlannedGlyphLayoutStep],
    style: &ResolvedRunStyle,
) {
    actual.inner.clear();
    reference.inner.clear();
    actual.calls.borrow_mut().clear();
    reference.calls.borrow_mut().clear();
    let expected = reference_batch(reference, tracked_font, source, steps, style);
    let result = renderer.try_draw_planned_text_batch(actual, 7.25, 25.0, source, style, steps);
    assert_eq!(result, expected);
    assert_eq!(actual.inner.ops(), reference.inner.ops());
    let actual_calls = actual.calls.borrow();
    let expected_calls = reference.calls.borrow();
    assert_eq!(actual_calls.len(), expected_calls.len());
    for (actual, expected) in actual_calls.iter().zip(expected_calls.iter()) {
        assert_eq!(
            (&actual.0, &actual.1, &actual.2),
            (&expected.0, &expected.1, &expected.2)
        );
    }
    assert_eq!(
        renderer.current_font.borrow().as_deref(),
        tracked_font.as_deref()
    );
}

#[test]
fn planned_batches_preserve_spans_orders_and_outcomes_with_reused_scratch() {
    let renderer = renderer();
    let actual = BatchPainter::new();
    let reference = BatchPainter::new();
    let mut tracked_font = None;
    let source = "😀Aé税終";
    for alias in [false, true] {
        for multi_font in [false, true] {
            for mode in [BatchMode::Accept, BatchMode::Decline, BatchMode::Error] {
                actual.mode.set(mode);
                reference.mode.set(mode);
                for (ranges, expected_text, borrowed) in [
                    (vec![(7, 10), (4, 5), (7, 10)], Some("税A税"), false),
                    (vec![(4, 5), (5, 7), (7, 10)], Some("Aé税"), true),
                    (vec![(4, 5), (5, 7), (10, 13)], Some("Aé終"), false),
                    (vec![(4, 5), (4, 5), (5, 7)], Some("AAé"), false),
                    (vec![(4, 5), (0, 4)], None, false),
                    (vec![(4, 5), (5, 5)], None, false),
                    (vec![(4, 7)], None, false),
                    (vec![], None, false),
                    (vec![(7, 10), (10, 13)], Some("税終"), true),
                ] {
                    let steps: Vec<_> = ranges
                        .iter()
                        .enumerate()
                        .map(|(i, &(start, end))| PlannedGlyphLayoutStep {
                            text_start: start,
                            text_end: end,
                            font_slot: if multi_font && i % 2 == 1 {
                                ResolvedFontSlot::LatinHansi
                            } else {
                                ResolvedFontSlot::LatinAscii
                            },
                            x_offset: i as f64 * -3.5,
                        })
                        .collect();
                    compare_step(
                        &renderer,
                        &actual,
                        &reference,
                        &mut tracked_font,
                        source,
                        &steps,
                        &style(alias),
                    );
                    let calls = actual.calls.borrow();
                    assert_eq!(calls.first().map(|call| call.0.as_str()), expected_text);
                    if borrowed {
                        assert_eq!(calls[0].3, source[ranges[0].0 as usize..].as_ptr() as usize);
                        assert!(renderer.text_batch_scratch.borrow().text.is_empty());
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct BatchOp {
    source: u8,
    glyphs: Vec<(u8, u8, i16)>,
    mode: u8,
    alias: bool,
    valid: bool,
}

fn operation() -> impl Strategy<Value = BatchOp> {
    (
        any::<u8>(),
        prop::collection::vec((any::<u8>(), any::<u8>(), -128i16..128), 0..80),
        0u8..3,
        any::<bool>(),
        prop_oneof![3 => Just(true), 1 => Just(false)],
    )
        .prop_map(|(source, glyphs, mode, alias, valid)| BatchOp {
            source,
            glyphs,
            mode,
            alias,
            valid,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn planned_batch_histories_match_the_owned_scalar_contract(ops in prop::collection::vec(operation(), 1..17)) {
        let renderer = renderer();
        let actual = BatchPainter::new(); let reference = BatchPainter::new();
        let mut tracked_font = None;
        for (step_index, op) in ops.iter().enumerate() {
            let source = match op.source % 3 { 0 => "Aé税終", 1 => "xyz", _ => "😀e\u{301}税" };
            let mut ranges: Vec<_> = source.char_indices().filter(|(_,ch)| !op.valid || ch.len_utf16() == 1)
                .map(|(start,ch)| (start as u32,(start+ch.len_utf8()) as u32)).collect();
            if !op.valid { ranges.extend([(0,0),(0,source.len() as u32)]); }
            let steps: Vec<_> = op.glyphs.iter().map(|&(range,slot,x)| {
                let (start,end) = ranges[range as usize % ranges.len()];
                PlannedGlyphLayoutStep { text_start: start,text_end: end,x_offset: f64::from(x)/8.0,
                    font_slot: match slot % 4 { 0 => ResolvedFontSlot::LatinAscii, 1 => ResolvedFontSlot::LatinHansi, 2 => ResolvedFontSlot::EastAsia, _ => ResolvedFontSlot::Complex } }
            }).collect();
            let mode = match op.mode { 0 => BatchMode::Accept, 1 => BatchMode::Decline, _ => BatchMode::Error };
            actual.mode.set(mode); reference.mode.set(mode);
            // Include the shrinking-friendly operation and index in a failure.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                compare_step(&renderer,&actual,&reference,&mut tracked_font,source,&steps,&style(op.alias));
            }));
            prop_assert!(result.is_ok(), "step {step_index}: {op:?}");
        }
    }
}
