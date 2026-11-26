//! Cost scenarios for the DOCX engine that count the same on every run.
//!
//! Wall-clock time on a shared CI runner swings tens of percent on unchanged
//! code, so a pull request cannot be judged by it. Instructions executed can:
//! `.github/workflows/perf.yml` runs each scenario here under callgrind, once
//! on the base commit and once on the head, and reports the difference.
//!
//! Each scenario does its setup first and then runs the part it measures
//! inside [`perf_measured_region`]. Callgrind is started with collection off
//! and toggles it on that function, so loading a fixture does not count
//! towards the cost of typing into it.
//!
//! The renderer is the one the browser runs. Text is measured with
//! `jade-docx-layout`'s shaping measurer and real fonts, as the native renderer
//! does; painting goes to [`PerfPainter`], which draws nothing, so what is
//! counted is layout plus the renderer's decisions about what to draw, not a
//! rasteriser.

use futures::executor::block_on;
use jade_docx_editor::canvas_renderer::CanvasRenderer;
use jade_docx_editor::commands::{CommandExecutor, HistoryRenderFootprint};
use jade_docx_editor::headless_api::{
    apply_headless_commands_to_document, apply_headless_commands_with_executor_to_document,
};
use jade_docx_editor::render_host::{CanvasRenderHost, ImageCropMapping};
use jade_docx_editor::word::fields::SelectionRange;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_editor::Options;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_layout::font_measurer::RegisteredFontStyle;
use jade_docx_layout::shaping_measurer::{ShapingMeasurer, ShapingOptions};
use jade_painter::{Painter, TextMetrics};

mod painter;
pub use painter::PerfPainter;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use wasm_bindgen::JsValue;

const PAGE_WIDTH_PX: f64 = 816.0;
const PAGE_HEIGHT_PX: f64 = 1056.0;

/// Keystrokes typed by both input scenarios. `JADE_PERF_KEYSTROKES`
/// overrides it for a sampling profiler that needs a longer run; the workflow
/// leaves it unset so both sides type the same.
pub const KEYSTROKES: usize = 20;

fn keystrokes() -> usize {
    std::env::var("JADE_PERF_KEYSTROKES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(KEYSTROKES)
}

/// The scenarios, in report order. `keystroke-band` is a diagnostic: the
/// workflow reports it in its own step, outside the frozen per-case gate of
/// `scripts/perf/check_regressions.py`.
pub const SCENARIOS: &[&str] = &[
    "load",
    "layout",
    "repaint",
    "keystroke",
    "keystroke-marks",
    "keystroke-band",
    "export",
];

/// Runs `body` as the measured part of a scenario.
///
/// Callgrind's `--toggle-collect=*perf_measured_region*` keys on this name, so
/// it must stay a real, uninlined function with this name.
#[inline(never)]
// SAFETY: This symbol is unique to the standalone performance harness. It is
// called only through its Rust signature, never through a foreign ABI.
#[unsafe(export_name = "jade_docx_perf_measured_region")]
pub fn perf_measured_region(body: &mut dyn FnMut()) {
    // Keep ThinLTO from specializing the callback away from the named boundary.
    std::hint::black_box(body)();
    std::hint::black_box(());
}

/// The repository root, where the fixtures and fonts live.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn options(scenario: &str) -> Options {
    let mut options = Options::default();
    options.show_formatting_marks = scenario == "keystroke-marks";
    options
}

/// A render host that measures with real fonts and draws nothing. Page geometry is answered for one page size; the bitmap and canvas
/// handles belong to the browser and are never reached.
pub struct PerfHost {
    painter: PerfPainter,
    pages: Rc<Cell<usize>>,
}

impl PerfHost {
    pub fn new() -> Result<Self, String> {
        let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
        for (weight, file) in [
            (400, "NotoSansJP-Regular.ttf"),
            (700, "NotoSansJP-Bold.ttf"),
        ] {
            let path = repo_root().join(format!("fonts/Noto_Sans_JP/static/{file}"));
            // The same stand-ins the native renderer's tests register: the
            // fixtures name these families, and the CI runner has none of them.
            for family in [
                "Noto Sans JP",
                "Times New Roman",
                "Yu Mincho",
                "MS Mincho",
                "serif",
            ] {
                measurer
                    .register_font_path(family, weight, RegisteredFontStyle::Normal, &path)
                    .map_err(|err| format!("{}: {err:?}", path.display()))?;
            }
        }
        Ok(Self {
            painter: PerfPainter::new(Rc::new(measurer)),
            pages: Rc::new(Cell::new(1)),
        })
    }
}

impl CanvasRenderHost for PerfHost {
    type Painter = PerfPainter;

    fn painter(&self) -> Self::Painter {
        self.painter.clone()
    }

    fn set_font(&self, font_descriptor: &str) {
        self.painter.set_font(font_descriptor);
    }

    fn draw_image_data_uri(
        &self,
        _uri: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        _crop: Option<ImageCropMapping>,
    ) -> bool {
        self.painter.fill_rect(x, y, width, height);
        true
    }

    fn measure_text_width(&self, text: &str) -> Result<f64, JsValue> {
        Ok(self.painter.measure_text_width(text).unwrap_or(0.0))
    }

    fn measure_text_metrics(&self, text: &str) -> Result<TextMetrics, JsValue> {
        Ok(match self.painter.measure_text_metrics(text) {
            Ok(metrics) => metrics,
            Err(never) => match never {},
        })
    }

    fn device_pixel_ratio(&self) -> f64 {
        1.0
    }

    fn update_device_pixel_ratio(&mut self) -> Result<bool, JsValue> {
        Ok(false)
    }

    fn device_to_css(&self, value: f64) -> f64 {
        value
    }

    fn canvas_css_size(&self) -> (f64, f64) {
        (PAGE_WIDTH_PX, PAGE_HEIGHT_PX)
    }

    fn page_count(&self) -> usize {
        self.pages.get()
    }

    fn page_dimensions(&self, _page_index: usize) -> Option<(f64, f64)> {
        Some((PAGE_WIDTH_PX, PAGE_HEIGHT_PX))
    }

    fn reset_pages(&mut self) -> Result<(), JsValue> {
        Ok(())
    }

    fn configure_base_canvas(
        &mut self,
        _width: f64,
        _height: f64,
        _pageless: bool,
        _margin_top: f64,
    ) -> Result<(), JsValue> {
        Ok(())
    }

    fn clear(&mut self, _page_width: f64, _pageless: bool, _full: f64) -> Result<(), JsValue> {
        Ok(())
    }

    fn ensure_pageless_height(&mut self, _height: f64, _margin_bottom: f64) -> Result<(), JsValue> {
        Ok(())
    }

    fn start_new_page(&mut self, _w: f64, _h: f64, _margin_top: f64) -> Result<(), JsValue> {
        self.pages.set(self.pages.get() + 1);
        Ok(())
    }

    fn context(&self) -> &web_sys::CanvasRenderingContext2d {
        unreachable!("perf host: the renderer must not reach for a canvas")
    }

    fn context_mut(&mut self) -> &mut web_sys::CanvasRenderingContext2d {
        unreachable!("perf host: the renderer must not reach for a canvas")
    }

    fn create_page_bitmap(&self, _page_index: usize) -> Result<js_sys::Promise, JsValue> {
        unreachable!("perf host: page bitmaps are a browser-side cache")
    }

    fn take_paged_snapshots(&mut self) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }

    fn take_paged_snapshots_for_pages(
        &mut self,
        _pages: Option<&[usize]>,
    ) -> Result<Option<Vec<JsValue>>, JsValue> {
        Ok(None)
    }
}

fn load(bytes: &[u8], options: &Options) -> Result<WordDocument, String> {
    block_on(WordDocument::load(bytes, options)).map_err(|err| format!("load: {err:?}"))
}

fn renderer() -> Result<CanvasRenderer<PerfPainter>, String> {
    CanvasRenderer::with_host(Box::new(PerfHost::new()?), false)
        .map_err(|err| format!("renderer: {err:?}"))
}

fn render_cold(
    renderer: &mut CanvasRenderer<PerfPainter>,
    model: &WordDocument,
    options: &Options,
) -> Result<(), String> {
    // The editor renders the model's own `Rc<Document>` (see `canvas_view`).
    // `CanvasRenderer::render` deep-clones a borrowed document instead, a
    // cost no editor render pays, which made up about a tenth of what the
    // layout and repaint scenarios measured.
    let document = model.document.clone().ok_or("no main document")?;
    renderer
        .render_with_numbering_rc(document, options, None, None, None, None)
        .map_err(|err| format!("render: {err:?}"))
}

/// A paragraph on page 1, preferring one a few paragraphs in (past a title),
/// so a repaint bounded to the first page actually draws the edit.
fn keystroke_paragraph(renderer: &CanvasRenderer<PerfPainter>) -> Option<usize> {
    (6..64)
        .chain(0..6)
        .find(|&index| renderer.cached_paragraph_start_page(index) == Some(1))
}

/// Runs one scenario on `fixture`. Setup happens outside the measured region.
pub fn run(scenario: &str, fixture: &Path) -> Result<(), String> {
    if std::env::var("JADE_PERF_WORD_PLATFORM").as_deref() == Ok("mac") {
        jade_docx_layout::line_metrics::set_word_layout_platform(
            jade_docx_layout::line_metrics::WordLayoutPlatform::Mac,
        );
    }
    let bytes = std::fs::read(fixture).map_err(|err| format!("{}: {err}", fixture.display()))?;
    // Built once: the editor keeps its options, and building them parses the
    // shortcut table, which no keystroke pays for.
    let options = &options(scenario);
    match scenario {
        "load" => {
            let mut loaded = None;
            perf_measured_region(&mut || loaded = Some(load(&bytes, options)));
            loaded.ok_or("load did not run")??;
        }
        "layout" => {
            let model = load(&bytes, options)?;
            let mut renderer = renderer()?;
            let mut result = Ok(());
            perf_measured_region(&mut || result = render_cold(&mut renderer, &model, options));
            result?;
        }
        "repaint" => {
            // Paint cost with layout already known: a full repaint of a laid
            // out document, which is what a theme or zoom switch runs.
            let model = load(&bytes, options)?;
            let mut renderer = renderer()?;
            render_cold(&mut renderer, &model, options)?;
            let mut result = Ok(());
            perf_measured_region(&mut || {
                renderer.mark_full_repaint();
                result = render_cold(&mut renderer, &model, options);
            });
            result?;
        }
        "keystroke" | "keystroke-marks" => {
            let mut model = load(&bytes, options)?;
            let mut renderer = renderer()?;
            render_cold(&mut renderer, &model, options)?;
            let paragraph = keystroke_paragraph(&renderer).ok_or("no paragraph on page 1")?;
            let mut executor = CommandExecutor::new();
            let mut result = Ok(());
            perf_measured_region(&mut || {
                for _ in 0..keystrokes() {
                    result = type_one(&mut renderer, &mut model, paragraph, options, &mut executor);
                    if result.is_err() {
                        return;
                    }
                }
            });
            result?;
        }
        "keystroke-band" => {
            // Typing as `NativeDocument` does it, band redraw included: see
            // `BandTyping`.
            let mut typing = BandTyping::open(&bytes, options, renderer()?)?;
            let mut result = Ok(());
            perf_measured_region(&mut || {
                for _ in 0..keystrokes() {
                    result = typing.type_one(options);
                    if result.is_err() {
                        return;
                    }
                }
            });
            result?;
        }
        "export" => {
            // An edited document, so the writer serialises the main part
            // instead of copying it through untouched.
            let mut model = load(&bytes, options)?;
            insert_text(&mut model, 0)?;
            let mut exported = Ok(Vec::new());
            perf_measured_region(&mut || {
                exported = model
                    .export_docx()
                    .map_err(|err| format!("export: {err:?}"));
            });
            exported?;
        }
        other => return Err(format!("unknown scenario {other:?}; known: {SCENARIOS:?}")),
    }
    Ok(())
}

fn insert_text(model: &mut WordDocument, paragraph: usize) -> Result<(), String> {
    apply_headless_commands_to_document(
        model,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph,
                offset: 0,
                text: "あ".to_string(),
            }],
        },
    )
    .map(|_| ())
    .map_err(|err| format!("insert text: {err:?}"))
}

/// One keystroke: the edit, then the repaint sequence `canvas_view` runs after
/// it, bounded to the first page as a window showing it would be.
fn type_one(
    renderer: &mut CanvasRenderer<PerfPainter>,
    model: &mut WordDocument,
    paragraph: usize,
    options: &Options,
    executor: &mut CommandExecutor,
) -> Result<(), String> {
    const VISIBLE_END_PAGE_1IDX: usize = 1;
    let revision = executor.history_revision();
    let text_edit_cache = renderer.local_text_edit_cache();
    // As `canvas_view` does before every edit: with the renderer still
    // holding the document, the edit would deep-clone it to mutate it.
    renderer.drop_document_cache();
    apply_headless_commands_with_executor_to_document(
        model,
        executor,
        None,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph,
                offset: 0,
                text: "あ".to_string(),
            }],
        },
    )
    .map_err(|err| format!("insert text: {err:?}"))?;
    // Browser/native typing can reuse feature facts only after the newly
    // recorded command proves it changed exactly this existing paragraph.
    if executor.history_revision() != revision
        && executor.next_undo_render_footprint()
            == Some(HistoryRenderFootprint::SingleParagraph { index: paragraph })
    {
        renderer.restore_local_text_edit_cache(text_edit_cache, model, paragraph);
    }
    let edited = model.document.as_ref().ok_or("no main document")?.clone();
    renderer.prefer_visible_window_seek_once();
    renderer.set_incremental_dirty_from(paragraph);
    renderer.invalidate_paragraph_wrap_cache_from(paragraph);
    renderer.mark_dirty_from_paragraph_to_end_bounded(paragraph, VISIBLE_END_PAGE_1IDX);
    let previous_limit = renderer.layout_page_limit();
    renderer.set_layout_page_limit(Some(VISIBLE_END_PAGE_1IDX));
    let result = renderer
        .render_with_numbering_rc(edited, options, None, None, None, None)
        .map_err(|err| format!("repaint: {err:?}"));
    renderer.set_layout_page_limit(previous_limit);
    result
}

/// Where a paragraph's lines start and end, as `NativeDocument` compares it
/// before and after an edit: line count, first line top, last line bottom.
type ParagraphExtent = (usize, f64, f64);

fn paragraph_extent(
    renderer: &CanvasRenderer<PerfPainter>,
    paragraph: usize,
) -> Option<ParagraphExtent> {
    let lines = &renderer
        .selection_manager()
        .paragraphs()
        .get(paragraph)?
        .lines;
    let (first, last) = (lines.first()?, lines.last()?);
    Some((
        lines.len(),
        first.absolute_top,
        last.absolute_top + last.height,
    ))
}

/// A document typed into the way `NativeDocument` types into it.
///
/// [`type_one`] is the browser-shaped sequence the frozen `keystroke` cases
/// measure, and it never enables band redraw, so it measures the full-page
/// fallback. Native typing (`open` and `apply_and_repaint_grouped` in
/// `jade-docx-native-render/src/document.rs`) takes the band path on every
/// single-paragraph keystroke. This follows that path step by step; the
/// shell's own bookkeeping (stale page sets, undo shell steps) is left out.
struct BandTyping {
    renderer: CanvasRenderer<PerfPainter>,
    model: WordDocument,
    executor: CommandExecutor,
    selection: Option<SelectionRange>,
    paragraph: usize,
    /// Pages the opening render laid out: what `NativeDocument::page_count`
    /// reports between whole-document renders.
    pages: usize,
}

impl BandTyping {
    /// Loads and lays out every page as `NativeDocument` opens a document,
    /// with its numbering, headers, footers and comments.
    fn open(
        bytes: &[u8],
        options: &Options,
        mut renderer: CanvasRenderer<PerfPainter>,
    ) -> Result<Self, String> {
        let model = load(bytes, options)?;
        let document = model.document.clone().ok_or("no main document")?;
        renderer.sync_word_document(&model);
        renderer.set_use_selection_overlay(false);
        renderer.mark_full_repaint();
        renderer
            .render_with_numbering_rc(
                document,
                options,
                model.numbering_part.as_ref(),
                model.header_lines.as_deref(),
                model.footer_lines.as_deref(),
                Some(&model.comments),
            )
            .map_err(|err| format!("render: {err:?}"))?;
        if let Some(error) = renderer.take_render_error() {
            return Err(error);
        }
        renderer.clear_dirty_regions();
        let paragraph = keystroke_paragraph(&renderer).ok_or("no paragraph on page 1")?;
        let pages = (0..model.paragraph_count())
            .rev()
            .find_map(|index| paragraph_extent(&renderer, index))
            .map_or(1, |(_, _, bottom)| {
                renderer.page_for_y((bottom - 0.5).max(0.0)) + 1
            });
        Ok(Self {
            renderer,
            model,
            executor: CommandExecutor::new(),
            selection: None,
            paragraph,
            pages,
        })
    }

    /// One keystroke through `apply_and_repaint_grouped` for an `InsertText`
    /// of one step, bounded to the first page as a window showing it would be.
    fn type_one(&mut self, options: &Options) -> Result<(), String> {
        const VISIBLE_END_PAGE_1IDX: usize = 1;
        let renderer = &mut self.renderer;
        let paragraph = self.paragraph;
        let old_height = renderer.cached_paragraph_height(paragraph);
        let extent_before = paragraph_extent(renderer, paragraph);
        let text_edit_cache = renderer.local_text_edit_cache_for_paragraph(&self.model, paragraph);
        let recorded = self.executor.history_revision();
        // `edit_in_place`: the renderer gives the document up for the edit and
        // takes the edited one back.
        renderer.drop_document_cache();
        let after = apply_headless_commands_with_executor_to_document(
            &mut self.model,
            &mut self.executor,
            self.selection.take(),
            &HeadlessEditRequest {
                commands: vec![HeadlessCommand::InsertText {
                    paragraph,
                    offset: 0,
                    text: "あ".to_string(),
                }],
            },
        );
        if let Some(document) = &self.model.document {
            renderer.restore_document_cache(Rc::clone(document));
        }
        self.selection = after.map_err(|err| format!("insert text: {err:?}"))?;
        renderer.set_selection_range_for_render(self.selection);
        let edited = self.model.document.clone().ok_or("no main document")?;
        let single_paragraph_edit = self.executor.history_revision() != recorded
            && self.executor.next_undo_render_footprint()
                == Some(HistoryRenderFootprint::SingleParagraph { index: paragraph });
        if !single_paragraph_edit {
            return Err("insert text did not record a single-paragraph edit".to_string());
        }
        renderer.set_band_redraw_enabled(true);
        renderer.restore_local_text_edit_cache(text_edit_cache, &self.model, paragraph);
        if let Some(height) = old_height {
            renderer.prepare_completed_pagination_edit(paragraph, height);
        }
        // Raised to the last page the edited paragraph reached.
        let visible_end_page_1idx = match extent_before {
            Some((_, _, bottom)) => {
                VISIBLE_END_PAGE_1IDX.max(renderer.page_for_y((bottom - 0.5).max(0.0)) + 1)
            }
            None => VISIBLE_END_PAGE_1IDX,
        };
        let bounded = visible_end_page_1idx < self.pages;
        if bounded {
            renderer.set_aside_edit_downstream(paragraph);
        }
        renderer.prepare_incremental_repaint(paragraph, visible_end_page_1idx);
        if bounded {
            renderer.set_visible_page_range(Some(1..=visible_end_page_1idx.max(1)));
        }
        let result = renderer.render_bounded_to_page_with_parts(
            edited,
            options,
            self.model.numbering_part.as_ref(),
            self.model.header_lines.as_deref(),
            self.model.footer_lines.as_deref(),
            Some(&self.model.comments),
            visible_end_page_1idx,
        );
        if bounded {
            renderer.clear_visible_page_range();
        }
        renderer.set_band_redraw_enabled(false);
        result.map_err(|err| format!("repaint: {err:?}"))?;
        if let Some(error) = renderer.take_render_error() {
            return Err(error);
        }
        let shown_elsewhere = renderer.paragraph_text_shows_elsewhere(paragraph);
        if let Some(saved) = renderer.take_edit_downstream() {
            let stood = extent_before.is_some()
                && !shown_elsewhere
                && paragraph_extent(renderer, paragraph) == extent_before
                && renderer.edit_downstream_kept_its_lines(&saved);
            if stood {
                renderer.restore_edit_downstream(saved);
            }
        }
        renderer.clear_dirty_regions();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_region_invokes_the_mutable_callback_once() {
        let mut calls = 0;
        perf_measured_region(&mut || calls += 1);
        assert_eq!(calls, 1);
    }

    #[test]
    fn measured_region_preserves_rust_unwinding() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            perf_measured_region(&mut || panic!("measured callback"));
        }));
        assert!(result.is_err());
    }

    #[test]
    fn formatting_mark_workload_draws_marks_during_warm_typing() {
        let bytes = std::fs::read(repo_root().join("public/fixtures/contract.docx")).unwrap();
        for scenario in ["keystroke", "keystroke-marks"] {
            let options = options(scenario);
            let mut model = load(&bytes, &options).unwrap();
            let host = PerfHost::new().unwrap();
            let painter = host.painter.clone();
            let mut renderer = CanvasRenderer::with_host(Box::new(host), false).unwrap();
            render_cold(&mut renderer, &model, &options).unwrap();
            let marks_visible = scenario == "keystroke-marks";
            assert_eq!(
                painter.take_paragraph_mark_count() > 0,
                marks_visible,
                "{scenario}: initial layout must draw the requested marks"
            );

            let paragraph = keystroke_paragraph(&renderer).unwrap();
            let mut executor = CommandExecutor::new();
            for _ in 0..3 {
                let revision = executor.history_revision();
                type_one(
                    &mut renderer,
                    &mut model,
                    paragraph,
                    &options,
                    &mut executor,
                )
                .unwrap();
                assert_ne!(executor.history_revision(), revision);
                assert_eq!(
                    executor.next_undo_render_footprint(),
                    Some(HistoryRenderFootprint::SingleParagraph { index: paragraph })
                );
                assert_eq!(
                    painter.take_paragraph_mark_count() > 0,
                    marks_visible,
                    "{scenario}: bounded input repaint must draw the requested marks"
                );
            }
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(repo_root().join("public/fixtures").join(name)).unwrap()
    }

    fn bands_taken(renderer: &CanvasRenderer<PerfPainter>) -> u64 {
        renderer.band_redraw_stats().1
    }

    #[test]
    fn band_workload_takes_a_band_on_every_measured_keystroke_but_a_wrap() {
        // Over the measured run, keystroke by keystroke. On yokohama.docx and
        // nda.docx every keystroke takes a band and keeps the completed
        // pagination. On contract.docx the fourth keystroke wraps the edited
        // paragraph onto a second line: the band is rejected as unstable and
        // the page is repainted. The keystrokes after it take bands again, but
        // the bounded pass that grew the paragraph left the layout incomplete,
        // so the completed pagination is no longer offered for reuse.
        for (name, wrapping_keystroke) in [
            ("yokohama.docx", None),
            ("nda.docx", None),
            ("contract.docx", Some(4)),
        ] {
            let options = options("keystroke-band");
            let mut typing =
                BandTyping::open(&fixture(name), &options, renderer().unwrap()).unwrap();
            let lines_before = paragraph_extent(&typing.renderer, typing.paragraph)
                .unwrap()
                .0;
            let mut expected_bands = 0;
            for keystroke in 1..=KEYSTROKES {
                typing.type_one(&options).unwrap();
                let wrapped = wrapping_keystroke.is_some_and(|wrap| keystroke >= wrap);
                if wrapping_keystroke != Some(keystroke) {
                    expected_bands += 1;
                }
                let (eligible, taken, fallbacks) = typing.renderer.band_redraw_stats();
                assert_eq!(
                    taken, expected_bands,
                    "{name}: keystroke {keystroke} (eligible {eligible}, fallbacks {fallbacks:?})"
                );
                assert!(
                    !typing.renderer.band_redraw_enabled(),
                    "{name}: band left on"
                );
                assert_eq!(
                    typing.renderer.last_completed_pagination_edit_preserved(),
                    !wrapped,
                    "{name}: keystroke {keystroke} completed pagination reuse"
                );
                let lines = paragraph_extent(&typing.renderer, typing.paragraph)
                    .unwrap()
                    .0;
                assert_eq!(
                    lines,
                    lines_before + usize::from(wrapped),
                    "{name}: keystroke {keystroke} line count"
                );
            }
        }
    }

    #[test]
    fn plain_keystroke_workload_never_takes_a_band() {
        // The frozen `keystroke` cases measure the full-page fallback; this is
        // what makes `keystroke-band` a separate measurement.
        let bytes = fixture("yokohama.docx");
        let options = options("keystroke");
        let mut model = load(&bytes, &options).unwrap();
        let mut renderer = renderer().unwrap();
        render_cold(&mut renderer, &model, &options).unwrap();
        let paragraph = keystroke_paragraph(&renderer).unwrap();
        let mut executor = CommandExecutor::new();
        for _ in 0..3 {
            type_one(
                &mut renderer,
                &mut model,
                paragraph,
                &options,
                &mut executor,
            )
            .unwrap();
        }
        assert_eq!(bands_taken(&renderer), 0);
    }

    #[test]
    fn band_workload_draws_the_edited_paragraph() {
        // A band that clipped away the edit, or painted a stale paragraph,
        // would still count as taken. The painter sees the inserted text, and
        // less of everything else than a page repaint draws.
        let bytes = fixture("yokohama.docx");
        let edited = "あ".repeat(3);

        let page_options = options("keystroke");
        let mut model = load(&bytes, &page_options).unwrap();
        let host = PerfHost::new().unwrap();
        let painter = host.painter.clone();
        let mut renderer = CanvasRenderer::with_host(Box::new(host), false).unwrap();
        render_cold(&mut renderer, &model, &page_options).unwrap();
        assert!(
            !painter.take_drawn_text().contains(&edited),
            "the fixture already shows the inserted text"
        );
        let paragraph = keystroke_paragraph(&renderer).unwrap();
        let mut executor = CommandExecutor::new();
        for _ in 0..3 {
            painter.take_drawn_text();
            type_one(
                &mut renderer,
                &mut model,
                paragraph,
                &page_options,
                &mut executor,
            )
            .unwrap();
        }
        let page = painter.take_drawn_text();
        assert!(page.contains(&edited), "page repaint must draw the edit");

        let band_options = options("keystroke-band");
        let host = PerfHost::new().unwrap();
        let painter = host.painter.clone();
        let renderer = CanvasRenderer::with_host(Box::new(host), false).unwrap();
        let mut typing = BandTyping::open(&bytes, &band_options, renderer).unwrap();
        assert_eq!(typing.paragraph, paragraph);
        assert!(!painter.take_drawn_text().contains(&edited));
        for _ in 0..3 {
            painter.take_drawn_text();
            typing.type_one(&band_options).unwrap();
        }
        let band = painter.take_drawn_text();
        assert_eq!(bands_taken(&typing.renderer), 3);
        assert!(band.contains(&edited), "band repaint must draw the edit");
        assert!(
            band.chars().count() < page.chars().count(),
            "a band repaint draws less than the page: {} vs {}",
            band.chars().count(),
            page.chars().count()
        );
    }
}
