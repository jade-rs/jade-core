use crate::{js_error, Deck};
use wasm_bindgen::prelude::*;
#[wasm_bindgen]
impl Deck {
    #[wasm_bindgen(js_name = slideLayouts)]
    pub fn slide_layouts(&self, index: usize) -> Result<String, JsValue> {
        serde_json::to_string(
            &jade_pptx_writer::slide_layouts(self.document.bytes(), index).map_err(js_error)?,
        )
        .map_err(js_error)
    }
    #[wasm_bindgen(js_name = validateSlideLayout)]
    pub fn validate_slide_layout(&self, index: usize, layout: &str) -> Result<(), JsValue> {
        jade_pptx_writer::validate_slide_layout(self.document.bytes(), index, layout)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = setSlideLayout)]
    pub fn set_slide_layout(&mut self, index: usize, layout: &str) -> Result<bool, JsValue> {
        self.validate_slide_layout(index, layout)?;
        self.slide_edit(|session| session.set_slide_layout(index, layout))
    }
    #[wasm_bindgen(js_name = validateSlideReset)]
    pub fn validate_slide_reset(&self, index: usize) -> Result<(), JsValue> {
        jade_pptx_writer::validate_slide_reset(self.document.bytes(), index).map_err(js_error)
    }
    #[wasm_bindgen(js_name = resetSlide)]
    pub fn reset_slide(&mut self, index: usize) -> Result<bool, JsValue> {
        self.validate_slide_reset(index)?;
        self.slide_edit(|session| session.reset_slide(index))
    }
    #[wasm_bindgen(js_name = validateSlideLayouts)]
    pub fn validate_slide_layouts(&self, json: &str, layout: &str) -> Result<(), JsValue> {
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        jade_pptx_writer::validate_slide_layouts(self.document.bytes(), &indices, layout)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = setSlideLayouts)]
    pub fn set_slide_layouts(&mut self, json: &str, layout: &str) -> Result<bool, JsValue> {
        self.validate_slide_layouts(json, layout)?;
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        self.slide_edit(|s| s.set_slide_layouts(&indices, layout))
    }
    #[wasm_bindgen(js_name = validateSlideResets)]
    pub fn validate_slide_resets(&self, json: &str) -> Result<(), JsValue> {
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        jade_pptx_writer::validate_slide_resets(self.document.bytes(), &indices).map_err(js_error)
    }
    #[wasm_bindgen(js_name = resetSlides)]
    pub fn reset_slides(&mut self, json: &str) -> Result<bool, JsValue> {
        self.validate_slide_resets(json)?;
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        self.slide_edit(|s| s.reset_slides(&indices))
    }
    #[wasm_bindgen(js_name = notesText)]
    pub fn notes_text(&self, index: usize) -> Result<String, JsValue> {
        let value = jade_pptx_writer::notes_text(self.document.bytes(), index).map_err(js_error)?;
        serde_json::to_string(&value).map_err(js_error)
    }
    #[wasm_bindgen(js_name = setSlideNotes)]
    pub fn set_slide_notes(
        &mut self,
        index: usize,
        text: &str,
        soft_break: bool,
    ) -> Result<bool, JsValue> {
        self.slide_edit(|session| session.set_slide_notes_with_breaks(index, text, soft_break))
    }
    #[wasm_bindgen(js_name = slideId)]
    pub fn slide_id(&self, index: usize) -> Option<u32> {
        self.inner.slides.get(index).and_then(|s| s.id)
    }
    #[wasm_bindgen(js_name = slideHidden)]
    pub fn slide_hidden(&self, index: usize) -> bool {
        self.inner
            .slides
            .get(index)
            .is_some_and(|s| s.content.hidden)
    }
    #[wasm_bindgen(js_name = historyDepth)]
    pub fn history_depth(&self) -> Vec<usize> {
        let (undo, redo) = self.document.history_depth();
        vec![undo, redo]
    }
    #[wasm_bindgen(js_name = duplicateSlide)]
    pub fn duplicate_slide(&mut self, index: usize) -> Result<bool, JsValue> {
        self.duplicate_slides(&serde_json::to_string(&vec![index]).map_err(js_error)?)
    }
    #[wasm_bindgen(js_name = validateSlideDuplications)]
    pub fn validate_slide_duplications(&self, json: &str) -> Result<(), JsValue> {
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        jade_pptx_writer::validate_slide_duplications(self.document.bytes(), &indices)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = duplicateSlides)]
    pub fn duplicate_slides(&mut self, json: &str) -> Result<bool, JsValue> {
        self.validate_slide_duplications(json)?;
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        self.slide_edit(|session| session.duplicate_slides(&indices))
    }
    #[wasm_bindgen(js_name = deleteSlide)]
    pub fn delete_slide(&mut self, index: usize) -> Result<bool, JsValue> {
        self.delete_slides(&serde_json::to_string(&vec![index]).map_err(js_error)?)
    }
    #[wasm_bindgen(js_name = validateSlideDeletions)]
    pub fn validate_slide_deletions(&self, json: &str) -> Result<(), JsValue> {
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        jade_pptx_writer::validate_slide_deletions(self.document.bytes(), &indices)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = deleteSlides)]
    pub fn delete_slides(&mut self, json: &str) -> Result<bool, JsValue> {
        self.validate_slide_deletions(json)?;
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        self.slide_edit(|s| s.delete_slides(&indices))
    }
    #[wasm_bindgen(js_name = insertSlide)]
    pub fn insert_slide(&mut self, index: usize) -> Result<bool, JsValue> {
        self.slide_edit(|session| session.insert_slide(index))
    }
    #[wasm_bindgen(js_name = moveSlide)]
    pub fn move_slide(&mut self, from: usize, to: usize) -> Result<bool, JsValue> {
        self.move_slides(&serde_json::to_string(&vec![from]).map_err(js_error)?, to)
    }
    #[wasm_bindgen(js_name = validateSlideMoves)]
    pub fn validate_slide_moves(&self, json: &str, to: usize) -> Result<(), JsValue> {
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        jade_pptx_writer::validate_slide_moves(self.document.bytes(), &indices, to)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = moveSlides)]
    pub fn move_slides(&mut self, json: &str, to: usize) -> Result<bool, JsValue> {
        self.validate_slide_moves(json, to)?;
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        self.slide_edit(|session| session.move_slides(&indices, to))
    }
    #[wasm_bindgen(js_name = setSlideHidden)]
    pub fn set_slide_hidden(&mut self, index: usize, hidden: bool) -> Result<bool, JsValue> {
        self.set_slides_hidden(
            &serde_json::to_string(&vec![index]).map_err(js_error)?,
            hidden,
        )
    }
    #[wasm_bindgen(js_name = validateSlidesHidden)]
    pub fn validate_slides_hidden(&self, json: &str, hidden: bool) -> Result<(), JsValue> {
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        jade_pptx_writer::validate_slides_hidden(self.document.bytes(), &indices, hidden)
            .map_err(js_error)
    }
    #[wasm_bindgen(js_name = setSlidesHidden)]
    pub fn set_slides_hidden(&mut self, json: &str, hidden: bool) -> Result<bool, JsValue> {
        self.validate_slides_hidden(json, hidden)?;
        let indices: Vec<usize> = serde_json::from_str(json).map_err(js_error)?;
        self.slide_edit(|session| session.set_slides_hidden(&indices, hidden))
    }
    fn slide_edit(
        &mut self,
        edit: impl FnOnce(&mut jade_pptx_writer::EditSession) -> jade_pptx_writer::Result<bool>,
    ) -> Result<bool, JsValue> {
        self.cancel_object_preview()?;
        let changed = edit(&mut self.document).map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }
    #[wasm_bindgen(js_name = hasImage)]
    pub fn has_image(&self, part: &str) -> bool {
        self.images.borrow().contains_key(part)
    }
}
