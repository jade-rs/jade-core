use criterion::{black_box, criterion_group, criterion_main, Criterion};
use jade_docx_layout::font_resolver::ResolvedRunStyle;
use jade_docx_layout::layout_cache::{ContentHash, ParagraphLayoutCache, ParagraphLayoutResult};
use jade_docx_layout::line_wrap::{
    line_available_segment, plan_usable_line_segments_around_floats, wrap_text_into_lines,
    EastAsianLineBreakingRules, FloatExclusion, FloatPoint, FloatWrapPolygon, LineWrapContext,
    ParagraphStyle, RunStyle,
};
use jade_docx_layout::selection::{LineLayout, SelectionManager};
use jade_docx_reader::document::{RunProperties, WrapTextSide};
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

fn make_entry(i: usize) -> ParagraphLayoutResult {
    ParagraphLayoutResult::new(
        ContentHash::new(i as u64),
        i as f64 * 20.0,               // start_y
        (i as f64 + 1.0) * 20.0,       // end_y
        i / 50,                        // start_page
        i / 50,                        // end_page
        (i / 50) as f64 * 800.0,       // start_page_offset_y
        (i / 50) as f64 * 800.0,       // end_page_offset_y
        0,                             // start_column
        0,                             // end_column
        i as f64 * 20.0 - 5.0,         // start_last_line_baseline
        (i as f64 + 1.0) * 20.0 - 5.0, // end_last_line_baseline
    )
}

fn build_cache(n: usize) -> ParagraphLayoutCache {
    let mut cache = ParagraphLayoutCache::with_capacity(n);
    for i in 0..n {
        cache.insert(i, make_entry(i));
    }
    cache
}

fn bench_invalidate_from(c: &mut Criterion) {
    let mut group = c.benchmark_group("invalidate_from");

    for &size in &[100, 500, 1000, 5000] {
        group.bench_function(format!("n={size}_from_middle"), |b| {
            b.iter_batched(
                || build_cache(size),
                |mut cache| {
                    cache.invalidate_from(black_box(size / 2));
                    cache
                },
                criterion::BatchSize::SmallInput,
            );
        });

        group.bench_function(format!("n={size}_from_near_end"), |b| {
            b.iter_batched(
                || build_cache(size),
                |mut cache| {
                    cache.invalidate_from(black_box(size - 10));
                    cache
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn next_grapheme_at_absolute_offset(
    text: &str,
    base_offset: usize,
    grapheme_end_abs: usize,
) -> Option<&str> {
    let rel = grapheme_end_abs.saturating_sub(base_offset);
    text.get(rel..)?.graphemes(true).next()
}

fn bench_next_grapheme_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("next_grapheme");

    // Simulate a paragraph with mixed ASCII and Japanese text
    let text = "Hello, World! これは日本語のテキストです。This is a mixed text paragraph with some content.";
    let base_offset = 0;

    group.bench_function("absolute_offset_per_grapheme", |b| {
        b.iter(|| {
            let mut count = 0u64;
            for (_, grapheme) in text.grapheme_indices(true) {
                let end = base_offset + grapheme.as_ptr() as usize - text.as_ptr() as usize
                    + grapheme.len();
                let next = next_grapheme_at_absolute_offset(black_box(text), base_offset, end);
                if next.is_some() {
                    count += 1;
                }
            }
            count
        });
    });

    group.bench_function("peekable_iterator", |b| {
        b.iter(|| {
            let mut count = 0u64;
            let mut iter = text.grapheme_indices(true).peekable();
            while let Some((_, grapheme)) = iter.next() {
                let next = match iter.peek() {
                    Some(&(_, next_g)) => Some(next_g),
                    None => {
                        let end = base_offset + grapheme.as_ptr() as usize - text.as_ptr() as usize
                            + grapheme.len();
                        next_grapheme_at_absolute_offset(black_box(text), base_offset, end)
                    }
                };
                if next.is_some() {
                    count += 1;
                }
            }
            count
        });
    });

    // Larger text test
    let large_text: String = "日本語のテストテキスト。".repeat(100);
    group.bench_function("large_text_absolute_offset", |b| {
        b.iter(|| {
            let mut count = 0u64;
            for (_, grapheme) in large_text.grapheme_indices(true) {
                let end =
                    grapheme.as_ptr() as usize - large_text.as_ptr() as usize + grapheme.len();
                let next = next_grapheme_at_absolute_offset(black_box(&large_text), 0, end);
                if next.is_some() {
                    count += 1;
                }
            }
            count
        });
    });

    group.bench_function("large_text_peekable", |b| {
        b.iter(|| {
            let mut count = 0u64;
            let mut iter = large_text.grapheme_indices(true).peekable();
            while let Some((_, grapheme)) = iter.next() {
                let next = match iter.peek() {
                    Some(&(_, next_g)) => Some(next_g),
                    None => {
                        let end = grapheme.as_ptr() as usize - large_text.as_ptr() as usize
                            + grapheme.len();
                        next_grapheme_at_absolute_offset(black_box(&large_text), 0, end)
                    }
                };
                if next.is_some() {
                    count += 1;
                }
            }
            count
        });
    });

    group.finish();
}

struct WrapBenchContext {
    style: Rc<ResolvedRunStyle>,
}

impl WrapBenchContext {
    fn new() -> Self {
        Self {
            style: Rc::new(ResolvedRunStyle::new(
                Rc::<str>::from("12pt Bench"),
                Rc::<str>::from("12pt Bench"),
                Rc::<str>::from("12pt Bench"),
                Rc::<str>::from("12pt Bench"),
                0.15,
                1.0,
                true,
                None,
            )),
        }
    }
}

impl LineWrapContext for WrapBenchContext {
    type Error = ();

    fn resolve_run_style(
        &self,
        _properties: Option<&RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        Ok(Rc::clone(&self.style))
    }

    fn measure_text(&self, text: &str, _font_descriptor: &str) -> Result<f64, Self::Error> {
        Ok(text
            .graphemes(true)
            .map(|grapheme| {
                if grapheme == "AV" {
                    10.0
                } else if grapheme.is_ascii() && !grapheme.trim().is_empty() {
                    6.0
                } else {
                    12.0
                }
            })
            .sum())
    }
}

fn bench_tab_segment_wrapping(c: &mut Criterion) {
    let mut group = c.benchmark_group("tab_segment_wrapping");
    let ctx = WrapBenchContext::new();
    let body = "AVAVAVAV日本語123,456終わり".repeat(4);
    let text = format!("\t{body}");
    let runs = vec![RunStyle {
        start: 0,
        end: text.len(),
        properties: None,
        revision: None,
        hyperlink: None,
        inline_object: None,
    }];
    let paragraph = ParagraphStyle {
        enable_line_break_rules: true,
        east_asian_rules: EastAsianLineBreakingRules {
            auto_space_de: Some(true),
            auto_space_dn: Some(true),
            ..EastAsianLineBreakingRules::default()
        },
        ..ParagraphStyle::default()
    };

    group.bench_function("wrap_single_tabbed_paragraph", |b| {
        b.iter(|| {
            let lines = wrap_text_into_lines(
                black_box(&ctx),
                black_box(&text),
                black_box(&runs),
                black_box(0.0),
                black_box(&paragraph),
                black_box(0),
                black_box(900.0),
            )
            .expect("tabbed paragraph wrap");
            black_box(lines);
        });
    });

    group.finish();
}

fn wrap_paragraph_bench(ctx: &WrapBenchContext, text: &str, kinsoku: bool) {
    let runs = vec![RunStyle {
        start: 0,
        end: text.len(),
        properties: None,
        revision: None,
        hyperlink: None,
        inline_object: None,
    }];
    let paragraph = ParagraphStyle {
        enable_line_break_rules: kinsoku,
        east_asian_rules: EastAsianLineBreakingRules {
            use_east_asian_layout: kinsoku,
            auto_space_de: Some(true),
            auto_space_dn: Some(true),
            ..EastAsianLineBreakingRules::default()
        },
        ..ParagraphStyle::default()
    };
    let lines = wrap_text_into_lines(
        black_box(ctx),
        black_box(text),
        black_box(&runs),
        black_box(0.0),
        black_box(&paragraph),
        black_box(0),
        black_box(480.0),
    )
    .expect("paragraph wrap");
    black_box(lines);
}

fn bench_large_paragraph_wrapping(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_paragraph_wrapping");
    let ctx = WrapBenchContext::new();

    // ~1000-grapheme dense Japanese paragraph (kinsoku punctuation heavy).
    let cjk = "本契約は、甲と乙との間の権利義務を定めるものである。（第１条）当事者は、誠実にこれを履行しなければならない。".repeat(10);
    group.bench_function("cjk_kinsoku", |b| {
        b.iter(|| wrap_paragraph_bench(&ctx, &cjk, true));
    });

    // Mixed Japanese + Latin + digits paragraph.
    let mixed = "Word互換のlayout engineは、ECMA-376 Part1 §17.3に基づき、CJK 123文字とLatin abcを混在させる。".repeat(10);
    group.bench_function("mixed_kinsoku", |b| {
        b.iter(|| wrap_paragraph_bench(&ctx, &mixed, true));
    });

    // Same mixed text without kinsoku rules (latin-heavy fast path).
    let latin =
        "The quick brown fox jumps over the lazy dog, and then runs away quickly. ".repeat(20);
    group.bench_function("latin_no_kinsoku", |b| {
        b.iter(|| wrap_paragraph_bench(&ctx, &latin, false));
    });

    group.finish();
}

fn bench_float_line_segment(c: &mut Criterion) {
    let mut group = c.benchmark_group("float_line_segment");
    let floats: Vec<_> = (0..4)
        .map(|index| FloatExclusion {
            top: 0.0,
            bottom: 100.0,
            left: 150.0 + index as f64 * 80.0,
            right: 180.0 + index as f64 * 80.0,
            wrap_text: WrapTextSide::BothSides,
            polygon: None,
        })
        .collect();
    for count in [1, 4] {
        group.bench_function(format!("rectangles_{count}"), |b| {
            b.iter(|| {
                black_box(line_available_segment(
                    black_box(10.0),
                    black_box(30.0),
                    black_box(100.0),
                    black_box(500.0),
                    black_box(&floats[..count]),
                ))
            });
        });
    }
    let through = FloatExclusion {
        top: 0.0,
        bottom: 100.0,
        left: 200.0,
        right: 400.0,
        wrap_text: WrapTextSide::BothSides,
        polygon: Some(FloatWrapPolygon {
            points: vec![
                FloatPoint { x: 200.0, y: 0.0 },
                FloatPoint { x: 400.0, y: 0.0 },
                FloatPoint { x: 400.0, y: 100.0 },
                FloatPoint { x: 370.0, y: 100.0 },
                FloatPoint { x: 370.0, y: 30.0 },
                FloatPoint { x: 230.0, y: 30.0 },
                FloatPoint { x: 230.0, y: 100.0 },
                FloatPoint { x: 200.0, y: 100.0 },
            ]
            .into(),
            distance_top: 0.0,
            distance_bottom: 0.0,
            distance_left: 0.0,
            distance_right: 0.0,
            allow_internal_gaps: true,
        }),
    };
    group.bench_function("through_polygon", |b| {
        b.iter(|| {
            black_box(line_available_segment(
                black_box(20.0),
                black_box(40.0),
                black_box(100.0),
                black_box(500.0),
                black_box(std::slice::from_ref(&through)),
            ))
        });
    });
    group.bench_function("plan_usable_64_lines", |b| {
        b.iter(|| {
            black_box(plan_usable_line_segments_around_floats(
                black_box(0.0),
                black_box(20.0),
                black_box(64),
                black_box(100.0),
                black_box(500.0),
                black_box(&floats[..1]),
            ))
        });
    });
    group.finish();
}

fn build_selection_manager(n: usize) -> SelectionManager {
    let mut manager = SelectionManager::new();
    manager.begin_render();
    let line_height = 18.0;
    for i in 0..n {
        manager.ensure_paragraph(i);
        manager.append_paragraph_text(i, "Hello, World! これは段落のサンプルテキストです。");
        let top = i as f64 * line_height;
        manager.record_line(
            i,
            LineLayout {
                column_index: 0,
                top,
                absolute_top: top,
                height: line_height,
                baseline: top + 14.0,
                start_x: 96.0,
                cursor_end_x: 480.0,
                allowed_trailing_overflow_width: 0.0,
                content_top: top,
                absolute_content_top: top,
                content_height: line_height,
                fragments: Rc::from([]),
                break_range: Some((0, 12)),
                tab_leaders: Rc::from([]),
                tab_stops: Rc::from([]),
                vertical: false,
                vertical_reversed: false,
            },
        );
    }
    manager
}

fn bench_hit_test(c: &mut Criterion) {
    let mut group = c.benchmark_group("hit_test");

    for &size in &[100usize, 1000, 5000] {
        let manager = build_selection_manager(size);
        let max_y = size as f64 * 18.0;
        group.bench_function(format!("n={size}_scan"), |b| {
            b.iter(|| {
                // Sweep several vertical positions to exercise the candidate
                // search (collect + sort of paragraph bounds) at different depths.
                let mut hits = 0u64;
                for step in 0..16u64 {
                    let y = max_y * (step as f64) / 16.0;
                    if manager.hit_test(black_box(240.0), black_box(y)).is_some() {
                        hits += 1;
                    }
                }
                hits
            });
        });
    }

    group.finish();
}

fn formatted_props() -> RunProperties {
    RunProperties {
        bold: Some(true),
        font_size: Some(10.5),
        color: Some("#1A1A1A".into()),
        font_family: Some("Yu Mincho".into()),
        ..RunProperties::default()
    }
}

// Measures the run-style cost of laying out a *formatted* paragraph and of
// cloning the resulting lines (what selection's `line.clone()` does). Uses
// `.into()` for the properties field so the same source compiles whether that
// field is `Option<RunProperties>` (baseline) or `Option<Rc<RunProperties>>`.
fn bench_formatted_segments(c: &mut Criterion) {
    let mut group = c.benchmark_group("formatted_segments");
    let ctx = WrapBenchContext::new();
    let text = "本契約は、甲と乙との間の権利義務を定める。The quick brown fox 123。".repeat(8);
    let runs = vec![RunStyle {
        start: 0,
        end: text.len(),
        properties: Some(formatted_props().into()),
        revision: None,
        hyperlink: None,
        inline_object: None,
    }];
    let paragraph = ParagraphStyle::default();
    let lines = wrap_text_into_lines(&ctx, &text, &runs, 0.0, &paragraph, 0, 480.0).expect("wrap");

    group.bench_function("wrap_formatted_paragraph", |b| {
        b.iter(|| {
            let l = wrap_text_into_lines(
                black_box(&ctx),
                black_box(&text),
                black_box(&runs),
                0.0,
                black_box(&paragraph),
                0,
                480.0,
            )
            .expect("wrap");
            black_box(l);
        });
    });
    group.bench_function("clone_wrapped_lines", |b| {
        b.iter(|| black_box(black_box(&lines).clone()));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_invalidate_from,
    bench_next_grapheme_lookup,
    bench_tab_segment_wrapping,
    bench_large_paragraph_wrapping,
    bench_float_line_segment,
    bench_hit_test,
    bench_formatted_segments
);
criterion_main!(benches);
