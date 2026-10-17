//! Reproduces the "large document, later pages, many accumulated edits" workload
//! that drove the worker to ~96% CPU (see PR #677 / #680), so we can measure it
//! in pure Rust without a browser/canvas.
//!
//! ## What it models
//!
//! The real renderer keeps a content-addressed, LRU **paragraph wrap cache**. As
//! you scroll/edit a large document, windows of paragraphs are re-rendered; each
//! render re-wraps a paragraph only on a wrap-cache miss, and each wrap drives a
//! burst of `TextMeasureCache` lookups (the trace's hottest symbol,
//! `find_index_in_bucket`). When the wrap cache is smaller than the working set,
//! scrolling evicts paragraphs and revisits re-wrap them — the slowdown.
//!
//! The `CanvasRenderer` wrap cache itself is private to the `editor` crate and
//! needs a canvas, which can't run natively. So this bench models the cache
//! (content-addressed + LRU + capacity) here, and uses the *real*
//! `jade_docx_layout::line_wrap::wrap_text_into_lines` + `TextMeasureCache` for the
//! actual wrap/measure cost. Comparing `cap=512` (pre-fix) vs `cap=working_set`
//! (post-fix, document-sized) shows the cost the wrap-cache sizing avoids.
//!
//! Run with:
//! ```text
//! cargo bench -p jade-docx-layout --bench scroll_edit_wrap_bench
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use jade_docx_layout::font_resolver::ResolvedRunStyle;
use jade_docx_layout::layout_cache::{TextMeasureCache, TextMeasureResult};
use jade_docx_layout::line_wrap::{
    wrap_text_into_lines, EastAsianLineBreakingRules, LineWrapContext, ParagraphStyle, RunStyle,
};
use jade_docx_reader::document::RunProperties;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

const PAGE_WIDTH_PX: f64 = 480.0;

/// Wrap context that routes text measurement through a `TextMeasureCache`, just
/// like the production `CanvasRenderer::measure_text_cached`, so the bench
/// exercises the same measure-cache lookup path that dominated the trace.
struct MeasuringWrapContext {
    style: Rc<ResolvedRunStyle>,
    measure_cache: RefCell<TextMeasureCache>,
}

impl MeasuringWrapContext {
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
            // Generously sized so measurement stays a cache hit and we isolate the
            // wrap-recomputation cost (the thing the wrap cache fix avoids).
            measure_cache: RefCell::new(TextMeasureCache::new(16384)),
        }
    }
}

impl LineWrapContext for MeasuringWrapContext {
    type Error = ();

    fn resolve_run_style(
        &self,
        _properties: Option<&RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        Ok(Rc::clone(&self.style))
    }

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        let mut cache = self.measure_cache.borrow_mut();
        Ok(cache
            .get_or_insert_fast(font_descriptor, text, || {
                TextMeasureResult::new(mock_width(text))
            })
            .width)
    }
}

fn mock_width(text: &str) -> f64 {
    text.graphemes(true)
        .map(|g| {
            if g.is_ascii() && !g.trim().is_empty() {
                6.0
            } else if g.trim().is_empty() {
                4.0
            } else {
                12.0
            }
        })
        .sum()
}

/// Builds `n` distinct paragraphs (distinct so their content hashes differ, as in
/// a real document). Each is a few sentences of mixed CJK/Latin so it wraps to
/// several lines.
fn build_document(n: usize) -> Vec<String> {
    let base = "本契約は、甲と乙との間の権利義務を定めるものである。当事者は誠実にこれを履行する。\
                The parties shall perform their obligations in good faith under Section 17.3.";
    (0..n).map(|i| format!("[{i}] {base}")).collect()
}

fn wrap_one(ctx: &MeasuringWrapContext, text: &str) {
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
            use_east_asian_layout: true,
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
        black_box(PAGE_WIDTH_PX),
    )
    .expect("wrap");
    black_box(lines);
}

fn content_hash(text: &str) -> u64 {
    let mut h = FxHasher::default();
    text.hash(&mut h);
    h.finish()
}

/// Models the editor's content-addressed LRU paragraph wrap cache: tracks which
/// paragraph hashes are currently cached, evicting the least-recently-used when
/// full. `note_render` returns true on a miss (the caller must actually wrap).
struct SimWrapCache {
    capacity: usize,
    present: HashSet<u64>,
    order: VecDeque<u64>,
}

impl SimWrapCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            present: HashSet::with_capacity(capacity.max(1)),
            order: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    /// Records a render of `hash`. Returns true if it was a miss (not cached),
    /// in which case it is inserted (evicting the LRU entry if at capacity).
    fn render(&mut self, hash: u64) -> bool {
        if self.present.contains(&hash) {
            // touch: move to MRU
            if let Some(pos) = self.order.iter().position(|&h| h == hash) {
                self.order.remove(pos);
            }
            self.order.push_back(hash);
            return false;
        }
        if self.present.len() >= self.capacity {
            if let Some(evicted) = self.order.pop_front() {
                self.present.remove(&evicted);
            }
        }
        self.present.insert(hash);
        self.order.push_back(hash);
        true
    }
}

/// One render of the visible window `[start, start+window)`: re-wraps each
/// paragraph that is a wrap-cache miss. Returns the number of (re)wraps performed.
fn render_window(
    ctx: &MeasuringWrapContext,
    paragraphs: &[String],
    cache: &mut SimWrapCache,
    start: usize,
    window: usize,
) -> usize {
    let end = (start + window).min(paragraphs.len());
    let mut wraps = 0;
    for text in &paragraphs[start..end] {
        if cache.render(content_hash(text)) {
            wrap_one(ctx, text);
            wraps += 1;
        }
    }
    wraps
}

/// Full scenario: scroll around a large document (sweep the visible window down
/// then back up, `sweeps` times — i.e. the user navigating a long document),
/// then perform `edits` edits in the late-page region (each edit mutates a
/// paragraph and re-renders the surrounding window). Returns total wraps.
///
/// The up/down sweeps are what expose the wrap-cache capacity: when the cache is
/// smaller than the document, paragraphs visited on the way down are evicted by
/// the time the sweep comes back up, forcing a re-wrap (the slowdown). A
/// document-sized cache keeps them, so only the first pass wraps.
fn scroll_then_edit(
    ctx: &MeasuringWrapContext,
    paragraphs: &[String],
    wrap_cache_capacity: usize,
    window: usize,
    sweeps: usize,
    edits: usize,
) -> usize {
    let n = paragraphs.len();
    let mut cache = SimWrapCache::new(wrap_cache_capacity);
    let mut wraps = 0;
    let step = (window / 2).max(1);

    // Build the list of window-start positions for one downward sweep.
    let mut starts = Vec::new();
    let mut start = 0;
    loop {
        starts.push(start);
        if start + window >= n {
            break;
        }
        start += step;
    }

    // Sweep down, then up, then down... (navigating the document).
    for sweep in 0..sweeps {
        if sweep % 2 == 0 {
            for &s in &starts {
                wraps += render_window(ctx, paragraphs, &mut cache, s, window);
            }
        } else {
            for &s in starts.iter().rev() {
                wraps += render_window(ctx, paragraphs, &mut cache, s, window);
            }
        }
    }

    // Edit repeatedly in the last window. Each edit mutates one paragraph (new
    // content hash) and re-renders the window.
    let edit_start = n.saturating_sub(window);
    let mut edited = paragraphs[edit_start..n].to_vec();
    for e in 0..edits {
        let idx = e % edited.len();
        edited[idx] = format!("{} (edit {e})", edited[idx]);
        for text in &edited {
            if cache.render(content_hash(text)) {
                wrap_one(ctx, text);
                wraps += 1;
            }
        }
    }
    wraps
}

fn bench_scroll_then_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("scroll_then_edit");
    group.sample_size(10);

    const WINDOW: usize = 48; // ~a few pages of paragraphs in the viewport
    const SWEEPS: usize = 4; // navigate the document down/up a few times
    const EDITS: usize = 30;

    // Document sizes in paragraphs. 8000 paragraphs exceeds the old 512 cap by a
    // lot, so the small cache thrashes while a document-sized cache does not.
    for &doc_paragraphs in &[2000usize, 8000] {
        let ctx = MeasuringWrapContext::new();
        let paragraphs = build_document(doc_paragraphs);

        // Pre-fix: fixed small wrap cache (the original 512 capacity).
        group.bench_with_input(
            BenchmarkId::new("wrap_cache_512", doc_paragraphs),
            &paragraphs,
            |b, paragraphs| {
                b.iter(|| {
                    black_box(scroll_then_edit(
                        &ctx, paragraphs, 512, WINDOW, SWEEPS, EDITS,
                    ));
                });
            },
        );

        // Post-fix: wrap cache sized to the document (PR #680).
        group.bench_with_input(
            BenchmarkId::new("wrap_cache_doc_sized", doc_paragraphs),
            &paragraphs,
            |b, paragraphs| {
                b.iter(|| {
                    black_box(scroll_then_edit(
                        &ctx,
                        paragraphs,
                        doc_paragraphs,
                        WINDOW,
                        SWEEPS,
                        EDITS,
                    ));
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_scroll_then_edit);
criterion_main!(benches);
