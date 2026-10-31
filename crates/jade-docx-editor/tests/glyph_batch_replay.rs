//! Replay must preserve positioned glyphs across reusable batches and fallback paths.
use jade_docx_editor::draw_command::{
    executor::{execute_command_with_font_state, FontStateTracker},
    DrawCommand, GlyphRunStep, TextStyle,
};
use jade_painter::{Painter, TextMetrics};
use proptest::prelude::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::{Cell, RefCell},
    rc::Rc,
};

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}
struct CountedAllocator;
fn count_allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}
unsafe impl GlobalAlloc for CountedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count_allocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountedAllocator = CountedAllocator;

fn allocations_during(action: impl FnOnce()) -> usize {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ALLOCATIONS.with(|count| count.set(None));
        }
    }
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let reset = Reset;
    action();
    let count = ALLOCATIONS.with(|count| count.get().unwrap());
    drop(reset);
    count
}

#[derive(Clone, Copy, Debug)]
enum BatchMode {
    Accept,
    Decline,
    Error,
}
#[derive(Debug, PartialEq)]
struct PaintedGlyph {
    text: String,
    font: String,
    x: f64,
    y: f64,
}
struct ProbePainter {
    mode: Cell<BatchMode>,
    record: bool,
    font: RefCell<String>,
    glyphs: RefCell<Vec<PaintedGlyph>>,
}
impl ProbePainter {
    fn new(record: bool) -> Self {
        Self {
            mode: Cell::new(BatchMode::Accept),
            record,
            font: RefCell::new(String::new()),
            glyphs: RefCell::new(Vec::new()),
        }
    }
    fn accepts(&self) -> Result<bool, &'static str> {
        match self.mode.get() {
            BatchMode::Accept => Ok(true),
            BatchMode::Decline => Ok(false),
            BatchMode::Error => Err("batch failed"),
        }
    }
}
macro_rules! noop_methods {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*);)*) => {
        $(fn $name(&self $(, $arg: $ty)*) {})*
    };
}
#[allow(clippy::too_many_arguments)]
impl Painter for ProbePainter {
    type Error = &'static str;
    noop_methods! {
        fn save(&self); fn restore(&self);
        fn set_fill_style_str(&self, _value: &str); fn set_stroke_style_str(&self, _value: &str);
        fn set_line_width(&self, _width: f64); fn set_global_alpha(&self, _alpha: f64);
        fn set_line_dash(&self, _segments: &[f64]); fn set_shadow_color(&self, _value: &str);
        fn set_shadow_blur(&self, _value: f64); fn set_shadow_offset_x(&self, _value: f64);
        fn set_shadow_offset_y(&self, _value: f64); fn set_blur_filter(&self, _radius: f64);
        fn set_text_baseline(&self, _value: &str);
        fn fill_rect(&self, _x: f64, _y: f64, _w: f64, _h: f64);
        fn clear_rect(&self, _x: f64, _y: f64, _w: f64, _h: f64);
        fn stroke_rect(&self, _x: f64, _y: f64, _w: f64, _h: f64);
        fn begin_path(&self); fn close_path(&self); fn move_to(&self, _x: f64, _y: f64);
        fn line_to(&self, _x: f64, _y: f64); fn rect(&self, _x: f64, _y: f64, _w: f64, _h: f64);
        fn quadratic_curve_to(&self, _cx: f64, _cy: f64, _x: f64, _y: f64);
        fn bezier_curve_to(&self, _a: f64, _b: f64, _c: f64, _d: f64, _x: f64, _y: f64);
        fn fill(&self); fn stroke(&self); fn clip(&self); fn use_fill_style_for_stroke(&self);
    }
    fn translate(&self, _: f64, _: f64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn scale(&self, _: f64, _: f64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn rotate(&self, _: f64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn arc(&self, _: f64, _: f64, _: f64, _: f64, _: f64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn ellipse(
        &self,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn ellipse_with_anticlockwise(
        &self,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
        _: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_font(&self, value: &str) {
        if self.record {
            self.font.replace(value.to_owned());
        }
    }
    fn fill_text(&self, text: &str, x: f64, y: f64) -> Result<(), Self::Error> {
        if self.record {
            self.glyphs.borrow_mut().push(PaintedGlyph {
                text: text.to_owned(),
                font: self.font.borrow().clone(),
                x,
                y,
            });
        }
        Ok(())
    }
    fn stroke_text(&self, _: &str, _: f64, _: f64) -> Result<(), Self::Error> {
        panic!("unexpected outline")
    }
    fn measure_text_width(&self, _: &str) -> Result<f64, Self::Error> {
        panic!("positioned replay must not remeasure")
    }
    fn measure_text_metrics(&self, _: &str) -> Result<TextMetrics, Self::Error> {
        panic!("positioned replay must not remeasure")
    }
    fn try_fill_glyph_run_batch(
        &self,
        text: &str,
        positions: &[f64],
        y: f64,
    ) -> Result<bool, Self::Error> {
        if !self.accepts()? {
            return Ok(false);
        }
        assert_eq!(text.chars().count(), positions.len());
        for (ch, x) in text.chars().zip(positions) {
            self.fill_text(ch.encode_utf8(&mut [0; 4]), *x, y)?;
        }
        Ok(true)
    }
    fn try_fill_glyph_run_batch_multi_font(
        &self,
        text: &str,
        positions: &[f64],
        indexes: &[u16],
        fonts: &[Rc<str>],
        y: f64,
    ) -> Result<bool, Self::Error> {
        if !self.accepts()? {
            return Ok(false);
        }
        assert_eq!(text.chars().count(), positions.len());
        assert_eq!(indexes.len(), positions.len());
        for ((ch, x), font) in text.chars().zip(positions).zip(indexes) {
            self.set_font(&fonts[usize::from(*font)]);
            self.fill_text(ch.encode_utf8(&mut [0; 4]), *x, y)?;
        }
        Ok(true)
    }
}

fn run(parts: &[(u8, u8, i16)]) -> DrawCommand {
    const SYMBOLS: [&str; 6] = ["a", "漢", "é", "😀", "e\u{301}", ""];
    let mut text = String::new();
    let mut glyphs = Vec::new();
    for &(symbol, font, offset) in parts {
        let start = text.len();
        text.push_str(SYMBOLS[usize::from(symbol)]);
        glyphs.push(GlyphRunStep {
            text_start: start as u32,
            text_end: text.len() as u32,
            font_index: u16::from(font),
            x_offset: f64::from(offset) / 4.0,
        });
    }
    DrawCommand::TextRun {
        text: Rc::from(text),
        x: 13.25,
        y: -7.5,
        style: TextStyle::default(),
        fonts: vec![Rc::from("12pt Arial"), Rc::from("10.5pt 'Noto Sans JP'")],
        glyphs,
    }
}

#[test]
fn tracker_construction_is_allocation_free() {
    assert_eq!(
        allocations_during(|| {
            std::hint::black_box(FontStateTracker::new());
        }),
        0
    );
}

#[test]
fn warm_single_font_and_declined_batches_do_not_allocate() {
    for mode in [BatchMode::Accept, BatchMode::Decline] {
        let painter = ProbePainter::new(false);
        painter.mode.set(mode);
        let command = run(&[(1, 0, 0), (0, 0, 11), (2, 0, 53)]);
        let mut tracker = FontStateTracker::new();
        execute_command_with_font_state(&painter, &command, &mut tracker).unwrap();
        let allocations = allocations_during(|| {
            for _ in 0..512 {
                execute_command_with_font_state(&painter, &command, &mut tracker).unwrap();
            }
        });
        assert_eq!(
            allocations, 0,
            "warm replay must reuse batch storage: {mode:?}"
        );
    }
}

#[test]
fn warm_multi_font_batch_only_allocates_the_existing_invalidated_fill_style() {
    let painter = ProbePainter::new(false);
    let command = run(&[(1, 0, 0), (0, 1, 17), (2, 0, 49)]);
    let mut tracker = FontStateTracker::new();
    execute_command_with_font_state(&painter, &command, &mut tracker).unwrap();
    let allocations = allocations_during(|| {
        for _ in 0..512 {
            execute_command_with_font_state(&painter, &command, &mut tracker).unwrap();
        }
    });
    // The successful multi-font path invalidates canvas state, including its fill string.
    assert_eq!(allocations, 512, "batch vectors must not add allocations");
}

#[test]
fn warm_storage_survives_backend_errors() {
    for parts in [vec![(1, 0, 0), (0, 0, 17)], vec![(1, 0, 0), (0, 1, 17)]] {
        let painter = ProbePainter::new(false);
        painter.mode.set(BatchMode::Error);
        let command = run(&parts);
        let mut tracker = FontStateTracker::new();
        assert_eq!(
            execute_command_with_font_state(&painter, &command, &mut tracker),
            Err("batch failed")
        );
        let allocations = allocations_during(|| {
            for _ in 0..512 {
                assert_eq!(
                    execute_command_with_font_state(&painter, &command, &mut tracker),
                    Err("batch failed")
                );
            }
        });
        assert_eq!(allocations, 0, "failed batches must return their storage");
    }
}

#[test]
fn growing_contiguous_cjk_run_only_grows_position_storage() {
    let painter = ProbePainter::new(false);
    let mut tracker = FontStateTracker::new();
    let small = run(&[(0, 0, 0)]);
    execute_command_with_font_state(&painter, &small, &mut tracker).unwrap();
    let large = run(&vec![(1, 0, 17); 4096]);
    let allocations = allocations_during(|| {
        execute_command_with_font_state(&painter, &large, &mut tracker).unwrap();
    });
    assert_eq!(allocations, 1, "contiguous text needs only position growth");
}

#[test]
fn glyph_subsets_gaps_and_reordering_preserve_exact_text() {
    let painter = ProbePainter::new(true);
    let mut tracker = FontStateTracker::new();
    for indexes in [vec![1, 2], vec![0, 2], vec![2, 1, 0], vec![1, 1], vec![]] {
        for mode in [BatchMode::Accept, BatchMode::Decline, BatchMode::Error] {
            let mut command = run(&[(0, 0, 0), (1, 1, 11), (2, 0, 53)]);
            let DrawCommand::TextRun { glyphs, .. } = &mut command else {
                unreachable!()
            };
            let source = glyphs.clone();
            *glyphs = indexes.iter().map(|index| source[*index].clone()).collect();
            painter.mode.set(mode);
            painter.glyphs.borrow_mut().clear();
            let DrawCommand::TextRun {
                text,
                fonts,
                glyphs,
                x,
                y,
                ..
            } = &command
            else {
                unreachable!()
            };
            let error = matches!(mode, BatchMode::Error) && !glyphs.is_empty();
            let expected: Vec<_> = if error {
                Vec::new()
            } else {
                glyphs
                    .iter()
                    .map(|glyph| PaintedGlyph {
                        text: text[glyph.text_start as usize..glyph.text_end as usize].to_owned(),
                        font: fonts[usize::from(glyph.font_index)].to_string(),
                        x: x + glyph.x_offset,
                        y: *y,
                    })
                    .collect()
            };
            assert_eq!(
                execute_command_with_font_state(&painter, &command, &mut tracker),
                if error { Err("batch failed") } else { Ok(()) },
                "{indexes:?} {mode:?}"
            );
            assert_eq!(&*painter.glyphs.borrow(), &expected, "{indexes:?} {mode:?}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn replay_sequences_preserve_glyphs_fonts_positions_and_errors(
        steps in prop::collection::vec((0u8..8, prop::collection::vec((0u8..6, 0u8..3, -800i16..800), 0..32)), 1..48)
    ) {
        let painter = ProbePainter::new(true);
        let mut tracker = FontStateTracker::new();
        for (step, (kind, mut parts)) in steps.into_iter().enumerate() {
            // Exercise accepted, declined and failed BMP batches, Unicode clusters,
            // invalid font indexes, and reversed, sparse and partial glyph runs.
            if kind != 4 {
                for (symbol, font, _) in &mut parts {
                    *symbol %= 3;
                    *font = if kind == 0 { 0 } else { *font % 2 };
                }
            }
            let mut command = run(&parts);
            if let DrawCommand::TextRun { glyphs, .. } = &mut command {
                match kind {
                    5 => { glyphs.reverse(); }
                    6 => { *glyphs = glyphs.iter().step_by(2).cloned().collect(); }
                    7 => { if !glyphs.is_empty() { glyphs.remove(0); } }
                    _ => {}
                }
            }
            if kind == 4 {
                if let DrawCommand::TextRun { glyphs, .. } = &mut command {
                    if let Some(glyph) = glyphs.first_mut() { glyph.text_start += 1; }
                }
            }
            painter.mode.set(match kind { 2 => BatchMode::Decline, 3 => BatchMode::Error, _ => BatchMode::Accept });
            painter.glyphs.borrow_mut().clear();
            let DrawCommand::TextRun { text, fonts, glyphs, x, y, .. } = &command else { unreachable!() };
            let eligible = !glyphs.is_empty() && glyphs.iter().all(|glyph| {
                let Some(value) = text.get(glyph.text_start as usize..glyph.text_end as usize) else { return false; };
                fonts.get(usize::from(glyph.font_index)).is_some() && value.chars().count() == 1 && value.encode_utf16().count() == 1
            });
            let should_error = kind == 3 && eligible;
            let expected: Vec<_> = if should_error { Vec::new() } else {
                glyphs.iter().filter_map(|glyph| Some(PaintedGlyph {
                    text: text.get(glyph.text_start as usize..glyph.text_end as usize)?.to_owned(),
                    font: fonts.get(usize::from(glyph.font_index))?.to_string(),
                    x: x + glyph.x_offset,
                    y: *y,
                })).collect()
            };
            let result = execute_command_with_font_state(&painter, &command, &mut tracker);
            prop_assert_eq!(result, if should_error { Err("batch failed") } else { Ok(()) }, "step {} kind {}", step, kind);
            prop_assert_eq!(&*painter.glyphs.borrow(), &expected, "step {} kind {}", step, kind);
        }
    }
}
