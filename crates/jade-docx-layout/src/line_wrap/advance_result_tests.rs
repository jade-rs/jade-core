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

#[test]
fn wrapping_preserves_baseline_output_calls_and_failures() {
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
    if let Ok(path) = std::env::var("JADE_ADVANCE_WRAP_SNAPSHOT_PATH") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.write_all(joined.as_bytes()).unwrap();
    }
    println!(
        "wrap baseline fingerprint={:016x} snapshots={} injected_failures={}",
        fingerprint(joined.as_bytes()),
        snapshots.len(),
        injected_failures
    );
    assert_eq!(injected_failures, 10_326);
    assert_eq!(snapshots.len(), 10_518);
    assert_eq!(fingerprint(joined.as_bytes()), 0xc6e16963d0597212);
}

fn advance_event(
    offset: usize,
    grapheme: &str,
    script: ScriptCategory,
    descriptor: &str,
    hash: u64,
    measured: f64,
    advance: &GraphemeAdvance,
) -> String {
    let widths = [
        measured,
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
        "visit:{offset}:{grapheme:?}:{script:?}:{descriptor}:{hash}:{widths:?}:{:?}",
        advance.trailing_auto_space
    )
}

fn walk_snapshot(
    style: &ResolvedRunStyle,
    text: &str,
    paragraph: Option<&ParagraphStyle>,
    initial: &TextAdvanceState,
    offset: Option<usize>,
    owned: bool,
    fail_at: Option<usize>,
) -> (String, Vec<String>) {
    let ctx = TraceContext::new(style, fail_at);
    let mut cache = FxHashMap::default();
    // An unrelated entry must survive both success and early errors.
    cache.insert(
        SnapToCharsCacheKey {
            east_asia_font_descriptor_hash: u64::MAX,
            text_scale_x_bits: 1.0_f64.to_bits(),
            char_space_px_bits: 0,
        },
        -0.0,
    );
    let result = if owned {
        assert!(offset.is_none());
        visit_owned_text_advance_steps(
            &ctx,
            text,
            style,
            paragraph,
            initial,
            Some("。"),
            &mut cache,
            |step| {
                let (descriptor, hash, _) =
                    style.descriptor_key_for_grapheme(&step.grapheme, step.script);
                assert!(std::ptr::eq(step.font_descriptor.as_ref(), descriptor));
                assert_eq!(step.font_descriptor_hash, hash);
                ctx.event(advance_event(
                    step.rel_offset,
                    &step.grapheme,
                    step.script,
                    &step.font_descriptor,
                    step.font_descriptor_hash,
                    step.measured_glyph_width,
                    &step.advance,
                ))
            },
        )
    } else {
        walk_text_advance_steps_at_offset(
            &ctx,
            text,
            style,
            paragraph,
            initial,
            Some("。"),
            offset,
            &mut cache,
            |step| {
                let (descriptor, hash, _) =
                    style.descriptor_key_for_grapheme(step.grapheme, step.script);
                assert!(std::ptr::eq(step.font_descriptor, descriptor));
                assert_eq!(step.font_descriptor_hash, hash);
                ctx.event(advance_event(
                    step.rel_offset,
                    step.grapheme,
                    step.script,
                    step.font_descriptor,
                    step.font_descriptor_hash,
                    step.measured_glyph_width,
                    &step.advance,
                ))
            },
        )
    };
    let mut entries: Vec<_> = cache
        .iter()
        .map(|(key, width)| {
            (
                key.east_asia_font_descriptor_hash,
                key.text_scale_x_bits,
                key.char_space_px_bits,
                width.to_bits(),
            )
        })
        .collect();
    entries.sort_unstable();
    let events = ctx.events.into_inner();
    (
        format!("{result:?};cache={entries:?};events={events:?}"),
        events,
    )
}

#[test]
fn walking_preserves_baseline_metadata_calls_state_and_errors() {
    let texts = [
        "",
        "AV é日𠀀e\u{301}👩\u{200d}💻12\t",
        "AVAV \0bé日𠀀e\u{301}👩\u{200d}💻12\t",
    ];
    let mut snapshots = Vec::new();
    let mut failures = 0;
    let mut successes = 0;
    for global in [WordLayoutPlatform::Windows, WordLayoutPlatform::Mac] {
        set_word_layout_platform(global);
        for paragraph_kind in 0..3 {
            for fixed in [false, true] {
                for hint in [
                    None,
                    Some(FontHint::Default),
                    Some(FontHint::EastAsia),
                    Some(FontHint::Complex),
                ] {
                    for (variant, text) in texts.into_iter().enumerate() {
                        let mut style = test_style(fixed, hint.clone());
                        if variant == 2 {
                            style.ascii_metrics_for_descriptor(&style.font_descriptor_latin_ascii);
                            style.monospace_em_px(&style.font_descriptor_east_asia);
                            style.windows_cjk_bold_extra_px(&style.font_descriptor_east_asia);
                            // Retain a warmed clone before replacing public descriptors.
                            style = style.clone();
                            style.font_descriptor_latin_ascii =
                                Rc::from("italic 11pt 'Arial', sans-serif");
                            style.font_descriptor_complex =
                                Rc::from("11pt 'Replacement Complex', serif");
                        }
                        let paragraph = (paragraph_kind != 0).then(|| ParagraphStyle {
                            word_platform: if paragraph_kind == 1 {
                                WordLayoutPlatform::Windows
                            } else {
                                WordLayoutPlatform::Mac
                            },
                            doc_grid_snap_to_chars: variant == 2,
                            doc_grid_char_space_px: Some(0.25),
                            oracle_advance_px_by_text_offset: match variant {
                                0 => None,
                                1 => Some(Rc::new(FxHashMap::default())),
                                _ => Some(Rc::new(FxHashMap::from_iter([
                                    (0, -0.0),
                                    (19, 17.25),
                                    (20, 0.0),
                                ]))),
                            },
                            ..ParagraphStyle::default()
                        });
                        let initial = if variant == 2 {
                            TextAdvanceState {
                                prev_prev_grapheme: Some(CompactString::new("日")),
                                prev_grapheme: Some(CompactString::new("1")),
                                prev_adjusted_glyph_width: Some(6.0),
                                prev_raw_glyph_width: Some(7.0),
                                prev_font_descriptor_hash: Some(
                                    style.font_descriptor_latin_ascii_hash,
                                ),
                                prev_auto_space: digit_auto_space(
                                    "1",
                                    &style.font_descriptor_latin_ascii,
                                ),
                                follows_inline_object: true,
                            }
                        } else {
                            TextAdvanceState::default()
                        };
                        for offset in [None, Some(0), Some(19)] {
                            let (snapshot, events) = walk_snapshot(
                                &style,
                                text,
                                paragraph.as_ref(),
                                &initial,
                                offset,
                                false,
                                None,
                            );
                            snapshots.push(snapshot);
                            successes += 1;
                            if offset.is_none() {
                                assert_eq!(
                                    walk_snapshot(
                                        &style,
                                        text,
                                        paragraph.as_ref(),
                                        &initial,
                                        offset,
                                        true,
                                        None
                                    )
                                    .0,
                                    snapshots.last().unwrap().as_str()
                                );
                            }
                            for fail_at in 1..=events.len() {
                                let (snapshot, actual_events) = walk_snapshot(
                                    &style,
                                    text,
                                    paragraph.as_ref(),
                                    &initial,
                                    offset,
                                    false,
                                    Some(fail_at),
                                );
                                assert_eq!(&actual_events[..fail_at], &events[..fail_at]);
                                if offset.is_none() {
                                    assert_eq!(
                                        walk_snapshot(
                                            &style,
                                            text,
                                            paragraph.as_ref(),
                                            &initial,
                                            offset,
                                            true,
                                            Some(fail_at)
                                        )
                                        .0,
                                        snapshot
                                    );
                                }
                                snapshots.push(snapshot);
                                failures += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    set_word_layout_platform(WordLayoutPlatform::Windows);
    let joined = snapshots.join("\n");
    if let Ok(path) = std::env::var("JADE_ADVANCE_WALK_SNAPSHOT_PATH") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.write_all(joined.as_bytes()).unwrap();
    }
    println!("advance baseline fingerprint={:016x} successes={successes} failures={failures} snapshots={}", fingerprint(joined.as_bytes()), snapshots.len());
    assert_eq!(successes, 432);
    assert_eq!(failures, 8_892);
    assert_eq!(snapshots.len(), 9_324);
    assert_eq!(fingerprint(joined.as_bytes()), 0x5fdb642092989117);
}
