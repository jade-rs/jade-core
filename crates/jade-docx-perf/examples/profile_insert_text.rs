//! Time insertion plus the visible page's layout and draw-command generation.
//! Initial loading, initial layout, Undo and its repaint are outside the timer.
//! The no-op painter does not include browser/Skia rasterization.
//!
//! cargo run -p jade-docx-perf --example profile_insert_text --profile profiling
//! -- <fixture.docx> [iterations] [warmup]
use futures::executor::block_on;
use jade_docx_editor::{
    canvas_renderer::CanvasRenderer,
    commands::{CommandContext, CommandExecutor, HistoryRenderFootprint},
    headless_api::apply_headless_commands_with_executor_to_document,
    word_document::WordDocument,
    Options,
};
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use jade_docx_perf::{PerfHost, PerfPainter};
use std::time::Instant;

fn repaint(
    renderer: &mut CanvasRenderer<PerfPainter>,
    model: &WordDocument,
    options: &Options,
    paragraph: usize,
) {
    renderer.prepare_incremental_repaint(paragraph, 1);
    renderer
        .render_bounded_to_page_with_parts(
            model.document.as_ref().unwrap().clone(),
            options,
            model.numbering_part.as_ref(),
            model.header_lines.as_deref(),
            model.footer_lines.as_deref(),
            1,
        )
        .expect("repaint");
    assert_eq!(renderer.take_render_error(), None, "render error");
}

// Keep this frame so a sampling profile can exclude setup and Undo.
#[inline(never)]
fn measure_insert_text(
    renderer: &mut CanvasRenderer<PerfPainter>,
    context: &mut CommandContext<'_>,
    executor: &mut CommandExecutor,
    request: &HeadlessEditRequest,
    options: &Options,
    paragraph: usize,
) -> (u128, u128, u128) {
    let started = Instant::now();
    let old_height = renderer.cached_paragraph_height(paragraph);
    let recorded = executor.history_revision();
    let text_edit_cache = renderer.local_text_edit_cache();
    renderer.drop_document_cache();
    *context.selection = apply_headless_commands_with_executor_to_document(
        context.document,
        executor,
        *context.selection,
        request,
    )
    .expect("insert text");
    let edited = Instant::now();
    // Match NativeDocument's single-paragraph typing path, including the
    // command's actual footprint rather than assuming every InsertText is local.
    if executor.history_revision() != recorded
        && executor.next_undo_render_footprint()
            == Some(HistoryRenderFootprint::SingleParagraph { index: paragraph })
    {
        renderer.restore_local_text_edit_cache(text_edit_cache, context.document, paragraph);
        if let Some(height) = old_height {
            renderer.prepare_completed_pagination_edit(paragraph, height);
        }
    }
    repaint(renderer, context.document, options, paragraph);
    let finished = Instant::now();
    (
        finished.duration_since(started).as_nanos(),
        edited.duration_since(started).as_nanos(),
        finished.duration_since(edited).as_nanos(),
    )
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let fixture = args
        .get(1)
        .expect("usage: profile_insert_text <fixture> [iterations] [warmup]");
    let iterations: usize = args.get(2).map_or(1000, |value| value.parse().unwrap());
    let warmup: usize = args.get(3).map_or(100, |value| value.parse().unwrap());
    let bytes = std::fs::read(fixture).expect("read fixture");
    let mut options = Options::default();
    options.show_formatting_marks = false;
    let mut model = block_on(WordDocument::load(&bytes, &options)).expect("load fixture");
    let mut renderer = CanvasRenderer::with_host(Box::new(PerfHost::new().unwrap()), false)
        .expect("create renderer");
    renderer
        .render_with_numbering_rc(
            model.document.as_ref().unwrap().clone(),
            &options,
            model.numbering_part.as_ref(),
            model.header_lines.as_deref(),
            model.footer_lines.as_deref(),
            None,
        )
        .expect("initial layout");
    let paragraph = (6..model.paragraph_count())
        .chain(0..model.paragraph_count().min(6))
        .find(|&index| {
            renderer.cached_paragraph_start_page(index) == Some(1)
                && model.paragraph(index).is_some_and(|p| !p.runs.is_empty())
        })
        .expect("editable paragraph on page one");
    let request = HeadlessEditRequest {
        commands: vec![HeadlessCommand::InsertText {
            paragraph,
            offset: 0,
            text: "あ".into(),
        }],
    };
    let mut executor = CommandExecutor::new();
    let mut selection = None;
    let mut total_ns = Vec::with_capacity(iterations);
    let mut edit_ns = Vec::with_capacity(iterations);
    let mut repaint_ns = Vec::with_capacity(iterations);
    let mut first_ns = (0, 0, 0);
    let mut first_pagination_preserved = false;
    for iteration in 0..warmup + iterations {
        let mut context = CommandContext::new(&mut model, &mut selection);
        let (total, edit, repaint_time) = measure_insert_text(
            &mut renderer,
            &mut context,
            &mut executor,
            &request,
            &options,
            paragraph,
        );
        if iteration == 0 {
            first_ns = (total, edit, repaint_time);
            first_pagination_preserved = renderer.last_completed_pagination_edit_preserved();
        }
        if iteration >= warmup {
            total_ns.push(total);
            edit_ns.push(edit);
            repaint_ns.push(repaint_time);
        }
        renderer.drop_document_cache();
        assert!(executor.undo(&mut context).expect("undo"));
        repaint(&mut renderer, &model, &options, paragraph);
    }
    println!(
        "{{\"paragraph\":{paragraph},\"iterations\":{iterations},\"warmup\":{warmup},\"first_ns\":[{},{},{}],\"first_pagination_preserved\":{first_pagination_preserved},\"total_ns\":{total_ns:?},\"edit_ns\":{edit_ns:?},\"repaint_ns\":{repaint_ns:?}}}",
        first_ns.0, first_ns.1, first_ns.2,
    );
}
