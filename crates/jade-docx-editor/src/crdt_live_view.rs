// Included in canvas_view so the replica shares the native editor's owner.
#[wasm_bindgen]
impl OffscreenDocumentView {
    #[wasm_bindgen(js_name = beginCrdt)]
    pub fn begin_crdt(
        &self,
        document: String,
        baseline: String,
        actor: String,
        snapshot: Option<Vec<u8>>,
    ) -> Result<(), JsValue> {
        if actor.trim().is_empty() {
            return Err(JsValue::from_str("CRDT actor is required"));
        }
        let mut state = self.state.borrow_mut();
        let doc = state
            .word_doc
            .as_mut()
            .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
        doc.ensure_roundtrip_xml()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let bytes = doc
            .export_docx()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let live = match snapshot {
            Some(snapshot) => crate::crdt_live::LiveDocument::open(
                &document,
                &baseline,
                &bytes,
                &snapshot,
                doc.raw_document.as_ref(),
            ),
            None => crate::crdt_live::LiveDocument::create(
                &document,
                &baseline,
                &bytes,
                doc.raw_document.as_ref(),
            ),
        }
        .map(crate::crdt_live::configure)
        .map_err(|e| JsValue::from_str(&e))?;
        state.crdt = Some(live);
        state.crdt_actor = actor;
        state.crdt_checkpoint = None;
        state.crdt_projection_error = None;
        Ok(())
    }

    #[wasm_bindgen(js_name = beginCrdtEdit)]
    pub fn begin_crdt_edit(&self, replacement: bool, incremental: bool) -> Result<(), JsValue> {
        let mut state = self.state.borrow_mut();
        if state.crdt.is_none() {
            return Ok(());
        }
        if let Some(error) = &state.crdt_projection_error {
            return Err(JsValue::from_str(error));
        }
        if state.crdt_checkpoint.is_some() {
            return Err(JsValue::from_str("nested CRDT edit"));
        }
        if let Some(main) = state
            .word_doc
            .as_mut()
            .and_then(|d| d.raw_document.as_mut())
        {
            if main.changed_elements().is_none() {
                main.track_changes();
            }
        }
        state.crdt_checkpoint = state.word_doc.clone();
        if !incremental {
            state.crdt.as_mut().unwrap().require_full_capture();
        }
        if replacement {
            state.crdt.as_mut().unwrap().reset_binding();
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = commitCrdtEdit)]
    pub fn commit_crdt_edit(&self) -> Result<(), JsValue> {
        let mut state = self.state.borrow_mut();
        if state.crdt.is_none() {
            return Ok(());
        }
        // Borrow disjoint fields directly: cloning the main XML defeats scoped capture.
        let state = &mut *state;
        let doc = state
            .word_doc
            .as_mut()
            .ok_or_else(|| JsValue::from_str("document is not loaded"))?;
        doc.ensure_roundtrip_xml()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let live = state.crdt.as_mut().unwrap();
        let captured = match doc.raw_document.as_ref() {
            Some(main) => live.capture_changed(&state.crdt_actor, main),
            None => Ok(false),
        }
        .map_err(|e| JsValue::from_str(&e))?;
        if !captured {
            let bytes = doc
                .export_docx()
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            live.capture(&state.crdt_actor, &bytes, doc.raw_document.as_ref(), vec![])
                .map_err(|e| JsValue::from_str(&e))?;
        }
        if let Some(main) = doc.raw_document.as_mut() {
            main.track_changes();
        }
        state.crdt_checkpoint = None;
        Ok(())
    }

    /// Acknowledge the native edit without waiting for a replica capture.
    #[wasm_bindgen(js_name = finishCrdtEdit)]
    pub fn finish_crdt_edit(&self) {
        self.state.borrow_mut().crdt_checkpoint = None;
    }

    /// Only the current command failed; earlier acknowledged edits may be pending.
    /// Their binding is unchanged, so rebinding to the older replica is incorrect.
    #[wasm_bindgen(js_name = abortPendingCrdtEdit)]
    pub fn abort_pending_crdt_edit(&self) -> Result<(), JsValue> {
        let restored = {
            let mut state = self.state.borrow_mut();
            if let Some(doc) = state.crdt_checkpoint.take() {
                state.word_doc = Some(doc);
                state.command_executor = CommandExecutor::new();
                true
            } else {
                false
            }
        };
        if restored {
            self.rerender_document()?;
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = abortCrdtEdit)]
    pub fn abort_crdt_edit(&self) -> Result<(), JsValue> {
        let restored = {
            let mut state = self.state.borrow_mut();
            if let Some(doc) = state.crdt_checkpoint.take() {
                let bytes = doc
                    .export_docx()
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                state
                    .crdt
                    .as_mut()
                    .unwrap()
                    .rebind(&bytes, doc.raw_document.as_ref())
                    .map_err(|e| JsValue::from_str(&e))?;
                state.word_doc = Some(doc);
                state.command_executor = CommandExecutor::new();
                true
            } else {
                false
            }
        };
        if restored {
            self.rerender_document()?;
        }
        Ok(())
    }

    /// DOCX, snapshot, facts and frontier are read synchronously from one owner.
    #[wasm_bindgen(js_name = exportCrdt)]
    pub fn export_crdt(&self) -> Result<JsValue, JsValue> {
        let state = self.state.borrow();
        if let Some(error) = &state.crdt_projection_error {
            return Err(JsValue::from_str(error));
        }
        let live = state
            .crdt
            .as_ref()
            .ok_or_else(|| JsValue::from_str("CRDT is not attached"))?;
        let result = Object::new();
        for (key, bytes) in [
            ("docx", live.projected_docx()),
            ("snapshot", live.inner.snapshot()),
            ("version", live.inner.version()),
        ] {
            let bytes = bytes.map_err(|e| JsValue::from_str(&e))?;
            Reflect::set(
                &result,
                &JsValue::from_str(key),
                &js_sys::Uint8Array::from(bytes.as_slice()),
            )?;
        }
        let facts = live.inner.facts().map_err(|e| JsValue::from_str(&e))?;
        Reflect::set(
            &result,
            &JsValue::from_str("facts"),
            &serde::Serialize::serialize(
                &facts,
                &serde_wasm_bindgen::Serializer::json_compatible(),
            )
            .map_err(|e| JsValue::from_str(&e.to_string()))?,
        )?;
        Reflect::set(
            &result,
            &JsValue::from_str("projections"),
            &serde::Serialize::serialize(
                &crate::crdt_live::projections(live).map_err(|e| JsValue::from_str(&e))?,
                &serde_wasm_bindgen::Serializer::json_compatible(),
            )
            .map_err(|e| JsValue::from_str(&e.to_string()))?,
        )?;
        Ok(result.into())
    }

    #[wasm_bindgen(js_name = crdtUpdatesSince)]
    pub fn crdt_updates_since(&self, version: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.state
            .borrow()
            .crdt
            .as_ref()
            .ok_or_else(|| JsValue::from_str("CRDT is not attached"))?
            .inner
            .updates_since(version)
            .map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen(js_name = appendCrdtFacts)]
    pub fn append_crdt_facts(&self, values: JsValue) -> Result<JsValue, JsValue> {
        let values: Vec<serde_json::Value> = serde_wasm_bindgen::from_value(values)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let mut state = self.state.borrow_mut();
        let actor = state.crdt_actor.clone();
        let facts = state
            .crdt
            .as_mut()
            .ok_or_else(|| JsValue::from_str("CRDT is not attached"))?
            .append_values(&actor, values)
            .map_err(|e| JsValue::from_str(&e))?;
        serde::Serialize::serialize(&facts, &serde_wasm_bindgen::Serializer::json_compatible())
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    #[wasm_bindgen(js_name = importCrdtUpdates)]
    pub fn import_crdt_updates(&self, updates: Vec<u8>) -> Promise {
        let view = self.clone();
        future_to_promise(async move {
            let metadata_only = view
                .state
                .borrow_mut()
                .crdt
                .as_mut()
                .ok_or_else(|| JsValue::from_str("CRDT is not attached"))?
                .import_metadata(&updates)
                .map_err(|e| JsValue::from_str(&e))?;
            if metadata_only {
                return view.rerender_after_display_change(|_| {});
            }
            let selection = view.crdt_selection_anchors();
            let revision_view = view
                .state
                .borrow()
                .word_doc
                .as_ref()
                .map(crate::crdt_live::display_state);
            let bytes = view
                .state
                .borrow()
                .crdt
                .as_ref()
                .ok_or_else(|| JsValue::from_str("CRDT is not attached"))?
                .preview_import(&updates)
                .map_err(|e| JsValue::from_str(&e))?;
            // Validate the native projection before changing the replica.
            let options = view.state.borrow().options.clone();
            let mut candidate = WordDocument::load(&bytes, &options)
                .await
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            candidate
                .ensure_roundtrip_xml()
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let exported = candidate
                .export_docx()
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let state = view.state.borrow();
            let live = state.crdt.as_ref().unwrap();
            let mut preview = jade_crdt::Document::open(
                live.document(),
                live.baseline(),
                &live.inner.snapshot().map_err(|e| JsValue::from_str(&e))?,
            )
            .map_err(|e| JsValue::from_str(&e))?;
            preview
                .import_updates(&updates)
                .map_err(|e| JsValue::from_str(&e))?;
            crate::crdt_live::LiveDocument::open(
                live.document(),
                live.baseline(),
                &exported,
                &preview.snapshot().map_err(|e| JsValue::from_str(&e))?,
                candidate.raw_document.as_ref(),
            )
            .map_err(|e| JsValue::from_str(&e))?;
            drop(state);
            view.crdt_load_projection(&bytes).await?;
            {
                let mut state = view.state.borrow_mut();
                let doc = state.word_doc.as_ref().unwrap();
                let exported = doc
                    .export_docx()
                    .map_err(|e| JsValue::from_str(&e.to_string()))?;
                let main = doc.raw_document.clone();
                let live = state.crdt.as_mut().unwrap();
                live.accept_import(&updates)
                    .map_err(|e| JsValue::from_str(&e))?;
                live.rebind(&exported, main.as_ref())
                    .map_err(|e| JsValue::from_str(&e))?;
                state.crdt_projection_error = None;
                if let Some(revision_view) = revision_view {
                    crate::crdt_live::restore_display_state(
                        state.word_doc.as_ref().unwrap(),
                        revision_view,
                    );
                }
            }
            view.restore_crdt_selection(selection);
            view.rerender_after_display_change(|_| {})
        })
    }
}

impl OffscreenDocumentView {
    async fn crdt_load_projection(&self, bytes: &[u8]) -> Result<(), JsValue> {
        // Preserve viewport rendering across history/import. The eager loader
        // snapshots every page, including lazy placeholders with no 2D context.
        let lazy = self.state.borrow().lazy_mode;
        let data = js_sys::Uint8Array::from(bytes);
        JsFuture::from(if lazy {
            self.load_docx_lazy(data)
        } else {
            self.load_docx(data)
        })
        .await?;
        Ok(())
    }
    fn crdt_selection_anchors(
        &self,
    ) -> Option<(
        SelectionRange,
        Option<crate::crdt_live::LivePosition>,
        Option<crate::crdt_live::LivePosition>,
    )> {
        let state = self.state.borrow();
        let doc = state.word_doc.as_ref()?;
        let live = state.crdt.as_ref()?;
        let selection = state.selection?;
        let anchor = |position| {
            let model = doc.rendered_to_model_text_position(position);
            live.position_anchor(doc.paragraph(model.paragraph)?.node_id?.raw(), model.offset)
                .ok()
        };
        Some((selection, anchor(selection.anchor), anchor(selection.focus)))
    }
    fn restore_crdt_selection(
        &self,
        saved: Option<(
            SelectionRange,
            Option<crate::crdt_live::LivePosition>,
            Option<crate::crdt_live::LivePosition>,
        )>,
    ) {
        let Some((fallback, anchor, focus)) = saved else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let Some(doc) = state.word_doc.as_ref() else {
            return;
        };
        let Some(live) = state.crdt.as_ref() else {
            return;
        };
        let resolve = |anchor: Option<crate::crdt_live::LivePosition>, fallback: TextPosition| {
            if let Some((id, offset)) = anchor.and_then(|a| live.resolve_position(&a).ok()) {
                for index in 0..doc.paragraph_count() {
                    if doc
                        .paragraph(index)
                        .and_then(|p| p.node_id)
                        .and_then(|id| live.native_id(id.raw()))
                        == Some(id.as_str())
                    {
                        return doc
                            .model_to_rendered_text_position(TextPosition::new(index, offset));
                    }
                }
            }
            let paragraph = fallback
                .paragraph
                .min(doc.paragraph_count().saturating_sub(1));
            let text = doc
                .paragraph(paragraph)
                .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                .unwrap_or_default();
            let mut offset = fallback.offset.min(text.len());
            while !text.is_char_boundary(offset) {
                offset -= 1;
            }
            TextPosition::new(paragraph, offset)
        };
        let selection = SelectionRange {
            anchor: resolve(anchor, fallback.anchor),
            focus: resolve(focus, fallback.focus),
        };
        state.selection = Some(selection);
        drop(state);
        self.renderer
            .borrow_mut()
            .set_selection_range(Some(selection));
    }
    async fn crdt_history(&self, redo: bool) -> Result<JsValue, JsValue> {
        // Selective Undo changes the replica exactly once. If OOXML cannot be
        // projected, retain that history and the last valid view, block further
        // edits/saves, and allow a later Undo/Redo/import to repair the projection.
        let selection = self.crdt_selection_anchors();
        let revision_view = self
            .state
            .borrow()
            .word_doc
            .as_ref()
            .map(crate::crdt_live::display_state);
        let outcome: Result<(), JsValue> = async {
            let bytes = {
                let mut state = self.state.borrow_mut();
                let actor = state.crdt_actor.clone();
                let live = state.crdt.as_mut().unwrap();
                live.history(&actor, redo)
                    .map_err(|e| JsValue::from_str(&e))?;
                live.projected_docx().map_err(|e| JsValue::from_str(&e))?
            };
            let options = self.state.borrow().options.clone();
            let mut candidate = WordDocument::load(&bytes, &options)
                .await
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            candidate
                .ensure_roundtrip_xml()
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            {
                let state = self.state.borrow();
                let live = state.crdt.as_ref().unwrap();
                crate::crdt_live::LiveDocument::open(
                    live.document(),
                    live.baseline(),
                    &candidate
                        .export_docx()
                        .map_err(|e| JsValue::from_str(&e.to_string()))?,
                    &live.inner.snapshot().map_err(|e| JsValue::from_str(&e))?,
                    candidate.raw_document.as_ref(),
                )
                .map_err(|e| JsValue::from_str(&e))?;
            }
            self.crdt_load_projection(&bytes).await?;
            let mut state = self.state.borrow_mut();
            let doc = state.word_doc.as_ref().unwrap();
            let bytes = doc
                .export_docx()
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let main = doc.raw_document.clone();
            state
                .crdt
                .as_mut()
                .unwrap()
                .rebind(&bytes, main.as_ref())
                .map_err(|e| JsValue::from_str(&e))?;
            if let Some(revision_view) = revision_view {
                crate::crdt_live::restore_display_state(
                    state.word_doc.as_ref().unwrap(),
                    revision_view,
                );
            }
            Ok(())
        }
        .await;
        if let Err(error) = outcome {
            self.state.borrow_mut().crdt_projection_error =
                Some(format!("CRDT projection requires repair: {:?}", error));
            return Err(error);
        }
        self.state.borrow_mut().crdt_projection_error = None;
        self.restore_crdt_selection(selection);
        self.rerender_after_display_change(|_| {})
    }
}
