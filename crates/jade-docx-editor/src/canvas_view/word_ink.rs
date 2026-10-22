//! The browser entry points for Word's 描画 tab: ink kept as Word keeps it
//! (`w14:contentPart` with an InkML part), so Word opens the strokes as
//! ink it can recolour and erase, not as a picture.

use super::*;
use crate::commands::set_ink_drawings::SetInkDrawingsCommand;
use crate::word_document::ink_drawing::{InkDrawingInfo, InkDrawingSpec};
use jade_docx_reader::inkml::InkTrace;

impl OffscreenDocumentView {
    fn ink_drawing_list(&self) -> Vec<InkDrawingInfo> {
        self.state
            .borrow()
            .word_doc
            .as_ref()
            .map(WordDocument::ink_drawings)
            .unwrap_or_default()
    }

    fn keep_ink(drawings: &[InkDrawingInfo], except: Option<usize>) -> Vec<InkDrawingSpec> {
        drawings
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != except)
            .map(|(_, drawing)| InkDrawingSpec::Keep {
                paragraph_index: drawing.paragraph_index,
                run_index: drawing.run_index,
            })
            .collect()
    }

    fn set_ink(&self, specs: Vec<InkDrawingSpec>) -> Result<JsValue, JsValue> {
        self.apply_document_command(move |_| Box::new(SetInkDrawingsCommand::new(specs)))
    }
}

#[wasm_bindgen]
impl OffscreenDocumentView {
    /// The body's ink drawings, in document order, with their strokes (px
    /// within each drawing's box).
    #[wasm_bindgen(js_name = inkDrawings)]
    pub fn ink_drawings(&self) -> JsValue {
        self.ink_drawing_list()
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .unwrap_or(JsValue::NULL)
    }

    /// A new ink drawing of `traces` (`[{ points: [[x, y], …], brush }]`, in
    /// page px from the page's top-left) anchored in the caret's paragraph;
    /// the other drawings stay.
    #[wasm_bindgen(js_name = addInkDrawing)]
    pub fn add_ink_drawing(&self, traces: JsValue) -> Result<JsValue, JsValue> {
        let traces: Vec<InkTrace> = serde_wasm_bindgen::from_value(traces)
            .map_err(|error| JsValue::from_str(&format!("invalid ink: {error}")))?;
        if traces.iter().all(|trace| trace.points.is_empty()) {
            return Err(JsValue::from_str("ink needs at least one stroke"));
        }
        let (start, _) = self.current_selection().normalized();
        let mut specs = Self::keep_ink(&self.ink_drawing_list(), None);
        specs.push(InkDrawingSpec::New {
            paragraph_index: start.paragraph,
            traces,
        });
        self.set_ink(specs)
    }

    /// Erases ink drawing `index` (as `inkDrawings` lists them).
    #[wasm_bindgen(js_name = removeInkDrawing)]
    pub fn remove_ink_drawing(&self, index: usize) -> Result<JsValue, JsValue> {
        let drawings = self.ink_drawing_list();
        if index >= drawings.len() {
            return Err(JsValue::from_str("no such ink drawing"));
        }
        self.set_ink(Self::keep_ink(&drawings, Some(index)))
    }

    /// Erases every ink drawing in the body.
    #[wasm_bindgen(js_name = clearInkDrawings)]
    pub fn clear_ink_drawings(&self) -> Result<JsValue, JsValue> {
        self.set_ink(Vec::new())
    }
}
