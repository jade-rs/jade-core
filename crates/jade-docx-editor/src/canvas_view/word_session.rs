//! The browser view as a [`WordSession`], so the Word models in
//! [`crate::word`] — styles, lists, search, outline, compare, AutoCorrect and
//! the rest of jade-office's ribbon — run in the IDE as they run in the
//! native and web apps.
//!
//! A model action runs on a detached session over the document: the view's
//! selection and what the last layout knew (cells, tables, pages, links) go
//! in, the action's commands run on a private executor, and the whole action
//! is one [`SnapshotEditCommand`] — one undo step, as one OK in a Word
//! dialog is — after which the view repaints the document.

use super::*;
use crate::canvas_renderer::HyperlinkRegion;
use crate::commands::snapshot_edit::SnapshotEditCommand;
use crate::word::{CellLayout, WordSession};
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// What the last layout knew, for the models that read it.
#[derive(Clone, Default)]
pub(super) struct LayoutFacts {
    cells: Vec<CellLayout>,
    cell_of_paragraph: HashMap<usize, usize>,
    table_orders: HashMap<usize, usize>,
    hyperlinks: Vec<HyperlinkRegion>,
    page_count: Option<usize>,
    paragraph_pages: HashMap<usize, usize>,
    /// `CanvasRenderer::displayed_page_labels`, page 1 first.
    page_labels: Vec<crate::fields::PageLabel>,
    /// `CanvasRenderer::has_page_labels`.
    has_page_labels: bool,
    /// Whether the layout reached the end of the document.
    layout_complete: bool,
}

/// Where an action left the selection, and what it returned.
type Outcome<T> = (Option<SelectionRange>, T);

/// A [`WordSession`] over a document taken out of the view for one action.
pub(super) struct DetachedSession<'a> {
    document: &'a mut WordDocument,
    selection: Option<SelectionRange>,
    executor: CommandExecutor,
    facts: Rc<LayoutFacts>,
    /// Cells a model selected (the first and last cell ids): the view's own
    /// cell selection, put in by [`OffscreenDocumentView::select_with_word_session`].
    cells_selected: Option<(usize, usize)>,
}

impl WordSession for DetachedSession<'_> {
    fn word_document(&self) -> &WordDocument {
        self.document
    }

    fn selection_range(&self) -> Option<SelectionRange> {
        self.selection
    }

    fn set_selection_range(&mut self, range: Option<SelectionRange>) {
        self.selection = range;
    }

    fn apply_command(
        &mut self,
        command: HeadlessCommand,
        keep_selection: bool,
    ) -> Result<(), String> {
        self.apply_commands(vec![command], keep_selection)
    }

    fn apply_commands(
        &mut self,
        commands: Vec<HeadlessCommand>,
        keep_selection: bool,
    ) -> Result<(), String> {
        let kept = self.selection;
        let after = crate::headless_api::apply_headless_commands_with_executor_to_document(
            self.document,
            &mut self.executor,
            self.selection,
            &HeadlessEditRequest { commands },
        )
        .map_err(|error| error.to_string())?;
        self.selection = if keep_selection { kept } else { after };
        Ok(())
    }

    fn execute_command(
        &mut self,
        command: Box<dyn Command>,
        keep_selection: bool,
    ) -> Result<(), String> {
        let kept = self.selection;
        let mut selection = self.selection;
        {
            let mut ctx = CommandContext::new(self.document, &mut selection);
            self.executor
                .execute(command, &mut ctx)
                .map_err(|error| error.to_string())?;
        }
        self.selection = if keep_selection { kept } else { selection };
        Ok(())
    }

    fn cell_id_for_paragraph(&self, paragraph: usize) -> Option<usize> {
        self.facts.cell_of_paragraph.get(&paragraph).copied()
    }

    fn cell_layout(&self, cell_id: usize) -> Option<&CellLayout> {
        self.facts.cells.get(cell_id)
    }

    fn cell_layouts(&self) -> &[CellLayout] {
        &self.facts.cells
    }

    fn table_order_for(&self, table_id: usize) -> Option<usize> {
        self.facts.table_orders.get(&table_id).copied()
    }

    fn hyperlink_regions(&self) -> Vec<HyperlinkRegion> {
        self.facts.hyperlinks.clone()
    }

    fn select_cells(&mut self, anchor_cell: usize, focus_cell: usize) -> Result<(), String> {
        // The view takes it after the action (`select_with_word_session`).
        self.cells_selected = Some((anchor_cell, focus_cell));
        Ok(())
    }

    fn page_count(&self) -> Option<usize> {
        self.facts.page_count
    }

    fn paragraph_page(&self, paragraph: usize) -> Option<usize> {
        self.facts.paragraph_pages.get(&paragraph).copied()
    }

    fn page_label(&self, page: usize) -> Option<crate::fields::PageLabel> {
        page.checked_sub(1)
            .and_then(|index| self.facts.page_labels.get(index))
            .cloned()
    }

    fn has_page_labels(&self) -> bool {
        self.facts.has_page_labels
    }

    /// The facts were read before the action ran, and the session cannot
    /// lay the document out itself: a host that runs a page-number command
    /// completes the layout first (`OffscreenDocumentView::complete_layout`).
    fn complete_layout(&mut self) -> bool {
        self.facts.layout_complete
    }
}

/// The most extra paints one host pass makes to settle what it drew, as the
/// renderer counts them: it raises a NUMPAGES or STYLEREF flag on at most
/// `MAX_FLAGGED_PAGE_TOTAL_PASSES` (2) passes in a row, so a total or
/// heading with no right value (one that moves the page it counts) stops
/// all the same. The native host draws its window once more at open and
/// leaves the pages stale after an edit; the browser has no idle paint of
/// its own, so it draws them here.
pub(super) const MAX_SETTLE_PAINTS: usize = 2;

/// What [`settle_drawn_pages`] did.
#[derive(Debug, Default)]
pub(super) struct Settled {
    /// Pages (0-based) drawn with a NUMPAGES total or a STYLEREF the pass
    /// proved wrong. The main thread fetches their bitmaps again; those in
    /// the visible range are drawn again already.
    pub(super) pages: std::collections::BTreeSet<usize>,
    /// The extra paints made, at most [`MAX_SETTLE_PAINTS`].
    pub(super) paints: usize,
}

/// The pages the pass just run drew a NUMPAGES or SECTIONPAGES total or a
/// STYLEREF on other than it ended with — every page for a total, as the
/// native host leaves them stale — and clears both flags. Empty, and
/// allocation-free, after a pass that drew them right, as every keystroke's
/// does.
#[inline]
fn take_wrong_pages<P>(renderer: &mut CanvasRenderer<P>) -> std::collections::BTreeSet<usize>
where
    P: jade_painter::Painter,
    P::Error: jade_canvas_painter::PainterError,
{
    let totals = renderer.take_page_totals_changed();
    if !totals && !renderer.has_style_reference_wrong_pages() {
        return std::collections::BTreeSet::new();
    }
    take_wrong_pages_cold(renderer, totals)
}

#[cold]
#[inline(never)]
fn take_wrong_pages_cold<P>(
    renderer: &mut CanvasRenderer<P>,
    totals: bool,
) -> std::collections::BTreeSet<usize>
where
    P: jade_painter::Painter,
    P::Error: jade_canvas_painter::PainterError,
{
    let count = renderer.page_count();
    let mut pages = if renderer.has_style_reference_wrong_pages() {
        renderer.take_style_reference_wrong_pages()
    } else {
        std::collections::BTreeSet::new()
    };
    if totals {
        pages.extend(0..count);
    }
    pages.retain(|&page| page < count);
    pages
}

/// Whole-page dirty rectangles for `pages` (0-based), as an edit's payload
/// reports a repainted page.
pub(super) fn whole_page_rects<P>(
    renderer: &CanvasRenderer<P>,
    pages: &std::collections::BTreeSet<usize>,
) -> Vec<DirtyRect>
where
    P: jade_painter::Painter,
    P::Error: jade_canvas_painter::PainterError,
{
    pages
        .iter()
        .filter_map(|&page| {
            let (width, height) = renderer.page_dimensions(page)?;
            Some(DirtyRect {
                page,
                x: 0.0,
                y: renderer.page_top_y(page),
                width,
                height,
            })
        })
        .collect()
}

/// Draws again the pages the pass just run drew a NUMPAGES total or a
/// STYLEREF on other than it ended with, as the native host's
/// `redraw_wrong_page_totals` does: the renderer shows the settled values
/// the second time. Only pages in the visible range are drawn; the rest
/// draw the settled values whenever they are drawn next. Each wrong page is
/// left a whole-page dirty rectangle, for the payload, and listed.
///
/// A pass that drew them right — typing that leaves the page count as it
/// was — costs two flag reads and paints nothing.
pub(super) fn settle_drawn_pages<P>(renderer: &mut CanvasRenderer<P>) -> Result<Settled, JsValue>
where
    P: jade_painter::Painter,
    P::Error: jade_canvas_painter::PainterError,
{
    let mut settled = Settled::default();
    loop {
        let wrong = take_wrong_pages(renderer);
        if wrong.is_empty() {
            break;
        }
        for rect in whole_page_rects(renderer, &wrong) {
            renderer.mark_dirty_rect(rect);
        }
        let visible = renderer.visible_page_range();
        let in_window = |page: usize| {
            visible
                .as_ref()
                .is_none_or(|range| range.contains(&(page + 1)))
        };
        // A page outside the window is not drawn here: its pixels are left
        // stale, so the next fetch of its bitmap draws it with the settled
        // values instead of handing out the old ones.
        for &page in wrong.iter().filter(|&&page| !in_window(page)) {
            renderer.forget_page_pixels(page);
        }
        let any_drawn = wrong.iter().any(|&page| in_window(page));
        settled.pages.extend(wrong);
        // Past the cap the pages stay as the last pass drew them: their
        // flags are taken above, so the next pass does not paint for them.
        if !any_drawn || settled.paints == MAX_SETTLE_PAINTS {
            break;
        }
        renderer.skip_change_detection_once();
        renderer.rerender_with_cached_state()?;
        settled.paints += 1;
    }
    Ok(settled)
}

/// Leaves `pages` (0-based) dirty for the next payload to report, for a
/// caller that settled them but reports no dirty regions itself.
pub(super) fn keep_for_next_payload<P>(
    renderer: &mut CanvasRenderer<P>,
    pages: &std::collections::BTreeSet<usize>,
) where
    P: jade_painter::Painter,
    P::Error: jade_canvas_painter::PainterError,
{
    for rect in whole_page_rects(renderer, pages) {
        renderer.mark_dirty_rect(rect);
    }
}

/// The dirty regions of a lazy-mode edit's payload, which the main thread
/// fetches bitmaps for on demand: the pass's own, and the pages
/// [`settle_drawn_pages`] draws again. Clears them.
pub(super) fn take_lazy_payload_dirty_regions<P>(
    renderer: &mut CanvasRenderer<P>,
) -> Result<Vec<DirtyRect>, JsValue>
where
    P: jade_painter::Painter,
    P::Error: jade_canvas_painter::PainterError,
{
    settle_drawn_pages(renderer)?;
    renderer.coalesce_dirty_regions();
    let dirty_regions = renderer.dirty_regions();
    renderer.clear_dirty_regions();
    Ok(dirty_regions)
}

/// [`OffscreenDocumentView::extend_layout`]'s layout: lays the document out,
/// without drawing, through `end_page` (1-based; `None`: to its end),
/// resuming from the last valid page-top snapshot where one qualifies, and
/// then settles what earlier passes drew ([`settle_drawn_pages`]): the pass
/// that reaches the last page is the one that learns the page count. The
/// side tables are read from `word_doc` in place.
pub(super) fn extend_renderer_layout<P>(
    renderer: &mut CanvasRenderer<P>,
    end_page: Option<usize>,
    options: &Options,
    word_doc: &WordDocument,
) -> Result<Settled, JsValue>
where
    P: jade_painter::Painter,
    P::Error: jade_canvas_painter::PainterError,
{
    let previous_visible_range = renderer.visible_page_range();
    let previous_limit = renderer.layout_page_limit();
    renderer.set_layout_page_limit(end_page);
    // Preserve the current viewport window while extending layout in the
    // background. Expanding the visible range to `1..=end_page` makes later
    // edit renders treat the whole laid-out prefix as visible, which causes
    // avoidable tail bitmap churn.
    renderer.set_visible_page_range(previous_visible_range);
    // Resume the layout-only walk from the last valid page-top snapshot
    // instead of re-walking every laid-out page; falls back to the
    // from-start walk when no snapshot qualifies.
    renderer.prepare_layout_extension_seek(previous_limit, end_page.unwrap_or(usize::MAX), options);
    if let Some(document) = word_doc.document.as_ref() {
        renderer
            .render_layout_with_numbering_rc(
                Rc::clone(document),
                options,
                word_doc.numbering_part.as_ref(),
                word_doc.header_lines.as_deref(),
                word_doc.footer_lines.as_deref(),
                Some(&word_doc.comments),
            )
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
    }
    // Layout expansion should not leave dirty regions that can slow
    // subsequent renders; the caller reports the settled pages itself.
    renderer.clear_dirty_regions();
    let settled = settle_drawn_pages(renderer)?;
    renderer.clear_dirty_regions();
    Ok(settled)
}

/// [`OffscreenDocumentView::complete_layout`]'s layout: lays the document
/// out through its end when the last layout stopped short, keeping the
/// host's lazy-layout limit, and settles what earlier passes drew. Returns
/// whether the layout is complete, and what was settled.
pub(super) fn complete_renderer_layout<P>(
    renderer: &mut CanvasRenderer<P>,
    options: &Options,
    word_doc: &WordDocument,
) -> Result<(bool, Settled), JsValue>
where
    P: jade_painter::Painter,
    P::Error: jade_canvas_painter::PainterError,
{
    if renderer.last_layout_complete() {
        return Ok((true, Settled::default()));
    }
    let previous_limit = renderer.layout_page_limit();
    let settled = extend_renderer_layout(renderer, None, options, word_doc)?;
    let complete = renderer.last_layout_complete();
    // The host's lazy-layout limit stays its own.
    renderer.set_layout_page_limit(previous_limit);
    Ok((complete, settled))
}

impl OffscreenDocumentView {
    /// What the last layout knew, for a detached session.
    pub(super) fn layout_facts(&self) -> LayoutFacts {
        let renderer = self.renderer.borrow();
        let cells = renderer.cell_layouts_debug_ref().to_vec();
        let mut cell_of_paragraph = HashMap::new();
        let mut table_orders = HashMap::new();
        for (id, cell) in cells.iter().enumerate() {
            for paragraph in &cell.paragraphs {
                cell_of_paragraph.entry(*paragraph).or_insert(id);
            }
            if let Some(order) = renderer.table_order_for(cell.table_id) {
                table_orders.insert(cell.table_id, order);
            }
        }
        let paragraph_count = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .map_or(0, WordDocument::paragraph_count);
        let paragraph_pages = (0..paragraph_count)
            .filter_map(|index| {
                renderer
                    .cached_paragraph_start_page(index)
                    .map(|page| (index, page))
            })
            .collect();
        LayoutFacts {
            cells,
            cell_of_paragraph,
            table_orders,
            hyperlinks: renderer.hyperlink_regions().to_vec(),
            page_count: Some(renderer.page_count()).filter(|count| *count > 0),
            paragraph_pages,
            page_labels: renderer.displayed_page_labels(),
            has_page_labels: renderer.has_page_labels(),
            layout_complete: renderer.last_layout_complete(),
        }
    }

    /// Lays the document out through its end, without drawing, when the last
    /// layout stopped short (a lazily opened document lays out only what the
    /// viewport needs): what F9 and a table of contents' page numbers read
    /// first, so every page has its number. Returns whether the layout is
    /// complete afterwards.
    pub(super) fn complete_layout(&self) -> Result<bool, JsValue> {
        if self.renderer.borrow().last_layout_complete() {
            return Ok(true);
        }
        let (complete, metadata) = {
            let state = self.state.borrow();
            let Some(word_doc) = state.word_doc.as_ref() else {
                return Ok(false);
            };
            let mut renderer = self.renderer.borrow_mut();
            // The command that follows repaints, and its render waits for
            // the images this pass started, as `ensureLayoutThroughPage`'s
            // caller waits for them.
            let (complete, settled) =
                complete_renderer_layout(&mut renderer, &state.options, word_doc)?;
            // The pages the pass proved wrong go out with the payload of the
            // command that follows.
            keep_for_next_payload(&mut renderer, &settled.pages);
            (complete, renderer.table_resize_metadata())
        };
        self.state.borrow_mut().table_resize = metadata;
        Ok(complete)
    }

    /// Reads from the document as a session: what a dialog opens on.
    pub(super) fn read_word_session<T>(
        &self,
        read: impl FnOnce(&DetachedSession<'_>) -> T,
    ) -> Result<T, JsValue> {
        let facts = Rc::new(self.layout_facts());
        let selection = self.current_selection();
        let mut state = self.state.borrow_mut();
        let document = state
            .word_doc
            .as_mut()
            .ok_or_else(|| JsValue::from_str("No document loaded"))?;
        let session = DetachedSession {
            document,
            selection: Some(selection),
            executor: CommandExecutor::new(),
            facts,
            cells_selected: None,
        };
        Ok(read(&session))
    }

    /// Runs a model action that only selects — 選択 ▸ セル/行/列/表, a word,
    /// a sentence — and puts the selection it made, cells included, in the
    /// view. Nothing in the document changes and no undo step is recorded.
    pub(super) fn select_with_word_session(
        &self,
        action: impl FnOnce(&mut DetachedSession<'_>) -> Result<(), String>,
    ) -> Result<(), JsValue> {
        let facts = Rc::new(self.layout_facts());
        let selection = self.current_selection();
        let (selection, cells) = {
            let mut state = self.state.borrow_mut();
            let document = state
                .word_doc
                .as_mut()
                .ok_or_else(|| JsValue::from_str("No document loaded"))?;
            let mut session = DetachedSession {
                document,
                selection: Some(selection),
                executor: CommandExecutor::new(),
                facts,
                cells_selected: None,
            };
            action(&mut session).map_err(|error| JsValue::from_str(&error))?;
            (session.selection, session.cells_selected)
        };
        let range = {
            let mut renderer = self.renderer.borrow_mut();
            match cells.and_then(|(anchor, focus)| {
                Some((anchor, focus, renderer.cell_selection_range(anchor, focus)?))
            }) {
                Some((anchor, focus, range)) => {
                    renderer.set_selection_range(Some(range));
                    renderer.set_cell_selection(anchor, focus);
                    Some(range)
                }
                None => {
                    renderer.clear_cell_selection();
                    renderer.set_selection_range(selection);
                    selection
                }
            }
        };
        if let Some(range) = range {
            self.state.borrow_mut().selection = Some(range);
        }
        Ok(())
    }

    /// Runs one Word model action — a dialog's OK, a ribbon button — as one
    /// undo step and repaints the document. `name` starts with `word_`, which
    /// tells the repaint to read the styles again. `action` returns its own result,
    /// which comes back with the repaint payload as `result`.
    pub(super) fn run_word_session<T: serde::Serialize + 'static>(
        &self,
        name: &'static str,
        action: impl FnOnce(&mut DetachedSession<'_>) -> Result<T, String> + 'static,
    ) -> Result<JsValue, JsValue> {
        let facts = Rc::new(self.layout_facts());
        let selection = self.current_selection();
        // Models that edit the package (themes, styles, the glossary) need
        // the round-trip XML a lazily opened document builds on its first
        // export.
        if let Some(document) = self.state.borrow_mut().word_doc.as_mut() {
            document
                .ensure_roundtrip_xml()
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
        }
        let action = RefCell::new(Some(action));
        let outcome: Rc<RefCell<Option<Outcome<T>>>> = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&outcome);
        // The document as the action left it: a redo restores it rather
        // than running the action (which may read the clock or the layout)
        // again.
        let after: RefCell<Option<crate::word_document::WordDocumentSnapshot>> = RefCell::new(None);
        let payload = self.apply_document_command(move |_| {
            Box::new(SnapshotEditCommand::new(name, move |document| {
                let Some(action) = action.borrow_mut().take() else {
                    if let Some(snapshot) = after.borrow().clone() {
                        document.restore_snapshot(snapshot);
                    }
                    return Ok(());
                };
                let mut session = DetachedSession {
                    document,
                    selection: Some(selection),
                    executor: CommandExecutor::new(),
                    facts: Rc::clone(&facts),
                    cells_selected: None,
                };
                let result = action(&mut session)?;
                *sink.borrow_mut() = Some((session.selection, result));
                *after.borrow_mut() = document.snapshot_including_package();
                Ok(())
            }))
        })?;
        let Some((selection, result)) = outcome.borrow_mut().take() else {
            return Ok(payload);
        };
        if let Some(range) = selection {
            self.state.borrow_mut().selection = Some(range);
            self.renderer.borrow_mut().set_selection_range(Some(range));
        }
        if payload.is_object() {
            let value = result
                .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
                .unwrap_or(JsValue::NULL);
            let _ = Reflect::set(&payload, &JsValue::from_str("result"), &value);
        }
        Ok(payload)
    }
}

/// The view shows markup itself ([`OffscreenDocumentView::set_markup_view`]);
/// a detached session only reads what the markup models read.
impl crate::word::markup::WordMarkupDisplay for DetachedSession<'_> {
    fn set_revision_display(
        &mut self,
        _markup: bool,
        _original: bool,
        _view: crate::revision_view::RevisionView,
    ) -> Result<bool, String> {
        Err("markup is the view's own".to_string())
    }
}

#[cfg(test)]
#[path = "word_session_settle_tests.rs"]
mod settle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::PageLabel;

    /// The detached session hands out the last layout's label for each
    /// physical page, and none for a page the layout did not have.
    #[test]
    fn page_label_reads_the_layouts_labels() {
        let mut document = futures::executor::block_on(WordDocument::load(
            &crate::word::new::blank_document(crate::word::new::Blank::Japanese),
            &Options::default(),
        ))
        .expect("a blank document loads");
        let labels = vec![
            PageLabel {
                value: 1,
                text: "1".into(),
            },
            PageLabel {
                value: 1,
                text: "i".into(),
            },
        ];
        let session = DetachedSession {
            document: &mut document,
            selection: None,
            executor: CommandExecutor::new(),
            facts: Rc::new(LayoutFacts {
                page_count: Some(2),
                page_labels: labels.clone(),
                ..LayoutFacts::default()
            }),
            cells_selected: None,
        };
        assert_eq!(session.page_label(1), Some(labels[0].clone()));
        assert_eq!(session.page_label(2), Some(labels[1].clone()));
        assert_eq!(session.page_label(0), None);
        assert_eq!(session.page_label(3), None);
    }
}
