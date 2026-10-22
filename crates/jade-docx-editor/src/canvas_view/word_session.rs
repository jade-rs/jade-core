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
        }
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
