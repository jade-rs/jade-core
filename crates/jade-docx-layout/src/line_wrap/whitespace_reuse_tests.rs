use super::*;
use crate::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use std::cell::RefCell;

struct TraceContext {
    style: Rc<ResolvedRunStyle>,
    events: RefCell<Vec<String>>,
    fail_at: Option<usize>,
}

impl TraceContext {
    fn new(style: &ResolvedRunStyle, fail_at: Option<usize>) -> Self {
        Self {
            style: Rc::new(style.clone()),
            events: RefCell::new(Vec::new()),
            fail_at,
        }
    }

    fn event(&self, event: String) -> Result<(), usize> {
        let mut events = self.events.borrow_mut();
        events.push(event);
        if self.fail_at == Some(events.len()) {
            Err(events.len())
        } else {
            Ok(())
        }
    }

    fn width(text: &str) -> f64 {
        text.graphemes(true).count() as f64 * 6.25
    }
}

impl LineWrapContext for TraceContext {
    type Error = usize;

    fn resolve_run_style(
        &self,
        properties: Option<&RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, usize> {
        self.event(format!("resolve:{properties:?}"))?;
        Ok(Rc::clone(&self.style))
    }

    fn measure_text(&self, text: &str, descriptor: &str) -> Result<f64, usize> {
        self.event(format!("measure:{text:?}:{descriptor}"))?;
        Ok(Self::width(text))
    }

    fn measure_text_with_font_key(
        &self,
        text: &str,
        descriptor: &str,
        hash: u64,
        id: Option<u32>,
    ) -> Result<f64, usize> {
        self.event(format!("key:{text:?}:{descriptor}:{hash}:{id:?}"))?;
        Ok(Self::width(text))
    }

    fn measure_trailing_overhang(&self, text: &str, descriptor: &str) -> Result<f64, usize> {
        self.event(format!("overhang:{text:?}:{descriptor}"))?;
        Ok(0.125)
    }

    fn measure_line_box_height(&self, descriptor: &str) -> Result<Option<f64>, usize> {
        self.event(format!("line-box:{descriptor}"))?;
        Ok(Some(16.0))
    }

    fn measure_font_vertical_metrics(&self, descriptor: &str) -> Result<Option<(f64, f64)>, usize> {
        self.event(format!("vertical-metrics:{descriptor}"))?;
        Ok(Some((12.0, 4.0)))
    }
}

fn test_style(fixed: bool, hint: Option<FontHint>) -> ResolvedRunStyle {
    ResolvedRunStyle::new(
        "bold 10.50pt 'Times New Roman', serif",
        "bold 10.50pt 'Arial', sans-serif",
        if fixed {
            "bold 10.50pt 'MS Mincho', serif"
        } else {
            "bold 10.50pt 'MS PMincho', serif"
        },
        "bold 10.50pt 'Test Complex', serif",
        0.375,
        0.9,
        true,
        hint,
    )
    .with_font_descriptor_ids(11, 22, 33, 44)
}

// Include all line fields and the ordered host calls. The full baseline
// snapshots are retained with the profiling artifacts; the small fingerprints
// below make this a portable regression test without a large golden file.
fn wrap_snapshot(result: &Result<Vec<WrappedLine>, usize>, ctx: &TraceContext) -> String {
    let mut snapshot = format!("events={:?};", ctx.events.borrow());
    match result {
        Err(error) => snapshot.push_str(&format!("error={error}")),
        Ok(lines) => {
            for line in lines {
                snapshot.push_str(&format!(
                    "line={:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?};",
                    line.break_kind,
                    line.break_range,
                    line.break_properties,
                    line.leaders,
                    line.tab_stops,
                    line.clusters,
                    line.hyphen,
                ));
                for segment in &line.segments {
                    snapshot.push_str(&format!(
                        "segment={:?}/{}/{}/{:?}/{:?}/{:?}/{:016x};",
                        segment.text,
                        segment.start,
                        segment.end,
                        segment.properties,
                        segment.revision,
                        segment.hyperlink,
                        segment.leading_width.to_bits(),
                    ));
                }
            }
        }
    }
    snapshot
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn advance_snapshot(step: &ComputedTextAdvanceStep<'_>) -> String {
    let advance = &step.advance;
    let bits = [
        step.measured_glyph_width,
        advance.glyph_width,
        advance.adjusted_glyph_width,
        advance.implicit_latin_gap,
        advance.implicit_latin_trailing_gap,
        advance.implicit_numeric_gap,
        advance.kerning_adjustment,
        advance.snap_leading,
        advance.snap_trailing,
        advance.balance_extra,
        advance.core_advance,
        advance.glyph_box_width,
        advance.total_advance,
    ]
    .map(f64::to_bits);
    format!(
        "{:?}/{}/{}/{}/{}/{bits:?}/{:?}",
        step.script,
        step.font_descriptor,
        step.font_descriptor_hash,
        step.measured_width_is_oracle,
        step.monospace_cell,
        advance
            .trailing_auto_space
            .map(|space| (space.em_px.to_bits(), space.trailing_share.to_bits()))
    )
}

#[test]
fn known_whitespace_preserves_complete_advances_cache_calls_and_errors() {
    let mut texts: Vec<String> = (0..=0x10ffff)
        .filter_map(char::from_u32)
        .filter(|ch| ch.is_whitespace())
        .map(|ch| ch.to_string())
        .collect();
    texts.extend(
        [
            "",
            "A",
            "12",
            "日",
            "𠀀",
            "。",
            "ｱ",
            "\0",
            "\u{feff}",
            "e\u{301}",
            " \u{301}",
            "\u{3000}\u{200d}",
            "👩\u{200d}💻",
            " \t\u{3000}",
            "日 ",
            " \u{200b}",
            "\r\n",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    for global in [WordLayoutPlatform::Windows, WordLayoutPlatform::Mac] {
        set_word_layout_platform(global);
        for platform in [WordLayoutPlatform::Windows, WordLayoutPlatform::Mac] {
            for fixed in [false, true] {
                for hint in [
                    None,
                    Some(FontHint::Default),
                    Some(FontHint::EastAsia),
                    Some(FontHint::Complex),
                ] {
                    let original = test_style(fixed, hint);
                    // Warm a clone's metadata before public descriptor mutation.
                    let mut style = original.clone();
                    style.descriptor_key_for_grapheme("日", ScriptCategory::EastAsia);
                    style.font_descriptor_complex =
                        Rc::from("italic 12pt 'Changed Complex', serif");
                    for oracle in 0..5 {
                        let paragraph = ParagraphStyle {
                            word_platform: platform,
                            oracle_advance_px_by_text_offset: match oracle {
                                0 | 1 => None,
                                2 => Some(Rc::new(FxHashMap::default())),
                                3 => Some(Rc::new(FxHashMap::from_iter([(0, 0.0)]))),
                                _ => Some(Rc::new(FxHashMap::from_iter([(0, 37.0)]))),
                            },
                            ..ParagraphStyle::default()
                        };
                        let paragraph = (oracle != 0).then_some(&paragraph);
                        for (index, text) in texts.iter().enumerate() {
                            let script = classify_script(text);
                            let (descriptor, hash, id) =
                                style.descriptor_key_for_grapheme(text, script);
                            let state = TextAdvanceState {
                                prev_prev_grapheme: Some(CompactString::new("日")),
                                prev_grapheme: Some(CompactString::new("A")),
                                prev_adjusted_glyph_width: Some(6.25),
                                prev_raw_glyph_width: Some(6.25),
                                prev_font_descriptor_hash: Some(hash),
                                ..TextAdvanceState::default()
                            };
                            let mut lazy_cache = PairMeasureCache::default();
                            let mut known_cache = PairMeasureCache::default();
                            // Independent predicate, including empty and mixed strings.
                            let known = text.trim().is_empty();
                            for repetition in 0..3 {
                                let lazy = TraceContext::new(&style, None);
                                let cached = TraceContext::new(&style, None);
                                let compute =
                                    |ctx: &TraceContext,
                                     cache: &mut PairMeasureCache,
                                     whitespace| {
                                        compute_text_advance_step_with_descriptor(
                                            ctx,
                                            text,
                                            &style,
                                            paragraph,
                                            &state,
                                            Some("日"),
                                            index % 2 == 0,
                                            Some(14.0),
                                            cache,
                                            script,
                                            descriptor,
                                            hash,
                                            id,
                                            Some(0),
                                            whitespace,
                                        )
                                        .map(|step| advance_snapshot(&step))
                                    };
                                let expected = compute(&lazy, &mut lazy_cache, None);
                                let actual = compute(&cached, &mut known_cache, Some(known));
                                assert_eq!(actual, expected, "{text:?} repetition={repetition}");
                                assert_eq!(*cached.events.borrow(), *lazy.events.borrow());
                                assert_eq!(format!("{known_cache:?}"), format!("{lazy_cache:?}"));
                                if repetition == 0 {
                                    for fail_at in 1..=lazy.events.borrow().len() {
                                        let baseline = TraceContext::new(&style, Some(fail_at));
                                        let candidate = TraceContext::new(&style, Some(fail_at));
                                        assert_eq!(
                                            compute(
                                                &baseline,
                                                &mut PairMeasureCache::default(),
                                                None
                                            ),
                                            compute(
                                                &candidate,
                                                &mut PairMeasureCache::default(),
                                                Some(known)
                                            ),
                                            "{text:?} failure at {fail_at}"
                                        );
                                        assert_eq!(
                                            *baseline.events.borrow(),
                                            *candidate.events.borrow()
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    set_word_layout_platform(WordLayoutPlatform::Windows);
}

#[test]
fn wrapping_whitespace_preserves_baseline_output_calls_and_failures() {
    let text = "A \u{a0}日\u{3000}e\u{301}\t12。）\n👩\u{200d}💻\u{2003}ב ";
    // Split a combining sequence across runs as well as ordinary boundaries.
    let split = text.find('\u{301}').unwrap();
    let runs: Vec<_> = [(0, split), (split, text.len())]
        .into_iter()
        .map(|(start, end)| RunStyle {
            start,
            end,
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        })
        .collect();
    let mut snapshots = Vec::new();
    let mut injected_failures = 0;
    for global in [WordLayoutPlatform::Windows, WordLayoutPlatform::Mac] {
        set_word_layout_platform(global);
        for paragraph_platform in [WordLayoutPlatform::Windows, WordLayoutPlatform::Mac] {
            for fixed in [false, true] {
                for hint in [
                    None,
                    Some(FontHint::Default),
                    Some(FontHint::EastAsia),
                    Some(FontHint::Complex),
                ] {
                    let style = test_style(fixed, hint);
                    for oracle in 0..3 {
                        let paragraph = ParagraphStyle {
                            word_platform: paragraph_platform,
                            enable_line_break_rules: true,
                            oracle_advance_px_by_text_offset: match oracle {
                                0 => None,
                                1 => Some(Rc::new(FxHashMap::default())),
                                _ => Some(Rc::new(FxHashMap::from_iter([
                                    (0, 0.0),
                                    (1, 8.25),
                                    (text.find('日').unwrap(), 17.0),
                                ]))),
                            },
                            ..ParagraphStyle::default()
                        };
                        for edge in [18.0, 53.0] {
                            let ctx = TraceContext::new(&style, None);
                            let result =
                                wrap_text_into_lines(&ctx, text, &runs, 0.0, &paragraph, 0, edge);
                            let lines = result.as_ref().unwrap();
                            assert!(lines.len() > 1, "exercise line-break retries");
                            snapshots.push(wrap_snapshot(&result, &ctx));
                            // Inject at every operation of the successful path,
                            // retaining even errors that layout intentionally handles.
                            let events = ctx.events.borrow().clone();
                            for fail_at in 1..=events.len() {
                                let failing = TraceContext::new(&style, Some(fail_at));
                                let result = wrap_text_into_lines(
                                    &failing, text, &runs, 0.0, &paragraph, 0, edge,
                                );
                                assert_eq!(
                                    &failing.events.borrow()[..fail_at],
                                    &events[..fail_at],
                                    "event prefix before injected failure"
                                );
                                if let Err(error) = &result {
                                    assert_eq!(*error, fail_at);
                                }
                                snapshots.push(wrap_snapshot(&result, &failing));
                                injected_failures += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    set_word_layout_platform(WordLayoutPlatform::Windows);
    let joined = snapshots.join("\n");
    if let Ok(path) = std::env::var("JADE_WHITESPACE_SNAPSHOT_PATH") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.write_all(joined.as_bytes()).unwrap();
    }
    println!(
        "whitespace baseline fingerprint={:016x} snapshots={} injected_failures={}",
        fingerprint(joined.as_bytes()),
        snapshots.len(),
        injected_failures
    );
    assert_eq!(injected_failures, 10_326);
    assert_eq!(snapshots.len(), 10_518);
    assert_eq!(fingerprint(joined.as_bytes()), 0xc6e16963d0597212);
}
