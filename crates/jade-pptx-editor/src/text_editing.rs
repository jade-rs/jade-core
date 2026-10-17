use crate::{js_error, Deck};
use jade_canvas_painter::Canvas2dPainter;
use jade_pptx_layout::{PlacedOrigin, PlacedShape, TextMeasure};
use jade_pptx_render::{PainterMeasure, RenderOptions};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectionRequest {
    shape_id: u32,
    start: usize,
    end: usize,
    focus: usize,
    #[serde(default)]
    upstream: bool,
    #[serde(default)]
    preferred_x: Option<f64>,
}

#[wasm_bindgen]
impl Deck {
    #[wasm_bindgen(js_name = endTextHistory)]
    pub fn end_text_history(&mut self) {
        self.document.end_text_history();
    }
    #[wasm_bindgen(js_name = applyNotesInput)]
    pub fn apply_notes_input(&mut self, index: usize, json: &str) -> Result<String, JsValue> {
        let edit: jade_pptx_writer::NotesInputEdit =
            serde_json::from_str(json).map_err(js_error)?;
        self.cancel_object_preview()?;
        let result = self
            .document
            .apply_notes_input(index, &edit)
            .map_err(js_error)?;
        if result.changed {
            self.reload_document()?;
        }
        serde_json::to_string(&result).map_err(js_error)
    }
    #[wasm_bindgen(js_name = applyTextInput)]
    pub fn apply_text_input(&mut self, index: usize, json: &str) -> Result<String, JsValue> {
        let edit: jade_pptx_writer::TextInputEdit = serde_json::from_str(json).map_err(js_error)?;
        self.cancel_object_preview()?;
        let result = self
            .document
            .apply_text_input(index, &edit)
            .map_err(js_error)?;
        if result.changed {
            self.reload_document()?;
        }
        serde_json::to_string(&result).map_err(js_error)
    }
    #[wasm_bindgen(js_name = previewTextInput)]
    pub fn preview_text_input(&mut self, index: usize, json: &str) -> Result<(), JsValue> {
        let edit: jade_pptx_writer::TextInputEdit = serde_json::from_str(json).map_err(js_error)?;
        self.cancel_object_preview()?;
        let mut preview =
            jade_pptx_writer::EditSession::open(self.document.bytes()).map_err(js_error)?;
        preview.apply_text_input(index, &edit).map_err(js_error)?;
        self.inner = jade_pptx_reader::read_deck(preview.bytes()).map_err(js_error)?;
        self.cached.get_mut().clear();
        self.object_info.get_mut().clear();
        self.previewing = true;
        Ok(())
    }
    #[wasm_bindgen(js_name = paragraphFormat)]
    pub fn paragraph_format(&self, index: usize, json: &str) -> Result<String, JsValue> {
        let request: SelectionRequest = serde_json::from_str(json).map_err(js_error)?;
        serde_json::to_string(&jade_pptx_layout::paragraph_format_for_deck_shape(
            &self.inner,
            index,
            request.shape_id,
            request.start,
            request.end,
        ))
        .map_err(js_error)
    }
    #[wasm_bindgen(js_name = formatParagraphs)]
    pub fn format_paragraphs(&mut self, index: usize, json: &str) -> Result<bool, JsValue> {
        let edit: jade_pptx_writer::ParagraphFormatEdit =
            serde_json::from_str(json).map_err(js_error)?;
        self.cancel_object_preview()?;
        let changed = self
            .document
            .format_paragraphs(index, &edit)
            .map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }

    #[wasm_bindgen(js_name = textFormat)]
    pub fn text_format(&self, index: usize, json: &str) -> Result<String, JsValue> {
        let request: SelectionRequest = serde_json::from_str(json).map_err(js_error)?;
        let value = jade_pptx_layout::text_format_for_deck_shape(
            &self.inner,
            index,
            request.shape_id,
            request.start,
            request.end,
        );
        serde_json::to_string(&value).map_err(js_error)
    }

    #[wasm_bindgen(js_name = formatTextRange)]
    pub fn format_text_range(&mut self, index: usize, json: &str) -> Result<bool, JsValue> {
        let edit: jade_pptx_writer::TextFormatEdit =
            serde_json::from_str(json).map_err(js_error)?;
        self.cancel_object_preview()?;
        let changed = self
            .document
            .format_text_range(index, &edit)
            .map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }

    #[wasm_bindgen(js_name = setShapeTextFormatted)]
    pub fn set_shape_text_formatted(
        &mut self,
        index: usize,
        id: u32,
        text: &str,
        soft_break: bool,
        json: &str,
    ) -> Result<bool, JsValue> {
        let format: jade_pptx_writer::TextFormat = serde_json::from_str(json).map_err(js_error)?;
        self.cancel_object_preview()?;
        let changed = self
            .document
            .set_shape_text_formatted(index, id, text, soft_break, &format)
            .map_err(js_error)?;
        if changed {
            self.reload_document()?;
        }
        Ok(changed)
    }

    #[wasm_bindgen(js_name = previewShapeTextFormatted)]
    pub fn preview_shape_text_formatted(
        &mut self,
        index: usize,
        id: u32,
        text: &str,
        json: &str,
    ) -> Result<(), JsValue> {
        let format: jade_pptx_writer::TextFormat = serde_json::from_str(json).map_err(js_error)?;
        self.cancel_object_preview()?;
        let mut preview =
            jade_pptx_writer::EditSession::open(self.document.bytes()).map_err(js_error)?;
        preview
            .set_shape_text_formatted(index, id, text, false, &format)
            .map_err(js_error)?;
        self.inner = jade_pptx_reader::read_deck(preview.bytes()).map_err(js_error)?;
        self.cached.get_mut().clear();
        self.object_info.get_mut().clear();
        self.previewing = true;
        Ok(())
    }
    fn query_text<T: Serialize>(
        &self,
        index: usize,
        id: u32,
        canvas: &HtmlCanvasElement,
        query: impl FnOnce(&PlacedShape, &dyn TextMeasure) -> Option<T>,
    ) -> Result<String, JsValue> {
        let context = canvas
            .get_context("2d")?
            .ok_or_else(|| js_error("canvas has no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let layout = self.layout(index, &context)?;
        let shape = layout
            .shapes
            .iter()
            .find(|shape| shape.origin == PlacedOrigin::Slide && shape.source_id == Some(id))
            .ok_or_else(|| js_error("shape has no layout"))?;
        let options = RenderOptions::default();
        let measure = PainterMeasure::new(Canvas2dPainter::new(&context), &options.default_font);
        serde_json::to_string(&query(shape, &measure)).map_err(js_error)
    }

    #[wasm_bindgen(js_name = hitText)]
    pub fn hit_text(
        &self,
        index: usize,
        id: u32,
        x: f64,
        y: f64,
        canvas: &HtmlCanvasElement,
    ) -> Result<String, JsValue> {
        self.query_text(index, id, canvas, |shape, measure| {
            shape.hit_text(x, y, measure)
        })
    }

    #[wasm_bindgen(js_name = textSelection)]
    pub fn text_selection(
        &self,
        index: usize,
        json: &str,
        canvas: &HtmlCanvasElement,
    ) -> Result<String, JsValue> {
        let request: SelectionRequest = serde_json::from_str(json).map_err(js_error)?;
        if request.start > request.end
            || request.focus < request.start
            || request.focus > request.end
        {
            return Err(js_error("invalid text selection"));
        }
        self.query_text(index, request.shape_id, canvas, |shape, measure| {
            shape.selection_geometry(
                request.start,
                request.end,
                request.focus,
                request.upstream,
                measure,
            )
        })
    }

    #[wasm_bindgen(js_name = moveTextCaret)]
    pub fn move_text_caret(
        &self,
        index: usize,
        json: &str,
        direction: &str,
        canvas: &HtmlCanvasElement,
    ) -> Result<String, JsValue> {
        let request: SelectionRequest = serde_json::from_str(json).map_err(js_error)?;
        self.query_text(index, request.shape_id, canvas, |shape, measure| {
            shape.move_text_caret_with_x(
                request.focus,
                direction,
                request.upstream,
                request.preferred_x,
                measure,
            )
        })
    }

    #[wasm_bindgen(js_name = previewShapeText)]
    pub fn preview_shape_text(&mut self, index: usize, id: u32, text: &str) -> Result<(), JsValue> {
        self.cancel_object_preview()?;
        let mut preview =
            jade_pptx_writer::EditSession::open(self.document.bytes()).map_err(js_error)?;
        preview
            .set_shape_text(index, id, text, false)
            .map_err(js_error)?;
        self.inner = jade_pptx_reader::read_deck(preview.bytes()).map_err(js_error)?;
        self.cached.get_mut().clear();
        self.object_info.get_mut().clear();
        self.previewing = true;
        Ok(())
    }
}
