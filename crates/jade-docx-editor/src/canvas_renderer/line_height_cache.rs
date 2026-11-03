//! Windows line-height plans for immutable cached wraps.
use jade_docx_layout::line_metrics::{
    word_layout_platform, WordGridLineSizing, WordLayoutPlatform, WordProportionalLineSizing,
};
use jade_docx_layout::line_wrap::{ParagraphStyle, WrappedLine};
use rustc_hash::FxHashMap;
use std::rc::{Rc, Weak};

// Bound both derivative plans and the slice allocations weak handles retain.
const MAX_ENTRIES: usize = 64;
const MAX_LINES: usize = 128;

#[derive(Clone, Copy, PartialEq)]
struct Inputs {
    fallback: f64,
    grid: Option<WordGridLineSizing>,
    proportional: Option<WordProportionalLineSizing>,
    mark: Option<(f64, f64)>,
    label: Option<f64>,
    first_band_end: usize,
}

#[derive(Debug)]
pub(super) enum LineHeights {
    Fresh(Vec<f64>),
    Shared(Rc<[f64]>),
}
impl std::ops::Deref for LineHeights {
    type Target = [f64];
    fn deref(&self) -> &[f64] {
        match self {
            Self::Fresh(values) => values,
            Self::Shared(values) => values,
        }
    }
}
#[cfg(test)]
impl PartialEq for LineHeights {
    fn eq(&self, other: &Self) -> bool {
        self[..] == other[..]
    }
}

struct Entry {
    lines: Weak<[WrappedLine]>,
    inputs: Inputs,
    heights: Option<Rc<[f64]>>,
}

#[derive(Default)]
pub(super) struct LineHeightPlanCache {
    entries: FxHashMap<usize, Entry>,
}

impl LineHeightPlanCache {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn heights(
        &mut self,
        lines: &Rc<[WrappedLine]>,
        style: &ParagraphStyle,
        fallback: f64,
        mark: Option<(f64, f64)>,
        label: Option<f64>,
    ) -> Option<LineHeights> {
        let compute = || {
            if label.is_some() {
                WrappedLine::line_heights_px_with_numbering_ascent(
                    lines, style, fallback, mark, label,
                )
            } else {
                WrappedLine::line_heights_px_with_paragraph_mark(lines, style, fallback, mark)
            }
        };
        // Preserve the Mac path, including mixed-platform diagnostic contexts.
        if style.word_platform != WordLayoutPlatform::Windows
            || word_layout_platform() != WordLayoutPlatform::Windows
            || lines.len() > MAX_LINES
        {
            return compute().map(LineHeights::Fresh);
        }
        let inputs = Inputs {
            fallback,
            grid: style.word_grid_line_sizing,
            proportional: style.word_proportional_line_sizing,
            mark,
            label,
            first_band_end: (1..lines.len())
                .find(|&i| !style.line_shares_float_band(i))
                .unwrap_or(lines.len()),
        };
        let key = Rc::as_ptr(lines) as *const WrappedLine as usize;
        if let Some(entry) = self.entries.get(&key) {
            if entry.inputs == inputs
                && entry
                    .lines
                    .upgrade()
                    .is_some_and(|cached| Rc::ptr_eq(&cached, lines))
            {
                return entry
                    .heights
                    .as_ref()
                    .map(|heights| LineHeights::Shared(Rc::clone(heights)));
            }
        }
        let heights: Option<Rc<[f64]>> = compute().map(Rc::from);
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.clear();
        }
        // Safe against public cluster mutation: get_mut cannot succeed while
        // this weak handle exists; make_mut dissociates it or clones the slice.
        self.entries.insert(
            key,
            Entry {
                lines: Rc::downgrade(lines),
                inputs,
                heights: heights.clone(),
            },
        );
        heights.map(LineHeights::Shared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_docx_layout::font_resolver::ResolvedRunStyle;
    use jade_docx_layout::line_metrics::set_word_layout_platform;
    use jade_docx_layout::line_wrap::{wrap_text_into_lines, FloatLineSegment, LineWrapContext};
    struct Host;
    impl LineWrapContext for Host {
        type Error = ();
        fn resolve_run_style(
            &self,
            _: Option<&jade_docx_reader::RunProperties>,
        ) -> Result<Rc<ResolvedRunStyle>, ()> {
            Ok(Rc::new(ResolvedRunStyle::new(
                "12pt Test",
                "12pt Test",
                "12pt Test",
                "12pt Test",
                0.0,
                1.0,
                false,
                None,
            )))
        }
        fn measure_text(&self, text: &str, _: &str) -> Result<f64, ()> {
            Ok(text.chars().count() as f64 * 10.0)
        }
    }
    fn fixture() -> (Rc<[WrappedLine]>, ParagraphStyle) {
        set_word_layout_platform(WordLayoutPlatform::Windows);
        let grid = jade_docx_reader::DocGrid {
            grid_type: Some("lines".into()),
            line_pitch: Some(18.0),
            char_space: None,
        };
        let style = ParagraphStyle {
            word_platform: WordLayoutPlatform::Windows,
            word_grid_line_sizing: WordGridLineSizing::new(None, true, Some(&grid), None, 12.0),
            ..Default::default()
        };
        let text = "ab\ncd";
        let run = jade_docx_layout::line_wrap::RunStyle {
            start: 0,
            end: text.len(),
            properties: None,
            revision: None,
            hyperlink: None,
            inline_object: None,
        };
        let mut lines = wrap_text_into_lines(&Host, text, &[run], 0.0, &style, 0, 200.0).unwrap();
        assert_eq!(lines.len(), 2);
        for line in &mut lines {
            for cluster in &mut line.clusters {
                let layout = cluster.layout.as_mut().unwrap();
                layout.font_ascent = Some(10.0);
                layout.font_descent = Some(3.0);
            }
        }
        (lines.into(), style)
    }
    fn fresh(
        lines: &[WrappedLine],
        style: &ParagraphStyle,
        fallback: f64,
        mark: Option<(f64, f64)>,
        label: Option<f64>,
    ) -> Option<LineHeights> {
        WrappedLine::line_heights_px_with_numbering_ascent(lines, style, fallback, mark, label)
            .map(LineHeights::Fresh)
    }
    #[test]
    fn plans_reuse_only_unchanged_wraps_and_spacing_inputs() {
        let (mut lines, mut style) = fixture();
        let mut cache = LineHeightPlanCache::default();
        let expected = fresh(&lines, &style, 16.0, None, None);
        assert_eq!(cache.heights(&lines, &style, 16.0, None, None), expected);
        assert_eq!(cache.heights(&lines, &style, 16.0, None, None), expected);
        assert_eq!(cache.entries.len(), 1);
        let key = Rc::as_ptr(&lines) as *const WrappedLine as usize;
        let weak = cache.entries[&key].lines.clone();
        Rc::make_mut(&mut lines)[0].clusters[0]
            .layout
            .as_mut()
            .unwrap()
            .font_ascent = Some(60.0);
        assert!(weak.upgrade().is_none());
        let changed = cache.heights(&lines, &style, 16.0, None, None);
        assert_ne!(changed, expected);
        assert_eq!(changed, fresh(&lines, &style, 16.0, None, None));
        for (fallback, mark, label) in [(20.0, None, None), (20.0, Some((18.0, 7.0)), Some(80.0))] {
            assert_eq!(
                cache.heights(&lines, &style, fallback, mark, label),
                fresh(&lines, &style, fallback, mark, label)
            );
        }
        style.word_grid_line_sizing = None;
        style.word_proportional_line_sizing = WordProportionalLineSizing::new(
            Some(&jade_docx_reader::Spacing {
                line: Some(1.5),
                line_rule: Some("auto".into()),
                ..Default::default()
            }),
            false,
            true,
        );
        assert_eq!(
            cache.heights(&lines, &style, 20.0, None, Some(80.0)),
            fresh(&lines, &style, 20.0, None, Some(80.0))
        );
    }
    #[test]
    fn labels_track_float_bands_and_mac_always_uses_the_uncached_plan() {
        let (lines, mut style) = fixture();
        let mut cache = LineHeightPlanCache::default();
        let separate = cache.heights(&lines, &style, 16.0, None, Some(60.0));
        style.float_line_segments = vec![
            FloatLineSegment {
                start_x: 0.0,
                width: 200.0,
                skipped_lines_before: 0,
                exact_clearance_height: 0.0,
                shares_band_with_previous: false,
            },
            FloatLineSegment {
                start_x: 0.0,
                width: 200.0,
                skipped_lines_before: 0,
                exact_clearance_height: 0.0,
                shares_band_with_previous: true,
            },
        ];
        let shared = cache.heights(&lines, &style, 16.0, None, Some(60.0));
        assert_ne!(separate, shared);
        assert_eq!(shared, fresh(&lines, &style, 16.0, None, Some(60.0)));
        cache.clear();
        style.word_platform = WordLayoutPlatform::Mac;
        set_word_layout_platform(WordLayoutPlatform::Mac);
        assert_eq!(
            cache.heights(&lines, &style, 18.0, Some((80.0, 9.0)), Some(40.0)),
            fresh(&lines, &style, 18.0, Some((80.0, 9.0)), Some(40.0))
        );
        assert!(cache.entries.is_empty());
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }
    #[test]
    fn windows_hits_share_the_plan_and_mac_keeps_its_fresh_vector() {
        let (lines, mut style) = fixture();
        let mut cache = LineHeightPlanCache::default();
        let first = cache.heights(&lines, &style, 16.0, None, None).unwrap();
        let second = cache.heights(&lines, &style, 16.0, None, None).unwrap();
        let (LineHeights::Shared(a), LineHeights::Shared(b)) = (&first, &second) else {
            panic!("Windows plans should retain a single shared allocation");
        };
        assert!(Rc::ptr_eq(a, b));
        cache.clear();
        assert_eq!(first, second);
        style.word_platform = WordLayoutPlatform::Mac;
        set_word_layout_platform(WordLayoutPlatform::Mac);
        let mac = cache.heights(&lines, &style, 16.0, None, None).unwrap();
        assert!(matches!(mac, LineHeights::Fresh(_)));
        assert_eq!(mac, fresh(&lines, &style, 16.0, None, None).unwrap());
        assert!(cache.entries.is_empty());
        set_word_layout_platform(WordLayoutPlatform::Windows);
    }

    #[test]
    fn retained_weak_slices_and_plan_vectors_are_bounded() {
        let (lines, style) = fixture();
        let mut cache = LineHeightPlanCache::default();
        for _ in 0..MAX_ENTRIES * 3 {
            let copy = Rc::from(lines.as_ref().to_vec());
            cache.heights(&copy, &style, 16.0, None, None);
            assert!(cache.entries.len() <= MAX_ENTRIES);
        }
        let large = Rc::from(vec![lines[0].clone(); MAX_LINES + 1]);
        let count = cache.entries.len();
        assert_eq!(
            cache.heights(&large, &style, 16.0, None, None),
            fresh(&large, &style, 16.0, None, None)
        );
        assert_eq!(cache.entries.len(), count);
    }
}
